//! MCP Gateway Module
//!
//! Provides a unified gateway endpoint that aggregates multiple MCP servers
//! into a single interface with namespace-based routing and deferred loading.

pub mod deferred;
#[allow(clippy::module_inception)]
mod gateway;
mod merger;
pub mod router;
pub mod sampling;
pub mod session;
pub mod types;

#[cfg(test)]
mod tests;

// Re-export public API
pub use gateway::McpGateway;
pub use types::GatewayConfig;
