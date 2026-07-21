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
    ClientCtx, ConnectDecision, InterceptAction, ObservedExchange, ProxyInterceptor,
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
    async fn on_request(&self, ex: &ObservedExchange) -> InterceptAction<()> {
        self.0.on_request(ex).await
    }
    async fn on_response(&self, ex: &ObservedExchange) -> InterceptAction<()> {
        self.0.on_response(ex).await
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
async fn spawn_upstream(ca: &TestCa, sse: bool) -> (u16, tokio::task::JoinHandle<()>) {
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
            tokio::spawn(async move {
                let Ok(tls) = acceptor.accept(tcp).await else {
                    return;
                };
                let svc = service_fn(move |_req: Request<Incoming>| async move {
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
    let (up_port, _up) = spawn_upstream(&up_ca, sse).await;

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
