//! Push Relay gateway foundations, driven through the real gateway against
//! wiremock HTTP upstreams and a scripted stdio upstream:
//! - `events/*` are method-not-found without a relay and routed to it with one
//! - push capabilities and instructions come only from the relay; upstream
//!   `experimental` and the events extension are never forwarded on their own
//! - upstream capabilities are remembered once, persisted only on change, and
//!   a changed server identity invalidates them
//! - `initialize` waits (bounded) for servers it has never seen
//! - upstream channel notifications reach the relay; upstream streams and
//!   plain requests are available on a session's connection

use async_trait::async_trait;
use chrono::Utc;
use futures_util::StreamExt;
use localrouter::config::{
    AppConfig, ConfigManager, McpServerConfig, McpTransportConfig, McpTransportType,
};
use localrouter::mcp::gateway::push_relay::{
    PushAdvertisement, PushRelayService, PushRelaySessionContext, UpstreamPushNotification,
    CLAUDE_CHANNEL_CAPABILITY, EVENTS_EXTENSION_ID,
};
use localrouter::mcp::gateway::{GatewayConfig, McpGateway};
use localrouter::mcp::protocol::{JsonRpcNotification, JsonRpcRequest, JsonRpcResponse};
use localrouter::mcp::transport::RpcStreamMessage;
use localrouter::mcp::McpServerManager;
use localrouter::monitoring::metrics::MetricsCollector;
use localrouter::monitoring::storage::MetricsDatabase;
use localrouter::providers::registry::ProviderRegistry;
use localrouter::router::{RateLimiterManager, Router};
use lr_config::{McpPermissions, PermissionState, UpstreamCapabilitySnapshot};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use wiremock::{
    matchers::method as http_method, Match, Mock, MockServer, Request, ResponseTemplate,
};

const SERVER_ID: &str = "push-server";

type Broadcast = tokio::sync::broadcast::Sender<(String, JsonRpcNotification)>;

fn test_router() -> Arc<Router> {
    let config_manager = Arc::new(ConfigManager::new(
        AppConfig::default(),
        std::path::PathBuf::from("/tmp/test_push_relay_router.yaml"),
    ));
    let metrics_db = Arc::new(
        MetricsDatabase::new(std::env::temp_dir().join(format!(
            "test_push_relay_metrics_{}.db",
            uuid::Uuid::new_v4()
        )))
        .unwrap(),
    );
    Arc::new(Router::new(
        config_manager,
        Arc::new(ProviderRegistry::new()),
        Arc::new(RateLimiterManager::new(None)),
        Arc::new(MetricsCollector::new(metrics_db)),
        Arc::new(lr_router::FreeTierManager::new(None)),
    ))
}

struct JsonRpcMethod(&'static str);

impl Match for JsonRpcMethod {
    fn matches(&self, request: &Request) -> bool {
        serde_json::from_slice::<Value>(&request.body)
            .ok()
            .and_then(|body| body.get("method")?.as_str().map(|m| m == self.0))
            .unwrap_or(false)
    }
}

async fn mock(server: &MockServer, method: &'static str, result: Value, delay: Duration) {
    Mock::given(http_method("POST"))
        .and(JsonRpcMethod(method))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"jsonrpc": "2.0", "id": 1, "result": result}))
                .set_delay(delay),
        )
        .mount(server)
        .await;
}

