//! MCP Bridge - STDIO ↔ HTTP Proxy
//!
//! This module provides a lightweight bridge that allows external MCP clients
//! (Claude Desktop, Cursor, VS Code) to connect to LocalRouter's unified MCP
//! gateway via standard input/output.
//!
//! ## Architecture
//!
//! ```text
//! External Client (e.g., Claude Desktop)
//!     ↓ STDIO (via command invocation)
//! LocalRouter STDIO Bridge Process (--mcp-bridge)
//!     ↓ HTTP Client (localhost:3625/mcp)
//!     Authorization: Bearer <client_secret>
//!     ↓
//! LocalRouter GUI Instance (ALREADY RUNNING)
//!     ↓ HTTP Server (Axum on :3625)
//!     ↓ MCP Gateway (aggregates multiple MCP servers)
//!     ↓
//! Multiple MCP Servers (filesystem, web, github, etc.)
//! ```
//!
//! ## Usage
//!
//! ```bash
//! # Auto-detect client (first enabled client with MCP servers)
//! localrouter --mcp-bridge
//!
//! # Specify client ID explicitly
//! localrouter --mcp-bridge --client-id claude_desktop
//!
//! # With client secret via environment variable
//! LOCALROUTER_CLIENT_SECRET=lr_... localrouter --mcp-bridge --client-id claude_desktop
//! ```

mod stdio_bridge;

pub use stdio_bridge::StdioBridge;
