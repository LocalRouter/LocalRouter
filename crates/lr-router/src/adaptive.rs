//! Passive, bounded process-local measurements and cancellation-safe load tracking.
use lr_config::{AdaptiveRoutingConfig, CandidateOrdering};
use lr_providers::{CompletionChunk, EndpointType};
use lr_types::{AppError, AppResult};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use std::time::{Duration, Instant};
use utoipa::ToSchema;

const MAX_KEYS: usize = 1024;
const MAX_SAMPLES: usize = 128;
#[derive(Clone, PartialEq, Eq, Hash)]
struct Key([u8; 32], EndpointType, u64);
impl Key {
    fn new(provider: &str, model: &str, endpoint: EndpointType, revision: u64) -> Self {
        let mut hash = Sha256::new();
        hash.update(provider.len().to_le_bytes());
        hash.update(provider.as_bytes());
        hash.update(model.as_bytes());
        Self(hash.finalize().into(), endpoint, revision)
    }
}
#[derive(Clone)]
struct Sample {
    at: Instant,
    ttft_ms: Option<f64>,
    output_tps: Option<f64>,
}
#[derive(Default)]
struct Entry {
    in_flight: usize,
    samples: VecDeque<Sample>,
    failures: u32,
    cooldown: Option<Instant>,
}
#[derive(Default)]
pub struct AdaptiveMeasurements {
    entries: Mutex<HashMap<Key, Arc<Mutex<Entry>>>>,
}
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct ObservedPerformance {
    pub in_flight: Option<usize>,
    pub ttft_ms: Option<f64>,
    pub output_tokens_per_second: Option<f64>,
    pub ttft_samples: usize,
    pub throughput_samples: usize,
    pub retry_after_ms: Option<u64>,
}
impl AdaptiveMeasurements {
    fn entry(&self, key: Key) -> Option<Arc<Mutex<Entry>>> {
        let mut entries = self.entries.lock();
        if let Some(entry) = entries.get(&key) {
            return Some(entry.clone());
        }
        if entries.len() >= MAX_KEYS {
            // Never evict an active permit or a recovery state.
            let victim = entries
                .iter()
                .filter(|(_, entry)| {
                    let entry = entry.lock();
                    entry.in_flight == 0
                        && entry.cooldown.is_none_or(|until| until <= Instant::now())
                })
                .min_by_key(|(_, entry)| entry.lock().samples.back().map(|sample| sample.at))
                .map(|(key, _)| key.clone());
            let victim = victim?;
            entries.remove(&victim);
        }
        let entry = Arc::new(Mutex::new(Entry::default()));
        entries.insert(key, entry.clone());
        Some(entry)
    }
    pub fn record_infrastructure_failure(
        &self,
        provider: &str,
        model: &str,
        endpoint: EndpointType,
        revision: u64,
        enabled: bool,
    ) {
        if enabled {
            let mut observation = self.begin(provider, model, endpoint, revision);
            observation.infrastructure_failure(true);
        }
    }
    pub fn begin(
        &self,
        provider: &str,
        model: &str,
        endpoint: EndpointType,
        revision: u64,
    ) -> AttemptObservation {
        let entry = self.entry(Key::new(provider, model, endpoint, revision));
        if let Some(entry) = &entry {
            entry.lock().in_flight += 1;
        }
        AttemptObservation {
            entry,
            start: Instant::now(),
            first_output: None,
            output_tokens: None,
            completed: false,
        }
    }
    pub fn snapshot(
        &self,
        provider: &str,
        model: &str,
        endpoint: EndpointType,
        config: &AdaptiveRoutingConfig,
        revision: u64,
    ) -> ObservedPerformance {
        let entries = self.entries.lock();
        let key = Key::new(provider, model, endpoint, revision);
        let entry = entries.get(&key);
        let matching: Vec<_> = entries
            .iter()
            .filter(|(candidate, _)| candidate.0 == key.0)
            .map(|(candidate, entry)| {
                let entry = entry.lock();
                (candidate.2, entry.in_flight, entry.cooldown)
            })
            .collect();
        let in_flight = if matching
            .iter()
            .any(|(epoch, count, _)| *epoch != revision && *count > 0)
        {
            None
        } else {
            Some(
                matching
                    .iter()
                    .filter(|(epoch, _, _)| *epoch == revision)
                    .map(|(_, count, _)| *count)
                    .sum(),
            )
        };
        let retry_after_ms = matching
            .iter()
            .filter(|(epoch, _, _)| *epoch == revision)
            .filter_map(|(_, _, until)| {
                until.and_then(|until| until.checked_duration_since(Instant::now()))
            })
            .max()
            .map(|duration| duration.as_millis().min(u64::MAX as u128) as u64);
        let mut result = ObservedPerformance {
            in_flight,
            ttft_ms: None,
            output_tokens_per_second: None,
            ttft_samples: 0,
            throughput_samples: 0,
            retry_after_ms,
        };
        let Some(entry) = entry else {
            if entries.len() >= MAX_KEYS && matching.is_empty() {
                result.in_flight = None;
            }
            return result;
        };
        let entry = entry.lock();
        let mut ttft = 0.0;
        let mut tps = 0.0;
        for sample in entry.samples.iter().filter(|sample| {
            sample.at.elapsed() <= Duration::from_secs(config.observation_window_secs)
        }) {
            if let Some(value) = sample.ttft_ms {
                ttft += value;
                result.ttft_samples += 1;
            }
            if let Some(value) = sample.output_tps {
                tps += value;
                result.throughput_samples += 1;
            }
        }
        if result.ttft_samples >= config.min_samples {
            result.ttft_ms = Some(ttft / result.ttft_samples as f64);
        }
        if result.throughput_samples >= config.min_samples {
            result.output_tokens_per_second = Some(tps / result.throughput_samples as f64);
        }
        result
    }
}
/// Owned by an execution future or returned stream. Dropping releases load even on cancellation.
pub struct AttemptObservation {
    entry: Option<Arc<Mutex<Entry>>>,
    start: Instant,
    first_output: Option<Instant>,
    output_tokens: Option<u32>,
    completed: bool,
}
impl AttemptObservation {
    pub fn observe(&mut self, chunk: &CompletionChunk) {
        if self.first_output.is_none()
            && lr_providers::usage_estimate::chunk_output_chars(chunk) > 0
        {
            self.first_output = Some(Instant::now());
        }
        if let Some(usage) = &chunk.usage {
            self.output_tokens = Some(usage.completion_tokens);
        }
    }
    pub fn success(&mut self) {
        self.completed = true;
        if let Some(entry) = &self.entry {
            let mut entry = entry.lock();
            entry.failures = 0;
            entry.cooldown = None;
        }
    }
    pub fn infrastructure_failure(&mut self, enabled: bool) {
        if !enabled {
            return;
        }
        if let Some(entry) = &self.entry {
            let mut entry = entry.lock();
            entry.failures = entry.failures.saturating_add(1);
            // Capped exponential recovery, with stable jitter that never launches a probe.
            let base = 1000u64 * (1u64 << entry.failures.min(5));
            let jitter = (self.start.elapsed().as_micros() as u64 % 251) * base / 1000;
            entry.cooldown =
                Some(Instant::now() + Duration::from_millis((base + jitter).min(60_000)));
        }
    }
}
impl Drop for AttemptObservation {
    fn drop(&mut self) {
        let Some(entry) = &self.entry else {
            return;
        };
        let mut entry = entry.lock();
        entry.in_flight = entry.in_flight.saturating_sub(1);
        let ttft_ms = self
            .first_output
            .map(|at| at.duration_since(self.start).as_secs_f64() * 1000.0);
        let output_tps = if self.completed {
            self.first_output
                .zip(self.output_tokens)
                .and_then(|(at, tokens)| {
                    let secs = at.elapsed().as_secs_f64();
                    (secs > 0.001 && tokens > 0).then_some(tokens as f64 / secs)
                })
        } else {
            None
        };
        if ttft_ms.is_some() || output_tps.is_some() {
            entry.samples.push_back(Sample {
                at: Instant::now(),
                ttft_ms,
                output_tps,
            });
            while entry.samples.len() > MAX_SAMPLES {
                entry.samples.pop_front();
            }
        }
    }
}
#[derive(Default)]
struct DispatchLimit {
    maximum: Option<usize>,
    infrastructure_cooldown: bool,
    calls: AtomicUsize,
    costs: Mutex<CompletedCosts>,
}
#[derive(Default)]
struct CompletedCosts {
    reported: f64,
    estimated: f64,
    reported_calls: usize,
    estimated_calls: usize,
}
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct AttemptCosts {
    /// Partial sums; unknown calls may add further cost.
    pub reported_usd: Option<f64>,
    pub estimated_usd: Option<f64>,
    pub unknown_calls: usize,
}
#[derive(Clone)]
pub struct CostHandle(Arc<DispatchLimit>);
impl CostHandle {
    pub fn record(&self, cost: f64, reported: bool) {
        if !cost.is_finite() || cost < 0.0 {
            return;
        }
        let mut totals = self.0.costs.lock();
        if reported {
            if (totals.reported + cost).is_finite() {
                totals.reported += cost;
                totals.reported_calls += 1;
            }
        } else if (totals.estimated + cost).is_finite() {
            totals.estimated += cost;
            totals.estimated_calls += 1;
        }
    }
    pub fn snapshot(&self) -> AttemptCosts {
        let costs = self.0.costs.lock();
        AttemptCosts {
            reported_usd: (costs.reported_calls > 0).then_some(costs.reported),
            estimated_usd: (costs.estimated_calls > 0).then_some(costs.estimated),
            unknown_calls: self
                .0
                .calls
                .load(Ordering::SeqCst)
                .saturating_sub(costs.reported_calls + costs.estimated_calls),
        }
    }
}
pub(crate) fn record_completed_cost(cost: f64, reported: bool) {
    let _ =
        ACTIVE_DISPATCH_LIMIT.try_with(|limit| CostHandle(limit.clone()).record(cost, reported));
}
tokio::task_local! { static ACTIVE_DISPATCH_LIMIT: Arc<DispatchLimit>; static INFERENCE_DEADLINE: Option<tokio::time::Instant>; }
pub(crate) async fn with_inference_deadline<F: Future>(
    deadline: Option<tokio::time::Instant>,
    future: F,
) -> F::Output {
    INFERENCE_DEADLINE.scope(deadline, future).await
}
/// Counts each native call or each chat call made by decision emulation.
/// Model discovery is preflight; classifier spending is separately reported.
pub(crate) fn reserve_inference_dispatch() -> AppResult<()> {
    if INFERENCE_DEADLINE
        .try_with(|deadline| {
            deadline.is_some_and(|deadline| tokio::time::Instant::now() >= deadline)
        })
        .unwrap_or(false)
    {
        return Err(AppError::RoutingDeadlineExceeded { scope: "attempt" });
    }
    ACTIVE_DISPATCH_LIMIT
        .try_with(|limit| {
            let reserved = limit
                .calls
                .try_update(Ordering::SeqCst, Ordering::SeqCst, |calls| {
                    limit
                        .maximum
                        .is_none_or(|maximum| calls < maximum)
                        .then_some(calls + 1)
                });
            reserved.map(|_| ()).map_err(|_| {
                AppError::Provider("Routing max_attempts_exceeded before dispatch".into())
            })
        })
        .unwrap_or(Ok(()))
}

