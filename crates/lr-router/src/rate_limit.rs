//! Rate limiting system with sliding window algorithm
//!
//! Supports multiple rate limit types:
//! - Request rate limits (requests per time window)
//! - Token rate limits (input/output/total tokens per time window)
//! - Cost rate limits (USD per time window)
//!
//! Supports per-API-key and per-router rate limiting.
//!
//! Usage is recorded when a request completes, so a check against recorded
//! usage alone lets every concurrent request through. Admission therefore
//! reserves: [`RateLimiterManager::reserve_api_key`] checks and holds a
//! request's estimate in one step, and the returned [`AdmissionReservation`]
//! counts as used until it is dropped. Native batches reserve in the durable
//! `lr_batches` ledger; both kinds and recorded usage add up in one check.

use chrono::{DateTime, Duration, Utc};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::fs;
use tracing::{debug, warn};

use lr_types::{AppError, AppResult};

/// Type of rate limit
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RateLimitType {
    /// Limit on number of requests
    Requests,
    /// Limit on input tokens
    InputTokens,
    /// Limit on output tokens
    OutputTokens,
    /// Limit on total tokens (input + output)
    TotalTokens,
    /// Limit on cost in USD
    Cost,
}

/// Rate limiter configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateLimiter {
    /// Type of limit
    pub limit_type: RateLimitType,
    /// Maximum value allowed in the time window
    pub value: f64,
    /// Time window for the limit (in seconds)
    pub time_window_secs: i64,
}

impl RateLimiter {
    /// Create a new rate limiter
    pub fn new(limit_type: RateLimitType, value: f64, time_window_secs: i64) -> Self {
        Self {
            limit_type,
            value,
            time_window_secs,
        }
    }

    /// Get the time window as a Duration
    pub fn time_window(&self) -> Duration {
        Duration::seconds(self.time_window_secs)
    }
}

fn batch_metric(kind: RateLimitType) -> lr_batches::BatchMetric {
    match kind {
        RateLimitType::Requests => lr_batches::BatchMetric::Requests,
        RateLimitType::InputTokens => lr_batches::BatchMetric::InputTokens,
        RateLimitType::OutputTokens => lr_batches::BatchMetric::OutputTokens,
        RateLimitType::TotalTokens => lr_batches::BatchMetric::TotalTokens,
        RateLimitType::Cost => lr_batches::BatchMetric::Cost,
    }
}

/// All windows of one metric share a single history and record each use once.
fn retention_window(limiters: &[RateLimiter], limit_type: RateLimitType) -> Duration {
    limiters
        .iter()
        .filter(|limiter| limiter.limit_type == limit_type)
        .map(RateLimiter::time_window)
        .max()
        .unwrap_or_else(Duration::zero)
}

/// A single usage event in the sliding window
#[derive(Debug, Clone, Serialize, Deserialize)]
struct UsageEvent {
    timestamp: DateTime<Utc>,
    value: f64,
}

/// State for a single rate limiter (sliding window)
#[derive(Debug, Clone, Serialize, Deserialize)]
struct RateLimiterState {
    /// Events in the sliding window (ordered by timestamp)
    events: VecDeque<UsageEvent>,
}

impl RateLimiterState {
    fn new() -> Self {
        Self {
            events: VecDeque::new(),
        }
    }

    /// Remove events older than the time window
    fn cleanup(&mut self, window_start: DateTime<Utc>) {
        while let Some(event) = self.events.front() {
            if event.timestamp < window_start {
                self.events.pop_front();
            } else {
                break;
            }
        }
    }

    /// Calculate total usage in the current window
    fn current_usage(&self, window_start: DateTime<Utc>) -> f64 {
        self.events
            .iter()
            .filter(|e| e.timestamp >= window_start)
            .map(|e| e.value)
            .sum()
    }

    /// Record a new usage event
    fn record(&mut self, timestamp: DateTime<Utc>, value: f64) {
        // Concurrent requests can obtain their timestamps in a different
        // order than they acquire the state lock. Keep expiry order stable.
        let index = self
            .events
            .partition_point(|event| event.timestamp <= timestamp);
        self.events.insert(index, UsageEvent { timestamp, value });
    }
}

type SharedState = Arc<parking_lot::Mutex<RateLimiterState>>;

/// What one admitted request holds while it is in flight. The unit is the
/// one native batches reserve in, so ordinary and batch admissions are summed
/// through the same [`lr_batches::BatchMetric`] projection.
pub type ReservedUsage = lr_batches::BatchUsage;

/// In-flight reservations of one scope (a client or a strategy), by id. The
/// lock also serialises admission for the scope: check and reserve happen
/// under it.
type Scope = Arc<parking_lot::Mutex<HashMap<u64, ReservedUsage>>>;

fn in_flight_total(held: &HashMap<u64, ReservedUsage>) -> ReservedUsage {
    let mut total = ReservedUsage::default();
    let mut cost = 0.0;
    for usage in held.values() {
        total.requests = total.requests.saturating_add(usage.requests);
        total.input_tokens = total.input_tokens.saturating_add(usage.input_tokens);
        total.output_tokens = total.output_tokens.saturating_add(usage.output_tokens);
        cost += usage.cost_usd.unwrap_or(0.0);
    }
    total.cost_usd = Some(cost);
    total
}

/// An admitted request's share of its limits while it is in flight.
///
/// Admission counts it as used, so concurrent requests cannot all pass a
/// check that only sees completed usage. Dropping it releases the share:
/// hold it until the request's actual usage has been recorded (or the request
/// failed or was cancelled) and then let it go.
#[must_use = "dropping an admission releases its reservation"]
#[derive(Debug, Default)]
pub struct AdmissionReservation {
    holds: Vec<(Scope, u64)>,
}

impl AdmissionReservation {
    /// An admission that holds nothing: no limit applies to the request.
    pub fn unreserved() -> Self {
        Self::default()
    }

    /// Whether anything is held.
    pub fn is_reserved(&self) -> bool {
        !self.holds.is_empty()
    }

    /// Carry another admission of the same request (its strategy's, next to
    /// its client's) so both are released together.
    pub fn absorb(&mut self, mut other: AdmissionReservation) {
        self.holds.append(&mut other.holds);
    }
}

impl Drop for AdmissionReservation {
    fn drop(&mut self) {
        for (scope, id) in self.holds.drain(..) {
            scope.lock().remove(&id);
        }
    }
}

/// One configured window of a metric, as reported by
/// [`RateLimiterManager::get_api_key_usage_windows`].
#[derive(Debug, Clone, PartialEq)]
pub struct WindowUsage {
    /// Completed usage recorded in the window.
    pub current_usage: f64,
    /// In-flight reservations, which admission counts on top of
    /// `current_usage`.
    pub reserved: f64,
    pub limit: f64,
    pub window_secs: i64,
    pub window_start: DateTime<Utc>,
}

/// Key for identifying a rate limiter (API key or router + limit type)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RateLimiterKey {
    /// Rate limit for a specific API key
    ApiKey {
        key_id: String,
        limit_type: RateLimitType,
    },
    /// Rate limit for a router
    Router {
        router_name: String,
        limit_type: RateLimitType,
    },
}

impl fmt::Display for RateLimiterKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RateLimiterKey::ApiKey { key_id, limit_type } => {
                write!(f, "apikey:{}:{:?}", key_id, limit_type)
            }
            RateLimiterKey::Router {
                router_name,
                limit_type,
            } => {
                write!(f, "router:{}:{:?}", router_name, limit_type)
            }
        }
    }
}

/// Usage information for a request
#[derive(Debug, Clone)]
pub struct UsageInfo {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd: f64,
}

impl UsageInfo {
    pub fn total_tokens(&self) -> u64 {
        self.input_tokens.saturating_add(self.output_tokens)
    }
}

