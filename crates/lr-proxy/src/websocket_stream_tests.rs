//! Frame-level decoding and per-message streaming of client websocket
//! messages (plan 2026-11-16, Phase 2), plus the server-message cap.

use super::*;
use crate::active::{ActiveInterceptor, Firewall, FirewallRequest, StreamRequest};
use crate::interceptor::{StreamMasking, StreamPolicy};
use crate::passive::PassiveInterceptor;
use crate::request_stream::{StreamAbort, StreamSession, TailUnit, UnitScope};
use lr_monitor::{EventStatus, MonitorEventStore};
use lr_secret_scanner::request_stream::RequestMetadata;
use serde_json::json;
use std::sync::atomic::AtomicUsize;
use std::time::Duration;
use tokio::io::{ReadHalf, WriteHalf};

const KIB: usize = 1024;
const WAIT: Duration = Duration::from_secs(20);
/// Marks a unit the test session refuses while it streams.
const ABORT_MARKER: &str = "ABORT-HERE";

// --- codec ---

/// Data events of `wire`, fed in pieces of `step` bytes.
fn events(wire: &[u8], step: usize) -> (Vec<FrameEvent>, Option<Poison>) {
    let mut reader = FrameReader::new(MAX_MESSAGE);
    let mut out = Vec::new();
    for piece in wire.chunks(step.max(1)) {
        reader.push(piece, &mut out);
    }
    (out, reader.poison())
}

/// Concatenated data payload of a list of events.
fn payload_of(events: &[FrameEvent]) -> Vec<u8> {
    events
        .iter()
        .filter_map(|e| match e {
            FrameEvent::DataChunk(c) => Some(c.as_slice()),
            _ => None,
        })
        .flatten()
        .copied()
        .collect()
}

#[test]
fn payload_is_delivered_before_the_frame_is_complete() {
    let payload = vec![b'x'; 100_000];
    let wire = encode_frame(&WsMessage::Binary(payload.clone()), true);
    let mut reader = FrameReader::new(MAX_MESSAGE);
    let mut out = Vec::new();
    // Header (2 + 8 + 4 bytes) plus 10 payload bytes.
    reader.push(&wire[..24], &mut out);
    assert_eq!(
        out,
        vec![
            FrameEvent::DataStart { opcode: 0x2 },
            FrameEvent::DataChunk(vec![b'x'; 10])
        ]
    );
    out.clear();
    reader.push(&wire[24..], &mut out);
    assert_eq!(out.last(), Some(&FrameEvent::DataEnd));
    assert_eq!(payload_of(&out).len(), payload.len() - 10);
}

#[test]
fn masking_offset_is_carried_across_reads() {
    let payload: Vec<u8> = (0..=255u8).cycle().take(70_001).collect();
    let wire = encode_frame(&WsMessage::Binary(payload.clone()), true);
    for step in [1, 3, 5, 4096] {
        let (events, poison) = events(&wire, step);
        assert_eq!(poison, None);
        assert_eq!(payload_of(&events), payload, "step {step}");
        assert_eq!(events.last(), Some(&FrameEvent::DataEnd));
    }
}

#[test]
fn fragments_and_control_frames_keep_their_order() {
    let mut wire = Vec::new();
    push_frame(&mut wire, false, 0x1, b"he", true);
    push_frame(&mut wire, true, 0x9, b"p", true);
    push_frame(&mut wire, false, 0x0, b"", true);
    push_frame(&mut wire, true, 0x0, b"llo", true);
    let (events, poison) = events(&wire, 1);
    assert_eq!(poison, None);
    let shape: Vec<String> = events
        .iter()
        .map(|e| match e {
            FrameEvent::DataStart { opcode } => format!("start{opcode}"),
            FrameEvent::DataChunk(c) => String::from_utf8(c.clone()).unwrap(),
            FrameEvent::DataEnd => "end".into(),
            FrameEvent::Control(m) => format!("{m:?}"),
        })
        .collect();
    // Byte-by-byte delivery cuts the chunks into single bytes.
    assert_eq!(
        shape,
        ["start1", "h", "e", "Ping([112])", "l", "l", "o", "end"]
    );
}

#[test]
fn empty_data_messages_start_and_end() {
    let wire = encode_frame(&WsMessage::Text(String::new()), true);
    let (events, poison) = events(&wire, 64);
    assert_eq!(poison, None);
    assert_eq!(
        events,
        vec![FrameEvent::DataStart { opcode: 0x1 }, FrameEvent::DataEnd]
    );
    assert_eq!(
        FrameDecoder::new().push(&wire),
        vec![WsMessage::Text(String::new())]
    );
}

