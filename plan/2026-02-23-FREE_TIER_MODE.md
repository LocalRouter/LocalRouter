# Free Tier Mode - Implementation Plan

## Context

Users want to add many LLM providers/models to the auto-router and have it use only free-tier resources until they're exhausted. Currently, there's no concept of "free tier" in the system. This feature adds:
- **Provider-level free tier tracking** (shared across all clients) with per-provider configuration
- **Per-strategy budget mode** that restricts routing to free resources only
- **Approval popup** when free tier is exhausted (like the existing guardrail popup)

The core challenge: providers have fundamentally different free tier models (rate-limited vs credit-based vs local vs none). The design must handle all of these with a common abstraction.

---

## Provider Research Summary

### Provider Classification

| Provider | Free Tier Type | Free Tier Details | Credit/Balance API? | Rate Limit Headers? |
|----------|---------------|-------------------|--------------------|--------------------|
| **Ollama** | Always Free (local) | No external billing | N/A | N/A |
| **LM Studio** | Always Free (local) | No external billing | N/A | N/A |
| **OpenAI Compatible** | Always Free (local default) | Self-hosted, configurable | N/A | Varies |
| **OpenRouter** | Credit-based + free models | Per-key credits, `is_free_tier` flag, 32 free `:free` models | **YES**: `GET /api/v1/key` (same key) → `usage`, `limit_remaining`, `is_free_tier` | Yes |
| **Google Gemini** | Rate-limited free | Per-model RPM/TPM/RPD limits, all models accessible, no credit card needed | **NO** (no headers on 200, only on 429) | **NO on success**, only on 429 errors |
| **Groq** | Rate-limited free | Per-model RPM/TPM/TPD limits, no credit card needed, no expiry | **NO** | **YES**: `x-ratelimit-limit-requests`, `x-ratelimit-remaining-requests`, `x-ratelimit-limit-tokens`, `x-ratelimit-remaining-tokens` |
| **Cerebras** | Rate-limited free + credits | 1M tokens/day free, 8192 context on free, credits expire 1yr | **NO** | **YES**: `x-ratelimit-limit-requests-day`, `x-ratelimit-remaining-requests-day`, `x-ratelimit-limit-tokens-minute`, `x-ratelimit-remaining-tokens-minute` |
| **xAI** | Credit-based | $25 promo (expires 30d) + $150/mo data sharing opt-in | **YES**: Management API (separate key) `GET /v1/billing/teams/{id}/prepaid/balance` | **YES**: `x-ratelimit-limit-requests`, `x-ratelimit-remaining-requests` |
| **Mistral** | Rate-limited free | "Experiment" plan: 1 RPS, 500K TPM, 1B tokens/month, all models, no credit card. **Data training risk** | **NO** | Not documented |
| **Cohere** | Rate-limited free (trial key) | Trial key: 1,000 calls/month, 20 RPM chat, 100K TPM. All models accessible | **NO** | Not documented |
| **Together AI** | One free model only | No free tier (min $5 purchase). One free model: `Llama-3.3-70B-Instruct-Turbo-Free` at 0.6 RPM | **NO** | **YES**: `x-ratelimit-limit`, `x-ratelimit-remaining`, `x-tokenlimit-limit`, `x-tokenlimit-remaining` |
| **DeepInfra** | Credit-based | $5/month free credits, no credit card needed | **NO** | Not documented |
| **Perplexity** | Credit-based (Pro only) | $5/month if Pro subscriber ($20/mo plan). Prepaid credits, 402 when exhausted | **NO** | Not documented |
| **OpenAI** | None | No API free tier. Usage/Costs API exists (same key, org perms) | Usage API only (not balance) | **YES**: `x-ratelimit-*` standard headers |
| **Anthropic** | None | No API free tier. Admin API (separate `sk-ant-admin` key) | Admin API only (separate key) | **YES**: `anthropic-ratelimit-*` headers |
| **GitHub Copilot** | Subscription | Included in GitHub Copilot subscription | No | Varies |

### Detailed Per-Provider Free Tier Limits

**Gemini free tier (per-model, resets daily at midnight PT):**

| Model | RPM | TPM | RPD |
|-------|-----|---------|-----|
| Gemini 2.5 Pro | 5 | 250,000 | 100 |
| Gemini 2.5 Flash | 10 | 250,000 | 250 |
| Gemini 2.5 Flash-Lite | 15 | 250,000 | 1,000 |
| Gemma 3 | 30 | 15,000 | 14,400 |

**Groq free tier (per-model, no expiry):**

| Model | RPM | RPD | TPM | TPD |
|-------|-----|-------|-------|---------|
| Llama 3.3 70B | 30 | 14,400 | 6,000 | 100,000-500,000 |
| Llama 3.1 8B | 30 | 14,400 | 6,000 | ~500,000 |

**Cerebras free tier (per-model, 1M TPD total):**

| Model | RPM | RPD | TPM |
|-------|-----|-------|-------|
| GPT OSS 120B | 30 | 14,400 | 64,000 |
| Llama 3.1 8B | 30 | 14,400 | 60,000 |

**Mistral free tier ("Experiment" plan):**
- All models: 1 RPS (~60 RPM), 500K TPM, 1B tokens/month

**Cohere free tier (Trial key):**
- Chat: 20 RPM, 100K TPM, 1,000 calls/month total

### Key Detection Mechanisms

