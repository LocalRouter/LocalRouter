//! WebSocket transport for MCP
//!
//! Communicates with MCP servers via WebSocket for bidirectional JSON-RPC messaging.

use crate::protocol::{JsonRpcMessage, JsonRpcNotification, JsonRpcRequest, JsonRpcResponse};
use crate::transport::Transport;
use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use lr_types::{AppError, AppResult};
use parking_lot::RwLock;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::net::TcpStream;
use tokio::sync::{oneshot, Mutex};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, protocol::Message},
    MaybeTlsStream, WebSocketStream,
};

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

/// Notification callback type for WebSocket transport
pub type WebSocketNotificationCallback = Arc<dyn Fn(JsonRpcNotification) + Send + Sync>;

/// Type alias for the WebSocket write handle
type WsSink = Arc<
    Mutex<
        Option<
            futures_util::stream::SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>,
        >,
    >,
>;

/// WebSocket transport implementation
///
/// Maintains a persistent WebSocket connection for bidirectional JSON-RPC communication.
/// Supports concurrent requests with request/response correlation.
/// Supports notification handling for server-initiated messages.
pub struct WebSocketTransport {
    /// WebSocket URL
    #[allow(dead_code)]
    url: String,

    /// WebSocket write handle
    write: WsSink,

    /// Custom headers (stored for reconnection)
    #[allow(dead_code)]
    headers: HashMap<String, String>,

    /// Pending requests waiting for responses
    /// Maps request ID to response sender
    pending: Arc<RwLock<HashMap<String, oneshot::Sender<JsonRpcResponse>>>>,

    /// Next request ID
    next_id: Arc<RwLock<u64>>,

    /// Whether the transport is closed
    closed: Arc<RwLock<bool>>,

    /// Notification callback (optional)
    notification_callback: Arc<RwLock<Option<WebSocketNotificationCallback>>>,
}

impl WebSocketTransport {
    /// Connect to a WebSocket MCP server
    ///
    /// # Arguments
    /// * `url` - WebSocket URL (ws:// or wss://)
    /// * `headers` - Custom headers to include in the connection request
    ///
    /// # Returns
    /// * The transport instance with established connection
    pub async fn connect(url: String, headers: HashMap<String, String>) -> AppResult<Self> {
        tracing::info!("Connecting to MCP WebSocket server: {}", url);

        // Connect to WebSocket
        let mut handshake = url
            .as_str()
            .into_client_request()
            .map_err(|e| AppError::Mcp(format!("Invalid WebSocket URL: {}", e)))?;
        for (name, value) in &headers {
            let name = name
                .parse::<tokio_tungstenite::tungstenite::http::HeaderName>()
                .map_err(|e| AppError::Mcp(format!("Invalid WebSocket header name: {}", e)))?;
            let value = value
                .parse::<tokio_tungstenite::tungstenite::http::HeaderValue>()
                .map_err(|e| AppError::Mcp(format!("Invalid WebSocket header value: {}", e)))?;
            handshake.headers_mut().insert(name, value);
        }
        let (ws_stream, _) = connect_async(handshake)
            .await
            .map_err(|e| AppError::Mcp(format!("Failed to connect to WebSocket server: {}", e)))?;

        // Split the WebSocket stream
        let (write, mut read) = ws_stream.split();

        let transport = Self {
            url: url.clone(),
            write: Arc::new(Mutex::new(Some(write))),
            headers,
            pending: Arc::new(RwLock::new(HashMap::new())),
            next_id: Arc::new(RwLock::new(1)),
            closed: Arc::new(RwLock::new(false)),
            notification_callback: Arc::new(RwLock::new(None)),
        };

        // Start background task to read messages
        let pending = transport.pending.clone();
        let closed = transport.closed.clone();
        let notification_callback = transport.notification_callback.clone();

        tokio::spawn(async move {
            loop {
                match read.next().await {
                    Some(Ok(Message::Text(text))) => {
                        // Parse JSON-RPC message (response or notification)
                        match serde_json::from_str::<JsonRpcMessage>(&text) {
                            Ok(JsonRpcMessage::Response(response)) => {
                                // Extract ID and find pending sender using normalized ID
                                let id_str = normalize_response_id(&response.id);

                                if let Some(sender) = pending.write().remove(&id_str) {
                                    // Send response to waiting caller
                                    if sender.send(response).is_err() {
                                        tracing::warn!(
                                            "Failed to send response for request ID: {}",
                                            id_str
                                        );
                                    }
                                } else {
                                    tracing::warn!(
                                        "Received response for unknown request ID: {}",
                                        id_str
                                    );
                                }
                            }
                            Ok(JsonRpcMessage::Notification(notification)) => {
                                // Handle notification
                                tracing::debug!("Received notification: {}", notification.method);
                                if let Some(callback) = notification_callback.read().as_ref() {
                                    callback(notification);
                                }
                            }
                            Ok(JsonRpcMessage::Request(_)) => {
                                // Unexpected: server shouldn't send requests to client
                                tracing::warn!(
                                    "Received unexpected request from server (ignored): {}",
                                    text
                                );
                            }
                            Err(e) => {
                                tracing::error!(
                                    "Failed to parse JSON-RPC message: {}\nMessage: {}",
                                    e,
                                    text
                                );
                            }
                        }
                    }
                    Some(Ok(Message::Close(_))) => {
                        tracing::info!("WebSocket connection closed by server");
                        *closed.write() = true;
                        break;
                    }
                    Some(Ok(_)) => {
                        // Ignore ping/pong/binary messages
                    }
                    Some(Err(e)) => {
                        tracing::error!("WebSocket read error: {}", e);
                        *closed.write() = true;
                        break;
                    }
                    None => {
                        // Stream ended
                        tracing::info!("WebSocket stream ended");
                        *closed.write() = true;
                        break;
                    }
                }
            }

            // Clean up pending requests on shutdown
            let mut pending = pending.write();
            for (id, _sender) in pending.drain() {
                tracing::warn!("Request ID {} terminated without response", id);
            }
        });

        tracing::info!("MCP WebSocket transport connected successfully");

        Ok(transport)
    }

