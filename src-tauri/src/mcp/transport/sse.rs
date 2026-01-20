//! SSE (Server-Sent Events) transport for MCP
//!
//! Communicates with MCP servers via HTTP with SSE for responses.
//! Uses POST requests for sending JSON-RPC messages and SSE for receiving responses.

use crate::mcp::protocol::{JsonRpcNotification, JsonRpcRequest, JsonRpcResponse};
use crate::mcp::transport::Transport;
use crate::utils::errors::{AppError, AppResult};
use async_trait::async_trait;
use parking_lot::RwLock;
use reqwest::Client;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::oneshot;

/// Notification callback type for SSE transport
pub type SseNotificationCallback = Arc<dyn Fn(JsonRpcNotification) + Send + Sync>;

/// SSE transport implementation
///
/// Sends JSON-RPC requests via HTTP POST and receives responses via Server-Sent Events.
/// Maintains a persistent SSE connection for receiving responses.
/// Note: SSE notification support requires persistent streaming (not yet implemented).
pub struct SseTransport {
    /// Base URL of the MCP server
    url: String,

    /// HTTP client for sending requests
    client: Client,

    /// Custom headers to include in requests
    headers: HashMap<String, String>,

    /// Pending requests waiting for responses
    /// Maps request ID to response sender
    #[allow(dead_code)]
    pending: Arc<RwLock<HashMap<String, oneshot::Sender<JsonRpcResponse>>>>,

    /// Next request ID
    next_id: Arc<RwLock<u64>>,

    /// Whether the transport is closed
    closed: Arc<RwLock<bool>>,

    /// Notification callback (optional, currently unused)
    #[allow(dead_code)]
    notification_callback: Arc<RwLock<Option<SseNotificationCallback>>>,
}

impl SseTransport {
    /// Parse SSE response and extract JSON data
    ///
    /// SSE responses have the format:
    /// ```
    /// event: message
    /// data: {"jsonrpc":"2.0",...}
    /// ```
    fn parse_sse_response(sse_text: &str) -> AppResult<String> {
        for line in sse_text.lines() {
            let line = line.trim();
            if line.starts_with("data:") {
                // Extract JSON after "data: "
                let json_str = line.strip_prefix("data:").unwrap_or("").trim();
                if !json_str.is_empty() {
                    return Ok(json_str.to_string());
                }
            }
        }
        Err(AppError::Mcp(
            "No data field found in SSE response".to_string(),
        ))
    }

    /// Create a new SSE transport
    ///
    /// # Arguments
    /// * `url` - Base URL of the MCP server
    /// * `headers` - Custom headers to include in requests
    ///
    /// # Returns
    /// * The transport instance
    ///
    /// # Errors
    /// * Returns an error if the HTTP client cannot be created
    /// * Returns an error if the server is not reachable or returns an error status
    pub async fn connect(url: String, headers: HashMap<String, String>) -> AppResult<Self> {
        tracing::info!("Connecting to MCP SSE server: {}", url);

        // Build HTTP client with timeout
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| AppError::Mcp(format!("Failed to create HTTP client: {}", e)))?;

