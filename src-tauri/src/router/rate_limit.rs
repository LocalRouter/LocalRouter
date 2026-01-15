//! Rate limiting system with sliding window algorithm
//!
//! Supports multiple rate limit types:
//! - Request rate limits (requests per time window)
//! - Token rate limits (input/output/total tokens per time window)
//! - Cost rate limits (USD per time window)
//!
//! Supports per-API-key and per-router rate limiting.

use chrono::{DateTime, Duration, Utc};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::fs;
use tokio::sync::RwLock;
use tokio::time::{interval, Duration as TokioDuration};
use tracing::{debug, error, warn};

use crate::utils::errors::{AppError, AppResult};

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
        self.events.push_back(UsageEvent { timestamp, value });
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
        self.input_tokens + self.output_tokens
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
}

impl RateLimiterManager {
    /// Create a new rate limiter manager
    pub fn new(persist_path: Option<PathBuf>) -> Self {
        Self {
            api_key_limiters: Arc::new(DashMap::new()),
            router_limiters: Arc::new(DashMap::new()),
            states: Arc::new(DashMap::new()),
            persist_path,
        }
    }

    /// Load persisted state from disk
    pub async fn load_state(&self) -> AppResult<()> {
        if let Some(path) = &self.persist_path {
            if path.exists() {
                match fs::read_to_string(path).await {
                    Ok(contents) => {
                        match serde_json::from_str::<Vec<(String, RateLimiterState)>>(&contents) {
                            Ok(states) => {
                                for (key_str, state) in states {
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

            for entry in self.states.iter() {
                let key = entry.key().clone();
                let state = entry.value().read().await.clone();
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
        state.cleanup(window_start);

        // Calculate current usage
        let current_usage = state.current_usage(window_start);

        // Check if under limit
        let allowed = current_usage < limiter.value;

        // Calculate retry_after if not allowed
        let retry_after_secs = if !allowed {
            // Find the oldest event in the window
            if let Some(oldest_event) = state.events.front() {
                let time_until_oldest_expires =
                    oldest_event.timestamp + limiter.time_window() - now;
                Some(time_until_oldest_expires.num_seconds().max(0))
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

            // We can't check token/cost limits before the request
            // So we skip those here - they'll be checked after recording
            match limiter.limit_type {
                RateLimitType::Requests => {
                    let result = self.check_limiter(&key, limiter).await;
                    if !result.allowed {
                        return Ok(result);
                    }
                }
                _ => {
                    // Token and cost limits are checked after the request
                }
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

            // We can only check request limits before the request
            match limiter.limit_type {
                RateLimitType::Requests => {
                    let result = self.check_limiter(&key, limiter).await;
                    if !result.allowed {
                        return Ok(result);
                    }
                }
                _ => {
                    // Token and cost limits are checked after the request
                }
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
        let limiters = match self.api_key_limiters.get(key_id) {
            Some(l) => l.clone(),
            None => return Ok(()),
        };

        let now = Utc::now();

        for limiter in &limiters {
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

        for limiter in &limiters {
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
        let limiters = self.api_key_limiters.get(key_id)?;
        let limiter = limiters.iter().find(|l| l.limit_type == limit_type)?;

        let key = RateLimiterKey::ApiKey {
            key_id: key_id.to_string(),
            limit_type,
        };

        let now = Utc::now();
        let window_start = now - limiter.time_window();
        let key_str = key.to_string();

        let state_lock = self.states.get(&key_str)?;
        let mut state = state_lock.write().await;

        state.cleanup(window_start);
        let current_usage = state.current_usage(window_start);

        Some((current_usage, limiter.value, window_start))
    }

    /// Get current usage statistics for a router
    pub async fn get_router_usage(
        &self,
        router_name: &str,
        limit_type: RateLimitType,
    ) -> Option<(f64, f64, DateTime<Utc>)> {
        let limiters = self.router_limiters.get(router_name)?;
        let limiter = limiters.iter().find(|l| l.limit_type == limit_type)?;

        let key = RateLimiterKey::Router {
            router_name: router_name.to_string(),
            limit_type,
        };

        let now = Utc::now();
        let window_start = now - limiter.time_window();
        let key_str = key.to_string();

        let state_lock = self.states.get(&key_str)?;
        let mut state = state_lock.write().await;

        state.cleanup(window_start);
        let current_usage = state.current_usage(window_start);

        Some((current_usage, limiter.value, window_start))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::sleep;

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
}
