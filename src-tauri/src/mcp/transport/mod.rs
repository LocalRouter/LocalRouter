//! MCP transport layer implementations
//!
//! Supports three transport types:
//! - STDIO: Subprocess with piped stdin/stdout
//! - SSE: Server-Sent Events over HTTP
//! - WebSocket: Bidirectional WebSocket connection

pub mod stdio;

pub use stdio::StdioTransport;

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
