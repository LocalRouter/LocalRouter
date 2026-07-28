//! WebSocket support for the MITM data-path.
//!
//! Codex CLI upgrades `GET /backend-api/codex/responses` to a websocket, sends
//! each request as one text message (`{"type":"response.create", ...}`), and
//! receives text messages that are exactly the JSON events the SSE transport
//! carries in `data:` lines. One connection is reused across turns, so a
//! single websocket holds many request/response cycles.
//!
//! The relay here is **message-aware on the client→upstream direction**: every
//! client data message runs through [`ProxyInterceptor::on_request`] — the same
//! firewall hook the HTTP path uses — before being re-framed and forwarded, so
//! model rules, rate limits, and Ask-mode approval apply to websocket traffic
//! too. The upstream→client direction is a pure byte tee (never rewritten):
//! server messages are only decoded for capture.
//!
//! Captured server messages are buffered in SSE shape (`data: {...}\n\n`) so
//! the whole existing SSE reconstruction pipeline (model, tokens, previews,
//! cost) is reused unchanged. The transport strips
//! `Sec-WebSocket-Extensions` before forwarding the upgrade, so no
//! compression is ever negotiated and every frame payload stays plaintext.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Instant;

use serde_json::Value;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::interceptor::{ObservedExchange, ProxyInterceptor, RequestAction};
use crate::wire::{self, WireFormat};

/// Hard cap on a single decoded websocket message (and the decoder's internal
/// buffer). Far above any real LLM payload; exceeding it poisons the decoder
/// and the relay shuts the connection down rather than ferrying traffic the
/// firewall could not inspect.
const MAX_MESSAGE: usize = 64 * 1024 * 1024;

/// Cap on the SSE-shaped capture buffer per request/response cycle. Matches
/// the HTTP path's per-body capture cap.
const CAPTURE_CAP: usize = 1024 * 1024;

// ---------------------------------------------------------------------------
// Frame codec
// ---------------------------------------------------------------------------

/// One complete websocket message (fragments already reassembled).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WsMessage {
    Text(String),
    Binary(Vec<u8>),
    Ping(Vec<u8>),
    Pong(Vec<u8>),
    Close(Vec<u8>),
}

impl WsMessage {
    fn opcode(&self) -> u8 {
        match self {
            WsMessage::Text(_) => 0x1,
            WsMessage::Binary(_) => 0x2,
            WsMessage::Close(_) => 0x8,
            WsMessage::Ping(_) => 0x9,
            WsMessage::Pong(_) => 0xA,
        }
    }

    fn payload(&self) -> &[u8] {
        match self {
            WsMessage::Text(s) => s.as_bytes(),
            WsMessage::Binary(b)
            | WsMessage::Ping(b)
            | WsMessage::Pong(b)
            | WsMessage::Close(b) => b,
        }
    }
}

/// Incremental RFC 6455 frame decoder: feed it raw stream bytes, get complete
/// messages back. Handles masking (client→server frames), 16/64-bit payload
/// lengths, fragmentation, and interleaved control frames.
///
/// Any protocol surprise — RSV bits (an extension we never negotiated),
/// unknown opcodes, broken fragmentation, oversized frames — poisons the
/// decoder permanently; the caller decides what to do (the relay closes the
/// connection).
#[derive(Default)]
pub struct FrameDecoder {
    buf: Vec<u8>,
    frag_opcode: Option<u8>,
    frag: Vec<u8>,
    poisoned: bool,
}

enum Step {
    Message(WsMessage),
    /// Consumed a non-final fragment; nothing to deliver yet.
    Consumed,
    Incomplete,
    Poison,
}

