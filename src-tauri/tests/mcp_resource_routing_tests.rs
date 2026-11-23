//! `resources/read` and `resources/subscribe` find the owning upstream
//! without the client listing first: a resource that appeared after the
//! session's listing, a URI matching one server's resource template, and a
//! namespaced name of a server whose list does not carry it. Also: the
//! initialize description carries each server's catalog counts.
//!
//! Upstreams are local wiremock servers.

use chrono::Utc;
use localrouter::config::{
    AppConfig, ConfigManager, McpServerConfig, McpTransportConfig, McpTransportType,
};
use localrouter::mcp::gateway::{GatewayConfig, McpGateway};
use localrouter::mcp::protocol::{JsonRpcRequest, JsonRpcResponse};
use localrouter::mcp::McpServerManager;
use localrouter::monitoring::metrics::MetricsCollector;
use localrouter::monitoring::storage::MetricsDatabase;
use localrouter::providers::registry::ProviderRegistry;
use localrouter::router::{RateLimiterManager, Router};
use lr_config::{McpPermissions, PermissionState};
use serde_json::{json, Value};
use std::sync::Arc;
use wiremock::{
    matchers::method as http_method, Match, Mock, MockServer, Request, ResponseTemplate,
};

const FILES_ID: &str = "files-id";
const LOGS_ID: &str = "logs-id";

fn test_router() -> Arc<Router> {
    let config_manager = Arc::new(ConfigManager::new(
        AppConfig::default(),
        std::env::temp_dir().join(format!("resource_routing_{}.yaml", uuid::Uuid::new_v4())),
    ));
    let metrics_db = Arc::new(
        MetricsDatabase::new(
            std::env::temp_dir().join(format!("resource_routing_{}.db", uuid::Uuid::new_v4())),
        )
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
            .and_then(|b| b.get("method")?.as_str().map(|m| m == self.0))
            .unwrap_or(false)
    }
}

async fn answer(server: &MockServer, method: &'static str, result: Value, times: Option<u64>) {
    let mock = Mock::given(http_method("POST"))
        .and(JsonRpcMethod(method))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"jsonrpc": "2.0", "id": 1, "result": result})),
        );
    match times {
        Some(n) => mock.up_to_n_times(n).mount(server).await,
        None => mock.mount(server).await,
    }
}

/// A resource server: `first_list` answers the initial `resources/list`,
/// `later_list` every later one; reads answer `read_text`.
async fn resource_server(
    name: &str,
    first_list: Value,
    later_list: Value,
    template: &str,
    read_text: &str,
) -> MockServer {
    let server = MockServer::start().await;
    answer(
        &server,
        "initialize",
        json!({"protocolVersion": "2025-11-25",
            "capabilities": {"tools": {}, "resources": {"subscribe": true}},
            "serverInfo": {"name": name, "version": "1"}}),
        None,
    )
    .await;
    answer(
        &server,
        "tools/list",
        json!({"tools": [{"name": "t", "inputSchema": {"type": "object"}}]}),
        None,
    )
    .await;
    answer(
        &server,
        "resources/list",
        json!({"resources": first_list}),
        Some(1),
    )
    .await;
    answer(
        &server,
        "resources/list",
        json!({"resources": later_list}),
        None,
    )
    .await;
    answer(
        &server,
        "resources/templates/list",
        json!({"resourceTemplates": [{"name": "tpl", "uriTemplate": template}]}),
        None,
    )
    .await;
    answer(
        &server,
        "resources/read",
        json!({"contents": [{"uri": "x", "text": read_text}]}),
        None,
    )
    .await;
    answer(&server, "resources/subscribe", json!({}), None).await;
    Mock::given(http_method("POST"))
        .respond_with(ResponseTemplate::new(202))
        .mount(&server)
        .await;
    server
}

fn http_server(id: &str, name: &str, url: String) -> McpServerConfig {
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
        enabled: true,
        created_at: Utc::now(),
        capability_snapshot: None,
    }
}

async fn send(gateway: &McpGateway, method: &str, params: Value) -> JsonRpcResponse {
    gateway
        .handle_request_with_skills(
            "client-1",
            Some("s1"),
            vec![FILES_ID.to_string(), LOGS_ID.to_string()],
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
            JsonRpcRequest::new(Some(json!(1)), method.to_string(), Some(params)),
            None,
        )
        .await
        .unwrap_or_else(|e| panic!("{method} failed: {e}"))
}

