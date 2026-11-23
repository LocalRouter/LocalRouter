//! WebSocket support for the MITM data-path.
//!
//! Codex CLI upgrades `GET /backend-api/codex/responses` to a websocket, sends
//! each request as one text message (`{"type":"response.create", ...}`), and
//! receives text messages that are exactly the JSON events the SSE transport
//! carries in `data:` lines. One connection is reused across turns, so a
//! single websocket holds many request/response cycles.
//!
//! # Client → upstream
//!
//! The client direction is decoded frame by frame ([`FrameReader`]): payload
//! bytes are unmasked as they arrive, without waiting for a whole frame or
//! message. Every client data message is judged by the firewall:
//!
//! - A message of at most the stream lag (and any message the firewall does
//!   not let stream) is held whole and run through
//!   [`ProxyInterceptor::on_request`] — the same hook the HTTP path uses —
//!   before it is re-framed and forwarded. A refusal is answered with a
//!   synthesized error event and the connection stays open.
//! - A larger message whose [`ProxyInterceptor::body_policy`] is
//!   [`BodyPolicy::Stream`] is offered to [`ProxyInterceptor::open_stream`]
//!   once the HTTP [`RequestPipeline`] is ready, and then forwarded through
//!   that pipeline while it is still arriving: released output is
//!   re-fragmented (data opcode on the first frame, continuations, FIN on the
//!   last), client pings and pongs are forwarded between the fragments. If
//!   the stream has to stop after a fragment went out, the upstream is
//!   closed with 1008 so the partial message is never completed, the client
//!   gets the error event and a 1011 close.
//!
//! Small content-free control messages (`response.cancel`, Codex's
//! `response.interrupt`) are recognized by a bounded parse and forwarded
//! without a firewall decision or a cycle of their own.
//!
//! # Upstream → client
//!
//! Server messages are reassembled whole (they are small events, capped at
//! [`MAX_SERVER_MESSAGE`]); protected responses are restored before client
//! delivery and monitoring retains the masked provider messages. Captured
//! server messages are buffered in SSE shape (`data: {...}\n\n`) so the
//! whole existing SSE reconstruction pipeline (model, tokens, previews,
//! cost) is reused unchanged. The transport strips `Sec-WebSocket-Extensions`
//! before forwarding the upgrade, so no compression is ever negotiated and
//! every frame payload stays plaintext.

use std::sync::atomic::{AtomicU32, AtomicU8, Ordering};
use std::sync::Arc;
use std::time::Instant;

use serde_json::Value;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::interceptor::{
    BodyPolicy, BoundedCapture, ObservedExchange, ProxyInterceptor, RequestAction, RequestHead,
    StreamOpen, StreamPolicy, StreamedRequest,
};
use crate::request_stream::{RequestPipeline, StreamAbort, StreamSession, StreamSummary};
use crate::wire::{self, WireFormat};

/// Most bytes of one client message the relay holds. A message the firewall
/// judges whole (one it does not let stream) is refused above it with 1009
/// (Message Too Big), and Codex retries the request over HTTPS; a streamed
/// message is not limited in size, only in what its pipeline holds at once.
/// Also the largest single frame accepted from the client.
pub(crate) const MAX_MESSAGE: usize = 256 * 1024 * 1024;

/// Most bytes of one server message. Server messages are single events
/// (deltas, a terminal event carrying the response); they are reassembled
/// whole for restoration and routing. Above it the relay closes with 1009.
pub(crate) const MAX_SERVER_MESSAGE: usize = 64 * 1024 * 1024;

/// Largest frame payload the relay writes. Data messages above it are sent as
/// a fragmented message, the way Codex frames large requests: the upstream
/// accepts large messages only in bounded frames.
const MAX_FRAME: usize = 1024 * 1024;

/// Close codes (RFC 6455 §7.4.1).
const CLOSE_GOING_AWAY: u16 = 1001;
const CLOSE_PROTOCOL_ERROR: u16 = 1002;
const CLOSE_POLICY: u16 = 1008;
const CLOSE_TOO_BIG: u16 = 1009;
const CLOSE_INTERNAL: u16 = 1011;

/// Cap on the SSE-shaped capture buffer per request/response cycle. Matches
/// the HTTP path's per-body capture cap.
const CAPTURE_CAP: usize = 1024 * 1024;

/// Largest client message checked for being a control message. Control
/// messages are a few hundred bytes; anything larger is a request.
const CONTROL_SNIFF: usize = 64 * 1024;

/// Client bytes of a streamed message gathered before a pipeline step runs
/// (the step also runs at the end of every read).
const STREAM_STEP: usize = 256 * 1024;

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

/// Why a decoder stopped parsing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Poison {
    /// A frame or reassembled message exceeds the decoder's limit.
    TooBig,
    /// The stream violates RFC 6455 in a way the relay cannot re-frame.
    Protocol,
}

impl Poison {
    /// The close code that tells the peer why the relay closed.
    pub fn close_code(self) -> u16 {
        match self {
            Poison::TooBig => CLOSE_TOO_BIG,
            Poison::Protocol => CLOSE_PROTOCOL_ERROR,
        }
    }

    fn reason(self, limit: usize) -> String {
        match self {
            Poison::TooBig => format!(
                "WebSocket message exceeds LocalRouter's {} MiB limit",
                limit / (1024 * 1024)
            ),
            Poison::Protocol => "WebSocket stream could not be parsed".into(),
        }
    }
}

/// One step of a decoded frame stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameEvent {
    /// The first frame of a data message began (`0x1` text, `0x2` binary).
    DataStart { opcode: u8 },
    /// Unmasked payload bytes of the data message in progress, delivered as
    /// they arrive (never empty).
    DataChunk(Vec<u8>),
    /// The data message in progress ended (its FIN frame is complete).
    DataEnd,
    /// A complete control frame, possibly between two fragments of a data
    /// message.
    Control(WsMessage),
}

/// The frame whose payload is being read.
struct OpenFrame {
    fin: bool,
    opcode: u8,
    remaining: u64,
    key: Option<[u8; 4]>,
    /// Position in the masking key of the next payload byte.
    offset: usize,
    /// Payload of a control frame, collected whole (at most 125 bytes).
    control: Vec<u8>,
}

/// Incremental RFC 6455 frame reader: feed it raw stream bytes, get frame
/// events back as soon as the bytes arrive. Data payloads are unmasked
/// incrementally (the masking-key offset is carried across reads), so a
/// large frame is never waited for. Handles 16/64-bit payload lengths,
/// fragmentation and control frames between fragments.
///
/// Any protocol surprise — RSV bits (an extension we never negotiated),
/// unknown opcodes, broken fragmentation, malformed control or close frames,
/// a frame above the limit — poisons the reader permanently; the caller
/// decides what to do (the relay closes the connection).
pub struct FrameReader {
    max_frame: u64,
    header: Vec<u8>,
    frame: Option<OpenFrame>,
    /// Opcode of the data message in progress.
    message: Option<u8>,
    poisoned: Option<Poison>,
}

impl FrameReader {
    /// A reader that refuses frames whose payload exceeds `max_frame` bytes.
    pub fn new(max_frame: usize) -> Self {
        Self {
            max_frame: max_frame as u64,
            header: Vec::with_capacity(14),
            frame: None,
            message: None,
            poisoned: None,
        }
    }

    /// Why the reader stopped, once it has.
    pub fn poison(&self) -> Option<Poison> {
        self.poisoned
    }

    /// Header bytes needed for the frame being started, given what arrived.
    fn header_len(&self) -> usize {
        let Some(&b1) = self.header.get(1) else {
            return 2;
        };
        let extended = match b1 & 0x7F {
            126 => 2,
            127 => 8,
            _ => 0,
        };
        2 + extended + if b1 & 0x80 != 0 { 4 } else { 0 }
    }

    /// Feed raw bytes; appends every event they complete, in order.
    pub fn push(&mut self, mut bytes: &[u8], out: &mut Vec<FrameEvent>) {
        while self.poisoned.is_none() && !bytes.is_empty() {
            let Some(frame) = self.frame.as_mut() else {
                let want = self.header_len();
                let take = (want - self.header.len()).min(bytes.len());
                self.header.extend_from_slice(&bytes[..take]);
                bytes = &bytes[take..];
                if self.header.len() < self.header_len() {
                    continue;
                }
                if let Err(poison) = self.start_frame(out) {
                    self.poisoned = Some(poison);
                }
                continue;
            };
            let n = frame.remaining.min(bytes.len() as u64) as usize;
            let mut chunk = bytes[..n].to_vec();
            bytes = &bytes[n..];
            if let Some(key) = frame.key {
                for (i, byte) in chunk.iter_mut().enumerate() {
                    *byte ^= key[(frame.offset + i) % 4];
                }
                frame.offset = (frame.offset + n) % 4;
            }
            frame.remaining -= n as u64;
            if frame.opcode & 0x8 != 0 {
                frame.control.extend_from_slice(&chunk);
            } else if !chunk.is_empty() {
                out.push(FrameEvent::DataChunk(chunk));
            }
            if frame.remaining == 0 {
                if let Err(poison) = self.end_frame(out) {
                    self.poisoned = Some(poison);
                }
            }
        }
        if self.poisoned.is_some() {
            self.header.clear();
            self.frame = None;
        }
    }

    /// The header is complete: validate it and open its frame.
    fn start_frame(&mut self, out: &mut Vec<FrameEvent>) -> Result<(), Poison> {
        let header = std::mem::take(&mut self.header);
        let (b0, b1) = (header[0], header[1]);
        if b0 & 0x70 != 0 {
            // RSV bits imply an extension (e.g. permessage-deflate) — the
            // transport refuses extension negotiation, so this is a protocol
            // violation we cannot parse through.
            return Err(Poison::Protocol);
        }
        let fin = b0 & 0x80 != 0;
        let opcode = b0 & 0x0F;
        let masked = b1 & 0x80 != 0;
        let (len, len_bytes) = match b1 & 0x7F {
            126 => (u64::from(u16::from_be_bytes([header[2], header[3]])), 2),
            127 => {
                let mut be = [0u8; 8];
                be.copy_from_slice(&header[2..10]);
                let len = u64::from_be_bytes(be);
                // The most significant bit must be 0 (RFC 6455 §5.2).
                if len >> 63 != 0 {
                    return Err(Poison::Protocol);
                }
                (len, 8)
            }
            n => (u64::from(n), 0),
        };
        let control = opcode & 0x8 != 0;
        if !matches!(opcode, 0x0..=0x2 | 0x8..=0xA) {
            return Err(Poison::Protocol);
        }
        // Control frames: never fragmented, payload ≤ 125 (RFC 6455 §5.5).
        // Rejecting oversized/fragmented ones matters because the relay
        // re-encodes each control frame as a single FIN frame.
        if control && (!fin || len > 125) {
            return Err(Poison::Protocol);
        }
        if len > self.max_frame {
            return Err(Poison::TooBig);
        }
        if !control {
            match (opcode, self.message) {
                (0x1 | 0x2, None) => {
                    self.message = Some(opcode);
                    out.push(FrameEvent::DataStart { opcode });
                }
                (0x0, Some(_)) => {}
                // Continuation without a start, or a new message
                // mid-fragmentation.
                _ => return Err(Poison::Protocol),
            }
        }
        let key = masked.then(|| {
            let at = 2 + len_bytes;
            [header[at], header[at + 1], header[at + 2], header[at + 3]]
        });
        self.header = header;
        self.header.clear();
        self.frame = Some(OpenFrame {
            fin,
            opcode,
            remaining: len,
            key,
            offset: 0,
            control: Vec::new(),
        });
        if len == 0 {
            self.end_frame(out)?;
        }
        Ok(())
    }

