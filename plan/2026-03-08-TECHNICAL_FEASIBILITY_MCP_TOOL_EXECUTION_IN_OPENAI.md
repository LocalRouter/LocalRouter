# Technical Feasibility: MCP Tool Execution in OpenAI Chat Completions

## Context

LocalRouter exposes an OpenAI-compatible `POST /v1/chat/completions` endpoint. Currently, when an LLM responds with `finish_reason: "tool_calls"`, those calls are passed through to the client. The proposal is to intercept tool calls that match MCP tools, execute them server-side via the MCP gateway, append results to the conversation, and re-submit to the LLM — creating an "agentic loop" that resolves MCP tools transparently before returning the final response to the client.

**Goal**: Clients get AI responses enriched with MCP tool results without needing MCP awareness.

---

## A. Core Agentic Loop — Feasibility: MEDIUM

### MCP Tool → OpenAI Tool Conversion

Direct mapping, no lossy conversion:
```
NamespacedTool { name: "filesystem__read_file", description, input_schema }
    ↓
Tool { type: "function", function: { name: "filesystem__read_file", description, parameters: input_schema } }
```

MCP `input_schema` is JSON Schema — identical to OpenAI's `parameters` format. The `__` namespace separator is valid in OpenAI function names (allows `a-z, A-Z, 0-9, _, -`, max 64 chars).

### Injection Point

After `convert_to_provider_request` builds the provider request, merge MCP tools into the `tools` array. Set `tool_choice` to `"auto"` if not already specified.

### Interception Point

After `router.complete()` returns, check `finish_reason == "tool_calls"` and match tool names against the injected MCP tool set.

### Tool Execution

Route each MCP tool call through the existing gateway:
1. Build `JsonRpcRequest { method: "tools/call", params: { name, arguments } }`
2. Call `mcp_gateway.handle_tools_call(session, request)`
3. Convert MCP result content to OpenAI tool message: `{ role: "tool", tool_call_id: "call_xxx", content: "<text>" }`

**Key requirement**: The chat handler needs an MCP gateway session. `AppState` already has `mcp_gateway: Arc<McpGateway>`, and the gateway supports `get_or_create_session` keyed by client_id — so the plumbing exists.

### Loop Structure

```
loop {
    response = router.complete(request_with_mcp_tools)
    if finish_reason != "tool_calls" → break, return response
    if iteration >= max_iterations → break, return response

    (mcp_calls, client_calls) = partition by MCP tool name set
    if !client_calls.is_empty() → break, return with client tool_calls only

    for call in mcp_calls:
        result = gateway.execute_tool(call)  // or inject error on failure
        append assistant tool_call msg + tool result msg to conversation

    iteration += 1
}
```

### Termination Conditions
1. `finish_reason` is `"stop"` or `"length"`
2. Max iteration limit reached (configurable per-client, default 10)
3. Tool execution error injected as result — LLM decides next step
4. Remaining tool_calls are all client-side (not MCP) — return to client
5. Client disconnect / cancellation

**Verdict**: Fully feasible. No architectural blockers.

---

## B. Streaming vs Non-Streaming — Feasibility: HARD (streaming), MEDIUM (non-streaming)

### Non-Streaming
Straightforward — the loop buffers internally, returns only the final response. Aggregate token usage across iterations.

### Streaming: Three Approaches

| Approach | Description | Compatibility | Complexity |
|----------|-------------|---------------|------------|
| **Stream-only-final** | Buffer intermediate iterations, stream only the last | Full | Low |
| **Stream with annotations** | Custom SSE events for intermediate tool calls | Breaks standard clients | High |
| **Multi-stream** | Multiple `[DONE]` markers per iteration | Breaks most clients | Medium |

**Recommendation: Stream-only-final for V1.**
- Intermediate iterations use `router.complete()` (non-streaming)
- Final iteration uses `router.stream_complete()` (streaming)
- During tool execution, SSE keepalive comments prevent timeouts (already configured via Axum `KeepAlive`)
- Optionally emit SSE comments like `:executing tool filesystem__read_file\n\n` for status

**Verdict**: Feasible with stream-only-final. Full intermediate streaming is a V2 enhancement.

---

## C. History Management — Feasibility: HARD (most complex area)

### The Problem

Server-side tool execution creates hidden intermediate messages:
```
[user] → [assistant + tool_calls] → [tool results] → [assistant final]
```
The client only sees `[user] → [assistant final]`. On follow-up, they send history without the intermediate messages.

