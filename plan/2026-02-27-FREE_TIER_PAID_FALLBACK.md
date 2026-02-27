# Free-Tier Paid Fallback

## Context

Free-Tier Mode restricts a strategy to only use free/free-tier models. Currently, when all free-tier models are exhausted, the request simply fails. The user wants a configurable fallback: **Off** (return 429), **Ask** (popup for approval), or **Allow** (auto-proceed with paid models).

This requires:
1. A new config field on Strategy
2. A new `FirewallCheckContext` variant following the unified `check_needs_approval` pattern
3. Router-level free-tier filtering + fallback signaling
4. Server-level interception of the fallback signal + popup triggering
5. A new approval tracker for time-based approvals
6. Frontend UI for the fallback setting + popup rendering

**Important**: The free-tier filtering in the router is not yet implemented. The `free_tier_only` field exists on `Strategy` but the router doesn't use it. This plan implements both the filtering AND the fallback in one go.

---

## 1. Config: Add `FreeTierFallback` enum

**File: `crates/lr-config/src/types.rs`**

Add enum after `FreeTierKind` (around line 270):
```rust
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FreeTierFallback {
    #[default]
    Off,
    Ask,
    Allow,
}
```

Add field to `Strategy` struct (after `free_tier_only`):
```rust
#[serde(default)]
pub free_tier_fallback: FreeTierFallback,
```

Update `Strategy::new()` and `Strategy::new_for_client()` to include `free_tier_fallback: FreeTierFallback::default()`.

---

## 2. Unified Check: Add `FreeTierFallback` variant to `FirewallCheckContext`

**File: `crates/lr-mcp/src/gateway/access_control.rs`**

Add new variant to `FirewallCheckContext`:
```rust
FreeTierFallback {
    fallback_mode: FreeTierFallback, // from lr_config
    has_time_based_approval: bool,
},
```

Add match arm in `check_needs_approval`:
```rust
FirewallCheckContext::FreeTierFallback {
    fallback_mode,
    has_time_based_approval,
} => {
    match fallback_mode {
        FreeTierFallback::Off => FirewallCheckResult::Deny,
        FreeTierFallback::Allow => FirewallCheckResult::Allow,
        FreeTierFallback::Ask => {
            if *has_time_based_approval {
                FirewallCheckResult::Allow
            } else {
                FirewallCheckResult::Ask
            }
        }
    }
}
```

This follows the same single-function pattern used by MCP tools, models, skills, and guardrails. The re-evaluation in `reevaluate_pending_approvals` will also use this.

Add unit tests for the new variant.

---

## 3. Firewall: Add `is_free_tier_fallback` flag

**File: `crates/lr-mcp/src/gateway/firewall.rs`**

Add `is_free_tier_fallback: bool` to:
- `FirewallApprovalSession` struct
- `PendingApprovalInfo` struct (with `#[serde(default)]`)
- `request_approval_internal()` signature
- SSE notification params

Add new public method:
```rust
pub async fn request_free_tier_fallback_approval(
    &self,
    client_id: String,
    client_name: String,
    exhausted_summary: String,
    retry_after_secs: u64,
) -> AppResult<FirewallApprovalResponse>
```

This calls `request_approval_internal` with `is_free_tier_fallback: true`, using the exhausted models summary as `arguments_preview`.

Update all existing callers of `request_approval_internal` to pass `is_free_tier_fallback: false`.

---

## 4. Approval Tracker

**File: `crates/lr-server/src/state.rs`**

Add `FreeTierApprovalTracker` (same pattern as `ModelApprovalTracker` but keyed only by `client_id`):
```rust
pub struct FreeTierApprovalTracker {
    approvals: Arc<DashMap<String, Instant>>,  // client_id → expiry
}
```

Methods: `has_valid_approval(client_id)`, `add_1_hour_approval(client_id)`, `cleanup_expired()`.

Add to `AppState`: `pub free_tier_approval_tracker: Arc<FreeTierApprovalTracker>`.

---

## 5. Router: Free-tier filtering + fallback signal

**File: `src-tauri/src/router/mod.rs`**

The Router needs a reference to `FreeTierManager`. It's already passed in `main.rs` but not stored. Add `free_tier_manager: Arc<FreeTierManager>` to the `Router` struct and `Router::new()`.