/// An HTTP upstream declaring MCP Events (top-level and extension), Claude
/// channels and an unknown capability; `initialize` answers after `delay`.
async fn push_upstream(delay: Duration) -> MockServer {
    let server = MockServer::start().await;
    mock(
        &server,
        "initialize",
        json!({
            "protocolVersion": "2025-11-25",
            "capabilities": {
                "tools": {},
                "events": {"listChanged": true},
                "extensions": {EVENTS_EXTENSION_ID: {"listChanged": true}},
                "experimental": {CLAUDE_CHANNEL_CAPABILITY: {}},
                "futureCapability": {}
            },
            "serverInfo": {"name": "push", "version": "1.0"}
        }),
        delay,
    )
    .await;
    mock(
        &server,
        "events/list",
        json!({"events": [
            {"name": "message.received", "description": "A message", "delivery": ["push", "poll", "webhook"]},
            {"name": "hook.only", "delivery": ["webhook"]}
        ]}),
        Duration::ZERO,
    )
    .await;
    mock(&server, "tools/list", json!({"tools": []}), Duration::ZERO).await;
    mock(
        &server,
        "resources/list",
        json!({"resources": []}),
        Duration::ZERO,
    )
    .await;
    mock(
        &server,
        "prompts/list",
        json!({"prompts": []}),
        Duration::ZERO,
    )
    .await;
    server
}

fn http_server(id: &str, url: String) -> McpServerConfig {
    McpServerConfig {
        id: id.to_string(),
        name: "Push Server".to_string(),
        transport: McpTransportType::HttpSse,
        transport_config: McpTransportConfig::HttpSse {
            url,
            headers: HashMap::new(),
        },
        auth_config: None,
        discovered_oauth: None,
        oauth_config: None,
        enabled: true,
        created_at: Utc::now(),
        capability_snapshot: None,
    }
}

fn gateway_with(
    servers: Vec<McpServerConfig>,
    broadcast: Option<Arc<Broadcast>>,
) -> Arc<McpGateway> {
    let manager = Arc::new(McpServerManager::new());
    for server in servers {
        manager.add_config(server);
    }
    Arc::new(McpGateway::new_with_broadcast(
        manager,
        GatewayConfig::default(),
        test_router(),
        broadcast,
    ))
}

async fn send(
    gateway: &McpGateway,
    session: &str,
    method: &str,
    params: Option<Value>,
) -> JsonRpcResponse {
    gateway
        .handle_request_with_skills(
            "client-1",
            Some(session),
            vec![SERVER_ID.to_string()],
            vec![],
            McpPermissions {
                global: PermissionState::Allow,
                ..Default::default()
            },
            lr_config::SkillsPermissions::default(),
            "Test Client".to_string(),
            PermissionState::Off,
            PermissionState::Off,
            None,
            None,
            PermissionState::default(),
            PermissionState::default(),
            None,
            None,
            lr_config::ClientMode::default(),
            lr_config::ClientPushSettings::default(),
            JsonRpcRequest::new(Some(json!(1)), method.to_string(), params),
            None,
        )
        .await
        .expect("gateway request")
}

async fn initialize(gateway: &McpGateway, session: &str) -> JsonRpcResponse {
    send(
        gateway,
        session,
        "initialize",
        Some(json!({
            "protocolVersion": "2025-11-25",
            "capabilities": {"extensions": {EVENTS_EXTENSION_ID: {}}},
            "clientInfo": {"name": "test", "version": "1"}
        })),
    )
    .await
}

#[derive(Default)]
struct FakeRelay {
    consume_upstream: bool,
    calls: parking_lot::Mutex<Vec<(String, PushRelaySessionContext)>>,
    upstream: parking_lot::Mutex<Vec<UpstreamPushNotification>>,
}

impl FakeRelay {
    fn answer(
        &self,
        method: &str,
        ctx: &PushRelaySessionContext,
        request: JsonRpcRequest,
    ) -> lr_types::AppResult<JsonRpcResponse> {
        self.calls.lock().push((method.to_string(), ctx.clone()));
        Ok(JsonRpcResponse::success(
            request.id.unwrap_or(Value::Null),
            json!({"handledBy": "relay", "method": method}),
        ))
    }
}

#[async_trait]
impl PushRelayService for FakeRelay {
    fn advertisement(&self, ctx: &PushRelaySessionContext) -> PushAdvertisement {
        let mut ad = PushAdvertisement::default();
        ad.extensions
            .insert(EVENTS_EXTENSION_ID.into(), json!({"listChanged": true}));
        ad.capabilities.insert("events".into(), json!({}));
        if ctx.is_stdio_bridge {
            ad.experimental
                .insert(CLAUDE_CHANNEL_CAPABILITY.into(), json!({}));
        }
        ad
    }

