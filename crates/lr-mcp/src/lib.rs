//! Model Context Protocol (MCP) support
//!
//! This module provides MCP server management and proxy functionality.

pub mod bridge;
pub mod gateway;
pub mod manager;
pub mod oauth;
pub mod oauth_browser;
pub mod protocol;
pub mod transport;

pub use bridge::StdioBridge;
pub use gateway::McpGateway;
pub use manager::McpServerManager;
