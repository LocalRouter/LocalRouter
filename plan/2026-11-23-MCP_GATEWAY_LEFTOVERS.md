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
`/private/tmp/localrouter-mcp-try-it-out` exists. Only reported.

### 8. Bug hunt

See "Bugs found".

## Checklist

- [ ] Plan saved and committed
- [ ] Item 1 (sampling limits)
- [ ] Item 2 (`Both` fallback E2E)
- [ ] Item 3 (template names)
- [ ] Item 4 (route sampling leftover)
- [ ] Item 5 (reserved `_meta` key)
- [ ] Item 6 (forwards end with their session)
- [ ] Item 8 (bug hunt)
- [ ] Validation
- [ ] Plan Review
- [ ] Test Coverage Review
- [ ] Bug Hunt (re-read of this branch's diff)
- [ ] Commit and push

## Bugs found
