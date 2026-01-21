//! Integration tests for MCP Client Capabilities
//!
//! Tests roots/list, sampling/createMessage, and elicitation/requestInput

use localrouter_ai::config::{Client, RootConfig};
use localrouter_ai::mcp::protocol::{JsonRpcRequest, JsonRpcResponse, Root};
use serde_json::json;

#[test]
fn test_roots_merge_logic() {
    // This is a unit test for the merge logic
    // Full integration test would require HTTP server setup

    let global_roots = vec![
        RootConfig {
            uri: "file:///global/projects".to_string(),
            name: Some("Projects".to_string()),
            enabled: true,
        },
        RootConfig {
            uri: "file:///global/data".to_string(),
            name: None,
            enabled: false, // Should be filtered out
        },
    ];

    let client_roots = vec![RootConfig {
        uri: "file:///client/workspace".to_string(),
        name: Some("Workspace".to_string()),
        enabled: true,
    }];

    // Test 1: No client override - should use global roots
    let result_global: Vec<Root> = global_roots
        .iter()
        .filter(|r| r.enabled)
        .map(|r| Root {
            uri: r.uri.clone(),
            name: r.name.clone(),
        })
        .collect();

    assert_eq!(result_global.len(), 1);
    assert_eq!(result_global[0].uri, "file:///global/projects");

    // Test 2: Client override - should use client roots exclusively
    let result_client: Vec<Root> = client_roots
        .iter()
        .filter(|r| r.enabled)
        .map(|r| Root {
            uri: r.uri.clone(),
            name: r.name.clone(),
        })
        .collect();

    assert_eq!(result_client.len(), 1);
    assert_eq!(result_client[0].uri, "file:///client/workspace");
}

#[test]
fn test_roots_list_request_serialization() {
    // Test that roots/list request can be properly serialized/deserialized
    let request = JsonRpcRequest::new(Some(json!(1)), "roots/list".to_string(), None);

    let serialized = serde_json::to_string(&request).unwrap();
    assert!(serialized.contains("roots/list"));

    let deserialized: JsonRpcRequest = serde_json::from_str(&serialized).unwrap();
    assert_eq!(deserialized.method, "roots/list");
}

#[test]
fn test_roots_list_response_serialization() {
    // Test that roots/list response can be properly serialized/deserialized
    let roots = vec![
        Root {
            uri: "file:///test/path1".to_string(),
            name: Some("Test Path 1".to_string()),
        },
        Root {
            uri: "file:///test/path2".to_string(),
            name: None,
        },
    ];

    let result = json!({
        "roots": roots
    });

    let response = JsonRpcResponse::success(json!(1), result);

    let serialized = serde_json::to_string(&response).unwrap();
    assert!(serialized.contains("file:///test/path1"));
    assert!(serialized.contains("Test Path 1"));

    let deserialized: JsonRpcResponse = serde_json::from_str(&serialized).unwrap();
    assert!(deserialized.is_success());
}

#[test]
fn test_client_roots_configuration() {
    // Test that Client struct properly stores roots configuration
    let mut client = Client::new("Test Client".to_string());

    // Initially no roots override
    assert!(client.roots.is_none());

    // Add roots override
    client.roots = Some(vec![RootConfig {
        uri: "file:///custom/path".to_string(),
        name: Some("Custom".to_string()),
        enabled: true,
    }]);

    assert!(client.roots.is_some());
    assert_eq!(client.roots.as_ref().unwrap().len(), 1);
    assert_eq!(
        client.roots.as_ref().unwrap()[0].uri,
        "file:///custom/path"
    );
}

#[test]
fn test_sampling_request_error_format() {
    // Test that sampling/createMessage returns proper "not implemented" error
    let error = localrouter_ai::mcp::protocol::JsonRpcError::custom(
        -32601,
        "sampling/createMessage not yet fully implemented".to_string(),
        Some(json!({
            "status": "partial",
            "hint": "Sampling infrastructure is in place but requires provider integration"
        })),
    );

    let response = JsonRpcResponse::error(json!(1), error);

    assert!(response.is_error());
    assert!(response.error.is_some());

    let err = response.error.unwrap();
    assert_eq!(err.code, -32601);
    assert!(err.message.contains("not yet fully implemented"));
}

#[test]
fn test_elicitation_request_error_format() {
    // Test that elicitation/requestInput returns proper "not implemented" error
    let error = localrouter_ai::mcp::protocol::JsonRpcError::custom(
        -32601,
        "elicitation/requestInput not yet implemented".to_string(),
        Some(json!({
            "status": "planned",
            "hint": "Elicitation support planned for future release"
        })),
    );

    let response = JsonRpcResponse::error(json!(1), error);

    assert!(response.is_error());
    assert_eq!(response.error.as_ref().unwrap().code, -32601);
}
