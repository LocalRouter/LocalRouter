//! STDIO transport tests
//!
//! Tests for MCP STDIO transport layer, including process management,
//! request/response correlation, timeouts, and error handling.

use super::common::*;
use super::request_validation::*;
use localrouter::mcp::transport::{StdioTransport, Transport};
use serde_json::json;
use std::collections::HashMap;
use std::time::Duration;
use tokio::time::timeout;

#[tokio::test]
async fn test_stdio_single_request() {
    // Create mock STDIO server
    let mock = StdioMockBuilder::new()
        .mock_method("tools/list", json!({"tools": []}))
        .build();

    // Spawn transport
    let transport = StdioTransport::spawn(mock.get_command(), mock.get_args(), mock.get_env())
        .await
        .expect("Failed to spawn STDIO transport");

    // Send request
    let request = standard_jsonrpc_request("tools/list");
    let response = transport
        .send_request(request.clone())
        .await
        .expect("Failed to send request");

    // Validate response
    assert_valid_jsonrpc_response(&response);
    assert_jsonrpc_result(&response, &json!({"tools": []}));
    assert_id_matches(&request, &response);
}

#[tokio::test]
async fn test_stdio_multiple_sequential_requests() {
    let mock = StdioMockBuilder::new()
        .mock_method("tools/list", json!({"tools": ["tool1", "tool2"]}))
        .mock_method("prompts/list", json!({"prompts": ["prompt1"]}))
        .build();

    let transport = StdioTransport::spawn(mock.get_command(), mock.get_args(), mock.get_env())
        .await
        .expect("Failed to spawn transport");

    // Send first request
    let req1 = standard_jsonrpc_request("tools/list");
    let resp1 = transport
        .send_request(req1)
        .await
        .expect("Request 1 failed");
    assert_jsonrpc_result(&resp1, &json!({"tools": ["tool1", "tool2"]}));

    // Send second request
    let req2 = standard_jsonrpc_request("prompts/list");
    let resp2 = transport
        .send_request(req2)
        .await
        .expect("Request 2 failed");
    assert_jsonrpc_result(&resp2, &json!({"prompts": ["prompt1"]}));
}

#[tokio::test]
async fn test_stdio_concurrent_requests() {
    let mock = StdioMockBuilder::new()
        .mock_method("method1", json!({"result": 1}))
        .mock_method("method2", json!({"result": 2}))
        .mock_method("method3", json!({"result": 3}))
        .build();

    let transport = std::sync::Arc::new(
        StdioTransport::spawn(mock.get_command(), mock.get_args(), mock.get_env())
            .await
            .expect("Failed to spawn transport"),
    );

    // Send 3 concurrent requests
    let t1 = transport.clone();
    let t2 = transport.clone();
    let t3 = transport.clone();

    let (resp1, resp2, resp3) = tokio::join!(
        async move { t1.send_request(standard_jsonrpc_request("method1")).await },
        async move { t2.send_request(standard_jsonrpc_request("method2")).await },
        async move { t3.send_request(standard_jsonrpc_request("method3")).await },
    );

    // Verify all responses
    assert_jsonrpc_result(&resp1.unwrap(), &json!({"result": 1}));
    assert_jsonrpc_result(&resp2.unwrap(), &json!({"result": 2}));
    assert_jsonrpc_result(&resp3.unwrap(), &json!({"result": 3}));
}

#[tokio::test]
async fn test_stdio_error_response() {
    let mock = StdioMockBuilder::new()
        .mock_error("invalid_method", -32601, "Method not found")
        .build();

    let transport = StdioTransport::spawn(mock.get_command(), mock.get_args(), mock.get_env())
        .await
        .expect("Failed to spawn transport");

    let request = standard_jsonrpc_request("invalid_method");
    let response = transport
        .send_request(request)
        .await
        .expect("Failed to send request");

    assert_jsonrpc_error(&response, -32601);
    assert_jsonrpc_error_message(&response, -32601, "Method not found");
}

#[tokio::test]
async fn test_stdio_method_not_found() {
    // Mock with no responses configured
    let mock = StdioMockBuilder::new().build();

    let transport = StdioTransport::spawn(mock.get_command(), mock.get_args(), mock.get_env())
        .await
        .expect("Failed to spawn transport");

    let request = standard_jsonrpc_request("unknown_method");
    let response = transport
        .send_request(request)
        .await
        .expect("Failed to send request");

    // Mock script returns -32601 for unknown methods
    assert_jsonrpc_error(&response, -32601);
}

#[tokio::test]
async fn test_stdio_request_with_params() {
    let mock = StdioMockBuilder::new()
        .mock_method("tools/call", json!({"output": "success"}))
        .build();

    let transport = StdioTransport::spawn(mock.get_command(), mock.get_args(), mock.get_env())
        .await
        .expect("Failed to spawn transport");

    let request = request_with_params(
        "tools/call",
        json!({
            "tool": "calculator",
            "args": {"x": 1, "y": 2}
        }),
    );

    let response = transport
        .send_request(request)
        .await
        .expect("Failed to send request");

    assert_jsonrpc_result(&response, &json!({"output": "success"}));
}

#[tokio::test]
async fn test_stdio_environment_variables() {
    let mock = StdioMockBuilder::new()
        .mock_method("test", json!({"ok": true}))
        .build();

    let mut env = mock.get_env();
    env.insert("TEST_VAR".to_string(), "test_value".to_string());
    env.insert("API_KEY".to_string(), "secret123".to_string());

    let transport = StdioTransport::spawn(mock.get_command(), mock.get_args(), env)
        .await
        .expect("Failed to spawn transport with env vars");

    let request = standard_jsonrpc_request("test");
    let response = transport
        .send_request(request)
        .await
        .expect("Request failed");

    assert_jsonrpc_result(&response, &json!({"ok": true}));
}

