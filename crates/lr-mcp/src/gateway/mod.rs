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
pub mod sampling_approval;
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
pub use gateway_tools::FirewallDecisionResult;
pub use merger::{
    build_gateway_instructions, build_preview_instructions_context, build_preview_mock_realistic,
    compress_tool_definition, compute_catalog_compression_plan, compute_item_definition_sizes,
    format_prompt_as_markdown, format_resource_as_markdown, format_tool_as_markdown,
    InstructionsContext,
};
pub use types::GatewayConfig;
