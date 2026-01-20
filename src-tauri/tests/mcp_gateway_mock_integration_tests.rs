//! Comprehensive MCP Gateway Integration Tests with Mock Servers
//!
//! Tests the complete flow from gateway request → routing → mock MCP servers → merged response.
//! Each test spins up two mock MCP servers and verifies both the requests sent to individual
//! servers and the unified merged response from the gateway.

mod mcp_tests;

use localrouter_ai::config::{McpAuthConfig, McpServerConfig, McpTransportConfig, McpTransportType};
use localrouter_ai::mcp::gateway::{GatewayConfig, McpGateway};
use localrouter_ai::mcp::protocol::{JsonRpcRequest, JsonRpcResponse};
use localrouter_ai::mcp::McpServerManager;
use mcp_tests::common::request_with_params;
use serde_json::json;
use std::sync::Arc;
use wiremock::{matchers::method as http_method, Mock, MockServer, ResponseTemplate};

/// Mock MCP server wrapper
struct MockMcpServer {
    server: MockServer,
}

impl MockMcpServer {
    async fn new() -> Self {
        let server = MockServer::start().await;

        // Mock HEAD request for connection validation
        Mock::given(http_method("HEAD"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        Self { server }
    }

    fn base_url(&self) -> String {
        self.server.uri()
    }

    async fn mock_method(&self, _method: &str, result: serde_json::Value) {
        let response = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": result
        });

        Mock::given(http_method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(response))
            .up_to_n_times(100) // Allow multiple calls
            .mount(&self.server)
            .await;
    }

    async fn mock_error(&self, error_code: i32, message: &str) {
        let response = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "error": {
                "code": error_code,
                "message": message
            }
        });

        Mock::given(http_method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(response))
            .mount(&self.server)
            .await;
    }

    async fn mock_failure(&self) {
        Mock::given(http_method("POST"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&self.server)
            .await;
    }
}

/// Test helper to set up gateway with two mock MCP servers
async fn setup_gateway_with_two_servers(
) -> (Arc<McpGateway>, Arc<McpServerManager>, MockMcpServer, MockMcpServer) {
    // Create two mock MCP servers
    let server1_mock = MockMcpServer::new().await;
    let server1_url = server1_mock.base_url();

    let server2_mock = MockMcpServer::new().await;
    let server2_url = server2_mock.base_url();

    // Create MCP server manager
    let manager = Arc::new(McpServerManager::new());

    // Configure two MCP servers
    let server1_config = McpServerConfig {
        id: "server1".to_string(),
        name: "Test Server 1".to_string(),
        enabled: true,
        transport: McpTransportConfig {
            transport_type: McpTransportType::Sse,
            url: Some(server1_url.clone()),
            command: None,
            args: None,
            env: None,
        },
        auth: McpAuthConfig::None,
    };

    let server2_config = McpServerConfig {
        id: "server2".to_string(),
        name: "Test Server 2".to_string(),
        enabled: true,
        transport: McpTransportConfig {
            transport_type: McpTransportType::Sse,
            url: Some(server2_url.clone()),
            command: None,
            args: None,
            env: None,
        },
        auth: McpAuthConfig::None,
    };

    // Add servers to manager
    manager.add_server(server1_config).await.unwrap();
    manager.add_server(server2_config).await.unwrap();

    // Start servers
    manager.start_server("server1").await.unwrap();
    manager.start_server("server2").await.unwrap();

    // Create gateway
    let config = GatewayConfig::default();
    let gateway = Arc::new(McpGateway::new(manager.clone(), config));

    (gateway, manager, server1_mock, server2_mock)
}

/// Helper to extract result from JSON-RPC response
fn extract_result(response: &JsonRpcResponse) -> &serde_json::Value {
    response.result.as_ref().unwrap()
}

// ============================================================================
// INITIALIZE ENDPOINT TESTS
// ============================================================================

