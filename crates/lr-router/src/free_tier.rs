//! Free tier tracking and management
//!
//! Handles rate limit tracking, credit tracking, and provider backoff
//! for free tier mode. Shared across all clients.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use chrono::{DateTime, Datelike, Utc};
use dashmap::DashMap;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

use lr_config::FreeTierKind;

// ============================================================
// Universal Rate Limit Header Parser
// ============================================================

/// Parsed rate limit information from response headers.
/// Tries all known header naming conventions.
#[derive(Debug, Default, Clone)]
pub struct RateLimitHeaderInfo {
    /// Requests remaining in current window (per-minute typically)
    pub requests_remaining: Option<u64>,
    /// Request limit for current window
    pub requests_limit: Option<u64>,
    /// Seconds until request limit resets
    pub requests_reset_secs: Option<u64>,

    /// Tokens remaining in current window
    pub tokens_remaining: Option<u64>,
    /// Token limit for current window
    pub tokens_limit: Option<u64>,
    /// Seconds until token limit resets
    pub tokens_reset_secs: Option<u64>,

    /// Daily requests remaining (Cerebras-style)
    pub daily_requests_remaining: Option<u64>,
    /// Daily request limit
    pub daily_requests_limit: Option<u64>,

    /// Daily tokens remaining (Cerebras-style)
    pub daily_tokens_remaining: Option<u64>,
    /// Daily token limit
    pub daily_tokens_limit: Option<u64>,

    /// Retry-after seconds (from retry-after header)
    pub retry_after_secs: Option<u64>,
}

fn try_parse_header(headers: &HashMap<String, String>, name: &str) -> Option<u64> {
    headers.get(name).and_then(|s| s.parse::<u64>().ok())
}

fn try_parse_duration_header(headers: &HashMap<String, String>, name: &str) -> Option<u64> {
    let s = headers.get(name)?;
    // Try parsing as integer seconds first
    if let Ok(secs) = s.parse::<u64>() {
        return Some(secs);
    }
    // Try parsing duration strings like "1s", "500ms", "2m"
    if let Some(stripped) = s.strip_suffix("ms") {
        return stripped.parse::<u64>().ok().map(|ms: u64| ms / 1000);
    }
    if let Some(stripped) = s.strip_suffix('s') {
        return stripped.parse::<f64>().ok().map(|v: f64| v.ceil() as u64);
    }
    if let Some(stripped) = s.strip_suffix('m') {
        return stripped.parse::<u64>().ok().map(|m: u64| m * 60);
    }
    None
}

/// Parse rate limit info from ANY provider's response headers.
/// Tries all known header naming conventions.
#[allow(clippy::field_reassign_with_default)]
pub fn parse_rate_limit_headers(headers: &HashMap<String, String>) -> RateLimitHeaderInfo {
    let mut info = RateLimitHeaderInfo::default();

    // Standard format: x-ratelimit-remaining-requests (OpenAI, Groq, xAI)
    info.requests_remaining = try_parse_header(headers, "x-ratelimit-remaining-requests");
    info.requests_limit = try_parse_header(headers, "x-ratelimit-limit-requests");
    info.requests_reset_secs = try_parse_duration_header(headers, "x-ratelimit-reset-requests");
    info.tokens_remaining = try_parse_header(headers, "x-ratelimit-remaining-tokens");
    info.tokens_limit = try_parse_header(headers, "x-ratelimit-limit-tokens");
    info.tokens_reset_secs = try_parse_duration_header(headers, "x-ratelimit-reset-tokens");

    // Daily variant: x-ratelimit-remaining-requests-day (Cerebras)
    info.daily_requests_remaining = try_parse_header(headers, "x-ratelimit-remaining-requests-day");
    info.daily_requests_limit = try_parse_header(headers, "x-ratelimit-limit-requests-day");
    // Cerebras minute-level tokens
    if info.tokens_remaining.is_none() {
        info.tokens_remaining = try_parse_header(headers, "x-ratelimit-remaining-tokens-minute");
        info.tokens_limit = try_parse_header(headers, "x-ratelimit-limit-tokens-minute");
    }

    // Short form: x-ratelimit-remaining (Together AI)
    if info.requests_remaining.is_none() {
        info.requests_remaining = try_parse_header(headers, "x-ratelimit-remaining");
        info.requests_limit = try_parse_header(headers, "x-ratelimit-limit");
        info.requests_reset_secs = try_parse_duration_header(headers, "x-ratelimit-reset");
    }

    // Token-specific short form: x-tokenlimit-remaining (Together AI)
    if info.tokens_remaining.is_none() {
        info.tokens_remaining = try_parse_header(headers, "x-tokenlimit-remaining");
        info.tokens_limit = try_parse_header(headers, "x-tokenlimit-limit");
    }

    // Anthropic format: anthropic-ratelimit-requests-remaining
    if info.requests_remaining.is_none() {
        info.requests_remaining =
            try_parse_header(headers, "anthropic-ratelimit-requests-remaining");
        info.requests_limit = try_parse_header(headers, "anthropic-ratelimit-requests-limit");
    }
    if info.tokens_remaining.is_none() {
        info.tokens_remaining = try_parse_header(headers, "anthropic-ratelimit-tokens-remaining");
        info.tokens_limit = try_parse_header(headers, "anthropic-ratelimit-tokens-limit");
    }

    // Universal: retry-after (seconds or HTTP-date)
    info.retry_after_secs = try_parse_header(headers, "retry-after")
        .or_else(|| try_parse_header(headers, "retry-after-ms").map(|ms| ms / 1000));

    info
}

// ============================================================
// Free Tier Status Types
// ============================================================

/// Classification of whether a model/provider is free
#[derive(Debug, Clone, PartialEq)]
pub enum ModelFreeStatus {
    /// Always free: local provider, subscription, or $0 pricing
    AlwaysFree,
    /// Free within provider's rate limits or credit budget
    FreeWithinLimits,
    /// Free model specifically (FreeModelsOnly pattern match)
    FreeModel,
    /// Not free: no free tier or exhausted
    NotFree,
}

