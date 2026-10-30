//! Push Relay end to end through the real gateway: ingress from scripted
//! upstreams (HTTP poll over a relay-owned connection, stdio push and
//! Claude channel on the session's process), gating and fallback tools,
//! per-event Ask, Monitor records, Claude Channels egress and deduplication,
//! webhook egress and the `/events/watch` stream.

use chrono::Utc;
use futures_util::StreamExt;
use localrouter::config::{
    AppConfig, ConfigManager, McpServerConfig, McpTransportConfig, McpTransportType,
};
use localrouter::mcp::gateway::push_relay::journal::{IngressMode, Occurrence};
use localrouter::mcp::gateway::push_relay::relay::PushToolsState;
use localrouter::mcp::gateway::push_relay::{
    webhook, PushConfigChange, PushDecision, PushDecisionAction, PushRelay,
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
    EventDeliveryMode, PermissionState, PushFallbackTools, PushPermission, PushPermissions,
    PushRelayConfig, UpstreamCapabilitySnapshot, UpstreamPushSummary,
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

fn fast_config() -> PushRelayConfig {
    let mut config = PushRelayConfig::default();
    config.limits.upstream_poll_min_interval_ms = 50;
    config.limits.upstream_reconnect_backoff_min_ms = 50;
    config.limits.upstream_reconnect_backoff_max_ms = 200;
    config.limits.first_discovery_wait_ms = 2_000;
    config
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

fn harness(servers: Vec<McpServerConfig>, config: PushRelayConfig) -> Harness {
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
    let clients = Arc::new(parking_lot::RwLock::new(HashMap::new()));
    clients.write().insert(CLIENT.to_string(), client(CLIENT));
    let lookup = clients.clone();
    let relay = PushRelay::new(
        gateway.as_ref().clone(),
        config,
        Arc::new(move |id| lookup.read().get(id).cloned()),
    );
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
    let h = harness(vec![http_server(upstream.uri())], fast_config());

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

    // Everything blocked: nothing advertised, no tools, no instructions
    let mut blocked = client(CLIENT);
    blocked.push_permissions.global = Some(PushPermission::Block);
    h.set_client(blocked);
    let init = h.initialize("blocked", json!({})).await.result.unwrap();
    assert!(init["capabilities"].get("events").is_none());
    assert!(init["capabilities"]
        .get("extensions")
        .and_then(|e| e.get(EVENTS_EXTENSION_ID))
        .is_none());
    assert!(!h
        .tool_names("blocked")
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

#[tokio::test]
async fn fallback_tools_follow_the_show_rules() {
    let (upstream, _) = http_upstream().await;
    let h = harness(vec![http_server(upstream.uri())], fast_config());
    let state = |fallback, declares_events, bridge, launched| PushToolsState {
        client_id: CLIENT.into(),
        session_key: Some("s".into()),
        fallback,
        channels_launched: launched,
        is_stdio_bridge: bridge,
        declares_events,
        visible_servers: vec![HTTP_ID.into()],
    };
    let view = h
        .relay
        .tools_view(&state(PushFallbackTools::Auto, false, false, false));
    assert!(view.gate && view.list && view.poll && view.wait);
    // A client with native Events support does not need them
    let view = h
        .relay
        .tools_view(&state(PushFallbackTools::Auto, true, false, false));
    assert!(view.gate && !view.list && !view.poll && !view.wait);
    // Bridge without a confirmed channels launch: still all
    let view = h
        .relay
        .tools_view(&state(PushFallbackTools::Auto, false, true, false));
    assert!(view.wait);
    // Launched with channels: no EventWait, reading tools stay
    let view = h
        .relay
        .tools_view(&state(PushFallbackTools::Auto, false, true, true));
    assert!(view.list && view.poll && !view.wait);
    let view = h
        .relay
        .tools_view(&state(PushFallbackTools::Always, true, true, true));
    assert!(view.list && view.poll && view.wait);
    let view = h
        .relay
        .tools_view(&state(PushFallbackTools::Never, false, false, false));
    assert!(view.gate && !view.list && !view.poll && !view.wait);
    // No push source: nothing, whatever the mode
    let mut no_source = state(PushFallbackTools::Always, false, false, false);
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
    let h = harness(vec![http_server(upstream.uri())], fast_config());
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
    let h = harness(vec![http_server(upstream.uri())], fast_config());
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
    let h = harness(vec![http_server(upstream.uri())], fast_config());
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
    let mut h = harness(vec![stdio_server()], fast_config());
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
    let mut h = harness(vec![stdio_server()], fast_config());
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
}

#[tokio::test]
async fn channel_copies_stop_once_the_session_uses_events_for_the_name() {
    let (upstream, _) = http_upstream().await;
    let mut h = harness(vec![http_server(upstream.uri())], fast_config());
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
    let mut config = fast_config();
    config.limits.channel_max_content_bytes = 120;
    config.limits.channel_rate_per_minute = 2;
    let mut h = harness(vec![http_server(upstream.uri())], config);
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
    let h = harness(vec![http_server(upstream.uri())], fast_config());
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
async fn ask_is_fifo_per_type_and_denial_drops() {
    let (upstream, _) = http_upstream().await;
    let h = harness(vec![http_server(upstream.uri())], fast_config());
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
async fn ask_times_out_closed_and_respects_the_hold_cap() {
    let (upstream, _) = http_upstream().await;
    let mut config = fast_config();
    config.limits.ask_timeout_secs = 1;
    config.limits.ask_hold_cap = 2;
    let h = harness(vec![http_server(upstream.uri())], config);
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
    wait_until("timeouts", || {
        h.monitor_decisions("http-push__msg")
            .iter()
            .filter(|d| *d == "timeout")
            .count()
            == 2
    })
    .await;
    assert_eq!(
        h.poll("s1", "http-push__msg", Some(&start)).await["events"],
        json!([])
    );
    assert!(h.relay.pending_events(None).is_empty());
    // The popup was withdrawn
    wait_until("popup gone", || {
        !h.gateway
            .firewall_manager
            .list_pending()
            .iter()
            .any(|p| p.is_push_event_request)
    })
    .await;
}

#[tokio::test]
async fn permanent_decisions_persist_and_apply_to_later_events() {
    let (upstream, _) = http_upstream().await;
    let h = harness(vec![http_server(upstream.uri())], fast_config());
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
async fn blocked_and_oversized_events_are_not_delivered() {
    let (upstream, _) = http_upstream().await;
    let mut config = fast_config();
    config.limits.max_event_bytes = 64;
    let h = harness(vec![http_server(upstream.uri())], config);
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

    let mut blocked = client(CLIENT);
    blocked.push_permissions.events.insert(
        PushPermissions::event_key(HTTP_ID, "msg"),
        PushPermission::Block,
    );
    h.set_client(blocked);
    h.relay.admit(
        CLIENT,
        occurrence(HTTP_ID, "http-push", "msg", "b1", json!({})),
    );
    assert!(h
        .monitor_decisions("http-push__msg")
        .contains(&"blocked".to_string()));
    // A blocked type is no longer listed and cannot be polled
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
    let mut config = fast_config();
    config.limits.journal_max_events = 2;
    let h = harness(vec![http_server(upstream.uri())], config);
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
    let other_relay = harness(vec![http_server(upstream.uri())], fast_config());
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
    let mut config = fast_config();
    config.limits.max_streams_per_session = 1;
    let h = harness(vec![http_server(upstream.uri())], config);
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
    let h = harness(vec![http_server(upstream.uri())], fast_config());
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
    let mut config = fast_config();
    config.limits.webhook_retry_delays_secs = vec![0, 0];
    let h = harness(vec![http_server(upstream.uri())], config);
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

#[tokio::test]
async fn webhook_egress_can_be_switched_off() {
    let (upstream, _) = http_upstream().await;
    let mut config = fast_config();
    config.webhook_egress_enabled = false;
    let h = harness(vec![http_server(upstream.uri())], config);
    h.initialize("s1", json!({})).await;
    let response = h
        .send("s1", "events/subscribe", Some(json!({"name": "http-push__msg", "delivery": {"url": "https://example.com", "secret": "whsec_AAAA"}})))
        .await;
    assert_eq!(response.error.unwrap().code, -32026);
    let list = h.send("s1", "events/list", None).await.result.unwrap();
    assert_eq!(list["events"][0]["delivery"], json!(["poll", "push"]));
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
