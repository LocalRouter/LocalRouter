# Push Relay — phase 0 handoff (gateway foundations + config)

Branch `feat/push-relay`. Design: `plan/2026-10-30-PUSH_RELAY.md`. This note is for the agent
implementing the relay core (event store, journal/cursors, ingress managers, Ask hold, Monitor,
fallback tools, downstream `events/*` and Channels, `/events/watch`, webhook egress).

Everything below exists, is tested, and changes no behaviour while no relay is registered:
`events/*` stay -32601, nothing extra is advertised, upstream push notifications are forwarded
raw as before, and `initialize` never waits longer than its existing 2 s.

## Config (`crates/lr-config/src/push_relay.rs`, re-exported from `lr_config`)

| Item | Notes |
| --- | --- |
| `PushRelayConfig` | `AppConfig.push_relay`; `default_permission` (`Allow`; the only global control), `channel_messages_per_minute` (10, 1–600), `history_retention_hours` (24, 1–720) and `history_max_events` (10,000, 100–1,000,000) — the only global settings. `PushRelayConfig::validate` checks the bounds. Omitted from YAML while default; each absent field takes its default. Read through `PushRelayConfigFile` (`#[serde(from)]`): `enabled: false` loads as `default_permission: off`, `enabled: true` is ignored, the key is never written. Unknown keys (`limits`, `webhook_egress_enabled`, `allow_loopback_http`, `ask_timeout_secs`) are ignored on load. |
| `PushPermission` | `Allow | Ask | Off`, snake_case; `block` is read as `off`. |
| `PushPermissions` | `Client.push_permissions`: `global: Option<_>`, `servers: {server_id: _}`, `events: {"server_id__event_name": _}` (`PushPermissions::event_key`, unqualified upstream event name). `effective(app_default, server_id, event_name)` = event → server → client → app default; `effective_for_server`. |
| `Client::push_permission(&cfg.push_relay, server, event)` | Same, against the global default. Combine with `McpPermissions` yourself: push never widens MCP access. `Client::may_receive_push(&cfg.push_relay)`: the default or an override (`PushPermissions::enables_anything`) is not Off. |
| `Client.push_webhook_urls` | Approved callback URLs. Cloning a client copies its permissions, not these. |
| `UpstreamCapabilitySnapshot` | `McpServerConfig.capability_snapshot`: `fingerprint`, `protocol_version`, raw `capabilities` (incl. `experimental`, `extensions`, unknown fields), derived `push: UpstreamPushSummary { events_modes: {event: [push/poll/webhook]}, channels, resource_subscribe }`, raw `event_catalogue` entries, `observed_at`. `same_content` ignores `observed_at`; `has_consumable_push(include_resource_updates)` applies the plan's gate (webhook-only types do not count). |
| `fingerprint_for(&McpServerConfig)` | `sha256:` of transport + (stdio command and args | URL) + auth method and its non-secret identifiers (OAuth client id/URLs/scopes, header and env-var *names*). Never env values, header values or keychain refs. `McpServerConfig::current_capability_snapshot()` returns the snapshot only while the fingerprint matches. |
| `McpServerConfig::same_connection` | Equality ignoring the snapshot. `SessionTransportSet::configuration_matches` uses it, so recording a snapshot never forces reconnects. |
| `ConfigManager::update_derived` | Validated update that emits no `config-changed` and does not sync clients. |
| `ConfigManager::record_capability_snapshots(&map)` | Writes only entries whose content changed; returns whether to `save()`. |

Not exported or synced anywhere: no config export/sync path for MCP server entries exists in the
tree today (the launcher integrations write LocalRouter's own entry into apps, not upstream
configs), `clone_mcp_server` does not copy the snapshot, and the UI is never told about snapshot
writes (the config file watcher is disabled in `main.rs`; `config-changed` is emitted only by
`update`).

## Upstream transport layer (`crates/lr-mcp/src/transport/`)

- `Transport::open_rpc_stream(request, buffer) -> RpcStream` and `supports_rpc_streams()`;
  implemented for stdio and HTTP (`SseTransport`), WebSocket returns an error (default).