/// Capacity information for a provider's free tier
#[derive(Debug, Clone)]
pub struct FreeTierCapacity {
    /// Whether the provider has remaining capacity
    pub has_capacity: bool,
    /// For rate-limited: % of limits remaining (0.0 - 1.0)
    pub remaining_pct: Option<f32>,
    /// For credit-based: USD remaining
    pub remaining_usd: Option<f64>,
    /// Human-readable status
    pub status_message: String,
}

/// Backoff state for a provider after 429/402 errors
#[derive(Debug, Clone)]
pub struct ProviderBackoff {
    /// When the provider becomes available again
    pub available_at: Option<Instant>,
    /// Current backoff duration for exponential backoff
    pub current_backoff: Duration,
    /// Number of consecutive 429/402 errors
    pub consecutive_errors: u32,
    /// Whether the provider is in credit-exhausted state
    pub is_credit_exhausted: bool,
    /// Source of the backoff timing
    pub backoff_source: BackoffSource,
}

impl Default for ProviderBackoff {
    fn default() -> Self {
        Self {
            available_at: None,
            current_backoff: Duration::from_secs(1),
            consecutive_errors: 0,
            is_credit_exhausted: false,
            backoff_source: BackoffSource::ExponentialBackoff,
        }
    }
}

/// How the backoff timing was determined
#[derive(Debug, Clone, PartialEq)]
pub enum BackoffSource {
    /// Provider told us when to retry via header
    RetryAfterHeader,
    /// Provider rate limit headers show 0 remaining with reset time
    RateLimitResetHeader,
    /// No provider info - using exponential backoff
    ExponentialBackoff,
    /// Credit exhaustion - backed off until replenishment time
    CreditReplenishment,
}

/// Info about a provider's backoff state
#[derive(Debug, Clone)]
pub struct BackoffInfo {
    /// When the provider becomes available
    pub available_at: Instant,
    /// Seconds until available
    pub retry_after_secs: u64,
    /// Why it's backed off
    pub reason: String,
}

// ============================================================
// Tracking State (persisted)
// ============================================================

/// Tracks rate-limited free tier usage per provider
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateLimitTracker {
    /// Requests in current minute window
    pub minute_requests: u32,
    pub minute_window_start: DateTime<Utc>,
    /// Requests today
    pub daily_requests: u32,
    pub daily_window_start: DateTime<Utc>,
    /// Tokens in current minute window
    pub minute_tokens: u64,
    /// Tokens today
    pub daily_tokens: u64,
    /// Monthly requests (for Cohere)
    pub monthly_requests: u32,
    pub monthly_window_start: DateTime<Utc>,
    /// Monthly tokens (for Mistral)
    pub monthly_tokens: u64,
    /// Last known remaining from provider headers (most accurate)
    pub header_requests_remaining: Option<u64>,
    pub header_tokens_remaining: Option<u64>,
    pub header_daily_requests_remaining: Option<u64>,
    #[serde(skip)]
    pub header_updated_at: Option<Instant>,
}

impl Default for RateLimitTracker {
    fn default() -> Self {
        let now = Utc::now();
        Self {
            minute_requests: 0,
            minute_window_start: now,
            daily_requests: 0,
            daily_window_start: now,
            minute_tokens: 0,
            daily_tokens: 0,
            monthly_requests: 0,
            monthly_window_start: now,
            monthly_tokens: 0,
            header_requests_remaining: None,
            header_tokens_remaining: None,
            header_daily_requests_remaining: None,
            header_updated_at: None,
        }
    }
}

impl RateLimitTracker {
    /// Reset windows that have expired
    fn reset_expired_windows(&mut self) {
        let now = Utc::now();

        // Reset minute window (60 seconds)
        if (now - self.minute_window_start).num_seconds() >= 60 {
            self.minute_requests = 0;
            self.minute_tokens = 0;
            self.minute_window_start = now;
            self.header_requests_remaining = None;
            self.header_tokens_remaining = None;
        }

        // Reset daily window (next day)
        if now.ordinal() != self.daily_window_start.ordinal()
            || now.year() != self.daily_window_start.year()
        {
            self.daily_requests = 0;
            self.daily_tokens = 0;
            self.daily_window_start = now;
            self.header_daily_requests_remaining = None;
        }

        // Reset monthly window (next month)
        if now.month() != self.monthly_window_start.month()
            || now.year() != self.monthly_window_start.year()
        {
            self.monthly_requests = 0;
            self.monthly_tokens = 0;
            self.monthly_window_start = now;
        }
    }
}

/// Tracks credit-based free tier usage per provider
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreditTracker {
    /// Estimated cost so far in current period
    pub current_cost_usd: f64,
    /// When this period started
    pub period_start: DateTime<Utc>,
    /// Last known balance from provider API
    pub api_remaining_usd: Option<f64>,
    /// Whether the provider API says we're on free tier
    pub api_is_free_tier: Option<bool>,
    #[serde(skip)]
    pub api_last_checked: Option<Instant>,
}

impl Default for CreditTracker {
    fn default() -> Self {
        Self {
            current_cost_usd: 0.0,
            period_start: Utc::now(),
            api_remaining_usd: None,
            api_is_free_tier: None,
            api_last_checked: None,
        }
    }
}

/// Full persisted state for the FreeTierManager
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FreeTierState {
    pub rate_trackers: Vec<(String, RateLimitTracker)>,
    pub credit_trackers: Vec<(String, CreditTracker)>,
}

// ============================================================
// FreeTierManager
// ============================================================

/// Central manager for free tier tracking.
/// Shared across all clients; one instance per app.
pub struct FreeTierManager {
    /// Per-provider rate limit tracking (for RateLimitedFree providers)
    rate_trackers: DashMap<String, RwLock<RateLimitTracker>>,
    /// Per-provider credit tracking (for CreditBased providers)
    credit_trackers: DashMap<String, RwLock<CreditTracker>>,
    /// Per provider-model backoff tracking (in-memory only)
    backoffs: DashMap<String, RwLock<ProviderBackoff>>,
    /// Persistence path
    persist_path: Option<PathBuf>,
}

impl FreeTierManager {
    /// Create a new FreeTierManager
    pub fn new(persist_path: Option<PathBuf>) -> Self {
        Self {
            rate_trackers: DashMap::new(),
            credit_trackers: DashMap::new(),
            backoffs: DashMap::new(),
            persist_path,
        }
    }

