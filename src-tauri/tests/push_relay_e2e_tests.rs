//! Push Relay end to end through the real gateway: ingress from scripted
//! upstreams (HTTP poll over a relay-owned connection, stdio push and
//! Claude channel on the session's process), gating and fallback tools,
//! per-event Ask, Monitor records, Claude Channels egress and deduplication,
//! webhook egress, the `/events/watch` stream, and the built-in test source
//! that Try it out's Push panel sends through the same pipeline.

use chrono::Utc;
use futures_util::StreamExt;
use localrouter::config::{
    AppConfig, ConfigManager, McpServerConfig, McpTransportConfig, McpTransportType,
};
use localrouter::mcp::gateway::push_relay::journal::{IngressMode, Occurrence};
use localrouter::mcp::gateway::push_relay::relay::{
    PushToolsState, TEST_EVENT_NAME, TEST_SOURCE_SERVER_ID,
};
use localrouter::mcp::gateway::push_relay::{
    webhook, PushConfigChange, PushDecision, PushDecisionAction, PushRelay, PushRelayLimits,
    CLAUDE_CHANNEL_CAPABILITY, EVENTS_EXTENSION_ID,
};
use localrouter::mcp::gateway::types::session_notification_key;
use localrouter::mcp::gateway::{GatewayConfig, McpGateway};
use localrouter::mcp::protocol::{JsonRpcNotification, JsonRpcRequest, JsonRpcResponse};
use localrouter::mcp::McpServerManager;
use localrouter::monitoring::metrics::MetricsCollector;
use localrouter::monitoring::storage::MetricsDatabase;
use localrouter::providers::registry::ProviderRegistry;
use localrouter::router::{RateLimiterManager, Router};
use lr_config::{
    EventDeliveryMode, PermissionState, PushPermission, PushPermissions, PushRelayConfig,
    UpstreamCapabilitySnapshot, UpstreamPushSummary,
};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use wiremock::{
    matchers::method as http_method, Mock, MockServer, Request, Respond, ResponseTemplate,
};

const HTTP_ID: &str = "http-push";
const STDIO_ID: &str = "stdio-push";
const CLIENT: &str = "client-1";

type Broadcast = tokio::sync::broadcast::Sender<(String, JsonRpcNotification)>;
type MonitorLog = Arc<parking_lot::Mutex<Vec<lr_monitor::MonitorEvent>>>;

fn test_router() -> Arc<Router> {
    let config_manager = Arc::new(ConfigManager::new(
        AppConfig::default(),
        std::path::PathBuf::from("/tmp/test_push_relay_e2e_router.yaml"),
    ));
    let metrics_db = Arc::new(
        MetricsDatabase::new(std::env::temp_dir().join(format!(
            "test_push_relay_e2e_metrics_{}.db",
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

/// Internal limits shrunk so the tests run fast.
fn fast_limits() -> PushRelayLimits {
    PushRelayLimits {
        upstream_poll_min_interval_ms: 50,
        upstream_reconnect_backoff_min_ms: 50,
        upstream_reconnect_backoff_max_ms: 200,
        ..PushRelayLimits::default()
    }
}

fn snapshot(
    server: &McpServerConfig,
    events: &[(&str, &[EventDeliveryMode])],
    channels: bool,
) -> UpstreamCapabilitySnapshot {
    let mut events_modes = BTreeMap::new();
    let mut catalogue = Vec::new();
    for (name, modes) in events {
        events_modes.insert(name.to_string(), modes.to_vec());
        let delivery: Vec<&str> = modes
            .iter()
            .map(|m| match m {
                EventDeliveryMode::Push => "push",
                EventDeliveryMode::Poll => "poll",
                EventDeliveryMode::Webhook => "webhook",
            })
            .collect();
        catalogue.push(
            json!({"name": name, "description": format!("{name} happened"), "delivery": delivery,
            "inputSchema": {"type": "object", "properties": {}}}),
        );
    }
    UpstreamCapabilitySnapshot {
        fingerprint: lr_config::fingerprint_for(server),
        protocol_version: "2025-11-25".into(),
        capabilities: json!({"events": {}}),
        push: UpstreamPushSummary {
            events_modes,
            channels,
            resource_subscribe: false,
        },
        event_catalogue: catalogue,
        observed_at: Utc::now(),
    }
}

fn http_server(url: String) -> McpServerConfig {
    let mut server = McpServerConfig {
        id: HTTP_ID.to_string(),
        name: "Http Push".to_string(),
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
    };
    server.capability_snapshot = Some(snapshot(
        &server,
        &[
            ("msg", &[EventDeliveryMode::Poll]),
            ("alert", &[EventDeliveryMode::Poll]),
            ("hook.only", &[EventDeliveryMode::Webhook]),
        ],
        false,
    ));
    server
}

/// A stdio upstream: `events/stream` emits one event then completes; a
/// channel message follows `notifications/initialized`.
const STDIO_UPSTREAM: &str = r#"while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  sid=$(printf '%s' "$line" | sed -n 's/.*"id":\("[^"]*"\).*/\1/p')
  [ -z "$id" ] && id="$sid"
  case "$line" in
    *'"method":"server/discover"'*) printf '{"jsonrpc":"2.0","id":%s,"error":{"code":-32601,"message":"no"}}\n' "$id" ;;
    *'"method":"initialize"'*) printf '{"jsonrpc":"2.0","id":%s,"result":{"protocolVersion":"2025-11-25","capabilities":{"tools":{},"events":{},"experimental":{"claude/channel":{}}},"serverInfo":{"name":"stdio","version":"1"}}}\n' "$id" ;;
    *'"method":"notifications/initialized"'*) sleep 0.3; printf '{"jsonrpc":"2.0","method":"notifications/claude/channel","params":{"content":"hello from stdio","meta":{"chat":"1"}}}\n' ;;
    *'"method":"events/list"'*) printf '{"jsonrpc":"2.0","id":%s,"result":{"events":[{"name":"tick","delivery":["push","poll"]}]}}\n' "$id" ;;
    *'"method":"events/stream"'*) printf '{"jsonrpc":"2.0","method":"notifications/events/active","params":{"cursor":"c0","_meta":{"io.modelcontextprotocol/subscriptionId":%s}}}\n' "$id"; printf '{"jsonrpc":"2.0","method":"notifications/events/event","params":{"eventId":"t1","name":"tick","timestamp":"2026-10-30T00:00:00Z","data":{"n":1},"cursor":"c1","_meta":{"io.modelcontextprotocol/subscriptionId":%s}}}\n' "$id"; sleep 30 ;;
    *'"method":"notifications/'*) ;;
    *) if [ -n "$id" ]; then printf '{"jsonrpc":"2.0","id":%s,"result":{"tools":[],"prompts":[],"resources":[]}}\n' "$id"; fi ;;
  esac
done"#;

fn stdio_server() -> McpServerConfig {
    let mut server = McpServerConfig {
        id: STDIO_ID.to_string(),
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
    };
    server.capability_snapshot = Some(snapshot(
        &server,
        &[("tick", &[EventDeliveryMode::Push, EventDeliveryMode::Poll])],
        true,
    ));
    server
}

fn client(id: &str) -> lr_config::Client {
    let mut client = lr_config::Client::new_with_strategy(format!("Client {id}"), String::new());
    client.id = id.to_string();
    client.mcp_permissions.global = PermissionState::Allow;
    client
}

struct Harness {
    gateway: Arc<McpGateway>,
    relay: Arc<PushRelay>,
    clients: Arc<parking_lot::RwLock<HashMap<String, lr_config::Client>>>,
    rx: tokio::sync::broadcast::Receiver<(String, JsonRpcNotification)>,
    persisted: Arc<parking_lot::Mutex<Vec<(String, PushConfigChange)>>>,
    monitor: MonitorLog,
    servers: Vec<String>,
}

fn harness(servers: Vec<McpServerConfig>, limits: PushRelayLimits) -> Harness {
    let config = PushRelayConfig::default();
    let manager = Arc::new(McpServerManager::new());
    let server_ids = servers.iter().map(|s| s.id.clone()).collect();
    for server in servers {
        manager.add_config(server);
    }
    let (tx, rx) = tokio::sync::broadcast::channel::<(String, JsonRpcNotification)>(1024);
    let broadcast: Arc<Broadcast> = Arc::new(tx);
    let gateway = Arc::new(McpGateway::new_with_broadcast(
        manager,
        GatewayConfig::default(),
        test_router(),
        Some(broadcast),
    ));
    gateway.update_push_relay_config(&config);
    gateway.set_first_discovery_wait(Duration::from_secs(2));
    let clients = Arc::new(parking_lot::RwLock::new(HashMap::new()));
    clients.write().insert(CLIENT.to_string(), client(CLIENT));
    let lookup = clients.clone();
    let relay = PushRelay::new(
        gateway.as_ref().clone(),
        config,
        Arc::new(move |id| lookup.read().get(id).cloned()),
    );
    relay.set_limits(limits);
    let persisted = Arc::new(parking_lot::Mutex::new(Vec::new()));
    let seen = persisted.clone();
    relay.set_persister(Arc::new(move |client_id, change| {
        seen.lock().push((client_id, change));
        Box::pin(async { Ok(()) })
    }));
    relay.set_local_base_url("http://127.0.0.1:3625");
    gateway.set_push_relay(relay.clone());
    gateway.register_virtual_server(relay.tools_server());

    let monitor: MonitorLog = Arc::new(parking_lot::Mutex::new(Vec::new()));
    let log = monitor.clone();
    gateway.set_on_monitor_event(
        move |event_type, client_id, client_name, session_id, data, status, duration_ms| {
            let id = format!("mon-{}", uuid::Uuid::new_v4());
            log.lock().push(lr_monitor::MonitorEvent {
                payload_redacted: false,
                id: id.clone(),
                sequence: 0,
                timestamp: Utc::now(),
                event_type,
                client_id,
                client_name,
                session_id,
                conversation: None,
                data,
                status,
                duration_ms,
            });
            id
        },
    );
    let log = monitor.clone();
    gateway.set_on_monitor_update(Arc::new(move |id, updater| {
        if let Some(event) = log.lock().iter_mut().find(|e| e.id == id) {
            updater(event);
        }
    }));
    Harness {
        gateway,
        relay,
        clients,
        rx,
        persisted,
        monitor,
        servers: server_ids,
    }
}

impl Harness {
    fn set_client(&self, client: lr_config::Client) {
        self.clients.write().insert(client.id.clone(), client);
        self.relay.notify_clients_changed();
    }

    async fn send(&self, session: &str, method: &str, params: Option<Value>) -> JsonRpcResponse {
        self.send_as(CLIENT, session, method, params).await
    }

    async fn send_as(
        &self,
        client_id: &str,
        session: &str,
        method: &str,
        params: Option<Value>,
    ) -> JsonRpcResponse {
        let client = self.clients.read().get(client_id).cloned().unwrap();
        self.gateway
            .handle_request_with_skills(
                client_id,
                Some(session),
                self.servers.clone(),
                vec![],
                client.mcp_permissions.clone(),
                lr_config::SkillsPermissions::default(),
                client.name.clone(),
                PermissionState::Off,
                PermissionState::Off,
                None,
                Some(lr_config::ContextManagementOverrides {
                    context_management_enabled: Some(false),
                    catalog_compression_enabled: Some(false),
                }),
                PermissionState::default(),
                PermissionState::default(),
                None,
                None,
                lr_config::ClientMode::default(),
                lr_config::ClientPushSettings::from_client(&client),
                JsonRpcRequest::new(Some(json!(1)), method.to_string(), params),
                None,
            )
            .await
            .expect("gateway request")
    }

    async fn initialize(&self, session: &str, capabilities: Value) -> JsonRpcResponse {
        self.send(
            session,
            "initialize",
            Some(json!({
                "protocolVersion": "2025-11-25",
                "capabilities": capabilities,
                "clientInfo": {"name": "test", "version": "1"}
            })),
        )
        .await
    }

