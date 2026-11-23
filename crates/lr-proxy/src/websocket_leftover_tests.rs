//! Pass-through streaming, spill-backed holds and Ask questions on client
//! websocket messages.

use super::*;
use crate::request_stream::{Answer, Question};
use crate::spill::SpillConfig;

/// Forwards everything (pass-through policy) and answers `on_request` with
/// `action`; records every exchange it is shown.
struct PassFirewall {
    action: fn() -> RequestAction,
    requests: parking_lot::Mutex<Vec<ObservedExchange>>,
    /// Exchanges handed to `on_response`.
    recorded: parking_lot::Mutex<Vec<ObservedExchange>>,
    recorder: PassiveInterceptor,
}

#[async_trait::async_trait]
impl ProxyInterceptor for PassFirewall {
    fn on_connect(
        &self,
        host: &str,
        client: &crate::interceptor::ClientCtx,
    ) -> crate::interceptor::ConnectDecision {
        ProxyInterceptor::on_connect(&self.recorder, host, client)
    }
    fn body_policy(&self, _: &RequestHead<'_>) -> BodyPolicy {
        BodyPolicy::PassThrough
    }
    async fn on_request(&self, ex: &ObservedExchange) -> RequestAction {
        self.requests.lock().push(ex.clone());
        (self.action)()
    }
    fn begin(&self, ex: &ObservedExchange) -> Option<String> {
        ProxyInterceptor::begin(&self.recorder, ex)
    }
    async fn on_response(&self, ex: &ObservedExchange) {
        self.recorded.lock().push(ex.clone());
        ProxyInterceptor::on_response(&self.recorder, ex).await;
    }
}

fn pass_harness(
    action: fn() -> RequestAction,
    limits: Option<usize>,
) -> (Harness, Arc<PassFirewall>) {
    let store = Arc::new(MonitorEventStore::new(32));
    let interceptor = Arc::new(PassFirewall {
        action,
        requests: Default::default(),
        recorded: Default::default(),
        recorder: PassiveInterceptor::new(store.clone()),
    });
    let harness = start_session(store, interceptor.clone(), |session| match limits {
        Some(cap) => session.with_limits(cap, MAX_SERVER_MESSAGE),
        None => session,
    });
    (harness, interceptor)
}

/// The passive interceptor forwards a large message frame by frame without
/// holding it (the upstream holds the first fragment before the client sent
/// the last one), with no size limit, and the monitor still gets its model,
/// message count and a bounded capture.
#[tokio::test]
async fn pass_through_messages_are_forwarded_as_they_arrive() {
    let cap = 256 * KIB;
    let recorder_store = Arc::new(MonitorEventStore::new(32));
    let mut h = start_session(
        recorder_store.clone(),
        Arc::new(PassiveInterceptor::new(recorder_store.clone())),
        |s| s.with_limits(cap, MAX_SERVER_MESSAGE),
    );
    let request = conversation("gpt-5.5", 96, 16 * KIB, |_| None);
    assert!(request.len() > 5 * cap);
    let wire = frames(0x1, request.as_bytes(), 32 * KIB);
    let (head, last) = wire.split_at(wire.len() - 1);
    h.send_frames(head).await;
    h.wait_upstream("the first fragments", |r| {
        r.starts() == 1 && r.payload().len() > 2 * cap
    })
    .await;
    assert_eq!(h.recorded.lock().data_ends(), 0, "not complete yet");
    h.send_frames(last).await;
    h.wait_upstream("the message end", |r| r.data_ends() == 1)
        .await;
    assert_eq!(h.recorded.lock().messages()[0], request.as_bytes());
    h.upstream_sends(&completed("resp_1")).await;
    assert!(h.client_message().await.is_some());
    let store = h.finish().await;
    let events = store.list(0, 10, None).events;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].status, EventStatus::Complete);
    let event = event_json(&store, &events[0].id);
    assert_eq!(event["data"]["model"], "gpt-5.5");
    assert_eq!(event["data"]["message_count"], 96);
    let raw = event["data"]["raw_request"].as_str().unwrap_or_default();
    assert!(raw.len() <= crate::passive::RAW_CAP + 1024, "{}", raw.len());
    assert!(raw.starts_with("{\"type\":\"response.create\""));
}

