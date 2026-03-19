//! End-to-end integration test for Memory tool injection via the MCP gateway.
//!
//! Verifies that MemorySearch and MemoryRead tools appear in tools/list only
//! when `memory_enabled` is set, and are absent otherwise.

use localrouter::config::AppConfig;
use localrouter::config::ConfigManager;
use localrouter::mcp::gateway::{GatewayConfig, McpGateway};
use localrouter::mcp::protocol::JsonRpcRequest;
use localrouter::mcp::McpServerManager;
use localrouter::monitoring::metrics::MetricsCollector;
use localrouter::monitoring::storage::MetricsDatabase;
use localrouter::providers::registry::ProviderRegistry;
use localrouter::router::{RateLimiterManager, Router};
use serde_json::json;
use std::sync::Arc;
use tempfile::TempDir;

/// Helper to create a minimal test router
fn create_test_router() -> Arc<Router> {
    let config = AppConfig::default();
    let config_manager = Arc::new(ConfigManager::new(
        config,
        std::path::PathBuf::from("/tmp/test_memory_e2e_router.yaml"),
    ));

    let provider_registry = Arc::new(ProviderRegistry::new());
    let rate_limiter = Arc::new(RateLimiterManager::new(None));

    let metrics_db_path = std::env::temp_dir().join(format!(
        "test_memory_e2e_metrics_{}.db",
        uuid::Uuid::new_v4()
    ));
    let metrics_db = Arc::new(MetricsDatabase::new(metrics_db_path).unwrap());
    let metrics_collector = Arc::new(MetricsCollector::new(metrics_db));

    Arc::new(Router::new(
        config_manager,
        provider_registry,
        rate_limiter,
        metrics_collector,
        Arc::new(lr_router::FreeTierManager::new(None)),
    ))
}

/// Set up a gateway with the memory virtual server registered.
fn setup_gateway_with_memory() -> (Arc<McpGateway>, TempDir) {
    let temp_dir = TempDir::new().unwrap();
    let memory_service = Arc::new(lr_memory::MemoryService::new(
        lr_config::MemoryConfig::default(),
        temp_dir.path().to_path_buf(),
    ));

    let server_manager = Arc::new(McpServerManager::new());
    let router = create_test_router();
    let gateway = McpGateway::new(server_manager, GatewayConfig::default(), router);
    gateway.register_virtual_server(Arc::new(
        lr_mcp::gateway::virtual_memory::MemoryVirtualServer::new(memory_service),
    ));

    (Arc::new(gateway), temp_dir)
}

/// Helper to extract tool names from a tools/list response
fn extract_tool_names(response: &localrouter::mcp::protocol::JsonRpcResponse) -> Vec<String> {
    let result = response.result.as_ref().expect("should have result");
    let tools = result["tools"].as_array().expect("should have tools array");
    tools
        .iter()
        .filter_map(|t| t["name"].as_str().map(|s| s.to_string()))
        .collect()
}

/// Helper to call tools/list with a given memory_enabled setting
async fn tools_list(
    gateway: &McpGateway,
    client_id: &str,
    memory_enabled: Option<bool>,
) -> localrouter::mcp::protocol::JsonRpcResponse {
    let req = JsonRpcRequest::with_id(1, "tools/list".to_string(), Some(json!({})));
    gateway
        .handle_request_with_skills(
            client_id,
            None,
            vec![],
            vec![],
            lr_config::McpPermissions::default(),
            lr_config::SkillsPermissions::default(),
            "Test Client".to_string(),
            lr_config::PermissionState::Off,
            lr_config::PermissionState::Off,
            None,
            None,
            lr_config::PermissionState::default(),
            lr_config::PermissionState::default(),
            memory_enabled,
            req,
            None, // monitor_session_id
        )
        .await
        .expect("tools/list should succeed")
}

