//! Context Management virtual MCP server.
//!
//! Uses the native `lr-context` ContentStore for FTS5 search, content indexing,
//! and progressive catalog compression to reduce context window consumption.

use std::any::Any;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use lr_context::{
    format_search_results, ContentStore, SearchResult, READ_DEFAULT_LIMIT, SEARCH_OUTPUT_CAP,
};

use super::gateway_tools::FirewallDecisionResult;
use super::types::NamespacedTool;
use super::virtual_server::*;
use crate::protocol::McpTool;

/// Legacy tool name constants — used as fallbacks when no config is available.
const CTX_SEARCH_DEFAULT: &str = "IndexSearch";
const INDEX_READ_DEFAULT: &str = "IndexRead";

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
    /// Snapshotted search tool name (e.g. "IndexSearch").
    pub search_tool_name: String,
    /// Snapshotted read tool name (e.g. "IndexRead").
    pub read_tool_name: String,
    /// Snapshotted gateway indexing permissions.
    pub gateway_indexing: lr_config::GatewayIndexingPermissions,
    /// Snapshotted client tools indexing default.
    pub client_tools_indexing_default: lr_config::IndexingState,
    /// Snapshotted per-client tools indexing overrides.
    pub client_tools_indexing: Option<lr_config::ClientToolsIndexingPermissions>,
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
            search_tool_name: self.search_tool_name.clone(),
            read_tool_name: self.read_tool_name.clone(),
            gateway_indexing: self.gateway_indexing.clone(),
            client_tools_indexing_default: self.client_tools_indexing_default.clone(),
            client_tools_indexing: self.client_tools_indexing.clone(),
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

/// Build the tool definitions for the native context-mode server using configured names.
fn build_native_tools_with_names(search_name: &str, read_name: &str) -> Vec<McpTool> {
    let mut search_desc = format!("Search indexed content. Pass ALL search questions as queries array in ONE call.\n\nTIPS: 2-4 specific terms per query. Use 'source' to scope results.");
    search_desc.push_str(CTX_SEARCH_SOURCE_GUIDE);

    let mut source_desc = "Filter to a specific indexed source (partial match).".to_string();
    source_desc.push_str(CTX_SEARCH_SOURCE_PARAM_GUIDE);

    vec![
        McpTool {
            name: search_name.to_string(),
            description: Some(search_desc),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "A single search query string."
                    },
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
            name: read_name.to_string(),
            description: Some(
                format!("Read the full content of an indexed source. Use after {} to get complete context around a search hit.", search_name),
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
                        "description": format!("Number of lines to return (default: {})", READ_DEFAULT_LIMIT)
                    }
                },
                "required": ["label"]
            }),
        },
    ]
}

/// Build the tool definitions using default names.
fn build_native_tools() -> Vec<McpTool> {
    build_native_tools_with_names(CTX_SEARCH_DEFAULT, INDEX_READ_DEFAULT)
}

/// Build a fallback ctx_search tool definition (public for coding agents UI).
pub fn build_fallback_ctx_search_tool() -> McpTool {
    build_native_tools().into_iter().next().unwrap()
}

/// Build all native tool definitions (public for coding agents UI).
pub fn build_native_tool_definitions() -> Vec<McpTool> {
    build_native_tools()
}

