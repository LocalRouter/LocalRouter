//! HTTP-transport tests for the MCP 2026-07-28 stateless revision.
//!
//! Exercises the pieces that only exist at the HTTP layer (headers, status
//! codes, MRTR retry validation) against the real server, complementing the
//! gateway-level integration tests in mcp_gateway_mock_integration_tests.rs.

use localrouter::clients::{ClientManager, TokenStore};
use localrouter::config::{AppConfig, Client, ConfigManager, Strategy};
use localrouter::mcp::McpServerManager;
use localrouter::monitoring::metrics::MetricsCollector;
use localrouter::monitoring::storage::MetricsDatabase;
use localrouter::providers::registry::ProviderRegistry;
use localrouter::router::{RateLimiterManager, Router};
use localrouter::server;
use serde_json::json;
use std::sync::Arc;
use tokio::time::{sleep, Duration};

fn create_test_client(id: &str, strategy_id: &str) -> Client {
    let mut client = Client::new_with_strategy("Test Client".to_string(), strategy_id.to_string());
    client.id = id.to_string();
    client.enabled = true;
    client
}

/// Start the real HTTP server (no MCP backends configured — the gateway also
/// serves stateless lifecycle methods without any backend running).
async fn start_test_server() -> (String, String) {
    let test_client = create_test_client("test-api-key", "default");
    let strategy = Strategy::new("Default".to_string());

    let config = AppConfig {
        clients: vec![test_client.clone()],
        strategies: vec![strategy],
        ..Default::default()
    };

    let config_path =
        std::env::temp_dir().join(format!("test_stateless_http_{}.yaml", uuid::Uuid::new_v4()));
    let config_manager = Arc::new(ConfigManager::new(config, config_path));

    let provider_registry = Arc::new(ProviderRegistry::new());
    let mcp_server_manager = Arc::new(McpServerManager::new());
    let metrics_db_path =
        std::env::temp_dir().join(format!("test_stateless_http_{}.db", uuid::Uuid::new_v4()));
    let metrics_db = Arc::new(MetricsDatabase::new(metrics_db_path).unwrap());
    let metrics_collector = Arc::new(MetricsCollector::new(metrics_db));

    let rate_limiter = Arc::new(RateLimiterManager::new(None));
    let router = Arc::new(Router::new(
        config_manager.clone(),
        provider_registry.clone(),
        rate_limiter.clone(),
        metrics_collector.clone(),
        Arc::new(lr_router::FreeTierManager::new(None)),
    ));
    let client_manager = Arc::new(ClientManager::new(vec![test_client]));
    let token_store = Arc::new(TokenStore::new());

    let test_port = 43000 + (std::process::id() % 10000) as u16;
    let server_config = server::ServerConfig {
        host: "127.0.0.1".to_string(),
        port: test_port,
        enable_cors: true,
    };

    let (state, _handle, actual_port, _shutdown) = server::start_server(
        server_config,
        router,
        mcp_server_manager,
        rate_limiter,
        provider_registry,
        config_manager,
        client_manager,
        token_store,
        metrics_collector,
        None,
    )
    .await
    .expect("Failed to start test server");

    let base_url = format!("http://127.0.0.1:{}", actual_port);
    let secret = state.get_internal_test_secret();
    sleep(Duration::from_millis(200)).await;

    (base_url, secret)
}

fn stateless_meta() -> serde_json::Value {
    json!({
        "io.modelcontextprotocol/protocolVersion": "2026-07-28",
        "io.modelcontextprotocol/clientInfo": {"name": "http-test", "version": "1.0"}
    })
}

#[tokio::test]
async fn test_server_discover_over_http_with_version_echo() {
    let (base_url, secret) = start_test_server().await;
    let client = reqwest::Client::new();

    let response = client
        .post(&base_url)
        .bearer_auth(&secret)
        .header("MCP-Protocol-Version", "2026-07-28")
        .header("Mcp-Method", "server/discover")
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "server/discover",
            "params": { "_meta": stateless_meta() }
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    // The negotiated protocol version is echoed to stateless peers
    assert_eq!(
        response
            .headers()
            .get("mcp-protocol-version")
            .and_then(|v| v.to_str().ok()),
        Some("2026-07-28")
    );

    let body: serde_json::Value = response.json().await.unwrap();
    let result = &body["result"];
    assert!(result["protocolVersions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v == "2026-07-28"));
    assert_eq!(result["serverInfo"]["name"], "LocalRouter MCP Gateway");
    assert_eq!(result["resultType"], "complete");
}

#[tokio::test]
async fn test_mcp_method_header_mismatch_rejected() {
    let (base_url, secret) = start_test_server().await;
    let client = reqwest::Client::new();

    let response = client
        .post(&base_url)
        .bearer_auth(&secret)
        .header("MCP-Protocol-Version", "2026-07-28")
        .header("Mcp-Method", "tools/list") // disagrees with the body
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "server/discover",
            "params": { "_meta": stateless_meta() }
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 400);
    let body: serde_json::Value = response.json().await.unwrap();
    assert_eq!(body["error"]["code"], -32020); // HeaderMismatchError
}

#[tokio::test]
async fn test_future_protocol_version_rejected() {
    let (base_url, secret) = start_test_server().await;
    let client = reqwest::Client::new();

    let response = client
        .post(&base_url)
        .bearer_auth(&secret)
        .header("MCP-Protocol-Version", "2031-01-01")
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "server/discover",
            "params": {}
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 400);
    let body: serde_json::Value = response.json().await.unwrap();
    assert_eq!(body["error"]["code"], -32022); // UnsupportedProtocolVersion
    assert!(body["error"]["data"]["supported"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v == "2026-07-28"));
}

#[tokio::test]
async fn test_mrtr_unknown_request_state_rejected() {
    let (base_url, secret) = start_test_server().await;
    let client = reqwest::Client::new();

    // A stateless retry carrying a requestState we never issued (or that
    // expired) must be rejected, not silently re-executed.
    let response = client
        .post(&base_url)
        .bearer_auth(&secret)
        .header("MCP-Protocol-Version", "2026-07-28")
        .header("Mcp-Method", "tools/call")
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 7,
            "method": "tools/call",
            "params": {
                "name": "whatever__tool",
                "requestState": "no-such-state",
                "inputResponses": [{"id": "x", "response": {"action": "accept", "content": {}}}],
                "_meta": stateless_meta()
            }
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200); // JSON-RPC error in body
    let body: serde_json::Value = response.json().await.unwrap();
    assert_eq!(body["error"]["code"], -32602);
    assert!(body["error"]["message"]
        .as_str()
        .unwrap()
        .contains("requestState"));
}

#[tokio::test]
async fn test_legacy_mcp_post_unchanged() {
    let (base_url, secret) = start_test_server().await;
    let client = reqwest::Client::new();

    // A legacy client: no version header, no _meta. initialize must work and
    // the response must carry no stateless fields or version echo header.
    let response = client
        .post(&base_url)
        .bearer_auth(&secret)
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 0,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-11-25",
                "capabilities": {},
                "clientInfo": {"name": "legacy", "version": "1.0"}
            }
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    assert!(response.headers().get("mcp-protocol-version").is_none());

    let body: serde_json::Value = response.json().await.unwrap();
    let result = &body["result"];
    assert!(result["protocolVersion"].is_string());
    assert!(result.get("resultType").is_none());
}
