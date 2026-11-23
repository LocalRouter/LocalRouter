# MCP gateway leftovers

Branch: `fix/mcp-gateway-leftovers`. Items left open by
`2026-11-22-MCP_GATEWAY_FOLLOWUPS.md` ("Remaining / not done"), Push Relay P2
open questions (`2026-11-21-PUSH_RELAY_P2.md`) and the LocalRouter management
MCP server plan (`2026-11-22-LOCALROUTER_ADMIN_MCP.md`).

Not touched: `crates/lr-mcp-via-llm`, `crates/lr-proxy`, `src-tauri/src/ui/admin_mcp*`.

## Findings per item (verified against the code first)

### 1. Per-client sampling limits

True. `Client::mcp_sampling_max_tokens` / `mcp_sampling_rate_limit` are real,
serialized config fields (not deserialize-only shims: unlike
`mcp_sampling_permission` / `mcp_sampling_enabled` they have no
`skip_serializing`), with no UI, read nowhere outside tests. No plan retired
them; the 2026-03-19 redesign moved only the *permission* to the global
setting. Decision: enforce them for every sampling request an upstream sends
through a client's session, whatever answers it (passthrough to the client,
LocalRouter's LLM routing, or the `Both` fallback):

- `maxTokens` above the limit → JSON-RPC `-32602` naming the limit (the
  request is refused, not silently clamped: the server asked for an amount the
  client's owner did not allow).
- More than `rate_limit` sampling requests within a sliding hour (per client,
  across its sessions) → `-32000` with `data.retryAfterSecs`. A refused
  request (over the token limit, or rate limited) uses no slot.

Limits are read live from the client manager at call time (no session
staleness after a config edit).

### 2. `Both` fallback end-to-end

True: only unit coverage. New E2E test drives a real server, a mock upstream
whose tool samples, the bridge without the `sampling` capability, the
approval popup answered through `SamplingApprovalManager::submit_approval`,
and an OpenAI-compatible mock provider.

### 3. Template names

True: `refresh_resource_templates` namespaced template names with the server
UUID while tools, resources and prompts use `slugify(server name)`
(`build_server_id_to_name_mapping`). Template names are display-only — nothing
resolves a template by name (`resources/read` and `completion/complete`'s
`ref/resource` route by URI) — so the switch to the slug needs no alias for
old names.

### 4. Client-sent `sampling/createMessage` on the route

Reachable but wrong. A downstream MCP client sending `sampling/createMessage`
*to* the gateway is not an MCP flow (sampling is server→client). The route
intercepted it, asked the deprecated per-client `mcp_sampling_permission`
shim (always `Ask` since it is no longer serialized) for a popup, and then
for every client that can reach the route (`mcp_direct_enabled`; others are
refused before) broadcast a non-standard `sampling/createMessage`
*notification* back to the same client, waiting 120 s for an answer on
`POST /mcp/sampling/respond/{id}`, which nothing sends. Its LLM branch was
unreachable (`check_mcp_access_with_state` refuses non-direct clients).
Removed: the intercept, the respond route, `SamplingPassthroughManager` and
the AppState field. The request now reaches the gateway, which answers
`-32601` with a clear message. No working client used this.

### 5. Reserved `_meta` key

Partly true. Every `Transport` method that writes a request strips the key
(`param_headers::take` in stdio `send_request` / `open_rpc_stream`, WebSocket
`send_request`, SSE `send_request` / `stream_request` / `open_rpc_stream`;
default `stream_request` delegates to `send_request`). But a downstream client
could set the key itself on any request (e.g. `tools/call`): the gateway
forwarded it, and the HTTP transport turned it into `Mcp-Param-*` headers the
gateway never computed. Fix: the gateway strips it from every downstream
request at ingress (`handle_request_with_client_cwd`), so only the gateway's
own `param_headers::attach` (Push Relay) can set it. Tests per transport
(stdio, WebSocket, Streamable HTTP and legacy HTTP+SSE: send / stream /
rpc stream) and for the ingress.

### 6. Awaited requests across a stream drop

Correct for Streamable HTTP: the client answers a server request by POST,
which does not need the stream it arrived on, and the spec says a
disconnection is not a cancellation. Pruning: the waiter always times out
(`FORWARD_TIMEOUT`, 120 s), after which the entry is dropped on the next
prune. Gap: when the *gateway session* ends (DELETE, expiry, re-initialize,
legacy SSE disconnect), its forwarded requests kept waiting up to 120 s
(the upstream with them) although nobody can answer any more. Fix: ending a
session cancels its forwards (`ForwardedRequests::cancel_session`).

### 7. LocalRouter management MCP server

Not touched: the other session committed to `src-tauri/src/ui/admin_mcp*`
less than 6 hours ago (last `3e6360bf`, 2026-11-22 22:29) and a worktree
`/private/tmp/localrouter-mcp-try-it-out` exists. Still open in its plan:
configuration coverage against the manifest plus desktop/demo integration,
and verification of capability-aware push delivery, renewed scope checks and
the polling fallback for intercept monitoring.

### 8. Bug hunt

See "Bugs found".

## Checklist

- [x] Plan saved and committed
- [x] Item 1 (sampling limits)
- [x] Item 2 (`Both` fallback E2E)
- [x] Item 3 (template names)
- [x] Item 4 (route sampling leftover)
- [x] Item 5 (reserved `_meta` key)
- [x] Item 6 (forwards end with their session)
- [x] Item 8 (bug hunt)
- [ ] Item 7 (left to the admin MCP session, see above)
- [x] Validation
- [x] Plan Review
- [x] Test Coverage Review
- [x] Bug Hunt (re-read of this branch's diff)
- [x] Commit and push

## Bugs found

All fixed with regression tests.

1. **`completion/complete` never reached a prompt's server**: the ref name's
   prefix (server slug) was used as the server id; `ref/resource` (by `uri`)
   was refused. Now routed through the prompt mapping / resource
   resolution with the permission checks. Test:
   `completion_routes_prompt_refs_by_mapping_and_refuses_unknown_refs`.
2. **Client cancellations named the wrong request upstream**: transports send
   requests under their own ids; a forwarded `notifications/cancelled` kept the
   client's id (no-op, or cancelled an unrelated request). Transports keep
   caller→wire ids (`transport::WireIds`). Test:
   `cancellation_names_the_wire_id`.
3. **Two first requests for one key created two sessions** (the loser ran
   on an orphan with its own upstream processes); stale-session removal could
   remove a sibling's new session. Insert-if-absent and `remove_if` by
   identity. Test: `concurrent_first_requests_share_one_session`.
4. **DashMap shard lock held across `.await`** in `list_active_sessions` and
   `get_or_build_preview_context` (snapshot first, as elsewhere).
5. **`pending_requests` leaked** when a tool call's future was dropped
   (drop guard `PendingCallGuard`).
6. **`StdioTransport::kill` left waiters hanging** until their 30 s timeout
   and rpc streams open (aborted reader skipped its cleanup). Test:
   `kill_fails_waiting_requests_at_once`.
7. **The bridge never ended its old session on re-initialize** (old session
   and processes lived to idle expiry). Test:
   `reinitialize_deletes_the_previous_session`.
8. **Forwarded-request registry**: a finished wait removed a newer wait's
   entry under a reused upstream id. Test:
   `a_finished_wait_leaves_a_newer_one_under_the_same_id`.
9. Items 4-6 above (client-sent sampling, reserved key from clients, forwards
   outliving their session).

## Not fixed (reported)

- Sessions expire (1 h TTL, `touch` only on requests) while a legacy SSE
  stream, `events/stream` or `subscriptions/listen` is still open; the next
  lazily created session has no client capabilities.
- Client `resources/subscribe` is not re-issued upstream after a backend
  reconnects (`attach_servers_to_session`).
- `invalidate_*_cache` uses `try_write` and silently skips busy sessions
  (Push Relay list-changed path).
- An unknown `Mcp-Session-Id` silently creates a new session (no 404), and
  `stdio_bridge_sessions` / `push_test_sessions` entries are recorded before
  paths that never create a session.
- Push Relay per-session state survives a re-initialize under the same key.
- Only `tools/call` is tracked for client cancellation (`resources/read`,
  `prompts/get` cancellations are dropped).
- stdio / WebSocket transports cap every request at 30 s.
- Upstream `roots/list` is answered with no roots (may be deliberate).
- `cargo clippy --workspace --all-targets` fails on master in
  `crates/lr-proxy/src/websocket_stream_tests.rs:862` (missing `strict` in
  `TestFirewall`, from `8ebcd1fa`); not this branch's code.