    /// The open frame's payload is complete.
    fn end_frame(&mut self, out: &mut Vec<FrameEvent>) -> Result<(), Poison> {
        let frame = self.frame.take().expect("an open frame");
        match frame.opcode {
            0x8 => {
                // A close body is empty, or a code plus a UTF-8 reason.
                let body = &frame.control;
                if body.len() == 1 || (body.len() > 2 && std::str::from_utf8(&body[2..]).is_err()) {
                    return Err(Poison::Protocol);
                }
                out.push(FrameEvent::Control(WsMessage::Close(frame.control)));
            }
            0x9 => out.push(FrameEvent::Control(WsMessage::Ping(frame.control))),
            0xA => out.push(FrameEvent::Control(WsMessage::Pong(frame.control))),
            _ if frame.fin => {
                self.message = None;
                out.push(FrameEvent::DataEnd);
            }
            _ => {}
        }
        Ok(())
    }
}

/// Incremental message decoder: feed it raw stream bytes, get complete
/// messages back. Built on [`FrameReader`]; reassembles fragmented data
/// messages up to a size limit and passes control frames through in order.
pub struct FrameDecoder {
    reader: FrameReader,
    max_message: usize,
    opcode: u8,
    frag: Vec<u8>,
    poisoned: Option<Poison>,
}

impl Default for FrameDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl FrameDecoder {
    pub fn new() -> Self {
        Self::with_limit(MAX_MESSAGE)
    }

    /// A decoder that refuses messages (and frames) above `max_message`.
    pub fn with_limit(max_message: usize) -> Self {
        Self {
            reader: FrameReader::new(max_message),
            max_message,
            opcode: 0,
            frag: Vec::new(),
            poisoned: None,
        }
    }

    pub fn is_poisoned(&self) -> bool {
        self.poison().is_some()
    }

    /// Why the decoder stopped, once it has.
    pub fn poison(&self) -> Option<Poison> {
        self.poisoned.or(self.reader.poison())
    }