    async fn tool_names(&self, session: &str) -> Vec<String> {
        let result = self.send(session, "tools/list", None).await.result.unwrap();
        result["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap().to_string())
            .collect()
    }

    async fn call_tool(&self, session: &str, name: &str, arguments: Value) -> String {
        let response = self
            .send(
                session,
                "tools/call",
                Some(json!({"name": name, "arguments": arguments})),
            )
            .await;
        let result = response
            .result
            .unwrap_or_else(|| panic!("tool error: {:?}", response.error));
        result["content"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .to_string()
    }

    /// Bootstrap a poll cursor (no history) for one event name.
    async fn poll(&self, session: &str, name: &str, cursor: Option<&str>) -> Value {
        let response = self
            .send(
                session,
                "events/poll",
                Some(json!({"name": name, "cursor": cursor})),
            )
            .await;
        response
            .result
            .unwrap_or_else(|| panic!("poll error: {:?}", response.error))
    }

    async fn poll_until(
        &self,
        session: &str,
        name: &str,
        cursor: &str,
        done: impl Fn(&Value) -> bool,
    ) -> Value {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let batch = self.poll(session, name, Some(cursor)).await;
            if done(&batch) {
                return batch;
            }
            assert!(
                Instant::now() < deadline,
                "timed out polling {name}: {batch}"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    fn monitor_decisions(&self, event_name: &str) -> Vec<String> {
        self.monitor
            .lock()
            .iter()
            .filter_map(|e| match &e.data {
                lr_monitor::MonitorEventData::McpPushEvent {
                    event_name: n,
                    decision,
                    ..
                } if n == event_name => Some(decision.clone()),
                _ => None,
            })
            .collect()
    }

    async fn next_notification(
        &mut self,
        method: &str,
        key: &str,
        timeout: Duration,
    ) -> Option<JsonRpcNotification> {
        tokio::time::timeout(timeout, async {
            loop {
                let (k, n) = self.rx.recv().await.ok()?;
                if n.method == method && k == key {
                    return Some(n);
                }
            }
        })
        .await
        .ok()
        .flatten()
    }
}

fn occurrence(
    relay_server: &str,
    slug: &str,
    name: &str,
    id: &str,
    data: Value,
) -> Arc<Occurrence> {
    Arc::new(Occurrence {
        event_id: format!("{slug}__{id}"),
        name: format!("{slug}__{name}"),
        server_id: relay_server.to_string(),
        server_slug: slug.to_string(),
        upstream_name: name.to_string(),
        timestamp: Utc::now(),
        data,
        upstream_cursor: None,
        ingress: IngressMode::Push,
        args_key: "{}".to_string(),
        origin_session: None,
    })
}

async fn wait_until(what: &str, mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !condition() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

/// An HTTP upstream answering `events/poll`: the first call bootstraps, the
/// second returns one event, later calls nothing.
struct PollResponder {
    calls: Arc<AtomicUsize>,
    /// The event is emitted once, on the first poll after this is set.
    release: Arc<std::sync::atomic::AtomicBool>,
    emitted: Arc<std::sync::atomic::AtomicBool>,
}

impl Respond for PollResponder {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let body: Value = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
        let id = body["id"].clone();
        let n = self.calls.fetch_add(1, Ordering::SeqCst);
        let emit = n >= 1
            && self.release.load(Ordering::SeqCst)
            && !self.emitted.swap(true, Ordering::SeqCst);
        let events = if emit {
            json!([{"eventId": "m1", "name": "msg", "timestamp": "2026-10-30T10:00:00Z", "data": {"text": "hi"}}])
        } else {
            json!([])
        };
        ResponseTemplate::new(200).set_body_json(json!({"jsonrpc": "2.0", "id": id,
            "result": {"resultType": "complete", "events": events, "cursor": format!("c{n}"), "hasMore": false, "nextPollMs": 50}}))
    }
}

struct JsonRpcMethod(&'static str);

impl wiremock::Match for JsonRpcMethod {
    fn matches(&self, request: &Request) -> bool {
        serde_json::from_slice::<Value>(&request.body)
            .ok()
            .and_then(|b| b.get("method")?.as_str().map(|m| m == self.0))
            .unwrap_or(false)
    }
}

/// An HTTP push upstream that never emits an event.
async fn http_upstream() -> (MockServer, Arc<AtomicUsize>) {
    let (server, calls, _) = http_upstream_with(false).await;
    (server, calls)
}

/// An HTTP push upstream that emits one `msg` once `release` is set.
async fn http_upstream_with(
    released: bool,
) -> (
    MockServer,
    Arc<AtomicUsize>,
    Arc<std::sync::atomic::AtomicBool>,
) {
    let server = MockServer::start().await;
    let calls = Arc::new(AtomicUsize::new(0));
    let release = Arc::new(std::sync::atomic::AtomicBool::new(released));
    for (method, result) in [
        (
            "initialize",
            json!({"protocolVersion": "2025-11-25", "capabilities": {"tools": {}, "events": {}},
            "serverInfo": {"name": "push", "version": "1"}}),
        ),
        (
            "events/list",
            json!({"events": [
            {"name": "msg", "description": "msg happened", "delivery": ["poll"], "inputSchema": {"type": "object", "properties": {}}},
            {"name": "alert", "description": "alert happened", "delivery": ["poll"], "inputSchema": {"type": "object", "properties": {}}},
            {"name": "hook.only", "description": "hook.only happened", "delivery": ["webhook"], "inputSchema": {"type": "object", "properties": {}}}]}),
        ),
        ("tools/list", json!({"tools": []})),
        ("resources/list", json!({"resources": []})),
        ("prompts/list", json!({"prompts": []})),
    ] {
        Mock::given(http_method("POST"))
            .and(JsonRpcMethod(method))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"jsonrpc": "2.0", "id": 1, "result": result})),
            )
            .mount(&server)
            .await;
    }
    Mock::given(http_method("POST"))
        .and(JsonRpcMethod("events/poll"))
        .respond_with(PollResponder {
            calls: calls.clone(),
            release: release.clone(),
            emitted: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        })
        .mount(&server)
        .await;
    Mock::given(http_method("POST"))
        .respond_with(ResponseTemplate::new(202))
        .mount(&server)
        .await;
    (server, calls, release)
}

// ---------------------------------------------------------------------------
// Gating and fallback tools
// ---------------------------------------------------------------------------

#[tokio::test]
async fn gate_advertises_push_only_with_a_permitted_source() {
    let (upstream, _) = http_upstream().await;
    let h = harness(vec![http_server(upstream.uri())], fast_limits());

    let init = h.initialize("plain", json!({})).await.result.unwrap();
    let caps = &init["capabilities"];
    assert_eq!(
        caps["extensions"][EVENTS_EXTENSION_ID],
        json!({"listChanged": true})
    );
    assert_eq!(caps["events"], json!({}));
    assert!(caps
        .get("experimental")
        .and_then(|e| e.get(CLAUDE_CHANNEL_CAPABILITY))
        .is_none());
    let tools = h.tool_names("plain").await;
    for tool in ["EventList", "EventPoll", "EventWait"] {
        assert!(
            tools.contains(&tool.to_string()),
            "{tool} missing from {tools:?}"
        );
    }
    let instructions = init["instructions"].as_str().unwrap_or_default();
    assert!(instructions.contains("<events>"), "{instructions}");

    // Bridge sessions additionally get Claude Code channels
    h.gateway.set_stdio_bridge_session("bridge", true);
    let init = h.initialize("bridge", json!({})).await.result.unwrap();
    assert_eq!(
        init["capabilities"]["experimental"][CLAUDE_CHANNEL_CAPABILITY],
        json!({})
    );

    // events/list: permitted types only, webhook-only excluded, server prefix
    let list = h.send("plain", "events/list", None).await.result.unwrap();
    let names: Vec<&str> = list["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, vec!["http-push__alert", "http-push__msg"]);
    assert_eq!(
        list["events"][0]["delivery"],
        json!(["poll", "push", "webhook"])
    );
    assert!(list["events"][0]["description"]
        .as_str()
        .unwrap()
        .starts_with("[Http Push]"));

    // Everything Off: nothing advertised, no tools, no instructions
    let mut off = client(CLIENT);
    off.push_permissions.global = Some(PushPermission::Off);
    h.set_client(off);
    let init = h.initialize("off", json!({})).await.result.unwrap();
    assert!(init["capabilities"].get("events").is_none());
    assert!(init["capabilities"]
        .get("extensions")
        .and_then(|e| e.get(EVENTS_EXTENSION_ID))
        .is_none());
    assert!(!h
        .tool_names("off")
        .await
        .iter()
        .any(|t| t.starts_with("Event")));
    assert!(!init["instructions"]
        .as_str()
        .unwrap_or_default()
        .contains("<events>"));

    // Restricted privacy profiles never receive push
    let mut private = client(CLIENT);
    private.security.privacy.local_only = true;
    h.set_client(private);
    let init = h.initialize("private", json!({})).await.result.unwrap();
    assert!(init["capabilities"].get("events").is_none());

    // No MCP access to the server: no push either (never wider than MCP)
    let mut no_mcp = client(CLIENT);
    no_mcp.mcp_permissions.global = PermissionState::Off;
    h.set_client(no_mcp);
    assert!(h
        .relay
        .catalogue(&h.clients.read()[CLIENT], &[HTTP_ID.to_string()])
        .is_empty());
}

fn off_config() -> PushRelayConfig {
    PushRelayConfig {
        default_permission: PushPermission::Off,
        ..PushRelayConfig::default()
    }
}

/// Asserts that a session was told nothing about push.
async fn assert_no_push(h: &Harness, client_id: &str, session: &str) {
    let init = h
        .send_as(
            client_id,
            session,
            "initialize",
            Some(json!({"protocolVersion": "2025-11-25", "capabilities": {},
                "clientInfo": {"name": "t", "version": "1"}})),
        )
        .await
        .result
        .unwrap();
    let caps = &init["capabilities"];
    assert!(caps.get("events").is_none(), "{caps}");
    assert!(
        caps.get("extensions")
            .and_then(|e| e.get(EVENTS_EXTENSION_ID))
            .is_none(),
        "{caps}"
    );
    assert!(
        caps.get("experimental")
            .and_then(|e| e.get(CLAUDE_CHANNEL_CAPABILITY))
            .is_none(),
        "{caps}"
    );
    assert!(!init["instructions"]
        .as_str()
        .unwrap_or_default()
        .contains("<events>"));
    let tools = h
        .send_as(client_id, session, "tools/list", None)
        .await
        .result
        .unwrap();
    assert!(
        !tools["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["name"].as_str().unwrap_or_default().starts_with("Event")),
        "{tools}"
    );
}

#[tokio::test]
async fn global_off_with_no_overrides_does_no_push_work() {
    let (upstream, calls) = http_upstream().await;
    let h = harness(vec![http_server(upstream.uri())], fast_limits());
    h.relay.update_config(&off_config());
    assert!(!h.relay.push_possible(&h.clients.read()[CLIENT]));

    assert_no_push(&h, CLIENT, "plain").await;
    h.gateway.set_stdio_bridge_session("bridge", true);
    assert_no_push(&h, CLIENT, "bridge").await;

    // Nothing to list or subscribe to
    let list = h.send("plain", "events/list", None).await.result.unwrap();
    assert_eq!(list["events"], json!([]));
    let poll = h
        .send(
            "plain",
            "events/poll",
            Some(json!({"name": "http-push__msg"})),
        )
        .await;
    assert!(poll.result.is_none(), "an Off type is not found");
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(h.relay.active_subscriptions().is_empty());
    assert_eq!(calls.load(Ordering::SeqCst), 0, "upstream never polled");
}

#[tokio::test]
async fn turning_push_off_closes_upstream_subscriptions() {
    let (upstream, calls) = http_upstream().await;
    let h = harness(vec![http_server(upstream.uri())], fast_limits());
    h.poll("s1", "http-push__msg", None).await;
    wait_until("upstream polled", || calls.load(Ordering::SeqCst) >= 1).await;
    assert!(!h.relay.active_subscriptions().is_empty());

    h.relay.update_config(&off_config());
    wait_until("subscription closed", || {
        h.relay.active_subscriptions().is_empty()
    })
    .await;
    let after = calls.load(Ordering::SeqCst);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        calls.load(Ordering::SeqCst) <= after + 1,
        "upstream polling stopped"
    );
}

/// An HTTP upstream with events whose `initialize` answers after `delay`.
async fn slow_upstream(delay: Duration) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(http_method("POST"))
        .and(JsonRpcMethod("initialize"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"jsonrpc": "2.0", "id": 1, "result": {
                    "protocolVersion": "2025-11-25",
                    "capabilities": {"tools": {}, "events": {}},
                    "serverInfo": {"name": "slow", "version": "1"}}}))
                .set_delay(delay),
        )
        .mount(&server)
        .await;
    Mock::given(http_method("POST"))
        .and(JsonRpcMethod("events/list"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"jsonrpc": "2.0", "id": 1,
            "result": {"events": [{"name": "msg", "delivery": ["poll"]}]}})),
        )
        .mount(&server)
        .await;
    Mock::given(http_method("POST"))
        .and(JsonRpcMethod("tools/list"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"jsonrpc": "2.0", "id": 1, "result": {"tools": []}})),
        )
        .mount(&server)
        .await;
    Mock::given(http_method("POST"))
        .respond_with(ResponseTemplate::new(202))
        .mount(&server)
        .await;
    server
}

#[tokio::test]
async fn first_discovery_wait_happens_only_when_push_can_reach_the_client() {
    for off in [false, true] {
        let upstream = slow_upstream(Duration::from_millis(4_000)).await;
        let mut server = http_server(upstream.uri());
        server.capability_snapshot = None;
        let h = harness(vec![server], fast_limits());
        h.gateway.set_first_discovery_wait(Duration::from_secs(10));
        if off {
            h.relay.update_config(&off_config());
        }
        let started = Instant::now();
        h.initialize("s1", json!({})).await;
        let elapsed = started.elapsed();
        if off {
            assert!(
                elapsed < Duration::from_millis(3_000),
                "waited: {elapsed:?}"
            );
        } else {
            assert!(
                elapsed >= Duration::from_millis(3_500),
                "control did not wait: {elapsed:?}"
            );
        }
    }
}

#[tokio::test]
async fn global_off_with_one_event_allowed_reaches_only_that_client() {
    let (upstream, _) = http_upstream().await;
    let h = harness(vec![http_server(upstream.uri())], fast_limits());
    h.relay.update_config(&off_config());
    h.clients
        .write()
        .insert("client-2".into(), client("client-2"));
    let mut allowed = client(CLIENT);
    allowed.set_push_event_permission(HTTP_ID, "msg", Some(PushPermission::Allow));
    h.set_client(allowed);

    let init = h.initialize("a", json!({})).await.result.unwrap();
    assert_eq!(init["capabilities"]["events"], json!({}));
    let list = h.send("a", "events/list", None).await.result.unwrap();
    assert_eq!(event_names(&list), vec!["http-push__msg"]);
    assert!(h.tool_names("a").await.contains(&"EventPoll".to_string()));

    assert_no_push(&h, "client-2", "b").await;
}

#[tokio::test]
async fn fallback_tools_follow_the_show_rules() {
    let (upstream, _) = http_upstream().await;
    let h = harness(vec![http_server(upstream.uri())], fast_limits());
    let state = |declares_events, bridge| PushToolsState {
        client_id: CLIENT.into(),
        session_key: Some("s".into()),
        is_stdio_bridge: bridge,
        declares_events,
        visible_servers: vec![HTTP_ID.into()],
        push_test_source: false,
    };
    let view = h.relay.tools_view(&state(false, false));
    assert!(view.gate && view.list && view.poll && view.wait);
    // A client with native Events support does not need them
    let view = h.relay.tools_view(&state(true, false));
    assert!(view.gate && !view.list && !view.poll && !view.wait);
    // A bridge session gets all three: channel activation is undetectable
    let view = h.relay.tools_view(&state(false, true));
    assert!(view.list && view.poll && view.wait);
    // No push source: nothing
    let mut no_source = state(false, false);
    no_source.visible_servers.clear();
    assert_eq!(h.relay.tools_view(&no_source), Default::default());

    // Through the gateway: a client declaring the Events extension sees none
    h.initialize("native", json!({"extensions": {EVENTS_EXTENSION_ID: {}}}))
        .await;
    assert!(!h
        .tool_names("native")
        .await
        .iter()
        .any(|t| t.starts_with("Event")));
}

#[tokio::test]
async fn webhook_only_event_types_are_reported_unsupported() {
    let (upstream, _) = http_upstream().await;
    let h = harness(vec![http_server(upstream.uri())], fast_limits());
    let sources = h.relay.upstream_sources();
    let source = sources.iter().find(|s| s.server_id == HTTP_ID).unwrap();
    assert_eq!(source.connection, "shared");
    let hook = source
        .event_types
        .iter()
        .find(|e| e.name == "hook.only")
        .unwrap();
    assert!(!hook.supported);
    assert_eq!(
        hook.unsupported_reason.as_deref(),
        Some("webhook-only — not supported")
    );
    let msg = source.event_types.iter().find(|e| e.name == "msg").unwrap();
    assert_eq!(msg.ingress.as_deref(), Some("poll"));
}

// ---------------------------------------------------------------------------
// Ingress → egress
// ---------------------------------------------------------------------------

