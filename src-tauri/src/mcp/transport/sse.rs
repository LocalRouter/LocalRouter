//! SSE (Server-Sent Events) transport for MCP
//!
//! Implements Streamable HTTP transport per MCP spec 2025-06-18:
//! - POST endpoint: Send client→server requests
//! - GET endpoint: Persistent SSE stream for server→client responses and notifications
//!
//! This provides bidirectional communication using HTTP + SSE.

use crate::mcp::protocol::{
    JsonRpcMessage, JsonRpcNotification, JsonRpcRequest, JsonRpcResponse, StreamingChunk,
};
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
    /// Base URL of the MCP server (used for SSE connection)
    url: String,

    /// Message endpoint URL for POST requests (received from "endpoint" SSE event)
    /// If None, falls back to using `url` for POST requests
    message_endpoint: Arc<RwLock<Option<String>>>,

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

    /// Whether the SSE stream is connected and ready
    stream_ready: Arc<RwLock<bool>>,

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
    ///
    /// Also handles plain JSON responses (not wrapped in SSE format).
    fn parse_sse_response(sse_text: &str) -> AppResult<String> {
        let trimmed = sse_text.trim();

        // First, try to extract from SSE format
        for line in trimmed.lines() {
            let line = line.trim();
            if line.starts_with("data:") {
                // Extract JSON after "data: "
                let json_str = line.strip_prefix("data:").unwrap_or("").trim();
                if !json_str.is_empty() {
                    return Ok(json_str.to_string());
                }
            }
        }

        // Fallback: Check if it's plain JSON (starts with { or [)
        if trimmed.starts_with('{') || trimmed.starts_with('[') {
            // Validate it's actually JSON
            if serde_json::from_str::<Value>(trimmed).is_ok() {
                return Ok(trimmed.to_string());
            }
        }

        Err(AppError::Mcp(
            "No valid JSON found in response (expected SSE data: field or plain JSON)".to_string(),
        ))
    }

    /// Parse SSE event and extract event type and data
    ///
    /// SSE events have the format:
    /// ```
    /// event: endpoint
    /// data: /messages
    /// ```
    ///
    /// Returns (event_type, data) where either can be None if not present.
    fn parse_sse_event(sse_text: &str) -> (Option<String>, Option<String>) {
        let mut event_type = None;
        let mut data = None;

        for line in sse_text.lines() {
            let line = line.trim();
            if line.starts_with("event:") {
                event_type = Some(line.strip_prefix("event:").unwrap_or("").trim().to_string());
            } else if line.starts_with("data:") {
                data = Some(line.strip_prefix("data:").unwrap_or("").trim().to_string());
            }
        }

        (event_type, data)
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
        let stream_ready = Arc::new(RwLock::new(false));
        let notification_callback = Arc::new(RwLock::new(None));
        let message_endpoint = Arc::new(RwLock::new(None));

        // Start persistent SSE stream in background
        let stream_url = url.clone();
        let stream_headers = headers.clone();
        let stream_pending = pending.clone();
        let stream_closed = closed.clone();
        let stream_ready_clone = stream_ready.clone();
        let stream_callback = notification_callback.clone();
        let stream_client = client.clone();
        let stream_message_endpoint = message_endpoint.clone();

        let stream_task = tokio::spawn(async move {
            Self::sse_stream_task(
                stream_url,
                stream_headers,
                stream_client,
                stream_pending,
                stream_closed,
                stream_ready_clone,
                stream_callback,
                stream_message_endpoint,
            )
            .await;
        });

        // Wait for stream to be ready (with timeout)
        // This prevents race conditions where requests are sent before stream connects
        let ready_timeout = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if *stream_ready.read() {
                    break;
                }
                if *closed.read() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await;

        if ready_timeout.is_err() {
            tracing::warn!(
                "SSE stream did not become ready within timeout, proceeding anyway (inline responses will still work)"
            );
        }

        let transport = Self {
            url,
            message_endpoint,
            client,
            headers,
            pending,
            next_id: Arc::new(RwLock::new(2)), // Start at 2 since we used 1 for initialization
            closed,
            stream_ready,
            notification_callback,
            stream_task: Arc::new(RwLock::new(Some(stream_task))),
        };

        tracing::info!("MCP SSE transport connected successfully with persistent stream");

        Ok(transport)
    }

    /// Background task that maintains persistent SSE stream
    ///
    /// Reads from GET SSE endpoint and dispatches:
    /// - Endpoint events → message_endpoint for POST URL
    /// - Responses → pending request handlers
    /// - Notifications → notification callback
    ///
    /// Uses exponential backoff for reconnection with a maximum of 10 attempts.
    async fn sse_stream_task(
        url: String,
        headers: HashMap<String, String>,
        client: Client,
        pending: Arc<RwLock<HashMap<String, oneshot::Sender<JsonRpcResponse>>>>,
        closed: Arc<RwLock<bool>>,
        stream_ready: Arc<RwLock<bool>>,
        notification_callback: Arc<RwLock<Option<SseNotificationCallback>>>,
        message_endpoint: Arc<RwLock<Option<String>>>,
    ) {
        tracing::info!("Starting persistent SSE stream task for: {}", url);

        const MAX_RECONNECT_ATTEMPTS: u32 = 10;
        const BASE_DELAY_SECS: u64 = 1;
        const MAX_DELAY_SECS: u64 = 60;

        let mut reconnect_attempts = 0u32;
        let mut utf8_buffer = Vec::new(); // Buffer for incomplete UTF-8 sequences

        loop {
            // Check if closed
            if *closed.read() {
                tracing::info!("SSE stream task shutting down");
                break;
            }

            // Check reconnection limit
            if reconnect_attempts >= MAX_RECONNECT_ATTEMPTS {
                tracing::error!(
                    "SSE stream exceeded maximum reconnection attempts ({}), giving up",
                    MAX_RECONNECT_ATTEMPTS
                );
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
                    reconnect_attempts += 1;
                    let delay = std::cmp::min(
                        BASE_DELAY_SECS * 2u64.saturating_pow(reconnect_attempts - 1),
                        MAX_DELAY_SECS,
                    );
                    tracing::warn!(
                        "Failed to connect to SSE stream (attempt {}/{}): {}. Retrying in {}s",
                        reconnect_attempts,
                        MAX_RECONNECT_ATTEMPTS,
                        e,
                        delay
                    );
                    tokio::time::sleep(Duration::from_secs(delay)).await;
                    continue;
                }
            };

            // Check status
            if !response.status().is_success() {
                let status = response.status();

                // 405 Method Not Allowed means the server doesn't support GET SSE streams
                // This is a permanent failure - the server only supports POST (inline responses)
                // Mark as ready anyway so POST requests work, then exit the SSE stream task
                if status == reqwest::StatusCode::METHOD_NOT_ALLOWED {
                    tracing::info!(
                        "Server at {} doesn't support GET SSE stream (405). Transport will use inline responses only. Server-initiated notifications won't be received.",
                        url
                    );
                    *stream_ready.write() = true;
                    break;
                }

                // 404 Not Found also means no SSE endpoint exists - don't retry
                if status == reqwest::StatusCode::NOT_FOUND {
                    tracing::info!(
                        "SSE endpoint not found at {} (404). Transport will use inline responses only.",
                        url
                    );
                    *stream_ready.write() = true;
                    break;
                }

                reconnect_attempts += 1;
                let delay = std::cmp::min(
                    BASE_DELAY_SECS * 2u64.saturating_pow(reconnect_attempts - 1),
                    MAX_DELAY_SECS,
                );
                tracing::warn!(
                    "SSE stream returned error status: {} (attempt {}/{}). Retrying in {}s",
                    status,
                    reconnect_attempts,
                    MAX_RECONNECT_ATTEMPTS,
                    delay
                );
                tokio::time::sleep(Duration::from_secs(delay)).await;
                continue;
            }

            // Reset reconnection counter on successful connection
            reconnect_attempts = 0;
            *stream_ready.write() = true;
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
                        // Proper UTF-8 handling: append chunk to byte buffer and decode
                        utf8_buffer.extend_from_slice(&chunk);

                        // Try to decode as much valid UTF-8 as possible
                        match String::from_utf8(utf8_buffer.clone()) {
                            Ok(text) => {
                                buffer.push_str(&text);
                                utf8_buffer.clear();
                            }
                            Err(e) => {
                                // Partial UTF-8 sequence at the end
                                let valid_up_to = e.utf8_error().valid_up_to();
                                if valid_up_to > 0 {
                                    // Decode the valid portion
                                    let valid_text =
                                        String::from_utf8_lossy(&utf8_buffer[..valid_up_to]);
                                    buffer.push_str(&valid_text);
                                    // Keep the incomplete sequence for next chunk
                                    utf8_buffer = utf8_buffer[valid_up_to..].to_vec();
                                }
                                // If valid_up_to is 0, we have an invalid sequence at the start
                                // Skip one byte and try again
                                if valid_up_to == 0 && !utf8_buffer.is_empty() {
                                    tracing::warn!("Invalid UTF-8 byte in SSE stream, skipping");
                                    utf8_buffer.remove(0);
                                }
                            }
                        }

                        // Process complete SSE events (separated by \n\n)
                        while let Some(event_end) = buffer.find("\n\n") {
                            let event_text = buffer[..event_end].to_string();
                            buffer = buffer[event_end + 2..].to_string();

                            // Parse SSE event type and data
                            let (event_type, event_data) = Self::parse_sse_event(&event_text);

                            // Handle "endpoint" event (MCP SSE transport spec)
                            if event_type.as_deref() == Some("endpoint") {
                                if let Some(endpoint_path) = event_data {
                                    // Resolve endpoint URL relative to base URL
                                    let endpoint_url = if endpoint_path.starts_with("http://")
                                        || endpoint_path.starts_with("https://")
                                    {
                                        endpoint_path.clone()
                                    } else {
                                        // Resolve relative path against base URL
                                        if let Ok(base) = reqwest::Url::parse(&url) {
                                            base.join(&endpoint_path)
                                                .map(|u| u.to_string())
                                                .unwrap_or_else(|_| endpoint_path.clone())
                                        } else {
                                            endpoint_path.clone()
                                        }
                                    };
                                    tracing::info!(
                                        "Received MCP endpoint event: {} -> {}",
                                        endpoint_path,
                                        endpoint_url
                                    );
                                    *message_endpoint.write() = Some(endpoint_url);
                                }
                                continue;
                            }

                            // Parse SSE data as JSON
                            if let Ok(json_str) = Self::parse_sse_response(&event_text) {
                                // Try to parse as JSON-RPC message
                                if let Ok(message) =
                                    serde_json::from_str::<JsonRpcMessage>(&json_str)
                                {
                                    match message {
                                        JsonRpcMessage::Response(response) => {
                                            // Find pending request using normalized ID
                                            let id_str = Self::normalize_response_id(&response.id);
                                            let pending_keys: Vec<String> = pending.read().keys().cloned().collect();
                                            tracing::info!(
                                                "SSE transport received response: id={}, pending_keys={:?}",
                                                id_str,
                                                pending_keys
                                            );
                                            if let Some(sender) = pending.write().remove(&id_str) {
                                                if sender.send(response).is_err() {
                                                    tracing::warn!("Failed to send response to pending request: {}", id_str);
                                                }
                                            } else {
                                                tracing::warn!(
                                                    "Received response for unknown request ID: {} (pending_keys={:?})",
                                                    id_str,
                                                    pending_keys
                                                );
                                            }
                                        }
                                        JsonRpcMessage::Notification(notification) => {
                                            // Invoke notification callback
                                            if let Some(callback) =
                                                notification_callback.read().as_ref()
                                            {
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
                        *stream_ready.write() = false;
                        break;
                    }
                }
            }

            // Connection lost - mark not ready and reconnect after delay
            *stream_ready.write() = false;
            reconnect_attempts += 1;
            let delay = std::cmp::min(
                BASE_DELAY_SECS * 2u64.saturating_pow(reconnect_attempts - 1),
                MAX_DELAY_SECS,
            );
            tracing::info!(
                "SSE stream connection lost, reconnecting in {}s (attempt {}/{})",
                delay,
                reconnect_attempts,
                MAX_RECONNECT_ATTEMPTS
            );
            tokio::time::sleep(Duration::from_secs(delay)).await;
        }

        tracing::info!("SSE stream task terminated");
    }

    /// Normalize response ID for pending map lookup
    ///
    /// Handles the case where server returns `id: null` by converting to a special key.
    /// For other values, converts to string representation.
    fn normalize_response_id(id: &Value) -> String {
        match id {
            Value::Null => "__null_id__".to_string(),
            Value::Number(n) => n.to_string(),
            Value::String(s) => format!("\"{}\"", s),
            _ => id.to_string(),
        }
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

        // Store the original request ID to restore in response
        let original_request_id = request.id.clone();

        // Generate unique internal request ID for tracking pending requests
        let request_id = {
            let id = self.next_request_id();
            request.id = Some(Value::Number(id.into()));
            id.to_string()
        };

        tracing::info!(
            "SSE transport send_request: method={}, internal_id={}, original_id={:?}",
            request.method,
            request_id,
            original_request_id
        );

        // Create oneshot channel for response
        let (tx, rx) = oneshot::channel();

        // Register pending request
        self.pending.write().insert(request_id.clone(), tx);

        // Determine POST URL: use message_endpoint if available, otherwise fall back to base url
        let post_url = self
            .message_endpoint
            .read()
            .clone()
            .unwrap_or_else(|| self.url.clone());

        // Build POST request
        let mut req_builder = self.client.post(&post_url).json(&request);

        // Add Accept header for content negotiation
        req_builder = req_builder.header("Accept", "application/json, text/event-stream");

        // Add custom headers
        for (key, value) in &self.headers {
            req_builder = req_builder.header(key, value);
        }

        tracing::debug!(
            "SSE POST request: url={}, method={}, headers={:?}",
            post_url,
            request.method,
            self.headers
        );

        // Send POST request
        let post_response = req_builder.send().await.map_err(|e| {
            // Remove from pending on error
            self.pending.write().remove(&request_id);
            AppError::Mcp(format!("Failed to send request: {}", e))
        })?;

        // Check POST status (should be 202 Accepted or 200 OK)
        if !post_response.status().is_success() {
            self.pending.write().remove(&request_id);
            let status = post_response.status();
            let headers = post_response.headers().clone();
            let body = post_response.text().await.unwrap_or_default();
            tracing::error!(
                "SSE POST request failed: status={}, url={}, method={}, headers={:?}, body={}",
                status,
                post_url,
                request.method,
                headers,
                body
            );
            return Err(AppError::Mcp(format!(
                "Server returned error status: {} - {}",
                status,
                if body.is_empty() {
                    "no body".to_string()
                } else {
                    body
                }
            )));
        }

        // Check if the response contains an inline response
        // Some servers return the response directly in the POST body (as SSE-formatted text
        // or plain JSON) rather than sending it via the persistent SSE stream
        //
        // Try to read and parse the response body - if it contains valid JSON-RPC, use it
        if let Ok(body_text) = post_response.text().await {
            if !body_text.trim().is_empty() {
                tracing::debug!(
                    "SSE transport POST response body (request_id={}): {} bytes",
                    request_id,
                    body_text.len()
                );
                // Try SSE format first (data: {...}\n\n)
                if let Ok(json_str) = Self::parse_sse_response(&body_text) {
                    if let Ok(mut response) = serde_json::from_str::<JsonRpcResponse>(&json_str) {
                        // Got inline response - remove from pending and return
                        self.pending.write().remove(&request_id);
                        // Restore original request ID in response
                        response.id = original_request_id.clone().unwrap_or(Value::Null);
                        tracing::info!(
                            "SSE transport returning inline SSE response (internal_id={}, restored_id={:?})",
                            request_id,
                            response.id
                        );
                        return Ok(response);
                    }
                }
                // Try plain JSON format
                if let Ok(mut response) = serde_json::from_str::<JsonRpcResponse>(&body_text) {
                    // Got inline JSON response - remove from pending and return
                    self.pending.write().remove(&request_id);
                    // Restore original request ID in response
                    response.id = original_request_id.clone().unwrap_or(Value::Null);
                    tracing::info!(
                        "SSE transport returning inline JSON response (internal_id={}, restored_id={:?})",
                        request_id,
                        response.id
                    );
                    return Ok(response);
                }
            } else {
                tracing::debug!(
                    "SSE transport POST response body empty (request_id={}), waiting for SSE stream",
                    request_id
                );
            }
        }

        // No inline response - wait for response from SSE stream (with timeout)
        tracing::info!(
            "SSE transport waiting for SSE stream response (internal_id={}, timeout=30s)",
            request_id
        );

        let mut response = tokio::time::timeout(Duration::from_secs(30), rx)
            .await
            .map_err(|_| {
                self.pending.write().remove(&request_id);
                tracing::error!(
                    "SSE transport timeout waiting for response (internal_id={})",
                    request_id
                );
                AppError::Mcp("Request timeout waiting for response".to_string())
            })?
            .map_err(|e| {
                tracing::error!(
                    "SSE transport response channel error (internal_id={}): {}",
                    request_id,
                    e
                );
                AppError::Mcp("Response channel closed".to_string())
            })?;

        // Restore original request ID in response
        response.id = original_request_id.unwrap_or(Value::Null);
        tracing::info!(
            "SSE transport received response via SSE stream (internal_id={}, restored_id={:?})",
            request_id,
            response.id
        );
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
        let mut utf8_buffer = Vec::new(); // Buffer for incomplete UTF-8 sequences
        let mut chunk_index = 0u32;

        let stream = async_stream::stream! {
            loop {
                match byte_stream.next().await {
                    Some(Ok(chunk)) => {
                        // Proper UTF-8 handling: append chunk to byte buffer and decode
                        utf8_buffer.extend_from_slice(&chunk);

                        // Try to decode as much valid UTF-8 as possible
                        match String::from_utf8(utf8_buffer.clone()) {
                            Ok(text) => {
                                buffer.push_str(&text);
                                utf8_buffer.clear();
                            }
                            Err(e) => {
                                // Partial UTF-8 sequence at the end
                                let valid_up_to = e.utf8_error().valid_up_to();
                                if valid_up_to > 0 {
                                    let valid_text = String::from_utf8_lossy(&utf8_buffer[..valid_up_to]);
                                    buffer.push_str(&valid_text);
                                    utf8_buffer = utf8_buffer[valid_up_to..].to_vec();
                                }
                                if valid_up_to == 0 && !utf8_buffer.is_empty() {
                                    tracing::warn!("Invalid UTF-8 byte in streaming response, skipping");
                                    utf8_buffer.remove(0);
                                }
                            }
                        }

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
            message_endpoint: Arc::new(RwLock::new(None)),
            client: Client::new(),
            headers: HashMap::new(),
            pending: Arc::new(RwLock::new(HashMap::new())),
            next_id: Arc::new(RwLock::new(1)),
            closed: Arc::new(RwLock::new(false)),
            stream_ready: Arc::new(RwLock::new(false)),
            notification_callback: Arc::new(RwLock::new(None)),
            stream_task: Arc::new(RwLock::new(None)),
        };

        assert_eq!(transport.next_request_id(), 1);
        assert_eq!(transport.next_request_id(), 2);
        assert_eq!(transport.next_request_id(), 3);
    }

    #[test]
    fn test_parse_sse_response_plain_json() {
        // Test plain JSON (not wrapped in SSE format)
        let plain_json = r#"{"jsonrpc":"2.0","id":1,"result":{}}"#;
        let result = SseTransport::parse_sse_response(plain_json);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), plain_json);
    }

    #[test]
    fn test_parse_sse_response_sse_format() {
        // Test SSE format
        let sse_text = "event: message\ndata: {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}\n\n";
        let result = SseTransport::parse_sse_response(sse_text);
        assert!(result.is_ok());
        assert!(result.unwrap().contains("jsonrpc"));
    }

    #[test]
    fn test_normalize_response_id() {
        // Test null ID
        assert_eq!(
            SseTransport::normalize_response_id(&Value::Null),
            "__null_id__"
        );

        // Test numeric ID
        assert_eq!(SseTransport::normalize_response_id(&json!(42)), "42");

        // Test string ID
        assert_eq!(
            SseTransport::normalize_response_id(&json!("abc")),
            "\"abc\""
        );
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