/// Memory tools must be absent when memory_enabled is None (default).
#[tokio::test]
async fn test_memory_tools_absent_when_not_enabled() {
    let (gateway, _temp_dir) = setup_gateway_with_memory();

    let tool_names = extract_tool_names(&tools_list(&gateway, "client-no-mem", None).await);

    assert!(
        !tool_names.iter().any(|n| n.contains("Memory")),
        "Memory tools should not appear when memory_enabled is None. Found: {:?}",
        tool_names
    );
}

/// Memory tools must be absent when memory_enabled is explicitly false.
#[tokio::test]
async fn test_memory_tools_absent_when_disabled() {
    let (gateway, _temp_dir) = setup_gateway_with_memory();

    let tool_names =
        extract_tool_names(&tools_list(&gateway, "client-disabled", Some(false)).await);

    assert!(
        !tool_names.iter().any(|n| n.contains("Memory")),
        "Memory tools should not appear when memory_enabled is false. Found: {:?}",
        tool_names
    );
}

/// Memory tools must be present when memory_enabled is true.
#[tokio::test]
async fn test_memory_tools_present_when_enabled() {
    let (gateway, _temp_dir) = setup_gateway_with_memory();

    let tool_names = extract_tool_names(&tools_list(&gateway, "client-mem", Some(true)).await);

    assert!(
        tool_names.iter().any(|n| n.contains("Search")),
        "MemorySearch tool should appear when memory_enabled is true. Found: {:?}",
        tool_names
    );
    assert!(
        tool_names.iter().any(|n| n.contains("Read")),
        "MemoryRead tool should appear when memory_enabled is true. Found: {:?}",
        tool_names
    );
}

/// Memory tools must survive the tools/list cache (second call must still include them).
#[tokio::test]
async fn test_memory_tools_present_after_cache_hit() {
    let (gateway, _temp_dir) = setup_gateway_with_memory();
    let client_id = "cache-test-mem";

    // First call: populates cache
    let names1 = extract_tool_names(&tools_list(&gateway, client_id, Some(true)).await);
    assert!(
        names1.iter().any(|n| n.contains("Search")),
        "First call should include memory tools. Found: {:?}",
        names1
    );

    // Second call: should hit cache but still include memory tools
    let names2 = extract_tool_names(&tools_list(&gateway, client_id, Some(true)).await);
    assert!(
        names2.iter().any(|n| n.contains("Search")),
        "Cached tools/list must still include memory tools. Found: {:?}",
        names2
    );
}

/// Toggling memory_enabled across requests must update the tools list dynamically.
#[tokio::test]
async fn test_memory_tools_toggle_dynamic() {
    let (gateway, _temp_dir) = setup_gateway_with_memory();
    let client_id = "toggle-client";

    // Start with enabled
    let names_on = extract_tool_names(&tools_list(&gateway, client_id, Some(true)).await);
    assert!(
        names_on.iter().any(|n| n.contains("Search")),
        "Tools should be present when enabled. Found: {:?}",
        names_on
    );

    // Disable
    let names_off = extract_tool_names(&tools_list(&gateway, client_id, Some(false)).await);
    assert!(
        !names_off.iter().any(|n| n.contains("Memory")),
        "Tools should disappear when disabled. Found: {:?}",
        names_off
    );

    // Re-enable
    let names_on2 = extract_tool_names(&tools_list(&gateway, client_id, Some(true)).await);
    assert!(
        names_on2.iter().any(|n| n.contains("Search")),
        "Tools should reappear when re-enabled. Found: {:?}",
        names_on2
    );
}

/// Without the memory virtual server registered, memory_enabled has no effect.
#[tokio::test]
async fn test_no_memory_tools_without_virtual_server() {
    let server_manager = Arc::new(McpServerManager::new());
    let router = create_test_router();
    let gateway = Arc::new(McpGateway::new(
        server_manager,
        GatewayConfig::default(),
        router,
    ));

    let tool_names = extract_tool_names(&tools_list(&gateway, "no-vs-client", Some(true)).await);

    assert!(
        !tool_names.iter().any(|n| n.contains("Memory")),
        "Memory tools should not appear without the virtual server registered. Found: {:?}",
        tool_names
    );
}
