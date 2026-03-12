//! Context Management virtual MCP server.
//!
//! Uses the native `lr-context` ContentStore for FTS5 search, content indexing,
//! and progressive catalog compression to reduce context window consumption.

use std::any::Any;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use lr_context::{format_search_results, ContentStore, SearchResult, SEARCH_OUTPUT_CAP};

use super::gateway_tools::FirewallDecisionResult;
use super::types::NamespacedTool;
use super::virtual_server::*;
use crate::protocol::McpTool;

/// Tool names owned by the context-mode virtual server.
const CTX_SEARCH: &str = "ctx_search";
const INDEX_READ: &str = "ctx_read";

/// MCP Gateway source label guide appended to ctx_search description.
const CTX_SEARCH_SOURCE_GUIDE: &str = r#"

MCP Gateway source labels (use with 'source' parameter):
  source="catalog:"                — search all MCP catalog entries (tools, resources, prompts, server docs)
  source="catalog:filesystem"      — search within a specific server (docs + all its items)
  source="catalog:filesystem__"    — search tools/resources/prompts from a specific server
  source="filesystem__read_file:"  — find all compressed responses from a specific tool
  source="filesystem__read_file:3" — find a specific invocation

Searching catalog entries automatically activates matching tools/resources/prompts for use."#;

/// Additional source guide appended to ctx_search's `source` parameter description.
const CTX_SEARCH_SOURCE_PARAM_GUIDE: &str = r#" MCP examples: "catalog:" for all MCP entries, "catalog:filesystem" for one server, "filesystem__read_file:" for a tool's responses."#;

/// The type of catalog item associated with a source label.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CatalogItemType {
    Tool,
    Resource,
    Prompt,
    ServerWelcome,
}

/// Virtual MCP server for context-mode integration.
///
/// No shared state — all state is per-session via `ContextModeSessionState`.
pub struct ContextModeVirtualServer {
    /// Global context management config (read at session creation time).
    config: std::sync::RwLock<lr_config::ContextManagementConfig>,
}

impl ContextModeVirtualServer {
    pub fn new(config: lr_config::ContextManagementConfig) -> Self {
        Self {
            config: std::sync::RwLock::new(config),
        }
    }

    /// Update the global config (called when settings change).
    pub fn update_config(&self, config: lr_config::ContextManagementConfig) {
        *self.config.write().unwrap() = config;
    }
}

/// Per-session state for context-mode.
pub struct ContextModeSessionState {
    /// Whether this client has context management enabled.
    pub enabled: bool,
    /// Whether catalog compression (deferral) is enabled.
    pub catalog_compression_enabled: bool,
    /// Native content store (shared via Arc for cheap cloning).
    pub store: Arc<ContentStore>,
    /// Catalog source labels → item type (for activation on ctx_search).
    pub catalog_sources: HashMap<String, CatalogItemType>,
    /// Per-tool/resource/prompt response run ID counters.
    pub run_counters: HashMap<String, u32>,
    /// Full tool catalog (for search-based activation).
    pub full_tool_catalog: Vec<NamespacedTool>,
    /// Activated tools (subset of full_tool_catalog made visible).
    pub activated_tools: HashSet<String>,
    /// Full resource catalog (for search-based activation).
    pub full_resource_catalog: Vec<super::types::NamespacedResource>,
    /// Activated resources.
    pub activated_resources: HashSet<String>,
    /// Full prompt catalog (for search-based activation).
    pub full_prompt_catalog: Vec<super::types::NamespacedPrompt>,
    /// Activated prompts.
    pub activated_prompts: HashSet<String>,
    /// Catalog threshold in bytes.
    pub catalog_threshold_bytes: usize,
    /// Response threshold in bytes.
    pub response_threshold_bytes: usize,
}

impl ContextModeSessionState {
    /// Get the next run ID for a given namespaced name (tool/resource/prompt).
    pub fn next_run_id(&mut self, namespaced_name: &str) -> u32 {
        let counter = self
            .run_counters
            .entry(namespaced_name.to_string())
            .or_insert(0);
        *counter += 1;
        *counter
    }
}

