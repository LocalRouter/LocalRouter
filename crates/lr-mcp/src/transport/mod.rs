//! MCP transport layer implementations
//!
//! Supports three transport types:
//! - STDIO: Subprocess with piped stdin/stdout
//! - HTTP-SSE: Server-Sent Events over HTTP
//! - WebSocket: Bidirectional WebSocket connection

pub mod param_headers;
pub mod rpc_stream;
pub mod session_transport_set;
pub mod sse;
pub mod stdio;
pub mod websocket;

pub use rpc_stream::{RpcStream, RpcStreamMessage, SUBSCRIPTION_ID_META_KEY};
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

/// Remove request bookkeeping on every exit path, including cancellation of
/// the caller's future while it is writing or waiting for a response.
struct PendingRequestGuard<'a> {
    pending: &'a parking_lot::RwLock<
        std::collections::HashMap<String, tokio::sync::oneshot::Sender<JsonRpcResponse>>,
    >,
    id: String,
}

impl<'a> PendingRequestGuard<'a> {
    fn new(
        pending: &'a parking_lot::RwLock<
            std::collections::HashMap<String, tokio::sync::oneshot::Sender<JsonRpcResponse>>,
        >,
        id: String,
    ) -> Self {
        Self { pending, id }
    }
}

impl Drop for PendingRequestGuard<'_> {
    fn drop(&mut self) {
        self.pending.write().remove(&self.id);
    }
}

/// The wire ids of in-flight requests by their callers' ids. Transports send
/// every request under an id of their own; a `notifications/cancelled` the
/// caller sends names its own id and must reach the server with the wire id
/// ([`Self::translate_cancel`]).
#[derive(Default)]
pub(crate) struct WireIds {
    /// Caller id (JSON form) → (wire id, registration number).
    map: parking_lot::Mutex<std::collections::HashMap<String, (serde_json::Value, u64)>>,
    next: std::sync::atomic::AtomicU64,
}

/// Removes its registration when the request finishes (any exit path).
pub(crate) struct WireIdGuard<'a> {
    ids: &'a WireIds,
    key: String,
    registration: u64,
}

impl Drop for WireIdGuard<'_> {
    fn drop(&mut self) {
        let mut map = self.ids.map.lock();
        if map
            .get(&self.key)
            .is_some_and(|(_, registration)| *registration == self.registration)
        {
            map.remove(&self.key);
        }
    }
}

impl WireIds {
    /// Record that the request the caller calls `original` is on the wire
    /// as `wire`, for as long as the guard lives.
    pub(crate) fn register(
        &self,
        original: &serde_json::Value,
        wire: serde_json::Value,
    ) -> WireIdGuard<'_> {
        let registration = self.next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let key = original.to_string();
        self.map.lock().insert(key.clone(), (wire, registration));
        WireIdGuard {
            ids: self,
            key,
            registration,
        }
    }

    /// Rewrite a caller's `notifications/cancelled` to the wire id of the
    /// request it names. One naming no request in flight here is left as
    /// it is.
    pub(crate) fn translate_cancel(&self, request: &mut JsonRpcRequest) {
        if request.id.is_some() || request.method != "notifications/cancelled" {
            return;
        }
        let Some(params) = request
            .params
            .as_mut()
            .and_then(serde_json::Value::as_object_mut)
        else {
            return;
        };
        let Some(original) = params.get("requestId") else {
            return;
        };
        if let Some((wire, _)) = self.map.lock().get(&original.to_string()) {
            params.insert("requestId".to_string(), wire.clone());
        }
    }
}

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

    /// Issue a long-lived request (e.g. `events/stream`) and receive every
    /// correlated notification plus the final result as an [`RpcStream`].
    ///
    /// Unlike [`Self::send_request`] there is no total timeout; `buffer`
    /// bounds the messages held for a slow consumer before the transport
    /// waits. Dropping the stream before its result cancels the request.
    async fn open_rpc_stream(
        &self,
        request: JsonRpcRequest,
        buffer: usize,
    ) -> AppResult<RpcStream> {
        let _ = (request, buffer);
        Err(lr_types::AppError::Mcp(
            "This MCP transport does not support streaming requests".to_string(),
        ))
    }

    /// Whether [`Self::open_rpc_stream`] is implemented.
    fn supports_rpc_streams(&self) -> bool {
        false
    }

    /// Switch protocol-era transport behavior after backend discovery.
    fn set_protocol_revision(&self, _revision: crate::protocol::ProtocolRevision) {}

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
