//! Rate limiting system with sliding window algorithm
//!
//! Supports multiple rate limit types:
//! - Request rate limits (requests per time window)
//! - Token rate limits (input/output/total tokens per time window)
//! - Cost rate limits (USD per time window)
//!
//! Supports per-API-key and per-router rate limiting.

#![allow(dead_code)]

use chrono::{DateTime, Duration, Utc};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::collections::{HashSet, VecDeque};
use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::fs;
use tokio::sync::RwLock;
use tokio::time::{interval, Duration as TokioDuration};
use tracing::{debug, error, warn};

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

impl RateLimiterKey {
    #[allow(dead_code)]
    fn from_string(s: &str) -> Option<Self> {
        let parts: Vec<&str> = s.split(':').collect();
        if parts.len() != 3 {
            return None;
        }

        let limit_type = match parts[2] {
            "Requests" => RateLimitType::Requests,
            "InputTokens" => RateLimitType::InputTokens,
            "OutputTokens" => RateLimitType::OutputTokens,
            "TotalTokens" => RateLimitType::TotalTokens,
            "Cost" => RateLimitType::Cost,
            _ => return None,
        };

        match parts[0] {
            "apikey" => Some(RateLimiterKey::ApiKey {
                key_id: parts[1].to_string(),
                limit_type,
            }),
            "router" => Some(RateLimiterKey::Router {
                router_name: parts[1].to_string(),
                limit_type,
            }),
            _ => None,
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
    states: Arc<DashMap<String, Arc<RwLock<RateLimiterState>>>>,
    /// Path for persisting state
    persist_path: Option<PathBuf>,
    batch_store: parking_lot::RwLock<Option<Arc<lr_batches::BatchStore>>>,
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
        }
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
        let mut budgets = Vec::new();
        for limiter in &limiters {
            let key = RateLimiterKey::ApiKey {
                key_id: key_id.into(),
                limit_type: limiter.limit_type,
            }
            .to_string();
            let start = now - limiter.time_window();
            let state = self.states.get(&key).map(|v| v.value().clone());
            let ordinary_usage = match state {
                Some(state) => state.read().await.current_usage(start),
                None => 0.0,
            };
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
                                    self.states.insert(key_str, Arc::new(RwLock::new(state)));
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
                let state = state.read().await.clone();
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

    /// Start background task for periodic persistence
    pub fn start_persistence_task(self: Arc<Self>, interval_secs: u64) {
        tokio::spawn(async move {
            let mut ticker = interval(TokioDuration::from_secs(interval_secs));
            loop {
                ticker.tick().await;
                if let Err(e) = self.persist_state().await {
                    error!("Failed to persist rate limiter state: {}", e);
                }
            }
        });
    }

    /// Add rate limiters for an API key
    pub fn add_api_key_limiters(&self, key_id: String, limiters: Vec<RateLimiter>) {
        self.api_key_limiters.insert(key_id, limiters);
    }

    /// Add rate limiters for a router
    pub fn add_router_limiters(&self, router_name: String, limiters: Vec<RateLimiter>) {
        self.router_limiters.insert(router_name, limiters);
    }

    /// Remove rate limiters for an API key
    pub fn remove_api_key_limiters(&self, key_id: &str) {
        self.api_key_limiters.remove(key_id);
    }

    /// Remove rate limiters for a router
    pub fn remove_router_limiters(&self, router_name: &str) {
        self.router_limiters.remove(router_name);
    }

    /// Check if a request is allowed for a specific limiter
    async fn check_limiter(
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
            .or_insert_with(|| Arc::new(RwLock::new(RateLimiterState::new())))
            .value()
            .clone();

        let mut state = state_lock.write().await;

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

    /// Check all rate limits for an API key before making a request
    pub async fn check_api_key(
        &self,
        key_id: &str,
        _usage_estimate: &UsageInfo,
    ) -> AppResult<RateLimitCheckResult> {
        // Get limiters for this API key
        let limiters = match self.api_key_limiters.get(key_id) {
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
            let key = RateLimiterKey::ApiKey {
                key_id: key_id.to_string(),
                limit_type: limiter.limit_type,
            };

            // Final usage is unknown, but an already exhausted token/cost
            // budget must block the next request just like a request limit.
            let mut result = self
                .check_limiter(
                    &key,
                    limiter,
                    retention_window(&limiters, limiter.limit_type),
                )
                .await;
            let pending = self
                .pending_batch_usage(key_id, (Utc::now() - limiter.time_window()).timestamp())
                .await?;
            let batch_amount = batch_metric(limiter.limit_type)
                .amount(&pending)
                .ok_or(AppError::RateLimitExceeded)?;
            result.current_usage += batch_amount;
            result.allowed = result.current_usage < limiter.value;
            if !result.allowed {
                if batch_amount > 0.0 {
                    result.retry_after_secs = None;
                }
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

            let result = self
                .check_limiter(
                    &key,
                    limiter,
                    retention_window(&limiters, limiter.limit_type),
                )
                .await;
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
        let limiters = match self.api_key_limiters.get(key_id) {
            Some(l) => l.clone(),
            None => return Ok(()),
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
                .or_insert_with(|| Arc::new(RwLock::new(RateLimiterState::new())))
                .value()
                .clone();

            let mut state = state_lock.write().await;
            state.cleanup(now - retention_window(&limiters, limiter.limit_type));
            state.record(now, value);
        }

        Ok(())
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
                .or_insert_with(|| Arc::new(RwLock::new(RateLimiterState::new())))
                .value()
                .clone();

            let mut state = state_lock.write().await;
            state.cleanup(now - retention_window(&limiters, limiter.limit_type));
            state.record(now, value);
        }

        Ok(())
    }

    /// Get current usage statistics for an API key
    pub async fn get_api_key_usage(
        &self,
        key_id: &str,
        limit_type: RateLimitType,
    ) -> Option<(f64, f64, DateTime<Utc>)> {
        let limiters = self.api_key_limiters.get(key_id)?.clone();
        let limiter = limiters.iter().find(|l| l.limit_type == limit_type)?;

        let key = RateLimiterKey::ApiKey {
            key_id: key_id.to_string(),
            limit_type,
        };

        let now = Utc::now();
        let window_start = now - limiter.time_window();
        let key_str = key.to_string();

        let state_lock = self.states.get(&key_str)?.value().clone();
        let mut state = state_lock.write().await;

        state.cleanup(now - retention_window(&limiters, limit_type));
        let current_usage = state.current_usage(window_start);

        Some((current_usage, limiter.value, window_start))
    }

    /// Get current usage statistics for a router
    pub async fn get_router_usage(
        &self,
        router_name: &str,
        limit_type: RateLimitType,
    ) -> Option<(f64, f64, DateTime<Utc>)> {
        let limiters = self.router_limiters.get(router_name)?.clone();
        let limiter = limiters.iter().find(|l| l.limit_type == limit_type)?;

        let key = RateLimiterKey::Router {
            router_name: router_name.to_string(),
            limit_type,
        };

        let now = Utc::now();
        let window_start = now - limiter.time_window();
        let key_str = key.to_string();

        let state_lock = self.states.get(&key_str)?.value().clone();
        let mut state = state_lock.write().await;

        state.cleanup(now - retention_window(&limiters, limit_type));
        let current_usage = state.current_usage(window_start);

        Some((current_usage, limiter.value, window_start))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::sleep;

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
            let mut state = state.write().await;
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
        assert_eq!(state.read().await.events.len(), 2);
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
}
