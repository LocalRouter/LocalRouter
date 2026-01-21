//! SSE (Server-Sent Events) transport for MCP
//!
//! Implements Streamable HTTP transport per MCP spec 2025-06-18:
//! - POST endpoint: Send client→server requests
//! - GET endpoint: Persistent SSE stream for server→client responses and notifications
//!
//! This provides bidirectional communication using HTTP + SSE.

use crate::mcp::protocol::{JsonRpcMessage, JsonRpcNotification, JsonRpcRequest, JsonRpcResponse, StreamingChunk};
use crate::mcp::transport::Transport;
use crate::utils::errors::{AppError, AppResult};
use async_trait::async_trait;
use futures_util::{Stream, StreamExt};
use once_cell::sync::Lazy;
use parking_lot::RwLock;
use reqwest::Client;
use serde_json::Value;
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

/// Global shared HTTP client with connection pooling
///
/// This client is shared across all SSE transports to reuse connections.
/// Configuration:
/// - 10 idle connections per host
/// - 60 second idle timeout
/// - 30 second request timeout
static HTTP_CLIENT: Lazy<Client> = Lazy::new(|| {
    Client::builder()
        .pool_max_idle_per_host(10)
        .pool_idle_timeout(Duration::from_secs(60))
        .timeout(Duration::from_secs(30))
        .build()
        .expect("Failed to create global HTTP client")
});

/// Notification callback type for SSE transport
pub type SseNotificationCallback = Arc<dyn Fn(JsonRpcNotification) + Send + Sync>;

/// SSE transport implementation
///
/// Implements Streamable HTTP per MCP spec:
/// - POST requests to send client→server messages
/// - Persistent GET SSE stream for server→client responses and notifications
///
/// The persistent SSE connection is established on connect() and maintained
/// for the lifetime of the transport.
pub struct SseTransport {
    /// Base URL of the MCP server
    url: String,

    /// HTTP client for sending requests
    client: Client,

    /// Custom headers to include in requests
    headers: HashMap<String, String>,

    /// Pending requests waiting for responses
    /// Maps request ID to response sender
    pending: Arc<RwLock<HashMap<String, oneshot::Sender<JsonRpcResponse>>>>,

    /// Next request ID
    next_id: Arc<RwLock<u64>>,

    /// Whether the transport is closed
    closed: Arc<RwLock<bool>>,

    /// Notification callback
    notification_callback: Arc<RwLock<Option<SseNotificationCallback>>>,

    /// Background task handle for SSE stream reader
    #[allow(dead_code)]
    stream_task: Arc<RwLock<Option<JoinHandle<()>>>>,
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

        // Use shared HTTP client with connection pooling
        let client = HTTP_CLIENT.clone();

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

        let pending = Arc::new(RwLock::new(HashMap::new()));
        let closed = Arc::new(RwLock::new(false));
        let notification_callback = Arc::new(RwLock::new(None));

        // Start persistent SSE stream in background
        let stream_url = url.clone();
        let stream_headers = headers.clone();
        let stream_pending = pending.clone();
        let stream_closed = closed.clone();
        let stream_callback = notification_callback.clone();
        let stream_client = client.clone();

        let stream_task = tokio::spawn(async move {
            Self::sse_stream_task(
                stream_url,
                stream_headers,
                stream_client,
                stream_pending,
                stream_closed,
                stream_callback,
            )
            .await;
        });

        let transport = Self {
            url,
            client,
            headers,
            pending,
            next_id: Arc::new(RwLock::new(2)), // Start at 2 since we used 1 for initialization
            closed,
            notification_callback,
            stream_task: Arc::new(RwLock::new(Some(stream_task))),
        };

        tracing::info!("MCP SSE transport connected successfully with persistent stream");