pub(crate) fn current_infrastructure_cooldown() -> bool {
    ACTIVE_DISPATCH_LIMIT
        .try_with(|limit| limit.infrastructure_cooldown)
        .unwrap_or(false)
}

/// One budget for classification, preflight, dispatches and final stream lifetime.
pub struct RoutingBudget {
    pub deadline: Option<tokio::time::Instant>,
    config: AdaptiveRoutingConfig,
    calls: Arc<DispatchLimit>,
    inherited_turn: bool,
}
impl RoutingBudget {
    pub fn new(config: &AdaptiveRoutingConfig) -> Self {
        if let Some(turn) = current_turn() {
            let local = config
                .total_timeout_ms
                .map(|ms| tokio::time::Instant::now() + Duration::from_millis(ms));
            return Self {
                deadline: match (local, turn.budget.deadline) {
                    (Some(a), Some(b)) => Some(a.min(b)),
                    (a, b) => a.or(b),
                },
                config: config.clone(),
                calls: turn.budget.calls.clone(),
                inherited_turn: true,
            };
        }
        Self {
            inherited_turn: false,
            deadline: config
                .total_timeout_ms
                .map(|ms| tokio::time::Instant::now() + Duration::from_millis(ms)),
            config: config.clone(),
            calls: Arc::new(DispatchLimit {
                maximum: config.max_attempts,
                infrastructure_cooldown: config.infrastructure_cooldown,
                calls: AtomicUsize::new(0),
                costs: Mutex::new(CompletedCosts::default()),
            }),
        }
    }
    pub fn attempt_deadline(&self) -> Option<tokio::time::Instant> {
        self.config
            .attempt_timeout_ms
            .map(|ms| tokio::time::Instant::now() + Duration::from_millis(ms))
    }
    pub fn cost_handle(&self) -> CostHandle {
        CostHandle(self.calls.clone())
    }
    pub fn costs(&self) -> AttemptCosts {
        self.cost_handle().snapshot()
    }
    pub fn account_classifier_latency(&mut self, latency_ms: u64) {
        if self.inherited_turn {
            return;
        }
        if let Some(deadline) = self.deadline {
            self.deadline = Some(
                deadline
                    .checked_sub(Duration::from_millis(latency_ms))
                    .unwrap_or_else(tokio::time::Instant::now),
            );
        }
    }
    pub fn dispatch_count(&self) -> usize {
        self.calls.calls.load(Ordering::SeqCst)
    }
    pub fn exhausted(&self) -> Option<&'static str> {
        if self
            .deadline
            .is_some_and(|deadline| tokio::time::Instant::now() >= deadline)
        {
            return Some("total_deadline_exceeded");
        }
        if self
            .config
            .max_attempts
            .is_some_and(|maximum| self.dispatch_count() >= maximum)
        {
            return Some("max_attempts_exceeded");
        }
        None
    }
    pub async fn preflight<T>(&self, future: impl Future<Output = T>) -> Result<T, &'static str> {
        if let Some(deadline) = self.deadline {
            let result = tokio::time::timeout_at(deadline, future)
                .await
                .map_err(|_| "total_deadline_exceeded")?;
            if tokio::time::Instant::now() >= deadline {
                Err("total_deadline_exceeded")
            } else {
                Ok(result)
            }
        } else {
            Ok(future.await)
        }
    }
    pub async fn dispatch<T>(
        &mut self,
        future: impl Future<Output = AppResult<T>>,
    ) -> AppResult<T> {
        if let Some(reason) = self.exhausted() {
            return Err(if reason == "total_deadline_exceeded" {
                AppError::RoutingDeadlineExceeded { scope: "total" }
            } else {
                AppError::Provider(format!("Routing {reason}"))
            });
        }
        let per_attempt = self
            .config
            .attempt_timeout_ms
            .map(|ms| tokio::time::Instant::now() + Duration::from_millis(ms));
        let deadline = match (per_attempt, self.deadline) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        let future = with_inference_deadline(
            deadline,
            ACTIVE_DISPATCH_LIMIT.scope(self.calls.clone(), future),
        );
        if let Some(deadline) = deadline {
            let result = tokio::time::timeout_at(deadline, future).await;
            let scope = if self
                .deadline
                .is_some_and(|total| tokio::time::Instant::now() >= total)
            {
                "total"
            } else {
                "attempt"
            };
            if tokio::time::Instant::now() >= deadline {
                Err(AppError::RoutingDeadlineExceeded { scope })
            } else {
                result.unwrap_or(Err(AppError::RoutingDeadlineExceeded { scope }))
            }
        } else {
            future.await
        }
    }
}

