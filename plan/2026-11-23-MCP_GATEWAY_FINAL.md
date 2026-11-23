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

Allowed to touch: the last commit on `src-tauri/src/ui/admin_mcp*` is
`3e6360bf` (2026-11-22 22:29, over 6 h before this work), and the two
worktrees on it (`/private/tmp/localrouter-mcp-publish`,
`/private/tmp/localrouter-mcp-try-it-out`) hold the committed files
unchanged.

- Configuration coverage: manifest tests in `admin_mcp/operations.rs`
  (`coverage_tests`) require every `AppConfig`, `Client` and `Strategy`
  setting to be a config section, a dedicated tool group (with an existing
  tool) or categorized application state with a reason, and fail on new or
  stale entries. They found the client's `sync_config` without a path:
  `lr_clients_update` now sets it and, like the desktop, resyncs the app
  config when sync, modes or strategy change. The legacy client `firewall`
  rules (read by nothing) are categorized as compatibility state.
- Desktop/demo integration checked, nothing missing: the LocalRouter row in
  the sidebar and MCP permission tree, the endpoint constant, and demo mocks
  (`check-demo-mock-coverage.mjs`: 0 unmocked, 0 stale).
- Intercept monitoring verified end to end
  (`intercept_events_reach_only_granted_localrouter_sessions`): push
  delivery on `events/stream` and the polling fallback on the operator's
  dedicated session only; the same client's unified session cannot read it;
  revoking the grant stops reads of already journaled events and further
  publication.

## Checklist

- [x] Plan saved and committed
- [x] Item 1 (stream holds, capability carry-over)
- [x] Item 2 (resubscribe on reconnect)
- [x] Item 3 (invalidation epochs)
- [x] Item 4 (404 for unknown sessions, bridge recovery, flag leak)
- [x] Item 5 (relay state reset on session end)
- [x] Item 6 (read/get cancellations)
- [x] Item 7 (transport timeouts)
- [x] Item 8 (upstream roots/list)
- [x] Item 9 (admin MCP coverage and intercept verification)
- [x] Validation
- [x] Plan Review
- [x] Test Coverage Review
- [x] Bug Hunt
- [x] Commit and push

## Bugs found

All fixed, with regression tests.

1. Lazy backend initialization of a rebuilt session sent `capabilities: {}`
   upstream and overwrote the session's client capabilities with `{}`
   (the root cause of "the replacement session has no client
   capabilities"). Test:
   `held_sessions_outlive_their_ttl_and_successors_keep_client_capabilities`.
2. A session rebuilt for a changed server set lost the client's
   capabilities. Test:
   `rebuilt_sessions_keep_client_capabilities_but_ended_ones_do_not`.
3. `cleanup_expired_sessions` removed by key, so it could remove a session
   a request had just created in place of the expired one (now
   `remove_if` by identity).
4. `session.roots` was fixed at creation: a roots change in settings never
   reached a running session. Test: `upstream_roots_list_gets_the_configured_roots`.
5. `resources/subscribe` during backend warm-up failed with "Server not
   running"; it now gets the same retry error as `tools/call`.
6. The STDIO bridge's server stream stayed on the old session after a
   recovery; it now reopens on the new one. Test:
   `forgotten_session_is_reinitialized_and_the_request_retried`.
7. The push-test opt-in must be recorded before `subscriptions/listen`
   reads it (found while moving the flag recording for item 4).
8. `lr_clients_update` could not set `sync_config` and did not resync a
   synced app's config after a mode or strategy change (item 9).
9. Items 1-8 above.

## Validation

All on rustup stable, shared target dir, after touching every workspace
source file (other worktrees share the target dir):

- `cargo fmt --all -- --check`: clean.
- `cargo clippy --workspace --all-targets -- -D warnings`: clean (only the
  pre-existing manifest `unused dependency` warnings of lr-guardrails and
  lr-monitor).
- `cargo test -p lr-mcp -p lr-server`: 515 + 328 passed.
- `cargo test -p localrouter --lib --test push_relay_e2e_tests --test
  mcp_bridge_stream_tests --test mcp_resource_routing_tests --test
  mcp_gateway_stability_tests --test mcp_client_capabilities_tests --test
  mcp_bridge_tests --test push_relay_gateway_tests --test
  mcp_stateless_http_tests --test mcp_notification_forwarding_tests`: all
  passed (lib 501, e2e 47 + 1 ignored by design, stability 43, ...).
- `node scripts/check-demo-mock-coverage.mjs`: 0 unmocked, 0 stale.

## Known limitations

- Progress restarts the timeout of a request answered on a stream (stdio,
  WebSocket, legacy SSE); an HTTP request answered inline keeps a fixed
  per-method deadline (30 s, `tools/call` 24 h).
- A session rebuilt because its server set changed keeps the Push Relay's
  view of it (the client did not re-initialize); its events streams run on.
- Retired capabilities (background expiry) are kept per key for 24 h; a
  Streamable HTTP client never uses them (it gets 404 and initializes).