/// Small messages keep the held path (control messages open no cycle; other
/// small messages are recorded whole), and pings between the fragments of a
/// pass-through message keep their place.
#[tokio::test]
async fn pass_through_keeps_control_messages_and_frame_order() {
    let store = Arc::new(MonitorEventStore::new(32));
    let mut h = start_session(
        store.clone(),
        Arc::new(PassiveInterceptor::new(store.clone())),
        |s| s,
    );
    h.send(&encode_frame(
        &WsMessage::Text(r#"{"type":"response.cancel"}"#.into()),
        true,
    ))
    .await;
    h.wait_upstream("the cancel", |r| r.data_ends() == 1).await;
    let small = conversation("gpt-5.5", 1, 10, |_| None);
    h.send(&encode_frame(&WsMessage::Text(small.clone()), true))
        .await;
    h.wait_upstream("the small message", |r| r.data_ends() == 2)
        .await;
    let request = conversation("gpt-5.5", 32, 16 * KIB, |_| None);
    let wire = frames(0x1, request.as_bytes(), 16 * KIB);
    let middle = wire.len() / 2;
    h.send_frames(&wire[..middle]).await;
    h.wait_upstream("a fragment", |r| r.starts() == 3).await;
    h.send(&encode_frame(&WsMessage::Ping(b"keepalive".to_vec()), true))
        .await;
    h.send_frames(&wire[middle..]).await;
    h.wait_upstream("the message end", |r| r.data_ends() == 3)
        .await;
    {
        let recorded = h.recorded.lock();
        let messages = recorded.messages();
        assert_eq!(messages[1], small.as_bytes());
        assert_eq!(messages[2], request.as_bytes());
        let ping = recorded
            .events
            .iter()
            .position(|e| matches!(e, FrameEvent::Control(WsMessage::Ping(_))))
            .unwrap();
        let start = recorded
            .events
            .iter()
            .rposition(|e| matches!(e, FrameEvent::DataStart { .. }))
            .unwrap();
        let end = recorded
            .events
            .iter()
            .rposition(|e| matches!(e, FrameEvent::DataEnd))
            .unwrap();
        assert!(start < ping && ping < end, "ping between the fragments");
    }
    let store = h.finish().await;
    // The cancel opened no cycle; the two requests did.
    assert_eq!(store.list(0, 10, None).events.len(), 2);
}

/// Text split anywhere (7-byte frames) is forwarded intact; invalid UTF-8 is
/// never forwarded and the partial message never completed.
#[tokio::test]
async fn pass_through_text_is_validated_before_it_is_forwarded() {
    let (mut h, _) = pass_harness(|| RequestAction::Forward, None);
    let request = conversation("gpt-5.5", 64, 0, |i| {
        Some(format!("{i} é漢😀 {}", "é漢😀".repeat(200)))
    });
    h.send_frames(&frames(0x1, request.as_bytes(), 7)).await;
    h.wait_upstream("the message end", |r| r.data_ends() == 1)
        .await;
    let forwarded = h.recorded.lock().messages()[0].clone();
    assert_eq!(String::from_utf8(forwarded).unwrap(), request);
    h.finish().await;

    let (mut h, _) = pass_harness(|| RequestAction::Forward, None);
    let mut request = conversation("gpt-5.5", 64, KIB, |_| None).into_bytes();
    let at = request.len() - 200;
    request[at] = 0xFF;
    h.send_frames(&frames(0x1, &request, 2 * KIB)).await;
    h.wait_upstream("the close", |r| !r.closes().is_empty())
        .await;
    {
        let recorded = h.recorded.lock();
        assert_eq!(recorded.data_ends(), 0);
        assert_eq!(recorded.starts(), 1, "fragments had been sent");
        assert!(recorded.payload().len() <= at, "nothing past the bad byte");
        assert_eq!(recorded.closes()[0][..2], CLOSE_GOING_AWAY.to_be_bytes());
    }
    let close = h.client_message().await.unwrap();
    let WsMessage::Close(body) = close else {
        panic!("expected a close, got {close:?}");
    };
    assert_eq!(body[..2], CLOSE_PROTOCOL_ERROR.to_be_bytes());
    h.finish().await;
}

/// The interceptor sees a pass-through message like an HTTP pass-through
/// request (no body, a bounded capture) and may refuse it; a rewrite cannot
/// be honored and refuses it too. The socket stays open either way.
#[tokio::test]
async fn pass_through_refusals_are_answered_and_rewrites_refused() {
    for (action, status) in [
        (
            (|| RequestAction::reject_json(403, "not today")) as fn() -> RequestAction,
            403,
        ),
        (|| RequestAction::Replace(b"{}".to_vec()), 500),
    ] {
        let (mut h, fw) = pass_harness(action, None);
        let request = conversation("gpt-5.5", 16, 16 * KIB, |_| None);
        h.send_frames(&frames(0x1, request.as_bytes(), 16 * KIB))
            .await;
        let WsMessage::Text(reply) = h.client_message().await.unwrap() else {
            panic!("expected the error event");
        };
        let reply: Value = serde_json::from_str(&reply).unwrap();
        assert_eq!(reply["status"], status);
        {
            let requests = fw.requests.lock();
            assert_eq!(requests.len(), 1);
            assert!(requests[0].request_body.is_none());
            let capture = requests[0].request_capture.as_ref().unwrap();
            assert!(capture.head.len() <= crate::passive::RAW_CAP);
            assert!(!capture.complete);
            // Refused here: not counted as a request, and its content is
            // masked like every refusal's.
            let recorded = fw.recorded.lock();
            assert_eq!(recorded.len(), 1);
            assert!(recorded[0].refused_locally);
        }
        if status == 403 {
            // A small message is judged whole, and refused the same way.
            let small = conversation("gpt-5.5", 1, 10, |_| None);
            h.send(&encode_frame(&WsMessage::Text(small), true)).await;
            assert!(h.client_message().await.is_some());
        }
        assert_eq!(h.recorded.lock().starts(), 0, "nothing was forwarded");
        assert!(h.recorded.lock().closes().is_empty());
        h.finish().await;
    }
}

/// A conversation whose `model` comes after the (huge) input.
fn model_last(turns: usize, turn_bytes: usize, secret: &str) -> String {
    let input: Vec<Value> = (0..turns)
        .map(|i| {
            let text = if i == turns / 2 {
                format!("token={secret}")
            } else {
                format!("turn {i} {}", "z".repeat(turn_bytes))
            };
            json!({"role": "user", "content": [{"type": "input_text", "text": text}]})
        })
        .collect();
    format!(
        r#"{{"type":"response.create","stream":true,"input":{},"model":"gpt-5.5"}}"#,
        Value::Array(input)
    )
}

fn spill_config(dir: &std::path::Path, ram: usize, cap: u64) -> Arc<SpillConfig> {
    Arc::new(SpillConfig::with_limits(
        dir.to_path_buf(),
        ram,
        cap,
        64 * 1024 * 1024,
    ))
}

/// A message whose `model` arrives after its input is held beyond the RAM
/// hold cap in the encrypted spill and streamed once `model` is known: the
/// upstream gets it masked and complete, the spill is released and leaves
/// no file. Above the spill cap it is refused (1009) with nothing sent.
#[tokio::test]
async fn late_model_messages_spill_instead_of_hitting_the_hold_cap() {
    let cap = 512 * KIB;
    let secret = "LateModelSecret1234";
    let request = model_last(96, 16 * KIB, secret);
    assert!(request.len() > 2 * cap);

    let dir = tempfile::tempdir().unwrap();
    let spill = spill_config(dir.path(), 128 * KIB, 64 * 1024 * 1024);
    let config = spill.clone();
    let mut h = start_with(firewall(64 * KIB), move |s| {
        s.with_limits(cap, MAX_SERVER_MESSAGE)
            .with_spill(Some(config))
    });
    h.send_frames(&frames(0x1, request.as_bytes(), 64 * KIB))
        .await;
    h.wait_upstream("the message end", |r| r.data_ends() == 1)
        .await;
    let forwarded = h.recorded.lock().messages()[0].clone();
    let text = String::from_utf8(forwarded).unwrap();
    assert!(!text.contains(secret), "the secret was masked");
    let value: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(value["model"], "gpt-5.5");
    assert_eq!(value["input"].as_array().unwrap().len(), 96);
    h.finish().await;
    assert_eq!(spill.in_flight(), 0, "the spill budget is released");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);

    // Above the spill cap: refused cleanly, nothing forwarded.
    let dir = tempfile::tempdir().unwrap();
    let spill = spill_config(dir.path(), 128 * KIB, cap as u64);
    let config = spill.clone();
    let mut h = start_with(firewall(64 * KIB), move |s| {
        s.with_limits(64 * 1024 * 1024, MAX_SERVER_MESSAGE)
            .with_spill(Some(config))
    });
    h.send_frames(&frames(0x1, request.as_bytes(), 64 * KIB))
        .await;
    let close = h.client_message().await.unwrap();
    let WsMessage::Close(body) = close else {
        panic!("expected a close, got {close:?}");
    };
    assert_eq!(body[..2], CLOSE_TOO_BIG.to_be_bytes());
    h.wait_upstream("the close", |r| !r.closes().is_empty())
        .await;
    assert_eq!(h.recorded.lock().starts(), 0, "nothing was forwarded");
    let store = h.finish().await;
    assert_eq!(spill.in_flight(), 0);
    let events = store.list(0, 10, None).events;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].status, EventStatus::Error);

    // Without spill the held message keeps its RAM cap, as before.
    let mut h = start(firewall(64 * KIB), Some((cap, MAX_SERVER_MESSAGE)));
    h.send_frames(&frames(0x1, request.as_bytes(), 64 * KIB))
        .await;
    let close = h.client_message().await.unwrap();
    assert!(
        matches!(close, WsMessage::Close(ref body) if body[..2] == CLOSE_TOO_BIG.to_be_bytes())
    );
    h.finish().await;
}