#[test]
fn malformed_frames_poison_the_reader() {
    let cases: Vec<(&str, Vec<u8>)> = vec![
        // 64-bit length with the most significant bit set.
        ("length msb", {
            let mut f = vec![0x82, 127];
            f.extend_from_slice(&(1u64 << 63).to_be_bytes());
            f
        }),
        // Close body of one byte (a code needs two).
        ("close 1 byte", vec![0x88, 0x01, 0x03]),
        // Close reason that is not UTF-8.
        ("close reason", vec![0x88, 0x04, 0x03, 0xE8, 0xFF, 0xFE]),
        // A new data message while one is fragmented.
        ("data mid-message", vec![0x01, 0x01, b'a', 0x81, 0x01, b'b']),
        // RSV2 / RSV3.
        ("rsv2", vec![0xA1, 0x00]),
        ("rsv3", vec![0x91, 0x00]),
        // Fragmented close.
        ("fragmented close", vec![0x08, 0x02, 0x03, 0xE8]),
    ];
    for (name, wire) in cases {
        let (_, poison) = events(&wire, 1);
        assert_eq!(poison, Some(Poison::Protocol), "{name}");
        assert!(FrameDecoder::new().push(&wire).is_empty() || name == "data mid-message");
    }
    // A frame above the limit is refused from its header alone.
    let mut reader = FrameReader::new(1000);
    let mut out = Vec::new();
    reader.push(&[0x82, 126, 0x03, 0xE9], &mut out);
    assert_eq!(reader.poison(), Some(Poison::TooBig));
    assert!(out.is_empty(), "no DataStart for a refused frame");
}

#[test]
fn close_frames_carry_a_bounded_reason() {
    let long = "é".repeat(100);
    let frame = close_frame(1011, false, &long);
    let message = FrameDecoder::new().push(&frame).pop().unwrap();
    let WsMessage::Close(body) = message else {
        panic!("not a close")
    };
    assert_eq!(body[..2], 1011u16.to_be_bytes());
    assert!(body.len() <= 125);
    assert!(std::str::from_utf8(&body[2..]).is_ok());
}

#[test]
fn the_message_decoder_enforces_its_limit_across_fragments() {
    let mut decoder = FrameDecoder::with_limit(10);
    let mut wire = Vec::new();
    push_frame(&mut wire, false, 0x1, b"123456", false);
    push_frame(&mut wire, true, 0x0, b"78901", false);
    assert!(decoder.push(&wire).is_empty());
    assert_eq!(decoder.poison(), Some(Poison::TooBig));
}

// --- streaming relay ---

/// Detects `token=<value>` and masks every detected value in every string;
/// refuses a unit holding [`ABORT_MARKER`].
struct Masker {
    known: Arc<parking_lot::Mutex<Vec<String>>>,
}

fn mask_value(value: &mut Value, known: &[String]) {
    match value {
        Value::String(s) => {
            for secret in known {
                *s = s.replace(secret.as_str(), "[MASKED]");
            }
        }
        Value::Array(items) => items.iter_mut().for_each(|v| mask_value(v, known)),
        Value::Object(map) => map.values_mut().for_each(|v| mask_value(v, known)),
        _ => {}
    }
}

fn detect(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::String(s) => {
            for (i, _) in s.match_indices("token=") {
                let secret: String = s[i + 6..]
                    .chars()
                    .take_while(char::is_ascii_alphanumeric)
                    .collect();
                if secret.len() >= 8 {
                    out.push(secret);
                }
            }
        }
        Value::Array(items) => items.iter().for_each(|v| detect(v, out)),
        Value::Object(map) => map.values().for_each(|v| detect(v, out)),
        _ => {}
    }
}

impl StreamSession for Masker {
    fn inspect(&mut self, _: &UnitScope, value: &Value) -> Result<Vec<String>, StreamAbort> {
        if value.to_string().contains(ABORT_MARKER) {
            return Err(StreamAbort::new(403, "stopped by the test policy"));
        }
        let mut found = Vec::new();
        detect(value, &mut found);
        let mut known = self.known.lock();
        let added: Vec<String> = found.into_iter().filter(|s| !known.contains(s)).collect();
        known.extend(added.iter().cloned());
        Ok(added)
    }
    fn mask(&mut self, units: &mut [TailUnit]) -> Result<(), StreamAbort> {
        let known = self.known.lock().clone();
        for unit in units {
            if unit.scope.key() != "model" {
                mask_value(&mut unit.value, &known);
            }
        }
        Ok(())
    }
    fn diagnostic(&mut self, _: &str) {}
    fn finish(&mut self, _: &RequestMetadata) {}
}

/// A [`Masker`] that judges the whole message before any of it may leave
/// (as GuardRails do), for the model `verdict`.
struct Verdict(Masker);

impl StreamSession for Verdict {
    fn inspect(&mut self, scope: &UnitScope, value: &Value) -> Result<Vec<String>, StreamAbort> {
        self.0.inspect(scope, value)
    }
    fn mask(&mut self, units: &mut [TailUnit]) -> Result<(), StreamAbort> {
        self.0.mask(units)
    }
    fn diagnostic(&mut self, _: &str) {}
    fn finish(&mut self, _: &RequestMetadata) {}
    fn holds_until_verdict(&self) -> bool {
        true
    }
}

/// Streams messages above `lag` through a [`Masker`] (unless `stream` is
/// off); judges smaller ones with the same detector. Refuses the model
/// `refused` and any held message containing `REJECT-ME`.
struct TestFirewall {
    lag: usize,
    stream: bool,
    strict: bool,
    evaluated: AtomicUsize,
    opened: AtomicUsize,
}