pub struct TurnBudget {
    budget: RoutingBudget,
    latest: Mutex<Option<serde_json::Value>>,
}
tokio::task_local! { static TURN_BUDGET: Arc<TurnBudget>; }
pub fn current_turn() -> Option<Arc<TurnBudget>> {
    TURN_BUDGET.try_with(Arc::clone).ok()
}
pub async fn within_turn<F: Future>(turn: Arc<TurnBudget>, future: F) -> F::Output {
    TURN_BUDGET.scope(turn, future).await
}
pub(crate) fn remember_turn_metadata(metadata: &serde_json::Value) {
    if let Some(turn) = current_turn() {
        *turn.latest.lock() = Some(metadata.clone());
    }
}
impl TurnBudget {
    pub fn new(config: &AdaptiveRoutingConfig, classifier_latency: u64) -> Arc<Self> {
        if let Some(current) = current_turn() {
            return current;
        }
        let mut budget = RoutingBudget::new(config);
        budget.account_classifier_latency(classifier_latency);
        Arc::new(Self {
            budget,
            latest: Mutex::new(None),
        })
    }
    pub fn attempt_deadline(&self) -> Option<tokio::time::Instant> {
        self.budget.attempt_deadline()
    }
    pub fn deadline(&self) -> Option<tokio::time::Instant> {
        self.budget.deadline
    }
    pub fn cost_handle(&self) -> CostHandle {
        self.budget.cost_handle()
    }
    pub fn metadata(&self) -> Option<serde_json::Value> {
        self.latest.lock().clone().map(|mut metadata| {
            metadata["scope"] = serde_json::json!("mcp_turn");
            metadata["inference_dispatches"] = serde_json::json!(self.budget.dispatch_count());
            metadata["attempt_costs"] = serde_json::json!(self.budget.costs());
            metadata
        })
    }
    pub fn annotate_error(&self, error: AppError, mut fallback: serde_json::Value) -> AppError {
        if let AppError::RoutingFailure { explanation, .. }
        | AppError::RoutingExhausted { explanation, .. } = &error
        {
            fallback = (**explanation).clone();
        } else if let Some(metadata) = self.metadata() {
            fallback = metadata;
        }
        fallback["scope"] = serde_json::json!("mcp_turn");
        fallback["inference_dispatches"] = serde_json::json!(self.budget.dispatch_count());
        fallback["attempt_costs"] = serde_json::json!(self.budget.costs());
        AppError::RoutingFailure {
            error: Box::new(error),
            explanation: Box::new(fallback),
        }
    }
    pub fn apply_response(&self, response: &mut lr_providers::CompletionResponse) {
        if let Some(metadata) = self.metadata() {
            response
                .extensions
                .get_or_insert_with(Default::default)
                .insert("localrouter.routing".into(), metadata);
        }
    }
}
pub(crate) fn dispatch_in_turn<F, T>(future: F) -> impl Future<Output = AppResult<T>>
where
    F: Future<Output = AppResult<T>>,
{
    let future = Box::pin(future);
    async move {
        if ACTIVE_DISPATCH_LIMIT.try_with(|_| true).unwrap_or(false) {
            return future.await;
        }
        if let Some(turn) = current_turn() {
            let mut budget = RoutingBudget::new(&turn.budget.config);
            budget.dispatch(future).await
        } else {
            future.await
        }
    }
}

