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

/// Gateway with one working "Echo Server" (tools `echo` and `danger`,
/// prompts `shown` and `hidden`) plus one globally disabled server that always
/// fails to start.
async fn setup() -> (Arc<McpGateway>, MockServer) {
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
    mock(
        &echo,
        "tools/call",
        Some(json!({"content": [{"type": "text", "text": "echoed"}]})),
    )
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

    let gateway = Arc::new(McpGateway::new(
        manager,
        GatewayConfig::default(),
        test_router(),
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