#[tokio::test]
async fn upstream_poll_reaches_events_poll_and_the_poll_tool() {
    let (upstream, calls, _) = http_upstream_with(true).await;
    let h = harness(vec![http_server(upstream.uri())], fast_limits());
    h.initialize("s1", json!({})).await;

    // Bootstrap: no history, a cursor at "now"
    let first = h.poll("s1", "http-push__msg", None).await;
    assert_eq!(first["events"], json!([]));
    assert_eq!(first["truncated"], json!(false));
    let tool_first = h
        .call_tool("s1", "EventPoll", json!({"events": ["http-push__msg"]}))
        .await;
    let tool_cursor = tool_first
        .lines()
        .find_map(|l| l.strip_prefix("next_cursor: "))
        .unwrap()
        .to_string();

    // The relay polls upstream on its own connection; the event arrives
    wait_until("upstream polls", || calls.load(Ordering::SeqCst) >= 2).await;
    let cursor = first["cursor"].as_str().unwrap().to_string();
    let batch = h
        .poll_until("s1", "http-push__msg", &cursor, |b| {
            !b["events"].as_array().unwrap().is_empty()
        })
        .await;
    let event = &batch["events"][0];
    assert_eq!(event["name"], "http-push__msg");
    assert_eq!(event["eventId"], "http-push__m1");
    assert_eq!(event["data"], json!({"text": "hi"}));
    assert_eq!(batch["nextPollMs"], json!(1000));

    // The same event through the fallback tool, rendered and attributed
    let text = h
        .call_tool(
            "s1",
            "EventPoll",
            json!({"events": ["http-push__msg"], "cursor": tool_cursor}),
        )
        .await;
    assert!(
        text.contains("<event id=\"http-push__m1\" name=\"http-push__msg\" server=\"http-push\""),
        "{text}"
    );
    assert!(text.contains("<untrusted-data"));
    assert!(text.contains("(no more events)"));
    assert!(text.contains("EventWait"));

    // Reading again with the returned cursor acknowledges: nothing new
    let next = batch["cursor"].as_str().unwrap();
    assert_eq!(
        h.poll("s1", "http-push__msg", Some(next)).await["events"],
        json!([])
    );

    // Monitor recorded the relayed occurrence
    assert!(h
        .monitor_decisions("http-push__msg")
        .contains(&"allowed".to_string()));

    // One upstream subscription, shared
    let subs = h.relay.active_subscriptions();
    assert!(subs
        .iter()
        .any(|s| s.server_id == HTTP_ID && s.event == "msg"));
}

#[tokio::test]
async fn server_push_status_reports_live_listening_counts_and_consumers() {
    let (upstream, calls, release) = http_upstream_with(false).await;
    let h = harness(vec![http_server(upstream.uri())], fast_limits());

    // Nothing consumes the server yet: not listening, nothing received
    let idle = h.relay.server_push_status(HTTP_ID);
    assert!(!idle.listening);
    assert!(idle.event_types.is_empty());
    assert_eq!(idle.events_received, 0);
    assert!(idle.last_event_at.is_none());
    assert_eq!(idle.consuming_clients, 0);

    // A poll opens the upstream subscription: listening via poll, no events
    let first = h.poll("s1", "http-push__msg", None).await;
    wait_until("upstream polled", || calls.load(Ordering::SeqCst) >= 1).await;
    wait_until("listening", || {
        h.relay.server_push_status(HTTP_ID).listening
    })
    .await;
    let listening = h.relay.server_push_status(HTTP_ID);
    let msg = listening
        .event_types
        .iter()
        .find(|t| t.event == "msg")
        .expect("msg is listened to");
    assert_eq!(msg.state, "listening");
    assert_eq!(msg.ingress_mode.as_deref(), Some("poll"));
    assert_eq!(msg.subscriptions, 1);
    assert_eq!(msg.events_received, 0);
    assert_eq!(listening.events_received, 0);
    assert_eq!(listening.consuming_clients, 1);
    assert_eq!(listening.consuming_sessions, 1);

    // The upstream emits one event: counted once, with its time
    let before = Utc::now();
    release.store(true, Ordering::SeqCst);
    let cursor = first["cursor"].as_str().unwrap().to_string();
    h.poll_until("s1", "http-push__msg", &cursor, |b| {
        !b["events"].as_array().unwrap().is_empty()
    })
    .await;
    let received = h.relay.server_push_status(HTTP_ID);
    assert_eq!(received.events_received, 1);
    assert!(received.last_event_at.unwrap() >= before - chrono::Duration::seconds(1));
    let msg = received
        .event_types
        .iter()
        .find(|t| t.event == "msg")
        .unwrap();
    assert_eq!(msg.events_received, 1);
    assert_eq!(msg.last_event_at, received.last_event_at);

    // Push turned off: the subscription closes, the count since start stays
    h.relay.update_config(&off_config());
    wait_until("not listening", || {
        !h.relay.server_push_status(HTTP_ID).listening
    })
    .await;
    let off = h.relay.server_push_status(HTTP_ID);
    assert_eq!(off.consuming_clients, 0);
    assert_eq!(off.events_received, 1);
    let msg = off.event_types.iter().find(|t| t.event == "msg").unwrap();
    assert_eq!(msg.state, "idle");
    assert_eq!(msg.subscriptions, 0);

    // Other servers are unaffected
    assert_eq!(h.relay.server_push_status("other").events_received, 0);
}

async fn recv(rx: &mut tokio::sync::mpsc::Receiver<JsonRpcNotification>) -> JsonRpcNotification {
    tokio::time::timeout(Duration::from_secs(10), rx.recv())
        .await
        .expect("notification in time")
        .expect("stream open")
}

async fn wait_until_async<F, Fut>(mut condition: F)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let deadline = Instant::now() + Duration::from_secs(10);
    while !condition().await {
        assert!(Instant::now() < deadline, "timed out");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test]
async fn shared_upstream_subscription_serves_several_clients() {
    let (upstream, calls, release) = http_upstream_with(false).await;
    let h = harness(vec![http_server(upstream.uri())], fast_limits());
    h.clients
        .write()
        .insert("client-2".into(), client("client-2"));
    let a = h.poll("a", "http-push__msg", None).await;
    let b = {
        h.send_as("client-2", "b", "initialize", Some(json!({"protocolVersion": "2025-11-25", "capabilities": {}, "clientInfo": {"name": "t", "version": "1"}}))).await;
        h.send_as(
            "client-2",
            "b",
            "events/poll",
            Some(json!({"name": "http-push__msg"})),
        )
        .await
        .result
        .unwrap()
    };
    release.store(true, Ordering::SeqCst);
    wait_until("upstream polls", || calls.load(Ordering::SeqCst) >= 2).await;
    let subs: Vec<_> = h
        .relay
        .active_subscriptions()
        .into_iter()
        .filter(|s| s.event == "msg")
        .collect();
    assert_eq!(subs.len(), 1, "one upstream subscription for both clients");
    wait_until_async(|| async {
        !h.poll("a", "http-push__msg", a["cursor"].as_str()).await["events"]
            .as_array()
            .unwrap()
            .is_empty()
    })
    .await;
    let b_events = h
        .send_as(
            "client-2",
            "b",
            "events/poll",
            Some(json!({"name": "http-push__msg", "cursor": b["cursor"]})),
        )
        .await
        .result
        .unwrap();
    assert_eq!(b_events["events"][0]["eventId"], "http-push__m1");
}

#[cfg(unix)]
#[tokio::test]
async fn upstream_stream_reaches_downstream_events_stream() {
    let mut h = harness(vec![stdio_server()], fast_limits());
    h.initialize("s1", json!({})).await;

    let sinks = h.gateway.push_stream_sinks();
    let (mut notifications, _guard) = sinks.register("s1", &json!(7), 64);
    let gateway = h.gateway.clone();
    let client = h.clients.read()[CLIENT].clone();
    let servers = h.servers.clone();
    let stream = tokio::spawn(async move {
        gateway
            .handle_request_with_skills(
                CLIENT,
                Some("s1"),
                servers,
                vec![],
                client.mcp_permissions.clone(),
                lr_config::SkillsPermissions::default(),
                client.name.clone(),
                PermissionState::Off,
                PermissionState::Off,
                None,
                None,
                PermissionState::default(),
                PermissionState::default(),
                None,
                None,
                lr_config::ClientMode::default(),
                lr_config::ClientPushSettings::from_client(&client),
                JsonRpcRequest::new(
                    Some(json!(7)),
                    "events/stream".into(),
                    Some(json!({"name": "stdio-push__tick"})),
                ),
                None,
            )
            .await
    });
    let active = recv(&mut notifications).await;
    assert_eq!(active.method, "notifications/events/active");
    assert_eq!(
        active.params.as_ref().unwrap()["_meta"]["io.modelcontextprotocol/subscriptionId"],
        json!(7)
    );
    let event = recv(&mut notifications).await;
    assert_eq!(event.method, "notifications/events/event");
    let params = event.params.unwrap();
    assert_eq!(params["name"], "stdio-push__tick");
    assert_eq!(params["eventId"], "stdio-push__t1");
    assert_eq!(params["data"], json!({"n": 1}));
    assert!(params["cursor"].as_str().unwrap().starts_with("lr1."));

    // A channel copy is not sent for a name this session streams
    // (the stdio upstream's channel message is a different name)
    let _ = h.rx.try_recv();

    // Cancellation ends the stream with its result
    h.send(
        "s1",
        "notifications/cancelled",
        Some(json!({"requestId": 7})),
    )
    .await;
    let response = tokio::time::timeout(Duration::from_secs(5), stream)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(response.result.unwrap()["resultType"], "complete");
}

#[cfg(unix)]
#[tokio::test]
async fn upstream_channel_reaches_poll_and_bridge_channel() {
    let mut h = harness(vec![stdio_server()], fast_limits());
    h.gateway.set_stdio_bridge_session("bridge", true);
    let init = h.initialize("bridge", json!({})).await.result.unwrap();
    assert_eq!(
        init["capabilities"]["experimental"][CLAUDE_CHANNEL_CAPABILITY],
        json!({})
    );

    // The upstream's channel message comes back to the bridge as a channel
    // message attributed to its server
    let channel = h
        .next_notification(
            "notifications/claude/channel",
            &session_notification_key("bridge"),
            Duration::from_secs(10),
        )
        .await
        .expect("channel egress");
    let params = channel.params.unwrap();
    assert!(params["content"]
        .as_str()
        .unwrap()
        .contains("hello from stdio"));
    assert_eq!(params["meta"]["event"], "stdio-push__channel.message");
    assert_eq!(params["meta"]["source"], "stdio-push");
    assert!(params["meta"]["event_id"]
        .as_str()
        .unwrap()
        .starts_with("stdio-push__"));
    assert!(h
        .monitor_decisions("stdio-push__channel.message")
        .contains(&"allowed".to_string()));

    // The status counts the channel message and shows the live session
    let status = h.relay.server_push_status(STDIO_ID);
    let channel = status
        .event_types
        .iter()
        .find(|t| t.event == "channel.message")
        .expect("channel ingress reported");
    assert!(channel.events_received >= 1);
    assert!(channel.last_event_at.is_some());
    assert_eq!(channel.state, "listening");
    assert_eq!(channel.ingress_mode.as_deref(), Some("channel"));
    assert!(status.listening);
    assert!(status.consuming_clients >= 1);
}

#[tokio::test]
async fn channel_copies_stop_once_the_session_uses_events_for_the_name() {
    let (upstream, _) = http_upstream().await;
    let mut h = harness(vec![http_server(upstream.uri())], fast_limits());
    h.gateway.set_stdio_bridge_session("bridge", true);
    h.initialize("bridge", json!({})).await;
    let key = session_notification_key("bridge");

    h.relay.admit(
        CLIENT,
        occurrence(
            HTTP_ID,
            "http-push",
            "alert",
            "a1",
            json!({"level": "high"}),
        ),
    );
    let first = h
        .next_notification("notifications/claude/channel", &key, Duration::from_secs(5))
        .await
        .unwrap();
    let params = first.params.unwrap();
    assert_eq!(params["meta"]["event"], "http-push__alert");
    assert_eq!(params["meta"]["event_id"], "http-push__a1");
    assert!(params["content"]
        .as_str()
        .unwrap()
        .starts_with("[http-push] http-push__alert (http-push__a1)"));

    // The session calls events/poll for the name: no more channel copies
    h.poll("bridge", "http-push__alert", None).await;
    h.relay.admit(
        CLIENT,
        occurrence(HTTP_ID, "http-push", "alert", "a2", json!({})),
    );
    assert!(h
        .next_notification(
            "notifications/claude/channel",
            &key,
            Duration::from_millis(500)
        )
        .await
        .is_none());
    // Other names keep coming
    h.relay.admit(
        CLIENT,
        occurrence(HTTP_ID, "http-push", "msg", "m9", json!({})),
    );
    assert!(h
        .next_notification("notifications/claude/channel", &key, Duration::from_secs(5))
        .await
        .is_some());
}

#[tokio::test]
async fn channel_content_and_rate_limits_hold_at_their_bounds() {
    let (upstream, _) = http_upstream().await;
    let mut limits = fast_limits();
    limits.channel_max_content_bytes = 120;
    limits.channel_rate_per_minute = 2;
    let mut h = harness(vec![http_server(upstream.uri())], limits);
    h.gateway.set_stdio_bridge_session("bridge", true);
    h.initialize("bridge", json!({})).await;
    let key = session_notification_key("bridge");
    for i in 0..4 {
        h.relay.admit(
            CLIENT,
            occurrence(
                HTTP_ID,
                "http-push",
                "msg",
                &format!("m{i}"),
                json!({"text": "x".repeat(400)}),
            ),
        );
    }
    let mut contents = Vec::new();
    while let Some(n) = h
        .next_notification("notifications/claude/channel", &key, Duration::from_secs(3))
        .await
    {
        contents.push(n.params.unwrap()["content"].as_str().unwrap().to_string());
        if contents.len() == 3 {
            break;
        }
    }
    assert_eq!(
        contents.len(),
        2,
        "two messages within the rate: {contents:?}"
    );
    assert!(contents
        .iter()
        .all(|c| c.len() <= 120 && c.contains("EventPoll")));
}

