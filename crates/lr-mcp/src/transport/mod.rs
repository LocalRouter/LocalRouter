//! MCP transport layer implementations
//!
//! Supports three transport types:
//! - STDIO: Subprocess with piped stdin/stdout
//! - HTTP-SSE: Server-Sent Events over HTTP
//! - WebSocket: Bidirectional WebSocket connection

#![allow(dead_code)]

pub mod session_transport_set;
pub mod sse;
pub mod stdio;
pub mod websocket;

pub use session_transport_set::SessionTransportSet;
pub use sse::SseTransport;
pub use stdio::{StdioRequestCallback, StdioTransport};
pub use websocket::WebSocketTransport;

use crate::protocol::{JsonRpcNotification, JsonRpcRequest, JsonRpcResponse, StreamingChunk};
use async_trait::async_trait;
use futures_util::stream::Stream;
use lr_types::errors::AppResult;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

/// Notification callback type shared across all transport types.
pub type NotificationCallback = Arc<dyn Fn(JsonRpcNotification) + Send + Sync>;

/// Request callback type for server-initiated requests (sampling, elicitation, etc.)
///
/// Shared across transport types. Returns a future that resolves to the response.
pub type RequestCallback = Arc<
    dyn Fn(JsonRpcRequest) -> Pin<Box<dyn Future<Output = JsonRpcResponse> + Send>> + Send + Sync,
>;

/// Transport trait for MCP communication
///
/// All transport types must implement this trait to send/receive JSON-RPC messages.
#[async_trait]
pub trait Transport: Send + Sync {
    /// Send a JSON-RPC request and await the response
    ///
    /// # Arguments
    /// * `request` - The JSON-RPC request to send
    ///
    /// # Returns
    /// * The JSON-RPC response from the server
    async fn send_request(&self, request: JsonRpcRequest) -> AppResult<JsonRpcResponse>;

    /// Send a request and receive a streaming response
    ///
    /// # Arguments
    /// * `request` - The JSON-RPC request to send
    ///
    /// # Returns
    /// * A stream of chunks representing the response
    ///
    /// # Default Implementation
    /// Falls back to regular send_request and wraps in a single-chunk stream
    async fn stream_request(
        &self,
        request: JsonRpcRequest,
    ) -> AppResult<Pin<Box<dyn Stream<Item = AppResult<StreamingChunk>> + Send>>> {
        // Default: non-streaming transports return single chunk
        let response = self.send_request(request).await?;
        let chunk = StreamingChunk::final_chunk(
            response.id.clone(),
            0,
            response.result.unwrap_or(serde_json::json!(null)),
        );

        Ok(Box::pin(futures_util::stream::once(
            async move { Ok(chunk) },
        )))
    }

    /// Check if the transport supports streaming responses
    fn supports_streaming(&self) -> bool {
        false
    }

    /// Check if the transport is healthy/connected
    async fn is_healthy(&self) -> bool;

    /// Close/cleanup the transport
    async fn close(&self) -> AppResult<()>;

    /// Set a callback for server-originated notifications (e.g. list_changed).
    /// Default no-op for transports that don't support notifications.
    fn set_notification_callback(&self, _callback: NotificationCallback) {}

    /// Set a callback for server-initiated requests (e.g. sampling/createMessage).
    /// Default no-op for transports that don't support server-initiated requests.
    fn set_request_callback(&self, _callback: RequestCallback) {}
}
