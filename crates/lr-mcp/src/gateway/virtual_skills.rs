//! Skills virtual MCP server implementation.

use std::any::Any;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use super::access_control::{self, FirewallCheckContext};
use super::gateway_tools::FirewallDecisionResult;
use super::virtual_server::*;
use crate::protocol::McpTool;
use lr_skills::manager::SkillManager;

/// Virtual MCP server for AgentSkills.io skills.
pub struct SkillsVirtualServer {
    skill_manager: Arc<SkillManager>,
    /// Global context management config (read at session creation time).
    config: std::sync::RwLock<lr_config::ContextManagementConfig>,
    /// Skills config with configurable tool names.
    skills_config: std::sync::RwLock<lr_config::SkillsConfig>,
}

impl SkillsVirtualServer {
    pub fn new(
        skill_manager: Arc<SkillManager>,
        config: lr_config::ContextManagementConfig,
        skills_config: lr_config::SkillsConfig,
    ) -> Self {
        Self {
            skill_manager,
            config: std::sync::RwLock::new(config),
            skills_config: std::sync::RwLock::new(skills_config),
        }
    }

    /// Update the global context management config (called when settings change).
    pub fn update_config(&self, config: lr_config::ContextManagementConfig) {
        *self.config.write().unwrap() = config;
    }

    /// Update the skills config (called when settings change).
    pub fn update_skills_config(&self, config: lr_config::SkillsConfig) {
        *self.skills_config.write().unwrap() = config;
    }
}

/// Per-session state for skills.
#[derive(Clone)]
pub struct SkillsSessionState {
    pub permissions: lr_config::SkillsPermissions,
    pub context_management_enabled: bool,
    /// Configured tool name for the skill-read meta-tool.
    pub tool_name: String,
    /// Configured search tool name (e.g. "IndexSearch") for catalog hints.
    pub search_tool_name: String,
}

impl VirtualSessionState for SkillsSessionState {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
    fn clone_box(&self) -> Box<dyn VirtualSessionState> {
        Box::new(self.clone())
    }
}