    /// Load persisted state from disk
    pub fn load(path: &Path) -> Self {
        let persist_path = path.to_path_buf();
        let manager = Self::new(Some(persist_path.clone()));

        if let Ok(data) = std::fs::read_to_string(&persist_path) {
            if let Ok(state) = serde_json::from_str::<FreeTierState>(&data) {
                for (key, tracker) in state.rate_trackers {
                    manager.rate_trackers.insert(key, RwLock::new(tracker));
                }
                for (key, tracker) in state.credit_trackers {
                    manager.credit_trackers.insert(key, RwLock::new(tracker));
                }
                debug!("Loaded free tier state from {:?}", path);
            } else {
                warn!("Failed to parse free tier state from {:?}", path);
            }
        }

        manager
    }

    /// Persist state to disk
    pub fn persist(&self) -> Result<(), std::io::Error> {
        let Some(ref path) = self.persist_path else {
            return Ok(());
        };

        let state = FreeTierState {
            rate_trackers: self
                .rate_trackers
                .iter()
                .map(|entry| (entry.key().clone(), entry.value().read().clone()))
                .collect(),
            credit_trackers: self
                .credit_trackers
                .iter()
                .map(|entry| (entry.key().clone(), entry.value().read().clone()))
                .collect(),
        };

        let data = serde_json::to_string_pretty(&state).map_err(std::io::Error::other)?;

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, data)?;
        debug!("Persisted free tier state to {:?}", path);
        Ok(())
    }

    // ============================================================
    // Classification
    // ============================================================

    /// Determine if a model/provider combination is free
    pub fn classify_model(
        &self,
        provider_instance: &str,
        model: &str,
        free_tier: &FreeTierKind,
    ) -> ModelFreeStatus {
        match free_tier {
            FreeTierKind::AlwaysFreeLocal | FreeTierKind::Subscription => {
                ModelFreeStatus::AlwaysFree
            }
            FreeTierKind::None => ModelFreeStatus::NotFree,
            FreeTierKind::FreeModelsOnly {
                free_model_patterns,
                ..
            } => {
                if Self::model_matches_patterns(model, free_model_patterns) {
                    ModelFreeStatus::FreeModel
                } else {
                    ModelFreeStatus::NotFree
                }
            }
            FreeTierKind::RateLimitedFree { .. } => {
                let capacity = self.check_rate_limit_capacity(provider_instance, free_tier);
                if capacity.has_capacity {
                    ModelFreeStatus::FreeWithinLimits
                } else {
                    ModelFreeStatus::NotFree
                }
            }
            FreeTierKind::CreditBased { .. } => {
                let capacity = self.check_credit_balance(provider_instance, free_tier);
                if capacity.has_capacity {
                    ModelFreeStatus::FreeWithinLimits
                } else {
                    ModelFreeStatus::NotFree
                }
            }
        }
    }

    fn model_matches_patterns(model: &str, patterns: &[String]) -> bool {
        for pattern in patterns {
            if pattern.contains('*') {
                // Simple glob: "*:free" matches "anything:free"
                let parts: Vec<&str> = pattern.split('*').collect();
                if parts.len() == 2 {
                    let (prefix, suffix) = (parts[0], parts[1]);
                    if model.starts_with(prefix) && model.ends_with(suffix) {
                        return true;
                    }
                }
            } else if model == pattern {
                return true;
            }
        }
        false
    }

    // ============================================================
    // Rate Limit Tracking
    // ============================================================

    /// Check if rate-limited free tier has capacity
    pub fn check_rate_limit_capacity(
        &self,
        provider_instance: &str,
        free_tier: &FreeTierKind,
    ) -> FreeTierCapacity {
        let FreeTierKind::RateLimitedFree {
            max_rpm,
            max_rpd,
            max_tpm,
            max_tpd,
            max_monthly_calls,
            max_monthly_tokens,
        } = free_tier
        else {
            return FreeTierCapacity {
                has_capacity: true,
                remaining_pct: None,
                remaining_usd: None,
                status_message: "Not rate-limited".to_string(),
            };
        };

        let entry = self
            .rate_trackers
            .entry(provider_instance.to_string())
            .or_insert_with(|| RwLock::new(RateLimitTracker::default()));
        let mut tracker = entry.write();
        tracker.reset_expired_windows();

        // Check each limit, tracking the tightest constraint
        let mut min_remaining_pct: f32 = 1.0;
        let mut limiting_factor = String::new();

        // Prefer header-reported values over client-side counters
        if *max_rpm > 0 {
            let used = if let Some(remaining) = tracker.header_requests_remaining {
                let limit = tracker
                    .header_requests_remaining
                    .map(|r| r + tracker.minute_requests as u64)
                    .unwrap_or(*max_rpm as u64);
                let pct = if limit > 0 {
                    remaining as f32 / limit as f32
                } else {
                    0.0
                };
                if pct < min_remaining_pct {
                    min_remaining_pct = pct;
                    limiting_factor = format!("RPM: {} remaining", remaining);
                }
                remaining == 0
            } else {
                let pct = 1.0 - (tracker.minute_requests as f32 / *max_rpm as f32);
                if pct < min_remaining_pct {
                    min_remaining_pct = pct;
                    limiting_factor = format!("RPM: {}/{} used", tracker.minute_requests, max_rpm);
                }
                tracker.minute_requests >= *max_rpm
            };
            if used {
                return FreeTierCapacity {
                    has_capacity: false,
                    remaining_pct: Some(0.0),
                    remaining_usd: None,
                    status_message: limiting_factor,
                };
            }
        }

        if *max_rpd > 0 {
            let exhausted = if let Some(remaining) = tracker.header_daily_requests_remaining {
                let pct = remaining as f32 / *max_rpd as f32;
                if pct < min_remaining_pct {
                    min_remaining_pct = pct;
                    limiting_factor = format!("RPD: {} remaining", remaining);
                }
                remaining == 0
            } else {
                let pct = 1.0 - (tracker.daily_requests as f32 / *max_rpd as f32);
                if pct < min_remaining_pct {
                    min_remaining_pct = pct;
                    limiting_factor = format!("RPD: {}/{} used", tracker.daily_requests, max_rpd);
                }
                tracker.daily_requests >= *max_rpd
            };
            if exhausted {
                return FreeTierCapacity {
                    has_capacity: false,
                    remaining_pct: Some(0.0),
                    remaining_usd: None,
                    status_message: limiting_factor,
                };
            }
        }

        if *max_tpm > 0 {
            let exhausted = if let Some(remaining) = tracker.header_tokens_remaining {
                let pct = remaining as f32 / *max_tpm as f32;
                if pct < min_remaining_pct {
                    min_remaining_pct = pct;
                    limiting_factor = format!("TPM: {} remaining", remaining);
                }
                remaining == 0
            } else {
                let pct = 1.0 - (tracker.minute_tokens as f32 / *max_tpm as f32);
                if pct < min_remaining_pct {
                    min_remaining_pct = pct;
                    limiting_factor = format!("TPM: {}/{} used", tracker.minute_tokens, max_tpm);
                }
                tracker.minute_tokens >= *max_tpm
            };
            if exhausted {
                return FreeTierCapacity {
                    has_capacity: false,
                    remaining_pct: Some(0.0),
                    remaining_usd: None,
                    status_message: limiting_factor,
                };
            }
        }

        if *max_tpd > 0 {
            let pct = 1.0 - (tracker.daily_tokens as f32 / *max_tpd as f32);
            if pct < min_remaining_pct {
                min_remaining_pct = pct;
                limiting_factor = format!("TPD: {}/{} used", tracker.daily_tokens, max_tpd);
            }
            if tracker.daily_tokens >= *max_tpd {
                return FreeTierCapacity {
                    has_capacity: false,
                    remaining_pct: Some(0.0),
                    remaining_usd: None,
                    status_message: limiting_factor,
                };
            }
        }

        if *max_monthly_calls > 0 {
            let pct = 1.0 - (tracker.monthly_requests as f32 / *max_monthly_calls as f32);
            if pct < min_remaining_pct {
                min_remaining_pct = pct;
                limiting_factor = format!(
                    "Monthly calls: {}/{} used",
                    tracker.monthly_requests, max_monthly_calls
                );
            }
            if tracker.monthly_requests >= *max_monthly_calls {
                return FreeTierCapacity {
                    has_capacity: false,
                    remaining_pct: Some(0.0),
                    remaining_usd: None,
                    status_message: limiting_factor,
                };
            }
        }

        if *max_monthly_tokens > 0 {
            let pct = 1.0 - (tracker.monthly_tokens as f32 / *max_monthly_tokens as f32);
            if pct < min_remaining_pct {
                min_remaining_pct = pct;
                limiting_factor = format!(
                    "Monthly tokens: {}/{} used",
                    tracker.monthly_tokens, max_monthly_tokens
                );
            }
            if tracker.monthly_tokens >= *max_monthly_tokens {
                return FreeTierCapacity {
                    has_capacity: false,
                    remaining_pct: Some(0.0),
                    remaining_usd: None,
                    status_message: limiting_factor,
                };
            }
        }

        FreeTierCapacity {
            has_capacity: true,
            remaining_pct: Some(min_remaining_pct),
            remaining_usd: None,
            status_message: if limiting_factor.is_empty() {
                "Available".to_string()
            } else {
                limiting_factor
            },
        }
    }

    /// Update rate limit tracking from response headers
    pub fn update_from_headers(&self, provider_instance: &str, headers: &RateLimitHeaderInfo) {
        let entry = self
            .rate_trackers
            .entry(provider_instance.to_string())
            .or_insert_with(|| RwLock::new(RateLimitTracker::default()));
        let mut tracker = entry.write();

        if let Some(remaining) = headers.requests_remaining {
            tracker.header_requests_remaining = Some(remaining);
            tracker.header_updated_at = Some(Instant::now());
        }
        if let Some(remaining) = headers.tokens_remaining {
            tracker.header_tokens_remaining = Some(remaining);
            tracker.header_updated_at = Some(Instant::now());
        }
        if let Some(remaining) = headers.daily_requests_remaining {
            tracker.header_daily_requests_remaining = Some(remaining);
            tracker.header_updated_at = Some(Instant::now());
        }
    }

    /// Record a request for rate limit tracking
    pub fn record_rate_limit_usage(&self, provider_instance: &str, tokens: u64) {
        let entry = self
            .rate_trackers
            .entry(provider_instance.to_string())
            .or_insert_with(|| RwLock::new(RateLimitTracker::default()));
        let mut tracker = entry.write();
        tracker.reset_expired_windows();

        tracker.minute_requests += 1;
        tracker.daily_requests += 1;
        tracker.monthly_requests += 1;
        tracker.minute_tokens += tokens;
        tracker.daily_tokens += tokens;
        tracker.monthly_tokens += tokens;
    }

    // ============================================================
    // Credit Tracking
    // ============================================================

    /// Check credit-based free tier remaining balance
    pub fn check_credit_balance(
        &self,
        provider_instance: &str,
        free_tier: &FreeTierKind,
    ) -> FreeTierCapacity {
        let FreeTierKind::CreditBased {
            budget_usd,
            reset_period,
            ..
        } = free_tier
        else {
            return FreeTierCapacity {
                has_capacity: true,
                remaining_pct: None,
                remaining_usd: None,
                status_message: "Not credit-based".to_string(),
            };
        };

        let entry = self
            .credit_trackers
            .entry(provider_instance.to_string())
            .or_insert_with(|| RwLock::new(CreditTracker::default()));
        let mut tracker = entry.write();

        // Check if period has reset
        let now = Utc::now();
        let should_reset = match reset_period {
            lr_config::FreeTierResetPeriod::Daily => {
                now.ordinal() != tracker.period_start.ordinal()
                    || now.year() != tracker.period_start.year()
            }
            lr_config::FreeTierResetPeriod::Monthly => {
                now.month() != tracker.period_start.month()
                    || now.year() != tracker.period_start.year()
            }
            lr_config::FreeTierResetPeriod::Never => false,
        };

        if should_reset {
            tracker.current_cost_usd = 0.0;
            tracker.period_start = now;
            tracker.api_remaining_usd = None;
        }

        // Use API-reported balance if available and recent
        let remaining = if let Some(api_remaining) = tracker.api_remaining_usd {
            api_remaining
        } else {
            budget_usd - tracker.current_cost_usd
        };

        if remaining <= 0.0 {
            FreeTierCapacity {
                has_capacity: false,
                remaining_pct: Some(0.0),
                remaining_usd: Some(0.0),
                status_message: format!(
                    "Credits exhausted: ${:.2} / ${:.2} used",
                    tracker.current_cost_usd, budget_usd
                ),
            }
        } else {
            let pct = if *budget_usd > 0.0 {
                (remaining / budget_usd) as f32
            } else {
                1.0
            };
            FreeTierCapacity {
                has_capacity: true,
                remaining_pct: Some(pct),
                remaining_usd: Some(remaining),
                status_message: format!("${:.4} remaining", remaining),
            }
        }
    }

    /// Record cost for credit tracking
    pub fn record_credit_usage(&self, provider_instance: &str, cost_usd: f64) {
        let entry = self
            .credit_trackers
            .entry(provider_instance.to_string())
            .or_insert_with(|| RwLock::new(CreditTracker::default()));
        let mut tracker = entry.write();
        tracker.current_cost_usd += cost_usd;
    }

    /// Record usage based on the provider's free tier kind.
    ///
    /// Dispatches to the appropriate tracker:
    /// - `RateLimitedFree` → rate limit tracker
    /// - `CreditBased` → credit tracker
    /// - `FreeModelsOnly` → rate limit tracker (if has max_rpm)
    /// - `AlwaysFreeLocal` / `Subscription` / `None` → no-op
    pub fn record_usage(
        &self,
        provider_instance: &str,
        free_tier: &FreeTierKind,
        total_tokens: u64,
        cost_usd: f64,
    ) {
        match free_tier {
            FreeTierKind::RateLimitedFree { .. } => {
                self.record_rate_limit_usage(provider_instance, total_tokens);
            }
            FreeTierKind::CreditBased { .. } => {
                self.record_credit_usage(provider_instance, cost_usd);
            }
            FreeTierKind::FreeModelsOnly { max_rpm, .. } if *max_rpm > 0 => {
                self.record_rate_limit_usage(provider_instance, total_tokens);
            }
            // AlwaysFreeLocal, Subscription, None, FreeModelsOnly with max_rpm=0
            _ => {}
        }
    }

    /// Update credit tracker with info from provider API
    pub fn update_credits_from_api(
        &self,
        provider_instance: &str,
        info: &lr_providers::ProviderCreditsInfo,
    ) {
        let entry = self
            .credit_trackers
            .entry(provider_instance.to_string())
            .or_insert_with(|| RwLock::new(CreditTracker::default()));
        let mut tracker = entry.write();

        tracker.api_remaining_usd = info.remaining_credits_usd;
        tracker.api_is_free_tier = info.is_free_tier;
        tracker.api_last_checked = Some(Instant::now());

        if let Some(used) = info.used_credits_usd {
            tracker.current_cost_usd = used;
        }
    }

    // ============================================================
    // Backoff Tracking
    // ============================================================

    fn backoff_key(provider: &str, model: &str) -> String {
        format!("{}::{}", provider, model)
    }

    /// Record a 429/402 error and compute backoff
    pub fn record_rate_limit_error(
        &self,
        provider_instance: &str,
        model: &str,
        _status_code: u16,
        retry_after_secs: Option<u64>,
        rate_limit_reset_secs: Option<u64>,
        is_credit_exhaustion: bool,
    ) {
        let key = Self::backoff_key(provider_instance, model);
        let entry = self
            .backoffs
            .entry(key)
            .or_insert_with(|| RwLock::new(ProviderBackoff::default()));
        let mut backoff = entry.write();

        backoff.consecutive_errors += 1;
        backoff.is_credit_exhausted = is_credit_exhaustion;

        // Determine backoff duration (priority order)
        let (duration, source) = if let Some(retry_after) = retry_after_secs {
            (
                Duration::from_secs(retry_after),
                BackoffSource::RetryAfterHeader,
            )
        } else if let Some(reset) = rate_limit_reset_secs {
            (
                Duration::from_secs(reset),
                BackoffSource::RateLimitResetHeader,
            )
        } else if is_credit_exhaustion {
            // Credit exhaustion: longer backoff
            let secs = match backoff.consecutive_errors {
                1 => 300,   // 5 min
                2 => 900,   // 15 min
                3 => 3600,  // 1 hr
                4 => 21600, // 6 hr
                _ => 86400, // 24 hr
            };
            (
                Duration::from_secs(secs),
                BackoffSource::CreditReplenishment,
            )
        } else {
            // Exponential backoff: 1s, 2s, 4s, 8s, 16s, 32s, 60s max
            let secs = (1u64 << backoff.consecutive_errors.min(5)).min(60);
            (Duration::from_secs(secs), BackoffSource::ExponentialBackoff)
        };

        backoff.current_backoff = duration;
        backoff.available_at = Some(Instant::now() + duration);
        backoff.backoff_source = source;

        debug!(
            "Recorded backoff for {}/{}: {:?} ({}s, {} consecutive errors)",
            provider_instance,
            model,
            backoff.backoff_source,
            duration.as_secs(),
            backoff.consecutive_errors,
        );
    }

    /// Check if a provider-model is currently in backoff
    pub fn is_in_backoff(&self, provider_instance: &str, model: &str) -> Option<BackoffInfo> {
        let key = Self::backoff_key(provider_instance, model);
        let entry = self.backoffs.get(&key)?;
        let backoff = entry.read();

        let available_at = backoff.available_at?;
        let now = Instant::now();
        if now >= available_at {
            // Backoff expired
            return None;
        }

        let remaining = available_at - now;
        let reason = if backoff.is_credit_exhausted {
            format!("credits exhausted (available in {}s)", remaining.as_secs())
        } else {
            format!("rate limited (available in {}s)", remaining.as_secs())
        };

        Some(BackoffInfo {
            available_at,
            retry_after_secs: remaining.as_secs(),
            reason,
        })
    }

    /// Clear backoff after a successful request
    pub fn clear_backoff(&self, provider_instance: &str, model: &str) {
        let key = Self::backoff_key(provider_instance, model);
        self.backoffs.remove(&key);
    }

    /// Get minimum retry-after across all providers in the given model list.
    /// Returns None if no providers are in backoff.
    pub fn get_min_retry_after(&self, models: &[(String, String)]) -> Option<u64> {
        let mut min_retry: Option<u64> = None;

        for (provider, model) in models {
            if let Some(info) = self.is_in_backoff(provider, model) {
                let current_min = min_retry.unwrap_or(u64::MAX);
                if info.retry_after_secs < current_min {
                    min_retry = Some(info.retry_after_secs);
                }
            }
        }

        min_retry
    }

    // ============================================================
    // Status (for UI)
    // ============================================================

    /// Reset all usage for a provider
    pub fn reset_usage(&self, provider_instance: &str) {
        if let Some(entry) = self.rate_trackers.get(provider_instance) {
            *entry.write() = RateLimitTracker::default();
        }
        if let Some(entry) = self.credit_trackers.get(provider_instance) {
            *entry.write() = CreditTracker::default();
        }
        // Clear all backoffs for this provider
        let prefix = format!("{}::", provider_instance);
        self.backoffs.retain(|k, _| !k.starts_with(&prefix));
    }

    /// Manually set credit usage for a provider (from UI)
    pub fn set_credit_usage(
        &self,
        provider_instance: &str,
        used_usd: Option<f64>,
        remaining_usd: Option<f64>,
    ) {
        let entry = self
            .credit_trackers
            .entry(provider_instance.to_string())
            .or_insert_with(|| RwLock::new(CreditTracker::default()));
        let mut tracker = entry.write();
        if let Some(used) = used_usd {
            tracker.current_cost_usd = used;
        }
        if let Some(remaining) = remaining_usd {
            tracker.api_remaining_usd = Some(remaining);
        }
    }

    /// Manually set rate limit usage for a provider (from UI)
    pub fn set_rate_limit_usage(
        &self,
        provider_instance: &str,
        daily_requests: Option<u32>,
        monthly_requests: Option<u32>,
        monthly_tokens: Option<u64>,
    ) {
        let entry = self
            .rate_trackers
            .entry(provider_instance.to_string())
            .or_insert_with(|| RwLock::new(RateLimitTracker::default()));
        let mut tracker = entry.write();
        if let Some(v) = daily_requests {
            tracker.daily_requests = v;
        }
        if let Some(v) = monthly_requests {
            tracker.monthly_requests = v;
        }
        if let Some(v) = monthly_tokens {
            tracker.monthly_tokens = v;
        }
    }

    /// Get rate limit tracker for a provider (for UI status)
    pub fn get_rate_tracker(&self, provider_instance: &str) -> Option<RateLimitTracker> {
        self.rate_trackers
            .get(provider_instance)
            .map(|entry| entry.read().clone())
    }

    /// Get credit tracker for a provider (for UI status)
    pub fn get_credit_tracker(&self, provider_instance: &str) -> Option<CreditTracker> {
        self.credit_trackers
            .get(provider_instance)
            .map(|entry| entry.read().clone())
    }

    /// Check if any model for this provider is in backoff
    pub fn get_provider_backoff_info(&self, provider_instance: &str) -> Option<BackoffInfo> {
        let prefix = format!("{}::", provider_instance);
        let now = Instant::now();
        let mut best: Option<BackoffInfo> = None;

        for entry in self.backoffs.iter() {
            if entry.key().starts_with(&prefix) {
                let backoff = entry.value().read();
                if let Some(available_at) = backoff.available_at {
                    if now < available_at {
                        let remaining = available_at - now;
                        let info = BackoffInfo {
                            available_at,
                            retry_after_secs: remaining.as_secs(),
                            reason: if backoff.is_credit_exhausted {
                                "credits exhausted".to_string()
                            } else {
                                "rate limited".to_string()
                            },
                        };
                        if best
                            .as_ref()
                            .is_none_or(|b| info.retry_after_secs < b.retry_after_secs)
                        {
                            best = Some(info);
                        }
                    }
                }
            }
        }

        best
    }
}