#[tokio::test]
async fn default_channel_rate_coalesces_the_eleventh_message_in_a_minute() {
    let (upstream, _) = http_upstream().await;
    let mut h = harness(vec![http_server(upstream.uri())], fast_limits());
    assert_eq!(h.relay.limits().channel_rate_per_minute, 10);
    h.gateway.set_stdio_bridge_session("bridge", true);
    h.initialize("bridge", json!({})).await;
    // Poll from another session: polling a name on the bridge session would
    // stop its channel copies of that name
    h.initialize("poller", json!({})).await;
    let start = h.poll("poller", "http-push__msg", None).await["cursor"]
        .as_str()
        .unwrap()
        .to_string();
    let key = session_notification_key("bridge");
    for i in 0..11 {
        h.relay.admit(
            CLIENT,
            occurrence(HTTP_ID, "http-push", "msg", &format!("m{i}"), json!(i)),
        );
    }
    let mut ids = Vec::new();
    while let Some(n) = h
        .next_notification("notifications/claude/channel", &key, Duration::from_secs(3))
        .await
    {
        ids.push(n.params.unwrap()["meta"]["event_id"].clone());
        if ids.len() > 10 {
            break;
        }
    }
    assert_eq!(ids.len(), 10, "ten channel messages per minute: {ids:?}");
    assert!(!ids.contains(&json!("http-push__m10")));
    // The coalesced event is still in the journal for EventPoll
    let batch = h.poll("poller", "http-push__msg", Some(&start)).await;
    assert_eq!(batch["events"].as_array().unwrap().len(), 11);
}

#[tokio::test]
async fn rate_and_history_settings_apply_to_the_running_relay() {
    let (upstream, _) = http_upstream().await;
    let h = harness(vec![http_server(upstream.uri())], fast_limits());
    h.initialize("s1", json!({})).await;
    let start = h.poll("s1", "http-push__msg", None).await["cursor"]
        .as_str()
        .unwrap()
        .to_string();
    for i in 0..150 {
        h.relay.admit(
            CLIENT,
            occurrence(HTTP_ID, "http-push", "msg", &format!("m{i}"), json!(i)),
        );
    }
    assert_eq!(h.relay.journal_len(CLIENT), 150);

    // Lowering the history bound trims the existing journal at once
    h.relay.update_config(&PushRelayConfig {
        channel_messages_per_minute: 42,
        history_retention_hours: 2,
        history_max_events: 100,
        ..PushRelayConfig::default()
    });
    let limits = h.relay.limits();
    assert_eq!(limits.channel_rate_per_minute, 42);
    assert_eq!(limits.journal_retention_secs, 2 * 3600);
    assert_eq!(limits.journal_max_events, 100);
    assert_eq!(
        limits.upstream_poll_min_interval_ms, 50,
        "internal bounds kept"
    );
    assert_eq!(h.relay.journal_len(CLIENT), 100);
    let batch = h.poll("s1", "http-push__msg", Some(&start)).await;
    assert_eq!(batch["truncated"], json!(true));

    // The new bound holds for later events too
    for i in 150..160 {
        h.relay.admit(
            CLIENT,
            occurrence(HTTP_ID, "http-push", "msg", &format!("m{i}"), json!(i)),
        );
    }
    assert_eq!(h.relay.journal_len(CLIENT), 100);

    // Back to the defaults
    h.relay.update_config(&PushRelayConfig::default());
    let limits = h.relay.limits();
    assert_eq!(limits.channel_rate_per_minute, 10);
    assert_eq!(limits.journal_retention_secs, 24 * 3600);
    assert_eq!(limits.journal_max_events, 10_000);
}

#[tokio::test]
async fn relay_starts_with_the_configured_settings() {
    let config = PushRelayConfig {
        channel_messages_per_minute: 5,
        history_retention_hours: 48,
        history_max_events: 2_000,
        ..PushRelayConfig::default()
    };
    let gateway = McpGateway::new(
        Arc::new(McpServerManager::new()),
        GatewayConfig::default(),
        test_router(),
    );
    let relay = PushRelay::new(gateway, config, Arc::new(|_| None));
    let limits = relay.limits();
    assert_eq!(limits.channel_rate_per_minute, 5);
    assert_eq!(limits.journal_retention_secs, 48 * 3600);
    assert_eq!(limits.journal_max_events, 2_000);
}

// ---------------------------------------------------------------------------
// Ask
// ---------------------------------------------------------------------------

fn ask_client(event: &str) -> lr_config::Client {
    let mut c = client(CLIENT);
    c.push_permissions = PushPermissions::default();
    c.push_permissions.events.insert(
        PushPermissions::event_key(HTTP_ID, event),
        PushPermission::Ask,
    );
    c
}

