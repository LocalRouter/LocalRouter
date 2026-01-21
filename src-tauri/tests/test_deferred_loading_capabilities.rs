//! Tests for deferred loading with client capability checking

use localrouter_ai::config::{
    AppConfig, ConfigManager, McpAuthConfig, McpServerConfig, McpTransportConfig, McpTransportType,
};
use localrouter_ai::mcp::gateway::{GatewayConfig, McpGateway};
use localrouter_ai::mcp::protocol::JsonRpcRequest;
use localrouter_ai::mcp::McpServerManager;
use localrouter_ai::monitoring::database::MetricsDatabase;
use localrouter_ai::monitoring::metrics::MetricsCollector;
use localrouter_ai::providers::health::HealthCheckManager;
use localrouter_ai::providers::registry::ProviderRegistry;
use localrouter_ai::router::{RateLimiterManager, Router};
use serde_json::json;
use std::sync::Arc;
use wiremock::{matchers::method as http_method, Mock, MockServer, ResponseTemplate};

fn create_test_router() -> Arc<Router> {
    let config = AppConfig::default();
    let config_manager = Arc::new(ConfigManager::new(
        config,
        std::path::PathBuf::from("/tmp/test_deferred_cap.yaml"),
    ));

    let health_manager = Arc::new(HealthCheckManager::default());
    let provider_registry = Arc::new(ProviderRegistry::new(health_manager));
    let rate_limiter = Arc::new(RateLimiterManager::new(None));

    let metrics_db_path =
        std::env::temp_dir().join(format!("test_deferred_cap_{}.db", uuid::Uuid::new_v4()));
    let metrics_db = Arc::new(MetricsDatabase::new(metrics_db_path).unwrap());
    let metrics_collector = Arc::new(MetricsCollector::new(metrics_db));

    Arc::new(Router::new(
        config_manager,
        provider_registry,
        rate_limiter,
        metrics_collector,
    ))
}

async fn setup_test_gateway(
) -> (
    Arc<McpGateway>,
    Arc<McpServerManager>,
    MockServer,
) {
    let server_mock = MockServer::start().await;
    let server_url = server_mock.uri();

    let mut config = AppConfig::default();
    config.mcp.servers.insert(
        "test_server".to_string(),
        McpServerConfig {
            command: None,
            args: None,
            transport: McpTransportConfig {
                transport_type: McpTransportType::Sse,
                url: Some(server_url.clone()),
                headers: None,
            },
            env: None,
            oauth: None,
            auth: Some(McpAuthConfig::None),
        },
    );

    let config_manager = Arc::new(ConfigManager::new(
        config,
        std::path::PathBuf::from("/tmp/test_deferred_cap_config.yaml"),
    ));

    let metrics_db_path =
        std::env::temp_dir().join(format!("test_deferred_cap_metrics_{}.db", uuid::Uuid::new_v4()));
    let metrics_db = Arc::new(MetricsDatabase::new(metrics_db_path).unwrap());
    let metrics_collector = Arc::new(MetricsCollector::new(metrics_db));

    let server_manager = Arc::new(McpServerManager::new(config_manager.clone(), metrics_collector));

    let router = create_test_router();
    let gateway_config = GatewayConfig::default();
    let gateway = Arc::new(McpGateway::new(server_manager.clone(), gateway_config, router));

    (gateway, server_manager, server_mock)
}

fn extract_result(response: &localrouter_ai::mcp::protocol::JsonRpcResponse) -> serde_json::Value {
    response.result.as_ref().unwrap().clone()
}

