# Migration Plan: Remove Old Permission Fields

## Overview

Remove the old permission system fields and exclusively use the new hierarchical permission system (Allow/Ask/Off).

### Fields to Remove (OLD):
- `allowed_llm_providers: Vec<String>` → replaced by `model_permissions`
- `mcp_server_access: McpServerAccess` → replaced by `mcp_permissions`
- `skills_access: SkillsAccess` → replaced by `skills_permissions`
- `marketplace_enabled: bool` → replaced by `marketplace_permission`

### Fields to Keep (NEW):
- `model_permissions: ModelPermissions` (global/providers/models hierarchy)
- `mcp_permissions: McpPermissions` (global/servers/tools/resources/prompts hierarchy)
- `skills_permissions: SkillsPermissions` (global/skills/tools hierarchy)
- `marketplace_permission: PermissionState`

---

## Phase 1: Config Migration (v5 → v6)

### File: `crates/lr-config/src/migration.rs`
Add `migrate_to_v6()` function that converts old fields to new:

| Old Value | New Value |
|-----------|-----------|
| `allowed_llm_providers: []` | `model_permissions.global: Off` |
| `allowed_llm_providers: [p1, p2]` | `global: Off`, `providers: {p1: Allow, p2: Allow}` |
| `mcp_server_access: None` | `mcp_permissions.global: Off` |
| `mcp_server_access: All` | `mcp_permissions.global: Allow` |
| `mcp_server_access: Specific([s1])` | `global: Off`, `servers: {s1: Allow}` |
| `skills_access: None` | `skills_permissions.global: Off` |
| `skills_access: All` | `skills_permissions.global: Allow` |
| `skills_access: Specific([sk1])` | `global: Off`, `skills: {sk1: Allow}` |
| `marketplace_enabled: false` | `marketplace_permission: Off` |
| `marketplace_enabled: true` | `marketplace_permission: Allow` |

---

## Phase 2: Backend Access Check Updates

### File: `crates/lr-server/src/routes/chat.rs`
- Update `validate_client_provider_access()` (lines 318-393)
- Replace `client.allowed_llm_providers` check with:
  ```rust
  let state = client.model_permissions.resolve_provider(&provider);
  if !state.is_enabled() {
      return Err(...)
  }
  ```

### File: `crates/lr-server/src/routes/completions.rs`
- Same update as chat.rs (lines 410-462)

### File: `crates/lr-server/src/routes/embeddings.rs`
- Same update as chat.rs (lines 325-377)

### File: `crates/lr-server/src/routes/mcp.rs`
- Update lines 164-176 and 443-458
- Replace `mcp_server_access.has_any_access()` and `mcp_server_access` matching with:
  ```rust
  let state = client.mcp_permissions.resolve_server(&server_id);
  if !state.is_enabled() { ... }
  ```

### File: `crates/lr-server/src/routes/mcp_ws.rs`
- Update lines 80-96 similarly

### File: `crates/lr-mcp/src/gateway/gateway.rs`
- Update `handle_request_with_skills()` signature (line 306)
- Remove `skills_access` and `marketplace_enabled` parameters
- Pass `skills_permissions` and `marketplace_permission` instead
- Update session assignment (lines 337-354)

### File: `crates/lr-mcp/src/gateway/session.rs`
- Update `GatewaySession` struct fields
- Replace `skills_access: SkillsAccess` with `skills_permissions: SkillsPermissions`
- Replace `marketplace_enabled: bool` with `marketplace_permission: PermissionState`

---

## Phase 3: Remove Old Tauri Commands & UI Types

### File: `src-tauri/src/ui/commands_clients.rs`

**Remove enums (lines 18-81):**
- `McpAccessMode`
- `SkillsAccessMode`

**Remove conversion functions (lines 83-99):**
- `skills_access_to_ui()`
- `mcp_access_to_ui()`

**Remove from ClientInfo struct (lines 47-56):**
- `allowed_llm_providers`
- `mcp_access_mode`
- `mcp_servers`
- `skills_access_mode`
- `skills_names`

**Remove commands:**
- `add_client_llm_provider()` (lines 421-462)
- `remove_client_llm_provider()` (lines 465-508)
- `add_client_mcp_server()` (lines 511-550)
- `remove_client_mcp_server()` (lines 553-596)
- `set_client_mcp_access()` (lines 605-654)

**Update `list_clients()` and `create_client()`:**
- Remove conversions using old access modes

### File: `src-tauri/src/main.rs`
- Remove registrations for deleted commands

---

## Phase 4: Frontend Cleanup

### Remove old fields from TypeScript interfaces:
- `src/App.tsx` (lines 22-34)
- `src/views/clients/client-detail.tsx` (lines 14-32)
- `src/views/clients/index.tsx` (lines 22-29)
- `src/views/clients/tabs/config-tab.tsx` (lines 7-16)
- `src/views/clients/tabs/settings-tab.tsx` (lines 21-30)
- `src/components/wizard/ClientCreationWizard.tsx`
- `src/components/connection-graph/types.ts`

### Update wizard:
- Remove `mcpAccessMode`, `selectedMcpServers`, `skillsAccessMode`, `selectedSkills` from state
- Use new permission commands instead

---

## Phase 5: Remove Old Type Definitions

### File: `crates/lr-config/src/types.rs`

**Remove from Client struct (lines 1173-1250):**
- `allowed_llm_providers` (line 1176)
- `mcp_server_access` (lines 1182-1187)
- `skills_access` (lines 1199-1204)
- `marketplace_enabled` (line 1250)

**Remove enums:**
- `McpServerAccess` (lines 627-637)
- `SkillsAccess` (lines 667-675)

**Remove serialization functions:**
- `serialize_mcp_server_access` (lines 1727-1744)
- `deserialize_mcp_server_access` (lines 1748-1812)
- `serialize_skills_access` (lines 1085-1102)
- `deserialize_skills_access` (lines 1105-1153)

**Remove helper methods (lines 1562-1660):**
- `can_access_llm_provider()`, `add_llm_provider()`, `remove_llm_provider()`
- `can_access_mcp_server()`, `add_mcp_server()`, `remove_mcp_server()`, `set_mcp_server_access()`
- `can_access_skill()`, `set_skills_access()`

### File: `crates/lr-config/src/lib.rs`
- Update `create_client_with_strategy()` (lines 289-301) to remove old field initialization

---

## Phase 6: Update Tests

### Files to update:
- `src-tauri/tests/client_auth_tests.rs`
- `src-tauri/tests/access_control_tests.rs`
- `src-tauri/tests/route_helpers_tests.rs`
- `src-tauri/tests/router_strategy_tests.rs`
- `src-tauri/tests/mcp_bridge_tests.rs`
- `src-tauri/tests/skills_e2e_test.rs`

Replace old field usage with new permission structures.

---

## Note: Firewall System (DO NOT MODIFY)

The `firewall: FirewallRules` field is a **separate system** for runtime tool approval and should NOT be changed. It works alongside permissions:
- **Permissions**: "Can client access this provider/server/skill?" (access control)
- **Firewall**: "When tool is called, auto-approve or ask user?" (runtime behavior)

---

## Verification

1. Run `cargo test` - all tests pass
2. Run `cargo clippy` - no errors
3. Start app, verify existing clients still work
4. Create new client, verify permissions work
5. Test permission inheritance in UI (global → provider → model)
6. Verify config file migration from old format