impl Clone for ContextModeSessionState {
    fn clone(&self) -> Self {
        Self {
            enabled: self.enabled,
            catalog_compression_enabled: self.catalog_compression_enabled,
            store: self.store.clone(), // Arc clone — shares same ContentStore
            catalog_sources: self.catalog_sources.clone(),
            run_counters: self.run_counters.clone(),
            full_tool_catalog: self.full_tool_catalog.clone(),
            activated_tools: self.activated_tools.clone(),
            full_resource_catalog: self.full_resource_catalog.clone(),
            activated_resources: self.activated_resources.clone(),
            full_prompt_catalog: self.full_prompt_catalog.clone(),
            activated_prompts: self.activated_prompts.clone(),
            catalog_threshold_bytes: self.catalog_threshold_bytes,
            response_threshold_bytes: self.response_threshold_bytes,
        }
    }
}

impl VirtualSessionState for ContextModeSessionState {
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

/// Build the static tool definitions for the native context-mode server.
fn build_native_tools() -> Vec<McpTool> {
    let mut search_desc = "Search indexed content. Pass ALL search questions as queries array in ONE call.\n\nTIPS: 2-4 specific terms per query. Use 'source' to scope results.".to_string();
    search_desc.push_str(CTX_SEARCH_SOURCE_GUIDE);

    let mut source_desc = "Filter to a specific indexed source (partial match).".to_string();
    source_desc.push_str(CTX_SEARCH_SOURCE_PARAM_GUIDE);

    vec![
        McpTool {
            name: CTX_SEARCH.to_string(),
            description: Some(search_desc),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "queries": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Array of search queries. Batch ALL questions in one call."
                    },
                    "source": {
                        "type": "string",
                        "description": source_desc
                    },
                    "limit": {
                        "type": "number",
                        "description": "Results per query (default: 3)"
                    }
                }
            }),
        },
        McpTool {
            name: INDEX_READ.to_string(),
            description: Some(
                "Read the full content of an indexed source. Use after ctx_search to get complete context around a search hit."
                    .to_string(),
            ),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "label": {
                        "type": "string",
                        "description": "Source label to read (from search results)"
                    },
                    "offset": {
                        "type": "string",
                        "description": "Line offset to start from (e.g. \"5\" or \"5-2\" for sub-line). Default: start of content."
                    },
                    "limit": {
                        "type": "number",
                        "description": "Number of lines to return (default: ~100)"
                    }
                },
                "required": ["label"]
            }),
        },
    ]
}

/// Build a fallback ctx_search tool definition (public for coding agents UI).
pub fn build_fallback_ctx_search_tool() -> McpTool {
    build_native_tools().into_iter().next().unwrap()
}

/// Build all native tool definitions (public for coding agents UI).
pub fn build_native_tool_definitions() -> Vec<McpTool> {
    build_native_tools()
}

#[async_trait]
impl VirtualMcpServer for ContextModeVirtualServer {
    fn id(&self) -> &str {
        "_context_mode"
    }

    fn display_name(&self) -> &str {
        "Context Management"
    }

    fn owns_tool(&self, tool_name: &str) -> bool {
        tool_name == CTX_SEARCH || tool_name == INDEX_READ
    }

    fn is_enabled(&self, client: &lr_config::Client) -> bool {
        let config = self.config.read().unwrap();
        client.is_context_management_enabled(&config)
    }

    fn list_tools(&self, state: &dyn VirtualSessionState) -> Vec<McpTool> {
        let state = state
            .as_any()
            .downcast_ref::<ContextModeSessionState>()
            .expect("wrong state type for ContextModeVirtualServer");

        if !state.enabled {
            return Vec::new();
        }

        build_native_tools()
    }

