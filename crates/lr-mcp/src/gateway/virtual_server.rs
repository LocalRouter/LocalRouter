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
    ///
    /// `arguments` is passed so meta-tools (e.g. `skill_get_info`) can
    /// extract the target resource name for per-item permission checks.
    fn check_permissions(
        &self,
        state: &dyn VirtualSessionState,
        tool_name: &str,
        arguments: Option<&Value>,
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

    /// All tool names this virtual server can provide (regardless of session state).
    ///
    /// Used by the UI to enumerate tools for indexing permission controls.
    fn all_tool_names(&self) -> Vec<String>;

    /// Tool names eligible for catalog compression deferral.
    ///
    /// By default returns an empty list, meaning **none** of this server's tools
    /// can be deferred. Virtual servers that expose large catalogs may override
    /// this to list tools that are safe to hide behind `ctx_search` activation.
    fn deferrable_tools(&self, _state: &dyn VirtualSessionState) -> Vec<String> {
        Vec::new()
    }

    /// Whether a tool's output is worth indexing into FTS5.
    ///
    /// Returns false for action-only tools (e.g., install, start, interrupt)
    /// whose responses don't contain useful searchable content.
    /// Non-indexable tools are shown disabled in the indexing picker.
    /// By default all tools are indexable.
    fn is_tool_indexable(&self, _tool_name: &str) -> bool {
        true
    }

    /// Provide catalog entries for FTS5 indexing.
    ///
    /// Returns `Vec<(label, content)>` for `ContentStore::index()`.
    /// Virtual servers that expose searchable catalogs (e.g. skills)
    /// can override this to make their items discoverable via `IndexSearch`.
    fn catalog_index_entries(&self, _state: &dyn VirtualSessionState) -> Vec<(String, String)> {
        Vec::new()
    }
}

/// Per-session state for a virtual server. Stored in a HashMap on GatewaySession,
/// keyed by virtual server ID.
pub trait VirtualSessionState: Any + Send + Sync {
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
    fn clone_box(&self) -> Box<dyn VirtualSessionState>;
}

/// Result of a virtual server's permission check.
#[allow(clippy::large_enum_variant)]
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
        #[allow(clippy::type_complexity)]
        state_update: Option<Box<dyn FnOnce(&mut dyn VirtualSessionState) + Send>>,
        /// Optional: new server IDs to add to the session's allowed_servers list.
        /// Used by marketplace installs to make newly installed servers accessible.
        #[allow(clippy::type_complexity)]
        add_allowed_servers: Option<Vec<String>>,
    },
    /// Tool not found (shouldn't happen if owns_tool is correct)
    NotHandled,
    /// Error (returned as isError:true content)
    ToolError(String),
}

/// Instructions section produced by a virtual server for the system prompt.
#[derive(Clone)]
pub struct VirtualInstructions {
    pub section_title: String,
    pub content: String,
    pub tool_names: Vec<String>,
    /// Sort priority: lower = listed earlier (0=ctx, 10=coding, 20=marketplace, 30=skills, 50=fallback)
    pub priority: i32,
}
