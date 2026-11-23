//! Rate-limit admission of proxied requests, through the real AppFirewall:
//! concurrent requests against a strategy limit on the MITM (buffered and
//! streamed bodies), WebSocket and reverse-proxy paths, release of the
//! reservation however an exchange ends, and requests that loop through the
//! native server and the proxy. Every upstream is a local fixture.
use super::{secret_policy_tests::fixture, AppFirewall};
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::{body::Incoming, service::service_fn, Request, Response};
use hyper_util::rt::TokioIo;
use lr_proxy::{
    active::{ActiveInterceptor, Firewall, FirewallRequest, StreamRequest},
    cert::CertAuthority,
    interceptor::{
        BodyPolicy, ClientCtx, ConnectDecision, ExchangeSource, ObservedExchange, ProxyInterceptor,
        RequestAction, RequestHead, StreamOpen, StreamPolicy,
    },
    passive::PassiveInterceptor,
    resolver::ClientResolver,
    websocket::{encode_frame, relay, FrameDecoder, WsMessage, WsSession},
    wire::WireFormat,
    ProxyManager,
};
use serde_json::{json, Value};
use std::{
    convert::Infallible,
    sync::{
        atomic::{AtomicU16, AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

// ---- fixtures --------------------------------------------------------------

/// The shared fixture with Secret Scan off and, when given, a strategy limit
/// of `limit` requests per minute.
fn limited(root: &std::path::Path, limit: Option<f64>) -> (Arc<AppFirewall>, FirewallRequest) {
    let (firewall, mut request) = fixture(
        root,
        lr_config::LlmMode::Proxy,
        Arc::new(lr_api_keys::MockKeychain::new()),
    );
    firewall
        .config_manager
        .update(|c| {
            c.secret_scanning.action = lr_config::SecretScanAction::Off;
            if let Some(limit) = limit {
                c.strategies[0].rate_limits = vec![lr_config::StrategyRateLimit {
                    limit_type: lr_config::RateLimitType::Requests,
                    value: limit,
                    time_window: lr_config::RateLimitTimeWindow::Minute,
                    enabled: true,
                }];
            }
        })
        .unwrap();
    request.body = json!({"model":"local-test","input":"hello"});
    (Arc::new(firewall), request)
}

fn strategy_id(firewall: &AppFirewall) -> String {
    firewall.config_manager.get().strategies[0].id.clone()
}

/// Requests the strategy's admitted exchanges hold in flight.
fn in_flight(firewall: &AppFirewall) -> u64 {
    firewall
        .state
        .rate_limiter
        .strategy_in_flight(&strategy_id(firewall))
        .requests
}

/// Requests recorded for the strategy in the last minute.
fn recorded(firewall: &AppFirewall) -> u64 {
    firewall
        .metrics
        .get_recent_usage_for_strategy(&strategy_id(firewall), 60)
        .0
}

/// A copy of `request` with its own admission slot and `trace`.
fn copy(request: &FirewallRequest, trace: Option<&str>) -> FirewallRequest {
    FirewallRequest {
        client_id: request.client_id.clone(),
        session_id: request.session_id.clone(),
        secret_session: request.secret_session.clone(),
        host: request.host.clone(),
        method: request.method.clone(),
        path: request.path.clone(),
        provider: request.provider.clone(),
        model: request.model.clone(),
        has_tools: request.has_tools,
        message_count: request.message_count,
        body: request.body.clone(),
        trace: trace.map(|t| lr_types::RequestTrace::parse(t).unwrap()),
        admission: Default::default(),
    }
}

fn stream_request(request: &FirewallRequest) -> StreamRequest {
    StreamRequest {
        client_id: request.client_id.clone(),
        session_id: request.session_id.clone(),
        secret_session: None,
        host: request.host.clone(),
        method: request.method.clone(),
        path: request.path.clone(),
        provider: request.provider.clone(),
        model: request.model.clone(),
        format: WireFormat::OpenAiResponses,
        preview: None,
        trace: None,
        admission: Default::default(),
    }
}

fn stream_policy() -> StreamPolicy {
    StreamPolicy {
        masking: lr_proxy::interceptor::StreamMasking::Off,
        needs_model: false,
        whole_document: false,
        lag: lr_proxy::interceptor::DEFAULT_STREAM_LAG,
        restore_only: false,
        strict: false,
        incremental: false,
    }
}

/// The production interceptor, recording into the firewall's metrics.
fn interceptor(firewall: &Arc<AppFirewall>) -> Arc<ActiveInterceptor> {
    Arc::new(ActiveInterceptor::new(
        PassiveInterceptor::new(Arc::new(lr_monitor::MonitorEventStore::new(256)))
            .with_metrics(firewall.metrics.clone()),
        firewall.clone(),
    ))
}

/// Wait (up to ten seconds) until `done` holds.
async fn until(what: &str, done: impl Fn() -> bool) {
    for _ in 0..1000 {
        if done() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("timed out waiting until {what}");
}

fn rejected(action: &RequestAction, status: u16) -> bool {
    matches!(action, RequestAction::Reject { status: s, .. } if *s == status)
}

// ---- local upstream ----------------------------------------------------------

/// A provider that holds every response until it is opened.
struct Upstream {
    arrived: AtomicUsize,
    status: AtomicU16,
    open: tokio::sync::watch::Sender<bool>,
}

impl Upstream {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            arrived: AtomicUsize::new(0),
            status: AtomicU16::new(200),
            open: tokio::sync::watch::channel(false).0,
        })
    }
    fn arrived(&self) -> usize {
        self.arrived.load(Ordering::SeqCst)
    }
    fn open(&self) {
        self.open.send_replace(true);
    }

    async fn respond(
        self: Arc<Self>,
        request: Request<Incoming>,
    ) -> Result<Response<Full<Bytes>>, Infallible> {
        // The whole body arrives before the provider answers.
        let _ = request.into_body().collect().await;
        self.arrived.fetch_add(1, Ordering::SeqCst);
        let mut open = self.open.subscribe();
        let _ = open.wait_for(|open| *open).await;
        let status = self.status.load(Ordering::SeqCst);
        let body = json!({
            "id": "chatcmpl-1",
            "model": "local-test",
            "choices": [{"index":0,"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}],
            "usage": {"prompt_tokens": 10, "completion_tokens": 2},
        });
        Ok(Response::builder()
            .status(status)
            .header("content-type", "application/json")
            .body(Full::new(Bytes::from(body.to_string())))
            .unwrap())
    }
}

/// Serve `upstream` over plain HTTP; returns its port.
async fn serve_plain(upstream: Arc<Upstream>) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        loop {
            let (tcp, _) = listener.accept().await.unwrap();
            let upstream = upstream.clone();
            tokio::spawn(async move {
                hyper::server::conn::http1::Builder::new()
                    .serve_connection(
                        TokioIo::new(tcp),
                        service_fn(move |r| upstream.clone().respond(r)),
                    )
                    .await
                    .ok();
            });
        }
    });
    port
}