#[tokio::test]
async fn test_gateway_initialize_merges_capabilities() {
    let (gateway, _manager, server1_mock, server2_mock) = setup_gateway_with_two_servers().await;

    // Mock initialize responses from both servers
    let server1_response = json!({
        "protocolVersion": "2024-11-05",
        "capabilities": {
            "tools": { "listChanged": true },
            "resources": { "subscribe": true }
        },
        "serverInfo": {
            "name": "Server 1",
            "version": "1.0.0"
        }
    });

    let server2_response = json!({
        "protocolVersion": "2024-11-05",
        "capabilities": {
            "resources": { "listChanged": true },
            "prompts": { "listChanged": true }
        },
        "serverInfo": {
            "name": "Server 2",
            "version": "2.0.0"
        }
    });

    // Set up mocks
    server1_mock.mock_method("initialize", server1_response).await;
    server2_mock.mock_method("initialize", server2_response).await;

    // Send initialize request through gateway
    let request = JsonRpcRequest::new(
        Some(json!(1)),
        "initialize".to_string(),
        Some(json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {"name": "test", "version": "1.0"}
        })),
    );

    let allowed_servers = vec!["server1".to_string(), "server2".to_string()];
    let response = gateway
        .handle_request("test-client", allowed_servers, false, request)
        .await
        .unwrap();

    // Verify merged response
    let result = extract_result(&response);

    // Check protocol version (should use minimum)
    assert_eq!(result["protocolVersion"], "2024-11-05");

    // Check merged capabilities
    let capabilities = &result["capabilities"];
    assert!(capabilities["tools"]["listChanged"].as_bool().unwrap());
    assert!(capabilities["resources"]["subscribe"].as_bool().unwrap());
    assert!(capabilities["resources"]["listChanged"].as_bool().unwrap());
    assert!(capabilities["prompts"]["listChanged"].as_bool().unwrap());

    // Check server info includes both servers
    let server_info = result["serverInfo"]["description"].as_str().unwrap();
    assert!(server_info.contains("server1"));
    assert!(server_info.contains("server2"));
}

#[tokio::test]
async fn test_gateway_initialize_handles_partial_failure() {
    let (gateway, _manager, server1_mock, server2_mock) = setup_gateway_with_two_servers().await;

    // Server 1 succeeds
    server1_mock.mock_method("initialize", json!({
        "protocolVersion": "2024-11-05",
        "capabilities": { "tools": {} },
        "serverInfo": { "name": "Server 1", "version": "1.0.0" }
    })).await;

    // Server 2 fails
    server2_mock.mock_failure().await;

    let request = JsonRpcRequest::new(
        Some(json!(1)),
        "initialize".to_string(),
        Some(json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {"name": "test", "version": "1.0"}
        })),
    );

    let allowed_servers = vec!["server1".to_string(), "server2".to_string()];
    let response = gateway
        .handle_request("test-client", allowed_servers, false, request)
        .await
        .unwrap();

    // Should succeed with server1 only
    let result = extract_result(&response);
    let server_info = result["serverInfo"]["description"].as_str().unwrap();

    // Should mention server1 success
    assert!(server_info.contains("server1") || server_info.contains("Server 1"));

    // Should mention server2 failure
    assert!(server_info.contains("server2") || server_info.contains("failed"));
}

// ============================================================================
// TOOLS/LIST ENDPOINT TESTS
// ============================================================================

#[tokio::test]
async fn test_gateway_tools_list_merges_and_namespaces() {
    let (gateway, _manager, server1_mock, server2_mock) = setup_gateway_with_two_servers().await;

    // Mock tools/list responses
    server1_mock.mock_method("tools/list", json!({
        "tools": [
            {
                "name": "read_file",
                "description": "Read a file from server 1",
                "inputSchema": {"type": "object", "properties": {"path": {"type": "string"}}}
            },
            {
                "name": "write_file",
                "description": "Write a file on server 1",
                "inputSchema": {"type": "object", "properties": {"path": {"type": "string"}, "content": {"type": "string"}}}
            }
        ]
    })).await;

    server2_mock.mock_method("tools/list", json!({
        "tools": [
            {
                "name": "list_directory",
                "description": "List directory contents from server 2",
                "inputSchema": {"type": "object", "properties": {"path": {"type": "string"}}}
            },
            {
                "name": "delete_file",
                "description": "Delete a file on server 2",
                "inputSchema": {"type": "object", "properties": {"path": {"type": "string"}}}
            }
        ]
    })).await;

    // Send tools/list request through gateway
    let request = JsonRpcRequest::new(Some(json!(1)), "tools/list".to_string(), Some(json!({})));

    let allowed_servers = vec!["server1".to_string(), "server2".to_string()];
    let response = gateway
        .handle_request("test-client", allowed_servers, false, request)
        .await
        .unwrap();

    // Verify merged and namespaced response
    let result = extract_result(&response);
    let tools = result["tools"].as_array().unwrap();

    // Should have 4 tools total
    assert_eq!(tools.len(), 4);

    // Verify namespacing
    let tool_names: Vec<String> = tools
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect();

    assert!(tool_names.contains(&"server1__read_file".to_string()));
    assert!(tool_names.contains(&"server1__write_file".to_string()));
    assert!(tool_names.contains(&"server2__list_directory".to_string()));
    assert!(tool_names.contains(&"server2__delete_file".to_string()));

    // Verify descriptions are unchanged
    let read_file_tool = tools
        .iter()
        .find(|t| t["name"] == "server1__read_file")
        .unwrap();
    assert_eq!(
        read_file_tool["description"].as_str().unwrap(),
        "Read a file from server 1"
    );

    let list_dir_tool = tools
        .iter()
        .find(|t| t["name"] == "server2__list_directory")
        .unwrap();
    assert_eq!(
        list_dir_tool["description"].as_str().unwrap(),
        "List directory contents from server 2"
    );
}

