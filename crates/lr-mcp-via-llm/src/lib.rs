//! MCP via LLM - Agentic orchestrator for transparent MCP tool execution
//!
//! When a client uses `McpViaLlm` mode, this module intercepts LLM requests,
//! injects available MCP tools, executes tool calls server-side via the MCP
//! gateway, and loops until the LLM produces a final response. The client
//! speaks only the OpenAI protocol and never needs MCP awareness.

mod gateway_client;
mod manager;
mod orchestrator;
mod session;

pub use manager::{McpViaLlmError, McpViaLlmManager};
