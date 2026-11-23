# MCP gateway follow-ups

Branch: `fix/mcp-gateway-followups`. Gaps left by the dead-code pass
(`2026-11-22-REMOVE_DEAD_CODE.md`), Push Relay P2 leftovers
(`2026-11-21-PUSH_RELAY_P2.md`) and an older gateway survey.

Coordination: `gateway/gateway.rs` is edited concurrently by the management
MCP server work; changes there stay small hook calls into new modules.
`lr-mcp-via-llm`, `client_auth.rs`, `admin_mcp*` and the MCP / try-it-out UI
are not touched.

## Findings per item (verified against the code first)

### 1. Sampling passthrough

The premise is only half right. The removed `SamplingPassthroughManager`
field (commit `d708673d`) was a leftover of the first design (`db3749a7`,
2026-03-10: the downstream client posts its answer to a separate route). The
2026-03-19 redesign (`2026-03-19-MCP_PROTOCOL_FEATURE_GAPS.md`, `f00aa89a`)
replaced it with a server→client request over the session's SSE stream:
`McpGateway::client_request_sender`, wired in `lr-server/src/state.rs` to
`SseConnectionManager::send_request`, answered by the client's JSON-RPC
response POST (`resolve_server_request`). That path is live: an upstream's
`sampling/createMessage` reaches the per-session request callback
(`register_request_handlers_on_transports`), which applies the global
`mcp_gateway.sampling` behaviour (Passthrough / DirectAllow / DirectAsk / Off)
and the client mode, and forwards to the session's stream.

Per-client `mcp_sampling_permission` is a deserialize-only migration shim
since 2026-03-19 (the setting became global); the per-client control that
remains is the firewall's Monitor intercept (DirectAllow → DirectAsk). Newer
plans: sampling is Deprecated in MCP 2026-07-28 (SEP-2577); keep it working
for old peers, build nothing new on it (2026-07-02 gap analysis, 2026-07-03
implementation: no MRTR bridging for sampling). So the passthrough design is
kept; nothing is resurrected.

Gaps found and fixed:
- Forwarded to a client that never declared the `sampling` capability (the
  gateway always advertises sampling upstream), and with no stream the call
  just failed. Now: forwarded only when the client declared `sampling` and has
  a stream; otherwise a `Both` client falls back to LocalRouter's own routing
  behind the approval popup (DirectAsk), and an `McpOnly` client gets a clear
  error.
- No cancellation: an upstream `notifications/cancelled` for a forwarded
  request was rebroadcast raw with the upstream's id (meaningless to the
  client). Now it cancels the wait and the client receives
  `notifications/cancelled` with the id it saw. A passthrough that times out
  also cancels it at the client.
- `SseConnectionManager::pending_server_requests` kept every entry whose
  waiter timed out or whose stream went away (unbounded). See item 7.
- Same treatment for elicitation passthrough (shared helper).

### 2. Bridge GET-stream registration tests

`bridge/stdio_bridge/duplex_tests.rs` covers the bridge against a fake
gateway; nothing covered the real route's side (`mcp_gateway_get_handler`
registering the stream under the Streamable HTTP session key). New
`src-tauri/tests/mcp_bridge_stream_tests.rs` runs the real server and the real
bridge (in-memory stdio) against a local mock upstream.

### 3. Push Relay SEP-3415 optional items