#[async_trait::async_trait]
impl Firewall for TestFirewall {
    fn body_policy(&self, _: &RequestHead<'_>) -> BodyPolicy {
        if !self.stream {
            return BodyPolicy::Buffered;
        }
        BodyPolicy::Stream(StreamPolicy {
            masking: StreamMasking::Redact,
            needs_model: false,
            whole_document: false,
            lag: self.lag,
            restore_only: false,
            strict: self.strict,
            incremental: false,
        })
    }
    async fn open_stream(&self, req: &StreamRequest, _: &StreamPolicy) -> StreamOpen {
        if req.model.as_deref() == Some("refused") {
            return StreamOpen::Reject(RequestAction::reject_json(403, "model refused"));
        }
        self.opened.fetch_add(1, Ordering::SeqCst);
        let masker = Masker {
            known: Default::default(),
        };
        StreamOpen::Stream {
            session: if req.model.as_deref() == Some("verdict") {
                Box::new(Verdict(masker))
            } else {
                Box::new(masker)
            },
            vault: None,
            tee: false,
        }
    }
    async fn evaluate(&self, req: &FirewallRequest) -> RequestAction {
        self.evaluated.fetch_add(1, Ordering::SeqCst);
        if req.model.as_deref() == Some("refused") || req.body.to_string().contains("REJECT-ME") {
            return RequestAction::reject_json(403, "refused by the test policy");
        }
        let mut found = Vec::new();
        detect(&req.body, &mut found);
        if found.is_empty() {
            return RequestAction::Forward;
        }
        let mut body = req.body.clone();
        mask_value(&mut body, &found);
        RequestAction::Replace(body.to_string().into_bytes())
    }
    /// The same detector over any text.
    fn mask_for_monitor(&self, _: &str, content: &mut Vec<u8>) -> bool {
        let mut text = String::from_utf8_lossy(content).into_owned();
        let mut found = Vec::new();
        detect(&Value::String(text.clone()), &mut found);
        for secret in found {
            text = text.replace(&secret, "[MASKED]");
        }
        *content = text.into_bytes();
        true
    }
}

fn firewall(lag: usize) -> Arc<TestFirewall> {
    Arc::new(TestFirewall {
        lag,
        stream: true,
        strict: false,
        evaluated: AtomicUsize::new(0),
        opened: AtomicUsize::new(0),
    })
}

fn base() -> ObservedExchange {
    ObservedExchange {
        client_id: "client-1".into(),
        host: "chatgpt.com".into(),
        method: "GET".into(),
        path: "/backend-api/codex/responses".into(),
        ..Default::default()
    }
}

/// A Codex `response.create` with `turns` user turns of about `turn_bytes`
/// each; `text(i)` overrides a turn's text.
fn conversation(
    model: &str,
    turns: usize,
    turn_bytes: usize,
    text: impl Fn(usize) -> Option<String>,
) -> String {
    let input: Vec<Value> = (0..turns)
        .map(|i| {
            let text = text(i).unwrap_or_else(|| format!("turn {i} {}", "z".repeat(turn_bytes)));
            json!({"role": "user", "content": [{"type": "input_text", "text": text}]})
        })
        .collect();
    // Codex's key order (`model` before `input`), whatever serde_json's map
    // ordering in this build.
    format!(
        r#"{{"type":"response.create","model":{},"stream":true,"input":{}}}"#,
        Value::String(model.into()),
        Value::Array(input)
    )
}

/// `payload` as one message in frames of `frame` bytes (masked).
fn frames(opcode: u8, payload: &[u8], frame: usize) -> Vec<Vec<u8>> {
    let pieces: Vec<&[u8]> = payload.chunks(frame).collect();
    pieces
        .iter()
        .enumerate()
        .map(|(i, piece)| {
            let mut out = Vec::new();
            let op = if i == 0 { opcode } else { 0x0 };
            push_frame(&mut out, i + 1 == pieces.len(), op, piece, true);
            out
        })
        .collect()
}

/// Everything the upstream received, decoded frame by frame.
#[derive(Default)]
struct Recorded {
    events: Vec<FrameEvent>,
    raw_len: usize,
    eof: bool,
}

impl Recorded {
    fn data_ends(&self) -> usize {
        self.events
            .iter()
            .filter(|e| matches!(e, FrameEvent::DataEnd))
            .count()
    }
    fn starts(&self) -> usize {
        self.events
            .iter()
            .filter(|e| matches!(e, FrameEvent::DataStart { .. }))
            .count()
    }
    fn payload(&self) -> Vec<u8> {
        payload_of(&self.events)
    }
    fn closes(&self) -> Vec<Vec<u8>> {
        self.events
            .iter()
            .filter_map(|e| match e {
                FrameEvent::Control(WsMessage::Close(body)) => Some(body.clone()),
                _ => None,
            })
            .collect()
    }
    /// Payload of each complete message, in order.
    fn messages(&self) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        let mut current = Vec::new();
        for event in &self.events {
            match event {
                FrameEvent::DataStart { .. } => current.clear(),
                FrameEvent::DataChunk(c) => current.extend_from_slice(c),
                FrameEvent::DataEnd => out.push(std::mem::take(&mut current)),
                FrameEvent::Control(_) => {}
            }
        }
        out
    }
}

type Shared2 = Arc<parking_lot::Mutex<Recorded>>;

struct Harness {
    store: Arc<MonitorEventStore>,
    client_rd: ReadHalf<tokio::io::DuplexStream>,
    client_wr: WriteHalf<tokio::io::DuplexStream>,
    upstream_wr: WriteHalf<tokio::io::DuplexStream>,
    recorded: Shared2,
    relay: tokio::task::JoinHandle<()>,
    decoder: FrameDecoder,
    inbox: std::collections::VecDeque<WsMessage>,
}

