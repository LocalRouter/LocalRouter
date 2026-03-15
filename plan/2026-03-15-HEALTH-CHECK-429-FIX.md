# Fix Cohere 429 Health Check Feedback Loop + Reduce Frequency + Default Off

## Context

Cohere health check hits `GET /v1/models/command-r` (authenticated, counts against rate limits). Three problems:

1. **Feedback loop**: 429 treated as Unhealthy → recovery task re-checks every 30s → more 429s → stuck
2. **Excessive frequency**: Periodic checks every 10 min = ~4,320 calls/month, exceeding Cohere's 1,000/month trial limit
3. **On by default**: Health checks shouldn't be on by default since they consume API quota from providers' free tiers

## Part 1: Treat 429 (and 5xx) as Degraded across all cloud providers

**Why**: 429 means API is alive + auth valid + temporarily rate limited = `Degraded`, not `Unhealthy`. `Degraded` is excluded from `get_unhealthy_providers()` (health_cache.rs:378), breaking the recovery loop.

Change status classification in each provider's `health_check()`:

```rust
if status.is_success() || status.as_u16() == 404 {
    // Healthy
} else if status.as_u16() == 429 {
    // Degraded — "Rate limited (HTTP 429)"
} else if status.is_server_error() {
    // Degraded — "Server error (HTTP {status})"
} else {
    // Unhealthy — auth failure (401/403) or other client error
}
```

### Files to modify

| File | Method location |
|------|----------------|
| `crates/lr-providers/src/cohere.rs` | ~line 307 |
| `crates/lr-providers/src/openai.rs` | ~line 386 |
| `crates/lr-providers/src/anthropic.rs` | ~line 387 |
| `crates/lr-providers/src/deepinfra.rs` | ~line 194 |
| `crates/lr-providers/src/mistral.rs` | ~line 207 |
| `crates/lr-providers/src/togetherai.rs` | ~line 197 |
| `crates/lr-providers/src/cerebras.rs` | ~line 145 |
| `crates/lr-providers/src/gemini.rs` | ~line 212 |
| `crates/lr-providers/src/perplexity.rs` | ~line 153 |
| `crates/lr-providers/src/openai_compatible.rs` | ~line 215 |

**Groq and xAI**: delegate to `list_models()` which loses HTTP status. Catch `AppError::RateLimitExceeded` → map to `Degraded`.

**OpenRouter**: already handles this correctly. **Local providers** (Ollama, LMStudio): no change needed.

## Part 2: Per-provider health check interval multiplier

**Why**: Even with health checks opt-in, Cohere at 10-min intervals = 4,320 calls/month, exceeding 1,000/month trial limit.

### 2a. Add trait method to `ModelProvider`

**File**: `crates/lr-providers/src/lib.rs` (~line 160)

```rust
fn health_check_interval_multiplier(&self) -> u32 { 1 }
```

### 2b. Override in Cohere provider

**File**: `crates/lr-providers/src/cohere.rs`

```rust
fn health_check_interval_multiplier(&self) -> u32 {
    6 // Every 60 min → ~720 calls/month (within 1,000 limit)
}
```

### 2c. Use multiplier in periodic health check task

**File**: `src-tauri/src/main.rs` (~lines 1300-1367)

Add `HashMap<String, u32>` cycle counter before the loop. Skip provider when `counter % multiplier != 0`.

## Part 3: Health checks off by default + UI warning

### 3a. Change config default

**File**: `crates/lr-config/src/types.rs`

- Change `periodic_enabled` serde default from `default_true` to `default_false` (or just `#[serde(default)]` since bool defaults to false)
- Change `Default` impl for `HealthCheckConfig`: `periodic_enabled: false`

```rust
#[serde(default)]  // was: default = "default_true"
pub periodic_enabled: bool,
```

```rust
impl Default for HealthCheckConfig {
    fn default() -> Self {
        Self {
            mode: HealthCheckMode::default(),
            periodic_enabled: false,  // was: true
            ...
        }
    }
}
```

Note: Existing users who already have `periodic_enabled: true` in their config YAML won't be affected (serde only uses default for missing fields).

### 3b. Add warning note in settings UI

**File**: `src/views/settings/health-checks-tab.tsx`

Add an info/warning note below the checkbox explaining that periodic health checks make API calls to each provider, which may count against free-tier rate limits.

```tsx
<p className="text-xs text-muted-foreground">
  When enabled, provider and MCP server health is checked automatically on a schedule.
</p>
<p className="text-xs text-amber-600 dark:text-amber-400">
  Note: Health checks make API calls to each provider. Some providers count these
  against free-tier rate limits, which may exhaust your quota over time.
</p>
```

## No changes needed

- `health_cache.rs` — `get_unhealthy_providers()` already excludes `Degraded`
- `health.rs` — framework doesn't see HTTP codes
- Recovery task in `main.rs` — already only re-checks `Unhealthy`
- UI health rendering — already renders `Degraded` as yellow with error message

## Verification

1. `cargo test && cargo clippy` — no regressions
2. Fresh config: verify `periodic_enabled` defaults to `false`
3. Settings UI: verify warning note appears near checkbox
4. Manual: enable health checks, configure Cohere, confirm:
   - 429 shows "Degraded - Rate limited" not "Unhealthy"
   - Recovery task does NOT re-check every 30s
   - Cohere checks only every 6th cycle (debug logs)
