# Plan: Aggregate Usage Across MCP via LLM Iterations + `request_usage_entries` Breakdown

## Context

When a client sends a request to an MCP via LLM client, LocalRouter may make multiple LLM calls in an agentic loop (LLM returns tool calls -> execute MCP tools -> call LLM again -> repeat until final answer). The final response's `usage` field contains the **sum** of all LLM calls' token usage. A new `request_usage_entries` field provides a per-call breakdown.

## Changes Implemented

### 1. Added `request_usage_entries` field to provider-level `CompletionResponse`
- `crates/lr-providers/src/lib.rs` — struct definition
- All 25 provider files + `src-tauri/src/providers/mod.rs` — `None` at construction sites

### 2. Added `request_usage_entries` field to server-level response types
- `crates/lr-server/src/types.rs` — `ChatCompletionResponse`, `ChatCompletionChunk`, `CompletionResponse`
- `crates/lr-server/src/routes/chat.rs` — wire field in non-streaming response conversion
- `crates/lr-server/src/routes/completions.rs` — `None` at construction sites

### 3. Per-iteration tracking in non-streaming orchestrator
- `crates/lr-mcp-via-llm/src/orchestrator.rs`:
  - Added `usage_entries` accumulator in `run_agentic_loop`
  - Push `response.usage.clone()` after each LLM call
  - Updated `build_final_response` to accept and set entries (only when >1 iteration)
  - Added `initial_usage_entries` parameter for resume support

### 4. Threaded through `PendingMixedExecution` for resume
- `crates/lr-mcp-via-llm/src/session.rs` — added `accumulated_usage_entries` field
- `crates/lr-mcp-via-llm/src/orchestrator.rs` — populate on pending creation, pass to resumed loop
- `crates/lr-mcp-via-llm/src/orchestrator_stream.rs` — empty vec for streaming path

### 5. Streaming limitation
Streaming SSE chunks don't include per-iteration token counts. The `request_usage_entries` field is `None` for streaming responses. This is a known limitation documented in the code.

## Behavior

- **Single LLM call**: `request_usage_entries` is absent from the response (serialized as omitted due to `skip_serializing_if`)
- **Multiple LLM calls**: `request_usage_entries` contains an array of `TokenUsage` objects, one per LLM call, in order
- The sum of entries equals the aggregate `usage` field
- Streaming responses do not include `request_usage_entries` (limitation of SSE protocol)