/// Build native tool definitions with configured names.
pub fn build_native_tool_definitions_with_names(search_name: &str, read_name: &str) -> Vec<McpTool> {
    build_native_tools_with_names(search_name, read_name)
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
        let config = self.config.read().unwrap();
        tool_name == config.search_tool_name || tool_name == config.read_tool_name
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

        build_native_tools_with_names(&state.search_tool_name, &state.read_tool_name)
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

        // ContentStore uses parking_lot::Mutex internally — run blocking calls
        // off the async executor to avoid stalling the tokio runtime.
        let store = state.store.clone();
        let catalog_sources = state.catalog_sources.clone();
        let activated_tools = state.activated_tools.clone();
        let activated_resources = state.activated_resources.clone();
        let activated_prompts = state.activated_prompts.clone();
        let tool = tool_name.to_string();
        let search_name = state.search_tool_name.clone();
        let read_name = state.read_tool_name.clone();

        let result = tokio::task::spawn_blocking(move || {
            if tool == search_name {
                handle_ctx_search_blocking(
                    &store,
                    arguments,
                    &catalog_sources,
                    &activated_tools,
                    &activated_resources,
                    &activated_prompts,
                )
            } else if tool == read_name {
                handle_index_read_blocking(&store, arguments)
            } else {
                VirtualToolCallResult::NotHandled
            }
        })
        .await;

        match result {
            Ok(r) => r,
            Err(e) => {
                VirtualToolCallResult::ToolError(format!("Tool call panicked: {}", e))
            }
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
            content: format!(
                "Use {} to discover MCP capabilities and retrieve compressed content. Use {} to read full indexed sources.",
                state.search_tool_name, state.read_tool_name
            ),
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
            search_tool_name: config.search_tool_name.clone(),
            read_tool_name: config.read_tool_name.clone(),
            gateway_indexing: config.gateway_indexing.clone(),
            client_tools_indexing_default: config.client_tools_indexing_default.clone(),
            client_tools_indexing: client.client_tools_indexing.clone(),
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
        state.search_tool_name = config.search_tool_name.clone();
        state.read_tool_name = config.read_tool_name.clone();
        state.gateway_indexing = config.gateway_indexing.clone();
        state.client_tools_indexing_default = config.client_tools_indexing_default.clone();
        state.client_tools_indexing = client.client_tools_indexing.clone();
    }

    fn is_tool_indexable(&self, _tool_name: &str) -> bool {
        // Search/read tools are the indexing system itself — never index their responses
        false
    }
}