#[tokio::test]
async fn ask_holds_until_allowed_with_an_edit() {
    let (upstream, _) = http_upstream().await;
    let h = harness(vec![http_server(upstream.uri())], fast_limits());
    h.set_client(ask_client("msg"));
    h.initialize("s1", json!({})).await;
    let start = h.poll("s1", "http-push__msg", None).await["cursor"]
        .as_str()
        .unwrap()
        .to_string();

    h.relay.admit(
        CLIENT,
        occurrence(HTTP_ID, "http-push", "msg", "m1", json!({"text": "secret"})),
    );
    wait_until("held", || !h.relay.pending_events(Some(CLIENT)).is_empty()).await;
    // Held: not in the journal
    assert_eq!(
        h.poll("s1", "http-push__msg", Some(&start)).await["events"],
        json!([])
    );
    assert_eq!(h.monitor_decisions("http-push__msg"), vec!["pending"]);

    // The firewall popup shows client, server, event and data
    wait_until("popup", || {
        h.gateway
            .firewall_manager
            .list_pending()
            .iter()
            .any(|p| p.is_push_event_request)
    })
    .await;
    let popup = h
        .gateway
        .firewall_manager
        .list_pending()
        .into_iter()
        .find(|p| p.is_push_event_request)
        .unwrap();
    let details = popup.push_event_details.unwrap();
    assert_eq!(details.qualified_name, "http-push__msg");
    assert_eq!(details.data, json!({"text": "secret"}));
    assert_eq!(popup.client_id, CLIENT);
    assert_eq!(details.pending_of_type, 1);

    let hold = h.relay.pending_events(Some(CLIENT))[0].hold_id.clone();
    h.relay
        .decide(
            &hold,
            PushDecision {
                action: PushDecisionAction::Allow,
                edited_data: Some(json!({"text": "[redacted]"})),
            },
        )
        .unwrap();
    let batch = h
        .poll_until("s1", "http-push__msg", &start, |b| {
            !b["events"].as_array().unwrap().is_empty()
        })
        .await;
    assert_eq!(batch["events"][0]["data"], json!({"text": "[redacted]"}));
    wait_until("monitor", || {
        h.monitor_decisions("http-push__msg") == vec!["asked_edited"]
    })
    .await;
    let record = h
        .monitor
        .lock()
        .iter()
        .find(|e| e.event_type == lr_monitor::MonitorEventType::McpPushEvent)
        .cloned()
        .unwrap();
    match record.data {
        lr_monitor::MonitorEventData::McpPushEvent {
            original_payload,
            delivered_payload,
            ..
        } => {
            assert_eq!(original_payload, Some(json!({"text": "secret"})));
            assert_eq!(delivered_payload, Some(json!({"text": "[redacted]"})));
        }
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn held_queue_changes_are_announced_and_turning_off_releases_them() {
    let (upstream, _) = http_upstream().await;
    let h = harness(vec![http_server(upstream.uri())], fast_limits());
    let changes = Arc::new(AtomicUsize::new(0));
    let counter = changes.clone();
    h.relay.set_held_changed_listener(Arc::new(move || {
        counter.fetch_add(1, Ordering::SeqCst);
    }));
    h.set_client(ask_client("msg"));
    h.initialize("s1", json!({})).await;

    h.relay.admit(
        CLIENT,
        occurrence(HTTP_ID, "http-push", "msg", "m1", json!({"text": "one"})),
    );
    wait_until("held", || !h.relay.pending_events(Some(CLIENT)).is_empty()).await;
    let after_hold = changes.load(Ordering::SeqCst);
    assert!(after_hold >= 1, "holding an event is announced");

    // The user turns the type off: the held event is denied without an answer
    let mut off = ask_client("msg");
    off.push_permissions.events.insert(
        PushPermissions::event_key(HTTP_ID, "msg"),
        PushPermission::Off,
    );
    h.set_client(off);
    wait_until("released", || {
        h.relay.pending_events(Some(CLIENT)).is_empty()
    })
    .await;
    assert!(changes.load(Ordering::SeqCst) > after_hold);
    wait_until("monitor", || {
        h.monitor_decisions("http-push__msg") == vec!["denied"]
    })
    .await;
}

#[tokio::test]
async fn ask_is_fifo_per_type_and_denial_drops() {
    let (upstream, _) = http_upstream().await;
    let h = harness(vec![http_server(upstream.uri())], fast_limits());
    let mut c = ask_client("msg");
    c.push_permissions.events.insert(
        PushPermissions::event_key(HTTP_ID, "alert"),
        PushPermission::Ask,
    );
    h.set_client(c);
    h.initialize("s1", json!({})).await;
    let msg_start = h.poll("s1", "http-push__msg", None).await["cursor"]
        .as_str()
        .unwrap()
        .to_string();
    let alert_start = h.poll("s1", "http-push__alert", None).await["cursor"]
        .as_str()
        .unwrap()
        .to_string();

    for id in ["m1", "m2", "m3"] {
        h.relay.admit(
            CLIENT,
            occurrence(HTTP_ID, "http-push", "msg", id, json!({"id": id})),
        );
    }
    h.relay.admit(
        CLIENT,
        occurrence(HTTP_ID, "http-push", "alert", "a1", json!({})),
    );
    wait_until("held", || h.relay.pending_events(Some(CLIENT)).len() == 4).await;
    let pending = h.relay.pending_events(Some(CLIENT));
    let id_of = |event: &str| {
        pending
            .iter()
            .find(|p| p.event_id == format!("http-push__{event}"))
            .unwrap()
            .hold_id
            .clone()
    };

    // Deciding a later event first does not let it overtake the first one
    h.relay
        .decide(&id_of("m2"), PushDecision::new(PushDecisionAction::Allow))
        .unwrap();
    h.relay
        .decide(&id_of("m3"), PushDecision::new(PushDecisionAction::Deny))
        .unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        h.poll("s1", "http-push__msg", Some(&msg_start)).await["events"],
        json!([])
    );
    // Another type is not blocked by the held msg
    h.relay
        .decide(&id_of("a1"), PushDecision::new(PushDecisionAction::Allow))
        .unwrap();
    wait_until_async(|| async {
        !h.poll("s1", "http-push__alert", Some(&alert_start)).await["events"]
            .as_array()
            .unwrap()
            .is_empty()
    })
    .await;

    wait_until("popup for m1", || {
        h.gateway
            .firewall_manager
            .push_request_for_hold(&id_of("m1"))
            .is_some()
    })
    .await;
    h.relay
        .decide(&id_of("m1"), PushDecision::new(PushDecisionAction::Allow))
        .unwrap();
    let batch = h
        .poll_until("s1", "http-push__msg", &msg_start, |b| {
            b["events"].as_array().unwrap().len() >= 2
        })
        .await;
    let ids: Vec<String> = batch["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["eventId"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(ids, vec!["http-push__m1", "http-push__m2"], "m3 was denied");
    wait_until("decisions", || {
        h.relay.pending_events(Some(CLIENT)).is_empty()
    })
    .await;
    let decisions = h.monitor_decisions("http-push__msg");
    assert_eq!(
        decisions.iter().filter(|d| *d == "asked_allowed").count(),
        2
    );
    assert_eq!(decisions.iter().filter(|d| *d == "denied").count(), 1);
}

#[tokio::test]
async fn held_events_wait_without_a_timeout_and_respect_the_hold_cap() {
    let (upstream, _) = http_upstream().await;
    let mut limits = fast_limits();
    limits.ask_hold_cap = 2;
    let h = harness(vec![http_server(upstream.uri())], limits);
    h.set_client(ask_client("msg"));
    h.initialize("s1", json!({})).await;
    let start = h.poll("s1", "http-push__msg", None).await["cursor"]
        .as_str()
        .unwrap()
        .to_string();

    for id in ["m1", "m2", "m3"] {
        h.relay.admit(
            CLIENT,
            occurrence(HTTP_ID, "http-push", "msg", id, json!({})),
        );
    }
    // The third exceeds the cap of two
    assert!(h
        .monitor_decisions("http-push__msg")
        .contains(&"hold_cap".to_string()));
    wait_until("popup open", || {
        h.gateway
            .firewall_manager
            .list_pending()
            .iter()
            .any(|p| p.is_push_event_request)
    })
    .await;
    // Nothing decides the held events: they stay held and the popup stays
    tokio::time::sleep(Duration::from_millis(1_500)).await;
    assert!(h.gateway.firewall_manager.cleanup_expired().is_empty());
    assert_eq!(h.relay.pending_events(None).len(), 2);
    assert_eq!(
        h.monitor_decisions("http-push__msg")
            .iter()
            .filter(|d| *d == "pending")
            .count(),
        2
    );
    assert_eq!(
        h.poll("s1", "http-push__msg", Some(&start)).await["events"],
        json!([])
    );
    // A decision still releases them, in order
    let first = h.relay.pending_events(None)[0].hold_id.clone();
    h.relay
        .decide(&first, PushDecision::new(PushDecisionAction::Allow))
        .unwrap();
    wait_until("first delivered", || {
        h.monitor_decisions("http-push__msg")
            .contains(&"asked_allowed".to_string())
    })
    .await;
    assert_eq!(h.relay.pending_events(None).len(), 1);
}

/// The Monitor request id of the push record for one upstream event id.
fn monitor_request_of(h: &Harness, event_id: &str) -> Option<String> {
    h.monitor
        .lock()
        .iter()
        .find(|e| {
            matches!(&e.data, lr_monitor::MonitorEventData::McpPushEvent { event_id: id, .. } if id == event_id)
        })
        .expect("push record")
        .session_id
        .clone()
}

#[tokio::test]
async fn monitor_records_join_the_one_reading_session() {
    let (upstream, _) = http_upstream().await;
    let h = harness(vec![http_server(upstream.uri())], fast_limits());
    h.initialize("s1", json!({})).await;
    h.gateway
        .get_session("s1")
        .unwrap()
        .write()
        .await
        .monitor_session_id = Some("req-1".into());
    h.poll("s1", "http-push__msg", None).await;
    let admit = |id: &str, origin: Option<&str>| {
        let mut occurrence = (*occurrence(HTTP_ID, "http-push", "msg", id, json!({}))).clone();
        occurrence.origin_session = origin.map(str::to_string);
        h.relay.admit(CLIENT, Arc::new(occurrence));
    };

    // Only s1 reads the subscription: its request id
    admit("a1", None);
    assert_eq!(
        monitor_request_of(&h, "http-push__a1").as_deref(),
        Some("req-1")
    );

    // An event from s1's own upstream connection belongs to s1
    admit("a2", Some("s1"));
    assert_eq!(
        monitor_request_of(&h, "http-push__a2").as_deref(),
        Some("req-1")
    );

    // A session without a Monitor request id groups nothing, like its tool calls
    h.initialize("s2", json!({})).await;
    admit("a3", Some("s2"));
    assert_eq!(monitor_request_of(&h, "http-push__a3"), None);

    // Two sessions read the same subscription: no single session, standalone
    h.poll("s2", "http-push__msg", None).await;
    admit("a4", None);
    assert_eq!(monitor_request_of(&h, "http-push__a4"), None);
}

#[tokio::test]
async fn monitor_records_read_by_no_session_stand_alone() {
    let (upstream, _) = http_upstream().await;
    let h = harness(vec![http_server(upstream.uri())], fast_limits());
    h.initialize("s1", json!({})).await;
    h.gateway
        .get_session("s1")
        .unwrap()
        .write()
        .await
        .monitor_session_id = Some("req-1".into());
    // EventWait from s1: the events are read by the watcher command, which
    // no session is
    let text = h
        .call_tool("s1", "EventWait", json!({"events": ["http-push__msg"]}))
        .await;
    let url_start = text.find("http://127.0.0.1:3625/events/watch?").unwrap();
    let url = &text[url_start..text[url_start..].find('"').unwrap() + url_start];
    let parsed = reqwest::Url::parse(url).unwrap();
    let query: HashMap<String, String> = parsed.query_pairs().into_owned().collect();
    let watch = h
        .relay
        .open_watch(&query["token"], query.get("cursor").map(String::as_str))
        .unwrap();
    h.relay.admit(
        CLIENT,
        occurrence(HTTP_ID, "http-push", "msg", "w1", json!({})),
    );
    assert_eq!(monitor_request_of(&h, "http-push__w1"), None);
    drop(watch);
}

#[tokio::test]
async fn permanent_decisions_persist_and_apply_to_later_events() {
    let (upstream, _) = http_upstream().await;
    let h = harness(vec![http_server(upstream.uri())], fast_limits());
    h.set_client(ask_client("msg"));
    h.initialize("s1", json!({})).await;
    let start = h.poll("s1", "http-push__msg", None).await["cursor"]
        .as_str()
        .unwrap()
        .to_string();

    h.relay.admit(
        CLIENT,
        occurrence(HTTP_ID, "http-push", "msg", "m1", json!({})),
    );
    h.relay.admit(
        CLIENT,
        occurrence(HTTP_ID, "http-push", "msg", "m2", json!({})),
    );
    wait_until("held", || h.relay.pending_events(Some(CLIENT)).len() == 2).await;
    // "Allow all pending of this type" via the type-wide decision on the head
    let head = h.relay.pending_events(Some(CLIENT))[0].hold_id.clone();
    wait_until("popup", || {
        h.gateway
            .firewall_manager
            .push_request_for_hold(&head)
            .is_some()
    })
    .await;
    h.relay
        .decide(&head, PushDecision::new(PushDecisionAction::AllowAlways))
        .unwrap();
    wait_until("persisted", || !h.persisted.lock().is_empty()).await;
    assert_eq!(
        h.persisted.lock()[0],
        (
            CLIENT.to_string(),
            PushConfigChange::EventPermission {
                server_id: HTTP_ID.into(),
                event_name: "msg".into(),
                permission: PushPermission::Allow
            }
        )
    );
    // Both delivered; a later event goes straight through
    h.relay.admit(
        CLIENT,
        occurrence(HTTP_ID, "http-push", "msg", "m3", json!({})),
    );
    wait_until_async(|| async {
        h.poll("s1", "http-push__msg", Some(&start)).await["events"]
            .as_array()
            .unwrap()
            .len()
            == 3
    })
    .await;
    assert!(h.relay.pending_events(None).is_empty());
}

#[tokio::test]
async fn off_and_oversized_events_are_not_delivered() {
    let (upstream, _) = http_upstream().await;
    let mut limits = fast_limits();
    limits.max_event_bytes = 64;
    let h = harness(vec![http_server(upstream.uri())], limits);
    h.initialize("s1", json!({})).await;
    let start = h.poll("s1", "http-push__msg", None).await["cursor"]
        .as_str()
        .unwrap()
        .to_string();
    h.relay.admit(
        CLIENT,
        occurrence(
            HTTP_ID,
            "http-push",
            "msg",
            "big",
            json!({"text": "x".repeat(100)}),
        ),
    );
    h.relay.admit(
        CLIENT,
        occurrence(HTTP_ID, "http-push", "msg", "ok", json!({"text": "x"})),
    );
    let batch = h.poll("s1", "http-push__msg", Some(&start)).await;
    assert_eq!(batch["events"].as_array().unwrap().len(), 1);
    assert_eq!(
        h.monitor_decisions("http-push__msg"),
        vec!["too_large", "allowed"]
    );

    let mut off = client(CLIENT);
    off.push_permissions.events.insert(
        PushPermissions::event_key(HTTP_ID, "msg"),
        PushPermission::Off,
    );
    h.set_client(off);
    h.relay.admit(
        CLIENT,
        occurrence(HTTP_ID, "http-push", "msg", "b1", json!({})),
    );
    assert!(h
        .monitor_decisions("http-push__msg")
        .contains(&"off".to_string()));
    // An Off type is no longer listed and cannot be polled
    let poll = h
        .send("s1", "events/poll", Some(json!({"name": "http-push__msg"})))
        .await;
    assert_eq!(poll.error.unwrap().code, -32023);
}

// ---------------------------------------------------------------------------
// Cursors and journal bounds through the protocol
// ---------------------------------------------------------------------------

#[tokio::test]
async fn cursors_are_scoped_and_report_truncation() {
    let (upstream, _) = http_upstream().await;
    let mut limits = fast_limits();
    limits.journal_max_events = 2;
    let h = harness(vec![http_server(upstream.uri())], limits);
    h.initialize("s1", json!({})).await;
    let start = h.poll("s1", "http-push__msg", None).await["cursor"]
        .as_str()
        .unwrap()
        .to_string();
    for i in 0..3 {
        h.relay.admit(
            CLIENT,
            occurrence(HTTP_ID, "http-push", "msg", &format!("m{i}"), json!(i)),
        );
    }
    let batch = h.poll("s1", "http-push__msg", Some(&start)).await;
    assert_eq!(batch["truncated"], json!(true));
    assert_eq!(batch["events"].as_array().unwrap().len(), 2);
    // maxEvents pages with hasMore
    let paged = h
        .send(
            "s1",
            "events/poll",
            Some(json!({"name": "http-push__msg", "cursor": start, "maxEvents": 1})),
        )
        .await
        .result
        .unwrap();
    assert_eq!(paged["hasMore"], json!(true));
    // A cursor of another scope or a garbage cursor is refused
    let other = h
        .send(
            "s1",
            "events/poll",
            Some(json!({"name": "http-push__alert", "cursor": start})),
        )
        .await;
    assert_eq!(other.error.unwrap().code, -32602);
    let bad = h
        .send(
            "s1",
            "events/poll",
            Some(json!({"name": "http-push__msg", "cursor": "nope"})),
        )
        .await;
    assert_eq!(bad.error.unwrap().code, -32602);
    // A cursor from another LocalRouter run resets with truncation
    let other_relay = harness(vec![http_server(upstream.uri())], fast_limits());
    other_relay.initialize("x", json!({})).await;
    let foreign = other_relay.poll("x", "http-push__msg", None).await["cursor"]
        .as_str()
        .unwrap()
        .to_string();
    let reset = h.poll("s1", "http-push__msg", Some(&foreign)).await;
    assert_eq!(reset["truncated"], json!(true));
    assert_eq!(reset["events"], json!([]));
    // Unknown names are NotFound
    let missing = h
        .send(
            "s1",
            "events/poll",
            Some(json!({"name": "http-push__nope"})),
        )
        .await;
    assert_eq!(missing.error.unwrap().code, -32023);
}

#[tokio::test]
async fn stream_limit_per_session_holds() {
    let (upstream, _) = http_upstream().await;
    let mut limits = fast_limits();
    limits.max_streams_per_session = 1;
    let h = harness(vec![http_server(upstream.uri())], limits);
    h.initialize("s1", json!({})).await;
    let sinks = h.gateway.push_stream_sinks();
    let (_rx, _guard) = sinks.register("s1", &json!(1), 8);
    let gateway = h.gateway.clone();
    let client = h.clients.read()[CLIENT].clone();
    let servers = h.servers.clone();
    let first = tokio::spawn(async move {
        gateway
            .handle_request_with_skills(
                CLIENT,
                Some("s1"),
                servers,
                vec![],
                client.mcp_permissions.clone(),
                lr_config::SkillsPermissions::default(),
                client.name.clone(),
                PermissionState::Off,
                PermissionState::Off,
                None,
                None,
                PermissionState::default(),
                PermissionState::default(),
                None,
                None,
                lr_config::ClientMode::default(),
                lr_config::ClientPushSettings::from_client(&client),
                JsonRpcRequest::new(
                    Some(json!(1)),
                    "events/stream".into(),
                    Some(json!({"name": "http-push__msg"})),
                ),
                None,
            )
            .await
    });
    tokio::time::sleep(Duration::from_millis(300)).await;
    let second = h
        .send(
            "s1",
            "events/stream",
            Some(json!({"name": "http-push__msg"})),
        )
        .await;
    assert_eq!(second.error.unwrap().code, -32025);
    first.abort();
}

// ---------------------------------------------------------------------------
// Fallback EventWait and /events/watch
// ---------------------------------------------------------------------------

#[tokio::test]
async fn event_wait_command_watches_events() {
    let (upstream, _) = http_upstream().await;
    let h = harness(vec![http_server(upstream.uri())], fast_limits());
    h.initialize("s1", json!({})).await;
    let text = h
        .call_tool("s1", "EventWait", json!({"events": ["http-push__alert"]}))
        .await;
    assert!(text.contains("MULTI-SHOT") && text.contains("ONE-SHOT"));
    let url_start = text.find("http://127.0.0.1:3625/events/watch?").unwrap();
    let url = &text[url_start..text[url_start..].find('"').unwrap() + url_start];
    let parsed = reqwest::Url::parse(url).unwrap();
    let query: HashMap<String, String> = parsed.query_pairs().into_owned().collect();

    let mut watch = h
        .relay
        .open_watch(&query["token"], query.get("cursor").map(String::as_str))
        .unwrap();
    h.relay.admit(
        CLIENT,
        occurrence(HTTP_ID, "http-push", "msg", "m1", json!({})),
    );
    h.relay.admit(
        CLIENT,
        occurrence(HTTP_ID, "http-push", "alert", "a1", json!({"level": 1})),
    );
    let line = tokio::time::timeout(Duration::from_secs(5), watch.next())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(line["eventId"], "http-push__a1", "only the watched type");
    assert!(line["cursor"].as_str().is_some());

    // A tampered token is refused
    let forged = format!("{}x", query["token"]);
    assert!(h.relay.open_watch(&forged, None).is_err());
}

// ---------------------------------------------------------------------------
// Webhook egress
// ---------------------------------------------------------------------------

/// Headers and body of each delivery a receiver accepted.
type Deliveries = Arc<parking_lot::Mutex<Vec<(HashMap<String, String>, Vec<u8>)>>>;

struct Receiver {
    deliveries: Deliveries,
    fail_first: Arc<AtomicUsize>,
}

impl Respond for Receiver {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let body: Value = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
        if body["type"] == "verification" {
            return ResponseTemplate::new(200)
                .set_body_json(json!({"challenge": body["challenge"]}));
        }
        if self.fail_first.load(Ordering::SeqCst) > 0 {
            self.fail_first.fetch_sub(1, Ordering::SeqCst);
            return ResponseTemplate::new(500);
        }
        let headers = request
            .headers
            .iter()
            .map(|(k, v)| {
                (
                    k.as_str().to_string(),
                    v.to_str().unwrap_or_default().to_string(),
                )
            })
            .collect();
        self.deliveries.lock().push((headers, request.body.clone()));
        ResponseTemplate::new(200)
    }
}

#[tokio::test]
async fn webhook_subscription_asks_for_new_urls_signs_and_retries() {
    let (upstream, _) = http_upstream().await;
    let mut limits = fast_limits();
    limits.webhook_retry_delays_secs = vec![0, 0];
    let h = harness(vec![http_server(upstream.uri())], limits);
    h.initialize("s1", json!({})).await;

    let receiver = MockServer::start().await;
    let deliveries = Arc::new(parking_lot::Mutex::new(Vec::new()));
    let fail_first = Arc::new(AtomicUsize::new(1));
    Mock::given(http_method("POST"))
        .respond_with(Receiver {
            deliveries: deliveries.clone(),
            fail_first: fail_first.clone(),
        })
        .mount(&receiver)
        .await;
    use base64::Engine;
    let secret = format!(
        "whsec_{}",
        base64::engine::general_purpose::STANDARD.encode([9u8; 32])
    );
    let url = format!("{}/hook", receiver.uri());

    // Not HTTPS and not loopback: refused
    let refused = h
        .send("s1", "events/subscribe", Some(json!({"name": "http-push__msg", "delivery": {"mode": "webhook", "url": "http://example.com/hook", "secret": secret}})))
        .await;
    assert_eq!(refused.error.unwrap().code, -32602);

    let subscribed = h
        .send("s1", "events/subscribe", Some(json!({"name": "http-push__msg", "delivery": {"mode": "webhook", "url": url, "secret": secret}, "ttlMs": 10})))
        .await
        .result
        .unwrap();
    let sub_id = subscribed["id"].as_str().unwrap().to_string();
    assert!(subscribed["refreshBefore"].as_str().is_some());
    assert!(subscribed["cursor"].as_str().unwrap().starts_with("lr1."));

    // First delivery to a new URL waits for approval
    h.relay.admit(
        CLIENT,
        occurrence(HTTP_ID, "http-push", "msg", "m1", json!({"text": "hook"})),
    );
    let hold = format!("webhook:{sub_id}");
    wait_until("url approval", || {
        h.gateway
            .firewall_manager
            .push_request_for_hold(&hold)
            .is_some()
    })
    .await;
    assert!(deliveries.lock().is_empty());
    assert_eq!(
        h.relay.webhook_subscriptions(Some(CLIENT))[0].state,
        "awaiting_approval"
    );
    h.relay
        .decide(&hold, PushDecision::new(PushDecisionAction::AllowAlways))
        .unwrap();
    wait_until("delivery", || !deliveries.lock().is_empty()).await;
    assert!(h.persisted.lock().contains(&(
        CLIENT.to_string(),
        PushConfigChange::WebhookUrl { url: url.clone() }
    )));

    let (headers, body) = deliveries.lock()[0].clone();
    assert_eq!(headers["webhook-id"], "http-push__m1");
    assert_eq!(headers["x-mcp-subscription-id"], sub_id);
    let timestamp: i64 = headers["webhook-timestamp"].parse().unwrap();
    let key = webhook::decode_secret(&secret).unwrap();
    let expected = webhook::signed_headers(&body, "http-push__m1", &sub_id, &key, timestamp);
    let expected = expected
        .iter()
        .find(|(n, _)| *n == "webhook-signature")
        .unwrap()
        .1
        .clone();
    assert_eq!(headers["webhook-signature"], expected);
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["data"], json!({"text": "hook"}));
    assert_eq!(
        fail_first.load(Ordering::SeqCst),
        0,
        "retried after the failure"
    );
    assert_eq!(h.relay.webhook_subscriptions(Some(CLIENT))[0].delivered, 1);

    // Unsubscribe ends it
    let gone = h
        .send(
            "s1",
            "events/unsubscribe",
            Some(json!({"name": "http-push__msg", "delivery": {"url": url}})),
        )
        .await;
    assert!(gone.error.is_none());
    assert!(h.relay.webhook_subscriptions(Some(CLIENT)).is_empty());
}