/// Serve `upstream` over TLS for `localhost`; returns its port and the
/// roots that trust it.
async fn serve_tls(upstream: Arc<Upstream>) -> (u16, rustls::RootCertStore) {
    lr_proxy::tls::ensure_crypto_provider();
    let key = rcgen::KeyPair::generate().unwrap();
    let cert = rcgen::CertificateParams::new(vec!["localhost".into()])
        .unwrap()
        .self_signed(&key)
        .unwrap();
    let mut roots = rustls::RootCertStore::empty();
    roots.add(cert.der().clone()).unwrap();
    let key = rustls_pki_types::PrivateKeyDer::Pkcs8(rustls_pki_types::PrivatePkcs8KeyDer::from(
        key.serialize_der(),
    ));
    let tls = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert.der().clone()], key)
        .unwrap();
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(tls));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        loop {
            let (tcp, _) = listener.accept().await.unwrap();
            let acceptor = acceptor.clone();
            let upstream = upstream.clone();
            tokio::spawn(async move {
                let Ok(tls) = acceptor.accept(tcp).await else {
                    return;
                };
                hyper::server::conn::http1::Builder::new()
                    .serve_connection(
                        TokioIo::new(tls),
                        service_fn(move |r| upstream.clone().respond(r)),
                    )
                    .await
                    .ok();
            });
        }
    });
    (port, roots)
}

