//! End-to-end tests for the reverse proxy: a real listener in front of a real
//! (dummy) upstream, exercised over real TCP.

use std::convert::Infallible;
use std::sync::Arc;

use bytes::Bytes;
use http_body_util::{BodyExt, Full, StreamBody};
use hyper::body::Frame;
use hyper::service::service_fn;
use hyper::{Request, Response};
use hyper_util::rt::TokioIo;
use lr_proxy::reverse::{ReverseClient, ReverseExchange, ReverseProxy, ReverseRecorder};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;

/// Records every exchange for assertions.
#[derive(Default)]
struct CapturingRecorder {
    seen: Mutex<Vec<ReverseExchange>>,
}

#[async_trait::async_trait]
impl ReverseRecorder for CapturingRecorder {
    async fn record(&self, exchange: ReverseExchange) {
        self.seen.lock().await.push(exchange);
    }
}

type Boxed = http_body_util::combinators::UnsyncBoxBody<Bytes, std::io::Error>;

/// A dummy "Ollama": echoes the request body, streams NDJSON on /api/chat.
async fn upstream_handler(
    req: Request<hyper::body::Incoming>,
) -> Result<Response<Boxed>, Infallible> {
    assert_eq!(req.headers()["accept-encoding"], "identity");
    let path = req.uri().path().to_string();
    let had_host = req
        .headers()
        .get(hyper::header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    let trace = req
        .headers()
        .get("x-localrouter-trace")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    let body = req.into_body().collect().await.unwrap().to_bytes();

    if path == "/api/generate" {
        let request: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let text = request["prompt"].as_str().unwrap();
        assert!(!text.contains("synthetic reverse secret"));
        assert!(text.contains("localrouter-redacted"));
        let split = text.len() / 2;
        let events = [
            serde_json::json!({"response":&text[..split],"done":false}),
            serde_json::json!({"response":&text[split..],"done":true}),
        ];
        let wire = events
            .iter()
            .map(|event| format!("{event}\n"))
            .collect::<String>();
        let chunks = wire
            .as_bytes()
            .chunks(3)
            .map(|bytes| Ok::<_, std::io::Error>(Frame::data(Bytes::copy_from_slice(bytes))))
            .collect::<Vec<_>>();
        return Ok(Response::builder()
            .header("content-type", "application/x-ndjson")
            .body(StreamBody::new(futures::stream::iter(chunks)).boxed_unsync())
            .unwrap());
    }
    if path == "/api/chat" {
        // Two NDJSON chunks, delivered with a gap so a buffering proxy would
        // be visible as a single combined read on the client side.
        let chunks: Vec<Result<Frame<Bytes>, std::io::Error>> = vec![
            Ok(Frame::data(Bytes::from_static(b"{\"message\":\"a\"}\n"))),
            Ok(Frame::data(Bytes::from_static(b"{\"done\":true}\n"))),
        ];
        let stream = futures::stream::iter(chunks);
        return Ok(Response::builder()
            .status(200)
            .header("content-type", "application/x-ndjson")
            .body(StreamBody::new(stream).boxed_unsync())
            .unwrap());
    }

    let payload = format!(
        "{{\"path\":\"{path}\",\"host\":\"{had_host}\",\"trace\":\"{trace}\",\"body\":{}}}",
        if body.is_empty() {
            "null".to_string()
        } else {
            String::from_utf8_lossy(&body).to_string()
        }
    );
    Ok(Response::builder()
        .status(201)
        .header("content-type", "application/json")
        .header("x-upstream-marker", "yes")
        .body(
            Full::new(Bytes::from(payload))
                .map_err(|e: Infallible| match e {})
                .boxed_unsync(),
        )
        .unwrap())
}

async fn start_upstream() -> u16 {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                break;
            };
            tokio::spawn(async move {
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(TokioIo::new(stream), service_fn(upstream_handler))
                    .await;
            });
        }
    });
    port
}

async fn start_reverse(
    upstream_port: u16,
    recorder: Arc<dyn ReverseRecorder>,
) -> (u16, tokio::sync::oneshot::Sender<()>) {
    start_reverse_with_dedupe(upstream_port, recorder, true).await
}