/// Ascending score; throughput is negated. Unknown values keep priority after observed entries.
pub fn ordering_score(
    ordering: CandidateOrdering,
    observed: &ObservedPerformance,
    estimated_cost: Option<f64>,
    priority: usize,
) -> Option<f64> {
    match ordering {
        CandidateOrdering::Priority => Some(priority as f64),
        CandidateOrdering::EstimatedCost => estimated_cost,
        CandidateOrdering::ObservedTtft => observed.ttft_ms,
        CandidateOrdering::ObservedThroughput => {
            observed.output_tokens_per_second.map(|value| -value)
        }
        CandidateOrdering::LeastInFlight => observed.in_flight.map(|value| value as f64),
    }
    .filter(|value| value.is_finite())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dropping_cancelled_work_releases_load_without_training_throughput() {
        let store = AdaptiveMeasurements::default();
        let config = AdaptiveRoutingConfig::default();
        let guard = store.begin("a", "m", EndpointType::Chat, 0);
        assert_eq!(
            store
                .snapshot("a", "m", EndpointType::Chat, &config, 0)
                .in_flight,
            Some(1)
        );
        drop(guard);
        let observed = store.snapshot("a", "m", EndpointType::Chat, &config, 0);
        assert_eq!(observed.in_flight, Some(0));
        assert_eq!(observed.throughput_samples, 0);
        assert!(observed.ttft_ms.is_none());
    }
    #[test]
    fn cold_start_expiry_and_minimum_samples_keep_unknowns_explicit() {
        let store = AdaptiveMeasurements::default();
        let config = AdaptiveRoutingConfig {
            min_samples: 2,
            ..Default::default()
        };
        let entry = store
            .entry(Key::new("a", "m", EndpointType::Chat, 0))
            .unwrap();
        entry.lock().samples.extend([
            Sample {
                at: Instant::now() - Duration::from_secs(301),
                ttft_ms: Some(1.0),
                output_tps: Some(900.0),
            },
            Sample {
                at: Instant::now(),
                ttft_ms: Some(50.0),
                output_tps: Some(10.0),
            },
        ]);
        let observed = store.snapshot("a", "m", EndpointType::Chat, &config, 0);
        assert_eq!(observed.ttft_samples, 1);
        assert!(observed.ttft_ms.is_none());
        entry.lock().samples.push_back(Sample {
            at: Instant::now(),
            ttft_ms: Some(100.0),
            output_tps: Some(20.0),
        });
        let observed = store.snapshot("a", "m", EndpointType::Chat, &config, 0);
        assert_eq!(observed.ttft_ms, Some(75.0));
        assert_eq!(observed.output_tokens_per_second, Some(15.0));
        assert!(store
            .snapshot("a", "m", EndpointType::SystemOne, &config, 0)
            .ttft_ms
            .is_none());
        assert!(ordering_score(CandidateOrdering::EstimatedCost, &observed, None, 0).is_none());
        assert_eq!(
            ordering_score(CandidateOrdering::ObservedThroughput, &observed, None, 0),
            Some(-15.0)
        );
    }
    #[test]
    fn recovery_is_bounded_and_success_clears_failure_state() {
        let store = AdaptiveMeasurements::default();
        let config = AdaptiveRoutingConfig::default();
        let mut guard = store.begin("a", "m", EndpointType::Chat, 0);
        for _ in 0..40 {
            guard.infrastructure_failure(true);
        }
        let remaining = store
            .snapshot("a", "m", EndpointType::Chat, &config, 0)
            .retry_after_ms
            .unwrap();
        assert!(remaining <= 60_000 && remaining > 0);
        guard.success();
        assert!(store
            .snapshot("a", "m", EndpointType::Chat, &config, 0)
            .retry_after_ms
            .is_none());
    }
    #[tokio::test]
    async fn dispatch_budget_counts_calls_and_drops_timed_out_work() {
        let config = AdaptiveRoutingConfig {
            max_attempts: Some(1),
            attempt_timeout_ms: Some(100),
            ..Default::default()
        };
        let mut budget = RoutingBudget::new(&config);
        let store = AdaptiveMeasurements::default();
        let guard = store.begin("a", "m", EndpointType::Chat, 0);
        let result: AppResult<()> = budget
            .dispatch(async move {
                let _guard = guard;
                reserve_inference_dispatch()?;
                tokio::time::sleep(Duration::from_secs(1)).await;
                Ok(())
            })
            .await;
        assert!(result.unwrap_err().to_string().contains("deadline"));
        assert_eq!(
            store
                .snapshot("a", "m", EndpointType::Chat, &config, 0)
                .in_flight,
            Some(0)
        );
        assert_eq!(budget.exhausted(), Some("max_attempts_exceeded"));
    }
    #[tokio::test]
    async fn translated_decision_calls_share_one_actual_dispatch_limit() {
        let config = AdaptiveRoutingConfig {
            max_attempts: Some(2),
            ..Default::default()
        };
        let mut budget = RoutingBudget::new(&config);
        let result: AppResult<()> = budget
            .dispatch(async {
                reserve_inference_dispatch()?;
                reserve_inference_dispatch()?;
                reserve_inference_dispatch()?;
                Ok(())
            })
            .await;
        assert!(result.is_err());
        assert_eq!(budget.dispatch_count(), 2);
        // Scope leaves explicit models and independent requests unaffected.
        reserve_inference_dispatch().unwrap();
    }
    #[tokio::test]
    async fn tool_steps_share_calls_costs_and_absolute_deadline() {
        let config = AdaptiveRoutingConfig {
            max_attempts: Some(2),
            total_timeout_ms: Some(1000),
            ..Default::default()
        };
        let turn = TurnBudget::new(&config, 100);
        let deadline = turn.deadline();
        within_turn(turn.clone(), async {
            let mut initial = RoutingBudget::new(&config);
            initial.account_classifier_latency(100);
            assert_eq!(initial.deadline, deadline);
            initial
                .dispatch(async {
                    reserve_inference_dispatch()?;
                    record_completed_cost(0.03, true);
                    Ok(())
                })
                .await
                .unwrap();
            remember_turn_metadata(&serde_json::json!({"selected_route":"keep"}));
            dispatch_in_turn(async {
                reserve_inference_dispatch()?;
                Ok(())
            })
            .await
            .unwrap();
            let denied = dispatch_in_turn(async {
                reserve_inference_dispatch()?;
                panic!("over-budget inference started");
                #[allow(unreachable_code)]
                Ok(())
            })
            .await;
            assert!(denied.is_err());
            let metadata = turn.metadata().unwrap();
            assert_eq!(metadata["inference_dispatches"], 2);
            assert_eq!(metadata["scope"], "mcp_turn");
            assert_eq!(metadata["selected_route"], "keep");
            assert_eq!(turn.cost_handle().snapshot().reported_usd, Some(0.03));
        })
        .await;
        assert!(current_turn().is_none());
        reserve_inference_dispatch().unwrap();
    }
    #[tokio::test]
    async fn expired_tool_turn_cannot_reset_deadline_or_dispatch() {
        let config = AdaptiveRoutingConfig {
            total_timeout_ms: Some(100),
            ..Default::default()
        };
        let turn = TurnBudget::new(&config, 150);
        within_turn(turn.clone(), async {
            let reused = TurnBudget::new(&config, 0);
            assert!(Arc::ptr_eq(&turn, &reused));
            let denied = dispatch_in_turn(async {
                reserve_inference_dispatch()?;
                Ok(())
            })
            .await;
            assert!(matches!(
                denied,
                Err(AppError::RoutingDeadlineExceeded { .. })
            ));
            assert_eq!(turn.budget.dispatch_count(), 0);
            let error = turn.annotate_error(
                denied.unwrap_err(),
                serde_json::json!({"selected_route":"initial"}),
            );
            let AppError::RoutingFailure { explanation, .. } = error else {
                panic!("missing explanation")
            };
            assert_eq!(explanation["selected_route"], "initial");
            assert_eq!(explanation["inference_dispatches"], 0);
        })
        .await;
    }
    #[tokio::test]
    async fn total_deadline_bounds_preflight_before_dispatch() {
        let config = AdaptiveRoutingConfig {
            total_timeout_ms: Some(100),
            ..Default::default()
        };
        let budget = RoutingBudget::new(&config);
        assert_eq!(
            budget
                .preflight(tokio::time::sleep(Duration::from_secs(1)))
                .await,
            Err("total_deadline_exceeded")
        );
        assert_eq!(budget.dispatch_count(), 0);
        assert_eq!(budget.exhausted(), Some("total_deadline_exceeded"));
    }
}