// ---- MITM proxy --------------------------------------------------------------

/// The production interceptor, inspecting the local fixture host only.
struct LocalOnly(Arc<ActiveInterceptor>);
#[async_trait::async_trait]
impl ProxyInterceptor for LocalOnly {
    fn on_connect(&self, host: &str, _: &ClientCtx) -> ConnectDecision {
        if host == "localhost" {
            ConnectDecision::Mitm
        } else {
            ConnectDecision::Reject("fixture permits localhost only")
        }
    }
    fn body_policy(&self, head: &RequestHead<'_>) -> BodyPolicy {
        self.0.body_policy(head)
    }
    async fn on_request(&self, ex: &ObservedExchange) -> RequestAction {
        self.0.on_request(ex).await
    }
    async fn open_stream(
        &self,
        ex: &ObservedExchange,
        metadata: &lr_secret_scanner::request_stream::RequestMetadata,
        policy: &StreamPolicy,
    ) -> StreamOpen {
        self.0.open_stream(ex, metadata, policy).await
    }
    fn begin(&self, ex: &ObservedExchange) -> Option<String> {
        self.0.begin(ex)
    }
    async fn on_response(&self, ex: &ObservedExchange) {
        self.0.on_response(ex).await
    }
}

/// Authenticates the fixture client with its strategy, as the app does.
struct Resolver {
    client_id: String,
    strategy_id: String,
}
impl ClientResolver for Resolver {
    fn resolve(&self, client_id: &str, secret: &str) -> Option<ClientCtx> {
        (client_id == self.client_id && secret == "synthetic-proxy").then(|| ClientCtx {
            client_id: self.client_id.clone(),
            strategy_id: self.strategy_id.clone(),
            proxy_enabled: true,
        })
    }
}

/// Start the MITM proxy and return a client configured to use it.
async fn mitm_client(
    root: &std::path::Path,
    firewall: &Arc<AppFirewall>,
    client_id: &str,
    roots: rustls::RootCertStore,
) -> reqwest::Client {
    let ca = Arc::new(CertAuthority::load_or_create(&root.join("ca")).unwrap());
    let ca_pem = ca.ca_pem().to_string();
    let resolver = Arc::new(Resolver {
        client_id: client_id.to_owned(),
        strategy_id: strategy_id(firewall),
    });
    let manager = ProxyManager::with_upstream_roots(
        ca,
        Arc::new(LocalOnly(interceptor(firewall))),
        resolver,
        roots,
    )
    .unwrap();
    let listener = ProxyManager::bind("127.0.0.1", 0).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        manager.serve(listener, std::future::pending()).await;
    });
    reqwest::Client::builder()
        .no_proxy()
        .proxy(
            reqwest::Proxy::all(format!(
                "http://{client_id}:synthetic-proxy@127.0.0.1:{port}"
            ))
            .unwrap(),
        )
        .add_root_certificate(reqwest::Certificate::from_pem(ca_pem.as_bytes()).unwrap())
        .build()
        .unwrap()
}

fn chat_body() -> Value {
    json!({"model":"local-test","messages":[{"role":"user","content":"hello"}]})
}