**In `complete_with_auto_routing()`**: When `strategy.free_tier_only` is true, filter `selected_models` by free-tier status using `free_tier_manager.classify_model()`. If all filtered models are exhausted after trying:
- `FreeTierFallback::Off` → return `AppError::FreeTierExhausted`
- `FreeTierFallback::Ask` or `Allow` → return a new error `AppError::FreeTierFallbackAvailable { retry_after_secs, exhausted_models }`

**Add `AppError::FreeTierFallbackAvailable`** in `crates/lr-types/src/errors.rs`.

**Add `complete_with_paid_fallback()` and `stream_complete_with_paid_fallback()`**: Clone the strategy with `free_tier_only = false`, then delegate to the normal routing methods.

Apply the same pattern in `stream_complete_with_auto_routing()`.

**Specific model requests**: When `free_tier_only` is true and a specific paid model is requested (not `localrouter/auto`), check if the model is free using `free_tier_manager.classify_model()`. If not free (or free-tier exhausted), apply the same fallback logic: Off → block, Ask/Allow → return `FreeTierFallbackAvailable`. This applies in `complete()` and `stream_complete()` for non-auto requests.

---

## 6. Server: Catch fallback + trigger popup

**File: `crates/lr-server/src/routes/chat.rs`**

Add a helper function following the unified `check_needs_approval` pattern:
```rust
async fn check_free_tier_fallback(
    state: &AppState,
    client_id: &str,
    strategy: &Strategy,
    exhausted_models: &[(String, String)],
    retry_after_secs: u64,
) -> ApiResult<()>
```

This function:
1. Builds `FirewallCheckContext::FreeTierFallback` with `fallback_mode` and `has_time_based_approval` from `free_tier_approval_tracker`
2. Calls `check_needs_approval()`
3. On `Allow` → return Ok (caller proceeds with paid fallback)
4. On `Deny` → return 429 error
5. On `Ask` → call `firewall_manager.request_free_tier_fallback_approval()`, then handle the response (AllowOnce → Ok, Allow1Hour → add to tracker + Ok, Deny → 429)

In the main request handlers (`handle_non_streaming`, `handle_streaming`, etc.), catch `AppError::FreeTierFallbackAvailable` from the router, call `check_free_tier_fallback()`, and if approved, re-invoke the router via `complete_with_paid_fallback()`.

---

## 7. Re-evaluation: Update `reevaluate_pending_approvals`

**File: `src-tauri/src/ui/commands_clients.rs`**

In `reevaluate_pending_approvals()`:
- Accept `free_tier_approval_tracker` parameter
- Add a branch for `info.is_free_tier_fallback`:
  ```rust
  } else if info.is_free_tier_fallback {
      // Look up strategy from client to get fallback_mode
      let strategy = config.strategies.iter().find(|s| s.id == client.strategy_id);
      if let Some(strategy) = strategy {
          FirewallCheckContext::FreeTierFallback {
              fallback_mode: &strategy.free_tier_fallback,
              has_time_based_approval: free_tier_approval_tracker
                  .has_valid_approval(&info.client_id),
          }
      }
  }
  ```
- In `submit_firewall_approval`, handle `Allow1Hour` for free-tier fallback → add to `free_tier_approval_tracker`
- Resource-identity matching already works via `is_free_tier_fallback` comparison on `PendingApprovalInfo`

Update the `same_resource` check to include `is_free_tier_fallback`:
```rust
&& info.is_free_tier_fallback == submitted.is_free_tier_fallback;
```

---

## 8. Tauri Command: Accept `free_tier_fallback`

**File: `src-tauri/src/ui/commands_clients.rs`**

In `update_strategy` (line 492), add parameter:
```rust
free_tier_fallback: Option<lr_config::FreeTierFallback>,
```

And in the update closure:
```rust
if let Some(fallback) = free_tier_fallback {
    strategy.free_tier_fallback = fallback;
}
```

---

## 9. TypeScript Types

**File: `src/types/tauri-commands.ts`**

```typescript
export type FreeTierFallback = 'off' | 'ask' | 'allow'

// Update Strategy interface:
export interface Strategy {
  // ... existing fields ...
  free_tier_fallback?: FreeTierFallback
}

// Update UpdateStrategyParams:
export interface UpdateStrategyParams {
  // ... existing fields ...
  freeTierFallback?: FreeTierFallback | null
}
```

