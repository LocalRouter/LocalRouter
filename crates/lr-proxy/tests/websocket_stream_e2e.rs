//! A Codex-style websocket conversation through the MITM proxy over real
//! TLS (CONNECT, forged leaf, HTTP upgrade, masked client frames): a client
//! message far above the stream lag is streamed with its secret masked, a
//! refused message is answered with an error event while the socket stays
//! open, and a server message carrying a reference is restored for the
//! client.

mod support;

use std::convert::Infallible;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use http_body_util::Full;
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper::{Request, Response};
use hyper_util::rt::TokioIo;
use rustls::{ClientConfig, RootCertStore};
use rustls_pki_types::ServerName;
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_rustls::client::TlsStream;
use tokio_rustls::TlsConnector;

use lr_monitor::MonitorEventStore;
use lr_proxy::active::{ActiveInterceptor, Firewall, FirewallRequest, StreamRequest};
use lr_proxy::cert::CertAuthority;
use lr_proxy::interceptor::{
    BodyPolicy, ClientCtx, ConnectDecision, ObservedExchange, ProxyInterceptor, RequestAction,
    RequestHead, StreamMasking, StreamOpen, StreamPolicy,
};
use lr_proxy::passive::PassiveInterceptor;
use lr_proxy::request_stream::{StreamAbort, StreamSession, TailUnit, UnitScope};
use lr_proxy::resolver::StaticResolver;
use lr_proxy::websocket::{encode_frame, FrameDecoder, WsMessage};
use lr_proxy::{tls, ProxyManager};
use lr_secret_scanner::placeholders::Vault;
use lr_secret_scanner::request_stream::RequestMetadata;
use lr_types::secret_context::SecretContext;

use support::{make_ca, root_store, tls_listener, TestCa};

const HOST: &str = "localhost";
const CLIENT_ID: &str = "cid";
const SECRET: &str = "lr-secret";
const LAG: usize = 64 * 1024;
const WAIT: Duration = Duration::from_secs(60);
const VALUE: &str = "Ws12Xe34Rd56Tf78Gy90";

/// Registers every `token=<value>` with the vault and masks units with it,
/// so the upstream sees references the vault restores in server messages.
struct VaultMasker(Arc<Vault>);

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

impl StreamSession for VaultMasker {
    fn inspect(&mut self, _: &UnitScope, value: &Value) -> Result<Vec<String>, StreamAbort> {
        let mut found = Vec::new();
        detect(value, &mut found);
        for secret in &found {
            self.0
                .register(secret)
                .map_err(|_| StreamAbort::new(500, "register"))?;
        }
        Ok(found)
    }
    fn mask(&mut self, units: &mut [TailUnit]) -> Result<(), StreamAbort> {
        for unit in units {
            if unit.scope.key() != "model" {
                self.0
                    .protect(&mut unit.value)
                    .map_err(|_| StreamAbort::new(500, "protect"))?;
            }
        }
        Ok(())
    }
    fn diagnostic(&mut self, _: &str) {}
    fn finish(&mut self, _: &RequestMetadata) {}
}

/// Streams messages above the lag through a [`VaultMasker`]; refuses the
/// model `refused` when the stream would open; forwards small messages.
struct WsFirewall {
    opened: Arc<Mutex<usize>>,
}

