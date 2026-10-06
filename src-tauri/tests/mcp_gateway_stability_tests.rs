//! MCP gateway stability and permission-inheritance tests.
//!
//! Each test drives the gateway against wiremock-backed MCP servers:
//! - a server that can't start must not make the gateway rebuild the session
//!   (and lose its tool mappings) on every request
//! - `tools/call` works even when the client never listed tools in this session
//! - tool/prompt permissions are applied to lists and enforced on use
//! - one failing server doesn't take down the whole `tools/list`
//! - Tool Responses Indexing and MCP Catalog Indexing toggle independently

use chrono::Utc;
use localrouter::config::{
    AppConfig, ConfigManager, McpServerConfig, McpTransportConfig, McpTransportType,
};
use localrouter::mcp::gateway::context_mode::ContextModeVirtualServer;
use localrouter::mcp::gateway::{GatewayConfig, McpGateway};
use localrouter::mcp::protocol::{JsonRpcRequest, JsonRpcResponse};
use localrouter::mcp::McpServerManager;
use localrouter::monitoring::metrics::MetricsCollector;
use localrouter::monitoring::storage::MetricsDatabase;
use localrouter::providers::registry::ProviderRegistry;
use localrouter::router::{RateLimiterManager, Router};
use lr_config::{ContextManagementOverrides, McpPermissions, PermissionState};
use serde_json::{json, Value};
use std::sync::Arc;
use wiremock::{
    matchers::method as http_method, Match, Mock, MockServer, Request, ResponseTemplate,
};

const ECHO_ID: &str = "echo-id";
const DISABLED_ID: &str = "disabled-id";
/// slugify("Echo Server")
const ECHO_SLUG: &str = "echo-server";

