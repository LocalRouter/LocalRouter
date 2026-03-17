# Technical Feasibility: MCP via LLM Mode

## Context

LocalRouter exposes an OpenAI-compatible `POST /v1/chat/completions` endpoint. Currently, tool calls in LLM responses are passed through to the client. The proposal is to add a fourth client mode — **MCP via LLM** — where MCP tools are transparently injected into LLM requests, tool calls are intercepted and executed server-side via the MCP gateway, and the conversation loops until the LLM produces a final response. The client speaks only the OpenAI protocol and never needs MCP awareness.

**Goal**: Clients get AI responses enriched with MCP tool results without needing MCP awareness.

---

## 1. Fourth Client Mode

### Current (`crates/lr-config/src/types.rs:1966-1974`)
```rust
enum ClientMode {
    Both,     // LLM + MCP (default)
    LlmOnly,  // LLM only
    McpOnly,   // MCP only
}
```

### New
```rust
McpViaLlm,  // MCP tools injected into LLM requests, executed server-side (experimental)
```

When `McpViaLlm` is active:
- `/v1/chat/completions` requests enter the agentic orchestrator
- `/mcp/*` routes are blocked (client doesn't speak MCP)
- Enable/disable per-client, no "Ask" mode — this is a binary toggle
- Marked as **experimental** in the UI (existing `Badge variant="secondary"` pattern)

---

## 2. Injection Point

**Location**: `crates/lr-server/src/routes/chat.rs`, after validation/rate-limiting (~line 264), before `convert_to_provider_request()`.

```rust
// After rate limits checked, before provider conversion:
if client.client_mode == ClientMode::McpViaLlm {
    return mcp_via_llm_manager.handle_request(state, auth, client, request).await;
}
// ... existing code continues unchanged for other modes
```

### Why This Is Clean
- **All OpenAI validation already complete** (access control, model permissions, rate limits)
- **No pollution of provider code**: `convert_to_provider_request()`, `handle_streaming()`, `handle_non_streaming()` untouched
- **No pollution of MCP gateway**: Orchestrator creates its own in-memory MCP session using existing `McpGateway` API
- **Single return type**: Returns `ApiResult<Response>` — same as existing handlers
- **New crate**: `crates/lr-mcp-via-llm/` keeps all orchestration logic separate from `lr-server` and `lr-mcp`

### Internal MCP Session
The orchestrator creates a gateway session as if it were another client:
```rust
let session_key = format!("mcp-via-llm-{}", session_id);
gateway.get_or_create_session(&session_key, &client.id, allowed_servers, roots)
```
This session persists across requests and holds: tool mappings, resource mappings, ContextMode state, firewall approvals — identical to a regular MCP client session.

---

## 3. Session Matching via Message History

### The Problem
The OpenAI chat API is stateless — each request carries the full message history. We need to match incoming requests to existing sessions to preserve MCP gateway state (ContextMode, tool mappings, pending tool executions).

### Algorithm: Per-Message Hash Sequence

Instead of a single combined fingerprint, store **individual hashes for every message** in the session, plus markers for where our injected tool calls and results sit.

**Session stores**:
```rust
pub struct SessionHistory {
    /// Hash of each message in order (BLAKE3 of role + content + tool_call ids/names)
    pub message_hashes: Vec<u64>,
    /// Indices where our injected tool calls/results live (invisible to client)
    pub injected_ranges: Vec<Range<usize>>,
    /// The full message history (with injected messages)
    pub full_messages: Vec<ChatMessage>,
}
```

**On each incoming request**:
1. Compute hash for each message in `request.messages`
2. Build a **client hash sequence** (the hashes of all messages the client sent)
3. Build the **session's visible hash sequence** (session hashes with injected ranges removed — what the client would have seen)
4. Attempt matching in priority order:

### Matching Strategies (tried in order)

**1. Full match**: Client's hash sequence == session's visible hash sequence
- All messages match → this is a normal **continuation** (client appended new messages at the end)
- Re-inject tool call history at the stored injection points
- Append the client's new trailing messages

**2. Prefix match**: Client's hash sequence is a prefix of session's visible hash sequence
- Client sent the same request again (no new messages) → **retry/timeout resume**
- Check the resume cache; if in-progress, subscribe; if completed, return cached result

**3. Suffix match**: Client's hash sequence ends match the session's visible sequence end
- Client truncated/compacted old messages → **compaction**
- Only inject tool call history for the matching suffix portion (where we have fingerprint matches)
- Older injected tool calls are dropped (they were in the compacted region)
- Log: `"Session matched via suffix (compaction detected), dropping N early tool call injections"`

**4. Partial/subsequence match**: Some messages match but with gaps
- Best-effort: find the longest matching subsequence
- Inject tool calls only for the ranges that correspond to matched messages
- Log: `"Session matched via partial subsequence ({N}/{M} messages matched)"`

**5. No match**: No meaningful overlap found
- Create new session
- Log: `"No session match for client {client_id}, creating new session (had {N} candidates)"`

### Why Per-Message Hashing?

- **Granular injection**: We know exactly *which* assistant responses have associated tool call history, so we only inject where we have a match
- **Robust to edits**: If client edits one message, only that message's hash changes; the rest still match
- **Flexible**: Supports prefix, suffix, full, and partial matching from the same data structure
- **Safe degradation**: If we reuse a session we shouldn't have, the worst case is injecting some extra context (the tool call history). If we create a new session when we had one, we just lose ContextMode state. Neither is catastrophic.

### Matching Scope
Sessions are scoped to a `client_id`. We only attempt matching against sessions belonging to the same client. This prevents cross-client leakage and reduces the search space.

### Logging
All match outcomes are logged at INFO level:
- `"Session matched (full) for client {id}: session {session_id}"`
- `"Session matched (prefix/retry) for client {id}: session {session_id}"`
- `"Session matched (suffix/compaction) for client {id}: session {session_id}, dropped {N} early injections"`
- `"Session matched (partial) for client {id}: session {session_id}, {N}/{M} messages matched"`
- `"Session match failed for client {id}: creating new session (checked {N} candidates)"`

---

## 4. History Management

### Server-Side History Store
The session stores the **full conversation history** including all intermediate tool calls and results that the client never sees:

```
Client sends:  [user_1]
Server stores: [user_1, assistant_1(tool_calls), tool_result_1, assistant_2(final)]
Client sees:   [assistant_2(final)]

Client sends:  [user_1, assistant_2(final), user_2]
Server matches assistant_2 → same session
Server re-injects: [user_1, assistant_1(tool_calls), tool_result_1, assistant_2(final), user_2]
```

### Re-injection Algorithm
1. Match session via per-message hash sequence (Section 3)
2. For each matched message in the client's history, check if an `injected_range` follows it in the session
3. If yes, splice the injected tool call/result messages back into the conversation at that point
4. Only inject for matched message positions — unmatched regions get no injection
5. Append the client's new trailing messages (after the last matched message)
6. Submit the enriched conversation to the LLM

**Example** (full match + continuation):
```
Session stores:  [user_1, assistant_1(tool_calls)*, tool_result_1*, assistant_2(final), user_2, ...]
                  * = injected range [1..3]
Client sends:    [user_1, assistant_2(final), user_3]  ← client never saw the tool calls
Re-injected:     [user_1, assistant_1(tool_calls), tool_result_1, assistant_2(final), user_3]
```

**Example** (suffix match / compaction):
```
Session stores:  [user_1, ...(old)..., user_5, assistant_5(tool_calls)*, tool_result_5*, assistant_6(final)]
Client sends:    [user_5, assistant_6(final), user_7]  ← client compacted old messages
Suffix match on: [user_5, assistant_6(final)]
Re-injected:     [user_5, assistant_5(tool_calls), tool_result_5, assistant_6(final), user_7]
```

### Tracking Responses
Every response we return to the client is hashed and stored in the session's `message_hashes`. The injected tool calls/results are stored in `full_messages` with their index ranges tracked in `injected_ranges`. When the client echoes a response back, we recognize it by its hash and know exactly which injected messages belong with it.

---

## 5. Core Agentic Loop

### Non-Streaming Flow
```
Client request → Orchestrator
  ├── 1. Match/create session
  ├── 2. Re-inject full history from session
  ├── 3. Fetch MCP tools (gateway tools/list)
  ├── 4. Merge into request.tools (MCP takes precedence on conflicts)
  ├── 5. Convert to provider request
  ├── 6. router.complete()
  │
  ├── 7. Inspect response:
  │     ├── finish_reason == "stop" → store in history, return to client
  │     └── finish_reason == "tool_calls" → classify tools
  │
  ├── 8. Classify tool calls:
  │     ├── All MCP → execute via gateway, append to history, loop to step 5
  │     ├── All client → return to client with tool_calls
  │     └── Mixed → parallel execution (Section 6)
  │
  └── 9. Store final response in session history
```

### Loop Safeguards
- Max iterations: configurable, default 25
- Max total timeout: configurable, default 300s
- Max token budget: configurable (abort loop if approaching model context limit)

---

## 6. Parallel Mixed Tool Execution

This is the most complex scenario. When the LLM returns a mix of MCP and client tool calls:

### Flow

**Step 1**: LLM returns `tool_calls: [mcp_A, client_B, mcp_C]`, `finish_reason: "tool_calls"`

**Step 2**: Classify — `mcp_A`, `mcp_C` are in the MCP tool_mapping; `client_B` is not

**Step 3**: Start background MCP execution:
```rust
let mcp_futures = vec![
    tokio::spawn(gateway.execute_tool(session, mcp_A)),
    tokio::spawn(gateway.execute_tool(session, mcp_C)),
];
```

**Step 4**: Store pending state in session:
```rust
session.pending_mixed = Some(PendingMixedExecution {
    full_assistant_message,        // The original message with ALL 3 tool_calls
    mcp_futures,                   // Background handles
    mcp_tool_call_ids: ["A", "C"],
    client_tool_call_ids: ["B"],
});
```

**Step 5**: Return to client with **only client tools**:
```json
{
    "choices": [{
        "message": { "tool_calls": [client_B_only] },
        "finish_reason": "tool_calls"
    }]
}
```

**Step 6**: Client executes `client_B`, sends new request with `tool_result_B`

**Step 7**: Match to session → find pending mixed execution

**Step 8**: Await MCP futures (may already be done):
```rust
let mcp_results = join_all(session.pending_mixed.mcp_futures).await;
```

**Step 9**: Reconstruct full history:
```
[..., assistant(all 3 tool_calls), tool_result_A(MCP), tool_result_B(client), tool_result_C(MCP)]
```

**Step 10**: Continue agentic loop

### Edge Cases

| Scenario | Handling |
|----------|----------|
| MCP tools finish before client responds | Results cached in session, `await` returns instantly |
| MCP tool fails while waiting | Error stored as tool result, LLM sees it on next iteration |
| Client never responds | Session expires via TTL, background tasks cancelled via `AbortHandle` |
| Client sends different request (not tool results) | No matching pending state → pending execution dropped, new loop starts |

---

## 7. Streaming

### The Problem Explained

In a normal streaming response, SSE events flow as the LLM generates tokens:
```
data: {"delta":{"content":"Hello"}}
data: {"delta":{"content":" world"}}
data: {"finish_reason":"stop"}
data: [DONE]
```

In an agentic loop, there are **multiple LLM calls**. The problem: after iteration 1 produces `finish_reason: "tool_calls"`, we need to execute tools and call the LLM again. But the SSE stream is a **single HTTP response** — we can't close it and open a new one. The client expects one continuous stream ending with one `[DONE]`.

Three possible approaches:

| Approach | How it works | Pros | Cons |
|----------|-------------|------|------|
| **Stream-only-final** | Buffer all intermediate iterations silently, only stream the last one | Simple, fully compatible | Client sees a delay (no output during tool execution), then streaming starts |
| **Multi-segment** | Stream ALL iterations in one SSE connection, suppress intermediate `finish_reason: "tool_calls"` | Client sees continuous output, natural pauses during tool execution | Client may see partial text from abandoned iterations; some clients buffer until `finish_reason` and would see concatenated responses |
| **Custom events** | Use non-standard SSE event types for intermediate status | Full visibility | Breaks standard OpenAI client libraries |

### Recommendation: Multi-Segment Streaming

Stream all segments through one SSE connection:
```
Segment 1 (iteration 1):
  data: {"delta":{"content":"Let me check..."}}
  data: {"delta":{"tool_calls":[...]}}
  // ← finish_reason:"tool_calls" is SUPPRESSED, not sent to client
  // [tool execution happens silently]

Segment 2 (iteration 2):
  data: {"delta":{"content":"I found 5 files..."}}
  data: {"finish_reason":"stop"}
  data: [DONE]
```

The client sees a continuous stream with natural pauses during tool execution. SSE keepalive comments (`:ping\n\n`) prevent timeout during pauses.

**For mixed tools**: When client tools are detected, stream up to `finish_reason: "tool_calls"` with only client tool_calls in the delta, then `[DONE]`. The continuation is a new HTTP request/stream.

### Implementation
Use `router.stream_complete()` for each iteration. Wrap in a custom stream adapter that:
1. Buffers chunks to detect `finish_reason`
2. Reassembles the full assistant message from deltas
3. If all MCP tools → suppress finish, execute tools, start next `stream_complete()`
4. If mixed → filter tool_calls to client-only, emit finish + `[DONE]`
5. If stop → emit normally

---

## 8. Request Resume on Timeout/Retry

### Problem
Client request times out during a long agentic loop. Client retries the exact same request. Without caching, we'd re-execute the entire loop.

### Solution: Request Resume Cache

```rust
struct ResumeCache {
    // Key: BLAKE3 hash of full request (messages + model + params)
    entries: DashMap<String, ResumeEntry>,
}

enum ResumeState {
    InProgress(watch::Receiver<Option<Response>>),  // Await the ongoing result
    Completed(Response),                             // Return immediately
    Failed(String),                                  // Return cached error
}
```

**Flow**:
1. Compute cache key from request
2. If exists as `InProgress` → subscribe and await (client rides along with the existing execution)
3. If exists as `Completed` → return cached response
4. If not exists → create `InProgress`, run loop, update to `Completed` on finish

Cache entries expire with session TTL.

---

## 9. MCP Feature Presentation to the LLM

Following the standard established by Claude Code and other MCP clients: **tools are injected as standard function tools** in the API request (JSON schemas in the `tools` array), never as text embedded in prompts. This ensures the LLM uses its native function calling capabilities.

### 9.1 Tools → Standard Function Tools
MCP tools are injected directly into the `tools` array of the chat completion request:
```json
{
    "type": "function",
    "function": {
        "name": "filesystem__read_file",
        "description": "Read a file from the filesystem",
        "parameters": { "type": "object", "properties": { "path": { "type": "string" } }, "required": ["path"] }
    }
}
```
The `NamespacedTool.input_schema` (JSON Schema) maps directly to `function.parameters`. No transformation needed — same format. Tool names use the existing `__` namespace separator (e.g., `filesystem__read_file`).

### 9.2 Resources → Exposed as Function Tools (Lazy)
Resources are NOT pre-fetched and injected into prompts (that would waste tokens). Instead, each resource is exposed as a synthetic function tool:
```json
{
    "type": "function",
    "function": {
        "name": "mcp_resource__filesystem__config_json",
        "description": "Read the resource 'config.json' from filesystem server",
        "parameters": { "type": "object", "properties": {} }
    }
}
```
When the LLM calls this tool, the orchestrator executes `resources/read` via the gateway and returns the resource content as the tool result. This is lazy — resources are only fetched when the LLM decides it needs them.

### 9.3 Prompts → System Message Enrichment + Function Tools
MCP prompts are templates that resolve to messages. Two strategies based on whether they require arguments:

- **No-argument prompts**: Fetched during session init via `prompts/get`, their returned messages are prepended to the conversation (typically as system messages). This follows the MCP spec's intent — prompts without arguments are pre-canned context.
- **Parameterized prompts**: Exposed as function tools so the LLM can invoke them with arguments:
  ```json
  {
      "type": "function",
      "function": {
          "name": "mcp_prompt__code_review__review_code",
          "description": "Get the 'review_code' prompt template",
          "parameters": { "type": "object", "properties": { "language": { "type": "string" } } }
      }
  }
  ```
  When called, the orchestrator fetches the prompt via `prompts/get` with the provided arguments and injects the returned messages into the conversation for the next LLM iteration.

### 9.4 Sampling (Nested LLM Calls)
During MCP tool execution, an MCP server may request sampling (an LLM completion). **No changes needed** — the gateway's existing sampling handler routes through `router.complete()`. Tool execution in the agentic loop goes through the same `handle_tools_call` path that already supports sampling callbacks.

### 9.5 Elicitation (User Input)
If a tool triggers elicitation, the existing `ElicitationManager` blocks via a oneshot channel until the user responds in the Tauri UI. The HTTP request simply hangs awaiting the tool result. Per user requirement: wait indefinitely, timeout is acceptable.

### 9.6 Notifications
- `notifications/tools/list_changed` → invalidate cached tool list, re-fetch on next iteration
- `notifications/resources/list_changed` → invalidate resource list, re-fetch on next resource tool injection
- Session subscribes to the `mcp_notification_broadcast` channel during creation

### 9.7 Summary: How Each Feature Maps

| MCP Feature | Presentation to LLM | Execution |
|-------------|---------------------|-----------|
| Tools | Standard function tools in `tools` array | `gateway.handle_tools_call()` |
| Resources | Synthetic function tools (lazy fetch) | `gateway.handle_resources_read()` on call |
| Prompts (no args) | Pre-resolved, injected as system messages | `gateway.handle_prompts_get()` at session init |
| Prompts (with args) | Synthetic function tools | `gateway.handle_prompts_get()` on call, inject returned messages |
| Sampling | Transparent (nested LLM calls during tool execution) | Existing gateway sampling handler |
| Elicitation | Transparent (blocks until user responds) | Existing `ElicitationManager` |
| Notifications | Transparent (cache invalidation) | Existing notification broadcast |

---

## 10. Naming Conflicts

MCP tools take precedence. When injecting MCP tools:
1. Build MCP tool set from gateway `tools/list`
2. If a client tool has the same name as an MCP tool → shadow the client tool (remove it)
3. Log a warning
4. All calls to that name route to MCP

This is safe because the client explicitly opted into `McpViaLlm` mode.

---

## 11. Firewall & Permissions

Firewall behavior stays as-is:
- `Allow` → proceed
- `Off` → deny, inject error as tool result
- `Ask` → popup in Tauri UI, wait indefinitely for user response
- Session-scoped approvals carry across iterations (existing `firewall_session_approvals`)

No special handling needed. The agentic loop simply waits for firewall resolution, same as any MCP client.

---

## 12. Session Configuration

```rust
pub struct McpViaLlmConfig {
    pub session_ttl_seconds: u64,          // default: 3600 (60 min last access)
    pub max_concurrent_sessions: usize,    // default: 100
    pub max_loop_iterations: u32,          // default: 25
    pub max_loop_timeout_seconds: u64,     // default: 300
    pub expose_resources_as_tools: bool,   // default: true
    pub inject_prompts: bool,              // default: true
}
```

Background cleanup task sweeps expired sessions every 60 seconds.

---

## 13. Response Format & OpenAI Protocol Compatibility

### Standard Response (always)
```json
{
    "id": "chatcmpl-abc123",
    "object": "chat.completion",
    "model": "gpt-4",
    "choices": [{
        "message": { "role": "assistant", "content": "The file contains 42 lines." },
        "finish_reason": "stop"
    }],
    "usage": { "prompt_tokens": 1500, "completion_tokens": 50 }
}
```
Indistinguishable from a normal response. Usage aggregated across all iterations.

### Mixed Tool Response
Only client tools appear. MCP tools are stripped:
```json
{
    "choices": [{
        "message": { "tool_calls": [client_tool_only] },
        "finish_reason": "tool_calls"
    }]
}
```

### Optional Metadata (in `extensions` field)
```json
{
    "extensions": {
        "mcp_via_llm": {
            "iterations": 3,
            "mcp_tools_called": ["filesystem__read_file"],
            "total_prompt_tokens": 450
        }
    }
}
```

---

## 14. Prerequisite Bug Fix

**CRITICAL**: `crates/lr-server/src/routes/chat.rs:1311-1312` hardcodes `tool_calls: None` and `tool_call_id: None` for ALL messages in `convert_to_provider_request()`. This drops tool call history from conversation messages, breaking multi-turn tool calling even without this feature. Must be fixed as a prerequisite — it needs to forward `msg.tool_calls` and `msg.tool_call_id` from the original `ChatMessage`.

---

## 15. Key Data Structures

### McpViaLlmSession
```rust
pub struct McpViaLlmSession {
    pub session_id: String,
    pub client_id: String,
    pub gateway_session_key: String,        // For MCP gateway session reuse
    pub history: SessionHistory,             // Per-message hashes + full messages + injection ranges
    pub pending_mixed: Option<PendingMixedExecution>,
    pub resume_cache: HashMap<String, ResumeEntry>,
    pub last_activity: Instant,
    pub ttl: Duration,
}

pub struct SessionHistory {
    pub message_hashes: Vec<u64>,           // BLAKE3 hash of each message (role + content + tool_calls)
    pub injected_ranges: Vec<Range<usize>>, // Indices of our injected tool call/result messages
    pub full_messages: Vec<ChatMessage>,    // Complete history including injected messages
}
```

### McpViaLlmManager (lives in AppState)
```rust
pub struct McpViaLlmManager {
    /// All sessions for a client, tried in order during matching
    sessions_by_client: DashMap<String, Vec<Arc<RwLock<McpViaLlmSession>>>>,
    /// Resume cache: request hash → session (for retry detection)
    resume_cache: DashMap<String, Arc<RwLock<McpViaLlmSession>>>,
    gateway: Arc<McpGateway>,
    router: Arc<Router>,
    config: McpViaLlmConfig,
}
```

---

## 16. Risks & Mitigations

| Risk | Mitigation |
|------|------------|
| Session match false positive (reuse wrong session) | Scoped to client_id; worst case injects extra context. Logged at INFO. |
| Session match false negative (new session when one exists) | Only loses ContextMode state; tool calls still work. Logged at INFO. |
| Infinite agentic loops | Max iterations (25) + timeout (5 min) |
| Token cost explosion | Usage tracking per iteration; configurable token budget |
| Background MCP tasks leak | `AbortHandle` on session expiry; cleanup task |
| Provider doesn't support tools | Detect via provider capabilities; skip injection |
| Streaming multi-segment confuses clients | Suppress intermediate `finish_reason`; only final `[DONE]` |

---

## 17. Key Files for Implementation

| File | Change |
|------|--------|
| `crates/lr-server/src/routes/chat.rs:264` | Injection point — early return for McpViaLlm |
| `crates/lr-server/src/routes/chat.rs:1311` | Bug fix — forward tool_calls/tool_call_id |
| `crates/lr-config/src/types.rs:1966` | Add `McpViaLlm` to `ClientMode` enum |
| `crates/lr-config/src/types.rs` | Add `McpViaLlmConfig` struct |
| `crates/lr-server/src/state.rs` | Add `McpViaLlmManager` to `AppState` |
| `crates/lr-mcp-via-llm/` (new crate) | All orchestration logic |
| `crates/lr-mcp/src/gateway/gateway.rs` | Reference: session management, `handle_request_with_skills()` |
| `crates/lr-mcp/src/gateway/gateway_tools.rs` | Reference: tool listing/calling API |
| `src/views/clients/client-detail.tsx` | UI: add McpViaLlm mode option with experimental badge |

---

## 18. Implementation Phases

**Phase 1 — Foundation**: McpViaLlm mode, injection point, basic agentic loop (non-streaming, MCP-only tools), session fingerprinting, history store/re-injection, config, experimental UI badge

**Phase 2 — Mixed Tools**: Tool classification, parallel background MCP execution, session state for pending mixed execution, history reconstruction on client tool result return

**Phase 3 — Streaming**: Multi-segment streaming adapter, tool_call delta buffering, mixed tool streaming (filter to client-only), SSE keepalive during tool execution

**Phase 4 — Full MCP Features**: Resources as tools, prompt injection, notification handling, request resume cache

**Phase 5 — Polish**: Usage aggregation, metrics/logging, session cleanup task, session management UI, end-to-end tests
