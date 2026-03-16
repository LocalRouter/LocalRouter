//! Memory virtual MCP server implementation.
//!
//! Exposes `MemorySearch` and `MemoryRead` tools (IndexSearch/IndexRead format)
//! for searching and reading past conversation memories via native FTS5
//! (with optional vector search). Enabled per-client.

use std::any::Any;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use super::gateway_tools::FirewallDecisionResult;
use super::virtual_server::*;
use crate::protocol::McpTool;

const DEFAULT_SEARCH_TOOL: &str = "MemorySearch";
const DEFAULT_READ_TOOL: &str = "MemoryRead";
const DEFAULT_SEARCH_LIMIT: usize = 3;

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
    pub search_tool_name: String,
    pub read_tool_name: String,
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
        // We own both the search and read tool names
        tool_name == config.recall_tool_name
            || tool_name == format!("{}Read", config.recall_tool_name.trim_end_matches("Search"))
            // Also match the defaults
            || tool_name == DEFAULT_SEARCH_TOOL
            || tool_name == DEFAULT_READ_TOOL
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

        vec![
            // MemorySearch — like IndexSearch
            McpTool {
                name: state.search_tool_name.clone(),
                description: Some(format!(
                    "Search past conversation memories. Returns results with source labels \
                     and line numbers. Use {}(label, offset) to read full context around hits. \
                     Pass ALL search questions as queries array in ONE call.",
                    state.read_tool_name
                )),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "query": {
                            "type": "string",
                            "description": "Single search query"
                        },
                        "queries": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Multiple search queries to batch"
                        },
                        "source": {
                            "type": "string",
                            "description": "Filter to a specific source (e.g., \"session/abc123\")"
                        },
                        "limit": {
                            "type": "number",
                            "description": "Max results per query (default: 3)"
                        }
                    }
                }),
            },
            // MemoryRead — like IndexRead
            McpTool {
                name: state.read_tool_name.clone(),
                description: Some(format!(
                    "Read the full content of a memory source. Use after {} to get complete \
                     context around a search hit. Supports offset and limit for pagination.",
                    state.search_tool_name
                )),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "label": {
                            "type": "string",
                            "description": "Source label from search results (e.g., \"session/abc123\")"
                        },
                        "offset": {
                            "type": "string",
                            "description": "Line offset to start reading from (e.g., \"5\" or \"5-2\")"
                        },
                        "limit": {
                            "type": "number",
                            "description": "Number of lines to return (default: 15)"
                        }
                    },
                    "required": ["label"]
                }),
            },
        ]
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
        tool_name: &str,
        arguments: Value,
        client_id: &str,
        _client_name: &str,
    ) -> VirtualToolCallResult {
        // Ensure client directory exists
        if let Err(e) = self.memory_service.ensure_client_dir(client_id) {
            return VirtualToolCallResult::ToolError(format!(
                "Failed to initialize memory directory: {}",
                e
            ));
        }

        let config = self.memory_service.config();
        let read_tool_name = derive_read_tool_name(&config.recall_tool_name);

        if tool_name == read_tool_name || tool_name == DEFAULT_READ_TOOL {
            self.handle_memory_read(arguments, client_id)
        } else {
            self.handle_memory_search(arguments, client_id)
        }
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
                "You have access to persistent memory from past conversations.\n\
                 Use {}(queries: [...]) to search memories. Results include source labels and line numbers.\n\
                 Use {}(label, offset, limit) to read full context around search hits.\n\
                 If you have access to a subagent or forked context, prefer using {} \
                 within a subagent to avoid polluting the main conversation with search results.",
                state.search_tool_name, state.read_tool_name, state.search_tool_name
            ),
            tool_names: vec![
                state.search_tool_name.clone(),
                state.read_tool_name.clone(),
            ],
            priority: 40,
        })
    }

    fn create_session_state(&self, client: &lr_config::Client) -> Box<dyn VirtualSessionState> {
        let config = self.memory_service.config();
        let search_name = config.recall_tool_name.clone();
        let read_name = derive_read_tool_name(&search_name);
        Box::new(MemorySessionState {
            enabled: client.memory_enabled.unwrap_or(false),
            search_tool_name: search_name,
            read_tool_name: read_name,
        })
    }

    fn update_session_state(
        &self,
        state: &mut dyn VirtualSessionState,
        client: &lr_config::Client,
    ) {
        if let Some(state) = state.as_any_mut().downcast_mut::<MemorySessionState>() {
            state.enabled = client.memory_enabled.unwrap_or(false);
            let config = self.memory_service.config();
            state.search_tool_name = config.recall_tool_name.clone();
            state.read_tool_name = derive_read_tool_name(&state.search_tool_name);
        }
    }

    fn all_tool_names(&self) -> Vec<String> {
        let config = self.memory_service.config();
        let search_name = config.recall_tool_name.clone();
        let read_name = derive_read_tool_name(&search_name);
        vec![search_name, read_name]
    }
}

