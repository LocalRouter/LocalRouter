//! End-to-end tests for **non-LLM** traffic through the inspection proxy.
//!
//! `HTTPS_PROXY` is process-wide, so pointing a tool at LocalRouter also routes
//! that tool's git, package-manager and telemetry traffic here. Such traffic
//! must be forwarded byte-for-byte (nothing may break), and must show up in the
//! monitor as a passthrough carrying the destination but no content.

use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use base64::Engine;
use lr_monitor::{EventStatus, MonitorEventData, MonitorEventStore, MonitorEventType};
use lr_proxy::cert::CertAuthority;
use lr_proxy::passive::PassiveInterceptor;
use lr_proxy::resolver::StaticResolver;
use lr_proxy::tls;
use lr_proxy::ProxyManager;

const CLIENT_ID: &str = "cid";
const SECRET: &str = "lr-secret";

fn auth_header() -> String {
    let b64 = base64::engine::general_purpose::STANDARD.encode(format!("{CLIENT_ID}:{SECRET}"));
    format!("Proxy-Authorization: Basic {b64}\r\n")
}

fn temp_dir(tag: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "lr-proxy-passthrough-{tag}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

/// Stand up the proxy with a real passive interceptor. Nothing here is on the
/// MITM allow-list, so every destination is passthrough traffic.
async fn spawn_proxy(
    tag: &str,
) -> (
    u16,
    Arc<MonitorEventStore>,
    tokio::sync::oneshot::Sender<()>,
) {
    tls::ensure_crypto_provider();
    let ca = Arc::new(CertAuthority::load_or_create(&temp_dir(tag)).unwrap());
    let store = Arc::new(MonitorEventStore::new(64));
    let interceptor = Arc::new(PassiveInterceptor::new(store.clone()));
    let resolver = Arc::new(StaticResolver {
        client_id: CLIENT_ID.to_string(),
        secret: SECRET.to_string(),
        proxy_enabled: true,
    });
    let manager = ProxyManager::new(ca, interceptor, resolver).unwrap();
    let listener = ProxyManager::bind("127.0.0.1", 0).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    tokio::spawn(async move {
        manager
            .serve(listener, async {
                let _ = rx.await;
            })
            .await;
    });
    (port, store, tx)
}

/// Wait until the monitor holds `n` events (recording happens on a spawned task).
async fn wait_for_events(store: &MonitorEventStore, n: usize) {
    for _ in 0..200 {
        if store.list(0, 10, None).total >= n {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("timed out waiting for {n} monitor event(s)");
}

/// Wait until `n` events have left `Pending` — a passthrough event opens while
/// the connection is live and only settles when it closes.
async fn wait_for_settled(store: &MonitorEventStore, n: usize) {
    wait_for_events(store, n).await;
    for _ in 0..200 {
        let listed = store.list(0, 10, None);
        if listed
            .events
            .iter()
            .filter(|e| e.status != EventStatus::Pending)
            .count()
            >= n
        {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("timed out waiting for {n} settled monitor event(s)");
}

/// A plain TCP origin that echoes back whatever it is sent, prefixed with a
/// marker — stands in for "any non-LLM server" behind a blind tunnel.
async fn spawn_echo() -> (u16, TcpListener) {
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = l.local_addr().unwrap().port();
    (port, l)
}

#[tokio::test]
async fn non_llm_connect_is_tunneled_verbatim_and_recorded() {
    let (proxy_port, store, _shutdown) = spawn_proxy("tunnel").await;
    let (echo_port, echo) = spawn_echo().await;
    tokio::spawn(async move {
        let (mut sock, _) = echo.accept().await.unwrap();
        let mut buf = vec![0u8; 64];
        let n = sock.read(&mut buf).await.unwrap();
        sock.write_all(b"echo:").await.unwrap();
        sock.write_all(&buf[..n]).await.unwrap();
        sock.shutdown().await.unwrap();
    });

    let mut client = TcpStream::connect(("127.0.0.1", proxy_port)).await.unwrap();
    let connect = format!(
        "CONNECT 127.0.0.1:{echo_port} HTTP/1.1\r\nHost: 127.0.0.1:{echo_port}\r\n{}\r\n",
        auth_header()
    );
    client.write_all(connect.as_bytes()).await.unwrap();

    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        assert_eq!(client.read(&mut byte).await.unwrap(), 1, "proxy closed");
        head.push(byte[0]);
    }
    assert!(
        String::from_utf8_lossy(&head).contains("200"),
        "tunnel refused: {}",
        String::from_utf8_lossy(&head)
    );

    // Arbitrary (non-HTTP, non-TLS) bytes cross the tunnel untouched.
    client.write_all(b"hello-git").await.unwrap();
    let mut got = Vec::new();
    client.read_to_end(&mut got).await.unwrap();
    assert_eq!(got, b"echo:hello-git");
    drop(client);

    wait_for_settled(&store, 1).await;
    let listed = store.list(0, 10, None);
    assert_eq!(listed.events.len(), 1);
    assert_eq!(
        listed.events[0].event_type,
        MonitorEventType::ProxyPassthrough
    );
    let full = store.get(&listed.events[0].id).unwrap();
    assert_eq!(full.status, EventStatus::Complete);
    match &full.data {
        MonitorEventData::ProxyPassthrough {
            mode,
            host,
            port,
            method,
            path,
            bytes_sent,
            bytes_received,
            note,
            ..
        } => {
            assert_eq!(*mode, lr_monitor::PassthroughMode::Tunnel);
            assert_eq!((host.as_str(), *port), ("127.0.0.1", echo_port));
            // TLS is never terminated on a tunnel, so nothing inside is known.
            assert_eq!((method, path), (&None, &None));
            assert_eq!(*bytes_sent, Some(b"hello-git".len() as u64));
            assert_eq!(*bytes_received, Some(b"echo:hello-git".len() as u64));
            assert!(note.contains("Not an LLM call"));
        }
        other => panic!("unexpected data: {other:?}"),
    }
    // The tunneled payload itself is nowhere in the event.
    let json = serde_json::to_string(&full.data).unwrap();
    assert!(!json.contains("hello-git"), "content leaked: {json}");
}

#[tokio::test]
async fn plain_http_is_forwarded_to_the_origin_and_recorded() {
    let (proxy_port, store, _shutdown) = spawn_proxy("plain").await;
    let (origin_port, origin) = spawn_echo().await;
    let seen = Arc::new(std::sync::Mutex::new(String::new()));
    let seen_w = seen.clone();
    tokio::spawn(async move {
        let (mut sock, _) = origin.accept().await.unwrap();
        let mut head = Vec::new();
        let mut byte = [0u8; 1];
        while !head.ends_with(b"\r\n\r\n") {
            if sock.read(&mut byte).await.unwrap() == 0 {
                break;
            }
            head.push(byte[0]);
        }
        *seen_w.lock().unwrap() = String::from_utf8_lossy(&head).to_string();
        sock.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
            .await
            .unwrap();
        sock.shutdown().await.unwrap();
    });

    let mut client = TcpStream::connect(("127.0.0.1", proxy_port)).await.unwrap();
    let req = format!(
        "GET http://127.0.0.1:{origin_port}/info/refs?service=git-upload-pack HTTP/1.1\r\n\
         Host: 127.0.0.1:{origin_port}\r\n\
         User-Agent: git/2.44\r\n\
         Proxy-Connection: Keep-Alive\r\n{}\r\n",
        auth_header()
    );
    client.write_all(req.as_bytes()).await.unwrap();
    let mut got = String::new();
    client.read_to_string(&mut got).await.unwrap();
    assert!(got.ends_with("ok"), "origin response not relayed: {got}");

    // The origin saw an origin-form request line, its own headers intact, and
    // none of the proxy-only ones.
    let seen = seen.lock().unwrap().clone();
    assert!(
        seen.starts_with("GET /info/refs?service=git-upload-pack HTTP/1.1\r\n"),
        "unexpected request line: {seen}"
    );
    assert!(seen.contains("User-Agent: git/2.44\r\n"), "{seen}");
    assert!(
        !seen.to_lowercase().contains("proxy-authorization"),
        "{seen}"
    );
    assert!(!seen.to_lowercase().contains("proxy-connection"), "{seen}");

    // The event settles when the connection closes — a kept-alive one stays
    // live (and Pending) until the client hangs up.
    drop(client);
    wait_for_settled(&store, 1).await;
    let listed = store.list(0, 10, None);
    let full = store.get(&listed.events[0].id).unwrap();
    match &full.data {
        MonitorEventData::ProxyPassthrough {
            mode,
            host,
            port,
            method,
            path,
            ..
        } => {
            assert_eq!(*mode, lr_monitor::PassthroughMode::Http);
            assert_eq!((host.as_str(), *port), ("127.0.0.1", origin_port));
            assert_eq!(method.as_deref(), Some("GET"));
            // Query stripped: the destination is shown, its parameters are not.
            assert_eq!(path.as_deref(), Some("/info/refs"));
        }
        other => panic!("unexpected data: {other:?}"),
    }
}

#[tokio::test]
async fn unreachable_passthrough_answers_502_and_is_flagged() {
    let (proxy_port, store, _shutdown) = spawn_proxy("unreachable").await;
    // Bind then drop, so the port is (almost certainly) closed.
    let dead = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let dead_port = dead.local_addr().unwrap().port();
    drop(dead);

    let mut client = TcpStream::connect(("127.0.0.1", proxy_port)).await.unwrap();
    let connect = format!(
        "CONNECT 127.0.0.1:{dead_port} HTTP/1.1\r\nHost: 127.0.0.1:{dead_port}\r\n{}\r\n",
        auth_header()
    );
    client.write_all(connect.as_bytes()).await.unwrap();
    let mut got = String::new();
    client.read_to_string(&mut got).await.unwrap();
    assert!(got.contains("502"), "expected a 502, got: {got}");

    wait_for_settled(&store, 1).await;
    let listed = store.list(0, 10, None);
    assert_eq!(listed.events[0].status, EventStatus::Error);
}

#[tokio::test]
async fn unauthenticated_requests_are_still_challenged() {
    let (proxy_port, store, _shutdown) = spawn_proxy("noauth").await;
    let mut client = TcpStream::connect(("127.0.0.1", proxy_port)).await.unwrap();
    client
        .write_all(b"CONNECT github.com:443 HTTP/1.1\r\nHost: github.com:443\r\n\r\n")
        .await
        .unwrap();
    let mut got = String::new();
    client.read_to_string(&mut got).await.unwrap();
    assert!(got.contains("407"), "expected a 407, got: {got}");
    // A challenge is part of the normal auth handshake, not stray traffic.
    assert_eq!(store.list(0, 10, None).total, 0);
}