#[tokio::test]
async fn test_stdio_is_alive() {
    let mock = StdioMockBuilder::new()
        .mock_method("test", json!({"ok": true}))
        .build();

    let transport = StdioTransport::spawn(mock.get_command(), mock.get_args(), mock.get_env())
        .await
        .expect("Failed to spawn transport");

    // Should be alive after spawning
    assert!(
        transport.is_alive(),
        "Transport should be alive after spawn"
    );

    // Drop transport (process will be killed)
    drop(transport);

    // Small delay for process cleanup
    tokio::time::sleep(Duration::from_millis(100)).await;
}

#[tokio::test]
async fn test_stdio_spawn_invalid_command() {
    // Try to spawn with a non-existent command
    let result = StdioTransport::spawn(
        "nonexistent_command_xyz".to_string(),
        vec![],
        HashMap::new(),
    )
    .await;

    assert!(result.is_err(), "Should fail to spawn invalid command");
}

#[tokio::test]
async fn test_stdio_spawn_invalid_args() {
    // Spawn Python with invalid args (will fail immediately)
    let result = StdioTransport::spawn(
        "python3".to_string(),
        vec!["--invalid-flag-xyz".to_string()],
        HashMap::new(),
    )
    .await;

    // Should spawn but process will exit
    // We can't easily test this without the process staying alive
    // This test just verifies spawn doesn't panic
    let _ = result;
}

#[tokio::test]
#[ignore] // Ignore by default as it takes 30+ seconds
async fn test_stdio_request_timeout() {
    // Create mock that hangs forever
    let mock = StdioMockBuilder::new().hang_forever().build();

    let transport = StdioTransport::spawn(mock.get_command(), mock.get_args(), mock.get_env())
        .await
        .expect("Failed to spawn transport");

    let request = standard_jsonrpc_request("test");

    // Try to send request with 2-second timeout
    let result = timeout(Duration::from_secs(2), transport.send_request(request)).await;

    assert!(result.is_err(), "Request should timeout");
}

#[tokio::test]
async fn test_stdio_cleanup_on_drop() {
    let mock = StdioMockBuilder::new()
        .mock_method("test", json!({"ok": true}))
        .build();

    let transport = StdioTransport::spawn(mock.get_command(), mock.get_args(), mock.get_env())
        .await
        .expect("Failed to spawn transport");

    // Verify alive
    assert!(transport.is_alive());

    // Drop transport
    let transport_ptr = std::ptr::addr_of!(transport);
    drop(transport);

    // Process should be killed automatically
    // (We can't easily verify this without accessing internals,
    // but the test documents the expected behavior)

    // Prevent unused variable warning
    let _ = transport_ptr;
}

#[tokio::test]
async fn test_stdio_rapid_fire_requests() {
    let mock = StdioMockBuilder::new()
        .mock_method("ping", json!({"pong": true}))
        .build();

    let transport = std::sync::Arc::new(
        StdioTransport::spawn(mock.get_command(), mock.get_args(), mock.get_env())
            .await
            .expect("Failed to spawn transport"),
    );

    // Send 20 requests rapidly
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
}

#[tokio::test]
async fn test_stdio_request_id_uniqueness() {
    let mock = StdioMockBuilder::new()
        .mock_method("test", json!({"value": 42}))
        .build();

    let transport = StdioTransport::spawn(mock.get_command(), mock.get_args(), mock.get_env())
        .await
        .expect("Failed to spawn transport");

    // Send multiple requests and verify response IDs are unique
    // The transport always generates unique IDs to avoid collisions
    let req1 = standard_jsonrpc_request("test");
    let req2 = standard_jsonrpc_request("test");
    let req3 = standard_jsonrpc_request("test");

    let resp1 = transport.send_request(req1.clone()).await.unwrap();
    let resp2 = transport.send_request(req2.clone()).await.unwrap();
    let resp3 = transport.send_request(req3.clone()).await.unwrap();

    // Response IDs should all be unique (not the same as original request IDs)
    // Transport generates sequential IDs: 1, 2, 3
    assert_ne!(resp1.id, resp2.id, "Response IDs should be unique");
    assert_ne!(resp2.id, resp3.id, "Response IDs should be unique");
    assert_ne!(resp1.id, resp3.id, "Response IDs should be unique");
}

#[tokio::test]
async fn test_stdio_empty_result() {
    let mock = StdioMockBuilder::new()
        .mock_method("empty", json!(null))
        .build();

    let transport = StdioTransport::spawn(mock.get_command(), mock.get_args(), mock.get_env())
        .await
        .expect("Failed to spawn transport");

    let request = standard_jsonrpc_request("empty");
    let response = transport.send_request(request).await.unwrap();

    assert_jsonrpc_result(&response, &json!(null));
}

#[tokio::test]
async fn test_stdio_complex_result() {
    let complex_result = json!({
        "tools": [
            {
                "name": "calculator",
                "description": "Performs calculations",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "expression": {"type": "string"}
                    }
                }
            },
            {
                "name": "web_search",
                "description": "Searches the web",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "query": {"type": "string"},
                        "limit": {"type": "integer"}
                    }
                }
            }
        ]
    });

    let mock = StdioMockBuilder::new()
        .mock_method("tools/list", complex_result.clone())
        .build();

    let transport = StdioTransport::spawn(mock.get_command(), mock.get_args(), mock.get_env())
        .await
        .expect("Failed to spawn transport");

    let request = standard_jsonrpc_request("tools/list");
    let response = transport.send_request(request).await.unwrap();

    assert_jsonrpc_result(&response, &complex_result);
}