// ---------------------------------------------------------------------------
// Real peers: `netget --mcp` upstream, the real STDIO bridge downstream
// ---------------------------------------------------------------------------

/// The first `127.0.0.1:<port>` or `port <n>` / `port: <n>` in a text.
fn find_port(text: &str) -> Option<u16> {
    let digits = |rest: &str| -> Option<u16> {
        let n: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        n.parse().ok().filter(|p| *p > 0)
    };
    for marker in ["127.0.0.1:", "port: ", "port ", "Port: ", "Port "] {
        for (i, _) in text.match_indices(marker) {
            if let Some(port) = digits(&text[i + marker.len()..]) {
                return Some(port);
            }
        }
    }
    None
}

struct BridgeProcess {
    child: tokio::process::Child,
    stdin: tokio::process::ChildStdin,
    lines: tokio::io::Lines<tokio::io::BufReader<tokio::process::ChildStdout>>,
    seen: Vec<Value>,
}

impl BridgeProcess {
    async fn write(&mut self, message: Value) {
        use tokio::io::AsyncWriteExt;
        self.stdin
            .write_all(format!("{message}\n").as_bytes())
            .await
            .unwrap();
        self.stdin.flush().await.unwrap();
    }

    /// Read lines until one satisfies `want` (every line is kept in `seen`).
    async fn read_until(
        &mut self,
        timeout: Duration,
        want: impl Fn(&Value) -> bool,
    ) -> Option<Value> {
        if let Some(found) = self.seen.iter().find(|v| want(v)) {
            return Some(found.clone());
        }
        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let line = tokio::time::timeout(remaining, self.lines.next_line())
                .await
                .ok()?
                .ok()??;
            let Ok(value) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            self.seen.push(value.clone());
            if want(&value) {
                return Some(value);
            }
        }
    }

    async fn request(&mut self, id: u64, method: &str, params: Value) -> Value {
        self.write(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))
            .await;
        self.read_until(Duration::from_secs(60), |v| {
            v["id"] == json!(id) && v.get("method").is_none()
        })
        .await
        .unwrap_or_else(|| panic!("no answer to {method}"))
    }
}

