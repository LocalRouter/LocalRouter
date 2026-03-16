//! Memory virtual MCP server implementation.
//!
//! Exposes a configurable `MemoryRecall` tool (default name) that searches
//! past conversation memories via memsearch. Enabled per-client.

use std::any::Any;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use super::gateway_tools::FirewallDecisionResult;
use super::virtual_server::*;
use crate::protocol::McpTool;

/// Virtual MCP server for persistent conversation memory.
pub struct MemoryVirtualServer {
    memory_service: Arc<lr_memory::MemoryService>,
}

impl MemoryVirtualServer {
    pub fn new(memory_service: Arc<lr_memory::MemoryService>) -> Self {
        Self { memory_service }
    }
}

/// Per-session state for memory.
#[derive(Clone)]
pub struct MemorySessionState {
    pub enabled: bool,
    pub tool_name: String,
}

impl VirtualSessionState for MemorySessionState {
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
impl VirtualMcpServer for MemoryVirtualServer {
    fn id(&self) -> &str {
        "_memory"
    }

    fn display_name(&self) -> &str {
        "Memory"
    }

    fn owns_tool(&self, tool_name: &str) -> bool {
        let config = self.memory_service.config();
        tool_name == config.recall_tool_name
    }

    fn is_enabled(&self, client: &lr_config::Client) -> bool {
        client.memory_enabled.unwrap_or(false)
    }

    fn list_tools(&self, state: &dyn VirtualSessionState) -> Vec<McpTool> {
        let state = state
            .as_any()
            .downcast_ref::<MemorySessionState>()
            .expect("wrong state type for MemoryVirtualServer");

        if !state.enabled {
            return Vec::new();
        }

        vec![McpTool {
            name: state.tool_name.clone(),
            description: Some(
                "Search past conversation memories for relevant context. \
                 Use when the current conversation would benefit from \
                 information discussed in previous sessions."
                    .to_string(),
            ),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Search query describing what to recall"
                    }
                },
                "required": ["query"]
            }),
        }]
    }

    fn check_permissions(
        &self,
        _state: &dyn VirtualSessionState,
        _tool_name: &str,
        _arguments: Option<&Value>,
        _session_approved: bool,
        _session_denied: bool,
    ) -> VirtualFirewallResult {
        // Memory tools are always allowed when enabled — no firewall popup
        VirtualFirewallResult::Handled(FirewallDecisionResult::Proceed)
    }

    async fn handle_tool_call(
        &self,
        _state: Box<dyn VirtualSessionState>,
        _tool_name: &str,
        arguments: Value,
        client_id: &str,
        _client_name: &str,
    ) -> VirtualToolCallResult {
        let query = match arguments.get("query").and_then(|v| v.as_str()) {
            Some(q) if !q.is_empty() => q.to_string(),
            _ => {
                return VirtualToolCallResult::ToolError(
                    "Missing or empty 'query' parameter".to_string(),
                );
            }
        };

        let config = self.memory_service.config();

        // Ensure client directory and daemon are ready
        if let Err(e) = self.memory_service.ensure_client_dir(client_id) {
            return VirtualToolCallResult::ToolError(format!(
                "Failed to initialize memory directory: {}",
                e
            ));
        }
        // Search
        let results = match self
            .memory_service
            .search(client_id, &query, config.search_top_k)
            .await
        {
            Ok(r) => r,
            Err(e) => {
                return VirtualToolCallResult::ToolError(format!("Memory search failed: {}", e));
            }
        };

        if results.is_empty() {
            return VirtualToolCallResult::Success(serde_json::json!({
                "content": [{
                    "type": "text",
                    "text": "No relevant memories found."
                }]
            }));
        }

        // Format results with optional expand for richer context
        let mut formatted = format!("Found {} relevant memories:\n\n", results.len());

        for (i, result) in results.iter().enumerate() {
            let score = result
                .score
                .map(|s| format!(" [score: {:.2}]", s))
                .unwrap_or_default();

            formatted.push_str(&format!(
                "{}. {}{}\n   Source: {}\n\n",
                i + 1,
                result.content.trim(),
                score,
                result.source,
            ));

            // Try to expand the top result for more context
            if i == 0 {
                if let Some(ref hash) = result.chunk_hash {
                    let client_dir = self
                        .memory_service
                        .memory_dir()
                        .join(client_id);
                    match self.memory_service.cli.expand(&client_dir, hash).await {
                        Ok(expanded) if !expanded.is_empty() => {
                            formatted.push_str(&format!(
                                "   --- Expanded context ---\n{}\n\n",
                                expanded.trim()
                            ));
                        }
                        _ => {}
                    }
                }
            }
        }

        VirtualToolCallResult::Success(serde_json::json!({
            "content": [{
                "type": "text",
                "text": formatted
            }]
        }))
    }

    fn build_instructions(&self, state: &dyn VirtualSessionState) -> Option<VirtualInstructions> {
        let state = state
            .as_any()
            .downcast_ref::<MemorySessionState>()
            .expect("wrong state type for MemoryVirtualServer");

        if !state.enabled {
            return None;
        }

        Some(VirtualInstructions {
            section_title: "memory".to_string(),
            content: format!(
                "You have access to persistent memory from past conversations via the {} tool.\n\
                 Use it when the conversation would benefit from historical context.\n\
                 If you have access to a subagent or forked context, prefer using {} \
                 within a subagent to avoid polluting the main conversation with search results.",
                state.tool_name, state.tool_name
            ),
            tool_names: vec![state.tool_name.clone()],
            priority: 40,
        })
    }

    fn create_session_state(&self, client: &lr_config::Client) -> Box<dyn VirtualSessionState> {
        let config = self.memory_service.config();
        Box::new(MemorySessionState {
            enabled: client.memory_enabled.unwrap_or(false),
            tool_name: config.recall_tool_name.clone(),
        })
    }

    fn update_session_state(
        &self,
        state: &mut dyn VirtualSessionState,
        client: &lr_config::Client,
    ) {
        if let Some(state) = state.as_any_mut().downcast_mut::<MemorySessionState>() {
            state.enabled = client.memory_enabled.unwrap_or(false);
            state.tool_name = self.memory_service.config().recall_tool_name.clone();
        }
    }

    fn all_tool_names(&self) -> Vec<String> {
        vec![self.memory_service.config().recall_tool_name.clone()]
    }
}
