//! JSON-RPC 2.0 protocol types for MCP
//!
//! Implements the JSON-RPC 2.0 specification for Model Context Protocol communication.
//! Reference: https://www.jsonrpc.org/specification

#![allow(dead_code)]

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

/// Custom deserializer for the result field that preserves null distinction
///
/// JSON-RPC 2.0 allows null as a valid result value. This deserializer ensures
/// that `"result": null` is deserialized as `Some(Value::Null)` rather than `None`,
/// allowing us to distinguish between a missing result field and an explicit null result.
fn deserialize_result<'de, D>(deserializer: D) -> Result<Option<Value>, D::Error>
where
    D: Deserializer<'de>,
{
    // Deserialize the value directly - this captures null as Value::Null
    Ok(Some(Value::deserialize(deserializer)?))
}

/// JSON-RPC 2.0 request
///
/// Represents a request sent to an MCP server.
/// The method and params determine what action the server should take.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct JsonRpcRequest {
    /// JSON-RPC version (always "2.0")
    pub jsonrpc: String,

    /// Request identifier (can be string, number, or null)
    /// Used to correlate requests with responses
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<Value>,

    /// Method name to invoke
    pub method: String,

    /// Method parameters (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

/// JSON-RPC 2.0 response
///
/// Represents a successful response from an MCP server.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct JsonRpcResponse {
    /// JSON-RPC version (always "2.0")
    pub jsonrpc: String,

    /// Request identifier (matches the request)
    pub id: Value,

    /// Result data (present on success)
    /// Note: JSON-RPC 2.0 allows null as a valid result value.
    /// When deserializing, `"result": null` becomes `Some(Value::Null)`, not `None`.
    #[serde(default, deserialize_with = "deserialize_result")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,

    /// Error data (present on failure)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

/// JSON-RPC 2.0 error object
///
/// Represents an error that occurred during request processing.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct JsonRpcError {
    /// Error code (integer)
    pub code: i32,

    /// Human-readable error message
    pub message: String,

    /// Additional error data (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

/// JSON-RPC 2.0 notification
///
/// A notification is a request without an id.
/// The server will not send a response to a notification.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct JsonRpcNotification {
    /// JSON-RPC version (always "2.0")
    pub jsonrpc: String,

    /// Method name to invoke
    pub method: String,

    /// Method parameters (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

/// JSON-RPC 2.0 message envelope
///
/// Can be either a request, response, or notification.
/// Used for parsing incoming messages.
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
#[serde(untagged)]
pub enum JsonRpcMessage {
    Request(JsonRpcRequest),
    Response(JsonRpcResponse),
    Notification(JsonRpcNotification),
}

impl<'de> Deserialize<'de> for JsonRpcMessage {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;

        // Response: has "result" or "error" field (and must have "id")
        if value.get("result").is_some() || value.get("error").is_some() {
            return serde_json::from_value(value)
                .map(JsonRpcMessage::Response)
                .map_err(serde::de::Error::custom);
        }

        // Request: has "id" field (including null)
        if value.get("id").is_some() {
            return serde_json::from_value(value)
                .map(JsonRpcMessage::Request)
                .map_err(serde::de::Error::custom);
        }

        // Notification: has "method" but no "id"
        if value.get("method").is_some() {
            return serde_json::from_value(value)
                .map(JsonRpcMessage::Notification)
                .map_err(serde::de::Error::custom);
        }

        Err(serde::de::Error::custom(
            "Invalid JSON-RPC message: must have either 'id' or 'method' field",
        ))
    }
}

// Standard JSON-RPC 2.0 error codes
pub const PARSE_ERROR: i32 = -32700;
pub const INVALID_REQUEST: i32 = -32600;
pub const METHOD_NOT_FOUND: i32 = -32601;
pub const INVALID_PARAMS: i32 = -32602;
pub const INTERNAL_ERROR: i32 = -32603;

// Application-specific error codes (MCP Gateway)
pub const TOOL_NOT_FOUND: i32 = -32001;
pub const RESOURCE_NOT_FOUND: i32 = -32002;
pub const PROMPT_NOT_FOUND: i32 = -32003;
pub const SERVER_UNAVAILABLE: i32 = -32004;