- `RpcStream` (`rpc_stream.rs`) is a `Stream<Item = AppResult<RpcStreamMessage>>`:
  `Notification(_)` for every notification whose `_meta["io.modelcontextprotocol/subscriptionId"]`
  equals the request id, then one `Result(_)`. If the connection ends first the last item is an
  `Err` (reconnect on it). Ids are restored to the caller's (`request_id()`), including the
  subscription id inside each notification. No total timeout; `buffer` bounds the channel and a
  full buffer stops the transport reading (stdio: the whole pipe; HTTP: that response body).
  **Drain promptly and do not await other requests to the same upstream from inside the
  per-message handler** — spawn them.
- Cancellation: dropping (or `cancel()`) before the result sends `notifications/cancelled
  {requestId, reason}` on stdio and drops the response body on HTTP. Legacy HTTP servers that
  acknowledge the POST and answer on the GET stream are routed through the same registry.
- Uncorrelated notifications and server→client requests on a streamed POST body keep going to
  the transport's generic callbacks.
- `SessionTransportSet::open_rpc_stream(server_id, request, buffer)` adds the stateless `_meta`
  like `send_request`. `DEFAULT_RPC_STREAM_BUFFER` = 256.

## Gateway (`crates/lr-mcp/src/gateway/`)

### Contract: `push_relay.rs`

```rust
#[async_trait]
pub trait PushRelayService: Send + Sync {
    fn advertisement(&self, ctx: &PushRelaySessionContext) -> PushAdvertisement;
    fn instructions(&self, ctx: &PushRelaySessionContext) -> Option<String> { None }
    fn listen_opt_ins(&self, client_id: &str, session_key: &str) -> Vec<ListenOptIn> { vec![] }
    fn on_upstream_notification(&self, n: UpstreamPushNotification) -> bool { false }
    async fn events_list(&self, ctx, request) -> AppResult<JsonRpcResponse>;
    async fn events_poll(..); async fn events_stream(..);
    async fn events_subscribe(..); async fn events_unsubscribe(..);
}
```

- **Register**: in `AppState::with_mcp` (`crates/lr-server/src/state.rs`), after the gateway is
  built: `mcp_gateway.set_push_relay(Arc::new(relay))`. The relay will want a gateway handle:
  `McpGateway` is `Clone` (all `Arc`s), so construct the relay with `mcp_gateway.as_ref().clone()`.
  `gateway.push_relay()` returns it. `update_push_relay_config(&cfg.push_relay)` is called once
  at startup there; **call it again whenever the user changes Push Relay settings** (no Tauri
  command exists yet).
- `PushRelaySessionContext`: `client_id`, `session_key`, `client_name`, `is_stdio_bridge`,
  `protocol_revision`, raw `client_capabilities` (`client_declares_events()` checks both
  locations), `visible_servers` (permitted + enabled), `running_servers`, `notification_key`.
- `PushAdvertisement { extensions, capabilities, experimental }` is merged into every
  `initialize` result (all four paths: full, no servers, skills-only fallback, the 2 s
  "still initializing" answer) and into `server/discover`. Upstream `experimental`, unknown
  top-level fields and the `io.modelcontextprotocol/events` extension are **not** forwarded by
  the merger — this is the only way they reach a client. Advertise `claude/channel` only when
  `ctx.is_stdio_bridge`.
- `instructions()` is added to the gateway instructions as a virtual section ("Events",
  priority 25) via `collect_virtual_instructions`. If you also build the `_push_relay`
  `VirtualMcpServer` with its own `build_instructions`, return `None` here to avoid duplicates.