/// A message that may stream but ends before it outgrew its lag is judged
/// whole by the firewall, as before (it is only counted, never lexed).
#[tokio::test]
async fn messages_within_the_lag_are_judged_whole() {
    let fw = firewall(4 * 1024 * 1024);
    let mut h = start(fw.clone(), None);
    let request = conversation("gpt-5.5", 64, 16 * KIB, |i| {
        (i == 3).then(|| "token=HeldSecret12345".to_string())
    });
    h.send_frames(&frames(0x1, request.as_bytes(), 32 * KIB))
        .await;
    h.wait_upstream("the message end", |r| r.data_ends() == 1)
        .await;
    let forwarded = String::from_utf8(h.recorded.lock().messages()[0].clone()).unwrap();
    assert!(!forwarded.contains("HeldSecret12345"));
    assert_eq!(fw.evaluated.load(Ordering::SeqCst), 1);
    assert_eq!(fw.opened.load(Ordering::SeqCst), 0);
    h.finish().await;
}

/// Asks about every new `token=` value; the answer redacts it or denies the
/// request.
struct Asker {
    known: Vec<String>,
    queue: std::collections::VecDeque<String>,
    asked: Arc<AtomicUsize>,
    deny: bool,
}

impl StreamSession for Asker {
    fn inspect(&mut self, _: &UnitScope, value: &Value) -> Result<Vec<String>, StreamAbort> {
        let mut found = Vec::new();
        detect(value, &mut found);
        for secret in found {
            if !self.known.contains(&secret) && !self.queue.contains(&secret) {
                self.queue.push_back(secret);
            }
        }
        Ok(Vec::new())
    }
    fn mask(&mut self, units: &mut [TailUnit]) -> Result<(), StreamAbort> {
        for unit in units {
            if unit.scope.key() != "model" {
                mask_value(&mut unit.value, &self.known);
            }
        }
        Ok(())
    }
    fn diagnostic(&mut self, _: &str) {}
    fn finish(&mut self, _: &RequestMetadata) {}
    fn pending(&self) -> bool {
        !self.queue.is_empty()
    }
    fn question(&mut self) -> Option<Question> {
        if self.queue.is_empty() {
            return None;
        }
        self.asked.fetch_add(1, Ordering::SeqCst);
        let deny = self.deny;
        Some(Question(Box::pin(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            Answer(Box::new(deny))
        })))
    }
    fn answer(&mut self, answer: Answer) -> Result<Vec<String>, StreamAbort> {
        let deny = *answer.0.downcast::<bool>().unwrap();
        let secret = self.queue.pop_front().expect("a question was asked");
        if deny {
            return Err(StreamAbort::new(403, "denied by the test user"));
        }
        self.known.push(secret.clone());
        Ok(vec![secret])
    }
}

