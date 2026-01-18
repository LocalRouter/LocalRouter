//! MCP transport layer implementations
//!
//! Supports two transport types:
//! - STDIO: Subprocess with piped stdin/stdout
//! - HTTP-SSE: Server-Sent Events over HTTP

pub mod sse;
pub mod stdio;
// WebSocket transport has been removed - use HTTP-SSE or STDIO instead
// pub mod websocket;

pub use sse::SseTransport;
pub use stdio::StdioTransport;
// pub use websocket::WebSocketTransport;

use crate::mcp::protocol::{JsonRpcRequest, JsonRpcResponse};
use crate::utils::errors::AppResult;
use async_trait::async_trait;

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

    /// Check if the transport is healthy/connected
    async fn is_healthy(&self) -> bool;

    /// Close/cleanup the transport
    async fn close(&self) -> AppResult<()>;
}