    fn check_permissions(
        &self,
        _state: &dyn VirtualSessionState,
        _tool_name: &str,
        _arguments: Option<&Value>,
        _session_approved: bool,
        _session_denied: bool,
    ) -> VirtualFirewallResult {
        // Context-mode tools are always allowed — no firewall check needed
        VirtualFirewallResult::Handled(FirewallDecisionResult::Proceed)
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
            .downcast_ref::<ContextModeSessionState>()
            .expect("wrong state type for ContextModeVirtualServer");

        if !state.enabled {
            return VirtualToolCallResult::ToolError(
                "Context management is not enabled for this client".to_string(),
            );
        }

        match tool_name {
            CTX_SEARCH => self.handle_ctx_search(state, arguments),
            INDEX_READ => self.handle_index_read(state, arguments),
            _ => VirtualToolCallResult::NotHandled,
        }
    }

    fn build_instructions(&self, state: &dyn VirtualSessionState) -> Option<VirtualInstructions> {
        let state = state
            .as_any()
            .downcast_ref::<ContextModeSessionState>()
            .expect("wrong state type for ContextModeVirtualServer");

        if !state.enabled {
            return None;
        }

        Some(VirtualInstructions {
            section_title: "Context Management".to_string(),
            content:
                "Use ctx_search to discover MCP capabilities and retrieve compressed content. Use ctx_read to read full indexed sources."
                    .to_string(),
            tool_names: Vec::new(), // populated by gateway
            priority: 0,
        })
    }

    fn create_session_state(&self, client: &lr_config::Client) -> Box<dyn VirtualSessionState> {
        let config = self.config.read().unwrap();
        let enabled = client.is_context_management_enabled(&config);

        let store = Arc::new(
            ContentStore::new().expect("Failed to create in-memory ContentStore"),
        );

        Box::new(ContextModeSessionState {
            enabled,
            catalog_compression_enabled: enabled && client.is_catalog_compression_enabled(&config),
            store,
            catalog_sources: HashMap::new(),
            run_counters: HashMap::new(),
            full_tool_catalog: Vec::new(),
            activated_tools: HashSet::new(),
            full_resource_catalog: Vec::new(),
            activated_resources: HashSet::new(),
            full_prompt_catalog: Vec::new(),
            activated_prompts: HashSet::new(),
            catalog_threshold_bytes: config.catalog_threshold_bytes,
            response_threshold_bytes: config.response_threshold_bytes,
        })
    }

    fn update_session_state(
        &self,
        state: &mut dyn VirtualSessionState,
        client: &lr_config::Client,
    ) {
        let config = self.config.read().unwrap();
        let state = state
            .as_any_mut()
            .downcast_mut::<ContextModeSessionState>()
            .expect("wrong state type for ContextModeVirtualServer");

        state.enabled = client.is_context_management_enabled(&config);
        state.catalog_compression_enabled =
            state.enabled && client.is_catalog_compression_enabled(&config);
        state.catalog_threshold_bytes = config.catalog_threshold_bytes;
        state.response_threshold_bytes = config.response_threshold_bytes;
    }
}

