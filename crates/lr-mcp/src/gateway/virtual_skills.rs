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
use lr_skills::mcp_tools::{SKILL_META_TOOL_NAME, SKILL_READ_FILE_TOOL_NAME};

/// Virtual MCP server for AgentSkills.io skills.
pub struct SkillsVirtualServer {
    skill_manager: Arc<SkillManager>,
    /// Global context management config (read at session creation time).
    config: std::sync::RwLock<lr_config::ContextManagementConfig>,
}

impl SkillsVirtualServer {
    pub fn new(
        skill_manager: Arc<SkillManager>,
        config: lr_config::ContextManagementConfig,
    ) -> Self {
        Self {
            skill_manager,
            config: std::sync::RwLock::new(config),
        }
    }

    /// Update the global config (called when settings change).
    pub fn update_config(&self, config: lr_config::ContextManagementConfig) {
        *self.config.write().unwrap() = config;
    }
}

/// Per-session state for skills.
#[derive(Clone)]
pub struct SkillsSessionState {
    pub permissions: lr_config::SkillsPermissions,
    pub context_management_enabled: bool,
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
        tool_name == SKILL_META_TOOL_NAME || tool_name == SKILL_READ_FILE_TOOL_NAME
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

        lr_skills::mcp_tools::build_skill_tools(&self.skill_manager, &state.permissions)
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

        // Handle skill_read_file (internal tool, not listed to LLM)
        if tool_name == SKILL_READ_FILE_TOOL_NAME {
            let skill_name = arguments
                .get("skill")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let subpath = arguments.get("path").and_then(|v| v.as_str()).unwrap_or("");

            return match lr_skills::mcp_tools::read_skill_file(
                skill_name,
                subpath,
                &self.skill_manager,
                &state.permissions,
            ) {
                Ok(content) => VirtualToolCallResult::Success(serde_json::json!({
                    "content": [{ "type": "text", "text": content }]
                })),
                Err(e) => VirtualToolCallResult::ToolError(e),
            };
        }

        // Handle skill_read
        match lr_skills::mcp_tools::handle_skill_tool_call(
            tool_name,
            &arguments,
            &self.skill_manager,
            &state.permissions,
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
        );

        Some(VirtualInstructions {
            section_title: "Skills".to_string(),
            content: catalog?,
            tool_names: Vec::new(), // populated by gateway
            priority: 30,
        })
    }

    fn create_session_state(&self, client: &lr_config::Client) -> Box<dyn VirtualSessionState> {
        let config = self.config.read().unwrap();
        Box::new(SkillsSessionState {
            permissions: client.skills_permissions.clone(),
            context_management_enabled: client.is_context_management_enabled(&config),
        })
    }

    fn update_session_state(
        &self,
        state: &mut dyn VirtualSessionState,
        client: &lr_config::Client,
    ) {
        let config = self.config.read().unwrap();
        if let Some(s) = state.as_any_mut().downcast_mut::<SkillsSessionState>() {
            s.permissions = client.skills_permissions.clone();
            s.context_management_enabled = client.is_context_management_enabled(&config);
        }
    }

    fn is_tool_indexable(&self, tool_name: &str) -> bool {
        match tool_name {
            "skill_read" => true, // Skill content useful
            _ => false,
        }
    }
}
