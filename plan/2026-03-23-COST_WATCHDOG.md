# Plan: Adaptive Rate Limiter for Free Tier Providers (Expanded)

## Context

Free tier providers have varying, undocumented rate limits that change by model, region, and anti-abuse systems. Our static defaults may not match reality. We need the system to **learn** actual limits from provider feedback and adapt over time.

### Current State

The existing `FreeTierManager` (`crates/lr-router/src/free_tier.rs`) has:
- **Backoff system**: Tracks 429/402 errors per provider-model with exponential backoff
- **Rate limit header parsing**: `parse_rate_limit_headers()` extracts `x-ratelimit-*` headers
- **`update_from_headers()`**: Exists but is **never called** from production code — only tests
- **Persistence**: `FreeTierState` saves rate trackers + credit trackers to disk (not backoffs)

**Key gap**: Provider HTTP response headers (containing rate limit info) are **discarded** — `CompletionResponse` doesn't carry them back to the router. The `ModelProvider::complete()` trait returns `AppResult<CompletionResponse>` with no header passthrough.

---

## Part 1: Plumb Response Headers Through the Provider Pipeline

### Step 1.1: Add response headers to CompletionResponse

**File:** `crates/lr-providers/src/lib.rs` (CompletionResponse struct, ~line 1299)

```rust
/// HTTP response headers from the provider (rate limit info, etc.)
/// Not serialized to clients — internal use only.
#[serde(skip)]
pub response_headers: Option<HashMap<String, String>>,
```

### Step 1.2: Capture headers in each provider's HTTP call

All OpenAI-compatible providers use a shared HTTP client. Find the common response-building code and extract headers before deserializing the JSON body. The `x-ratelimit-*` and `retry-after` headers should be captured into `response_headers`.

**Files:** Each provider's `complete()` implementation (most use a shared `send_request` or similar pattern). Need to identify the common code path.

### Step 1.3: Pass headers to FreeTierManager after successful responses

**File:** `crates/lr-router/src/lib.rs` (after successful `execute_request`)

After a successful response, call:
```rust
let headers_info = parse_rate_limit_headers(&response.response_headers.unwrap_or_default());
self.free_tier_manager.update_from_headers(provider, &headers_info);
self.free_tier_manager.update_learned_limits(provider, &headers_info, &effective_free_tier);
```

---

## Part 2: Adaptive Limit Learning Algorithm

### How limits are learned

There are **two signal sources**, each with different adaptation logic:

#### Signal A: Rate limit headers on successful responses

Every response from providers like Groq, Cerebras, OpenAI includes headers:
```
x-ratelimit-limit-requests: 30      ← the provider's actual RPM limit
x-ratelimit-remaining-requests: 25   ← remaining in window
x-ratelimit-limit-tokens: 6000      ← the provider's actual TPM limit
```

**Algorithm**: When `x-ratelimit-limit-requests` < our configured `max_rpm`, adopt the provider's reported limit as our effective limit. This is an **authoritative** signal — the provider is telling us the actual limit.

#### Signal B: 429 errors (rate limit exceeded)

When we hit a 429, it means our effective limit was too high for the current conditions (model-specific limits, anti-abuse throttling, shared quota with other clients).

**Algorithm**: After a 429:
1. If we had a learned limit, **halve it** (aggressive reduction)
2. If no learned limit existed, set learned limit to **50% of configured default**
3. Back off as before (exponential/retry-after based)

### Recovery: relaxing limits over time

Limits should slowly **recover** toward the configured default in case the provider loosened restrictions:

- After a **successful request window** (no 429s for a full window period), increase the learned limit by **10%** (multiplicative increase)
- Cap at the configured default — never exceed what we configured
- This implements a classic **AIMD** (Additive Increase / Multiplicative Decrease) pattern, similar to TCP congestion control

### Per-provider-model granularity

Limits are learned at the **provider-model** level, since providers often have different limits per model (e.g. Groq: Llama 3.3 70B = 1K RPD, Llama 3.1 8B = 14.4K RPD).

