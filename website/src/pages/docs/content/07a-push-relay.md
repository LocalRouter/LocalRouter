<!-- @entry push-relay-overview -->

Push Relay carries server-initiated push from upstream MCP servers to your apps. MCP servers can push events — a new message, a finished job, an alert — but they push in different forms, and most apps understand only one of them or none at all. LocalRouter receives every supported kind of push, turns each occurrence into one normalised event, and delivers it to each app in whatever form that app can consume.

LocalRouter is a proxy with control over every event, never a passthrough. Every event flows through LocalRouter, where it can be seen in Monitor, held for approval, edited or dropped before any app receives it.

**Key points:**

- Enabled by default, with Block / Ask / Allow at the global, per-client, per-server and per-event-type levels
- The global settings are the on/off switch and the default permission; everything else is per client, and every bound (journal size, poll page size, webhook retries) is fixed internally
- Push is exposed to a client only when at least one upstream server it can already use offers push; Push Relay never widens a client's MCP permissions
- Every delivery form shares the same event id, so an app that combines mechanisms can deduplicate
- Apps with no native push mechanism get LocalRouter's polling tools instead

<!-- @entry push-relay-push-types -->

Every upstream push type reaches every delivery form through LocalRouter's event store. An upstream that is polled can be streamed to an app; an upstream stream can be read by an app that only polls; a Claude Code Channel message from one server can be delivered as an MCP Events occurrence, and the other way around.

The one lossy edge is Claude Code Channels: a channel message carries no cursor, replay or acknowledgement. Delivery over Channels is therefore a wake-up, and the agent is pointed at `EventPoll` to read the authoritative event journal.

<!-- @entry push-relay-ingress -->

### From Upstream Servers

| Push type | Transport | How LocalRouter consumes it |
| --- | --- | --- |
| MCP Events stream (`events/stream`) | STDIO, HTTP | Holds the stream, resumes from the last cursor on reconnect, watches heartbeats |
| MCP Events poll (`events/poll`) | STDIO, HTTP | Polls at the server's suggested interval, only while some app consumes that event type |
| Claude Code Channel (`notifications/claude/channel`) | STDIO | Becomes the event type `<server>__channel.message` |

When a server offers several modes, LocalRouter prefers the stream, then polling. Upstream subscriptions are opened only while at least one app consumes the event type, and closed when the last one goes.

**Upstream webhooks are not consumed.** Receiving them would need a public inbound HTTPS endpoint, and every webhook-capable server found also offers streaming or polling, which LocalRouter uses instead. An event type a server offers only by webhook is shown in the UI as "webhook-only — not supported".

Event names are qualified with the server name, as tools are: `vocalcord__message.received`.

<!-- @entry push-relay-egress -->

### To Apps

| Delivery form | For | Notes |
| --- | --- | --- |
| MCP Events stream and poll | Clients implementing MCP Events | Advertised under the MCP Events extension and the top-level `events` capability |
| MCP Events webhook | OpenClaw and other webhook subscribers | LocalRouter signs and POSTs each event to the client's callback URL |
| Claude Code Channel | Claude Code | STDIO bridge only — see Claude Code below |
| Fallback tools | Every other client | `EventList`, `EventPoll`, `EventWait` |

The client chooses a native mode by calling it (`events/stream`, `events/poll` or `events/subscribe`). Webhook subscriptions are always accepted. Webhook deliveries are outbound only and happen after the event has passed permissions, Ask and Monitor. Callback URLs must be HTTPS, except `http://` to a loopback address (`127.0.0.1`, `::1`, `localhost`), which is always accepted for local clients, and the first delivery to a new callback URL asks for approval.

<!-- @entry push-relay-permissions -->

Each event type resolves to one of three actions, the most specific level winning:

1. Global default — **Allow**
2. Per client
3. Per client, per upstream server
4. Per client, per server, per event type

- **Allow** — the event is delivered and recorded in Monitor.
- **Block** — the event type disappears for that client: it is absent from `events/list`, Channels and the fallback tools, and the upstream is not subscribed on that client's behalf.
- **Ask** — every individual event of that type is held for approval.

<!-- @entry push-relay-ask -->

### Ask: Held Events

A held event is not in the client's journal, so no delivery form can see it until it is decided. The approval view offers:

- **Allow** this event
- **Edit, then allow** — change the event's data (and the rendered channel text) before delivery
- **Deny** this event — dropped for this client only, and recorded
- **Allow or deny this event type** for the session or permanently, written back to the permission tree

Events of one subscription stay in order: later events of the same type wait behind a held one. A held event has no timeout: it waits until you decide, however long that takes. One approval popup is open per client and event type at a time, showing how many more are waiting, with an "allow all pending of this type" action; two clients consuming the same upstream event are approved independently. To bound memory, a client can have at most 1,000 held events; further Ask events are denied and recorded in Monitor.

<!-- @entry push-relay-monitor -->

### Monitor: Intercept and Edit

Every event is recorded in Monitor once per client as an **MCP Push Event**, with its source server, event name, how it arrived, how it was delivered, the decision (allowed, asked then allowed, asked then edited, denied, blocked, or dropped for being too large or over the held-event limit) and a payload preview under the existing privacy and redaction settings. Push events are listed under the MCP type filter.