/// Result of a rate limit check
#[derive(Debug, Clone)]
pub struct RateLimitCheckResult {
    /// Whether the request is allowed
    pub allowed: bool,
    /// If not allowed, time until the limit resets (in seconds)
    pub retry_after_secs: Option<i64>,
    /// Current usage in the window
    pub current_usage: f64,
    /// Maximum allowed usage
    pub limit: f64,
}

/// Manager for all rate limiters
pub struct RateLimiterManager {
    /// Rate limiter configurations (per API key)
    api_key_limiters: Arc<DashMap<String, Vec<RateLimiter>>>,
    /// Rate limiter configurations (per router)
    router_limiters: Arc<DashMap<String, Vec<RateLimiter>>>,
    /// Current state of all rate limiters
    states: Arc<DashMap<String, SharedState>>,
    /// Path for persisting state
    persist_path: Option<PathBuf>,
    batch_store: parking_lot::RwLock<Option<Arc<lr_batches::BatchStore>>>,
    /// In-flight reservations per scope. Never persisted: a request does not
    /// survive a restart.
    in_flight: DashMap<String, Scope>,
    next_reservation: AtomicU64,
}

impl RateLimiterManager {
    /// Create a new rate limiter manager
    pub fn new(persist_path: Option<PathBuf>) -> Self {
        Self {
            api_key_limiters: Arc::new(DashMap::new()),
            router_limiters: Arc::new(DashMap::new()),
            states: Arc::new(DashMap::new()),
            persist_path,
            batch_store: parking_lot::RwLock::new(None),
            in_flight: DashMap::new(),
            next_reservation: AtomicU64::new(1),
        }
    }

    fn api_key_scope(&self, key_id: &str) -> Scope {
        self.scope(format!("apikey:{key_id}"))
    }

    fn scope(&self, name: String) -> Scope {
        self.in_flight.entry(name).or_default().value().clone()
    }

    fn in_flight_of(&self, name: &str) -> ReservedUsage {
        // Clone the scope out so no map shard stays locked behind its mutex.
        let scope = self.in_flight.get(name).map(|scope| scope.value().clone());
        scope.map_or_else(ReservedUsage::default, |scope| {
            in_flight_total(&scope.lock())
        })
    }

    fn hold(
        &self,
        scope: &Scope,
        held: &mut HashMap<u64, ReservedUsage>,
        usage: ReservedUsage,
    ) -> AdmissionReservation {
        let id = self.next_reservation.fetch_add(1, Ordering::Relaxed);
        held.insert(id, usage);
        AdmissionReservation {
            holds: vec![(scope.clone(), id)],
        }
    }

    /// What a client's admitted requests currently hold.
    pub fn api_key_in_flight(&self, key_id: &str) -> ReservedUsage {
        self.in_flight_of(&format!("apikey:{key_id}"))
    }

    /// What a strategy's admitted requests currently hold.
    pub fn strategy_in_flight(&self, strategy_id: &str) -> ReservedUsage {
        if lr_types::is_duplicate_hop() {
            // The first hop of this request holds the reservation.
            return ReservedUsage::default();
        }
        self.in_flight_of(&format!("strategy:{strategy_id}"))
    }

    /// Atomic check-and-reserve for limits whose completed usage lives
    /// elsewhere (a strategy's limits are checked against recorded metrics).
    ///
    /// `admit` sees what the strategy's in-flight requests hold and decides
    /// with it; when it accepts, `estimate` is held until the returned
    /// reservation is dropped. Both happen under the strategy's lock, so
    /// concurrent callers are admitted one at a time.
    pub fn reserve_strategy<E>(
        &self,
        strategy_id: &str,
        estimate: ReservedUsage,
        admit: impl FnOnce(&ReservedUsage) -> Result<(), E>,
    ) -> Result<AdmissionReservation, E> {
        let scope = self.scope(format!("strategy:{strategy_id}"));
        let mut held = scope.lock();
        // A duplicate hop passes through a request whose first hop already
        // holds the reservation: it is neither blocked by that reservation
        // nor holds one of its own.
        if lr_types::is_duplicate_hop() {
            admit(&ReservedUsage::default())?;
            return Ok(AdmissionReservation::unreserved());
        }
        admit(&in_flight_total(&held))?;
        Ok(self.hold(&scope, &mut held, estimate))
    }

    /// Attach the same durable ledger used by native batch submission.
    pub fn set_batch_store(&self, store: Arc<lr_batches::BatchStore>) {
        *self.batch_store.write() = Some(store);
    }

    async fn pending_batch_usage(
        &self,
        key_id: &str,
        window_start: i64,
    ) -> AppResult<lr_batches::BatchUsage> {
        let store = self.batch_store.read().clone();
        let Some(store) = store else {
            return Ok(lr_batches::BatchUsage::default());
        };
        let owner = key_id.to_owned();
        tokio::task::spawn_blocking(move || store.batch_usage(&owner, window_start))
            .await
            .map_err(|_| AppError::Storage("Batch quota task failed".into()))?
            .map_err(|_| AppError::Storage("Batch quota ledger unavailable".into()))
    }

    /// Every configured window, with ordinary usage only. Native reservation
    /// adds the durable batch ledger transactionally; do not include it twice.
    /// Configuration-only check: tool charges lack a reliable upper bound.
    pub fn has_token_limits(&self, key_id: &str) -> bool {
        self.api_key_limiters.get(key_id).is_some_and(|limits| {
            limits.iter().any(|limit| {
                matches!(
                    limit.limit_type,
                    RateLimitType::InputTokens
                        | RateLimitType::OutputTokens
                        | RateLimitType::TotalTokens
                )
            })
        })
    }

    pub fn has_cost_limits(&self, key_id: &str) -> bool {
        self.api_key_limiters.get(key_id).is_some_and(|limits| {
            limits
                .iter()
                .any(|limit| limit.limit_type == RateLimitType::Cost)
        })
    }

    pub async fn batch_budget_snapshot(
        &self,
        key_id: &str,
    ) -> AppResult<Vec<lr_batches::BatchBudget>> {
        let Some(limiters) = self.api_key_limiters.get(key_id).map(|v| v.clone()) else {
            return Ok(Vec::new());
        };
        let now = Utc::now();
        let in_flight = self.api_key_in_flight(key_id);
        let mut budgets = Vec::new();
        for limiter in &limiters {
            let key = RateLimiterKey::ApiKey {
                key_id: key_id.into(),
                limit_type: limiter.limit_type,
            }
            .to_string();
            let start = now - limiter.time_window();
            let state = self.states.get(&key).map(|v| v.value().clone());
            let recorded = match state {
                Some(state) => state.lock().current_usage(start),
                None => 0.0,
            };
            // Admitted requests that have not finished are ordinary usage
            // too: a batch must not reserve the room they were promised.
            let ordinary_usage = recorded
                + batch_metric(limiter.limit_type)
                    .amount(&in_flight)
                    .unwrap_or(0.0);
            budgets.push(lr_batches::BatchBudget {
                metric: batch_metric(limiter.limit_type),
                window_start: start.timestamp(),
                limit: limiter.value,
                ordinary_usage,
            });
        }
        Ok(budgets)
    }

    /// Load persisted state from disk
    pub async fn load_state(&self) -> AppResult<()> {
        if let Some(path) = &self.persist_path {
            if path.exists() {
                match fs::read_to_string(path).await {
                    Ok(contents) => {
                        match serde_json::from_str::<Vec<(String, RateLimiterState)>>(&contents) {
                            Ok(states) => {
                                for (key_str, mut state) in states {
                                    // Older versions could persist concurrent
                                    // completions out of timestamp order.
                                    state
                                        .events
                                        .make_contiguous()
                                        .sort_by_key(|event| event.timestamp);
                                    self.states
                                        .insert(key_str, Arc::new(parking_lot::Mutex::new(state)));
                                }
                                debug!("Loaded rate limiter state from disk");
                            }
                            Err(e) => {
                                warn!("Failed to parse rate limiter state: {}", e);
                            }
                        }
                    }
                    Err(e) => {
                        warn!("Failed to read rate limiter state file: {}", e);
                    }
                }
            }
        }
        Ok(())
    }

