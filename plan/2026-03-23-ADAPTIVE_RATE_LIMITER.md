# Plan: Cost Watchdog for Free Tier Providers

## Context

When a free tier expires silently, providers start billing instead of returning 429. We need a simple watchdog that detects this and backs off.

### Core idea

Two hooks on every free-tier request:
- **Before**: Check if this provider is in cost-backoff. If yes, skip it.
- **After**: Check if the request cost money. If yes, flag the provider and enter backoff.

---

## Algorithm: Exponential Backoff with Linear Recovery

State per provider (in-memory + persisted):

```rust
struct CostBackoff {
    /// When the last paid request was detected
    last_trigger: Option<DateTime<Utc>>,
    /// Current backoff duration
    backoff_duration: Duration,
    /// Total times this provider has been flagged (lifetime)
    trigger_count: u32,
}
```

### After hook — request cost money (`cost_usd > 0`)

```
if last_trigger is None:
    // First offense — start conservative
    backoff_duration = 5 minutes
else:
    // Repeat offense — double the backoff (exponential increase)
    backoff_duration = min(backoff_duration * 2, 24 hours)

last_trigger = now
trigger_count += 1
```

### Before hook — should we skip this provider?

```
if last_trigger is None:
    // Never flagged → allow
    return ALLOW

elapsed = now - last_trigger

if elapsed < backoff_duration:
    // Still in backoff window → skip
    return SKIP (retry_after = backoff_duration - elapsed)

// Backoff expired → allow this request as a probe
return ALLOW
```

### After hook — probe request was FREE (`cost_usd == 0`)

```
if last_trigger is Some:
    // Free tier is back! Reduce backoff for next time (linear recovery)
    backoff_duration = max(backoff_duration - 5 minutes, 0)
    if backoff_duration == 0:
        // Fully recovered — clear the flag
        last_trigger = None
```

### Behavior walkthrough

```
Time  Event                    backoff_duration  last_trigger  State
─────────────────────────────────────────────────────────────────────
0:00  Request → costs $0.001   5min              0:00          Flagged
0:01  Before check             —                 —             SKIP (4min left)
0:05  Before check             —                 —             ALLOW (probe)
0:05  Probe → costs $0.002     10min             0:05          Double backoff
0:10  Before check             —                 —             SKIP (5min left)
0:15  Before check             —                 —             ALLOW (probe)
0:15  Probe → free!            5min (10-5)       0:05          Recovering
0:20  Before check             —                 —             ALLOW (probe)
0:20  Probe → free!            0min              None          Fully recovered
```

Key properties:
- **Exponential increase** on repeated paid requests (5m → 10m → 20m → 40m → ... → 24h max)
- **Linear decrease** on successful free probes (-5 minutes per free response)
- **Self-healing**: once free tier returns, we gradually restore full access
- **Probe-based**: after backoff expires, we try ONE request to test if free tier is back
- **No accumulation/budgets**: just watches individual request cost — simple and stateless

---

## Implementation

### Step 1: CostBackoff struct + methods

**File:** `crates/lr-router/src/free_tier.rs`

```rust
/// Cost-based backoff state for a provider.
/// Tracks when a provider started charging and manages exponential backoff
/// with linear recovery.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostBackoff {
    /// When the last paid request was detected (None = never flagged / fully recovered)
    pub last_trigger: Option<DateTime<Utc>>,
    /// Current backoff duration in seconds
    pub backoff_secs: u64,
    /// Total times this provider has been flagged (lifetime counter)
    pub trigger_count: u32,
}

const COST_BACKOFF_INITIAL_SECS: u64 = 300;      // 5 minutes
const COST_BACKOFF_MAX_SECS: u64 = 86400;         // 24 hours
const COST_BACKOFF_RECOVERY_SECS: u64 = 300;       // -5 minutes per free probe
```

Add to `FreeTierManager`:
```rust
cost_backoffs: DashMap<String, RwLock<CostBackoff>>,  // key: provider_instance
```

Methods:
```rust
/// Before hook: check if provider is in cost backoff.
/// Returns None if allowed, Some(retry_after_secs) if should skip.
pub fn check_cost_backoff(&self, provider_instance: &str) -> Option<u64>

/// After hook: a request cost money. Flag the provider.
pub fn record_cost_trigger(&self, provider_instance: &str)

/// After hook: a request was free. Reduce backoff if flagged.
pub fn record_cost_free(&self, provider_instance: &str)

/// Reset cost backoff for a provider (user-initiated).
pub fn reset_cost_backoff(&self, provider_instance: &str)

/// Get cost backoff info for UI display.
pub fn get_cost_backoff(&self, provider_instance: &str) -> Option<CostBackoff>
```

