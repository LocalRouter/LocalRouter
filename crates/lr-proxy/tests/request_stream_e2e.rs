//! End-to-end tests for request bodies the MITM proxy forwards without
//! buffering: real TLS on both sides, bodies generated on the fly.

mod support;

use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper::{Request, Response};
use hyper_util::rt::TokioIo;
use rustls::{ClientConfig, RootCertStore};
use rustls_pki_types::ServerName;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;
use tokio_rustls::client::TlsStream;
use tokio_rustls::TlsConnector;

use lr_monitor::{EventStatus, MonitorEventStore};
use lr_proxy::active::{ActiveInterceptor, Firewall, FirewallRequest};
use lr_proxy::cert::CertAuthority;
use lr_proxy::interceptor::{
    BodyPolicy, ClientCtx, ConnectDecision, ObservedExchange, PassthroughExchange,
    ProxyInterceptor, RequestAction, RequestHead,
};
use lr_proxy::passive::PassiveInterceptor;
use lr_proxy::resolver::StaticResolver;
use lr_proxy::{tls, ProxyManager};

use support::{make_ca, read_head, root_store, tls_listener, SyntheticConversation, TestCa};

const HOST: &str = "localhost";
const CLIENT_ID: &str = "cid";
const SECRET: &str = "lr-secret";
const MIB: u64 = 1024 * 1024;
const WAIT: Duration = Duration::from_secs(20);

/// MITMs the test host and delegates everything else, the body policy
/// included, to the wrapped interceptor.
struct Mitm<I>(I);

#[async_trait]
impl<I: ProxyInterceptor> ProxyInterceptor for Mitm<I> {
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

struct Proxy {
    port: u16,
    ca_pem: String,
    store: Arc<MonitorEventStore>,
    dir: std::path::PathBuf,
    _stop: oneshot::Sender<()>,
}

impl Drop for Proxy {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Run the proxy in front of an upstream signed by `up_ca`.
async fn start_proxy(
    up_ca: &TestCa,
    interceptor: impl FnOnce(Arc<MonitorEventStore>) -> Arc<dyn ProxyInterceptor>,
) -> Proxy {
    tls::ensure_crypto_provider();
    let dir = std::env::temp_dir().join(format!("lr-request-stream-{}", uuid::Uuid::new_v4()));
    let ca = Arc::new(CertAuthority::load_or_create(&dir).unwrap());
    let ca_pem = ca.ca_pem().to_string();
    let store = Arc::new(MonitorEventStore::new(64));
    let mut roots = RootCertStore::empty();
    roots.add(up_ca.ca_der.clone()).unwrap();
    let manager = ProxyManager::with_upstream_roots(
        ca,
        interceptor(store.clone()),
        Arc::new(StaticResolver {
            client_id: CLIENT_ID.into(),
            secret: SECRET.into(),
            proxy_enabled: true,
        }),
        roots,
    )
    .unwrap();
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
        dir,
        _stop: stop,
    }
}

/// A passive proxy: every body passes through.
async fn start_passive(up_ca: &TestCa) -> Proxy {
    start_proxy(up_ca, |store| {
        Arc::new(Mitm(PassiveInterceptor::new(store)))
    })
    .await
}

/// CONNECT to the upstream through the proxy and finish the client's TLS.
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
    assert!(String::from_utf8_lossy(&seen).contains("200"));
    let mut config = ClientConfig::builder()
        .with_root_certificates(root_store(&proxy.ca_pem))
        .with_no_client_auth();
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    TlsConnector::from(Arc::new(config))
        .connect(ServerName::try_from(HOST).unwrap(), tcp)
        .await
        .unwrap()
}

/// Send one request through the proxy with hyper; returns status and body.
async fn send<B>(proxy: &Proxy, up_port: u16, request: Request<B>) -> (u16, Vec<u8>)
where
    B: hyper::body::Body<Data = Bytes> + Send + 'static,
    B::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    let tls = connect(proxy, up_port).await;
    let (mut sender, conn) = hyper::client::conn::http1::handshake(TokioIo::new(tls))
        .await
        .unwrap();
    tokio::spawn(async move {
        let _ = conn.await;
    });
    let response = sender.send_request(request).await.unwrap();
    let status = response.status().as_u16();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    (status, body.to_vec())
}