impl FrameDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_poisoned(&self) -> bool {
        self.poisoned
    }

    /// Feed raw bytes; returns every message completed by them, in order.
    pub fn push(&mut self, bytes: &[u8]) -> Vec<WsMessage> {
        let mut out = Vec::new();
        if self.poisoned {
            return out;
        }
        self.buf.extend_from_slice(bytes);
        loop {
            match self.step() {
                Step::Message(m) => out.push(m),
                Step::Consumed => {}
                Step::Incomplete => break,
                Step::Poison => {
                    self.poisoned = true;
                    self.buf.clear();
                    self.frag.clear();
                    break;
                }
            }
        }
        out
    }

    /// Try to consume exactly one frame from the front of the buffer.
    fn step(&mut self) -> Step {
        if self.buf.len() < 2 {
            return Step::Incomplete;
        }
        let b0 = self.buf[0];
        let b1 = self.buf[1];
        if b0 & 0x70 != 0 {
            // RSV bits imply an extension (e.g. permessage-deflate) — the
            // transport refuses extension negotiation, so this is a protocol
            // violation we cannot parse through.
            return Step::Poison;
        }
        let fin = b0 & 0x80 != 0;
        let opcode = b0 & 0x0F;
        let masked = b1 & 0x80 != 0;

        let (payload_len, len_bytes) = match b1 & 0x7F {
            126 => {
                if self.buf.len() < 4 {
                    return Step::Incomplete;
                }
                (u64::from(u16::from_be_bytes([self.buf[2], self.buf[3]])), 2)
            }
            127 => {
                if self.buf.len() < 10 {
                    return Step::Incomplete;
                }
                let mut be = [0u8; 8];
                be.copy_from_slice(&self.buf[2..10]);
                (u64::from_be_bytes(be), 8)
            }
            n => (u64::from(n), 0),
        };
        if payload_len > MAX_MESSAGE as u64 {
            return Step::Poison;
        }
        let payload_len = payload_len as usize;
        let header = 2 + len_bytes + if masked { 4 } else { 0 };
        let total = header + payload_len;
        if self.buf.len() < total {
            return Step::Incomplete;
        }

        let mut payload = self.buf[header..total].to_vec();
        if masked {
            let key = [
                self.buf[2 + len_bytes],
                self.buf[3 + len_bytes],
                self.buf[4 + len_bytes],
                self.buf[5 + len_bytes],
            ];
            for (i, b) in payload.iter_mut().enumerate() {
                *b ^= key[i % 4];
            }
        }
        self.buf.drain(..total);

        // Control frames: never fragmented, payload ≤ 125 (RFC 6455 §5.5).
        // Rejecting oversized/fragmented ones matters because the relay
        // re-encodes each control frame as a single FIN frame.
        if (0x8..=0xA).contains(&opcode) && (!fin || payload_len > 125) {
            return Step::Poison;
        }
        match opcode {
            0x8 => return Step::Message(WsMessage::Close(payload)),
            0x9 => return Step::Message(WsMessage::Ping(payload)),
            0xA => return Step::Message(WsMessage::Pong(payload)),
            0x0..=0x2 => {}
            _ => return Step::Poison,
        }

        // Data frames, possibly fragmented.
        match (opcode, self.frag_opcode) {
            // Unfragmented message.
            (0x1 | 0x2, None) if fin => match Self::assemble(opcode, payload) {
                Some(msg) => Step::Message(msg),
                None => Step::Poison,
            },
            // First fragment.
            (0x1 | 0x2, None) => {
                self.frag_opcode = Some(opcode);
                self.frag = payload;
                Step::Consumed
            }
            // Continuation.
            (0x0, Some(op)) => {
                if self.frag.len() + payload.len() > MAX_MESSAGE {
                    return Step::Poison;
                }
                self.frag.extend_from_slice(&payload);
                if fin {
                    self.frag_opcode = None;
                    match Self::assemble(op, std::mem::take(&mut self.frag)) {
                        Some(msg) => Step::Message(msg),
                        None => Step::Poison,
                    }
                } else {
                    Step::Consumed
                }
            }
            // Continuation without a start, or a new message mid-fragmentation.
            _ => Step::Poison,
        }
    }

    /// Build a data message from a reassembled payload. Text frames must be
    /// valid UTF-8 (RFC 6455 §5.6) — rejecting invalid ones keeps decode →
    /// re-encode byte-exact, so a forwarded message is never altered.
    fn assemble(opcode: u8, payload: Vec<u8>) -> Option<WsMessage> {
        if opcode == 0x1 {
            String::from_utf8(payload).ok().map(WsMessage::Text)
        } else {
            Some(WsMessage::Binary(payload))
        }
    }
}

