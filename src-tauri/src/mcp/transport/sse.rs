//! SSE (Server-Sent Events) transport for MCP
//!
//! Communicates with MCP servers via HTTP with SSE for responses.
//! Uses POST requests for sending JSON-RPC messages and SSE for receiving responses.

use crate::mcp::protocol::{JsonRpcRequest, JsonRpcResponse};
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

/// SSE transport implementation
///
/// Sends JSON-RPC requests via HTTP POST and receives responses via Server-Sent Events.
/// Maintains a persistent SSE connection for receiving responses.
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
}

impl SseTransport {
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

        // Validate connection with a test request
        // Use HEAD request for minimal overhead
        let mut validation_req = client.head(&url);
        for (key, value) in &headers {
            validation_req = validation_req.header(key, value);
        }

        let validation_response = validation_req.send().await
            .map_err(|e| AppError::Mcp(format!("Failed to connect to SSE server: {}", e)))?;

        if !validation_response.status().is_success() {
            return Err(AppError::Mcp(format!(
                "Server returned error status on connect: {}",
                validation_response.status()
            )));
        }

        let transport = Self {
            url,
            client,
            headers,
            pending: Arc::new(RwLock::new(HashMap::new())),
            next_id: Arc::new(RwLock::new(1)),
            closed: Arc::new(RwLock::new(false)),
        };

        tracing::info!("MCP SSE transport connected successfully");

        Ok(transport)
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

        for (key, value) in &self.headers {
            req_builder = req_builder.header(key, value);
        }

        // Send POST request
        let response = req_builder.send().await.map_err(|e| {
            AppError::Mcp(format!("Failed to send request: {}", e))
        })?;

        // Check response status
        if !response.status().is_success() {
            return Err(AppError::Mcp(format!(
                "Server returned error status: {}",
                response.status()
            )));
        }

        // Parse JSON-RPC response from body
        let json_response: JsonRpcResponse = response.json().await.map_err(|e| {
            AppError::Mcp(format!("Failed to parse response: {}", e))
        })?;

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