/// Owns the upstream stream and load permit. Errors/deadlines terminate it;
/// the router never starts another model after returning a stream to a caller.
pub fn observe_stream(
    stream: std::pin::Pin<Box<dyn futures::Stream<Item = AppResult<CompletionChunk>> + Send>>,
    observation: AttemptObservation,
    deadline: Option<tokio::time::Instant>,
    first_output_deadline: Option<tokio::time::Instant>,
    infrastructure_cooldown: bool,
) -> std::pin::Pin<Box<dyn futures::Stream<Item = AppResult<CompletionChunk>> + Send>> {
    use futures::StreamExt;
    struct State {
        stream: std::pin::Pin<Box<dyn futures::Stream<Item = AppResult<CompletionChunk>> + Send>>,
        observation: Option<AttemptObservation>,
        deadline: Option<tokio::time::Instant>,
        first_output_deadline: Option<tokio::time::Instant>,
        terminal: bool,
        finished: bool,
        infrastructure_cooldown: bool,
    }
    Box::pin(futures::stream::unfold(
        State {
            stream,
            observation: Some(observation),
            deadline,
            first_output_deadline,
            terminal: false,
            finished: false,
            infrastructure_cooldown,
        },
        |mut state| async move {
            if state.finished {
                return None;
            }
            let effective_deadline = match (state.deadline, state.first_output_deadline) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (a, b) => a.or(b),
            };
            let scope = if effective_deadline == state.deadline {
                "total"
            } else {
                "attempt"
            };
            let next = if let Some(deadline) = effective_deadline {
                if tokio::time::Instant::now() >= deadline {
                    Err(AppError::RoutingDeadlineExceeded { scope })
                } else {
                    let next = tokio::time::timeout_at(deadline, state.stream.next()).await;
                    if tokio::time::Instant::now() >= deadline {
                        Err(AppError::RoutingDeadlineExceeded { scope })
                    } else {
                        next.map_err(|_| AppError::RoutingDeadlineExceeded { scope })
                    }
                }
            } else {
                Ok(state.stream.next().await)
            };
            match next {
                Ok(Some(Ok(chunk))) => {
                    state.terminal |= chunk
                        .choices
                        .iter()
                        .any(|choice| choice.finish_reason.is_some());
                    if let Some(observation) = state.observation.as_mut() {
                        observation.observe(&chunk);
                        if observation.first_output.is_some() || state.terminal {
                            state.first_output_deadline = None;
                        }
                    }
                    Some((Ok(chunk), state))
                }
                Ok(None) => {
                    if state.terminal {
                        if let Some(observation) = state.observation.as_mut() {
                            observation.success();
                        }
                    }
                    None
                }
                Ok(Some(Err(error))) | Err(error) => {
                    if matches!(
                        crate::RouterError::classify(&error, "", ""),
                        crate::RouterError::Unreachable { .. }
                    ) {
                        if let Some(observation) = state.observation.as_mut() {
                            observation.infrastructure_failure(state.infrastructure_cooldown);
                        }
                    }
                    state.observation.take();
                    state.finished = true;
                    Some((Err(error), state))
                }
            }
        },
    ))
}

