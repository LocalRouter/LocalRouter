# Built-in MCP Indexing Controls in Response RAG Settings

## Context

The Response RAG page's Tool Indexing section currently shows gateway MCP tools and client tools, but is missing virtual MCP servers (built-in MCPs). Users need to control which built-in MCP tool responses get indexed into FTS5. Some virtual MCP tools are indexable (their responses contain useful searchable content) while others are action-only tools that should always be Off.

## Changes

### Backend

**1. Add `all_tool_names()` to `VirtualMcpServer` trait**
- `crates/lr-mcp/src/gateway/virtual_server.rs` — new trait method with no default impl
- Implement in each virtual server:
  - `context_mode.rs`: `[config.search_tool_name, config.read_tool_name]`
  - `virtual_skills.rs`: `["skill_read"]`
  - `virtual_marketplace.rs`: `["marketplace__search", "marketplace__install"]`
  - `virtual_coding_agents.rs`: all 6 coding agent tools

**2. Add `virtual_indexing` config field**
- `crates/lr-config/src/types.rs` — add `#[serde(default)] pub virtual_indexing: GatewayIndexingPermissions` to `ContextManagementConfig` + default impl

**3. Add public method on `McpGateway`**
- `crates/lr-mcp/src/gateway/gateway.rs` — `pub fn list_virtual_server_indexing_info()` that reads virtual_servers lock and returns id, display_name, tools with indexable flag

**4. Add Tauri commands**
- `src-tauri/src/ui/commands.rs`:
  - `VirtualMcpIndexingInfo` + `VirtualMcpToolIndexingInfo` response structs
  - `list_virtual_mcp_indexing_info` command
  - `set_virtual_indexing_permission` command (mirrors `set_gateway_indexing_permission` but targets `virtual_indexing`)
- `src-tauri/src/main.rs` — register both commands (~line 1857)

### Frontend

**5. TypeScript types** — `src/types/tauri-commands.ts`
- `VirtualMcpIndexingInfo`, `VirtualMcpToolIndexingInfo` response types
- `SetVirtualIndexingPermissionParams` params type
- Add `virtual_indexing` to `ContextManagementConfig`

**6. New `VirtualMcpIndexingTree` component** — `src/components/permissions/VirtualMcpIndexingTree.tsx`
- Calls `list_virtual_mcp_indexing_info` to get server/tool data
- Uses `PermissionTreeSelector` + `IndexingStateButton` (same as `GatewayIndexingTree`)
- Non-indexable tools: forced to "disable" in `flatPermissions` + `disabled: true` on TreeNode
- Servers where ALL tools are non-indexable: also forced Off and disabled (e.g., Context Management)
- `defaultExpanded={false}` (tools collapsed by default)
- Calls `set_virtual_indexing_permission` for changes

**7. Update Response RAG page** — `src/views/response-rag/index.tsx`
- Three sections in order: Built-in MCPs → MCPs → Client MCPs
- Each section has a brief description underneath the section name
- Remove the single outer `border rounded-lg` wrapper; each section has its own border

**8. Rename `GatewayIndexingTree` label** — `src/components/permissions/GatewayIndexingTree.tsx`
- Change `globalLabel` from `"All Gateway Tools"` to `"MCPs"`

**9. Demo mock** — `website/src/components/demo/TauriMockSetup.ts`
- Add `virtual_indexing` to `get_context_management_config` mock
- Add `list_virtual_mcp_indexing_info` and `set_virtual_indexing_permission` mocks

### Permission Key Format
- Server keys: virtual server IDs (`_context_mode`, `_skills`, `_marketplace`, `_coding_agents`)
- Tool keys: `"{server_id}__{tool_name}"` (e.g., `_skills__skill_read`, `_coding_agents__coding_agent_status`)

## Verification

1. `cargo test && cargo clippy` — backend compiles and passes
2. `npx tsc --noEmit` — frontend types check
3. `cargo tauri dev` — run app, navigate to Response RAG → Info tab
4. Verify three sections appear: Built-in MCPs, MCPs, Client MCPs
5. Expand virtual server trees — non-indexable tools show as Off/disabled
6. Toggle indexable tools and server-level toggles — verify persistence
7. Context Management server shows entirely as Off/disabled
