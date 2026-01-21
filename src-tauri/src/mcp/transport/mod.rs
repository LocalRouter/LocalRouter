//! MCP transport layer implementations
//!
//! Supports three transport types:
//! - STDIO: Subprocess with piped stdin/stdout
//! - HTTP-SSE: Server-Sent Events over HTTP

#![allow(dead_code)]
//! - WebSocket: Bidirectional WebSocket connection

pub mod sse;
pub mod stdio;
pub mod websocket;

pub use sse::SseTransport;
pub use stdio::StdioTransport;
pub use websocket::WebSocketTransport;

use crate::mcp::protocol::{JsonRpcRequest, JsonRpcResponse, StreamingChunk};
use crate::utils::errors::AppResult;
use async_trait::async_trait;
use futures_util::stream::Stream;
use std::pin::Pin;

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

        Ok(Box::pin(futures_util::stream::once(async move {
            Ok(chunk)
        })))
    }

    /// Check if the transport supports streaming responses
    fn supports_streaming(&self) -> bool {
        false
    }

    /// Check if the transport is healthy/connected
    async fn is_healthy(&self) -> bool;

    /// Close/cleanup the transport
    async fn close(&self) -> AppResult<()>;
}