/// Encode one message as a single frame. Client→server frames must be masked
/// (RFC 6455 §5.3); any key is protocol-valid, so a cheap rolling key is fine.
pub fn encode_frame(msg: &WsMessage, mask: bool) -> Vec<u8> {
    static KEY_SEED: AtomicU32 = AtomicU32::new(0x9E37_79B9);
    let payload = msg.payload();
    let mut out = Vec::with_capacity(payload.len() + 14);
    out.push(0x80 | msg.opcode());
    let mask_bit = if mask { 0x80 } else { 0x00 };
    match payload.len() {
        n if n < 126 => out.push(mask_bit | n as u8),
        n if n <= 0xFFFF => {
            out.push(mask_bit | 126);
            out.extend_from_slice(&(n as u16).to_be_bytes());
        }
        n => {
            out.push(mask_bit | 127);
            out.extend_from_slice(&(n as u64).to_be_bytes());
        }
    }
    if mask {
        let key = KEY_SEED
            .fetch_add(0x9E37_79B9, Ordering::Relaxed)
            .to_be_bytes();
        out.extend_from_slice(&key);
        out.extend(payload.iter().enumerate().map(|(i, b)| b ^ key[i % 4]));
    } else {
        out.extend_from_slice(payload);
    }
    out
}

// ---------------------------------------------------------------------------
// Session: per-connection firewall + capture state
// ---------------------------------------------------------------------------

/// What the relay should do with one client data message, decided by the
/// interceptor (firewall).
enum ClientAction {
    Forward,
    Replace(Vec<u8>),
    /// Do not forward; answer the client with this synthesized error event.
    Reject(String),
}

/// One in-flight request/response cycle on the websocket.
struct Cycle {
    event_id: Option<String>,
    request_body: Vec<u8>,
    /// Server messages, re-shaped as an SSE stream (`data: {...}\n\n`) so the
    /// existing reconstruction pipeline parses them unchanged.
    capture: String,
    started: Instant,
}

/// Firewall + monitor capture for one intercepted LLM websocket connection.
pub struct WsSession {
    interceptor: Arc<dyn ProxyInterceptor>,
    format: WireFormat,
    /// Exchange template: client identity, host, path — cloned per cycle.
    base: ObservedExchange,
    cycle: parking_lot::Mutex<Option<Cycle>>,
}

impl WsSession {
    pub fn new(
        interceptor: Arc<dyn ProxyInterceptor>,
        format: WireFormat,
        base: ObservedExchange,
    ) -> Self {
        Self {
            interceptor,
            format,
            base,
            cycle: parking_lot::Mutex::new(None),
        }
    }

    /// Run one client data message through the firewall and open a monitor
    /// cycle for it when it is allowed through.
    async fn on_client_message(&self, payload: &[u8]) -> ClientAction {
        let mut ex = self.base.clone();
        ex.request_body = Some(payload.to_vec());
        match self.interceptor.on_request(&ex).await {
            RequestAction::Forward => {
                self.begin_cycle(payload.to_vec()).await;
                ClientAction::Forward
            }
            RequestAction::Replace(body) => {
                self.begin_cycle(body.clone()).await;
                ClientAction::Replace(body)
            }
            RequestAction::Reject { status, body, .. } => {
                let reply = reject_event(status, &body);
                // Record the blocked call in one push, like the HTTP path.
                let mut blocked = self.base.clone();
                blocked.request_body = Some(payload.to_vec());
                blocked.status = Some(status);
                blocked.response_body = Some(reply.clone().into_bytes());
                blocked.latency_ms = Some(0);
                self.interceptor.on_response(&blocked).await;
                ClientAction::Reject(reply)
            }
        }
    }

    /// Open a Pending monitor event for a new cycle (closing out any cycle the
    /// server never terminated).
    async fn begin_cycle(&self, request_body: Vec<u8>) {
        self.finish_cycle(None).await;
        let mut ex = self.base.clone();
        ex.request_body = Some(request_body.clone());
        let event_id = self.interceptor.begin(&ex);
        *self.cycle.lock() = Some(Cycle {
            event_id,
            request_body,
            capture: String::new(),
            started: Instant::now(),
        });
    }

