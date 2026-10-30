//! MCP Bridge - STDIO ↔ HTTP Proxy
//!
//! This module provides a lightweight bridge that allows external MCP clients
//! (Claude Desktop, Cursor, VS Code) to connect to LocalRouter's unified MCP
//! gateway via standard input/output.
//!
//! ## Architecture
//!
//! ```text
//! External client (e.g. Claude Code)
//!     ↕ stdio, newline-delimited JSON-RPC
//! localrouter --mcp-bridge
//!     → POST /mcp   client requests, notifications and responses
//!     ← GET  /mcp   SSE: server notifications and requests (after initialize)
//!     Authorization: Bearer <client_secret>, Mcp-Session-Id, X-LocalRouter-Bridge: stdio
//! LocalRouter app (already running) → MCP gateway → MCP servers
//! ```
//!
//! See `docs/MCP_BRIDGE.md` for the full behaviour, and [`negotiation`] for
//! why the bridge negotiates at most MCP 2025-11-25 with its client.
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
//!
//! # Explicit gateway URL (else LOCALROUTER_URL, else the configured port)
//! localrouter --mcp-bridge --url http://127.0.0.1:3625/mcp
//! ```

pub mod negotiation;
mod sse;
mod stdio_bridge;

pub use stdio_bridge::{
    resolve_server_url, StdioBridge, BRIDGE_HEADER, BRIDGE_HEADER_VALUE, URL_ENV,
};
