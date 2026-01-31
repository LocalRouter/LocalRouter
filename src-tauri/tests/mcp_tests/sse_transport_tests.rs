//! SSE transport tests
//!
//! Tests for MCP SSE (Server-Sent Events) transport layer.

use super::common::*;
use super::request_validation::*;
use localrouter::mcp::transport::{SseTransport, Transport};
use serde_json::json;
use std::collections::HashMap;

#[tokio::test]
async fn test_sse_single_request() {
    let mock = SseMockBuilder::new()
        .await
        .mock_method("tools/list", json!({"tools": []}))
        .await;

    let transport = SseTransport::connect(mock.base_url(), HashMap::new())
        .await
        .expect("Failed to connect SSE transport");

    let request = standard_jsonrpc_request("tools/list");
    let response = transport
        .send_request(request.clone())
        .await
        .expect("Failed to send request");

    assert_valid_jsonrpc_response(&response);
    assert_jsonrpc_result(&response, &json!({"tools": []}));
    assert_id_matches(&request, &response);
}

#[tokio::test]
async fn test_sse_multiple_requests() {
    let mock = SseMockBuilder::new()
        .await
        .mock_method("method1", json!({"result": 1}))
        .await;

    let transport = SseTransport::connect(mock.base_url(), HashMap::new())
        .await
        .expect("Failed to connect");

    let resp1 = transport
        .send_request(standard_jsonrpc_request("method1"))
        .await
        .unwrap();
    assert_jsonrpc_result(&resp1, &json!({"result": 1}));

    let resp2 = transport
        .send_request(standard_jsonrpc_request("method1"))
        .await
        .unwrap();
    assert_jsonrpc_result(&resp2, &json!({"result": 1}));
}

#[tokio::test]
async fn test_sse_custom_headers() {
    let mock = SseMockBuilder::new()
        .await
        .mock_method("test", json!({"ok": true}))
        .await;

    let mut headers = HashMap::new();
    headers.insert("X-Custom-Header".to_string(), "custom-value".to_string());
    headers.insert("Authorization".to_string(), "Bearer token123".to_string());

    let transport = SseTransport::connect(mock.base_url(), headers)
        .await
        .expect("Failed to connect with custom headers");

    let request = standard_jsonrpc_request("test");
    let response = transport.send_request(request).await.unwrap();

    assert_jsonrpc_result(&response, &json!({"ok": true}));
}

#[tokio::test]
async fn test_sse_error_response() {
    let mock = SseMockBuilder::new()
        .await
        // First mock initialize for connection validation
        .mock_method(
            "initialize",
            json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "serverInfo": {"name": "test", "version": "1.0"}
            }),
        )
        .await
        // Then mock error for invalid_method
        .mock_error("invalid_method", -32601, "Method not found")
        .await;

    let transport = SseTransport::connect(mock.base_url(), HashMap::new())
        .await
        .expect("Failed to connect");

    let request = standard_jsonrpc_request("invalid_method");
    let response = transport.send_request(request).await.unwrap();

    assert_jsonrpc_error(&response, -32601);
}

#[tokio::test]
async fn test_sse_404_error() {
    let mock = SseMockBuilder::new()
        .await
        // First mock initialize for connection validation
        .mock_method(
            "initialize",
            json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "serverInfo": {"name": "test", "version": "1.0"}
            }),
        )
        .await
        // Then set up 404 for subsequent requests
        .mock_404()
        .await;

    let transport = SseTransport::connect(mock.base_url(), HashMap::new())
        .await
        .expect("Failed to connect");

    let request = standard_jsonrpc_request("test");
    let result = transport.send_request(request).await;

    // Should get an error due to 404
    assert!(result.is_err(), "Should fail with 404");
}

#[tokio::test]
async fn test_sse_connection_to_invalid_url() {
    let result = SseTransport::connect("http://localhost:99999".to_string(), HashMap::new()).await;

    assert!(result.is_err(), "Should fail to connect to invalid URL");
}

#[tokio::test]
async fn test_sse_is_healthy() {
    let mock = SseMockBuilder::new()
        .await
        .mock_method("test", json!({"ok": true}))
        .await;

    let transport = SseTransport::connect(mock.base_url(), HashMap::new())
        .await
        .expect("Failed to connect");

    // SSE transport should report healthy after successful connection
    assert!(transport.is_healthy(), "Transport should report healthy");
}

#[tokio::test]
#[ignore] // Ignore by default as it takes 60+ seconds
async fn test_sse_timeout() {
    let mock = SseMockBuilder::new().await.mock_timeout().await;

    let transport = SseTransport::connect(mock.base_url(), HashMap::new())
        .await
        .expect("Failed to connect");

    let request = standard_jsonrpc_request("test");

    // Request should timeout (mock delays 60 seconds, transport has 30s timeout)
    let result = transport.send_request(request).await;

    assert!(result.is_err(), "Request should timeout");
}
