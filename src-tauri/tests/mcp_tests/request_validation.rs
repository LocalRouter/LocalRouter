//! JSON-RPC request and response validation helpers
//!
//! Provides assertion functions for validating JSON-RPC 2.0 messages
//! in MCP tests.

use localrouter::mcp::protocol::{JsonRpcError, JsonRpcRequest, JsonRpcResponse};
use serde_json::Value;

/// Assert that a JSON-RPC response is valid according to the spec
pub fn assert_valid_jsonrpc_response(response: &JsonRpcResponse) {
    assert_eq!(response.jsonrpc, "2.0", "JSON-RPC version must be 2.0");
    assert!(
        response.result.is_some() || response.error.is_some(),
        "Response must have either result or error"
    );
    assert!(
        !(response.result.is_some() && response.error.is_some()),
        "Response cannot have both result and error"
    );
}

/// Assert that a JSON-RPC response contains a successful result
pub fn assert_jsonrpc_result(response: &JsonRpcResponse, expected: &Value) {
    assert_valid_jsonrpc_response(response);
    assert!(
        response.error.is_none(),
        "Expected success, got error: {:?}",
        response.error
    );
    assert!(response.result.is_some(), "Expected result to be present");
    assert_eq!(
        response.result.as_ref().unwrap(),
        expected,
        "Result does not match expected value"
    );
}

/// Assert that a JSON-RPC response contains an error with the expected code
pub fn assert_jsonrpc_error(response: &JsonRpcResponse, expected_code: i32) {
    assert_valid_jsonrpc_response(response);
    assert!(
        response.result.is_none(),
        "Expected error, got result: {:?}",
        response.result
    );
    assert!(response.error.is_some(), "Expected error to be present");

    let error = response.error.as_ref().unwrap();
    assert_eq!(
        error.code, expected_code,
        "Error code mismatch. Expected {}, got {}. Message: {}",
        expected_code, error.code, error.message
    );
}

/// Assert that a JSON-RPC response contains an error with expected code and message
pub fn assert_jsonrpc_error_message(
    response: &JsonRpcResponse,
    expected_code: i32,
    expected_message_contains: &str,
) {
    assert_jsonrpc_error(response, expected_code);

    let error = response.error.as_ref().unwrap();
    assert!(
        error.message.contains(expected_message_contains),
        "Error message '{}' does not contain expected substring '{}'",
        error.message,
        expected_message_contains
    );
}

/// Assert that a JSON-RPC request is valid according to the spec
pub fn assert_valid_jsonrpc_request(request: &JsonRpcRequest) {
    assert_eq!(request.jsonrpc, "2.0", "JSON-RPC version must be 2.0");
    assert!(!request.method.is_empty(), "Method name must not be empty");
}

/// Assert that a JSON-RPC request is a notification (no id)
pub fn assert_is_notification(request: &JsonRpcRequest) {
    assert_valid_jsonrpc_request(request);
    assert!(request.id.is_none(), "Notification must not have an id");
}

/// Assert that a JSON-RPC request has a specific method
pub fn assert_request_method(request: &JsonRpcRequest, expected_method: &str) {
    assert_valid_jsonrpc_request(request);
    assert_eq!(request.method, expected_method, "Request method mismatch");
}

/// Assert that response ID matches request ID
pub fn assert_id_matches(request: &JsonRpcRequest, response: &JsonRpcResponse) {
    let request_id = request.id.as_ref().expect("Request must have an id");
    assert_eq!(
        &response.id, request_id,
        "Response ID must match request ID"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_assert_valid_response() {
        let response = JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id: json!(1),
            result: Some(json!({"ok": true})),
            error: None,
        };

        assert_valid_jsonrpc_response(&response);
    }

    #[test]
    #[should_panic(expected = "JSON-RPC version must be 2.0")]
    fn test_invalid_version() {
        let response = JsonRpcResponse {
            jsonrpc: "1.0".to_string(),
            id: json!(1),
            result: Some(json!({"ok": true})),
            error: None,
        };

        assert_valid_jsonrpc_response(&response);
    }

    #[test]
    fn test_assert_result() {
        let response = JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id: json!(1),
            result: Some(json!({"value": 42})),
            error: None,
        };

        assert_jsonrpc_result(&response, &json!({"value": 42}));
    }

    #[test]
    #[should_panic(expected = "Expected success, got error")]
    fn test_assert_result_on_error() {
        let response = JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id: json!(1),
            result: None,
            error: Some(JsonRpcError {
                code: -32603,
                message: "Internal error".to_string(),
                data: None,
            }),
        };

        assert_jsonrpc_result(&response, &json!({"value": 42}));
    }

    #[test]
    fn test_assert_error() {
        let response = JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id: json!(1),
            result: None,
            error: Some(JsonRpcError {
                code: -32601,
                message: "Method not found".to_string(),
                data: None,
            }),
        };

        assert_jsonrpc_error(&response, -32601);
    }

    #[test]
    fn test_assert_error_message() {
        let response = JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id: json!(1),
            result: None,
            error: Some(JsonRpcError {
                code: -32602,
                message: "Invalid params: missing 'name' field".to_string(),
                data: None,
            }),
        };

        assert_jsonrpc_error_message(&response, -32602, "missing 'name'");
    }

    #[test]
    fn test_assert_notification() {
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: None,
            method: "notify".to_string(),
            params: None,
        };

        assert_is_notification(&request);
    }

    #[test]
    fn test_assert_id_matches() {
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(json!(42)),
            method: "test".to_string(),
            params: None,
        };

        let response = JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id: json!(42),
            result: Some(json!({})),
            error: None,
        };

        assert_id_matches(&request, &response);
    }
}