    /// Feed raw bytes; returns every message completed by them, in order.
    pub fn push(&mut self, bytes: &[u8]) -> Vec<WsMessage> {
        let mut out = Vec::new();
        if self.is_poisoned() {
            return out;
        }
        let mut events = Vec::new();
        self.reader.push(bytes, &mut events);
        for event in events {
            match event {
                FrameEvent::DataStart { opcode } => {
                    self.opcode = opcode;
                    self.frag.clear();
                }
                FrameEvent::DataChunk(chunk) => {
                    if self.frag.len() + chunk.len() > self.max_message {
                        self.poisoned = Some(Poison::TooBig);
                        self.frag = Vec::new();
                        break;
                    }
                    self.frag.extend_from_slice(&chunk);
                }
                FrameEvent::DataEnd => {
                    match Self::assemble(self.opcode, std::mem::take(&mut self.frag)) {
                        Some(message) => out.push(message),
                        None => {
                            self.poisoned = Some(Poison::Protocol);
                            break;
                        }
                    }
                }
                FrameEvent::Control(message) => out.push(message),
            }
        }
        out
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
    let mut out = Vec::with_capacity(msg.payload().len() + 14);
    push_frame(&mut out, true, msg.opcode(), msg.payload(), mask);
    out
}

/// Encode one message for the wire. Data messages above [`MAX_FRAME`] become
/// a fragmented message (first frame with the data opcode, continuations,
/// FIN on the last), so a large message the client sent in bounded frames
/// reaches the peer in bounded frames too. Text may split inside a UTF-8
/// sequence: RFC 6455 validates text per message, not per frame.
pub fn encode_message(msg: &WsMessage, mask: bool) -> Vec<u8> {
    let payload = msg.payload();
    let data = matches!(msg, WsMessage::Text(_) | WsMessage::Binary(_));
    if !data || payload.len() <= MAX_FRAME {
        return encode_frame(msg, mask);
    }
    let mut out = Vec::with_capacity(payload.len() + payload.len().div_ceil(MAX_FRAME) * 14);
    push_fragments(&mut out, msg.opcode(), payload, true, mask);
    out
}

/// Append `payload` as frames of at most [`MAX_FRAME`] bytes. `opcode` is
/// the first frame's (`0x0` to continue a message already started); FIN is
/// set on the last frame when `fin`. An empty payload writes one empty frame.
fn push_fragments(out: &mut Vec<u8>, opcode: u8, payload: &[u8], fin: bool, mask: bool) {
    if payload.is_empty() {
        push_frame(out, fin, opcode, payload, mask);
        return;
    }
    let frames = payload.len().div_ceil(MAX_FRAME);
    for (index, chunk) in payload.chunks(MAX_FRAME).enumerate() {
        let opcode = if index == 0 { opcode } else { 0x0 };
        push_frame(out, fin && index + 1 == frames, opcode, chunk, mask);
    }
}

fn push_frame(out: &mut Vec<u8>, fin: bool, opcode: u8, payload: &[u8], mask: bool) {
    static KEY_SEED: AtomicU32 = AtomicU32::new(0x9E37_79B9);
    out.push(if fin { 0x80 } else { 0x00 } | opcode);
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
}

/// A close frame carrying `code` and a short reason (RFC 6455 §5.5.1: the
/// whole control payload is at most 125 bytes, so the reason is cut at a
/// character boundary to fit).
fn close_frame(code: u16, mask: bool, reason: &str) -> Vec<u8> {
    let mut end = reason.len().min(123);
    while !reason.is_char_boundary(end) {
        end -= 1;
    }
    let mut payload = code.to_be_bytes().to_vec();
    payload.extend_from_slice(&reason.as_bytes()[..end]);
    encode_frame(&WsMessage::Close(payload), mask)
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

/// The request half of a cycle, for the monitor.
enum CycleRequest {
    /// The whole (forwarded) message, judged buffered.
    Body(Vec<u8>),
    /// A streamed message: the head of the forwarded bytes, and what the
    /// pipeline learned about it once it was forwarded completely.
    Streamed(Box<(BoundedCapture, StreamedRequest)>),
}

/// One in-flight request/response cycle on the websocket.
struct Cycle {
    session_id: Option<String>,
    event_id: Option<String>,
    request: CycleRequest,
    /// Server messages, re-shaped as an SSE stream (`data: {...}\n\n`) so the
    /// existing reconstruction pipeline parses them unchanged.
    capture: String,
    started: Instant,
    restorer: Option<lr_secret_scanner::wire::ResponseRestorer>,
    response_id: Option<String>,
    item_ids: std::collections::BTreeSet<String>,
    /// The firewall's rate-limit reservation for this cycle.
    admission: crate::interceptor::ExchangeAdmission,
}

/// How a client message that outgrew its lag continues.
enum Opened {
    /// Keep holding it; it is judged whole at its end.
    Hold,
    /// Forward it through this pipeline session; its cycle is open.
    Stream {
        session: Box<dyn StreamSession>,
        id: String,
    },
    /// Refused: drop the rest and answer with this error event.
    Refuse { reply: String },
}

/// Firewall + monitor capture for one intercepted LLM websocket connection.
pub struct WsSession {
    interceptor: Arc<dyn ProxyInterceptor>,
    format: WireFormat,
    /// Exchange template: client identity, host, path — cloned per cycle.
    base: ObservedExchange,
    cycle: parking_lot::Mutex<Option<Cycle>>,
    released: parking_lot::Mutex<Vec<String>>,
    previous: parking_lot::Mutex<Vec<Cycle>>,
    /// Reads `codex.rate_limits` events off the stream.
    usage: Option<Arc<lr_usage::UsageTracker>>,
    /// Client message hold cap (see [`MAX_MESSAGE`]).
    max_message: usize,
    /// Server message cap (see [`MAX_SERVER_MESSAGE`]).
    max_server_message: usize,
}

impl WsSession {
    pub fn new(
        interceptor: Arc<dyn ProxyInterceptor>,
        format: WireFormat,
        mut base: ObservedExchange,
    ) -> Self {
        base.connection_trace = true;
        Self {
            interceptor,
            format,
            base,
            cycle: parking_lot::Mutex::new(None),
            released: parking_lot::Mutex::new(Vec::new()),
            previous: parking_lot::Mutex::new(Vec::new()),
            usage: None,
            max_message: MAX_MESSAGE,
            max_server_message: MAX_SERVER_MESSAGE,
        }
    }

    /// Report usage-limit events seen on the stream to the usage tracker.
    pub fn with_usage(mut self, usage: Option<Arc<lr_usage::UsageTracker>>) -> Self {
        self.usage = usage;
        self
    }

    /// Override the message caps (tests).
    #[cfg(test)]
    fn with_limits(mut self, max_message: usize, max_server_message: usize) -> Self {
        self.max_message = max_message;
        self.max_server_message = max_server_message;
        self
    }

    /// The firewall's stream policy for one client message, as for an HTTP
    /// `POST` of the same request to the connection's path. `None` unless
    /// the message may stream once it outgrows the policy's lag; every other
    /// decision is left to [`on_request`](ProxyInterceptor::on_request) (or
    /// to `open_stream`, which repeats the content-free ones).
    fn message_policy(&self) -> Option<StreamPolicy> {
        let headers = http::HeaderMap::new();
        let head = RequestHead {
            client_id: &self.base.client_id,
            strategy_id: &self.base.strategy_id,
            host: &self.base.host,
            port: self.base.port,
            method: "POST",
            path: &self.base.path,
            headers: &headers,
            trace: self.base.trace.as_ref(),
            provider_override: self.base.provider_override.as_deref(),
            source: self.base.source,
        };
        match self.interceptor.body_policy(&head) {
            BodyPolicy::Stream(policy) => Some(policy),
            _ => None,
        }
    }

    /// A message outgrew its lag with its metadata known: ask the firewall
    /// whether it streams. `head` is what arrived so far (for the monitor of
    /// a refusal).
    async fn open_message_stream(
        &self,
        policy: &StreamPolicy,
        metadata: &lr_secret_scanner::request_stream::RequestMetadata,
        head: &[u8],
    ) -> Opened {
        let mut ex = self.base.clone();
        let id = uuid::Uuid::new_v4().to_string();
        ex.session_id = Some(id.clone());
        // Each message is admitted on its own, not on the connection's slot.
        ex.admission = Default::default();
        match self.interceptor.open_stream(&ex, metadata, policy).await {
            StreamOpen::Buffer => Opened::Hold,
            StreamOpen::Reject(action) => {
                let (status, body) = match action {
                    RequestAction::Reject { status, body, .. } => (status, body),
                    _ => {
                        let RequestAction::Reject { status, body, .. } = RequestAction::reject_json(
                            500,
                            "Request policy returned an invalid refusal",
                        ) else {
                            unreachable!("reject_json builds a Reject")
                        };
                        (status, body)
                    }
                };
                let reply = reject_event(status, &body);
                let cap = crate::passive::RAW_CAP.min(head.len());
                ex.request_capture = Some(BoundedCapture {
                    head: head[..cap].to_vec(),
                    total_len: head.len() as u64,
                    truncated: cap < head.len(),
                    complete: false,
                    encoded: false,
                });
                ex.status = Some(status);
                ex.response_body = Some(reply.clone().into_bytes());
                ex.latency_ms = Some(0);
                ex.refused_locally = true;
                self.interceptor.on_response(&ex).await;
                Opened::Refuse { reply }
            }
            // `tee` asks for the client's exact bytes; a websocket message is
            // re-framed anyway, and a tee session never changes the content, so
            // forwarding the pipeline's output is equivalent.
            StreamOpen::Stream {
                session,
                vault,
                tee: _,
            } => {
                ex.request_stream = Some(crate::request_stream::streamed_request(
                    self.format,
                    metadata,
                    None,
                ));
                ex.request_capture = Some(BoundedCapture::default());
                self.begin_cycle(
                    ex,
                    vault.map(lr_secret_scanner::wire::ResponseRestorer::new),
                )
                .await;
                Opened::Stream { session, id }
            }
        }
    }

    /// Run `update` on the streamed cycle `id`, wherever it is.
    fn with_streamed(
        &self,
        id: &str,
        update: impl FnOnce(&mut BoundedCapture, &mut StreamedRequest),
    ) {
        let mut current = self.cycle.lock();
        let mut previous = self.previous.lock();
        if let Some(cycle) = previous
            .iter_mut()
            .chain(current.iter_mut())
            .find(|cycle| cycle.session_id.as_deref() == Some(id))
        {
            if let CycleRequest::Streamed(streamed) = &mut cycle.request {
                let (capture, stream) = &mut **streamed;
                update(capture, stream);
            }
        }
    }

    /// Bytes of a streamed message were forwarded: keep their head for the
    /// monitor.
    fn record_forwarded(&self, id: &str, forwarded: &[u8]) {
        if forwarded.is_empty() {
            return;
        }
        self.with_streamed(id, |capture, _| {
            let room = crate::passive::RAW_CAP.saturating_sub(capture.head.len());
            capture
                .head
                .extend_from_slice(&forwarded[..room.min(forwarded.len())]);
            capture.total_len += forwarded.len() as u64;
            capture.truncated = capture.total_len > capture.head.len() as u64;
        });
    }

    /// A streamed message was forwarded completely.
    fn complete_streamed(&self, id: &str, summary: StreamSummary) {
        let format = self.format;
        self.with_streamed(id, |capture, stream| {
            capture.complete = true;
            *stream = crate::request_stream::streamed_request(
                format,
                &summary.metadata,
                Some(summary.correlation),
            );
        });
    }

    /// A streamed message was stopped: close its cycle with the refusal.
    async fn stop_streamed(&self, id: &str, status: u16, error: String, reply: &str) {
        let cycle = {
            let mut current = self.cycle.lock();
            let mut previous = self.previous.lock();
            if current.as_ref().and_then(|c| c.session_id.as_deref()) == Some(id) {
                current.take()
            } else {
                previous
                    .iter()
                    .position(|c| c.session_id.as_deref() == Some(id))
                    .map(|at| previous.remove(at))
            }
        };
        let Some(mut cycle) = cycle else {
            return;
        };
        if let Some(restorer) = &mut cycle.restorer {
            self.released.lock().extend(
                restorer.abandon_messages("Streamed request stopped; original content delivered"),
            );
        }
        cycle.capture.push_str("data: ");
        cycle.capture.push_str(reply);
        cycle.capture.push_str("\n\n");
        self.record_cycle(cycle, Some(status), Some(error)).await;
    }

    /// Run one client data message through the firewall and open a monitor
    /// cycle for it when it is allowed through.
    async fn on_client_message(&self, payload: &[u8]) -> ClientAction {
        // Control messages for a response already in flight: no model call
        // of their own, so no firewall decision and no new cycle. (A cycle
        // here would never see a terminal event, and it would make the
        // in-flight response's events ambiguous.) Only small messages are
        // parsed for this; a larger one is a request whatever its type.
        // An interrupt is forwarded only in Codex's content-free shape, so
        // the bypass cannot carry text past secret scanning.
        if payload.len() <= CONTROL_SNIFF {
            if let Ok(Value::Object(message)) = serde_json::from_slice::<Value>(payload) {
                let control = match message.get("type").and_then(Value::as_str) {
                    Some("response.cancel") => true,
                    Some("response.interrupt") => message.iter().all(|(key, value)| {
                        matches!(key.as_str(), "type" | "response_id" | "mode" | "event_id")
                            && !value.is_object()
                            && !value.is_array()
                    }),
                    _ => false,
                };
                if control {
                    return ClientAction::Forward;
                }
            }
        }
        let mut ex = self.base.clone();
        ex.session_id = Some(uuid::Uuid::new_v4().to_string());
        ex.request_body = Some(payload.to_vec());
        ex.admission = Default::default();
        match self.interceptor.on_request(&ex).await {
            RequestAction::Forward => {
                self.begin_cycle(ex, None).await;
                ClientAction::Forward
            }
            RequestAction::Replace(body) => {
                ex.request_body = Some(body.clone());
                self.begin_cycle(ex, None).await;
                ClientAction::Replace(body)
            }
            RequestAction::Protect { body, vault } => {
                ex.request_body = Some(body.clone());
                self.begin_cycle(
                    ex,
                    Some(lr_secret_scanner::wire::ResponseRestorer::new(vault)),
                )
                .await;
                ClientAction::Replace(body)
            }
            RequestAction::Reject { status, body, .. } => {
                let reply = reject_event(status, &body);
                // Record the blocked call in one push, like the HTTP path.
                let mut blocked = ex;
                blocked.status = Some(status);
                blocked.response_body = Some(reply.clone().into_bytes());
                blocked.latency_ms = Some(0);
                blocked.refused_locally = true;
                self.interceptor.on_response(&blocked).await;
                ClientAction::Reject(reply)
            }
        }
    }

    /// Each request retains independent restoration state. Response/item IDs
    /// route identifiable events; ambiguous concurrent events are passed through.
    async fn begin_cycle(
        &self,
        ex: ObservedExchange,
        restorer: Option<lr_secret_scanner::wire::ResponseRestorer>,
    ) {
        let event_id = self.interceptor.begin(&ex);
        let request = match ex.request_stream {
            Some(stream) => {
                CycleRequest::Streamed(Box::new((ex.request_capture.unwrap_or_default(), stream)))
            }
            None => CycleRequest::Body(ex.request_body.unwrap_or_default()),
        };
        let overflow = {
            let mut current = self.cycle.lock();
            let mut previous = self.previous.lock();
            if let Some(old) = current.take() {
                previous.push(old);
            }
            *current = Some(Cycle {
                session_id: ex.session_id,
                event_id,
                request,
                capture: String::new(),
                started: Instant::now(),
                restorer,
                response_id: None,
                item_ids: Default::default(),
                admission: ex.admission,
            });
            if previous.len() > 64 {
                Some(previous.remove(0))
            } else {
                None
            }
        };
        if let Some(mut old) = overflow {
            if let Some(restorer) = &mut old.restorer {
                self.released.lock().extend(restorer.abandon_messages(
                    "Concurrent restoration limit reached; original content delivered",
                ));
            }
            self.record_cycle(
                old,
                None,
                Some(
                    "Monitoring concurrency limit reached; request continues without restoration tracking"
                        .into(),
                ),
            )
            .await;
        }
    }
    fn release_originals(&self, reason: &str) -> Vec<String> {
        let mut current = self.cycle.lock();
        let mut previous = self.previous.lock();
        let mut released = std::mem::take(&mut *self.released.lock());
        for cycle in previous.iter_mut().chain(current.iter_mut()) {
            if let Some(restorer) = &mut cycle.restorer {
                released.extend(restorer.abandon_messages(reason));
            }
        }
        released
    }
    async fn on_server_message(&self, text: &str) -> lr_types::AppResult<Vec<String>> {
        let json = serde_json::from_str::<Value>(text).ok();
        if let (Some(usage), Some(value)) = (&self.usage, &json) {
            usage.observe_stream_event(
                self.base.usage_account.as_ref(),
                value,
                lr_usage::DataSource::ProxyHeaders,
            );
        }
        let terminal = json
            .as_ref()
            .is_some_and(|value| wire::is_terminal_event(self.format, value));
        let kind = json
            .as_ref()
            .and_then(|value| value["type"].as_str())
            .unwrap_or("");
        let response_id = json
            .as_ref()
            .and_then(|value| {
                value
                    .get("response_id")
                    .or_else(|| value.pointer("/response/id"))
            })
            .and_then(Value::as_str);
        let item_id = json
            .as_ref()
            .and_then(|value| value.get("item_id").or_else(|| value.pointer("/item/id")))
            .and_then(Value::as_str);
        let (output, finished) = {
            let mut current = self.cycle.lock();
            let mut previous = self.previous.lock();
            let selected = {
                let all: Vec<_> = previous.iter().chain(current.iter()).collect();
                let responses: Vec<_> = all
                    .iter()
                    .enumerate()
                    .filter(|(_, cycle)| {
                        response_id.is_some_and(|id| cycle.response_id.as_deref() == Some(id))
                    })
                    .map(|(i, _)| i)
                    .collect();
                let items: Vec<_> = all
                    .iter()
                    .enumerate()
                    .filter(|(_, cycle)| item_id.is_some_and(|id| cycle.item_ids.contains(id)))
                    .map(|(i, _)| i)
                    .collect();
                if responses.len() == 1 {
                    Some(responses[0])
                } else if responses.len() > 1 {
                    None
                } else if items.len() == 1 {
                    Some(items[0])
                } else if items.len() > 1 {
                    None
                } else if kind == "response.created" {
                    all.iter().position(|cycle| cycle.response_id.is_none())
                } else if all.len() == 1 {
                    Some(0)
                } else {
                    let known: Vec<_> = all
                        .iter()
                        .enumerate()
                        .filter(|(_, cycle)| cycle.response_id.is_some())
                        .map(|(i, _)| i)
                        .collect();
                    (known.len() == 1).then(|| known[0])
                }
            };
            let Some(index) = selected else {
                if kind.starts_with("codex.") || kind.starts_with("responsesapi.") {
                    return Ok(vec![text.into()]);
                }
                let mut output = Vec::new();
                for cycle in previous.iter_mut().chain(current.iter_mut()) {
                    if let Some(restorer) = &mut cycle.restorer {
                        output.extend(restorer.abandon_messages(
                            "Concurrent response identity is ambiguous; original content delivered",
                        ));
                    }
                }
                output.push(text.into());
                return Ok(output);
            };
            let cycle = if index < previous.len() {
                &mut previous[index]
            } else {
                current.as_mut().expect("selected current cycle")
            };
            if let Some(id) = response_id {
                cycle.response_id = Some(id.into());
            }
            if let Some(id) = item_id {
                cycle.item_ids.insert(id.into());
            }
            let output = if let Some(restorer) = &mut cycle.restorer {
                restorer.message(text).unwrap_or_else(|_| vec![text.into()])
            } else {
                vec![text.into()]
            };
            let needed = text.len() + 8;
            if cycle.capture.len() + needed > CAPTURE_CAP && terminal {
                cycle.capture.clear();
                cycle
                    .capture
                    .push_str("data: {\"type\":\"localrouter.truncated\"}\n\n");
            }
            if needed <= CAPTURE_CAP && cycle.capture.len() + needed <= CAPTURE_CAP {
                cycle.capture.push_str("data: ");
                cycle.capture.push_str(text);
                cycle.capture.push_str("\n\n");
            }
            let finished = if terminal {
                if index < previous.len() {
                    Some(previous.remove(index))
                } else {
                    current.take()
                }
            } else {
                None
            };
            (output, finished)
        };
        if let Some(cycle) = finished {
            let status = json
                .as_ref()
                .map(|value| wire::terminal_status(self.format, value));
            self.record_cycle(cycle, status, None).await;
        }
        Ok(output)
    }
    async fn record_cycle(&self, cycle: Cycle, status: Option<u16>, error: Option<String>) {
        let mut ex = self.base.clone();
        ex.session_id = cycle.session_id;
        ex.event_id = cycle.event_id;
        ex.admission = cycle.admission;
        match cycle.request {
            CycleRequest::Body(body) => ex.request_body = Some(body),
            CycleRequest::Streamed(streamed) => {
                let (capture, stream) = *streamed;
                ex.request_capture = Some(capture);
                ex.request_stream = Some(stream);
            }
        }
        ex.status = status;
        ex.error = error.or_else(|| {
            status
                .is_none()
                .then(|| "WebSocket disconnected before a terminal response event".into())
        });
        ex.response_body = (!cycle.capture.is_empty()).then(|| cycle.capture.into_bytes());
        ex.response_is_sse = true;
        ex.latency_ms = Some(cycle.started.elapsed().as_millis() as u64);
        self.interceptor.on_response(&ex).await;
    }
    async fn finish_cycle(&self, status: Option<u16>) {
        self.finish_cycle_with_error(status, None).await;
    }
    async fn finish_cycle_with_error(&self, status: Option<u16>, error: Option<String>) {
        let cycles = {
            let mut current = self.cycle.lock();
            let mut previous = self.previous.lock();
            let mut cycles = std::mem::take(&mut *previous);
            if let Some(cycle) = current.take() {
                cycles.push(cycle);
            }
            cycles
        };
        for cycle in cycles {
            self.record_cycle(cycle, status, error.clone()).await;
        }
    }

    /// The connection ended: close out any cycle still open.
    pub async fn on_close(&self) {
        self.finish_cycle(None).await;
    }

    /// Record a client message the relay refused before it could open a cycle
    /// (one above the message size cap), so the failure is visible.
    pub(crate) async fn record_refused(&self, status: u16, error: String) {
        let mut ex = self.base.clone();
        ex.session_id = Some(uuid::Uuid::new_v4().to_string());
        ex.admission = Default::default();
        ex.status = Some(status);
        ex.error = Some(error);
        ex.latency_ms = Some(0);
        ex.refused_locally = true;
        self.interceptor.on_response(&ex).await;
    }
}

/// Which side ended a relayed connection first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
enum EndedBy {
    Open = 0,
    Client = 1,
    Upstream = 2,
    /// The relay closed it (an unparseable or oversized message).
    Relay = 3,
}

impl EndedBy {
    fn from_u8(value: u8) -> Self {
        match value {
            1 => EndedBy::Client,
            2 => EndedBy::Upstream,
            3 => EndedBy::Relay,
            _ => EndedBy::Open,
        }
    }

    /// Error for a cycle still open when the connection ended.
    fn cycle_error(self, relay_reason: Option<String>) -> String {
        match self {
            EndedBy::Client => "Client closed the WebSocket before a terminal response event \
                 (the client abandoned the request)"
                .into(),
            EndedBy::Upstream => {
                "Upstream closed the WebSocket before a terminal response event".into()
            }
            EndedBy::Relay => relay_reason.unwrap_or_else(|| {
                "WebSocket disconnected before a terminal response event".into()
            }),
            EndedBy::Open => "WebSocket disconnected before a terminal response event".into(),
        }
    }
}

/// Record the first side to end the connection; later ends are consequences.
fn mark_ended(ended: &AtomicU8, by: EndedBy) {
    let _ = ended.compare_exchange(
        EndedBy::Open as u8,
        by as u8,
        Ordering::AcqRel,
        Ordering::Acquire,
    );
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

/// One direction's write half. Nothing is written after a close frame
/// (RFC 6455 §5.5.1), so a later write fails instead.
struct Peer<W> {
    wr: W,
    closed: bool,
}

impl<W: AsyncWrite + Unpin> Peer<W> {
    fn new(wr: W) -> Self {
        Self { wr, closed: false }
    }

    async fn send(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        if self.closed {
            return Err(std::io::ErrorKind::BrokenPipe.into());
        }
        self.wr.write_all(bytes).await
    }

    /// Send a close frame; nothing is written after it.
    async fn close(&mut self, frame: &[u8]) -> std::io::Result<()> {
        let result = self.send(frame).await;
        self.closed = true;
        result
    }

    async fn shutdown(&mut self) {
        let _ = self.wr.shutdown().await;
    }
}

type Shared<W> = Arc<tokio::sync::Mutex<Peer<W>>>;

/// The relay ends this direction (the connection is being closed).
struct Stop;

/// A pipeline and its firewall session, moved to a blocking thread per step.
struct StreamWork {
    pipeline: RequestPipeline,
    session: Box<dyn StreamSession>,
}

impl StreamWork {
    /// Process one batch of message bytes; at the message end, release the
    /// rest.
    fn step(
        &mut self,
        data: &[u8],
        eof: bool,
    ) -> Result<(Vec<u8>, Option<StreamSummary>), StreamAbort> {
        let mut out = Vec::new();
        self.pipeline
            .push(data, Some(self.session.as_mut()), &mut out)?;
        if !eof {
            return Ok((out, None));
        }
        let summary = self.pipeline.finish(self.session.as_mut(), &mut out)?;
        Ok((out, Some(summary)))
    }
}

/// A client message being forwarded while it arrives.
struct Streaming {
    opcode: u8,
    /// The cycle's session id.
    id: String,
    work: Option<StreamWork>,
    /// Message bytes not yet given to the pipeline.
    pending: Vec<u8>,
    /// A fragment of the message reached the upstream.
    sent_first: bool,
}

/// A client message held whole.
struct Holding {
    opcode: u8,
    raw: Vec<u8>,
    /// A pipeline watching the message, while it may still stream.
    candidate: Option<(RequestPipeline, StreamPolicy)>,
}

/// The client data message in progress.
enum Inbound {
    Idle,
    Holding(Box<Holding>),
    Streaming(Box<Streaming>),
    /// Refused while it was arriving: the rest is dropped, and the client
    /// is answered with this error event at its end.
    Discarding {
        reply: String,
    },
}

/// Shared state of one intercepted connection.
struct Relay<CW, UW> {
    session: Arc<WsSession>,
    client: Shared<CW>,
    upstream: Shared<UW>,
    ended: Arc<AtomicU8>,
    relay_reason: Arc<parking_lot::Mutex<Option<String>>>,
    /// Ends the upstream→client direction (the relay closed the connection).
    stop_upstream: Arc<tokio::sync::Notify>,
}

impl<CW, UW> Relay<CW, UW>
where
    CW: AsyncWrite + Unpin + Send,
    UW: AsyncWrite + Unpin + Send,
{
    async fn to_client(&self, message: WsMessage) -> Result<(), Stop> {
        self.client
            .lock()
            .await
            .send(&encode_message(&message, false))
            .await
            .map_err(|_| Stop)
    }

    /// Deliver originals released by restoration state that was abandoned.
    async fn flush_released(&self) -> Result<(), Stop> {
        let released = std::mem::take(&mut *self.session.released.lock());
        for original in released {
            self.to_client(WsMessage::Text(original)).await?;
        }
        Ok(())
    }

    /// The client stream cannot be relayed further: say why to both sides.
    async fn refuse(&self, poison: Poison) {
        let reason = poison.reason(self.session.max_message);
        tracing::warn!(
            "websocket client message refused ({reason}); closing intercepted connection"
        );
        mark_ended(&self.ended, EndedBy::Relay);
        let error = format!(
            "{reason}; connection closed with code {}",
            poison.close_code()
        );
        *self.relay_reason.lock() = Some(error.clone());
        let status = match poison {
            Poison::TooBig => 413,
            Poison::Protocol => 400,
        };
        self.session.record_refused(status, error).await;
        // Tell the client why instead of dropping the socket.
        let _ = self
            .client
            .lock()
            .await
            .close(&close_frame(poison.close_code(), false, &reason))
            .await;
        // 1001: the proxy is going away from this connection. A message
        // already partly forwarded is never completed.
        let _ = self
            .upstream
            .lock()
            .await
            .close(&close_frame(CLOSE_GOING_AWAY, true, ""))
            .await;
    }

    async fn on_client_event(&self, inbound: &mut Inbound, event: FrameEvent) -> Result<(), Stop> {
        match event {
            FrameEvent::DataStart { opcode } => {
                let candidate = self.session.message_policy().map(|policy| {
                    let mut pipeline = RequestPipeline::new(policy.lag, self.session.max_message);
                    if policy.strict {
                        // Strict mode: nothing leaves before the whole message
                        // was inspected. Without spill on this path the message
                        // stays within `max_message`; above it the relay refuses
                        // it (1009) and nothing is forwarded.
                        pipeline.hold_until_end();
                    }
                    (pipeline, policy)
                });
                *inbound = Inbound::Holding(Box::new(Holding {
                    opcode,
                    raw: Vec::new(),
                    candidate,
                }));
            }
            FrameEvent::DataChunk(bytes) => match inbound {
                Inbound::Holding(holding) => {
                    if holding.raw.len() + bytes.len() > self.session.max_message {
                        self.refuse(Poison::TooBig).await;
                        return Err(Stop);
                    }
                    holding.raw.extend_from_slice(&bytes);
                    if let Some((pipeline, _)) = &mut holding.candidate {
                        // Before it opens the pipeline only lexes and holds.
                        let mut nothing = Vec::new();
                        let _ = pipeline.push(&bytes, None, &mut nothing);
                        if pipeline.unstreamable().is_some() {
                            holding.candidate = None;
                        }
                    }
                    if holding
                        .candidate
                        .as_ref()
                        .is_some_and(|(pipeline, _)| pipeline.ready_to_open())
                    {
                        self.open(inbound).await?;
                    }
                }
                Inbound::Streaming(streaming) => {
                    streaming.pending.extend_from_slice(&bytes);
                    if streaming.pending.len() >= STREAM_STEP {
                        self.flush(inbound, false).await?;
                    }
                }
                Inbound::Discarding { .. } | Inbound::Idle => {}
            },
            FrameEvent::DataEnd => match std::mem::replace(inbound, Inbound::Idle) {
                Inbound::Holding(holding) => self.forward_held(*holding).await?,
                streaming @ Inbound::Streaming(_) => {
                    *inbound = streaming;
                    let flushed = self.flush(inbound, true).await;
                    *inbound = Inbound::Idle;
                    flushed?;
                }
                Inbound::Discarding { reply } => self.to_client(WsMessage::Text(reply)).await?,
                Inbound::Idle => {}
            },
            FrameEvent::Control(message) => {
                // Keep the upstream's order: data that arrived before the
                // control frame goes first.
                if matches!(inbound, Inbound::Streaming(s) if !s.pending.is_empty()) {
                    self.flush(inbound, false).await?;
                }
                let frame = encode_frame(&message, true);
                let mut upstream = self.upstream.lock().await;
                let sent = if matches!(message, WsMessage::Close(_)) {
                    mark_ended(&self.ended, EndedBy::Client);
                    upstream.close(&frame).await
                } else {
                    upstream.send(&frame).await
                };
                sent.map_err(|_| Stop)?;
            }
        }
        Ok(())
    }

    /// The held message outgrew its lag with its metadata known.
    async fn open(&self, inbound: &mut Inbound) -> Result<(), Stop> {
        let Inbound::Holding(holding) = inbound else {
            return Ok(());
        };
        let Some((mut pipeline, policy)) = holding.candidate.take() else {
            return Ok(());
        };
        let opcode = holding.opcode;
        let metadata = pipeline.metadata().clone();
        match self
            .session
            .open_message_stream(&policy, &metadata, &holding.raw)
            .await
        {
            Opened::Hold => {}
            Opened::Refuse { reply } => *inbound = Inbound::Discarding { reply },
            Opened::Stream { session, id } => {
                pipeline.open();
                *inbound = Inbound::Streaming(Box::new(Streaming {
                    opcode,
                    id,
                    work: Some(StreamWork { pipeline, session }),
                    pending: Vec::new(),
                    sent_first: false,
                }));
                self.flush_released().await?;
                // Release what can go already.
                self.flush(inbound, false).await?;
            }
        }
        Ok(())
    }

    /// Give the pending bytes of a streamed message to its pipeline and
    /// forward what it releases; `eof` at the message end.
    async fn flush(&self, inbound: &mut Inbound, eof: bool) -> Result<(), Stop> {
        let Inbound::Streaming(streaming) = inbound else {
            return Ok(());
        };
        let data = std::mem::take(&mut streaming.pending);
        let Some(mut work) = streaming.work.take() else {
            return Err(Stop);
        };
        let joined = tokio::task::spawn_blocking(move || {
            let result = work.step(&data, eof);
            (work, result)
        })
        .await;
        let result = match joined {
            Ok((work, result)) => {
                streaming.work = Some(work);
                result
            }
            Err(_) => Err(StreamAbort::new(500, "Request inspection failed")),
        };
        let (out, summary) = match result {
            Ok(stepped) => stepped,
            Err(abort) => return self.stop_stream(inbound, abort, eof).await,
        };
        if !out.is_empty() || eof {
            let mut wire = Vec::with_capacity(out.len() + out.len().div_ceil(MAX_FRAME) * 14 + 14);
            let opcode = if streaming.sent_first {
                0x0
            } else {
                streaming.opcode
            };
            push_fragments(&mut wire, opcode, &out, eof, true);
            streaming.sent_first = true;
            self.session.record_forwarded(&streaming.id, &out);
            self.upstream
                .lock()
                .await
                .send(&wire)
                .await
                .map_err(|_| Stop)?;
        }
        if let Some(summary) = summary {
            self.session.complete_streamed(&streaming.id, summary);
        }
        Ok(())
    }

    /// A streamed message cannot continue. Before any fragment went out it
    /// is refused like a held one (the rest is dropped, the connection stays
    /// open). After that the upstream holds a partial message: it is closed
    /// with 1008 so the message is never completed, the client gets the
    /// error event and a 1011 close, and every open cycle ends with an error.
    async fn stop_stream(
        &self,
        inbound: &mut Inbound,
        abort: StreamAbort,
        eof: bool,
    ) -> Result<(), Stop> {
        let Inbound::Streaming(streaming) = std::mem::replace(inbound, Inbound::Idle) else {
            return Ok(());
        };
        tracing::warn!("websocket: streamed request stopped: {}", abort.message);
        let body = serde_json::to_vec(&serde_json::json!({
            "type": "error",
            "error": { "type": "localrouter_firewall", "message": abort.message },
        }))
        .unwrap_or_default();
        let reply = reject_event(abort.status, &body);
        let error = format!("Streamed WebSocket request stopped: {}", abort.message);
        if !streaming.sent_first {
            self.session
                .stop_streamed(&streaming.id, abort.status, error, &reply)
                .await;
            self.flush_released().await?;
            if eof {
                return self.to_client(WsMessage::Text(reply)).await;
            }
            *inbound = Inbound::Discarding { reply };
            return Ok(());
        }
        {
            let mut upstream = self.upstream.lock().await;
            let _ = upstream
                .close(&close_frame(
                    CLOSE_POLICY,
                    true,
                    "request stopped by LocalRouter",
                ))
                .await;
            upstream.shutdown().await;
        }
        mark_ended(&self.ended, EndedBy::Relay);
        *self.relay_reason.lock() = Some(format!(
            "{error}; the upstream was closed with code {CLOSE_POLICY} and the client with code {CLOSE_INTERNAL}"
        ));
        self.session
            .stop_streamed(&streaming.id, abort.status, error, &reply)
            .await;
        let originals = self.session.release_originals(
            "WebSocket closed after a streamed request was stopped; original content delivered",
        );
        self.session
            .finish_cycle_with_error(
                None,
                Some(
                    "WebSocket closed: a streamed request on the same connection was stopped mid-message"
                        .into(),
                ),
            )
            .await;
        {
            let mut client = self.client.lock().await;
            for original in originals {
                let _ = client
                    .send(&encode_message(&WsMessage::Text(original), false))
                    .await;
            }
            let _ = client
                .send(&encode_message(&WsMessage::Text(reply), false))
                .await;
            let _ = client
                .close(&close_frame(CLOSE_INTERNAL, false, "request stopped"))
                .await;
        }
        self.stop_upstream.notify_one();
        Err(Stop)
    }

    /// A held message is complete: judge it and forward, rewrite or refuse it.
    async fn forward_held(&self, holding: Holding) -> Result<(), Stop> {
        let Holding {
            opcode,
            raw,
            candidate,
        } = holding;
        drop(candidate);
        let message = if opcode == 0x1 {
            match String::from_utf8(raw) {
                Ok(text) => WsMessage::Text(text),
                // Invalid UTF-8 in a text message: rejecting it keeps the
                // relay byte-exact, so a forwarded message is never altered.
                Err(_) => {
                    self.refuse(Poison::Protocol).await;
                    return Err(Stop);
                }
            }
        } else {
            WsMessage::Binary(raw)
        };
        // Note: while the firewall deliberates (e.g. an Ask popup), later
        // client frames simply wait — natural backpressure, same as the HTTP
        // path.
        let action = self.session.on_client_message(message.payload()).await;
        self.flush_released().await?;
        let wire = match action {
            ClientAction::Forward => encode_message(&message, true),
            ClientAction::Replace(body) => {
                let replaced = match message {
                    WsMessage::Text(_) => {
                        WsMessage::Text(String::from_utf8_lossy(&body).into_owned())
                    }
                    _ => WsMessage::Binary(body),
                };
                encode_message(&replaced, true)
            }
            // Answer the client directly; the upstream never sees the
            // request and the connection stays open.
            ClientAction::Reject(reply) => return self.to_client(WsMessage::Text(reply)).await,
        };
        self.upstream
            .lock()
            .await
            .send(&wire)
            .await
            .map_err(|_| Stop)
    }

    /// Client → upstream for an intercepted LLM connection.
    async fn pump_client<R: AsyncRead + Unpin>(&self, mut rd: R, stop: &tokio::sync::Notify) {
        let mut reader = FrameReader::new(self.session.max_message);
        let mut buf = vec![0u8; 64 * 1024];
        let mut inbound = Inbound::Idle;
        loop {
            let read = tokio::select! {
                read = rd.read(&mut buf) => read,
                _ = stop.notified() => break,
            };
            let n = match read {
                Ok(0) | Err(_) => {
                    mark_ended(&self.ended, EndedBy::Client);
                    if matches!(&inbound, Inbound::Streaming(s) if s.sent_first) {
                        // The partial message must never be completed.
                        let _ = self
                            .upstream
                            .lock()
                            .await
                            .close(&close_frame(CLOSE_GOING_AWAY, true, ""))
                            .await;
                    }
                    break;
                }
                Ok(n) => n,
            };
            let mut events = Vec::new();
            reader.push(&buf[..n], &mut events);
            for event in events {
                if self.on_client_event(&mut inbound, event).await.is_err() {
                    return;
                }
            }
            if matches!(&inbound, Inbound::Streaming(s) if !s.pending.is_empty())
                && self.flush(&mut inbound, false).await.is_err()
            {
                return;
            }
            if let Some(poison) = reader.poison() {
                self.refuse(poison).await;
                break;
            }
        }
    }

    /// Deliver one server message to the client.
    async fn on_server_frame(&self, msg: WsMessage) -> Result<(), Stop> {
        let session = &self.session;
        let messages = match &msg {
            WsMessage::Text(text) => session.on_server_message(text).await,
            WsMessage::Binary(bytes) => match std::str::from_utf8(bytes) {
                Ok(text) => session.on_server_message(text).await,
                Err(_) => Err(lr_types::AppError::InvalidParams(
                    "Invalid protected WebSocket JSON".into(),
                )),
            },
            WsMessage::Close(_) => {
                mark_ended(&self.ended, EndedBy::Upstream);
                // Held originals go out before the close: nothing may follow it.
                let originals = session.release_originals(
                    "WebSocket closing; incomplete buffered response delivered unchanged",
                );
                let mut client = self.client.lock().await;
                for original in originals {
                    client
                        .send(&encode_message(&WsMessage::Text(original), false))
                        .await
                        .map_err(|_| Stop)?;
                }
                return client
                    .close(&encode_frame(&msg, false))
                    .await
                    .map_err(|_| Stop);
            }
            WsMessage::Ping(_) | WsMessage::Pong(_) => {
                return self
                    .client
                    .lock()
                    .await
                    .send(&encode_frame(&msg, false))
                    .await
                    .map_err(|_| Stop);
            }
        };
        let messages = match messages {
            Ok(messages) => messages,
            Err(_) => {
                let mut original = session.release_originals(
                    "WebSocket restoration unavailable; original content delivered",
                );
                if let WsMessage::Binary(_) = &msg {
                    let mut client = self.client.lock().await;
                    for text in original {
                        let _ = client
                            .send(&encode_message(&WsMessage::Text(text), false))
                            .await;
                    }
                    return client
                        .send(&encode_message(&msg, false))
                        .await
                        .map_err(|_| Stop);
                }
                if let WsMessage::Text(text) = &msg {
                    original.push(text.clone());
                }
                original
            }
        };
        let mut client = self.client.lock().await;
        for text in messages {
            let output = match msg {
                WsMessage::Binary(_) => WsMessage::Binary(text.into_bytes()),
                _ => WsMessage::Text(text),
            };
            client
                .send(&encode_message(&output, false))
                .await
                .map_err(|_| Stop)?;
        }
        Ok(())
    }

    /// Upstream → client for an intercepted LLM connection.
    async fn pump_upstream<R: AsyncRead + Unpin>(&self, mut rd: R) {
        let limit = self.session.max_server_message;
        let mut decoder = FrameDecoder::with_limit(limit);
        let mut buf = vec![0u8; 64 * 1024];
        'server: loop {
            let read = tokio::select! {
                read = rd.read(&mut buf) => read,
                _ = self.stop_upstream.notified() => break,
            };
            let n = match read {
                Ok(0) | Err(_) => {
                    mark_ended(&self.ended, EndedBy::Upstream);
                    break;
                }
                Ok(n) => n,
            };
            for msg in decoder.push(&buf[..n]) {
                if self.on_server_frame(msg).await.is_err() {
                    break 'server;
                }
            }
            if let Some(poison) = decoder.poison() {
                let reason = poison.reason(limit);
                tracing::warn!(
                    "websocket upstream message refused ({reason}); closing intercepted connection"
                );
                mark_ended(&self.ended, EndedBy::Relay);
                *self.relay_reason.lock() = Some(format!(
                    "Upstream sent an unrelayable message ({reason}); connection closed with code {}",
                    poison.close_code()
                ));
                let originals = self.session.release_originals(
                    "WebSocket closed; incomplete buffered response delivered unchanged",
                );
                {
                    let mut client = self.client.lock().await;
                    for original in originals {
                        let _ = client
                            .send(&encode_message(&WsMessage::Text(original), false))
                            .await;
                    }
                    let _ = client
                        .close(&close_frame(poison.close_code(), false, &reason))
                        .await;
                }
                let _ = self
                    .upstream
                    .lock()
                    .await
                    .close(&close_frame(poison.close_code(), true, &reason))
                    .await;
                break;
            }
        }
        let originals = self
            .session
            .release_originals("WebSocket ended; incomplete buffered response delivered unchanged");
        let mut client = self.client.lock().await;
        for original in originals {
            if client
                .send(&encode_message(&WsMessage::Text(original), false))
                .await
                .is_err()
            {
                break;
            }
        }
    }
}

/// Ferry an upgraded websocket connection between client and upstream.
///
/// With a [`WsSession`] (a recognized LLM path), client data messages are
/// decoded, firewalled, and re-framed (held whole, or streamed through the
/// request pipeline when the firewall allows it); protected server messages
/// are restored before delivery and captured in their masked form. Without
/// one (any other websocket on a MITM'd host), both directions are a blind
/// byte copy.
///
/// If a decoder poisons (protocol violation / oversized message), the relay
/// closes the client with a close code saying why (1009 for a message above
/// the limit, 1002 otherwise) and shuts the connection down: we never ferry
/// LLM traffic the firewall could not inspect. Codex answers an early 1009
/// by retrying the request over HTTPS.
pub async fn relay<C, U>(client: C, upstream: U, session: Option<Arc<WsSession>>)
where
    C: AsyncRead + AsyncWrite + Send,
    U: AsyncRead + AsyncWrite + Send,
{
    let (client_rd, client_wr) = tokio::io::split(client);
    let (upstream_rd, upstream_wr) = tokio::io::split(upstream);
    let Some(session) = session else {
        blind(client_rd, client_wr, upstream_rd, upstream_wr).await;
        return;
    };
    let relay = Relay {
        session: session.clone(),
        client: Arc::new(tokio::sync::Mutex::new(Peer::new(client_wr))),
        upstream: Arc::new(tokio::sync::Mutex::new(Peer::new(upstream_wr))),
        ended: Arc::new(AtomicU8::new(EndedBy::Open as u8)),
        relay_reason: Arc::new(parking_lot::Mutex::new(None::<String>)),
        stop_upstream: Arc::new(tokio::sync::Notify::new()),
    };
    let stop_client = tokio::sync::Notify::new();
    let c2u = async {
        relay.pump_client(client_rd, &stop_client).await;
        relay.upstream.lock().await.shutdown().await;
    };
    let u2c = async {
        relay.pump_upstream(upstream_rd).await;
        stop_client.notify_one();
        relay.client.lock().await.shutdown().await;
    };
    tokio::join!(c2u, u2c);
    let ended_by = EndedBy::from_u8(relay.ended.load(Ordering::Acquire));
    let error = ended_by.cycle_error(relay.relay_reason.lock().take());
    session.finish_cycle_with_error(None, Some(error)).await;
}

/// Both directions of a websocket nobody inspects, copied byte for byte.
async fn blind<CR, CW, UR, UW>(
    mut client_rd: CR,
    client_wr: CW,
    mut upstream_rd: UR,
    mut upstream_wr: UW,
) where
    CR: AsyncRead + Unpin,
    CW: AsyncWrite + Unpin,
    UR: AsyncRead + Unpin,
    UW: AsyncWrite + Unpin,
{
    let client_wr = tokio::sync::Mutex::new(client_wr);
    let stop_client = tokio::sync::Notify::new();
    let c2u = async {
        let mut buf = vec![0u8; 16 * 1024];
        loop {
            let read = tokio::select! {
                read = client_rd.read(&mut buf) => read,
                _ = stop_client.notified() => break,
            };
            match read {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if upstream_wr.write_all(&buf[..n]).await.is_err() {
                        break;
                    }
                }
            }
        }
        let _ = upstream_wr.shutdown().await;
    };
    let u2c = async {
        let mut buf = vec![0u8; 16 * 1024];
        loop {
            match upstream_rd.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if client_wr.lock().await.write_all(&buf[..n]).await.is_err() {
                        break;
                    }
                }
            }
        }
        stop_client.notify_one();
        let _ = client_wr.lock().await.shutdown().await;
    };
    tokio::join!(c2u, u2c);
}