    /// Capture one server message; completes the open cycle when the message
    /// is a terminal event for this wire format.
    async fn on_server_message(&self, text: &str) {
        let json = serde_json::from_str::<Value>(text).ok();
        let terminal = json
            .as_ref()
            .is_some_and(|j| wire::is_terminal_event(self.format, j));
        {
            let mut guard = self.cycle.lock();
            let Some(cycle) = guard.as_mut() else {
                return;
            };
            // Framed as one SSE event: "data: " + payload + "\n\n".
            let needed = text.len() + 8;
            if cycle.capture.len() + needed > CAPTURE_CAP {
                // Over the cap: drop deltas, but never the terminal event —
                // it carries the whole response object (model, usage, output),
                // so it wins the remaining room by dropping earlier deltas.
                if !terminal || needed > CAPTURE_CAP {
                    return;
                }
                cycle.capture.clear();
                cycle
                    .capture
                    .push_str("data: {\"type\":\"localrouter.truncated\"}\n\n");
            }
            cycle.capture.push_str("data: ");
            cycle.capture.push_str(text);
            cycle.capture.push_str("\n\n");
        }
        if terminal {
            let status = json.map(|j| wire::terminal_status(self.format, &j));
            self.finish_cycle(status).await;
        }
    }

    /// Complete the open cycle, if any. `status: None` marks it as ended
    /// without a terminal event (disconnect) → recorded as an error with
    /// whatever was salvaged, mirroring the SSE disconnect path.
    async fn finish_cycle(&self, status: Option<u16>) {
        let Some(cycle) = self.cycle.lock().take() else {
            return;
        };
        let mut ex = self.base.clone();
        ex.event_id = cycle.event_id;
        ex.request_body = Some(cycle.request_body);
        ex.status = status;
        ex.response_body = (!cycle.capture.is_empty()).then(|| cycle.capture.into_bytes());
        ex.response_is_sse = true;
        ex.latency_ms = Some(cycle.started.elapsed().as_millis() as u64);
        self.interceptor.on_response(&ex).await;
    }

    /// The connection ended: close out any cycle still open.
    pub async fn on_close(&self) {
        self.finish_cycle(None).await;
    }
}

/// Shape a firewall `Reject` as the wrapped error event websocket clients
/// understand (`{"type":"error","status":..,"error":{..}}` — Codex maps this
/// to a proper request failure instead of a dead connection).
fn reject_event(status: u16, body: &[u8]) -> String {
    let mut json = serde_json::from_slice::<Value>(body)
        .ok()
        .filter(Value::is_object)
        .unwrap_or_else(|| {
            serde_json::json!({
                "error": { "type": "localrouter_firewall",
                           "message": String::from_utf8_lossy(body) }
            })
        });
    let obj = json.as_object_mut().expect("object ensured above");
    obj.insert("type".into(), "error".into());
    obj.insert("status".into(), status.into());
    json.to_string()
}

// ---------------------------------------------------------------------------
// Relay
// ---------------------------------------------------------------------------

