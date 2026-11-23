//! End-to-end tests for inspected request bodies the MITM proxy streams
//! (BodyPolicy::Stream): real TLS on both sides, an upstream that records
//! every byte it receives (also from aborted uploads), bodies generated on
//! the fly.

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
use rustls::{ClientConfig, RootCertStore};
use rustls_pki_types::ServerName;
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::oneshot;
use tokio_rustls::client::TlsStream;
use tokio_rustls::TlsConnector;

use lr_monitor::{EventStatus, MonitorEventStore};
use lr_proxy::active::{ActiveInterceptor, Firewall, FirewallRequest, StreamRequest};
use lr_proxy::cert::CertAuthority;
use lr_proxy::interceptor::{
    BodyPolicy, ClientCtx, ConnectDecision, ObservedExchange, PassthroughExchange,
    ProxyInterceptor, RequestAction, RequestHead, StreamMasking, StreamOpen, StreamPolicy,
};
use lr_proxy::passive::PassiveInterceptor;
use lr_proxy::request_stream::{
    Answer as SessionAnswer, Question, StreamAbort, StreamSession, TailUnit, UnitScope,
};
use lr_proxy::resolver::StaticResolver;
use lr_proxy::{tls, ProxyManager};
use lr_secret_scanner::request_stream::RequestMetadata;

use support::{make_ca, root_store, tls_listener, TestCa};

const HOST: &str = "localhost";
const CLIENT_ID: &str = "cid";
const SECRET: &str = "lr-secret";
const MIB: usize = 1024 * 1024;
const WAIT: Duration = Duration::from_secs(60);
/// Lag of the test firewall's stream policy.
const LAG: usize = 256 * 1024;

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