    /// Generate the next request ID
    fn next_request_id(&self) -> u64 {
        let mut next_id = self.next_id.write();
        let id = *next_id;
        *next_id += 1;
        id
    }

    /// Set a notification callback
    ///
    /// # Arguments
    /// * `callback` - The callback to invoke when notifications are received
    pub fn set_notification_callback(&self, callback: WebSocketNotificationCallback) {
        *self.notification_callback.write() = Some(callback);
    }

    /// Check if the transport is healthy
    pub fn is_healthy(&self) -> bool {
        !*self.closed.read()
    }

    /// Close the WebSocket connection
    pub async fn disconnect(&self) -> AppResult<()> {
        tracing::info!("Closing MCP WebSocket connection");

        *self.closed.write() = true;

        // Take write handle and close it
        let write_handle = {
            let mut write = self.write.lock().await;
            write.take()
        };

        if let Some(mut write) = write_handle {
            write
                .close()
                .await
                .map_err(|e| AppError::Mcp(format!("Failed to close WebSocket: {}", e)))?;
        }

        Ok(())
    }
}

#[async_trait]
impl Transport for WebSocketTransport {
    async fn send_request(&self, mut request: JsonRpcRequest) -> AppResult<JsonRpcResponse> {
        if *self.closed.read() {
            return Err(AppError::Mcp("Transport is closed".to_string()));
        }

        // Check if this is a notification (no ID, starts with "notifications/")
        // Notifications are fire-and-forget - no response expected
        let is_notification = request.id.is_none() && request.method.starts_with("notifications/");

        if is_notification {
            // For notifications: send without waiting for response
            tracing::debug!("WebSocket sending notification: {}", request.method);

            // Serialize notification to JSON (no ID added)
            let json = serde_json::to_string(&request)
                .map_err(|e| AppError::Mcp(format!("Failed to serialize notification: {}", e)))?;

            // Send message via WebSocket
            {
                let mut write_guard = self.write.lock().await;
                let write = write_guard.as_mut().ok_or_else(|| {
                    AppError::Mcp("WebSocket write handle not available".to_string())
                })?;
                write
                    .send(Message::Text(json))
                    .await
                    .map_err(|e| AppError::Mcp(format!("Failed to send notification: {}", e)))?;
            }

            // Return empty success response for notifications
            return Ok(JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                id: Value::Null,
                result: Some(Value::Null),
                error: None,
            });
        }

        // For regular requests: assign ID and wait for response

        // Store the original request ID to restore in response
        let original_request_id = request.id.clone();

        // Always generate a unique request ID to avoid collisions
        // This prevents race conditions when concurrent requests might have the same ID
        let request_id = {
            let id = self.next_request_id();
            request.id = Some(Value::Number(id.into()));
            id.to_string()
        };

        // Create channel for response
        let (tx, rx) = oneshot::channel();