        // Validate connection with an MCP initialize request
        let init_request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(Value::Number(1.into())),
            method: "initialize".to_string(),
            params: Some(serde_json::json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {
                    "name": "localrouter",
                    "version": "1.0.0"
                }
            })),
        };

        let mut validation_req = client.post(&url).json(&init_request);

        // Add custom headers
        for (key, value) in &headers {
            validation_req = validation_req.header(key, value);
        }

        // Add required SSE headers
        validation_req = validation_req.header("Accept", "application/json, text/event-stream");

        let validation_response = validation_req
            .send()
            .await
            .map_err(|e| AppError::Mcp(format!("Failed to connect to SSE server: {}", e)))?;

        if !validation_response.status().is_success() {
            let status = validation_response.status();
            let body = validation_response.text().await.unwrap_or_default();
            return Err(AppError::Mcp(format!(
                "Server returned error status on connect: {} - {}",
                status, body
            )));
        }

        // Read the SSE response as text
        let sse_text = validation_response
            .text()
            .await
            .map_err(|e| AppError::Mcp(format!("Failed to read initialize response: {}", e)))?;

        // Parse SSE format to extract JSON
        let json_str = Self::parse_sse_response(&sse_text)?;

        // Parse the JSON-RPC response
        let init_response: JsonRpcResponse = serde_json::from_str(&json_str).map_err(|e| {
            AppError::Mcp(format!("Failed to parse initialize response JSON: {}", e))
        })?;

        if let Some(error) = init_response.error {
            return Err(AppError::Mcp(format!(
                "MCP server returned error on initialize: {} (code {})",
                error.message, error.code
            )));
        }

        let transport = Self {
            url,
            client,
            headers,
            pending: Arc::new(RwLock::new(HashMap::new())),
            next_id: Arc::new(RwLock::new(2)), // Start at 2 since we used 1 for initialization
            closed: Arc::new(RwLock::new(false)),
            notification_callback: Arc::new(RwLock::new(None)),
        };

        tracing::info!("MCP SSE transport connected successfully");

        Ok(transport)
    }

    /// Set a notification callback
    ///
    /// # Arguments
    /// * `callback` - The callback to invoke when notifications are received
    ///
    /// Note: SSE notifications require persistent streaming (not yet implemented)
    pub fn set_notification_callback(&self, callback: SseNotificationCallback) {
        *self.notification_callback.write() = Some(callback);
    }

    /// Generate the next request ID
    fn next_request_id(&self) -> u64 {
        let mut next_id = self.next_id.write();
        let id = *next_id;
        *next_id += 1;
        id
    }

    /// Check if the transport is healthy
    pub fn is_healthy(&self) -> bool {
        !*self.closed.read()
    }

    /// Close the transport
    pub async fn disconnect(&self) -> AppResult<()> {
        tracing::info!("Disconnecting MCP SSE transport");
        *self.closed.write() = true;
        Ok(())
    }
}

#[async_trait]
impl Transport for SseTransport {
    async fn send_request(&self, mut request: JsonRpcRequest) -> AppResult<JsonRpcResponse> {
        if *self.closed.read() {
            return Err(AppError::Mcp("Transport is closed".to_string()));
        }

        // Always generate a unique request ID to avoid collisions
        // This prevents race conditions when concurrent requests might have the same ID
        let _request_id = {
            let id = self.next_request_id();
            request.id = Some(Value::Number(id.into()));
            id.to_string()
        };

        // Build request with custom headers
        let mut req_builder = self.client.post(&self.url).json(&request);

        // Add required SSE headers
        req_builder = req_builder.header("Accept", "application/json, text/event-stream");

        for (key, value) in &self.headers {
            req_builder = req_builder.header(key, value);
        }

        // Send POST request
        let response = req_builder
            .send()
            .await
            .map_err(|e| AppError::Mcp(format!("Failed to send request: {}", e)))?;

        // Check response status
        if !response.status().is_success() {
            return Err(AppError::Mcp(format!(
                "Server returned error status: {}",
                response.status()
            )));
        }

        // Read the SSE response as text
        let sse_text = response
            .text()
            .await
            .map_err(|e| AppError::Mcp(format!("Failed to read response: {}", e)))?;

        // Parse SSE format to extract JSON
        let json_str = Self::parse_sse_response(&sse_text)?;

        // Parse the JSON-RPC response
        let json_response: JsonRpcResponse = serde_json::from_str(&json_str)
            .map_err(|e| AppError::Mcp(format!("Failed to parse response JSON: {}", e)))?;

        Ok(json_response)
    }

    async fn is_healthy(&self) -> bool {
        self.is_healthy()
    }

    async fn close(&self) -> AppResult<()> {
        self.disconnect().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn test_request_id_generation() {
        let transport = SseTransport {
            url: "http://localhost:3000".to_string(),
            client: Client::new(),
            headers: HashMap::new(),
            pending: Arc::new(RwLock::new(HashMap::new())),
            next_id: Arc::new(RwLock::new(1)),
            closed: Arc::new(RwLock::new(false)),
            notification_callback: Arc::new(RwLock::new(None)),
        };

        assert_eq!(transport.next_request_id(), 1);
        assert_eq!(transport.next_request_id(), 2);
        assert_eq!(transport.next_request_id(), 3);
    }

    #[test]
    fn test_json_rpc_request_serialization() {
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(json!(1)),
            method: "test_method".to_string(),
            params: Some(json!({"key": "value"})),
        };

        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains("\"jsonrpc\":\"2.0\""));
        assert!(json.contains("\"method\":\"test_method\""));
    }
}