struct AskFirewall {
    deny: bool,
    asked: Arc<AtomicUsize>,
    preview: parking_lot::Mutex<Option<crate::active::StreamPreview>>,
}

#[async_trait::async_trait]
impl Firewall for AskFirewall {
    fn body_policy(&self, _: &RequestHead<'_>) -> BodyPolicy {
        BodyPolicy::Stream(StreamPolicy {
            masking: StreamMasking::Ask,
            needs_model: false,
            whole_document: false,
            lag: 64 * KIB,
            restore_only: false,
            strict: false,
            incremental: false,
        })
    }
    async fn open_stream(&self, req: &StreamRequest, _: &StreamPolicy) -> StreamOpen {
        *self.preview.lock() = req.preview.clone();
        StreamOpen::Stream {
            session: Box::new(Asker {
                known: Vec::new(),
                queue: Default::default(),
                asked: self.asked.clone(),
                deny: self.deny,
            }),
            vault: None,
            tee: false,
        }
    }
    async fn evaluate(&self, _: &FirewallRequest) -> RequestAction {
        RequestAction::Forward
    }
}

/// Secret Scan Ask on a streamed message: each finding is asked while
/// nothing is released; an allow-with-redaction masks it (also when the
/// finding is in the last unit, asked at the end of the message), a denial
/// closes the upstream with 1008 before the finding left. The approval sees
/// a read-only preview of the first bytes.
#[tokio::test]
async fn streamed_messages_ask_their_questions() {
    for (deny, at) in [(false, 40), (false, 63), (true, 40)] {
        let fw = Arc::new(AskFirewall {
            deny,
            asked: Default::default(),
            preview: Default::default(),
        });
        let mut h = start_with(fw.clone(), |s| s);
        let secret = format!("AskedSecret{at}xyz");
        let request = conversation("gpt-5.5", 64, 16 * KIB, |i| {
            (i == at).then(|| format!("token={secret}"))
        });
        h.send_frames(&frames(0x1, request.as_bytes(), 32 * KIB))
            .await;
        if deny {
            h.wait_upstream("the close", |r| !r.closes().is_empty())
                .await;
            let recorded = h.recorded.lock();
            assert_eq!(recorded.data_ends(), 0);
            assert_eq!(recorded.closes()[0][..2], CLOSE_POLICY.to_be_bytes());
            assert!(!String::from_utf8_lossy(&recorded.payload()).contains(&secret));
        } else {
            h.wait_upstream("the message end", |r| r.data_ends() == 1)
                .await;
            let text = String::from_utf8(h.recorded.lock().messages()[0].clone()).unwrap();
            assert!(!text.contains(&secret), "at {at}");
            assert!(text.contains("[MASKED]"));
            serde_json::from_str::<Value>(&text).unwrap();
        }
        assert_eq!(fw.asked.load(Ordering::SeqCst), 1, "at {at}");
        let preview = fw.preview.lock().clone().expect("a preview");
        assert!(preview.text.starts_with("{\"type\":\"response.create\""));
        assert!(preview.received > 64 * KIB as u64);
        assert!(preview.text.len() <= crate::stream_driver::PREVIEW_LIMIT);
        h.finish().await;
    }
}
