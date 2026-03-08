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
use lr_skills::types::sanitize_name;

/// Virtual MCP server for AgentSkills.io skills.
pub struct SkillsVirtualServer {
    skill_manager: Arc<SkillManager>,
}

impl SkillsVirtualServer {
    pub fn new(skill_manager: Arc<SkillManager>) -> Self {
        Self { skill_manager }
    }
}

/// Per-session state for skills.
#[derive(Clone)]
pub struct SkillsSessionState {
    pub permissions: lr_config::SkillsPermissions,
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

/// Extract skill name from a skill tool name for firewall rule matching.
fn extract_skill_name_from_tool(tool_name: &str) -> String {
    let rest = tool_name.strip_prefix("skill_").unwrap_or(tool_name);

    if let Some(pos) = rest.find("_get_info") {
        if pos > 0 {
            return rest[..pos].to_string();
        }
    }

    rest.to_string()
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
        lr_skills::mcp_tools::is_skill_tool(tool_name)
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
        )
    }

    fn check_permissions(
        &self,
        state: &dyn VirtualSessionState,
        tool_name: &str,
        session_approved: bool,
        session_denied: bool,
    ) -> VirtualFirewallResult {
        let skill_name = extract_skill_name_from_tool(tool_name);

        // Global utility tools (e.g. skill_get_async_status) have no skill name.
        // These don't execute skill code, so skip permission checks.
        if skill_name.is_empty() {
            return VirtualFirewallResult::Handled(FirewallDecisionResult::Proceed);
        }

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

        let all_skills = self.skill_manager.get_all();
        let accessible: Vec<_> = all_skills
            .iter()
            .filter(|s| {
                s.enabled
                    && state
                        .permissions
                        .resolve_skill(&s.metadata.name)
                        .is_enabled()
            })
            .collect();

        if accessible.is_empty() {
            return None;
        }

        let mut content = String::from(
            "Call a skill's `get_info` tool to view its instructions, then use `ctx_execute_file` with the absolute script path to run it.\n\n",
        );
        for skill in &accessible {
            let sname = sanitize_name(&skill.metadata.name);
            content.push_str(&format!(
                "- **{}**: `skill_{}_get_info`",
                skill.metadata.name, sname
            ));
            if let Some(desc) = &skill.metadata.description {
                content.push_str(&format!(" — {}", desc));
            }
            content.push('\n');
        }

        Some(VirtualInstructions {
            section_title: "Skills".to_string(),
            content,
            tool_names: Vec::new(), // populated by gateway
            priority: 30,
        })
    }

    fn create_session_state(&self, client: &lr_config::Client) -> Box<dyn VirtualSessionState> {
        Box::new(SkillsSessionState {
            permissions: client.skills_permissions.clone(),
        })
    }

    fn update_session_state(
        &self,
        state: &mut dyn VirtualSessionState,
        client: &lr_config::Client,
    ) {
        if let Some(s) = state.as_any_mut().downcast_mut::<SkillsSessionState>() {
            s.permissions = client.skills_permissions.clone();
        }
    }
}