fn start(firewall: Arc<TestFirewall>, limits: Option<(usize, usize)>) -> Harness {
    start_with(firewall, |session| match limits {
        Some((message, server)) => session.with_limits(message, server),
        None => session,
    })
}

/// A relay whose session judges through `firewall` (behind the active
/// interceptor), configured by `configure`.
fn start_with(
    firewall: Arc<dyn Firewall>,
    configure: impl FnOnce(WsSession) -> WsSession,
) -> Harness {
    let store = Arc::new(MonitorEventStore::new(32));
    let interceptor = Arc::new(ActiveInterceptor::new(
        PassiveInterceptor::new(store.clone()),
        firewall,
    ));
    start_session(store, interceptor, configure)
}

fn start_session(
    store: Arc<MonitorEventStore>,
    interceptor: Arc<dyn ProxyInterceptor>,
    configure: impl FnOnce(WsSession) -> WsSession,
) -> Harness {
    let session = configure(WsSession::new(
        interceptor,
        WireFormat::OpenAiResponses,
        base(),
    ));
    let (client_side, proxy_client) = tokio::io::duplex(256 * KIB);
    let (upstream_side, proxy_upstream) = tokio::io::duplex(256 * KIB);
    let relay = tokio::spawn(relay(proxy_client, proxy_upstream, Some(Arc::new(session))));
    let (client_rd, client_wr) = tokio::io::split(client_side);
    let (mut upstream_rd, upstream_wr) = tokio::io::split(upstream_side);
    let recorded: Shared2 = Default::default();
    let sink = recorded.clone();
    tokio::spawn(async move {
        let mut reader = FrameReader::new(usize::MAX);
        let mut buf = vec![0u8; 64 * KIB];
        loop {
            let n = upstream_rd.read(&mut buf).await.unwrap_or(0);
            let mut recorded = sink.lock();
            if n == 0 {
                recorded.eof = true;
                break;
            }
            recorded.raw_len += n;
            let mut events = Vec::new();
            reader.push(&buf[..n], &mut events);
            recorded.events.extend(events);
        }
    });
    Harness {
        store,
        client_rd,
        client_wr,
        upstream_wr,
        recorded,
        relay,
        decoder: FrameDecoder::new(),
        inbox: Default::default(),
    }
}