/// Send `n` copies of `body` to `url` at once; their statuses, once all
/// answered.
fn send_all(
    client: &reqwest::Client,
    url: &str,
    body: &Value,
    n: usize,
) -> Vec<tokio::task::JoinHandle<u16>> {
    (0..n)
        .map(|_| {
            let request = client.post(url).json(body);
            tokio::spawn(async move { request.send().await.unwrap().status().as_u16() })
        })
        .collect()
}

async fn statuses(handles: Vec<tokio::task::JoinHandle<u16>>) -> Vec<u16> {
    let mut statuses = Vec::new();
    for handle in handles {
        statuses.push(handle.await.unwrap());
    }
    statuses.sort_unstable();
    statuses
}

// ---- the firewall alone ------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_evaluations_admit_exactly_the_limit() {
    let root = tempfile::tempdir().unwrap();
    let (firewall, request) = limited(root.path(), Some(3.0));
    let requests: Vec<_> = (0..12).map(|_| Arc::new(copy(&request, None))).collect();
    let decisions = futures::future::join_all(requests.iter().map(|request| {
        let firewall = firewall.clone();
        let request = request.clone();
        async move { firewall.evaluate(&request).await }
    }))
    .await;
    let admitted = decisions
        .iter()
        .filter(|d| !matches!(d, RequestAction::Reject { .. }))
        .count();
    assert_eq!(admitted, 3);
    assert_eq!(decisions.iter().filter(|d| rejected(d, 429)).count(), 9);
    assert_eq!(in_flight(&firewall), 3);
    // Only the admitted ones hold a share; it ends with the exchange.
    assert_eq!(requests.iter().filter(|r| r.admission.is_held()).count(), 3);
    drop(decisions);
    drop(requests);
    assert_eq!(in_flight(&firewall), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_stream_opens_admit_exactly_the_limit() {
    let root = tempfile::tempdir().unwrap();
    let (firewall, request) = limited(root.path(), Some(2.0));
    let requests: Vec<_> = (0..8).map(|_| Arc::new(stream_request(&request))).collect();
    let opened = futures::future::join_all(requests.iter().map(|request| {
        let firewall = firewall.clone();
        let request = request.clone();
        async move {
            matches!(
                firewall.open_stream(&request, &stream_policy()).await,
                StreamOpen::Stream { .. }
            )
        }
    }))
    .await;
    assert_eq!(opened.iter().filter(|o| **o).count(), 2);
    assert_eq!(in_flight(&firewall), 2);
    drop(requests);
    assert_eq!(in_flight(&firewall), 0);
}

#[tokio::test]
async fn header_time_refusal_matches_the_reservations() {
    let root = tempfile::tempdir().unwrap();
    let (firewall, request) = limited(root.path(), Some(1.0));
    let headers = hyper::HeaderMap::new();
    let head = |trace: Option<&'static lr_types::RequestTrace>| RequestHead {
        client_id: &request.client_id,
        strategy_id: "",
        host: &request.host,
        port: 443,
        method: &request.method,
        path: &request.path,
        headers: &headers,
        trace,
        provider_override: None,
        source: ExchangeSource::Proxy,
    };
    let held = copy(&request, None);
    assert!(!rejected(&firewall.evaluate(&held).await, 429));
    // The held share fills the limit: the head and the body agree.
    assert_eq!(
        firewall.body_policy(&head(None)),
        BodyPolicy::Reject(429, "Rate limit exceeded (requests)".into())
    );
    assert!(rejected(
        &firewall.evaluate(&copy(&request, None)).await,
        429
    ));
    // A duplicate hop of the held request is not refused by its own share.
    let duplicate: &'static lr_types::RequestTrace = Box::leak(Box::new(
        lr_types::RequestTrace::parse("abc;hop=2").unwrap(),
    ));
    assert!(!matches!(
        firewall.body_policy(&head(Some(duplicate))),
        BodyPolicy::Reject(..)
    ));
    drop(held);
    assert!(!matches!(
        firewall.body_policy(&head(None)),
        BodyPolicy::Reject(..)
    ));
}

