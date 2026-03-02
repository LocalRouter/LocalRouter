# Plan: Add Allow/Ask/Off Permission to Auto Router

## Context

When auto-routing is enabled (`AutoModelConfig.enabled: true`), ALL requests are silently overridden to `localrouter/auto` and ALL model-level permission checks are bypassed (both `validate_model_access` and `check_model_firewall_permission` in `chat.rs` lines 84-97). There is no way for users to be notified or approve auto-routing decisions. This is a security gap since every other permission-controlled feature (MCP tools, skills, models, marketplace, coding agents) supports Allow/Ask/Off granularity.

**Goal:** Replace the `enabled: bool` on `AutoModelConfig` with `PermissionState` (Allow/Ask/Off), integrating with the existing firewall approval system for the "Ask" mode.

---

## Changes

### 1. Config Type: `enabled: bool` → `permission: PermissionState`

**File:** `crates/lr-config/src/types.rs` (lines 229-244)

- Replace `pub enabled: bool` with `pub permission: PermissionState` on `AutoModelConfig`
- Add backward-compatible serde: keep `enabled` as `#[serde(default, skip_serializing)]` `Option<bool>`, add `migrate_enabled_field()` method
- Bump `CONFIG_VERSION` from 16 → 17

**File:** `crates/lr-config/src/migration.rs`

- Add `migrate_to_v17` that calls `auto_config.migrate_enabled_field()` on all strategies

### 2. Access Control: Add `AutoRouter` variant

**File:** `crates/lr-mcp/src/gateway/access_control.rs`

- Add `AutoRouter { permission, has_time_based_approval }` variant to `FirewallCheckContext`
- Add match arm in `check_needs_approval()`: Allow→Allow, Off→Deny, Ask→check time-based then Ask

### 3. Firewall: Add auto-router approval request

**File:** `crates/lr-mcp/src/gateway/firewall.rs`

- Add `is_auto_router_request: bool` to `FirewallApprovalSession` and `PendingApprovalInfo` (default `false`)
- Add `request_auto_router_approval()` method on `FirewallManager` (reuses `request_approval_internal` with `tool_name: "localrouter/auto"`, `server_name: "Auto Router"`, prioritized models as preview)
- Pass `is_auto_router_request` through `request_approval_internal`

### 4. Time-Based Approval Tracker

**File:** `crates/lr-server/src/state.rs`

- Add `AutoRouterApprovalTracker` (clone of `FreeTierApprovalTracker` pattern: `DashMap<String, Instant>` keyed by client_id)
- Methods: `has_valid_approval()`, `add_1_minute_approval()`, `add_1_hour_approval()`, `cleanup_expired()`
- Add `auto_router_approval_tracker: Arc<AutoRouterApprovalTracker>` to `AppState` and init in `AppState::new()`

### 5. Chat Route Enforcement (primary enforcement point)

**File:** `crates/lr-server/src/routes/chat.rs` (lines 69-82)

Replace the boolean check:
```rust
// BEFORE:
if auto_config.enabled { request.model = "localrouter/auto" }

// AFTER:
match auto_config.permission {
    Allow => override model to localrouter/auto,
    Ask => check time-based tracker → if expired, call request_auto_router_approval() → if approved, override; if denied, return 403,
    Off => skip (use client's requested model),
}
```

The existing `localrouter/auto` skip conditions on lines 84-97 remain correct (the model is only set to `localrouter/auto` after approval).

### 6. Other Backend `auto_config.enabled` References

| File | Line(s) | Change |
|------|---------|--------|
| `crates/lr-router/src/lib.rs` | 903, 1071, 1712 | `auto_config.enabled` → `auto_config.permission.is_enabled()` |
| `crates/lr-server/src/routes/models.rs` | 47, 149 | Same |
| `src-tauri/src/ui/tray_menu.rs` | 351 | Same |

### 7. Submit Approval Handler

**File:** `src-tauri/src/ui/commands_clients.rs`

In `submit_firewall_approval`, add `is_auto_router_request` handling:
- `AllowPermanent` → update strategy `auto_config.permission` to `Allow`
- `DenyAlways` → update strategy `auto_config.permission` to `Off`
- `Allow1Minute` / `Allow1Hour` → add to `auto_router_approval_tracker`
- Add `update_auto_router_permission()` helper (finds client's strategy, updates `auto_config.permission`)

In `reevaluate_pending_approvals`, add `AutoRouter` context for `is_auto_router_request` entries.

### 8. Frontend: PermissionStateButton in Auto Router Config

**File:** `src/components/strategy/StrategyModelConfiguration.tsx`

- In the "Auto Router Configuration" card (line 535), add `PermissionStateButton` in the card header (Allow/Ask/Off)
- Update `routingMode` derivation: `auto_config?.permission !== 'off'` instead of `auto_config?.enabled`
- Update `handleModeChange`: switching to auto sets `permission: 'allow'`, switching to allowed sets `permission: 'off'`

### 9. Frontend: TypeScript Types

**File:** `src/types/tauri-commands.ts`

- Fix stale `AutoModelConfig` interface: replace with `{ permission: PermissionState, model_name: string, prioritized_models: [string, string][], available_models: [string, string][], routellm_config?: RouteLLMConfig }`

### 10. Frontend: Firewall Popup

**File:** `src/components/shared/FirewallApprovalCard.tsx`

- Add `"auto_router"` to `RequestType` union
- Handle in `getRequestType()`: check `is_auto_router_request`
- Add header content: icon=Bot, title="Auto Router", description="Auto-routing will select a model for this request"

**File:** `src/views/firewall-approval.tsx`

- Pass `is_auto_router_request` from pending approval info to the card component

### 11. Other Frontend References

| File | Change |
|------|--------|
| `src/views/settings/routing-tab.tsx:417` | `auto_config.enabled` → `auto_config.permission !== 'off'` |
| `website/src/components/demo/TauriMockSetup.ts` | Update mock `auto_config` to use `permission` field |

### 12. Tests

- Config migration v17 test (`crates/lr-config/src/migration.rs`)
- Backward compat deserialization: `{"enabled": true}` → Allow, `{"enabled": false}` → Off
- `AutoRouterApprovalTracker` unit test (`crates/lr-server/src/state.rs`)
- `FirewallCheckContext::AutoRouter` test cases (`crates/lr-mcp/src/gateway/access_control.rs`)
- Update `src-tauri/tests/router_routellm_integration_tests.rs:78` (`auto_config.enabled` → `permission`)

---

## Verification

1. `cargo test` — all tests pass including new migration and access control tests
2. `cargo clippy` — no warnings
3. `npx tsc --noEmit` — frontend types check
4. Manual: Create a client with auto-routing set to "Ask", send a chat completion, verify popup appears
5. Manual: Approve with "Allow 1 Hour", verify subsequent requests auto-route without popup
6. Manual: Set to "Off", verify requests use the client's specified model
7. Manual: Set to "Allow", verify silent auto-routing (current behavior)
8. Manual: Verify existing configs with `enabled: true/false` migrate correctly on app start