impl Harness {
    async fn wait_upstream(&self, what: &str, cond: impl Fn(&Recorded) -> bool) {
        tokio::time::timeout(WAIT, async {
            loop {
                if cond(&self.recorded.lock()) {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("upstream never saw: {what}"));
    }

    async fn send(&mut self, bytes: &[u8]) {
        self.client_wr.write_all(bytes).await.unwrap();
    }

    /// Send frames until the relay stops reading (it does once it closed
    /// the connection, so a refused message is not written to its end).
    async fn send_frames(&mut self, frames: &[Vec<u8>]) {
        for frame in frames {
            let written =
                tokio::time::timeout(Duration::from_secs(2), self.client_wr.write_all(frame)).await;
            if !matches!(written, Ok(Ok(()))) {
                break;
            }
        }
    }

    /// Next message the client receives; `None` once it is closed.
    async fn client_message(&mut self) -> Option<WsMessage> {
        let mut buf = vec![0u8; 64 * KIB];
        tokio::time::timeout(WAIT, async {
            loop {
                if let Some(message) = self.inbox.pop_front() {
                    return Some(message);
                }
                let n = self.client_rd.read(&mut buf).await.unwrap_or(0);
                if n == 0 {
                    return None;
                }
                self.inbox.extend(self.decoder.push(&buf[..n]));
            }
        })
        .await
        .expect("client message")
    }

    async fn upstream_sends(&mut self, message: &str) {
        self.upstream_wr
            .write_all(&encode_frame(&WsMessage::Text(message.into()), false))
            .await
            .unwrap();
    }

    async fn finish(self) -> Arc<MonitorEventStore> {
        let Harness {
            store,
            client_rd,
            client_wr,
            upstream_wr,
            relay,
            ..
        } = self;
        drop(client_wr);
        drop(client_rd);
        drop(upstream_wr);
        tokio::time::timeout(WAIT, relay)
            .await
            .expect("relay ends")
            .unwrap();
        store
    }
}

fn completed(id: &str) -> String {
    json!({"type": "response.completed", "response": {"id": id, "model": "gpt-5.5",
        "status": "completed", "output": [],
        "usage": {"input_tokens": 12, "output_tokens": 3}}})
    .to_string()
}

fn event_json(store: &MonitorEventStore, id: &str) -> Value {
    serde_json::to_value(store.get(id).expect("event exists")).unwrap()
}

/// The upstream holds the first fragment of a large message before the
/// client sends its last, and the message arrives intact and complete.
#[tokio::test]
async fn upstream_sees_the_first_fragment_before_the_client_sends_the_last() {
    let fw = firewall(64 * KIB);
    let mut h = start(fw.clone(), None);
    let request = conversation("gpt-5.5", 64, 16 * KIB, |_| None);
    let wire = frames(0x1, request.as_bytes(), 32 * KIB);
    let (head, last) = wire.split_at(wire.len() - 1);
    h.send_frames(head).await;
    h.wait_upstream("the first fragment", |r| {
        r.starts() == 1 && !r.payload().is_empty()
    })
    .await;
    assert_eq!(h.recorded.lock().data_ends(), 0, "not complete yet");
    h.send_frames(last).await;
    h.wait_upstream("the message end", |r| r.data_ends() == 1)
        .await;
    assert_eq!(h.recorded.lock().messages()[0], request.as_bytes());
    assert_eq!(fw.opened.load(Ordering::SeqCst), 1);
    assert_eq!(fw.evaluated.load(Ordering::SeqCst), 0);
    // Forwarded frames stay bounded.
    h.upstream_sends(&completed("resp_1")).await;
    assert!(h.client_message().await.is_some());
    let store = h.finish().await;
    let events = store.list(0, 10, None).events;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].status, EventStatus::Complete);
    let event = event_json(&store, &events[0].id);
    assert_eq!(event["data"]["model"], "gpt-5.5");
    assert_eq!(event["data"]["message_count"], 64);
    assert_eq!(event["data"]["input_tokens"], 12);
    // The request capture is the bounded head of what was forwarded.
    let raw = event["data"]["raw_request"].as_str().unwrap_or_default();
    assert!(raw.len() <= crate::passive::RAW_CAP + 1024, "{}", raw.len());
    assert!(raw.starts_with("{\"type\":\"response.create\""));
}

/// A session that must judge the whole message (GuardRails) cannot stream
/// fragments: the message is held and judged whole, as before.
#[tokio::test]
async fn a_verdict_session_holds_the_message_whole() {
    let fw = firewall(64 * KIB);
    let mut h = start(fw.clone(), None);
    let request = conversation("verdict", 64, 16 * KIB, |_| None);
    let wire = frames(0x1, request.as_bytes(), 32 * KIB);
    let (head, last) = wire.split_at(wire.len() - 1);
    h.send_frames(head).await;
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    assert!(
        h.recorded.lock().payload().is_empty(),
        "a verdict session streamed fragments"
    );
    assert_eq!(fw.opened.load(Ordering::SeqCst), 1);
    h.send_frames(last).await;
    h.wait_upstream("the message end", |r| r.data_ends() == 1)
        .await;
    assert_eq!(fw.evaluated.load(Ordering::SeqCst), 1, "judged whole");
    assert_eq!(h.recorded.lock().messages()[0], request.as_bytes());
    h.upstream_sends(&completed("resp_1")).await;
    assert!(h.client_message().await.is_some());
    let _ = h.finish().await;
}

#[tokio::test]
async fn strict_mode_forwards_nothing_before_the_message_ends() {
    let fw = Arc::new(TestFirewall {
        lag: 64 * KIB,
        stream: true,
        strict: true,
        evaluated: AtomicUsize::new(0),
        opened: AtomicUsize::new(0),
    });
    let mut h = start(fw.clone(), None);
    let request = conversation("gpt-5.5", 64, 16 * KIB, |_| None);
    let wire = frames(0x1, request.as_bytes(), 32 * KIB);
    let (head, last) = wire.split_at(wire.len() - 1);
    h.send_frames(head).await;
    // Everything but the last fragment is far above the lag; strict mode
    // still releases nothing until the message is complete.
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    assert!(
        h.recorded.lock().payload().is_empty(),
        "strict mode released bytes before the message ended"
    );
    h.send_frames(last).await;
    h.wait_upstream("the message end", |r| r.data_ends() == 1)
        .await;
    assert_eq!(h.recorded.lock().messages()[0], request.as_bytes());
    h.upstream_sends(&completed("resp_1")).await;
    assert!(h.client_message().await.is_some());
    let _ = h.finish().await;
}

#[tokio::test]
async fn client_ping_is_forwarded_between_fragments() {
    let mut h = start(firewall(64 * KIB), None);
    let request = conversation("gpt-5.5", 32, 16 * KIB, |_| None);
    let wire = frames(0x1, request.as_bytes(), 16 * KIB);
    let middle = wire.len() / 2;
    h.send_frames(&wire[..middle]).await;
    h.wait_upstream("a fragment", |r| !r.payload().is_empty())
        .await;
    h.send(&encode_frame(&WsMessage::Ping(b"keepalive".to_vec()), true))
        .await;
    h.wait_upstream("the ping", |r| {
        r.events
            .iter()
            .any(|e| matches!(e, FrameEvent::Control(WsMessage::Ping(_))))
    })
    .await;
    h.send_frames(&wire[middle..]).await;
    h.wait_upstream("the message end", |r| r.data_ends() == 1)
        .await;
    {
        let recorded = h.recorded.lock();
        let ping = recorded
            .events
            .iter()
            .position(|e| matches!(e, FrameEvent::Control(WsMessage::Ping(_))))
            .unwrap();
        let start = recorded
            .events
            .iter()
            .position(|e| matches!(e, FrameEvent::DataStart { .. }))
            .unwrap();
        let end = recorded
            .events
            .iter()
            .position(|e| matches!(e, FrameEvent::DataEnd))
            .unwrap();
        assert!(start < ping && ping < end, "ping between the fragments");
        assert_eq!(recorded.messages()[0], request.as_bytes());
    }
    h.finish().await;
}

/// Text fragments may split a UTF-8 character; the forwarded message is
/// still the same valid text.
#[tokio::test]
async fn utf8_split_across_fragments_is_forwarded_intact() {
    let mut h = start(firewall(4 * KIB), None);
    let request = conversation("gpt-5.5", 24, 0, |i| {
        Some(format!("{i} é漢😀 {}", "é漢😀".repeat(200)))
    });
    // Odd frame sizes split multi-byte characters everywhere.
    let wire = frames(0x1, request.as_bytes(), 7);
    h.send_frames(&wire).await;
    h.wait_upstream("the message end", |r| r.data_ends() == 1)
        .await;
    let forwarded = h.recorded.lock().messages()[0].clone();
    assert_eq!(String::from_utf8(forwarded).unwrap(), request);
    h.finish().await;
}

/// Invalid UTF-8 inside a streamed text message stops it; the upstream never
/// receives a complete message.
#[tokio::test]
async fn invalid_utf8_in_a_streamed_text_message_never_completes_upstream() {
    let mut h = start(firewall(4 * KIB), None);
    let mut request = conversation("gpt-5.5", 64, KIB, |_| None).into_bytes();
    let at = request.len() - 200;
    request[at] = 0xFF;
    h.send_frames(&frames(0x1, &request, 2 * KIB)).await;
    h.wait_upstream("the close", |r| !r.closes().is_empty())
        .await;
    {
        let recorded = h.recorded.lock();
        assert_eq!(recorded.data_ends(), 0);
        assert!(recorded.starts() == 1, "fragments had been sent");
        assert_eq!(recorded.closes()[0][..2], CLOSE_POLICY.to_be_bytes());
    }
    h.finish().await;
}

/// A refusal after fragments were sent closes the upstream with 1008 (the
/// partial message is never completed), answers the client with the error
/// event and closes it with 1011, and ends every open cycle with an error.
/// Nothing from the refused unit onwards reaches the upstream.
#[tokio::test]
async fn refusal_mid_message_closes_1008_upstream_and_1011_to_the_client() {
    let mut h = start(firewall(64 * KIB), None);
    // A cycle already in flight on the same connection.
    let small = conversation("gpt-5.5", 1, 10, |_| None);
    h.send(&encode_frame(&WsMessage::Text(small.clone()), true))
        .await;
    h.wait_upstream("the first message", |r| r.data_ends() == 1)
        .await;
    let request = conversation("gpt-5.5", 64, 16 * KIB, |i| match i {
        40 => Some(format!("{ABORT_MARKER} and token=AFTERabort123456")),
        _ => None,
    });
    h.send_frames(&frames(0x1, request.as_bytes(), 32 * KIB))
        .await;
    h.wait_upstream("the close", |r| !r.closes().is_empty())
        .await;
    {
        let recorded = h.recorded.lock();
        assert_eq!(
            recorded.data_ends(),
            1,
            "the streamed message never completes"
        );
        assert_eq!(recorded.starts(), 2);
        assert_eq!(recorded.closes()[0][..2], CLOSE_POLICY.to_be_bytes());
        let payload = String::from_utf8_lossy(&recorded.payload()).into_owned();
        assert!(payload.contains("turn 10 "), "earlier units were forwarded");
        assert!(!payload.contains(ABORT_MARKER));
        assert!(!payload.contains("AFTERabort123456"));
    }
    let reply = h.client_message().await.unwrap();
    let WsMessage::Text(reply) = reply else {
        panic!("expected the error event, got {reply:?}");
    };
    let reply: Value = serde_json::from_str(&reply).unwrap();
    assert_eq!(reply["type"], "error");
    assert_eq!(reply["status"], 403);
    let close = h.client_message().await.unwrap();
    let WsMessage::Close(body) = close else {
        panic!("expected a close, got {close:?}");
    };
    assert_eq!(body[..2], CLOSE_INTERNAL.to_be_bytes());
    assert_eq!(h.client_message().await, None, "nothing after the close");
    let store = h.finish().await;
    let events = store.list(0, 10, None).events;
    assert_eq!(
        events.len(),
        2,
        "the stopped message and the cycle in flight"
    );
    assert!(events.iter().all(|e| e.status == EventStatus::Error));
    let errors: Vec<String> = events
        .iter()
        .map(|e| {
            event_json(&store, &e.id)["data"]["error"]
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect();
    assert!(
        errors
            .iter()
            .any(|e| e.contains("stopped by the test policy")),
        "{errors:?}"
    );
    assert!(
        errors.iter().any(|e| e.contains("mid-message")),
        "{errors:?}"
    );
}

/// A refusal before any fragment went out (here: `open_stream` refuses the
/// model) is answered like a held message's: the rest is dropped, the
/// client gets the error event and the connection stays open. A message of
/// at most the lag refused by the firewall keeps today's behaviour too.
#[tokio::test]
async fn refusals_before_forwarding_keep_the_socket_open() {
    let mut h = start(firewall(64 * KIB), None);
    let refused = conversation("refused", 32, 16 * KIB, |_| None);
    h.send_frames(&frames(0x1, refused.as_bytes(), 32 * KIB))
        .await;
    let WsMessage::Text(reply) = h.client_message().await.unwrap() else {
        panic!("expected the error event");
    };
    assert!(reply.contains("model refused"), "{reply}");

    let small = conversation("gpt-5.5", 1, 10, |_| Some("REJECT-ME".into()));
    h.send(&encode_frame(&WsMessage::Text(small), true)).await;
    let WsMessage::Text(reply) = h.client_message().await.unwrap() else {
        panic!("expected the error event");
    };
    assert!(reply.contains("refused by the test policy"), "{reply}");

    // The connection still works.
    let allowed = conversation("gpt-5.5", 1, 10, |_| None);
    h.send(&encode_frame(&WsMessage::Text(allowed.clone()), true))
        .await;
    h.wait_upstream("the allowed message", |r| r.data_ends() == 1)
        .await;
    {
        let recorded = h.recorded.lock();
        assert_eq!(
            recorded.starts(),
            1,
            "refused messages never reached the upstream"
        );
        assert_eq!(recorded.messages()[0], allowed.as_bytes());
        assert!(recorded.closes().is_empty());
    }
    let store = h.finish().await;
    let events = store.list(0, 10, None).events;
    assert_eq!(events.len(), 3);
}

/// A message refused before anything was forwarded (at stream open, or held
/// and judged whole) never leaves its detected secret in the monitor: not in
/// the raw capture, the parsed body or the conversation title.
#[tokio::test]
async fn refused_messages_keep_no_secret_in_the_monitor() {
    const SECRET: &str = "RefusedSecret0123456";
    let mut h = start(firewall(64 * KIB), None);
    // Refused at stream open: the secret is in the captured head.
    let refused = conversation("refused", 32, 16 * KIB, |i| {
        (i == 0).then(|| format!("token={SECRET} first"))
    });
    h.send_frames(&frames(0x1, refused.as_bytes(), 32 * KIB))
        .await;
    let WsMessage::Text(reply) = h.client_message().await.unwrap() else {
        panic!("expected the error event");
    };
    assert!(reply.contains("model refused"), "{reply}");
    // Held and refused whole.
    let small = conversation("gpt-5.5", 1, 10, |_| {
        Some(format!("REJECT-ME token={SECRET}"))
    });
    h.send(&encode_frame(&WsMessage::Text(small), true)).await;
    let WsMessage::Text(reply) = h.client_message().await.unwrap() else {
        panic!("expected the error event");
    };
    assert!(reply.contains("refused by the test policy"), "{reply}");
    let store = h.finish().await;
    let events = store.list(0, 10, None).events;
    assert_eq!(events.len(), 2);
    let events: Vec<_> = events
        .iter()
        .map(|summary| event_json(&store, &summary.id))
        .collect();
    let json = serde_json::to_string(&events).unwrap();
    assert!(!json.contains(SECRET), "{json}");
    assert!(json.contains("[MASKED]"), "{json}");
}

#[tokio::test]
async fn control_messages_bypass_detection_with_a_bounded_parse() {
    let fw = firewall(64 * KIB);
    let store = Arc::new(MonitorEventStore::new(16));
    let session = WsSession::new(
        Arc::new(ActiveInterceptor::new(
            PassiveInterceptor::new(store.clone()),
            fw.clone(),
        )),
        WireFormat::OpenAiResponses,
        base(),
    );
    for control in [
        r#"{"type":"response.cancel"}"#.to_owned(),
        r#"{"type":"response.interrupt","response_id":"resp_1","mode":"discard_partial_items"}"#
            .to_owned(),
    ] {
        assert!(matches!(
            session.on_client_message(control.as_bytes()).await,
            ClientAction::Forward
        ));
    }
    assert_eq!(fw.evaluated.load(Ordering::SeqCst), 0);
    assert!(session.cycle.lock().is_none());
    // A "cancel" above the sniff bound is a request like any other.
    let large = json!({"type": "response.cancel", "pad": "x".repeat(CONTROL_SNIFF)}).to_string();
    session.on_client_message(large.as_bytes()).await;
    assert_eq!(fw.evaluated.load(Ordering::SeqCst), 1);
}

/// A streamed message is not limited by the hold cap; a message the
/// firewall judges whole still is (1009).
#[tokio::test]
async fn streamed_messages_may_exceed_the_hold_cap() {
    let cap = 512 * KIB;
    let mut h = start(firewall(64 * KIB), Some((cap, MAX_SERVER_MESSAGE)));
    let request = conversation("gpt-5.5", 192, 16 * KIB, |_| None);
    assert!(request.len() > 5 * cap);
    h.send_frames(&frames(0x1, request.as_bytes(), 256 * KIB))
        .await;
    h.wait_upstream("the message end", |r| r.data_ends() == 1)
        .await;
    assert_eq!(h.recorded.lock().messages()[0], request.as_bytes());
    h.finish().await;

    let fw = Arc::new(TestFirewall {
        lag: 64 * KIB,
        stream: false,
        strict: false,
        evaluated: AtomicUsize::new(0),
        opened: AtomicUsize::new(0),
    });
    let mut h = start(fw, Some((cap, MAX_SERVER_MESSAGE)));
    let request = conversation("gpt-5.5", 64, 16 * KIB, |_| None);
    h.send_frames(&frames(0x1, request.as_bytes(), 256 * KIB))
        .await;
    let close = h.client_message().await.unwrap();
    let WsMessage::Close(body) = close else {
        panic!("expected a close, got {close:?}");
    };
    assert_eq!(body[..2], CLOSE_TOO_BIG.to_be_bytes());
    h.wait_upstream("the close", |r| !r.closes().is_empty())
        .await;
    assert_eq!(h.recorded.lock().starts(), 0);
    h.finish().await;
}

/// An upstream recording every byte never receives a detected secret, in
/// any spelling the client used, across frame splits, before and after the
/// lag, and in the partial message of a stopped stream.
#[tokio::test]
async fn upstream_never_receives_a_detected_secret() {
    const SECRETS: [&str; 3] = ["EarlySecret123456", "LateSecret7890123", "SplitSecret55555"];
    for (abort, frame) in [(false, 1000), (true, 4093)] {
        let mut h = start(firewall(32 * KIB), None);
        let mut request = conversation("gpt-5.5", 96, 4 * KIB, |i| match i {
            0 => Some(format!("token={} start", SECRETS[0])),
            // The earlier secret again, after the lag.
            50 => Some(format!("again {} and token={}", SECRETS[0], SECRETS[1])),
            60 if abort => Some(format!("{ABORT_MARKER} {}", SECRETS[1])),
            70 => Some(format!("token={} split", SECRETS[2])),
            _ => None,
        });
        // The client spells one occurrence with JSON escapes.
        request = request.replacen("again Early", "again \\u0045arly", 1);
        h.send_frames(&frames(0x1, request.as_bytes(), frame)).await;
        if abort {
            h.wait_upstream("the close", |r| !r.closes().is_empty())
                .await;
        } else {
            h.wait_upstream("the message end", |r| r.data_ends() == 1)
                .await;
        }
        let payload = h.recorded.lock().payload();
        let text = String::from_utf8_lossy(&payload);
        for secret in SECRETS {
            assert!(!text.contains(secret), "{secret} leaked (abort={abort})");
            let escaped = serde_json::to_string(secret).unwrap();
            assert!(!text.contains(&escaped[1..escaped.len() - 1]));
            // No escape spelling either: decode every escape and look again.
            let unescaped = text.replace("\\u0045", "E");
            assert!(!unescaped.contains(secret), "{secret} leaked escaped");
        }
        if !abort {
            let forwarded: Value = serde_json::from_slice(&payload).unwrap();
            assert!(forwarded.to_string().contains("[MASKED]"));
        }
        h.finish().await;
    }
}

/// The monitor cycle of a streamed request stays bounded and keeps the
/// terminal event.
#[tokio::test]
async fn streamed_cycle_capture_is_bounded_and_keeps_the_terminal_event() {
    let mut h = start(firewall(64 * KIB), None);
    let request = conversation("gpt-5.5", 64, 16 * KIB, |_| None);
    h.send_frames(&frames(0x1, request.as_bytes(), 64 * KIB))
        .await;
    h.wait_upstream("the message end", |r| r.data_ends() == 1)
        .await;
    let delta =
        json!({"type": "response.output_text.delta", "delta": "x".repeat(4096)}).to_string();
    for _ in 0..(CAPTURE_CAP / 4096 + 4) {
        h.upstream_sends(&delta).await;
        assert!(h.client_message().await.is_some());
    }
    h.upstream_sends(&completed("resp_big")).await;
    assert!(h.client_message().await.is_some());
    let store = h.finish().await;
    let events = store.list(0, 10, None).events;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].status, EventStatus::Complete);
    let event = event_json(&store, &events[0].id);
    assert_eq!(event["data"]["input_tokens"], 12);
    assert_eq!(event["data"]["model"], "gpt-5.5");
}

/// Server messages above their cap close both sides with 1009 and the open
/// cycle records why.
#[tokio::test]
async fn server_message_above_its_cap_closes_with_1009() {
    let mut h = start(firewall(64 * KIB), Some((MAX_MESSAGE, 64 * KIB)));
    let small = conversation("gpt-5.5", 1, 10, |_| None);
    h.send(&encode_frame(&WsMessage::Text(small), true)).await;
    h.wait_upstream("the message", |r| r.data_ends() == 1).await;
    let big =
        json!({"type": "response.output_text.delta", "delta": "y".repeat(100 * KIB)}).to_string();
    h.upstream_sends(&big).await;
    let close = h.client_message().await.unwrap();
    let WsMessage::Close(body) = close else {
        panic!("expected a close, got {close:?}");
    };
    assert_eq!(body[..2], CLOSE_TOO_BIG.to_be_bytes());
    h.wait_upstream("the close", |r| !r.closes().is_empty())
        .await;
    assert_eq!(
        h.recorded.lock().closes()[0][..2],
        CLOSE_TOO_BIG.to_be_bytes()
    );
    let store = h.finish().await;
    let events = store.list(0, 10, None).events;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].status, EventStatus::Error);
    let error = event_json(&store, &events[0].id)["data"]["error"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(error.contains("1009"), "{error}");
}

/// A client that disconnects in the middle of a streamed message leaves the
/// upstream with a close, never with a completed message.
#[tokio::test]
async fn client_disconnect_mid_stream_never_completes_the_message() {
    let mut h = start(firewall(64 * KIB), None);
    let request = conversation("gpt-5.5", 64, 16 * KIB, |_| None);
    let wire = frames(0x1, request.as_bytes(), 32 * KIB);
    h.send_frames(&wire[..wire.len() / 2]).await;
    h.wait_upstream("a fragment", |r| !r.payload().is_empty())
        .await;
    let Harness {
        client_rd,
        client_wr,
        recorded,
        relay,
        upstream_wr,
        ..
    } = h;
    drop(client_wr);
    drop(client_rd);
    tokio::time::timeout(WAIT, async {
        while !recorded.lock().eof {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    {
        let recorded = recorded.lock();
        assert_eq!(recorded.data_ends(), 0);
        assert_eq!(recorded.closes()[0][..2], CLOSE_GOING_AWAY.to_be_bytes());
    }
    drop(upstream_wr);
    relay.await.unwrap();
}

#[path = "websocket_leftover_tests.rs"]
mod leftovers;