- Dispatch: `events/list|poll|stream|subscribe|unsubscribe` hit `handle_push_request` from both
  the direct and broadcast dispatchers; the service is awaited with no session lock held. They
  are never forwarded upstream. (Stateless clients' first `events/*` call still lazily starts
  the session's upstreams, like any non-discovery request.)
- `on_upstream_notification` receives `notifications/claude/channel` and every
  `notifications/events/*` that was not correlated with an open `RpcStream`, with the server id,
  session key and client id. Return `true` to stop the raw rebroadcast. Everything else is
  forwarded as before.
- Constants: `EVENTS_EXTENSION_ID`, `EVENTS_TOP_LEVEL_CAPABILITY`, `CLAUDE_CHANNEL_CAPABILITY`,
  `CLAUDE_CHANNEL_NOTIFICATION`, `EVENTS_NOTIFICATION_PREFIX`, `EVENTS_LIST_CHANGED_NOTIFICATION`,
  `STDIO_BRIDGE_HEADER` (`x-localrouter-bridge`), `STDIO_BRIDGE_HEADER_VALUE` (`stdio`).
- Helpers: `derive_push_summary(&ServerCapabilities, &catalogue)`, `event_catalogue_page(&result)`,
  `PushMethod`.

### Gateway API for the relay (`gateway.rs`)

| Method | Use |
| --- | --- |
| `upstream_request(session_key, server_id, method, params)` | Any request (`events/list`, `events/poll`, …) on that session's connection to the server; normal request timeout. Refuses servers the session cannot see. |
| `open_upstream_stream(session_key, server_id, method, params, buffer)` | `events/stream` etc. as an `RpcStream`. |
| `push_session_context(session_key)` | Context of a live session. |
| `session_capability_providers(session_key)` | `CapabilityProviders`: each upstream's own declared capabilities (`servers_with_events()`, `servers_with_claude_channel()`, `servers_with_extension(id)`, `get(id)`). Also on `MergedCapabilities.providers`. |
| `remembered_push_sources(&ids, include_resource_updates)` / `remembered_push_sources_for_session(session_key, ..)` | Enabled servers whose current snapshot has consumable push → `RememberedPushSource { server_id, server_name, snapshot }`. This is the gate input at `initialize` time. |
| `send_session_notification(session_key, notification)` | Deliver to that session's client via the notification broadcast (legacy SSE / WebSocket connection, `subscriptions/listen`). |
| `set_stdio_bridge_session(session_key, bool)` | Called by the route on every request; read via the context. |
| `set_capability_snapshot_persister`, `set_capability_snapshot_debounce` | Wired in `AppState`; the debounce setter exists for tests. |

Upstream connections are **per downstream session** (each session owns its
`SessionTransportSet` and its own stdio processes), so channel messages arrive once per session
and these APIs act on one session's connection. The plan's shared, reference-counted upstream
subscriptions need a relay-owned connection: `McpServerManager::create_transport(server_id)`
returns an uninitialised `Arc<dyn Transport>` (you must send `initialize` /
`notifications/initialized` yourself, or `server/discover` for 2026-07-28 backends) on which
`open_rpc_stream` works the same way. No helper for that exists yet.

### Remembered capabilities and the first-discovery wait

- After every successful upstream initialisation (initial warm-up and backend recovery) the
  gateway snapshots the capabilities, fetches `events/list` (up to 10 pages, 5 s each) when the
  server declares events in either location, and records it if the content changed:
  immediately in `McpServerManager`'s config (`set_capability_snapshot`) and, batched over 5 s,
  in the config file via `ConfigManager::record_capability_snapshots` + `save()`. An upstream
  `notifications/events/list_changed` re-snapshots that server. Each session's warm-up reads
  `events/list` once per events-capable server; only changes are written.
- `McpServerManager::add_config` keeps a current in-memory snapshot when handed a config copy
  without one, and drops it when the identity changed.
- `initialize`: with a relay registered **and** push able to reach the client
  (`PushRelayService::push_possible_for(client_id)`), if any visible enabled
  server has no current snapshot, the answer waits past the usual 2 s until each such server has
  a snapshot or failed, bounded by the internal `FIRST_DISCOVERY_WAIT` (10 s,
  `push_relay/limits.rs`) from the start of the request; `McpGateway::set_first_discovery_wait`
  exists for tests.
  `server/discover` does the same, starting the session's upstreams if needed. Disabled and
  invisible servers are never waited on. Without a relay there is no wait at all (nothing would
  read the snapshot).

### Routes (`crates/lr-server/src/routes/mcp.rs`)

- `X-LocalRouter-Bridge: stdio` (case-insensitive value) → `set_stdio_bridge_session` for the
  gateway session key (`Mcp-Session-Id`-derived key, else client id) before every POST.
- `subscriptions/listen`: opt-in keys returned by `listen_opt_ins` are accepted when sent as
  `true`, and notifications whose method starts with that opt-in's `method_prefix` pass (tagged
  with the subscription id). SEP-2575 has no key for events, so the relay names its own (the
  tests use `events` → `notifications/events/` and `claudeChannel` →
  `notifications/claude/channel`). Without a relay the filter is unchanged.

## Tests

- `crates/lr-config/src/push_relay.rs`: old configs load unchanged, a config carrying keys that
  are not read (`limits`, `webhook_egress_enabled`, `allow_loopback_http`, `ask_timeout_secs`,
  `push_fallback_tools`, `claude_channels_launched`) loads and drops them on save, defaults not
  serialised, non-default round trip, most-specific permission, global Off unless an override
  enables something, `block` read as Off, `enabled: false` read as a default of Off, fingerprint excludes secrets/includes identity, snapshot invalidation, snapshot not part
  of connection identity, `same_content`, push gate, `record_capability_snapshots` writes only on
  change.