    fn instructions(&self, _ctx: &PushRelaySessionContext) -> Option<String> {
        Some("Use EventPoll to read pushed events.".to_string())
    }

    fn on_upstream_notification(&self, notification: UpstreamPushNotification) -> bool {
        self.upstream.lock().push(notification);
        self.consume_upstream
    }

    async fn events_list(
        &self,
        ctx: &PushRelaySessionContext,
        request: JsonRpcRequest,
    ) -> lr_types::AppResult<JsonRpcResponse> {
        self.answer("events/list", ctx, request)
    }

    async fn events_poll(
        &self,
        ctx: &PushRelaySessionContext,
        request: JsonRpcRequest,
    ) -> lr_types::AppResult<JsonRpcResponse> {
        self.answer("events/poll", ctx, request)
    }

    async fn events_stream(
        &self,
        ctx: &PushRelaySessionContext,
        request: JsonRpcRequest,
    ) -> lr_types::AppResult<JsonRpcResponse> {
        self.answer("events/stream", ctx, request)
    }

    async fn events_subscribe(
        &self,
        ctx: &PushRelaySessionContext,
        request: JsonRpcRequest,
    ) -> lr_types::AppResult<JsonRpcResponse> {
        self.answer("events/subscribe", ctx, request)
    }

    async fn events_unsubscribe(
        &self,
        ctx: &PushRelaySessionContext,
        request: JsonRpcRequest,
    ) -> lr_types::AppResult<JsonRpcResponse> {
        self.answer("events/unsubscribe", ctx, request)
    }
}

/// Snapshot persistence recorded by tests: every batch handed to the persister.
fn record_persists(
    gateway: &McpGateway,
) -> Arc<parking_lot::Mutex<Vec<HashMap<String, UpstreamCapabilitySnapshot>>>> {
    let batches = Arc::new(parking_lot::Mutex::new(Vec::new()));
    let seen = batches.clone();
    gateway.set_capability_snapshot_debounce(Duration::from_millis(50));
    gateway.set_capability_snapshot_persister(Arc::new(move |batch| {
        seen.lock().push(batch);
        Box::pin(async {})
    }));
    batches
}

