//! Usage of long responses: an LLM response above the 1 MiB capture head
//! carries its token counts in the terminal event (SSE / NDJSON) or at the end
//! of the body (JSON). Both proxies, plain and protected (restored) responses,
//! every wire format; the response is generated on the fly.

mod support;

use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use http_body_util::{BodyExt, Full, StreamBody};
use hyper::body::{Frame, Incoming};
use hyper::service::service_fn;
use hyper::{Request, Response};
use hyper_util::rt::TokioIo;
use rustls::{ClientConfig, RootCertStore};
use rustls_pki_types::ServerName;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;
use tokio_rustls::TlsConnector;

use lr_monitor::{EventStatus, MonitorEventData, MonitorEventStore};
use lr_proxy::active::{ActiveInterceptor, Firewall, FirewallRequest};
use lr_proxy::cert::CertAuthority;
use lr_proxy::interceptor::{
    BodyPolicy, ClientCtx, ConnectDecision, ObservedExchange, PassthroughExchange,
    ProxyInterceptor, RequestAction, RequestHead,
};
use lr_proxy::passive::PassiveInterceptor;
use lr_proxy::resolver::StaticResolver;
use lr_proxy::reverse::{ReverseClient, ReverseProxy, ReverseRecorder};
use lr_proxy::{tls, ProxyManager};

use support::{make_ca, root_store, tls_listener, TestCa};

const HOST: &str = "localhost";
const CLIENT_ID: &str = "cid";
const SECRET: &str = "lr-secret";
/// Generated response size: well above the 1 MiB capture head.
const RESPONSE_BYTES: usize = 3 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    Chat,
    Responses,
    Messages,
    OllamaChat,
    /// A non-streamed chat completion, one JSON document.
    Json,
}

const KINDS: [Kind; 5] = [
    Kind::Chat,
    Kind::Responses,
    Kind::Messages,
    Kind::OllamaChat,
    Kind::Json,
];

impl Kind {
    fn path(self) -> &'static str {
        match self {
            Kind::Chat | Kind::Json => "/v1/chat/completions",
            Kind::Responses => "/v1/responses",
            Kind::Messages => "/v1/messages",
            Kind::OllamaChat => "/api/chat",
        }
    }

    fn content_type(self) -> &'static str {
        match self {
            Kind::OllamaChat => "application/x-ndjson",
            Kind::Json => "application/json",
            _ => "text/event-stream",
        }
    }

    /// The response, as frames: about [`RESPONSE_BYTES`] of deltas, then
    /// the terminal event with `input 11` / `output 22` tokens.
    fn frames(self) -> Vec<Bytes> {
        let piece = "streamed words ".repeat(16);
        let events = RESPONSE_BYTES / (piece.len() + 120);
        let sse = |value: serde_json::Value| format!("data: {value}\n\n");
        let mut frames = Vec::new();
        match self {
            Kind::Chat => {
                for _ in 0..events {
                    frames.push(sse(serde_json::json!({"id":"c1","object":"chat.completion.chunk","model":"m","choices":[{"index":0,"delta":{"content":piece}}]})));
                }
                frames.push(sse(serde_json::json!({"id":"c1","object":"chat.completion.chunk","model":"m","choices":[],"usage":{"prompt_tokens":11,"completion_tokens":22,"total_tokens":33}})));
                frames.push("data: [DONE]\n\n".into());
            }
            Kind::Responses => {
                frames.push(format!(
                    "event: response.created\n{}",
                    sse(serde_json::json!({"type":"response.created","response":{"id":"r1","model":"m","status":"in_progress","output":[]}}))
                ));
                for _ in 0..events {
                    frames.push(format!(
                        "event: response.output_text.delta\n{}",
                        sse(serde_json::json!({"type":"response.output_text.delta","item_id":"i1","output_index":0,"content_index":0,"delta":piece}))
                    ));
                }
                // The terminal event repeats the whole output: here larger
                // than everything the capture keeps of a response's end
                // (1 MiB, extended to a line start up to 8 MiB), so its
                // usage is only known from reading it as it streamed.
                let text = "x".repeat(12 * 1024 * 1024);
                frames.push(format!(
                    "event: response.completed\n{}",
                    sse(serde_json::json!({"type":"response.completed","response":{"id":"r1","model":"m","status":"completed","output":[{"type":"message","id":"i1","role":"assistant","content":[{"type":"output_text","text":text}]}],"usage":{"input_tokens":11,"output_tokens":22,"total_tokens":33}}}))
                ));
            }
            Kind::Messages => {
                frames.push(format!(
                    "event: message_start\n{}",
                    sse(serde_json::json!({"type":"message_start","message":{"id":"msg1","model":"m","usage":{"input_tokens":11,"output_tokens":1}}}))
                ));
                frames.push(format!(
                    "event: content_block_start\n{}",
                    sse(serde_json::json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}))
                ));
                for _ in 0..events {
                    frames.push(format!(
                        "event: content_block_delta\n{}",
                        sse(serde_json::json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":piece}}))
                    ));
                }
                frames.push(format!(
                    "event: content_block_stop\n{}",
                    sse(serde_json::json!({"type":"content_block_stop","index":0}))
                ));
                frames.push(format!(
                    "event: message_delta\n{}",
                    sse(serde_json::json!({"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":22}}))
                ));
                frames.push(format!(
                    "event: message_stop\n{}",
                    sse(serde_json::json!({"type":"message_stop"}))
                ));
            }
            Kind::OllamaChat => {
                for _ in 0..events {
                    frames.push(format!(
                        "{}\n",
                        serde_json::json!({"model":"m","message":{"role":"assistant","content":piece},"done":false})
                    ));
                }
                frames.push(format!(
                    "{}\n",
                    serde_json::json!({"model":"m","message":{"role":"assistant","content":""},"done":true,"done_reason":"stop","prompt_eval_count":11,"eval_count":22})
                ));
            }
            Kind::Json => {
                // Larger than the head and the end kept together: the
                // middle is omitted, so the document is never parsed whole.
                let text = "y".repeat(12 * 1024 * 1024);
                frames.push(
                    serde_json::json!({"id":"c1","object":"chat.completion","model":"m","choices":[{"index":0,"message":{"role":"assistant","content":text},"finish_reason":"stop"}],"usage":{"prompt_tokens":11,"completion_tokens":22,"total_tokens":33}})
                        .to_string(),
                );
            }
        }
        // Delivered in reads of at most 64 KiB, as a provider would.
        let wire: Vec<u8> = frames.concat().into_bytes();
        wire.chunks(64 * 1024).map(Bytes::copy_from_slice).collect()
    }
}

