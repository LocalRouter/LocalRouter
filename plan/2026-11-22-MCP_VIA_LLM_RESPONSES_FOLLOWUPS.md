# MCP-via-LLM, `/v1/responses` and `/v1/chat/completions` follow-ups

Date: 2026-11-22
Branch: `fix/mcp-via-llm-responses-followups`

Leftovers from `2026-11-21-RESPONSES_MCP_APPROVALS.md` (its Review section)
and `2026-11-22-REMOVE_DEAD_CODE.md` (fields it deleted because nothing read
them), plus a bug hunt over `crates/lr-mcp-via-llm` and the chat / responses
routes. Out of scope: MCP sampling passthrough (`lr-mcp`), rate-limit
admission, native body limits.

## Findings (verified in code before changing anything)

1. **Chat streaming leaks gateway calls and doubles client calls — confirmed.**
   `orchestrator_stream::streaming_loop` forwards every non-finish chunk when
   `withhold_tool_call_deltas` is false (chat's default). A chat client sees
   the `delta.tool_calls` of gateway-executed calls, then (for a client-owned
   call) `build_finish_chunk_with_tools` sends the complete call again in the
   finish chunk, re-indexed from 0. An accumulating client therefore gets
   every argument twice, and when a gateway call precedes the client call the
   finish chunk's index 0 collides with the gateway call's deltas.
2. **Mixed resume drops totals — confirmed.** `resume_after_mixed` reran the
   loop with only `accumulated_usage_entries`; iterations and
   `mcp_tools_called` from before the mixed call were lost (the dead-code pass
   then deleted the unread fields). Streaming built the pending execution with
   no usage entries and no tools at all; the streaming resume path
   (`response_to_chunk`) dropped `usage` and `extensions` entirely, so chat
   and responses estimated usage and lost the metadata.
3. **Error monitor events lack session/endpoint — confirmed, wider than
   documented.** The fields were removed from `TurnContext`. In practice:
   the orchestrator's per-iteration `LlmCall` events hardcode
   `/v1/chat/completions` (wrong for `/v1/responses`); a failed iteration
   leaves its event `Pending` forever; the resume path passes no monitor
   session, emitter or event id at all; chat's MCP-via-LLM handler consumes
   the `LlmCallGuard` before the orchestrator runs, so any failure (firewall
   denial, guardrail failure, orchestrator error) leaves the request's
   `LlmCall` event `Pending`.
4. **Re-keying at turn start — confirmed.** `TurnOptions::session_key` is
   stamped when the turn starts. A failed turn leaves the session keyed to a
   response that was never stored; two continuations from one
   `previous_response_id` share and overwrite one session (the second finds
   no key, falls back to hash matching and may graft the first branch's
   history).
5. **Responses overwrites the orchestrator's monitor view — confirmed.**
   Streaming MCP-via-LLM sets `skip_monitor_completion: false` and on error
   calls `complete_llm_call_error`, replacing the iteration-0 detail.
6. **`instructions` persist — confirmed.** The instructions system message is
   stored in the session row and in the orchestrator history; the next turn
   prepends its own instructions, so the old ones stay.
7. **Reasoning and custom tool items dropped — confirmed.**
   `response_item_to_server_message` returns `None` for `reasoning`,
   `custom_tool_call`, `custom_tool_call_output`.
8. **Text before a gateway call — confirmed.** Streaming folds every
   iteration's text into one `message` item; non-streaming returns only the
   final message. The stored assistant message is the concatenation, which
   also breaks the orchestrator's history anchor on continuation.

## Design

1. Streaming orchestrator routes tool-call deltas per call: a call is held
   until its name is known, then classified. Gateway-executed (and unknown)
   calls are never forwarded. Client calls are forwarded with a client-side
   index (0..n over client calls only) — as deltas for chat, or held and
   sent once in the finish chunk when `withhold_tool_call_deltas` is set
   (Responses). The finish chunk only carries calls not yet sent, so nothing
   is sent twice. Finish reason for client calls is always `tool_calls`.
2. `PendingMixedExecution` carries `PriorTurnTotals` (iterations, tokens,
   tools called, usage entries) from both orchestrators; the resumed loop
   starts from them. `usage` stays per HTTP request (the first response was
   already billed); the `mcp_via_llm` extension and `request_usage_entries`
   cover the whole tool loop. Streaming resume emits a chunk with `usage`
   and `extensions`; the streaming loop also reports the `mcp_via_llm`
   extension on its last chunk.
3. `TurnContext` gets `session_id` and `endpoint` back and the routes pass
   them in `TurnOptions::monitor_endpoint` / the monitor session.
   Per-iteration events use the endpoint; failed iterations are completed
   as Error; resume threads monitor callbacks through. Chat keeps the
   `LlmCallGuard` until the orchestrator returns so failures complete the
   request's event; chat maps orchestrator errors through
   `ApiErrorResponse::from` (status preserved) instead of a blanket 502.
4. Keyed sessions (Responses): lookup by `resume_session_key` only (no hash
   fallback, and chat's hash matching ignores keyed sessions). A hit is
   forked: the turn runs on a copy (same gateway session) and the original
   keeps its key and history, so branches each get the right history. The
   new key is stamped only when the turn succeeds (non-streaming: after the
   orchestrator returns Ok; streaming: after the final chunk was delivered).
   Sessions per client are capped (LRU).
5. Responses streaming MCP-via-LLM: `skip_monitor_completion: true`, and
   stream errors mark the event failed without replacing its body.
6. Responses route stores history without the leading instructions message;
   keyed orchestrator history drops this turn's instructions when the turn
   is recorded and reconstruction puts the current turn's instructions in
   front (`TurnOptions::leading_instructions`).
7. Reasoning items: carried in the request extension
   `localrouter.responses.reasoning_items`, anchored to the next function
   call (`call_id`) or assistant message; `translate_completion_request`
   re-emits them as `reasoning` input items, so only providers whose
   upstream API is the Responses API (structured: `upstream_api()`) get
   them; chat providers do not. Custom tool calls/outputs become chat tool
   calls/tool messages; the Responses translator emits `custom_tool_call` /
   `custom_tool_call_output` when the request declares the tool as
   `type: "custom"`. Custom tool definitions keep their `format`.
8. Separate `message` items per assistant message (OpenAI Responses emits
   one output item per message, with hosted tool items in between).
   Streaming: `TurnOptions::separate_assistant_messages` makes the
   orchestrator send an iteration-boundary chunk; the emitter closes the
   current message item and the next text opens a new one. Non-streaming:
   the orchestrator attaches the earlier messages' text as an extension the
   route turns into leading `message` items. Stored history holds one
   assistant message per item.

## Checklist

- [ ] 1 chat streaming delta routing + tests
- [ ] 2 prior totals on resume (chat/responses, stream/non-stream) + tests
- [ ] 3 monitor endpoint/session/error completion + chat guard + tests
- [ ] 4 keyed session fork, re-key on success + tests
- [ ] 5 responses monitor skip + test
- [ ] 6 instructions not carried over + tests
- [ ] 7 reasoning / custom tool items + tests
- [ ] 8 separate message items + tests
- [ ] 9 bug hunt fixes + tests
- [ ] OpenAPI check
- [ ] Plan Review
- [ ] Test Coverage Review
- [ ] Bug Hunt
- [ ] fmt, clippy, tests
- [ ] Commit and push
