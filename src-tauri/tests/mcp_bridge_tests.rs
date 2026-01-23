//! Integration tests for MCP Bridge Mode
//!
//! These tests verify that the STDIO bridge correctly forwards JSON-RPC requests
//! to the LocalRouter HTTP server and returns responses.

use chrono::Utc;
use localrouter_ai::config::{AppConfig, Client, McpServerAccess};
use localrouter_ai::mcp::bridge::StdioBridge;
use localrouter_ai::mcp::protocol::{JsonRpcRequest, JsonRpcResponse};
use serde_json::{json, Value};

/// Helper to create a test configuration
fn test_config() -> AppConfig {
    let mut config = AppConfig::default();
    config.clients = vec![
        Client {
            id: "test_client".to_string(),
            name: "Test Client".to_string(),
            enabled: true,
            allowed_llm_providers: vec![],
            mcp_server_access: McpServerAccess::Specific(vec!["filesystem".to_string(), "web".to_string()]),
            mcp_deferred_loading: false,
            created_at: Utc::now(),
            last_used: None,
            strategy_id: "default".to_string(),
            routing_config: None,
            roots: None,
            mcp_sampling_enabled: false,
            mcp_sampling_requires_approval: true,
            mcp_sampling_max_tokens: None,
            mcp_sampling_rate_limit: None,
        },
        Client {
            id: "disabled_client".to_string(),
            name: "Disabled Client".to_string(),
            enabled: false,
            allowed_llm_providers: vec![],
            mcp_server_access: McpServerAccess::Specific(vec!["github".to_string()]),
            mcp_deferred_loading: false,
            created_at: Utc::now(),
            last_used: None,
            strategy_id: "default".to_string(),
            routing_config: None,
            roots: None,
            mcp_sampling_enabled: false,
            mcp_sampling_requires_approval: true,
            mcp_sampling_max_tokens: None,
            mcp_sampling_rate_limit: None,
        },
        Client {
            id: "no_mcp_client".to_string(),
            name: "No MCP Client".to_string(),
            enabled: true,
            allowed_llm_providers: vec!["openai".to_string()],
            mcp_server_access: McpServerAccess::None,
            mcp_deferred_loading: false,
            created_at: Utc::now(),
            last_used: None,
            strategy_id: "default".to_string(),
            routing_config: None,
            roots: None,
            mcp_sampling_enabled: false,
            mcp_sampling_requires_approval: true,
            mcp_sampling_max_tokens: None,
            mcp_sampling_rate_limit: None,
        },
    ];
    config
}

/// Test JSON-RPC request parsing
#[test]
fn test_jsonrpc_request_parsing() {
    let json_str = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#;
    let request: JsonRpcRequest = serde_json::from_str(json_str).unwrap();

    assert_eq!(request.jsonrpc, "2.0");
    assert_eq!(request.id, Some(Value::from(1)));
    assert_eq!(request.method, "initialize");
    assert_eq!(request.params, Some(json!({})));
}

/// Test JSON-RPC response serialization
#[test]
fn test_jsonrpc_response_serialization() {
    let response = JsonRpcResponse {
        jsonrpc: "2.0".to_string(),
        id: Value::from(1),
        result: Some(json!({"status": "ok"})),
        error: None,
    };

    let json_str = serde_json::to_string(&response).unwrap();
    assert!(json_str.contains("\"jsonrpc\":\"2.0\""));
    assert!(json_str.contains("\"id\":1"));
    assert!(json_str.contains("\"result\""));
}

/// Test client resolution with explicit ID
#[test]
fn test_client_resolution_explicit() {
    let config = test_config();

    // Find existing enabled client
    let client = config
        .clients
        .iter()
        .find(|c| c.id == "test_client" && c.enabled)
        .unwrap();

    assert_eq!(client.id, "test_client");
    assert!(client.enabled);
    assert!(client.mcp_server_access.has_any_access());
}

/// Test client resolution with disabled client
#[test]
fn test_client_resolution_disabled() {
    let config = test_config();

    // Disabled client should not be usable
    let client = config
        .clients
        .iter()
        .find(|c| c.id == "disabled_client")
        .unwrap();

    assert!(!client.enabled);
}