/// The provider: answers every request with the response of its path (the
/// `x-kind` header picks the non-streamed JSON).
async fn serve_provider<S>(io: S)
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let service = service_fn(|req: Request<Incoming>| async move {
        let json = req.headers().get("x-kind").is_some_and(|v| v == "json");
        let path = req.uri().path().to_string();
        let _ = req.into_body().collect().await;
        let kind = match path.as_str() {
            "/v1/chat/completions" if json => Kind::Json,
            "/v1/chat/completions" => Kind::Chat,
            "/v1/responses" => Kind::Responses,
            "/v1/messages" => Kind::Messages,
            _ => Kind::OllamaChat,
        };
        let frames = kind.frames();
        let body = StreamBody::new(futures::stream::iter(
            frames
                .into_iter()
                .map(|b| Ok::<_, Infallible>(Frame::data(b))),
        ));
        Ok::<_, Infallible>(
            Response::builder()
                .header("content-type", kind.content_type())
                .body(body)
                .unwrap(),
        )
    });
    let _ = hyper::server::conn::http1::Builder::new()
        .serve_connection(TokioIo::new(io), service)
        .await;
}

/// Forwards every request: unchanged, or protected with an empty vault so
/// the response goes through restoration (and its masked-wire capture).
struct Forward {
    protect: bool,
}

