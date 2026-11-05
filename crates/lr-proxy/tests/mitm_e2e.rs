//! End-to-end MITM data-path test.
//!
//! Stands up a local TLS "upstream" (standing in for api.anthropic.com), runs
//! the proxy in front of it, and drives a real client through the tunnel:
//! CONNECT → forged-leaf TLS (trusting the proxy's root CA) → HTTP request.
//!
//! Asserts both that the response reaches the client **faithfully** and that the
//! passive interceptor **recorded** the exchange to the monitor — the checks we
//! would otherwise have to do by hand against a live Claude Code + Anthropic.

use std::convert::Infallible;
use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper::{Request, Response};
use hyper_util::rt::TokioIo;
use rcgen::{CertificateParams, DistinguishedName, DnType, KeyPair};
use rustls::{ClientConfig, RootCertStore, ServerConfig};
use rustls_pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::{TlsAcceptor, TlsConnector};

use lr_monitor::MonitorEventStore;
use lr_proxy::cert::CertAuthority;
use lr_proxy::interceptor::{
    ClientCtx, ConnectDecision, ObservedExchange, PassthroughExchange, ProxyInterceptor,
    RequestAction,
};
use lr_proxy::passive::PassiveInterceptor;
use lr_proxy::resolver::StaticResolver;
use lr_proxy::tls;
use lr_proxy::ProxyManager;

const HOST: &str = "localhost";
const CLIENT_ID: &str = "cid";
const SECRET: &str = "lr-secret";

/// Interceptor that forces MITM for the test host, delegating recording to the
/// real passive interceptor so we exercise the true recording path.
struct ForceMitm(PassiveInterceptor);

#[async_trait]
impl ProxyInterceptor for ForceMitm {
    fn on_connect(&self, _host: &str, client: &ClientCtx) -> ConnectDecision {
        if client.proxy_enabled {
            ConnectDecision::Mitm
        } else {
            ConnectDecision::Reject("disabled")
        }
    }
    async fn on_request(&self, ex: &ObservedExchange) -> RequestAction {
        self.0.on_request(ex).await
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

struct TestCa {
    issuer: rcgen::Issuer<'static, KeyPair>,
    ca_der: CertificateDer<'static>,
}

fn make_ca() -> TestCa {
    let key = KeyPair::generate().unwrap();
    let mut params = CertificateParams::new(Vec::new()).unwrap();
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, "Test Upstream CA");
    params.distinguished_name = dn;
    params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    let cert = params.self_signed(&key).unwrap();
    let ca_der = cert.der().clone();
    let issuer = rcgen::Issuer::from_ca_cert_pem(&cert.pem(), key).unwrap();
    TestCa { issuer, ca_der }
}

fn make_leaf(ca: &TestCa, host: &str) -> (CertificateDer<'static>, PrivateKeyDer<'static>) {
    let key = KeyPair::generate().unwrap();
    let params = CertificateParams::new(vec![host.to_string()]).unwrap();
    let cert = params.signed_by(&key, &ca.issuer).unwrap();
    let der = cert.der().clone();
    let pk = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key.serialize_der()));
    (der, pk)
}

/// Spawn a TLS upstream that answers one request with the given response.
/// `seen_ae` records the `Accept-Encoding` header the upstream received (so the
/// test can assert the proxy stripped it).
async fn spawn_upstream(
    ca: &TestCa,
    sse: bool,
    seen_ae: std::sync::Arc<std::sync::Mutex<Option<Option<String>>>>,
) -> (u16, tokio::task::JoinHandle<()>) {
    let (leaf, key) = make_leaf(ca, HOST);
    let mut cfg = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![leaf], key)
        .unwrap();
    cfg.alpn_protocols = vec![b"http/1.1".to_vec()];
    let acceptor = TlsAcceptor::from(Arc::new(cfg));

    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let port = listener.local_addr().unwrap().port();

    let handle = tokio::spawn(async move {
        loop {
            let Ok((tcp, _)) = listener.accept().await else {
                break;
            };
            let acceptor = acceptor.clone();
            let seen_ae = seen_ae.clone();
            tokio::spawn(async move {
                let Ok(tls) = acceptor.accept(tcp).await else {
                    return;
                };
                let svc = service_fn(move |req: Request<Incoming>| {
                    let seen_ae = seen_ae.clone();
                    *seen_ae.lock().unwrap() = Some(
                        req.headers()
                            .get("accept-encoding")
                            .map(|v| v.to_str().unwrap_or("").to_string()),
                    );
                    async move {
                        let resp = if sse {
                            let body = "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":11}}}\n\nevent: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"delta\":{\"text\":\"Hi\"}}\n\nevent: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":4}}\n\n";
                            Response::builder()
                                .header("content-type", "text/event-stream")
                                .body(Full::new(Bytes::from(body)))
                                .unwrap()
                        } else {
                            let body = r#"{"content":[{"type":"text","text":"pong"}],"stop_reason":"end_turn","usage":{"input_tokens":7,"output_tokens":2}}"#;
                            Response::builder()
                                .header("content-type", "application/json")
                                .body(Full::new(Bytes::from(body)))
                                .unwrap()
                        };
                        Ok::<_, Infallible>(resp)
                    }
                });
                hyper::server::conn::http1::Builder::new()
                    .serve_connection(TokioIo::new(tls), svc)
                    .await
                    .ok();
            });
        }
    });
    (port, handle)
}

