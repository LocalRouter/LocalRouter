//! Unified API integration tests
//!
//! Tests the unified API surface where MCP and OpenAI endpoints coexist
//! under the same base URL with no path conflicts.

use localrouter::clients::{ClientManager, TokenStore};
use localrouter::config::AppConfig;
use localrouter::mcp::McpServerManager;
use localrouter::monitoring::metrics::MetricsCollector;
use localrouter::monitoring::storage::MetricsDatabase;
use localrouter::providers::registry::ProviderRegistry;
use localrouter::router::{RateLimiterManager, Router};
use localrouter::server;
use std::sync::Arc;
use tokio::time::{sleep, Duration};

/// Helper to start a test server on an available port
async fn start_test_server() -> (String, tokio::task::JoinHandle<()>) {
    // Create test configuration
    let config = AppConfig::default();
    let config_manager = Arc::new(localrouter::config::ConfigManager::new(
        config.clone(),
        std::path::PathBuf::from("/tmp/test_unified_api.yaml"),
    ));

    // Create dependencies
    let provider_registry = Arc::new(ProviderRegistry::new());
    let mcp_server_manager = Arc::new(McpServerManager::new());

    // Create metrics collector with temporary database
    let metrics_db_path =
        std::env::temp_dir().join(format!("test_unified_api_{}.db", uuid::Uuid::new_v4()));
    let metrics_db = Arc::new(MetricsDatabase::new(metrics_db_path).unwrap());
    let metrics_collector = Arc::new(MetricsCollector::new(metrics_db));

    let rate_limiter = Arc::new(RateLimiterManager::new(None));
    let router = Arc::new(Router::new(
        config_manager.clone(),
        provider_registry.clone(),
        rate_limiter.clone(),
        metrics_collector.clone(),
    ));
    let client_manager = Arc::new(ClientManager::new(vec![]));
    let token_store = Arc::new(TokenStore::new());

    // Find an available port by trying a range
    let test_port = 40000 + (std::process::id() % 10000);
    let server_config = server::ServerConfig {
        host: "127.0.0.1".to_string(),
        port: test_port as u16,
        enable_cors: true,
    };

    let (_, handle, actual_port) = server::start_server(
        server_config,
        router,
        mcp_server_manager,
        rate_limiter,
        provider_registry,
        config_manager,
        client_manager,
        token_store,
        metrics_collector,
    )
    .await
    .expect("Failed to start test server");

    let base_url = format!("http://127.0.0.1:{}", actual_port);

    // Give server a moment to fully start
    sleep(Duration::from_millis(500)).await;

    (base_url, handle)
}

#[tokio::test]
async fn test_root_get_returns_documentation() {
    let (base_url, _handle) = start_test_server().await;

    let client = reqwest::Client::new();
    let response = client.get(&base_url).send().await.expect("Failed to GET /");

    assert_eq!(response.status(), 200);
    let body = response.text().await.expect("Failed to read body");

    // Should contain API documentation
    assert!(body.contains("LocalRouter"));
    assert!(body.contains("MCP Gateway"));
    assert!(body.contains("JSON-RPC"));
}

#[tokio::test]
async fn test_root_post_requires_auth() {
    let (base_url, _handle) = start_test_server().await;

    let client = reqwest::Client::new();
    let response = client
        .post(&base_url)
        .json(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/list",
            "params": {}
        }))
        .send()
        .await
        .expect("Failed to POST /");

    // Should require authentication
    assert_eq!(response.status(), 401);
}

#[tokio::test]
async fn test_health_endpoint() {
    let (base_url, _handle) = start_test_server().await;

    let client = reqwest::Client::new();
    let response = client
        .get(format!("{}/health", base_url))
        .send()
        .await
        .expect("Failed to GET /health");

    assert_eq!(response.status(), 200);
}

#[tokio::test]
async fn test_openapi_json_endpoint() {
    let (base_url, _handle) = start_test_server().await;

    let client = reqwest::Client::new();
    let response = client
        .get(format!("{}/openapi.json", base_url))
        .send()
        .await
        .expect("Failed to GET /openapi.json");

    assert_eq!(response.status(), 200);

    let body: serde_json::Value = response.json().await.expect("Invalid JSON");
    assert_eq!(body["openapi"], "3.1.0");
    assert_eq!(body["info"]["title"], "LocalRouter API");
}

#[tokio::test]
async fn test_openapi_yaml_endpoint() {
    let (base_url, _handle) = start_test_server().await;

    let client = reqwest::Client::new();
    let response = client
        .get(format!("{}/openapi.yaml", base_url))
        .send()
        .await
        .expect("Failed to GET /openapi.yaml");

    assert_eq!(response.status(), 200);
    let body = response.text().await.expect("Failed to read body");

    assert!(body.contains("openapi: 3.1.0"));
    assert!(body.contains("title: LocalRouter API"));
}

#[tokio::test]
async fn test_models_endpoint_requires_auth() {
    let (base_url, _handle) = start_test_server().await;

    let client = reqwest::Client::new();
    let response = client
        .get(format!("{}/models", base_url))
        .send()
        .await
        .expect("Failed to GET /models");

    // Should require authentication
    assert_eq!(response.status(), 401);
}