impl ContextModeVirtualServer {
    /// Handle ctx_search tool call using native ContentStore.
    fn handle_ctx_search(
        &self,
        state: &ContextModeSessionState,
        arguments: Value,
    ) -> VirtualToolCallResult {
        // Parse arguments
        let queries: Vec<String> = arguments
            .get("queries")
            .and_then(|q| serde_json::from_value(q.clone()).ok())
            .unwrap_or_default();

        if queries.is_empty() {
            return VirtualToolCallResult::ToolError(
                "Missing required parameter: queries (array of search strings)".to_string(),
            );
        }

        let source = arguments
            .get("source")
            .and_then(|s| s.as_str())
            .map(|s| s.to_string());

        let limit = arguments
            .get("limit")
            .and_then(|l| l.as_u64())
            .unwrap_or(3) as usize;

        // Execute search using native store
        let results = match state.store.search(&queries, limit, source.as_deref()) {
            Ok(results) => results,
            Err(e) => {
                return VirtualToolCallResult::ToolError(format!("Search failed: {}", e));
            }
        };

        // Format results
        let formatted = format_search_results(&results, SEARCH_OUTPUT_CAP);
        let result = json!({
            "content": [{
                "type": "text",
                "text": formatted,
            }]
        });

        // Extract catalog activations from search results
        let activated = extract_catalog_activations_from_results(
            &results,
            &state.catalog_sources,
            &state.activated_tools,
            &state.activated_resources,
            &state.activated_prompts,
        );

        if activated.is_empty() {
            VirtualToolCallResult::Success(result)
        } else {
            // Append activation message to result
            let mut modified_result = result;
            let names: Vec<&str> = activated.iter().map(|(n, _)| n.as_str()).collect();
            let activation_msg = format!(
                "\n\n---\nActivated: {}\nThese items are now available for use.",
                names.join(", ")
            );
            append_text_to_mcp_result(&mut modified_result, &activation_msg);

            // Build state updater to mark items as activated by their correct type
            let activated_clone = activated.clone();
            let state_update: Box<
                dyn FnOnce(&mut dyn super::virtual_server::VirtualSessionState) + Send,
            > = Box::new(move |s| {
                if let Some(cm) = s.as_any_mut().downcast_mut::<ContextModeSessionState>() {
                    for (name, item_type) in &activated_clone {
                        match item_type {
                            CatalogItemType::Tool => {
                                cm.activated_tools.insert(name.clone());
                            }
                            CatalogItemType::Resource => {
                                cm.activated_resources.insert(name.clone());
                            }
                            CatalogItemType::Prompt => {
                                cm.activated_prompts.insert(name.clone());
                            }
                            CatalogItemType::ServerWelcome => {} // No activation needed
                        }
                    }
                }
            });

            VirtualToolCallResult::SuccessWithSideEffects {
                response: modified_result,
                invalidate_cache: true,
                send_list_changed: true,
                state_update: Some(state_update),
            }
        }
    }

    /// Handle ctx_read tool call using native ContentStore.
    fn handle_index_read(
        &self,
        state: &ContextModeSessionState,
        arguments: Value,
    ) -> VirtualToolCallResult {
        let label = match arguments.get("label").and_then(|l| l.as_str()) {
            Some(l) => l.to_string(),
            None => {
                return VirtualToolCallResult::ToolError(
                    "Missing required parameter: label".to_string(),
                );
            }
        };

        let offset = arguments
            .get("offset")
            .and_then(|o| o.as_str())
            .map(|s| s.to_string());

        let limit = arguments
            .get("limit")
            .and_then(|l| l.as_u64())
            .map(|l| l as usize);

        match state.store.read(&label, offset.as_deref(), limit) {
            Ok(read_result) => {
                let formatted = read_result.to_string();
                VirtualToolCallResult::Success(json!({
                    "content": [{
                        "type": "text",
                        "text": formatted,
                    }]
                }))
            }
            Err(e) => VirtualToolCallResult::ToolError(format!("Read failed: {}", e)),
        }
    }
}

/// Extract catalog items that should be activated based on search results.
/// Uses direct access to SearchResult/SearchHit structs rather than text parsing.
fn extract_catalog_activations_from_results(
    results: &[SearchResult],
    catalog_sources: &HashMap<String, CatalogItemType>,
    activated_tools: &HashSet<String>,
    activated_resources: &HashSet<String>,
    activated_prompts: &HashSet<String>,
) -> Vec<(String, CatalogItemType)> {
    let mut newly_activated = Vec::new();
    let mut seen = HashSet::new();

    for result in results {
        for hit in &result.hits {
            // The hit.source contains the source label (e.g. "catalog:filesystem__read_file")
            let source_label = &hit.source;

            if let Some(item_type) = catalog_sources.get(source_label) {
                // Extract the namespaced name from the source label (strip "catalog:" prefix)
                let name = source_label.strip_prefix("catalog:").unwrap_or(source_label);

                if seen.contains(name) {
                    continue;
                }
                seen.insert(name.to_string());

                let already_active = match item_type {
                    CatalogItemType::Tool => activated_tools.contains(name),
                    CatalogItemType::Resource => activated_resources.contains(name),
                    CatalogItemType::Prompt => activated_prompts.contains(name),
                    CatalogItemType::ServerWelcome => true, // Server welcome doesn't need activation
                };
                if !already_active {
                    newly_activated.push((name.to_string(), item_type.clone()));
                }
            }
        }
    }

    newly_activated
}