/// `netget --mcp --llm-agent` behind LocalRouter, reached by a client through
/// the real `localrouter --mcp-bridge`: NetGet's MCP Events come back to the
/// client as Claude Code channel messages, and are readable with `events/poll`.
///
/// Needs the netget binary (`NETGET_BIN`, default `/Users/matus/bin/netget`)
/// and a built `localrouter` binary next to this test.
#[tokio::test]
#[ignore = "drives the real netget binary and the real STDIO bridge; run with --ignored"]
async fn netget_events_reach_a_client_through_the_real_bridge() {
    use localrouter::clients::{ClientManager, TokenStore};
    use tokio::io::AsyncBufReadExt;

    let netget = std::env::var("NETGET_BIN").unwrap_or_else(|_| "/Users/matus/bin/netget".into());
    assert!(
        std::path::Path::new(&netget).exists(),
        "netget binary not found at {netget}"
    );
    let bridge_bin =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../target/debug/localrouter");
    assert!(bridge_bin.exists(), "build the localrouter binary first");

    let mut netget_server = stdio_server();
    netget_server.id = "netget".into();
    netget_server.name = "NetGet".into();
    netget_server.transport_config = McpTransportConfig::Stdio {
        command: netget.clone(),
        args: vec!["--mcp".into(), "--llm-agent".into()],
        env: HashMap::new(),
        cwd: Some(std::env::temp_dir().to_string_lossy().to_string()),
    };
    netget_server.capability_snapshot = None;

    let mut pr_client = client("pr-e2e");
    pr_client.enabled = true;
    let config = AppConfig {
        clients: vec![pr_client.clone()],
        strategies: vec![lr_config::Strategy::new("Default".to_string())],
        mcp_servers: vec![netget_server.clone()],
        ..Default::default()
    };
    let config_manager = Arc::new(ConfigManager::new(
        config,
        std::env::temp_dir().join(format!("pr_e2e_{}.yaml", uuid::Uuid::new_v4())),
    ));
    let mcp_server_manager = Arc::new(McpServerManager::new());
    mcp_server_manager.add_config(netget_server);
    let metrics_db = Arc::new(
        MetricsDatabase::new(
            std::env::temp_dir().join(format!("pr_e2e_{}.db", uuid::Uuid::new_v4())),
        )
        .unwrap(),
    );
    let metrics_collector = Arc::new(MetricsCollector::new(metrics_db));
    let provider_registry = Arc::new(ProviderRegistry::new());
    let rate_limiter = Arc::new(RateLimiterManager::new(None));
    let router = Arc::new(Router::new(
        config_manager.clone(),
        provider_registry.clone(),
        rate_limiter.clone(),
        metrics_collector.clone(),
        Arc::new(lr_router::FreeTierManager::new(None)),
    ));
    let token_store = Arc::new(TokenStore::new());
    let (token, _) = token_store.generate_token("pr-e2e".into()).unwrap();
    let (state, _handle, port, shutdown) = localrouter::server::start_server(
        localrouter::server::ServerConfig {
            host: "127.0.0.1".into(),
            port: 0,
            enable_cors: false,
        },
        router,
        mcp_server_manager,
        rate_limiter,
        provider_registry,
        config_manager,
        Arc::new(ClientManager::new(vec![pr_client])),
        token_store,
        metrics_collector,
        None,
    )
    .await
    .expect("server");

    let home = std::env::temp_dir().join(format!("pr_e2e_home_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&home).unwrap();
    let mut child = tokio::process::Command::new(&bridge_bin)
        .args(["--mcp-bridge", "--client-id", "pr-e2e", "--url"])
        .arg(format!("http://127.0.0.1:{port}/mcp"))
        .env("LOCALROUTER_CLIENT_SECRET", &token)
        .env("HOME", &home)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .expect("bridge");
    let mut bridge = BridgeProcess {
        stdin: child.stdin.take().unwrap(),
        lines: tokio::io::BufReader::new(child.stdout.take().unwrap()).lines(),
        child,
        seen: Vec::new(),
    };

    // Claude Code's legacy handshake through the bridge
    let init = bridge
        .request(
            1,
            "initialize",
            json!({"protocolVersion": "2025-11-25", "capabilities": {},
            "clientInfo": {"name": "claude-code", "version": "2"}}),
        )
        .await;
    let caps = &init["result"]["capabilities"];
    assert_eq!(
        caps["experimental"][CLAUDE_CHANNEL_CAPABILITY],
        json!({}),
        "{init}"
    );
    assert_eq!(
        caps["extensions"][EVENTS_EXTENSION_ID],
        json!({"listChanged": true})
    );
    bridge
        .write(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}))
        .await;

    let list = bridge.request(2, "events/list", json!({})).await;
    let names: Vec<String> = list["result"]["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["name"].as_str().unwrap().to_string())
        .collect();
    assert!(
        names.contains(&"netget__llm.request.pending".to_string()),
        "{names:?}"
    );
    assert!(
        names.contains(&"netget__channel.message".to_string()),
        "{names:?}"
    );
    let tools = bridge.request(3, "tools/list", json!({})).await;
    let tool_names: Vec<&str> = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert!(tool_names.contains(&"EventPoll"), "{tool_names:?}");
    let poll = bridge
        .request(
            4,
            "events/poll",
            json!({"name": "netget__access_log.recorded"}),
        )
        .await;
    let access_cursor = poll["result"]["cursor"].as_str().unwrap().to_string();

    // Make NetGet produce events: a TCP server whose requests queue for the agent
    let started = bridge
        .request(
            5,
            "tools/call",
            json!({"name": "netget__start_server", "arguments": {
            "protocol": "tcp", "port": 0, "instruction": "Answer every line with OK"}}),
        )
        .await;
    let text = started["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    assert!(text.contains("started"), "{started}");
    let listed = bridge
        .request(
            50,
            "tools/call",
            json!({"name": "netget__list_servers", "arguments": {}}),
        )
        .await;
    let listed = listed["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    let tcp_port = find_port(&format!("{text}\n{listed}"))
        .unwrap_or_else(|| panic!("no port in {text} / {listed}"));
    let mut tcp = tokio::net::TcpStream::connect(("127.0.0.1", tcp_port))
        .await
        .unwrap();
    use tokio::io::AsyncWriteExt;
    tcp.write_all(b"hello\n").await.unwrap();

    // NetGet's MCP Events (relayed from its events/stream) arrive as Claude
    // channel messages, attributed. (NetGet speaks 2026-07-28 to LocalRouter,
    // a revision on which it sends no channel messages of its own.)
    let relayed = bridge
        .read_until(Duration::from_secs(60), |v| {
            v["method"] == "notifications/claude/channel"
                && v["params"]["meta"]["event"] == "netget__llm.request.pending"
        })
        .await
        .expect("relayed MCP Event as a channel message");
    assert!(relayed["params"]["content"]
        .as_str()
        .unwrap()
        .contains("netget__llm.request.pending"));
    assert_eq!(relayed["params"]["meta"]["source"], "netget");

    // events/poll reads the same journal
    let read = bridge
        .request(
            6,
            "events/poll",
            json!({"name": "netget__access_log.recorded", "cursor": access_cursor}),
        )
        .await;
    assert!(read["result"]["events"].is_array(), "{read}");

    drop(tcp);
    let _ = bridge.child.kill().await;
    shutdown.cancel();
    if let Some(relay) = &state.push_relay {
        relay.notify_clients_changed();
    }
    let _ = std::fs::remove_dir_all(&home);
}

// ---------------------------------------------------------------------------
// Built-in test source (Try it out → MCP → Push)
// ---------------------------------------------------------------------------

const TEST_EVENT: &str = "localrouter__test.event";

fn event_names(list: &Value) -> Vec<String> {
    list["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["name"].as_str().unwrap().to_string())
        .collect()
}

fn monitor_ingress(h: &Harness, event_id: &str) -> Option<String> {
    h.monitor.lock().iter().find_map(|e| match &e.data {
        lr_monitor::MonitorEventData::McpPushEvent {
            event_id: id,
            ingress_mode,
            ..
        } if id == event_id => Some(ingress_mode.clone()),
        _ => None,
    })
}

#[tokio::test]
async fn test_source_reaches_only_sessions_that_opted_in() {
    // No upstream offers push at all
    let h = harness(vec![], fast_limits());
    h.clients
        .write()
        .insert("client-2".to_string(), client("client-2"));

    // A plain session: the test source does not make the gate pass
    let init = h.initialize("plain", json!({})).await.result.unwrap();
    assert!(init["capabilities"].get("events").is_none());
    assert!(init["capabilities"]
        .get("extensions")
        .and_then(|e| e.get(EVENTS_EXTENSION_ID))
        .is_none());
    assert!(!h
        .tool_names("plain")
        .await
        .iter()
        .any(|t| t.starts_with("Event")));
    assert!(!init["instructions"]
        .as_str()
        .unwrap_or_default()
        .contains("<events>"));
    // Nobody opted in: nothing to send to
    assert!(h.relay.send_test_event(None, None, None).is_err());

    // The Try it out session opts in with the header
    h.gateway.set_push_test_session("try", true);
    let init = h.initialize("try", json!({})).await.result.unwrap();
    assert_eq!(
        init["capabilities"]["extensions"][EVENTS_EXTENSION_ID],
        json!({"listChanged": true})
    );
    assert_eq!(init["capabilities"]["events"], json!({}));
    let list = h.send("try", "events/list", None).await.result.unwrap();
    assert_eq!(event_names(&list), vec![TEST_EVENT]);
    assert_eq!(
        list["events"][0]["delivery"],
        json!(["poll", "push", "webhook"])
    );
    assert!(h.tool_names("try").await.contains(&"EventPoll".to_string()));

    // The plain session of the same client still sees nothing
    let list = h.send("plain", "events/list", None).await.result.unwrap();
    assert!(event_names(&list).is_empty());
    // Neither does a client with no opted-in session, nor the upstream catalogue
    let init = h
        .send_as(
            "client-2",
            "other",
            "initialize",
            Some(json!({
                "protocolVersion": "2025-11-25",
                "capabilities": {},
                "clientInfo": {"name": "test", "version": "1"}
            })),
        )
        .await
        .result
        .unwrap();
    assert!(init["capabilities"].get("events").is_none());
    assert!(h.relay.catalogue(&h.clients.read()[CLIENT], &[]).is_empty());
    // It is no upstream: no remembered capabilities, no source entry
    assert!(h.relay.upstream_sources().is_empty());
    // The client's permission tree lists it, marked as the test source
    let types = h.relay.client_event_types(CLIENT);
    assert_eq!(types.len(), 1);
    assert!(types[0].test_source);
    assert_eq!(types[0].server_id, TEST_SOURCE_SERVER_ID);
    assert_eq!(types[0].event_name, TEST_EVENT_NAME);
    assert!(h.relay.client_event_types("client-2").is_empty());

    // A test event goes only to the client with an opted-in session
    let sent = h.relay.send_test_event(None, None, None).unwrap();
    assert_eq!(sent.client_ids, vec![CLIENT.to_string()]);
    assert_eq!(sent.name, TEST_EVENT);
    assert!(h
        .relay
        .send_test_event(Some("client-2"), None, None)
        .is_err());
}

#[tokio::test]
async fn test_event_flows_through_allow_ask_off_monitor_poll_and_stream() {
    let h = harness(vec![], fast_limits());
    h.gateway.set_push_test_session("try", true);
    h.initialize("try", json!({})).await;
    h.gateway
        .get_session("try")
        .unwrap()
        .write()
        .await
        .monitor_session_id = Some("req-try".into());
    let start = h.poll("try", TEST_EVENT, None).await["cursor"]
        .as_str()
        .unwrap()
        .to_string();

    // A live events/stream on the same session
    let sinks = h.gateway.push_stream_sinks();
    let (mut notifications, _guard) = sinks.register("try", &json!(9), 64);
    let gateway = h.gateway.clone();
    let client_config = h.clients.read()[CLIENT].clone();
    let stream = tokio::spawn(async move {
        gateway
            .handle_request_with_skills(
                CLIENT,
                Some("try"),
                vec![],
                vec![],
                client_config.mcp_permissions.clone(),
                lr_config::SkillsPermissions::default(),
                client_config.name.clone(),
                PermissionState::Off,
                PermissionState::Off,
                None,
                None,
                PermissionState::default(),
                PermissionState::default(),
                None,
                None,
                lr_config::ClientMode::default(),
                lr_config::ClientPushSettings::from_client(&client_config),
                JsonRpcRequest::new(
                    Some(json!(9)),
                    "events/stream".into(),
                    Some(json!({"name": TEST_EVENT})),
                ),
                None,
            )
            .await
    });
    assert_eq!(
        recv(&mut notifications).await.method,
        "notifications/events/active"
    );

    // Allow (the default): stream, poll and Monitor all see it
    let sent = h
        .relay
        .send_test_event(Some(CLIENT), Some("hello".into()), None)
        .unwrap();
    let event = recv(&mut notifications).await;
    assert_eq!(event.method, "notifications/events/event");
    let params = event.params.unwrap();
    assert_eq!(params["eventId"], json!(sent.event_id));
    assert_eq!(params["name"], TEST_EVENT);
    assert_eq!(params["data"]["message"], "hello");
    let batch = h
        .poll_until("try", TEST_EVENT, &start, |b| {
            !b["events"].as_array().unwrap().is_empty()
        })
        .await;
    assert_eq!(batch["events"][0]["eventId"], json!(sent.event_id));
    assert_eq!(h.monitor_decisions(TEST_EVENT), vec!["allowed"]);
    assert_eq!(monitor_ingress(&h, &sent.event_id).as_deref(), Some("test"));
    // Grouped with the one session that reads it
    assert_eq!(
        monitor_request_of(&h, &sent.event_id).as_deref(),
        Some("req-try")
    );
    let next = batch["cursor"].as_str().unwrap().to_string();

    // Ask: held, the real approval popup opens, Edit then allow delivers the edit
    let mut ask = client(CLIENT);
    ask.push_permissions.events.insert(
        PushPermissions::event_key(TEST_SOURCE_SERVER_ID, TEST_EVENT_NAME),
        PushPermission::Ask,
    );
    h.set_client(ask);
    let held = h
        .relay
        .send_test_event(None, None, Some(json!({"secret": "s3cr3t"})))
        .unwrap();
    wait_until("held", || !h.relay.pending_events(Some(CLIENT)).is_empty()).await;
    assert_eq!(
        h.poll("try", TEST_EVENT, Some(&next)).await["events"],
        json!([])
    );
    wait_until("popup", || {
        h.gateway
            .firewall_manager
            .list_pending()
            .iter()
            .any(|p| p.is_push_event_request)
    })
    .await;
    let popup = h
        .gateway
        .firewall_manager
        .list_pending()
        .into_iter()
        .find(|p| p.is_push_event_request)
        .unwrap();
    let details = popup.push_event_details.unwrap();
    assert_eq!(details.qualified_name, TEST_EVENT);
    assert_eq!(details.event_id, held.event_id);
    let hold = h.relay.pending_events(Some(CLIENT))[0].hold_id.clone();
    h.relay
        .decide(
            &hold,
            PushDecision {
                action: PushDecisionAction::Allow,
                edited_data: Some(json!({"secret": "[redacted]"})),
            },
        )
        .unwrap();
    let batch = h
        .poll_until("try", TEST_EVENT, &next, |b| {
            !b["events"].as_array().unwrap().is_empty()
        })
        .await;
    assert_eq!(batch["events"][0]["data"], json!({"secret": "[redacted]"}));
    let streamed = recv(&mut notifications).await;
    assert_eq!(
        streamed.params.unwrap()["data"],
        json!({"secret": "[redacted]"})
    );
    wait_until("asked_edited", || {
        h.monitor_decisions(TEST_EVENT) == vec!["allowed", "asked_edited"]
    })
    .await;
    let next = batch["cursor"].as_str().unwrap().to_string();

    // Off: recorded in Monitor, delivered nowhere, gone from the catalogue
    let mut block = client(CLIENT);
    block.push_permissions.events.insert(
        PushPermissions::event_key(TEST_SOURCE_SERVER_ID, TEST_EVENT_NAME),
        PushPermission::Off,
    );
    h.set_client(block);
    h.relay.send_test_event(None, None, None).unwrap();
    assert_eq!(
        h.monitor_decisions(TEST_EVENT),
        vec!["allowed", "asked_edited", "off"]
    );
    let response = h
        .send(
            "try",
            "events/poll",
            Some(json!({"name": TEST_EVENT, "cursor": next})),
        )
        .await;
    assert!(response.result.is_none(), "an Off type is not found");
    let list = h.send("try", "events/list", None).await.result.unwrap();
    assert!(event_names(&list).is_empty());

    h.send(
        "try",
        "notifications/cancelled",
        Some(json!({"requestId": 9})),
    )
    .await;
    let response = tokio::time::timeout(Duration::from_secs(5), stream)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(response.result.unwrap()["resultType"], "complete");
}

#[tokio::test]
async fn test_events_reach_opted_in_bridge_channels_and_the_poll_tool_only() {
    let (upstream, _) = http_upstream().await;
    let mut h = harness(vec![http_server(upstream.uri())], fast_limits());
    // A real Claude Code bridge session that did not opt in
    h.gateway.set_stdio_bridge_session("bridge", true);
    h.initialize("bridge", json!({})).await;
    // A bridge started with LOCALROUTER_PUSH_TEST=1
    h.gateway.set_stdio_bridge_session("bridge-test", true);
    h.gateway.set_push_test_session("bridge-test", true);
    let init = h.initialize("bridge-test", json!({})).await.result.unwrap();
    assert_eq!(
        init["capabilities"]["experimental"][CLAUDE_CHANNEL_CAPABILITY],
        json!({})
    );
    let instructions = init["instructions"].as_str().unwrap_or_default();
    assert!(instructions.contains(TEST_EVENT), "{instructions}");
    let first = h
        .call_tool("bridge-test", "EventPoll", json!({"events": [TEST_EVENT]}))
        .await;
    let cursor = first
        .lines()
        .find_map(|l| l.strip_prefix("next_cursor: "))
        .unwrap()
        .to_string();
    // The plain bridge's tools do not know the test source
    let refused = h
        .send(
            "bridge",
            "tools/call",
            Some(json!({"name": "EventPoll", "arguments": {"events": [TEST_EVENT]}})),
        )
        .await;
    let refused = serde_json::to_string(&refused).unwrap();
    assert!(refused.contains("Unknown event type"), "{refused}");

    let sent = h.relay.send_test_event(None, None, None).unwrap();
    let channel = h
        .next_notification(
            "notifications/claude/channel",
            &session_notification_key("bridge-test"),
            Duration::from_secs(5),
        )
        .await
        .expect("channel copy for the opted-in bridge");
    let params = channel.params.unwrap();
    assert_eq!(params["meta"]["event"], TEST_EVENT);
    assert_eq!(params["meta"]["event_id"], json!(sent.event_id));
    let (content, meta) = h
        .relay
        .preview_channel_message(&json!({
            "eventId": sent.event_id,
            "name": TEST_EVENT,
            "data": {"message": "Test event from LocalRouter"},
        }))
        .unwrap();
    assert!(content.starts_with(&format!("[localrouter] {TEST_EVENT} ({})", sent.event_id)));
    assert_eq!(meta, params["meta"]);
    assert!(h
        .next_notification(
            "notifications/claude/channel",
            &session_notification_key("bridge"),
            Duration::from_millis(500),
        )
        .await
        .is_none());

    let text = h
        .call_tool(
            "bridge-test",
            "EventPoll",
            json!({"events": [TEST_EVENT], "cursor": cursor}),
        )
        .await;
    assert!(text.contains(&sent.event_id), "{text}");
}

// ---------------------------------------------------------------------------
// A VocalCord-shaped upstream: the SEP-3415 draft profile over HTTP
// ---------------------------------------------------------------------------

const VC_ID: &str = "vocalcord-like";
const VC_EVENT: &str = "message.received";
const VC_MODERN: &str = "2026-07-28";

/// A scripted HTTP upstream that enforces what VocalCord's MCP transport
/// (`vocalcord-lambda-mcp/src/transport.rs`) enforces: `initialize`
/// negotiates 2025-03-26, `server/discover` lists 2026-07-28, and
/// `events/poll` / `events/stream` always use the draft profile, so they need
/// `MCP-Protocol-Version: 2026-07-28`, `Mcp-Method` and `Mcp-Name` (values
/// may use the `=?base64?…?=` sentinel) and a body `_meta` revision equal to
/// the header; `events/stream` needs `Accept: text/event-stream`. Cursors are
/// scoped to (event name, canonical arguments) and a null cursor starts at
/// the head. A stream sends `notifications/events/active`, delivers stored
/// events, sends heartbeats and closes after its lifetime with a final
/// `{"resultType": "complete"}`.
struct VocalcordLike {
    events: parking_lot::Mutex<Vec<Value>>,
    rejections: parking_lot::Mutex<Vec<String>>,
    /// Cursors the streams were opened with, in order.
    stream_cursors: parking_lot::Mutex<Vec<Option<String>>>,
    polls: AtomicUsize,
    stream_lifetime: Duration,
    /// Answer every stream the way a server that cannot serve it would.
    refuse_streams: bool,
    /// `delivery` of the event in `events/list` (the gateway refreshes the
    /// remembered snapshot from it).
    delivery: Value,
}

fn vc_cursor(name: &str, arguments: &Value, sequence: usize) -> String {
    format!("vc1|{name}|{arguments}|{sequence}")
}

impl VocalcordLike {
    fn new(stream_lifetime: Duration, refuse_streams: bool, delivery: &[&str]) -> Arc<Self> {
        Arc::new(Self {
            events: parking_lot::Mutex::new(Vec::new()),
            rejections: parking_lot::Mutex::new(Vec::new()),
            stream_cursors: parking_lot::Mutex::new(Vec::new()),
            polls: AtomicUsize::new(0),
            stream_lifetime,
            refuse_streams,
            delivery: json!(delivery),
        })
    }

    fn store(&self, id: &str, preview: &str) {
        self.events.lock().push(json!({
            "eventId": id, "name": VC_EVENT, "timestamp": "2026-10-31T12:34:07Z",
            "data": {"id": id, "channel": "telegram", "from": "owner", "preview": preview}
        }));
    }

    fn decoded(headers: &axum::http::HeaderMap, name: &str) -> Result<Option<String>, String> {
        use base64::Engine;
        let Some(value) = headers.get(name) else {
            return Ok(None);
        };
        let text = value
            .to_str()
            .map_err(|_| "Invalid MCP request metadata header")?;
        match text
            .strip_prefix("=?base64?")
            .and_then(|s| s.strip_suffix("?="))
        {
            Some(encoded) => base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .ok()
                .and_then(|b| String::from_utf8(b).ok())
                .map(Some)
                .ok_or_else(|| "Invalid Base64 MCP header".to_string()),
            None => Ok(Some(text.to_string())),
        }
    }

    /// VocalCord's `validate_modern` without the compatibility allowance,
    /// which never applies to `events/poll` and `events/stream`.
    fn validate(headers: &axum::http::HeaderMap, body: &Value) -> Result<(), String> {
        let method = body["method"].as_str().unwrap_or_default();
        let header_version = Self::decoded(headers, "mcp-protocol-version")?;
        let meta_version = body
            .pointer("/params/_meta/io.modelcontextprotocol~1protocolVersion")
            .and_then(Value::as_str);
        if let (Some(header), Some(meta)) = (header_version.as_deref(), meta_version) {
            if header != meta {
                return Err("MCP protocol version header does not match request metadata".into());
            }
        }
        if header_version.as_deref() != Some(VC_MODERN) {
            return Err("MCP-Protocol-Version is required for modern MCP requests".into());
        }
        if meta_version.is_some_and(|v| v != VC_MODERN) {
            return Err("Events and discovery require MCP protocol 2026-07-28".into());
        }
        if Self::decoded(headers, "mcp-method")?.as_deref() != Some(method) {
            return Err("Mcp-Method must match the request method".into());
        }
        let name = body.pointer("/params/name").and_then(Value::as_str);
        if name.is_none() || Self::decoded(headers, "mcp-name")?.as_deref() != name {
            return Err("Mcp-Name must match the named request parameter".into());
        }
        Ok(())
    }

    /// The sequence a cursor stands for, refusing one minted for another
    /// (name, arguments) scope.
    fn sequence(&self, params: &Value) -> Result<usize, String> {
        let name = params["name"].as_str().unwrap_or_default();
        if name != VC_EVENT {
            return Err("Unknown event name".into());
        }
        let arguments = params.get("arguments").cloned().unwrap_or(json!({}));
        let head = self.events.lock().len();
        match params.get("cursor") {
            None | Some(Value::Null) => Ok(head),
            Some(Value::String(cursor)) => {
                let (scope, sequence) = cursor.rsplit_once('|').ok_or("Invalid cursor")?;
                if scope != format!("vc1|{name}|{arguments}") {
                    return Err("cursor does not belong to this subscription".into());
                }
                let sequence: usize = sequence.parse().map_err(|_| "Invalid cursor")?;
                if sequence > head {
                    return Err("cursor is ahead of the event feed".into());
                }
                Ok(sequence)
            }
            Some(_) => Err("cursor must be a string or null".into()),
        }
    }

    fn page(&self, params: &Value, from: usize) -> (Vec<Value>, String) {
        let arguments = params.get("arguments").cloned().unwrap_or(json!({}));
        let events = self.events.lock();
        let page = events[from..]
            .iter()
            .enumerate()
            .map(|(i, e)| {
                let mut e = e.clone();
                e["cursor"] = json!(vc_cursor(VC_EVENT, &arguments, from + i + 1));
                e
            })
            .collect();
        (page, vc_cursor(VC_EVENT, &arguments, events.len()))
    }

    fn reject(
        &self,
        status: u16,
        id: &Value,
        code: i64,
        message: &str,
    ) -> axum::response::Response {
        use axum::response::IntoResponse;
        self.rejections.lock().push(message.to_string());
        (
            axum::http::StatusCode::from_u16(status).unwrap(),
            axum::Json(
                json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}}),
            ),
        )
            .into_response()
    }

    fn handle(
        self: Arc<Self>,
        method: axum::http::Method,
        headers: axum::http::HeaderMap,
        body: axum::body::Bytes,
    ) -> axum::response::Response {
        use axum::response::IntoResponse;
        if method != axum::http::Method::POST {
            return axum::http::StatusCode::METHOD_NOT_ALLOWED.into_response();
        }
        let Ok(body) = serde_json::from_slice::<Value>(&body) else {
            return self.reject(400, &Value::Null, -32700, "Invalid JSON");
        };
        let id = body.get("id").cloned().unwrap_or(Value::Null);
        let rpc = body["method"].as_str().unwrap_or_default().to_string();
        let ok = |result: Value| {
            axum::Json(json!({"jsonrpc": "2.0", "id": id, "result": result})).into_response()
        };
        if body.get("id").is_none() {
            return axum::http::StatusCode::ACCEPTED.into_response();
        }
        let catalogue = json!({"events": [{"name": VC_EVENT, "description": "A message arrived",
            "delivery": self.delivery, "inputSchema": {"type": "object", "properties": {}}}],
            "resultType": "complete"});
        match rpc.as_str() {
            "initialize" => {
                return ok(json!({"protocolVersion": "2025-03-26",
                "capabilities": {"tools": {}, "events": {}}, "serverInfo": {"name": "VocalCord", "version": "1"}}))
            }
            "server/discover" => {
                return ok(json!({"protocolVersions": [VC_MODERN, "2025-03-26"],
                "capabilities": {"tools": {}, "events": {}}, "serverInfo": {"name": "VocalCord", "version": "1"},
                "resultType": "complete"}))
            }
            "events/list" => return ok(catalogue),
            "tools/list" => return ok(json!({"tools": [], "resultType": "complete"})),
            "resources/list" => return ok(json!({"resources": []})),
            "prompts/list" => return ok(json!({"prompts": []})),
            "events/poll" | "events/stream" => {}
            // Like VocalCord (`subscriptions/listen` and the rest); not an
            // events refusal, so not recorded.
            _ => {
                return (
                    axum::http::StatusCode::NOT_FOUND,
                    axum::Json(json!({"jsonrpc": "2.0", "id": id,
                        "error": {"code": -32601, "message": "Method not found"}})),
                )
                    .into_response()
            }
        }
        if let Err(reason) = Self::validate(&headers, &body) {
            return self.reject(400, &id, -32020, &reason);
        }
        let params = body.get("params").cloned().unwrap_or(json!({}));
        let from = match self.sequence(&params) {
            Ok(from) => from,
            Err(reason) => return self.reject(200, &id, -32602, &reason),
        };
        if rpc == "events/poll" {
            self.polls.fetch_add(1, Ordering::SeqCst);
            let (events, cursor) = if params.get("cursor").is_some_and(|c| !c.is_null()) {
                self.page(&params, from)
            } else {
                (vec![], self.page(&params, from).1)
            };
            return ok(
                json!({"events": events, "cursor": cursor, "truncated": false,
                "hasMore": false, "nextPollMs": 50, "resultType": "complete"}),
            );
        }
        if !headers
            .get("accept")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.contains("text/event-stream"))
        {
            return self.reject(406, &id, -32020, "Accept must include text/event-stream");
        }
        if self.refuse_streams {
            return self.reject(
                400,
                &id,
                -32026,
                "Streaming requires the streaming endpoint",
            );
        }
        self.stream_cursors
            .lock()
            .push(params["cursor"].as_str().map(str::to_string));
        let (tx, rx) = tokio::sync::mpsc::channel::<axum::body::Bytes>(64);
        let up = self.clone();
        tokio::spawn(async move {
            let frame = |value: Value| axum::body::Bytes::from(format!("data: {value}\n\n"));
            let notify = |method: &str, mut params: Value| {
                params["_meta"] = json!({"io.modelcontextprotocol/subscriptionId": id});
                frame(json!({"jsonrpc": "2.0", "method": method, "params": params}))
            };
            let mut sent = from;
            let arguments = params.get("arguments").cloned().unwrap_or(json!({}));
            let cursor = vc_cursor(VC_EVENT, &arguments, from);
            if tx
                .send(notify(
                    "notifications/events/active",
                    json!({"cursor": cursor, "truncated": false}),
                ))
                .await
                .is_err()
            {
                return;
            }
            let deadline = tokio::time::Instant::now() + up.stream_lifetime;
            let mut beats = 0u32;
            while tokio::time::Instant::now() < deadline {
                tokio::time::sleep(Duration::from_millis(50)).await;
                let (page, cursor) = up.page(&params, sent);
                sent += page.len();
                for event in page {
                    if tx
                        .send(notify("notifications/events/event", event))
                        .await
                        .is_err()
                    {
                        return;
                    }
                }
                beats += 1;
                if beats.is_multiple_of(4)
                    && tx
                        .send(notify(
                            "notifications/events/heartbeat",
                            json!({"cursor": cursor}),
                        ))
                        .await
                        .is_err()
                {
                    return;
                }
            }
            let _ = tx
                .send(frame(
                    json!({"jsonrpc": "2.0", "id": id, "result": {"resultType": "complete",
                    "_meta": {"io.modelcontextprotocol/subscriptionId": id}}}),
                ))
                .await;
        });
        let body =
            axum::body::Body::from_stream(futures_util::stream::unfold(rx, |mut rx| async move {
                rx.recv()
                    .await
                    .map(|b| (Ok::<_, std::convert::Infallible>(b), rx))
            }));
        (
            [
                ("content-type", "text/event-stream"),
                ("cache-control", "no-store"),
            ],
            body,
        )
            .into_response()
    }
}

