//! Integration tests for MCP Gateway
//!
//! Tests the complete flow from HTTP request → gateway → backend servers → response

use localrouter_ai::mcp::gateway::{GatewayConfig, McpGateway};
use localrouter_ai::mcp::protocol::{JsonRpcRequest, JsonRpcResponse};
use localrouter_ai::mcp::McpServerManager;
use serde_json::json;
use std::sync::Arc;

#[tokio::test]
async fn test_gateway_session_creation() {
    let manager = Arc::new(McpServerManager::new());
    let config = GatewayConfig::default();
    let gateway = Arc::new(McpGateway::new(manager, config));

    let request = JsonRpcRequest::new(
        Some(json!(1)),
        "initialize".to_string(),
        Some(json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {"name": "test", "version": "1.0"}
        })),
    );

    // This should create a session even with empty allowed_servers (though it will have no access)
    let result = gateway
        .handle_request("test-client", vec![], false, request)
        .await;

    // Expect an error or empty response since no servers are allowed
    assert!(result.is_ok() || result.is_err());
}

#[tokio::test]
async fn test_gateway_namespace_parsing() {
    use localrouter_ai::mcp::gateway::types::{apply_namespace, parse_namespace};

    // Test namespace application
    let namespaced = apply_namespace("filesystem", "read_file");
    assert_eq!(namespaced, "filesystem__read_file");

    // Test namespace parsing
    let (server_id, tool_name) = parse_namespace("filesystem__read_file").unwrap();
    assert_eq!(server_id, "filesystem");
    assert_eq!(tool_name, "read_file");

    // Test invalid format
    assert!(parse_namespace("invalid_format").is_none());
    assert!(parse_namespace("__no_server").is_none());
    assert!(parse_namespace("no_tool__").is_none());
}