fn post(path: &str, up_port: u16) -> hyper::http::request::Builder {
    Request::builder()
        .method("POST")
        .uri(path)
        .header("host", format!("{HOST}:{up_port}"))
        .header("content-type", "application/json")
}

/// What a hyper upstream saw of one request.
struct Seen {
    headers: hyper::HeaderMap,
    len: u64,
    /// The body, kept only when it is small.
    body: Vec<u8>,
}

/// A hyper upstream that reads each request body without keeping it and
/// answers with a small JSON message.
async fn spawn_counting_upstream(ca: &TestCa) -> (u16, tokio::sync::mpsc::Receiver<Seen>) {
    let (listener, acceptor, port) = tls_listener(ca, HOST).await;
    let (tx, rx) = tokio::sync::mpsc::channel(8);
    tokio::spawn(async move {
        loop {
            let Ok((tcp, _)) = listener.accept().await else {
                break;
            };
            let acceptor = acceptor.clone();
            let tx = tx.clone();
            tokio::spawn(async move {
                let Ok(tls) = acceptor.accept(tcp).await else {
                    return;
                };
                let service = service_fn(move |req: Request<Incoming>| {
                    let tx = tx.clone();
                    async move {
                        let headers = req.headers().clone();
                        let mut body = req.into_body();
                        let (mut len, mut kept) = (0u64, Vec::new());
                        while let Some(frame) = body.frame().await {
                            if let Ok(data) = frame.unwrap().into_data() {
                                len += data.len() as u64;
                                if len <= 64 * 1024 {
                                    kept.extend_from_slice(&data);
                                }
                            }
                        }
                        tx.send(Seen {
                            headers,
                            len,
                            body: kept,
                        })
                        .await
                        .unwrap();
                        Ok::<_, Infallible>(
                            Response::builder()
                                .header("content-type", "application/json")
                                .body(Full::new(Bytes::from_static(
                                    br#"{"content":[{"type":"text","text":"pong"}],"stop_reason":"end_turn","usage":{"input_tokens":7,"output_tokens":2}}"#,
                                )))
                                .unwrap(),
                        )
                    }
                });
                hyper::server::conn::http1::Builder::new()
                    .serve_connection(TokioIo::new(tls), service)
                    .await
                    .ok();
            });
        }
    });
    (port, rx)
}

