# Refactor Unified MCP Gateway Welcome Page

**Date**: 2026-03-07

## Summary

Refactoring the gateway welcome page (system prompt instructions) for consistency, completeness, and better deferred mode behavior.

## Key Changes

1. Add `tool_names: Vec<String>` to `VirtualInstructions` struct
2. Rewrite `collect_virtual_instructions()` to populate tool names from `list_tools()`
3. Fix marketplace to return instructions (was returning `None`)
4. Move skills `get_info` workflow into skills instructions XML
5. Consistent format: all servers use `**name**` + tool listing + `<xml>` instructions
6. Virtual servers listed first, regular servers second, unavailable last
7. All tool items annotated with `(tool)`, `(resource)`, `(prompt)`
8. Deferred mode: omit regular server instructions, add `server_info` tool, show first 20 tools
9. Add `server_instructions` to `DeferredLoadingState` for `server_info` tool
10. Simplified header text (3 cases: normal, deferred, empty)

## Files Modified

- `crates/lr-mcp/src/gateway/virtual_server.rs` - Add `tool_names` field
- `crates/lr-mcp/src/gateway/gateway.rs` - Rewrite `collect_virtual_instructions()`
- `crates/lr-mcp/src/gateway/virtual_skills.rs` - Add `tool_names`, move get_info text
- `crates/lr-mcp/src/gateway/virtual_marketplace.rs` - Implement `build_instructions()`
- `crates/lr-mcp/src/gateway/virtual_coding_agents.rs` - Add `tool_names`
- `crates/lr-mcp/src/gateway/merger.rs` - Rewrite rendering functions + tests
- `crates/lr-mcp/src/gateway/deferred.rs` - Add `create_server_info_tool()`
- `crates/lr-mcp/src/gateway/types.rs` - Add `server_instructions` to `DeferredLoadingState`
- `crates/lr-mcp/src/gateway/gateway_tools.rs` - Add `server_info` tool + handler