        // Register pending request
        self.pending.write().insert(request_id.clone(), tx);
        let _pending_guard = super::PendingRequestGuard::new(&self.pending, request_id.clone());

        // Serialize request to JSON
        let json = serde_json::to_string(&request).map_err(|e| {
            self.pending.write().remove(&request_id);
            AppError::Mcp(format!("Failed to serialize request: {}", e))
        })?;

        // Send message via WebSocket
        // Serialize writes without taking the sink out of shared state. A
        // concurrent request waits its turn and cancellation leaves it available.
        {
            let mut write_guard = self.write.lock().await;
            let write = write_guard
                .as_mut()
                .ok_or_else(|| AppError::Mcp("WebSocket write handle not available".to_string()))?;
            write
                .send(Message::Text(json))
                .await
                .map_err(|e| AppError::Mcp(format!("Failed to send message: {}", e)))?;
        }

        // Wait for response (with timeout)
        let mut response = tokio::time::timeout(std::time::Duration::from_secs(30), rx)
            .await
            .map_err(|_| {
                self.pending.write().remove(&request_id);
                AppError::Mcp(format!("Request timeout for ID: {}", request_id))
            })?
            .map_err(|_| {
                AppError::Mcp(format!("Response channel closed for ID: {}", request_id))
            })?;

        // Restore original request ID in response
        response.id = original_request_id.unwrap_or(Value::Null);
        Ok(response)
    }

    async fn is_healthy(&self) -> bool {
        self.is_healthy()
    }

    async fn close(&self) -> AppResult<()> {
        self.disconnect().await
    }

    fn set_notification_callback(&self, callback: super::NotificationCallback) {
        *self.notification_callback.write() = Some(callback);
    }

    // WebSocket transport does not support server-initiated requests (no set_request_callback)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    // Tungstenite fixes the handshake callback's error type to an HTTP response.
    #[allow(clippy::result_large_err)]
    async fn websocket_headers_concurrent_requests_and_cancellation() {
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let server = tokio::spawn(async move {
                let (stream, _) = listener.accept().await.unwrap();
                let mut socket = tokio_tungstenite::accept_hdr_async(stream, |request: &tokio_tungstenite::tungstenite::handshake::server::Request, response| {
                    assert_eq!(request.headers().get("x-test-auth").unwrap(), "local-test-only");
                    Ok(response)
                }).await.unwrap();
                while let Some(message) = socket.next().await {
                    match message.unwrap() {
                        Message::Text(text) => {
                            let request: JsonRpcRequest = serde_json::from_str(&text).unwrap();
                            let response = serde_json::json!({"jsonrpc": "2.0", "id": request.id, "result": {}});
                            socket.send(Message::Text(response.to_string())).await.unwrap();
                        }
                        Message::Close(_) => break,
                        _ => {}
                    }
                }
            });
            let transport = Arc::new(WebSocketTransport::connect(
                format!("ws://{address}"),
                HashMap::from([("x-test-auth".to_string(), "local-test-only".to_string())]),
            ).await.unwrap());

            // Cancel a request queued behind another write. Its registration
            // must disappear without consuming the shared connection.
            let write_guard = transport.write.lock().await;
            let cancelled_transport = transport.clone();
            let cancelled = tokio::spawn(async move {
                cancelled_transport.send_request(JsonRpcRequest::new(Some(Value::from(99)), "test".to_string(), None)).await
            });
            while transport.pending.read().is_empty() {
                tokio::task::yield_now().await;
            }
            cancelled.abort();
            assert!(cancelled.await.unwrap_err().is_cancelled());
            assert!(transport.pending.read().is_empty());
            drop(write_guard);

            let requests = (0..16).map(|id| {
                let transport = transport.clone();
                async move {
                    let request = JsonRpcRequest::new(Some(Value::from(id)), "test".to_string(), Some(serde_json::json!({"payload": "x".repeat(256 * 1024)})));
                    let response = transport.send_request(request).await.unwrap();
                    assert_eq!(response.id, Value::from(id));
                }
            });
            futures_util::future::join_all(requests).await;
            assert!(transport.pending.read().is_empty());
            transport.disconnect().await.unwrap();
            server.await.unwrap();
        }).await.expect("local WebSocket test timed out");
    }

    #[tokio::test]
    async fn test_request_id_generation() {
        let transport = WebSocketTransport {
            url: "ws://localhost:3000".to_string(),
            write: Arc::new(Mutex::new(None)),
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
}