/// Append text to an MCP tool result's content array.
fn append_text_to_mcp_result(result: &mut Value, text: &str) {
    if let Some(content) = result.get_mut("content").and_then(|c| c.as_array_mut()) {
        content.push(json!({
            "type": "text",
            "text": text,
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── extract_catalog_activations_from_results tests ────────────────

    fn make_catalog_sources() -> HashMap<String, CatalogItemType> {
        let mut sources = HashMap::new();
        sources.insert(
            "catalog:filesystem__read_file".to_string(),
            CatalogItemType::Tool,
        );
        sources.insert(
            "catalog:filesystem__write_file".to_string(),
            CatalogItemType::Tool,
        );
        sources.insert("catalog:db__users".to_string(), CatalogItemType::Resource);
        sources.insert("catalog:db__query".to_string(), CatalogItemType::Prompt);
        sources.insert(
            "catalog:filesystem".to_string(),
            CatalogItemType::ServerWelcome,
        );
        sources
    }

    fn make_search_result(hits: Vec<(&str, &str)>) -> SearchResult {
        use lr_context::{ContentType, MatchLayer, SearchHit};
        SearchResult {
            query: "test".to_string(),
            hits: hits
                .into_iter()
                .map(|(source, title)| SearchHit {
                    title: title.to_string(),
                    content: "test content".to_string(),
                    source: source.to_string(),
                    rank: -1.0,
                    content_type: ContentType::Prose,
                    match_layer: MatchLayer::Porter,
                    line_start: 1,
                    line_end: 10,
                })
                .collect(),
            corrected_query: None,
        }
    }

    #[test]
    fn test_activates_tools_from_search_results() {
        let sources = make_catalog_sources();
        let results = vec![make_search_result(vec![
            ("catalog:filesystem__read_file", "Read File"),
            ("catalog:filesystem__write_file", "Write File"),
        ])];

        let activated = extract_catalog_activations_from_results(
            &results,
            &sources,
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
        );

        assert_eq!(activated.len(), 2);
        let names: Vec<&str> = activated.iter().map(|(n, _)| n.as_str()).collect();
        assert!(names.contains(&"filesystem__read_file"));
        assert!(names.contains(&"filesystem__write_file"));
        for (_, item_type) in &activated {
            assert_eq!(*item_type, CatalogItemType::Tool);
        }
    }

    #[test]
    fn test_skips_already_activated_tools() {
        let sources = make_catalog_sources();
        let results = vec![make_search_result(vec![(
            "catalog:filesystem__read_file",
            "Read File",
        )])];

        let mut activated_tools = HashSet::new();
        activated_tools.insert("filesystem__read_file".to_string());

        let activated = extract_catalog_activations_from_results(
            &results,
            &sources,
            &activated_tools,
            &HashSet::new(),
            &HashSet::new(),
        );

        assert!(activated.is_empty());
    }

    #[test]
    fn test_activates_resources_and_prompts() {
        let sources = make_catalog_sources();
        let results = vec![make_search_result(vec![
            ("catalog:db__users", "Users"),
            ("catalog:db__query", "Query"),
        ])];

        let activated = extract_catalog_activations_from_results(
            &results,
            &sources,
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
        );

        assert_eq!(activated.len(), 2);
        let resource = activated.iter().find(|(n, _)| n == "db__users").unwrap();
        assert_eq!(resource.1, CatalogItemType::Resource);
        let prompt = activated.iter().find(|(n, _)| n == "db__query").unwrap();
        assert_eq!(prompt.1, CatalogItemType::Prompt);
    }

    #[test]
    fn test_server_welcome_not_activated() {
        let sources = make_catalog_sources();
        let results = vec![make_search_result(vec![(
            "catalog:filesystem",
            "Filesystem",
        )])];

        let activated = extract_catalog_activations_from_results(
            &results,
            &sources,
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
        );

        assert!(activated.is_empty());
    }

    #[test]
    fn test_ignores_non_catalog_sources() {
        let sources = make_catalog_sources();
        let results = vec![make_search_result(vec![
            ("execute:abc123", "Exec Output"),
            ("unknown_label", "Unknown"),
        ])];

        let activated = extract_catalog_activations_from_results(
            &results,
            &sources,
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
        );

        assert!(activated.is_empty());
    }

    #[test]
    fn test_handles_empty_results() {
        let sources = make_catalog_sources();
        let activated = extract_catalog_activations_from_results(
            &[],
            &sources,
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
        );

        assert!(activated.is_empty());
    }

    #[test]
    fn test_deduplicates_across_results() {
        let sources = make_catalog_sources();
        let results = vec![
            make_search_result(vec![("catalog:filesystem__read_file", "Read File")]),
            make_search_result(vec![("catalog:filesystem__read_file", "Read File Again")]),
        ];

        let activated = extract_catalog_activations_from_results(
            &results,
            &sources,
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
        );

        assert_eq!(activated.len(), 1);
    }

    // ── append_text_to_mcp_result tests ─────────────────────────────

    #[test]
    fn test_appends_text_to_result() {
        let mut result = json!({
            "content": [{"type": "text", "text": "original"}]
        });
        append_text_to_mcp_result(&mut result, "\nappended");

        let content = result["content"].as_array().unwrap();
        assert_eq!(content.len(), 2);
        assert_eq!(content[1]["text"].as_str().unwrap(), "\nappended");
    }

    #[test]
    fn test_append_no_content_is_noop() {
        let mut result = json!({"other": "field"});
        append_text_to_mcp_result(&mut result, "text");
        assert!(result.get("content").is_none());
    }

    // ── tool definition tests ────────────────────────────────────────

    #[test]
    fn test_native_tool_definitions() {
        let tools = build_native_tools();
        assert_eq!(tools.len(), 2);
        assert_eq!(tools[0].name, "ctx_search");
        assert_eq!(tools[1].name, "ctx_read");

        let search_desc = tools[0].description.as_ref().unwrap();
        assert!(search_desc.contains("Search indexed content"));
        assert!(search_desc.contains("MCP Gateway source labels"));
        assert!(search_desc.contains("catalog:"));

        let read_desc = tools[1].description.as_ref().unwrap();
        assert!(read_desc.contains("Read the full content"));
    }

    // ── ContextModeSessionState tests ───────────────────────────────

    #[test]
    fn test_next_run_id_increments() {
        let mut state = ContextModeSessionState {
            enabled: true,
            catalog_compression_enabled: true,
            store: Arc::new(ContentStore::new().unwrap()),
            catalog_sources: HashMap::new(),
            run_counters: HashMap::new(),
            full_tool_catalog: Vec::new(),
            activated_tools: HashSet::new(),
            full_resource_catalog: Vec::new(),
            activated_resources: HashSet::new(),
            full_prompt_catalog: Vec::new(),
            activated_prompts: HashSet::new(),
            catalog_threshold_bytes: 8192,
            response_threshold_bytes: 4096,
        };

        assert_eq!(state.next_run_id("fs__read_file"), 1);
        assert_eq!(state.next_run_id("fs__read_file"), 2);
        assert_eq!(state.next_run_id("fs__write_file"), 1);
        assert_eq!(state.next_run_id("fs__read_file"), 3);
    }

    #[test]
    fn test_session_state_cm_enabled_compression_disabled() {
        let config = lr_config::ContextManagementConfig {
            enabled: true,
            catalog_compression: false,
            ..Default::default()
        };
        let vs = ContextModeVirtualServer::new(config);
        let mut client =
            lr_config::Client::new_with_strategy("test".to_string(), "strat-1".to_string());
        client.context_management_enabled = None;
        client.catalog_compression_enabled = None;

        let state = vs.create_session_state(&client);
        let cm = state
            .as_any()
            .downcast_ref::<ContextModeSessionState>()
            .unwrap();
        assert!(cm.enabled);
        assert!(!cm.catalog_compression_enabled);
    }

    #[test]
    fn test_session_state_client_override_disables_compression() {
        let config = lr_config::ContextManagementConfig {
            enabled: true,
            catalog_compression: true,
            ..Default::default()
        };
        let vs = ContextModeVirtualServer::new(config);
        let mut client =
            lr_config::Client::new_with_strategy("test".to_string(), "strat-1".to_string());
        client.context_management_enabled = None;
        client.catalog_compression_enabled = Some(false);

        let state = vs.create_session_state(&client);
        let cm = state
            .as_any()
            .downcast_ref::<ContextModeSessionState>()
            .unwrap();
        assert!(cm.enabled);
        assert!(!cm.catalog_compression_enabled);
    }

    #[test]
    fn test_session_state_client_override_enables_compression() {
        let config = lr_config::ContextManagementConfig {
            enabled: true,
            catalog_compression: false,
            ..Default::default()
        };
        let vs = ContextModeVirtualServer::new(config);
        let mut client =
            lr_config::Client::new_with_strategy("test".to_string(), "strat-1".to_string());
        client.context_management_enabled = None;
        client.catalog_compression_enabled = Some(true);

        let state = vs.create_session_state(&client);
        let cm = state
            .as_any()
            .downcast_ref::<ContextModeSessionState>()
            .unwrap();
        assert!(cm.enabled);
        assert!(cm.catalog_compression_enabled);
    }

    #[test]
    fn test_session_state_cm_disabled_disables_all() {
        let config = lr_config::ContextManagementConfig {
            enabled: true,
            catalog_compression: true,
            ..Default::default()
        };
        let vs = ContextModeVirtualServer::new(config);
        let mut client =
            lr_config::Client::new_with_strategy("test".to_string(), "strat-1".to_string());
        client.context_management_enabled = Some(false);

        let state = vs.create_session_state(&client);
        let cm = state
            .as_any()
            .downcast_ref::<ContextModeSessionState>()
            .unwrap();
        assert!(!cm.enabled);
        assert!(!cm.catalog_compression_enabled);
    }

    #[test]
    fn test_update_session_state_reflects_config_change() {
        let config = lr_config::ContextManagementConfig {
            enabled: true,
            catalog_compression: true,
            ..Default::default()
        };
        let vs = ContextModeVirtualServer::new(config);
        let mut client =
            lr_config::Client::new_with_strategy("test".to_string(), "strat-1".to_string());

        let mut state = vs.create_session_state(&client);
        let cm = state
            .as_any()
            .downcast_ref::<ContextModeSessionState>()
            .unwrap();
        assert!(cm.catalog_compression_enabled);

        client.catalog_compression_enabled = Some(false);
        vs.update_session_state(state.as_mut(), &client);

        let cm = state
            .as_any()
            .downcast_ref::<ContextModeSessionState>()
            .unwrap();
        assert!(!cm.catalog_compression_enabled);
    }

    #[test]
    fn test_list_tools_disabled_returns_empty() {
        let config = lr_config::ContextManagementConfig {
            enabled: false,
            ..Default::default()
        };
        let vs = ContextModeVirtualServer::new(config);
        let client =
            lr_config::Client::new_with_strategy("test".to_string(), "strat-1".to_string());

        let state = vs.create_session_state(&client);
        let tools = vs.list_tools(state.as_ref());
        assert!(tools.is_empty());
    }

    #[test]
    fn test_list_tools_enabled_returns_search_and_read() {
        let config = lr_config::ContextManagementConfig {
            enabled: true,
            catalog_compression: false,
            ..Default::default()
        };
        let vs = ContextModeVirtualServer::new(config);
        let client =
            lr_config::Client::new_with_strategy("test".to_string(), "strat-1".to_string());

        let state = vs.create_session_state(&client);
        let tools = vs.list_tools(state.as_ref());
        assert_eq!(tools.len(), 2);
        assert!(tools.iter().any(|t| t.name == "ctx_search"));
        assert!(tools.iter().any(|t| t.name == "ctx_read"));
    }
}