impl MemoryVirtualServer {
    /// Handle MemorySearch tool call.
    fn handle_memory_search(&self, arguments: Value, client_id: &str) -> VirtualToolCallResult {
        let query = arguments
            .get("query")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let queries: Option<Vec<String>> =
            arguments
                .get("queries")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(|s| s.to_string()))
                        .collect()
                });
        let source = arguments
            .get("source")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let limit = arguments
            .get("limit")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize)
            .unwrap_or(DEFAULT_SEARCH_LIMIT);

        // Need at least one query
        if query.as_ref().map_or(true, |q| q.is_empty())
            && queries.as_ref().map_or(true, |qs| qs.is_empty())
        {
            return VirtualToolCallResult::ToolError(
                "At least one 'query' or 'queries' parameter is required".to_string(),
            );
        }

        let results = match self.memory_service.search_combined(
            client_id,
            query.as_deref(),
            queries.as_deref(),
            limit,
            source.as_deref(),
        ) {
            Ok(r) => r,
            Err(e) => {
                return VirtualToolCallResult::ToolError(format!("Memory search failed: {}", e));
            }
        };

        // Check if we got any hits
        let has_hits = results.iter().any(|r| !r.hits.is_empty());

        if has_hits {
            // Format using lr_context's Display (includes line numbers, source labels)
            let formatted =
                lr_context::format_search_results(&results, lr_context::SEARCH_OUTPUT_CAP);
            VirtualToolCallResult::Success(serde_json::json!({
                "content": [{ "type": "text", "text": formatted }]
            }))
        } else {
            // Fallback: return a summary of available memory sources
            self.build_summary_fallback(client_id, &results)
        }
    }

    /// Handle MemoryRead tool call.
    fn handle_memory_read(&self, arguments: Value, client_id: &str) -> VirtualToolCallResult {
        let label = match arguments.get("label").and_then(|v| v.as_str()) {
            Some(l) if !l.is_empty() => l,
            _ => {
                return VirtualToolCallResult::ToolError(
                    "Missing or empty 'label' parameter".to_string(),
                );
            }
        };

        let offset = arguments
            .get("offset")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let limit = arguments
            .get("limit")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize);

        match self
            .memory_service
            .read(client_id, label, offset.as_deref(), limit)
        {
            Ok(result) => {
                let formatted = result.to_string();
                VirtualToolCallResult::Success(serde_json::json!({
                    "content": [{ "type": "text", "text": formatted }]
                }))
            }
            Err(e) => VirtualToolCallResult::ToolError(format!("Read failed: {}", e)),
        }
    }

    /// Build a summary of available memory sources when search finds nothing.
    fn build_summary_fallback(
        &self,
        client_id: &str,
        results: &[lr_context::SearchResult],
    ) -> VirtualToolCallResult {
        // Show "no results" for the queries
        let mut output = String::new();
        for result in results {
            output.push_str(&format!("### No results for {:?}\n\n", result.query));
        }

        // List available sources as a summary
        match self.memory_service.list_sources(client_id) {
            Ok(sources) if !sources.is_empty() => {
                output.push_str("## Available memory sources\n\n");
                let mut total_lines = 0usize;
                let mut total_chunks = 0usize;
                for src in &sources {
                    output.push_str(&format!(
                        "- **{}** — {} lines, {} chunks\n",
                        src.label, src.total_lines, src.chunk_count,
                    ));
                    total_lines += src.total_lines;
                    total_chunks += src.chunk_count;
                }
                output.push_str(&format!(
                    "\n{} sources, {} total lines, {} total chunks.\n\
                     Try different search terms, or use read(label) to browse a source directly.",
                    sources.len(),
                    total_lines,
                    total_chunks,
                ));
            }
            Ok(_) => {
                output.push_str(
                    "No memories have been recorded yet for this client.\n\
                     Memories are captured automatically during conversations when memory is enabled.",
                );
            }
            Err(e) => {
                tracing::warn!("Failed to list memory sources for fallback: {}", e);
                output.push_str("No relevant memories found.");
            }
        }

        VirtualToolCallResult::Success(serde_json::json!({
            "content": [{ "type": "text", "text": output }]
        }))
    }
}

/// Derive the read tool name from the search tool name.
/// "MemorySearch" → "MemoryRead", "MemoryRecall" → "MemoryRead", etc.
fn derive_read_tool_name(search_name: &str) -> String {
    if search_name.ends_with("Search") {
        format!("{}Read", &search_name[..search_name.len() - "Search".len()])
    } else if search_name.ends_with("Recall") {
        format!("{}Read", &search_name[..search_name.len() - "Recall".len()])
    } else {
        format!("{}Read", search_name)
    }
}
