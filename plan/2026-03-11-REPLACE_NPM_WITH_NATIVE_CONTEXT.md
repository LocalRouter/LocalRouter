# Replace External `context-mode` npm with Native `lr-context`

## Date: 2026-03-11

## Goal
Replace the external `context-mode` npm STDIO process with direct calls to the native `lr-context` Rust crate. Remove all indexing tools. Add `IndexRead` tool. Remove `indexing_tools` config. Remove npm/Node.js dependency from UI.

## Files Modified
- `crates/lr-mcp/Cargo.toml` — add `lr-context` dep
- `crates/lr-mcp/src/gateway/context_mode.rs` — full rewrite
- `crates/lr-mcp/src/gateway/gateway_tools.rs` — remove ensure_context_mode_tools_cached, update compress
- `crates/lr-mcp/src/gateway/gateway.rs` — update catalog indexing, remove indexing_tools refs
- `crates/lr-mcp/src/gateway/merger.rs` — remove indexing_tools_enabled field
- `crates/lr-config/src/types.rs` — remove indexing_tools from config + client
- `crates/lr-config/src/migration.rs` — bump to v20
- `src-tauri/src/ui/commands.rs` — remove get_context_mode_info, install_context_mode, indexing_tools param
- `src-tauri/src/ui/commands_clients.rs` — remove toggle_client_indexing_tools, indexing_tools from ClientInfo
- `src-tauri/src/ui/commands_coding_agents.rs` — remove indexing_tools_enabled param
- `src-tauri/src/ui/tray_menu.rs` — remove indexing tools toggle
- `src-tauri/src/ui/tray.rs` — remove toggle handler
- `src-tauri/src/main.rs` — remove command registrations
- `src/types/tauri-commands.ts` — remove related types
- `src/views/context-management/index.tsx` — remove deps section, indexing tools toggle
- `src/views/mcp-optimization/index.tsx` — same
- `src/views/clients/tabs/compression-tab.tsx` — remove indexing tools toggle
- `src/views/clients/tabs/info-tab.tsx` — remove indexing tools pill
- `website/src/components/demo/TauriMockSetup.ts` — update mocks