async fn start_reverse_with_dedupe(
    upstream_port: u16,
    recorder: Arc<dyn ReverseRecorder>,
    dedupe: bool,
) -> (u16, tokio::sync::oneshot::Sender<()>) {
    let proxy = Arc::new(
        ReverseProxy::new(
            &format!("http://127.0.0.1:{upstream_port}"),
            ReverseClient {
                client_id: "client-1".into(),
                strategy_id: "strategy-1".into(),
                provider_instance: Some("Ollama".into()),
            },
            recorder,
        )
        .unwrap()
        .with_dedupe_flag(Arc::new(std::sync::atomic::AtomicBool::new(dedupe))),
    );
    let listener = ReverseProxy::bind("127.0.0.1", 0).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (tx, rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        proxy
            .serve(listener, async {
                let _ = rx.await;
            })
            .await;
    });
    (port, tx)
}

/// Send one request through the proxy and return (status, headers, body).
async fn request(
    port: u16,
    method: &str,
    path: &str,
    body: &str,
) -> (u16, hyper::HeaderMap, String) {
    request_with_headers(port, method, path, body, &[]).await
}

async fn request_with_headers(
    port: u16,
    method: &str,
    path: &str,
    body: &str,
    extra: &[(&str, &str)],
) -> (u16, hyper::HeaderMap, String) {
    request_bytes(port, method, path, body.as_bytes(), extra).await
}

async fn request_bytes(
    port: u16,
    method: &str,
    path: &str,
    body: &[u8],
    extra: &[(&str, &str)],
) -> (u16, hyper::HeaderMap, String) {
    let stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let (mut sender, conn) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
        .await
        .unwrap();
    tokio::spawn(async move {
        let _ = conn.await;
    });
    let mut builder = Request::builder()
        .method(method)
        .uri(path)
        .header("host", "127.0.0.1")
        .header("accept-encoding", "gzip");
    for (k, v) in extra {
        builder = builder.header(*k, *v);
    }
    let req = builder
        .body(Full::new(Bytes::copy_from_slice(body)))
        .unwrap();
    let resp = sender.send_request(req).await.unwrap();
    let status = resp.status().as_u16();
    let headers = resp.headers().clone();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (status, headers, String::from_utf8_lossy(&bytes).to_string())
}

#[tokio::test]
async fn forwards_request_and_response_verbatim() {
    let up = start_upstream().await;
    let recorder = Arc::new(CapturingRecorder::default());
    let (port, _shutdown) = start_reverse(up, recorder.clone()).await;

    let (status, headers, body) = request(
        port,
        "POST",
        "/v1/chat/completions",
        r#"{"model":"llama3"}"#,
    )
    .await;

    assert_eq!(status, 201, "upstream status must pass through");
    assert_eq!(
        headers.get("x-upstream-marker").unwrap(),
        "yes",
        "upstream headers must pass through"
    );
    assert_eq!(
        headers.get("x-localrouter-reverse-proxy").unwrap(),
        "1",
        "hop marker is added"
    );
    // The upstream saw our path, our body, and a Host rewritten to itself.
    assert!(body.contains("\"path\":\"/v1/chat/completions\""), "{body}");
    assert!(body.contains("\"model\":\"llama3\""), "{body}");
    assert!(
        body.contains(&format!("\"host\":\"127.0.0.1:{up}\"")),
        "{body}"
    );

    // Give the spawned recorder task a moment.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let seen = recorder.seen.lock().await;
    assert_eq!(seen.len(), 1);
    let ex = &seen[0];
    assert_eq!(ex.method, "POST");
    assert_eq!(ex.path, "/v1/chat/completions");
    assert_eq!(ex.status, Some(201));
    assert_eq!(ex.client_id, "client-1");
    assert_eq!(ex.strategy_id, "strategy-1");
    assert_eq!(ex.provider_instance.as_deref(), Some("Ollama"));
    assert_eq!(
        String::from_utf8_lossy(ex.request_body.as_ref().unwrap()),
        r#"{"model":"llama3"}"#
    );
    assert!(ex.response_body.is_some(), "response captured");
    assert!(ex.latency_ms.is_some());
    assert!(ex.error.is_none());
}

