//! Model Context Protocol (MCP) support
//!
//! This module provides MCP server management and proxy functionality.

pub mod manager;
pub mod oauth;
pub mod protocol;
pub mod transport;

pub use manager::McpServerManager;
pub use oauth::McpOAuthManager;
pub use protocol::{JsonRpcRequest, JsonRpcResponse, JsonRpcError};
pub use transport::{Transport, StdioTransport};