#[async_trait]
impl Firewall for Forward {
    async fn evaluate(&self, req: &FirewallRequest) -> RequestAction {
        if !self.protect {
            return RequestAction::Forward;
        }
        let vault = Arc::new(lr_secret_scanner::placeholders::Vault::new(
            Arc::new(lr_api_keys::MockKeychain::new()),
            &req.client_id,
            "capture",
            true,
        ));
        RequestAction::Protect {
            body: serde_json::to_vec(&req.body).unwrap(),
            vault,
        }
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

fn interceptor(store: &Arc<MonitorEventStore>, protect: bool) -> ActiveInterceptor {
    ActiveInterceptor::new(
        PassiveInterceptor::new(store.clone()),
        Arc::new(Forward { protect }),
    )
}

/// Send one small request over `io` and return the response body length.
async fn exchange<S>(io: S, kind: Kind) -> usize
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (mut sender, conn) = hyper::client::conn::http1::handshake(TokioIo::new(io))
        .await
        .unwrap();
    tokio::spawn(conn);
    let body = serde_json::json!({
        "model": "m",
        "messages": [{"role": "user", "content": "hello"}],
        "stream": kind != Kind::Json,
    })
    .to_string();
    let request = Request::builder()
        .method("POST")
        .uri(kind.path())
        .header("host", HOST)
        .header("content-type", "application/json")
        .header("x-kind", if kind == Kind::Json { "json" } else { "stream" })
        .body(Full::new(Bytes::from(body)))
        .unwrap();
    let response = sender.send_request(request).await.unwrap();
    assert_eq!(response.status(), 200, "{kind:?}");
    response
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes()
        .len()
}

/// Wait for the one event of the store to complete and check its usage.
async fn assert_usage(store: &MonitorEventStore, kind: Kind, label: &str, received: usize) {
    let expected: usize = kind.frames().iter().map(Bytes::len).sum();
    assert_eq!(
        received, expected,
        "{label} {kind:?}: whole response delivered"
    );
    let mut event = None;
    for _ in 0..500 {
        event = store
            .list(0, 10, None)
            .events
            .into_iter()
            .find(|e| e.status != EventStatus::Pending);
        if event.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let summary = event.unwrap_or_else(|| panic!("{label} {kind:?}: event completed"));
    assert_eq!(summary.status, EventStatus::Complete, "{label} {kind:?}");
    let event = store.get(&summary.id).unwrap();
    let MonitorEventData::LlmCall {
        input_tokens,
        output_tokens,
        raw_response,
        ..
    } = &event.data
    else {
        panic!("{label} {kind:?}: an LLM call event");
    };
    assert_eq!(
        (*input_tokens, *output_tokens),
        (Some(11), Some(22)),
        "{label} {kind:?}: usage from the terminal event"
    );
    let raw = raw_response.as_deref().unwrap_or_default();
    assert!(
        raw.contains("bytes omitted]"),
        "{label} {kind:?}: raw response shows head, omission and end"
    );
    assert!(raw.len() < 300 * 1024, "{label} {kind:?}: raw text bounded");
}

async fn reverse_run(kind: Kind, protect: bool) {
    let upstream = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let upstream_port = upstream.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((tcp, _)) = upstream.accept().await {
            tokio::spawn(serve_provider(tcp));
        }
    });
    let store = Arc::new(MonitorEventStore::new(16));
    let recorder: Arc<dyn ReverseRecorder> = Arc::new(interceptor(&store, protect));
    let proxy = Arc::new(
        ReverseProxy::new(
            &format!("http://127.0.0.1:{upstream_port}"),
            ReverseClient {
                client_id: CLIENT_ID.into(),
                strategy_id: "strategy".into(),
                provider_instance: Some("ollama".into()),
            },
            recorder,
        )
        .unwrap(),
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
    let tcp = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let received = exchange(tcp, kind).await;
    let label = if protect {
        "reverse protected"
    } else {
        "reverse"
    };
    assert_usage(&store, kind, label, received).await;
    let _ = stop.send(());
}

async fn mitm_run(up_ca: &TestCa, kind: Kind, protect: bool) {
    let (listener, acceptor, up_port) = tls_listener(up_ca, HOST).await;
    tokio::spawn(async move {
        while let Ok((tcp, _)) = listener.accept().await {
            let acceptor = acceptor.clone();
            tokio::spawn(async move {
                if let Ok(tls) = acceptor.accept(tcp).await {
                    serve_provider(tls).await;
                }
            });
        }
    });
    tls::ensure_crypto_provider();
    let dir = std::env::temp_dir().join(format!("lr-capture-{}", uuid::Uuid::new_v4()));
    let ca = Arc::new(CertAuthority::load_or_create(&dir).unwrap());
    let ca_pem = ca.ca_pem().to_string();
    let store = Arc::new(MonitorEventStore::new(16));
    let mut roots = RootCertStore::empty();
    roots.add(up_ca.ca_der.clone()).unwrap();
    let manager = ProxyManager::with_upstream_roots(
        ca,
        Arc::new(Mitm(interceptor(&store, protect))),
        Arc::new(StaticResolver {
            client_id: CLIENT_ID.into(),
            secret: SECRET.into(),
            proxy_enabled: true,
        }),
        roots,
    )
    .unwrap();
    let proxy_listener = ProxyManager::bind("127.0.0.1", 0).await.unwrap();
    let proxy_port = proxy_listener.local_addr().unwrap().port();
    let (stop, stopped) = oneshot::channel::<()>();
    tokio::spawn(async move {
        manager
            .serve(proxy_listener, async {
                let _ = stopped.await;
            })
            .await;
    });

    use base64::Engine;
    let mut tcp = TcpStream::connect(("127.0.0.1", proxy_port)).await.unwrap();
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
        .with_root_certificates(root_store(&ca_pem))
        .with_no_client_auth();
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    let tls = TlsConnector::from(Arc::new(config))
        .connect(ServerName::try_from(HOST).unwrap(), tcp)
        .await
        .unwrap();
    let received = exchange(tls, kind).await;
    let label = if protect { "mitm protected" } else { "mitm" };
    assert_usage(&store, kind, label, received).await;
    let _ = stop.send(());
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn reverse_proxy_records_usage_of_responses_above_the_capture_head() {
    for protect in [false, true] {
        for kind in KINDS {
            reverse_run(kind, protect).await;
        }
    }
}

#[tokio::test]
async fn mitm_proxy_records_usage_of_responses_above_the_capture_head() {
    let up_ca = make_ca();
    for protect in [false, true] {
        for kind in KINDS {
            mitm_run(&up_ca, kind, protect).await;
        }
    }
}
