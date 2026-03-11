//! Coding agents virtual MCP server implementation.

use std::any::Any;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use super::access_control::{self, AccessDecision, FirewallCheckResult};
use super::virtual_server::*;
use crate::protocol::McpTool;
use lr_coding_agents::manager::CodingAgentManager;

/// Virtual MCP server for AI coding agent orchestration.
pub struct CodingAgentVirtualServer {
    manager: Arc<CodingAgentManager>,
}

impl CodingAgentVirtualServer {
    pub fn new(manager: Arc<CodingAgentManager>) -> Self {
        Self { manager }
    }
}

/// Per-session state for coding agents.
#[derive(Clone)]
pub struct CodingAgentSessionState {
    pub permission: lr_config::PermissionState,
    pub agent_type: Option<lr_config::CodingAgentType>,
}

impl VirtualSessionState for CodingAgentSessionState {
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

#[async_trait]
impl VirtualMcpServer for CodingAgentVirtualServer {
    fn id(&self) -> &str {
        "_coding_agents"
    }

    fn display_name(&self) -> &str {
        "Coding Agents"
    }

    fn owns_tool(&self, tool_name: &str) -> bool {
        lr_coding_agents::mcp_tools::is_coding_agent_tool(tool_name)
    }

    fn is_enabled(&self, client: &lr_config::Client) -> bool {
        client.coding_agent_permission.is_enabled() && client.coding_agent_type.is_some()
    }

    fn list_tools(&self, state: &dyn VirtualSessionState) -> Vec<McpTool> {
        let state = state
            .as_any()
            .downcast_ref::<CodingAgentSessionState>()
            .expect("wrong state type for CodingAgentVirtualServer");

        if !state.permission.is_enabled() {
            return Vec::new();
        }

        lr_coding_agents::mcp_tools::build_coding_agent_tools(
            &self.manager,
            &state.permission,
            state.agent_type,
        )
    }

    fn check_permissions(
        &self,
        state: &dyn VirtualSessionState,
        tool_name: &str,
        _arguments: Option<&Value>,
        session_approved: bool,
        session_denied: bool,
    ) -> VirtualFirewallResult {
        let state = state
            .as_any()
            .downcast_ref::<CodingAgentSessionState>()
            .expect("wrong state type for CodingAgentVirtualServer");

        let decision = access_control::check_coding_agent_access(&state.permission);

        if state.agent_type.is_none() {
            let result = match decision {
                AccessDecision::Deny => FirewallCheckResult::Deny,
                _ => FirewallCheckResult::Deny, // no agent selected
            };
            return VirtualFirewallResult::Standard(result);
        }

        // Only session creation (coding_agent_start) requires approval popup.
        // All other tools (say, status, respond, interrupt, list) proceed freely.
        if tool_name != "coding_agent_start" {
            let result = match decision {
                AccessDecision::Deny => FirewallCheckResult::Deny,
                _ => FirewallCheckResult::Allow,
            };
            return VirtualFirewallResult::Standard(result);
        }

        // coding_agent_start goes through normal permission flow
        let result = match decision {
            AccessDecision::Allow => FirewallCheckResult::Allow,
            AccessDecision::Deny => FirewallCheckResult::Deny,
            AccessDecision::Ask => {
                if session_denied {
                    FirewallCheckResult::Deny
                } else if session_approved {
                    FirewallCheckResult::Allow
                } else {
                    FirewallCheckResult::Ask
                }
            }
        };
        VirtualFirewallResult::Standard(result)
    }

    async fn handle_tool_call(
        &self,
        state: Box<dyn VirtualSessionState>,
        tool_name: &str,
        arguments: Value,
        client_id: &str,
        _client_name: &str,
    ) -> VirtualToolCallResult {
        let state = state
            .as_any()
            .downcast_ref::<CodingAgentSessionState>()
            .expect("wrong state type for CodingAgentVirtualServer");

        let agent_type = match state.agent_type {
            Some(at) => at,
            None => {
                return VirtualToolCallResult::ToolError(
                    "No coding agent type selected for this client".to_string(),
                );
            }
        };

        match lr_coding_agents::mcp_tools::handle_coding_agent_tool_call(
            tool_name,
            &arguments,
            &self.manager,
            client_id,
            agent_type,
        )
        .await
        {
            Ok(Some(response)) => VirtualToolCallResult::Success(json!({
                "content": [{
                    "type": "text",
                    "text": serde_json::to_string_pretty(&response)
                        .unwrap_or_else(|_| response.to_string())
                }]
            })),
            Ok(None) => VirtualToolCallResult::NotHandled,
            Err(e) => VirtualToolCallResult::ToolError(e),
        }
    }

    fn deferrable_tools(&self, state: &dyn VirtualSessionState) -> Vec<String> {
        // All coding agent tools are deferrable — they can be activated via ctx_search
        self.list_tools(state).into_iter().map(|t| t.name).collect()
    }

    fn build_instructions(&self, state: &dyn VirtualSessionState) -> Option<VirtualInstructions> {
        let state = state
            .as_any()
            .downcast_ref::<CodingAgentSessionState>()
            .expect("wrong state type for CodingAgentVirtualServer");

        if !state.permission.is_enabled() {
            return None;
        }

        let agent_type = state.agent_type?;

        if !self.manager.is_agent_enabled(agent_type) {
            return None;
        }

        let content = format!(
            "You have access to **{}** as a coding agent. Use the unified tools: \
             `coding_agent_start`, `coding_agent_say`, `coding_agent_status`, \
             `coding_agent_respond`, `coding_agent_interrupt`, `coding_agent_list`.\n\n\
             Workflow: Start a session → poll status → respond to questions → get results.\n\
             Use `wait: true` with `coding_agent_status` to block until the agent needs attention, \
             instead of polling in a loop.\n",
            agent_type.display_name(),
        );

        Some(VirtualInstructions {
            section_title: "Coding Agents".to_string(),
            content,
            tool_names: Vec::new(), // populated by gateway
            priority: 10,
        })
    }

    fn create_session_state(&self, client: &lr_config::Client) -> Box<dyn VirtualSessionState> {
        Box::new(CodingAgentSessionState {
            permission: client.coding_agent_permission.clone(),
            agent_type: client.coding_agent_type,
        })
    }

    fn update_session_state(
        &self,
        state: &mut dyn VirtualSessionState,
        client: &lr_config::Client,
    ) {
        if let Some(s) = state.as_any_mut().downcast_mut::<CodingAgentSessionState>() {
            s.permission = client.coding_agent_permission.clone();
            s.agent_type = client.coding_agent_type;
        }
    }
}
