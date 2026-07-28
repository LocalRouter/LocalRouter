# HTTPS Proxy: WebSocket Support (Codex Responses transport)

**Date**: 2026-07-28
**Status**: In progress

## Problem

Codex CLI (0.144.x) now prefers a **WebSocket transport** for the ChatGPT
backend Responses API: it upgrades `GET /backend-api/codex/responses` to a
websocket, sends the request as a text message
`{"type":"response.create", ...ResponsesApiRequest fields...}`, and receives
text messages that are byte-for-byte the same JSON events that the SSE
transport carries in `data:` lines (`response.created`,
`response.output_text.delta`, …, `response.completed`). One connection is
reused across turns (one `response.create` → `response.completed` cycle per
turn). The client negotiates `permessage-deflate` compression.

Our MITM data-path (`lr-proxy/src/transport.rs`) is an HTTP/1.1
request/response ferry and never completes the upgrade, so:

1. Codex's websocket dies (`⚠ Falling back from WebSockets to HTTPS
   transport. stream disconnected before completion: failed to send websocket
   request: Connection closed normally`).
2. The upgrade GET itself was recorded as a garbage `llm_call` monitor event:
   `status_code: 101`, `model: ""`, `message_count: 0`, `request_body: null`.
3. The active firewall never sees the model/body (they live inside websocket
   messages, not the upgrade request), so a blind byte-tunnel would have been
   a firewall bypass.

## Design

### Transport (`transport.rs`)

- Detect `Upgrade: websocket` requests in `proxy_request`.
- Strip `Sec-WebSocket-Extensions` before forwarding (same reasoning as the
  existing `Accept-Encoding` strip: refuse compression negotiation so every
  frame payload stays parseable plaintext; the client falls back to
  uncompressed frames per RFC 6455).
- Forward the upgrade request upstream. On a `101` response, complete the
  upgrade on both sides (`hyper::upgrade::on` + `.with_upgrades()` on both the
  server and client connection drivers) and hand both raw streams to the
  websocket relay. On non-101 (auth failure etc.) fall through to the normal
  tapped-body path, which already records error statuses.
- The upgrade request itself is **not** recorded as an llm_call event —
  monitor events are recorded per message cycle by the relay (fixes the
  garbage 101 event).
- The upgrade request also bypasses `on_request` (the firewall): it carries
  no LLM request body, and Ask-mode would otherwise pop a model-less approval
  dialog for the handshake. Every websocket message is firewalled instead.

### WebSocket relay (`websocket.rs`, new)

Frame codec:
- Streaming `FrameDecoder`: handles masking (client→server), 16/64-bit
  payload lengths, fragmentation/continuation, interleaved control frames.
  Any RSV bit (unexpected — we refused extensions) or oversized
  message/buffer poisons the decoder.
- `encode_frame`: single-frame encoder (with client-side masking) used to
  re-emit client messages and synthesize firewall rejections.

Relay, message-aware on the client→upstream direction:
- Each complete client **data message** goes through
  `ProxyInterceptor::on_request` (the same firewall hook the HTTP path uses):
  - `Forward` → re-encode and send upstream.
  - `Replace(body)` → send the rewritten body instead (Ask-popup edits work).
  - `Reject{..}` → do **not** forward; send a synthesized
    `{"type":"error", "status": .., "error": {..}}` text message back to the
    client (Codex's wrapped-error handling understands this shape) and record
    the blocked cycle to the monitor.
- Control frames (ping/pong/close) are re-encoded and forwarded as-is.
- On an allowed request message, `begin()` opens a Pending monitor event
  (same as the HTTP path), with the message JSON as the request body.

Upstream→client direction is a pure byte tee (never rewritten):
- Bytes are forwarded verbatim; a second decoder extracts server messages
  for capture only.
- Each server text message is appended to the open cycle's capture buffer in
  SSE format (`data: {...}\n\n`, capped at 1 MiB) so the **entire existing
  SSE reconstruction pipeline is reused unchanged** for parsing, previews,
  tokens, cost, and metrics (`response_is_sse = true`).
- `wire::is_terminal_event(format, &json)` decides when a cycle is complete
  (`response.completed` / `response.failed` / `response.incomplete` /
  `error` for OpenAI Responses; `message_stop` / `error` for Anthropic, so a
  future Anthropic websocket transport is captured with zero new code);
  `wire::terminal_status` maps the terminal event to an HTTP-ish status for
  the monitor event (200 for completed/incomplete, the error event's own
  `status` when present, else 502).
- Connection close with a cycle still open → the cycle completes as an error
  with whatever was salvaged (mirrors the SSE disconnect salvage path).
- If either decoder poisons, the relay shuts the connection down (never
  ferries unparseable traffic past the firewall); Codex falls back to SSE.

### Shared parsing hardening (`wire.rs`, `anthropic.rs`, `openai.rs`)

- New `wire::sse_json_events(raw) -> Vec<Value>`: a single spec-correct SSE
  event splitter (CRLF tolerant, multi-line `data:` accumulation, comments /
  `event:` / `id:` fields ignored, `[DONE]` skipped, malformed JSON skipped).
  All three reconstructors (Anthropic Messages, OpenAI Chat, OpenAI
  Responses) now iterate its output instead of hand-rolling per-line
  parsing — one place to harden, shared by Claude, Codex, and any future
  format.
- The websocket relay reuses the same reconstruction by emitting an
  SSE-shaped capture buffer, so SSE and websocket transports share one
  parser end-to-end.

## Files

- `crates/lr-proxy/src/websocket.rs` — new: codec + relay + cycle capture.
- `crates/lr-proxy/src/transport.rs` — upgrade detection + wiring.
- `crates/lr-proxy/src/wire.rs` — `sse_json_events`, terminal-event helpers.
- `crates/lr-proxy/src/anthropic.rs`, `openai.rs` — use shared splitter.
- `crates/lr-proxy/src/lib.rs` — module registration.
- `crates/lr-proxy/tests/mitm_e2e.rs` — websocket e2e case.

## Final steps (mandatory)

1. **Plan Review** — re-read this plan against the implementation; close gaps.
2. **Test Coverage Review** — codec unit tests (masking, fragmentation,
   lengths, partial feeds, poison), relay tests over `tokio::io::duplex`
   (capture, firewall reject, multi-cycle), e2e MITM websocket test
   (faithful ferry + extensions stripped + monitor event recorded).
3. **Bug Hunt** — fresh-eyes pass over framing math, cap handling, cycle
   state transitions, upgrade error paths.
4. **Commit** — conventional commit, only files touched by this work.