#[tokio::test]
async fn retrievals_reserve_nothing() {
    let root = tempfile::tempdir().unwrap();
    let (firewall, request) = limited(root.path(), Some(1.0));
    let mut retrieval = copy(&request, None);
    retrieval.method = "GET".into();
    retrieval.path = "/backend-api/codex/responses/resp_stored".into();
    retrieval.body = Value::Null;
    assert!(!matches!(
        firewall.evaluate(&retrieval).await,
        RequestAction::Reject { .. }
    ));
    assert!(!retrieval.admission.is_held());
    assert_eq!(in_flight(&firewall), 0);
}

/// A request whose second hop is marked as a duplicate holds one share,
/// whichever path it entered first. Only a trusted in-process trace marks a
/// duplicate: a trace header on the wire always starts a new hop
/// (`RequestTrace::outbound_for`), so a forged header cannot skip a limit.
#[tokio::test]
async fn a_request_looping_through_both_paths_is_admitted_once() {
    let root = tempfile::tempdir().unwrap();
    let (firewall, request) = limited(root.path(), Some(1.0));
    let router = firewall.state.router.clone();

    // Native first: the gateway holds the only share; the proxied duplicate
    // hop passes and holds nothing, a fresh proxied request is refused.
    let native = router
        .admit_direct_request(&request.client_id)
        .await
        .unwrap();
    assert_eq!(in_flight(&firewall), 1);
    let duplicate = copy(&request, Some("loop;hop=2"));
    assert!(!matches!(
        firewall.evaluate(&duplicate).await,
        RequestAction::Reject { .. }
    ));
    assert!(!duplicate.admission.is_held());
    assert_eq!(in_flight(&firewall), 1);
    assert!(rejected(
        &firewall.evaluate(&copy(&request, None)).await,
        429
    ));
    drop(native);
    assert_eq!(in_flight(&firewall), 0);

    // Proxy first: the proxied request holds the share; the native duplicate
    // hop passes without one, a fresh native request is refused.
    let proxied = copy(&request, Some("loop2;hop=1"));
    assert!(!matches!(
        firewall.evaluate(&proxied).await,
        RequestAction::Reject { .. }
    ));
    assert_eq!(in_flight(&firewall), 1);
    let looped = lr_types::with_outbound_trace(
        lr_types::RequestTrace::parse("loop2;hop=2"),
        router.admit_direct_request(&request.client_id),
    )
    .await
    .expect("the duplicate hop is admitted");
    assert_eq!(in_flight(&firewall), 1);
    assert!(router
        .admit_direct_request(&request.client_id)
        .await
        .is_err());
    drop(looped);
    drop(proxied);
    assert_eq!(in_flight(&firewall), 0);
}

#[tokio::test]
async fn client_limits_are_reserved_and_charged_with_recorded_usage() {
    use lr_proxy::interceptor::RecordedUsage;
    use lr_router::rate_limit::{RateLimitType, RateLimiter};
    let root = tempfile::tempdir().unwrap();
    let (firewall, request) = limited(root.path(), None);
    let limiter = firewall.state.rate_limiter.clone();
    limiter.add_api_key_limiters(
        request.client_id.clone(),
        vec![RateLimiter::new(RateLimitType::Requests, 2.0, 60)],
    );
    let first = copy(&request, None);
    let second = copy(&request, None);
    assert!(!rejected(&firewall.evaluate(&first).await, 429));
    assert!(!rejected(&firewall.evaluate(&second).await, 429));
    let third = copy(&request, None);
    match firewall.evaluate(&third).await {
        RequestAction::Reject {
            status,
            body,
            retry_after,
            ..
        } => {
            assert_eq!(status, 429);
            assert!(String::from_utf8_lossy(&body).contains("Rate limit exceeded (client)"));
            // When room comes back, as the native router answers it: the
            // full window here (the two requests are still in flight).
            assert_eq!(retry_after, Some(60));
        }
        _ => panic!("the client's limit is full"),
    }
    assert_eq!(limiter.api_key_in_flight(&request.client_id).requests, 2);

    // Billed usage replaces the first share: still two counted.
    first.admission.settle(&RecordedUsage {
        billed: true,
        input_tokens: 10,
        output_tokens: 2,
        cost_usd: 0.0,
    });
    assert_eq!(limiter.api_key_in_flight(&request.client_id).requests, 1);
    assert!(rejected(
        &firewall.evaluate(&copy(&request, None)).await,
        429
    ));
    // An unbilled exchange (it failed) gives its room back.
    second.admission.settle(&RecordedUsage::default());
    assert_eq!(limiter.api_key_in_flight(&request.client_id).requests, 0);
    assert!(!rejected(
        &firewall.evaluate(&copy(&request, None)).await,
        429
    ));
}