#[async_trait]
impl Firewall for WsFirewall {
    fn body_policy(&self, _: &RequestHead<'_>) -> BodyPolicy {
        BodyPolicy::Stream(StreamPolicy {
            masking: StreamMasking::Redact,
            needs_model: false,
            whole_document: false,
            lag: LAG,
            restore_only: false,
            strict: false,
            incremental: false,
        })
    }
    async fn open_stream(&self, req: &StreamRequest, policy: &StreamPolicy) -> StreamOpen {
        assert!(policy.incremental, "a websocket releases as it goes");
        if req.model.as_deref() == Some("refused") {
            return StreamOpen::Reject(RequestAction::reject_json(403, "model refused"));
        }
        *self.opened.lock().unwrap() += 1;
        let vault = Arc::new(Vault::new(
            Arc::new(lr_api_keys::MockKeychain::new()),
            &req.client_id,
            "ws-e2e",
            true,
        ));
        StreamOpen::Stream {
            session: Box::new(VaultMasker(vault.clone())),
            vault: Some(vault),
            tee: false,
        }
    }
    async fn evaluate(&self, _: &FirewallRequest) -> RequestAction {
        RequestAction::Forward
    }
    fn protects_secrets(&self, _: &str) -> bool {
        true
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
}

/// A websocket upstream (Codex's Responses transport): records every client
/// message whole, and answers each with a text delta and
/// `response.completed` that echo the first reference it was sent (or
/// `pong`).
async fn spawn_upstream(ca: &TestCa) -> (u16, Arc<Mutex<Vec<String>>>) {
    let (listener, acceptor, port) = tls_listener(ca, HOST).await;
    let seen = Arc::new(Mutex::new(Vec::new()));
    let all = seen.clone();
    tokio::spawn(async move {
        while let Ok((tcp, _)) = listener.accept().await {
            let acceptor = acceptor.clone();
            let all = all.clone();
            tokio::spawn(async move {
                let Ok(tls) = acceptor.accept(tcp).await else {
                    return;
                };
                let service = service_fn(move |mut req: Request<Incoming>| {
                    let all = all.clone();
                    let on_upgrade = hyper::upgrade::on(&mut req);
                    tokio::spawn(async move {
                        let Ok(upgraded) = on_upgrade.await else {
                            return;
                        };
                        let mut io = TokioIo::new(upgraded);
                        let mut decoder = FrameDecoder::with_limit(64 * 1024 * 1024);
                        let mut buf = vec![0u8; 64 * 1024];
                        loop {
                            let Ok(n) = io.read(&mut buf).await else {
                                return;
                            };
                            if n == 0 {
                                return;
                            }
                            for message in decoder.push(&buf[..n]) {
                                let WsMessage::Text(text) = message else {
                                    continue;
                                };
                                all.lock().unwrap().push(text.clone());
                                let reference = text.find("<localrouter-redacted").map(|at| {
                                    let end = text[at..].find("/>").unwrap() + at + 2;
                                    // The reference as it appears inside a
                                    // JSON string.
                                    serde_json::from_str::<String>(&format!(
                                        "\"{}\"",
                                        &text[at..end]
                                    ))
                                    .unwrap()
                                });
                                let answer = reference.unwrap_or_else(|| "pong".into());
                                let delta = json!({"type":"response.output_text.delta",
                                    "item_id":"msg","content_index":0,"delta":answer});
                                let completed = json!({"type":"response.completed","response":{
                                    "id":"resp_ws","model":"gpt-test","status":"completed",
                                    "output":[{"type":"message","content":[{"type":"output_text","text":answer}]}],
                                    "usage":{"input_tokens":3,"output_tokens":4}}});
                                let mut frames =
                                    encode_frame(&WsMessage::Text(delta.to_string()), false);
                                frames.extend(encode_frame(
                                    &WsMessage::Text(completed.to_string()),
                                    false,
                                ));
                                if io.write_all(&frames).await.is_err() {
                                    return;
                                }
                            }
                        }
                    });
                    async move {
                        Ok::<_, Infallible>(
                            Response::builder()
                                .status(101)
                                .header("connection", "Upgrade")
                                .header("upgrade", "websocket")
                                .header("sec-websocket-accept", "test-accept")
                                .body(Full::new(Bytes::new()))
                                .unwrap(),
                        )
                    }
                });
                hyper::server::conn::http1::Builder::new()
                    .serve_connection(TokioIo::new(tls), service)
                    .with_upgrades()
                    .await
                    .ok();
            });
        }
    });
    (port, seen)
}

struct Proxy {
    port: u16,
    ca_pem: String,
    store: Arc<MonitorEventStore>,
    opened: Arc<Mutex<usize>>,
    dir: std::path::PathBuf,
    _stop: tokio::sync::oneshot::Sender<()>,
}

impl Drop for Proxy {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

async fn start_proxy(up_ca: &TestCa) -> Proxy {
    tls::ensure_crypto_provider();
    let dir = std::env::temp_dir().join(format!("lr-ws-stream-e2e-{}", uuid::Uuid::new_v4()));
    let ca = Arc::new(CertAuthority::load_or_create(&dir).unwrap());
    let ca_pem = ca.ca_pem().to_string();
    let store = Arc::new(MonitorEventStore::new(64));
    let opened = Arc::new(Mutex::new(0));
    let mut roots = RootCertStore::empty();
    roots.add(up_ca.ca_der.clone()).unwrap();
    let interceptor = Mitm(ActiveInterceptor::new(
        PassiveInterceptor::new(store.clone()),
        Arc::new(WsFirewall {
            opened: opened.clone(),
        }),
    ));
    let spill = Arc::new(lr_proxy::spill::SpillConfig::with_limits(
        dir.join("spill"),
        1024 * 1024,
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
    .with_spill(Some(spill));
    let listener = ProxyManager::bind("127.0.0.1", 0).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
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
        dir,
        _stop: stop,
    }
}

/// CONNECT, TLS with the proxy's forged leaf, and the websocket upgrade.
async fn open_socket(proxy: &Proxy, up_port: u16) -> TlsStream<TcpStream> {
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
    let (head, _) = support::read_head(&mut tcp).await;
    assert!(head.starts_with("http/1.1 200"), "{head}");
    let mut config = ClientConfig::builder()
        .with_root_certificates(root_store(&proxy.ca_pem))
        .with_no_client_auth();
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    let mut tls = TlsConnector::from(Arc::new(config))
        .connect(ServerName::try_from(HOST).unwrap(), tcp)
        .await
        .unwrap();
    tls.write_all(
        format!(
            "GET /backend-api/codex/responses HTTP/1.1\r\nHost: {HOST}:{up_port}\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n"
        )
        .as_bytes(),
    )
    .await
    .unwrap();
    let (head, rest) = support::read_head(&mut tls).await;
    assert!(head.starts_with("http/1.1 101"), "{head}");
    assert!(rest.is_empty());
    tls
}

/// A `response.create` message of about `bytes`, its secret early (so the
/// units carrying it are released long before the message ends).
fn create(model: &str, bytes: usize) -> String {
    let filler = "lorem ipsum dolor ".repeat(40);
    let mut input = vec![json!({"type":"message","role":"user",
        "content":[{"type":"input_text","text":format!("token={VALUE} and again {VALUE}")}]})];
    let mut size = 0;
    while size < bytes {
        let element = json!({"type":"message","role":"user",
            "content":[{"type":"input_text","text":filler}]});
        size += element.to_string().len();
        input.push(element);
    }
    json!({"type":"response.create","model":model,"input":input,"stream":true}).to_string()
}

/// Read client-bound messages until a `response.completed` or an error.
async fn read_until_done(tls: &mut TlsStream<TcpStream>, decoder: &mut FrameDecoder) -> Vec<Value> {
    let mut messages = Vec::new();
    let mut buf = vec![0u8; 64 * 1024];
    tokio::time::timeout(WAIT, async {
        loop {
            let n = tls.read(&mut buf).await.unwrap();
            assert!(n > 0, "the proxy closed the socket");
            for message in decoder.push(&buf[..n]) {
                if let WsMessage::Text(text) = message {
                    let value: Value = serde_json::from_str(&text).unwrap();
                    let done = value["type"] == "response.completed" || value["type"] == "error";
                    messages.push(value);
                    if done {
                        return;
                    }
                }
            }
        }
    })
    .await
    .expect("an answer in time");
    messages
}

#[tokio::test]
async fn a_websocket_conversation_streams_masks_refuses_and_restores_over_tls() {
    let up_ca = make_ca();
    let (up_port, seen) = spawn_upstream(&up_ca).await;
    let proxy = start_proxy(&up_ca).await;
    let mut tls = open_socket(&proxy, up_port).await;
    let mut decoder = FrameDecoder::with_limit(64 * 1024 * 1024);

    // 1. A message 30x the lag streams; the upstream never sees the value,
    //    only references, and the client gets the value back restored.
    let message = create("gpt-test", 30 * LAG);
    tls.write_all(&encode_frame(&WsMessage::Text(message.clone()), true))
        .await
        .unwrap();
    let answers = read_until_done(&mut tls, &mut decoder).await;
    assert_eq!(*proxy.opened.lock().unwrap(), 1, "streamed");
    {
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert!(!seen[0].contains(VALUE), "the value reached the upstream");
        assert!(seen[0].contains("localrouter-redacted"));
        let forwarded: Value = serde_json::from_str(&seen[0]).unwrap();
        let original: Value = serde_json::from_str(&message).unwrap();
        assert_eq!(
            forwarded["input"].as_array().unwrap().len(),
            original["input"].as_array().unwrap().len()
        );
        assert_eq!(forwarded["input"][5], original["input"][5]);
    }
    let delta = answers
        .iter()
        .find(|m| m["type"] == "response.output_text.delta")
        .expect("the delta");
    assert_eq!(delta["delta"], VALUE, "restored for the client");
    let completed = answers.last().unwrap();
    assert_eq!(completed["type"], "response.completed");
    assert_eq!(
        completed["response"]["output"][0]["content"][0]["text"],
        VALUE
    );

    // 2. A refused model, decided when the stream would open: an error
    //    event, nothing upstream, and the socket stays open.
    tls.write_all(&encode_frame(
        &WsMessage::Text(create("refused", 4 * LAG)),
        true,
    ))
    .await
    .unwrap();
    let refusal = read_until_done(&mut tls, &mut decoder).await;
    let error = refusal.last().unwrap();
    assert_eq!(error["type"], "error");
    assert_eq!(error["status"], 403);
    assert!(error.to_string().contains("model refused"), "{error}");
    assert_eq!(seen.lock().unwrap().len(), 1, "nothing of it upstream");

    // 3. The same socket carries a small message (held, judged whole).
    let small = json!({"type":"response.create","model":"gpt-test","input":"hello"}).to_string();
    tls.write_all(&encode_frame(&WsMessage::Text(small.clone()), true))
        .await
        .unwrap();
    let answers = read_until_done(&mut tls, &mut decoder).await;
    assert_eq!(answers.last().unwrap()["type"], "response.completed");
    assert_eq!(answers[0]["delta"], "pong");
    assert_eq!(seen.lock().unwrap()[1], small);
    assert_eq!(*proxy.opened.lock().unwrap(), 1, "the small one was held");

    // Every cycle is recorded once the socket closes; none carries the value.
    drop(tls);
    let recorded = tokio::time::timeout(WAIT, async {
        loop {
            let listed = proxy.store.list(0, 16, None);
            if listed.total >= 3
                && listed
                    .events
                    .iter()
                    .all(|e| e.status != lr_monitor::EventStatus::Pending)
            {
                return listed
                    .events
                    .iter()
                    .map(|e| serde_json::to_string(&proxy.store.get(&e.id).unwrap()).unwrap())
                    .collect::<Vec<_>>();
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("three cycles recorded");
    assert_eq!(recorded.len(), 3);
    // The forwarded cycles never record the value (the refusal's capture
    // is the client's own message, which never left).
    let forwarded: Vec<_> = recorded
        .iter()
        .filter(|event| !event.contains("model refused"))
        .collect();
    assert_eq!(forwarded.len(), 2);
    assert!(forwarded.iter().all(|event| !event.contains(VALUE)));
}
