# Virtual MCP Server Refactoring

**Date**: 2026-03-07
**Status**: In Progress

## Goal
Refactor Skills, Marketplace, and Coding Agents from custom if-else extensions in the MCP gateway into virtual MCP servers — in-memory implementations of a `VirtualMcpServer` trait that the gateway treats uniformly.

## Phases
1. Add trait + infrastructure (no behavior change)
2. Implement concrete virtual servers (no behavior change)
3. Switch gateway to virtual servers (behavior change)
4. Remove old code
5. Cleanup & verify

## Key Files
- `crates/lr-mcp/src/gateway/virtual_server.rs` — trait + types
- `crates/lr-mcp/src/gateway/virtual_skills.rs` — SkillsVirtualServer
- `crates/lr-mcp/src/gateway/virtual_marketplace.rs` — MarketplaceVirtualServer
- `crates/lr-mcp/src/gateway/virtual_coding_agents.rs` — CodingAgentVirtualServer
- `crates/lr-mcp/src/gateway/gateway.rs` — Replace OnceLock fields
- `crates/lr-mcp/src/gateway/gateway_tools.rs` — Replace if-else dispatch
- `crates/lr-mcp/src/gateway/session.rs` — Add virtual_server_state
- `crates/lr-mcp/src/gateway/merger.rs` — Update InstructionsContext
- `crates/lr-server/src/routes/mcp.rs` — Update to pass &Client
- `src-tauri/src/main.rs` — Replace OnceLock setup
