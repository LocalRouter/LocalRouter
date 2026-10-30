# MCP Bridge Mode

`localrouter --mcp-bridge` runs LocalRouter as a **STDIO ↔ HTTP bridge**: an MCP
client that launches stdio servers (Claude Code, Claude Desktop, Cursor, VS Code,
…) starts the bridge as a subprocess, and the bridge relays MCP in both
directions between the client's stdin/stdout and the running LocalRouter app's
unified MCP gateway.

The bridge is the only way Claude Code can receive pushed events from
LocalRouter: Claude Code Channels (`notifications/claude/channel`) are delivered
only by a stdio server that connects on the handshake from before MCP revision
2026-07-28. Over plain HTTP, LocalRouter can serve tools but cannot wake Claude
when an event arrives.

## Architecture

```
MCP client (e.g. Claude Code)
    ↕ stdio: newline-delimited JSON-RPC
localrouter --mcp-bridge
    → POST  /mcp   every client message (requests, notifications, responses)
    ← GET   /mcp   held SSE stream: server notifications and requests
    → DELETE /mcp  ends the session when stdin closes
    Authorization: Bearer <client secret>
    X-LocalRouter-Bridge: stdio
    Mcp-Session-Id: <issued by initialize>
    ↕
LocalRouter app (must already be running) → MCP gateway → MCP servers
```

The bridge loads the config only to find the client's secret and the gateway
address; it starts no managers and no MCP servers itself.

## What the bridge does

**Client → LocalRouter.** Each stdin line is one JSON-RPC message.

- *Requests* are POSTed concurrently, so a long `tools/call` or a long-lived
  `events/stream` never blocks the next request.
