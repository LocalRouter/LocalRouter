# Proxy Passthrough Visibility

**Date**: 2026-08-25
**Status**: implemented

## Problem

`HTTPS_PROXY` is a **process-wide** setting. Once a client (Claude Code, Codex,
a shell profile, …) points it at LocalRouter's inspection proxy, *every* HTTPS
egress from that process tree goes through LocalRouter — `git push`, `npm
install`, crash telemetry, OAuth token refreshes, update checks. Only LLM API
hosts are decrypted; everything else is blind-tunneled.

Two consequences:

1. **Silent.** Non-LLM traffic leaves no trace at all, so a user has no way to
   discover that their git client is being proxied.
2. **Fragile.** The proxy only spoke `CONNECT`. A plain-HTTP proxy request
   (`GET http://host/path HTTP/1.1`, absolute-form — what every client sends
   when it resolves the proxy for an `http://` URL) was answered with a
   protocol error and a dropped socket.

## Goals

- Anything that is not a recognized LLM API call is forwarded **as-is**, byte
  for byte, so nothing breaks.
- Those forwards are **visible in the Monitor** — with the destination
  (host/port, and method/path when it is legitimately visible) so the user can
  recognize "that's my git client" — carrying an explanation and a warning.
- **No content.** No request/response bodies, no headers, no query strings, for
  passthrough traffic — not in the Monitor, not in the logs. LocalRouter only
  cares about LLM calls.

## Design

### New monitor event: `proxy_passthrough`

`MonitorEventData::ProxyPassthrough` (category `proxy`):

| field | meaning |
|---|---|
| `mode` | `tunnel` \| `http` \| `inspected` \| `websocket` |
| `host`, `port` | destination |
| `method`, `path` | only when the request line was visible in cleartext; **query stripped** |
| `status_code` | only for `inspected` (we already parsed that response head) |
| `bytes_sent` / `bytes_received` | byte counts for tunnels (volume, never content) |
| `note` | human-readable explanation + warning |

Modes:

- **`tunnel`** — a `CONNECT` to a host that is not on the MITM allow-list. TLS
  is never terminated; LocalRouter copies bytes in both directions. Only
  `host:port` is knowable.
- **`http`** — an absolute-form plain-HTTP proxy request. Forwarded to the
  origin in origin-form with the hop-by-hop proxy headers stripped.
- **`inspected`** — a decrypted request on an allow-listed LLM host whose path
  is not a recognized LLM API path (auth preflight, `/v1/models`, telemetry).
- **`websocket`** — an upgraded connection on an inspected host that is not a
  recognized LLM path; relayed frame-for-frame without inspection.

Events open `Pending` when the connection starts and complete when it closes,
so an in-flight tunnel is visible while it is live.

### Transport

`read_request_head` replaces `read_connect` and returns either a `Connect` or a
`PlainHttp` head. Plain HTTP is authenticated with the same
`Proxy-Authorization` credentials, then forwarded verbatim.

Passthrough recording is **only** done for the MITM proxy
(`ExchangeSource::Proxy`). The reverse proxy deliberately wraps a provider's
port, so its non-LLM paths (`/api/tags`, …) are not accidental and stay
unrecorded.

### Interceptor seam

Two new `ProxyInterceptor` hooks, so the transport stays protocol-agnostic:

```rust
fn begin_passthrough(&self, ex: &PassthroughExchange) -> Option<String>;
fn end_passthrough(&self, event_id: Option<String>, ex: &PassthroughExchange);
```

`PassiveInterceptor` implements them; `ActiveInterceptor` delegates. The
`inspected` mode needs no transport change: `emit_pending` / `complete` /
`record` already run for every decrypted request, and now branch to a
passthrough event when `wire::detect` returns `None`.

## Files

- `crates/lr-monitor/src/types.rs`, `summary.rs` — event type + data + summary
- `crates/lr-proxy/src/interceptor.rs` — `PassthroughExchange`, hooks
- `crates/lr-proxy/src/passive.rs` — recording
- `crates/lr-proxy/src/active.rs` — delegation
- `crates/lr-proxy/src/transport.rs` — plain-HTTP forwarding, tunnel accounting
- `src/types/tauri-commands.ts` — `'proxy_passthrough'`
- `src/views/monitor/{event-list,event-filters,event-detail}.tsx` — UI

## Final steps

1. Plan review against implementation
2. Test coverage review (unit + e2e)
3. Bug hunt
4. Commit, push, release
