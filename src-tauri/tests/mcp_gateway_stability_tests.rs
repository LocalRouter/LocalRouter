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

async fn send_result(
    gateway: &McpGateway,
    session: &str,
    allowed: &[&str],
    perms: &McpPermissions,
    overrides: Option<ContextManagementOverrides>,
    method: &str,
    params: Option<Value>,
) -> lr_types::AppResult<JsonRpcResponse> {
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
    send_result(gateway, session, allowed, perms, overrides, method, params)
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
    assert_eq!(
        list.result.as_ref().unwrap()["_meta"]["partial_failure"],
        true
    );
    assert!(list.result.as_ref().unwrap()["_meta"]["failures"]
        .as_array()
        .unwrap()
        .iter()
        .any(|failure| failure["server_id"] == DISABLED_ID));
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
    // An unrelated successful listing must not erase cached tool failures.
    send(
        &gateway,
        "s5",
        &["broken-id"],
        &allow_all(),
        None,
        "prompts/list",
        None,
    )
    .await;
    let cached = send(
        &gateway,
        "s5",
        &["broken-id"],
        &allow_all(),
        None,
        "tools/list",
        None,
    )
    .await;
    assert_eq!(cached.result.unwrap()["_meta"], result["_meta"]);
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
            VirtualToolCallResult::Success(json!({
                "content": [{"type": "text", "text": "line of output\n".repeat(2000)}],
                "structuredContent": {"lines": vec!["line of output"; 2000]},
            }))
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

async fn call_big_dump(virtual_indexing: lr_config::GatewayIndexingPermissions) -> Value {
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
    call.result.unwrap()
}

fn first_text(result: &Value) -> &str {
    result["content"][0]["text"].as_str().unwrap()
}

#[tokio::test]
async fn large_virtual_tool_responses_are_indexed_when_enabled() {
    let result = call_big_dump(lr_config::GatewayIndexingPermissions::default()).await;
    let text = first_text(&result);
    assert!(text.starts_with("[Response compressed"), "{text}");
    // The placeholder names the exact read call for the label it reports
    let label = format!("{}:", big_output::TOOL);
    assert!(
        text.contains(&format!("IndexRead(label=\"{label}")),
        "{text}"
    );
    assert!(text.contains("limit=2000)"), "{text}");
    // structuredContent repeats the payload; it must not slip past
    assert!(result.get("structuredContent").is_none(), "{result}");
}

#[tokio::test]
async fn virtual_indexing_disabled_for_a_server_keeps_full_responses() {
    let mut perms = lr_config::GatewayIndexingPermissions::default();
    perms.servers.insert(
        big_output::ID.to_string(),
        lr_config::IndexingState::Disable,
    );
    let result = call_big_dump(perms).await;
    assert_eq!(first_text(&result), "line of output\n".repeat(2000));
    assert!(result.get("structuredContent").is_some());
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

// ── Skill script locations per client mode ──────────────────────────

async fn skill_read_as(gateway: &McpGateway, session: &str, mode: lr_config::ClientMode) -> String {
    let skills = lr_config::SkillsPermissions {
        global: PermissionState::Allow,
        ..Default::default()
    };
    let response = gateway
        .handle_request_with_skills(
            "client-1",
            Some(session),
            vec![],
            vec![],
            McpPermissions::default(),
            skills,
            "Test Client".to_string(),
            PermissionState::Off,
            PermissionState::Off,
            None,
            overrides(false, false),
            PermissionState::default(),
            PermissionState::default(),
            None,
            None,
            mode,
            JsonRpcRequest::new(
                Some(json!(1)),
                "tools/call".to_string(),
                Some(json!({"name": "SkillRead", "arguments": {"name": "runner"}})),
            ),
            None,
        )
        .await
        .unwrap();
    assert!(response.error.is_none(), "SkillRead: {:?}", response.error);
    response.result.unwrap()["content"][0]["text"]
        .as_str()
        .unwrap()
        .to_string()
}

#[tokio::test]
async fn skill_read_shows_disk_paths_to_direct_clients_only() {
    let tmp = tempfile::tempdir().unwrap();
    let skill_dir = tmp.path().join("runner");
    std::fs::create_dir_all(&skill_dir).unwrap();
    std::fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: runner\n---\nRun `{{SKILL_DIR}}/run.sh`.\n",
    )
    .unwrap();
    std::fs::write(skill_dir.join("run.sh"), "#!/bin/bash\necho run").unwrap();
    let skill_dir = std::fs::canonicalize(&skill_dir).unwrap();

    let manager = Arc::new(lr_skills::SkillManager::new());
    manager.initial_scan(&[skill_dir.display().to_string()], &[]);
    let gateway = McpGateway::new(
        Arc::new(McpServerManager::new()),
        GatewayConfig::default(),
        test_router(),
    );
    gateway.register_virtual_server(Arc::new(
        localrouter::mcp::gateway::virtual_skills::SkillsVirtualServer::new(
            manager,
            lr_config::ContextManagementConfig::default(),
            lr_config::SkillsConfig::default(),
        ),
    ));

    let script = format!("{}/run.sh", skill_dir.display());
    let direct = skill_read_as(&gateway, "direct", lr_config::ClientMode::McpOnly).await;
    assert!(direct.contains(&format!("Run `{script}`")), "{direct}");
    assert!(
        direct.contains(&format!("**Location:** `{}/`", skill_dir.display())),
        "{direct}"
    );

    let via_llm = skill_read_as(&gateway, "via-llm", lr_config::ClientMode::McpViaLlm).await;
    assert!(via_llm.contains("Run `run.sh`"), "{via_llm}");
    assert!(
        !via_llm.contains(&skill_dir.display().to_string()),
        "{via_llm}"
    );
}

#[tokio::test]
async fn skill_read_does_not_initialize_unavailable_remote_backends() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("SKILL.md"), "---\nname: audit-skill\ndescription: availability fixture\n---\nLocal instructions remain readable.").unwrap();
    let skills = Arc::new(lr_skills::SkillManager::new());
    skills.initial_scan(&[directory.path().to_string_lossy().into_owned()], &[]);
    let manager = Arc::new(McpServerManager::new_for_test());
    manager.add_config(http_server(
        "offline",
        "Offline",
        "http://127.0.0.1:1/mcp".into(),
        true,
    ));
    let (notifications, mut received) = tokio::sync::broadcast::channel(16);
    let gateway = McpGateway::new_with_broadcast(
        manager,
        GatewayConfig::default(),
        test_router(),
        Some(Arc::new(notifications)),
    );
    gateway.register_virtual_server(Arc::new(
        lr_mcp::gateway::virtual_skills::SkillsVirtualServer::new(
            skills.clone(),
            Default::default(),
            Default::default(),
        ),
    ));
    register_context_mode(&gateway);
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        gateway.handle_request_with_skills(
            "skills-client",
            Some("local-skill-session"),
            vec!["offline".into()],
            vec![],
            allow_all(),
            lr_config::SkillsPermissions {
                global: PermissionState::Allow,
                ..Default::default()
            },
            "Skills client".into(),
            PermissionState::Off,
            PermissionState::Off,
            None,
            overrides(false, true),
            PermissionState::Off,
            PermissionState::Off,
            None,
            None,
            lr_config::ClientMode::default(),
            JsonRpcRequest::with_id(
                1,
                "tools/call".into(),
                Some(json!({"name":"SkillRead","arguments":{"name":"audit-skill"}})),
            ),
            None,
        ),
    )
    .await
    .expect("local SkillRead must not wait for offline upstreams")
    .unwrap();
    assert!(response.error.is_none(), "{:?}", response.error);
    assert_ne!(response.result.as_ref().unwrap()["isError"], json!(true));
    assert!(response
        .result
        .unwrap()
        .to_string()
        .contains("Local instructions remain readable"));
    assert!(gateway
        .get_session("local-skill-session")
        .unwrap()
        .read()
        .await
        .transports
        .is_none());
    gateway.notify_skills_changed().await;
    let session = gateway.get_session("local-skill-session").unwrap();
    let store = session
        .read()
        .await
        .virtual_server_state
        .get("_context_mode")
        .unwrap()
        .as_any()
        .downcast_ref::<localrouter::mcp::gateway::context_mode::ContextModeSessionState>()
        .unwrap()
        .store
        .clone();
    assert!(store.read("catalog:skills/audit-skill", None, None).is_ok());
    std::fs::write(
        directory.path().join("SKILL.md"),
        "---\nname: audit-renamed\ndescription: updated fixture\n---\nUpdated instructions.",
    )
    .unwrap();
    while received.try_recv().is_ok() {}
    skills.rescan(&[directory.path().to_string_lossy().into_owned()], &[]);
    gateway.notify_skills_changed().await;
    let (target, notification) =
        tokio::time::timeout(std::time::Duration::from_secs(1), received.recv())
            .await
            .unwrap()
            .unwrap();
    assert_eq!(
        target,
        lr_mcp::gateway::types::session_notification_key("local-skill-session")
    );
    assert_eq!(notification.method, "notifications/tools/list_changed");
    assert!(store
        .read("catalog:skills/audit-renamed", None, None)
        .is_ok());
    assert!(store
        .read("catalog:skills/audit-skill", None, None)
        .is_err());
}

