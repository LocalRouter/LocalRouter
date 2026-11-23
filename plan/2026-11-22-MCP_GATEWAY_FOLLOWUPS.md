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

## Checklist

- [ ] Plan saved and committed
- [ ] Item 5 (comment, inline request concurrency)
- [ ] Item 6 (description counts)
- [ ] Item 4 (resource resolution without a prior list)
- [ ] Item 3 (Mcp-Param headers, heartbeatMs)
- [ ] Item 1 (sampling passthrough gaps, cancellation)
- [ ] Item 2 (bridge stream route tests)
- [ ] Item 7 (bug hunt fixes)
- [ ] Validation (fmt, clippy, lr-mcp, lr-server, push_relay_e2e, new tests)
- [ ] Plan Review
- [ ] Test Coverage Review
- [ ] Bug Hunt (re-read of this branch's diff)
- [ ] Commit and push

## Bugs found

(filled in as found)