Key: `provider_instance::model_id` → same keying as existing backoff.

---

## Part 3: Data Model

### Step 3.1: LearnedLimits struct

**File:** `crates/lr-router/src/free_tier.rs`

```rust
/// Adaptively learned rate limits from provider behavior.
/// Layered on top of the configured static defaults.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LearnedLimits {
    /// Learned RPM (requests per minute). None = use configured default.
    pub rpm: Option<u32>,
    /// Learned RPD (requests per day). None = use configured default.
    pub rpd: Option<u32>,
    /// Learned TPM (tokens per minute). None = use configured default.
    pub tpm: Option<u64>,
    /// Learned TPD (tokens per day). None = use configured default.
    pub tpd: Option<u64>,
    /// Source of the learned limits
    pub source: LearnedLimitSource,
    /// When these limits were last updated
    pub updated_at: DateTime<Utc>,
    /// Number of consecutive successful windows (for recovery)
    pub successful_windows: u32,
    /// Number of 429s that led to current limits
    pub total_429_adaptations: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum LearnedLimitSource {
    /// Learned from provider response headers (authoritative)
    ProviderHeaders,
    /// Inferred from 429 errors (estimated)
    BackoffAdaptation,
    /// Combination of both
    Mixed,
}
```

### Step 3.2: Add to FreeTierManager

**File:** `crates/lr-router/src/free_tier.rs`

```rust
pub struct FreeTierManager {
    rate_trackers: DashMap<String, RwLock<RateLimitTracker>>,
    credit_trackers: DashMap<String, RwLock<CreditTracker>>,
    backoffs: DashMap<String, RwLock<ProviderBackoff>>,
    // NEW:
    learned_limits: DashMap<String, RwLock<LearnedLimits>>,  // key: "provider::model"
    persist_path: Option<PathBuf>,
}
```

### Step 3.3: Core methods

```rust
impl FreeTierManager {
    /// Update learned limits from response headers (Signal A).
    /// Called after every successful response.
    pub fn update_learned_limits_from_headers(
        &self,
        provider_instance: &str,
        model: &str,
        headers: &RateLimitHeaderInfo,
        configured: &FreeTierKind,
    ) {
        // If header reports limit < configured, adopt header value
        // If header reports limit >= configured, don't learn (already within expected range)
    }

    /// Reduce learned limits after a 429 error (Signal B).
    /// Called alongside record_rate_limit_error().
    pub fn adapt_limits_on_429(
        &self,
        provider_instance: &str,
        model: &str,
        configured: &FreeTierKind,
    ) {
        // If learned limit exists: halve it
        // If no learned limit: set to 50% of configured default
        // Increment total_429_adaptations
        // Reset successful_windows to 0
    }

    /// Recover learned limits toward configured defaults.
    /// Called periodically (e.g. once per minute window).
    pub fn try_recover_limits(
        &self,
        provider_instance: &str,
        model: &str,
        configured: &FreeTierKind,
    ) {
        // Increment successful_windows
        // If successful_windows >= recovery_threshold (e.g. 5):
        //   Increase learned limit by 10%, capped at configured default
        //   Reset successful_windows to 0
    }

    /// Get effective limits: min(configured, learned).
    /// Returns the FreeTierKind with limits adjusted by learning.
    pub fn get_effective_limits(
        &self,
        provider_instance: &str,
        model: &str,
        configured: &FreeTierKind,
    ) -> FreeTierKind {
        // For each limit dimension (rpm, rpd, tpm, tpd):
        //   effective = min(configured_value, learned_value_or_configured)
        // Return modified FreeTierKind
    }

    /// Clear learned limits for a provider (user-initiated reset)
    pub fn reset_learned_limits(&self, provider_instance: &str) {
        // Remove all entries with prefix "provider_instance::"
    }

    /// Get learned limits info for UI display
    pub fn get_learned_limits(
        &self,
        provider_instance: &str,
        model: &str,
    ) -> Option<LearnedLimits>
}
```

