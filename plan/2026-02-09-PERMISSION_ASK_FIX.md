# Fix Permission "Ask" State & Centralize Access Control

## Context

The UI shows Allow/Ask/Off buttons for MCP servers, Skills, Models, and Marketplace. These set `PermissionState` values on the client config (via `McpPermissions`, `SkillsPermissions`, `ModelPermissions`, `marketplace_permission`).

**The bug:** Setting "Ask" has no effect for MCP, Skills, or Marketplace. The backend only checks `is_enabled()` which returns `true` for both Allow and Ask - they're indistinguishable. No approval popup appears.

**Root cause:** There are two parallel permission systems:
1. **PermissionState** (Allow/Ask/Off) - what the UI controls. Has full hierarchical resolution (tool -> server -> global). Has `requires_approval()` method but it's never called for MCP/Skills/Marketplace.
2. **FirewallRules** (FirewallPolicy: Allow/Ask/Deny) - has NO UI. The Tauri commands exist but no frontend component calls them. Always defaults to Allow. This is the only system where "Ask" actually triggers a popup.

`FirewallRules` is dead code from the user's perspective. The `PermissionState` system already has the same per-server/tool/skill granularity. The fix: wire up `PermissionState` enforcement and deprecate `FirewallRules`.

**Working reference:** Models correctly implement Ask in `routes/chat.rs:398-499` via `check_model_firewall_permission()`, which checks `model_permissions.resolve_model()` and triggers `firewall_manager.request_model_approval()` when Ask.

## Plan

### Step 1: Create centralized access control module

**New file:** `crates/lr-mcp/src/gateway/access_control.rs`

Simple module that resolves `PermissionState` for any resource type and returns a decision:

```rust
/// Resolve PermissionState into an access decision
pub enum AccessDecision { Allow, Ask, Deny }

impl From<&PermissionState> for AccessDecision {
    fn from(p: &PermissionState) -> Self {
        match p {
            PermissionState::Allow => AccessDecision::Allow,
            PermissionState::Ask => AccessDecision::Ask,
            PermissionState::Off => AccessDecision::Deny,
        }
    }
}

/// Check access for an MCP tool call using mcp_permissions hierarchy
pub fn check_mcp_tool_access(perms: &McpPermissions, server_id: &str, tool_name: &str) -> AccessDecision
/// Check access for a skill tool call using skills_permissions hierarchy
pub fn check_skill_tool_access(perms: &SkillsPermissions, skill_name: &str, tool_name: &str) -> AccessDecision
/// Check access for marketplace
pub fn check_marketplace_access(perm: &PermissionState) -> AccessDecision
/// Check access for a model
pub fn check_model_access(perms: &ModelPermissions, provider: &str, model_id: &str) -> AccessDecision
```

Each function calls the existing `resolve_*()` methods on the permission structs and converts to `AccessDecision`. Unit tests for all cases.

Register in `crates/lr-mcp/src/gateway/mod.rs`.

### Step 2: Fix MCP tool "Ask" enforcement

**File:** `crates/lr-mcp/src/gateway/gateway_tools.rs`

Modify `check_firewall_mcp_tool()` (~line 331):
- Currently: only checks `firewall_rules.resolve_mcp_tool()` (always Allow since no UI sets it)
- Change to: call `access_control::check_mcp_tool_access(session.mcp_permissions, server_id, tool_name)`
- If `AccessDecision::Ask` and not already session-approved, trigger existing `firewall_manager.request_approval()` popup
- If `AccessDecision::Deny`, return error response

Rename `apply_firewall_policy()` -> `apply_access_decision()`, accepting `&AccessDecision` instead of `&FirewallPolicy`. The internal logic (Allow->proceed, Ask->popup, Deny->error) stays identical.

### Step 3: Fix Skill tool "Ask" enforcement

**File:** `crates/lr-mcp/src/gateway/gateway_tools.rs`

Modify `check_firewall_skill_tool()` (~line 364):
- Currently: only checks `firewall_rules.resolve_skill_tool()` (always Allow)
- Change to: call `access_control::check_skill_tool_access(session.skills_permissions, skill_name, tool_name)`
- Same popup flow via `apply_access_decision()`

### Step 4: Fix Marketplace "Ask" enforcement

**File:** `crates/lr-mcp/src/gateway/gateway_tools.rs`

Modify `handle_marketplace_tool_call()` (~line 818):
- Currently: only checks `marketplace_permission.is_enabled()` (Ask treated same as Allow)
- Change to: call `access_control::check_marketplace_access(session.marketplace_permission)`
- If `AccessDecision::Ask`, route through `apply_access_decision()` to trigger approval popup before proceeding
- If `AccessDecision::Deny`, return error (existing behavior for Off)
- If `AccessDecision::Allow`, proceed as before

### Step 5: Unify model permission check (consistency)

**File:** `crates/lr-server/src/routes/chat.rs`

Refactor `validate_client_provider_access()` and `check_model_firewall_permission()` to use `access_control::check_model_access()` for the resolution step. Already works correctly - this is purely for consistency.

### Step 6: Deprecate FirewallRules

- Remove `FirewallRules` usage from `gateway_tools.rs` (no longer read from session)
- Keep the `firewall_rules` field in client config for backwards compatibility (with `#[serde(default)]`) but stop using it
- Remove `firewall_rules` from `GatewaySession` struct
- Remove the unused Tauri commands (`set_client_firewall_rule`, `set_client_default_firewall_policy`, `get_client_firewall_rules`) and their TypeScript types
- Remove `to_firewall_policy()` from `PermissionState`
- Keep `FirewallManager` (it's the popup mechanism, not the rules)

## Files to modify

| File | Change |
|------|--------|
| `crates/lr-mcp/src/gateway/access_control.rs` | **NEW** - AccessDecision, check functions, unit tests |
| `crates/lr-mcp/src/gateway/mod.rs` | Add `pub mod access_control;` |
| `crates/lr-mcp/src/gateway/gateway_tools.rs` | Replace FirewallRules checks with access_control checks, rename apply_firewall_policy |
| `crates/lr-mcp/src/gateway/session.rs` | Remove `firewall_rules` field from GatewaySession |
| `crates/lr-mcp/src/gateway/gateway.rs` | Stop passing firewall_rules into session |
| `crates/lr-server/src/routes/chat.rs` | Use access_control for model checks (optional consistency) |
| `crates/lr-server/src/routes/mcp.rs` | Remove firewall_rules from handle_request_with_skills call |
| `src-tauri/src/ui/commands_clients.rs` | Remove firewall Tauri commands |
| `src/types/tauri-commands.ts` | Remove firewall TypeScript types |
| `website/src/components/demo/TauriMockSetup.ts` | Remove firewall mock handlers |

## Files NOT modified

- `crates/lr-config/src/types.rs` - Keep FirewallRules/FirewallPolicy structs (serde backwards compat), remove `to_firewall_policy()` from PermissionState
- Frontend permission components - Already correct (they set PermissionState)
- `middleware/auth_layer.rs`, `middleware/client_auth.rs` - Auth stays separate

## Verification

1. `cargo test` - Unit tests for access_control module + ensure nothing breaks
2. `cargo clippy && cargo fmt`
3. Manual: Set MCP global to "Ask" -> make tool call -> verify approval popup
4. Manual: Set a specific MCP tool to "Ask" -> call that tool -> verify popup
5. Manual: Set skill to "Ask" -> call skill tool -> verify popup
6. Manual: Set marketplace to "Ask" -> use marketplace tool -> verify popup
7. Manual: Set MCP server to "Off" -> verify tools not visible
8. Manual: Verify models still work with Ask (regression)
