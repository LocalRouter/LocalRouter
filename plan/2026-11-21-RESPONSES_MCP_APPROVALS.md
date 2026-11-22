# `/v1/responses`: firewall approvals and MCP-via-LLM parity with chat

Date: 2026-11-21
Branch: `feat/responses-mcp-approvals`

## Goal

`/v1/responses` must go through the same firewall-approval and MCP-via-LLM
machinery as `/v1/chat/completions`, for streaming and non-streaming requests,
by sharing the chat path's code. Reasoning items and custom tools stay
pass-through. The MITM / reverse proxy path is not touched.

## Findings: where the Responses path diverges

`create_response` (`crates/lr-server/src/routes/responses.rs`) already
translates to a `ChatCompletionRequest`, runs `pipeline::run_turn_pipeline`,
and dispatches to `McpViaLlmManager` for MCP-via-LLM clients. So the header
comment overstates the gap. What is actually missing or wrong:

### Approvals

| Approval | Chat | Responses (before) |
|---|---|---|
| Auto-router popup (`localrouter/auto`, permission Ask) | `apply_model_access_checks` | Same code, works. Denial event labelled `/v1/chat/completions` (hardcoded). |
| Model permission Ask, normal client | `apply_model_access_checks` | Same code, works. |
| Model permission Ask, **MCP-via-LLM client** | `apply_model_access_checks` skips it on purpose; `chat.rs::handle_mcp_via_llm` runs it later with the injected MCP tools shown in the popup | **Never runs.** The pipeline skips it and `responses.rs` has no second pass, so an Ask or Off model permission is bypassed entirely. |
| Secret-scan Ask | `run_secret_scan_check` in the pipeline | Same code, works. |
| Guardrail approval | parallel handle or sequential | Sequential inside the pipeline (`PipelineCaps::responses`), works. |
| MCP tool approval | inside the gateway, reached through the orchestrator | Same code, works once the orchestrator runs. |
| Free-tier paid-fallback approval | `check_free_tier_fallback` + `*_with_paid_fallback` retry | **Missing.** `AppError::FreeTierFallbackAvailable` is returned to the client as an error; no popup, no retry. |

Also: chat skips the orchestrator on a duplicate gateway hop
(`!is_duplicate`); Responses ignores `TurnContext::is_duplicate` and would run
the agentic loop a second time.

Side finding in the shared code: `handle_mcp_via_llm` applies the popup's
edits to the `ChatCompletionRequest` after the provider request was already
built, so edits made in the model popup never reach the provider for
MCP-via-LLM clients.

### MCP-via-LLM

1. **Gateway-executed tool calls leak into the Responses stream.** The
   streaming orchestrator forwards every non-finish chunk, including
   `delta.tool_calls` of MCP tools it then executes itself, and only
   suppresses the `finish_reason`. A chat client sees stray deltas; the
   Responses emitter turns each into a `function_call` output item
   (`response.output_item.added` / `.done`, and in `response.completed`'s
   `output`), which tells the client it has a call to answer.
2. **Client tool calls are emitted twice on that path.** For client-owned
   calls the orchestrator forwards the deltas and then sends a finish chunk
   carrying the complete calls again (`build_finish_chunk_with_tools`). The
   emitter appends both, so `arguments` are doubled.
3. **Stored history is corrupt after such a turn.** `build_stream_response`
   accumulates every tool-call delta into the assistant message saved for
   `previous_response_id`. A turn with gateway-executed calls is stored as an
   assistant message with `tool_calls` and no tool results; replaying it is an
   invalid conversation for most providers.
4. **The deterministic session key never matches.** Responses passes
   `previous_response_id` as the orchestrator's explicit session key, but a
   session is only ever stamped with the id it was *looked up* by, never with
   the id of the response it *produced*. Turn N+1 therefore looks up an id no
   session carries and falls back to hash matching, which fails whenever the
   client resends `instructions` (the stored history already holds the earlier
   system message, so the prefix differs). The hidden tool history and the
   gateway session are then lost.

Non-streaming is correct on points 1-3: the orchestrator returns only the
final message (or only the client-owned calls for a mixed turn).

## Design

### Approvals

- `pipeline.rs`: new `check_mcp_via_llm_model_firewall` holding the block that
  lived in `handle_mcp_via_llm` (pre-fetch MCP tools for the popup, model
  firewall check, apply edits). It rebuilds the provider request when the user
  edited the request, so edits take effect. Called by `chat.rs` and
  `responses.rs`. No change near rate-limit code.
- `pipeline.rs`: `apply_model_access_checks` takes the endpoint label instead
  of hardcoding `/v1/chat/completions`.
- `responses.rs`: honour `is_duplicate`; call the shared firewall pass before
  dispatch; on `FreeTierFallbackAvailable` call chat's
  `check_free_tier_fallback` and retry with the paid-fallback router entry
  points (streaming and non-streaming).

### MCP-via-LLM