#[cfg(test)]
#[path = "websocket_stream_tests.rs"]
mod stream_tests;

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

    /// `codex.rate_limits` events are filed under the login that opened the
    /// connection, so a second ChatGPT account gets its own usage account.
    #[tokio::test]
    async fn rate_limit_events_go_to_the_connections_login() {
        let store = Arc::new(MonitorEventStore::new(16));
        let interceptor = Arc::new(PassiveInterceptor::new(store));
        let usage = Arc::new(lr_usage::UsageTracker::new(Default::default()));
        // Another login already holds the family's base account.
        usage.record_request(
            &lr_usage::AccountRef::chatgpt(Some("acc-1")),
            lr_usage::LedgerEntry::default(),
        );
        let login = lr_usage::AccountRef::chatgpt(Some("acc-2"));
        let mut base = base_exchange();
        base.usage_account = Some(login.clone());
        let session = WsSession::new(interceptor, WireFormat::OpenAiResponses, base)
            .with_usage(Some(usage.clone()));
        session
            .on_server_message(
                &json!({"type":"codex.rate_limits","plan_type":"prolite","rate_limits":{
                    "primary":{"used_percent":13,"window_minutes":10080,"reset_at":4102444800i64},
                    "secondary":null}})
                .to_string(),
            )
            .await
            .unwrap();
        let snap = usage.snapshot();
        let account = snap
            .accounts
            .iter()
            .find(|a| a.id == login.id())
            .expect("the second login's account");
        assert_eq!(account.windows.len(), 1);
        assert_eq!(account.windows[0].id, "seven_day");
        assert_eq!(account.windows[0].used_percent, 13.0);
        let first = snap
            .accounts
            .iter()
            .find(|a| a.id == "openai:subscription")
            .unwrap();
        assert!(first.windows.is_empty());
    }

    #[tokio::test]
    async fn each_websocket_cycle_has_a_distinct_request_id() {
        let store = Arc::new(MonitorEventStore::new(16));
        let mut base = base_exchange();
        base.trace = Some(lr_types::RequestTrace::parse("connection;hop=1").unwrap());
        let session = WsSession::new(
            Arc::new(PassiveInterceptor::new(store.clone())),
            WireFormat::OpenAiResponses,
            base,
        );
        for _ in 0..2 {
            session
                .on_client_message(response_create().as_bytes())
                .await;
            let pending = store.list(0, 1, None).events.remove(0);
            assert!(pending.session_id.is_some());
            session.on_server_message(&completed_event()).await.unwrap();
            let completed = store.list(0, 1, None).events.remove(0);
            assert_eq!(completed.session_id, pending.session_id);
        }
        let events = store.list(0, 10, None).events;
        assert_eq!(events.len(), 2);
        assert_ne!(events[0].session_id, events[1].session_id);
        assert!(events.iter().all(|e| e.trace_id.is_none()));
        assert_ne!(
            events[0].conversation.as_ref().unwrap().id,
            events[1].conversation.as_ref().unwrap().id
        );
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
            session.on_server_message(&delta).await.unwrap();
        }
        // ...then the terminal event, which must still be captured.
        session.on_server_message(&completed_event()).await.unwrap();

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
    async fn oversized_terminal_completes_the_cycle_without_waiting_for_disconnect() {
        let store = Arc::new(MonitorEventStore::new(16));
        let session = WsSession::new(
            Arc::new(PassiveInterceptor::new(store.clone())),
            WireFormat::OpenAiResponses,
            base_exchange(),
        );
        session
            .on_client_message(response_create().as_bytes())
            .await;
        let mut event: Value = serde_json::from_str(&completed_event()).unwrap();
        event["response"]["output"][0]["content"][0]["text"] =
            Value::String("x".repeat(CAPTURE_CAP + 1));
        let event = event.to_string();
        assert_eq!(
            session.on_server_message(&event).await.unwrap(),
            vec![event]
        );
        let events = store.list(0, 10, None);
        assert_eq!(events.events.len(), 1);
        assert_eq!(events.events[0].status, EventStatus::Complete);
        assert!(session.cycle.lock().is_none());
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
                .await
                .unwrap();
            session.on_server_message(&completed_event()).await.unwrap();
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
            .await
            .unwrap();
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

#[cfg(test)]
mod protected_cancellation_tests {
    use super::*;
    struct Protect;
    #[async_trait::async_trait]
    impl ProxyInterceptor for Protect {
        fn on_connect(
            &self,
            _: &str,
            _: &crate::interceptor::ClientCtx,
        ) -> crate::interceptor::ConnectDecision {
            crate::interceptor::ConnectDecision::Mitm
        }
        async fn on_request(&self, ex: &ObservedExchange) -> RequestAction {
            RequestAction::Protect {
                body: ex.request_body.clone().unwrap(),
                vault: Arc::new(lr_secret_scanner::placeholders::Vault::new(
                    Arc::new(lr_api_keys::MockKeychain::new()),
                    "cancel",
                    "scope",
                    true,
                )),
            }
        }
    }
    #[tokio::test]
    async fn unsupported_provider_event_passes_through_without_failing_the_call() {
        struct RecordedProtect(crate::passive::PassiveInterceptor);
        #[async_trait::async_trait]
        impl ProxyInterceptor for RecordedProtect {
            fn on_connect(
                &self,
                _: &str,
                _: &crate::interceptor::ClientCtx,
            ) -> crate::interceptor::ConnectDecision {
                crate::interceptor::ConnectDecision::Mitm
            }
            async fn on_request(&self, ex: &ObservedExchange) -> RequestAction {
                Protect.on_request(ex).await
            }
            fn begin(&self, ex: &ObservedExchange) -> Option<String> {
                self.0.begin(ex)
            }
            async fn on_response(&self, ex: &ObservedExchange) {
                self.0.on_response(ex).await
            }
        }
        let store = Arc::new(lr_monitor::MonitorEventStore::new(10));
        let session = Arc::new(WsSession::new(
            Arc::new(RecordedProtect(crate::passive::PassiveInterceptor::new(
                store.clone(),
            ))),
            WireFormat::OpenAiResponses,
            ObservedExchange {
                client_id: "cancel".into(),
                host: "chatgpt.com".into(),
                path: "/backend-api/codex/responses".into(),
                ..Default::default()
            },
        ));
        let (mut client, proxy_client) = tokio::io::duplex(8192);
        let (mut upstream, proxy_upstream) = tokio::io::duplex(8192);
        let task = tokio::spawn(relay(proxy_client, proxy_upstream, Some(session)));
        client
            .write_all(&encode_frame(
                &WsMessage::Text(r#"{"type":"response.create","input":"hello"}"#.into()),
                true,
            ))
            .await
            .unwrap();
        let mut buffer = [0; 8192];
        assert!(upstream.read(&mut buffer).await.unwrap() > 0);
        let malformed = "not valid protected JSON";
        upstream
            .write_all(&encode_frame(&WsMessage::Text(malformed.into()), false))
            .await
            .unwrap();
        let mut decoder = FrameDecoder::new();
        let reply = tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                let read = client.read(&mut buffer).await.unwrap();
                assert!(read > 0, "restoration failed without a client error");
                if let Some(WsMessage::Text(reply)) = decoder.push(&buffer[..read]).pop() {
                    break reply;
                }
            }
        })
        .await
        .unwrap();
        assert_eq!(reply, malformed);
        let binary = vec![0xff, 0, 0xfe];
        upstream
            .write_all(&encode_frame(&WsMessage::Binary(binary.clone()), false))
            .await
            .unwrap();
        let received = tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                let count = client.read(&mut buffer).await.unwrap();
                assert!(
                    count > 0,
                    "unsupported binary content must not close the call"
                );
                if let Some(message) = decoder.push(&buffer[..count]).pop() {
                    break message;
                }
            }
        })
        .await
        .unwrap();
        assert_eq!(received, WsMessage::Binary(binary));
        upstream
            .write_all(&encode_frame(
                &WsMessage::Text(
                    r#"{"type":"response.completed","response":{"id":"done","output":[]}}"#.into(),
                ),
                false,
            ))
            .await
            .unwrap();
        let _ = tokio::time::timeout(std::time::Duration::from_secs(2), client.read(&mut buffer))
            .await
            .unwrap()
            .unwrap();
        drop(client);
        drop(upstream);
        task.await.unwrap();
        let summary = store.list(0, 10, None).events.pop().unwrap();
        assert_eq!(summary.status, lr_monitor::EventStatus::Complete);
        let event = serde_json::to_value(store.get(&summary.id).unwrap()).unwrap();
        assert!(event["data"]["error"].is_null());
        assert!(event["data"]["raw_response"]
            .as_str()
            .unwrap()
            .contains(malformed));
    }

    #[tokio::test]
    async fn cancellation_retains_capability_and_forwards_provider_specific_fields() {
        let session = WsSession::new(
            Arc::new(Protect),
            WireFormat::OpenAiResponses,
            ObservedExchange::default(),
        );
        assert!(matches!(
            session
                .on_client_message(br#"{"type":"response.create","input":"hello"}"#)
                .await,
            ClientAction::Replace(_)
        ));
        assert!(matches!(
            session
                .on_client_message(br#"{"type":"response.cancel"}"#)
                .await,
            ClientAction::Forward
        ));
        assert!(session.cycle.lock().as_ref().unwrap().restorer.is_some());
        assert!(matches!(
            session
                .on_client_message(br#"{"type":"response.cancel","input":"provider-specific"}"#)
                .await,
            ClientAction::Forward
        ));
        session
            .on_server_message(r#"{"type":"response.incomplete","response":{"id":"resp_cancel"}}"#)
            .await
            .unwrap();
        assert!(session.cycle.lock().is_none());
        assert!(matches!(
            session
                .on_client_message(br#"{"type":"response.create","input":"next"}"#)
                .await,
            ClientAction::Replace(_)
        ));
        session.on_close().await;
    }
}

#[cfg(test)]
mod concurrent_restoration_tests {
    use super::*;
    use lr_types::secret_context::SecretContext;
    struct Protected;
    #[async_trait::async_trait]
    impl ProxyInterceptor for Protected {
        fn on_connect(
            &self,
            _: &str,
            _: &crate::interceptor::ClientCtx,
        ) -> crate::interceptor::ConnectDecision {
            crate::interceptor::ConnectDecision::Mitm
        }
        async fn on_request(&self, ex: &ObservedExchange) -> RequestAction {
            let mut body: Value =
                serde_json::from_slice(ex.request_body.as_ref().unwrap()).unwrap();
            let vault = Arc::new(lr_secret_scanner::placeholders::Vault::for_client(
                Arc::new(lr_api_keys::MockKeychain::new()),
                "concurrent-client",
            ));
            vault.register(body["input"].as_str().unwrap()).unwrap();
            vault.protect(&mut body).unwrap();
            RequestAction::Protect {
                body: body.to_string().into_bytes(),
                vault,
            }
        }
    }
    async fn start(session: &WsSession, value: &str, id: &str) -> String {
        let payload =
            serde_json::json!({"type":"response.create","model":"test","input":value}).to_string();
        let ClientAction::Replace(body) = session.on_client_message(payload.as_bytes()).await
        else {
            panic!("concurrent call was rejected")
        };
        let tag = serde_json::from_slice::<Value>(&body).unwrap()["input"]
            .as_str()
            .unwrap()
            .to_owned();
        session
            .on_server_message(
                &serde_json::json!({"type":"response.created","response":{"id":id}}).to_string(),
            )
            .await
            .unwrap();
        tag
    }
    fn deltas(output: Vec<String>) -> String {
        output
            .iter()
            .filter_map(|s| {
                serde_json::from_str::<Value>(s).unwrap()["delta"]
                    .as_str()
                    .map(str::to_owned)
            })
            .collect()
    }
    /// Held restoration content goes to the client before the upstream's
    /// close frame: nothing may follow a close (it used to be written after).
    #[tokio::test]
    async fn held_originals_precede_the_forwarded_close() {
        let session = Arc::new(WsSession::new(
            Arc::new(Protected),
            WireFormat::OpenAiResponses,
            ObservedExchange::default(),
        ));
        let (client, proxy_client) = tokio::io::duplex(64 * 1024);
        let (upstream, proxy_upstream) = tokio::io::duplex(64 * 1024);
        let task = tokio::spawn(relay(proxy_client, proxy_upstream, Some(session)));
        let (mut client_rd, mut client_wr) = tokio::io::split(client);
        let (mut upstream_rd, mut upstream_wr) = tokio::io::split(upstream);
        let request = serde_json::json!({"type":"response.create","model":"test",
            "input":"first-synthetic-original"})
        .to_string();
        client_wr
            .write_all(&encode_frame(&WsMessage::Text(request), true))
            .await
            .unwrap();
        let mut buf = vec![0u8; 64 * 1024];
        let mut upstream_decoder = FrameDecoder::new();
        loop {
            let n = upstream_rd.read(&mut buf).await.unwrap();
            assert!(n > 0);
            if !upstream_decoder.push(&buf[..n]).is_empty() {
                break;
            }
        }
        let held = serde_json::json!({"type":"response.function_call_arguments.delta",
            "response_id":"response-a","item_id":"a",
            "delta":"{\"input\":\"<localrouter-redacted"})
        .to_string();
        let created = serde_json::json!({"type":"response.created","response":{"id":"response-a"}})
            .to_string();
        let mut wire = encode_frame(&WsMessage::Text(created), false);
        wire.extend(encode_frame(&WsMessage::Text(held.clone()), false));
        wire.extend(close_frame(1000, false, ""));
        upstream_wr.write_all(&wire).await.unwrap();
        let mut received = Vec::new();
        let mut decoder = FrameDecoder::new();
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                let n = client_rd.read(&mut buf).await.unwrap();
                if n == 0 {
                    break;
                }
                received.extend(decoder.push(&buf[..n]));
                if matches!(received.last(), Some(WsMessage::Close(_))) {
                    break;
                }
            }
        })
        .await
        .unwrap();
        // The upstream hangs up: the relay ends and must write nothing more.
        drop(upstream_wr);
        drop(upstream_rd);
        let mut rest = Vec::new();
        tokio::time::timeout(
            std::time::Duration::from_secs(10),
            client_rd.read_to_end(&mut rest),
        )
        .await
        .unwrap()
        .unwrap();
        received.extend(decoder.push(&rest));
        assert_eq!(received.len(), 3, "{received:?}");
        assert_eq!(received[1], WsMessage::Text(held));
        assert!(matches!(received[2], WsMessage::Close(_)));
        drop(client_wr);
        task.await.unwrap();
    }
    #[tokio::test]
    async fn identifiable_concurrent_responses_restore_independently_and_never_return_409() {
        let session = WsSession::new(
            Arc::new(Protected),
            WireFormat::OpenAiResponses,
            ObservedExchange::default(),
        );
        let first = start(&session, "first-synthetic-original", "response-a").await;
        let second = start(&session, "second-synthetic-original", "response-b").await;
        let split = first.len() / 2;
        assert!(session.on_server_message(&serde_json::json!({"type":"response.output_text.delta","response_id":"response-a","item_id":"a","delta":&first[..split]}).to_string()).await.unwrap().is_empty());
        let output=session.on_server_message(&serde_json::json!({"type":"response.output_text.delta","response_id":"response-b","item_id":"b","delta":second}).to_string()).await.unwrap();
        assert_eq!(deltas(output), "second-synthetic-original");
        let output=session.on_server_message(&serde_json::json!({"type":"response.output_text.delta","item_id":"a","delta":&first[split..]}).to_string()).await.unwrap();
        assert_eq!(deltas(output), "first-synthetic-original");
        session
            .on_server_message(r#"{"type":"response.completed","response":{"id":"response-b"}}"#)
            .await
            .unwrap();
        assert!(session.cycle.lock().is_none());
        assert_eq!(session.previous.lock().len(), 1);
        session
            .on_server_message(r#"{"type":"response.completed","response":{"id":"response-a"}}"#)
            .await
            .unwrap();
        assert!(session.previous.lock().is_empty());
        let third = start(&session, "third-synthetic-original", "response-c").await;
        let output=session.on_server_message(&serde_json::json!({"type":"response.output_text.delta","item_id":"c","delta":third}).to_string()).await.unwrap();
        assert_eq!(deltas(output), "third-synthetic-original");
    }
    #[tokio::test]
    async fn ambiguous_concurrent_events_release_held_content_without_failing_or_disabling_future_turns(
    ) {
        let session = WsSession::new(
            Arc::new(Protected),
            WireFormat::OpenAiResponses,
            ObservedExchange::default(),
        );
        let _ = start(&session, "first-synthetic-original", "response-a").await;
        let _ = start(&session, "second-synthetic-original", "response-b").await;
        let held=serde_json::json!({"type":"response.function_call_arguments.delta","response_id":"response-a","item_id":"a","delta":"{\"input\":\"<localrouter-redacted"}).to_string();
        assert!(session.on_server_message(&held).await.unwrap().is_empty());
        let ambiguous=serde_json::json!({"type":"response.output_text.delta","item_id":"unassigned","delta":"unchanged"}).to_string();
        let output = session.on_server_message(&ambiguous).await.unwrap();
        assert_eq!(output, vec![held, ambiguous]);
        for id in ["response-a", "response-b"] {
            session
                .on_server_message(
                    &serde_json::json!({"type":"response.completed","response":{"id":id}})
                        .to_string(),
                )
                .await
                .unwrap();
        }
        let third = start(&session, "third-synthetic-original", "response-c").await;
        let output=session.on_server_message(&serde_json::json!({"type":"response.output_text.delta","item_id":"c","delta":third}).to_string()).await.unwrap();
        assert_eq!(deltas(output), "third-synthetic-original");
    }
}

#[cfg(test)]
mod codex_disconnect_tests {
    //! Regressions for Codex sessions failing behind the proxy: a large
    //! conversation resent on a fresh websocket, steering interrupts, and the
    //! monitor's account of who ended a connection.
    use super::*;
    use crate::passive::PassiveInterceptor;
    use lr_monitor::{EventStatus, MonitorEventStore};
    use serde_json::json;
    use std::sync::atomic::AtomicUsize;

    fn base_exchange() -> ObservedExchange {
        ObservedExchange {
            client_id: "client-1".into(),
            host: "chatgpt.com".into(),
            method: "GET".into(),
            path: "/backend-api/codex/responses".into(),
            ..Default::default()
        }
    }

    /// Records to the monitor and counts firewall decisions.
    struct Counting {
        recorder: PassiveInterceptor,
        requests: AtomicUsize,
    }

    #[async_trait::async_trait]
    impl ProxyInterceptor for Counting {
        fn on_connect(
            &self,
            _: &str,
            _: &crate::interceptor::ClientCtx,
        ) -> crate::interceptor::ConnectDecision {
            crate::interceptor::ConnectDecision::Mitm
        }
        async fn on_request(&self, _: &ObservedExchange) -> RequestAction {
            self.requests.fetch_add(1, Ordering::SeqCst);
            RequestAction::Forward
        }
        fn begin(&self, ex: &ObservedExchange) -> Option<String> {
            self.recorder.begin(ex)
        }
        async fn on_response(&self, ex: &ObservedExchange) {
            self.recorder.on_response(ex).await
        }
    }

    fn counting(store: &Arc<MonitorEventStore>) -> Arc<Counting> {
        Arc::new(Counting {
            recorder: PassiveInterceptor::new(store.clone()),
            requests: AtomicUsize::new(0),
        })
    }

    /// (FIN, opcode, payload length) of each frame in an encoded stream.
    fn frame_lengths(mut wire: &[u8]) -> Vec<(bool, u8, usize)> {
        let mut frames = Vec::new();
        while wire.len() >= 2 {
            let fin = wire[0] & 0x80 != 0;
            let opcode = wire[0] & 0x0F;
            let masked = wire[1] & 0x80 != 0;
            let (len, header) = match wire[1] & 0x7F {
                126 => (u16::from_be_bytes([wire[2], wire[3]]) as usize, 4),
                127 => {
                    let mut be = [0u8; 8];
                    be.copy_from_slice(&wire[2..10]);
                    (u64::from_be_bytes(be) as usize, 10)
                }
                n => (n as usize, 2),
            };
            let header = header + if masked { 4 } else { 0 };
            frames.push((fin, opcode, len));
            wire = &wire[header + len..];
        }
        frames
    }

    fn event_json(store: &MonitorEventStore, id: &str) -> Value {
        serde_json::to_value(store.get(id).expect("event exists")).unwrap()
    }

    /// Read until one whole message arrives; `raw` collects the bytes read.
    async fn read_message<R: AsyncRead + Unpin>(rd: &mut R, raw: &mut Vec<u8>) -> WsMessage {
        let mut decoder = FrameDecoder::new();
        let mut buf = vec![0u8; 64 * 1024];
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                let n = rd.read(&mut buf).await.unwrap();
                assert!(n > 0, "connection closed before a whole message arrived");
                raw.extend_from_slice(&buf[..n]);
                if let Some(message) = decoder.push(&buf[..n]).into_iter().next() {
                    return message;
                }
            }
        })
        .await
        .expect("message arrives")
    }

    #[test]
    fn large_messages_are_encoded_as_bounded_fragments() {
        // A two-byte UTF-8 character straddles the first frame boundary.
        let text = format!(
            "{}é{}",
            "a".repeat(MAX_FRAME - 1),
            "b".repeat(MAX_FRAME + 7)
        );
        let message = WsMessage::Text(text);
        for mask in [true, false] {
            let wire = encode_message(&message, mask);
            assert_eq!(FrameDecoder::new().push(&wire), vec![message.clone()]);
            assert_eq!(
                frame_lengths(&wire),
                vec![
                    (false, 0x1, MAX_FRAME),
                    (false, 0x0, MAX_FRAME),
                    (true, 0x0, 8),
                ]
            );
        }
        let small = WsMessage::Text("hi".into());
        assert_eq!(
            frame_lengths(&encode_message(&small, false)),
            vec![(true, 0x1, 2)]
        );
    }

    /// Codex sends a request above 1 MiB as 1 MiB fragments. The relay used to
    /// coalesce it into a single frame carrying the whole message; it must
    /// keep the upstream's frames bounded.
    #[tokio::test]
    async fn relay_forwards_a_fragmented_codex_request_in_bounded_frames() {
        let store = Arc::new(MonitorEventStore::new(16));
        let session = Arc::new(WsSession::new(
            counting(&store),
            WireFormat::OpenAiResponses,
            base_exchange(),
        ));
        let (client_side, proxy_client) = tokio::io::duplex(64 * 1024);
        let (upstream_side, proxy_upstream) = tokio::io::duplex(64 * 1024);
        let relay_task = tokio::spawn(relay(proxy_client, proxy_upstream, Some(session)));
        let (mut client_rd, mut client_wr) = tokio::io::split(client_side);
        let (mut upstream_rd, mut upstream_wr) = tokio::io::split(upstream_side);

        let request = json!({
            "type": "response.create",
            "model": "gpt-5.5",
            "input": [{"role": "user", "content": [{"type": "input_text",
                "text": "z".repeat(3 * MAX_FRAME)}]}],
        })
        .to_string();
        let wire = encode_message(&WsMessage::Text(request.clone()), true);
        assert!(frame_lengths(&wire).len() > 1, "the client fragments");
        let writer = tokio::spawn(async move {
            client_wr.write_all(&wire).await.unwrap();
            client_wr
        });

        let mut raw = Vec::new();
        let forwarded = read_message(&mut upstream_rd, &mut raw).await;
        assert_eq!(forwarded, WsMessage::Text(request));
        let frames = frame_lengths(&raw);
        assert!(frames.len() > 1);
        assert!(frames.iter().all(|(_, _, len)| *len <= MAX_FRAME));

        upstream_wr
            .write_all(&encode_frame(
                &WsMessage::Text(
                    json!({"type": "response.completed", "response": {"id": "resp_1",
                        "output": [], "usage": {"input_tokens": 1, "output_tokens": 1}}})
                    .to_string(),
                ),
                false,
            ))
            .await
            .unwrap();
        let mut ignored = Vec::new();
        read_message(&mut client_rd, &mut ignored).await;

        drop(writer.await.unwrap());
        drop(client_rd);
        drop(upstream_wr);
        drop(upstream_rd);
        relay_task.await.unwrap();
        let events = store.list(0, 10, None);
        assert_eq!(events.events.len(), 1);
        assert_eq!(events.events[0].status, EventStatus::Complete);
    }

    /// A message above the cap used to end in a bare TCP close, which Codex
    /// reports as "Connection reset without closing handshake" and retries on
    /// a new websocket five times. The relay answers 1009 instead, so Codex
    /// switches to HTTPS at once, and records why.
    #[tokio::test]
    async fn oversized_client_message_is_closed_with_1009_and_recorded() {
        let store = Arc::new(MonitorEventStore::new(16));
        let session = Arc::new(WsSession::new(
            counting(&store),
            WireFormat::OpenAiResponses,
            base_exchange(),
        ));
        let (client_side, proxy_client) = tokio::io::duplex(64 * 1024);
        let (upstream_side, proxy_upstream) = tokio::io::duplex(64 * 1024);
        let relay_task = tokio::spawn(relay(proxy_client, proxy_upstream, Some(session)));
        let (mut client_rd, mut client_wr) = tokio::io::split(client_side);
        let (mut upstream_rd, upstream_wr) = tokio::io::split(upstream_side);

        // A masked text frame header announcing more than MAX_MESSAGE bytes.
        let mut header = vec![0x81, 0x80 | 127];
        header.extend_from_slice(&(MAX_MESSAGE as u64 + 1).to_be_bytes());
        header.extend_from_slice(&[1, 2, 3, 4]);
        client_wr.write_all(&header).await.unwrap();

        let mut raw = Vec::new();
        let close = read_message(&mut client_rd, &mut raw).await;
        let WsMessage::Close(body) = close else {
            panic!("expected a close, got {close:?}");
        };
        assert_eq!(body[..2], 1009u16.to_be_bytes());
        assert!(std::str::from_utf8(&body[2..]).unwrap().contains("256 MiB"));

        let mut upstream_bytes = Vec::new();
        upstream_rd.read_to_end(&mut upstream_bytes).await.unwrap();
        assert_eq!(
            FrameDecoder::new().push(&upstream_bytes),
            vec![WsMessage::Close(1001u16.to_be_bytes().to_vec())],
            "nothing but a close reaches the upstream"
        );
        drop(upstream_wr);
        drop(upstream_rd);
        drop(client_wr);
        drop(client_rd);
        relay_task.await.unwrap();

        let events = store.list(0, 10, None);
        assert_eq!(events.events.len(), 1);
        assert_eq!(events.events[0].status, EventStatus::Error);
        let event = event_json(&store, &events.events[0].id);
        assert_eq!(event["data"]["status_code"], 413);
        assert!(event["data"]["error"].as_str().unwrap().contains("1009"));
    }

    /// Codex 0.169 steers a running response with `response.interrupt`. It is
    /// a control message: no firewall decision and no cycle of its own.
    #[tokio::test]
    async fn interrupt_is_forwarded_without_opening_a_cycle() {
        let store = Arc::new(MonitorEventStore::new(16));
        let interceptor = counting(&store);
        let session = WsSession::new(
            interceptor.clone(),
            WireFormat::OpenAiResponses,
            base_exchange(),
        );
        session
            .on_client_message(br#"{"type":"response.create","model":"gpt-5.5","input":"go"}"#)
            .await;
        session
            .on_server_message(r#"{"type":"response.created","response":{"id":"resp_1"}}"#)
            .await
            .unwrap();
        assert!(matches!(
            session
                .on_client_message(
                    br#"{"type":"response.interrupt","response_id":"resp_1","mode":"discard_partial_items"}"#
                )
                .await,
            ClientAction::Forward
        ));
        assert_eq!(interceptor.requests.load(Ordering::SeqCst), 1);
        assert!(session.previous.lock().is_empty());
        session
            .on_server_message(r#"{"type":"response.interrupt.accepted","response_id":"resp_1"}"#)
            .await
            .unwrap();
        session
            .on_server_message(
                r#"{"type":"response.incomplete","response":{"id":"resp_1","output":[]}}"#,
            )
            .await
            .unwrap();
        session.on_close().await;

        let events = store.list(0, 10, None);
        assert_eq!(events.events.len(), 1, "no event for the interrupt itself");
        assert_eq!(events.events[0].status, EventStatus::Complete);
    }

    /// An interrupt with fields beyond Codex's control shape is not trusted
    /// as content-free: it goes through the firewall like any request.
    #[tokio::test]
    async fn interrupt_with_extra_fields_is_inspected() {
        let store = Arc::new(MonitorEventStore::new(16));
        let interceptor = counting(&store);
        let session = WsSession::new(
            interceptor.clone(),
            WireFormat::OpenAiResponses,
            base_exchange(),
        );
        session
            .on_client_message(
                br#"{"type":"response.interrupt","response_id":"resp_1","input":"api_key=x"}"#,
            )
            .await;
        assert_eq!(interceptor.requests.load(Ordering::SeqCst), 1);
    }

    async fn abandoned_cycle_error(client_closes: bool) -> String {
        let store = Arc::new(MonitorEventStore::new(16));
        let session = Arc::new(WsSession::new(
            counting(&store),
            WireFormat::OpenAiResponses,
            base_exchange(),
        ));
        let (client_side, proxy_client) = tokio::io::duplex(64 * 1024);
        let (upstream_side, proxy_upstream) = tokio::io::duplex(64 * 1024);
        let relay_task = tokio::spawn(relay(proxy_client, proxy_upstream, Some(session)));
        let (mut client_rd, mut client_wr) = tokio::io::split(client_side);
        let (mut upstream_rd, upstream_wr) = tokio::io::split(upstream_side);
        client_wr
            .write_all(&encode_frame(
                &WsMessage::Text(r#"{"type":"response.create","model":"gpt-5.5"}"#.into()),
                true,
            ))
            .await
            .unwrap();
        let mut raw = Vec::new();
        read_message(&mut upstream_rd, &mut raw).await;
        let mut rest = Vec::new();
        if client_closes {
            drop(client_wr);
            drop(client_rd);
            upstream_rd.read_to_end(&mut rest).await.unwrap();
            drop(upstream_wr);
            drop(upstream_rd);
        } else {
            drop(upstream_wr);
            drop(upstream_rd);
            client_rd.read_to_end(&mut rest).await.unwrap();
            drop(client_wr);
            drop(client_rd);
        }
        relay_task.await.unwrap();
        let events = store.list(0, 10, None);
        assert_eq!(events.events.len(), 1);
        assert_eq!(events.events[0].status, EventStatus::Error);
        event_json(&store, &events.events[0].id)["data"]["error"]
            .as_str()
            .unwrap()
            .to_owned()
    }

    #[tokio::test]
    async fn abandoned_cycle_names_the_side_that_closed() {
        let client = abandoned_cycle_error(true).await;
        assert!(
            client.starts_with("Client closed the WebSocket"),
            "{client}"
        );
        let upstream = abandoned_cycle_error(false).await;
        assert!(
            upstream.starts_with("Upstream closed the WebSocket"),
            "{upstream}"
        );
    }
}