- Held events appear as pending, and open the same approval and edit view inline.
- Any event record offers "Ask for this event type from now on", the quick way to start intercepting a stream that is currently allowed.
- Edits are never invisible: Monitor keeps both the original and the delivered payload, subject to privacy settings.
- **Grouping.** A push event joins the request and conversation of the one client session that reads it, so it sits beside that session's tool calls and LLM calls (and the `EventPoll` call that read it). It stands alone when no single session reads it: when it is delivered by webhook or through `/events/watch`, when several sessions of the client read the same subscription, or when the session has no recorded request (as with a client connected directly to the MCP gateway, whose tool calls are likewise ungrouped). Monitor never joins events by client or timing alone.

<!-- @entry push-relay-fallback-tools -->

Clients without a native push mechanism get three built-in tools, shown only when at least one permitted upstream offers push:

| Tool | What it does |
| --- | --- |
| `EventList` | Lists the event types this client may receive: name, source server, description, filter schema, and which native modes it could use instead |
| `EventPoll` | Returns events since a cursor, then `next_cursor`. Reading with a cursor acknowledges. Optional `wait_seconds` (up to 50) makes it a bounded long-poll. Without a cursor it starts at "now" |
| `EventWait` | Never blocks. Returns a ready-to-run `curl -N` command against a local LocalRouter endpoint that prints one line per event — **multi-shot** (stays armed, for agents that can watch a background command) or **one-shot** (exits after the first event, for agents that can only block) |

Agents are told not to call `EventPoll` in a loop, to run the `EventWait` command as a background task instead, and that event content is untrusted data, never instructions. Upstream servers' own waiting tools stay available.

The tools are automatic: there is no setting. They are shown whenever push is available to the client, unless the client declared MCP Events support when it connected (`extensions["io.modelcontextprotocol/events"]` or a top-level `events` capability). Claude Code sessions through the STDIO bridge keep all three: a server cannot tell whether Claude was launched with Channels, and without the flag channel messages are dropped silently.

<!-- @entry push-relay-client-setup -->

| Client | What Push Relay uses |
| --- | --- |
| Claude Code | STDIO bridge → Claude Code Channels, plus fallback tools |
| OpenClaw | MCP Events webhook delivery |
| Codex | Fallback tools (multi-shot `EventWait`) |
| Cursor | Fallback tools (multi-shot) |
| OpenCode, Goose | Fallback tools (one-shot) |
| Droid, Pi, Onyx | Fallback tools |
| Claude Desktop, claude.ai | Fallback tools |

Cloud-hosted clients such as ChatGPT cannot reach LocalRouter on `127.0.0.1` and are out of scope.

<!-- @entry push-relay-claude-code -->

### Claude Code (STDIO Bridge + Channels)

Claude Code receives pushed events through **Channels**, which has two requirements:

- **Channels is STDIO only.** Claude Code accepts channel messages only from an MCP server it launched as a subprocess, not from an HTTP server.
- **Channels requires the pre-2026-07-28 handshake.** Claude Code does not register a channel server that connects on MCP revision 2026-07-28. The LocalRouter STDIO bridge declines that revision toward Claude Code and connects on the earlier `initialize` handshake, so it registers as a channel. This applies to LocalRouter alone; nothing needs to be set for the rest of the Claude session.

LocalRouter over HTTP can still serve Claude Code's tools, but cannot wake Claude when an event arrives. That is why the Claude Code template uses the STDIO bridge by default:

```json
"localrouter": {
  "type": "stdio",
  "command": "<LocalRouter binary>",
  "args": ["--mcp-bridge", "--client-id", "<client id>"],
  "env": { "LOCALROUTER_CLIENT_SECRET": "<secret>" }
}
```

While Channels is a research preview, Claude Code must be launched with the development-channels flag naming the server:

```bash
claude --dangerously-load-development-channels server:localrouter
```

- The flag works in interactive sessions only; it is ignored with `-p` and by the Agent SDK.
- An organisation's `channelsEnabled` policy can block it.
- Without the flag, channel messages are dropped silently. LocalRouter cannot detect the flag, which is why the fallback tools stay visible for Claude Code too.
- After adding a push-capable upstream server, restart `claude`.

Existing HTTP configurations are not rewritten; the client page offers a "switch to STDIO bridge" action, and the HTTP setup remains available as a tools-only alternative.

<!-- @entry push-relay-openclaw -->

### OpenClaw (Webhook)

OpenClaw's `mcp-events` plugin subscribes by webhook. It calls `events/subscribe` on LocalRouter with a callback URL and secret; LocalRouter obtains the events from upstream however the upstream offers them, runs them through permissions, Ask and Monitor, and POSTs each one with Standard Webhooks signatures (`webhook-id` equal to the event id). A new callback URL is verified with a challenge and asks for approval before its first delivery. Subscriptions are leases the client refreshes; deliveries happen only while LocalRouter is running.

<!-- @entry push-relay-other-clients -->

### Codex, Cursor and Others

No other shipping client implements MCP Events today, so they use the fallback tools. Agents that can watch a background command (Codex, Cursor, Cline, Claude Code's `Monitor`) run `EventWait`'s multi-shot command; agents that can only block on a command (OpenCode, Goose, Copilot, Gemini CLI) use the one-shot form. When a client gains native MCP Events support, it can call `events/stream` or `events/poll` directly and the tools step aside.
