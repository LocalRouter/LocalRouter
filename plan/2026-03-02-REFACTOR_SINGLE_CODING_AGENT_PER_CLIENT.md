# Refactor: Single Coding Agent Per Client

## Context

Currently each client has hierarchical coding agent permissions (`global` + per-agent overrides), allowing multiple agents simultaneously. Each agent exposes 6 MCP tools with agent-specific prefixes (e.g., `claude_code_start`, `gemini_cli_start`). There's a dedicated "Coding Agents" page in the sidebar with per-agent configuration (working directory, model override, etc.).

**Goal**: Simplify to one coding agent per client with unified MCP tool names, move global coding agent settings to Settings, and remove per-agent configuration.

---

## Changes

### 1. Unified MCP Tool Prefix

Replace agent-specific tool names with a single `coding_agent_` prefix:
- `coding_agent_start`, `coding_agent_say`, `coding_agent_status`, `coding_agent_respond`, `coding_agent_interrupt`, `coding_agent_list`
- The selected agent type is resolved from the client's session, not from the tool name
- Only 6 tools total (not 6 × N agents)

### 2. Config Type Changes

**`crates/lr-config/src/types.rs`**

**`CodingAgentsConfig`** (global config) — simplify:
```rust
pub struct CodingAgentsConfig {
    #[serde(default, skip_serializing)]  // migration shim
    pub agents: Vec<CodingAgentConfig>,
    #[serde(default, skip_serializing)]  // migration shim
    pub default_working_directory: Option<String>,
    pub max_concurrent_sessions: usize,  // keep
    pub output_buffer_size: usize,       // keep
}
```

**`Client`** struct — replace `coding_agents_permissions` with:
```rust
#[serde(default, skip_serializing)]  // migration shim
pub coding_agents_permissions: CodingAgentsPermissions,

#[serde(default)]
pub coding_agent_permission: PermissionState,        // Allow/Ask/Off

#[serde(default, skip_serializing_if = "Option::is_none")]
pub coding_agent_type: Option<CodingAgentType>,      // which agent
```

Keep `CodingAgentsPermissions` and `CodingAgentConfig` structs for deserialization only (migration shims).

### 3. Config Migration (v15 → v16)

**`crates/lr-config/src/migration.rs`**

- If `global` was Allow/Ask → set `coding_agent_permission` to that state, `coding_agent_type = None` (user must select)
- If `global` was Off but had per-agent overrides → find first enabled agent, set `coding_agent_permission` to its state, `coding_agent_type` to that agent
- If everything Off → `coding_agent_permission = Off`, `coding_agent_type = None`

### 4. MCP Tool Changes

**`crates/lr-coding-agents/src/mcp_tools.rs`**

- `is_coding_agent_tool()` → check for `coding_agent_` prefix only
- `agent_type_from_tool()` → remove (no longer needed, agent type comes from session)
- `action_from_tool()` → extract suffix after `coding_agent_` prefix
- `build_coding_agent_tools(manager, permission, agent_type)` → if permission is Off or agent_type is None, return empty. Otherwise build 6 tools with `coding_agent_` prefix using the selected agent's display name in descriptions
- `handle_coding_agent_tool_call()` → receive agent_type from caller (session), not from tool name

### 5. MCP Gateway Changes

**`crates/lr-mcp/src/gateway/session.rs`** — `GatewaySession`:
- Replace `coding_agents_permissions: CodingAgentsPermissions` with `coding_agent_permission: PermissionState` + `coding_agent_type: Option<CodingAgentType>`

**`crates/lr-mcp/src/gateway/gateway.rs`** — `handle_request_with_skills`:
- Update parameter from `CodingAgentsPermissions` to the two new fields

**`crates/lr-mcp/src/gateway/gateway_tools.rs`**:
- `append_coding_agent_tools()` → accept `(permission, agent_type)` instead of `CodingAgentsPermissions`
- `handle_coding_agent_tool_call()` → read `coding_agent_type` from session to know which agent to dispatch to

### 6. Manager Changes

**`crates/lr-coding-agents/src/manager.rs`**

- `start_session()` — remove working directory fallback chain that checks per-agent config and default_working_directory. When no working directory provided by MCP client, always create temp dir under `std::env::temp_dir()`
- Remove `agent_config()` method (no per-agent configs)
- Remove model/permission_mode resolution from per-agent config

### 7. Server Route

**`crates/lr-server/src/routes/mcp.rs`**

- Pass `client.coding_agent_permission` and `client.coding_agent_type` instead of `client.coding_agents_permissions`

### 8. Tauri Commands

**`src-tauri/src/ui/commands_coding_agents.rs`**