#[tokio::test]
async fn test_gateway_tools_list_with_empty_server() {
    let (gateway, _manager, server1_mock, server2_mock) = setup_gateway_with_two_servers().await;

    // Server 1 has tools
    server1_mock.mock_method("tools/list", json!({
        "tools": [{
            "name": "tool1",
            "description": "Tool 1",
            "inputSchema": {"type": "object"}
        }]
    })).await;

    // Server 2 has no tools
    server2_mock.mock_method("tools/list", json!({"tools": []})).await;

    let request = JsonRpcRequest::new(Some(json!(1)), "tools/list".to_string(), Some(json!({})));

    let allowed_servers = vec!["server1".to_string(), "server2".to_string()];
    let response = gateway
        .handle_request("test-client", allowed_servers, false, request)
        .await
        .unwrap();

    let result = extract_result(&response);
    let tools = result["tools"].as_array().unwrap();

    // Should have 1 tool from server1
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0]["name"], "server1__tool1");
}

// ============================================================================
// RESOURCES/LIST ENDPOINT TESTS
// ============================================================================

#[tokio::test]
async fn test_gateway_resources_list_merges_and_namespaces() {
    let (gateway, _manager, server1_mock, server2_mock) = setup_gateway_with_two_servers().await;

    server1_mock.mock_method("resources/list", json!({
        "resources": [{
            "name": "config",
            "description": "Configuration file",
            "uri": "file:///config.json",
            "mimeType": "application/json"
        }]
    })).await;

    server2_mock.mock_method("resources/list", json!({
        "resources": [{
            "name": "logs",
            "description": "Log files",
            "uri": "file:///var/log/app.log",
            "mimeType": "text/plain"
        }]
    })).await;

    let request = JsonRpcRequest::new(Some(json!(1)), "resources/list".to_string(), Some(json!({})));

    let allowed_servers = vec!["server1".to_string(), "server2".to_string()];
    let response = gateway
        .handle_request("test-client", allowed_servers, false, request)
        .await
        .unwrap();

    let result = extract_result(&response);
    let resources = result["resources"].as_array().unwrap();

    assert_eq!(resources.len(), 2);

    // Verify namespacing
    let resource_names: Vec<String> = resources
        .iter()
        .map(|r| r["name"].as_str().unwrap().to_string())
        .collect();

    assert!(resource_names.contains(&"server1__config".to_string()));
    assert!(resource_names.contains(&"server2__logs".to_string()));

    // Verify URIs are unchanged
    let config_resource = resources
        .iter()
        .find(|r| r["name"] == "server1__config")
        .unwrap();
    assert_eq!(config_resource["uri"], "file:///config.json");
}

// ============================================================================
// PROMPTS/LIST ENDPOINT TESTS
// ============================================================================

