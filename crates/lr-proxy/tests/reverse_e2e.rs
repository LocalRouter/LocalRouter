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
    let path = req.uri().path().to_string();
    let had_host = req
        .headers()
        .get(hyper::header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    let body = req.into_body().collect().await.unwrap().to_bytes();

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
        "{{\"path\":\"{path}\",\"host\":\"{had_host}\",\"body\":{}}}",
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
    recorder: Arc<CapturingRecorder>,
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
        .unwrap(),
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
    let stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let (mut sender, conn) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
        .await
        .unwrap();
    tokio::spawn(async move {
        let _ = conn.await;
    });
    let req = Request::builder()
        .method(method)
        .uri(path)
        .header("host", "127.0.0.1")
        .header("accept-encoding", "gzip")
        .body(Full::new(Bytes::from(body.to_string())))
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