/// Test client resolution with no MCP servers
#[test]
fn test_client_resolution_no_mcp() {
    let config = test_config();

    // Client with no MCP servers should not be usable for bridge mode
    let client = config
        .clients
        .iter()
        .find(|c| c.id == "no_mcp_client")
        .unwrap();

    assert!(client.enabled);
    assert!(!client.mcp_server_access.has_any_access());
}

/// Test auto-detection of first enabled client with MCP servers
#[test]
fn test_client_auto_detection() {
    let config = test_config();

    // Should find test_client (first enabled with MCP servers)
    let client = config
        .clients
        .iter()
        .find(|c| c.enabled && c.mcp_server_access.has_any_access())
        .unwrap();

    assert_eq!(client.id, "test_client");
}

/// Test JSON-RPC error response
#[test]
fn test_jsonrpc_error_response() {
    use localrouter_ai::mcp::protocol::JsonRpcError;

    let error_response = JsonRpcResponse {
        jsonrpc: "2.0".to_string(),
        id: Value::Null,
        result: None,
        error: Some(JsonRpcError {
            code: -32700,
            message: "Parse error".to_string(),
            data: None,
        }),
    };

    let json_str = serde_json::to_string(&error_response).unwrap();
    assert!(json_str.contains("\"error\""));
    assert!(json_str.contains("-32700"));
    assert!(json_str.contains("Parse error"));
}

/// Test malformed JSON handling
#[test]
fn test_malformed_json() {
    let json_str = r#"{"jsonrpc":"2.0","id":1,"method":"initialize"#; // Missing closing brace
    let result = serde_json::from_str::<JsonRpcRequest>(json_str);
    assert!(result.is_err());
}

/// Test empty mcp_server_access validation
#[test]
fn test_empty_mcp_servers_validation() {
    let config = test_config();

    // Client with no MCP servers should be found but not usable
    let no_mcp_client = config
        .clients
        .iter()
        .find(|c| c.id == "no_mcp_client")
        .unwrap();

    assert!(!no_mcp_client.mcp_server_access.has_any_access());
}

/// Test deferred loading flag
#[test]
fn test_deferred_loading_flag() {
    let mut config = test_config();
    config.clients[0].mcp_deferred_loading = true;

    let client = &config.clients[0];
    assert!(client.mcp_deferred_loading);
}

/// Test multiple clients configuration
#[test]
fn test_multiple_clients() {
    let config = test_config();

    assert_eq!(config.clients.len(), 3);

    // Count enabled clients
    let enabled_count = config.clients.iter().filter(|c| c.enabled).count();
    assert_eq!(enabled_count, 2);

    // Count clients with MCP servers
    let mcp_count = config
        .clients
        .iter()
        .filter(|c| c.mcp_server_access.has_any_access())
        .count();
    assert_eq!(mcp_count, 2);
}

// Note: Full end-to-end integration tests require a running LocalRouter HTTP server
// These would be added in a separate test suite with proper server setup/teardown

#[cfg(test)]
mod stdio_tests {
    use super::*;

    /// Test that we can serialize and deserialize a full request-response cycle
    #[test]
    fn test_request_response_cycle() {
        // Create a request
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(Value::from(1)),
            method: "tools/list".to_string(),
            params: None,
        };

        // Serialize to JSON (what bridge would send to HTTP server)
        let request_json = serde_json::to_string(&request).unwrap();

        // Deserialize (what HTTP server would receive)
        let received_request: JsonRpcRequest = serde_json::from_str(&request_json).unwrap();
        assert_eq!(received_request.method, "tools/list");

        // Create a response
        let response = JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id: Value::from(1),
            result: Some(json!({"tools": []})),
            error: None,
        };

        // Serialize to JSON (what HTTP server would send back)
        let response_json = serde_json::to_string(&response).unwrap();

        // Deserialize (what bridge would receive)
        let received_response: JsonRpcResponse = serde_json::from_str(&response_json).unwrap();
        assert!(received_response.result.is_some());
        assert!(received_response.error.is_none());
    }

    /// Test notification (request without id)
    #[test]
    fn test_notification_format() {
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: None, // Notification has no ID
            method: "notifications/initialized".to_string(),
            params: None,
        };

        let json_str = serde_json::to_string(&request).unwrap();
        assert!(!json_str.contains("\"id\"")); // ID should be omitted
    }
}