fn client_root_store(proxy_ca_pem: &str) -> RootCertStore {
    let mut roots = RootCertStore::empty();
    let mut rd = std::io::BufReader::new(proxy_ca_pem.as_bytes());
    for cert in rustls_pemfile::certs(&mut rd) {
        roots.add(cert.unwrap()).unwrap();
    }
    roots
}

/// Read the proxy's CONNECT response (up to the blank line), byte by byte.
async fn read_connect_response(stream: &mut TcpStream) -> String {
    let mut buf = Vec::new();
    let mut b = [0u8; 1];
    loop {
        let n = stream.read(&mut b).await.unwrap();
        assert!(n != 0, "proxy closed before CONNECT response");
        buf.push(b[0]);
        if buf.ends_with(b"\r\n\r\n") {
            break;
        }
    }
    String::from_utf8_lossy(&buf).to_string()
}

async fn run_case(sse: bool) -> (u16, Vec<u8>, usize) {
    tls::ensure_crypto_provider();

    // --- test upstream (stands in for api.anthropic.com) ---
    let up_ca = make_ca();
    let seen_ae = std::sync::Arc::new(std::sync::Mutex::new(None));
    let (up_port, _up) = spawn_upstream(&up_ca, sse, seen_ae.clone()).await;

    // proxy validates the upstream against the test CA.
    let mut up_roots = RootCertStore::empty();
    up_roots.add(up_ca.ca_der.clone()).unwrap();

    // --- proxy ---
    let dir = std::env::temp_dir().join(format!(
        "lr-proxy-e2e-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let ca = Arc::new(CertAuthority::load_or_create(&dir).unwrap());
    let proxy_ca_pem = ca.ca_pem().to_string();

    let store = Arc::new(MonitorEventStore::new(64));
    let interceptor = Arc::new(ForceMitm(PassiveInterceptor::new(store.clone())));
    let resolver = Arc::new(StaticResolver {
        client_id: CLIENT_ID.to_string(),
        secret: SECRET.to_string(),
        proxy_enabled: true,
    });

    let manager = ProxyManager::with_upstream_roots(ca, interceptor, resolver, up_roots).unwrap();
    let proxy_listener = ProxyManager::bind("127.0.0.1", 0).await.unwrap();
    let proxy_port = proxy_listener.local_addr().unwrap().port();
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    tokio::spawn(async move {
        manager
            .serve(proxy_listener, async {
                let _ = shutdown_rx.await;
            })
            .await;
    });

    // --- client: CONNECT localhost:up_port through the proxy ---
    let mut stream = TcpStream::connect(("127.0.0.1", proxy_port)).await.unwrap();
    use base64::Engine;
    let auth = base64::engine::general_purpose::STANDARD.encode(format!("{CLIENT_ID}:{SECRET}"));
    let connect = format!(
        "CONNECT {HOST}:{up_port} HTTP/1.1\r\nHost: {HOST}:{up_port}\r\nProxy-Authorization: Basic {auth}\r\n\r\n"
    );
    stream.write_all(connect.as_bytes()).await.unwrap();
    let resp = read_connect_response(&mut stream).await;
    assert!(resp.contains("200"), "CONNECT failed: {resp}");

    // --- client TLS, trusting the proxy's root CA, then send the request ---
    let mut cc = ClientConfig::builder()
        .with_root_certificates(client_root_store(&proxy_ca_pem))
        .with_no_client_auth();
    cc.alpn_protocols = vec![b"http/1.1".to_vec()];
    let connector = TlsConnector::from(Arc::new(cc));
    let server_name = ServerName::try_from(HOST).unwrap();
    let tls = connector.connect(server_name, stream).await.unwrap();

    let (mut sender, conn) = hyper::client::conn::http1::handshake(TokioIo::new(tls))
        .await
        .unwrap();
    tokio::spawn(async move {
        let _ = conn.await;
    });

    let req = Request::builder()
        .method("POST")
        .uri("/v1/messages")
        .header("host", format!("{HOST}:{up_port}"))
        .header("content-type", "application/json")
        // The client asks for compression; the proxy must strip it so it can
        // read (and we can assert on) an uncompressed upstream response.
        .header("accept-encoding", "gzip, br")
        .body(Full::new(Bytes::from(
            r#"{"model":"claude-sonnet-4-20250514","messages":[{"role":"user","content":"ping"}]}"#,
        )))
        .unwrap();
    let response = sender.send_request(req).await.unwrap();
    let status = response.status().as_u16();
    let body = response
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes()
        .to_vec();

    // Give the on-end recording task a moment to run.
    for _ in 0..50 {
        if store.list(0, 10, None).total > 0 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let total = store.list(0, 10, None).total;

    // The proxy must have stripped Accept-Encoding before forwarding upstream.
    assert_eq!(
        *seen_ae.lock().unwrap(),
        Some(None),
        "proxy should strip Accept-Encoding so responses are inspectable"
    );

    let _ = shutdown_tx.send(());
    (status, body, total)
}

#[tokio::test]
async fn non_streaming_exchange_is_faithful_and_recorded() {
    let (status, body, recorded) = run_case(false).await;
    assert_eq!(status, 200);
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    // Client received the upstream response verbatim.
    assert_eq!(json["content"][0]["text"], "pong");
    assert_eq!(json["usage"]["input_tokens"], 7);
    // And the proxy recorded exactly one monitor event.
    assert_eq!(recorded, 1, "expected the exchange to be recorded once");
}

/// Spawn a TLS upstream that accepts a websocket upgrade on any path, reads
/// one client message, and answers with `response.created` + a full
/// `response.completed` — the Codex Responses-over-websocket shape.
/// `seen_ext` records the `Sec-WebSocket-Extensions` header the upstream saw
/// (the proxy must strip it so no compression is ever negotiated).
async fn spawn_ws_upstream(
    ca: &TestCa,
    seen_ext: std::sync::Arc<std::sync::Mutex<Option<Option<String>>>>,
    protected: bool,
) -> (u16, tokio::task::JoinHandle<()>) {
    use lr_proxy::websocket::{encode_frame, FrameDecoder, WsMessage};

    let (leaf, key) = make_leaf(ca, HOST);
    let mut cfg = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![leaf], key)
        .unwrap();
    cfg.alpn_protocols = vec![b"http/1.1".to_vec()];
    let acceptor = TlsAcceptor::from(Arc::new(cfg));

    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let port = listener.local_addr().unwrap().port();

    let handle = tokio::spawn(async move {
        loop {
            let Ok((tcp, _)) = listener.accept().await else {
                break;
            };
            let acceptor = acceptor.clone();
            let seen_ext = seen_ext.clone();
            tokio::spawn(async move {
                let Ok(tls) = acceptor.accept(tcp).await else {
                    return;
                };
                let svc = service_fn(move |mut req: Request<Incoming>| {
                    let seen_ext = seen_ext.clone();
                    *seen_ext.lock().unwrap() = Some(
                        req.headers()
                            .get("sec-websocket-extensions")
                            .map(|v| v.to_str().unwrap_or("").to_string()),
                    );
                    let on_upgrade = hyper::upgrade::on(&mut req);
                    tokio::spawn(async move {
                        let Ok(upgraded) = on_upgrade.await else {
                            return;
                        };
                        let mut io = TokioIo::new(upgraded);
                        let mut decoder = FrameDecoder::new();
                        let mut buf = [0u8; 8192];
                        // Wait for the client's request message.
                        let request = 'outer: loop {
                            let Ok(n) = io.read(&mut buf).await else {
                                return;
                            };
                            if n == 0 {
                                return;
                            }
                            for msg in decoder.push(&buf[..n]) {
                                if let WsMessage::Text(text) = msg {
                                    break 'outer text;
                                }
                            }
                        };
                        // Answer with created + completed (unmasked: server→client).
                        let created = r#"{"type":"response.created","response":{"id":"resp_ws","model":"gpt-5.5"}}"#;
                        let completed = r#"{"type":"response.completed","response":{"id":"resp_ws","model":"gpt-5.5","status":"completed","output":[{"type":"message","content":[{"type":"output_text","text":"pong-ws"}]}],"usage":{"input_tokens":21,"output_tokens":5}}}"#;
                        let mut frames = encode_frame(&WsMessage::Text(created.into()), false);
                        let completed = if protected {
                            let request: serde_json::Value =
                                serde_json::from_str(&request).unwrap();
                            assert!(!request.to_string().contains(SYNTHETIC_KEY));
                            let content = request["input"][0]["content"].as_str().unwrap();
                            let tag = content
                                .strip_prefix("aws_secret_access_key=")
                                .unwrap()
                                .to_ascii_uppercase();
                            for fragment in [&tag[..11], &tag[11..]] {
                                let delta = serde_json::json!({"type":"response.output_text.delta","item_id":"msg","content_index":0,"delta":fragment});
                                frames.extend(encode_frame(
                                    &WsMessage::Text(delta.to_string()),
                                    false,
                                ));
                            }
                            let mut value: serde_json::Value =
                                serde_json::from_str(completed).unwrap();
                            value["response"]["output"][0]["content"][0]["text"] =
                                serde_json::json!(tag);
                            value.to_string()
                        } else {
                            completed.to_owned()
                        };
                        frames.extend(encode_frame(&WsMessage::Text(completed), false));
                        let _ = io.write_all(&frames).await;
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
                    .serve_connection(TokioIo::new(tls), svc)
                    .with_upgrades()
                    .await
                    .ok();
            });
        }
    });
    (port, handle)
}

/// Drive a Codex-style websocket exchange through the proxy end-to-end:
/// CONNECT → forged-leaf TLS → HTTP upgrade → masked request frame →
/// server frames teed back verbatim → one monitor event per cycle.
async fn websocket_case(protected: bool) {
    use lr_proxy::websocket::{encode_frame, FrameDecoder, WsMessage};

    tls::ensure_crypto_provider();

    let up_ca = make_ca();
    let seen_ext = std::sync::Arc::new(std::sync::Mutex::new(None));
    let (up_port, _up) = spawn_ws_upstream(&up_ca, seen_ext.clone(), protected).await;

    let mut up_roots = RootCertStore::empty();
    up_roots.add(up_ca.ca_der.clone()).unwrap();

    let dir = std::env::temp_dir().join(format!(
        "lr-proxy-ws-e2e-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let ca = Arc::new(CertAuthority::load_or_create(&dir).unwrap());
    let proxy_ca_pem = ca.ca_pem().to_string();

    let store = Arc::new(MonitorEventStore::new(64));
    let interceptor: Arc<dyn ProxyInterceptor> = if protected {
        Arc::new(ProtectingMitm(lr_proxy::active::ActiveInterceptor::new(
            PassiveInterceptor::new(store.clone()),
            Arc::new(ProtectingFirewall),
        )))
    } else {
        Arc::new(ForceMitm(PassiveInterceptor::new(store.clone())))
    };
    let resolver = Arc::new(StaticResolver {
        client_id: CLIENT_ID.to_string(),
        secret: SECRET.to_string(),
        proxy_enabled: true,
    });

    let manager = ProxyManager::with_upstream_roots(ca, interceptor, resolver, up_roots).unwrap();
    let proxy_listener = ProxyManager::bind("127.0.0.1", 0).await.unwrap();
    let proxy_port = proxy_listener.local_addr().unwrap().port();
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    tokio::spawn(async move {
        manager
            .serve(proxy_listener, async {
                let _ = shutdown_rx.await;
            })
            .await;
    });

    // CONNECT through the proxy, then TLS with the proxy's forged leaf.
    let mut stream = TcpStream::connect(("127.0.0.1", proxy_port)).await.unwrap();
    use base64::Engine;
    let auth = base64::engine::general_purpose::STANDARD.encode(format!("{CLIENT_ID}:{SECRET}"));
    let connect = format!(
        "CONNECT {HOST}:{up_port} HTTP/1.1\r\nHost: {HOST}:{up_port}\r\nProxy-Authorization: Basic {auth}\r\n\r\n"
    );
    stream.write_all(connect.as_bytes()).await.unwrap();
    let resp = read_connect_response(&mut stream).await;
    assert!(resp.contains("200"), "CONNECT failed: {resp}");

    let mut cc = ClientConfig::builder()
        .with_root_certificates(client_root_store(&proxy_ca_pem))
        .with_no_client_auth();
    cc.alpn_protocols = vec![b"http/1.1".to_vec()];
    let connector = TlsConnector::from(Arc::new(cc));
    let server_name = ServerName::try_from(HOST).unwrap();
    let mut tls = connector.connect(server_name, stream).await.unwrap();

    // Raw HTTP upgrade request (what tungstenite sends), with a compression
    // extension the proxy must strip before forwarding.
    let upgrade = format!(
        "GET /backend-api/codex/responses HTTP/1.1\r\n\
         Host: {HOST}:{up_port}\r\n\
         Connection: Upgrade\r\n\
         Upgrade: websocket\r\n\
         Sec-WebSocket-Version: 13\r\n\
         Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
         Sec-WebSocket-Extensions: permessage-deflate\r\n\r\n"
    );
    tls.write_all(upgrade.as_bytes()).await.unwrap();

    // Read the 101 response head.
    let mut head = Vec::new();
    let mut b = [0u8; 1];
    loop {
        let n = tls.read(&mut b).await.unwrap();
        assert!(n != 0, "proxy closed before upgrade response");
        head.push(b[0]);
        if head.ends_with(b"\r\n\r\n") {
            break;
        }
    }
    let head = String::from_utf8_lossy(&head).to_string();
    assert!(
        head.starts_with("HTTP/1.1 101"),
        "expected 101, got: {head}"
    );
    assert!(
        head.to_ascii_lowercase().contains("sec-websocket-accept"),
        "upstream's accept header must pass through: {head}"
    );

    // Send the request message (masked, as clients must).
    let request = r#"{"type":"response.create","model":"gpt-5.5","stream":true,"input":[{"role":"user","content":[{"type":"input_text","text":"ping"}]}]}"#;
    let request = if protected {
        serde_json::json!({"type":"response.create","model":"gpt-5.5","input":[{"role":"user","content":format!("aws_secret_access_key={SYNTHETIC_KEY}")}]}).to_string()
    } else {
        request.to_owned()
    };
    tls.write_all(&encode_frame(&WsMessage::Text(request), true))
        .await
        .unwrap();

    // Read both server messages back through the proxy.
    let mut decoder = FrameDecoder::new();
    let mut messages = Vec::new();
    let mut buf = [0u8; 8192];
    while !messages
        .iter()
        .any(|msg| matches!(msg, WsMessage::Text(text) if text.contains("response.completed")))
    {
        let n = tls.read(&mut buf).await.unwrap();
        assert!(n != 0, "connection closed before both events arrived");
        messages.extend(decoder.push(&buf[..n]));
    }
    let WsMessage::Text(last) = messages.last().unwrap() else {
        panic!("expected text frame");
    };
    assert!(last.contains("response.completed"));
    assert!(last.contains(if protected { SYNTHETIC_KEY } else { "pong-ws" }));
    if protected {
        let text: String = messages
            .iter()
            .filter_map(|msg| {
                if let WsMessage::Text(text) = msg {
                    serde_json::from_str::<serde_json::Value>(text).ok()
                } else {
                    None
                }
            })
            .filter(|v| v["type"] == "response.output_text.delta")
            .filter_map(|v| v["delta"].as_str().map(str::to_owned))
            .collect();
        assert_eq!(text, SYNTHETIC_KEY);
    }

    // Close the websocket; the relay then finalizes the cycle event.
    drop(tls);
    let mut recorded = 0;
    for _ in 0..50 {
        recorded = store.list(0, 10, None).total;
        if recorded > 0 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(recorded, 1, "expected one monitor event for the ws cycle");
    let summary = &store.list(0, 10, None).events[0];
    let event = store.get(&summary.id).unwrap();
    assert_eq!(event.status, lr_monitor::EventStatus::Complete);
    let lr_monitor::MonitorEventData::LlmCall {
        model,
        input_tokens,
        output_tokens,
        status_code,
        content_preview,
        ..
    } = &event.data
    else {
        panic!("expected LlmCall event");
    };
    assert_eq!(model, "gpt-5.5");
    assert_eq!(*input_tokens, Some(21));
    assert_eq!(*output_tokens, Some(5));
    assert_eq!(*status_code, Some(200));
    if protected {
        assert!(content_preview
            .as_ref()
            .unwrap()
            .to_ascii_lowercase()
            .contains("localrouter-redacted"));
        assert!(!serde_json::to_string(&event)
            .unwrap()
            .contains(SYNTHETIC_KEY));
    } else {
        assert_eq!(content_preview.as_deref(), Some("pong-ws"));
    }

    // The proxy must have stripped the compression extension offer.
    assert_eq!(
        *seen_ext.lock().unwrap(),
        Some(None),
        "proxy should strip Sec-WebSocket-Extensions so frames stay plaintext"
    );

    let _ = shutdown_tx.send(());
}

#[tokio::test]
async fn websocket_exchange_is_relayed_and_recorded() {
    websocket_case(false).await;
}
#[tokio::test]
async fn codex_websocket_restores_fragmented_uppercase_references_over_tls() {
    tokio::time::timeout(std::time::Duration::from_secs(15), websocket_case(true))
        .await
        .unwrap();
}

/// A websocket on a path that carries no LLM traffic is relayed without
/// inspection, and shows up as a passthrough carrying the destination only.
#[tokio::test]
async fn non_llm_websocket_is_relayed_as_a_passthrough() {
    use lr_proxy::websocket::{encode_frame, FrameDecoder, WsMessage};

    tls::ensure_crypto_provider();

    let up_ca = make_ca();
    let seen_ext = std::sync::Arc::new(std::sync::Mutex::new(None));
    let (up_port, _up) = spawn_ws_upstream(&up_ca, seen_ext, false).await;
    let mut up_roots = RootCertStore::empty();
    up_roots.add(up_ca.ca_der.clone()).unwrap();

    let dir = std::env::temp_dir().join(format!(
        "lr-proxy-ws-passthrough-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let ca = Arc::new(CertAuthority::load_or_create(&dir).unwrap());
    let proxy_ca_pem = ca.ca_pem().to_string();

    let store = Arc::new(MonitorEventStore::new(64));
    let interceptor = Arc::new(ForceMitm(PassiveInterceptor::new(store.clone())));
    let resolver = Arc::new(StaticResolver {
        client_id: CLIENT_ID.to_string(),
        secret: SECRET.to_string(),
        proxy_enabled: true,
    });
    let manager = ProxyManager::with_upstream_roots(ca, interceptor, resolver, up_roots).unwrap();
    let proxy_listener = ProxyManager::bind("127.0.0.1", 0).await.unwrap();
    let proxy_port = proxy_listener.local_addr().unwrap().port();
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    tokio::spawn(async move {
        manager
            .serve(proxy_listener, async {
                let _ = shutdown_rx.await;
            })
            .await;
    });

    let mut stream = TcpStream::connect(("127.0.0.1", proxy_port)).await.unwrap();
    use base64::Engine;
    let auth = base64::engine::general_purpose::STANDARD.encode(format!("{CLIENT_ID}:{SECRET}"));
    let connect = format!(
        "CONNECT {HOST}:{up_port} HTTP/1.1\r\nHost: {HOST}:{up_port}\r\nProxy-Authorization: Basic {auth}\r\n\r\n"
    );
    stream.write_all(connect.as_bytes()).await.unwrap();
    assert!(read_connect_response(&mut stream).await.contains("200"));

    let mut cc = ClientConfig::builder()
        .with_root_certificates(client_root_store(&proxy_ca_pem))
        .with_no_client_auth();
    cc.alpn_protocols = vec![b"http/1.1".to_vec()];
    let connector = TlsConnector::from(Arc::new(cc));
    let mut tls = connector
        .connect(ServerName::try_from(HOST).unwrap(), stream)
        .await
        .unwrap();

    // Not an LLM endpoint — telemetry, notifications, whatever the tool does.
    let upgrade = format!(
        "GET /telemetry/socket?token=secret HTTP/1.1\r\n\
         Host: {HOST}:{up_port}\r\n\
         Connection: Upgrade\r\n\
         Upgrade: websocket\r\n\
         Sec-WebSocket-Version: 13\r\n\
         Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n"
    );
    tls.write_all(upgrade.as_bytes()).await.unwrap();
    let mut head = Vec::new();
    let mut b = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        assert!(tls.read(&mut b).await.unwrap() != 0, "proxy closed");
        head.push(b[0]);
    }
    assert!(String::from_utf8_lossy(&head).starts_with("HTTP/1.1 101"));

    // Frames still cross verbatim.
    tls.write_all(&encode_frame(&WsMessage::Text("hello".into()), true))
        .await
        .unwrap();
    let mut decoder = FrameDecoder::new();
    let mut messages = Vec::new();
    let mut buf = [0u8; 8192];
    while messages.is_empty() {
        let n = tls.read(&mut buf).await.unwrap();
        assert!(n != 0, "connection closed before a reply arrived");
        messages.extend(decoder.push(&buf[..n]));
    }
    drop(tls);

    let mut event = None;
    for _ in 0..100 {
        let listed = store.list(0, 10, None);
        if let Some(e) = listed
            .events
            .iter()
            .find(|e| e.status != lr_monitor::EventStatus::Pending)
        {
            event = store.get(&e.id);
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let event = event.expect("passthrough event settles when the relay ends");
    assert_eq!(
        event.event_type,
        lr_monitor::MonitorEventType::ProxyPassthrough
    );
    let lr_monitor::MonitorEventData::ProxyPassthrough {
        mode, host, path, ..
    } = &event.data
    else {
        panic!("expected a ProxyPassthrough event, got {:?}", event.data);
    };
    assert_eq!(*mode, lr_monitor::PassthroughMode::Websocket);
    assert_eq!(host, HOST);
    // Query stripped — the destination is shown, its parameters are not.
    assert_eq!(path.as_deref(), Some("/telemetry/socket"));
    let json = serde_json::to_string(&event.data).unwrap();
    assert!(!json.contains("hello"), "frame content leaked: {json}");
    assert!(!json.contains("secret"), "query leaked: {json}");

    let _ = shutdown_tx.send(());
}

#[tokio::test]
async fn streaming_sse_exchange_is_faithful_and_recorded() {
    let (status, body, recorded) = run_case(true).await;
    assert_eq!(status, 200);
    let text = String::from_utf8(body).unwrap();
    // Client received the full SSE stream.
    assert!(text.contains("message_start"));
    assert!(text.contains("content_block_delta"));
    assert_eq!(recorded, 1, "expected the SSE exchange to be recorded once");
}

const SYNTHETIC_KEY: &str = "6nmCEa00cbNmH0B4zKPSYOPP5z1oYFR26doJldQr";

/// Runs the actual active interception + scanner + native response data path.
struct ProtectingFirewall;
#[async_trait]
impl lr_proxy::active::Firewall for ProtectingFirewall {
    async fn evaluate(&self, req: &lr_proxy::active::FirewallRequest) -> RequestAction {
        use lr_types::secret_context::SecretContext;
        let vault = Arc::new(lr_secret_scanner::placeholders::Vault::new(
            Arc::new(lr_api_keys::MockKeychain::new()),
            &req.client_id,
            req.secret_session.as_deref().unwrap_or("isolated"),
            true,
        ));
        let engine =
            lr_secret_scanner::SecretScanEngine::new(&lr_secret_scanner::SecretScanEngineConfig {
                entropy_threshold: 3.0,
                allowlist: vec![],
                scan_system_messages: true,
            })
            .unwrap();
        let texts = vec![lr_secret_scanner::ExtractedText {
            label: "request".into(),
            text: req.body.to_string(),
            message_index: 0,
        }];
        let findings = engine.scan(&texts).findings;
        assert!(!findings.is_empty());
        for finding in findings {
            vault.register(&finding.raw_text).unwrap();
        }
        let mut body = req.body.clone();
        vault.protect(&mut body).unwrap();
        vault.set_request_contract(&body).unwrap();
        assert!(!body.to_string().contains(SYNTHETIC_KEY));
        RequestAction::Protect {
            body: serde_json::to_vec(&body).unwrap(),
            vault,
        }
    }
    fn protects_secrets(&self, _: &str) -> bool {
        true
    }
}
struct ProtectingMitm(lr_proxy::active::ActiveInterceptor);
#[async_trait]
impl ProxyInterceptor for ProtectingMitm {
    fn on_connect(&self, _: &str, _: &ClientCtx) -> ConnectDecision {
        ConnectDecision::Mitm
    }
    async fn on_request(&self, ex: &ObservedExchange) -> RequestAction {
        self.0.on_request(ex).await
    }
    fn begin(&self, ex: &ObservedExchange) -> Option<String> {
        self.0.begin(ex)
    }
    async fn on_response(&self, ex: &ObservedExchange) {
        self.0.on_response(ex).await
    }
}

async fn protected_roundtrip(sse: bool, compact: bool) {
    use serde_json::{json, Value};
    tls::ensure_crypto_provider();
    let up_ca = make_ca();
    let (leaf, key) = make_leaf(&up_ca, HOST);
    let acceptor = TlsAcceptor::from(Arc::new(
        ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(vec![leaf], key)
            .unwrap(),
    ));
    let up = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let up_port = up.local_addr().unwrap().port();
    let upstream = tokio::spawn(async move {
        let (tcp, _) = up.accept().await.unwrap();
        let tls = acceptor.accept(tcp).await.unwrap();
        let service = service_fn(move |req: Request<Incoming>| async move {
            assert_eq!(
                req.uri().path(),
                if compact {
                    "/backend-api/codex/responses/compact"
                } else {
                    "/backend-api/codex/responses"
                }
            );
            let bytes = req.into_body().collect().await.unwrap().to_bytes();
            let raw = String::from_utf8(bytes.to_vec()).unwrap();
            assert!(!raw.contains(SYNTHETIC_KEY));
            let body: Value = serde_json::from_str(&raw).unwrap();
            let content = body["input"][0]["content"].as_str().unwrap();
            let tag = content.strip_prefix("aws_secret_access_key=").unwrap();
            assert!(tag.starts_with("<localrouter-redacted"));
            let args = json!({"content":tag}).to_string();
            let mut response = json!({"id":"resp_native","object":"response","model":"local-test","output":[{"id":"fc_test","type":"function_call","name":"write_file","arguments":args},{"id":"msg","type":"message","role":"assistant","content":[{"type":"output_text","text":tag}]}]});
            if compact {
                response["output"].as_array_mut().unwrap().push(
                    json!({"type":"compaction","encrypted_content":"provider-owned-compact-state"}),
                );
            }
            let payload = if sse {
                let mut events = vec![
                    json!({"type":"response.created","response":{"id":"resp_native"}}),
                    json!({"type":"response.output_item.added","item":{"id":"fc_test","type":"function_call","name":"write_file","arguments":""}}),
                ];
                for fragment in tag.as_bytes().chunks(1) {
                    events.push(json!({"type":"response.output_text.delta","item_id":"msg","content_index":0,"delta":std::str::from_utf8(fragment).unwrap()}));
                }
                for fragment in args.as_bytes().chunks(2) {
                    events.push(json!({"type":"response.function_call_arguments.delta","item_id":"fc_test","delta":std::str::from_utf8(fragment).unwrap()}));
                }
                events.push(json!({"type":"response.completed","response":response}));
                events
                    .iter()
                    .map(|e| format!("data: {e}\r\n\r\n"))
                    .collect::<String>()
            } else {
                response.to_string()
            };
            // Split actual HTTP body bytes, including inside event and JSON boundaries.
            let chunks = payload
                .as_bytes()
                .chunks(7)
                .map(|b| {
                    Ok::<_, std::io::Error>(hyper::body::Frame::data(Bytes::copy_from_slice(b)))
                })
                .collect::<Vec<_>>();
            let response = Response::builder()
                .header(
                    "content-type",
                    if sse {
                        "text/event-stream"
                    } else {
                        "application/json"
                    },
                )
                .body(http_body_util::StreamBody::new(futures::stream::iter(
                    chunks,
                )))
                .unwrap();
            Ok::<_, Infallible>(response)
        });
        hyper::server::conn::http1::Builder::new()
            .serve_connection(TokioIo::new(tls), service)
            .await
            .ok();
    });
    let mut roots = RootCertStore::empty();
    roots.add(up_ca.ca_der).unwrap();
    let dir = std::env::temp_dir().join(format!("lr-proxy-protected-{}", uuid::Uuid::new_v4()));
    let ca = Arc::new(CertAuthority::load_or_create(&dir).unwrap());
    let pem = ca.ca_pem().to_owned();
    let store = Arc::new(MonitorEventStore::new(64));
    let interceptor = Arc::new(ProtectingMitm(lr_proxy::active::ActiveInterceptor::new(
        PassiveInterceptor::new(store.clone()),
        Arc::new(ProtectingFirewall),
    )));
    let resolver = Arc::new(StaticResolver {
        client_id: CLIENT_ID.into(),
        secret: SECRET.into(),
        proxy_enabled: true,
    });
    let manager = ProxyManager::with_upstream_roots(ca, interceptor, resolver, roots).unwrap();
    let listener = ProxyManager::bind("127.0.0.1", 0).await.unwrap();
    let proxy_port = listener.local_addr().unwrap().port();
    let task =
        tokio::spawn(async move { manager.serve(listener, std::future::pending::<()>()).await });
    let mut tcp = TcpStream::connect(("127.0.0.1", proxy_port)).await.unwrap();
    use base64::Engine;
    let auth = base64::engine::general_purpose::STANDARD.encode(format!("{CLIENT_ID}:{SECRET}"));
    tcp.write_all(format!("CONNECT {HOST}:{up_port} HTTP/1.1\r\nHost: {HOST}:{up_port}\r\nProxy-Authorization: Basic {auth}\r\n\r\n").as_bytes()).await.unwrap();
    assert!(read_connect_response(&mut tcp).await.contains("200"));
    let cc = ClientConfig::builder()
        .with_root_certificates(client_root_store(&pem))
        .with_no_client_auth();
    let tls = TlsConnector::from(Arc::new(cc))
        .connect(ServerName::try_from(HOST).unwrap(), tcp)
        .await
        .unwrap();
    let (mut sender, conn) = hyper::client::conn::http1::handshake(TokioIo::new(tls))
        .await
        .unwrap();
    let conn_task = tokio::spawn(async move {
        conn.await.ok();
    });
    let body = json!({"model":"local-test","input":[{"role":"user","content":format!("aws_secret_access_key={SYNTHETIC_KEY}")}],"tools":[{"type":"function","name":"write_file","parameters":{"type":"object","properties":{"content":{"type":"string"}},"required":["content"]}}],"stream":sse});
    let req = Request::builder()
        .method("POST")
        .uri(if compact {
            "/backend-api/codex/responses/compact"
        } else {
            "/backend-api/codex/responses"
        })
        .header("host", format!("{HOST}:{up_port}"))
        .header("session_id", "synthetic-conversation")
        .header("x-localrouter-trace", "test-hop;hop=2")
        .body(Full::new(Bytes::from(body.to_string())))
        .unwrap();
    let response = sender.send_request(req).await.unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["cache-control"], "no-store");
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let final_response = if sse {
        let raw = String::from_utf8(bytes.to_vec()).unwrap();
        let events = lr_proxy::wire::sse_json_events(&raw);
        let text: String = events
            .iter()
            .filter(|e| e["type"] == "response.output_text.delta")
            .filter_map(|e| e["delta"].as_str())
            .collect();
        assert_eq!(text, SYNTHETIC_KEY);
        let args: String = events
            .iter()
            .filter(|e| e["type"] == "response.function_call_arguments.delta")
            .filter_map(|e| e["delta"].as_str())
            .collect();
        assert_eq!(
            serde_json::from_str::<Value>(&args).unwrap()["content"],
            SYNTHETIC_KEY
        );
        events.last().unwrap()["response"].clone()
    } else {
        serde_json::from_slice::<Value>(&bytes).unwrap()
    };
    let args: Value =
        serde_json::from_str(final_response["output"][0]["arguments"].as_str().unwrap()).unwrap();
    let file = dir.join(".env");
    std::fs::write(&file, args["content"].as_str().unwrap()).unwrap();
    assert_eq!(std::fs::read_to_string(file).unwrap(), SYNTHETIC_KEY);
    for _ in 0..100 {
        if store
            .list(0, 10, None)
            .events
            .iter()
            .any(|e| e.status != lr_monitor::EventStatus::Pending)
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    let recorded = serde_json::to_string(&store.list(0, 10, None)).unwrap();
    assert!(!recorded.contains(SYNTHETIC_KEY));
    assert!(recorded.contains("localrouter-redacted"));
    task.abort();
    conn_task.abort();
    upstream.abort();
    std::fs::remove_dir_all(dir).unwrap();
}
#[tokio::test]
async fn codex_http_restores_tool_and_text_and_records_only_masked_data() {
    protected_roundtrip(false, false).await;
}
#[tokio::test]
async fn codex_sse_fragmented_tools_and_text_survive_real_tls_and_http_framing() {
    protected_roundtrip(true, false).await;
}

#[tokio::test]
async fn codex_compaction_masks_input_and_preserves_opaque_state_over_tls() {
    protected_roundtrip(false, true).await;
}
