# MCP gateway: final leftovers

Branch: `fix/mcp-gateway-final`. Items from "Not fixed (reported)" in
`2026-11-23-MCP_GATEWAY_LEFTOVERS.md`, the two open items of
`2026-11-22-LOCALROUTER_ADMIN_MCP.md`, and a bug hunt of the touched code.

Not touched: `crates/lr-proxy`, `src-tauri/src/launcher/proxy*.rs`,
`crates/lr-mcp-via-llm`, `routes/{chat,responses}.rs`.

## Findings per item (verified against the code first)

### 1. Session expiry under open streams

True. `GatewaySession::touch` runs only per request; `cleanup_expired_sessions`
and `get_or_create_session` drop a session whose last request is older than
the TTL (1 h) even while its legacy SSE GET stream, an `events/stream` or a
`subscriptions/listen` is open. The session created next has no
`client_capabilities`, so passthrough sampling/elicitation stop
(`client_declares` is false). The same loss happens when
`get_or_create_session` rebuilds a session because its server set changed.

Decision: the gateway keeps a per-key hold count (`McpGateway::hold_session`
returns a guard); a held session never expires, and its TTL restarts when the
last hold is released. Holds are taken by the legacy/Streamable GET stream,
`subscriptions/listen` and every `events/stream` (inside the gateway, so all
routes are covered). A rebuilt session (server set changed, expired in place)
inherits the old one's client capabilities; a session removed by the
background expiry leaves its capabilities for a later lazily created session
under the same key (kept 24 h, never after DELETE / re-initialize).

### 2. Subscriptions after a backend reconnect

True. `attach_servers_to_session` rebuilds a server's transport; nothing
re-sends `resources/subscribe`, so the client's subscriptions (and the Push
Relay's until its next 5-minute re-issue) are silently dead. Fix: after a
successful reattach, re-subscribe every URI the session holds on that server
(client and relay holds, each once).

### 3. Cache invalidation skipped busy sessions

True. `invalidate_*_cache` used `try_write` and skipped a locked session; the
Push Relay then told the client to re-list and it got the stale cached list.
Fix: per-client invalidation epochs (lock-free counters). Invalidation bumps
them; every request compares the session's seen epochs under the session
write lock it already takes and drops stale caches before any list is
served. No session lock is taken by the invalidator, so no deadlock.

### 4. Unknown `Mcp-Session-Id`

True: a request with an unknown id lazily created a new session. Per the
Streamable HTTP spec the gateway now answers 404 to any POST (other than
`initialize`) or GET carrying an `Mcp-Session-Id` it does not know. The STDIO
bridge, on 404 for its session, re-runs the stdio client's `initialize`
against the gateway (not shown to the client), sends
`notifications/initialized`, and retries the request once; its GET stream
does the same. The desktop Try it out client uses the legacy SSE transport
(`?sessionId`) and the lr-mcp HTTP transport already maps 404 to
`McpSessionExpired`, which triggers a reconnect.

Leak: `set_stdio_bridge_session` / `set_push_test_session` ran before checks
that return without a session (disabled client, refused mode, 404, ...).
Fix: they are set only on paths that reach the gateway, and the expiry sweep
drops entries without a session.

### 5. Push Relay state across re-initialize

True. The relay's per-session state (`SessionInfo`: claimed names, channel,
signature; held streams; session holders; pending permission prompts) is
keyed by session key and only swept when the key has no gateway session,
which never happens when a re-initialize replaces it. Fix:
`PushRelayService::on_session_ended`, called by the gateway whenever it ends
a session (re-initialize, DELETE / terminate, expiry, rebuild).

### 6. Cancellation of `resources/read` / `prompts/get`

True. Only `tools/call` recorded `pending_requests`. Both now record it with
the same guard, so `notifications/cancelled` reaches the owning server (the
transport translates to the wire id).

### 7. 30 s request cap

True for stdio and WebSocket, and also for the HTTP transports (reqwest
client `timeout(30 s)` covers the body, and a stream-delivered answer waits
30 s), so "match the HTTP transports" would not help. Fix (all transports):
`transport::request_timeout(method)`: `tools/call` gets the gateway route's
tool budget (24 h, the client can cancel); other requests keep 30 s. A
`notifications/progress` for a request's `progressToken` restarts its timer
(`resetTimeoutOnProgress`), with the 24 h maximum. HTTP long requests use the
client without a total timeout.

### 8. Upstream `roots/list`

Not deliberate. `2026-01-21-MCP-CLIENT-CAPABILITIES.md` specifies that
backend servers get the configured roots (global, or the client's override);
the empty list was a stub from `e1d6c04d`. Fix: answer with the session's
roots (kept current from config on every request, upstream told with
`notifications/roots/list_changed` when they change). With no configured
roots and a client that declares `roots`, the request is forwarded to the
client (same passthrough as sampling).

### 9. LocalRouter management MCP server

See the section at the end (checked last commit / worktrees first).

## Checklist

- [x] Plan saved and committed
- [ ] Item 1
- [ ] Item 2
- [ ] Item 3
- [ ] Item 4
- [ ] Item 5
- [ ] Item 6
- [ ] Item 7
- [ ] Item 8
- [ ] Item 9
- [ ] Validation
- [ ] Plan Review
- [ ] Test Coverage Review
- [ ] Bug Hunt
- [ ] Commit and push

## Bugs found