### Three Approaches

**C1: Stateless / Collapse (Recommended for V1)**
- The final assistant response already contains information derived from tools
- Client stores `[user, assistant_final]` — works for follow-ups
- No server-side state needed
- **Tradeoff**: Structured tool call history is lost; LLM can't reference specific tool outputs in later turns
- **Mitigation**: For most use cases (file reading, web search, data lookup), the final text response carries sufficient context

**C2: Server-side conversation state (V2)**
- Store full conversation keyed by `x-localrouter-conversation-id` header
- On follow-up, server reconstructs full history including intermediate tool calls
- Requires: storage backend, TTL/eviction, memory management
- Enables: multi-turn agentic conversations with full tool context

**C3: Return intermediate history to client (Alternative)**
- Include tool call history in a custom response field `_tool_history`
- Client expected to echo it back in subsequent requests
- **Problem**: Non-standard, requires client awareness, increases payload size

**Verdict**: C1 (stateless) is fully feasible and sufficient for V1. C2 is feasible as a V2 enhancement with moderate effort.

---

## D. Tool Namespace Conflicts — Feasibility: EASY

### Scenario
Client sends their own `tools` in the request + we inject MCP tools. Possible name collisions.

### Solution
MCP tools are already namespaced with `__` (e.g., `filesystem__read_file`). Collisions with client tool names are extremely unlikely.

**Routing logic**: Maintain a `HashSet<String>` of injected MCP tool names. When the LLM returns tool_calls:
- Name in MCP set → execute server-side
- Name not in MCP set → pass through to client

**On collision**: Client tools take priority (skip the colliding MCP tool). Log a warning.

**Verdict**: Trivially feasible. The existing namespace separator handles this naturally.

---

## E. Mixed Tool Calls (MCP + Client) — Feasibility: HARD

### Scenario
LLM returns `finish_reason: "tool_calls"` with 3 tool calls: 2 are MCP tools, 1 is a client tool. We can execute the MCP tools but must return the client tool call to the client.

### Approach: Sequential Execution (Recommended)
1. Execute all MCP tool calls from the response
2. Append MCP tool results to conversation
3. Re-submit to LLM
4. If LLM now returns only client tool calls → return to client with `finish_reason: "tool_calls"`
5. If LLM returns stop → return final response
6. If LLM returns more MCP tools → continue loop

**Edge case**: LLM might loop indefinitely calling MCP tools without ever surfacing client tool calls. The max iteration limit handles this.

**Alternative (simpler)**: If client sends their own tools in the request, disable agentic MCP execution for that request entirely. This avoids the mixed scenario at the cost of flexibility.

**Verdict**: Feasible via sequential execution. The simpler "disable if client has tools" is a reasonable V1 constraint.

---

## F. Error Handling — Feasibility: MEDIUM

| Error | Handling |
|-------|----------|
| MCP tool execution fails | Inject error as tool result: `{ role: "tool", content: "Error: <message>" }`. LLM decides how to proceed. |
| Provider error on re-submission | Router already has retry logic. If exhausted, return error with iteration context. |
| Timeout during tool execution | Per-tool timeout (existing `server_timeout_seconds`). Per-loop timeout (new, e.g., 120s). Inject error on timeout. |
| Client disconnect | Cancellation token propagated via `tokio::select!` (see Section I). |

**Verdict**: Fully feasible. Error-as-tool-result is the standard agentic pattern.

---

## G. Cost & Rate Limiting — Feasibility: MEDIUM

### Token Usage
Aggregate across all iterations:
```
total_prompt_tokens = Σ(iteration.prompt_tokens)
total_completion_tokens = Σ(iteration.completion_tokens)
```
Report aggregated totals in the standard `usage` field. Optionally include per-iteration breakdown in `_agentic` metadata.

### Rate Limiting
Internal loop calls bypass client-facing rate limits but still count toward usage tracking. Provider-side rate limits are handled as provider errors (Section F).

### Token Budget
Track cumulative prompt tokens across iterations. If approaching the model's context limit, abort the loop and return the best available response. The existing `lr-compression` crate could compress intermediate results between iterations.

**Verdict**: Feasible. Usage aggregation is straightforward.

---

## H. Firewall & Permissions — Feasibility: HARD (partial blocker)

### The "Ask" Problem
The firewall has three states: `Allow`, `Off` (deny), and `Ask` (UI popup, blocks until user responds). In an agentic loop, `Ask` breaks the automated flow.

