//! Marketplace virtual MCP server implementation.

use std::any::Any;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use super::access_control::{self, AccessDecision, FirewallCheckResult};
use super::virtual_server::*;
use crate::protocol::McpTool;
use lr_marketplace::MarketplaceService;

/// Virtual MCP server for the marketplace (MCP server/skill discovery).
pub struct MarketplaceVirtualServer {
    service: Arc<MarketplaceService>,
}

impl MarketplaceVirtualServer {
    pub fn new(service: Arc<MarketplaceService>) -> Self {
        Self { service }
    }
}

/// Per-session state for marketplace.
#[derive(Clone)]
pub struct MarketplaceSessionState {
    pub permission: lr_config::PermissionState,
}

impl VirtualSessionState for MarketplaceSessionState {
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
impl VirtualMcpServer for MarketplaceVirtualServer {
    fn id(&self) -> &str {
        "_marketplace"
    }

    fn display_name(&self) -> &str {
        "Marketplace"
    }

    fn owns_tool(&self, tool_name: &str) -> bool {
        self.service.is_marketplace_tool(tool_name)
    }

    fn is_enabled(&self, client: &lr_config::Client) -> bool {
        client.marketplace_permission.is_enabled()
    }

    fn list_tools(&self, state: &dyn VirtualSessionState) -> Vec<McpTool> {
        let state = state
            .as_any()
            .downcast_ref::<MarketplaceSessionState>()
            .expect("wrong state type for MarketplaceVirtualServer");

        if !state.permission.is_enabled() || !self.service.is_enabled() {
            return Vec::new();
        }

        // MarketplaceService::list_tools returns Vec<Value>, convert to McpTool
        let tool_values = self.service.list_tools();
        tool_values
            .into_iter()
            .filter_map(|v| {
                serde_json::from_value::<McpTool>(v)
                    .map_err(|e| {
                        tracing::warn!("Failed to deserialize marketplace tool: {}", e);
                    })
                    .ok()
            })
            .collect()
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
            .downcast_ref::<MarketplaceSessionState>()
            .expect("wrong state type for MarketplaceVirtualServer");

        let decision = access_control::check_marketplace_access(&state.permission);

        // Search tools are read-only and never need approval
        if self.service.is_marketplace_search_tool(tool_name) {
            let result = match decision {
                AccessDecision::Deny => FirewallCheckResult::Deny,
                _ => FirewallCheckResult::Allow,
            };
            return VirtualFirewallResult::Standard(result);
        }

        // Install tools go through normal permission flow
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
        _state: Box<dyn VirtualSessionState>,
        tool_name: &str,
        arguments: Value,
        client_id: &str,
        client_name: &str,
    ) -> VirtualToolCallResult {
        match self
            .service
            .handle_tool_call(tool_name, arguments, client_id, client_name)
            .await
        {
            Ok(result) => VirtualToolCallResult::Success(json!({
                "content": [{
                    "type": "text",
                    "text": serde_json::to_string_pretty(&result).unwrap_or_else(|_| result.to_string())
                }]
            })),
            Err(e) => VirtualToolCallResult::ToolError(e.to_string()),
        }
    }

    fn deferrable_tools(&self, state: &dyn VirtualSessionState) -> Vec<String> {
        // All marketplace tools are deferrable — they can be activated via ctx_search
        self.list_tools(state).into_iter().map(|t| t.name).collect()
    }

    fn build_instructions(&self, state: &dyn VirtualSessionState) -> Option<VirtualInstructions> {
        let state = state
            .as_any()
            .downcast_ref::<MarketplaceSessionState>()
            .expect("wrong state type for MarketplaceVirtualServer");

        if !state.permission.is_enabled() || !self.service.is_enabled() {
            return None;
        }

        let content = if self.service.is_mcp_enabled() && self.service.is_skills_enabled() {
            "Use marketplace tools to discover and install new MCP servers and skills.\n"
        } else if self.service.is_mcp_enabled() {
            "Use marketplace tools to discover and install new MCP servers.\n"
        } else {
            "Use marketplace tools to discover and install new skills.\n"
        };

        Some(VirtualInstructions {
            section_title: "Marketplace".to_string(),
            content: content.to_string(),
            tool_names: Vec::new(), // populated by gateway
            priority: 20,
        })
    }

    fn create_session_state(&self, client: &lr_config::Client) -> Box<dyn VirtualSessionState> {
        Box::new(MarketplaceSessionState {
            permission: client.marketplace_permission.clone(),
        })
    }

    fn update_session_state(
        &self,
        state: &mut dyn VirtualSessionState,
        client: &lr_config::Client,
    ) {
        if let Some(s) = state.as_any_mut().downcast_mut::<MarketplaceSessionState>() {
            s.permission = client.marketplace_permission.clone();
        }
    }

    fn all_tool_names(&self) -> Vec<String> {
        vec![
            self.service.search_tool_name(),
            self.service.install_tool_name(),
        ]
    }

    fn is_tool_indexable(&self, tool_name: &str) -> bool {
        // Only the search tool produces indexable results; install is an action tool
        tool_name == self.service.search_tool_name()
    }
}
