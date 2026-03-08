//! Virtual MCP Server trait and supporting types.
//!
//! Virtual servers are in-process implementations that the gateway treats
//! uniformly alongside real transport-based MCP servers. They provide tools,
//! handle tool calls, and generate system prompt instructions without
//! requiring STDIO/network transport.

use std::any::Any;

use async_trait::async_trait;
use serde_json::Value;

use super::access_control::FirewallCheckResult;
use super::gateway_tools::FirewallDecisionResult;
use crate::protocol::McpTool;

/// A virtual MCP server — an in-process tool provider that the gateway
/// treats uniformly alongside real transport-based MCP servers.
#[async_trait]
pub trait VirtualMcpServer: Send + Sync {
    /// Stable identifier (e.g., "_skills", "_marketplace", "_coding_agents")
    fn id(&self) -> &str;

    /// Human-readable name for UI/logs
    fn display_name(&self) -> &str;

    /// Does this tool name belong to this virtual server?
    fn owns_tool(&self, tool_name: &str) -> bool;

    /// Is this virtual server enabled for the given client config?
    fn is_enabled(&self, client: &lr_config::Client) -> bool;

    /// List tools available for this client.
    fn list_tools(&self, state: &dyn VirtualSessionState) -> Vec<McpTool>;

    /// Check permissions for a tool call.
    fn check_permissions(
        &self,
        state: &dyn VirtualSessionState,
        tool_name: &str,
        session_approved: bool,
        session_denied: bool,
    ) -> VirtualFirewallResult;

    /// Handle a tool call. Receives cloned state (owned) — can't hold session lock across await.
    async fn handle_tool_call(
        &self,
        state: Box<dyn VirtualSessionState>,
        tool_name: &str,
        arguments: Value,
        client_id: &str,
        client_name: &str,
    ) -> VirtualToolCallResult;

    /// Build system prompt instructions section. None = no section.
    fn build_instructions(&self, state: &dyn VirtualSessionState) -> Option<VirtualInstructions>;

    /// Create per-session state from client config.
    fn create_session_state(&self, client: &lr_config::Client) -> Box<dyn VirtualSessionState>;

    /// Update existing session state when client config changes (called on each request).
    fn update_session_state(&self, state: &mut dyn VirtualSessionState, client: &lr_config::Client);
}

/// Per-session state for a virtual server. Stored in a HashMap on GatewaySession,
/// keyed by virtual server ID.
pub trait VirtualSessionState: Any + Send + Sync {
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
    fn clone_box(&self) -> Box<dyn VirtualSessionState>;
}

/// Result of a virtual server's permission check.
pub enum VirtualFirewallResult {
    /// Delegate to gateway's standard firewall popup flow
    Standard(FirewallCheckResult),
    /// Already resolved — skip firewall, use this result directly
    Handled(FirewallDecisionResult),
}

/// Result of a virtual server's tool call.
pub enum VirtualToolCallResult {
    /// Simple success
    Success(Value),
    /// Success with side effects
    SuccessWithSideEffects {
        response: Value,
        invalidate_cache: bool,
        send_list_changed: bool,
        /// Optional closure to mutate the virtual server's session state
        state_update: Option<Box<dyn FnOnce(&mut dyn VirtualSessionState) + Send>>,
    },
    /// Tool not found (shouldn't happen if owns_tool is correct)
    NotHandled,
    /// Error (returned as isError:true content)
    ToolError(String),
}

/// Instructions section produced by a virtual server for the system prompt.
pub struct VirtualInstructions {
    pub section_title: String,
    pub content: String,
    pub tool_names: Vec<String>,
    /// Sort priority: lower = listed earlier (0=ctx, 10=coding, 20=marketplace, 30=skills, 50=fallback)
    pub priority: i32,
}