- `CodingAgentInfo` — remove `workingDirectory`, `modelId`, `permissionMode` fields
- `list_coding_agents` — simplify (just iterate types, check install status)
- Remove `update_coding_agent_config` command entirely
- Replace `set_client_coding_agents_permission` with:
  - `set_client_coding_agent_permission(client_id, permission: PermissionState)`
  - `set_client_coding_agent_type(client_id, agent_type: Option<CodingAgentType>)`

**`src-tauri/src/ui/commands_clients.rs`** — `ClientInfo`:
- Replace `coding_agents_permissions` with `coding_agent_permission` + `coding_agent_type`

**`src-tauri/src/main.rs`** — update command registrations

### 9. Tray Menu

**`src-tauri/src/ui/tray_menu.rs`** + **`tray.rs`**

- Replace per-agent toggle list with simpler coding agent section
- Show selected agent name, permission toggle

### 10. Frontend — Remove Coding Agents Page

- **`src/components/layout/sidebar.tsx`** — remove `coding-agents` from nav items and keyboard shortcut
- **`src/App.tsx`** — remove `CodingAgentsView` import and case
- **`src/views/coding-agents/index.tsx`** — delete file (or keep for potential future use)

### 11. Frontend — Add Settings Tab

- **`src/views/settings/index.tsx`** — add `<TabsTrigger value="coding-agents">Coding Agents</TabsTrigger>`
- **`src/views/settings/coding-agents-tab.tsx`** — new file:
  - List all 10 coding agents with install status badges (installed/not found)
  - Max concurrent sessions setting (existing `get_max_coding_sessions` / `set_max_coding_sessions`)
  - No per-agent configuration

### 12. Frontend — Simplify Client Tab

**`src/views/clients/tabs/coding-agents-tab.tsx`**:
- Replace `CodingAgentsPermissionTree` with:
  - PermissionState selector (Allow/Ask/Off)
  - Dropdown to select ONE agent type (showing only installed agents + "None")
  - Always show dropdown (disabled when Off for consistency)

**`src/views/clients/client-detail.tsx`** — update Client interface

**`src/components/permissions/CodingAgentsPermissionTree.tsx`** — delete

### 13. Frontend — Connection Graph

**`src/components/connection-graph/utils/buildGraph.ts`**:
```typescript
// Old: multiple agents per client via permissions map
// New: at most one agent per client
const clientCodingAgents = client.coding_agent_permission !== 'off' && client.coding_agent_type
    ? [client.coding_agent_type] : []
```

**`src/components/connection-graph/types.ts`** — update Client interface

### 14. TypeScript Types

**`src/types/tauri-commands.ts`**:
- Update `CodingAgentInfo` (remove `workingDirectory`, `modelId`, `permissionMode`)
- Remove `UpdateCodingAgentConfigParams`, `SetClientCodingAgentsPermissionParams`
- Add `SetClientCodingAgentPermissionParams`, `SetClientCodingAgentTypeParams`
- Update `ClientInfo` to use new fields

**`src/components/permissions/types.ts`** — remove `CodingAgentsPermissions` export

### 15. Website Mocks

- **`website/src/components/demo/TauriMockSetup.ts`** — update mocks
- **`website/src/components/demo/mockData.ts`** — `coding_agent_permission` + `coding_agent_type`
- **`website/src/components/demo/MacOSTrayMenu.tsx`** — simplify tray menu

### 16. Tests

- **`src-tauri/tests/coding_agents_e2e_test.rs`** — update to new API
- **`crates/lr-mcp/src/bridge/stdio_bridge.rs`** — update test defaults
- **`crates/lr-coding-agents/src/mcp_tools.rs`** — rewrite tests for `coding_agent_` prefix
- **`src-tauri/tests/route_helpers_tests.rs`**, **`router_strategy_tests.rs`**, **`mcp_bridge_tests.rs`** — update client construction

---

## Implementation Order

1. Backend config types + migration (lr-config)
2. MCP tools crate (lr-coding-agents) — unified prefix, simplified API
3. MCP gateway (lr-mcp) — session, tool dispatch
4. Server route (lr-server)
5. Tauri commands + tray menu (src-tauri)
6. Frontend types + views
7. Website mocks
8. Tests + verify (`cargo test && cargo clippy && npx tsc --noEmit`)

## Verification

1. `cargo test && cargo clippy && cargo fmt` — all passing
2. `npx tsc --noEmit` — frontend types valid
3. Start app with existing v15 config → migration runs, config updated to v16
4. Client with coding agent selected → `coding_agent_start` etc. tools appear in MCP
5. Client with coding agent Off or None → no coding agent tools
6. Settings > Coding Agents tab shows all agents with install status
7. Connection graph shows single edge from client to selected agent