async fn send_with_skills(
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
            lr_config::SkillsPermissions {
                global: PermissionState::Allow,
                ..Default::default()
            },
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

async fn slow_start_fixture(
    fail_initialize: bool,
) -> (
    Arc<McpGateway>,
    MockServer,
    tempfile::TempDir,
    tokio::sync::broadcast::Receiver<(String, lr_mcp::protocol::JsonRpcNotification)>,
) {
    let server = MockServer::start().await;
    Mock::given(http_method("POST")).and(JsonRpcMethod("initialize"))
        .respond_with(ResponseTemplate::new(if fail_initialize { 410 } else { 200 }).set_delay(std::time::Duration::from_secs(6)).set_body_json(if fail_initialize {
            json!({"error":"account_deleted"})
        } else {json!({
            "jsonrpc":"2.0", "id":1, "result":{"protocolVersion":"2025-11-25","capabilities":{"tools":{}},"serverInfo":{"name":"Slow","version":"1"}}
        })})).mount(&server).await;
    mock(
        &server,
        "tools/list",
        Some(json!({"tools":[{"name":"echo","inputSchema":{"type":"object"}}]})),
    )
    .await;
    mock(&server, "notifications/initialized", Some(json!({}))).await;
    let manager = Arc::new(McpServerManager::new_for_test());
    manager.add_config(http_server("slow-id", "Slow", server.uri(), true));
    let (broadcast, receiver) = tokio::sync::broadcast::channel(32);
    let gateway = Arc::new(McpGateway::new_with_broadcast(
        manager,
        GatewayConfig {
            server_timeout_seconds: 12,
            max_retry_attempts: 0,
            ..Default::default()
        },
        test_router(),
        Some(Arc::new(broadcast)),
    ));
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(
        directory.path().join("SKILL.md"),
        "---\nname: warmup-skill\ndescription: local fixture\n---\nAvailable while warming up.",
    )
    .unwrap();
    let skills = Arc::new(lr_skills::SkillManager::new());
    skills.initial_scan(&[directory.path().to_string_lossy().into_owned()], &[]);
    gateway.register_virtual_server(Arc::new(
        lr_mcp::gateway::virtual_skills::SkillsVirtualServer::new(
            skills,
            Default::default(),
            Default::default(),
        ),
    ));
    (gateway, server, directory, receiver)
}

#[tokio::test]
async fn slow_upstream_does_not_block_initialize_skills_discovery_or_ping() {
    use std::time::Duration;
    let (gateway, server, _directory, mut notifications) = slow_start_fixture(false).await;
    let initialized = tokio::time::timeout(Duration::from_secs(4), send_with_skills(&gateway, "slow-session", &["slow-id"], &allow_all(), overrides(false, false), "initialize", Some(json!({
        "protocolVersion":"2025-06-18", "capabilities":{},"clientInfo":{"name":"claude-code","version":"fixture"}
    })))).await.expect("foreground handshake must finish before Claude Code's 25s deadline");
    assert!(initialized.error.is_none());
    assert_eq!(initialized.result.unwrap()["_meta"]["initializing"], true);
    for (method, params) in [
        ("tools/list", None),
        ("ping", None),
        (
            "tools/call",
            Some(json!({"name":"SkillRead","arguments":{"name":"warmup-skill"}})),
        ),
    ] {
        let response = tokio::time::timeout(
            Duration::from_secs(1),
            send_with_skills(
                &gateway,
                "slow-session",
                &["slow-id"],
                &allow_all(),
                overrides(false, false),
                method,
                params,
            ),
        )
        .await
        .expect("local gateway operations must stay usable during upstream warm-up");
        assert!(response.error.is_none(), "{method}: {:?}", response.error);
        let result = response.result.unwrap();
        if method == "tools/list" {
            assert_eq!(result["_meta"]["initializing"], true);
            assert!(result["tools"]
                .as_array()
                .unwrap()
                .iter()
                .any(|tool| tool["name"] == "SkillRead"));
        } else if method == "tools/call" {
            assert!(result.to_string().contains("Available while warming up"));
        }
    }
    while notifications.try_recv().is_ok() {}
    let (key, notification) = tokio::time::timeout(Duration::from_secs(10), notifications.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        key,
        lr_mcp::gateway::types::session_notification_key("slow-session")
    );
    assert_eq!(notification.method, "notifications/tools/list_changed");
    let listed = send_with_skills(
        &gateway,
        "slow-session",
        &["slow-id"],
        &allow_all(),
        overrides(false, false),
        "tools/list",
        None,
    )
    .await;
    assert!(tool_names(&listed).contains(&"slow__echo".to_string()));
    let calls = server.received_requests().await.unwrap();
    assert_eq!(
        calls
            .iter()
            .filter(|call| serde_json::from_slice::<Value>(&call.body)
                .ok()
                .is_some_and(|body| body["method"] == "initialize"))
            .count(),
        1
    );
}

#[tokio::test]
async fn terminating_session_cancels_background_initialization() {
    let (gateway, _server, _directory, _notifications) = slow_start_fixture(false).await;
    initialize(&gateway, "cancel-warmup", &["slow-id"], &allow_all(), None).await;
    let session = gateway.get_session("cancel-warmup").unwrap();
    let task = session
        .read()
        .await
        .initialization_abort
        .clone()
        .expect("slow warm-up is running");
    gateway.terminate_session("cancel-warmup").await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        while !task.is_finished() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("teardown must abort warm-up instead of publishing into an orphaned session");
    assert!(gateway.get_session("cancel-warmup").is_none());
}

#[tokio::test]
async fn concurrent_stateless_discovery_shares_warmup_and_keeps_result_metadata() {
    use std::time::Duration;
    let (gateway, server, _directory, _notifications) = slow_start_fixture(false).await;
    let permissions = allow_all();
    let params = json!({"_meta": {
        "io.modelcontextprotocol/protocolVersion":"2026-07-28",
        "io.modelcontextprotocol/clientInfo":{"name":"stateless-fixture","version":"1"}
    }});
    let (first, second) = tokio::time::timeout(Duration::from_secs(4), async {
        tokio::join!(
            send_with_skills(
                &gateway,
                "stateless-warmup",
                &["slow-id"],
                &permissions,
                overrides(false, false),
                "tools/list",
                Some(params.clone())
            ),
            send_with_skills(
                &gateway,
                "stateless-warmup",
                &["slow-id"],
                &permissions,
                overrides(false, false),
                "tools/list",
                Some(params.clone())
            )
        )
    })
    .await
    .expect("concurrent discovery must not wait for remote initialization");
    for response in [first, second] {
        assert!(response.error.is_none());
        let result = response.result.unwrap();
        assert_eq!(result["resultType"], "complete");
        assert_eq!(result["_meta"]["initializing"], true);
        assert!(result["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["name"] == "SkillRead"));
    }
    tokio::time::sleep(Duration::from_secs(5)).await;
    let calls = server.received_requests().await.unwrap();
    assert_eq!(
        calls
            .iter()
            .filter(|call| serde_json::from_slice::<Value>(&call.body)
                .ok()
                .is_some_and(|b| b["method"] == "initialize"))
            .count(),
        1
    );
    gateway.terminate_session("stateless-warmup").await.unwrap();
}

#[tokio::test]
async fn failed_background_handshake_preserves_skills_and_partial_failure() {
    use std::time::Duration;
    let (gateway, _server, _directory, mut notifications) = slow_start_fixture(true).await;
    initialize(
        &gateway,
        "failed-warmup",
        &["slow-id"],
        &allow_all(),
        overrides(false, false),
    )
    .await;
    while notifications.try_recv().is_ok() {}
    tokio::time::timeout(Duration::from_secs(10), notifications.recv())
        .await
        .unwrap()
        .unwrap();
    let listed = send_with_skills(
        &gateway,
        "failed-warmup",
        &["slow-id"],
        &allow_all(),
        overrides(false, false),
        "tools/list",
        None,
    )
    .await;
    assert!(listed.error.is_none());
    let result = listed.result.unwrap();
    assert_eq!(result["_meta"]["partial_failure"], true);
    assert!(result["tools"]
        .as_array()
        .unwrap()
        .iter()
        .any(|t| t["name"] == "SkillRead"));
    let read = send_with_skills(
        &gateway,
        "failed-warmup",
        &["slow-id"],
        &allow_all(),
        overrides(false, false),
        "tools/call",
        Some(json!({"name":"SkillRead","arguments":{"name":"warmup-skill"}})),
    )
    .await;
    assert!(read.error.is_none());
    assert!(read
        .result
        .unwrap()
        .to_string()
        .contains("Available while warming up"));
}

#[tokio::test]
async fn compressed_skill_catalog_is_indexed_before_remote_warmup() {
    let (gateway, _server, _directory, _notifications) = slow_start_fixture(false).await;
    register_context_mode(&gateway);
    send_with_skills(
        &gateway,
        "indexed-warmup",
        &["slow-id"],
        &allow_all(),
        overrides(false, true),
        "initialize",
        Some(json!({"capabilities":{}})),
    )
    .await;
    let session = gateway.get_session("indexed-warmup").unwrap();
    let state = session.read().await;
    assert!(state.initialization_pending);
    let cm = state
        .virtual_server_state
        .get("_context_mode")
        .unwrap()
        .as_any()
        .downcast_ref::<localrouter::mcp::gateway::context_mode::ContextModeSessionState>()
        .unwrap();
    assert!(cm
        .store
        .read("catalog:skills/warmup-skill", None, None)
        .is_ok());
    drop(state);
    gateway.terminate_session("indexed-warmup").await.unwrap();
}

// Failed transports must recover inside the same client session. Tool writes
// with uncertain outcomes are never automatically replayed.
async fn wait_for_recovery(gateway: &McpGateway, key: &str) {
    tokio::time::timeout(std::time::Duration::from_secs(8), async {
        loop {
            let session = gateway.get_session(key).unwrap();
            let state = session.read().await;
            if !state.initialization_pending
                && state
                    .initialization_abort
                    .as_ref()
                    .is_none_or(|task| task.is_finished())
            {
                break;
            }
            drop(state);
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("background recovery must finish");
}

async fn recovery_backend(server: &MockServer) {
    mock(
        server,
        "initialize",
        Some(json!({
            "protocolVersion":"2025-11-25", "capabilities":{"tools":{}},
            "serverInfo":{"name":"recovering", "version":"1"}
        })),
    )
    .await;
    mock(
        server,
        "tools/list",
        Some(json!({"tools":[{
            "name":"write", "inputSchema":{"type":"object"}
        }]})),
    )
    .await;
    mock(
        server,
        "tools/call",
        Some(json!({"content":[{"type":"text","text":"ok"}]})),
    )
    .await;
}

async fn method_count(server: &MockServer, method: &str) -> usize {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| {
            serde_json::from_slice::<Value>(&r.body)
                .ok()
                .is_some_and(|v| v["method"] == method)
        })
        .count()
}

#[tokio::test]
async fn recovery_reattaches_failed_startup_without_rebuilding_the_client() {
    let recovering = MockServer::start().await;
    mock(&recovering, "initialize", None).await;
    let (gateway, _echo) = setup_full(
        None,
        vec![http_server(
            "recovering",
            "Recovering",
            recovering.uri(),
            true,
        )],
    )
    .await;
    initialize(
        &gateway,
        "recovery",
        &[ECHO_ID, "recovering"],
        &allow_all(),
        None,
    )
    .await;
    let before = gateway.get_session("recovery").unwrap();
    let attempts = method_count(&recovering, "initialize").await;
    assert!(attempts > 0);
    // A healthy tool is independent of the failed backend.
    send(
        &gateway,
        "recovery",
        &[ECHO_ID, "recovering"],
        &allow_all(),
        None,
        "tools/call",
        Some(json!({"name":"echo-server__echo","arguments":{}})),
    )
    .await;
    assert_eq!(method_count(&recovering, "initialize").await, attempts);
    recovering.reset().await;
    recovery_backend(&recovering).await;
    let _ = send(
        &gateway,
        "recovery",
        &[ECHO_ID, "recovering"],
        &allow_all(),
        None,
        "tools/list",
        None,
    )
    .await;
    wait_for_recovery(&gateway, "recovery").await;
    let response = send(
        &gateway,
        "recovery",
        &[ECHO_ID, "recovering"],
        &allow_all(),
        None,
        "tools/list",
        None,
    )
    .await;
    assert!(tool_names(&response).contains(&"recovering__write".to_string()));
    assert!(tool_names(&response).contains(&"echo-server__echo".to_string()));
    assert!(Arc::ptr_eq(
        &before,
        &gateway.get_session("recovery").unwrap()
    ));
    assert!(before.read().await.get_failed_servers().is_empty());
    assert!(response.result.unwrap().get("_meta").is_none());
    assert_eq!(method_count(&recovering, "initialize").await, 1);
}

#[tokio::test]
async fn recovery_bounds_failed_retries_and_respects_revoked_server_access() {
    let recovering = MockServer::start().await;
    mock(&recovering, "initialize", None).await;
    let (gateway, _echo) = setup_full(
        None,
        vec![http_server(
            "recovering",
            "Recovering",
            recovering.uri(),
            true,
        )],
    )
    .await;
    initialize(
        &gateway,
        "cooldown",
        &[ECHO_ID, "recovering"],
        &allow_all(),
        None,
    )
    .await;
    send(
        &gateway,
        "cooldown",
        &[ECHO_ID, "recovering"],
        &allow_all(),
        None,
        "tools/list",
        None,
    )
    .await;
    wait_for_recovery(&gateway, "cooldown").await;
    let attempts = method_count(&recovering, "initialize").await;
    send(
        &gateway,
        "cooldown",
        &[ECHO_ID, "recovering"],
        &allow_all(),
        None,
        "tools/list",
        None,
    )
    .await;
    assert_eq!(method_count(&recovering, "initialize").await, attempts);
    recovering.reset().await;
    recovery_backend(&recovering).await;
    let list = send(
        &gateway,
        "cooldown",
        &[ECHO_ID],
        &allow_all(),
        None,
        "tools/list",
        None,
    )
    .await;
    assert!(!tool_names(&list).contains(&"recovering__write".to_string()));
    assert_eq!(method_count(&recovering, "initialize").await, 0);
}

#[tokio::test]
async fn recovery_updates_running_connections_after_endpoint_changes() {
    let old = MockServer::start().await;
    recovery_backend(&old).await;
    let new = MockServer::start().await;
    recovery_backend(&new).await;
    let manager = Arc::new(McpServerManager::new_for_test());
    manager.add_config(http_server("changing", "Changing", old.uri(), true));
    let gateway = McpGateway::new(manager.clone(), GatewayConfig::default(), test_router());
    initialize(
        &gateway,
        "endpoint-change",
        &["changing"],
        &allow_all(),
        None,
    )
    .await;
    let before = gateway.get_session("endpoint-change").unwrap();
    manager.add_config(http_server("changing", "Changing", new.uri(), true));
    send(
        &gateway,
        "endpoint-change",
        &["changing"],
        &allow_all(),
        None,
        "tools/call",
        Some(json!({"name":"changing__write","arguments":{}})),
    )
    .await;
    assert_eq!(method_count(&old, "tools/call").await, 0);
    assert_eq!(method_count(&new, "initialize").await, 1);
    assert_eq!(method_count(&new, "tools/call").await, 1);
    assert!(Arc::ptr_eq(
        &before,
        &gateway.get_session("endpoint-change").unwrap()
    ));
}

#[tokio::test]
async fn recovery_retries_explicitly_expired_sessions_but_not_uncertain_writes() {
    use axum::{
        body::{Body, Bytes},
        http::{HeaderMap, StatusCode},
        response::IntoResponse,
        routing::get,
        Json, Router as HttpRouter,
    };
    use futures::StreamExt;
    use std::sync::atomic::{AtomicUsize, Ordering};
    let initialized = Arc::new(AtomicUsize::new(0));
    let writes = Arc::new(AtomicUsize::new(0));
    let mode = Arc::new(AtomicUsize::new(0));
    let init = initialized.clone();
    let written = writes.clone();
    let failure = mode.clone();
    let app=HttpRouter::new().route("/mcp",get(|| async {StatusCode::METHOD_NOT_ALLOWED}).post(move |headers:HeaderMap,Json(request):Json<Value>| {
        let init=init.clone();let written=written.clone();let failure=failure.clone();
        async move {
            let result=match request["method"].as_str().unwrap_or_default() {
                "initialize"=>{
                    let generation=init.fetch_add(1,Ordering::SeqCst)+1;
                    return ([("Mcp-Session-Id",format!("upstream-{generation}"))],Json(json!({"jsonrpc":"2.0","id":request["id"],"result":{
                        "protocolVersion":"2025-11-25","capabilities":{"tools":{}},"serverInfo":{"name":"recovery","version":"1"}
                    }}))).into_response();
                }
                "tools/list"=>json!({"tools":[{"name":"write","inputSchema":{"type":"object"}}]}),
                "tools/call"=>{
                    if failure.load(Ordering::SeqCst)==1 && headers.get("Mcp-Session-Id").is_some_and(|h| h=="upstream-1") {
                        return StatusCode::NOT_FOUND.into_response();
                    }
                    written.fetch_add(1,Ordering::SeqCst);
                    if failure.load(Ordering::SeqCst)==2 {
                        // Headers and a partial SSE event arrive, then the body
                        // fails. The write happened; replay would duplicate it.
                        let body=futures::stream::once(async {
                            Ok::<Bytes,std::io::Error>(Bytes::from_static(b"data: {\"jsonrpc\":"))
                        }).chain(futures::stream::once(async {
                            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
                            Err(std::io::Error::other("fixture stream reset"))
                        }));
                        return ([("content-type","text/event-stream")],Body::from_stream(body)).into_response();
                    }
                    json!({"content":[{"type":"text","text":"written"}]})
                }
                "notifications/initialized"=>return StatusCode::ACCEPTED.into_response(),
                _=>return Json(json!({"jsonrpc":"2.0","id":request["id"],"error":{"code":-32601,"message":"unsupported"}})).into_response(),
            };
            Json(json!({"jsonrpc":"2.0","id":request["id"],"result":result})).into_response()
        }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/mcp", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let manager = Arc::new(McpServerManager::new_for_test());
    manager.add_config(http_server("reset", "Reset", url, true));
    let gateway = McpGateway::new(manager, GatewayConfig::default(), test_router());
    initialize(&gateway, "reset-session", &["reset"], &allow_all(), None).await;
    let before = gateway.get_session("reset-session").unwrap();
    mode.store(1, Ordering::SeqCst);
    let responses = futures::future::join_all((0..8).map(|_| async {
        send(
            &gateway,
            "reset-session",
            &["reset"],
            &allow_all(),
            None,
            "tools/call",
            Some(json!({"name":"reset__write","arguments":{}})),
        )
        .await
    }))
    .await;
    assert!(responses.iter().all(|r| r.error.is_none()));
    assert_eq!(
        initialized.load(Ordering::SeqCst),
        2,
        "concurrent expired-session responses must share one repair"
    );
    assert_eq!(
        writes.load(Ordering::SeqCst),
        8,
        "404 rejections never executed a write"
    );
    mode.store(2, Ordering::SeqCst);
    let error = send_result(
        &gateway,
        "reset-session",
        &["reset"],
        &allow_all(),
        None,
        "tools/call",
        Some(json!({"name":"reset__write","arguments":{}})),
    )
    .await;
    assert!(error
        .unwrap_err()
        .to_string()
        .contains("MCP response stream failed"));
    assert_eq!(
        writes.load(Ordering::SeqCst),
        9,
        "broken response must not replay a write"
    );
    mode.store(0, Ordering::SeqCst);
    send(
        &gateway,
        "reset-session",
        &["reset"],
        &allow_all(),
        None,
        "tools/call",
        Some(json!({"name":"reset__write","arguments":{}})),
    )
    .await;
    assert_eq!(initialized.load(Ordering::SeqCst), 3);
    assert_eq!(writes.load(Ordering::SeqCst), 10);
    assert!(Arc::ptr_eq(
        &before,
        &gateway.get_session("reset-session").unwrap()
    ));
    gateway.terminate_session("reset-session").await.unwrap();
    task.abort();
}

#[tokio::test]
async fn recovery_monitor_marks_tool_errors_inside_successful_http_responses() {
    let (gateway, server) = setup().await;
    let events = Arc::new(parking_lot::Mutex::new(std::collections::HashMap::<
        String,
        lr_monitor::MonitorEvent,
    >::new()));
    let emitted = events.clone();
    gateway.set_on_monitor_event(
        move |event_type, client_id, client_name, session_id, data, status, duration_ms| {
            let id = uuid::Uuid::new_v4().to_string();
            emitted.lock().insert(
                id.clone(),
                lr_monitor::MonitorEvent {
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
                },
            );
            id
        },
    );
    let updated = events.clone();
    gateway.set_on_monitor_update(Arc::new(move |id, update| {
        if let Some(event) = updated.lock().get_mut(id) {
            update(event);
        }
    }));
    initialize(&gateway, "tool-error", &[ECHO_ID], &allow_all(), None).await;
    Mock::given(http_method("POST")).and(JsonRpcMethod("tools/call"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"jsonrpc":"2.0","id":1,"result":{
            "isError":true,"content":[{"type":"text","text":"Unauthorized; scope does not match"}]
        }}))).with_priority(1).mount(&server).await;
    let response = send(
        &gateway,
        "tool-error",
        &[ECHO_ID],
        &allow_all(),
        None,
        "tools/call",
        Some(json!({"name":"echo-server__echo","arguments":{}})),
    )
    .await;
    assert!(response.is_tool_error());
    {
        let events = events.lock();
        let event = events
            .values()
            .find(|e| e.event_type == lr_monitor::MonitorEventType::McpToolCall)
            .unwrap();
        assert_eq!(event.status, lr_monitor::EventStatus::Error);
        assert!(matches!(
            event.data,
            lr_monitor::MonitorEventData::McpToolCall {
                success: Some(false),
                ..
            }
        ));
    }
    // A scope denial is not treated as a dropped transport and does not trigger
    // replay or repeated reinitialization of a healthy MCP connection.
    assert_eq!(method_count(&server, "initialize").await, 1);
    assert_eq!(method_count(&server, "tools/call").await, 1);
}

#[tokio::test]
async fn recovery_negotiates_stateless_backends_after_failed_legacy_startup() {
    let server = MockServer::start().await;
    mock(&server, "initialize", None).await;
    let manager = Arc::new(McpServerManager::new_for_test());
    manager.add_config(http_server("modern", "Modern", server.uri(), true));
    let gateway = McpGateway::new(manager, GatewayConfig::default(), test_router());
    register_context_mode(&gateway);
    let cm = overrides(false, true);
    initialize(
        &gateway,
        "modern-recovery",
        &["modern"],
        &allow_all(),
        cm.clone(),
    )
    .await;
    server.reset().await;
    mock(
        &server,
        "server/discover",
        Some(json!({
            "protocolVersions":["2026-07-28"],"capabilities":{"tools":{}},
            "serverInfo":{"name":"modern","version":"2"}
        })),
    )
    .await;
    mock(&server,"tools/list",Some(json!({"tools":[{"name":"fresh","description":"Recovered tool","inputSchema":{"type":"object"}}]}))).await;
    send(
        &gateway,
        "modern-recovery",
        &["modern"],
        &allow_all(),
        cm.clone(),
        "tools/list",
        None,
    )
    .await;
    wait_for_recovery(&gateway, "modern-recovery").await;
    let session = gateway.get_session("modern-recovery").unwrap();
    let state = session.read().await;
    assert!(state
        .transports
        .as_ref()
        .unwrap()
        .revision("modern")
        .is_stateless());
    let context = state
        .virtual_server_state
        .get("_context_mode")
        .unwrap()
        .as_any()
        .downcast_ref::<localrouter::mcp::gateway::context_mode::ContextModeSessionState>()
        .unwrap();
    assert!(context
        .full_tool_catalog
        .iter()
        .any(|t| t.name == "modern__fresh"));
    assert!(context
        .catalog_sources
        .keys()
        .any(|label| label.contains("modern__fresh")));
    drop(state);
    assert_eq!(
        method_count(&server, "initialize").await,
        0,
        "modern recovery must not require legacy initialize"
    );
}

#[tokio::test]
async fn recovery_discovery_stays_responsive_and_teardown_cancels_reconnect() {
    let server = MockServer::start().await;
    mock(&server, "initialize", None).await;
    let (gateway, _echo) = setup_full(
        None,
        vec![http_server(
            "slow-recovery",
            "Slow Recovery",
            server.uri(),
            true,
        )],
    )
    .await;
    initialize(
        &gateway,
        "cancel-recovery",
        &[ECHO_ID, "slow-recovery"],
        &allow_all(),
        None,
    )
    .await;
    server.reset().await;
    Mock::given(http_method("POST")).and(JsonRpcMethod("initialize"))
        .respond_with(ResponseTemplate::new(200).set_delay(std::time::Duration::from_secs(6)).set_body_json(json!({
            "jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-11-25","capabilities":{"tools":{}},"serverInfo":{"name":"slow","version":"1"}}
        }))).mount(&server).await;
    tokio::time::timeout(
        std::time::Duration::from_secs(1),
        send(
            &gateway,
            "cancel-recovery",
            &[ECHO_ID, "slow-recovery"],
            &allow_all(),
            None,
            "tools/list",
            None,
        ),
    )
    .await
    .expect("failed remote recovery must not block discovery");
    let session = gateway.get_session("cancel-recovery").unwrap();
    let recovery = session
        .read()
        .await
        .initialization_abort
        .clone()
        .expect("recovery is tracked for teardown");
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while method_count(&server, "initialize").await == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    gateway.terminate_session("cancel-recovery").await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        while !recovery.is_finished() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("session teardown must abort recovery");
    assert!(session
        .read()
        .await
        .transports
        .as_ref()
        .unwrap()
        .running_server_ids()
        .is_empty());
}