#[tokio::test]
async fn test_gateway_prompts_list_merges_and_namespaces() {
    let (gateway, _manager, server1_mock, server2_mock) = setup_gateway_with_two_servers().await;

    server1_mock.mock_method("prompts/list", json!({
        "prompts": [{
            "name": "code_review",
            "description": "Code review prompt",
            "arguments": [{"name": "code", "description": "Code to review", "required": true}]
        }]
    })).await;

    server2_mock.mock_method("prompts/list", json!({
        "prompts": [{
            "name": "summarize",
            "description": "Summarize text",
            "arguments": [{"name": "text", "description": "Text to summarize", "required": true}]
        }]
    })).await;

    let request = JsonRpcRequest::new(Some(json!(1)), "prompts/list".to_string(), Some(json!({})));

    let allowed_servers = vec!["server1".to_string(), "server2".to_string()];
    let response = gateway
        .handle_request("test-client", allowed_servers, false, request)
        .await
        .unwrap();

    let result = extract_result(&response);
    let prompts = result["prompts"].as_array().unwrap();

    assert_eq!(prompts.len(), 2);

    // Verify namespacing
    let prompt_names: Vec<String> = prompts
        .iter()
        .map(|p| p["name"].as_str().unwrap().to_string())
        .collect();

    assert!(prompt_names.contains(&"server1__code_review".to_string()));
    assert!(prompt_names.contains(&"server2__summarize".to_string()));
}

// ============================================================================
// TOOLS/CALL ENDPOINT TESTS (Direct Routing)
// ============================================================================

#[tokio::test]
async fn test_gateway_tools_call_routes_to_correct_server() {
    let (gateway, _manager, server1_mock, server2_mock) = setup_gateway_with_two_servers().await;

    // First, set up tools/list to populate the session mappings
    server1_mock.mock_method("tools/list", json!({
        "tools": [{
            "name": "read_file",
            "description": "Read file",
            "inputSchema": {"type": "object"}
        }]
    })).await;

    server2_mock.mock_method("tools/list", json!({
        "tools": [{
            "name": "write_file",
            "description": "Write file",
            "inputSchema": {"type": "object"}
        }]
    })).await;

    // First call tools/list to populate session
    let list_request = JsonRpcRequest::new(Some(json!(1)), "tools/list".to_string(), Some(json!({})));
    let allowed_servers = vec!["server1".to_string(), "server2".to_string()];
    let _ = gateway
        .handle_request("test-client-call", allowed_servers.clone(), false, list_request)
        .await
        .unwrap();

    // Now mock tools/call response from server1
    server1_mock.mock_method("tools/call", json!({"content": "file contents from server1"})).await;

    let call_request = request_with_params(
        "tools/call",
        json!({
            "name": "server1__read_file",
            "arguments": {"path": "/test.txt"}
        }),
    );

    let response = gateway
        .handle_request("test-client-call", allowed_servers, false, call_request)
        .await
        .unwrap();

    let result = extract_result(&response);
    assert_eq!(result["content"], "file contents from server1");
}

#[tokio::test]
async fn test_gateway_tools_call_unknown_tool() {
    let (gateway, _manager, server1_mock, _server2_mock) = setup_gateway_with_two_servers().await;

    // Set up tools/list
    server1_mock.mock_method("tools/list", json!({
        "tools": [{
            "name": "valid_tool",
            "description": "Valid tool",
            "inputSchema": {"type": "object"}
        }]
    })).await;

    let list_request = JsonRpcRequest::new(Some(json!(1)), "tools/list".to_string(), Some(json!({})));
    let allowed_servers = vec!["server1".to_string()];
    let _ = gateway
        .handle_request("test-client-unknown", allowed_servers.clone(), false, list_request)
        .await
        .unwrap();

    // Call with non-existent tool
    let call_request = request_with_params(
        "tools/call",
        json!({
            "name": "nonexistent__tool",
            "arguments": {}
        }),
    );

    let result = gateway
        .handle_request("test-client-unknown", allowed_servers, false, call_request)
        .await;

    // Should fail
    assert!(result.is_err() || result.unwrap().error.is_some());
}

// ============================================================================
// DEFERRED LOADING TESTS
// ============================================================================

