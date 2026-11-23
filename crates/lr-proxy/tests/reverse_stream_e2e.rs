//! End-to-end tests for inspected request bodies the reverse proxy streams
//! (`BodyPolicy::Stream`): a real listener in front of a provider that
//! records every byte it receives (also of aborted uploads), bodies
//! generated on the fly.

mod support;

use std::convert::Infallible;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use futures::StreamExt;
use http_body_util::{BodyExt, Full, StreamBody};
use hyper::body::{Frame, Incoming};
use hyper::service::service_fn;
use hyper::{Request, Response};
use hyper_util::rt::TokioIo;
use serde_json::Value;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;

use lr_monitor::MonitorEventStore;
use lr_proxy::active::{ActiveInterceptor, Firewall, FirewallRequest, StreamRequest};
use lr_proxy::interceptor::{
    BodyPolicy, RequestAction, RequestHead, StreamMasking, StreamOpen, StreamPolicy,
};
use lr_proxy::passive::PassiveInterceptor;
use lr_proxy::request_stream::{
    Answer as SessionAnswer, Question, StreamAbort, StreamSession, TailUnit, UnitScope,
};
use lr_proxy::reverse::{ReverseClient, ReverseProxy, ReverseRecorder};
use lr_secret_scanner::request_stream::RequestMetadata;

use support::SyntheticConversation;

const MIB: usize = 1024 * 1024;
const SECRET_VALUE: &str = "abcdef0123456789";

