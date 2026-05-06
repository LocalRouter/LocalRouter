//! AI coding agent orchestration for LocalRouter MCP Gateway.
//!
//! Manages coding agent sessions (Claude Code, Gemini CLI, Codex, etc.)
//! as MCP tools through the Unified MCP Gateway.

pub mod approval;
pub mod discovery;
pub mod manager;
pub mod mcp_tools;
pub mod types;