#[tokio::test]
async fn preserves_query_strings_and_get_requests() {
    let up = start_upstream().await;
    let recorder = Arc::new(CapturingRecorder::default());
    let (port, _shutdown) = start_reverse(up, recorder.clone()).await;

    let (status, _, body) = request(port, "GET", "/api/tags?verbose=1", "").await;
    assert_eq!(status, 201);
    assert!(body.contains("\"path\":\"/api/tags\""), "{body}");

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let seen = recorder.seen.lock().await;
    assert_eq!(seen[0].path, "/api/tags?verbose=1", "query is recorded");
    assert!(seen[0].request_body.is_none(), "empty body stays None");
}

#[tokio::test]
async fn streams_ndjson_and_flags_it() {
    let up = start_upstream().await;
    let recorder = Arc::new(CapturingRecorder::default());
    let (port, _shutdown) = start_reverse(up, recorder.clone()).await;

    let (status, headers, body) = request(port, "POST", "/api/chat", r#"{"stream":true}"#).await;
    assert_eq!(status, 200);
    assert_eq!(headers.get("content-type").unwrap(), "application/x-ndjson");
    assert_eq!(body, "{\"message\":\"a\"}\n{\"done\":true}\n");

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let seen = recorder.seen.lock().await;
    assert!(
        seen[0].response_is_ndjson,
        "ndjson flagged for the recorder"
    );
    assert!(!seen[0].response_is_sse);
    assert_eq!(
        String::from_utf8_lossy(seen[0].response_body.as_ref().unwrap()),
        "{\"message\":\"a\"}\n{\"done\":true}\n",
        "both streamed chunks captured"
    );
}

#[tokio::test]
async fn reports_unreachable_upstream_as_502() {
    // Bind and immediately drop a listener to get a port nothing listens on.
    let dead = {
        let l = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        l.local_addr().unwrap().port()
    };
    let recorder = Arc::new(CapturingRecorder::default());
    let (port, _shutdown) = start_reverse(dead, recorder.clone()).await;

    let (status, headers, body) = request(port, "GET", "/api/tags", "").await;
    assert_eq!(status, 502);
    assert_eq!(headers.get("content-type").unwrap(), "application/json");
    assert!(
        body.contains(&format!("127.0.0.1:{dead}")),
        "the 502 names the upstream so a missed relocation is diagnosable: {body}"
    );

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let seen = recorder.seen.lock().await;
    assert_eq!(seen[0].status, Some(502));
    assert!(seen[0].error.is_some(), "failure recorded for the monitor");
}

#[tokio::test]
async fn stamps_trace_header_for_the_next_hop() {
    let up = start_upstream().await;
    let recorder = Arc::new(CapturingRecorder::default());
    let (port, _shutdown) = start_reverse(up, recorder.clone()).await;

    // Fresh request: upstream sees hop 1 of a new trace.
    let (_, _, body) = request(port, "POST", "/v1/chat/completions", "{}").await;
    let trace: serde_json::Value = serde_json::from_str(&body).unwrap();
    let header = trace["trace"].as_str().unwrap();
    let t = lr_types::RequestTrace::parse(header).expect("valid trace header");
    assert_eq!(t.hop, 1);

    // Untrusted hop claims preserve correlation but do not confer privileges.
    let (_, _, body) = request_with_headers(
        port,
        "POST",
        "/v1/chat/completions",
        "{}",
        &[("x-localrouter-trace", "abc;hop=99")],
    )
    .await;
    let trace: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(trace["trace"].as_str().unwrap(), "abc;hop=1");

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let seen = recorder.seen.lock().await;
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[0].trace.as_ref().unwrap().hop, 1);
    let dup = seen[1].trace.as_ref().unwrap();
    assert_eq!((dup.trace_id.as_str(), dup.hop), ("abc", 1));
    assert!(!dup.is_duplicate());
}

#[tokio::test]
async fn disabled_dedupe_leaves_headers_alone() {
    let up = start_upstream().await;
    let recorder = Arc::new(CapturingRecorder::default());
    let (port, _shutdown) = start_reverse_with_dedupe(up, recorder.clone(), false).await;

    let (_, _, body) = request(port, "POST", "/v1/chat/completions", "{}").await;
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["trace"].as_str().unwrap(), "", "no header added");

    let (_, _, body) = request_with_headers(
        port,
        "POST",
        "/v1/chat/completions",
        "{}",
        &[("x-localrouter-trace", "abc;hop=1")],
    )
    .await;
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        v["trace"].as_str().unwrap(),
        "abc;hop=1",
        "forwarded verbatim"
    );

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let seen = recorder.seen.lock().await;
    assert!(seen.iter().all(|e| e.trace.is_none()));
}