/// Publish the final attempt-cost snapshot after EOF. Failed/interrupted streams
/// retain an unknown call cost; a partial reported amount is not a final invoice.
pub fn routing_stream(
    stream: std::pin::Pin<Box<dyn futures::Stream<Item = AppResult<CompletionChunk>> + Send>>,
    metadata: serde_json::Value,
    costs: CostHandle,
    pricing: lr_providers::PricingInfo,
) -> std::pin::Pin<Box<dyn futures::Stream<Item = AppResult<CompletionChunk>> + Send>> {
    use futures::StreamExt;
    struct State {
        stream: std::pin::Pin<Box<dyn futures::Stream<Item = AppResult<CompletionChunk>> + Send>>,
        metadata: serde_json::Value,
        costs: CostHandle,
        pricing: lr_providers::PricingInfo,
        reported: Option<f64>,
        usage: Option<lr_providers::TokenUsage>,
        terminal: bool,
        failed: bool,
        first: bool,
        final_sent: bool,
        identity: Option<(String, i64, String, Option<String>)>,
    }
    let state = State {
        stream,
        metadata,
        costs,
        pricing,
        reported: None,
        usage: None,
        terminal: false,
        failed: false,
        first: true,
        final_sent: false,
        identity: None,
    };
    Box::pin(futures::stream::unfold(state, |mut state| async move {
        if state.final_sent {
            return None;
        }
        match state.stream.next().await {
            Some(Ok(mut chunk)) => {
                state.identity = Some((
                    chunk.id.clone(),
                    chunk.created,
                    chunk.model.clone(),
                    chunk.provider.clone(),
                ));
                if let Some(cost) = lr_providers::gateway::reported_cost(chunk.extensions.as_ref())
                {
                    state.reported = Some(cost);
                }
                if let Some(usage) = &chunk.usage {
                    state.usage = Some(usage.clone());
                }
                state.terminal |= chunk
                    .choices
                    .iter()
                    .any(|choice| choice.finish_reason.is_some());
                if state.first {
                    state.first = false;
                    chunk
                        .extensions
                        .get_or_insert_with(Default::default)
                        .insert("localrouter.routing".into(), state.metadata.clone());
                }
                Some((Ok(chunk), state))
            }
            Some(Err(error)) => {
                state.failed = true;
                state.metadata["attempt_costs"] = serde_json::json!(state.costs.snapshot());
                let error = AppError::RoutingFailure {
                    error: Box::new(error),
                    explanation: Box::new(state.metadata.clone()),
                };
                Some((Err(error), state))
            }
            None => {
                if !state.failed && state.terminal {
                    if let Some(cost) = state.reported {
                        state.costs.record(cost, true);
                    } else if let Some(usage) = &state.usage {
                        state.costs.record(state.pricing.cost(usage), false);
                    }
                }
                state.final_sent = true;
                state.metadata["attempt_costs"] = serde_json::json!(state.costs.snapshot());
                remember_turn_metadata(&state.metadata);
                let (id, created, model, provider) = state
                    .identity
                    .clone()
                    .unwrap_or_else(|| ("localrouter-routing".into(), 0, String::new(), None));
                let chunk = CompletionChunk {
                    id,
                    object: "chat.completion.chunk".into(),
                    created,
                    model,
                    provider,
                    choices: Vec::new(),
                    usage: None,
                    extensions: Some(
                        [("localrouter.routing".into(), state.metadata.clone())]
                            .into_iter()
                            .collect(),
                    ),
                };
                Some((Ok(chunk), state))
            }
        }
    }))
}

