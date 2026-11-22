# Push Relay — phase 2: resource-update ingress and the Channels permission relay

**Status: in progress on `feat/push-relay-p2`.**

Design: `plan/2026-10-30-PUSH_RELAY.md` (P2 row of "Phases"). Handoff notes of P0/P1:
`plan/2026-10-30-PUSH_RELAY_HANDOFF.md`. Webhook egress (D3), the third P2 item, already shipped
with P1.

## Scope

| # | Item | Plan reference |
| --- | --- | --- |
| 1 | Ingress U5: classic `notifications/resources/updated` becomes the synthetic event type `<server>__resource.updated` | "Push types we will support" (U5), "Gating" |
| 2 | Claude Channels permission relay (`claude/channel/permission`) between a STDIO bridge session and upstream channel servers | "Protocol facts", "Phases" |
| 3 | Route-level test of the `events/stream` SSE answer (`crates/lr-server/src/routes/mcp.rs`, `events_stream_response`) | Handoff, "Not covered by an automated route-level test" |
| 4 | Re-check of the two open protocol questions | "Open questions" |

Out of scope (P3): resource-update hints D6 for VS Code Copilot, guardrails and secret scanning
over event payloads.

## Task list

- [x] Read both plan files; verify their file references against the code
- [x] Save this plan
- [ ] U5: event type, gate, demand-driven upstream `resources/subscribe`, ingest, native delivery
- [ ] Permission relay: advertisement, request forwarding, verdict admission and delivery
- [ ] SEP-3415 final-text deltas (`heartbeatMs`, `eventsListChanged`)
- [ ] Unit tests (journal scope, permission table, hub resource mode)
- [ ] `push_relay_e2e_tests.rs`: a U5 scenario, a permission-relay scenario, the route-level
      `events/stream` test
- [ ] UI / docs / demo touch-ups for the two new event types
- [ ] Update the status and checklists of the two 2026-10-30 plan files
- [ ] **Plan Review** — compare this plan with the implementation, implement anything missed
- [ ] **Test Coverage Review** — add tests for uncovered paths of new and changed code
- [ ] **Bug Hunt** — re-read the implementation looking for bugs
- [ ] Validation (fmt, clippy, targeted tests), commit and push

## File references checked against the code

The plan's line numbers drifted; the references below are the current ones.

| Plan says | Now |
| --- | --- |
| `routes/mcp.rs:1522`, `:1608` (`subscriptions/listen`) | `subscription_type_for_method` `:1625`, `build_subscription_notification` `:1670`, `subscriptions_listen_response` `:1727`, `events_stream_response` `:1824` |
| `gateway.rs:1840` `register_notification_handlers_on_transports` | `gateway.rs:2318` |
| `gateway/types.rs:456` `ServerCapabilities` | still in `gateway/types.rs` (`has_resource_subscribe` `:516`) |
| `gateway/firewall.rs:63` `edited_arguments` | unchanged in purpose |
| relay modules | `crates/lr-mcp/src/gateway/push_relay/{relay,journal,cursor,upstream,ask,webhook,tools,render,limits}.rs` |

`notifications/resources/updated` was handled only by the raw rebroadcast in
`register_notification_handlers_on_transports` and by the `subscriptions/listen` filter in
`routes/mcp.rs`; nothing in `push_relay/` read it. No permission-relay code existed (one
capability-preservation test mentions the key).

## Design

### 1. Resource updates (U5)