impl JsonRpcRequest {
    /// Create a new JSON-RPC request
    pub fn new(id: Option<Value>, method: String, params: Option<Value>) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            method,
            params,
        }
    }

    /// Create a request with a numeric ID
    pub fn with_id(id: u64, method: String, params: Option<Value>) -> Self {
        Self::new(Some(Value::Number(id.into())), method, params)
    }

    /// Create a request with a string ID
    #[allow(dead_code)]
    pub fn with_string_id(id: String, method: String, params: Option<Value>) -> Self {
        Self::new(Some(Value::String(id)), method, params)
    }

    /// Check if this is a notification (no id)
    pub fn is_notification(&self) -> bool {
        self.id.is_none()
    }
}

impl JsonRpcResponse {
    /// Create a success response
    pub fn success(id: Value, result: Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: Some(result),
            error: None,
        }
    }

    /// Create an error response
    pub fn error(id: Value, error: JsonRpcError) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: None,
            error: Some(error),
        }
    }

    /// Check if this response is an error
    pub fn is_error(&self) -> bool {
        self.error.is_some()
    }

    /// Check if this response is a success
    pub fn is_success(&self) -> bool {
        self.result.is_some()
    }
}

impl JsonRpcError {
    /// Create a new JSON-RPC error
    pub fn new(code: i32, message: String, data: Option<Value>) -> Self {
        Self {
            code,
            message,
            data,
        }
    }

    /// Create a parse error (-32700)
    pub fn parse_error(message: impl Into<String>) -> Self {
        Self::new(PARSE_ERROR, message.into(), None)
    }

    /// Create an invalid request error (-32600)
    pub fn invalid_request(message: impl Into<String>) -> Self {
        Self::new(INVALID_REQUEST, message.into(), None)
    }

    /// Create a method not found error (-32601)
    pub fn method_not_found(method: impl Into<String>) -> Self {
        Self::new(
            METHOD_NOT_FOUND,
            format!("Method not found: {}", method.into()),
            None,
        )
    }

    /// Create an invalid params error (-32602)
    pub fn invalid_params(message: impl Into<String>) -> Self {
        Self::new(INVALID_PARAMS, message.into(), None)
    }

    /// Create an internal error (-32603)
    pub fn internal_error(message: impl Into<String>) -> Self {
        Self::new(INTERNAL_ERROR, message.into(), None)
    }

    /// Create a custom error with application-specific code
    #[allow(dead_code)]
    pub fn custom(code: i32, message: impl Into<String>, data: Option<Value>) -> Self {
        Self::new(code, message.into(), data)
    }

    /// Create a tool not found error (-32001)
    pub fn tool_not_found(name: impl Into<String>) -> Self {
        Self::new(
            TOOL_NOT_FOUND,
            format!("Tool not found: {}", name.into()),
            None,
        )
    }

    /// Create a resource not found error (-32002)
    pub fn resource_not_found(uri: impl Into<String>) -> Self {
        Self::new(
            RESOURCE_NOT_FOUND,
            format!("Resource not found: {}", uri.into()),
            None,
        )
    }

    /// Create a prompt not found error (-32003)
    pub fn prompt_not_found(name: impl Into<String>) -> Self {
        Self::new(
            PROMPT_NOT_FOUND,
            format!("Prompt not found: {}", name.into()),
            None,
        )
    }

    /// Create a server unavailable error (-32004)
    pub fn server_unavailable(message: impl Into<String>) -> Self {
        Self::new(SERVER_UNAVAILABLE, message.into(), None)
    }
}

impl JsonRpcNotification {
    /// Create a new JSON-RPC notification
    pub fn new(method: String, params: Option<Value>) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            method,
            params,
        }
    }
}

// ===== Streaming Support =====

/// Streaming chunk for partial responses
///
/// Used when responses are too large to send at once or need to be sent progressively.
/// Common for large file resources or long-running operations.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct StreamingChunk {
    /// Request ID this chunk belongs to
    pub id: Value,

    /// Chunk sequence number (0-indexed)
    pub chunk_index: u32,

    /// Whether this is the final chunk
    pub is_final: bool,

    /// Partial response data (base64-encoded if binary)
    pub data: Value,

    /// Optional metadata about the chunk
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
}

impl StreamingChunk {
    /// Create a new streaming chunk
    pub fn new(id: Value, chunk_index: u32, is_final: bool, data: Value) -> Self {
        Self {
            id,
            chunk_index,
            is_final,
            data,
            metadata: None,
        }
    }