/// Detects `token=<value>` and masks every detected value in every string.
struct Masker {
    known: Arc<Mutex<Vec<String>>>,
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
        let mut found = Vec::new();
        detect(value, &mut found);
        let mut known = self.known.lock().unwrap();
        let added: Vec<String> = found.into_iter().filter(|s| !known.contains(s)).collect();
        known.extend(added.iter().cloned());
        Ok(added)
    }
    fn mask(&mut self, units: &mut [TailUnit]) -> Result<(), StreamAbort> {
        let known = self.known.lock().unwrap().clone();
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

/// Asks about every value it detects; the answer (after a pause, so the
/// upload keeps arriving) masks it or denies the request.
struct Asker {
    masker: Masker,
    queued: std::collections::VecDeque<String>,
    asking: Option<String>,
    deny: bool,
}

impl StreamSession for Asker {
    fn inspect(&mut self, _: &UnitScope, value: &Value) -> Result<Vec<String>, StreamAbort> {
        let mut found = Vec::new();
        detect(value, &mut found);
        let known = self.masker.known.lock().unwrap().clone();
        for secret in found {
            if !known.contains(&secret)
                && !self.queued.contains(&secret)
                && self.asking.as_ref() != Some(&secret)
            {
                self.queued.push_back(secret);
            }
        }
        Ok(Vec::new())
    }
    fn mask(&mut self, units: &mut [TailUnit]) -> Result<(), StreamAbort> {
        self.masker.mask(units)
    }
    fn diagnostic(&mut self, _: &str) {}
    fn finish(&mut self, _: &RequestMetadata) {}
    fn pending(&self) -> bool {
        self.asking.is_some() || !self.queued.is_empty()
    }
    fn question(&mut self) -> Option<Question> {
        if self.asking.is_some() {
            return None;
        }
        self.asking = Some(self.queued.pop_front()?);
        Some(Question(Box::pin(async {
            tokio::time::sleep(Duration::from_millis(200)).await;
            SessionAnswer(Box::new(()))
        })))
    }
    fn answer(&mut self, _: SessionAnswer) -> Result<Vec<String>, StreamAbort> {
        let secret = self.asking.take().expect("an open question");
        if self.deny {
            let RequestAction::Reject { body, .. } = RequestAction::secret_scan_denied() else {
                unreachable!()
            };
            return Err(StreamAbort::with_json(403, "denied", body));
        }
        self.masker.known.lock().unwrap().push(secret.clone());
        Ok(vec![secret])
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Mode {
    Redact,
    Strict,
    Tee,
    Ask { deny: bool },
}

/// Streams every LLM request through a [`Masker`]; small bodies are judged
/// by `evaluate`, which masks with the same detector. A model named
/// `refused` is refused when the stream would open (the model gate).
struct StreamingFirewall {
    lag: usize,
    mode: Mode,
    opened: Arc<Mutex<usize>>,
    evaluated: Arc<Mutex<usize>>,
}

#[async_trait]
impl Firewall for StreamingFirewall {
    fn body_policy(&self, _: &RequestHead<'_>) -> BodyPolicy {
        BodyPolicy::Stream(StreamPolicy {
            masking: match self.mode {
                Mode::Redact | Mode::Strict => StreamMasking::Redact,
                Mode::Tee => StreamMasking::Notify,
                Mode::Ask { .. } => StreamMasking::Ask,
            },
            needs_model: true,
            whole_document: false,
            lag: self.lag,
            restore_only: false,
            strict: self.mode == Mode::Strict,
            incremental: false,
        })
    }
    async fn open_stream(&self, req: &StreamRequest, _: &StreamPolicy) -> StreamOpen {
        assert_eq!(req.provider, "ollama", "the wrapped provider names it");
        if req.model.as_deref() == Some("refused") {
            return StreamOpen::Reject(RequestAction::reject_json(403, "model refused"));
        }
        *self.opened.lock().unwrap() += 1;
        let masker = Masker {
            known: Default::default(),
        };
        let session: Box<dyn StreamSession> = match self.mode {
            Mode::Ask { deny } => Box::new(Asker {
                masker,
                queued: Default::default(),
                asking: None,
                deny,
            }),
            _ => Box::new(masker),
        };
        StreamOpen::Stream {
            session,
            vault: None,
            tee: self.mode == Mode::Tee,
        }
    }
    async fn evaluate(&self, req: &FirewallRequest) -> RequestAction {
        *self.evaluated.lock().unwrap() += 1;
        if req.model.as_deref() == Some("refused") {
            return RequestAction::reject_json(403, "model refused");
        }
        let mut found = Vec::new();
        detect(&req.body, &mut found);
        let mut body = req.body.clone();
        mask_value(&mut body, &found);
        RequestAction::Replace(serde_json::to_vec(&body).unwrap())
    }
}

/// Everything one upstream request delivered.
#[derive(Default, Debug)]
struct Received {
    headers: Option<hyper::HeaderMap>,
    /// Body bytes, kept up to the provider's `keep`.
    body: Vec<u8>,
    len: usize,
    /// The body ended normally.
    complete: bool,
    /// The secret seen anywhere in the body (checked across reads).
    leaked: bool,
}

/// How the provider answers.
#[derive(Clone, Copy, PartialEq)]
enum Answer {
    /// Read the whole body, then 200.
    AfterBody,
    /// 411 to a chunked body (without reading it), else 200.
    RejectChunked,
}

type Seen = Arc<Mutex<Vec<Received>>>;

/// A provider that records every byte of every request, also of bodies that
/// never complete, and searches them for the secret across reads.
async fn spawn_provider(answer: Answer, keep: usize) -> (u16, Seen) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen: Seen = Default::default();
    let all = seen.clone();
    tokio::spawn(async move {
        while let Ok((tcp, _)) = listener.accept().await {
            let all = all.clone();
            tokio::spawn(async move {
                let service = service_fn(move |req: Request<Incoming>| {
                    let all = all.clone();
                    async move {
                        let index = {
                            let mut all = all.lock().unwrap();
                            all.push(Received {
                                headers: Some(req.headers().clone()),
                                ..Default::default()
                            });
                            all.len() - 1
                        };
                        let chunked = req.headers().get("transfer-encoding").is_some();
                        if answer == Answer::RejectChunked && chunked {
                            return Ok::<_, Infallible>(
                                Response::builder()
                                    .status(411)
                                    .header("connection", "close")
                                    .body(Full::new(Bytes::from_static(b"length required")))
                                    .unwrap(),
                            );
                        }
                        let mut body = req.into_body();
                        let mut window = Vec::<u8>::new();
                        loop {
                            match body.frame().await {
                                None => {
                                    all.lock().unwrap()[index].complete = true;
                                    break;
                                }
                                Some(Err(_)) => break,
                                Some(Ok(frame)) => {
                                    let Ok(data) = frame.into_data() else {
                                        continue;
                                    };
                                    let mut all = all.lock().unwrap();
                                    let seen = &mut all[index];
                                    seen.len += data.len();
                                    let room = keep.saturating_sub(seen.body.len());
                                    seen.body.extend_from_slice(&data[..room.min(data.len())]);
                                    window.extend_from_slice(&data);
                                    if window
                                        .windows(SECRET_VALUE.len())
                                        .any(|w| w == SECRET_VALUE.as_bytes())
                                    {
                                        seen.leaked = true;
                                    }
                                    let cut = window.len().saturating_sub(SECRET_VALUE.len());
                                    window.drain(..cut);
                                }
                            }
                        }
                        Ok(Response::builder()
                            .header("content-type", "application/json")
                            .body(Full::new(Bytes::from_static(
                                br#"{"message":{"role":"assistant","content":"ok"},"done":true,"prompt_eval_count":1,"eval_count":1}"#,
                            )))
                            .unwrap())
                    }
                });
                hyper::server::conn::http1::Builder::new()
                    .serve_connection(TokioIo::new(tcp), service)
                    .await
                    .ok();
            });
        }
    });
    (port, seen)
}

struct Proxy {
    port: u16,
    store: Arc<MonitorEventStore>,
    opened: Arc<Mutex<usize>>,
    evaluated: Arc<Mutex<usize>>,
    spill: Arc<lr_proxy::spill::SpillConfig>,
    length_required: Arc<lr_proxy::framing::LengthRequiredHosts>,
    dir: std::path::PathBuf,
    _stop: oneshot::Sender<()>,
}

impl Drop for Proxy {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// A reverse proxy for `upstream_port` whose firewall streams in `mode` with
/// `lag`, holding at most 1 MiB per buffer in RAM (the rest spills).
async fn start_proxy(upstream_port: u16, lag: usize, mode: Mode) -> Proxy {
    let dir = std::env::temp_dir().join(format!("lr-reverse-stream-{}", uuid::Uuid::new_v4()));
    let store = Arc::new(MonitorEventStore::new(64));
    let opened = Arc::new(Mutex::new(0));
    let evaluated = Arc::new(Mutex::new(0));
    let recorder: Arc<dyn ReverseRecorder> = Arc::new(ActiveInterceptor::new(
        PassiveInterceptor::new(store.clone()),
        Arc::new(StreamingFirewall {
            lag,
            mode,
            opened: opened.clone(),
            evaluated: evaluated.clone(),
        }),
    ));
    let spill = Arc::new(lr_proxy::spill::SpillConfig::with_limits(
        dir.join("spill"),
        MIB,
        lr_proxy::spill::DEFAULT_REQUEST_CAP,
        lr_proxy::spill::DEFAULT_GLOBAL_BUDGET,
    ));
    let length_required = Arc::new(lr_proxy::framing::LengthRequiredHosts::default());
    let proxy = Arc::new(
        ReverseProxy::new(
            &format!("http://127.0.0.1:{upstream_port}"),
            ReverseClient {
                client_id: "client".into(),
                strategy_id: "strategy".into(),
                provider_instance: Some("ollama".into()),
            },
            recorder,
        )
        .unwrap()
        .with_spill(Some(spill.clone()))
        .with_length_required(length_required.clone()),
    );
    let listener = ReverseProxy::bind("127.0.0.1", 0).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (stop, stopped) = oneshot::channel::<()>();
    tokio::spawn(async move {
        proxy
            .serve(listener, async {
                let _ = stopped.await;
            })
            .await;
    });
    Proxy {
        port,
        store,
        opened,
        evaluated,
        spill,
        length_required,
        dir,
        _stop: stop,
    }
}

/// Send one request; returns the status and the body.
async fn send<B>(port: u16, path: &str, body: B) -> (u16, String)
where
    B: hyper::body::Body<Data = Bytes> + Send + 'static,
    B::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    let stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let (mut sender, conn) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
        .await
        .unwrap();
    tokio::spawn(async move {
        let _ = conn.await;
    });
    let request = Request::builder()
        .method("POST")
        .uri(path)
        .header("host", "127.0.0.1")
        .header("content-type", "application/json")
        .body(body)
        .unwrap();
    let response = sender.send_request(request).await.unwrap();
    let status = response.status().as_u16();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

fn marked(target: u64) -> SyntheticConversation {
    // The secret early (released long before the end when streamed) and
    // late (detected after earlier units were released).
    SyntheticConversation::messages(target).with_marker(3, &format!("token={SECRET_VALUE}"))
}

/// What the provider received for a body sent through a proxy with `lag`.
async fn forwarded(path: &str, conversation: &SyntheticConversation, lag: usize) -> (Value, usize) {
    let (up, seen) = spawn_provider(Answer::AfterBody, usize::MAX).await;
    let proxy = start_proxy(up, lag, Mode::Redact).await;
    let (status, body) = send(proxy.port, path, conversation.body()).await;
    assert_eq!(status, 200, "{body}");
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 1);
    assert!(seen[0].complete);
    assert!(!seen[0].leaked, "no secret reached the provider");
    let opened = *proxy.opened.lock().unwrap();
    (serde_json::from_slice(&seen[0].body).unwrap(), opened)
}

/// A body streamed through the pipeline (small lag) reaches the provider
/// equal to the buffered path's output (lag above the body), on every
/// streamed reverse-proxy path.
#[tokio::test]
async fn streamed_and_buffered_bodies_are_masked_alike() {
    let conversation = marked(2 * MIB as u64);
    for path in ["/api/chat", "/v1/chat/completions", "/v1/messages"] {
        let (streamed, opened) = forwarded(path, &conversation, 64 * 1024).await;
        assert_eq!(opened, 1, "{path}: streamed");
        let (buffered, opened) = forwarded(path, &conversation, 64 * MIB).await;
        assert_eq!(opened, 0, "{path}: buffered");
        assert_eq!(streamed, buffered, "{path}");
        let text = streamed.to_string();
        assert!(text.contains("token=[MASKED]"), "{path}");
        assert_eq!(
            streamed["messages"].as_array().unwrap().len(),
            conversation.turns()
        );
    }
}

/// A 64+ MiB Ollama chat body, four times the reverse proxy's buffered
/// limit, is masked and forwarded while it uploads (and its held bytes
/// spill beyond 1 MiB of RAM).
#[tokio::test]
async fn a_64_mib_ollama_chat_body_is_masked_and_forwarded() {
    let conversation = marked(66 * MIB as u64);
    let total = conversation.byte_len() as usize;
    let (up, seen) = spawn_provider(Answer::AfterBody, 0).await;
    let proxy = start_proxy(up, 256 * 1024, Mode::Redact).await;
    let (status, body) = send(proxy.port, "/api/chat", conversation.body()).await;
    assert_eq!(status, 200, "{body}");
    {
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert!(seen[0].complete && !seen[0].leaked);
        // Masking shortened one string; whitespace is unchanged (compact
        // JSON).
        assert!(
            seen[0].len > 64 * MIB && seen[0].len < total,
            "{}",
            seen[0].len
        );
        let headers = seen[0].headers.as_ref().unwrap();
        assert_eq!(headers["transfer-encoding"], "chunked");
    }
    assert_eq!(*proxy.opened.lock().unwrap(), 1);
    assert_eq!(*proxy.evaluated.lock().unwrap(), 0);
    for _ in 0..100 {
        if proxy.spill.in_flight() == 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(proxy.spill.in_flight(), 0, "spill budget released");
}

/// A model refused when the stream would open (also when `model` comes
/// after the conversation) answers 403 without contacting the provider.
#[tokio::test]
async fn a_refused_model_never_reaches_the_provider() {
    for model_last in [false, true] {
        let (up, seen) = spawn_provider(Answer::AfterBody, 0).await;
        let proxy = start_proxy(up, 64 * 1024, Mode::Redact).await;
        let mut conversation = marked(2 * MIB as u64);
        conversation.model = "refused";
        conversation.model_last = model_last;
        let (status, body) = send(proxy.port, "/api/chat", conversation.body()).await;
        assert_eq!(status, 403, "{body}");
        let body: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(body["error"]["message"], "model refused");
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(seen.lock().unwrap().is_empty(), "provider never contacted");
        // The refusal is recorded.
        let events = proxy.store.list(0, 10, None).events;
        assert_eq!(events.len(), 1);
    }
}

/// Secret Scan Ask: a denial answered after part of the body was forwarded
/// ends the upload (the provider never gets a complete body, nor the
/// secret) and answers the client with the denial; an approval to mask
/// forwards the masked body.
#[tokio::test]
async fn ask_holds_the_upload_and_a_denial_aborts_it() {
    // The secret far past the lag: earlier units were already released.
    let conversation = SyntheticConversation::messages(4 * MIB as u64)
        .with_marker(600, &format!("token={SECRET_VALUE}"));
    let (up, seen) = spawn_provider(Answer::AfterBody, 0).await;
    let proxy = start_proxy(up, 64 * 1024, Mode::Ask { deny: true }).await;
    let (status, body) = send(proxy.port, "/api/chat", conversation.body()).await;
    assert_eq!(status, 403, "{body}");
    assert!(body.contains("secret_scan_denied"), "{body}");
    tokio::time::sleep(Duration::from_millis(100)).await;
    {
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert!(!seen[0].complete, "the provider never got a complete body");
        assert!(seen[0].len > 0 && !seen[0].leaked);
    }

    let (up, seen) = spawn_provider(Answer::AfterBody, usize::MAX).await;
    let proxy = start_proxy(up, 64 * 1024, Mode::Ask { deny: false }).await;
    let (status, body) = send(proxy.port, "/api/chat", conversation.body()).await;
    assert_eq!(status, 200, "{body}");
    let seen = seen.lock().unwrap();
    assert!(seen[0].complete && !seen[0].leaked);
    let forwarded: Value = serde_json::from_slice(&seen[0].body).unwrap();
    assert!(forwarded.to_string().contains("token=[MASKED]"));
    drop(proxy);
}

/// Strict mode: nothing reaches the provider before the whole body was
/// inspected.
#[tokio::test]
async fn strict_mode_sends_nothing_before_the_end() {
    let conversation = marked(2 * MIB as u64);
    let bytes = conversation.to_vec();
    let (up, seen) = spawn_provider(Answer::AfterBody, usize::MAX).await;
    let proxy = start_proxy(up, 64 * 1024, Mode::Strict).await;
    let (mut tx, rx) = futures::channel::mpsc::channel::<Bytes>(4);
    let body = StreamBody::new(rx.map(|b| Ok::<_, Infallible>(Frame::data(b))));
    let port = proxy.port;
    let request = tokio::spawn(async move { send(port, "/api/chat", body).await });
    use futures::SinkExt;
    let half = bytes.len() * 3 / 4;
    for piece in bytes[..half].chunks(64 * 1024) {
        tx.send(Bytes::copy_from_slice(piece)).await.unwrap();
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        seen.lock().unwrap().is_empty(),
        "nothing sent while the body is still uploading"
    );
    for piece in bytes[half..].chunks(64 * 1024) {
        tx.send(Bytes::copy_from_slice(piece)).await.unwrap();
    }
    drop(tx);
    let (status, body) = request.await.unwrap();
    assert_eq!(status, 200, "{body}");
    let seen = seen.lock().unwrap();
    assert!(seen[0].complete && !seen[0].leaked);
    assert_eq!(*proxy.opened.lock().unwrap(), 1);
}

/// Off/Notify forward the client's original bytes with their length.
#[tokio::test]
async fn a_tee_forwards_the_original_bytes_with_their_length() {
    let conversation = SyntheticConversation::messages(2 * MIB as u64);
    let (up, seen) = spawn_provider(Answer::AfterBody, usize::MAX).await;
    let proxy = start_proxy(up, 64 * 1024, Mode::Tee).await;
    let (status, body) = send(proxy.port, "/api/generate", conversation.body()).await;
    assert_eq!(status, 200, "{body}");
    let seen = seen.lock().unwrap();
    assert_eq!(seen[0].body, conversation.to_vec());
    let headers = seen[0].headers.as_ref().unwrap();
    assert_eq!(
        headers["content-length"],
        conversation.byte_len().to_string().as_str()
    );
    assert_eq!(*proxy.opened.lock().unwrap(), 1);
}

/// A provider that refuses a chunked body is learned: that request fails,
/// the next one is held and sent with a `Content-Length`.
#[tokio::test]
async fn a_provider_refusing_chunked_bodies_gets_a_length() {
    let conversation = marked(2 * MIB as u64);
    let (up, seen) = spawn_provider(Answer::RejectChunked, usize::MAX).await;
    let proxy = start_proxy(up, 64 * 1024, Mode::Redact).await;
    let (status, _) = send(proxy.port, "/api/chat", conversation.body()).await;
    assert_eq!(status, 411);
    assert_eq!(
        proxy.length_required.learned(),
        vec![format!("127.0.0.1:{up}")]
    );
    let (status, body) = send(proxy.port, "/api/chat", conversation.body()).await;
    assert_eq!(status, 200, "{body}");
    let seen = seen.lock().unwrap();
    let last = seen.last().unwrap();
    assert!(last.complete && !last.leaked);
    let headers = last.headers.as_ref().unwrap();
    assert_eq!(headers["content-length"], last.len.to_string().as_str());
    assert!(headers.get("transfer-encoding").is_none());
}

/// A body at most the lag keeps the buffered path exactly: evaluated
/// whole, never streamed, sent with a `Content-Length`.
#[tokio::test]
async fn bodies_within_the_lag_stay_buffered() {
    let conversation = marked(512 * 1024);
    let (up, seen) = spawn_provider(Answer::AfterBody, usize::MAX).await;
    let proxy = start_proxy(up, MIB, Mode::Redact).await;
    let (status, body) = send(proxy.port, "/api/chat", conversation.body()).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(*proxy.opened.lock().unwrap(), 0);
    assert_eq!(*proxy.evaluated.lock().unwrap(), 1);
    {
        let seen = seen.lock().unwrap();
        assert!(seen[0]
            .headers
            .as_ref()
            .unwrap()
            .get("content-length")
            .is_some());
        assert!(!seen[0].leaked);
    }
    // An unstreamable body above the reverse proxy's buffered limit is
    // still refused with 413 (here: not JSON at all).
    let filler =
        support::SizedBody::new(support::filler(17 * MIB as u64, 64 * 1024), 17 * MIB as u64);
    let (status, _) = send(proxy.port, "/api/chat", filler).await;
    assert_eq!(status, 413);
    // The refusal is recorded, with its status.
    let summary = proxy.store.list(0, 10, None).events.into_iter().find(|e| {
        matches!(
            proxy.store.get(&e.id).map(|e| e.data),
            Some(lr_monitor::MonitorEventData::LlmCall {
                status_code: Some(413),
                ..
            })
        )
    });
    assert!(summary.is_some(), "413 recorded in the monitor");
}

/// A pass-through upload (a model blob) whose provider cannot be reached is
/// drained, so the client reads the 502 instead of a reset connection.
#[tokio::test]
async fn an_abandoned_pass_through_upload_is_drained() {
    let closed = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let up = closed.local_addr().unwrap().port();
    drop(closed);
    let proxy = start_proxy(up, 64 * 1024, Mode::Redact).await;
    use tokio::io::AsyncWriteExt;
    let mut tcp = TcpStream::connect(("127.0.0.1", proxy.port)).await.unwrap();
    let len = 32 * MIB;
    tcp.write_all(
        format!("POST /api/blobs/sha256:00 HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: {len}\r\n\r\n")
            .as_bytes(),
    )
    .await
    .unwrap();
    let block = vec![b'u'; 64 * 1024];
    for _ in 0..len / block.len() {
        tcp.write_all(&block)
            .await
            .expect("the upload is read to its end");
    }
    tcp.flush().await.unwrap();
    let (head, _) = tokio::time::timeout(Duration::from_secs(30), support::read_head(&mut tcp))
        .await
        .expect("an answer in time");
    assert!(head.starts_with("http/1.1 502"), "{head}");
}

/// The monitor shows a streamed reverse-proxy request with its sniffed
/// metadata and the head of the masked bytes.
#[tokio::test]
async fn streamed_requests_are_recorded_with_their_metadata() {
    let conversation = marked(2 * MIB as u64);
    let (up, _seen) = spawn_provider(Answer::AfterBody, 0).await;
    let proxy = start_proxy(up, 64 * 1024, Mode::Redact).await;
    let (status, _) = send(proxy.port, "/api/chat", conversation.body()).await;
    assert_eq!(status, 200);
    let mut events = Vec::new();
    for _ in 0..200 {
        events = proxy.store.list(0, 10, None).events;
        if events
            .iter()
            .any(|e| e.status != lr_monitor::EventStatus::Pending)
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let event = proxy.store.get(&events.first().unwrap().id).unwrap();
    let lr_monitor::MonitorEventData::LlmCall {
        requested_model,
        message_count,
        stream,
        raw_request,
        input_tokens,
        ..
    } = &event.data
    else {
        panic!("an LLM call");
    };
    assert_eq!(requested_model.as_deref(), Some("synthetic-model"));
    assert_eq!(*message_count, conversation.turns());
    assert!(*stream);
    assert_eq!(*input_tokens, Some(1));
    let raw = raw_request.as_deref().unwrap();
    assert!(!raw.contains(SECRET_VALUE) && raw.contains("token=[MASKED]"));
}

/// Send one request with extra headers; returns the status and the body.
async fn send_with<B>(port: u16, path: &str, body: B, headers: &[(&str, &str)]) -> (u16, String)
where
    B: hyper::body::Body<Data = Bytes> + Send + 'static,
    B::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    let stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let (mut sender, conn) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
        .await
        .unwrap();
    tokio::spawn(async move {
        let _ = conn.await;
    });
    let mut request = Request::builder()
        .method("POST")
        .uri(path)
        .header("host", "127.0.0.1")
        .header("content-type", "application/json");
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    let response = sender
        .send_request(request.body(body).unwrap())
        .await
        .unwrap();
    let status = response.status().as_u16();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

/// Off/Notify to a provider that needs a `Content-Length`: a client that
/// sent its body chunked gets its original bytes held and sent with a
/// length, never with `Transfer-Encoding: chunked` (hyper frames the body
/// from its size hint, which an explicit chunked header would override).
#[tokio::test]
async fn a_held_tee_of_a_chunked_body_is_sent_with_a_length() {
    let conversation = SyntheticConversation::messages(2 * MIB as u64);
    let bytes = conversation.to_vec();
    let (up, seen) = spawn_provider(Answer::AfterBody, usize::MAX).await;
    let proxy = start_proxy(up, 64 * 1024, Mode::Tee).await;
    proxy.length_required.learn(&format!("127.0.0.1:{up}"));
    let (mut tx, rx) = futures::channel::mpsc::channel::<Bytes>(4);
    let body = StreamBody::new(rx.map(|b| Ok::<_, Infallible>(Frame::data(b))));
    let port = proxy.port;
    let request = tokio::spawn(async move { send(port, "/api/chat", body).await });
    use futures::SinkExt;
    for piece in bytes.chunks(64 * 1024) {
        tx.send(Bytes::copy_from_slice(piece)).await.unwrap();
    }
    drop(tx);
    let (status, body) = request.await.unwrap();
    assert_eq!(status, 200, "{body}");
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 1);
    assert!(seen[0].complete);
    assert!(seen[0].body == bytes, "the original bytes");
    let headers = seen[0].headers.as_ref().unwrap();
    assert!(
        headers.get("transfer-encoding").is_none(),
        "{:?}",
        headers.get("transfer-encoding")
    );
    assert_eq!(headers["content-length"], bytes.len().to_string().as_str());
    assert_eq!(*proxy.opened.lock().unwrap(), 1);
}

/// A transformed zstd body is re-encoded as zstd on the reverse proxy, as on
/// the HTTPS proxy, whether it streamed (above the lag) or was buffered;
/// an identity body stays identity.
#[tokio::test]
async fn transformed_zstd_bodies_are_re_encoded_streamed_or_buffered() {
    let conversation = marked(2 * MIB as u64);
    let bytes = conversation.to_vec();
    let compressed = zstd::encode_all(&bytes[..], 3).unwrap();
    for (lag, streamed) in [(64 * 1024, true), (64 * MIB, false)] {
        let (up, seen) = spawn_provider(Answer::AfterBody, usize::MAX).await;
        let proxy = start_proxy(up, lag, Mode::Redact).await;
        let (status, body) = send_with(
            proxy.port,
            "/api/chat",
            Full::new(Bytes::from(compressed.clone())),
            &[("content-encoding", "zstd")],
        )
        .await;
        assert_eq!(status, 200, "{body}");
        assert_eq!(*proxy.opened.lock().unwrap(), usize::from(streamed));
        let seen = seen.lock().unwrap();
        assert!(seen[0].complete);
        let headers = seen[0].headers.as_ref().unwrap();
        assert_eq!(headers["content-encoding"], "zstd", "streamed {streamed}");
        let decoded = zstd::decode_all(&seen[0].body[..]).unwrap();
        let text = String::from_utf8_lossy(&decoded);
        assert!(!text.contains(SECRET_VALUE), "streamed {streamed}");
        let forwarded: Value = serde_json::from_slice(&decoded).unwrap();
        assert!(forwarded.to_string().contains("token=[MASKED]"));
        assert_eq!(
            forwarded["messages"].as_array().unwrap().len(),
            conversation.turns()
        );
    }
    // Identity stays identity on both paths.
    for lag in [64 * 1024, 64 * MIB] {
        let (up, seen) = spawn_provider(Answer::AfterBody, usize::MAX).await;
        let proxy = start_proxy(up, lag, Mode::Redact).await;
        let (status, body) = send(proxy.port, "/api/chat", conversation.body()).await;
        assert_eq!(status, 200, "{body}");
        let seen = seen.lock().unwrap();
        let headers = seen[0].headers.as_ref().unwrap();
        assert!(headers.get("content-encoding").is_none());
        assert!(!seen[0].leaked);
    }
}