async fn wait_until(what: &str, mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !condition() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

fn upstream_methods(requests: &[Request]) -> Vec<String> {
    requests
        .iter()
        .filter_map(|r| {
            serde_json::from_slice::<Value>(&r.body)
                .ok()?
                .get("method")?
                .as_str()
                .map(str::to_string)
        })
        .collect()
}

#[tokio::test]
async fn events_methods_are_method_not_found_without_a_relay() {
    let upstream = push_upstream(Duration::ZERO).await;
    let gateway = gateway_with(vec![http_server(SERVER_ID, upstream.uri())], None);
    let init = initialize(&gateway, "s1").await;
    let caps = &init.result.unwrap()["capabilities"];
    // The upstream's push capabilities are not passed through on their own
    assert!(caps.get("experimental").is_none(), "{caps}");
    assert!(caps.get("events").is_none(), "{caps}");
    assert!(caps.get("futureCapability").is_none(), "{caps}");
    assert!(
        caps.get("extensions")
            .and_then(|e| e.get(EVENTS_EXTENSION_ID))
            .is_none(),
        "{caps}"
    );

    for method in [
        "events/list",
        "events/poll",
        "events/stream",
        "events/subscribe",
        "events/unsubscribe",
    ] {
        let response = send(&gateway, "s1", method, Some(json!({}))).await;
        assert_eq!(response.error.expect(method).code, -32601, "{method}");
    }
    // Only the gateway's own snapshot read reached the upstream, never the client's calls
    let requests = upstream.received_requests().await.unwrap();
    let events_calls = upstream_methods(&requests)
        .into_iter()
        .filter(|m| m.starts_with("events/") && m != "events/list")
        .count();
    assert_eq!(events_calls, 0);
}

#[tokio::test]
async fn events_methods_route_to_the_registered_relay() {
    let upstream = push_upstream(Duration::ZERO).await;
    let gateway = gateway_with(vec![http_server(SERVER_ID, upstream.uri())], None);
    let relay = Arc::new(FakeRelay::default());
    gateway.set_push_relay(relay.clone());
    gateway.set_stdio_bridge_session("bridge", true);

    initialize(&gateway, "bridge").await;
    let list_calls_before = upstream_methods(&upstream.received_requests().await.unwrap())
        .iter()
        .filter(|m| m.as_str() == "events/list")
        .count();

    for method in [
        "events/list",
        "events/poll",
        "events/stream",
        "events/subscribe",
        "events/unsubscribe",
    ] {
        let response = send(&gateway, "bridge", method, Some(json!({}))).await;
        assert!(response.error.is_none(), "{method}: {:?}", response.error);
        assert_eq!(response.result.unwrap()["method"], method);
    }

    let calls = relay.calls.lock().clone();
    assert_eq!(calls.len(), 5);
    let ctx = &calls[0].1;
    assert_eq!(ctx.client_id, "client-1");
    assert_eq!(ctx.session_key, "bridge");
    assert!(ctx.is_stdio_bridge);
    assert_eq!(ctx.visible_servers, vec![SERVER_ID.to_string()]);
    assert!(ctx.client_declares_events());

    // The client's events/* never reach an upstream
    let list_calls_after = upstream_methods(&upstream.received_requests().await.unwrap())
        .iter()
        .filter(|m| m.as_str() == "events/list")
        .count();
    assert_eq!(list_calls_before, list_calls_after);
}

#[tokio::test]
async fn push_capabilities_and_instructions_come_from_the_relay() {
    let upstream = push_upstream(Duration::ZERO).await;
    let gateway = gateway_with(vec![http_server(SERVER_ID, upstream.uri())], None);
    gateway.set_push_relay(Arc::new(FakeRelay::default()));
    gateway.set_stdio_bridge_session("bridge", true);

    let bridge = initialize(&gateway, "bridge").await.result.unwrap();
    let caps = &bridge["capabilities"];
    assert_eq!(caps["experimental"][CLAUDE_CHANNEL_CAPABILITY], json!({}));
    assert_eq!(caps["events"], json!({}));
    assert_eq!(
        caps["extensions"][EVENTS_EXTENSION_ID],
        json!({"listChanged": true})
    );
    assert!(bridge["instructions"]
        .as_str()
        .unwrap()
        .contains("Use EventPoll to read pushed events."));

    let http = initialize(&gateway, "http").await.result.unwrap();
    assert!(http["capabilities"].get("experimental").is_none());
    assert_eq!(http["capabilities"]["events"], json!({}));

    // server/discover advertises the same
    let discover = send(&gateway, "bridge", "server/discover", None)
        .await
        .result
        .unwrap();
    assert_eq!(
        discover["capabilities"]["experimental"][CLAUDE_CHANNEL_CAPABILITY],
        json!({})
    );

    // Clearing the bridge flag takes effect on the next request
    gateway.set_stdio_bridge_session("bridge", false);
    let discover = send(&gateway, "bridge", "server/discover", None)
        .await
        .result
        .unwrap();
    assert!(discover["capabilities"].get("experimental").is_none());
}

#[tokio::test]
async fn capabilities_are_remembered_and_persisted_only_on_change() {
    let upstream = push_upstream(Duration::ZERO).await;
    let gateway = gateway_with(vec![http_server(SERVER_ID, upstream.uri())], None);
    let batches = record_persists(&gateway);

    initialize(&gateway, "s1").await;
    wait_until("first persist", || batches.lock().len() == 1).await;
    let snapshot = batches.lock()[0][SERVER_ID].clone();
    assert_eq!(snapshot.protocol_version, "2025-11-25");
    assert!(snapshot.push.channels);
    assert!(!snapshot.push.resource_subscribe);
    assert_eq!(snapshot.event_catalogue.len(), 2);
    assert_eq!(snapshot.push.events_modes.len(), 2);
    assert_eq!(snapshot.capabilities["futureCapability"], json!({}));
    assert_eq!(
        snapshot.capabilities["events"],
        json!({"listChanged": true})
    );

    let sources = gateway.remembered_push_sources(&[SERVER_ID.to_string()], false);
    assert_eq!(sources.len(), 1);
    assert_eq!(sources[0].server_name, "Push Server");
    assert_eq!(
        gateway
            .remembered_push_sources_for_session("s1", false)
            .await
            .len(),
        1
    );

    // A second session observes the same capabilities: nothing is written
    initialize(&gateway, "s2").await;
    let requests_with_list = || async {
        upstream_methods(&upstream.received_requests().await.unwrap())
            .iter()
            .filter(|m| m.as_str() == "events/list")
            .count()
    };
    let deadline = Instant::now() + Duration::from_secs(5);
    while requests_with_list().await < 2 {
        assert!(Instant::now() < deadline, "second observation");
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        batches.lock().len(),
        1,
        "unchanged capabilities were written"
    );

    // The providers map says who offers what
    let providers = gateway.session_capability_providers("s1").await.unwrap();
    assert_eq!(providers.servers_with_events(), vec![SERVER_ID]);
    assert_eq!(providers.servers_with_claude_channel(), vec![SERVER_ID]);
}

#[tokio::test]
async fn changed_server_identity_invalidates_the_snapshot() {
    let upstream = push_upstream(Duration::ZERO).await;
    let gateway = gateway_with(vec![http_server(SERVER_ID, upstream.uri())], None);
    let batches = record_persists(&gateway);
    initialize(&gateway, "s1").await;
    wait_until("persist", || batches.lock().len() == 1).await;
    assert_eq!(
        gateway
            .remembered_push_sources(&[SERVER_ID.to_string()], false)
            .len(),
        1
    );

    // Same server id, different URL: the remembered capabilities no longer apply
    let other = push_upstream(Duration::ZERO).await;
    let mut moved = http_server(SERVER_ID, other.uri());
    moved.capability_snapshot = Some(batches.lock()[0][SERVER_ID].clone());
    let manager_view = gateway_with(vec![moved], None);
    assert!(manager_view
        .remembered_push_sources(&[SERVER_ID.to_string()], false)
        .is_empty());
}

#[tokio::test]
async fn initialize_waits_for_a_server_it_has_never_seen() {
    let upstream = push_upstream(Duration::from_millis(3500)).await;
    let gateway = gateway_with(vec![http_server(SERVER_ID, upstream.uri())], None);
    gateway.set_push_relay(Arc::new(FakeRelay::default()));
    record_persists(&gateway);

    let started = Instant::now();
    let first = initialize(&gateway, "s1").await.result.unwrap();
    let elapsed = started.elapsed();
    assert!(
        elapsed >= Duration::from_secs(3),
        "returned after {elapsed:?}"
    );
    assert!(
        elapsed < Duration::from_secs(10),
        "returned after {elapsed:?}"
    );
    assert!(
        first
            .get("_meta")
            .and_then(|m| m.get("initializing"))
            .is_none(),
        "the full handshake result is returned: {first}"
    );
    wait_until("snapshot", || {
        !gateway
            .remembered_push_sources(&[SERVER_ID.to_string()], false)
            .is_empty()
    })
    .await;

    // Remembered now: the next session answers on the normal 2 s budget
    let started = Instant::now();
    let second = initialize(&gateway, "s2").await.result.unwrap();
    let elapsed = started.elapsed();
    assert!(
        elapsed < Duration::from_secs(3),
        "waited again: {elapsed:?}"
    );
    assert_eq!(second["_meta"]["initializing"], true);
    // ... still advertising push from what was remembered
    assert_eq!(second["capabilities"]["events"], json!({}));
}

#[tokio::test]
async fn first_discovery_wait_is_bounded() {
    let upstream = push_upstream(Duration::from_millis(4500)).await;
    let gateway = gateway_with(vec![http_server(SERVER_ID, upstream.uri())], None);
    gateway.set_push_relay(Arc::new(FakeRelay::default()));
    gateway.set_first_discovery_wait(Duration::from_millis(2_500));

    let started = Instant::now();
    let response = initialize(&gateway, "s1").await.result.unwrap();
    let elapsed = started.elapsed();
    assert!(elapsed >= Duration::from_millis(2_400), "{elapsed:?}");
    assert!(elapsed < Duration::from_millis(4_000), "{elapsed:?}");
    assert_eq!(response["_meta"]["initializing"], true);
}

#[tokio::test]
async fn no_wait_without_a_relay_or_when_disabled() {
    for register in [false, true] {
        let upstream = push_upstream(Duration::from_millis(3500)).await;
        let gateway = gateway_with(vec![http_server(SERVER_ID, upstream.uri())], None);
        if register {
            gateway.set_push_relay(Arc::new(FakeRelay::default()));
            gateway.update_push_relay_config(&lr_config::PushRelayConfig {
                enabled: false,
                ..Default::default()
            });
        }
        let started = Instant::now();
        let response = initialize(&gateway, "s1").await.result.unwrap();
        assert!(started.elapsed() < Duration::from_secs(3));
        assert_eq!(response["_meta"]["initializing"], true);
    }
}

#[tokio::test]
async fn disabled_servers_are_never_waited_on() {
    let upstream = push_upstream(Duration::from_millis(3500)).await;
    let mut disabled = http_server(SERVER_ID, upstream.uri());
    disabled.enabled = false;
    let gateway = gateway_with(vec![disabled], None);
    gateway.set_push_relay(Arc::new(FakeRelay::default()));
    let started = Instant::now();
    initialize(&gateway, "s1").await;
    assert!(started.elapsed() < Duration::from_secs(3));
}

/// A stdio upstream: answers the handshake, sends a Claude channel message
/// once initialised, serves `events/list`, and answers `events/stream` with
/// one tagged event and the final result.
const STDIO_UPSTREAM: &str = r#"while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  case "$line" in
    *'"method":"server/discover"'*) printf '{"jsonrpc":"2.0","id":%s,"error":{"code":-32601,"message":"no"}}\n' "$id" ;;
    *'"method":"initialize"'*) printf '{"jsonrpc":"2.0","id":%s,"result":{"protocolVersion":"2025-11-25","capabilities":{"tools":{},"events":{},"experimental":{"claude/channel":{}}},"serverInfo":{"name":"stdio","version":"1"}}}\n' "$id" ;;
    *'"method":"notifications/initialized"'*) printf '{"jsonrpc":"2.0","method":"notifications/claude/channel","params":{"content":"hello","meta":{"chat":"1"}}}\n' ;;
    *'"method":"events/list"'*) printf '{"jsonrpc":"2.0","id":%s,"result":{"events":[{"name":"tick","delivery":["push","poll"]}]}}\n' "$id" ;;
    *'"method":"events/stream"'*) printf '{"jsonrpc":"2.0","method":"notifications/events/event","params":{"name":"tick","_meta":{"io.modelcontextprotocol/subscriptionId":%s}}}\n' "$id"; printf '{"jsonrpc":"2.0","id":%s,"result":{"resultType":"complete"}}\n' "$id" ;;
    *'"method":"notifications/'*) ;;
    *) if [ -n "$id" ]; then printf '{"jsonrpc":"2.0","id":%s,"result":{"tools":[],"prompts":[],"resources":[]}}\n' "$id"; fi ;;
  esac