struct SecretFirewall;
#[async_trait::async_trait]
impl lr_proxy::active::Firewall for SecretFirewall {
    async fn evaluate(
        &self,
        request: &lr_proxy::active::FirewallRequest,
    ) -> lr_proxy::interceptor::RequestAction {
        use lr_types::secret_context::SecretContext;
        let vault = Arc::new(lr_secret_scanner::placeholders::Vault::new(
            Arc::new(lr_api_keys::MockKeychain::new()),
            &request.client_id,
            "reverse",
            true,
        ));
        vault.register("synthetic reverse secret").unwrap();
        let mut body = request.body.clone();
        vault.protect(&mut body).unwrap();
        lr_proxy::interceptor::RequestAction::Protect {
            body: serde_json::to_vec(&body).unwrap(),
            vault,
        }
    }
}
async fn protected_reverse(compressed: bool) {
    let upstream_port = start_upstream().await;
    let store = Arc::new(lr_monitor::MonitorEventStore::new(16));
    let recorder = Arc::new(lr_proxy::active::ActiveInterceptor::new(
        lr_proxy::passive::PassiveInterceptor::new(store.clone()),
        Arc::new(SecretFirewall),
    ));
    let proxy = Arc::new(
        ReverseProxy::new(
            &format!("http://127.0.0.1:{upstream_port}"),
            ReverseClient {
                client_id: "reverse-client".into(),
                strategy_id: "strategy".into(),
                provider_instance: Some("ollama".into()),
            },
            recorder,
        )
        .unwrap(),
    );
    let listener = ReverseProxy::bind("127.0.0.1", 0).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let task = tokio::spawn(async move {
        proxy.serve(listener, std::future::pending::<()>()).await;
    });
    let tcp = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let (mut sender, conn) = hyper::client::conn::http1::handshake(TokioIo::new(tcp))
        .await
        .unwrap();
    let connection = tokio::spawn(async move {
        conn.await.ok();
    });
    let payload =
        serde_json::json!({"model":"local","prompt":"synthetic reverse secret","stream":true})
            .to_string()
            .into_bytes();
    let payload = if compressed {
        zstd::stream::encode_all(payload.as_slice(), 3).unwrap()
    } else {
        payload
    };
    let mut builder = Request::builder()
        .method("POST")
        .uri("/api/generate")
        .header("host", format!("127.0.0.1:{port}"));
    if compressed {
        builder = builder.header("content-encoding", "zstd");
    }
    let request = builder.body(Full::new(Bytes::from(payload))).unwrap();
    let response = sender.send_request(request).await.unwrap();
    assert_eq!(response.status(), 200);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let text: String = std::str::from_utf8(&bytes)
        .unwrap()
        .lines()
        .map(|line| {
            serde_json::from_str::<serde_json::Value>(line).unwrap()["response"]
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect();
    assert_eq!(text, "synthetic reverse secret");
    for _ in 0..100 {
        if store
            .list(0, 10, None)
            .events
            .iter()
            .any(|event| event.status != lr_monitor::EventStatus::Pending)
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    let events = serde_json::to_string(&store.list(0, 10, None)).unwrap();
    assert!(!events.contains("synthetic reverse secret"));
    assert!(events.contains("localrouter-redacted"));
    task.abort();
    connection.abort();
}

#[tokio::test]
async fn native_reverse_ndjson_restores_fragmented_refs_and_captures_masked_wire() {
    protected_reverse(false).await;
}
#[tokio::test]
async fn compressed_reverse_requests_are_masked_and_response_is_restored() {
    protected_reverse(true).await;
}

async fn unchanged_forwarding(active: bool) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let upstream_port = listener.local_addr().unwrap().port();
    let (tx, mut rx) = tokio::sync::mpsc::channel(4);
    tokio::spawn(async move {
        loop {
            let (tcp, _) = listener.accept().await.unwrap();
            let tx = tx.clone();
            tokio::spawn(async move {
                hyper::server::conn::http1::Builder::new()
                    .serve_connection(
                        TokioIo::new(tcp),
                        service_fn(move |req: Request<hyper::body::Incoming>| {
                            let tx = tx.clone();
                            async move {
                                let headers = req.headers().clone();
                                let body = req.into_body().collect().await.unwrap().to_bytes();
                                tx.send((headers, body)).await.unwrap();
                                Ok::<_, Infallible>(Response::new(Full::new(Bytes::from_static(
                                    b"{}",
                                ))))
                            }
                        }),
                    )
                    .await
                    .ok();
            });
        }
    });
    struct ForwardPolicy;
    #[async_trait::async_trait]
    impl lr_proxy::active::Firewall for ForwardPolicy {
        async fn evaluate(
            &self,
            req: &lr_proxy::active::FirewallRequest,
        ) -> lr_proxy::interceptor::RequestAction {
            assert_eq!(req.model.as_deref(), Some("local"));
            assert_eq!(req.body["input"], "no secret");
            lr_proxy::interceptor::RequestAction::Forward
        }
    }
    let recorder: Arc<dyn ReverseRecorder> = if active {
        Arc::new(lr_proxy::active::ActiveInterceptor::new(
            lr_proxy::passive::PassiveInterceptor::new(Arc::new(
                lr_monitor::MonitorEventStore::new(16),
            )),
            Arc::new(ForwardPolicy),
        ))
    } else {
        Arc::new(CapturingRecorder::default())
    };
    let (port, _shutdown) = start_reverse(upstream_port, recorder).await;
    for (encoding, body) in [
        (
            "zstd",
            zstd::stream::encode_all(br#"{"model":"local","input":"no secret"}"#.as_slice(), 3)
                .unwrap(),
        ),
        ("unknown", b"opaque representation".to_vec()),
    ] {
        if active && encoding == "unknown" {
            continue;
        }
        let (status, _, _) = request_bytes(
            port,
            "POST",
            "/v1/responses",
            &body,
            &[("content-encoding", encoding), ("content-md5", "unchanged")],
        )
        .await;
        assert_eq!(status, 200);
        let (headers, seen) = rx.recv().await.unwrap();
        assert_eq!(seen.as_ref(), body.as_slice());
        assert_eq!(headers["content-encoding"], encoding);
        assert_eq!(headers["content-md5"], "unchanged");
    }
}

#[tokio::test]
async fn active_decode_failures_do_not_contact_upstream() {
    let upstream = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let upstream_port = upstream.local_addr().unwrap().port();
    let recorder = Arc::new(lr_proxy::active::ActiveInterceptor::new(
        lr_proxy::passive::PassiveInterceptor::new(Arc::new(lr_monitor::MonitorEventStore::new(
            16,
        ))),
        Arc::new(SecretFirewall),
    ));
    let proxy = Arc::new(
        ReverseProxy::new(
            &format!("http://127.0.0.1:{upstream_port}"),
            ReverseClient::default(),
            recorder,
        )
        .unwrap(),
    );
    let listener = ReverseProxy::bind("127.0.0.1", 0).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let task = tokio::spawn(async move {
        proxy.serve(listener, std::future::pending::<()>()).await;
    });
    for (encoding, payload, expected) in [
        ("zstd", b"invalid".to_vec(), 400),
        ("unknown", b"{}".to_vec(), 415),
        ("gzip", b"invalid".to_vec(), 400),
        (
            "zstd",
            zstd::stream::encode_all(vec![b'x'; 16 * 1024 * 1024 + 1].as_slice(), 3).unwrap(),
            413,
        ),
    ] {
        let (status, _, body) = request_bytes(
            port,
            "POST",
            "/v1/responses",
            &payload,
            &[("content-encoding", encoding)],
        )
        .await;
        assert_eq!(status, expected, "{body}");
        assert!(body.contains("localrouter_firewall"));
    }
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(30), upstream.accept())
            .await
            .is_err()
    );
    task.abort();
}

#[tokio::test]
async fn passive_forwarding_preserves_encoded_bytes_and_integrity_headers() {
    unchanged_forwarding(false).await;
}
#[tokio::test]
async fn active_unchanged_compressed_json_is_inspected_and_forwarded_exactly() {
    unchanged_forwarding(true).await;
}