/// Asks about every `token=<value>` it detects, one at a time; the answer
/// (after a pause, so the upload keeps arriving meanwhile) is the test's.
struct Asker {
    masker: Masker,
    queued: std::collections::VecDeque<String>,
    asking: Option<String>,
    deny: bool,
    asked: Arc<Mutex<usize>>,
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
        *self.asked.lock().unwrap() += 1;
        Some(Question(Box::pin(async {
            tokio::time::sleep(Duration::from_millis(300)).await;
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

/// A GuardRails-like session: masks like [`Masker`], collects the text of
/// every released (masked) unit, and judges it once the body ended. Denies
/// when the text contains `harmful`.
struct Judge {
    masker: Masker,
    /// Text of the released units (the projection).
    text: String,
    ended: bool,
    asked: bool,
    judged: Arc<Mutex<usize>>,
}

fn collect_text(value: &Value, out: &mut String) {
    match value {
        Value::String(s) => out.push_str(s),
        Value::Array(items) => items.iter().for_each(|v| collect_text(v, out)),
        Value::Object(map) => map.values().for_each(|v| collect_text(v, out)),
        _ => {}
    }
}

impl StreamSession for Judge {
    fn inspect(&mut self, scope: &UnitScope, value: &Value) -> Result<Vec<String>, StreamAbort> {
        self.masker.inspect(scope, value)
    }
    fn mask(&mut self, units: &mut [TailUnit]) -> Result<(), StreamAbort> {
        assert!(!self.ended, "released after the end");
        self.masker.mask(units)?;
        for unit in units.iter() {
            collect_text(&unit.value, &mut self.text);
        }
        Ok(())
    }
    fn diagnostic(&mut self, _: &str) {}
    fn finish(&mut self, _: &RequestMetadata) {
        self.ended = true;
    }
    fn question(&mut self) -> Option<Question> {
        if !self.ended || self.asked {
            return None;
        }
        self.asked = true;
        *self.judged.lock().unwrap() += 1;
        let harmful = self.text.contains("harmful");
        // A popup nobody answers.
        let wait = if self.text.contains("unanswered") {
            Duration::from_secs(3600)
        } else {
            Duration::from_millis(200)
        };
        Some(Question(Box::pin(async move {
            tokio::time::sleep(wait).await;
            SessionAnswer(Box::new(harmful))
        })))
    }
    fn answer(&mut self, answer: SessionAnswer) -> Result<Vec<String>, StreamAbort> {
        if *answer.0.downcast::<bool>().unwrap() {
            let RequestAction::Reject { body, .. } =
                RequestAction::reject_json(403, "Request blocked by safety guardrails")
            else {
                unreachable!()
            };
            return Err(StreamAbort::with_json(
                403,
                "Request blocked by safety guardrails",
                body,
            ));
        }
        Ok(Vec::new())
    }
    fn holds_until_verdict(&self) -> bool {
        true
    }
}

/// How the test firewall streams.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Mode {
    /// A GuardRails-like verdict over the whole body (`tee`: forwarding
    /// the original bytes).
    Guard { tee: bool },
    /// Detect and mask (`token=` values).
    Redact,
    /// Redact, releasing nothing before the end.
    Strict,
    /// Forward the original bytes and only inspect.
    Tee,
    /// Ask about each value: mask it, or deny the request.
    Ask { deny: bool },
}

/// Streams every LLM request through a [`Masker`]; small bodies are judged
/// by `evaluate`, which masks with the same detector.
struct StreamingFirewall {
    lag: usize,
    opened: Arc<Mutex<usize>>,
    mode: Mode,
    asked: Arc<Mutex<usize>>,
}

#[async_trait]
impl Firewall for StreamingFirewall {
    fn body_policy(&self, _: &RequestHead<'_>) -> BodyPolicy {
        BodyPolicy::Stream(StreamPolicy {
            masking: match self.mode {
                Mode::Redact | Mode::Strict | Mode::Guard { tee: false } => StreamMasking::Redact,
                Mode::Tee | Mode::Guard { tee: true } => StreamMasking::Notify,
                Mode::Ask { .. } => StreamMasking::Ask,
            },
            needs_model: false,
            whole_document: false,
            lag: self.lag,
            restore_only: false,
            strict: self.mode == Mode::Strict,
            incremental: false,
        })
    }
    async fn open_stream(&self, req: &StreamRequest, _: &StreamPolicy) -> StreamOpen {
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
                asked: self.asked.clone(),
            }),
            Mode::Guard { .. } => Box::new(Judge {
                masker,
                text: String::new(),
                ended: false,
                asked: false,
                judged: self.asked.clone(),
            }),
            _ => Box::new(masker),
        };
        StreamOpen::Stream {
            session,
            vault: None,
            tee: matches!(self.mode, Mode::Tee | Mode::Guard { tee: true }),
        }
    }
    async fn evaluate(&self, req: &FirewallRequest) -> RequestAction {
        let mut found = Vec::new();
        detect(&req.body, &mut found);
        let mut body = req.body.clone();
        mask_value(&mut body, &found);
        RequestAction::Replace(serde_json::to_vec(&body).unwrap())
    }
}

struct Mitm(ActiveInterceptor);

#[async_trait]
impl ProxyInterceptor for Mitm {
    fn on_connect(&self, _: &str, _: &ClientCtx) -> ConnectDecision {
        ConnectDecision::Mitm
    }
    fn body_policy(&self, head: &RequestHead<'_>) -> BodyPolicy {
        ProxyInterceptor::body_policy(&self.0, head)
    }
    async fn open_stream(
        &self,
        ex: &ObservedExchange,
        metadata: &RequestMetadata,
        policy: &StreamPolicy,
    ) -> StreamOpen {
        ProxyInterceptor::open_stream(&self.0, ex, metadata, policy).await
    }
    async fn on_request(&self, ex: &ObservedExchange) -> RequestAction {
        ProxyInterceptor::on_request(&self.0, ex).await
    }
    fn begin(&self, ex: &ObservedExchange) -> Option<String> {
        ProxyInterceptor::begin(&self.0, ex)
    }
    async fn on_response(&self, ex: &ObservedExchange) {
        self.0.on_response(ex).await
    }
    fn begin_passthrough(&self, ex: &PassthroughExchange) -> Option<String> {
        self.0.begin_passthrough(ex)
    }
    fn end_passthrough(&self, event_id: Option<String>, ex: &PassthroughExchange) {
        self.0.end_passthrough(event_id, ex)
    }
}

struct Proxy {
    port: u16,
    ca_pem: String,
    store: Arc<MonitorEventStore>,
    opened: Arc<Mutex<usize>>,
    asked: Arc<Mutex<usize>>,
    length_required: Arc<lr_proxy::framing::LengthRequiredHosts>,
    spill: Arc<lr_proxy::spill::SpillConfig>,
    dir: std::path::PathBuf,
    _stop: oneshot::Sender<()>,
}

impl Drop for Proxy {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

async fn start_proxy(up_ca: &TestCa, lag: usize) -> Proxy {
    start_proxy_with(up_ca, lag, Mode::Redact, &[]).await
}

/// A proxy whose firewall streams in `mode`, holding at most 1 MiB in RAM
/// per buffer (the rest spills to the test's directory), with `hosts`
/// configured to receive a Content-Length.
async fn start_proxy_with(up_ca: &TestCa, lag: usize, mode: Mode, hosts: &[&str]) -> Proxy {
    tls::ensure_crypto_provider();
    let dir = std::env::temp_dir().join(format!("lr-streamed-{}", uuid::Uuid::new_v4()));
    let ca = Arc::new(CertAuthority::load_or_create(&dir).unwrap());
    let ca_pem = ca.ca_pem().to_string();
    let store = Arc::new(MonitorEventStore::new(64));
    let opened = Arc::new(Mutex::new(0));
    let asked = Arc::new(Mutex::new(0));
    let mut roots = RootCertStore::empty();
    roots.add(up_ca.ca_der.clone()).unwrap();
    let interceptor = Mitm(ActiveInterceptor::new(
        PassiveInterceptor::new(store.clone()),
        Arc::new(StreamingFirewall {
            lag,
            opened: opened.clone(),
            mode,
            asked: asked.clone(),
        }),
    ));
    let length_required = Arc::new(lr_proxy::framing::LengthRequiredHosts::new(
        hosts.iter().map(|h| h.to_string()),
    ));
    let spill = Arc::new(lr_proxy::spill::SpillConfig::with_limits(
        dir.join("spill"),
        MIB,
        lr_proxy::spill::DEFAULT_REQUEST_CAP,
        lr_proxy::spill::DEFAULT_GLOBAL_BUDGET,
    ));
    let manager = ProxyManager::with_upstream_roots(
        ca,
        Arc::new(interceptor),
        Arc::new(StaticResolver {
            client_id: CLIENT_ID.into(),
            secret: SECRET.into(),
            proxy_enabled: true,
        }),
        roots,
    )
    .unwrap()
    .with_spill(Some(spill.clone()))
    .with_length_required(length_required.clone());
    let listener = ProxyManager::bind("127.0.0.1", 0).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (stop, stopped) = oneshot::channel::<()>();
    tokio::spawn(async move {
        manager
            .serve(listener, async {
                let _ = stopped.await;
            })
            .await;
    });
    Proxy {
        port,
        ca_pem,
        store,
        opened,
        asked,
        length_required,
        spill,
        dir,
        _stop: stop,
    }
}

async fn connect(proxy: &Proxy, up_port: u16) -> TlsStream<TcpStream> {
    use base64::Engine;
    let mut tcp = TcpStream::connect(("127.0.0.1", proxy.port)).await.unwrap();
    let auth = base64::engine::general_purpose::STANDARD.encode(format!("{CLIENT_ID}:{SECRET}"));
    tcp.write_all(
        format!(
            "CONNECT {HOST}:{up_port} HTTP/1.1\r\nHost: {HOST}:{up_port}\r\nProxy-Authorization: Basic {auth}\r\n\r\n"
        )
        .as_bytes(),
    )
    .await
    .unwrap();
    let mut seen = Vec::new();
    let mut byte = [0u8; 1];
    while !seen.ends_with(b"\r\n\r\n") {
        assert_eq!(tcp.read(&mut byte).await.unwrap(), 1, "proxy closed");
        seen.push(byte[0]);
    }
    let mut config = ClientConfig::builder()
        .with_root_certificates(root_store(&proxy.ca_pem))
        .with_no_client_auth();
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    TlsConnector::from(Arc::new(config))
        .connect(ServerName::try_from(HOST).unwrap(), tcp)
        .await
        .unwrap()
}

/// Everything one upstream request delivered.
#[derive(Default, Debug)]
struct Received {
    headers: Option<hyper::HeaderMap>,
    /// Body bytes, kept up to `keep`.
    body: Vec<u8>,
    len: usize,
    /// The body ended normally (a complete request). False when the
    /// connection was reset or closed before its end.
    complete: bool,
    /// Bytes received when the client was told the first bytes arrived.
    first_at: Option<usize>,
    /// Some forbidden string seen anywhere in the body (checked across reads).
    leaked: Option<String>,
}

/// How the test upstream answers.
#[derive(Clone, Copy, PartialEq)]
enum Answer {
    /// Read the whole body, then 200.
    AfterBody,
    /// Answer 401 right away, without reading the body.
    Early401,
    /// Read this many body bytes, then drop the connection.
    DropAfter(usize),
    /// Answer 411 to a chunked request body (with `Connection: close`,
    /// without reading it); read others and answer 200.
    RejectChunked,
}

/// A hyper upstream that records every byte of every request it receives,
/// also of bodies that never complete, and searches it for `forbidden`
/// strings across read boundaries.
async fn spawn_upstream(
    ca: &TestCa,
    answer: Answer,
    keep: usize,
    forbidden: Vec<String>,
) -> (
    u16,
    Arc<Mutex<Vec<Received>>>,
    tokio::sync::mpsc::Receiver<()>,
) {
    let (listener, acceptor, port) = tls_listener(ca, HOST).await;
    let seen = Arc::new(Mutex::new(Vec::<Received>::new()));
    let (first_tx, first_rx) = tokio::sync::mpsc::channel(4);
    let all = seen.clone();
    let forbidden = Arc::new(forbidden);
    tokio::spawn(async move {
        loop {
            let Ok((tcp, _)) = listener.accept().await else {
                break;
            };
            let acceptor = acceptor.clone();
            let all = all.clone();
            let first_tx = first_tx.clone();
            let forbidden = forbidden.clone();
            tokio::spawn(async move {
                let Ok(tls) = acceptor.accept(tcp).await else {
                    return;
                };
                let service = service_fn(move |req: Request<Incoming>| {
                    let all = all.clone();
                    let first_tx = first_tx.clone();
                    let forbidden = forbidden.clone();
                    async move {
                        let index = {
                            let mut all = all.lock().unwrap();
                            all.push(Received {
                                headers: Some(req.headers().clone()),
                                ..Default::default()
                            });
                            all.len() - 1
                        };
                        let chunked = req
                            .headers()
                            .get("transfer-encoding")
                            .is_some_and(|v| v.as_bytes().eq_ignore_ascii_case(b"chunked"));
                        if answer == Answer::RejectChunked && chunked {
                            return Ok::<_, std::io::Error>(
                                Response::builder()
                                    .status(411)
                                    .header("connection", "close")
                                    .body(Full::new(Bytes::from_static(b"length required")))
                                    .unwrap(),
                            );
                        }
                        if answer == Answer::Early401 {
                            return Ok::<_, std::io::Error>(
                                Response::builder()
                                    .status(401)
                                    .body(Full::new(Bytes::from_static(b"{\"error\":\"auth\"}")))
                                    .unwrap(),
                            );
                        }
                        let mut body = req.into_body();
                        let mut tail = Vec::<u8>::new();
                        let longest = forbidden.iter().map(String::len).max().unwrap_or(0);
                        let mut first = true;
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
                                    tail.extend_from_slice(&data);
                                    let text = String::from_utf8_lossy(&tail).into_owned();
                                    if let Some(hit) =
                                        forbidden.iter().find(|f| text.contains(f.as_str()))
                                    {
                                        seen.leaked.get_or_insert(hit.clone());
                                    }
                                    let cut = tail.len().saturating_sub(longest * 6 + 8);
                                    tail.drain(..cut);
                                    if first {
                                        first = false;
                                        seen.first_at = Some(seen.len);
                                        let _ = first_tx.try_send(());
                                    }
                                    if let Answer::DropAfter(limit) = answer {
                                        if seen.len >= limit {
                                            // Panic the service: hyper drops the connection.
                                            drop(all);
                                            return Err(std::io::Error::other("drop"));
                                        }
                                    }
                                }
                            }
                        }
                        Ok(Response::builder()
                            .header("content-type", "application/json")
                            .body(Full::new(Bytes::from_static(
                                br#"{"output":[],"usage":{"input_tokens":1,"output_tokens":1}}"#,
                            )))
                            .unwrap())
                    }
                });
                hyper::server::conn::http1::Builder::new()
                    .serve_connection(TokioIo::new(tls), service)
                    .await
                    .ok();
            });
        }
    });
    (port, seen, first_rx)
}