done"#;

fn stdio_server() -> McpServerConfig {
    McpServerConfig {
        id: SERVER_ID.to_string(),
        name: "Stdio Push".to_string(),
        transport: McpTransportType::Stdio,
        transport_config: McpTransportConfig::Stdio {
            command: "sh".to_string(),
            args: vec!["-c".to_string(), STDIO_UPSTREAM.to_string()],
            env: HashMap::new(),
            cwd: None,
        },
        auth_config: None,
        discovered_oauth: None,
        oauth_config: None,
        enabled: true,
        created_at: Utc::now(),
        capability_snapshot: None,
    }
}

#[cfg(unix)]
#[tokio::test]
async fn upstream_channel_messages_reach_the_relay() {
    let (tx, mut rx) = tokio::sync::broadcast::channel(64);
    let gateway = gateway_with(vec![stdio_server()], Some(Arc::new(tx)));
    let relay = Arc::new(FakeRelay {
        consume_upstream: true,
        ..Default::default()
    });
    gateway.set_push_relay(relay.clone());
    initialize(&gateway, "s1").await;

    wait_until("channel message", || !relay.upstream.lock().is_empty()).await;
    let received = relay.upstream.lock()[0].clone();
    assert_eq!(received.server_id, SERVER_ID);
    assert_eq!(received.session_key, "s1");
    assert_eq!(received.client_id, "client-1");
    assert_eq!(received.notification.method, "notifications/claude/channel");
    assert_eq!(received.notification.params.unwrap()["content"], "hello");

    // Consumed by the relay: not rebroadcast raw
    tokio::time::sleep(Duration::from_millis(200)).await;
    while let Ok((_, notification)) = rx.try_recv() {
        assert_ne!(notification.method, "notifications/claude/channel");
    }
}