- `crates/lr-mcp/src/transport/rpc_stream.rs` (6), `stdio.rs` (3 new, `sh` fake upstream):
  many results interleaved with a plain request, uncorrelated notifications to the generic
  callback, cancellation sends `notifications/cancelled`, process exit ends the stream with an
  error. `sse.rs` (3 new, axum fake upstream): multi-result POST SSE body, dropping closes the
  body, legacy 202 + GET-stream routing.
- `crates/lr-mcp/src/gateway/tests.rs` (3 new): unknown fields/`experimental` preserved, two
  conflicting providers, events-only extensions dropped. `push_relay.rs` (7): summary derivation,
  catalogue paging, advertisement merge, client events detection, debounced write-only-on-change
  recorder, manager snapshot retention.
- `crates/lr-server/src/routes/mcp.rs`: push opt-ins need the relay and the client's opt-in.
- `src-tauri/tests/push_relay_gateway_tests.rs` (12, real gateway, wiremock + `sh` stdio
  upstreams): -32601 without a relay and nothing forwarded upstream; routing to the relay with
  the right context; advertisement only from the relay and `claude/channel` only for bridge
  sessions (also on `server/discover`); snapshot persisted once and not on unchanged
  re-observation; fingerprint invalidation; first-discovery wait (waits once, then not again),
  its bound, no wait without a relay or when push cannot reach the client, never for disabled
  servers; channel
  notifications consumed by the relay vs raw forwarding; `upstream_request` and
  `open_upstream_stream` on a session.

## Left for P1 / deviations from the plan

- **Downstream streaming at the HTTP layer is not done.** `events/stream` reaches the relay
  through `handle_request_with_skills`, but the route's SSE-async path cuts non-`initialize`
  requests at 15 s and the stateless JSON path answers once. Give `events/stream` (and long-poll
  `events/poll`) route handling like `subscriptions_listen_response`: a POST answered with an SSE
  body fed from the session's notifications, ending with the relay's result.