#[tokio::test]
async fn test_deferred_loading_enabled_with_client_capability() {
    let (gateway, _manager, server_mock) = setup_test_gateway().await;

    // Mock initialize response
    let init_response = json!({
        "protocolVersion": "2024-11-05",
        "capabilities": { "tools": {} },
        "serverInfo": { "name": "Test Server", "version": "1.0.0" }
    });

    let sse_body1 = format!("data: {}\n\n", serde_json::to_string(&json!({
        "jsonrpc": "2.0",
        "id": 1,
        "result": init_response
    })).unwrap());

    Mock::given(http_method("POST"))
        .respond_with(ResponseTemplate::new(200)
            .set_body_string(sse_body1)
            .insert_header("content-type", "text/event-stream"))
        .mount(&server_mock)
        .await;

    // Mock tools/list for catalog fetch
    let tools_response = json!({
        "tools": [
            {"name": "read_file", "description": "Read file", "inputSchema": {"type": "object"}},
            {"name": "write_file", "description": "Write file", "inputSchema": {"type": "object"}}
        ]
    });

    let sse_body2 = format!("data: {}\n\n", serde_json::to_string(&json!({
        "jsonrpc": "2.0",
        "id": 2,
        "result": tools_response
    })).unwrap());

    Mock::given(http_method("POST"))
        .respond_with(ResponseTemplate::new(200)
            .set_body_string(sse_body2)
            .insert_header("content-type", "text/event-stream"))
        .mount(&server_mock)
        .await;

    // Mock resources/list and prompts/list
    let empty_resources = format!("data: {}\n\n", serde_json::to_string(&json!({
        "jsonrpc": "2.0",
        "id": 3,
        "result": {"resources": []}
    })).unwrap());

    let empty_prompts = format!("data: {}\n\n", serde_json::to_string(&json!({
        "jsonrpc": "2.0",
        "id": 4,
        "result": {"prompts": []}
    })).unwrap());

    Mock::given(http_method("POST"))
        .respond_with(ResponseTemplate::new(200)
            .set_body_string(empty_resources)
            .insert_header("content-type", "text/event-stream"))
        .mount(&server_mock)
        .await;

    Mock::given(http_method("POST"))
        .respond_with(ResponseTemplate::new(200)
            .set_body_string(empty_prompts)
            .insert_header("content-type", "text/event-stream"))
        .mount(&server_mock)
        .await;

    // Client declares support for tools.listChanged
    let initialize_request = JsonRpcRequest::new(
        Some(json!(1)),
        "initialize".to_string(),
        Some(json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {
                "tools": { "listChanged": true }  // Client supports listChanged!
            },
            "clientInfo": {"name": "test-client", "version": "1.0"}
        })),
    );

    let allowed_servers = vec!["test_server".to_string()];

    // Request with deferred_loading = true
    let response = gateway
        .handle_request("test-client-deferred", allowed_servers.clone(), true, vec![], initialize_request)
        .await
        .unwrap();

    // Verify initialize succeeded
    assert!(response.result.is_some());

    // Now request tools/list - should return only the search tool initially
    let tools_request = JsonRpcRequest::new(Some(json!(2)), "tools/list".to_string(), Some(json!({})));

    let tools_response = gateway
        .handle_request("test-client-deferred", allowed_servers, false, vec![], tools_request)
        .await
        .unwrap();

    let result = extract_result(&tools_response);
    let tools = result["tools"].as_array().unwrap();

    // With deferred loading enabled, should see only the search tool initially
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0]["name"], "search");
    assert!(tools[0]["description"].as_str().unwrap().contains("Search for tools"));
}

#[tokio::test]
async fn test_deferred_loading_falls_back_without_client_capability() {
    let (gateway, _manager, server_mock) = setup_test_gateway().await;

    // Mock initialize response
    let init_response = json!({
        "protocolVersion": "2024-11-05",
        "capabilities": { "tools": {} },
        "serverInfo": { "name": "Test Server", "version": "1.0.0" }
    });

    let sse_body1 = format!("data: {}\n\n", serde_json::to_string(&json!({
        "jsonrpc": "2.0",
        "id": 1,
        "result": init_response
    })).unwrap());

    Mock::given(http_method("POST"))
        .respond_with(ResponseTemplate::new(200)
            .set_body_string(sse_body1)
            .insert_header("content-type", "text/event-stream"))
        .mount(&server_mock)
        .await;

    // Mock tools/list for normal mode
    let tools_response = json!({
        "tools": [
            {"name": "read_file", "description": "Read file", "inputSchema": {"type": "object"}},
            {"name": "write_file", "description": "Write file", "inputSchema": {"type": "object"}}
        ]
    });

    let sse_body2 = format!("data: {}\n\n", serde_json::to_string(&json!({
        "jsonrpc": "2.0",
        "id": 2,
        "result": tools_response
    })).unwrap());

    Mock::given(http_method("POST"))
        .respond_with(ResponseTemplate::new(200)
            .set_body_string(sse_body2)
            .insert_header("content-type", "text/event-stream"))
        .mount(&server_mock)
        .await;

    // Client does NOT declare support for tools.listChanged
    let initialize_request = JsonRpcRequest::new(
        Some(json!(1)),
        "initialize".to_string(),
        Some(json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {
                // No tools.listChanged declared!
            },
            "clientInfo": {"name": "test-client", "version": "1.0"}
        })),
    );

    let allowed_servers = vec!["test_server".to_string()];

    // Request with deferred_loading = true, but client doesn't support it
    let response = gateway
        .handle_request("test-client-no-cap", allowed_servers.clone(), true, vec![], initialize_request)
        .await
        .unwrap();

    // Verify initialize succeeded
    assert!(response.result.is_some());

    // Now request tools/list - should return ALL tools (normal mode fallback)
    let tools_request = JsonRpcRequest::new(Some(json!(2)), "tools/list".to_string(), Some(json!({})));

    let tools_response = gateway
        .handle_request("test-client-no-cap", allowed_servers, false, vec![], tools_request)
        .await
        .unwrap();

    let result = extract_result(&tools_response);
    let tools = result["tools"].as_array().unwrap();

    // Without client capability, should fall back to normal mode - all tools visible
    assert!(tools.len() >= 2);
    assert!(tools.iter().any(|t| t["name"] == "test_server__read_file"));
    assert!(tools.iter().any(|t| t["name"] == "test_server__write_file"));

    // Should NOT have the search tool
    assert!(!tools.iter().any(|t| t["name"] == "search"));
}
