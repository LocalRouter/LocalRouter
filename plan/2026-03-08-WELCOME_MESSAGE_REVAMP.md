# Welcome Message Revamp + Compression Preview UI

## Date: 2026-03-08

## Goals
1. Add `priority` field to `VirtualInstructions` for deterministic ordering
2. Revamp welcome message to use unified per-server XML block format
3. Add `build_preview_instructions_context()` for mock data
4. Add Tauri command `preview_catalog_compression` for live preview
5. Add preview UI in Context Management Settings tab

## Files Modified
- `crates/lr-mcp/src/gateway/virtual_server.rs` — add priority field
- `crates/lr-mcp/src/gateway/context_mode.rs` — return instructions with priority 0
- `crates/lr-mcp/src/gateway/virtual_coding_agents.rs` — priority 10
- `crates/lr-mcp/src/gateway/virtual_marketplace.rs` — priority 20
- `crates/lr-mcp/src/gateway/virtual_skills.rs` — priority 30
- `crates/lr-mcp/src/gateway/gateway.rs` — sort by priority, set fallback priority 50
- `crates/lr-mcp/src/gateway/merger.rs` — revamp format + preview mock data
- `crates/lr-mcp/src/gateway/mod.rs` — re-exports
- `src-tauri/src/ui/commands.rs` — preview_catalog_compression command
- `src-tauri/src/main.rs` — register command
- `src/types/tauri-commands.ts` — TypeScript types
- `src/views/context-management/index.tsx` — preview UI
- `website/src/components/demo/TauriMockSetup.ts` — demo mock