fn read_text(response: &JsonRpcResponse) -> String {
    response
        .result
        .as_ref()
        .unwrap_or_else(|| panic!("no result: {:?}", response.error))["contents"][0]["text"]
        .as_str()
        .unwrap()
        .to_string()
}

async fn requests_of(server: &MockServer, method: &str) -> Vec<Value> {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter_map(|r| serde_json::from_slice::<Value>(&r.body).ok())
        .filter(|b| b["method"] == method)
        .collect()
}

#[tokio::test]
async fn resources_resolve_without_a_prior_list() {
    let files = resource_server(
        "files",
        json!([{"uri": "file:///a.txt", "name": "a"}]),
        json!([{"uri": "file:///a.txt", "name": "a"}, {"uri": "file:///late.txt", "name": "late"}]),
        "file:///{path}",
        "from files",
    )
    .await;
    let logs = resource_server("logs", json!([]), json!([]), "logs://{day}", "from logs").await;
    let manager = Arc::new(McpServerManager::new());
    manager.add_config(http_server(FILES_ID, "Files", files.uri()));
    manager.add_config(http_server(LOGS_ID, "Logs", logs.uri()));
    let gateway = McpGateway::new(manager, GatewayConfig::default(), test_router());

    let init = send(
        &gateway,
        "initialize",
        json!({"protocolVersion": "2025-11-25", "capabilities": {},
            "clientInfo": {"name": "t", "version": "1"}}),
    )
    .await;
    assert!(init.error.is_none(), "{:?}", init.error);
    // Backends may finish connecting after initialize answered
    let session = gateway.get_session("s1").unwrap();
    let mut description = None;
    for _ in 0..1000 {
        {
            let state = session.read().await;
            if !state.initialization_pending {
                description = state
                    .merged_capabilities
                    .as_ref()
                    .and_then(|m| m.server_info.description.clone());
            }
        }
        if description
            .as_deref()
            .is_some_and(|d| d.contains("Catalog:"))
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    // The description carries the catalog counts fetched at initialize
    let description = description.expect("merged description");
    assert!(
        description.contains("Catalog: 1 tool, 1 resource"),
        "{description}"
    );
    assert!(
        description.contains("Catalog: 1 tool, 0 resources"),
        "{description}"
    );

    // Listed at initialize: routed from the mapping
    let read = send(&gateway, "resources/read", json!({"uri": "file:///a.txt"})).await;
    assert_eq!(read_text(&read), "from files");

    // Appeared after the session's listing: found by listing again
    let read = send(
        &gateway,
        "resources/read",
        json!({"uri": "file:///late.txt"}),
    )
    .await;
    assert_eq!(read_text(&read), "from files");
    let late = gateway
        .get_session("s1")
        .unwrap()
        .read()
        .await
        .resource_uri_mapping
        .get("file:///late.txt")
        .cloned();
    assert_eq!(late.map(|(id, _)| id).as_deref(), Some(FILES_ID));

    // Listed nowhere: the server whose template matches serves it
    let read = send(
        &gateway,
        "resources/read",
        json!({"uri": "logs://2026-11-22"}),
    )
    .await;
    assert_eq!(read_text(&read), "from logs");
    let routed = requests_of(&logs, "resources/read").await;
    assert_eq!(routed.last().unwrap()["params"]["uri"], "logs://2026-11-22");

    // A namespaced name of a server that lists nothing: its prefix routes it,
    // with the namespace stripped
    let read = send(&gateway, "resources/read", json!({"name": "logs__daily"})).await;
    assert_eq!(read_text(&read), "from logs");
    let routed = requests_of(&logs, "resources/read").await;
    assert_eq!(routed.last().unwrap()["params"]["name"], "daily");

    // Subscribe needs no prior list either
    let subscribed = send(
        &gateway,
        "resources/subscribe",
        json!({"uri": "logs://today"}),
    )
    .await;
    assert!(subscribed.error.is_none(), "{:?}", subscribed.error);
    let subscribes = requests_of(&logs, "resources/subscribe").await;
    assert_eq!(subscribes.last().unwrap()["params"]["uri"], "logs://today");

    // Owned by nobody: not found
    let missing = send(&gateway, "resources/read", json!({"uri": "s3://bucket/x"})).await;
    assert!(missing.error.is_some());
    let missing = send(&gateway, "resources/read", json!({"name": "nobody__x"})).await;
    assert!(missing.error.is_some());
}
