//! MCP Gateway Module
//!
//! Provides a unified gateway endpoint that aggregates multiple MCP servers
//! into a single interface with namespace-based routing and deferred loading.

pub mod access_control;
pub mod deferred;
pub mod elicitation;
pub mod firewall;
#[allow(clippy::module_inception)]
mod gateway;
mod gateway_prompts;
mod gateway_resources;
mod gateway_tools;
mod merger;
pub mod router;
pub mod sampling;
pub mod session;
pub mod streaming_notifications;
pub mod types;

#[cfg(test)]
mod tests;

// Re-export public API
pub use gateway::McpGateway;
pub use types::GatewayConfig;