- `lr-mcp-via-llm`: `TurnOptions` replaces the trailing
  `explicit_session_key` argument of `handle_request` /
  `handle_streaming_request`:
  - `resume_session_key`: key to look the session up by (Responses:
    `previous_response_id`).
  - `session_key`: key stamped on the session for the next turn (Responses:
    the id of the response being produced).
  - `withhold_tool_call_deltas`: streaming only. Tool-call deltas are held
    back until the calls are classified. Gateway-executed calls are never
    sent; client-owned calls arrive once, complete, in the finish chunk.
  Chat passes `TurnOptions::default()`, which keeps its behaviour unchanged.
- `responses.rs`: pass the options above. With deltas withheld the emitter
  only ever sees client-owned calls, so the wire output, the
  `response.completed` object and the stored history all exclude
  gateway-executed calls. Item ids stay `fc_<response_id>_<index>` and are
  stable across `added` / `arguments.delta` / `done`.
- Session DB path is taken from a helper so tests use a temp directory.

### Out of scope / remains

- Reasoning items and custom (non-function) tool calls in `input` are still
  dropped during translation; native Responses backends still pass their
  output through verbatim.
- `instructions` of an earlier turn stay in stored history (pre-existing).
- Hidden tool history lives in the in-memory orchestrator session, not in the
  Responses session DB: after a restart or session expiry a continuation
  replays only the client-visible history.
- Chat streaming still forwards MCP tool-call deltas (unchanged on purpose).

## Review

### Plan Review

Every design item is implemented: `TurnOptions` (manager.rs, exported from
lib.rs), delta withholding (`strip_tool_call_deltas` in
orchestrator_stream.rs), session re-keying, `check_mcp_via_llm_model_firewall`
used by chat and responses, endpoint label on `apply_model_access_checks`,
duplicate-hop skip, free-tier paid-fallback approval (`route_complete` /
`route_stream`), test-only session DB path, accurate header comment. A side
fix fell out of the continuation test: `reconstruct_history` now keeps a
system message the client first sends on a later turn (it was dropped when
the stored history had no client system message).

### Test Coverage Review

- `lr-mcp-via-llm`: chained response ids stay on one session; an unknown id
  starts a new one; `strip_tool_call_deltas` (tool-only chunk dropped, text /
  role kept, empty chunk untouched); late system message reconstruction.
- `lr-server` (`routes/responses_tests.rs`, real router, gateway, firewall
  manager and orchestrator with a scripted provider and a virtual MCP
  server): plain request JSON and event sequence; plain streaming client
  tool call; model-permission Ask (normal and MCP-via-LLM client x stream x
  Allow/Deny, popup shows injected tools); popup edits reach the provider;
  auto-router Ask; free-tier paid-fallback Ask; MCP tool approval
  Allow/Deny; MCP tool loop non-streaming and streaming (event order,
  sequence numbers, stable item id, no gateway tool in any event);
  continuation via the orchestrator session and via stored history only;
  client tool round trip; mixed gateway + client calls.
- Not covered by a route test: Secret Scan Ask and guardrail approvals.
  Both run in `run_turn_pipeline`, which this change does not touch and
  which already had them for `/v1/responses`.

### Bug Hunt

- Re-keying happens when the turn starts. If the turn then fails, the
  session carries the id of a response that was never stored; a retry that
  names the older `previous_response_id` falls back to hash matching. Two
  continuations branching from one response id race the same way (the
  later one wins the key). Both degrade to the pre-existing hash matching,
  not to wrong history.
- Streaming MCP-via-LLM on `/v1/responses` finalizes the endpoint monitor
  event with the Responses view (`skip_monitor_completion: false`), as it did
  before; chat skips it so the orchestrator's per-iteration view stays.
  Unchanged.
- Rebuilding the provider request after popup edits keeps
  `pre_computed_routing`; the credential-scope cache extension is only set
  for non-MCP-via-LLM clients, so nothing else is lost.

## Checklist

- [x] Read chat / pipeline / orchestrator / responses paths; record findings
- [x] Save this plan
- [x] `TurnOptions` + session key stamping + delta withholding in `lr-mcp-via-llm`
- [x] Shared `check_mcp_via_llm_model_firewall`; endpoint label
- [x] `responses.rs`: duplicate hop, firewall pass, free-tier approval, options
- [x] Header comment in `responses.rs` made accurate
- [x] Tests: `lr-mcp-via-llm` (session key, withholding)
- [x] Tests: `lr-server` route-level (approval Allow/Deny streaming + not,
      MCP loop output and event sequence, continuation, plain request)
- [x] OpenAPI: no request/response type change; nothing to update
- [x] Plan Review: compare implementation against this plan, close gaps
- [x] Test Coverage Review: uncovered paths in new/modified code
- [x] Bug Hunt: re-read the diff for races, off-by-one, state errors
- [ ] fmt, clippy (workspace, all targets), `cargo test -p lr-server -p lr-mcp-via-llm`
- [ ] Commit and push the branch
