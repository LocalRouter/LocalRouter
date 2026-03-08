//! MCP Gateway Module
//!
//! Provides a unified gateway endpoint that aggregates multiple MCP servers
//! into a single interface with namespace-based routing and deferred loading.

pub mod access_control;
pub mod context_mode;
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
pub mod virtual_coding_agents;
pub mod virtual_marketplace;
pub mod virtual_server;
pub mod virtual_skills;

#[cfg(test)]
mod tests;

// Re-export public API
pub use gateway::{ActiveSessionInfo, CatalogSourceEntry, McpGateway};
pub use merger::{
    build_gateway_instructions, build_preview_instructions_context, build_preview_mock_realistic,
    compute_catalog_compression_plan, InstructionsContext,
};
pub use types::GatewayConfig;