/// Wait until the monitor holds `n` events and none of them is pending, then
/// return them as JSON, oldest first.
async fn settled_events(store: &MonitorEventStore, n: usize) -> Vec<serde_json::Value> {
    tokio::time::timeout(WAIT, async {
        loop {
            let listed = store.list(0, 64, None);
            if listed.total >= n
                && listed
                    .events
                    .iter()
                    .all(|event| event.status != EventStatus::Pending)
            {
                let mut events: Vec<_> = listed
                    .events
                    .iter()
                    .map(|summary| store.get(&summary.id).unwrap())
                    .collect();
                events.sort_by_key(|event| event.sequence);
                return events
                    .into_iter()
                    .map(|event| serde_json::to_value(event).unwrap())
                    .collect();
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("monitor events did not settle")
}

/// One 64 KiB block of an upload: its index, then filler.
const BLOCK: usize = 64 * 1024;
fn block(index: u64) -> Vec<u8> {
    let mut block = vec![b'u'; BLOCK];
    block[..8].copy_from_slice(&index.to_be_bytes());
    block
}

/// The headline case: an upload larger than the buffered limit, on a path
/// nothing inspects. The upstream holds its first bytes while the client has
/// barely started, and while the upstream reads nothing the client cannot
/// push more than the sockets and the proxy's small buffers take.
#[tokio::test]
async fn large_upload_on_a_non_llm_path_is_streamed_with_bounded_buffering() {
    // Above the 256 MiB a buffered request may have.
    const TOTAL: u64 = 300 * MIB;
    let up_ca = make_ca();
    let (listener, acceptor, up_port) = tls_listener(&up_ca, HOST).await;
    let (first_tx, first_rx) = oneshot::channel::<()>();
    let (resume_tx, resume_rx) = oneshot::channel::<()>();
    let upstream = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let mut tls = acceptor.accept(tcp).await.unwrap();
        let (head, mut pending) = read_head(&mut tls).await;
        let mut buffer = vec![0u8; 256 * 1024];
        while pending.is_empty() {
            let n = tls.read(&mut buffer).await.unwrap();
            assert!(n > 0);
            pending.extend_from_slice(&buffer[..n]);
        }
        first_tx.send(()).unwrap();
        // Read nothing more until the client has run into backpressure.
        resume_rx.await.unwrap();
        let (mut received, mut index) = (0u64, 0u64);
        let filler = block(0);
        while received < TOTAL {
            while pending.len() >= BLOCK {
                assert_eq!(
                    pending[..8],
                    index.to_be_bytes(),
                    "block {index} out of order"
                );
                assert!(pending[8..BLOCK] == filler[8..], "block {index} corrupted");
                pending.drain(..BLOCK);
                received += BLOCK as u64;
                index += 1;
            }
            if received < TOTAL {
                let n = tls.read(&mut buffer).await.unwrap();
                assert!(n > 0, "upstream saw the end after {received} bytes");
                pending.extend_from_slice(&buffer[..n]);
            }
        }
        assert!(pending.is_empty(), "upstream received more than was sent");
        tls.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok")
            .await
            .unwrap();
        tls.flush().await.unwrap();
        let _ = tls.read(&mut buffer).await;
        head
    });

    let proxy = start_passive(&up_ca).await;
    let mut client = connect(&proxy, up_port).await;
    client
        .write_all(
            format!(
                "POST /v1/files HTTP/1.1\r\nHost: {HOST}:{up_port}\r\nContent-Type: application/octet-stream\r\nContent-Length: {TOTAL}\r\n\r\n"
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    client.write_all(&block(0)).await.unwrap();
    client.flush().await.unwrap();
    tokio::time::timeout(WAIT, first_rx)
        .await
        .expect("the upstream saw no body byte while the client was still uploading")
        .unwrap();

    // Keep writing until a write makes no progress: that is backpressure
    // reaching the client. `accepted` counts what was taken before it.
    let mut accepted = BLOCK as u64;
    let mut current = block(1);
    let (mut index, mut offset) = (1u64, 0usize);
    let mut stalled = false;
    while accepted < TOTAL {
        match tokio::time::timeout(Duration::from_millis(750), client.write(&current[offset..]))
            .await
        {
            Err(_) => {
                stalled = true;
                break;
            }
            Ok(written) => {
                let written = written.unwrap();
                assert!(written > 0);
                offset += written;
                accepted += written as u64;
                if offset == BLOCK {
                    index += 1;
                    offset = 0;
                    current = block(index);
                }
            }
        }
    }
    assert!(
        stalled,
        "the whole upload was accepted while the upstream read nothing"
    );
    assert!(
        accepted < 64 * MIB,
        "{accepted} bytes were held between the client and a stalled upstream"
    );

    resume_tx.send(()).unwrap();
    while accepted < TOTAL {
        client.write_all(&current[offset..]).await.unwrap();
        accepted += (BLOCK - offset) as u64;
        index += 1;
        offset = 0;
        current = block(index);
    }
    client.flush().await.unwrap();
    let (head, body) = tokio::time::timeout(WAIT, read_head(&mut client))
        .await
        .expect("no response after the upload");
    assert!(head.starts_with("http/1.1 200"), "{head}");
    assert_eq!(body, b"ok");
    drop(client);

    // The length and framing the client chose reached the upstream.
    let head = upstream.await.unwrap();
    assert!(
        head.contains(&format!("content-length: {TOTAL}\r\n")),
        "{head}"
    );
    assert!(!head.contains("transfer-encoding"), "{head}");

    // Recorded as a passthrough: destination and volume, no content.
    let events = settled_events(&proxy.store, 1).await;
    assert_eq!(events.len(), 1);
    let event = &events[0];
    assert_eq!(event["event_type"], "proxy_passthrough", "{event}");
    assert_eq!(event["status"], "complete", "{event}");
    assert_eq!(event["data"]["bytes_sent"], TOTAL, "{event}");
    assert_eq!(event["data"]["status_code"], 200, "{event}");
    assert_eq!(event["data"]["path"], "/v1/files", "{event}");
}

/// LLM requests through a passive proxy: forwarded byte for byte with their
/// length, and the monitor event gets its request half when the exchange
/// ends — whole for a small body, a bounded head for a large one.
#[tokio::test]
async fn passive_llm_requests_pass_through_and_are_captured_bounded() {
    let up_ca = make_ca();
    let (up_port, mut seen) = spawn_counting_upstream(&up_ca).await;
    let proxy = start_passive(&up_ca).await;

    // A small request is recorded exactly as a buffered one would be.
    let small = br#"{"model":"claude-x","stream":false,"messages":[{"role":"user","content":"ping"}],"tools":[{"name":"lookup"}]}"#;
    let (status, body) = send(
        &proxy,
        up_port,
        post("/v1/messages", up_port)
            .body(Full::new(Bytes::from_static(small)))
            .unwrap(),
    )
    .await;
    assert_eq!(status, 200);
    assert!(String::from_utf8_lossy(&body).contains("pong"));
    let upstream = seen.recv().await.unwrap();
    assert_eq!(upstream.body, small);
    assert_eq!(
        upstream.headers["content-length"],
        small.len().to_string().as_str()
    );
    let events = settled_events(&proxy.store, 1).await;
    let event = &events[0];
    assert_eq!(event["event_type"], "llm_call", "{event}");
    assert_eq!(event["status"], "complete", "{event}");
    assert_eq!(event["data"]["model"], "claude-x", "{event}");
    assert_eq!(event["data"]["message_count"], 1, "{event}");
    assert_eq!(event["data"]["has_tools"], true, "{event}");
    assert_eq!(event["data"]["tool_count"], 1, "{event}");
    assert_eq!(
        event["data"]["request_body"]["model"], "claude-x",
        "{event}"
    );
    assert_eq!(
        event["data"]["raw_request"],
        String::from_utf8_lossy(small).as_ref(),
        "{event}"
    );
    assert_eq!(event["data"]["input_tokens"], 7, "{event}");

    // A long conversation: forwarded whole, captured as a head.
    let conversation = SyntheticConversation::messages(24 * MIB);
    let len = conversation.byte_len();
    assert!(len >= 24 * MIB);
    let (status, _) = send(
        &proxy,
        up_port,
        post("/v1/messages", up_port)
            .body(conversation.body())
            .unwrap(),
    )
    .await;
    assert_eq!(status, 200);
    let upstream = seen.recv().await.unwrap();
    assert_eq!(upstream.len, len);
    assert_eq!(upstream.headers["content-length"], len.to_string().as_str());
    assert!(upstream.headers.get("transfer-encoding").is_none());
    let events = settled_events(&proxy.store, 2).await;
    let event = &events[1];
    assert_eq!(event["status"], "complete", "{event}");
    let raw = event["data"]["raw_request"].as_str().unwrap();
    assert!(raw.starts_with(r#"{"model":"synthetic-model","max_tokens":1024,"messages":["#));
    let captured = 256 * 1024;
    assert!(
        raw.ends_with(&format!("\n… [truncated {} bytes]", len - captured)),
        "{}",
        &raw[raw.len() - 80..]
    );
    assert!(raw.len() < captured as usize + 64);
    // A head is not a document; nothing is parsed from it.
    assert!(event["data"]["request_body"].is_null(), "{event}");
    assert_eq!(event["data"]["message_count"], 0, "{event}");

    // A compressed request keeps its encoding; the monitor does not decode it.
    let encoded = zstd::stream::encode_all(&small[..], 3).unwrap();
    let (status, _) = send(
        &proxy,
        up_port,
        post("/v1/messages", up_port)
            .header("content-encoding", "zstd")
            .body(Full::new(Bytes::from(encoded.clone())))
            .unwrap(),
    )
    .await;
    assert_eq!(status, 200);
    let upstream = seen.recv().await.unwrap();
    assert_eq!(upstream.body, encoded);
    assert_eq!(upstream.headers["content-encoding"], "zstd");
    let events = settled_events(&proxy.store, 3).await;
    let event = &events[2];
    assert_eq!(
        event["data"]["raw_request"],
        format!(
            "[{} bytes forwarded with their Content-Encoding; not decoded]",
            encoded.len()
        ),
        "{event}"
    );
    assert!(event["data"]["request_body"].is_null(), "{event}");
    assert_eq!(proxy.store.pending_requests(), 0);
}

/// A chunked request stays chunked: no length is invented for it.
#[tokio::test]
async fn chunked_requests_pass_through_chunked() {
    let up_ca = make_ca();
    let (up_port, mut seen) = spawn_counting_upstream(&up_ca).await;
    let proxy = start_passive(&up_ca).await;
    let chunks = SyntheticConversation::responses(MIB)
        .chunks()
        .map(|chunk| Ok::<_, Infallible>(hyper::body::Frame::data(chunk)));
    let len = SyntheticConversation::responses(MIB).byte_len();
    let (status, _) = send(
        &proxy,
        up_port,
        post("/backend-api/codex/responses", up_port)
            .body(http_body_util::StreamBody::new(futures::stream::iter(
                chunks,
            )))
            .unwrap(),
    )
    .await;
    assert_eq!(status, 200);
    let upstream = seen.recv().await.unwrap();
    assert_eq!(upstream.len, len);
    assert_eq!(upstream.headers["transfer-encoding"], "chunked");
    assert!(upstream.headers.get("content-length").is_none());
}

/// The upstream may answer before the upload ends (an early 401). The client
/// gets that answer while most of its body is still unsent.
#[tokio::test]
async fn upstream_can_answer_before_a_pass_through_upload_finishes() {
    const TOTAL: u64 = 64 * MIB;
    let up_ca = make_ca();
    let (listener, acceptor, up_port) = tls_listener(&up_ca, HOST).await;
    tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let mut tls = acceptor.accept(tcp).await.unwrap();
        let (_, mut pending) = read_head(&mut tls).await;
        let mut buffer = vec![0u8; 64 * 1024];
        while pending.is_empty() {
            let n = tls.read(&mut buffer).await.unwrap();
            assert!(n > 0);
            pending.extend_from_slice(&buffer[..n]);
        }
        tls.write_all(
            b"HTTP/1.1 401 Unauthorized\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\nno",
        )
        .await
        .unwrap();
        tls.flush().await.unwrap();
        // Keep reading so the answer is not lost to a reset.
        while matches!(tls.read(&mut buffer).await, Ok(n) if n > 0) {}
    });

    let proxy = start_passive(&up_ca).await;
    let mut client = connect(&proxy, up_port).await;
    client
        .write_all(
            format!(
                "POST /v1/messages HTTP/1.1\r\nHost: {HOST}:{up_port}\r\nContent-Type: application/json\r\nContent-Length: {TOTAL}\r\n\r\n"
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    client.write_all(&vec![b' '; BLOCK]).await.unwrap();
    client.flush().await.unwrap();
    // Only 64 KiB of 64 MiB were sent, and the answer is already here.
    let (head, mut body) = tokio::time::timeout(WAIT, read_head(&mut client))
        .await
        .expect("the early response did not reach the client");
    assert!(head.starts_with("http/1.1 401"), "{head}");
    while body.len() < 2 {
        let mut byte = [0u8; 1];
        assert_eq!(client.read(&mut byte).await.unwrap(), 1);
        body.push(byte[0]);
    }
    assert_eq!(body, b"no");

    let events = settled_events(&proxy.store, 1).await;
    let event = &events[0];
    assert_eq!(event["event_type"], "llm_call", "{event}");
    assert_eq!(event["status"], "error", "{event}");
    assert_eq!(event["data"]["status_code"], 401, "{event}");
    assert!(
        event["data"]["raw_request"]
            .as_str()
            .unwrap()
            .ends_with("\n… [request body incomplete]"),
        "{event}"
    );
    assert_eq!(proxy.store.pending_requests(), 0);
}

/// A client that disappears mid-upload: the upstream request is abandoned
/// short of its length, and the pending monitor event is closed.
#[tokio::test]
async fn client_abort_mid_upload_closes_the_pending_event_and_the_upstream() {
    const TOTAL: u64 = 64 * MIB;
    let up_ca = make_ca();
    let (listener, acceptor, up_port) = tls_listener(&up_ca, HOST).await;
    let (first_tx, first_rx) = oneshot::channel::<()>();
    let upstream = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let mut tls = acceptor.accept(tcp).await.unwrap();
        let (_, pending) = read_head(&mut tls).await;
        let mut received = pending.len() as u64;
        let mut buffer = vec![0u8; 64 * 1024];
        let mut first = Some(first_tx);
        loop {
            if received > 0 {
                if let Some(first) = first.take() {
                    first.send(()).unwrap();
                }
            }
            match tls.read(&mut buffer).await {
                Ok(n) if n > 0 => received += n as u64,
                // End of stream or a reset: the proxy gave the request up.
                _ => return received,
            }
        }
    });

    let proxy = start_passive(&up_ca).await;
    let mut client = connect(&proxy, up_port).await;
    client
        .write_all(
            format!(
                "POST /v1/messages HTTP/1.1\r\nHost: {HOST}:{up_port}\r\nContent-Type: application/json\r\nContent-Length: {TOTAL}\r\n\r\n"
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    client.write_all(&vec![b' '; BLOCK]).await.unwrap();
    client.flush().await.unwrap();
    tokio::time::timeout(WAIT, first_rx).await.unwrap().unwrap();
    assert_eq!(proxy.store.pending_requests(), 1);

    drop(client);

    let received = tokio::time::timeout(WAIT, upstream)
        .await
        .expect("the upstream request was left open after the client aborted")
        .unwrap();
    assert!(received < TOTAL, "upstream received {received} bytes");
    let events = settled_events(&proxy.store, 1).await;
    let event = &events[0];
    assert_eq!(event["status"], "error", "{event}");
    let error = event["data"]["error"].as_str().unwrap();
    assert!(
        error.contains("cancelled before response delivery")
            || error.contains("client request body"),
        "{error}"
    );
    assert_eq!(proxy.store.pending_requests(), 0);
}

/// Refuses every LLM request from its head, and must never be asked to
/// evaluate a body.
struct HeadRejecting;

#[async_trait]
impl Firewall for HeadRejecting {
    fn body_policy(&self, _: &RequestHead<'_>) -> BodyPolicy {
        BodyPolicy::Reject(403, "LLM access is disabled for this client".into())
    }
    async fn evaluate(&self, _: &FirewallRequest) -> RequestAction {
        panic!("a request refused from its head must not be evaluated");
    }
}

/// A refusal decided from the head: the upstream is never contacted, a client
/// that is mid-upload still gets to read the refusal, and a client waiting
/// for `100 Continue` is refused without sending its body.
#[tokio::test]
async fn header_time_rejects_answer_without_contacting_the_upstream() {
    let up_ca = make_ca();
    // Never accepted: any connection attempt would sit in the backlog.
    let upstream = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let up_port = upstream.local_addr().unwrap().port();
    let proxy = start_proxy(&up_ca, |store| {
        Arc::new(Mitm(ActiveInterceptor::new(
            PassiveInterceptor::new(store),
            Arc::new(HeadRejecting),
        )))
    })
    .await;

    let conversation = SyntheticConversation::messages(2 * MIB);
    let len = conversation.byte_len();
    let (status, body) = send(
        &proxy,
        up_port,
        post("/v1/messages", up_port)
            .body(conversation.body())
            .unwrap(),
    )
    .await;
    assert_eq!(status, 403);
    let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["error"]["type"], "localrouter_firewall");
    assert_eq!(
        body["error"]["message"],
        "LLM access is disabled for this client"
    );
    let events = settled_events(&proxy.store, 1).await;
    let event = &events[0];
    assert_eq!(event["event_type"], "llm_call", "{event}");
    assert_eq!(event["status"], "error", "{event}");
    assert_eq!(event["data"]["status_code"], 403, "{event}");
    // The refused body was read to its end and only its head was kept.
    let raw = event["data"]["raw_request"].as_str().unwrap();
    assert!(
        raw.ends_with(&format!("\n… [truncated {} bytes]", len - 256 * 1024)),
        "{}",
        &raw[raw.len() - 80..]
    );

    // `Expect: 100-continue`: refused before a single body byte is sent.
    let mut client = connect(&proxy, up_port).await;
    client
        .write_all(
            format!(
                "POST /v1/messages HTTP/1.1\r\nHost: {HOST}:{up_port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nExpect: 100-continue\r\n\r\n",
                64 * MIB
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    client.flush().await.unwrap();
    let (head, _) = tokio::time::timeout(WAIT, read_head(&mut client))
        .await
        .expect("no refusal for a client waiting to continue");
    assert!(head.starts_with("http/1.1 403"), "{head}");

    assert!(
        tokio::time::timeout(Duration::from_millis(100), upstream.accept())
            .await
            .is_err(),
        "a refused request reached the upstream"
    );
    // Both refusals are recorded; the second never had a body to capture.
    let events = settled_events(&proxy.store, 2).await;
    assert_eq!(events[1]["data"]["status_code"], 403, "{}", events[1]);
    assert!(events[1]["data"]["raw_request"].is_null(), "{}", events[1]);
}

/// An interceptor that lets a body pass through and then tries to rewrite it.
struct Contradictory;

#[async_trait]
impl ProxyInterceptor for Contradictory {
    fn on_connect(&self, _: &str, _: &ClientCtx) -> ConnectDecision {
        ConnectDecision::Mitm
    }
    fn body_policy(&self, _: &RequestHead<'_>) -> BodyPolicy {
        BodyPolicy::PassThrough
    }
    async fn on_request(&self, ex: &ObservedExchange) -> RequestAction {
        // The contract of a pass-through request: no body, an empty capture.
        assert!(ex.request_body.is_none());
        assert_eq!(ex.request_capture.as_ref().unwrap().total_len, 0);
        RequestAction::Replace(b"{}".to_vec())
    }
}

/// A rewrite of a body that was never read cannot be honored, and the
/// original must not be forwarded in its place.
#[tokio::test]
async fn rewriting_a_pass_through_request_fails_closed() {
    let up_ca = make_ca();
    let upstream = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let up_port = upstream.local_addr().unwrap().port();
    let proxy = start_proxy(&up_ca, |_| Arc::new(Contradictory)).await;
    let (status, body) = send(
        &proxy,
        up_port,
        post("/v1/messages", up_port)
            .body(Full::new(Bytes::from_static(b"{\"model\":\"m\"}")))
            .unwrap(),
    )
    .await;
    assert_eq!(status, 500);
    assert!(String::from_utf8_lossy(&body).contains("Request policy changed"));
    assert!(
        tokio::time::timeout(Duration::from_millis(100), upstream.accept())
            .await
            .is_err(),
        "the unrewritten request reached the upstream"
    );
}

/// The generators the larger suites build on produce what they claim.
#[test]
fn synthetic_conversations_are_valid_and_sized() {
    for conversation in [
        SyntheticConversation::responses(64 * 1024),
        SyntheticConversation::messages(64 * 1024),
        SyntheticConversation::responses(64 * 1024).model_last(),
        SyntheticConversation::messages(64 * 1024).with_marker(3, "MARKER-TEXT"),
    ] {
        let bytes = conversation.to_vec();
        assert_eq!(bytes.len() as u64, conversation.byte_len());
        assert!(bytes.len() >= 64 * 1024 && bytes.len() < 64 * 1024 + 8 * 1024);
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["model"], "synthetic-model");
        let turns = match conversation.api {
            support::Api::Responses => json["input"].as_array().unwrap(),
            support::Api::Messages => json["messages"].as_array().unwrap(),
        };
        assert_eq!(turns.len(), conversation.turns());
        assert_eq!(turns[0]["role"], "user");
        assert_eq!(turns[1]["role"], "assistant");
        let text = String::from_utf8(bytes).unwrap();
        // Key order is part of the fixture: `model` first unless asked last.
        assert_eq!(text.starts_with("{\"model\""), !conversation.model_last);
        assert_eq!(
            text.ends_with("\"model\":\"synthetic-model\"}"),
            conversation.model_last
        );
        assert_eq!(text.contains("MARKER-TEXT"), conversation.marker.is_some());
        // Escapes and multi-byte text are present in every turn.
        assert!(text.contains("\\\"continue\\\"\\n") && text.contains("日本語"));
    }
    // Sizes scale without the body ever being materialized.
    let large = SyntheticConversation::responses(50 * MIB);
    assert!(large.byte_len() >= 50 * MIB && large.byte_len() < 50 * MIB + 8 * 1024);
}