        Ok(transport)
    }

    /// Background task that maintains persistent SSE stream
    ///
    /// Reads from GET SSE endpoint and dispatches:
    /// - Responses → pending request handlers
    /// - Notifications → notification callback
    async fn sse_stream_task(
        url: String,
        headers: HashMap<String, String>,
        client: Client,
        pending: Arc<RwLock<HashMap<String, oneshot::Sender<JsonRpcResponse>>>>,
        closed: Arc<RwLock<bool>>,
        notification_callback: Arc<RwLock<Option<SseNotificationCallback>>>,
    ) {
        tracing::info!("Starting persistent SSE stream task for: {}", url);

        loop {
            // Check if closed
            if *closed.read() {
                tracing::info!("SSE stream task shutting down");
                break;
            }

            // Connect to GET SSE endpoint
            let mut request = client.get(&url);

            // Add headers
            for (key, value) in &headers {
                request = request.header(key, value);
            }
            request = request.header("Accept", "text/event-stream");

            // Send request and get streaming response
            let response = match request.send().await {
                Ok(resp) => resp,
                Err(e) => {
                    tracing::warn!("Failed to connect to SSE stream: {}", e);
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    continue;
                }
            };

            // Check status
            if !response.status().is_success() {
                tracing::warn!("SSE stream returned error status: {}", response.status());
                tokio::time::sleep(Duration::from_secs(5)).await;
                continue;
            }

            tracing::info!("Connected to persistent SSE stream");

            // Read SSE events
            let mut stream = response.bytes_stream();
            let mut buffer = String::new();

            while let Some(chunk_result) = stream.next().await {
                // Check if closed
                if *closed.read() {
                    break;
                }

                match chunk_result {
                    Ok(chunk) => {
                        // Append to buffer
                        buffer.push_str(&String::from_utf8_lossy(&chunk));

                        // Process complete SSE events (separated by \n\n)
                        while let Some(event_end) = buffer.find("\n\n") {
                            let event_text = buffer[..event_end].to_string();
                            buffer = buffer[event_end + 2..].to_string();

                            // Parse SSE event
                            if let Ok(json_str) = Self::parse_sse_response(&event_text) {
                                // Try to parse as JSON-RPC message
                                if let Ok(message) = serde_json::from_str::<JsonRpcMessage>(&json_str) {
                                    match message {
                                        JsonRpcMessage::Response(response) => {
                                            // Find pending request
                                            let id_str = response.id.to_string();
                                            if let Some(sender) = pending.write().remove(&id_str) {
                                                if sender.send(response).is_err() {
                                                    tracing::warn!("Failed to send response to pending request: {}", id_str);
                                                }
                                            } else {
                                                tracing::debug!("Received response for unknown request ID: {}", id_str);
                                            }
                                        }
                                        JsonRpcMessage::Notification(notification) => {
                                            // Invoke notification callback
                                            if let Some(callback) = notification_callback.read().as_ref() {
                                                callback(notification);
                                            }
                                        }
                                        JsonRpcMessage::Request(_) => {
                                            // Server→client requests not yet supported
                                            tracing::debug!("Received server→client request (not yet supported)");
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        tracing::warn!("Error reading SSE stream: {}", e);
                        break;
                    }
                }
            }

            // Connection lost - reconnect after delay
            tracing::info!("SSE stream connection lost, reconnecting in 5s...");
            tokio::time::sleep(Duration::from_secs(5)).await;
        }

        tracing::info!("SSE stream task terminated");
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

        // Stop the SSE stream task
        if let Some(task) = self.stream_task.write().take() {
            task.abort();
            tracing::debug!("SSE stream task aborted");
        }

        Ok(())
    }
}

#[async_trait]
impl Transport for SseTransport {
    async fn send_request(&self, mut request: JsonRpcRequest) -> AppResult<JsonRpcResponse> {
        if *self.closed.read() {
            return Err(AppError::Mcp("Transport is closed".to_string()));
        }

        // Generate unique request ID
        let request_id = {
            let id = self.next_request_id();
            request.id = Some(Value::Number(id.into()));
            id.to_string()
        };

        // Create oneshot channel for response
        let (tx, rx) = oneshot::channel();

        // Register pending request
        self.pending.write().insert(request_id.clone(), tx);

        // Build POST request
        let mut req_builder = self.client.post(&self.url).json(&request);

        // Add headers
        for (key, value) in &self.headers {
            req_builder = req_builder.header(key, value);
        }

        // Send POST request (don't wait for response - it arrives via SSE stream)
        let post_response = req_builder
            .send()
            .await
            .map_err(|e| {
                // Remove from pending on error
                self.pending.write().remove(&request_id);
                AppError::Mcp(format!("Failed to send request: {}", e))
            })?;

        // Check POST status (should be 202 Accepted or 200 OK)
        if !post_response.status().is_success() {
            self.pending.write().remove(&request_id);
            return Err(AppError::Mcp(format!(
                "Server returned error status: {}",
                post_response.status()
            )));
        }

        // Wait for response from SSE stream (with timeout)
        let response = tokio::time::timeout(Duration::from_secs(30), rx)
            .await
            .map_err(|_| {
                self.pending.write().remove(&request_id);
                AppError::Mcp("Request timeout waiting for response".to_string())
            })?
            .map_err(|_| AppError::Mcp("Response channel closed".to_string()))?;

        Ok(response)
    }

    async fn stream_request(
        &self,
        mut request: JsonRpcRequest,
    ) -> AppResult<Pin<Box<dyn Stream<Item = AppResult<StreamingChunk>> + Send>>> {
        if *self.closed.read() {
            return Err(AppError::Mcp("Transport is closed".to_string()));
        }

        // Generate unique request ID
        let request_id = {
            let id = self.next_request_id();
            request.id = Some(Value::Number(id.into()));
            id
        };

        // Add streaming parameter to request
        if let Some(params) = request.params.as_mut() {
            if let Some(obj) = params.as_object_mut() {
                obj.insert("stream".to_string(), serde_json::json!(true));
            }
        } else {
            request.params = Some(serde_json::json!({"stream": true}));
        }

        // Build POST request
        let mut req_builder = self.client.post(&self.url).json(&request);

        // Add headers
        for (key, value) in &self.headers {
            req_builder = req_builder.header(key, value);
        }
        req_builder = req_builder.header("Accept", "text/event-stream");

        // Send POST request
        let response = req_builder
            .send()
            .await
            .map_err(|e| AppError::Mcp(format!("Failed to send streaming request: {}", e)))?;

        // Check status
        if !response.status().is_success() {
            return Err(AppError::Mcp(format!(
                "Server returned error status: {}",
                response.status()
            )));
        }

        // Create async stream from response
        let mut byte_stream = response.bytes_stream();
        let mut buffer = String::new();
        let mut chunk_index = 0u32;

        let stream = async_stream::stream! {
            loop {
                match byte_stream.next().await {
                    Some(Ok(chunk)) => {
                        // Append to buffer
                        buffer.push_str(&String::from_utf8_lossy(&chunk));

                        // Process complete SSE events
                        while let Some(event_end) = buffer.find("\n\n") {
                            let event_text = buffer[..event_end].to_string();
                            buffer = buffer[event_end + 2..].to_string();

                            // Parse SSE event
                            if let Ok(json_str) = Self::parse_sse_response(&event_text) {
                                // Try to parse as StreamingChunk
                                if let Ok(chunk) = serde_json::from_str::<StreamingChunk>(&json_str) {
                                    let is_final = chunk.is_final;
                                    yield Ok(chunk);

                                    if is_final {
                                        return;
                                    }
                                    chunk_index += 1;
                                } else {
                                    // Fallback: wrap as chunk
                                    let data: Value = serde_json::from_str(&json_str).unwrap_or(serde_json::json!(null));
                                    yield Ok(StreamingChunk::new(
                                        Value::Number(request_id.into()),
                                        chunk_index,
                                        false,
                                        data,
                                    ));
                                    chunk_index += 1;
                                }
                            }
                        }
                    }
                    Some(Err(e)) => {
                        yield Err(AppError::Mcp(format!("Stream error: {}", e)));
                        return;
                    }
                    None => {
                        // Stream ended
                        return;
                    }
                }
            }
        };

        Ok(Box::pin(stream))
    }

    fn supports_streaming(&self) -> bool {
        true
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
            stream_task: Arc::new(RwLock::new(None)),
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