/// Handle ctx_search tool call using native ContentStore (runs on blocking thread).
fn handle_ctx_search_blocking(
    store: &ContentStore,
    arguments: Value,
    catalog_sources: &HashMap<String, CatalogItemType>,
    activated_tools: &HashSet<String>,
    activated_resources: &HashSet<String>,
    activated_prompts: &HashSet<String>,
) -> VirtualToolCallResult {
    // Parse arguments — accept both `query` (string) and `queries` (array), both optional
    let query = arguments
        .get("query")
        .and_then(|q| q.as_str())
        .map(|s| s.to_string());

    let queries: Option<Vec<String>> = arguments
        .get("queries")
        .and_then(|q| serde_json::from_value(q.clone()).ok());

    let source = arguments
        .get("source")
        .and_then(|s| s.as_str())
        .map(|s| s.to_string());

    let limit = arguments
        .get("limit")
        .and_then(|l| l.as_u64())
        .unwrap_or(3) as usize;

    // Execute search using search_combined (handles both query + queries)
    let results = match store.search_combined(
        query.as_deref(),
        queries.as_deref(),
        limit,
        source.as_deref(),
    ) {
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
        catalog_sources,
        activated_tools,
        activated_resources,
        activated_prompts,
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

/// Handle ctx_read tool call using native ContentStore (runs on blocking thread).
fn handle_index_read_blocking(store: &ContentStore, arguments: Value) -> VirtualToolCallResult {
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

    match store.read(&label, offset.as_deref(), limit) {
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

/// Compress and index a client tool response into the session's ContentStore.
///
/// Used by MCP via LLM orchestrator to index eligible client tool results.
/// Returns the compressed text if indexing was performed, or None if skipped.
pub fn compress_client_tool_response(
    store: &ContentStore,
    tool_name: &str,
    run_id: u32,
    full_text: &str,
    response_threshold_bytes: usize,
    search_tool_name: &str,
) -> Option<String> {
    if full_text.len() <= response_threshold_bytes {
        return None;
    }

    let source = format!("__client__{}:{}", tool_name, run_id);
    let byte_size = full_text.len();

    if let Err(e) = store.index(&source, full_text) {
        tracing::warn!(
            "Failed to index client tool response for {} ({}): {}",
            tool_name,
            source,
            e
        );
        return None;
    }

    let preview_bytes = (response_threshold_bytes / 8).clamp(200, 500);
    let preview = &full_text[..full_text
        .char_indices()
        .take_while(|(i, _)| *i < preview_bytes)
        .last()
        .map(|(i, c)| i + c.len_utf8())
        .unwrap_or(full_text.len())
        .min(full_text.len())];

    let compressed = format!(
        "[Response compressed — {} bytes indexed as {}]\n\n{}\n\nFull output indexed. \
         Use {}(queries=[\"your search terms\"], source=\"{}\") to retrieve specific sections.",
        byte_size, source, preview, search_tool_name, source
    );

    tracing::info!(
        "Compressed client tool response for {} ({} bytes → {} bytes, source={})",
        tool_name,
        byte_size,
        compressed.len(),
        source
    );

    Some(compressed)
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
        assert_eq!(tools[0].name, "IndexSearch");
        assert_eq!(tools[1].name, "IndexRead");

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
            search_tool_name: "IndexSearch".to_string(),
            read_tool_name: "IndexRead".to_string(),
            gateway_indexing: lr_config::GatewayIndexingPermissions::default(),
            client_tools_indexing_default: lr_config::IndexingState::Enable,
            client_tools_indexing: None,
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
        assert!(tools.iter().any(|t| t.name == "IndexSearch"));
        assert!(tools.iter().any(|t| t.name == "IndexRead"));
    }

    // ── ctx_search schema tests ─────────────────────────────────────

    #[test]
    fn test_ctx_search_schema_has_both_query_and_queries() {
        let tools = build_native_tools();
        let search_tool = tools.iter().find(|t| t.name == "IndexSearch").unwrap();
        let props = &search_tool.input_schema["properties"];
        assert!(props["query"]["type"].as_str() == Some("string"), "IndexSearch should have a 'query' string parameter");
        assert!(props["queries"]["type"].as_str() == Some("array"), "IndexSearch should have a 'queries' array parameter");
        // Neither is required — both are optional
        assert!(search_tool.input_schema.get("required").is_none(), "IndexSearch should not have required fields");
    }

    #[test]
    fn test_ctx_read_schema_uses_read_default_limit_constant() {
        let tools = build_native_tools();
        let read_tool = tools.iter().find(|t| t.name == "IndexRead").unwrap();
        let limit_desc = read_tool.input_schema["properties"]["limit"]["description"]
            .as_str()
            .unwrap();
        let expected = format!("(default: {})", lr_context::READ_DEFAULT_LIMIT);
        assert!(
            limit_desc.contains(&expected),
            "IndexRead limit description should reference READ_DEFAULT_LIMIT ({}), got: {}",
            lr_context::READ_DEFAULT_LIMIT,
            limit_desc,
        );
    }

    // ── handle_ctx_search_blocking tests ────────────────────────────

    fn make_store_with_content() -> Arc<ContentStore> {
        let store = Arc::new(ContentStore::new().unwrap());
        store
            .index("catalog:filesystem__read_file", "Read file from disk with path parameter")
            .unwrap();
        store
            .index("catalog:filesystem__write_file", "Write content to a file on disk")
            .unwrap();
        store
            .index("catalog:db__users", "Database users table resource")
            .unwrap();
        store
            .index("response:tool1:1", "The quick brown fox jumps over the lazy dog")
            .unwrap();
        store
    }

    #[test]
    fn test_ctx_search_with_queries_array() {
        let store = make_store_with_content();
        let result = handle_ctx_search_blocking(
            &store,
            json!({"queries": ["read file"]}),
            &HashMap::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
        );

        match result {
            VirtualToolCallResult::Success(v) => {
                let text = v["content"][0]["text"].as_str().unwrap();
                assert!(!text.is_empty(), "Search should return results");
            }
            VirtualToolCallResult::SuccessWithSideEffects { response, .. } => {
                let text = response["content"][0]["text"].as_str().unwrap();
                assert!(!text.is_empty(), "Search should return results");
            }
            other => panic!("Expected Success, got: {:?}", format!("{:?}", std::mem::discriminant(&other))),
        }
    }

    #[test]
    fn test_ctx_search_with_single_query_string() {
        let store = make_store_with_content();
        let result = handle_ctx_search_blocking(
            &store,
            json!({"query": "read file"}),
            &HashMap::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
        );

        match result {
            VirtualToolCallResult::Success(v) | VirtualToolCallResult::SuccessWithSideEffects { response: v, .. } => {
                let text = v["content"][0]["text"].as_str().unwrap();
                assert!(!text.is_empty(), "Single query search should return results");
            }
            other => panic!("Expected Success, got: {:?}", format!("{:?}", std::mem::discriminant(&other))),
        }
    }

    #[test]
    fn test_ctx_search_with_both_query_and_queries() {
        let store = make_store_with_content();
        let result = handle_ctx_search_blocking(
            &store,
            json!({"query": "read file", "queries": ["write content"]}),
            &HashMap::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
        );

        match result {
            VirtualToolCallResult::Success(v) | VirtualToolCallResult::SuccessWithSideEffects { response: v, .. } => {
                let text = v["content"][0]["text"].as_str().unwrap();
                assert!(!text.is_empty(), "Combined search should return results");
            }
            other => panic!("Expected Success, got: {:?}", format!("{:?}", std::mem::discriminant(&other))),
        }
    }

    #[test]
    fn test_ctx_search_no_query_or_queries_returns_error() {
        let store = make_store_with_content();
        let result = handle_ctx_search_blocking(
            &store,
            json!({}),
            &HashMap::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
        );

        match result {
            VirtualToolCallResult::ToolError(msg) => {
                assert!(msg.contains("query"), "Error should mention missing query: {}", msg);
            }
            _ => panic!("Expected ToolError for empty arguments"),
        }
    }

    #[test]
    fn test_ctx_search_with_source_filter() {
        let store = make_store_with_content();
        let result = handle_ctx_search_blocking(
            &store,
            json!({"queries": ["content"], "source": "response:"}),
            &HashMap::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
        );

        match result {
            VirtualToolCallResult::Success(v) | VirtualToolCallResult::SuccessWithSideEffects { response: v, .. } => {
                let text = v["content"][0]["text"].as_str().unwrap();
                // Source filter "response:" should only match the response:tool1:1 entry
                assert!(text.contains("response:tool1:1") || text.contains("brown fox") || text.contains("No results"),
                    "Source filter should scope results to response: entries");
            }
            other => panic!("Expected Success, got: {:?}", format!("{:?}", std::mem::discriminant(&other))),
        }
    }

    #[test]
    fn test_ctx_search_activates_catalog_items() {
        let store = make_store_with_content();
        let result = handle_ctx_search_blocking(
            &store,
            json!({"queries": ["read file disk"]}),
            &make_catalog_sources(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
        );

        match result {
            VirtualToolCallResult::SuccessWithSideEffects { response, state_update, .. } => {
                let text = response["content"].as_array().unwrap();
                assert!(text.len() >= 2, "Should have results + activation message");
                let activation_text = text.last().unwrap()["text"].as_str().unwrap();
                assert!(activation_text.contains("Activated"), "Should show activation: {}", activation_text);
                assert!(state_update.is_some(), "Should have state updater");
            }
            _ => panic!("Expected SuccessWithSideEffects with catalog activation"),
        }
    }

    // ── handle_index_read_blocking tests ────────────────────────────

    #[test]
    fn test_ctx_read_returns_indexed_content() {
        let store = make_store_with_content();
        let result = handle_index_read_blocking(
            &store,
            json!({"label": "response:tool1:1"}),
        );

        match result {
            VirtualToolCallResult::Success(v) => {
                let text = v["content"][0]["text"].as_str().unwrap();
                assert!(text.contains("brown fox"), "Should return the indexed content");
            }
            _ => panic!("Expected Success for read"),
        }
    }

    #[test]
    fn test_ctx_read_missing_label_returns_error() {
        let store = make_store_with_content();
        let result = handle_index_read_blocking(&store, json!({}));

        match result {
            VirtualToolCallResult::ToolError(msg) => {
                assert!(msg.contains("label"), "Error should mention missing label: {}", msg);
            }
            _ => panic!("Expected ToolError for missing label"),
        }
    }

    #[test]
    fn test_ctx_read_nonexistent_label_returns_error() {
        let store = make_store_with_content();
        let result = handle_index_read_blocking(
            &store,
            json!({"label": "nonexistent:label"}),
        );

        match result {
            VirtualToolCallResult::ToolError(msg) => {
                assert!(msg.contains("Read failed"), "Should report read failure: {}", msg);
            }
            _ => panic!("Expected ToolError for nonexistent label"),
        }
    }

    #[test]
    fn test_ctx_read_with_offset_and_limit() {
        let store = Arc::new(ContentStore::new().unwrap());
        store
            .index("multiline", "line one\nline two\nline three\nline four\nline five")
            .unwrap();

        let result = handle_index_read_blocking(
            &store,
            json!({"label": "multiline", "offset": "2", "limit": 2}),
        );

        match result {
            VirtualToolCallResult::Success(v) => {
                let text = v["content"][0]["text"].as_str().unwrap();
                assert!(text.contains("line two"), "Should start from offset 2: {}", text);
            }
            _ => panic!("Expected Success for read with offset"),
        }
    }

    // ── handle_tool_call async integration tests ────────────────────

    #[tokio::test]
    async fn test_handle_tool_call_ctx_search_via_spawn_blocking() {
        let config = lr_config::ContextManagementConfig {
            enabled: true,
            ..Default::default()
        };
        let vs = ContextModeVirtualServer::new(config);
        let client =
            lr_config::Client::new_with_strategy("test".to_string(), "strat-1".to_string());
        let state = vs.create_session_state(&client);

        // Index some content first
        let cm = state
            .as_any()
            .downcast_ref::<ContextModeSessionState>()
            .unwrap();
        cm.store.index("test:doc", "Rust programming language features").unwrap();

        let result = vs
            .handle_tool_call(
                state,
                "IndexSearch",
                json!({"queries": ["Rust programming"]}),
                "test-client",
                "Test Client",
            )
            .await;

        match result {
            VirtualToolCallResult::Success(v) | VirtualToolCallResult::SuccessWithSideEffects { response: v, .. } => {
                let text = v["content"][0]["text"].as_str().unwrap();
                assert!(!text.is_empty(), "Async search should return results");
            }
            VirtualToolCallResult::ToolError(e) => panic!("Unexpected error: {}", e),
            _ => panic!("Unexpected result type"),
        }
    }

    #[tokio::test]
    async fn test_handle_tool_call_ctx_read_via_spawn_blocking() {
        let config = lr_config::ContextManagementConfig {
            enabled: true,
            ..Default::default()
        };
        let vs = ContextModeVirtualServer::new(config);
        let client =
            lr_config::Client::new_with_strategy("test".to_string(), "strat-1".to_string());
        let state = vs.create_session_state(&client);

        let cm = state
            .as_any()
            .downcast_ref::<ContextModeSessionState>()
            .unwrap();
        cm.store.index("test:doc", "Hello world content").unwrap();

        let result = vs
            .handle_tool_call(
                state,
                "IndexRead",
                json!({"label": "test:doc"}),
                "test-client",
                "Test Client",
            )
            .await;

        match result {
            VirtualToolCallResult::Success(v) => {
                let text = v["content"][0]["text"].as_str().unwrap();
                assert!(text.contains("Hello world"), "Async read should return content: {}", text);
            }
            VirtualToolCallResult::ToolError(e) => panic!("Unexpected error: {}", e),
            _ => panic!("Unexpected result type"),
        }
    }

    #[tokio::test]
    async fn test_handle_tool_call_disabled_returns_error() {
        let config = lr_config::ContextManagementConfig {
            enabled: false,
            ..Default::default()
        };
        let vs = ContextModeVirtualServer::new(config);
        let client =
            lr_config::Client::new_with_strategy("test".to_string(), "strat-1".to_string());
        let state = vs.create_session_state(&client);

        let result = vs
            .handle_tool_call(
                state,
                "IndexSearch",
                json!({"queries": ["test"]}),
                "test-client",
                "Test Client",
            )
            .await;

        match result {
            VirtualToolCallResult::ToolError(msg) => {
                assert!(msg.contains("not enabled"), "Should report not enabled: {}", msg);
            }
            _ => panic!("Expected ToolError for disabled client"),
        }
    }

    #[tokio::test]
    async fn test_handle_tool_call_unknown_tool_returns_not_handled() {
        let config = lr_config::ContextManagementConfig {
            enabled: true,
            ..Default::default()
        };
        let vs = ContextModeVirtualServer::new(config);
        let client =
            lr_config::Client::new_with_strategy("test".to_string(), "strat-1".to_string());
        let state = vs.create_session_state(&client);

        let result = vs
            .handle_tool_call(
                state,
                "unknown_tool",
                json!({}),
                "test-client",
                "Test Client",
            )
            .await;

        matches!(result, VirtualToolCallResult::NotHandled);
    }
}