- The per-client push fields are not threaded through `handle_request_with_skills` /
  `ClientListingSnapshot` (plan's P1 configuration item). The relay can read the `Client` from
  `ConfigManager` by `ctx.client_id` meanwhile.
- No Monitor event type, firewall `InterceptCategory::PushEvent`, Tauri commands, TS types, UI,
  docs or website — all P1.
- Fingerprint covers stdio command + args, not `cwd` (plan names command + args).
- `EventDeliveryMode` also records `webhook` (so webhook-only types can be shown as unsupported);
  an `events/list` entry without `delivery` counts as poll; unknown modes are ignored.
- Three limits come from settings: `PushRelayLimits::apply_config` sets `channel_rate_per_minute`,
  `journal_retention_secs` and `journal_max_events` from `channel_messages_per_minute`,
  `history_retention_hours` and `history_max_events`. `PushRelay::new` starts from the config and
  `PushRelay::update_config` re-applies it, pruning every journal to the new bounds at once. Every
  other limit is an internal constant in `crates/lr-mcp/src/gateway/push_relay/limits.rs`
  (`PushRelayLimits::default()`), not configuration; tests shrink them with `PushRelay::set_limits`
  (which replaces the settings-derived three as well until the next `update_config`).
  `webhook_retries` is `webhook_retry_delays_secs: [5, 30, 120, 600, 1800]` (first try + one
  retry per delay = 6 attempts).
- `observed_at` is when the content last changed, not when it was last confirmed (writing on
  every confirmation would contradict write-only-on-change).
- HTTP stream cancellation only drops the body; it does not also POST `notifications/cancelled`.
- WebSocket upstreams cannot open RPC streams yet.
- `client_capabilities_raw` is captured from `initialize` (and the stateless lazy init); later
  per-request `_meta` capabilities of stateless clients do not update it.
- The STDIO bridge header is consumed but the bridge does not send it yet (parallel worktree
  `localrouterai-push-bridge`).

# Phase 1 backend — handoff to the UI agent

The relay core is implemented and registered (`crates/lr-mcp/src/gateway/push_relay/`,
`AppState::with_mcp`). Everything below is backend API the desktop UI (Tauri commands, TS
types, `TauriMockSetup.ts`, Push Relay page, client Push card, approval popup) builds on. No
Tauri command for Push Relay exists yet.

## Where things live

| Module (`gateway/push_relay/`) | Contents |
| --- | --- |
| `relay.rs` | `PushRelay`: implements `PushRelayService`, owns everything below. Public API in the table that follows. |
| `journal.rs` | `Occurrence` (normalised event: `event_id` `<slug>__<upstream id>`, `name` `<slug>__<event>`, `server_id`, `server_slug`, `upstream_name`, `timestamp`, `data`, `upstream_cursor`, `ingress`, `args_key`), the per-client `ClientJournal` (count / bytes / retention bounds, truncation). |
| `cursor.rs` | HMAC cursors `lr1.…` (epoch, position, scope digest, bound to the client). Foreign epoch → reset to now + `truncated`. Also signs watch tokens. |
| `upstream.rs` | `UpstreamHub` (demand-driven subscriptions), `RelayConnection` + `connect_relay_upstream` (relay-owned, initialised upstream connection), `SessionPort`, backoff/heartbeat. |
| `ask.rs` | Held-event queues, `PushDecision` / `PushDecisionAction`, `HeldPushEventInfo`. |
| `webhook.rs` | Callback URL rules, `whsec_` secrets, Standard Webhooks signing, challenge, delivery. |
| `tools.rs` | `_push_relay` virtual server (`EventList`, `EventPoll`, `EventWait`, `<events>` block). |
| `render.rs` | Event blocks, channel content, instructions, watcher commands. |

Server identity in names is the **slugified server name** (the tool-prefix convention), e.g.
`vocalcord__message.received`. Permission keys stay `server_id__event` (`PushPermissions::event_key`).

## Backend API for the UI

Get the relay from `state.push_relay` (`Option<Arc<PushRelay>>` on `lr_server::state::AppState`;
always `Some` after `with_mcp`). All types are `Serialize`.

| Need | API |
| --- | --- |
| Global settings | `config.push_relay` (`PushRelayConfig`: `default_permission`, `channel_messages_per_minute`, `history_retention_hours`, `history_max_events`). `update_push_relay_config` validates through `apply_push_relay_update` (an out-of-bounds value changes nothing), then saves with `config_manager.update` + `save()`; a `config-changed` listener in `main.rs` then calls `AppState::sync_push_relay_settings`, so a command only has to save. The Info tab's Settings card (`src/views/push-relay/RelaySettingsCard.tsx`) edits the three numeric settings. |
| Per-client fields | `Client.push_permissions` (`PushPermissions { global, servers, events }`) and `push_webhook_urls`. The fallback tools have no setting: they are shown unless the session declared MCP Events. Saving the client fires `clients-changed`, which calls `relay.notify_clients_changed()` (list_changed to sessions, held events re-evaluated, streams of types now Off end, subscriptions no reachable client needs close). Helpers: `Client::set_push_event_permission(server_id, event, Option<PushPermission>)`, `Client::approve_push_webhook_url(url)`. |
| Permission tree data | `relay.client_event_types(client_id) -> Vec<ClientEventTypeInfo { qualified_name, server_id, event_name, permission (effective), session_override }>` — only servers the client may see under MCP permissions. Session overrides come from "for this session" decisions; clear one with `relay.clear_session_override(client_id, server_id, event_name)`. |
| Upstream sources | `relay.upstream_sources() -> Vec<UpstreamSourceInfo { server_id, server_name, enabled, transport (stdio/http/websocket), connection (shared/per_session), snapshot_current, protocol_version, observed_at, channels, resource_subscribe, event_types: [UpstreamEventTypeInfo { name, description, modes, ingress (push/poll/channel/null), supported, unsupported_reason ("webhook-only — not supported") }] }>`. Built from the remembered capability snapshots. |
| Active subscriptions | `relay.active_subscriptions() -> Vec<SubscriptionInfo { server_id, event, arguments, scope ({kind: shared} or {kind: session, session_key}), status { ingress_mode, connected, events_received, last_event_at, last_error, reconnects } }>`. |
| Webhook subscriptions | `relay.webhook_subscriptions(Option<client_id>) -> Vec<WebhookSubscriptionInfo { id, client_id, event, url, state (awaiting_approval/active/suspended/denied), expires_at, delivered, last_error }>`. |
| Pending held events | `relay.pending_events(Option<client_id>) -> Vec<HeldPushEventInfo { hold_id, client_id, client_name, server_id, server_name, event_name, qualified_name, event_id, ingress_mode, data, held_at, held_secs, position, firewall_request_id, decided }>` (oldest first; `position` 0 = being asked now). A held event has no timeout: it waits until decided; at most `ask_hold_cap` (1,000) per client are held. |
| Decide | `relay.decide(hold_id, PushDecision { action, edited_data })` with `action` = `allow` / `deny` / `allow_session` / `deny_session` / `allow_always` / `deny_always` (`PushDecisionAction`, snake_case). `edited_data` replaces the event's `data` (edit-then-allow). Works for a queued (not yet asked) event too: it is applied in order. Webhook URL approvals use `hold_id = "webhook:<subscription id>"`. `relay.decide_all(client_id, qualified_name, decision)` = "allow/deny all pending of this type". Permanent decisions are written to the client's config by the relay. |
| Monitor | `MonitorEventType::McpPushEvent` (`mcp_push_event`, category `mcp`), data `McpPushEvent { server_id, server_name, event_name, event_id, ingress_mode, egress_modes[], decision, original_payload?, delivered_payload?, held_ms?, error? }`. `decision`: `pending` (status Pending while held), `allowed`, `asked_allowed`, `asked_edited`, `denied`, `off`, `hold_cap`, `too_large` (the UI shows records stored as `blocked` as `off`). Payloads are omitted for clients whose privacy redacts payloads. The record's `session_id` is the reading session's Monitor request id when exactly one session reads the event (see "Monitor grouping" in the plan), else none. |

### Approval popup

Each held event at the head of its (client, event type) queue opens a regular firewall approval
(`_firewall` broadcast → `main.rs` opens the window), with `is_push_event_request: true` and
`push_event_details: PushEventApprovalDetails { kind (event/webhook_url), hold_id, server_id,
server_name, event_name, qualified_name, event_id, ingress_mode, data, rendered_content,
webhook_url?, pending_for_client, pending_of_type }` on both the notification and
`PendingApprovalInfo`. `full_arguments` is the event data, so the existing edit mode edits
`data`. The popup must offer: Allow (`allow_once`), Edit then allow (`allow_once` +
`edited_arguments`), Deny (`deny`), Allow/Deny this event type for the session
(`allow_session` / `deny_session`) or permanently (`allow_permanent` / `deny_always`), and show
`pending_of_type` / `pending_for_client` ("N more waiting"). `submit_firewall_approval` already
skips its own permission writes for these (the relay applies them) and
`reevaluate_pending_approvals` skips them. One popup per (client, event type) is open at a time;
a single per-client queue window built on `pending_events` + `decide` / `decide_all` is up to the
UI.

## What clients see (implemented)

- Gate: push is advertised only when a visible, enabled server's current snapshot has push/poll
  event types or channels that are not Off for the client, and push can reach the client
  (`PushRelay::push_possible`: enabled, no restricted privacy profile, and the global default or
  an override — configured or a session decision — not Off). An unreachable client gets nothing
  and causes no work: no subscription is held for it (`reconcile` drops its demand), no
  first-discovery wait, and its upstream channel notifications are forwarded raw. Then: `extensions["io.modelcontextprotocol/events"]
  = {listChanged:true}`, top-level `events: {}`, `experimental["claude/channel"]` for STDIO
  bridge sessions, the `_push_relay` tools per the show rules, and the `<events>` block.
- `events/list` (`delivery`: poll, push, webhook), `events/poll`, `events/stream`
  (HTTP: a long-lived SSE POST response; legacy SSE: via the connection, no 15 s cut),
  `events/subscribe` / `events/unsubscribe` (webhook), on legacy and 2026-07-28 sessions.
- Claude Channels for bridge sessions, `GET /events/watch` for `EventWait`.
- `notifications/events/list_changed` + `notifications/tools/list_changed` when a session's
  catalogue or tool set changes (checked every 10 s and on config/client changes).

## Not done / deviations

- **Stdio upstreams are subscribed on the session's own process**, not on a relay-owned shared
  connection: each downstream session runs its own stdio process and that process's events
  belong to it (NetGet's queue, for instance). HTTP and WebSocket upstreams use one relay-owned
  connection shared by all clients (`RelayConnection`). Webhook and watch consumers of a stdio
  upstream use any live session of the client.
- Relay-owned connections always use the legacy `initialize` handshake.
- A channel message from a relay-owned (HTTP) connection is delivered to the clients consuming
  anything from that server; Claude channels are stdio-only in practice.
- "Session" decisions (`allow_session` / `deny_session`) last until LocalRouter restarts or the
  override is cleared; they are per client, not per MCP session.
- `/events/watch` authenticates with a watch token minted by `EventWait` (HMAC, bound to the
  client, the event types and filter, valid for `watch_max_lifetime`), not the client's bearer
  token, so no secret is echoed into tool output.
- `EventPoll`: default 20, max `min(100, poll_max_events)`; `events/poll`: default
  `poll_default_events`, max `poll_max_events`.
- The webhook verification challenge runs synchronously in `events/subscribe`; the new-URL Ask
  happens before the first delivery.
- `InterceptCategory::PushEvent` (`push_event`) is new: the TS `InterceptCategory` list and the
  Monitor intercept picker need it. It covers held events and the `_push_relay` tool calls.
- Monitor "Ask for this event type from now on" is a client config write
  (`set_push_event_permission(.., Some(Ask))`); the Monitor intercept rule with
  `InterceptCategory::PushEvent` turns Allow into Ask while active.
- Not implemented: U5 / D6 (resource updates), the Channels permission relay, guardrails over
  payloads (P2/P3 per plan); UI, TS types, docs, website, demo (UI agent).
- Real peers: `netget_events_reach_a_client_through_the_real_bridge` (`#[ignore]`d) starts the
  real HTTP server with `netget --mcp --llm-agent` as a stdio upstream and drives it through
  the real `localrouter --mcp-bridge`: legacy handshake advertises `claude/channel` + events,
  `events/list`, `EventPoll` tool, `netget__start_server`, a TCP request → NetGet's
  `llm.request.pending` arrives at the client as a Claude channel message. Run with
  `NETGET_BIN=<netget ≥ 0.3.9> cargo test -p localrouter --test push_relay_e2e_tests -- --ignored`.
  The installed `/Users/matus/bin/netget` is 0.3.5 (no Events); the run used a debug build of
  netget HEAD (`dbe5763fd`+) in a temp target. Two gateway fixes came out of it: the backend
  `server/discover` probe also accepts rmcp's `supportedVersions` (NetGet otherwise rejected
  the following legacy `initialize`), and `initialize` waits for a first-seen server's snapshot
  even when the handshake itself finished within 2 s.
- Because NetGet now negotiates 2026-07-28 with LocalRouter, it sends no Claude channel
  messages of its own on that connection (it only does after a legacy `initialize`); its MCP
  Events are what reach the client. Channel ingress (U4) is covered by the scripted stdio
  upstream test instead.
- Not covered by an automated route-level test: the HTTP SSE answer to `events/stream` and the
  bridge GET-stream registration (exercised by the real bridge run above).

# Declaring MCP Events to upstreams

`protocol::declare_events_extension(&mut caps)` and `protocol::declare_events_on_request(&mut
request)` (re-exported constant `EVENTS_EXTENSION_ID`). LocalRouter declares
`extensions["io.modelcontextprotocol/events"]` in every upstream `initialize`
(`initialize_backends` merges it into the downstream's forwarded capabilities, so stateless
backends get it in `_meta` too; reconnection; the two preview handshakes;
`connect_relay_upstream`), and `SessionTransportSet::prepare_request_for` and `RelayConnection`
add it to `params._meta["io.modelcontextprotocol/clientCapabilities"]` on every `events/*`
request on any revision. Regression: `upstreams_are_told_localrouter_supports_events`
(`push_relay_gateway_tests.rs`), a wiremock upstream that answers `events/list` with
push/poll/webhook only when the request declares Events (VocalCord's behaviour) and webhook-only
otherwise; a stale webhook-only snapshot is corrected on the next initialise.

# MCP server Info → Push

`src/views/resources/mcp-server-push-card.tsx`, last on the Info tab in `mcp-servers-panel.tsx`.
Data: `list_push_sources` (filtered by server id), `get_push_relay_config`, and the live
`get_mcp_server_push_status({ serverId })` (`McpServerPushStatus`, from
`relay.server_push_status(server_id)`), re-read every 5 s while the page is visible
(`src/views/push-relay/use-push-status.ts`).

The live status is what the relay is doing, not what permissions allow: per event type
`state` (`listening` / `connecting` / `reconnecting` with `last_error` / `idle`) and
`ingress_mode` (`push` / `poll` / `channel`) from the open upstream subscriptions (channel:
`listening` while a session whose client push can reach runs the server, or a shared connection
is up); `events_received` / `last_event_at` per (server, upstream event) since start, counted
once per upstream delivery in `ingest`, `ingest_shared_channel` and `on_upstream_notification`
(they outlive subscriptions); and `consuming_clients` / `consuming_sessions` from live holders
and channel sessions. Test: `server_push_status_reports_live_listening_counts_and_consumers`
plus the channel assertions in `upstream_channel_reaches_poll_and_bridge_channel`.

Rendering: `PushSourceDetails` (`src/views/push-relay/PushSourceRow.tsx`, shared with the Push
Relay page's `UpstreamSourcesList`, which polls the same status for servers with push). One line
per event type (`message.received — push, poll`; `webhook` is never listed for a usable type,
webhook-only types read "webhook only — not supported"), the description truncated with the
full text on hover, Claude Code channels / resource updates as one line, then the live line —
green only while listening — and the muted "Capabilities checked … · MCP …" footer. Wording:
`pushLiveLine`, `offersLine`, `eventModesLabel`, `extrasLine`, `capabilitiesFooter`,
`eventIssue` in `push-relay-utils.ts` (unit-tested in `tests/e2e/unit/push-relay.spec.ts`).
The demo has VocalCord (listening via push stream, events received), Home Assistant (polling,
no events yet), NetGet (reconnecting with an error), Build Alerts (not listening), a webhook-only
billing server, Slack (not known yet) and servers without push.

Try it out → MCP → Connection lists every push mechanism for the session (Events extension and
`listChanged`, top-level `events`, Claude Code Channels, fallback tools, push stream / poll /
webhook from `events/list`) from the raw initialize capabilities and the push tester's state:
`sessionPushMechanisms` / `sessionNoPushReason` in `push-log.ts`.

# Testing pushes — the Try it out Push panel and the test source

Design and scope: "Testing pushes" in `plan/2026-10-30-PUSH_RELAY.md`. What exists:

| Piece | Where |
| --- | --- |
| Opt-in header | `PUSH_TEST_HEADER` (`x-localrouter-push-test`) = `1`; route → `McpGateway::set_push_test_session`; `GatewaySession::push_test_source`; `PushRelaySessionContext::push_test_source`; `PushToolsState::push_test_source`. The STDIO bridge sends it when `LOCALROUTER_PUSH_TEST=1` (`stdio_bridge::PUSH_TEST_ENV`). |
| Test source | `relay.rs`: `TEST_SOURCE_SERVER_ID` (`localrouter`), `TEST_EVENT_NAME` (`test.event`), `IngressMode::Test`; `catalogue_for(client, visible, include_test)` (the public `catalogue` excludes it); `send_test_event(client_id, message, data) -> TestEventSent { event_id, name, client_ids }` (errors when no live session opted in); `preview_channel_message(&wire) -> (content, meta)`. `client_event_types` lists it (`test_source: true`) when a permission is configured or a session opted in. |
| Tauri commands | `send_push_test_event`, `preview_push_channel_message` (`commands_push_relay.rs`, TS types `PushTestEventResult`, `PushChannelPreview`, `SendPushTestEventParams`, `PreviewPushChannelMessageParams`, demo mocks). |
| Panel | `src/views/try-it-out/mcp-tab/push-panel.tsx`, `use-push-tester.ts`, `push-log.ts`; `mcp-client.ts` (`pushTest` config, `listEvents`, `pollEvents`, `streamEvents`, `rawServerCapabilities`, `onEventsListChanged`). |
| Tests | `push_relay_e2e_tests.rs`: `test_source_reaches_only_sessions_that_opted_in`, `test_event_flows_through_allow_ask_off_monitor_poll_and_stream`, `test_events_reach_opted_in_bridge_channels_and_the_poll_tool_only`; `tests/e2e/unit/try-it-out-push.spec.ts` (dedupe, notices, tool-text parsers, notification routing). |

Try it out sessions against a client always opt in, so they see the test source (and with it
the events capability, the fallback tools and the `<events>` block) even when no upstream offers
push. Try it out's "All" and "Direct" modes use the internal test client, which has no push
permissions; the panel asks for Against Client mode there.
