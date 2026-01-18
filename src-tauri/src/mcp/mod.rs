//! Model Context Protocol (MCP) support
//!
//! This module provides MCP server management and proxy functionality.

pub mod manager;
pub mod oauth;
pub mod protocol;
pub mod transport;

pub use manager::McpServerManager;