/// Extract skill name from tool call arguments for firewall rule matching.
///
/// With the meta-tool pattern, the skill name comes from the `name`
/// argument rather than being encoded in the tool name.
fn extract_skill_name_from_arguments(arguments: Option<&Value>) -> Option<String> {
    arguments
        .and_then(|args| args.get("name").or_else(|| args.get("skill")))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

#[async_trait]
impl VirtualMcpServer for SkillsVirtualServer {
    fn id(&self) -> &str {
        "_skills"
    }

    fn display_name(&self) -> &str {
        "Skills"
    }

    fn owns_tool(&self, tool_name: &str) -> bool {
        let skills_config = self.skills_config.read().unwrap();
        tool_name == skills_config.tool_name
    }

    fn is_enabled(&self, client: &lr_config::Client) -> bool {
        client.skills_permissions.global.is_enabled()
            || !client.skills_permissions.skills.is_empty()
    }

    fn list_tools(&self, state: &dyn VirtualSessionState) -> Vec<McpTool> {
        let state = state
            .as_any()
            .downcast_ref::<SkillsSessionState>()
            .expect("wrong state type for SkillsVirtualServer");

        lr_skills::mcp_tools::build_skill_tools(
            &self.skill_manager,
            &state.permissions,
            &state.tool_name,
        )
    }

    fn check_permissions(
        &self,
        state: &dyn VirtualSessionState,
        tool_name: &str,
        arguments: Option<&Value>,
        session_approved: bool,
        session_denied: bool,
    ) -> VirtualFirewallResult {
        // Extract skill name from the `name` argument of the meta-tool
        let skill_name = match extract_skill_name_from_arguments(arguments) {
            Some(name) => name,
            None => {
                // No skill specified — allow the call through; handle_tool_call
                // will return a proper error for the missing parameter.
                return VirtualFirewallResult::Handled(FirewallDecisionResult::Proceed);
            }
        };

        let state = state
            .as_any()
            .downcast_ref::<SkillsSessionState>()
            .expect("wrong state type for SkillsVirtualServer");

        let ctx = FirewallCheckContext::SkillTool {
            permissions: &state.permissions,
            skill_name: &skill_name,
            tool_name,
            session_approved,
            session_denied,
        };
        VirtualFirewallResult::Standard(access_control::check_needs_approval(&ctx))
    }

    async fn handle_tool_call(
        &self,
        state: Box<dyn VirtualSessionState>,
        tool_name: &str,
        arguments: Value,
        _client_id: &str,
        _client_name: &str,
    ) -> VirtualToolCallResult {
        let state = state
            .as_any()
            .downcast_ref::<SkillsSessionState>()
            .expect("wrong state type for SkillsVirtualServer");

        // Handle skill read meta-tool (includes file reading via optional `path` param)
        match lr_skills::mcp_tools::handle_skill_tool_call(
            tool_name,
            &arguments,
            &self.skill_manager,
            &state.permissions,
            &state.tool_name,
        )
        .await
        {
            Ok(Some(result)) => {
                use lr_skills::mcp_tools::SkillToolResult;
                match result {
                    SkillToolResult::Response(response) => VirtualToolCallResult::Success(response),
                }
            }
            Ok(None) => VirtualToolCallResult::NotHandled,
            Err(e) => VirtualToolCallResult::ToolError(e),
        }
    }

    fn build_instructions(&self, state: &dyn VirtualSessionState) -> Option<VirtualInstructions> {
        let state = state
            .as_any()
            .downcast_ref::<SkillsSessionState>()
            .expect("wrong state type for SkillsVirtualServer");

        let has_any_access =
            state.permissions.global.is_enabled() || !state.permissions.skills.is_empty();
        if !has_any_access {
            return None;
        }

        // Build skill catalog using the shared function from lr_skills
        let catalog = lr_skills::mcp_tools::build_skill_catalog(
            &self.skill_manager,
            &state.permissions,
            state.context_management_enabled,
            &state.tool_name,
            &state.search_tool_name,
        );

        Some(VirtualInstructions {
            section_title: "Skills".to_string(),
            content: catalog?,
            tool_names: Vec::new(), // populated by gateway
            priority: 30,
        })
    }

    fn catalog_index_entries(&self, state: &dyn VirtualSessionState) -> Vec<(String, String)> {
        let state = state
            .as_any()
            .downcast_ref::<SkillsSessionState>()
            .expect("wrong state type for SkillsVirtualServer");

        lr_skills::mcp_tools::build_skill_index_entries(&self.skill_manager, &state.permissions)
    }

    fn create_session_state(&self, client: &lr_config::Client) -> Box<dyn VirtualSessionState> {
        let config = self.config.read().unwrap();
        let skills_config = self.skills_config.read().unwrap();
        Box::new(SkillsSessionState {
            permissions: client.skills_permissions.clone(),
            context_management_enabled: client.is_context_management_enabled(&config),
            tool_name: skills_config.tool_name.clone(),
            search_tool_name: config.search_tool_name.clone(),
        })
    }

    fn update_session_state(
        &self,
        state: &mut dyn VirtualSessionState,
        client: &lr_config::Client,
    ) {
        let config = self.config.read().unwrap();
        let skills_config = self.skills_config.read().unwrap();
        if let Some(s) = state.as_any_mut().downcast_mut::<SkillsSessionState>() {
            s.permissions = client.skills_permissions.clone();
            s.context_management_enabled = client.is_context_management_enabled(&config);
            s.tool_name = skills_config.tool_name.clone();
            s.search_tool_name = config.search_tool_name.clone();
        }
    }

    fn all_tool_names(&self) -> Vec<String> {
        let skills_config = self.skills_config.read().unwrap();
        vec![skills_config.tool_name.clone()]
    }

    fn is_tool_indexable(&self, tool_name: &str) -> bool {
        let skills_config = self.skills_config.read().unwrap();
        tool_name == skills_config.tool_name
    }
}