// ---- MITM proxy end to end ---------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_buffered_requests_through_the_proxy_admit_exactly_the_limit() {
    let root = tempfile::tempdir().unwrap();
    let (firewall, request) = limited(root.path(), Some(3.0));
    let upstream = Upstream::new();
    let (port, roots) = serve_tls(upstream.clone()).await;
    let client = mitm_client(root.path(), &firewall, &request.client_id, roots).await;
    let url = format!("https://localhost:{port}/v1/chat/completions");

    let sent = send_all(&client, &url, &chat_body(), 8);
    until("three requests reached the provider", || {
        upstream.arrived() == 3
    })
    .await;
    assert_eq!(in_flight(&firewall), 3);
    upstream.open();
    let mut expected = vec![200; 3];
    expected.extend([429; 5]);
    assert_eq!(statuses(sent).await, expected);
    assert_eq!(upstream.arrived(), 3);

    // Each share ends once its metrics row stands for it; refusals are no
    // requests against the limit.
    until("every share is released", || in_flight(&firewall) == 0).await;
    assert_eq!(recorded(&firewall), 3);
    let status = client.post(&url).json(&chat_body()).send().await.unwrap();
    assert_eq!(status.status().as_u16(), 429);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_forged_trace_header_does_not_skip_the_limit() {
    let root = tempfile::tempdir().unwrap();
    let (firewall, request) = limited(root.path(), Some(1.0));
    let upstream = Upstream::new();
    let (port, roots) = serve_tls(upstream.clone()).await;
    let client = mitm_client(root.path(), &firewall, &request.client_id, roots).await;
    let url = format!("https://localhost:{port}/v1/chat/completions");
    let held = send_all(&client, &url, &chat_body(), 1);
    until("the request reached the provider", || {
        upstream.arrived() == 1
    })
    .await;
    let forged = client
        .post(&url)
        .header(lr_types::TRACE_HEADER, "forged;hop=9")
        .json(&chat_body())
        .send()
        .await
        .unwrap();
    assert_eq!(forged.status().as_u16(), 429);
    upstream.open();
    assert_eq!(statuses(held).await, vec![200]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_streamed_requests_through_the_proxy_admit_exactly_the_limit() {
    let root = tempfile::tempdir().unwrap();
    let (firewall, request) = limited(root.path(), Some(1.0));
    let upstream = Upstream::new();
    let (port, roots) = serve_tls(upstream.clone()).await;
    let client = mitm_client(root.path(), &firewall, &request.client_id, roots).await;
    let url = format!("https://localhost:{port}/v1/responses");
    // Above the stream lag, so each body is offered to `open_stream`.
    let body = json!({
        "model": "local-test",
        "input": "a".repeat(lr_proxy::interceptor::DEFAULT_STREAM_LAG + 1024 * 1024),
    });

    let sent = send_all(&client, &url, &body, 3);
    until("one request reached the provider", || {
        upstream.arrived() == 1
    })
    .await;
    assert_eq!(in_flight(&firewall), 1);
    upstream.open();
    assert_eq!(statuses(sent).await, vec![200, 429, 429]);
    until("the share is released", || in_flight(&firewall) == 0).await;
    assert_eq!(recorded(&firewall), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_client_that_hangs_up_releases_its_share() {
    let root = tempfile::tempdir().unwrap();
    let (firewall, request) = limited(root.path(), Some(1.0));
    let upstream = Upstream::new();
    let (port, roots) = serve_tls(upstream.clone()).await;
    let client = mitm_client(root.path(), &firewall, &request.client_id, roots).await;
    let url = format!("https://localhost:{port}/v1/chat/completions");
    let sent = send_all(&client, &url, &chat_body(), 1);
    until("the request reached the provider", || {
        upstream.arrived() == 1
    })
    .await;
    assert_eq!(in_flight(&firewall), 1);
    for handle in sent {
        handle.abort();
    }
    until("the share is released", || in_flight(&firewall) == 0).await;
    // The abandoned call reached the provider: it counts as a failed request.
    until("the failure is recorded", || recorded(&firewall) == 1).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn upstream_failures_release_their_share() {
    let root = tempfile::tempdir().unwrap();
    let (firewall, request) = limited(root.path(), Some(5.0));
    let upstream = Upstream::new();
    upstream.status.store(500, Ordering::SeqCst);
    let (port, roots) = serve_tls(upstream.clone()).await;
    let client = mitm_client(root.path(), &firewall, &request.client_id, roots).await;

    // The provider answers with an error.
    let url = format!("https://localhost:{port}/v1/chat/completions");
    let sent = send_all(&client, &url, &chat_body(), 1);
    until("the request reached the provider", || {
        upstream.arrived() == 1
    })
    .await;
    assert_eq!(in_flight(&firewall), 1);
    upstream.open();
    assert_eq!(statuses(sent).await, vec![500]);
    until("the share is released", || in_flight(&firewall) == 0).await;

    // The provider cannot be reached at all.
    let closed = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let closed_port = closed.local_addr().unwrap().port();
    drop(closed);
    let url = format!("https://localhost:{closed_port}/v1/chat/completions");
    assert_eq!(
        statuses(send_all(&client, &url, &chat_body(), 1)).await,
        vec![502]
    );
    until("the share is released", || in_flight(&firewall) == 0).await;
    until("both failures are recorded", || recorded(&firewall) == 2).await;
}

// ---- WebSocket ---------------------------------------------------------------

/// One relayed websocket connection: the client's and the upstream's ends.
struct Connection {
    client: tokio::io::DuplexStream,
    upstream: tokio::io::DuplexStream,
}

fn connect(
    interceptor: &Arc<ActiveInterceptor>,
    firewall: &AppFirewall,
    client_id: &str,
) -> Connection {
    let session = Arc::new(WsSession::new(
        interceptor.clone(),
        WireFormat::OpenAiResponses,
        ObservedExchange {
            client_id: client_id.to_owned(),
            strategy_id: strategy_id(firewall),
            host: "chatgpt.com".into(),
            method: "GET".into(),
            path: "/backend-api/codex/responses".into(),
            ..Default::default()
        },
    ));
    let (client, proxy_client) = tokio::io::duplex(64 * 1024);
    let (upstream, proxy_upstream) = tokio::io::duplex(64 * 1024);
    tokio::spawn(relay(proxy_client, proxy_upstream, Some(session)));
    Connection { client, upstream }
}

/// The next text message from `side`.
async fn next_text(side: &mut tokio::io::DuplexStream) -> String {
    let mut decoder = FrameDecoder::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = side.read(&mut buf).await.unwrap();
        assert!(n > 0, "connection closed");
        if let Some(WsMessage::Text(text)) = decoder.push(&buf[..n]).pop() {
            return text;
        }
    }
}

fn response_create() -> String {
    json!({"type":"response.create","model":"local-test","input":[{"role":"user","content":"hi"}]})
        .to_string()
}

fn completed(id: &str) -> String {
    json!({"type":"response.completed","response":{"id":id,"model":"local-test","status":"completed",
        "output":[],"usage":{"input_tokens":12,"output_tokens":3}}})
    .to_string()
}

/// Whether the message was forwarded upstream (true) or refused to the
/// client (false).
async fn forwarded(connection: &mut Connection) -> bool {
    tokio::select! {
        _ = next_text(&mut connection.upstream) => true,
        reply = next_text(&mut connection.client) => {
            assert!(reply.contains("Rate limit exceeded"), "{reply}");
            false
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_websocket_requests_admit_exactly_the_limit() {
    let root = tempfile::tempdir().unwrap();
    let (firewall, request) = limited(root.path(), Some(2.0));
    let interceptor = interceptor(&firewall);
    let mut connections: Vec<_> = (0..5)
        .map(|_| connect(&interceptor, &firewall, &request.client_id))
        .collect();
    for connection in &mut connections {
        connection
            .client
            .write_all(&encode_frame(&WsMessage::Text(response_create()), true))
            .await
            .unwrap();
    }
    let mut admitted = Vec::new();
    for (i, connection) in connections.iter_mut().enumerate() {
        if forwarded(connection).await {
            admitted.push(i);
        }
    }
    assert_eq!(admitted.len(), 2);
    assert_eq!(in_flight(&firewall), 2);

    // Each cycle's share ends when its terminal event is recorded.
    for (n, i) in admitted.iter().enumerate() {
        let connection = &mut connections[*i];
        connection
            .upstream
            .write_all(&encode_frame(
                &WsMessage::Text(completed(&format!("resp_{n}"))),
                false,
            ))
            .await
            .unwrap();
        next_text(&mut connection.client).await;
    }
    until("every share is released", || in_flight(&firewall) == 0).await;
    assert_eq!(recorded(&firewall), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn each_websocket_message_holds_its_own_share() {
    let root = tempfile::tempdir().unwrap();
    let (firewall, request) = limited(root.path(), Some(2.0));
    let interceptor = interceptor(&firewall);
    let mut connection = connect(&interceptor, &firewall, &request.client_id);
    for expected in [true, true, false] {
        connection
            .client
            .write_all(&encode_frame(&WsMessage::Text(response_create()), true))
            .await
            .unwrap();
        assert_eq!(forwarded(&mut connection).await, expected);
    }
    assert_eq!(in_flight(&firewall), 2);
    // A connection that ends with responses outstanding releases them.
    drop(connection);
    until("every share is released", || in_flight(&firewall) == 0).await;
}

// ---- reverse proxy -----------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_reverse_proxy_requests_admit_exactly_the_limit() {
    let root = tempfile::tempdir().unwrap();
    let (firewall, request) = limited(root.path(), Some(2.0));
    let upstream = Upstream::new();
    let upstream_port = serve_plain(upstream.clone()).await;
    let reverse = Arc::new(
        lr_proxy::reverse::ReverseProxy::new(
            &format!("http://127.0.0.1:{upstream_port}"),
            lr_proxy::reverse::ReverseClient {
                client_id: request.client_id.clone(),
                strategy_id: strategy_id(&firewall),
                provider_instance: Some("local".into()),
            },
            interceptor(&firewall),
        )
        .unwrap(),
    );
    let listener = lr_proxy::reverse::ReverseProxy::bind("127.0.0.1", 0)
        .await
        .unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(reverse.serve(listener, std::future::pending()));

    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let url = format!("http://127.0.0.1:{port}/v1/chat/completions");
    let sent = send_all(&client, &url, &chat_body(), 6);
    until("two requests reached the provider", || {
        upstream.arrived() == 2
    })
    .await;
    assert_eq!(in_flight(&firewall), 2);
    upstream.open();
    assert_eq!(statuses(sent).await, vec![200, 200, 429, 429, 429, 429]);
    until("every share is released", || in_flight(&firewall) == 0).await;
    assert_eq!(recorded(&firewall), 2);
}