- **Event type.** A server whose remembered capabilities declare `resources.subscribe` gets the
  synthetic event type `<slug>__resource.updated` (`RESOURCE_EVENT_NAME`), unless its own
  `events/list` already has an event of that name (the upstream's wins). Data:
  `{"uri": <uri>, "resource": <gateway resource name>}`. Permission key
  `<server id>__resource.updated`, in the same Allow / Ask / Off tree as every other type.
- **Namespacing.** The gateway lists resources under their backend URI and namespaces only the
  resource *name* (`<slug>__<name>`, `merger.rs`). The event therefore carries the URI exactly as
  `resources/list` shows it to the client, and the namespaced name under `resource` when the
  session's resource mapping knows the URI.
- **Gate.** U5 counts toward the gate ("`resources/updated`, if U5 is enabled"): the relay now
  asks `remembered_push_sources(.., include_resource_updates)` with
  `PushRelayLimits::resource_update_ingress` (internal, default on — the one seam that turns U5
  off). Setting `resource.updated` to Off for a client removes it for that client.
- **Demand-driven subscription.** `inputSchema` has one property, `uri`.
  - With `arguments.uri` (`events/stream`, `events/poll`, `events/subscribe`, `EventPoll` /
    `EventWait` `filter`), the relay holds an upstream `resources/subscribe` for that URI while a
    consumer exists, reference-counted like every other subscription
    (`SubscriptionKey{scope, server, "resource.updated", {"uri": …}}`), and sends
    `resources/unsubscribe` when the last consumer goes. HTTP / WebSocket upstreams use the
    relay-owned connection; stdio upstreams the session's own process. The subscription is
    re-issued every `resource_resubscribe_secs` (5 min), which is also its liveness check.
  - Without `uri`, the reader gets every resource update that reaches the client's journal
    (its sessions' own `resources/subscribe` calls, and other consumers' subscriptions) and opens
    nothing upstream. Claude Channels sessions read this way, as they do for every type.
- **Classic subscribers keep working.** A session that called `resources/subscribe` itself still
  receives the native `notifications/resources/updated`, now after the relay's controls: Allow
  forwards it (and journals the occurrence), Ask holds it until approved, Off drops it with a
  Monitor record. A client push cannot reach (`push_possible` false) gets the notification raw,
  as before Push Relay.
- **Shared connection, one upstream subscription.** On a session's own connection the relay's
  subscription and the client's own `resources/subscribe` for the same URI share one upstream
  subscription: the session records what the relay holds (`relay_resource_subscriptions`), the
  relay does not unsubscribe a URI the client subscribed to itself, and the client's
  `resources/unsubscribe` does not unsubscribe a URI the relay still holds.
- **Event ids / dedupe.** A resource update has no upstream id; each notification gets one
  `event_id` (`<slug>__<uuid>`) shared by every delivery mode, exactly like a channel message.
- **Backpressure.** Unchanged rules: `max_event_bytes`, the Ask hold cap, the journal bounds and
  the Claude Channels rate all apply, because the occurrence goes through `admit` and the
  journal like any other.

### 2. Claude Channels permission relay

Wire form (code.claude.com/docs/en/channels-reference, "Relay permission prompts", read
2026-11-21):

- A channel server opts in with `capabilities.experimental["claude/channel/permission"] = {}`
  (`false` or absent opts out).
- Claude Code → server: `notifications/claude/channel/permission_request`
  `{request_id, tool_name, description, input_preview}` (all strings; `request_id` is five
  letters of `a`–`z` without `l`).
- Server → Claude Code: `notifications/claude/channel/permission` `{request_id, behavior}` with
  `behavior` `allow` or `deny`. Claude Code applies a verdict only for an id it issued and that is
  still open; the local dialog stays live and the first answer wins.

LocalRouter sits in the middle, as a channel server toward the STDIO bridge session and as the
client of the upstream channel servers:

- **Advertisement.** `experimental["claude/channel/permission"] = {}` is added for a STDIO bridge
  session only when `claude/channel` is advertised and at least one visible, running-capable
  server's remembered capabilities declare both `claude/channel` and `claude/channel/permission`
  with the synthetic event type `<slug>__channel.permission` not Off for the client.
- **Request (client → upstream).** The gateway hands the client notification to the relay
  (`PushRelayService::on_client_notification`). The relay validates it (bridge session, the four
  string fields, an id of at most 64 ASCII letters/digits, total size within `max_event_bytes`),
  records it as pending for `(session, request_id)` and forwards exactly the four documented
  fields to every eligible upstream **on that session's own connection**. Each forward gets a
  Monitor record (`<slug>__channel.permission_request`, ingress `client`). Requests are not held
  under Ask: the local dialog is open at the same time and a held prompt only delays the remote
  one; what Ask controls is the verdict.
- **Verdict (upstream → client).** The verdict is an occurrence of `<slug>__channel.permission`
  (ingress `channel`, one `event_id`), admitted through the normal path: size limit, permission
  (Allow / Ask / Off), Ask hold with the approval popup (the popup shows the tool, description and
  input preview of the pending request; an edit may change `behavior` only), Monitor record. It is
  accepted only when the session has that `request_id` pending, the server was one the request
  went to, and no verdict was accepted for it yet (first wins; a repeated id is never relayed,
  matching Claude Code). On Allow it is delivered as `notifications/claude/channel/permission` to
  the originating session and **never journaled**: a verdict has no meaning to any other reader,
  and a channel rendering of it would put a permission answer in front of the model as text. This
  is the plan's lossy Channels edge (no cursor, replay or acknowledgement).
- **Not in `events/list`.** `channel.permission` appears in the permission tree
  (`client_event_types`) and in Upstream sources, not in the catalogue.
- **Bounds.** `permission_pending_cap` (64 per session; the oldest goes first) and
  `permission_request_ttl_secs` (1 h), both internal, swept with everything else.
- **Unreachable clients.** For a client push cannot reach nothing is advertised, a request is
  dropped, and an upstream verdict is forwarded raw as before Push Relay.

### 3. SEP-3415 deltas applied

`notifications/events/active` now carries the granted `heartbeatMs`; the upstream stream watchdog
uses three times the upstream's granted `heartbeatMs` when that is longer than the internal
timeout; `subscriptions/listen` accepts the SEP's `eventsListChanged` opt-in beside the relay's
own `events` key.

## Open protocol questions

| Question | Source consulted | Outcome |
| --- | --- | --- |
| 1. Wire form of Claude Code's 2026-07-28 probe, and that declining it yields channel registration | code.claude.com/docs/en/mcp ("MCP client runtimes") on 2026-11-21; `crates/lr-mcp/src/bridge/negotiation.rs` | **Still not documented.** The page says Claude Code "asks" stdio servers and that a server that does not support the revision "connects on the earlier handshake and registers as before"; it names no method. The repo's note derives the probe from the 2026-07-28 stdio transport and TypeScript SDK v2 (`server/discover`, -32601 → legacy). The plan's default stays as implemented, behind `bridge::negotiation::intercept`. Not captured from a real Claude Code (needs an interactive session). New since the plan: Claude Code may skip asking a stdio server for 7 days, and v2.1.234+ sends permission requests only to servers it registered as channels — so the permission relay depends on the same unverified registration. |
| 2. SEP-3415 may change before acceptance | PR modelcontextprotocol/modelcontextprotocol#3415, `seps/3415-events-extension.md` | **Resolved.** Merged 2026-11-19 as **Final** (merge commit `8a18b4b3292b9f9b3aee73f6325f9f62f19a89f9`, PR head `6a4bf07c96abad167976c1156c6daa63ced40535`). Diff against the pinned `b08e4f9d`: capability location, method and notification names, error codes and the list-changed transport are unchanged. Added: `heartbeatMs` (suggested on `events/stream`, granted on `notifications/events/active`; dead after 3× the grant), `x-mcp-header` annotations on `inputSchema` mirrored into `Mcp-Param-*` headers on Streamable HTTP, jitter before reconnecting, Server Card key `extensionsInfo`. Applied here: see "SEP-3415 deltas applied". **Not applied:** `Mcp-Param-*` headers on upstream `events/*` requests (a client MUST for annotated arguments), and honouring a client's suggested `heartbeatMs` (the relay grants its own interval, which the SEP allows). |
| Permission relay wire form (not listed in the plan, new in P2) | code.claude.com/docs/en/channels-reference | Documented; implemented as documented. Uncertain: whether Claude Code accepts the capability and verdicts from a server that is a relay for several upstreams (nothing in the docs forbids it), and the id format is validated loosely (≤ 64 ASCII alphanumerics) in case it changes. |

## Deviations and things left open

Filled in at the end of the work.