#[tokio::test]
async fn test_models_endpoint_with_v1_prefix_requires_auth() {
    let (base_url, _handle) = start_test_server().await;

    let client = reqwest::Client::new();
    let response = client
        .get(format!("{}/v1/models", base_url))
        .send()
        .await
        .expect("Failed to GET /v1/models");

    // Should require authentication
    assert_eq!(response.status(), 401);
}

#[tokio::test]
async fn test_mcp_individual_server_requires_auth() {
    let (base_url, _handle) = start_test_server().await;

    let client = reqwest::Client::new();
    let response = client
        .post(format!("{}/mcp/test-server", base_url))
        .json(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/list",
            "params": {}
        }))
        .send()
        .await
        .expect("Failed to POST /mcp/test-server");

    // Should require authentication
    assert_eq!(response.status(), 401);
}

#[tokio::test]
async fn test_oauth_token_endpoint() {
    let (base_url, _handle) = start_test_server().await;

    let client = reqwest::Client::new();
    let response = client
        .post(format!("{}/oauth/token", base_url))
        .json(&serde_json::json!({
            "grant_type": "client_credentials",
            "client_id": "test",
            "client_secret": "test"
        }))
        .send()
        .await
        .expect("Failed to POST /oauth/token");

    // Should return 401 for invalid credentials (not 500)
    assert!(response.status() == 401 || response.status() == 400);
}

#[tokio::test]
async fn test_no_path_conflicts_get_vs_post_root() {
    let (base_url, _handle) = start_test_server().await;

    let client = reqwest::Client::new();

    // GET / should return documentation
    let get_response = client.get(&base_url).send().await.expect("Failed to GET /");
    assert_eq!(get_response.status(), 200);
    let get_body = get_response.text().await.expect("Failed to read body");
    assert!(get_body.contains("LocalRouter"));

    // POST / should route to MCP gateway (with auth error)
    let post_response = client
        .post(&base_url)
        .json(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/list",
            "params": {}
        }))
        .send()
        .await
        .expect("Failed to POST /");
    assert_eq!(post_response.status(), 401); // Requires auth, proving it routes to MCP
}

#[tokio::test]
async fn test_all_expected_endpoints_exist() {
    let (base_url, _handle) = start_test_server().await;

    let client = reqwest::Client::new();

    // System endpoints
    let endpoints = vec![
        ("GET", "/"),
        ("GET", "/health"),
        ("GET", "/openapi.json"),
        ("GET", "/openapi.yaml"),
        // OpenAI endpoints (all require auth, will get 401)
        ("GET", "/models"),
        ("GET", "/v1/models"),
        ("POST", "/chat/completions"),
        ("POST", "/v1/chat/completions"),
        ("POST", "/completions"),
        ("POST", "/v1/completions"),
        // MCP endpoints (require auth, will get 401)
        ("POST", "/"),
        ("POST", "/mcp/test"),
        // OAuth endpoint
        ("POST", "/oauth/token"),
    ];

    for (method, path) in endpoints {
        let url = format!("{}{}", base_url, path);
        let response = match method {
            "GET" => client.get(&url).send().await,
            "POST" => client.post(&url).json(&serde_json::json!({})).send().await,
            _ => panic!("Unsupported method: {}", method),
        }
        .expect(&format!("Failed to {} {}", method, path));

        // Should not get 404 (endpoint exists)
        assert_ne!(
            response.status(),
            404,
            "{} {} returned 404 - endpoint doesn't exist",
            method,
            path
        );
    }
}

#[tokio::test]
async fn test_deprecated_endpoints_removed() {
    let (base_url, _handle) = start_test_server().await;

    let client = reqwest::Client::new();

    // The old /mcp/health endpoint should be removed
    // Now /mcp/health would match /mcp/:server_id pattern, but:
    // 1. It requires auth (all /mcp/* routes require auth)
    // 2. GET method is not supported (only POST is defined for /mcp/:server_id)

    // Test without auth - should require authentication
    let response_no_auth = client
        .get(format!("{}/mcp/health", base_url))
        .send()
        .await
        .expect("Failed to GET /mcp/health");

    // Auth middleware runs first, so we get 401
    assert_eq!(
        response_no_auth.status(),
        401,
        "/mcp/health requires auth (proving it's not a public health endpoint)"
    );

    // Test with auth using POST (the actual route method)
    // This will try to proxy to a server named "health" which doesn't exist
    let response_with_auth = client
        .post(format!("{}/mcp/health", base_url))
        .header("Authorization", "Bearer test-client-123")
        .json(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "ping"
        }))
        .send()
        .await
        .expect("Failed to POST /mcp/health");

    // Should get an error (not a health check response)
    // Could be 404 (server not found) or 500 (error processing)
    assert!(
        response_with_auth.status().is_client_error()
            || response_with_auth.status().is_server_error(),
        "/mcp/health with auth should return error (not health check), got: {}",
        response_with_auth.status()
    );
}

#[tokio::test]
async fn test_cors_headers() {
    let (base_url, _handle) = start_test_server().await;

    let client = reqwest::Client::new();
    let response = client
        .get(format!("{}/health", base_url))
        .send()
        .await
        .expect("Failed to GET /health");

    // CORS should be enabled
    let headers = response.headers();
    assert!(
        headers.contains_key("access-control-allow-origin"),
        "CORS headers should be present"
    );
}