### Step 2: Wire into router

**File:** `crates/lr-router/src/lib.rs`

#### Before hook (in auto-routing model selection loop, ~line 994)

Inside the existing `if strategy.free_tier_only` block:

```rust
// Cost backoff check
if let Some(retry_secs) = self.free_tier_manager.check_cost_backoff(provider) {
    debug!("Skipping {}: cost backoff ({}s remaining)", provider, retry_secs);
    continue; // try next provider
}
```

#### After hook (after `record_usage`, ~line 879)

```rust
if strategy.free_tier_only {
    if cost > 0.0 {
        self.free_tier_manager.record_cost_trigger(provider);
    } else {
        self.free_tier_manager.record_cost_free(provider);
    }
}
```

Same for the streaming path in `wrap_stream_with_usage_tracking`.

#### Direct model requests (~line 1496)

For non-auto-routed requests (user specified a model directly), the before hook should still check but instead of `continue`, return a 429 with retry-after.

### Step 3: Persistence

**File:** `crates/lr-router/src/free_tier.rs`

Extend `FreeTierState`:
```rust
pub struct FreeTierState {
    pub rate_trackers: Vec<(String, RateLimitTracker)>,
    pub credit_trackers: Vec<(String, CreditTracker)>,
    #[serde(default)]
    pub cost_backoffs: Vec<(String, CostBackoff)>,
}
```

Update `persist()` and `load()`.

### Step 4: Tauri commands + UI

**File:** `src-tauri/src/ui/commands_free_tier.rs`

Add to `ProviderFreeTierStatus`:
```rust
pub cost_backoff_active: bool,
pub cost_backoff_retry_after_secs: Option<u64>,
pub cost_backoff_duration_secs: Option<u64>,
pub cost_backoff_last_trigger: Option<String>,  // ISO 8601
pub cost_backoff_trigger_count: u32,
```

Add command:
```rust
#[tauri::command]
pub async fn reset_cost_backoff(
    provider_instance: String,
    free_tier_manager: State<'_, Arc<FreeTierManager>>,
) -> Result<(), String>
```

**File:** `src/types/tauri-commands.ts` — add fields to ProviderFreeTierStatus interface.

**File:** `src/views/resources/providers-panel.tsx` — show cost backoff status in the existing Free Tier tab backoff section (alongside the existing 429 backoff display). Include a "Reset" button.

**File:** `website/src/components/demo/TauriMockSetup.ts` — add mock values.

### Step 5: Register command in main.rs

**File:** `src-tauri/src/main.rs` — add `reset_cost_backoff` to the Tauri command list.

---

## Critical Files

| File | Change |
|---|---|
| `crates/lr-router/src/free_tier.rs` | `CostBackoff` struct, 4 methods, persistence |
| `crates/lr-router/src/lib.rs` | Before/after hooks in auto-route + direct route + streaming |
| `src-tauri/src/ui/commands_free_tier.rs` | Status fields + reset command |
| `src-tauri/src/main.rs` | Register new command |
| `src/types/tauri-commands.ts` | TypeScript fields |
| `src/views/resources/providers-panel.tsx` | Display cost backoff status |
| `website/src/components/demo/TauriMockSetup.ts` | Mock data |

## Verification

1. `cargo test -p lr-router` — existing + new tests
2. `cargo clippy && cargo fmt`
3. `npx tsc --noEmit`
4. Unit tests:
   - `cost > 0` → flags provider, backoff = 5min
   - During backoff → `check_cost_backoff` returns `Some(remaining)`
   - After backoff expires → returns `None` (probe allowed)
   - Probe costs money → backoff doubles (10min)
   - Probe is free → backoff reduces by 5min
   - Multiple free probes → fully recovers to `None`
   - Backoff caps at 24 hours
   - Persistence roundtrip
5. Manual: use a paid provider, verify backoff kicks in and shows in UI

---

## Final Steps (mandatory)

1. **Plan Review** — verify before/after hooks are in all code paths
2. **Test Coverage Review** — edge cases: zero cost, backoff exactly at boundary, recovery to zero
3. **Bug Hunt** — check streaming path, direct model path, concurrent access
4. **Commit** — stage only modified files
