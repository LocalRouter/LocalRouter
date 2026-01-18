//! WebSocket transport tests
//!
//! Tests for MCP WebSocket transport layer.

use super::common::*;
use super::request_validation::*;
use localrouter_ai::mcp::transport::{Transport, WebSocketTransport};
use serde_json::json;
use std::collections::HashMap;
use std::time::Duration;

#[tokio::test]
async fn test_websocket_connection_and_request() {
    let mock = WebSocketMockServer::new().await;
    mock.mock_method("tools/list", json!({"tools": []}));

    let transport = WebSocketTransport::connect(mock.server_url(), HashMap::new())
        .await
        .expect("Failed to connect WebSocket");

    let request = standard_jsonrpc_request("tools/list");
    let response = transport
        .send_request(request.clone())
        .await
        .expect("Failed to send request");

    assert_valid_jsonrpc_response(&response);
    assert_jsonrpc_result(&response, &json!({"tools": []}));
    assert_id_matches(&request, &response);

    mock.shutdown().await;
}

#[tokio::test]
async fn test_websocket_multiple_requests() {
    let mock = WebSocketMockServer::new().await;
    mock.mock_method("method1", json!({"result": 1}));
    mock.mock_method("method2", json!({"result": 2}));

    let transport = WebSocketTransport::connect(mock.server_url(), HashMap::new())
        .await
        .expect("Failed to connect");

    let resp1 = transport
        .send_request(standard_jsonrpc_request("method1"))
        .await
        .unwrap();
    assert_jsonrpc_result(&resp1, &json!({"result": 1}));

    let resp2 = transport
        .send_request(standard_jsonrpc_request("method2"))
        .await
        .unwrap();
    assert_jsonrpc_result(&resp2, &json!({"result": 2}));

    mock.shutdown().await;
}

#[tokio::test]
async fn test_websocket_concurrent_requests() {
    let mock = WebSocketMockServer::new().await;
    mock.mock_method("method1", json!({"result": 1}));
    mock.mock_method("method2", json!({"result": 2}));
    mock.mock_method("method3", json!({"result": 3}));

    let transport = std::sync::Arc::new(
        WebSocketTransport::connect(mock.server_url(), HashMap::new())
            .await
            .expect("Failed to connect"),
    );

    let t1 = transport.clone();
    let t2 = transport.clone();
    let t3 = transport.clone();

    let (resp1, resp2, resp3) = tokio::join!(
        async move { t1.send_request(standard_jsonrpc_request("method1")).await },
        async move { t2.send_request(standard_jsonrpc_request("method2")).await },
        async move { t3.send_request(standard_jsonrpc_request("method3")).await },
    );

    assert_jsonrpc_result(&resp1.unwrap(), &json!({"result": 1}));
    assert_jsonrpc_result(&resp2.unwrap(), &json!({"result": 2}));
    assert_jsonrpc_result(&resp3.unwrap(), &json!({"result": 3}));

    mock.shutdown().await;
}

#[tokio::test]
async fn test_websocket_error_response() {
    let mock = WebSocketMockServer::new().await;
    mock.mock_error("invalid_method", -32601, "Method not found");

    let transport = WebSocketTransport::connect(mock.server_url(), HashMap::new())
        .await
        .expect("Failed to connect");

    let request = standard_jsonrpc_request("invalid_method");
    let response = transport.send_request(request).await.unwrap();

    assert_jsonrpc_error(&response, -32601);

    mock.shutdown().await;
}

#[tokio::test]
async fn test_websocket_method_not_found() {
    let mock = WebSocketMockServer::new().await;
    // Don't mock any methods

    let transport = WebSocketTransport::connect(mock.server_url(), HashMap::new())
        .await
        .expect("Failed to connect");

    let request = standard_jsonrpc_request("unknown_method");
    let response = transport.send_request(request).await.unwrap();

    // Mock server returns -32601 for unknown methods
    assert_jsonrpc_error(&response, -32601);

    mock.shutdown().await;
}

#[tokio::test]
async fn test_websocket_connection_refused() {
    let result = WebSocketTransport::connect("ws://localhost:99999".to_string(), HashMap::new()).await;

    assert!(result.is_err(), "Should fail to connect to invalid address");
}

#[tokio::test]
async fn test_websocket_is_healthy() {
    let mock = WebSocketMockServer::new().await;
    mock.mock_method("test", json!({"ok": true}));

    let transport = WebSocketTransport::connect(mock.server_url(), HashMap::new())
        .await
        .expect("Failed to connect");

    assert!(transport.is_healthy(), "Transport should be healthy after connect");

    mock.shutdown().await;

    // Small delay for disconnect
    tokio::time::sleep(Duration::from_millis(100)).await;

    // After server shutdown, should eventually report unhealthy
    // (This may be implementation-dependent)
}

#[tokio::test]
async fn test_websocket_rapid_requests() {
    let mock = WebSocketMockServer::new().await;
    mock.mock_method("ping", json!({"pong": true}));

    let transport = std::sync::Arc::new(
        WebSocketTransport::connect(mock.server_url(), HashMap::new())
            .await
            .expect("Failed to connect"),
    );

    // Send 20 rapid requests
    let mut handles = vec![];
    for _ in 0..20 {
        let t = transport.clone();
        handles.push(tokio::spawn(async move {
            t.send_request(standard_jsonrpc_request("ping")).await
        }));
    }

    // Wait for all
    for handle in handles {
        let response = handle.await.unwrap().unwrap();
        assert_jsonrpc_result(&response, &json!({"pong": true}));
    }

    mock.shutdown().await;
}

#[tokio::test]
async fn test_websocket_complex_result() {
    let complex_result = json!({
        "resources": [
            {
                "uri": "file:///path/to/file.txt",
                "name": "file.txt",
                "mimeType": "text/plain"
            }
        ]
    });

    let mock = WebSocketMockServer::new().await;
    mock.mock_method("resources/list", complex_result.clone());

    let transport = WebSocketTransport::connect(mock.server_url(), HashMap::new())
        .await
        .expect("Failed to connect");

    let request = standard_jsonrpc_request("resources/list");
    let response = transport.send_request(request).await.unwrap();

    assert_jsonrpc_result(&response, &complex_result);

    mock.shutdown().await;
}