    /// Create a final chunk
    pub fn final_chunk(id: Value, chunk_index: u32, data: Value) -> Self {
        Self {
            id,
            chunk_index,
            is_final: true,
            data,
            metadata: None,
        }
    }
}

// ===== MCP Entity Types =====

/// MCP Tool definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpTool {
    pub name: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
}

/// MCP Resource definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpResource {
    pub name: String,

    pub uri: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    #[serde(rename = "mimeType", skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
}

/// MCP Prompt definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpPrompt {
    pub name: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub arguments: Option<Vec<McpPromptArgument>>,
}

/// MCP Prompt argument
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpPromptArgument {
    pub name: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub required: Option<bool>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_request_serialization() {
        let req = JsonRpcRequest::with_id(
            1,
            "test_method".to_string(),
            Some(json!({"param": "value"})),
        );
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"jsonrpc\":\"2.0\""));
        assert!(json.contains("\"id\":1"));
        assert!(json.contains("\"method\":\"test_method\""));
    }

    #[test]
    fn test_request_notification() {
        let req = JsonRpcRequest::new(None, "notify".to_string(), None);
        assert!(req.is_notification());

        let req_with_id = JsonRpcRequest::with_id(1, "call".to_string(), None);
        assert!(!req_with_id.is_notification());
    }

    #[test]
    fn test_response_success() {
        let resp = JsonRpcResponse::success(json!(1), json!({"result": "ok"}));
        assert!(resp.is_success());
        assert!(!resp.is_error());
    }

    #[test]
    fn test_response_error() {
        let error = JsonRpcError::internal_error("Something went wrong");
        let resp = JsonRpcResponse::error(json!(1), error);
        assert!(resp.is_error());
        assert!(!resp.is_success());
    }

    #[test]
    fn test_error_codes() {
        let err = JsonRpcError::parse_error("Invalid JSON");
        assert_eq!(err.code, PARSE_ERROR);

        let err = JsonRpcError::invalid_request("Bad request");
        assert_eq!(err.code, INVALID_REQUEST);

        let err = JsonRpcError::method_not_found("unknown_method");
        assert_eq!(err.code, METHOD_NOT_FOUND);

        let err = JsonRpcError::invalid_params("Wrong params");
        assert_eq!(err.code, INVALID_PARAMS);

        let err = JsonRpcError::internal_error("Server error");
        assert_eq!(err.code, INTERNAL_ERROR);

        // Application-specific error codes
        let err = JsonRpcError::tool_not_found("test_tool");
        assert_eq!(err.code, TOOL_NOT_FOUND);
        assert!(err.message.contains("test_tool"));

        let err = JsonRpcError::resource_not_found("file:///test.txt");
        assert_eq!(err.code, RESOURCE_NOT_FOUND);
        assert!(err.message.contains("file:///test.txt"));

        let err = JsonRpcError::prompt_not_found("test_prompt");
        assert_eq!(err.code, PROMPT_NOT_FOUND);
        assert!(err.message.contains("test_prompt"));

        let err = JsonRpcError::server_unavailable("Server offline");
        assert_eq!(err.code, SERVER_UNAVAILABLE);
    }

    #[test]
    fn test_message_parsing() {
        // Parse request
        let json = r#"{"jsonrpc":"2.0","id":1,"method":"test","params":{}}"#;
        let msg: JsonRpcMessage = serde_json::from_str(json).unwrap();
        assert!(matches!(msg, JsonRpcMessage::Request(_)));

        // Parse response
        let json = r#"{"jsonrpc":"2.0","id":1,"result":{"ok":true}}"#;
        let msg: JsonRpcMessage = serde_json::from_str(json).unwrap();
        assert!(matches!(msg, JsonRpcMessage::Response(_)));

        // Parse notification
        let json = r#"{"jsonrpc":"2.0","method":"notify","params":{}}"#;
        let msg: JsonRpcMessage = serde_json::from_str(json).unwrap();
        assert!(matches!(msg, JsonRpcMessage::Notification(_)));
    }

    #[test]
    fn test_roundtrip() {
        let req = JsonRpcRequest::with_id(42, "test".to_string(), Some(json!({"key": "value"})));
        let json = serde_json::to_string(&req).unwrap();
        let parsed: JsonRpcRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.id, req.id);
        assert_eq!(parsed.method, req.method);
        assert_eq!(parsed.params, req.params);
    }
}