#[cfg(test)]
mod stream_tests {
    use super::*;
    use futures::StreamExt;
    fn chunk(terminal: bool, cost: Option<f64>) -> CompletionChunk {
        let mut value = serde_json::json!({"id":"stream","object":"chat.completion.chunk","created":1,"model":"m","choices":[{"index":0,"delta":{"content":"answer"},"finish_reason":if terminal {Some("stop")}else{None}}]});
        if let Some(cost) = cost {
            value["extensions"] = serde_json::json!({"localrouter.gateway":{"cost_usd":cost}});
        }
        if terminal {
            value["usage"] =
                serde_json::json!({"prompt_tokens":10,"completion_tokens":2,"total_tokens":12});
        }
        serde_json::from_value(value).unwrap()
    }
    #[tokio::test]
    async fn final_stream_metadata_records_one_reported_cost_without_multiplying_retained_values() {
        let mut budget = RoutingBudget::new(&AdaptiveRoutingConfig::default());
        budget
            .dispatch(async {
                reserve_inference_dispatch()?;
                Ok(())
            })
            .await
            .unwrap();
        let upstream = Box::pin(futures::stream::iter(vec![
            Ok(chunk(false, Some(0.02))),
            Ok(chunk(true, Some(0.02))),
        ]));
        let frames: Vec<_> = routing_stream(
            upstream,
            serde_json::json!({}),
            budget.cost_handle(),
            lr_providers::PricingInfo::free(),
        )
        .collect()
        .await;
        let last = frames.last().unwrap().as_ref().unwrap();
        let trace = &last.extensions.as_ref().unwrap()["localrouter.routing"];
        assert_eq!(trace["attempt_costs"]["reported_usd"], 0.02);
        assert_eq!(trace["attempt_costs"]["unknown_calls"], 0);
        assert_eq!(frames.len(), 3);
    }
    #[tokio::test]
    async fn deadline_after_output_ends_the_stream_and_releases_load_with_unknown_final_cost() {
        let store = AdaptiveMeasurements::default();
        let settings = AdaptiveRoutingConfig {
            total_timeout_ms: Some(100),
            ..Default::default()
        };
        let mut budget = RoutingBudget::new(&settings);
        budget
            .dispatch(async {
                reserve_inference_dispatch()?;
                Ok(())
            })
            .await
            .unwrap();
        let upstream = Box::pin(
            futures::stream::iter(vec![Ok(chunk(false, Some(0.01)))])
                .chain(futures::stream::pending()),
        );
        let observed = observe_stream(
            upstream,
            store.begin("a", "m", EndpointType::Chat, 0),
            budget.deadline,
            None,
            false,
        );
        let mut stream = routing_stream(
            observed,
            serde_json::json!({"explanation":{"reason":"test"}}),
            budget.cost_handle(),
            lr_providers::PricingInfo::free(),
        );
        assert!(stream.next().await.unwrap().is_ok());
        let error = stream.next().await.unwrap().unwrap_err();
        assert!(matches!(error, AppError::RoutingFailure { .. }));
        assert_eq!(
            store
                .snapshot("a", "m", EndpointType::Chat, &settings, 0)
                .in_flight,
            Some(0)
        );
        assert_eq!(
            store
                .snapshot("a", "m", EndpointType::Chat, &settings, 0)
                .throughput_samples,
            0
        );
        let final_metadata = stream.next().await.unwrap().unwrap();
        assert_eq!(
            final_metadata.extensions.unwrap()["localrouter.routing"]["attempt_costs"]
                ["unknown_calls"],
            1
        );
        assert!(stream.next().await.is_none());
        assert_eq!(budget.dispatch_count(), 1);
    }
    #[tokio::test]
    async fn expired_stream_deadline_cannot_be_bypassed_by_ready_chunks() {
        let store = AdaptiveMeasurements::default();
        let upstream = futures::stream::iter(vec![Ok(chunk(true, None))]);
        let mut stream = observe_stream(
            Box::pin(upstream),
            store.begin("a", "m", EndpointType::Chat, 0),
            Some(tokio::time::Instant::now()),
            None,
            false,
        );
        assert!(matches!(
            stream.next().await.unwrap(),
            Err(AppError::RoutingDeadlineExceeded { scope: "total" })
        ));
        assert!(stream.next().await.is_none());
    }
    #[tokio::test]
    async fn first_output_deadline_ends_silent_stream_without_restart() {
        let store = AdaptiveMeasurements::default();
        let mut stream = observe_stream(
            Box::pin(futures::stream::pending()),
            store.begin("a", "m", EndpointType::Chat, 0),
            None,
            Some(tokio::time::Instant::now() + Duration::from_millis(10)),
            false,
        );
        assert!(matches!(
            stream.next().await.unwrap(),
            Err(AppError::RoutingDeadlineExceeded { scope: "attempt" })
        ));
        assert!(stream.next().await.is_none());
        assert_eq!(
            store
                .snapshot("a", "m", EndpointType::Chat, &Default::default(), 0)
                .in_flight,
            Some(0)
        );
    }
    #[tokio::test]
    async fn first_output_clears_attempt_timer_but_keeps_total_deadline() {
        let store = AdaptiveMeasurements::default();
        let upstream =
            futures::stream::iter(vec![Ok(chunk(false, None))]).chain(futures::stream::pending());
        let mut stream = observe_stream(
            Box::pin(upstream),
            store.begin("a", "m", EndpointType::Chat, 0),
            Some(tokio::time::Instant::now() + Duration::from_millis(50)),
            Some(tokio::time::Instant::now() + Duration::from_millis(10)),
            false,
        );
        assert!(stream.next().await.unwrap().is_ok());
        assert!(matches!(
            stream.next().await.unwrap(),
            Err(AppError::RoutingDeadlineExceeded { scope: "total" })
        ));
    }
    #[tokio::test]
    async fn dropping_a_returned_stream_releases_its_permit_before_any_poll() {
        let store = AdaptiveMeasurements::default();
        let stream = observe_stream(
            Box::pin(futures::stream::pending()),
            store.begin("a", "m", EndpointType::Chat, 0),
            None,
            None,
            false,
        );
        assert_eq!(
            store
                .snapshot("a", "m", EndpointType::Chat, &Default::default(), 0)
                .in_flight,
            Some(1)
        );
        drop(stream);
        assert_eq!(
            store
                .snapshot("a", "m", EndpointType::Chat, &Default::default(), 0)
                .in_flight,
            Some(0)
        );
        assert_eq!(
            store
                .snapshot("a", "m", EndpointType::Chat, &Default::default(), 0)
                .throughput_samples,
            0
        );
    }
    #[test]
    fn changed_provider_configuration_invalidates_samples_and_marks_old_active_load_unknown() {
        let store = AdaptiveMeasurements::default();
        let guard = store.begin("a", "m", EndpointType::Chat, 1);
        assert_eq!(
            store
                .snapshot("a", "m", EndpointType::SystemOne, &Default::default(), 1)
                .in_flight,
            Some(1)
        );
        assert_eq!(
            store
                .snapshot("a", "m", EndpointType::Chat, &Default::default(), 2)
                .in_flight,
            None
        );
        drop(guard);
        assert_eq!(
            store
                .snapshot("a", "m", EndpointType::Chat, &Default::default(), 2)
                .in_flight,
            Some(0)
        );
        assert!(store
            .snapshot("a", "m", EndpointType::Chat, &Default::default(), 2)
            .ttft_ms
            .is_none());
    }
}