### Solution: Session-Scoped Approval (Recommended)
The gateway already has `firewall_session_approvals` and `FirewallApprovalAction::AllowSession`. On first encounter of an `Ask` tool in the loop:
1. Trigger the approval popup (user sees which tool the AI wants to use)
2. If approved with "Allow for session", subsequent calls in the same loop skip the popup
3. If denied, inject denial as tool error result

### Client Config Extension
```rust
pub mcp_agentic_mode: AgenticMode,  // Off (default) | Enabled | RequiresApproval
```
- `Off`: Tool calls pass through to client (current behavior)
- `Enabled`: Agentic loop active, tool permissions still apply per-tool
- `RequiresApproval`: Prompt user before starting each agentic loop

**Verdict**: Feasible using existing session-scoped approval infrastructure. The `Ask` UX is slightly awkward but workable. For fully automated use cases, users set tool permissions to `Allow`.

---

## I. Cancellation — Feasibility: MEDIUM

### Non-Streaming
Use `tokio::select!` between the loop body and a connection-dropped signal:
```rust
tokio::select! {
    result = agentic_loop(&gateway, &router, request) => result,
    _ = request_cancelled => Err(Cancelled)
}
```

### Streaming
SSE stream detects client disconnect when `send` fails. Propagate back via a shared `CancellationToken`.

### MCP Tool Cancellation
Send `notifications/cancelled` to the MCP server (best-effort). Drop the pending future. Most MCP servers don't implement graceful cancellation, so this is pragmatic.

**Verdict**: Fully feasible with standard tokio patterns.

---

## J. Response Format — Feasibility: EASY

### Standard Response (default)
Identical to a normal `ChatCompletionResponse` with `finish_reason: "stop"`. Client cannot distinguish from a non-agentic response. Full backward compatibility.

### Extended Metadata (opt-in, via header or client config)
```json
{
    "usage": { "prompt_tokens": 1500, "completion_tokens": 50 },
    "_agentic": {
        "iterations": 3,
        "tool_calls_executed": 5,
        "tools_used": ["filesystem__read_file", "search__query"],
        "total_wall_time_ms": 4500
    }
}
```

Underscore-prefixed fields are ignored by standard OpenAI client libraries.

**Verdict**: Trivially feasible. No compatibility concerns.

---

## Prerequisite Bug Fix

**CRITICAL**: `crates/lr-server/src/routes/chat.rs:1311-1312` hardcodes `tool_calls: None` and `tool_call_id: None` for ALL messages in `convert_to_provider_request`. This drops tool call history from conversation messages, breaking multi-turn tool calling even without this feature. Must be fixed first.

---

## Feasibility Summary

| Area | Feasibility | Blocker? | V1 Approach |
|------|------------|----------|-------------|
| A. Core Loop | Medium | No | Direct implementation |
| B. Streaming | Hard | No | Stream-only-final |
| C. History | Hard | No | Stateless / collapse |
| D. Namespaces | Easy | No | Existing `__` separator |
| E. Mixed Tools | Hard | No | Disable if client has tools |
| F. Errors | Medium | No | Error-as-tool-result |
| G. Cost/Limits | Medium | No | Aggregate usage |
| H. Firewall | Hard | Partial | Session-scoped approval |
| I. Cancellation | Medium | No | tokio::select! |
| J. Response | Easy | No | Standard format |

**Overall verdict: FEASIBLE.** No hard blockers. The main complexity is in streaming (solvable with stream-only-final) and history management (solvable with stateless approach). The firewall "Ask" permission is the only friction point, addressed by session-scoped approvals.

---

## Key Files

| File | Role |
|------|------|
| `crates/lr-server/src/routes/chat.rs` | Chat handler — add agentic loop, fix tool_calls bug at L1311 |
| `crates/lr-server/src/types.rs` | OpenAI-compatible types (already complete for tool calling) |
| `crates/lr-mcp/src/gateway/gateway_tools.rs` | MCP tool list/execution — expose for chat handler use |
| `crates/lr-mcp/src/gateway/gateway.rs` | Gateway session management |
| `crates/lr-mcp/src/gateway/session.rs` | Session creation/reuse for chat handler |
| `crates/lr-config/src/types.rs` | Client config — add `mcp_agentic_mode` field |
| `crates/lr-mcp/src/gateway/sampling.rs` | Existing server-initiated LLM call pattern (reference architecture) |