---

## Part 4: Configuration — Per-Provider Enable/Disable

### Step 4.1: Add to ProviderConfig

**File:** `crates/lr-config/src/types.rs` (ProviderConfig struct)

```rust
/// Whether adaptive rate limiting is enabled for this provider.
/// When enabled, the system learns actual rate limits from provider
/// feedback and adjusts effective limits automatically.
/// Default: true (enabled)
#[serde(default = "default_true")]
pub adaptive_rate_limiting: bool,
```

### Step 4.2: Check flag in FreeTierManager

In `update_learned_limits_from_headers()`, `adapt_limits_on_429()`, and `try_recover_limits()` — skip if `adaptive_rate_limiting` is false for the provider.

The router passes the flag when calling these methods:
```rust
if provider_config.adaptive_rate_limiting {
    self.free_tier_manager.update_learned_limits_from_headers(...);
}
```

---

## Part 5: Persistence

### Step 5.1: Extend FreeTierState

**File:** `crates/lr-router/src/free_tier.rs`

```rust
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FreeTierState {
    pub rate_trackers: Vec<(String, RateLimitTracker)>,
    pub credit_trackers: Vec<(String, CreditTracker)>,
    // NEW:
    #[serde(default)]
    pub learned_limits: Vec<(String, LearnedLimits)>,
}
```

Note: `#[serde(default)]` ensures backward compatibility — existing state files without `learned_limits` will deserialize fine (empty vec).

### Step 5.2: Save/load learned limits

In `persist()` and `load()`, include `learned_limits` alongside existing trackers.

---

## Part 6: UI — Free Tier Tab Enhancements

### Step 6.1: Add fields to ProviderFreeTierStatus

**File:** `src-tauri/src/ui/commands_free_tier.rs`

```rust
pub struct ProviderFreeTierStatus {
    // ... existing fields ...

    // NEW: Adaptive rate limiting
    /// Whether adaptive rate limiting is enabled for this provider
    pub adaptive_rate_limiting_enabled: bool,
    /// Whether any limits have been learned (tighter than configured)
    pub has_learned_limits: bool,
    /// Per-model learned limits summary
    pub learned_limits: Vec<LearnedLimitEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LearnedLimitEntry {
    pub model: String,
    pub learned_rpm: Option<u32>,
    pub learned_rpd: Option<u32>,
    pub learned_tpm: Option<u64>,
    pub learned_tpd: Option<u64>,
    pub configured_rpm: u32,
    pub configured_rpd: u32,
    pub configured_tpm: u64,
    pub configured_tpd: u64,
    pub source: String, // "provider_headers" | "backoff_adaptation" | "mixed"
    pub updated_at: String, // ISO 8601
    pub total_adaptations: u32,
}
```

### Step 6.2: Add Tauri commands

**File:** `src-tauri/src/ui/commands_free_tier.rs`

```rust
/// Toggle adaptive rate limiting for a provider
#[tauri::command]
pub async fn set_adaptive_rate_limiting(
    provider_instance: String,
    enabled: bool,
    config_manager: State<'_, ConfigManager>,
    app: tauri::AppHandle,
) -> Result<(), String>

/// Reset learned limits for a provider (back to configured defaults)
#[tauri::command]
pub async fn reset_learned_limits(
    provider_instance: String,
    free_tier_manager: State<'_, Arc<FreeTierManager>>,
) -> Result<(), String>
```

### Step 6.3: TypeScript types

**File:** `src/types/tauri-commands.ts`

Add `LearnedLimitEntry` interface and extend `ProviderFreeTierStatus` with new fields.

### Step 6.4: UI in providers-panel.tsx Free Tier tab

**File:** `src/views/resources/providers-panel.tsx` (Free Tier tab, ~line 987)

Add section after the existing "Usage & Status" card:

1. **Adaptive Rate Limiting toggle** — Switch to enable/disable per provider
2. **Learned Limits table** — When limits have been learned, show:
   - Model name
   - Configured limit → Learned limit (with colored indicator: green = same, yellow = reduced)
   - Source (headers / 429 adaptation)
   - Last updated timestamp
   - Adaptation count
3. **Reset button** — "Reset to configured defaults" clears all learned limits

---

## Part 7: Wire into Router

### Step 7.1: After successful responses

**File:** `crates/lr-router/src/lib.rs`

After `clear_backoff` on success (~line 1030):
```rust
// Update learned limits from response headers
if let Some(ref headers_map) = response.response_headers {
    let headers_info = parse_rate_limit_headers(headers_map);
    self.free_tier_manager.update_from_headers(provider, &headers_info);
    if provider_config.adaptive_rate_limiting {
        self.free_tier_manager.update_learned_limits_from_headers(
            provider, model, &headers_info, &effective_free_tier,
        );
        self.free_tier_manager.try_recover_limits(
            provider, model, &effective_free_tier,
        );
    }
}
```

### Step 7.2: After 429 errors

**File:** `crates/lr-router/src/lib.rs`

After `record_rate_limit_error` (~line 1050):
```rust
if provider_config.adaptive_rate_limiting {
    self.free_tier_manager.adapt_limits_on_429(
        provider, model, &effective_free_tier,
    );
}
```

### Step 7.3: Use effective limits for capacity checks

Wherever `check_rate_limit_capacity` is called, use `get_effective_limits()` to get the learned-adjusted limits instead of raw configured ones.

---

## Algorithm Summary

```
AIMD (Additive Increase / Multiplicative Decrease):

On each successful response:
  1. Read x-ratelimit-limit-* headers → if < configured, adopt as learned limit
  2. Increment successful_windows counter
  3. Every 5 clean windows: increase learned limit by 10% (capped at configured)

On 429 error:
  1. If learned limit exists: halve it (multiplicative decrease)
  2. If no learned limit: set to 50% of configured default
  3. Reset successful_windows to 0
  4. Normal backoff still applies (exponential/retry-after)

Persistence:
  - Learned limits saved to free_tier_state.json alongside existing data
  - Survives app restarts
  - User can reset per-provider via UI

Per-provider control:
  - adaptive_rate_limiting: bool in ProviderConfig (default true)
  - Stored in config.yaml, toggled via UI
```

---

## Critical Files

| File | Change |
|---|---|
| `crates/lr-providers/src/lib.rs` | Add `response_headers` to CompletionResponse |
| `crates/lr-providers/src/...` | Capture HTTP headers in provider `complete()` impls |
| `crates/lr-router/src/free_tier.rs` | LearnedLimits struct, AIMD algorithm, persistence |
| `crates/lr-router/src/lib.rs` | Wire header capture + adaptive calls after success/429 |
| `crates/lr-config/src/types.rs` | `adaptive_rate_limiting` field on ProviderConfig |
| `src-tauri/src/ui/commands_free_tier.rs` | New Tauri commands + status fields |
| `src/types/tauri-commands.ts` | TypeScript types |
| `src/views/resources/providers-panel.tsx` | UI toggle + learned limits display |

## Verification

1. `cargo test -p lr-router` — all tests pass (existing + new adaptive tests)
2. `cargo test -p lr-providers` — provider tests pass with new header field
3. `cargo clippy` — no warnings
4. `npx tsc --noEmit` — TypeScript compiles
5. Manual: connect to Groq free tier, make requests, verify headers are captured and limits learned
6. Manual: exhaust rate limit (get 429), verify limit is halved and recovers over time
7. Manual: toggle adaptive rate limiting off, verify limits stay at configured defaults

---

## Final Steps (mandatory)

1. **Plan Review** — check all steps against implementation
2. **Test Coverage Review** — AIMD algorithm needs thorough unit tests
3. **Bug Hunt** — persistence backward compat, DashMap concurrency, recovery timing
4. **Commit** — stage only modified files