Spec text read from the Final SEP (`seps/3415-events-extension.md` at
`8a18b4b3`) and the 2026-07-28 Streamable HTTP transport ("Custom Headers from
Tool Parameters"):
- `Mcp-Param-{Name}` on upstream `events/poll` / `events/stream` (the relay
  sends no upstream `events/subscribe` / `unsubscribe`) for every
  `x-mcp-header`-annotated property with a value in `arguments`, encoded as
  the transport says (string as is, integer decimal, boolean lowercase,
  `=?base64?…?=` otherwise). Event types whose annotations violate the
  constraints are excluded from the catalogue with a warning.
- `heartbeatMs` suggested on a downstream `events/stream` is honoured within
  bounds and the grant is reported on `notifications/events/active`.

### 4. `resources/read` without `resources/list`

Partly wrong: a read by URI already listed on demand, but only once per
session (a later miss, e.g. after `list_changed`, failed), a read by the
gateway's namespaced `name` had no fallback at all, and `resources/subscribe`
demanded a prior list.

### 5. SSE transport notification callback

Wrong: notifications are delivered (persistent stream, POST SSE bodies, RPC
streams); only the doc comment was stale. A server→client request inside a
POST SSE body was awaited inline, so the body was not read meanwhile (an
upstream's `notifications/cancelled` for it could not arrive).

### 6. Counts in the merged server description

True: `build_server_description` had a placeholder comment. Counts are known
after the initial catalog refresh.

### 7. Bug hunt

See "Bugs found" below.

## Design decisions

- Passthrough target check uses the client's declared capabilities stored on
  the session (`client_capabilities`).
- Fallback for `Both` clients is DirectAsk (popup), never silent routing.
- Cancellation registry: `gateway/server_requests.rs`, keyed by
  (session, server, upstream request id), entries removed by a drop guard.
- Param headers reach the HTTP transport through a reserved `_meta` key
  (`io.localrouter/mcpParamHeaders`) that every transport removes before the
  request is serialized.
- Heartbeat bounds: granted = clamp(suggestion, 5 s, 120 s), default the
  configured `downstream_heartbeat_secs`; the lower bound never exceeds the
  configured default.

## Implementation

| Item | Where |
| --- | --- |
| 1 | `gateway/server_requests.rs` (target check, forward with cancellation / timeout notice, early-cancel memory, approval helper); hooks in `gateway.rs` request callback (sampling + elicitation passthrough arms) and notification callback (`notifications/cancelled`) |
| 2 | `src-tauri/tests/mcp_bridge_stream_tests.rs` |
| 3 | `transport/param_headers.rs`; `transport/sse.rs` sends, `stdio.rs` / `websocket.rs` strip; `push_relay/relay.rs` (`CatalogueEntry::param_headers`, catalogue exclusion, `heartbeatMs` parsing), `push_relay/upstream.rs` (`SubscriptionSpec::param_headers`), `push_relay/limits.rs` (`granted_downstream_heartbeat`), `push_relay.rs` (warning when the catalogue is read) |
| 4 | `gateway/resource_routing.rs`, `gateway_resources.rs` (read and subscribe) |
| 5 | `transport/sse.rs` (`read_inline_response` spawns the answer; doc comment) |
| 6 | `merger.rs` (`build_server_description` with `CatalogCounts`), `session.rs` (`record_catalog_counts`), hooks in `gateway.rs` (catalog refresh, initialize, recovery) |
| 7 | `lr-server/src/state.rs` (`SseConnectionManager`), `lr-server/src/routes/mcp.rs` (`GetStreamCleanup`, DELETE) |

Initialize may answer before the backends finish (`_meta.initializing`); the
counts are then only in the session's merged capabilities, which later
initializes and the UI read.

## Checklist

- [x] Plan saved and committed
- [x] Item 5 (comment, inline request concurrency)
- [x] Item 6 (description counts)
- [x] Item 4 (resource resolution without a prior list)
- [x] Item 3 (Mcp-Param headers, heartbeatMs)
- [x] Item 1 (sampling passthrough gaps, cancellation)
- [x] Item 2 (bridge stream route tests)
- [x] Item 7 (bug hunt fixes)
- [x] Validation (fmt, clippy, lr-mcp, lr-server, push_relay_e2e, new tests)
- [x] Plan Review
- [x] Test Coverage Review
- [x] Bug Hunt (re-read of this branch's diff)
- [x] Commit and push

## Bugs found (all fixed, with regression tests)

1. **GET SSE cleanup never ran on client disconnect.** The cleanup sat after
   the stream's loop; axum drops the stream mid-await when the client goes, so
   a legacy SSE connection's gateway session (and its upstream transports /
   processes) was never terminated and the registration stayed. POSTs for
   that key were then accepted with 202 and their answers sent into a dead
   channel. Now a drop guard (`GetStreamCleanup`). Test:
   `legacy_sse_disconnect_unregisters_and_ends_the_session`.
2. **A replaced GET stream never ended.** `register_tracked` returned a clone
   of the sender, so the old stream's channel never closed; two streams of one
   session both received its broadcast notifications (duplicate delivery).
   Registrations are now identified by an id (`SseConnectionHandle`), the map
   holds the only sender. Tests: `sse_replaced_connection_ends_and_leaves_its_successor`,
   the reconnect part of `bridge_get_stream_registers_delivers_reconnects_and_cleans_up`.
3. **`pending_server_requests` grew without bound**: an entry whose waiter
   timed out (120 s passthrough) was never removed. Pruned on every request
   and when a connection goes; awaited ones stay (a Streamable HTTP client may
   answer by POST after its stream reconnected). Test:
   `sse_pending_server_requests_stay_bounded`.
4. **`gateway_task_handles` never shrank** (one abort handle per legacy
   connection that sent `initialize`). Removed with the connection.
5. **DELETE left the session's GET stream open** until its socket was found
   dead. DELETE now unregisters it (the stream ends).
6. **Server→client request in a POST SSE body blocked reading that body**
   (item 5): the upstream's cancellation for it could not be read meanwhile.
   Test: `post_sse_request_is_answered_while_the_body_is_still_read`.
7. **Passthrough to clients without the capability; wrong-id cancellation;
   no timeout notice** (item 1). Elicitation passthrough now also requires the
   client's `elicitation` capability (else Direct, as with no stream).
8. **`resources/subscribe` did no permission check of its own**; with
   on-demand resolution it now applies the read rule (`resolve_resource`).
9. **Cancellation racing its request**: the HTTP transport hands a request and
   the cancellation right behind it to two tasks; an early cancellation is
   remembered (30 s, at most 256) and the request is then never shown.

## Remaining / not done

- The `Both` fallback (client without sampling → approval popup → LLM
  routing) has unit coverage of its parts only; no end-to-end test drives the
  popup.
- `mcp_sampling_max_tokens` / `mcp_sampling_rate_limit` (per client, no UI)
  are still not enforced.
- `refresh_resource_templates` namespaces template names with the server id
  (UUID) while resources use the server slug: inconsistent, left alone.
- The route's handling of a *client*-sent `sampling/createMessage`
  (`routes/mcp.rs`, broadcast with `passthrough_request_id`, answered on
  `sampling_passthrough_response_handler`) is the 2026-03-10 design's leftover;
  untouched (MCP clients do not send sampling requests to servers).
- Relay-suggested `heartbeatMs` toward upstreams is not sent (optional).