async fn vocalcord_like_upstream(upstream: Arc<VocalcordLike>) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/mcp", listener.local_addr().unwrap());
    let app = axum::Router::new().route(
        "/mcp",
        axum::routing::any(
            move |method: axum::http::Method,
                  headers: axum::http::HeaderMap,
                  body: axum::body::Bytes| {
                let upstream = upstream.clone();
                async move { upstream.handle(method, headers, body) }
            },
        ),
    );
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    url
}

fn vocalcord_server(url: String, modes: &[EventDeliveryMode]) -> McpServerConfig {
    let mut server = http_server(url);
    server.id = VC_ID.to_string();
    server.name = "VocalCord".to_string();
    server.capability_snapshot = Some(snapshot(&server, &[(VC_EVENT, modes)], false));
    server
}

/// Open a downstream `events/stream` for `name` on `session`, returning the
/// notification receiver (and the guard that keeps it registered).
fn open_downstream_stream(
    h: &Harness,
    session: &str,
    name: &str,
) -> (
    tokio::sync::mpsc::Receiver<JsonRpcNotification>,
    localrouter::mcp::gateway::push_relay::PushStreamSinkGuard,
    tokio::task::JoinHandle<()>,
) {
    let sinks = h.gateway.push_stream_sinks();
    let (notifications, guard) = sinks.register(session, &json!(41), 64);
    let gateway = h.gateway.clone();
    let client = h.clients.read()[CLIENT].clone();
    let servers = h.servers.clone();
    let session = session.to_string();
    let name = name.to_string();
    let task = tokio::spawn(async move {
        let _ = gateway
            .handle_request_with_skills(
                CLIENT,
                Some(&session),
                servers,
                vec![],
                client.mcp_permissions.clone(),
                lr_config::SkillsPermissions::default(),
                client.name.clone(),
                PermissionState::Off,
                PermissionState::Off,
                None,
                None,
                PermissionState::default(),
                PermissionState::default(),
                None,
                None,
                lr_config::ClientMode::default(),
                lr_config::ClientPushSettings::from_client(&client),
                JsonRpcRequest::new(
                    Some(json!(41)),
                    "events/stream".into(),
                    Some(json!({"name": name})),
                ),
                None,
            )
            .await;
    });
    (notifications, guard, task)
}

async fn next_event(rx: &mut tokio::sync::mpsc::Receiver<JsonRpcNotification>) -> Value {
    loop {
        let n = recv(rx).await;
        if n.method == "notifications/events/event" {
            return n.params.unwrap();
        }
    }
}

/// The real-world failure: VocalCord answers `initialize` with 2025-03-26 and
/// refuses `events/stream` / `events/poll` unless they are 2026-07-28 draft
/// requests. The relay must reach it the way the gateway does (a
/// `server/discover` probe, then stateless requests), survive the stream's
/// normal completion by reopening from its cursor, and deliver the event to
/// a downstream stream and poll.
#[tokio::test]
async fn vocalcord_draft_profile_stream_reaches_downstream_stream_and_poll() {
    let upstream = VocalcordLike::new(
        Duration::from_millis(700),
        false,
        &["push", "poll", "webhook"],
    );
    let url = vocalcord_like_upstream(upstream.clone()).await;
    let h = harness(
        vec![vocalcord_server(
            url,
            &[
                EventDeliveryMode::Push,
                EventDeliveryMode::Poll,
                EventDeliveryMode::Webhook,
            ],
        )],
        fast_limits(),
    );
    h.initialize("s1", json!({})).await;
    let name = "vocalcord__message.received";
    let (mut rx, _guard, _task) = open_downstream_stream(&h, "s1", name);
    let bootstrap = h.poll("s1", name, None).await;
    let cursor = bootstrap["cursor"].as_str().unwrap().to_string();

    // The upstream stream completes at its lifetime and is reopened from
    // the cursor it handed out, without a failure.
    wait_until("the upstream stream to be reopened", || {
        upstream.stream_cursors.lock().len() >= 2
    })
    .await;
    upstream.store(
        "01a191f3-fa35-71f6-bb08-417676aa5ba3",
        "hello from telegram",
    );

    let event = next_event(&mut rx).await;
    assert_eq!(event["name"], name);
    assert_eq!(
        event["eventId"],
        "vocalcord__01a191f3-fa35-71f6-bb08-417676aa5ba3"
    );
    assert_eq!(event["data"]["preview"], "hello from telegram");

    let batch = h
        .poll_until("s1", name, &cursor, |b| {
            !b["events"].as_array().unwrap().is_empty()
        })
        .await;
    assert_eq!(
        batch["events"][0]["eventId"],
        "vocalcord__01a191f3-fa35-71f6-bb08-417676aa5ba3"
    );

    assert!(
        upstream.rejections.lock().is_empty(),
        "upstream refused requests: {:?}",
        upstream.rejections.lock()
    );
    let cursors = upstream.stream_cursors.lock().clone();
    assert_eq!(cursors[0], None, "the first stream starts at the head");
    assert!(
        cursors[1..].iter().all(Option::is_some),
        "reopened streams resume from a cursor: {cursors:?}"
    );

    let subscription = h
        .relay
        .active_subscriptions()
        .into_iter()
        .find(|s| s.server_id == VC_ID && s.event == VC_EVENT)
        .expect("subscription");
    assert_eq!(subscription.status.ingress_mode.as_deref(), Some("push"));
    assert_eq!(
        subscription.status.reconnects, 0,
        "a completed stream is not a failure: {:?}",
        subscription.status.last_error
    );
    let status = h.relay.server_push_status(VC_ID);
    let ingress = status
        .event_types
        .iter()
        .find(|t| t.event == VC_EVENT)
        .unwrap();
    assert_eq!(ingress.state, "listening");
    assert_eq!(ingress.last_error, None);
    assert!(ingress.events_received >= 1);
}

/// Poll ingress against the same draft profile: `events/poll` carries the
/// modern headers and `_meta`, a null cursor bootstraps at the head, and the
/// scoped cursor is sent back.
#[tokio::test]
async fn vocalcord_draft_profile_poll_reaches_downstream_poll() {
    let upstream = VocalcordLike::new(Duration::from_millis(700), false, &["poll"]);
    let url = vocalcord_like_upstream(upstream.clone()).await;
    let h = harness(
        vec![vocalcord_server(url, &[EventDeliveryMode::Poll])],
        fast_limits(),
    );
    h.initialize("s1", json!({})).await;
    let name = "vocalcord__message.received";
    let cursor = h.poll("s1", name, None).await["cursor"]
        .as_str()
        .unwrap()
        .to_string();
    wait_until("upstream polls", || {
        upstream.polls.load(Ordering::SeqCst) >= 2
    })
    .await;
    upstream.store("m-poll", "polled");
    let polls = upstream.polls.load(Ordering::SeqCst);
    wait_until("an upstream poll after the event", || {
        upstream.polls.load(Ordering::SeqCst) > polls + 1
    })
    .await;
    assert!(
        upstream.rejections.lock().is_empty(),
        "upstream refused requests: {:?}",
        upstream.rejections.lock()
    );
    let batch = h
        .poll_until("s1", name, &cursor, |b| {
            !b["events"].as_array().unwrap().is_empty()
        })
        .await;
    assert_eq!(batch["events"][0]["eventId"], "vocalcord__m-poll");
    assert!(
        upstream.rejections.lock().is_empty(),
        "upstream refused requests: {:?}",
        upstream.rejections.lock()
    );
    let status = h.relay.server_push_status(VC_ID);
    let ingress = status
        .event_types
        .iter()
        .find(|t| t.event == VC_EVENT)
        .unwrap();
    assert_eq!(ingress.state, "listening");
    assert_eq!(ingress.ingress_mode.as_deref(), Some("poll"));
}

/// An upstream that refuses the stream must not read as listening: the
/// server's push status says reconnecting and carries the server's own
/// error text.
#[tokio::test]
async fn refused_upstream_stream_shows_in_server_push_status() {
    let upstream = VocalcordLike::new(Duration::from_millis(700), true, &["push"]);
    let url = vocalcord_like_upstream(upstream.clone()).await;
    let h = harness(
        vec![vocalcord_server(url, &[EventDeliveryMode::Push])],
        fast_limits(),
    );
    h.initialize("s1", json!({})).await;
    let (_rx, _guard, _task) = open_downstream_stream(&h, "s1", "vocalcord__message.received");
    wait_until("the refusal to reach the status", || {
        h.relay
            .server_push_status(VC_ID)
            .event_types
            .iter()
            .any(|t| t.last_error.is_some())
    })
    .await;
    let status = h.relay.server_push_status(VC_ID);
    assert!(!status.listening);
    let ingress = status
        .event_types
        .iter()
        .find(|t| t.event == VC_EVENT)
        .unwrap();
    assert_eq!(ingress.state, "reconnecting");
    let error = ingress.last_error.clone().unwrap();
    assert!(
        error.contains("-32026") && error.contains("Streaming requires the streaming endpoint"),
        "{error}"
    );
}