    /// Persist current state to disk
    pub async fn persist_state(&self) -> AppResult<()> {
        if let Some(path) = &self.persist_path {
            // Collect all states
            let mut states_vec: Vec<(String, RateLimiterState)> = Vec::new();

            // Release DashMap shard guards before awaiting a state lock.
            let states: Vec<_> = self
                .states
                .iter()
                .map(|entry| (entry.key().clone(), entry.value().clone()))
                .collect();
            for (key, state) in states {
                let state = state.lock().clone();
                states_vec.push((key, state));
            }

            // Serialize to JSON
            let json = serde_json::to_string_pretty(&states_vec)
                .map_err(|e| AppError::Internal(format!("Failed to serialize state: {}", e)))?;

            // Write to file
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).await?;
            }
            fs::write(path, json).await?;

            debug!("Persisted rate limiter state to disk");
        }
        Ok(())
    }

    /// Add rate limiters for an API key
    pub fn add_api_key_limiters(&self, key_id: String, limiters: Vec<RateLimiter>) {
        self.api_key_limiters.insert(key_id, limiters);
    }

    /// Add rate limiters for a router
    pub fn add_router_limiters(&self, router_name: String, limiters: Vec<RateLimiter>) {
        self.router_limiters.insert(router_name, limiters);
    }

    /// Check if a request is allowed for a specific limiter
    fn check_limiter(
        &self,
        key: &RateLimiterKey,
        limiter: &RateLimiter,
        retention_window: Duration,
    ) -> RateLimitCheckResult {
        let now = Utc::now();
        let window_start = now - limiter.time_window();
        let key_str = key.to_string();

        // Get or create state for this limiter
        let state_lock = self
            .states
            .entry(key_str)
            .or_insert_with(|| Arc::new(parking_lot::Mutex::new(RateLimiterState::new())))
            .value()
            .clone();

        let mut state = state_lock.lock();

        // Clean up old events
        // Several windows for one metric share the same event history. A
        // short-window check must not erase events needed by a longer one.
        state.cleanup(now - retention_window);

        // Calculate current usage
        let current_usage = state.current_usage(window_start);

        // Check if under limit
        let allowed = current_usage < limiter.value;

        // Calculate retry_after if not allowed
        let retry_after_secs = if !allowed {
            // Find the oldest event in the window
            if let Some(oldest_event) = state
                .events
                .iter()
                .find(|event| event.timestamp >= window_start)
            {
                let time_until_oldest_expires =
                    oldest_event.timestamp + limiter.time_window() - now;
                Some((time_until_oldest_expires.num_milliseconds().max(0) + 999) / 1000)
            } else {
                Some(0)
            }
        } else {
            None
        };

        RateLimitCheckResult {
            allowed,
            retry_after_secs,
            current_usage,
            limit: limiter.value,
        }
    }

    /// Check all rate limits for an API key before making a request.
    ///
    /// Counts what admitted requests hold in flight but reserves nothing: use
    /// [`Self::reserve_api_key`] to admit a request.
    pub async fn check_api_key(
        &self,
        key_id: &str,
        _usage_estimate: &UsageInfo,
    ) -> AppResult<RateLimitCheckResult> {
        Ok(match self.admit_api_key(key_id, None).await? {
            Ok(_) => RateLimitCheckResult {
                allowed: true,
                retry_after_secs: None,
                current_usage: 0.0,
                limit: f64::MAX,
            },
            Err(denied) => denied,
        })
    }

    /// Admit a request: check every limit of the API key and, when all have
    /// room, reserve the request's estimate in one step.
    ///
    /// A limit has room while completed usage, in-flight reservations and
    /// native batch reservations together are below it, so of N concurrent
    /// requests against a limit of K requests exactly K are admitted. The
    /// request itself counts immediately; tokens and cost are held at
    /// `estimate` and can only be as exact as it is.
    ///
    /// Record the request's actual usage with
    /// [`Self::record_api_key_usage`] and then drop the reservation. Dropping
    /// it without recording (failure, cancellation) gives the room back.
    pub async fn reserve_api_key(
        &self,
        key_id: &str,
        estimate: &UsageInfo,
    ) -> AppResult<Result<AdmissionReservation, RateLimitCheckResult>> {
        self.admit_api_key(key_id, Some(estimate)).await
    }

    async fn admit_api_key(
        &self,
        key_id: &str,
        reserve: Option<&UsageInfo>,
    ) -> AppResult<Result<AdmissionReservation, RateLimitCheckResult>> {
        // Get limiters for this API key
        let limiters = match self.api_key_limiters.get(key_id) {
            Some(l) => l.clone(),
            None => return Ok(Ok(AdmissionReservation::unreserved())),
        };

        // The durable batch ledger is read off the async runtime, before the
        // synchronous check-and-reserve below.
        let now = Utc::now();
        let mut batch_amounts = Vec::with_capacity(limiters.len());
        for limiter in &limiters {
            let pending = self
                .pending_batch_usage(key_id, (now - limiter.time_window()).timestamp())
                .await?;
            batch_amounts.push(
                batch_metric(limiter.limit_type)
                    .amount(&pending)
                    .ok_or(AppError::RateLimitExceeded)?,
            );
        }

        // A duplicate hop passes through a request whose first hop holds the
        // reservation: it must not be blocked by that reservation, and holds
        // none of its own (it is never charged either).
        let duplicate_hop = lr_types::is_duplicate_hop();
        let scope = self.api_key_scope(key_id);
        let mut held = scope.lock();
        let in_flight = if duplicate_hop {
            ReservedUsage::default()
        } else {
            in_flight_total(&held)
        };

        // Check each limiter
        for (limiter, batch_amount) in limiters.iter().zip(batch_amounts) {
            let key = RateLimiterKey::ApiKey {
                key_id: key_id.to_string(),
                limit_type: limiter.limit_type,
            };

            // Final usage is unknown, but an already exhausted token/cost
            // budget must block the next request just like a request limit.
            let mut result = self.check_limiter(
                &key,
                limiter,
                retention_window(&limiters, limiter.limit_type),
            );
            let in_flight_amount = batch_metric(limiter.limit_type)
                .amount(&in_flight)
                .unwrap_or(0.0);
            result.current_usage += in_flight_amount + batch_amount;
            result.allowed = result.current_usage < limiter.value;
            if !result.allowed {
                if batch_amount > 0.0 {
                    result.retry_after_secs = None;
                } else if result.retry_after_secs.is_none() {
                    // Only in-flight requests stand in the way; room returns
                    // as they finish.
                    result.retry_after_secs = Some(1);
                }
                return Ok(Err(result));
            }
        }

        Ok(Ok(match reserve {
            Some(estimate) if !duplicate_hop => self.hold(
                &scope,
                &mut held,
                ReservedUsage {
                    requests: 1,
                    input_tokens: estimate.input_tokens,
                    output_tokens: estimate.output_tokens,
                    cost_usd: Some(if estimate.cost_usd.is_finite() {
                        estimate.cost_usd.max(0.0)
                    } else {
                        0.0
                    }),
                },
            ),
            _ => AdmissionReservation::unreserved(),
        }))
    }

    /// Check all rate limits for a router before making a request
    pub async fn check_router(
        &self,
        router_name: &str,
        _usage_estimate: &UsageInfo,
    ) -> AppResult<RateLimitCheckResult> {
        // Get limiters for this router
        let limiters = match self.router_limiters.get(router_name) {
            Some(l) => l.clone(),
            None => {
                return Ok(RateLimitCheckResult {
                    allowed: true,
                    retry_after_secs: None,
                    current_usage: 0.0,
                    limit: f64::MAX,
                })
            }
        };

        // Check each limiter
        for limiter in &limiters {
            let key = RateLimiterKey::Router {
                router_name: router_name.to_string(),
                limit_type: limiter.limit_type,
            };

            let result = self.check_limiter(
                &key,
                limiter,
                retention_window(&limiters, limiter.limit_type),
            );
            if !result.allowed {
                return Ok(result);
            }
        }

        Ok(RateLimitCheckResult {
            allowed: true,
            retry_after_secs: None,
            current_usage: 0.0,
            limit: f64::MAX,
        })
    }

    /// Record usage for an API key after a request completes
    pub async fn record_api_key_usage(&self, key_id: &str, usage: &UsageInfo) -> AppResult<()> {
        // A duplicate hop of an already-handled request is a pass-through:
        // the first hop charged the usage.
        if lr_types::is_duplicate_hop() {
            return Ok(());
        }
        self.charge_api_key(key_id, usage);
        Ok(())
    }

    /// Record usage for an API key from a synchronous context (a stream being
    /// dropped). The caller decides whether the request is charged at all:
    /// this does not look at the request trace.
    pub fn charge_api_key(&self, key_id: &str, usage: &UsageInfo) {
        let limiters = match self.api_key_limiters.get(key_id) {
            Some(l) => l.clone(),
            None => return,
        };

        let now = Utc::now();
        let mut recorded = HashSet::new();

        for limiter in &limiters {
            if !recorded.insert(limiter.limit_type) {
                continue;
            }
            let key = RateLimiterKey::ApiKey {
                key_id: key_id.to_string(),
                limit_type: limiter.limit_type,
            };

            let value = match limiter.limit_type {
                RateLimitType::Requests => 1.0,
                RateLimitType::InputTokens => usage.input_tokens as f64,
                RateLimitType::OutputTokens => usage.output_tokens as f64,
                RateLimitType::TotalTokens => usage.total_tokens() as f64,
                RateLimitType::Cost => usage.cost_usd,
            };

            // Get or create state
            let key_str = key.to_string();
            let state_lock = self
                .states
                .entry(key_str)
                .or_insert_with(|| Arc::new(parking_lot::Mutex::new(RateLimiterState::new())))
                .value()
                .clone();

            let mut state = state_lock.lock();
            state.cleanup(now - retention_window(&limiters, limiter.limit_type));
            state.record(now, value);
        }
    }

    /// Record usage for a router after a request completes
    pub async fn record_router_usage(&self, router_name: &str, usage: &UsageInfo) -> AppResult<()> {
        let limiters = match self.router_limiters.get(router_name) {
            Some(l) => l.clone(),
            None => return Ok(()),
        };

        let now = Utc::now();
        let mut recorded = HashSet::new();

        for limiter in &limiters {
            if !recorded.insert(limiter.limit_type) {
                continue;
            }
            let key = RateLimiterKey::Router {
                router_name: router_name.to_string(),
                limit_type: limiter.limit_type,
            };

            let value = match limiter.limit_type {
                RateLimitType::Requests => 1.0,
                RateLimitType::InputTokens => usage.input_tokens as f64,
                RateLimitType::OutputTokens => usage.output_tokens as f64,
                RateLimitType::TotalTokens => usage.total_tokens() as f64,
                RateLimitType::Cost => usage.cost_usd,
            };

            // Get or create state
            let key_str = key.to_string();
            let state_lock = self
                .states
                .entry(key_str)
                .or_insert_with(|| Arc::new(parking_lot::Mutex::new(RateLimiterState::new())))
                .value()
                .clone();

            let mut state = state_lock.lock();
            state.cleanup(now - retention_window(&limiters, limiter.limit_type));
            state.record(now, value);
        }

        Ok(())
    }

    /// Every configured window of one metric, in configuration order.
    fn usage_windows(
        &self,
        key: &RateLimiterKey,
        limiters: &[RateLimiter],
        limit_type: RateLimitType,
        in_flight: &ReservedUsage,
    ) -> Vec<WindowUsage> {
        let now = Utc::now();
        let state_lock = self.states.get(&key.to_string()).map(|s| s.value().clone());
        let reserved = batch_metric(limit_type).amount(in_flight).unwrap_or(0.0);
        let mut state = state_lock.as_ref().map(|state| state.lock());
        if let Some(state) = state.as_mut() {
            state.cleanup(now - retention_window(limiters, limit_type));
        }
        limiters
            .iter()
            .filter(|limiter| limiter.limit_type == limit_type)
            .map(|limiter| {
                let window_start = now - limiter.time_window();
                WindowUsage {
                    current_usage: state
                        .as_ref()
                        .map_or(0.0, |state| state.current_usage(window_start)),
                    reserved,
                    limit: limiter.value,
                    window_secs: limiter.time_window_secs,
                    window_start,
                }
            })
            .collect()
    }

    /// Usage of an API key in every configured window of a metric, in
    /// configuration order. Empty when the key has no limit of that type.
    pub async fn get_api_key_usage_windows(
        &self,
        key_id: &str,
        limit_type: RateLimitType,
    ) -> Vec<WindowUsage> {
        let Some(limiters) = self.api_key_limiters.get(key_id).map(|l| l.clone()) else {
            return Vec::new();
        };
        let key = RateLimiterKey::ApiKey {
            key_id: key_id.to_string(),
            limit_type,
        };
        self.usage_windows(&key, &limiters, limit_type, &self.api_key_in_flight(key_id))
    }

    /// Usage of a router in every configured window of a metric, in
    /// configuration order. Empty when the router has no limit of that type.
    pub async fn get_router_usage_windows(
        &self,
        router_name: &str,
        limit_type: RateLimitType,
    ) -> Vec<WindowUsage> {
        let Some(limiters) = self.router_limiters.get(router_name).map(|l| l.clone()) else {
            return Vec::new();
        };
        let key = RateLimiterKey::Router {
            router_name: router_name.to_string(),
            limit_type,
        };
        self.usage_windows(&key, &limiters, limit_type, &ReservedUsage::default())
    }

    /// Completed usage, limit and window start of the first configured window
    /// of a metric, once usage has been recorded for the API key.
    ///
    /// A metric can have several windows; this reports one. Use
    /// [`Self::get_api_key_usage_windows`] for all of them.
    pub async fn get_api_key_usage(
        &self,
        key_id: &str,
        limit_type: RateLimitType,
    ) -> Option<(f64, f64, DateTime<Utc>)> {
        let key = RateLimiterKey::ApiKey {
            key_id: key_id.to_string(),
            limit_type,
        };
        if !self.states.contains_key(&key.to_string()) {
            return None;
        }
        let window = self
            .get_api_key_usage_windows(key_id, limit_type)
            .await
            .into_iter()
            .next()?;
        Some((window.current_usage, window.limit, window.window_start))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::{sleep, Duration as TokioDuration};

    #[tokio::test]
    async fn exhausted_token_and_cost_budgets_block_subsequent_requests() {
        for (limit_type, value) in [
            (RateLimitType::InputTokens, 100.0),
            (RateLimitType::OutputTokens, 50.0),
            (RateLimitType::TotalTokens, 150.0),
            (RateLimitType::Cost, 0.25),
        ] {
            let manager = RateLimiterManager::new(None);
            let limits = vec![RateLimiter::new(limit_type, value, 60)];
            manager.add_api_key_limiters("client".into(), limits.clone());
            manager.add_router_limiters("router".into(), limits);
            let usage = UsageInfo {
                input_tokens: 100,
                output_tokens: 50,
                cost_usd: 0.25,
            };
            assert!(
                manager
                    .check_api_key("client", &usage)
                    .await
                    .unwrap()
                    .allowed
            );
            assert!(
                manager
                    .check_router("router", &usage)
                    .await
                    .unwrap()
                    .allowed
            );
            manager
                .record_api_key_usage("client", &usage)
                .await
                .unwrap();
            manager.record_router_usage("router", &usage).await.unwrap();
            assert!(
                !manager
                    .check_api_key("client", &usage)
                    .await
                    .unwrap()
                    .allowed
            );
            assert!(
                !manager
                    .check_router("router", &usage)
                    .await
                    .unwrap()
                    .allowed
            );
        }
    }

    #[tokio::test]
    async fn multiple_windows_charge_once_and_preserve_long_window_history() {
        let manager = RateLimiterManager::new(None);
        manager.add_api_key_limiters(
            "client".into(),
            vec![
                RateLimiter::new(RateLimitType::Requests, 10.0, 60),
                RateLimiter::new(RateLimitType::Requests, 2.0, 3600),
            ],
        );
        let usage = UsageInfo {
            input_tokens: 0,
            output_tokens: 0,
            cost_usd: 0.0,
        };
        manager
            .record_api_key_usage("client", &usage)
            .await
            .unwrap();
        let key = RateLimiterKey::ApiKey {
            key_id: "client".into(),
            limit_type: RateLimitType::Requests,
        }
        .to_string();
        let state = manager.states.get(&key).unwrap().value().clone();
        {
            let mut state = state.lock();
            assert_eq!(state.events.len(), 1, "one request must be counted once");
            state.record(Utc::now() - Duration::minutes(30), 1.0);
        }
        // Inspecting the short window must retain the older event too.
        assert_eq!(
            manager
                .get_api_key_usage("client", RateLimitType::Requests)
                .await
                .unwrap()
                .0,
            1.0
        );
        let result = manager.check_api_key("client", &usage).await.unwrap();
        assert!(!result.allowed, "the hourly budget is exhausted");
        assert_eq!(result.current_usage, 2.0);
        assert_eq!(state.lock().events.len(), 2);
    }

    #[test]
    fn out_of_order_events_expire_and_token_totals_do_not_overflow() {
        let now = Utc::now();
        let mut state = RateLimiterState::new();
        state.record(now, 1.0);
        state.record(now - Duration::minutes(2), 2.0);
        state.cleanup(now - Duration::minutes(1));
        assert_eq!(state.events.len(), 1);
        assert_eq!(state.current_usage(now - Duration::minutes(1)), 1.0);
        assert_eq!(
            UsageInfo {
                input_tokens: u64::MAX,
                output_tokens: 1,
                cost_usd: 0.0
            }
            .total_tokens(),
            u64::MAX
        );
    }

    /// Usage recorded while a duplicate-hop trace is in scope is ignored —
    /// the first LocalRouter hop already charged it.
    #[tokio::test]
    async fn test_duplicate_hop_does_not_charge_usage() {
        let manager = Arc::new(RateLimiterManager::new(None));
        manager.add_api_key_limiters(
            "dup-key".to_string(),
            vec![RateLimiter::new(RateLimitType::TotalTokens, 1_000.0, 60)],
        );
        let usage = UsageInfo {
            input_tokens: 100,
            output_tokens: 50,
            cost_usd: 0.01,
        };

        let dup = lr_types::RequestTrace::parse("abc;hop=2").unwrap();
        lr_types::with_outbound_trace(Some(dup), async {
            manager
                .record_api_key_usage("dup-key", &usage)
                .await
                .unwrap();
        })
        .await;
        // Nothing was recorded, so there may be no usage state at all yet.
        let used = manager
            .get_api_key_usage("dup-key", RateLimitType::TotalTokens)
            .await
            .map_or(0.0, |(used, _, _)| used);
        assert_eq!(used, 0.0, "duplicate hop must not be charged");

        manager
            .record_api_key_usage("dup-key", &usage)
            .await
            .unwrap();
        let (used, _, _) = manager
            .get_api_key_usage("dup-key", RateLimitType::TotalTokens)
            .await
            .unwrap();
        assert_eq!(used, 150.0, "normal request is charged");
    }

    #[tokio::test]
    async fn test_request_rate_limiter() {
        let manager = Arc::new(RateLimiterManager::new(None));

        // Add a rate limiter: 5 requests per 10 seconds
        let limiters = vec![RateLimiter::new(RateLimitType::Requests, 5.0, 10)];
        manager.add_api_key_limiters("test-key".to_string(), limiters);

        let usage = UsageInfo {
            input_tokens: 100,
            output_tokens: 50,
            cost_usd: 0.01,
        };

        // First 5 requests should succeed
        for i in 0..5 {
            let result = manager.check_api_key("test-key", &usage).await.unwrap();
            assert!(result.allowed, "Request {} should be allowed", i);

            // Record the request
            manager
                .record_api_key_usage("test-key", &usage)
                .await
                .unwrap();
        }

        // 6th request should be rate limited
        let result = manager.check_api_key("test-key", &usage).await.unwrap();
        assert!(!result.allowed, "Request should be rate limited");
        assert!(result.retry_after_secs.is_some());
    }

    #[tokio::test]
    async fn test_token_rate_limiter() {
        let manager = Arc::new(RateLimiterManager::new(None));

        // Add a rate limiter: 1000 total tokens per 60 seconds
        let limiters = vec![RateLimiter::new(RateLimitType::TotalTokens, 1000.0, 60)];
        manager.add_api_key_limiters("test-key".to_string(), limiters);

        // Record 500 tokens
        let usage1 = UsageInfo {
            input_tokens: 300,
            output_tokens: 200,
            cost_usd: 0.05,
        };
        manager
            .record_api_key_usage("test-key", &usage1)
            .await
            .unwrap();

        // Check usage
        let (current, limit, _) = manager
            .get_api_key_usage("test-key", RateLimitType::TotalTokens)
            .await
            .unwrap();
        assert_eq!(current, 500.0);
        assert_eq!(limit, 1000.0);

        // Record another 600 tokens (should exceed limit)
        let usage2 = UsageInfo {
            input_tokens: 400,
            output_tokens: 200,
            cost_usd: 0.06,
        };
        manager
            .record_api_key_usage("test-key", &usage2)
            .await
            .unwrap();

        // Check that we've exceeded the limit
        let (current, limit, _) = manager
            .get_api_key_usage("test-key", RateLimitType::TotalTokens)
            .await
            .unwrap();
        assert_eq!(current, 1100.0);
        assert!(current > limit);
    }

    #[tokio::test]
    async fn test_cost_rate_limiter() {
        let manager = Arc::new(RateLimiterManager::new(None));

        // Add a rate limiter: $10 per month (2592000 seconds)
        let limiters = vec![RateLimiter::new(RateLimitType::Cost, 10.0, 2592000)];
        manager.add_api_key_limiters("test-key".to_string(), limiters);

        // Record $3.50 in costs
        let usage1 = UsageInfo {
            input_tokens: 10000,
            output_tokens: 5000,
            cost_usd: 3.5,
        };
        manager
            .record_api_key_usage("test-key", &usage1)
            .await
            .unwrap();

        // Check usage
        let (current, limit, _) = manager
            .get_api_key_usage("test-key", RateLimitType::Cost)
            .await
            .unwrap();
        assert_eq!(current, 3.5);
        assert_eq!(limit, 10.0);

        // Record another $7.00 (total $10.50, exceeding limit)
        let usage2 = UsageInfo {
            input_tokens: 20000,
            output_tokens: 10000,
            cost_usd: 7.0,
        };
        manager
            .record_api_key_usage("test-key", &usage2)
            .await
            .unwrap();

        // Check that we've exceeded the limit
        let (current, limit, _) = manager
            .get_api_key_usage("test-key", RateLimitType::Cost)
            .await
            .unwrap();
        assert_eq!(current, 10.5);
        assert!(current > limit);
    }

    #[tokio::test]
    async fn test_router_rate_limiter() {
        let manager = Arc::new(RateLimiterManager::new(None));

        // Add a rate limiter for a router: 10 requests per 30 seconds
        let limiters = vec![RateLimiter::new(RateLimitType::Requests, 10.0, 30)];
        manager.add_router_limiters("test-router".to_string(), limiters);

        let usage = UsageInfo {
            input_tokens: 100,
            output_tokens: 50,
            cost_usd: 0.01,
        };

        // First 10 requests should succeed
        for i in 0..10 {
            let result = manager.check_router("test-router", &usage).await.unwrap();
            assert!(result.allowed, "Request {} should be allowed", i);

            manager
                .record_router_usage("test-router", &usage)
                .await
                .unwrap();
        }

        // 11th request should be rate limited
        let result = manager.check_router("test-router", &usage).await.unwrap();
        assert!(!result.allowed, "Request should be rate limited");
    }

    #[tokio::test]
    async fn test_sliding_window() {
        let manager = Arc::new(RateLimiterManager::new(None));

        // Add a rate limiter: 5 requests per 2 seconds
        let limiters = vec![RateLimiter::new(RateLimitType::Requests, 5.0, 2)];
        manager.add_api_key_limiters("test-key".to_string(), limiters);

        let usage = UsageInfo {
            input_tokens: 100,
            output_tokens: 50,
            cost_usd: 0.01,
        };

        // Make 5 requests
        for i in 0..5 {
            let result = manager.check_api_key("test-key", &usage).await.unwrap();
            assert!(result.allowed, "Request {} should be allowed", i);

            manager
                .record_api_key_usage("test-key", &usage)
                .await
                .unwrap();
        }

        // 6th request should be rate limited
        let result = manager.check_api_key("test-key", &usage).await.unwrap();
        assert!(!result.allowed);

        // Wait for 2 seconds (window to expire)
        sleep(TokioDuration::from_secs(2)).await;

        // Now the request should succeed again
        let result = manager.check_api_key("test-key", &usage).await.unwrap();
        assert!(
            result.allowed,
            "Request should be allowed after window expires"
        );
    }

    #[tokio::test]
    async fn test_persistence() {
        use tempfile::TempDir;

        let temp_dir = TempDir::new().unwrap();
        let persist_path = temp_dir.path().join("rate_limit_state.json");

        // Create manager and add some usage
        {
            let manager = RateLimiterManager::new(Some(persist_path.clone()));
            let limiters = vec![RateLimiter::new(RateLimitType::Requests, 10.0, 60)];
            manager.add_api_key_limiters("test-key".to_string(), limiters);

            let usage = UsageInfo {
                input_tokens: 100,
                output_tokens: 50,
                cost_usd: 0.01,
            };

            // Record 3 requests
            for _ in 0..3 {
                manager
                    .record_api_key_usage("test-key", &usage)
                    .await
                    .unwrap();
            }

            // Persist state
            manager.persist_state().await.unwrap();

            // Verify usage before dropping
            let (current, _, _) = manager
                .get_api_key_usage("test-key", RateLimitType::Requests)
                .await
                .unwrap();
            assert_eq!(current, 3.0);
        }

        // Create new manager and load state
        {
            let manager = RateLimiterManager::new(Some(persist_path.clone()));
            let limiters = vec![RateLimiter::new(RateLimitType::Requests, 10.0, 60)];
            manager.add_api_key_limiters("test-key".to_string(), limiters);

            manager.load_state().await.unwrap();

            // Verify usage was restored
            let (current, _, _) = manager
                .get_api_key_usage("test-key", RateLimitType::Requests)
                .await
                .unwrap();
            assert_eq!(current, 3.0);
        }
    }
    fn batch_job_fixture(store: &lr_batches::BatchStore) -> lr_batches::BatchJob {
        let file = store
            .upload("client", "input.jsonl", "batch", b"{\"custom_id\":\"one\",\"method\":\"POST\",\"url\":\"/v1/chat/completions\",\"body\":{\"model\":\"model\"}}\n{\"custom_id\":\"two\",\"method\":\"POST\",\"url\":\"/v1/chat/completions\",\"body\":{\"model\":\"model\"}}\n", 3600)
            .unwrap();
        let job = store
            .create_job(lr_batches::NewBatchJob {
                options: lr_batches::BatchJobOptions::default(),
                owner: "client".into(),
                input_file_id: file.id,
                provider_instance: "provider".into(),
                credential_binding: "a".repeat(64),
                endpoint: "/v1/chat/completions".into(),
                model: "model".into(),
                request_fingerprint: "b".repeat(64),
                policy_fingerprint: "c".repeat(64),
                idempotency_key_hash: None,
                custom_ids: vec!["one".into(), "two".into()],
            })
            .unwrap()
            .0;
        store
            .prepare_job(
                "client",
                &job.id,
                job.revision,
                &job.input_file_id,
                &job.policy_fingerprint,
            )
            .unwrap()
    }

    #[tokio::test]
    async fn native_batch_reservations_share_regular_quota_windows() {
        let directory = tempfile::tempdir().unwrap();
        let store = Arc::new(lr_batches::BatchStore::new(
            directory.path().join("batches"),
        ));
        let manager = RateLimiterManager::new(None);
        manager.set_batch_store(store.clone());
        manager.add_api_key_limiters(
            "client".into(),
            vec![
                RateLimiter::new(RateLimitType::Requests, 3.0, 60),
                RateLimiter::new(RateLimitType::Requests, 100.0, 3600),
                RateLimiter::new(RateLimitType::Cost, 1.0, 86400),
            ],
        );
        let ordinary = UsageInfo {
            input_tokens: 10,
            output_tokens: 5,
            cost_usd: 0.1,
        };
        manager
            .record_api_key_usage("client", &ordinary)
            .await
            .unwrap();
        let budgets = manager.batch_budget_snapshot("client").await.unwrap();
        assert_eq!(budgets.len(), 3);
        assert_eq!(budgets[0].ordinary_usage, 1.0);
        assert_eq!(budgets[1].ordinary_usage, 1.0);
        let job = batch_job_fixture(&store);
        store
            .reserve_batch(
                "client",
                &job.id,
                lr_batches::BatchUsage {
                    requests: 2,
                    input_tokens: 100,
                    output_tokens: 50,
                    cost_usd: Some(0.02),
                },
                &budgets,
            )
            .unwrap();
        let check = manager.check_api_key("client", &ordinary).await.unwrap();
        assert!(!check.allowed);
        assert_eq!(check.current_usage, 3.0);
        assert_eq!(check.retry_after_secs, None);
        assert!(
            manager
                .check_api_key("other-client", &ordinary)
                .await
                .unwrap()
                .allowed
        );
        store.reject_unsubmitted("client", &job.id).unwrap();
        assert!(
            manager
                .check_api_key("client", &ordinary)
                .await
                .unwrap()
                .allowed
        );
        // Native usage was never copied into the ordinary in-memory ledger.
        assert_eq!(
            manager.batch_budget_snapshot("client").await.unwrap()[0].ordinary_usage,
            1.0
        );
    }

    #[tokio::test]
    async fn batch_unknown_spend_and_corrupt_ledger_fail_closed() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("batches");
        let store = Arc::new(lr_batches::BatchStore::new(root.clone()));
        let manager = RateLimiterManager::new(None);
        manager.set_batch_store(store.clone());
        manager.add_api_key_limiters(
            "client".into(),
            vec![RateLimiter::new(RateLimitType::Cost, 10.0, 60)],
        );
        let job = batch_job_fixture(&store);
        store
            .reserve_batch(
                "client",
                &job.id,
                lr_batches::BatchUsage {
                    requests: 2,
                    input_tokens: 10,
                    output_tokens: 5,
                    cost_usd: None,
                },
                &[],
            )
            .unwrap();
        let usage = UsageInfo {
            input_tokens: 0,
            output_tokens: 0,
            cost_usd: 0.0,
        };
        assert!(matches!(
            manager.check_api_key("client", &usage).await,
            Err(AppError::RateLimitExceeded)
        ));
        std::fs::write(root.join("batches.sqlite3"), b"damaged database").unwrap();
        assert!(matches!(
            manager.check_api_key("client", &usage).await,
            Err(AppError::Storage(_))
        ));
    }

    #[tokio::test]
    async fn regular_quota_checks_do_not_create_batch_payload_storage() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("batches");
        let manager = RateLimiterManager::new(None);
        manager.set_batch_store(Arc::new(lr_batches::BatchStore::new(root.clone())));
        manager.add_api_key_limiters(
            "client".into(),
            vec![RateLimiter::new(RateLimitType::Requests, 10.0, 60)],
        );
        assert!(
            manager
                .check_api_key(
                    "client",
                    &UsageInfo {
                        input_tokens: 0,
                        output_tokens: 0,
                        cost_usd: 0.0
                    }
                )
                .await
                .unwrap()
                .allowed
        );
        assert!(!root.exists());
    }

    fn estimate(input_tokens: u64, output_tokens: u64, cost_usd: f64) -> UsageInfo {
        UsageInfo {
            input_tokens,
            output_tokens,
            cost_usd,
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_reservations_admit_exactly_the_request_limit() {
        let manager = Arc::new(RateLimiterManager::new(None));
        manager.add_api_key_limiters(
            "client".into(),
            vec![
                RateLimiter::new(RateLimitType::Requests, 5.0, 60),
                RateLimiter::new(RateLimitType::Requests, 100.0, 3600),
            ],
        );
        let start = Arc::new(tokio::sync::Barrier::new(40));
        let mut tasks = Vec::new();
        for _ in 0..40 {
            let manager = manager.clone();
            let start = start.clone();
            tasks.push(tokio::spawn(async move {
                start.wait().await;
                manager
                    .reserve_api_key("client", &estimate(0, 0, 0.0))
                    .await
                    .unwrap()
            }));
        }
        let mut admitted = Vec::new();
        let mut denied = 0;
        for task in tasks {
            match task.await.unwrap() {
                Ok(reservation) => admitted.push(reservation),
                Err(check) => {
                    assert!(!check.allowed);
                    assert_eq!(check.limit, 5.0);
                    assert_eq!(check.current_usage, 5.0);
                    // Nothing has completed: room returns as requests finish.
                    assert_eq!(check.retry_after_secs, Some(1));
                    denied += 1;
                }
            }
        }
        assert_eq!(
            admitted.len(),
            5,
            "nothing completed, yet only K are admitted"
        );
        assert_eq!(denied, 35);
        assert_eq!(manager.api_key_in_flight("client").requests, 5);

        // One request finishes: its slot is taken by its recorded usage.
        manager
            .record_api_key_usage("client", &estimate(0, 0, 0.0))
            .await
            .unwrap();
        admitted.pop();
        assert!(manager
            .reserve_api_key("client", &estimate(0, 0, 0.0))
            .await
            .unwrap()
            .is_err());

        // One request fails: its slot is free again, for one request.
        admitted.pop();
        let retry = manager
            .reserve_api_key("client", &estimate(0, 0, 0.0))
            .await
            .unwrap()
            .expect("a released slot is available");
        assert!(manager
            .reserve_api_key("client", &estimate(0, 0, 0.0))
            .await
            .unwrap()
            .is_err());
        drop(retry);
    }

    #[tokio::test]
    async fn reservation_holds_the_estimate_and_is_replaced_by_actual_usage() {
        let manager = RateLimiterManager::new(None);
        manager.add_api_key_limiters(
            "client".into(),
            vec![
                RateLimiter::new(RateLimitType::InputTokens, 1_000.0, 60),
                RateLimiter::new(RateLimitType::OutputTokens, 1_000.0, 60),
                RateLimiter::new(RateLimitType::TotalTokens, 1_500.0, 60),
                RateLimiter::new(RateLimitType::Cost, 1.0, 60),
            ],
        );

        let first = manager
            .reserve_api_key("client", &estimate(400, 600, 0.6))
            .await
            .unwrap()
            .unwrap();
        assert!(first.is_reserved());
        let held = manager.api_key_in_flight("client");
        assert_eq!(
            (held.requests, held.input_tokens, held.output_tokens),
            (1, 400, 600)
        );
        assert_eq!(held.cost_usd, Some(0.6));
        let window = &manager
            .get_api_key_usage_windows("client", RateLimitType::TotalTokens)
            .await[0];
        assert_eq!((window.current_usage, window.reserved), (0.0, 1_000.0));

        // A second request fits every budget that still has room.
        let second = manager
            .reserve_api_key("client", &estimate(400, 600, 0.6))
            .await
            .unwrap()
            .unwrap();
        // Output tokens (1200 of 1000), total tokens (2000 of 1500) and cost
        // (1.2 of 1.0) are now spoken for, with nothing completed.
        let denied = manager
            .reserve_api_key("client", &estimate(1, 1, 0.0))
            .await
            .unwrap()
            .unwrap_err();
        assert_eq!(denied.limit, 1_000.0, "the output token budget");
        assert_eq!(denied.current_usage, 1_200.0);
        assert!(
            !manager
                .check_api_key("client", &estimate(0, 0, 0.0))
                .await
                .unwrap()
                .allowed,
            "the non-reserving check counts in-flight requests too"
        );

        // Both finish well under their estimates.
        for reservation in [first, second] {
            manager
                .record_api_key_usage("client", &estimate(50, 20, 0.01))
                .await
                .unwrap();
            drop(reservation);
        }
        let held = manager.api_key_in_flight("client");
        assert_eq!(
            (held.requests, held.input_tokens, held.output_tokens),
            (0, 0, 0)
        );
        let window = &manager
            .get_api_key_usage_windows("client", RateLimitType::OutputTokens)
            .await[0];
        assert_eq!((window.current_usage, window.reserved), (40.0, 0.0));
        assert!(manager
            .reserve_api_key("client", &estimate(400, 600, 0.6))
            .await
            .unwrap()
            .is_ok());
    }

    #[tokio::test]
    async fn checking_reserves_nothing_and_unlimited_keys_hold_nothing() {
        let manager = RateLimiterManager::new(None);
        manager.add_api_key_limiters(
            "client".into(),
            vec![RateLimiter::new(RateLimitType::Requests, 1.0, 60)],
        );
        for _ in 0..3 {
            assert!(
                manager
                    .check_api_key("client", &estimate(10, 10, 0.0))
                    .await
                    .unwrap()
                    .allowed
            );
        }
        assert_eq!(manager.api_key_in_flight("client").requests, 0);

        let unlimited = manager
            .reserve_api_key("no-limits", &estimate(10, 10, 0.0))
            .await
            .unwrap()
            .unwrap();
        assert!(!unlimited.is_reserved());
        assert_eq!(manager.api_key_in_flight("no-limits").requests, 0);
    }

    #[tokio::test]
    async fn duplicate_hop_is_neither_blocked_by_nor_added_to_in_flight_requests() {
        let manager = RateLimiterManager::new(None);
        manager.add_api_key_limiters(
            "client".into(),
            vec![RateLimiter::new(RateLimitType::Requests, 1.0, 60)],
        );
        // The first hop holds the only slot while it forwards the request.
        let first_hop = manager
            .reserve_api_key("client", &estimate(0, 0, 0.0))
            .await
            .unwrap()
            .unwrap();
        let one_request = || ReservedUsage {
            requests: 1,
            ..ReservedUsage::default()
        };
        let first_hop_strategy = manager
            .reserve_strategy("strategy", one_request(), |_| Ok::<(), ()>(()))
            .unwrap();

        let duplicate = lr_types::RequestTrace::parse("abc;hop=2").unwrap();
        lr_types::with_outbound_trace(Some(duplicate), async {
            let second_hop = manager
                .reserve_api_key("client", &estimate(0, 0, 0.0))
                .await
                .unwrap()
                .expect("the pass-through hop of an admitted request is let through");
            assert!(!second_hop.is_reserved());
            // A strategy limit of one request: the first hop's share must
            // not count against its own pass-through hop.
            let strategy = manager
                .reserve_strategy("strategy", one_request(), |in_flight| {
                    if in_flight.requests >= 1 {
                        Err(())
                    } else {
                        Ok(())
                    }
                })
                .expect("the pass-through hop is not blocked by its first hop");
            assert!(!strategy.is_reserved());
            assert_eq!(manager.strategy_in_flight("strategy").requests, 0);
        })
        .await;

        assert_eq!(manager.api_key_in_flight("client").requests, 1);
        assert_eq!(manager.strategy_in_flight("strategy").requests, 1);
        drop(first_hop);
        drop(first_hop_strategy);
        assert_eq!(manager.api_key_in_flight("client").requests, 0);
        assert_eq!(manager.strategy_in_flight("strategy").requests, 0);
    }

    #[tokio::test]
    async fn every_configured_window_is_reported() {
        let manager = RateLimiterManager::new(None);
        manager.add_api_key_limiters(
            "client".into(),
            vec![
                RateLimiter::new(RateLimitType::Requests, 10.0, 60),
                RateLimiter::new(RateLimitType::Cost, 5.0, 86_400),
                RateLimiter::new(RateLimitType::Requests, 100.0, 3_600),
            ],
        );
        manager.add_router_limiters(
            "router".into(),
            vec![
                RateLimiter::new(RateLimitType::Requests, 10.0, 60),
                RateLimiter::new(RateLimitType::Requests, 100.0, 3_600),
            ],
        );
        // Before any usage: every window, empty.
        let windows = manager
            .get_api_key_usage_windows("client", RateLimitType::Requests)
            .await;
        assert_eq!(windows.len(), 2);
        assert!(windows.iter().all(|window| window.current_usage == 0.0));
        assert!(manager
            .get_api_key_usage("client", RateLimitType::Requests)
            .await
            .is_none());

        let usage = estimate(0, 0, 0.5);
        manager
            .record_api_key_usage("client", &usage)
            .await
            .unwrap();
        manager.record_router_usage("router", &usage).await.unwrap();
        for scope in ["apikey:client:Requests", "router:router:Requests"] {
            let state = manager.states.get(scope).unwrap().value().clone();
            state.lock().record(Utc::now() - Duration::minutes(30), 1.0);
        }
        let _in_flight = manager
            .reserve_api_key("client", &usage)
            .await
            .unwrap()
            .unwrap();

        let windows = manager
            .get_api_key_usage_windows("client", RateLimitType::Requests)
            .await;
        assert_eq!(
            windows
                .iter()
                .map(|window| (
                    window.window_secs,
                    window.limit,
                    window.current_usage,
                    window.reserved
                ))
                .collect::<Vec<_>>(),
            [(60, 10.0, 1.0, 1.0), (3_600, 100.0, 2.0, 1.0)],
            "the hourly window sees the older request the minute window does not"
        );
        assert!(windows[0].window_start > windows[1].window_start);
        let router = manager
            .get_router_usage_windows("router", RateLimitType::Requests)
            .await;
        assert_eq!(
            router
                .iter()
                .map(|window| (window.window_secs, window.current_usage))
                .collect::<Vec<_>>(),
            [(60, 1.0), (3_600, 2.0)]
        );
        // The single-window accessors report the first configured window.
        let (used, limit, _) = manager
            .get_api_key_usage("client", RateLimitType::Requests)
            .await
            .unwrap();
        assert_eq!((used, limit), (1.0, 10.0));
        assert!(manager
            .get_api_key_usage_windows("client", RateLimitType::InputTokens)
            .await
            .is_empty());
        assert!(manager
            .get_api_key_usage_windows("unknown", RateLimitType::Requests)
            .await
            .is_empty());
    }

    #[tokio::test]
    async fn strategy_reservation_checks_and_holds_in_one_step() {
        let manager = RateLimiterManager::new(None);
        let one_request = || ReservedUsage {
            requests: 1,
            input_tokens: 250,
            output_tokens: 0,
            cost_usd: Some(0.25),
        };
        // A limit of two requests, with nothing completed.
        let admit = |in_flight: &ReservedUsage| {
            if in_flight.requests + 1 > 2 {
                Err(in_flight.clone())
            } else {
                Ok(())
            }
        };

        let first = manager
            .reserve_strategy("strategy", one_request(), admit)
            .unwrap();
        let second = manager
            .reserve_strategy("strategy", one_request(), admit)
            .unwrap();
        let seen = manager
            .reserve_strategy("strategy", one_request(), admit)
            .unwrap_err();
        assert_eq!((seen.requests, seen.input_tokens), (2, 500));
        assert_eq!(seen.cost_usd, Some(0.5));
        assert_eq!(
            manager.strategy_in_flight("strategy").requests,
            2,
            "a rejected request holds nothing"
        );
        assert_eq!(manager.strategy_in_flight("other").requests, 0);

        // One admission can carry another; both are released together.
        let mut both = first;
        both.absorb(second);
        drop(both);
        assert_eq!(manager.strategy_in_flight("strategy").requests, 0);
        manager
            .reserve_strategy("strategy", one_request(), admit)
            .map(drop)
            .unwrap();
    }

    #[tokio::test]
    async fn in_flight_requests_and_native_batches_share_one_budget() {
        let directory = tempfile::tempdir().unwrap();
        let store = Arc::new(lr_batches::BatchStore::new(
            directory.path().join("batches"),
        ));
        let manager = RateLimiterManager::new(None);
        manager.set_batch_store(store.clone());
        manager.add_api_key_limiters(
            "client".into(),
            vec![RateLimiter::new(RateLimitType::Requests, 3.0, 60)],
        );

        // Two ordinary requests are in flight; a two-row batch does not fit
        // beside them, although nothing has been recorded.
        let first = manager
            .reserve_api_key("client", &estimate(0, 0, 0.0))
            .await
            .unwrap()
            .unwrap();
        let second = manager
            .reserve_api_key("client", &estimate(0, 0, 0.0))
            .await
            .unwrap()
            .unwrap();
        let budgets = manager.batch_budget_snapshot("client").await.unwrap();
        assert_eq!(budgets[0].ordinary_usage, 2.0);
        let job = batch_job_fixture(&store);
        let batch = lr_batches::BatchUsage {
            requests: 2,
            input_tokens: 0,
            output_tokens: 0,
            cost_usd: Some(0.0),
        };
        assert!(matches!(
            store.reserve_batch("client", &job.id, batch.clone(), &budgets),
            Err(lr_batches::StoreError::Quota)
        ));

        // One of them fails: the batch fits, and then leaves no room for a
        // second ordinary request.
        drop(second);
        let budgets = manager.batch_budget_snapshot("client").await.unwrap();
        assert_eq!(budgets[0].ordinary_usage, 1.0);
        store
            .reserve_batch("client", &job.id, batch, &budgets)
            .unwrap();
        let denied = manager
            .reserve_api_key("client", &estimate(0, 0, 0.0))
            .await
            .unwrap()
            .unwrap_err();
        assert_eq!(denied.current_usage, 3.0);
        assert_eq!(denied.retry_after_secs, None, "a batch has no known end");
        drop(first);
    }
}