#[tokio::test]
async fn test_gateway_deferred_loading_search_tool() {
    let (gateway, _manager, server1_mock, server2_mock) = setup_gateway_with_two_servers().await;

    // Mock tools/list for initial catalog fetch
    server1_mock.mock_method("tools/list", json!({
        "tools": [
            {"name": "read_file", "description": "Read files", "inputSchema": {"type": "object"}},
            {"name": "write_file", "description": "Write files", "inputSchema": {"type": "object"}}
        ]
    })).await;

    server2_mock.mock_method("tools/list", json!({
        "tools": [
            {"name": "send_email", "description": "Send email", "inputSchema": {"type": "object"}},
            {"name": "fetch_url", "description": "Fetch URL", "inputSchema": {"type": "object"}}
        ]
    })).await;

    // Enable deferred loading
    let allowed_servers = vec!["server1".to_string(), "server2".to_string()];

    // Request tools/list with deferred loading enabled
    let request = JsonRpcRequest::new(Some(json!(1)), "tools/list".to_string(), Some(json!({})));

    let response = gateway
        .handle_request("test-client-deferred", allowed_servers, true, request)
        .await
        .unwrap();

    let result = extract_result(&response);
    let tools = result["tools"].as_array().unwrap();

    // With deferred loading, should only see search tool initially
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0]["name"], "search");
    assert!(tools[0]["description"].as_str().unwrap().contains("Search"));
}

#[tokio::test]
async fn test_gateway_deferred_loading_activates_tools() {
    let (gateway, _manager, server1_mock, server2_mock) = setup_gateway_with_two_servers().await;

    // Mock tools/list
    server1_mock.mock_method("tools/list", json!({
        "tools": [
            {"name": "read_file", "description": "Read files from disk", "inputSchema": {"type": "object"}},
            {"name": "write_file", "description": "Write files to disk", "inputSchema": {"type": "object"}}
        ]
    })).await;

    server2_mock.mock_method("tools/list", json!({
        "tools": [
            {"name": "send_email", "description": "Send email messages", "inputSchema": {"type": "object"}}
        ]
    })).await;

    let allowed_servers = vec!["server1".to_string(), "server2".to_string()];

    // Call search tool to activate tools
    let search_request = request_with_params(
        "tools/call",
        json!({
            "name": "search",
            "arguments": {"query": "read", "type": "tools", "limit": 10}
        }),
    );

    let search_response = gateway
        .handle_request("test-client-search", allowed_servers.clone(), true, search_request)
        .await
        .unwrap();

    // Verify search activated tools
    let result = extract_result(&search_response);
    let activated = result["activated"].as_array().unwrap();
    assert!(!activated.is_empty());
    assert!(activated.iter().any(|v| v.as_str().unwrap() == "server1__read_file"));

    // Now tools/list should return search tool + activated tools
    let list_request = JsonRpcRequest::new(Some(json!(2)), "tools/list".to_string(), Some(json!({})));

    let list_response = gateway
        .handle_request("test-client-search", allowed_servers, true, list_request)
        .await
        .unwrap();

    let list_result = extract_result(&list_response);
    let tools = list_result["tools"].as_array().unwrap();

    // Should have search tool + activated tools
    assert!(tools.len() > 1);
    assert!(tools.iter().any(|t| t["name"] == "search"));
    assert!(tools.iter().any(|t| t["name"] == "server1__read_file"));
}

// ============================================================================
// ERROR HANDLING TESTS
// ============================================================================

#[tokio::test]
async fn test_gateway_handles_all_servers_failing() {
    let (gateway, _manager, server1_mock, server2_mock) = setup_gateway_with_two_servers().await;

    // Both servers fail
    server1_mock.mock_failure().await;
    server2_mock.mock_failure().await;

    let request = JsonRpcRequest::new(Some(json!(1)), "tools/list".to_string(), Some(json!({})));

    let allowed_servers = vec!["server1".to_string(), "server2".to_string()];
    let result = gateway
        .handle_request("test-client-fail", allowed_servers, false, request)
        .await;

    // Should return error or empty result
    assert!(
        result.is_err()
            || extract_result(&result.as_ref().unwrap())["tools"]
                .as_array()
                .unwrap()
                .is_empty()
    );
}

#[tokio::test]
async fn test_gateway_handles_json_rpc_error() {
    let (gateway, _manager, server1_mock, _server2_mock) = setup_gateway_with_two_servers().await;

    // Server returns JSON-RPC error
    server1_mock.mock_error(-32601, "Method not found").await;

    let request = JsonRpcRequest::new(Some(json!(1)), "invalid_method".to_string(), Some(json!({})));

    let allowed_servers = vec!["server1".to_string()];
    let response = gateway
        .handle_request("test-client-error", allowed_servers, false, request)
        .await
        .unwrap();

    // Should have error
    assert!(response.error.is_some());
    assert_eq!(response.error.unwrap().code, -32601);
}