| Provider | How to detect free tier exhaustion | HTTP Code |
|----------|-----------------------------------|-----------|
| OpenRouter | `GET /api/v1/key` → `is_free_tier`, `limit_remaining` | 402 |
| Gemini | 429 with `quota_metric` in body (e.g. `generate_content_daily_requests`) | 429 |
| Groq | `x-ratelimit-remaining-requests` header → 0 | 429 |
| Cerebras | `x-ratelimit-remaining-requests-day` → 0; credits → 402 | 429 / 402 |
| Mistral | 429 with `"Requests rate limit exceeded"` | 429 |
| Cohere | 429 with `"You are using a Trial key..."` message | 429 |
| Together AI | `x-ratelimit-remaining` header → 0 | 429 / 503 |
| DeepInfra | Credits exhausted (behavior undocumented) | 429 |
| Perplexity | Credits exhausted | 401 / 402 |
| xAI | Credits exhausted; Management API for balance | 429 |

---

## Architecture

### Core Abstraction: `FreeTierKind`

The key insight: providers have fundamentally different free tier models. A single `budget_usd` doesn't work for rate-limited providers like Gemini/Groq. We need a discriminated union:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FreeTierKind {
    /// No known free tier. Always treated as paid.
    None,

    /// Local / self-hosted. Always free, no limits from provider.
    AlwaysFreeLocal,

    /// Subscription-based. Free within existing subscription.
    Subscription,

    /// Rate-limited free access (RPM/RPD/TPM) but no dollar credits.
    /// Used by: Gemini, Groq, Cerebras, Mistral, Cohere
    RateLimitedFree {
        /// Max requests per minute (0 = not tracked)
        max_rpm: u32,
        /// Max requests per day (0 = not tracked)
        max_rpd: u32,
        /// Max tokens per minute (0 = not tracked)
        max_tpm: u64,
        /// Max tokens per day (0 = not tracked)
        max_tpd: u64,
        /// Monthly call limit (0 = not tracked, Cohere: 1000)
        max_monthly_calls: u32,
        /// Monthly token limit (0 = not tracked, Mistral: 1B)
        max_monthly_tokens: u64,
    },

    /// Credit-based free tier (e.g. OpenRouter, xAI, DeepInfra, Perplexity)
    CreditBased {
        /// Budget in USD
        budget_usd: f64,
        /// Reset period
        reset_period: FreeTierResetPeriod,
        /// How credits are tracked
        detection: CreditDetection,
    },

    /// Specific free models only (e.g. Together AI free model, OpenRouter :free models)
    FreeModelsOnly {
        /// Model ID patterns that are free (e.g. ["*:free", "meta-llama/Llama-3.3-70B-Instruct-Turbo-Free"])
        free_model_patterns: Vec<String>,
        /// Rate limits on free models
        max_rpm: u32,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FreeTierResetPeriod {
    Daily,
    Monthly,
    Never, // one-time credits
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CreditDetection {
    /// All accounting is local (Together, DeepInfra, Perplexity, startup grants)
    LocalOnly,
    /// Use provider's built-in API (OpenRouter `/api/v1/key`)
    ProviderApi,
    /// Custom HTTP endpoint for checking credits
    CustomEndpoint {
        url: String,
        method: String, // GET or POST
        /// Headers with {{API_KEY}} template support
        headers: Vec<(String, String)>,
        /// JSONPath-like dotted paths to extract credit info from response
        remaining_credits_path: Option<String>,
        total_credits_path: Option<String>,
        is_free_tier_path: Option<String>,
    },
}
```

### Provider Default Mapping

Each provider factory returns its default `FreeTierKind`:

| Provider | Default `FreeTierKind` |
|----------|----------------------|
| Ollama | `AlwaysFreeLocal` |
| LM Studio | `AlwaysFreeLocal` |
| OpenAI Compatible | `AlwaysFreeLocal` |
| GitHub Copilot | `Subscription` |
| **Gemini** | `RateLimitedFree { max_rpm: 10, max_rpd: 250, max_tpm: 250_000, max_tpd: 0, max_monthly_calls: 0, max_monthly_tokens: 0 }` (Flash defaults) |
| **Groq** | `RateLimitedFree { max_rpm: 30, max_rpd: 14_400, max_tpm: 6_000, max_tpd: 500_000, max_monthly_calls: 0, max_monthly_tokens: 0 }` |
| **Cerebras** | `RateLimitedFree { max_rpm: 30, max_rpd: 14_400, max_tpm: 60_000, max_tpd: 1_000_000, max_monthly_calls: 0, max_monthly_tokens: 0 }` |
| **Mistral** | `RateLimitedFree { max_rpm: 60, max_rpd: 0, max_tpm: 500_000, max_tpd: 0, max_monthly_calls: 0, max_monthly_tokens: 1_000_000_000 }` |
| **Cohere** | `RateLimitedFree { max_rpm: 20, max_rpd: 0, max_tpm: 100_000, max_tpd: 0, max_monthly_calls: 1_000, max_monthly_tokens: 0 }` |
| **OpenRouter** | `CreditBased { budget_usd: 0.0, reset_period: Never, detection: ProviderApi }` (auto-detected) |
| **xAI** | `CreditBased { budget_usd: 25.0, reset_period: Never, detection: LocalOnly }` (promo expires 30d) |
| **DeepInfra** | `CreditBased { budget_usd: 5.0, reset_period: Monthly, detection: LocalOnly }` |
| **Perplexity** | `None` (free credits only if Pro subscriber - user must configure) |
| **Together AI** | `FreeModelsOnly { free_model_patterns: ["meta-llama/Llama-3.3-70B-Instruct-Turbo-Free"], max_rpm: 3 }` |
| **OpenAI** | `None` |
| **Anthropic** | `None` |

All defaults are overridable by the user per provider instance.

### Strategy-Level: "Free-Tier Only" Toggle

Simple boolean on Strategy (not an enum):
```rust
/// When true, the router only uses free-tier models/providers.
/// When all free providers are exhausted, returns 429 with retry-after.
#[serde(default)]
pub free_tier_only: bool,
```

No popup/Ask mode. When exhausted → 429 to the caller with `retry-after` = minimum across all backed-off providers.

### Design Principle: Minimize Per-Provider Code

**No custom per-provider code for handling free tiers.** The system works through:

1. **Config-only provider setup**: Each provider declares its `FreeTierKind` as data (via `default_free_tier()` on the factory). No provider-specific Rust logic needed beyond that declaration.

2. **Universal rate limit header parser**: A single parser tries ALL known header formats on every response and extracts whatever is available. No per-provider `parse_rate_limit_headers()` method.

3. **Generic handling per FreeTierKind variant**: The FreeTierManager has one code path per `FreeTierKind` variant (not per provider). A custom provider configured as `RateLimitedFree` automatically inherits all header parsing, backoff, and tracking.

4. **Only truly unique APIs get custom code**: Currently only OpenRouter's `check_credits()` endpoint (`GET /api/v1/key`). Everything else is handled generically.

### Universal Rate Limit Header Parser

A single function that tries all known header naming conventions:

```rust
pub fn parse_rate_limit_headers(headers: &HeaderMap) -> RateLimitHeaderInfo {
    // Try all known header patterns, return whatever we find:
    //
    // Standard (OpenAI, Groq, xAI):
    //   x-ratelimit-limit-requests, x-ratelimit-remaining-requests, x-ratelimit-reset-requests
    //   x-ratelimit-limit-tokens, x-ratelimit-remaining-tokens, x-ratelimit-reset-tokens
    //
    // Cerebras (daily variant):
    //   x-ratelimit-limit-requests-day, x-ratelimit-remaining-requests-day
    //   x-ratelimit-limit-tokens-minute, x-ratelimit-remaining-tokens-minute
    //
    // Together AI (short form):
    //   x-ratelimit-limit, x-ratelimit-remaining, x-ratelimit-reset
    //   x-tokenlimit-limit, x-tokenlimit-remaining
    //
    // Anthropic:
    //   anthropic-ratelimit-requests-limit, anthropic-ratelimit-requests-remaining
    //   anthropic-ratelimit-tokens-limit, anthropic-ratelimit-tokens-remaining
    //
    // Universal:
    //   retry-after (seconds or HTTP-date)
    //   retry-after-ms (milliseconds)
}
```

This means a custom OpenAI-compatible provider will automatically benefit from header parsing if its backend returns standard `x-ratelimit-*` headers. Zero configuration needed.

### Two Tracking Systems

**1. Rate Limit Tracker** (for `RateLimitedFree` providers):
- Uses universal header parser output (when headers available)
- Falls back to client-side counters (RPM/RPD/TPM/TPD/monthly) when no headers
- Resets: RPM per minute, RPD at midnight (PT for Gemini, UTC for others), monthly on 1st

**2. Credit Tracker** (for `CreditBased` providers):
- Tracks `current_cost_usd` per provider per period
- For `ProviderApi` (OpenRouter): periodically calls `check_credits()` to sync
- For `LocalOnly`: estimates cost from token usage + catalog pricing
- For `CustomEndpoint`: calls configured endpoint
- Period resets: Daily/Monthly/Never

**3. Backoff Tracker** (for ALL providers, always active):
- In-memory tracking of 429/402 errors per provider-model
- Uses `retry-after` header when available, exponential backoff otherwise
- Allows the router to skip providers it already knows are unavailable
- When returning 429 to caller: `retry-after` = min(all provider backoff remaining times)

---

## Phase 1: Data Model Changes

### File: `crates/lr-config/src/types.rs`

**New types** (add after existing rate limit types):
- `FreeTierKind` enum (as defined in Architecture)
- `FreeTierResetPeriod` enum
- `CreditDetection` enum

**Modify `Strategy`** (line ~250): Add field:
```rust
/// When true, the router only uses free-tier models/providers.
#[serde(default)]
pub free_tier_only: bool,
```

**Modify `ProviderConfig`** (line ~1955): Add field:
```rust
/// Free tier configuration for this provider instance.
/// If None, uses the provider type's default free tier.
#[serde(default, skip_serializing_if = "Option::is_none")]
pub free_tier: Option<FreeTierKind>,
```

**Config migration**: Bump `CONFIG_VERSION` to 14, add no-op `migrate_to_v14`.

### File: `crates/lr-config/src/migration.rs`
- Add `migrate_to_v14` (no-op, new fields have defaults)

---

## Phase 2: Provider Changes (Minimal Per-Provider Code)

### Goal: Each provider only needs to declare its `FreeTierKind` — no custom handling code

### File: `crates/lr-providers/src/lib.rs`

Add to `ModelProvider` trait (only one method — credit checking for providers with APIs):
```rust
/// Check remaining credits/balance with this provider's API (if supported).
/// Only OpenRouter implements this today. Other providers return None.
async fn check_credits(&self) -> Option<ProviderCreditsInfo> { None }
```

New struct:
```rust
pub struct ProviderCreditsInfo {
    pub total_credits_usd: Option<f64>,
    pub used_credits_usd: Option<f64>,
    pub remaining_credits_usd: Option<f64>,
    pub is_free_tier: Option<bool>,
}
```

**NO `parse_rate_limit_headers()` on the provider trait.** Header parsing is universal (see Phase 3).

### File: `crates/lr-providers/src/factory.rs`

Add to `ProviderFactory` trait:
```rust
/// Default free tier configuration for this provider type.
/// This is the ONLY thing each provider needs to specify for free tier support.
fn default_free_tier(&self) -> FreeTierKind { FreeTierKind::None }
```

### Per-provider changes (config-only, ~1-5 lines each)

Each provider file gets ONE small change: override `default_free_tier()` on its factory. No custom handling code.

```rust
// Example: groq.rs factory
fn default_free_tier(&self) -> FreeTierKind {
    FreeTierKind::RateLimitedFree {
        max_rpm: 30, max_rpd: 14_400, max_tpm: 6_000,
        max_tpd: 500_000, max_monthly_calls: 0, max_monthly_tokens: 0,
    }
}
```

**Only OpenRouter gets custom code** (implements `check_credits()` to call `GET /api/v1/key`).

### File: `crates/lr-router/src/free_tier.rs` (NEW) — Universal Rate Limit Header Parser

A single standalone function (not on any provider trait):

```rust
/// Parse rate limit info from ANY provider's response headers.
/// Tries all known header naming conventions.
pub fn parse_rate_limit_headers(headers: &HeaderMap) -> RateLimitHeaderInfo {
    let mut info = RateLimitHeaderInfo::default();

    // Try standard format: x-ratelimit-remaining-requests (OpenAI, Groq, xAI)
    info.requests_remaining = try_parse(headers, "x-ratelimit-remaining-requests");
    info.requests_limit = try_parse(headers, "x-ratelimit-limit-requests");
    info.requests_reset = try_parse_duration(headers, "x-ratelimit-reset-requests");
    info.tokens_remaining = try_parse(headers, "x-ratelimit-remaining-tokens");
    info.tokens_limit = try_parse(headers, "x-ratelimit-limit-tokens");
    info.tokens_reset = try_parse_duration(headers, "x-ratelimit-reset-tokens");

    // Try daily variant: x-ratelimit-remaining-requests-day (Cerebras)
    if info.daily_requests_remaining.is_none() {
        info.daily_requests_remaining = try_parse(headers, "x-ratelimit-remaining-requests-day");
        info.daily_requests_limit = try_parse(headers, "x-ratelimit-limit-requests-day");
    }

    // Try short form: x-ratelimit-remaining (Together AI)
    if info.requests_remaining.is_none() {
        info.requests_remaining = try_parse(headers, "x-ratelimit-remaining");
        info.requests_limit = try_parse(headers, "x-ratelimit-limit");
    }

    // Try token-specific short form: x-tokenlimit-remaining (Together AI)
    if info.tokens_remaining.is_none() {
        info.tokens_remaining = try_parse(headers, "x-tokenlimit-remaining");
        info.tokens_limit = try_parse(headers, "x-tokenlimit-limit");
    }

    // Try Anthropic format: anthropic-ratelimit-requests-remaining
    if info.requests_remaining.is_none() {
        info.requests_remaining = try_parse(headers, "anthropic-ratelimit-requests-remaining");
        info.requests_limit = try_parse(headers, "anthropic-ratelimit-requests-limit");
        info.tokens_remaining = try_parse(headers, "anthropic-ratelimit-tokens-remaining");
        info.tokens_limit = try_parse(headers, "anthropic-ratelimit-tokens-limit");
    }

    // Universal: retry-after (seconds or HTTP-date)
    info.retry_after_secs = try_parse(headers, "retry-after")
        .or_else(|| try_parse::<u64>(headers, "retry-after-ms").map(|ms| ms / 1000));

    info
}
```

This means:
- A custom OpenAI-compatible provider that returns `x-ratelimit-*` headers → automatically parsed
- A provider with Anthropic-style headers → automatically parsed
- A provider with no headers → falls back to client-side counting + exponential backoff
- **Zero per-provider code needed for header parsing**

### Files to modify
- `crates/lr-providers/src/lib.rs` - Add `check_credits()` to trait, `ProviderCreditsInfo` struct
- `crates/lr-providers/src/factory.rs` - Add `default_free_tier()` to factory trait
- `crates/lr-providers/src/openrouter.rs` - Implement `check_credits()` (only custom code)
- All other provider files (~15 files) - Override `default_free_tier()` (~1-5 lines each, config only)

---

## Phase 3: Free Tier Manager (`crates/lr-router/src/free_tier.rs` - NEW)

### FreeTierManager

Central manager that handles both rate-limit tracking and credit tracking per provider instance. Shared across all clients.

```rust
pub struct FreeTierManager {
    /// Per-provider rate limit tracking (for RateLimitedFree providers)
    rate_trackers: DashMap<String, RwLock<RateLimitTracker>>,
    /// Per-provider credit tracking (for CreditBased providers)
    credit_trackers: DashMap<String, RwLock<CreditTracker>>,
    /// Persistence path
    persist_path: Option<PathBuf>,
}

/// Tracks rate-limited free tier usage per provider
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
    pub header_requests_remaining: Option<u32>,
    pub header_tokens_remaining: Option<u64>,
    pub header_daily_requests_remaining: Option<u32>,
    pub header_updated_at: Option<DateTime<Utc>>,
}

/// Tracks credit-based free tier usage per provider
pub struct CreditTracker {
    pub current_cost_usd: f64,
    pub period_start: DateTime<Utc>,
    /// Last known balance from provider API
    pub api_remaining_usd: Option<f64>,
    pub api_is_free_tier: Option<bool>,
    pub api_last_checked: Option<DateTime<Utc>>,
    /// Usage events for debugging
    pub events: VecDeque<CreditEvent>,
}
```

### Key Methods

```rust
impl FreeTierManager {
    // === Classification ===

    /// Determine if a model/provider combination is free
    fn classify_model(
        &self,
        provider_instance: &str,
        provider_type: &ProviderType,
        model: &str,
        pricing: &PricingInfo,
        free_tier: &FreeTierKind,
    ) -> ModelFreeStatus;

    // === Rate Limit Tracking ===

    /// Check if rate-limited free tier has capacity
    fn check_rate_limit_capacity(
        &self, provider_instance: &str, free_tier: &FreeTierKind
    ) -> FreeTierCapacity;

    /// Update rate limit tracking from response headers
    fn update_from_headers(
        &self, provider_instance: &str, headers: &RateLimitHeaderInfo
    );

    /// Record a request for rate limit tracking
    fn record_rate_limit_usage(
        &self, provider_instance: &str, tokens: u64
    );

    // === Credit Tracking ===

    /// Check credit-based free tier remaining balance
    fn check_credit_balance(
        &self, provider_instance: &str, free_tier: &FreeTierKind
    ) -> FreeTierCapacity;

    /// Record cost for credit tracking
    fn record_credit_usage(
        &self, provider_instance: &str, cost_usd: f64, model: &str, client_id: &str
    );

    /// Sync with provider API (for ProviderApi detection)
    async fn sync_credits_from_api(
        &self, provider_instance: &str, provider: &dyn ModelProvider
    ) -> Option<ProviderCreditsInfo>;

    // === General ===

    /// Get overall free tier status for a provider
    fn get_provider_status(
        &self, provider_instance: &str, free_tier: &FreeTierKind
    ) -> ProviderFreeTierStatus;

    /// Get all provider statuses (for UI)
    fn get_all_statuses(&self, config: &AppConfig) -> Vec<ProviderFreeTierStatus>;

    /// Reset usage for a provider
    fn reset_usage(&self, provider_instance: &str);

    /// Persist/load state
    fn persist(&self) -> AppResult<()>;
    fn load(path: &Path) -> AppResult<Self>;
}
```

### ModelFreeStatus

```rust
pub enum ModelFreeStatus {
    /// Always free: local provider, subscription, or $0 pricing
    AlwaysFree,
    /// Free within provider's rate limits or credit budget
    FreeWithinLimits,
    /// Free model specifically (FreeModelsOnly pattern match)
    FreeModel,
    /// Not free: no free tier or exhausted
    NotFree,
    /// Rate limited: still on free tier but approaching/at limits
    RateLimitApproaching { remaining_pct: f32 },
}
```

### FreeTierCapacity

```rust
pub struct FreeTierCapacity {
    pub has_capacity: bool,
    /// For rate-limited: % of limits remaining
    pub remaining_pct: Option<f32>,
    /// For credit-based: USD remaining
    pub remaining_usd: Option<f64>,
    /// Human-readable status
    pub status_message: String,
}
```

### Classification Logic

1. `FreeTierKind::AlwaysFreeLocal` or `Subscription` → `AlwaysFree`
2. `FreeTierKind::None` → `NotFree`
3. `FreeTierKind::FreeModelsOnly` → check if model matches patterns → `FreeModel` or `NotFree`
4. `FreeTierKind::RateLimitedFree` → check rate tracker capacity → `FreeWithinLimits` or `NotFree`
5. `FreeTierKind::CreditBased` → check credit tracker balance → `FreeWithinLimits` or `NotFree`
6. Fallback: If model pricing is $0/$0 → `AlwaysFree`

### Rate Limit Header Integration

After every successful API response, the router calls:
```rust
if let Some(header_info) = provider.parse_rate_limit_headers(&response_headers) {
    free_tier_manager.update_from_headers(provider_instance, &header_info);
}
```

This updates the `RateLimitTracker.header_*` fields, providing accurate remaining capacity from the provider itself. When headers are available, they take precedence over client-side counters.

### Provider Backoff Tracking

When a provider returns 429 (rate limited) or 402 (credits exhausted), we must avoid retrying it on subsequent requests until the backoff period expires. Without this, every incoming request wastes time hitting rate-limited providers in sequence (A→429, B→429, C→429) when we already know A and B are unavailable.

**Backoff state per provider-model pair** (stored in-memory on FreeTierManager):

```rust
/// Tracks backoff state for a provider after 429/402 errors
pub struct ProviderBackoff {
    /// When the provider becomes available again (None = available now)
    pub available_at: Option<Instant>,
    /// Current backoff duration for exponential backoff
    pub current_backoff: Duration,
    /// Number of consecutive 429/402 errors
    pub consecutive_errors: u32,
    /// Whether the provider is in credit-exhausted state (longer backoff)
    pub is_credit_exhausted: bool,
    /// Source of the backoff timing
    pub backoff_source: BackoffSource,
}

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
```

**Backoff resolution priority** (from most to least accurate):
1. `retry-after` or `retry-after-ms` response header → use exact value
2. `x-ratelimit-reset-*` header → calculate duration until reset time
3. Gemini 429 body `quota_metric` → infer reset time (RPM=60s, RPD=midnight PT)
4. Credit exhaustion with known replenishment → use period reset time
5. Exponential backoff fallback: 1s, 2s, 4s, 8s, 16s, 32s, 60s max (with jitter)

**Key methods on FreeTierManager:**

```rust
/// Record a 429/402 error and compute backoff
fn record_rate_limit_error(
    &self,
    provider_instance: &str,
    model: &str,
    status_code: u16,
    retry_after_secs: Option<u64>,
    rate_limit_reset_secs: Option<u64>,
    is_credit_exhaustion: bool,
    credit_replenish_at: Option<DateTime<Utc>>,
);

/// Check if a provider is currently in backoff (should be skipped)
fn is_in_backoff(&self, provider_instance: &str, model: &str) -> Option<BackoffInfo>;

/// Clear backoff after a successful request
fn clear_backoff(&self, provider_instance: &str, model: &str);
```

```rust
pub struct BackoffInfo {
    /// When the provider becomes available
    pub available_at: Instant,
    /// Seconds until available
    pub retry_after_secs: u64,
    /// Why it's backed off
    pub reason: String, // e.g. "rate limited (retry in 45s)", "credits exhausted (resets Feb 24)"
}
```

**Router integration**: Before attempting each provider in the auto-routing loop, check backoff state:

```rust
// BEFORE free tier classification check:
if let Some(backoff) = self.free_tier_manager.is_in_backoff(provider, model) {
    debug!("Skipping {}/{}: {}", provider, model, backoff.reason);
    last_error = Some(RouterError::RateLimited {
        provider: provider.clone(),
        model: model.clone(),
        retry_after_secs: backoff.retry_after_secs,
    });
    continue; // Skip to next model immediately
}
```

**After error responses**: Parse headers and update backoff:

```rust
Err(e) => {
    let router_error = RouterError::classify(&e, provider, model);
    if matches!(router_error, RouterError::RateLimited { .. }) {
        // Extract retry-after from response headers
        let retry_after = parse_retry_after(&response_headers);
        let reset_time = parse_rate_limit_reset(&response_headers);
        let is_credit = matches!(status_code, 402);
        self.free_tier_manager.record_rate_limit_error(
            provider, model, status_code,
            retry_after, reset_time, is_credit, None,
        );
    }
}
```

**After successful responses**: Clear backoff for that provider-model:

```rust
Ok(response) => {
    self.free_tier_manager.clear_backoff(provider, model);
    // ... rest of success handling
}
```

**Credit exhaustion (402) specific behavior**:
- If credit replenishment time is known (from `FreeTierKind::CreditBased` period): backoff until that time
- If unknown: exponential backoff with much longer intervals (5min, 15min, 1hr, 6hr, 24hr)
- Mark `is_credit_exhausted = true` so the UI can show "credits exhausted" vs "rate limited"

**Persistence**: Backoff state is in-memory only (not persisted to disk). On restart, all backoffs reset to zero - this is fine since rate limits also reset or the provider may have recovered.

### Files
- `crates/lr-router/src/free_tier.rs` - NEW: All tracking logic including backoff
- `crates/lr-router/src/lib.rs` - Add `pub mod free_tier;` and re-export

---

## Phase 4: Router Integration

### File: `crates/lr-router/src/lib.rs`

**Modify `Router` struct** (line ~263):
```rust
pub struct Router {
    config_manager: Arc<ConfigManager>,
    provider_registry: Arc<ProviderRegistry>,
    rate_limiter: Arc<RateLimiterManager>,
    metrics_collector: Arc<lr_monitoring::metrics::MetricsCollector>,
    routellm_service: Option<Arc<lr_routellm::RouteLLMService>>,
    free_tier_manager: Arc<FreeTierManager>,  // NEW
}
```

### Auto-routing filter

In `complete_with_auto_routing` (line ~769) and `stream_complete_with_auto_routing` (line ~842), inside the model iteration loop, before each attempt:

```rust
// 1. BACKOFF CHECK (always applies, even in Unlimited mode)
//    Skip providers that recently returned 429/402
if let Some(backoff) = self.free_tier_manager.is_in_backoff(provider, model) {
    debug!("Skipping {}/{}: {}", provider, model, backoff.reason);
    // Track smallest retry_after across all backed-off providers
    min_retry_after = min_retry_after.min(backoff.retry_after_secs);
    continue;
}

// 2. FREE TIER CHECK (only when free_tier_only is enabled)
if strategy.free_tier_only {
    let free_tier = self.get_effective_free_tier(provider);
    let pricing = /* get from provider */;
    let status = self.free_tier_manager.classify_model(
        provider, provider_type, model, &pricing, &free_tier
    );
    match status {
        ModelFreeStatus::AlwaysFree
        | ModelFreeStatus::FreeWithinLimits
        | ModelFreeStatus::FreeModel => { /* allow */ }
        ModelFreeStatus::NotFree => {
            debug!("Skipping {}/{} in free-tier-only mode", provider, model);
            continue;
        }
    }
}
```

### When ALL models exhausted → return 429 with smart retry-after

When the loop exhausts all candidates and `free_tier_only` is enabled:

```rust
// All providers exhausted. Return 429 with retry-after = min backoff across all providers.
// This tells the caller when the soonest provider becomes available.
let retry_after = self.free_tier_manager
    .get_min_retry_after(&selected_models)
    .unwrap_or(60); // default 60s if no info available

return Err(AppError::FreeTierExhausted { retry_after_secs: retry_after });
```

The `AppError::FreeTierExhausted` maps to HTTP 429 with `retry-after` header in the Axum response handler.

### Specific model requests (non-auto)

In `complete()` (line ~1102) and `stream_complete()` (line ~1179), after model validation:
- Check backoff state → if backed off, return 429 with retry-after
- If `free_tier_only` → check free tier classification → if `NotFree`, return 429

### Post-response tracking

**On successful response** (in `execute_request` or stream wrapper):
```rust
// Clear backoff
self.free_tier_manager.clear_backoff(provider_name, model);

// Parse rate limit headers (universal parser — works for any provider)
let header_info = free_tier::parse_rate_limit_headers(&response.headers);
self.free_tier_manager.update_from_headers(provider_name, &header_info);

// Record usage for tracking
self.free_tier_manager.record_rate_limit_usage(provider_name, total_tokens);
let cost = calculate_cost(input_tokens, output_tokens, &pricing);
if cost > 0.0 {
    self.free_tier_manager.record_credit_usage(provider_name, cost, model, client_id);
}
```

**On error response (429/401/402):**
```rust
Err(e) => {
    let router_error = RouterError::classify(&e, provider, model);
    if router_error.should_retry() {
        // Universal header parsing for retry-after
        let header_info = free_tier::parse_rate_limit_headers(&error_headers);
        let status_code = extract_status_code(&e);
        let is_credit = matches!(status_code, Some(401 | 402));

        self.free_tier_manager.record_rate_limit_error(
            provider, model, status_code.unwrap_or(429),
            header_info.retry_after_secs,
            header_info.requests_reset_secs,
            is_credit,
        );
    }
    // ... existing fallback logic continues
}
```

### Helper: get_effective_free_tier

```rust
fn get_effective_free_tier(&self, provider_instance: &str) -> FreeTierKind {
    let config = self.config_manager.get();
    // User override on provider config takes priority
    config.providers.iter()
        .find(|p| p.name == provider_instance)
        .and_then(|p| p.free_tier.clone())
        .unwrap_or_else(|| {
            // Fall back to factory default
            self.provider_registry.get_factory_for_instance(provider_instance)
                .map(|f| f.default_free_tier())
                .unwrap_or(FreeTierKind::None)
        })
}
```

### File: `crates/lr-types/src/lib.rs`
- Add `AppError::FreeTierExhausted { retry_after_secs: u64 }` variant

### File: `src-tauri/src/server/routes/` (completion handlers)
- Map `AppError::FreeTierExhausted` → HTTP 429 with `retry-after` header

---

## Phase 5: Tauri Commands

### New file: `src-tauri/src/ui/commands_free_tier.rs`

```rust
/// Get free tier status for all configured providers
#[tauri::command]
pub async fn get_free_tier_status(...) -> Result<Vec<ProviderFreeTierStatus>, String>

/// Update free tier config for a provider (user override)
#[tauri::command]
pub async fn set_provider_free_tier(
    provider_instance: String,
    free_tier: Option<FreeTierKind>, // None to reset to default
    ...
) -> Result<(), String>

/// Reset free tier usage counters for a provider
#[tauri::command]
pub async fn reset_provider_free_tier_usage(provider_instance: String, ...) -> Result<(), String>

/// Trigger credit check for a provider (calls check_credits() API)
#[tauri::command]
pub async fn check_provider_credits(provider_instance: String, ...) -> Result<Option<ProviderCreditsInfo>, String>

/// Get the default free tier config for a provider type
#[tauri::command]
pub async fn get_default_free_tier(provider_type: String, ...) -> Result<FreeTierKind, String>
```

### Response type for UI
```rust
pub struct ProviderFreeTierStatus {
    pub provider_instance: String,
    pub provider_type: String,
    pub display_name: String,
    pub free_tier: FreeTierKind,        // Effective config (user override or default)
    pub is_user_override: bool,
    pub supports_credit_check: bool,    // Has check_credits() API
    // Rate-limited status:
    pub rate_rpm_used: Option<u32>,
    pub rate_rpm_limit: Option<u32>,
    pub rate_rpd_used: Option<u32>,
    pub rate_rpd_limit: Option<u32>,
    pub rate_tpm_used: Option<u64>,
    pub rate_tpm_limit: Option<u64>,
    pub rate_monthly_calls_used: Option<u32>,
    pub rate_monthly_calls_limit: Option<u32>,
    // Credit-based status:
    pub credit_used_usd: Option<f64>,
    pub credit_budget_usd: Option<f64>,
    pub credit_remaining_usd: Option<f64>,
    pub credit_resets_at: Option<String>,
    // Backoff status:
    pub is_backed_off: bool,
    pub backoff_retry_after_secs: Option<u64>,
    pub backoff_reason: Option<String>,
    // Summary:
    pub has_capacity: bool,
    pub status_message: String,
}
```

### Modify existing commands

**`update_strategy`** in `commands_clients.rs`: Accept `free_tier_only: Option<bool>`.

### Files
- `src-tauri/src/ui/commands_free_tier.rs` - NEW
- `src-tauri/src/ui/commands_clients.rs` - Add `free_tier_only` param
- `src-tauri/src/ui/mod.rs` - Register module
- `src-tauri/src/main.rs` - Register Tauri commands

---

## Phase 6: Frontend

### TypeScript types (`src/types/tauri-commands.ts`)

```typescript
export type FreeTierResetPeriod = 'daily' | 'monthly' | 'never';
export type CreditDetectionType = 'local_only' | 'provider_api' | 'custom_endpoint';

export type FreeTierKind =
  | { kind: 'none' }
  | { kind: 'always_free_local' }
  | { kind: 'subscription' }
  | { kind: 'rate_limited_free'; maxRpm: number; maxRpd: number; maxTpm: number; maxTpd: number; maxMonthlyCalls: number; maxMonthlyTokens: number }
  | { kind: 'credit_based'; budgetUsd: number; resetPeriod: FreeTierResetPeriod; detection: CreditDetection }
  | { kind: 'free_models_only'; freeModelPatterns: string[]; maxRpm: number };

// ... ProviderFreeTierStatus matching Rust type
```

### Strategy settings — "Free-Tier Only" toggle

In the client strategy settings (alongside rate limits):
- **Toggle**: "Free-Tier Only" (default: off)
- When enabled: show compact summary of provider free tier statuses
- Brief explanation: "Only routes to free-tier models and providers. Returns 429 when all free resources are exhausted."

### Provider settings — Free Tier tab

Each provider's settings panel gets a **"Free Tier"** tab showing the default handling with override ability:

**For `AlwaysFreeLocal`**: Badge: "Always Free (Local)"

**For `RateLimitedFree`**: Shows default rate limits (from factory):
- RPM limit, RPD limit, TPM limit, TPD limit, Monthly calls, Monthly tokens
- Current usage bars for each active limit (live from FreeTierManager)
- "Last updated from headers: X seconds ago"
- Override toggle → editable fields

**For `CreditBased`**: Shows credit budget:
- Budget ($), Period (Daily/Monthly/One-time), Detection method
- Usage bar: "$X.XX / $Y.YY used"
- "Check Now" button (if ProviderApi/CustomEndpoint)
- Override toggle → editable fields

**For `FreeModelsOnly`**: Shows free model patterns + RPM limit
- Override toggle → editable patterns

**For `None`**: "No Free Tier" with override toggle → user can set any FreeTierKind

**For `Subscription`**: "Included in Subscription"

All show: current backoff status if backed off ("Rate limited, available in 45s")

### Demo mock
- `website/src/components/demo/TauriMockSetup.ts` - Mock handlers

---

## Phase 7: Wiring

### File: `crates/lr-server/src/state.rs`
Add `pub free_tier_manager: Arc<FreeTierManager>` to `AppState` (line ~479).

### File: `src-tauri/src/main.rs`
- Create `FreeTierManager` (load persisted state from data dir)
- Pass to `Router::new()`
- Pass to `AppState`
- Start background persistence task
- Start periodic credit sync task (for ProviderApi providers, every 5 minutes)
- Register new Tauri commands

---

## Implementation Order

1. **Phase 1**: Config types + migration
2. **Phase 2**: Provider changes (factory `default_free_tier()` for all providers + OpenRouter `check_credits()`)
3. **Phase 3**: FreeTierManager (universal header parser, rate/credit/backoff trackers, persistence, unit tests)
4. **Phase 4**: Router integration (backoff check, free tier filter, 429 response, header tracking)
5. **Phase 5**: Tauri commands
6. **Phase 6**: Frontend (provider free tier tab, strategy toggle)
7. **Phase 7**: Wiring (AppState, main.rs, background tasks)

---

## Verification

1. **Unit tests**: Universal header parser (all formats), FreeTierManager classification, rate tracking with window resets, credit tracking with period resets, backoff with exponential fallback, persistence round-trip
2. **Integration tests**: Router auto-routing with backoff skipping, free tier filter, 429 with correct retry-after
3. **Manual testing**:
   - Add Ollama (local) → "Always Free" badge
   - Add OpenRouter → auto-detection via `/api/v1/key`
   - Add Groq → verify standard headers parsed, RPM/TPD counters track
   - Add Gemini → client-side RPD counter (no headers from provider)
   - Enable "Free-Tier Only" on strategy with mixed providers
   - Exhaust Groq RPD → verify skipped, falls back to Ollama
   - Exhaust all providers → 429 response with correct `retry-after`
   - Verify backoff tracking: after 429, next request skips provider immediately
   - Two clients in free mode sharing same provider pool
   - Custom OpenAI-compatible provider configured as `RateLimitedFree` → inherits all handling
4. **Build**: `cargo test && cargo clippy && cargo fmt && npx tsc --noEmit`