---

## 10. Frontend UI: Fallback selector

**File: `src/views/clients/tabs/models-tab.tsx`**

When `free_tier_only` is enabled, show a sub-option in the card. Add `CardContent` with a `ToggleGroup` (Off/Ask/Allow):

```tsx
{currentStrategy?.free_tier_only && (
  <CardContent className="pt-0">
    <div className="border-t pt-3">
      <div className="flex items-center justify-between">
        <div>
          <span className="text-sm font-medium">Paid Fallback</span>
          <p className="text-xs text-muted-foreground mt-0.5">
            What to do when free-tier usage is depleted
          </p>
        </div>
        <ToggleGroup type="single" value={...} onValueChange={...}>
          <ToggleGroupItem value="off">Off</ToggleGroupItem>
          <ToggleGroupItem value="ask">Ask</ToggleGroupItem>
          <ToggleGroupItem value="allow">Allow</ToggleGroupItem>
        </ToggleGroup>
      </div>
    </div>
  </CardContent>
)}
```

Add `handleFreeTierFallbackChange` callback (same pattern as `handleFreeTierToggle`).

No `ToggleGroup` component exists. Use inline buttons with `variant="outline"` / `variant="default"` for active state, grouped in a `div` with `flex gap-1` — lightweight and no new component needed. Alternatively, could use `RadioGroup` from `src/components/ui/radio-group.tsx` but that's more verbose for 3 short options.

---

## 11. Popup: New request type `free_tier_fallback`

**File: `src/components/shared/FirewallApprovalCard.tsx`**

- Add `"free_tier_fallback"` to `RequestType` union
- Update `getRequestType()`: check `is_free_tier_fallback` first
- Add header content:
  ```typescript
  case "free_tier_fallback":
    return {
      icon: <Coins className="h-5 w-5 text-amber-500" />,
      title: "Free Tier Exhausted",
      description: "All free-tier models are at capacity. Proceed with paid models?",
    }
  ```
- Simplify button options for this type: only Deny, Allow Once, Allow 1 Hour (no session/permanent/edit)

**File: `src/views/firewall-approval.tsx`**

- Add `is_free_tier_fallback?: boolean` to `ApprovalDetails` interface
- Window sizing: use a smaller size (~400x280) for free-tier fallback
- No edit mode for this type

---

## 12. Demo Mock

**File: `website/src/components/demo/TauriMockSetup.ts`**

Update `update_strategy` mock to handle `freeTierFallback`.

---

## 13. Tests

**Files:**
- `crates/lr-mcp/src/gateway/access_control.rs` — unit tests for `FreeTierFallback` variant
- `src-tauri/tests/router_strategy_tests.rs` — update test fixtures with `free_tier_fallback` field

---

## Implementation Order

1. Config types (`lr-config`) — enum + field
2. Error types (`lr-types`) — `FreeTierFallbackAvailable`
3. Access control (`access_control.rs`) — new `FirewallCheckContext` variant + tests
4. Firewall (`firewall.rs`) — `is_free_tier_fallback` flag + new approval method
5. Approval tracker (`state.rs`) — `FreeTierApprovalTracker`
6. Router (`router/mod.rs`) — free-tier filtering + fallback signal + paid fallback methods
7. Server handler (`chat.rs`) — catch fallback + trigger popup
8. Re-evaluation (`commands_clients.rs`) — handle free-tier in `reevaluate_pending_approvals` + `submit_firewall_approval`
9. Tauri command (`commands_clients.rs`) — accept `freeTierFallback` param
10. TypeScript types
11. Frontend UI (models-tab)
12. Popup components (FirewallApprovalCard + firewall-approval)
13. Demo mock
14. Test fixture updates

## Verification

1. `cargo test` — all tests pass including new access_control tests
2. `cargo clippy` — no warnings
3. `npx tsc --noEmit` — TypeScript compiles
4. Manual: enable Free-Tier Mode → set fallback to "Ask" → make a request that exhausts free-tier → verify popup appears → approve → verify paid model is used
5. Manual: verify "Allow 1 Hour" skips popup on subsequent requests
6. Manual: verify multiple simultaneous popups auto-resolve via `reevaluate_pending_approvals`