- `initialize` is POSTed first and on its own. The `Mcp-Session-Id` header of
  its response is kept and sent on every later request, the GET stream and the
  final DELETE, so every message lands on the same gateway session (the one
  that recorded the client's capabilities).
- *Notifications* (no `id`) are POSTed in order. LocalRouter answers `202
  Accepted` with an empty body, and the bridge writes nothing to stdout.
- *Responses* (an `id` with `result` or `error` and no `method`) are the
  client's answers to server-initiated requests (sampling, elicitation, roots).
  They are POSTed so the gateway can resolve the pending request; nothing is
  written to stdout.
- Unparseable lines get a `-32700` parse error; JSON that is not a JSON-RPC
  message (including batch arrays) gets `-32600`.

**LocalRouter → client.**

- A POST may be answered with a JSON body or with an SSE stream (Streamable
  HTTP). Every JSON-RPC message in an SSE body is written to stdout in order,
  until the stream ends. No total timeout applies: an `events/stream` request
  delivers notifications for as long as it is open, then its final result. If a
  stream ends without answering the request, the client gets an error for it.
- A request answered `202 Accepted` is answered later on the GET stream.
- After `initialize`, the bridge holds the session's GET SSE stream and writes
  every message on it to stdout **verbatim**: list-changed notifications,
  progress, logging, server-initiated requests, and methods the bridge does not
  know about (`notifications/claude/channel`, `notifications/events/*`). If the
  stream drops, the bridge reconnects with backoff (250 ms doubling to 30 s).
  The legacy `endpoint` event is ignored.
- Errors keep the request's `id`. A JSON-RPC error body on a non-2xx status is
  forwarded as is; otherwise the bridge writes a `-32603` error explaining the
  status (bad credentials, no MCP access, unknown session, app not running).
- All stdout output goes through a single writer, one message per line, so
  concurrent responses and notifications never interleave.

**Shutdown.** On stdin EOF (or when stdout closes) the bridge stops the GET
stream and any in-flight requests, sends `DELETE` with the session id, flushes
what is already queued for stdout, and exits.

## Protocol version toward the client

The bridge does **not** support MCP revision 2026-07-28 toward its stdio client,
whatever the gateway behind it speaks. Claude Code probes stdio servers for that
revision and does not register a server that negotiates it as a channel; a
server that declines connects on the earlier handshake and registers as before.
This affects only LocalRouter, unlike Claude Code's `MCP_PROTOCOL_NEGOTIATION`
variable, which LocalRouter neither needs nor sets.

| Client sends | Bridge answers |
| --- | --- |
| `server/discover` (the 2026-07-28 probe) | `-32601 Method not found`, no `data`, without contacting LocalRouter. The 2026-07-28 stdio transport and TypeScript SDK v2 treat this as a legacy server and fall back to `initialize`. `-32022 UnsupportedProtocolVersion` is never sent: it marks a server as modern and forbids the fallback. |
| Any other request before `initialize` whose `_meta["io.modelcontextprotocol/protocolVersion"]` is newer than 2025-11-25 | `-32600 Server not initialized`, not forwarded |
| `initialize` asking for a version newer than 2025-11-25 | Forwarded asking for 2025-11-25, with any 2026-07-28 `_meta` version declaration removed |
| `initialize` asking for 2025-11-25 or older | Forwarded unchanged |
| An `initialize` result whose `protocolVersion` is newer than what was negotiated | `protocolVersion` rewritten to the negotiated version |

Subsequent requests carry `MCP-Protocol-Version` with the negotiated version.

The bridge keeps no state between processes and binds nothing, so a client that
probes on a short-lived extra copy of the server (as TypeScript SDK v2's base
stdio transport does) is harmless.

## Setup

1. In LocalRouter, create a client (or use an existing one) and grant it MCP
   access. Its secret is shown in the app.
2. Keep the LocalRouter app running; the bridge connects to it.
3. Configure your MCP client to launch the bridge:

```json
{
  "mcpServers": {
    "localrouter": {
      "type": "stdio",
      "command": "/Applications/LocalRouter.app/Contents/MacOS/localrouter",
      "args": ["--mcp-bridge", "--client-id", "<client id>"],
      "env": { "LOCALROUTER_CLIENT_SECRET": "<client secret>" }
    }
  }
}
```

For Claude Code, launch with
`claude --dangerously-load-development-channels server:localrouter` to receive
channel messages. The flag works only in interactive sessions, an
organisation's `channelsEnabled` policy can block it, and an upstream server
that newly offers push takes effect after restarting `claude`.

### Options

| Option | Meaning |
| --- | --- |
| `--client-id <id>` | Client to authenticate as. Without it, the first enabled client with MCP access is used. |
| `--url <url>` | Gateway URL, e.g. `http://127.0.0.1:3625/mcp`. |

| Environment variable | Meaning |
| --- | --- |
| `LOCALROUTER_CLIENT_SECRET` | Client secret. Without it the secret is read from the OS keychain (the app must have run once). |
| `LOCALROUTER_URL` | Gateway URL, used when `--url` is not given. |
| `LOCALROUTER_KEYCHAIN` | Force the keychain type (`file` or `system`). `file` stores secrets in plain text; use it only for development. |
| `RUST_LOG` | Log filter, e.g. `lr_mcp=debug`. |

### Gateway address

Without `--url` or `LOCALROUTER_URL`, the bridge uses the server host and port
from the LocalRouter config it loads: `http://127.0.0.1:3625/mcp` for release
builds and `http://127.0.0.1:33625/mcp` for debug builds (which also use the
`~/.localrouter-dev` config). A host of `0.0.0.0` or `::` is reached through
loopback.

If the app found its configured port taken at startup it listens on the next
free port, which the bridge cannot discover; pass `--url` in that case.

## Logging

The bridge logs to **stderr** only; stdout carries nothing but JSON-RPC. MCP
clients usually keep a server's stderr in their own logs (Claude Code:
`claude --debug`; Claude Desktop: `~/Library/Logs/Claude/mcp*.log`).

```bash
RUST_LOG=lr_mcp=debug localrouter --mcp-bridge --client-id <id>
```

## Troubleshooting

| Message | Cause |
| --- | --- |
| `Could not connect to LocalRouter at …. Is the app running?` | The app is not running, or is listening elsewhere (see "Gateway address"). |
| `Client '…' not found` / `is disabled` | Wrong `--client-id`, or the client is disabled in LocalRouter. |
| `Client secret not found` | Set `LOCALROUTER_CLIENT_SECRET`, or run the app once so the secret is in the keychain. |
| `Invalid client credentials` | The secret is wrong; copy it again from the app. |
| `not allowed to access MCP servers` | Grant the client MCP access in LocalRouter. |
| `LocalRouter no longer knows this MCP session` | The app restarted; restart the MCP client to start a new session. |
| `LocalRouter offers no server stream` (stderr) | The gateway refused the GET stream; server notifications cannot reach the client. |

## Limitations

- The LocalRouter app must be running; the bridge is not a standalone gateway.
- Server-initiated requests (sampling, elicitation, roots) reach the client
  only when the gateway delivers them on the session's GET stream and resolves
  the client's POSTed response by its `Mcp-Session-Id`. The bridge does both of
  its parts; see the Push Relay plan for the gateway side.
- The server → client channel is the Streamable HTTP GET stream; the bridge
  does not use `/mcp/ws` or 2026-07-28 `subscriptions/listen`.