// ============================================================
// Tests
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn make_headers(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn test_parse_standard_headers() {
        let headers = make_headers(&[
            ("x-ratelimit-remaining-requests", "10"),
            ("x-ratelimit-limit-requests", "100"),
            ("x-ratelimit-remaining-tokens", "5000"),
            ("x-ratelimit-limit-tokens", "10000"),
        ]);

        let info = parse_rate_limit_headers(&headers);
        assert_eq!(info.requests_remaining, Some(10));
        assert_eq!(info.requests_limit, Some(100));
        assert_eq!(info.tokens_remaining, Some(5000));
        assert_eq!(info.tokens_limit, Some(10000));
    }

    #[test]
    fn test_parse_cerebras_daily_headers() {
        let headers = make_headers(&[
            ("x-ratelimit-remaining-requests-day", "1000"),
            ("x-ratelimit-limit-requests-day", "14400"),
            ("x-ratelimit-remaining-tokens-minute", "50000"),
            ("x-ratelimit-limit-tokens-minute", "60000"),
        ]);

        let info = parse_rate_limit_headers(&headers);
        assert_eq!(info.daily_requests_remaining, Some(1000));
        assert_eq!(info.daily_requests_limit, Some(14400));
        assert_eq!(info.tokens_remaining, Some(50000));
        assert_eq!(info.tokens_limit, Some(60000));
    }

    #[test]
    fn test_parse_together_ai_headers() {
        let headers = make_headers(&[
            ("x-ratelimit-remaining", "5"),
            ("x-ratelimit-limit", "10"),
            ("x-tokenlimit-remaining", "2000"),
            ("x-tokenlimit-limit", "5000"),
        ]);

        let info = parse_rate_limit_headers(&headers);
        assert_eq!(info.requests_remaining, Some(5));
        assert_eq!(info.requests_limit, Some(10));
        assert_eq!(info.tokens_remaining, Some(2000));
        assert_eq!(info.tokens_limit, Some(5000));
    }

    #[test]
    fn test_parse_anthropic_headers() {
        let headers = make_headers(&[
            ("anthropic-ratelimit-requests-remaining", "50"),
            ("anthropic-ratelimit-requests-limit", "100"),
            ("anthropic-ratelimit-tokens-remaining", "80000"),
            ("anthropic-ratelimit-tokens-limit", "100000"),
        ]);

        let info = parse_rate_limit_headers(&headers);
        assert_eq!(info.requests_remaining, Some(50));
        assert_eq!(info.requests_limit, Some(100));
        assert_eq!(info.tokens_remaining, Some(80000));
        assert_eq!(info.tokens_limit, Some(100000));
    }

    #[test]
    fn test_parse_retry_after() {
        let headers = make_headers(&[("retry-after", "30")]);
        let info = parse_rate_limit_headers(&headers);
        assert_eq!(info.retry_after_secs, Some(30));
    }

    #[test]
    fn test_parse_retry_after_ms() {
        let headers = make_headers(&[("retry-after-ms", "5000")]);
        let info = parse_rate_limit_headers(&headers);
        assert_eq!(info.retry_after_secs, Some(5));
    }

    #[test]
    fn test_model_matches_patterns() {
        assert!(FreeTierManager::model_matches_patterns(
            "meta-llama/Llama-3.3-70B-Instruct-Turbo-Free",
            &["meta-llama/Llama-3.3-70B-Instruct-Turbo-Free".to_string()]
        ));

        assert!(FreeTierManager::model_matches_patterns(
            "anything:free",
            &["*:free".to_string()]
        ));

        assert!(!FreeTierManager::model_matches_patterns(
            "gpt-4",
            &["*:free".to_string()]
        ));
    }

    #[test]
    fn test_classify_always_free() {
        let manager = FreeTierManager::new(None);
        assert_eq!(
            manager.classify_model("ollama", "llama3", &FreeTierKind::AlwaysFreeLocal),
            ModelFreeStatus::AlwaysFree
        );
    }

    #[test]
    fn test_classify_none() {
        let manager = FreeTierManager::new(None);
        assert_eq!(
            manager.classify_model("openai", "gpt-4", &FreeTierKind::None),
            ModelFreeStatus::NotFree
        );
    }

    #[test]
    fn test_classify_free_models_only() {
        let manager = FreeTierManager::new(None);
        let free_tier = FreeTierKind::FreeModelsOnly {
            free_model_patterns: vec!["meta-llama/Llama-3.3-70B-Instruct-Turbo-Free".to_string()],
            max_rpm: 3,
        };
        assert_eq!(
            manager.classify_model(
                "togetherai",
                "meta-llama/Llama-3.3-70B-Instruct-Turbo-Free",
                &free_tier
            ),
            ModelFreeStatus::FreeModel
        );
        assert_eq!(
            manager.classify_model("togetherai", "gpt-4", &free_tier),
            ModelFreeStatus::NotFree
        );
    }

    #[test]
    fn test_rate_limit_tracking() {
        let manager = FreeTierManager::new(None);
        let free_tier = FreeTierKind::RateLimitedFree {
            max_rpm: 3,
            max_rpd: 0,
            max_tpm: 0,
            max_tpd: 0,
            max_monthly_calls: 0,
            max_monthly_tokens: 0,
        };

        // Initially has capacity
        let cap = manager.check_rate_limit_capacity("groq", &free_tier);
        assert!(cap.has_capacity);

        // Record 3 requests
        manager.record_rate_limit_usage("groq", 100);
        manager.record_rate_limit_usage("groq", 100);
        manager.record_rate_limit_usage("groq", 100);

        // Should be exhausted
        let cap = manager.check_rate_limit_capacity("groq", &free_tier);
        assert!(!cap.has_capacity);
    }

    #[test]
    fn test_credit_tracking() {
        let manager = FreeTierManager::new(None);
        let free_tier = FreeTierKind::CreditBased {
            budget_usd: 5.0,
            reset_period: lr_config::FreeTierResetPeriod::Monthly,
            detection: lr_config::CreditDetection::LocalOnly,
        };

        let cap = manager.check_credit_balance("deepinfra", &free_tier);
        assert!(cap.has_capacity);
        assert_eq!(cap.remaining_usd, Some(5.0));

        manager.record_credit_usage("deepinfra", 5.0);

        let cap = manager.check_credit_balance("deepinfra", &free_tier);
        assert!(!cap.has_capacity);
    }

    #[test]
    fn test_backoff_tracking() {
        let manager = FreeTierManager::new(None);

        // No backoff initially
        assert!(manager.is_in_backoff("groq", "llama3").is_none());

        // Record error
        manager.record_rate_limit_error("groq", "llama3", 429, Some(30), None, false);

        // Should be in backoff
        let info = manager.is_in_backoff("groq", "llama3");
        assert!(info.is_some());
        assert!(info.unwrap().retry_after_secs <= 30);

        // Clear backoff
        manager.clear_backoff("groq", "llama3");
        assert!(manager.is_in_backoff("groq", "llama3").is_none());
    }

    #[test]
    fn test_exponential_backoff() {
        let manager = FreeTierManager::new(None);

        // First error: 1s backoff
        manager.record_rate_limit_error("p", "m", 429, None, None, false);
        let info = manager.is_in_backoff("p", "m").unwrap();
        assert!(info.retry_after_secs <= 2); // 2^1 = 2

        // Clear and record more errors to verify exponential growth
        manager.clear_backoff("p", "m");
    }

    #[test]
    fn test_persist_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("free_tier.json");

        // Create manager, add data, persist
        let manager = FreeTierManager::new(Some(path.clone()));
        manager.record_rate_limit_usage("groq", 100);
        manager.record_credit_usage("openrouter", 1.5);
        manager.persist().unwrap();

        // Load and verify
        let loaded = FreeTierManager::load(&path);
        let tracker = loaded.get_rate_tracker("groq").unwrap();
        assert_eq!(tracker.minute_requests, 1);
        let credit = loaded.get_credit_tracker("openrouter").unwrap();
        assert!((credit.current_cost_usd - 1.5).abs() < f64::EPSILON);
    }

    #[test]
    fn test_reset_usage() {
        let manager = FreeTierManager::new(None);
        manager.record_rate_limit_usage("groq", 100);
        manager.record_credit_usage("groq", 1.0);
        manager.record_rate_limit_error("groq", "llama3", 429, Some(30), None, false);

        manager.reset_usage("groq");

        let tracker = manager.get_rate_tracker("groq").unwrap();
        assert_eq!(tracker.minute_requests, 0);
        let credit = manager.get_credit_tracker("groq").unwrap();
        assert!((credit.current_cost_usd).abs() < f64::EPSILON);
        assert!(manager.is_in_backoff("groq", "llama3").is_none());
    }

    #[test]
    fn test_min_retry_after() {
        let manager = FreeTierManager::new(None);

        let models = vec![
            ("a".to_string(), "m1".to_string()),
            ("b".to_string(), "m2".to_string()),
        ];

        // No backoffs
        assert!(manager.get_min_retry_after(&models).is_none());

        // Add backoffs
        manager.record_rate_limit_error("a", "m1", 429, Some(60), None, false);
        manager.record_rate_limit_error("b", "m2", 429, Some(30), None, false);

        let min = manager.get_min_retry_after(&models).unwrap();
        assert!(min <= 30);
    }

    // ============================================================
    // record_usage() tests
    // ============================================================

    #[test]
    fn test_record_usage_rate_limited_free() {
        let manager = FreeTierManager::new(None);
        let free_tier = FreeTierKind::RateLimitedFree {
            max_rpm: 10,
            max_rpd: 0,
            max_tpm: 0,
            max_tpd: 0,
            max_monthly_calls: 0,
            max_monthly_tokens: 0,
        };

        manager.record_usage("groq", &free_tier, 500, 0.001);

        let tracker = manager.get_rate_tracker("groq").unwrap();
        assert_eq!(tracker.minute_requests, 1);
        assert_eq!(tracker.minute_tokens, 500);
        // Credit tracker should not be touched
        assert!(manager.get_credit_tracker("groq").is_none());
    }

    #[test]
    fn test_record_usage_credit_based() {
        let manager = FreeTierManager::new(None);
        let free_tier = FreeTierKind::CreditBased {
            budget_usd: 5.0,
            reset_period: lr_config::FreeTierResetPeriod::Monthly,
            detection: lr_config::CreditDetection::LocalOnly,
        };

        manager.record_usage("openrouter", &free_tier, 500, 0.01);

        let credit = manager.get_credit_tracker("openrouter").unwrap();
        assert!((credit.current_cost_usd - 0.01).abs() < f64::EPSILON);
        // Rate tracker should not be touched
        assert!(manager.get_rate_tracker("openrouter").is_none());
    }

    #[test]
    fn test_record_usage_free_models_only() {
        let manager = FreeTierManager::new(None);
        let free_tier = FreeTierKind::FreeModelsOnly {
            free_model_patterns: vec!["*:free".to_string()],
            max_rpm: 5,
        };

        manager.record_usage("togetherai", &free_tier, 200, 0.0);

        let tracker = manager.get_rate_tracker("togetherai").unwrap();
        assert_eq!(tracker.minute_requests, 1);
        assert_eq!(tracker.minute_tokens, 200);
    }

    #[test]
    fn test_record_usage_always_free_local_is_noop() {
        let manager = FreeTierManager::new(None);
        manager.record_usage("ollama", &FreeTierKind::AlwaysFreeLocal, 1000, 0.0);
        assert!(manager.get_rate_tracker("ollama").is_none());
        assert!(manager.get_credit_tracker("ollama").is_none());
    }

    #[test]
    fn test_record_usage_subscription_is_noop() {
        let manager = FreeTierManager::new(None);
        manager.record_usage("lmstudio", &FreeTierKind::Subscription, 1000, 0.0);
        assert!(manager.get_rate_tracker("lmstudio").is_none());
        assert!(manager.get_credit_tracker("lmstudio").is_none());
    }

    #[test]
    fn test_record_usage_none_is_noop() {
        let manager = FreeTierManager::new(None);
        manager.record_usage("openai", &FreeTierKind::None, 1000, 0.05);
        assert!(manager.get_rate_tracker("openai").is_none());
        assert!(manager.get_credit_tracker("openai").is_none());
    }

    #[test]
    fn test_record_usage_accumulates_across_requests() {
        let manager = FreeTierManager::new(None);
        let free_tier = FreeTierKind::RateLimitedFree {
            max_rpm: 100,
            max_rpd: 0,
            max_tpm: 0,
            max_tpd: 0,
            max_monthly_calls: 0,
            max_monthly_tokens: 0,
        };

        manager.record_usage("gemini", &free_tier, 100, 0.0);
        manager.record_usage("gemini", &free_tier, 200, 0.0);
        manager.record_usage("gemini", &free_tier, 300, 0.0);

        let tracker = manager.get_rate_tracker("gemini").unwrap();
        assert_eq!(tracker.minute_requests, 3);
        assert_eq!(tracker.minute_tokens, 600);
    }

    #[test]
    fn test_record_usage_credit_exhaustion() {
        let manager = FreeTierManager::new(None);
        let free_tier = FreeTierKind::CreditBased {
            budget_usd: 1.0,
            reset_period: lr_config::FreeTierResetPeriod::Never,
            detection: lr_config::CreditDetection::LocalOnly,
        };

        // Use up the full budget
        manager.record_usage("deepinfra", &free_tier, 500, 0.50);
        manager.record_usage("deepinfra", &free_tier, 500, 0.50);

        let cap = manager.check_credit_balance("deepinfra", &free_tier);
        assert!(!cap.has_capacity);
        assert_eq!(cap.remaining_usd, Some(0.0));
    }

    #[test]
    fn test_record_usage_rate_limit_exhaustion() {
        let manager = FreeTierManager::new(None);
        let free_tier = FreeTierKind::RateLimitedFree {
            max_rpm: 2,
            max_rpd: 0,
            max_tpm: 0,
            max_tpd: 0,
            max_monthly_calls: 0,
            max_monthly_tokens: 0,
        };

        manager.record_usage("cerebras", &free_tier, 100, 0.0);
        manager.record_usage("cerebras", &free_tier, 100, 0.0);

        let cap = manager.check_rate_limit_capacity("cerebras", &free_tier);
        assert!(!cap.has_capacity);
    }
}