/// Ferry an upgraded websocket connection between client and upstream.
///
/// With a [`WsSession`] (a recognized LLM path), client data messages are
/// decoded, firewalled, and re-framed; server bytes are forwarded verbatim
/// and decoded only for capture. Without one (any other websocket on a
/// MITM'd host), both directions are a blind byte copy.
///
/// If either decoder poisons (protocol violation / oversized frame), the
/// relay shuts the connection down: we never ferry LLM traffic the firewall
/// could not inspect. Clients fall back to their HTTPS transport.
pub async fn relay<C, U>(client: C, upstream: U, session: Option<Arc<WsSession>>)
where
    C: AsyncRead + AsyncWrite + Send,
    U: AsyncRead + AsyncWrite + Send,
{
    let (mut client_rd, client_wr) = tokio::io::split(client);
    let (mut upstream_rd, mut upstream_wr) = tokio::io::split(upstream);
    // Shared because rejections are answered from the client→upstream task.
    let client_wr = Arc::new(tokio::sync::Mutex::new(client_wr));

    let c2u = {
        let session = session.clone();
        let client_wr = client_wr.clone();
        async move {
            let mut decoder = FrameDecoder::new();
            let mut buf = vec![0u8; 16 * 1024];
            'conn: loop {
                let n = match client_rd.read(&mut buf).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                };
                let Some(session) = session.as_ref() else {
                    if upstream_wr.write_all(&buf[..n]).await.is_err() {
                        break;
                    }
                    continue;
                };
                for msg in decoder.push(&buf[..n]) {
                    let forwarded = match msg {
                        WsMessage::Text(_) | WsMessage::Binary(_) => {
                            // Note: while the firewall deliberates (e.g. an
                            // Ask popup), later client frames simply wait —
                            // natural backpressure, same as the HTTP path.
                            match session.on_client_message(msg.payload()).await {
                                ClientAction::Forward => encode_frame(&msg, true),
                                ClientAction::Replace(body) => {
                                    let replaced = match msg {
                                        WsMessage::Text(_) => WsMessage::Text(
                                            String::from_utf8_lossy(&body).into_owned(),
                                        ),
                                        _ => WsMessage::Binary(body),
                                    };
                                    encode_frame(&replaced, true)
                                }
                                ClientAction::Reject(reply) => {
                                    // Answer the client directly; the upstream
                                    // never sees the request.
                                    let frame = encode_frame(&WsMessage::Text(reply), false);
                                    if client_wr.lock().await.write_all(&frame).await.is_err() {
                                        break 'conn;
                                    }
                                    continue;
                                }
                            }
                        }
                        control => encode_frame(&control, true),
                    };
                    if upstream_wr.write_all(&forwarded).await.is_err() {
                        break 'conn;
                    }
                }
                if decoder.is_poisoned() {
                    tracing::warn!(
                        "websocket client stream unparseable; closing intercepted connection"
                    );
                    break;
                }
            }
            let _ = upstream_wr.shutdown().await;
        }
    };

    let u2c = {
        let session = session.clone();
        let client_wr = client_wr.clone();
        async move {
            let mut decoder = FrameDecoder::new();
            let mut buf = vec![0u8; 16 * 1024];
            loop {
                let n = match upstream_rd.read(&mut buf).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                };
                // Tee: the client always receives the upstream bytes verbatim.
                if client_wr.lock().await.write_all(&buf[..n]).await.is_err() {
                    break;
                }
                let Some(session) = session.as_ref() else {
                    continue;
                };
                for msg in decoder.push(&buf[..n]) {
                    match msg {
                        WsMessage::Text(t) => session.on_server_message(&t).await,
                        WsMessage::Binary(b) => {
                            session
                                .on_server_message(&String::from_utf8_lossy(&b))
                                .await
                        }
                        _ => {}
                    }
                }
                if decoder.is_poisoned() {
                    tracing::warn!(
                        "websocket upstream stream unparseable; closing intercepted connection"
                    );
                    break;
                }
            }
            let _ = client_wr.lock().await.shutdown().await;
        }
    };

    tokio::join!(c2u, u2c);
    if let Some(session) = session {
        session.on_close().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use lr_monitor::{EventStatus, MonitorEventData, MonitorEventStore};
    use serde_json::json;

    // --- codec ---

    fn masked(msg: &WsMessage) -> Vec<u8> {
        encode_frame(msg, true)
    }
    fn unmasked(msg: &WsMessage) -> Vec<u8> {
        encode_frame(msg, false)
    }

    #[test]
    fn roundtrips_masked_and_unmasked_frames() {
        for mask in [true, false] {
            let mut d = FrameDecoder::new();
            let msg = WsMessage::Text("hello".into());
            let out = d.push(&encode_frame(&msg, mask));
            assert_eq!(out, vec![msg]);
        }
    }

    #[test]
    fn decodes_byte_by_byte() {
        let mut d = FrameDecoder::new();
        let frame = masked(&WsMessage::Text("x".repeat(300))); // 16-bit length
        let mut out = Vec::new();
        for b in frame {
            out.extend(d.push(&[b]));
        }
        assert_eq!(out, vec![WsMessage::Text("x".repeat(300))]);
    }

    #[test]
    fn decodes_64bit_length_frames() {
        let payload = "y".repeat(70_000);
        let mut frame = vec![0x81, 127];
        frame.extend_from_slice(&(payload.len() as u64).to_be_bytes());
        frame.extend_from_slice(payload.as_bytes());
        let mut d = FrameDecoder::new();
        assert_eq!(d.push(&frame), vec![WsMessage::Text(payload)]);
    }

    #[test]
    fn reassembles_fragments_with_interleaved_control() {
        let mut d = FrameDecoder::new();
        let mut bytes = Vec::new();
        // First fragment (text, no FIN).
        bytes.extend([0x01, 0x03]);
        bytes.extend(b"Hel");
        // Interleaved ping.
        bytes.extend(unmasked(&WsMessage::Ping(b"p".to_vec())));
        // Final continuation.
        bytes.extend([0x80, 0x02]);
        bytes.extend(b"lo");
        let out = d.push(&bytes);
        assert_eq!(
            out,
            vec![
                WsMessage::Ping(b"p".to_vec()),
                WsMessage::Text("Hello".into())
            ]
        );
    }

    #[test]
    fn poisons_on_rsv_bits_and_unknown_opcodes() {
        let mut d = FrameDecoder::new();
        assert!(d.push(&[0xC1, 0x00]).is_empty()); // RSV1 set
        assert!(d.is_poisoned());

        let mut d = FrameDecoder::new();
        assert!(d.push(&[0x83, 0x00]).is_empty()); // opcode 0x3 reserved
        assert!(d.is_poisoned());

        // Continuation without a start.
        let mut d = FrameDecoder::new();
        assert!(d.push(&[0x80, 0x01, 0x41]).is_empty());
        assert!(d.is_poisoned());
    }

    #[test]
    fn poisons_on_invalid_utf8_text_and_bad_control_frames() {
        // Invalid UTF-8 in a text frame: rejecting it keeps decode → re-encode
        // byte-exact, so a forwarded message can never be altered.
        let mut d = FrameDecoder::new();
        assert!(d.push(&[0x81, 0x02, 0xFF, 0xFE]).is_empty());
        assert!(d.is_poisoned());

        // Fragmented control frame (FIN clear).
        let mut d = FrameDecoder::new();
        assert!(d.push(&[0x09, 0x01, 0x41]).is_empty());
        assert!(d.is_poisoned());

        // Control frame with a >125-byte payload.
        let mut d = FrameDecoder::new();
        let mut frame = vec![0x89, 126, 0x00, 0x80];
        frame.extend(std::iter::repeat_n(0x41, 128));
        assert!(d.push(&frame).is_empty());
        assert!(d.is_poisoned());
    }

    #[tokio::test]
    async fn oversized_capture_keeps_the_terminal_event() {
        let store = Arc::new(MonitorEventStore::new(16));
        let interceptor = Arc::new(PassiveInterceptor::new(store.clone()));
        let session = WsSession::new(interceptor, WireFormat::OpenAiResponses, base_exchange());

        session
            .on_client_message(response_create().as_bytes())
            .await;
        // Flood past the capture cap with deltas...
        let delta =
            json!({"type":"response.output_text.delta","delta":"x".repeat(4096)}).to_string();
        for _ in 0..(CAPTURE_CAP / 4096 + 2) {
            session.on_server_message(&delta).await;
        }
        // ...then the terminal event, which must still be captured.
        session.on_server_message(&completed_event()).await;

        let events = store.list(0, 10, None);
        let ev = store.get(&events.events[0].id).expect("event exists");
        assert_eq!(ev.status, EventStatus::Complete);
        let MonitorEventData::LlmCall {
            model,
            input_tokens,
            ..
        } = &ev.data
        else {
            panic!("expected LlmCall event");
        };
        assert_eq!(model, "gpt-5.5");
        assert_eq!(*input_tokens, Some(12));
    }

    #[test]
    fn multiple_messages_in_one_push() {
        let mut d = FrameDecoder::new();
        let mut bytes = masked(&WsMessage::Text("a".into()));
        bytes.extend(masked(&WsMessage::Text("b".into())));
        let out = d.push(&bytes);
        assert_eq!(
            out,
            vec![WsMessage::Text("a".into()), WsMessage::Text("b".into())]
        );
    }

    // --- session + relay ---

    use crate::interceptor::{ClientCtx, ConnectDecision};
    use crate::passive::PassiveInterceptor;

    fn base_exchange() -> ObservedExchange {
        ObservedExchange {
            client_id: "client-1".into(),
            host: "chatgpt.com".into(),
            method: "GET".into(),
            path: "/backend-api/codex/responses".into(),
            ..Default::default()
        }
    }

    fn response_create() -> String {
        json!({
            "type": "response.create",
            "model": "gpt-5.5",
            "stream": true,
            "input": [{"role": "user", "content": [{"type": "input_text", "text": "hi"}]}],
        })
        .to_string()
    }

    fn completed_event() -> String {
        json!({
            "type": "response.completed",
            "response": {
                "id": "resp_1", "model": "gpt-5.5", "status": "completed",
                "output": [{"type": "message", "content": [{"type": "output_text", "text": "hey"}]}],
                "usage": {"input_tokens": 12, "output_tokens": 3},
            }
        })
        .to_string()
    }

    #[tokio::test]
    async fn session_records_one_event_per_cycle() {
        let store = Arc::new(MonitorEventStore::new(16));
        let interceptor = Arc::new(PassiveInterceptor::new(store.clone()));
        let session = WsSession::new(interceptor, WireFormat::OpenAiResponses, base_exchange());

        for _ in 0..2 {
            let action = session
                .on_client_message(response_create().as_bytes())
                .await;
            assert!(matches!(action, ClientAction::Forward));
            session
                .on_server_message(
                    &json!({"type":"response.output_text.delta","delta":"he"}).to_string(),
                )
                .await;
            session.on_server_message(&completed_event()).await;
        }
        session.on_close().await;

        let events = store.list(0, 10, None);
        assert_eq!(events.events.len(), 2, "one monitor event per cycle");
        for summary in &events.events {
            let ev = store.get(&summary.id).expect("event exists");
            assert_eq!(ev.status, EventStatus::Complete);
            let MonitorEventData::LlmCall {
                model,
                message_count,
                stream,
                request_body,
                input_tokens,
                output_tokens,
                status_code,
                content_preview,
                ..
            } = &ev.data
            else {
                panic!("expected LlmCall event");
            };
            // The symptom that started this: websocket calls used to land with
            // an empty model, zero messages, and a null request body.
            assert_eq!(model, "gpt-5.5");
            assert_eq!(*message_count, 1);
            assert!(*stream);
            assert_eq!(request_body["model"], "gpt-5.5");
            assert_eq!(*input_tokens, Some(12));
            assert_eq!(*output_tokens, Some(3));
            assert_eq!(*status_code, Some(200));
            assert_eq!(content_preview.as_deref(), Some("hey"));
        }
    }

    #[tokio::test]
    async fn disconnect_mid_cycle_records_error_with_salvage() {
        let store = Arc::new(MonitorEventStore::new(16));
        let interceptor = Arc::new(PassiveInterceptor::new(store.clone()));
        let session = WsSession::new(interceptor, WireFormat::OpenAiResponses, base_exchange());

        session
            .on_client_message(response_create().as_bytes())
            .await;
        session
            .on_server_message(
                &json!({"type":"response.output_text.delta","delta":"par"}).to_string(),
            )
            .await;
        session.on_close().await;

        let events = store.list(0, 10, None);
        assert_eq!(events.events.len(), 1);
        let ev = store.get(&events.events[0].id).expect("event exists");
        assert_eq!(ev.status, EventStatus::Error);
        let MonitorEventData::LlmCall {
            content_preview, ..
        } = &ev.data
        else {
            panic!("expected LlmCall event");
        };
        // The partial delta was still salvaged.
        assert_eq!(content_preview.as_deref(), Some("par"));
    }

    /// Interceptor that rejects every request (a firewall deny).
    struct RejectAll(PassiveInterceptor);
    #[async_trait]
    impl ProxyInterceptor for RejectAll {
        fn on_connect(&self, _host: &str, _client: &ClientCtx) -> ConnectDecision {
            ConnectDecision::Mitm
        }
        async fn on_request(&self, _ex: &ObservedExchange) -> RequestAction {
            RequestAction::reject_json(403, "Model 'gpt-5.5' is not permitted")
        }
        async fn on_response(&self, ex: &ObservedExchange) {
            self.0.on_response(ex).await
        }
    }

    #[tokio::test]
    async fn relay_rejects_firewalled_request_without_contacting_upstream() {
        let store = Arc::new(MonitorEventStore::new(16));
        let interceptor = Arc::new(RejectAll(PassiveInterceptor::new(store.clone())));
        let session = Arc::new(WsSession::new(
            interceptor,
            WireFormat::OpenAiResponses,
            base_exchange(),
        ));

        let (client_side, proxy_client_end) = tokio::io::duplex(64 * 1024);
        let (mut upstream_side, proxy_upstream_end) = tokio::io::duplex(64 * 1024);
        let relay_task = tokio::spawn(relay(proxy_client_end, proxy_upstream_end, Some(session)));

        // Client sends a masked request message.
        let (mut client_rd, mut client_wr) = tokio::io::split(client_side);
        client_wr
            .write_all(&encode_frame(&WsMessage::Text(response_create()), true))
            .await
            .unwrap();

        // Client receives the synthesized error event.
        let mut d = FrameDecoder::new();
        let mut buf = [0u8; 4096];
        let reply = loop {
            let n = client_rd.read(&mut buf).await.unwrap();
            assert!(n > 0, "relay closed without replying");
            if let Some(WsMessage::Text(t)) = d.push(&buf[..n]).pop() {
                break t;
            }
        };
        let reply: Value = serde_json::from_str(&reply).unwrap();
        assert_eq!(reply["type"], "error");
        assert_eq!(reply["status"], 403);

        // The upstream never received the request. (Client hangs up → the
        // relay shuts its upstream write half → read_to_end sees EOF.)
        drop(client_wr);
        drop(client_rd);
        let mut upstream_bytes = Vec::new();
        upstream_side
            .read_to_end(&mut upstream_bytes)
            .await
            .unwrap();
        assert!(
            upstream_bytes.is_empty(),
            "rejected request must not reach the upstream"
        );
        drop(upstream_side);
        relay_task.await.unwrap();

        // And the blocked call was recorded.
        let events = store.list(0, 10, None);
        assert_eq!(events.events.len(), 1);
        assert_eq!(events.events[0].status, EventStatus::Error);
    }

    #[tokio::test]
    async fn relay_ferries_and_captures_a_full_cycle() {
        let store = Arc::new(MonitorEventStore::new(16));
        let interceptor = Arc::new(PassiveInterceptor::new(store.clone()));
        let session = Arc::new(WsSession::new(
            interceptor,
            WireFormat::OpenAiResponses,
            base_exchange(),
        ));

        let (client_side, proxy_client_end) = tokio::io::duplex(64 * 1024);
        let (upstream_side, proxy_upstream_end) = tokio::io::duplex(64 * 1024);
        let relay_task = tokio::spawn(relay(proxy_client_end, proxy_upstream_end, Some(session)));

        let (mut client_rd, mut client_wr) = tokio::io::split(client_side);
        let (mut upstream_rd, mut upstream_wr) = tokio::io::split(upstream_side);

        // Client → upstream: masked request arrives re-framed but intact.
        client_wr
            .write_all(&encode_frame(&WsMessage::Text(response_create()), true))
            .await
            .unwrap();
        let mut d = FrameDecoder::new();
        let mut buf = [0u8; 8192];
        let forwarded = loop {
            let n = upstream_rd.read(&mut buf).await.unwrap();
            assert!(n > 0);
            if let Some(WsMessage::Text(t)) = d.push(&buf[..n]).pop() {
                break t;
            }
        };
        assert_eq!(
            serde_json::from_str::<Value>(&forwarded).unwrap()["model"],
            "gpt-5.5"
        );

        // Upstream → client: server events are teed to the client verbatim.
        upstream_wr
            .write_all(&encode_frame(&WsMessage::Text(completed_event()), false))
            .await
            .unwrap();
        let mut d = FrameDecoder::new();
        let received = loop {
            let n = client_rd.read(&mut buf).await.unwrap();
            assert!(n > 0);
            if let Some(WsMessage::Text(t)) = d.push(&buf[..n]).pop() {
                break t;
            }
        };
        assert_eq!(received, completed_event());

        drop(client_wr);
        drop(client_rd);
        drop(upstream_wr);
        drop(upstream_rd);
        relay_task.await.unwrap();

        // The cycle was recorded with parsed model + usage.
        let events = store.list(0, 10, None);
        assert_eq!(events.events.len(), 1);
        let ev = store.get(&events.events[0].id).expect("event exists");
        assert_eq!(ev.status, EventStatus::Complete);
        let MonitorEventData::LlmCall {
            model,
            input_tokens,
            ..
        } = &ev.data
        else {
            panic!("expected LlmCall event");
        };
        assert_eq!(model, "gpt-5.5");
        assert_eq!(*input_tokens, Some(12));
    }
}