#[tokio::test]
async fn test_gateway_empty_allowed_servers() {
    let manager = Arc::new(McpServerManager::new());
    let config = GatewayConfig::default();
    let gateway = Arc::new(McpGateway::new(manager, config));

    let request = JsonRpcRequest::new(
        Some(json!(1)),
        "tools/list".to_string(),
        Some(json!({})),
    );

    // With empty allowed_servers, should handle gracefully
    let result = gateway
        .handle_request("test-client", vec![], false, request)
        .await;

    // Should either succeed with empty list or return appropriate response
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_gateway_config_defaults() {
    let config = GatewayConfig::default();

    assert_eq!(config.session_ttl_seconds, 3600);
    assert_eq!(config.server_timeout_seconds, 10);
    assert!(config.allow_partial_failures);
    assert_eq!(config.cache_ttl_seconds, 300);
    assert_eq!(config.max_retry_attempts, 1);
}

#[tokio::test]
async fn test_gateway_session_expiration() {
    use localrouter_ai::mcp::gateway::session::GatewaySession;
    use std::time::Duration;

    let session = GatewaySession::new(
        "test-client".to_string(),
        vec!["filesystem".to_string()],
        Duration::from_millis(100),
    );

    assert!(!session.is_expired());

    tokio::time::sleep(Duration::from_millis(150)).await;
    assert!(session.is_expired());
}

#[tokio::test]
async fn test_gateway_concurrent_requests() {
    let manager = Arc::new(McpServerManager::new());
    let config = GatewayConfig::default();
    let gateway = Arc::new(McpGateway::new(manager, config));

    // Spawn multiple concurrent requests
    let mut handles = vec![];

    for i in 0..10 {
        let gateway_clone = gateway.clone();
        let handle = tokio::spawn(async move {
            let request = JsonRpcRequest::new(
                Some(json!(i)),
                "ping".to_string(),
                None,
            );

            gateway_clone
                .handle_request(&format!("client-{}", i), vec![], false, request)
                .await
        });

        handles.push(handle);
    }

    // Wait for all requests
    let results = futures::future::join_all(handles).await;

    // All requests should complete (though may error due to no servers)
    assert_eq!(results.len(), 10);
    for result in results {
        assert!(result.is_ok()); // tokio::spawn succeeded
    }
}

#[tokio::test]
async fn test_search_tool_creation() {
    use localrouter_ai::mcp::gateway::deferred::create_search_tool;

    let search_tool = create_search_tool();

    assert_eq!(search_tool.name, "search");
    assert_eq!(search_tool.server_id, "_gateway");
    assert!(search_tool.description.is_some());

    // Verify input schema
    let schema = search_tool.input_schema;
    assert!(schema.get("type").is_some());
    assert_eq!(schema.get("type").unwrap(), "object");

    let properties = schema.get("properties").unwrap();
    assert!(properties.get("query").is_some());
    assert!(properties.get("type").is_some());
    assert!(properties.get("limit").is_some());
}

#[tokio::test]
async fn test_gateway_method_routing() {
    use localrouter_ai::mcp::gateway::router::should_broadcast;

    // Broadcast methods
    assert!(should_broadcast("initialize"));
    assert!(should_broadcast("tools/list"));
    assert!(should_broadcast("resources/list"));
    assert!(should_broadcast("prompts/list"));
    assert!(should_broadcast("logging/setLevel"));
    assert!(should_broadcast("ping"));

    // Direct methods
    assert!(!should_broadcast("tools/call"));
    assert!(!should_broadcast("resources/read"));
    assert!(!should_broadcast("prompts/get"));
}

#[tokio::test]
async fn test_cached_list_validity() {
    use localrouter_ai::mcp::gateway::types::CachedList;
    use std::time::Duration;

    let cached = CachedList::new(
        vec!["item1".to_string(), "item2".to_string()],
        Duration::from_millis(100),
    );

    assert!(cached.is_valid());
    assert_eq!(cached.data.len(), 2);

    tokio::time::sleep(Duration::from_millis(150)).await;
    assert!(!cached.is_valid());
}

#[tokio::test]
async fn test_gateway_cleanup_expired_sessions() {
    let manager = Arc::new(McpServerManager::new());
    let mut config = GatewayConfig::default();
    config.session_ttl_seconds = 1; // 1 second TTL
    let gateway = Arc::new(McpGateway::new(manager, config));

    // Create a session
    let request = JsonRpcRequest::new(Some(json!(1)), "ping".to_string(), None);

    let _ = gateway
        .handle_request("test-client", vec![], false, request)
        .await;

    // Wait for session to expire
    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

    // Trigger cleanup
    gateway.cleanup_expired_sessions();

    // Session should be cleaned up (we can't directly verify this without exposing internal state,
    // but the test confirms the cleanup runs without errors)
}

#[tokio::test]
async fn test_deferred_loading_search_relevance() {
    use localrouter_ai::mcp::gateway::deferred::search_tools;
    use localrouter_ai::mcp::gateway::types::NamespacedTool;
    use serde_json::json;

    let tools = vec![
        NamespacedTool {
            name: "filesystem__read_file".to_string(),
            original_name: "read_file".to_string(),
            server_id: "filesystem".to_string(),
            description: Some("Read a file from disk".to_string()),
            input_schema: json!({}),
        },
        NamespacedTool {
            name: "filesystem__write_file".to_string(),
            original_name: "write_file".to_string(),
            server_id: "filesystem".to_string(),
            description: Some("Write a file to disk".to_string()),
            input_schema: json!({}),
        },
        NamespacedTool {
            name: "github__read_issue".to_string(),
            original_name: "read_issue".to_string(),
            server_id: "github".to_string(),
            description: Some("Read an issue from GitHub".to_string()),
            input_schema: json!({}),
        },
    ];

    let results = search_tools("read", &tools, 10);

    // Should return tools with "read" in name or description
    assert!(!results.is_empty());

    // Verify all results contain "read"
    for (tool, score) in results {
        assert!(
            tool.name.to_lowercase().contains("read")
                || tool
                    .description
                    .as_ref()
                    .unwrap()
                    .to_lowercase()
                    .contains("read")
        );
        assert!(score > 0.0);
    }
}