#[cfg(unix)]
#[tokio::test]
async fn unconsumed_channel_messages_keep_the_raw_forwarding() {
    let (tx, mut rx) = tokio::sync::broadcast::channel(64);
    let gateway = gateway_with(vec![stdio_server()], Some(Arc::new(tx)));
    let relay = Arc::new(FakeRelay::default());
    gateway.set_push_relay(relay.clone());
    initialize(&gateway, "s1").await;

    let forwarded = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let (key, notification) = rx.recv().await.unwrap();
            if notification.method == "notifications/claude/channel" {
                return key;
            }
        }
    })
    .await
    .expect("raw forwarding");
    assert_eq!(
        forwarded,
        localrouter::mcp::gateway::types::session_server_notification_key("s1", SERVER_ID)
    );
    assert_eq!(relay.upstream.lock().len(), 1);
}

#[cfg(unix)]
#[tokio::test]
async fn relay_can_stream_and_query_an_upstream_on_a_session() {
    let gateway = gateway_with(vec![stdio_server()], None);
    initialize(&gateway, "s1").await;

    let list = gateway
        .upstream_request("s1", SERVER_ID, "events/list", Some(json!({})))
        .await
        .unwrap();
    assert_eq!(list.result.unwrap()["events"][0]["name"], "tick");

    let mut stream = gateway
        .open_upstream_stream(
            "s1",
            SERVER_ID,
            "events/stream",
            Some(json!({"name": "tick"})),
            8,
        )
        .await
        .unwrap();
    let id = stream.request_id().clone();
    match tokio::time::timeout(Duration::from_secs(5), stream.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap()
    {
        RpcStreamMessage::Notification(n) => {
            assert_eq!(n.method, "notifications/events/event");
            assert_eq!(
                n.params.unwrap()["_meta"]["io.modelcontextprotocol/subscriptionId"],
                id
            );
        }
        other => panic!("unexpected {other:?}"),
    }
    match tokio::time::timeout(Duration::from_secs(5), stream.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap()
    {
        RpcStreamMessage::Result(r) => assert_eq!(r.id, id),
        other => panic!("unexpected {other:?}"),
    }

    // Servers the session cannot see are refused
    assert!(gateway
        .upstream_request("s1", "other-server", "events/list", None)
        .await
        .is_err());
    assert!(gateway
        .upstream_request("no-such-session", SERVER_ID, "events/list", None)
        .await
        .is_err());
}
