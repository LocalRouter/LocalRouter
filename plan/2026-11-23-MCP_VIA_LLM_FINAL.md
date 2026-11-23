# MCP-via-LLM final fixes (chat / Responses)

Date: 2026-11-23
Branch: `fix/mcp-via-llm-final`

The "Found, not fixed" list of `2026-11-23-MCP_VIA_LLM_LEFTOVERS.md`, plus a
bug hunt over the code touched.

## Findings (verified in code before changing anything)

1. **Streaming failures other than router errors — confirmed.**
   `orchestrator_stream::stream_error` turns every non-router
   `McpViaLlmError` into `AppError::Internal`, so a guardrail denial, loop
   timeout or max-iterations failure reaches a streaming client as an
   `internal_error` (Responses: `response.failed` with `type:
   internal_error`, no code; chat: `type: server_error`, code
   `streaming_error`). Non-streaming maps them to 403 `permission_error`
   / `guardrail_denied`, 504 `timeout_error`, 502 `provider_error`
   (`From<McpViaLlmError> for ApiErrorResponse`). The pass-through
   `gated_stream` denial goes through the same function.
2. **Translated Responses stream events — confirmed.** `ResponsesEmitter`
   emits `response.created`, `output_item.added`, `output_text.delta`,
   `function_call_arguments.delta`, `output_item.done`, and the terminal
   event. Missing against OpenAI's sequence: `response.in_progress`,
   `response.content_part.added` / `.done`, `response.output_text.done`,
   and `response.function_call_arguments.done`. A message still open when
   a function call starts is closed only at the end, so items overlap
   (native upstreams finish one output item before adding the next).
3. **Loop totals read only `chunk.usage` — confirmed.**
   `streaming_loop` keeps `iteration_usage` from `chunk.usage` only;
   `StreamTracker` (lr-server) also reads `extensions.usage` through a
   private helper. A provider that reports usage only in
   `extensions.usage` leaves the `mcp_via_llm` totals at zero and its
   iteration monitor events without tokens.
4. **Gate task left running — confirmed.** `GuardrailGate` is a bare
   `JoinHandle`; dropping it detaches the task. The gate is dropped without
   being awaited when the first LLM call fails (streaming: before the gate
   is moved into the chunk loop; non-streaming: `?` on the router call),
   when the upstream stream errors, when the client disconnects (the loop
   future / handler future is dropped), and when the chat route fails
   after spawning it (client lookup, model firewall). The approval popup
   the task raised (`request_guardrail_approval`) stays open until
   answered.
5. **Pending mixed execution in memory only — confirmed.**
   `PendingMixedExecution` lives in `McpViaLlmManager::pending_executions`;
   after a restart the client's tool results match nothing and the turn
   continues from the visible history only (the gateway calls of the
   stopped turn and their results are lost).

## Design

1. New additive `AppError::Classified { status, error_type, code, message }`
   in `lr-types`: a failure whose HTTP status/type/code were decided where
   it happened, for paths that can only carry an `AppError` (a stream's
   error item). `McpViaLlmError::into_app_error()` is the one mapping
   (router errors unchanged; guardrail 403 `permission_error` /
   `guardrail_denied`; timeout 504 `timeout_error`; the rest 502
   `provider_error`); the non-streaming `ApiErrorResponse` conversion and
   the streaming `stream_error` both use it. `ApiErrorResponse` maps the
   variant to its status/type/code. MCP-via-LLM chat streams send the
   error body as non-streaming does (type and code kept, plus a numeric
   `status`); Responses `response.failed` carries `type`, `status` and a
   `code` (the error's code, else its type: `code` is required there).
2. `ResponsesEmitter` emits the full sequence: `created`, `in_progress`;
   per message `output_item.added`, `content_part.added`,
   `output_text.delta`…, `output_text.done`, `content_part.done`,
   `output_item.done`; per function call `output_item.added`,
   `function_call_arguments.delta`…, `function_call_arguments.done`,
   `output_item.done`; then the terminal event. An open message is closed
   before a function call item opens. Native pass-through is unchanged
   (envelopes forwarded verbatim).
3. `lr_providers::usage_estimate::chunk_usage`: `chunk.usage`, else an
   OpenAI-shaped `extensions.usage` (never both — no double count). Used
   by `StreamTracker` and the streaming orchestrator.
4. `GuardrailGate` becomes a newtype over the `JoinHandle` that aborts the
   task on drop (and is a `Future`). Aborting drops the
   `request_guardrail_approval` future, whose `PendingRequestGuard`
   removes the pending approval and broadcasts
   `firewall/approvalResolved` (the existing cancellation mechanism).
5. Persisted pending mixed executions (Responses only):
   - `StoredPendingMixed` (serde) in `lr-mcp-via-llm`: the assistant
     message with every call, client call ids, messages before the call
     (instructions and gateway instructions stripped), the gateway
     results finished so far, prior totals.
   - `TurnOptions::persist_pending` hook: called when the turn stops for a
     mixed call and again as each background gateway call finishes
     (serialized under one lock, so the last write is the latest state).
   - `lr-responses-sessions`: additive table `responses_pending_mixed`
     (`response_id` PK, `client_id`, `updated_at`, `pending_json`), upsert,
     owner-checked read, swept with the retention. Capped at
     `MAX_REPLAY_BYTES` (larger: not stored).
   - The route installs the hook only for `store: true` on a client with
     retention; reads it for `previous_response_id` only when that
     response is active for the client; passes it as
     `TurnOptions::resume_pending`.
   - `prepare_turn`: when no in-memory pending execution matches, a
     stored one whose client call ids match the request's tool results
     is resumed; gateway calls with no stored result get an error result
     saying their outcome is unknown (never re-executed).
   - Chat completions: not resumable across a restart (no server-side id
     and no stored conversation: chat persists nothing, by design). After
     a restart the turn continues from the visible history.

## Checklist

- [x] Plan saved
- [ ] 1 streaming error classification + tests
- [ ] 2 full Responses event sequence + tests
- [ ] 3 extension usage in loop totals + tests
- [ ] 4 gate abort on drop + popup dismissal + tests
- [ ] 5 persisted pending mixed execution + tests
- [ ] 6 bug hunt fixes + tests
- [ ] OpenAPI check
- [ ] Plan Review
- [ ] Test Coverage Review
- [ ] Bug Hunt
- [ ] fmt, clippy, tests
- [ ] Commit and push
