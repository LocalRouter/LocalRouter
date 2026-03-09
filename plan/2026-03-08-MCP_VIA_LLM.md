# MCP via LLM Mode Implementation

**Date**: 2026-03-08
**Status**: In Progress - Phase 1

## Overview

Add a fourth client mode — **MCP via LLM** — where MCP tools are transparently injected into LLM requests, tool calls are intercepted and executed server-side via the MCP gateway, and the conversation loops until the LLM produces a final response.

## Phases

### Phase 1 — Foundation (Current)
- [x] Add `McpViaLlm` to `ClientMode` enum
- [x] Add `McpViaLlmConfig` struct
- [x] Fix prerequisite bug: forward tool_calls/tool_call_id in convert_to_provider_request
- [x] Create `crates/lr-mcp-via-llm/` crate
- [x] Implement session management (per-message hash matching)
- [x] Implement basic agentic loop (non-streaming, MCP-only tools)
- [x] Add injection point in chat.rs
- [x] Add `McpViaLlmManager` to AppState
- [ ] UI: experimental badge on client mode

### Phase 2 — Mixed Tools
- Tool classification (MCP vs client)
- Parallel background MCP execution
- Session state for pending mixed execution
- History reconstruction on client tool result return

### Phase 3 — Streaming
- Multi-segment streaming adapter
- tool_call delta buffering
- Mixed tool streaming (filter to client-only)
- SSE keepalive during tool execution

### Phase 4 — Full MCP Features
- Resources as tools
- Prompt injection
- Notification handling
- Request resume cache

### Phase 5 — Polish
- Usage aggregation
- Metrics/logging
- Session cleanup task
- Session management UI
- End-to-end tests

## Key Files
- `crates/lr-config/src/types.rs` - ClientMode enum, McpViaLlmConfig
- `crates/lr-mcp-via-llm/` - New crate with orchestration logic
- `crates/lr-server/src/routes/chat.rs` - Injection point
- `crates/lr-server/src/state.rs` - AppState integration