/// A request body fed from a channel, so a test controls its pace.
type ClientBody = StreamBody<
    futures::stream::Map<
        futures::channel::mpsc::Receiver<Bytes>,
        fn(Bytes) -> Result<Frame<Bytes>, Infallible>,
    >,
>;

fn channel_body() -> (futures::channel::mpsc::Sender<Bytes>, ClientBody) {
    let (tx, rx) = futures::channel::mpsc::channel(4);
    let map: fn(Bytes) -> Result<Frame<Bytes>, Infallible> = |b| Ok(Frame::data(b));
    (tx, StreamBody::new(rx.map(map)))
}

/// One element of the generated conversation.
fn element(i: usize, text: &str) -> String {
    serde_json::to_string(&serde_json::json!({
        "type": "message",
        "role": "user",
        "content": [{"type":"input_text","text": format!("{i}: {text}")}]
    }))
    .unwrap()
}

/// A Responses body with `model` first, generated element by element.
struct Conversation {
    model: &'static str,
    elements: usize,
    filler: usize,
    /// Pseudo-random filler instead of repeated words.
    noise: bool,
    marks: Vec<(usize, String)>,
}

impl Conversation {
    fn pieces(&self) -> impl Iterator<Item = Bytes> + '_ {
        let filler = "lorem ipsum dolor ".repeat(self.filler / 18 + 1);
        std::iter::once(Bytes::from(format!(
            r#"{{"model":"{}","instructions":"Be brief.","input":["#,
            self.model
        )))
        .chain((0..self.elements).map(move |i| {
            let text = self
                .marks
                .iter()
                .find(|(at, _)| *at == i)
                .map(|(_, t)| t.clone())
                .unwrap_or_else(|| {
                    if self.noise {
                        // Barely compressible text, so a compressed body is
                        // still large when it streams.
                        let mut state = (i as u64 + 1).wrapping_mul(0x9e37_79b9_7f4a_7c15);
                        (0..self.filler / 16 + 1)
                            .map(|_| {
                                state ^= state << 13;
                                state ^= state >> 7;
                                state ^= state << 17;
                                format!("{state:016x}")
                            })
                            .collect()
                    } else {
                        filler.clone()
                    }
                });
            let sep = if i == 0 { "" } else { "," };
            Bytes::from(format!("{sep}{}", element(i, &text)))
        }))
        .chain(std::iter::once(Bytes::from_static(br#"],"stream":true}"#)))
    }

    fn bytes(&self) -> Vec<u8> {
        self.pieces().flat_map(|b| b.to_vec()).collect()
    }
}

/// Every spelling of `secret` the upstream must never see.
fn spellings(secret: &str) -> Vec<String> {
    use base64::Engine;
    let escaped = serde_json::to_string(secret).unwrap();
    let nested = serde_json::to_string(&escaped).unwrap();
    vec![
        secret.to_owned(),
        escaped[1..escaped.len() - 1].to_owned(),
        nested[1..nested.len() - 1].to_owned(),
        base64::engine::general_purpose::STANDARD.encode(secret),
        percent_encode(secret),
    ]
}

fn percent_encode(text: &str) -> String {
    text.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

async fn send_split(
    proxy: &Proxy,
    up_port: u16,
    bytes: Vec<u8>,
    split: usize,
    extra_headers: &[(&str, &str)],
) -> (u16, Vec<u8>) {
    let tls = connect(proxy, up_port).await;
    let (mut sender, conn) = hyper::client::conn::http1::handshake(TokioIo::new(tls))
        .await
        .unwrap();
    tokio::spawn(async move {
        let _ = conn.await;
    });
    let (mut tx, body) = channel_body();
    let mut request = Request::builder()
        .method("POST")
        .uri("/v1/responses")
        .header("host", format!("{HOST}:{up_port}"))
        .header("content-type", "application/json");
    for (name, value) in extra_headers {
        request = request.header(*name, *value);
    }
    let request = request.body(body).unwrap();
    let feeder = tokio::spawn(async move {
        use futures::SinkExt;
        for piece in bytes.chunks(split.max(1)) {
            if tx.send(Bytes::copy_from_slice(piece)).await.is_err() {
                break;
            }
        }
    });
    let response = tokio::time::timeout(WAIT, sender.send_request(request))
        .await
        .expect("response in time")
        .unwrap();
    let status = response.status().as_u16();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    feeder.abort();
    (status, body.to_vec())
}

async fn settled(store: &MonitorEventStore) -> Vec<Value> {
    tokio::time::timeout(WAIT, async {
        loop {
            let listed = store.list(0, 64, None);
            if listed.total >= 1
                && listed
                    .events
                    .iter()
                    .all(|event| event.status != EventStatus::Pending)
            {
                return listed
                    .events
                    .iter()
                    .map(|s| serde_json::to_value(store.get(&s.id).unwrap()).unwrap())
                    .collect();
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("monitor events settle")
}

/// A secret first detectable far into the body, with bare occurrences
/// before (within the lag) and after it, split across every TLS record and
/// read boundary: the upstream never sees it in any spelling.
#[tokio::test]
async fn a_streamed_body_never_forwards_a_detected_secret() {
    let secret = format!("{}{}", "Zq81RkLmPw0Xy7Tb", "Hs52");
    let conversation = Conversation {
        model: "gpt-test",
        elements: 600,
        filler: 900,
        noise: false,
        marks: vec![
            (395, format!("earlier bare {secret} occurrence")),
            (400, format!("token={secret}")),
            (401, format!("{secret}{secret}")),
            (599, format!("late \"{secret}\" again")),
        ],
    };
    let bytes = conversation.bytes();
    assert!(bytes.len() > 2 * LAG);
    let up_ca = make_ca();
    let (up_port, seen, _) =
        spawn_upstream(&up_ca, Answer::AfterBody, usize::MAX, spellings(&secret)).await;
    let proxy = start_proxy(&up_ca, LAG).await;
    for split in [7, 1500, 16 * 1024 + 3] {
        let (status, _) = send_split(&proxy, up_port, bytes.clone(), split, &[]).await;
        assert_eq!(status, 200, "split {split}");
    }
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 3);
    for received in seen.iter() {
        assert!(received.complete);
        assert_eq!(received.leaked, None);
        let headers = received.headers.as_ref().unwrap();
        assert_eq!(headers["transfer-encoding"], "chunked");
        assert!(headers.get("content-length").is_none());
        let forwarded: Value = serde_json::from_slice(&received.body).unwrap();
        let mut expected: Value = serde_json::from_slice(&bytes).unwrap();
        mask_value(&mut expected["input"], std::slice::from_ref(&secret));
        assert_eq!(forwarded, expected);
    }
    assert_eq!(*proxy.opened.lock().unwrap(), 3);
}

/// The upstream holds the first bytes while the client is still uploading.
#[tokio::test]
async fn a_large_inspected_body_reaches_the_upstream_before_the_client_finishes() {
    let conversation = Conversation {
        model: "gpt-test",
        elements: 3000,
        filler: 1000,
        noise: false,
        marks: vec![],
    };
    let bytes = conversation.bytes();
    let up_ca = make_ca();
    let (up_port, seen, mut first) = spawn_upstream(&up_ca, Answer::AfterBody, 0, vec![]).await;
    let proxy = start_proxy(&up_ca, LAG).await;
    let tls = connect(&proxy, up_port).await;
    let (mut sender, conn) = hyper::client::conn::http1::handshake(TokioIo::new(tls))
        .await
        .unwrap();
    tokio::spawn(async move {
        let _ = conn.await;
    });
    let (mut tx, body) = channel_body();
    let request = Request::builder()
        .method("POST")
        .uri("/v1/responses")
        .header("host", format!("{HOST}:{up_port}"))
        .body(body)
        .unwrap();
    let response = tokio::spawn(async move { sender.send_request(request).await });
    use futures::SinkExt;
    let half = bytes.len() / 2;
    for piece in bytes[..half].chunks(64 * 1024) {
        tx.send(Bytes::copy_from_slice(piece)).await.unwrap();
    }
    // Half sent and the client waits: the upstream already has data.
    tokio::time::timeout(WAIT, first.recv())
        .await
        .expect("upstream received bytes before the upload finished")
        .unwrap();
    for piece in bytes[half..].chunks(64 * 1024) {
        tx.send(Bytes::copy_from_slice(piece)).await.unwrap();
    }
    drop(tx);
    let response = tokio::time::timeout(WAIT, response)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(response.status(), 200);
    let seen = seen.lock().unwrap();
    assert!(seen[0].complete);
    assert_eq!(seen[0].len, bytes.len());
    assert!(seen[0].first_at.unwrap() < half);
}

/// A body within the lag is judged whole by `evaluate`, exactly as before.
#[tokio::test]
async fn a_body_within_the_lag_takes_the_buffered_path() {
    let conversation = Conversation {
        model: "gpt-test",
        elements: 20,
        filler: 100,
        noise: false,
        marks: vec![(3, "token=AbCdEfGh12345678".into())],
    };
    let bytes = conversation.bytes();
    assert!(bytes.len() < LAG);
    let up_ca = make_ca();
    let (up_port, seen, _) = spawn_upstream(&up_ca, Answer::AfterBody, usize::MAX, vec![]).await;
    let proxy = start_proxy(&up_ca, LAG).await;
    let (status, _) = send_split(&proxy, up_port, bytes, 100, &[]).await;
    assert_eq!(status, 200);
    assert_eq!(*proxy.opened.lock().unwrap(), 0);
    let seen = seen.lock().unwrap();
    let headers = seen[0].headers.as_ref().unwrap();
    assert!(headers.get("content-length").is_some());
    assert!(!String::from_utf8_lossy(&seen[0].body).contains("AbCdEfGh12345678"));
}

/// A zstd body is decoded while it streams, masked, and forwarded re-encoded
/// as zstd (chunked); a stream that turns out corrupt after release resets
/// the upstream.
#[tokio::test]
async fn compressed_bodies_are_decoded_while_streaming_and_corruption_aborts() {
    let conversation = Conversation {
        model: "gpt-test",
        elements: 1500,
        filler: 500,
        noise: true,
        marks: vec![(700, "token=Kk11Ll22Mm33Nn44".into())],
    };
    let bytes = conversation.bytes();
    let compressed = zstd::encode_all(&bytes[..], 3).unwrap();
    let up_ca = make_ca();
    let (up_port, seen, _) = spawn_upstream(
        &up_ca,
        Answer::AfterBody,
        usize::MAX,
        spellings("Kk11Ll22Mm33Nn44"),
    )
    .await;
    let proxy = start_proxy(&up_ca, LAG).await;
    let (status, _) = send_split(
        &proxy,
        up_port,
        compressed.clone(),
        4096,
        &[("content-encoding", "zstd")],
    )
    .await;
    assert_eq!(status, 200);
    {
        let seen = seen.lock().unwrap();
        assert!(seen[0].complete);
        assert_eq!(seen[0].leaked, None);
        let headers = seen[0].headers.as_ref().unwrap();
        assert_eq!(headers["content-encoding"], "zstd");
        assert!(headers.get("content-length").is_none());
        assert_eq!(headers["transfer-encoding"], "chunked");
        let decoded = zstd::decode_all(&seen[0].body[..]).unwrap();
        assert!(!String::from_utf8_lossy(&decoded).contains("Kk11Ll22Mm33Nn44"));
        let forwarded: Value = serde_json::from_slice(&decoded).unwrap();
        let mut expected: Value = serde_json::from_slice(&bytes).unwrap();
        mask_value(&mut expected["input"], &["Kk11Ll22Mm33Nn44".to_owned()]);
        assert_eq!(forwarded, expected);
        assert!(seen[0].len < decoded.len(), "the upload stays compressed");
    }

    // Corrupt the compressed stream well after the lag: the upload is ended
    // without a complete body and the client is told why.
    // A valid frame of the first three quarters, then bytes that are not a
    // zstd frame at all.
    let mut corrupt = zstd::encode_all(&bytes[..bytes.len() * 3 / 4], 3).unwrap();
    corrupt.extend_from_slice(&[0xde, 0xad, 0xbe, 0xef, 0x01, 0x02, 0x03, 0x04]);
    let (status, body) = send_split(
        &proxy,
        up_port,
        corrupt,
        4096,
        &[("content-encoding", "zstd")],
    )
    .await;
    assert!(matches!(status, 400 | 502), "{status}");
    let _ = body;
    // Truncated: the stream ends inside a frame.
    let truncated = compressed[..compressed.len() - 100].to_vec();
    let (status, _) = send_split(
        &proxy,
        up_port,
        truncated,
        4096,
        &[("content-encoding", "zstd")],
    )
    .await;
    assert!(matches!(status, 400 | 502), "{status}");
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 3);
    assert!(!seen[1].complete, "a corrupt upload must not complete");
    assert!(!seen[2].complete, "a truncated upload must not complete");
    assert!(
        seen[1].len > 0 && seen[2].len > 0,
        "both had started streaming"
    );
}

/// The upstream answers 401 before reading the body: the client gets it.
#[tokio::test]
async fn an_early_upstream_answer_reaches_the_client() {
    let conversation = Conversation {
        model: "gpt-test",
        elements: 2000,
        filler: 500,
        noise: false,
        marks: vec![],
    };
    let up_ca = make_ca();
    let (up_port, _, _) = spawn_upstream(&up_ca, Answer::Early401, 0, vec![]).await;
    let proxy = start_proxy(&up_ca, LAG).await;
    let (status, body) = send_split(&proxy, up_port, conversation.bytes(), 8192, &[]).await;
    assert_eq!(status, 401);
    assert_eq!(body, b"{\"error\":\"auth\"}");
}

/// The upstream drops the connection mid-upload: the client gets a 502 and
/// the monitor records the failure.
#[tokio::test]
async fn an_upstream_abort_mid_upload_fails_the_request() {
    let conversation = Conversation {
        model: "gpt-test",
        elements: 2000,
        filler: 500,
        noise: false,
        marks: vec![],
    };
    let up_ca = make_ca();
    let (up_port, _, _) = spawn_upstream(&up_ca, Answer::DropAfter(64 * 1024), 0, vec![]).await;
    let proxy = start_proxy(&up_ca, LAG).await;
    let (status, _) = send_split(&proxy, up_port, conversation.bytes(), 8192, &[]).await;
    assert_eq!(status, 502);
    let events = settled(&proxy.store).await;
    assert!(events
        .iter()
        .any(|e| e["status"] == "error" || e["status"] == "Error"));
}

/// The client goes away mid-upload: the upstream never gets a complete
/// body and the pending event is closed.
#[tokio::test]
async fn a_client_abort_resets_the_upstream_and_closes_the_pending_event() {
    let conversation = Conversation {
        model: "gpt-test",
        elements: 3000,
        filler: 600,
        noise: false,
        marks: vec![],
    };
    let bytes = conversation.bytes();
    let up_ca = make_ca();
    let (up_port, seen, mut first) = spawn_upstream(&up_ca, Answer::AfterBody, 0, vec![]).await;
    let proxy = start_proxy(&up_ca, LAG).await;
    let tls = connect(&proxy, up_port).await;
    let (mut sender, conn) = hyper::client::conn::http1::handshake(TokioIo::new(tls))
        .await
        .unwrap();
    let conn = tokio::spawn(async move {
        let _ = conn.await;
    });
    let (mut tx, body) = channel_body();
    let request = Request::builder()
        .method("POST")
        .uri("/v1/responses")
        .header("host", format!("{HOST}:{up_port}"))
        .body(body)
        .unwrap();
    let response = tokio::spawn(async move { sender.send_request(request).await });
    use futures::SinkExt;
    for piece in bytes[..bytes.len() / 2].chunks(64 * 1024) {
        tx.send(Bytes::copy_from_slice(piece)).await.unwrap();
    }
    tokio::time::timeout(WAIT, first.recv())
        .await
        .unwrap()
        .unwrap();
    // The client disappears.
    conn.abort();
    response.abort();
    drop(tx);
    tokio::time::timeout(WAIT, async {
        loop {
            {
                let seen = seen.lock().unwrap();
                if seen.len() == 1 && seen[0].len > 0 {
                    // Give the abort time to propagate.
                }
            }
            let events = proxy.store.list(0, 8, None);
            if events.total == 1 && events.events[0].status != EventStatus::Pending {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("pending event closed");
    tokio::time::sleep(Duration::from_millis(200)).await;
    let seen = seen.lock().unwrap();
    assert!(
        !seen[0].complete,
        "an abandoned upload must not complete upstream"
    );
    assert!(seen[0].len < bytes.len());
}

/// A refusal decided once the model is known: nothing is forwarded.
#[tokio::test]
async fn a_refusal_at_open_sends_nothing_upstream() {
    let conversation = Conversation {
        model: "refused",
        elements: 2000,
        filler: 500,
        noise: false,
        marks: vec![],
    };
    let up_ca = make_ca();
    let (up_port, seen, _) = spawn_upstream(&up_ca, Answer::AfterBody, 0, vec![]).await;
    let proxy = start_proxy(&up_ca, LAG).await;
    let (status, body) = send_split(&proxy, up_port, conversation.bytes(), 8192, &[]).await;
    assert_eq!(status, 403);
    assert!(String::from_utf8_lossy(&body).contains("model refused"));
    assert!(seen.lock().unwrap().is_empty());
}

/// Above the old 256 MiB buffered limit, with masking: forwarded whole,
/// nothing detected leaks, memory stays bounded by the pipeline.
#[tokio::test]
async fn a_body_above_the_buffered_limit_is_streamed_with_redaction() {
    let secret = "Rt55Yu66Io77Pa88";
    let elements = 300 * MIB / (64 * 1024) + 16;
    let conversation = Conversation {
        model: "gpt-test",
        elements,
        filler: 64 * 1024,
        noise: false,
        marks: vec![
            (10, format!("bare {secret}")),
            (12, format!("token={secret}")),
            (elements - 1, format!("end {secret}")),
        ],
    };
    let up_ca = make_ca();
    let (up_port, seen, _) = spawn_upstream(&up_ca, Answer::AfterBody, 0, spellings(secret)).await;
    let proxy = start_proxy(&up_ca, 16 * MIB).await;
    let tls = connect(&proxy, up_port).await;
    let (mut sender, conn) = hyper::client::conn::http1::handshake(TokioIo::new(tls))
        .await
        .unwrap();
    tokio::spawn(async move {
        let _ = conn.await;
    });
    let (mut tx, body) = channel_body();
    let request = Request::builder()
        .method("POST")
        .uri("/v1/responses")
        .header("host", format!("{HOST}:{up_port}"))
        .body(body)
        .unwrap();
    let response = tokio::spawn(async move { sender.send_request(request).await });
    use futures::SinkExt;
    let mut total = 0usize;
    for piece in conversation.pieces() {
        total += piece.len();
        tx.send(piece).await.unwrap();
    }
    drop(tx);
    assert!(total > 256 * MIB);
    let response = tokio::time::timeout(Duration::from_secs(600), response)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(response.status(), 200);
    let seen = seen.lock().unwrap();
    assert!(seen[0].complete);
    assert_eq!(seen[0].leaked, None);
    assert!(seen[0].len > 256 * MIB);
}

/// Send `bytes` with a Content-Length (one exact-size body) to `path`.
async fn send_sized(
    proxy: &Proxy,
    up_port: u16,
    path: &str,
    bytes: Vec<u8>,
    extra_headers: &[(&str, &str)],
) -> (u16, Vec<u8>) {
    let tls = connect(proxy, up_port).await;
    let (mut sender, conn) = hyper::client::conn::http1::handshake(TokioIo::new(tls))
        .await
        .unwrap();
    tokio::spawn(async move {
        let _ = conn.await;
    });
    let mut request = Request::builder()
        .method("POST")
        .uri(path)
        .header("host", format!("{HOST}:{up_port}"))
        .header("content-type", "application/json");
    for (name, value) in extra_headers {
        request = request.header(*name, *value);
    }
    let request = request.body(Full::new(Bytes::from(bytes))).unwrap();
    let response = tokio::time::timeout(WAIT, sender.send_request(request))
        .await
        .expect("response in time")
        .expect("a response, not a reset");
    let status = response.status().as_u16();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    (status, body.to_vec())
}

fn is_denial(body: &[u8]) -> bool {
    serde_json::from_slice::<Value>(body)
        .ok()
        .is_some_and(|v| v["error"]["code"] == "secret_scan_denied_by_user")
}

/// Secret Scan Ask: a denial after part of the body was released ends the
/// upload without completing it upstream, and the client is told why. The
/// denied value never reaches the upstream.
#[tokio::test]
async fn an_ask_denial_mid_stream_never_completes_the_upstream_request() {
    let secret = "Dd44Ff55Gg66Hh77";
    let conversation = Conversation {
        model: "gpt-test",
        elements: 2000,
        filler: 500,
        noise: false,
        marks: vec![(1500, format!("token={secret}"))],
    };
    let bytes = conversation.bytes();
    let up_ca = make_ca();
    let (up_port, seen, _) = spawn_upstream(&up_ca, Answer::AfterBody, 0, spellings(secret)).await;
    let proxy = start_proxy_with(&up_ca, LAG, Mode::Ask { deny: true }, &[]).await;
    let (status, body) = send_split(&proxy, up_port, bytes, 8192, &[]).await;
    assert_eq!(status, 403);
    assert!(is_denial(&body), "{}", String::from_utf8_lossy(&body));
    assert_eq!(*proxy.asked.lock().unwrap(), 1);
    tokio::time::sleep(Duration::from_millis(200)).await;
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 1);
    assert!(seen[0].len > 0, "part of the body was released");
    assert!(!seen[0].complete, "a denied upload must not complete");
    assert_eq!(seen[0].leaked, None);
}

/// A denial before anything was released never contacts the upstream.
#[tokio::test]
async fn an_ask_denial_before_any_release_sends_nothing() {
    let conversation = Conversation {
        model: "gpt-test",
        elements: 2000,
        filler: 500,
        noise: false,
        marks: vec![(3, "token=Jj88Kk99Ll00Mm11".into())],
    };
    let up_ca = make_ca();
    let (up_port, seen, _) = spawn_upstream(&up_ca, Answer::AfterBody, 0, vec![]).await;
    let proxy = start_proxy_with(&up_ca, LAG, Mode::Ask { deny: true }, &[]).await;
    let (status, body) = send_split(&proxy, up_port, conversation.bytes(), 8192, &[]).await;
    assert_eq!(status, 403);
    assert!(is_denial(&body));
    assert!(
        seen.lock().unwrap().is_empty(),
        "the upstream was contacted"
    );
}

/// Secret Scan Ask answered with masking: the upload keeps arriving (and is
/// held) while each question is open, and every occurrence within the lag,
/// earlier ones included, is masked.
#[tokio::test]
async fn ask_answers_mask_the_held_content() {
    let first = "Nn22Bb33Vv44Cc55";
    let second = "Xx66Zz77Aa88Ss99";
    let conversation = Conversation {
        model: "gpt-test",
        elements: 2000,
        filler: 500,
        noise: false,
        marks: vec![
            (1000, format!("bare {first}")),
            (1002, format!("token={first}")),
            (1900, format!("token={second} and {first}")),
        ],
    };
    let bytes = conversation.bytes();
    let mut forbidden = spellings(first);
    forbidden.extend(spellings(second));
    let up_ca = make_ca();
    let (up_port, seen, _) = spawn_upstream(&up_ca, Answer::AfterBody, usize::MAX, forbidden).await;
    let proxy = start_proxy_with(&up_ca, LAG, Mode::Ask { deny: false }, &[]).await;
    let (status, _) = send_split(&proxy, up_port, bytes.clone(), 8192, &[]).await;
    assert_eq!(status, 200);
    assert_eq!(*proxy.asked.lock().unwrap(), 2);
    let seen = seen.lock().unwrap();
    assert!(seen[0].complete);
    assert_eq!(seen[0].leaked, None);
    let forwarded: Value = serde_json::from_slice(&seen[0].body).unwrap();
    let mut expected: Value = serde_json::from_slice(&bytes).unwrap();
    mask_value(
        &mut expected["input"],
        &[first.to_owned(), second.to_owned()],
    );
    assert_eq!(forwarded, expected);
}

/// Strict mode: nothing reaches the upstream before the client finished.
#[tokio::test]
async fn strict_mode_sends_nothing_before_the_body_ended() {
    let conversation = Conversation {
        model: "gpt-test",
        elements: 2000,
        filler: 500,
        noise: false,
        marks: vec![(1999, "token=Qa12Ws34Ed56Rf78".into())],
    };
    let bytes = conversation.bytes();
    let up_ca = make_ca();
    let (up_port, seen, mut first) = spawn_upstream(
        &up_ca,
        Answer::AfterBody,
        usize::MAX,
        spellings("Qa12Ws34Ed56Rf78"),
    )
    .await;
    let proxy = start_proxy_with(&up_ca, LAG, Mode::Strict, &[]).await;
    let tls = connect(&proxy, up_port).await;
    let (mut sender, conn) = hyper::client::conn::http1::handshake(TokioIo::new(tls))
        .await
        .unwrap();
    tokio::spawn(async move {
        let _ = conn.await;
    });
    let (mut tx, body) = channel_body();
    let request = Request::builder()
        .method("POST")
        .uri("/v1/responses")
        .header("host", format!("{HOST}:{up_port}"))
        .body(body)
        .unwrap();
    let response = tokio::spawn(async move { sender.send_request(request).await });
    use futures::SinkExt;
    let cut = bytes.len() - 100;
    for piece in bytes[..cut].chunks(64 * 1024) {
        tx.send(Bytes::copy_from_slice(piece)).await.unwrap();
    }
    // Everything but the end is there: still nothing upstream.
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(first.try_recv().is_err(), "strict mode released early");
    assert!(seen.lock().unwrap().is_empty());
    tx.send(Bytes::copy_from_slice(&bytes[cut..]))
        .await
        .unwrap();
    drop(tx);
    let response = tokio::time::timeout(WAIT, response)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(response.status(), 200);
    let seen = seen.lock().unwrap();
    assert!(seen[0].complete);
    assert_eq!(seen[0].leaked, None);
}

/// Off/Notify tee: the upstream receives the client's exact bytes (spacing
/// included) with its Content-Length and Content-Encoding.
#[tokio::test]
async fn a_tee_forwards_the_original_bytes_with_their_length_and_encoding() {
    let conversation = Conversation {
        model: "gpt-test",
        elements: 1500,
        filler: 500,
        noise: true,
        marks: vec![(700, "token=Ty12Gh34Bn56Mj78".into())],
    };
    let value: Value = serde_json::from_slice(&conversation.bytes()).unwrap();
    // Spacing the pipeline would normalize when it re-assembles units.
    let pretty = serde_json::to_vec_pretty(&value).unwrap();
    let compressed = zstd::encode_all(&pretty[..], 3).unwrap();
    let up_ca = make_ca();
    let (up_port, seen, _) = spawn_upstream(&up_ca, Answer::AfterBody, usize::MAX, vec![]).await;
    let proxy = start_proxy_with(&up_ca, LAG, Mode::Tee, &[]).await;
    let (status, _) = send_sized(&proxy, up_port, "/v1/responses", pretty.clone(), &[]).await;
    assert_eq!(status, 200);
    let (status, _) = send_sized(
        &proxy,
        up_port,
        "/v1/responses",
        compressed.clone(),
        &[("content-encoding", "zstd")],
    )
    .await;
    assert_eq!(status, 200);
    let seen = seen.lock().unwrap();
    assert_eq!(*proxy.opened.lock().unwrap(), 2);
    for (received, sent, encoding) in [
        (&seen[0], &pretty, None),
        (&seen[1], &compressed, Some("zstd")),
    ] {
        assert!(received.complete);
        let headers = received.headers.as_ref().unwrap();
        assert_eq!(
            headers["content-length"].to_str().unwrap(),
            sent.len().to_string()
        );
        assert!(headers.get("transfer-encoding").is_none());
        assert_eq!(
            headers.get("content-encoding").map(|v| v.to_str().unwrap()),
            encoding
        );
        assert!(received.body == *sent, "the bytes were changed");
    }
}

/// A configured host gets the transformed body held and sent with a
/// Content-Length.
#[tokio::test]
async fn length_required_hosts_get_the_held_body_with_a_content_length() {
    let secret = "Lk09Jh87Gf65Ds43";
    let conversation = Conversation {
        model: "gpt-test",
        elements: 2000,
        filler: 500,
        noise: false,
        marks: vec![
            (10, format!("token={secret}")),
            (1990, format!("again {secret}")),
        ],
    };
    let bytes = conversation.bytes();
    let up_ca = make_ca();
    let (up_port, seen, _) =
        spawn_upstream(&up_ca, Answer::AfterBody, usize::MAX, spellings(secret)).await;
    let proxy = start_proxy_with(&up_ca, LAG, Mode::Redact, &[HOST]).await;
    let (status, _) = send_split(&proxy, up_port, bytes.clone(), 8192, &[]).await;
    assert_eq!(status, 200);
    let seen = seen.lock().unwrap();
    assert!(seen[0].complete);
    assert_eq!(seen[0].leaked, None);
    let headers = seen[0].headers.as_ref().unwrap();
    assert!(headers.get("transfer-encoding").is_none());
    assert_eq!(
        headers["content-length"].to_str().unwrap(),
        seen[0].len.to_string()
    );
    let forwarded: Value = serde_json::from_slice(&seen[0].body).unwrap();
    let mut expected: Value = serde_json::from_slice(&bytes).unwrap();
    mask_value(&mut expected["input"], &[secret.to_owned()]);
    assert_eq!(forwarded, expected);
}

/// A length-required host gets a zstd body re-encoded and held, with the
/// Content-Length of the compressed bytes; an identity body stays identity.
#[tokio::test]
async fn a_held_zstd_body_is_re_encoded_with_its_compressed_length() {
    let secret = "Zs12Td34Ue56Vf78";
    let conversation = Conversation {
        model: "gpt-test",
        elements: 2000,
        filler: 500,
        noise: false,
        marks: vec![
            (10, format!("token={secret}")),
            (1990, format!("again {secret}")),
        ],
    };
    let bytes = conversation.bytes();
    let compressed = zstd::encode_all(&bytes[..], 3).unwrap();
    let up_ca = make_ca();
    let (up_port, seen, _) =
        spawn_upstream(&up_ca, Answer::AfterBody, usize::MAX, spellings(secret)).await;
    let proxy = start_proxy_with(&up_ca, LAG, Mode::Redact, &[HOST]).await;
    let (status, _) = send_split(
        &proxy,
        up_port,
        compressed,
        8192,
        &[("content-encoding", "zstd")],
    )
    .await;
    assert_eq!(status, 200);
    let seen = seen.lock().unwrap();
    assert!(seen[0].complete);
    let headers = seen[0].headers.as_ref().unwrap();
    assert!(headers.get("transfer-encoding").is_none());
    assert_eq!(headers["content-encoding"], "zstd");
    assert_eq!(
        headers["content-length"].to_str().unwrap(),
        seen[0].len.to_string()
    );
    let decoded = zstd::decode_all(&seen[0].body[..]).unwrap();
    assert!(!String::from_utf8_lossy(&decoded).contains(secret));
    let forwarded: Value = serde_json::from_slice(&decoded).unwrap();
    let mut expected: Value = serde_json::from_slice(&bytes).unwrap();
    mask_value(&mut expected["input"], &[secret.to_owned()]);
    assert_eq!(forwarded, expected);
}

/// An upstream that refuses a chunked body with 411: that request fails
/// (it cannot be replayed), the host is learned, and the next request to it
/// is sent with a Content-Length.
#[tokio::test]
async fn a_host_that_refuses_chunked_bodies_is_learned() {
    let conversation = Conversation {
        model: "gpt-test",
        elements: 2000,
        filler: 500,
        noise: false,
        marks: vec![],
    };
    let bytes = conversation.bytes();
    let up_ca = make_ca();
    let (up_port, seen, _) =
        spawn_upstream(&up_ca, Answer::RejectChunked, usize::MAX, vec![]).await;
    let proxy = start_proxy_with(&up_ca, LAG, Mode::Redact, &[]).await;
    assert!(!proxy.length_required.requires(HOST));
    let (status, _) = send_split(&proxy, up_port, bytes.clone(), 8192, &[]).await;
    assert_eq!(status, 411);
    assert!(proxy.length_required.requires(HOST));
    let (status, _) = send_split(&proxy, up_port, bytes.clone(), 8192, &[]).await;
    assert_eq!(status, 200);
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 2);
    let headers = seen[1].headers.as_ref().unwrap();
    assert!(headers.get("transfer-encoding").is_none());
    assert_eq!(
        headers["content-length"].to_str().unwrap(),
        bytes.len().to_string()
    );
    assert!(seen[1].complete);
    assert_eq!(seen[1].body, bytes);
}

/// A body whose `model` comes last is held (here mostly on disk) until the
/// model is known; a refused model then sends nothing at all, an allowed
/// one forwards the whole body.
#[tokio::test]
async fn a_late_model_is_held_on_disk_until_it_is_known() {
    let secret = "Mo12Ni34Lu56Bv78";
    let conversation = Conversation {
        model: "unused",
        elements: 6000,
        filler: 900,
        noise: true,
        marks: vec![(5, format!("token={secret}"))],
    };
    let value: Value = serde_json::from_slice(&conversation.bytes()).unwrap();
    let body_for = |model: &str| -> Vec<u8> {
        format!(
            r#"{{"instructions":"Be brief.","input":{},"stream":true,"model":"{model}"}}"#,
            value["input"]
        )
        .into_bytes()
    };
    let up_ca = make_ca();
    let (up_port, seen, _) =
        spawn_upstream(&up_ca, Answer::AfterBody, usize::MAX, spellings(secret)).await;
    let proxy = start_proxy_with(&up_ca, LAG, Mode::Redact, &[]).await;
    let refused = body_for("refused");
    assert!(refused.len() > 4 * MIB, "larger than the RAM limit");
    let (status, body) = send_split(&proxy, up_port, refused, 16 * 1024, &[]).await;
    assert_eq!(status, 403);
    assert!(String::from_utf8_lossy(&body).contains("model refused"));
    assert!(seen.lock().unwrap().is_empty(), "nothing may be sent");

    let allowed = body_for("gpt-test");
    let (status, _) = send_split(&proxy, up_port, allowed.clone(), 16 * 1024, &[]).await;
    assert_eq!(status, 200);
    {
        let seen = seen.lock().unwrap();
        assert!(seen[0].complete);
        assert_eq!(seen[0].leaked, None);
        let forwarded: Value = serde_json::from_slice(&seen[0].body).unwrap();
        let mut expected: Value = serde_json::from_slice(&allowed).unwrap();
        mask_value(&mut expected["input"], &[secret.to_owned()]);
        assert_eq!(forwarded, expected);
    }
    // Held bytes went to disk and were released with the requests; the
    // spill files are anonymous.
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(proxy.spill.in_flight(), 0);
    let files = std::fs::read_dir(proxy.spill.dir())
        .map(|entries| entries.count())
        .unwrap_or(0);
    assert_eq!(files, 0);
}

/// A pass-through upload (a path nothing inspects) whose upstream cannot be
/// reached: the rest of the upload is drained so a client that writes its
/// whole body before reading reads the 502 instead of a reset.
#[tokio::test]
async fn a_pass_through_upload_reads_the_error_when_the_upstream_is_unreachable() {
    let closed = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let up_port = closed.local_addr().unwrap().port();
    drop(closed);
    let up_ca = make_ca();
    let proxy = start_proxy(&up_ca, LAG).await;
    let mut tls = connect(&proxy, up_port).await;
    let len = 32 * MIB;
    tls.write_all(
        format!(
            "POST /upload/blob HTTP/1.1\r\nHost: {HOST}:{up_port}\r\nContent-Length: {len}\r\n\r\n"
        )
        .as_bytes(),
    )
    .await
    .unwrap();
    let block = vec![b'u'; 64 * 1024];
    for _ in 0..len / block.len() {
        tls.write_all(&block)
            .await
            .expect("the upload is read to its end");
    }
    tls.flush().await.unwrap();
    let (head, _) = tokio::time::timeout(WAIT, support::read_head(&mut tls))
        .await
        .expect("an answer in time");
    assert!(head.starts_with("http/1.1 502"), "{head}");
}

/// A GuardRails denial: whether the flagged text comes first or last, and
/// whether the session masks or forwards the original bytes, the upstream
/// is never contacted (every byte it receives is recorded), and the client
/// and the monitor get the buffered path's refusal.
#[tokio::test]
async fn a_guardrails_denial_never_contacts_the_upstream() {
    for tee in [false, true] {
        for at in [0, 1999] {
            let conversation = Conversation {
                model: "gpt-test",
                elements: 2000,
                filler: 500,
                noise: false,
                marks: vec![(at, "something harmful".into())],
            };
            let up_ca = make_ca();
            let (up_port, seen, _) = spawn_upstream(&up_ca, Answer::AfterBody, 0, vec![]).await;
            let proxy = start_proxy_with(&up_ca, LAG, Mode::Guard { tee }, &[]).await;
            let (status, body) = send_split(&proxy, up_port, conversation.bytes(), 8192, &[]).await;
            assert_eq!(status, 403, "tee {tee}, at {at}");
            let body: Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(
                body["error"]["message"], "Request blocked by safety guardrails",
                "the buffered path's refusal"
            );
            assert_eq!(body["error"]["type"], "localrouter_firewall");
            assert_eq!(*proxy.asked.lock().unwrap(), 1);
            tokio::time::sleep(Duration::from_millis(200)).await;
            assert!(
                seen.lock().unwrap().is_empty(),
                "the upstream was contacted (tee {tee}, at {at})"
            );
            let events = settled(&proxy.store).await;
            let recorded = serde_json::to_string(&events).unwrap();
            assert!(
                recorded.contains("Request blocked by safety guardrails"),
                "{recorded}"
            );
        }
    }
}

/// A GuardRails Allow: the body (masked) goes upstream whole, with a
/// Content-Length, only after the client finished and the verdict was
/// given; a tee forwards the client's exact bytes.
#[tokio::test]
async fn a_guardrails_allow_forwards_the_masked_body_after_the_verdict() {
    let secret = "Gr12Ab34Cd56Ef78";
    let conversation = Conversation {
        model: "gpt-test",
        elements: 2000,
        filler: 500,
        noise: false,
        marks: vec![
            // Within the lag of the detection, so masked (masking keeps
            // its lag semantics while the output is held).
            (1985, format!("bare {secret}")),
            (1990, format!("token={secret}")),
        ],
    };
    let bytes = conversation.bytes();
    for tee in [false, true] {
        let up_ca = make_ca();
        let forbidden = if tee { vec![] } else { spellings(secret) };
        let (up_port, seen, mut first) =
            spawn_upstream(&up_ca, Answer::AfterBody, usize::MAX, forbidden).await;
        let proxy = start_proxy_with(&up_ca, LAG, Mode::Guard { tee }, &[]).await;
        let tls = connect(&proxy, up_port).await;
        let (mut sender, conn) = hyper::client::conn::http1::handshake(TokioIo::new(tls))
            .await
            .unwrap();
        tokio::spawn(async move {
            let _ = conn.await;
        });
        let (mut tx, body) = channel_body();
        let request = Request::builder()
            .method("POST")
            .uri("/v1/responses")
            .header("host", format!("{HOST}:{up_port}"))
            .body(body)
            .unwrap();
        let response = tokio::spawn(async move { sender.send_request(request).await });
        use futures::SinkExt;
        let cut = bytes.len() - 100;
        for piece in bytes[..cut].chunks(64 * 1024) {
            tx.send(Bytes::copy_from_slice(piece)).await.unwrap();
        }
        // Far beyond the lag, but the verdict is not given: nothing sent.
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert!(first.try_recv().is_err(), "released before the verdict");
        assert!(seen.lock().unwrap().is_empty());
        tx.send(Bytes::copy_from_slice(&bytes[cut..]))
            .await
            .unwrap();
        drop(tx);
        let response = tokio::time::timeout(WAIT, response)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(response.status(), 200);
        assert_eq!(*proxy.asked.lock().unwrap(), 1);
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert!(seen[0].complete);
        assert_eq!(seen[0].leaked, None);
        let headers = seen[0].headers.as_ref().unwrap();
        assert_eq!(
            headers["content-length"].to_str().unwrap(),
            seen[0].len.to_string()
        );
        let forwarded: Value = serde_json::from_slice(&seen[0].body).unwrap();
        let mut expected: Value = serde_json::from_slice(&bytes).unwrap();
        if tee {
            assert!(seen[0].body == bytes, "a tee forwards the exact bytes");
        } else {
            mask_value(&mut expected["input"], &[secret.to_owned()]);
        }
        assert_eq!(forwarded, expected);
    }
}

/// Above the old 256 MiB buffered limit with GuardRails on: the body is
/// held (encrypted, on disk past 1 MiB) until the verdict, then forwarded
/// whole; a denial of the same size sends nothing. The spill is released.
#[tokio::test]
async fn a_guarded_body_above_the_buffered_limit_is_held_until_the_verdict() {
    for harmful in [false, true] {
        let elements = 260 * MIB / (64 * 1024) + 16;
        let conversation = Conversation {
            model: "gpt-test",
            elements,
            filler: 64 * 1024,
            noise: false,
            marks: if harmful {
                vec![(elements - 1, "harmful".into())]
            } else {
                vec![]
            },
        };
        let up_ca = make_ca();
        let (up_port, seen, _) = spawn_upstream(&up_ca, Answer::AfterBody, 0, vec![]).await;
        let proxy = start_proxy_with(&up_ca, 16 * MIB, Mode::Guard { tee: false }, &[]).await;
        let tls = connect(&proxy, up_port).await;
        let (mut sender, conn) = hyper::client::conn::http1::handshake(TokioIo::new(tls))
            .await
            .unwrap();
        tokio::spawn(async move {
            let _ = conn.await;
        });
        let (mut tx, body) = channel_body();
        let request = Request::builder()
            .method("POST")
            .uri("/v1/responses")
            .header("host", format!("{HOST}:{up_port}"))
            .body(body)
            .unwrap();
        let response = tokio::spawn(async move { sender.send_request(request).await });
        use futures::SinkExt;
        let mut total = 0usize;
        for piece in conversation.pieces() {
            total += piece.len();
            tx.send(piece).await.unwrap();
        }
        drop(tx);
        assert!(total > 256 * MIB);
        let response = tokio::time::timeout(Duration::from_secs(600), response)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        {
            let seen = seen.lock().unwrap();
            if harmful {
                assert_eq!(response.status(), 403);
                assert!(seen.is_empty(), "the upstream was contacted");
            } else {
                assert_eq!(response.status(), 200);
                assert!(seen[0].complete);
                assert!(seen[0].len > 256 * MIB);
            }
        }
        // Every held byte was released.
        tokio::time::timeout(WAIT, async {
            while proxy.spill.in_flight() > 0 {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("the spill budget is released");
    }
}

/// A client that goes away while its request waits for the GuardRails
/// verdict ends the wait: nothing is sent and the held body is released
/// (rather than waiting for a popup nobody answers).
#[tokio::test]
async fn a_client_leaving_before_the_verdict_releases_the_held_body() {
    let conversation = Conversation {
        model: "gpt-test",
        elements: 400,
        filler: 64 * 1024,
        noise: false,
        marks: vec![(399, "unanswered".into())],
    };
    let up_ca = make_ca();
    let (up_port, seen, _) = spawn_upstream(&up_ca, Answer::AfterBody, 0, vec![]).await;
    let proxy = start_proxy_with(&up_ca, LAG, Mode::Guard { tee: false }, &[]).await;
    let tls = connect(&proxy, up_port).await;
    let (mut sender, conn) = hyper::client::conn::http1::handshake(TokioIo::new(tls))
        .await
        .unwrap();
    let connection = tokio::spawn(async move {
        let _ = conn.await;
    });
    let request = Request::builder()
        .method("POST")
        .uri("/v1/responses")
        .header("host", format!("{HOST}:{up_port}"))
        .body(Full::new(Bytes::from(conversation.bytes())))
        .unwrap();
    let response = tokio::spawn(async move { sender.send_request(request).await });
    // The whole body arrived and is held (more than the 1 MiB RAM limit).
    tokio::time::timeout(WAIT, async {
        while *proxy.asked.lock().unwrap() == 0 {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the verdict is asked");
    assert!(proxy.spill.in_flight() > 0);
    response.abort();
    connection.abort();
    tokio::time::timeout(Duration::from_secs(10), async {
        while proxy.spill.in_flight() > 0 {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the held body is released when the client leaves");
    assert!(seen.lock().unwrap().is_empty());
}

/// A secret first detectable far more than the lag after an earlier bare
/// occurrence. When the body is held until its end anyway (a GuardRails
/// verdict, a host that needs a Content-Length, strict mode) masking is
/// strict: the earlier occurrence never reaches the upstream in any
/// spelling. Without a hold the earlier occurrence was released before the
/// secret was known (the documented residual risk of streaming).
#[tokio::test]
async fn a_held_body_masks_occurrences_released_before_the_secret_was_detected() {
    let secret = "Ea71Rl09Oc23Cu45";
    let conversation = Conversation {
        model: "gpt-test",
        elements: 2000,
        filler: 500,
        noise: false,
        marks: vec![
            (3, format!("bare {secret} first")),
            (1990, format!("token={secret}")),
        ],
    };
    let bytes = conversation.bytes();
    assert!(bytes.len() > 4 * LAG, "the detection is far beyond the lag");
    let held = [
        (Mode::Guard { tee: false }, &[][..]),
        (Mode::Redact, &[HOST][..]),
        (Mode::Strict, &[][..]),
    ];
    for (mode, hosts) in held {
        let up_ca = make_ca();
        let (up_port, seen, _) =
            spawn_upstream(&up_ca, Answer::AfterBody, usize::MAX, spellings(secret)).await;
        let proxy = start_proxy_with(&up_ca, LAG, mode, hosts).await;
        let (status, _) = send_split(&proxy, up_port, bytes.clone(), 8192, &[]).await;
        assert_eq!(status, 200, "{mode:?}");
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 1, "{mode:?}");
        assert!(seen[0].complete, "{mode:?}");
        assert_eq!(seen[0].leaked, None, "{mode:?}");
        let forwarded: Value = serde_json::from_slice(&seen[0].body).unwrap();
        let mut expected: Value = serde_json::from_slice(&bytes).unwrap();
        mask_value(&mut expected["input"], &[secret.to_owned()]);
        assert_eq!(forwarded, expected, "{mode:?}");
    }
    // Control: streamed chunked, the earlier occurrence was already sent.
    let up_ca = make_ca();
    let (up_port, seen, _) =
        spawn_upstream(&up_ca, Answer::AfterBody, usize::MAX, vec![secret.into()]).await;
    let proxy = start_proxy_with(&up_ca, LAG, Mode::Redact, &[]).await;
    let (status, _) = send_split(&proxy, up_port, bytes.clone(), 8192, &[]).await;
    assert_eq!(status, 200);
    let seen = seen.lock().unwrap();
    assert_eq!(seen[0].leaked.as_deref(), Some(secret));
    let text = String::from_utf8_lossy(&seen[0].body);
    assert_eq!(
        text.matches(secret).count(),
        1,
        "only the earlier occurrence"
    );
}

/// A body within the lag that the firewall rewrites is forwarded re-encoded
/// as zstd when the client sent zstd alone, as a streamed body is, with the
/// Content-Length of the compressed bytes.
#[tokio::test]
async fn a_rewritten_buffered_zstd_body_is_re_encoded() {
    let secret = "Bz12Cy34Dx56Ew78";
    let conversation = Conversation {
        model: "gpt-test",
        elements: 20,
        filler: 100,
        noise: false,
        marks: vec![(3, format!("token={secret}"))],
    };
    let bytes = conversation.bytes();
    assert!(bytes.len() < LAG);
    let compressed = zstd::encode_all(&bytes[..], 3).unwrap();
    let up_ca = make_ca();
    let (up_port, seen, _) =
        spawn_upstream(&up_ca, Answer::AfterBody, usize::MAX, spellings(secret)).await;
    let proxy = start_proxy(&up_ca, LAG).await;
    let (status, _) = send_split(
        &proxy,
        up_port,
        compressed,
        100,
        &[("content-encoding", "zstd")],
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(*proxy.opened.lock().unwrap(), 0, "buffered");
    let received = std::mem::take(&mut seen.lock().unwrap()[0]);
    let seen = [received];
    assert!(seen[0].complete);
    let headers = seen[0].headers.as_ref().unwrap();
    assert_eq!(headers["content-encoding"], "zstd");
    assert_eq!(
        headers["content-length"].to_str().unwrap(),
        seen[0].len.to_string()
    );
    let decoded = zstd::decode_all(&seen[0].body[..]).unwrap();
    assert!(!String::from_utf8_lossy(&decoded).contains(secret));
    let forwarded: Value = serde_json::from_slice(&decoded).unwrap();
    let mut expected: Value = serde_json::from_slice(&bytes).unwrap();
    mask_value(&mut expected, &[secret.to_owned()]);
    assert_eq!(forwarded, expected);
    // The monitor shows the masked plain body.
    let events = settled(&proxy.store).await;
    let recorded = serde_json::to_string(&events).unwrap();
    assert!(recorded.contains("token=[MASKED]"), "{recorded}");
}