fn test_router() -> Arc<Router> {
    let config_manager = Arc::new(ConfigManager::new(
        AppConfig::default(),
        std::path::PathBuf::from("/tmp/test_gateway_stability_router.yaml"),
    ));
    let metrics_db = Arc::new(
        MetricsDatabase::new(std::env::temp_dir().join(format!(
            "test_gateway_stability_metrics_{}.db",
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
            .and_then(|body| {
                body.get("method")
                    .and_then(|m| m.as_str())
                    .map(|m| m == self.0)
            })
            .unwrap_or(false)
    }
}

/// Answer `method` with `result` (or a 500 when `result` is None).
async fn mock(server: &MockServer, method: &'static str, result: Option<Value>) {
    let response = match result {
        Some(result) => ResponseTemplate::new(200)
            .set_body_json(json!({"jsonrpc": "2.0", "id": 1, "result": result})),
        None => ResponseTemplate::new(500),
    };
    Mock::given(http_method("POST"))
        .and(JsonRpcMethod(method))
        .respond_with(response)
        .mount(server)
        .await;
}

fn http_server(id: &str, name: &str, url: String, enabled: bool) -> McpServerConfig {
    McpServerConfig {
        id: id.to_string(),
        name: name.to_string(),
        transport: McpTransportType::HttpSse,
        transport_config: McpTransportConfig::HttpSse {
            url,
            headers: std::collections::HashMap::new(),
        },
        auth_config: None,
        discovered_oauth: None,
        oauth_config: None,
        enabled,
        created_at: Utc::now(),
    }
}

type Broadcast =
    tokio::sync::broadcast::Sender<(String, localrouter::mcp::protocol::JsonRpcNotification)>;

/// Gateway with one working "Echo Server" (tools `echo` and `danger`,
/// prompts `shown` and `hidden`) plus one globally disabled server that always
/// fails to start.
async fn setup() -> (Arc<McpGateway>, MockServer) {
    setup_with_broadcast(None).await
}

async fn setup_with_broadcast(broadcast: Option<Arc<Broadcast>>) -> (Arc<McpGateway>, MockServer) {
    setup_full(broadcast, Vec::new()).await
}

/// `extra_servers` are configured but not part of any session's server list.
async fn setup_full(
    broadcast: Option<Arc<Broadcast>>,
    extra_servers: Vec<McpServerConfig>,
) -> (Arc<McpGateway>, MockServer) {
    let echo = MockServer::start().await;
    mock(
        &echo,
        "initialize",
        Some(json!({
            "protocolVersion": "2025-11-25",
            "capabilities": {"tools": {}, "prompts": {}},
            "serverInfo": {"name": "echo", "version": "1.0"}
        })),
    )
    .await;
    mock(
        &echo,
        "tools/list",
        Some(json!({"tools": [
            {"name": "echo", "description": "Echo the input back", "inputSchema": {"type": "object"}},
            {"name": "danger", "description": "Something risky", "inputSchema": {"type": "object"}}
        ]})),
    )
    .await;
    // tools/call streams a progress notification before its result
    let progress = json!({"jsonrpc": "2.0", "method": "notifications/progress",
        "params": {"progressToken": "tok-1", "progress": 1}});
    let result = json!({"jsonrpc": "2.0", "id": 1,
        "result": {"content": [{"type": "text", "text": "echoed"}]}});
    Mock::given(http_method("POST"))
        .and(JsonRpcMethod("tools/call"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            format!("data: {progress}\n\ndata: {result}\n\n"),
            "text/event-stream",
        ))
        .mount(&echo)
        .await;
    mock(
        &echo,
        "prompts/list",
        Some(json!({"prompts": [{"name": "shown"}, {"name": "hidden"}]})),
    )
    .await;
    mock(
        &echo,
        "prompts/get",
        Some(json!({"messages": [{"role": "user", "content": {"type": "text", "text": "hi"}}]})),
    )
    .await;
    mock(&echo, "resources/list", Some(json!({"resources": []}))).await;

    let manager = Arc::new(McpServerManager::new());
    manager.add_config(http_server(ECHO_ID, "Echo Server", echo.uri(), true));
    manager.add_config(http_server(
        DISABLED_ID,
        "Disabled Server",
        "http://127.0.0.1:9/mcp".to_string(),
        false,
    ));
    for server in extra_servers {
        manager.add_config(server);
    }

    let gateway = Arc::new(McpGateway::new_with_broadcast(
        manager,
        GatewayConfig::default(),
        test_router(),
        broadcast,
    ));
    (gateway, echo)
}

fn allow_all() -> McpPermissions {
    McpPermissions {
        global: PermissionState::Allow,
        ..Default::default()
    }
}

async fn send(
    gateway: &McpGateway,
    session: &str,
    allowed: &[&str],
    perms: &McpPermissions,
    overrides: Option<ContextManagementOverrides>,
    method: &str,
    params: Option<Value>,
) -> JsonRpcResponse {
    gateway
        .handle_request_with_skills(
            "client-1",
            Some(session),
            allowed.iter().map(|s| s.to_string()).collect(),
            vec![],
            perms.clone(),
            lr_config::SkillsPermissions::default(),
            "Test Client".to_string(),
            PermissionState::Off,
            PermissionState::Off,
            None,
            overrides,
            PermissionState::default(),
            PermissionState::default(),
            None,
            None,
            lr_config::ClientMode::default(),
            JsonRpcRequest::new(Some(json!(1)), method.to_string(), params),
            None,
        )
        .await
        .unwrap_or_else(|e| panic!("{method} failed: {e}"))
}

async fn initialize(
    gateway: &McpGateway,
    session: &str,
    allowed: &[&str],
    perms: &McpPermissions,
    overrides: Option<ContextManagementOverrides>,
) {
    let response = send(
        gateway,
        session,
        allowed,
        perms,
        overrides,
        "initialize",
        Some(json!({
            "protocolVersion": "2025-11-25",
            "capabilities": {},
            "clientInfo": {"name": "test", "version": "1.0"}
        })),
    )
    .await;
    assert!(response.error.is_none(), "initialize: {:?}", response.error);
}

fn tool_names(response: &JsonRpcResponse) -> Vec<String> {
    response.result.as_ref().unwrap()["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect()
}

fn echo_tool(name: &str) -> String {
    format!("{ECHO_SLUG}__{name}")
}

#[tokio::test]
async fn session_is_reused_when_a_server_fails_to_start() {
    let (gateway, _echo) = setup().await;
    let allowed = [ECHO_ID, DISABLED_ID];
    initialize(&gateway, "s1", &allowed, &allow_all(), None).await;
    let before = gateway.get_session("s1").unwrap();

    let list = send(
        &gateway,
        "s1",
        &allowed,
        &allow_all(),
        None,
        "tools/list",
        None,
    )
    .await;
    assert!(tool_names(&list).contains(&echo_tool("echo")));

    let after = gateway.get_session("s1").unwrap();
    assert!(
        Arc::ptr_eq(&before, &after),
        "a server that failed to start must not force a session rebuild"
    );

    let call = send(
        &gateway,
        "s1",
        &allowed,
        &allow_all(),
        None,
        "tools/call",
        Some(json!({"name": echo_tool("echo"), "arguments": {}})),
    )
    .await;
    assert!(call.error.is_none(), "tools/call: {:?}", call.error);
    assert!(Arc::ptr_eq(&before, &gateway.get_session("s1").unwrap()));
}

#[tokio::test]
async fn tools_call_without_prior_tools_list_resolves_the_tool() {
    let (gateway, _echo) = setup().await;
    initialize(&gateway, "s2", &[ECHO_ID], &allow_all(), None).await;

    // A client reconnecting with a tool list cached from an earlier session
    let call = send(
        &gateway,
        "s2",
        &[ECHO_ID],
        &allow_all(),
        None,
        "tools/call",
        Some(json!({"name": echo_tool("echo"), "arguments": {}})),
    )
    .await;
    assert!(call.error.is_none(), "tools/call: {:?}", call.error);
    assert_eq!(
        call.result.unwrap()["content"][0]["text"].as_str(),
        Some("echoed")
    );

    let missing = send(
        &gateway,
        "s2",
        &[ECHO_ID],
        &allow_all(),
        None,
        "tools/call",
        Some(json!({"name": echo_tool("nope"), "arguments": {}})),
    )
    .await;
    assert!(missing.error.unwrap().message.contains("Tool not found"));
}

#[tokio::test]
async fn tool_level_off_hides_the_tool_and_blocks_calls() {
    let (gateway, _echo) = setup().await;
    let mut perms = allow_all();
    perms
        .tools
        .insert(format!("{ECHO_ID}__danger"), PermissionState::Off);
    initialize(&gateway, "s3", &[ECHO_ID], &perms, None).await;

    let list = send(&gateway, "s3", &[ECHO_ID], &perms, None, "tools/list", None).await;
    let names = tool_names(&list);
    assert!(names.contains(&echo_tool("echo")));
    assert!(!names.contains(&echo_tool("danger")));

    let call = send(
        &gateway,
        "s3",
        &[ECHO_ID],
        &perms,
        None,
        "tools/call",
        Some(json!({"name": echo_tool("danger"), "arguments": {}})),
    )
    .await;
    assert!(
        call.error.is_some(),
        "a tool set to Off must not be callable"
    );
}

#[tokio::test]
async fn prompt_permissions_filter_list_and_block_get() {
    let (gateway, _echo) = setup().await;
    let mut perms = allow_all();
    perms
        .prompts
        .insert(format!("{ECHO_ID}__hidden"), PermissionState::Off);
    initialize(&gateway, "s4", &[ECHO_ID], &perms, None).await;

    let list = send(
        &gateway,
        "s4",
        &[ECHO_ID],
        &perms,
        None,
        "prompts/list",
        None,
    )
    .await;
    let names: Vec<&str> = list.result.as_ref().unwrap()["prompts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, vec![echo_tool("shown").as_str()]);

    let hidden = send(
        &gateway,
        "s4",
        &[ECHO_ID],
        &perms,
        None,
        "prompts/get",
        Some(json!({"name": echo_tool("hidden")})),
    )
    .await;
    assert!(hidden.error.is_some());

    let shown = send(
        &gateway,
        "s4",
        &[ECHO_ID],
        &perms,
        None,
        "prompts/get",
        Some(json!({"name": echo_tool("shown")})),
    )
    .await;
    assert!(shown.error.is_none(), "prompts/get: {:?}", shown.error);
}

#[tokio::test]
async fn tools_list_survives_every_server_failing() {
    let broken = MockServer::start().await;
    mock(
        &broken,
        "initialize",
        Some(json!({
            "protocolVersion": "2025-11-25",
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "broken", "version": "1.0"}
        })),
    )
    .await;
    mock(&broken, "tools/list", None).await;

    let manager = Arc::new(McpServerManager::new());
    manager.add_config(http_server("broken-id", "Broken", broken.uri(), true));
    let gateway = McpGateway::new(manager, GatewayConfig::default(), test_router());

    initialize(&gateway, "s5", &["broken-id"], &allow_all(), None).await;
    let list = send(
        &gateway,
        "s5",
        &["broken-id"],
        &allow_all(),
        None,
        "tools/list",
        None,
    )
    .await;
    assert!(list.error.is_none(), "tools/list: {:?}", list.error);
    let result = list.result.unwrap();
    assert_eq!(result["tools"], json!([]));
    assert_eq!(result["_meta"]["partial_failure"], json!(true));
}

/// Register context management with a 1-byte catalog threshold so catalog
/// compression defers every server whenever it is on.
fn register_context_mode(gateway: &McpGateway) {
    gateway.register_virtual_server(Arc::new(ContextModeVirtualServer::new(
        lr_config::ContextManagementConfig {
            catalog_threshold_bytes: 1,
            ..Default::default()
        },
    )));
}

fn overrides(responses: bool, catalog: bool) -> Option<ContextManagementOverrides> {
    Some(ContextManagementOverrides {
        context_management_enabled: Some(responses),
        catalog_compression_enabled: Some(catalog),
    })
}

#[tokio::test]
async fn response_indexing_alone_keeps_all_tools_listed() {
    let (gateway, _echo) = setup().await;
    register_context_mode(&gateway);
    let cm = overrides(true, false);
    initialize(&gateway, "s6", &[ECHO_ID], &allow_all(), cm.clone()).await;

    let names = tool_names(
        &send(
            &gateway,
            "s6",
            &[ECHO_ID],
            &allow_all(),
            cm,
            "tools/list",
            None,
        )
        .await,
    );
    assert!(names.contains(&echo_tool("echo")), "{names:?}");
    assert!(names.contains(&echo_tool("danger")), "{names:?}");
    assert!(names.contains(&"IndexSearch".to_string()), "{names:?}");
}

#[tokio::test]
async fn catalog_indexing_alone_defers_tools_behind_search() {
    let (gateway, _echo) = setup().await;
    register_context_mode(&gateway);
    let cm = overrides(false, true);
    initialize(&gateway, "s7", &[ECHO_ID], &allow_all(), cm.clone()).await;

    let names = tool_names(
        &send(
            &gateway,
            "s7",
            &[ECHO_ID],
            &allow_all(),
            cm,
            "tools/list",
            None,
        )
        .await,
    );
    assert!(!names.contains(&echo_tool("echo")), "{names:?}");
    assert!(names.contains(&"IndexSearch".to_string()), "{names:?}");

    // Deferred tools stay callable
    let call = send(
        &gateway,
        "s7",
        &[ECHO_ID],
        &allow_all(),
        overrides(false, true),
        "tools/call",
        Some(json!({"name": echo_tool("echo"), "arguments": {}})),
    )
    .await;
    assert!(call.error.is_none(), "tools/call: {:?}", call.error);
}

#[tokio::test]
async fn both_indexing_features_off_exposes_no_search_tools() {
    let (gateway, _echo) = setup().await;
    register_context_mode(&gateway);
    let cm = overrides(false, false);
    initialize(&gateway, "s8", &[ECHO_ID], &allow_all(), cm.clone()).await;

    let mut names = tool_names(
        &send(
            &gateway,
            "s8",
            &[ECHO_ID],
            &allow_all(),
            cm,
            "tools/list",
            None,
        )
        .await,
    );
    names.sort();
    assert_eq!(names, vec![echo_tool("danger"), echo_tool("echo")]);
}

// ── Tool Responses Indexing for virtual servers ─────────────────────

mod big_output {
    use async_trait::async_trait;
    use localrouter::mcp::gateway::virtual_server::{
        VirtualFirewallResult, VirtualInstructions, VirtualMcpServer, VirtualSessionState,
        VirtualToolCallResult,
    };
    use localrouter::mcp::gateway::FirewallDecisionResult;
    use localrouter::mcp::protocol::McpTool;
    use serde_json::{json, Value};
    use std::any::Any;

    pub const ID: &str = "_big";
    pub const TOOL: &str = "BigDump";

    #[derive(Clone)]
    struct State;

    impl VirtualSessionState for State {
        fn as_any(&self) -> &dyn Any {
            self
        }
        fn as_any_mut(&mut self) -> &mut dyn Any {
            self
        }
        fn clone_box(&self) -> Box<dyn VirtualSessionState> {
            Box::new(self.clone())
        }
    }

    /// Virtual server whose single tool returns a 5 KB text result.
    pub struct BigOutput;

    #[async_trait]
    impl VirtualMcpServer for BigOutput {
        fn id(&self) -> &str {
            ID
        }
        fn display_name(&self) -> &str {
            "Big Output"
        }
        fn owns_tool(&self, tool_name: &str) -> bool {
            tool_name == TOOL
        }
        fn is_enabled(&self, _client: &lr_config::Client) -> bool {
            true
        }
        fn list_tools(&self, _state: &dyn VirtualSessionState) -> Vec<McpTool> {
            vec![McpTool {
                name: TOOL.to_string(),
                description: None,
                input_schema: json!({"type": "object"}),
            }]
        }
        fn check_permissions(
            &self,
            _state: &dyn VirtualSessionState,
            _tool_name: &str,
            _arguments: Option<&Value>,
            _session_approved: bool,
            _session_denied: bool,
        ) -> VirtualFirewallResult {
            VirtualFirewallResult::Handled(FirewallDecisionResult::Proceed)
        }
        async fn handle_tool_call(
            &self,
            _state: Box<dyn VirtualSessionState>,
            _tool_name: &str,
            _arguments: Value,
            _client_id: &str,
            _client_name: &str,
        ) -> VirtualToolCallResult {
            VirtualToolCallResult::Success(
                json!({"content": [{"type": "text", "text": "line of output\n".repeat(400)}]}),
            )
        }
        fn build_instructions(
            &self,
            _state: &dyn VirtualSessionState,
        ) -> Option<VirtualInstructions> {
            None
        }
        fn create_session_state(
            &self,
            _client: &lr_config::Client,
        ) -> Box<dyn VirtualSessionState> {
            Box::new(State)
        }
        fn update_session_state(
            &self,
            _state: &mut dyn VirtualSessionState,
            _client: &lr_config::Client,
        ) {
        }
        fn all_tool_names(&self) -> Vec<String> {
            vec![TOOL.to_string()]
        }
    }
}

async fn call_big_dump(virtual_indexing: lr_config::GatewayIndexingPermissions) -> String {
    let (gateway, _echo) = setup().await;
    gateway.register_virtual_server(Arc::new(ContextModeVirtualServer::new(
        lr_config::ContextManagementConfig {
            virtual_indexing,
            ..Default::default()
        },
    )));
    gateway.register_virtual_server(Arc::new(big_output::BigOutput));
    let cm = overrides(true, false);
    initialize(&gateway, "s9", &[ECHO_ID], &allow_all(), cm.clone()).await;
    let call = send(
        &gateway,
        "s9",
        &[ECHO_ID],
        &allow_all(),
        cm,
        "tools/call",
        Some(json!({"name": big_output::TOOL, "arguments": {}})),
    )
    .await;
    assert!(call.error.is_none(), "tools/call: {:?}", call.error);
    call.result.unwrap()["content"][0]["text"]
        .as_str()
        .unwrap()
        .to_string()
}

#[tokio::test]
async fn large_virtual_tool_responses_are_indexed_when_enabled() {
    let text = call_big_dump(lr_config::GatewayIndexingPermissions::default()).await;
    assert!(text.starts_with("[Response compressed"), "{text}");
}

#[tokio::test]
async fn virtual_indexing_disabled_for_a_server_keeps_full_responses() {
    let mut perms = lr_config::GatewayIndexingPermissions::default();
    perms.servers.insert(
        big_output::ID.to_string(),
        lr_config::IndexingState::Disable,
    );
    let text = call_big_dump(perms).await;
    assert_eq!(text, "line of output\n".repeat(400));
}

// ── Notification scoping & settings-change notifications ────────────

use localrouter::mcp::gateway::types::notification_target;

fn new_broadcast() -> Arc<Broadcast> {
    Arc::new(tokio::sync::broadcast::channel(64).0)
}

/// Drain everything currently queued on a receiver.
async fn drain(
    rx: &mut tokio::sync::broadcast::Receiver<(
        String,
        localrouter::mcp::protocol::JsonRpcNotification,
    )>,
) -> Vec<(String, localrouter::mcp::protocol::JsonRpcNotification)> {
    // Notification forwarding runs on spawned tasks
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let mut out = Vec::new();
    while let Ok(item) = rx.try_recv() {
        out.push(item);
    }
    out
}

#[tokio::test]
async fn backend_notifications_reach_only_their_session_unchanged() {
    let broadcast = new_broadcast();
    let mut rx = broadcast.subscribe();
    let (gateway, _echo) = setup_with_broadcast(Some(broadcast)).await;
    initialize(&gateway, "s10", &[ECHO_ID], &allow_all(), None).await;
    send(
        &gateway,
        "s10",
        &[ECHO_ID],
        &allow_all(),
        None,
        "tools/call",
        Some(
            json!({"name": echo_tool("echo"), "arguments": {"_meta": {"progressToken": "tok-1"}}}),
        ),
    )
    .await;

    let progress: Vec<_> = drain(&mut rx)
        .await
        .into_iter()
        .filter(|(_, n)| n.method == "notifications/progress")
        .collect();
    assert_eq!(progress.len(), 1);
    let (key, notification) = &progress[0];
    let allowed = vec![ECHO_ID.to_string()];
    assert_eq!(notification_target(key, "s10", &allowed), Some(ECHO_ID));
    // Another client allowed the same server must not receive it
    assert_eq!(notification_target(key, "other-session", &allowed), None);
    // The client's own token comes back as-is
    assert_eq!(
        notification.params.as_ref().unwrap()["progressToken"],
        json!("tok-1")
    );
}

#[tokio::test]
async fn settings_change_between_requests_notifies_that_session() {
    let broadcast = new_broadcast();
    let mut rx = broadcast.subscribe();
    let (gateway, _echo) = setup_with_broadcast(Some(broadcast)).await;
    initialize(&gateway, "s11", &[ECHO_ID], &allow_all(), None).await;
    send(
        &gateway,
        "s11",
        &[ECHO_ID],
        &allow_all(),
        None,
        "tools/list",
        None,
    )
    .await;
    drain(&mut rx).await;

    // Same settings: no notification
    send(
        &gateway,
        "s11",
        &[ECHO_ID],
        &allow_all(),
        None,
        "tools/list",
        None,
    )
    .await;
    assert!(drain(&mut rx).await.is_empty());

    // Turning catalog indexing on changes the tool list
    send(
        &gateway,
        "s11",
        &[ECHO_ID],
        &allow_all(),
        overrides(false, true),
        "tools/list",
        None,
    )
    .await;
    let events = drain(&mut rx).await;
    let methods: Vec<&str> = events
        .iter()
        .filter(|(key, _)| notification_target(key, "s11", &[]).is_some())
        .map(|(_, n)| n.method.as_str())
        .collect();
    assert_eq!(methods, vec!["notifications/tools/list_changed"]);
    assert!(events
        .iter()
        .all(|(key, _)| notification_target(key, "s12", &[]).is_none()));
}

#[tokio::test]
async fn config_change_hook_waits_for_busy_sessions() {
    let (gateway, _echo) = setup().await;
    initialize(&gateway, "s13", &[ECHO_ID], &allow_all(), None).await;
    send(
        &gateway,
        "s13",
        &[ECHO_ID],
        &allow_all(),
        None,
        "tools/list",
        None,
    )
    .await;

    let mut client = lr_config::Client::new_with_strategy("Test Client".to_string(), String::new());
    client.id = "client-1".to_string();
    client.mcp_permissions = allow_all();
    client
        .mcp_permissions
        .tools
        .insert(format!("{ECHO_ID}__danger"), PermissionState::Off);

    // Hold the session lock briefly, as an in-flight request would
    let session = gateway.get_session("s13").unwrap();
    let guard = session.clone().read_owned().await;
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        drop(guard);
    });

    let notified = std::sync::Mutex::new(Vec::new());
    gateway
        .check_and_notify_permission_changes(&[client], &[ECHO_ID.to_string()], |id, t, r, p| {
            notified.lock().unwrap().push((id.to_string(), t, r, p));
        })
        .await;
    assert_eq!(
        notified.into_inner().unwrap(),
        vec![("client-1".to_string(), true, false, false)]
    );

    // The new permission applies to the next list
    let names = tool_names(
        &send(
            &gateway,
            "s13",
            &[ECHO_ID],
            &{
                let mut p = allow_all();
                p.tools
                    .insert(format!("{ECHO_ID}__danger"), PermissionState::Off);
                p
            },
            None,
            "tools/list",
            None,
        )
        .await,
    );
    assert!(!names.contains(&echo_tool("danger")));
}

// ── Servers granted mid-session (marketplace install) ───────────────

mod installer {
    use async_trait::async_trait;
    use localrouter::mcp::gateway::virtual_server::{
        VirtualFirewallResult, VirtualInstructions, VirtualMcpServer, VirtualSessionState,
        VirtualToolCallResult,
    };
    use localrouter::mcp::gateway::FirewallDecisionResult;
    use localrouter::mcp::protocol::McpTool;
    use serde_json::{json, Value};
    use std::any::Any;

    pub const TOOL: &str = "InstallSecond";

    #[derive(Clone)]
    struct State;

    impl VirtualSessionState for State {
        fn as_any(&self) -> &dyn Any {
            self
        }
        fn as_any_mut(&mut self) -> &mut dyn Any {
            self
        }
        fn clone_box(&self) -> Box<dyn VirtualSessionState> {
            Box::new(self.clone())
        }
    }

    /// Mimics a marketplace install granting `server_id` to the session.
    pub struct Installer {
        pub server_id: String,
    }

    #[async_trait]
    impl VirtualMcpServer for Installer {
        fn id(&self) -> &str {
            "_installer"
        }
        fn display_name(&self) -> &str {
            "Installer"
        }
        fn owns_tool(&self, tool_name: &str) -> bool {
            tool_name == TOOL
        }
        fn is_enabled(&self, _client: &lr_config::Client) -> bool {
            true
        }
        fn list_tools(&self, _state: &dyn VirtualSessionState) -> Vec<McpTool> {
            vec![McpTool {
                name: TOOL.to_string(),
                description: None,
                input_schema: json!({"type": "object"}),
            }]
        }
        fn check_permissions(
            &self,
            _state: &dyn VirtualSessionState,
            _tool_name: &str,
            _arguments: Option<&Value>,
            _session_approved: bool,
            _session_denied: bool,
        ) -> VirtualFirewallResult {
            VirtualFirewallResult::Handled(FirewallDecisionResult::Proceed)
        }
        async fn handle_tool_call(
            &self,
            _state: Box<dyn VirtualSessionState>,
            _tool_name: &str,
            _arguments: Value,
            _client_id: &str,
            _client_name: &str,
        ) -> VirtualToolCallResult {
            VirtualToolCallResult::SuccessWithSideEffects {
                response: json!({"content": [{"type": "text", "text": "installed"}]}),
                invalidate_cache: true,
                send_list_changed: true,
                state_update: None,
                add_allowed_servers: Some(vec![self.server_id.clone()]),
            }
        }
        fn build_instructions(
            &self,
            _state: &dyn VirtualSessionState,
        ) -> Option<VirtualInstructions> {
            None
        }
        fn create_session_state(
            &self,
            _client: &lr_config::Client,
        ) -> Box<dyn VirtualSessionState> {
            Box::new(State)
        }
        fn update_session_state(
            &self,
            _state: &mut dyn VirtualSessionState,
            _client: &lr_config::Client,
        ) {
        }
        fn all_tool_names(&self) -> Vec<String> {
            vec![TOOL.to_string()]
        }
    }
}

#[tokio::test]
async fn servers_granted_mid_session_become_routable_without_rebuild() {
    let second = MockServer::start().await;
    mock(
        &second,
        "initialize",
        Some(json!({
            "protocolVersion": "2025-11-25",
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "second", "version": "1.0"}
        })),
    )
    .await;
    mock(
        &second,
        "tools/list",
        Some(json!({"tools": [{"name": "fresh", "inputSchema": {"type": "object"}}]})),
    )
    .await;
    mock(
        &second,
        "tools/call",
        Some(json!({"content": [{"type": "text", "text": "fresh result"}]})),
    )
    .await;

    let (gateway, _echo) = setup_full(
        None,
        vec![http_server(
            "second-id",
            "Second Server",
            second.uri(),
            true,
        )],
    )
    .await;
    gateway.register_virtual_server(Arc::new(installer::Installer {
        server_id: "second-id".to_string(),
    }));

    initialize(&gateway, "s14", &[ECHO_ID], &allow_all(), None).await;
    let before = gateway.get_session("s14").unwrap();
    let install = send(
        &gateway,
        "s14",
        &[ECHO_ID],
        &allow_all(),
        None,
        "tools/call",
        Some(json!({"name": installer::TOOL, "arguments": {}})),
    )
    .await;
    assert!(install.error.is_none(), "install: {:?}", install.error);

    let names = tool_names(
        &send(
            &gateway,
            "s14",
            &[ECHO_ID],
            &allow_all(),
            None,
            "tools/list",
            None,
        )
        .await,
    );
    assert!(
        names.contains(&"second-server__fresh".to_string()),
        "{names:?}"
    );

    let call = send(
        &gateway,
        "s14",
        &[ECHO_ID],
        &allow_all(),
        None,
        "tools/call",
        Some(json!({"name": "second-server__fresh", "arguments": {}})),
    )
    .await;
    assert!(call.error.is_none(), "tools/call: {:?}", call.error);
    assert!(Arc::ptr_eq(&before, &gateway.get_session("s14").unwrap()));
}
