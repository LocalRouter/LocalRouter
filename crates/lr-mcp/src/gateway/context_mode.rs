//! Context Management virtual MCP server.
//!
//! Spawns a per-session context-mode STDIO process (via `npx -y context-mode`)
//! that provides FTS5 search, content indexing, and progressive catalog compression
//! to reduce context window consumption.

use std::any::Any;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::Mutex;

use super::gateway_tools::FirewallDecisionResult;
use super::types::NamespacedTool;
use super::virtual_server::*;
use crate::protocol::{JsonRpcRequest, McpTool};
use crate::transport::{StdioTransport, Transport};

/// Tool names owned by the context-mode virtual server.
const CTX_SEARCH: &str = "ctx_search";
const CTX_EXECUTE: &str = "ctx_execute";
const CTX_EXECUTE_FILE: &str = "ctx_execute_file";
const CTX_BATCH_EXECUTE: &str = "ctx_batch_execute";
const CTX_INDEX: &str = "ctx_index";
const CTX_FETCH_AND_INDEX: &str = "ctx_fetch_and_index";

/// Tools exposed only when indexing tools are enabled.
const INDEXING_TOOLS: &[&str] = &[
    CTX_EXECUTE,
    CTX_EXECUTE_FILE,
    CTX_BATCH_EXECUTE,
    CTX_INDEX,
    CTX_FETCH_AND_INDEX,
];

/// Tools filtered from context-mode's tools/list (managed via UI, not AI).
const FILTERED_TOOLS: &[&str] = &["ctx_stats", "ctx_doctor", "ctx_upgrade"];

/// MCP Gateway source label guide appended to ctx_search description.
const CTX_SEARCH_SOURCE_GUIDE: &str = r#"

MCP Gateway source labels (use with 'source' parameter):
  source="catalog:"                — search all MCP catalog entries (tools, resources, prompts, server docs)
  source="catalog:filesystem"      — search within a specific server (docs + all its items)
  source="catalog:filesystem__"    — search tools/resources/prompts from a specific server
  source="filesystem__read_file:"  — find all compressed responses from a specific tool
  source="filesystem__read_file:3" — find a specific invocation

Searching catalog entries automatically activates matching tools/resources/prompts for use."#;

/// Additional source guide appended when indexing tools are enabled.
const CTX_SEARCH_INDEXING_SOURCE_GUIDE: &str = r#"

Other indexed content (from ctx_execute, ctx_index, etc.):
  source="execute:"     — find auto-indexed output from ctx_execute
  source="batch:"       — find auto-indexed output from ctx_batch_execute
  (omit source to search everything)"#;

/// Additional source guide appended to ctx_search's `source` parameter description.
const CTX_SEARCH_SOURCE_PARAM_GUIDE: &str =
    r#" MCP examples: "catalog:" for all MCP entries, "catalog:filesystem" for one server, "filesystem__read_file:" for a tool's responses."#;

/// Atomic counter for generating unique JSON-RPC request IDs.
static REQUEST_ID: AtomicU64 = AtomicU64::new(1);

fn next_request_id() -> Value {
    Value::Number(REQUEST_ID.fetch_add(1, Ordering::Relaxed).into())
}

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
    /// Whether indexing tools are exposed.
    pub indexing_tools_enabled: bool,
    /// Lazy STDIO transport — spawned on first use.
    transport: Mutex<Option<StdioTransport>>,
    /// Cached tool definitions from the context-mode process.
    cached_tools: Mutex<Option<Vec<McpTool>>>,
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
    /// Get or spawn the STDIO transport for this session.
    pub async fn get_transport(&self) -> Result<tokio::sync::MutexGuard<'_, Option<StdioTransport>>, String> {
        let mut guard = self.transport.lock().await;
        if guard.is_none() {
            let transport = spawn_context_mode_process().await?;
            // Initialize the MCP connection
            initialize_context_mode(&transport).await?;
            *guard = Some(transport);
        }
        Ok(guard)
    }

    /// Send a tools/call request to the context-mode process.
    pub async fn call_tool(&self, tool_name: &str, arguments: Value) -> Result<Value, String> {
        let guard = self.get_transport().await?;
        let transport = guard.as_ref().ok_or("Transport not available")?;

        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(next_request_id()),
            method: "tools/call".to_string(),
            params: Some(json!({
                "name": tool_name,
                "arguments": arguments,
            })),
        };

        let response = transport
            .send_request(request)
            .await
            .map_err(|e| format!("context-mode tools/call failed: {e}"))?;

        if let Some(error) = response.error {
            return Err(format!(
                "context-mode error: {} (code: {})",
                error.message, error.code
            ));
        }

        Ok(response.result.unwrap_or(Value::Null))
    }

    /// Send a tools/list request to get tool definitions from context-mode.
    pub async fn list_remote_tools(&self) -> Result<Vec<McpTool>, String> {
        // Return cached tools if available
        {
            let cached = self.cached_tools.lock().await;
            if let Some(tools) = cached.as_ref() {
                return Ok(tools.clone());
            }
        }

        let guard = self.get_transport().await?;
        let transport = guard.as_ref().ok_or("Transport not available")?;

        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(next_request_id()),
            method: "tools/list".to_string(),
            params: None,
        };

        let response = transport
            .send_request(request)
            .await
            .map_err(|e| format!("context-mode tools/list failed: {e}"))?;

        if let Some(error) = response.error {
            return Err(format!(
                "context-mode tools/list error: {} (code: {})",
                error.message, error.code
            ));
        }

        let result = response.result.unwrap_or(Value::Null);
        let tools_value = result.get("tools").cloned().unwrap_or(Value::Array(vec![]));
        let tools: Vec<McpTool> =
            serde_json::from_value(tools_value).map_err(|e| format!("Failed to parse tools: {e}"))?;

        // Cache the tools
        *self.cached_tools.lock().await = Some(tools.clone());

        Ok(tools)
    }

    /// Get the next run ID for a given namespaced name (tool/resource/prompt).
    pub fn next_run_id(&mut self, namespaced_name: &str) -> u32 {
        let counter = self.run_counters.entry(namespaced_name.to_string()).or_insert(0);
        *counter += 1;
        *counter
    }
}

impl Clone for ContextModeSessionState {
    fn clone(&self) -> Self {
        Self {
            enabled: self.enabled,
            indexing_tools_enabled: self.indexing_tools_enabled,
            transport: Mutex::new(None), // Transport is not cloned — new session gets fresh transport
            cached_tools: Mutex::new(None),
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

/// Spawn a context-mode STDIO process via npx.
async fn spawn_context_mode_process() -> Result<StdioTransport, String> {
    let env = crate::manager::shell_env();

    // Use npx to run context-mode (auto-installs if needed)
    let command = "npx".to_string();
    let args = vec!["-y".to_string(), "context-mode".to_string()];

    tracing::info!("Spawning context-mode STDIO process");
    StdioTransport::spawn(command, args, env)
        .await
        .map_err(|e| format!("Failed to spawn context-mode: {e}"))
}

/// Initialize the MCP connection with the context-mode process.
async fn initialize_context_mode(transport: &StdioTransport) -> Result<(), String> {
    let request = JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        id: Some(next_request_id()),
        method: "initialize".to_string(),
        params: Some(json!({
            "protocolVersion": "2025-03-26",
            "capabilities": {},
            "clientInfo": {
                "name": "localrouter-context-mode",
                "version": "1.0.0"
            }
        })),
    };

    let response = transport
        .send_request(request)
        .await
        .map_err(|e| format!("context-mode initialize failed: {e}"))?;

    if let Some(error) = response.error {
        return Err(format!(
            "context-mode initialize error: {} (code: {})",
            error.message, error.code
        ));
    }

    tracing::info!("context-mode initialized successfully");
    Ok(())
}

/// Build the list of tools to expose from context-mode, applying filtering and description injection.
fn build_context_mode_tools(
    remote_tools: &[McpTool],
    indexing_tools_enabled: bool,
) -> Vec<McpTool> {
    let mut tools = Vec::new();

    for tool in remote_tools {
        // Filter out stats/doctor/upgrade tools
        if FILTERED_TOOLS.contains(&tool.name.as_str()) {
            continue;
        }

        // Filter out indexing tools if not enabled
        if !indexing_tools_enabled && INDEXING_TOOLS.contains(&tool.name.as_str()) {
            continue;
        }

        let mut tool = tool.clone();

        // Inject MCP source label guide into ctx_search description
        if tool.name == CTX_SEARCH {
            if let Some(ref desc) = tool.description {
                let mut new_desc = desc.clone();
                new_desc.push_str(CTX_SEARCH_SOURCE_GUIDE);
                if indexing_tools_enabled {
                    new_desc.push_str(CTX_SEARCH_INDEXING_SOURCE_GUIDE);
                }
                tool.description = Some(new_desc);
            }

            // Inject source parameter description
            if let Some(properties) = tool.input_schema.get_mut("properties") {
                if let Some(source) = properties.get_mut("source") {
                    if let Some(desc) = source.get("description").and_then(|d| d.as_str()) {
                        let new_desc = format!("{desc}{CTX_SEARCH_SOURCE_PARAM_GUIDE}");
                        source["description"] = Value::String(new_desc);
                    }
                }
            }
        }

        tools.push(tool);
    }

    tools
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
        tool_name == CTX_SEARCH || INDEXING_TOOLS.contains(&tool_name)
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

        // Try to use cached tools; if not yet available, return a minimal ctx_search definition
        let cached = state.cached_tools.try_lock();
        if let Ok(guard) = cached {
            if let Some(ref tools) = *guard {
                return build_context_mode_tools(tools, state.indexing_tools_enabled);
            }
        }

        // Fallback: return a static ctx_search definition before transport is initialized
        vec![build_fallback_ctx_search_tool(state.indexing_tools_enabled)]
    }

    fn check_permissions(
        &self,
        _state: &dyn VirtualSessionState,
        _tool_name: &str,
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

        // Forward the tool call to the context-mode STDIO process
        match state.call_tool(tool_name, arguments).await {
            Ok(result) => {
                if tool_name == CTX_SEARCH {
                    // Post-process search results for catalog activation
                    let activated = extract_catalog_activations(&result, &state.catalog_sources, &state.activated_tools, &state.activated_resources, &state.activated_prompts);

                    if activated.is_empty() {
                        VirtualToolCallResult::Success(result)
                    } else {
                        // Append activation message to result
                        let mut modified_result = result.clone();
                        let activation_msg = format!(
                            "\n\n---\nActivated: {}\nThese items are now available for use.",
                            activated.join(", ")
                        );
                        append_text_to_mcp_result(&mut modified_result, &activation_msg);

                        // Build state updater to mark items as activated
                        let activated_clone = activated.clone();
                        let state_update: Box<dyn FnOnce(&mut dyn super::virtual_server::VirtualSessionState) + Send> =
                            Box::new(move |s| {
                                if let Some(cm) = s.as_any_mut().downcast_mut::<ContextModeSessionState>() {
                                    for name in &activated_clone {
                                        cm.activated_tools.insert(name.clone());
                                        cm.activated_resources.insert(name.clone());
                                        cm.activated_prompts.insert(name.clone());
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
                } else {
                    VirtualToolCallResult::Success(result)
                }
            }
            Err(e) => VirtualToolCallResult::ToolError(e),
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

        // Context-mode's prompting lives entirely in tool descriptions,
        // so we only need a minimal instructions section
        None
    }

    fn create_session_state(&self, client: &lr_config::Client) -> Box<dyn VirtualSessionState> {
        let config = self.config.read().unwrap();
        let enabled = client.is_context_management_enabled(&config);

        Box::new(ContextModeSessionState {
            enabled,
            indexing_tools_enabled: enabled && config.indexing_tools,
            transport: Mutex::new(None),
            cached_tools: Mutex::new(None),
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
        state.indexing_tools_enabled = state.enabled && config.indexing_tools;
        state.catalog_threshold_bytes = config.catalog_threshold_bytes;
        state.response_threshold_bytes = config.response_threshold_bytes;
    }
}

/// Extract catalog items that should be activated based on ctx_search results.
/// Parses source labels from the result text and identifies newly activatable items.
fn extract_catalog_activations(
    result: &Value,
    catalog_sources: &HashMap<String, CatalogItemType>,
    activated_tools: &HashSet<String>,
    activated_resources: &HashSet<String>,
    activated_prompts: &HashSet<String>,
) -> Vec<String> {
    let mut newly_activated = Vec::new();

    // Extract text content from MCP result
    let text = result
        .get("content")
        .and_then(|c| c.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|item| item.get("text").and_then(|t| t.as_str()))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default();

    // Parse source labels from result text: --- [source_label] ---
    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(label) = trimmed
            .strip_prefix("--- [")
            .and_then(|s| s.strip_suffix("] ---"))
        {
            // Check if this is a catalog source that needs activation
            if let Some(item_type) = catalog_sources.get(label) {
                // Extract the namespaced name from the source label (strip "catalog:" prefix)
                let name = label.strip_prefix("catalog:").unwrap_or(label);
                let already_active = match item_type {
                    CatalogItemType::Tool => activated_tools.contains(name),
                    CatalogItemType::Resource => activated_resources.contains(name),
                    CatalogItemType::Prompt => activated_prompts.contains(name),
                    CatalogItemType::ServerWelcome => true, // Server welcome doesn't need activation
                };
                if !already_active {
                    newly_activated.push(name.to_string());
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

/// Build a fallback ctx_search tool definition for use before the transport is initialized.
pub fn build_fallback_ctx_search_tool(indexing_tools_enabled: bool) -> McpTool {
    let mut description = "Search indexed content. Pass ALL search questions as queries array in ONE call.\n\nTIPS: 2-4 specific terms per query. Use 'source' to scope results.".to_string();
    description.push_str(CTX_SEARCH_SOURCE_GUIDE);
    if indexing_tools_enabled {
        description.push_str(CTX_SEARCH_INDEXING_SOURCE_GUIDE);
    }

    let mut source_desc = "Filter to a specific indexed source (partial match).".to_string();
    source_desc.push_str(CTX_SEARCH_SOURCE_PARAM_GUIDE);

    McpTool {
        name: CTX_SEARCH.to_string(),
        description: Some(description),
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_mcp_tool(name: &str, desc: &str) -> McpTool {
        McpTool {
            name: name.to_string(),
            description: Some(desc.to_string()),
            input_schema: json!({"type": "object"}),
        }
    }

    // ── build_context_mode_tools tests ──────────────────────────────

    #[test]
    fn test_filters_stats_doctor_upgrade() {
        let tools = vec![
            make_mcp_tool("ctx_search", "Search"),
            make_mcp_tool("ctx_execute", "Execute"),
            make_mcp_tool("ctx_stats", "Stats"),
            make_mcp_tool("ctx_doctor", "Doctor"),
            make_mcp_tool("ctx_upgrade", "Upgrade"),
        ];

        let result = build_context_mode_tools(&tools, true);
        let names: Vec<&str> = result.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"ctx_search"));
        assert!(names.contains(&"ctx_execute"));
        assert!(!names.contains(&"ctx_stats"));
        assert!(!names.contains(&"ctx_doctor"));
        assert!(!names.contains(&"ctx_upgrade"));
    }

    #[test]
    fn test_filters_indexing_tools_when_disabled() {
        let tools = vec![
            make_mcp_tool("ctx_search", "Search"),
            make_mcp_tool("ctx_execute", "Execute"),
            make_mcp_tool("ctx_execute_file", "Execute file"),
            make_mcp_tool("ctx_batch_execute", "Batch"),
            make_mcp_tool("ctx_index", "Index"),
            make_mcp_tool("ctx_fetch_and_index", "Fetch"),
        ];

        let result = build_context_mode_tools(&tools, false);
        let names: Vec<&str> = result.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"ctx_search"));
        assert!(!names.contains(&"ctx_execute"));
        assert!(!names.contains(&"ctx_execute_file"));
        assert!(!names.contains(&"ctx_batch_execute"));
        assert!(!names.contains(&"ctx_index"));
        assert!(!names.contains(&"ctx_fetch_and_index"));
    }

    #[test]
    fn test_shows_indexing_tools_when_enabled() {
        let tools = vec![
            make_mcp_tool("ctx_search", "Search"),
            make_mcp_tool("ctx_execute", "Execute"),
            make_mcp_tool("ctx_execute_file", "Execute file"),
            make_mcp_tool("ctx_batch_execute", "Batch"),
            make_mcp_tool("ctx_index", "Index"),
            make_mcp_tool("ctx_fetch_and_index", "Fetch"),
        ];

        let result = build_context_mode_tools(&tools, true);
        assert_eq!(result.len(), 6);
    }

    #[test]
    fn test_injects_source_guide_into_ctx_search() {
        let tools = vec![McpTool {
            name: "ctx_search".to_string(),
            description: Some("Base description".to_string()),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "source": {
                        "type": "string",
                        "description": "Source filter"
                    }
                }
            }),
        }];

        let result = build_context_mode_tools(&tools, false);
        let search = &result[0];
        let desc = search.description.as_ref().unwrap();
        assert!(desc.contains("MCP Gateway source labels"));
        assert!(desc.contains("catalog:"));
        // Should NOT have indexing guide when disabled
        assert!(!desc.contains("ctx_execute"));

        // With indexing enabled
        let result = build_context_mode_tools(&tools, true);
        let search = &result[0];
        let desc = search.description.as_ref().unwrap();
        assert!(desc.contains("ctx_execute"));
    }

    #[test]
    fn test_injects_source_param_description() {
        let tools = vec![McpTool {
            name: "ctx_search".to_string(),
            description: Some("Search".to_string()),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "source": {
                        "type": "string",
                        "description": "Filter to source"
                    }
                }
            }),
        }];

        let result = build_context_mode_tools(&tools, false);
        let source_desc = result[0].input_schema["properties"]["source"]["description"]
            .as_str()
            .unwrap();
        assert!(source_desc.contains("MCP examples:"));
    }

    // ── extract_catalog_activations tests ───────────────────────────

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
        sources.insert(
            "catalog:db__users".to_string(),
            CatalogItemType::Resource,
        );
        sources.insert(
            "catalog:db__query".to_string(),
            CatalogItemType::Prompt,
        );
        sources.insert(
            "catalog:filesystem".to_string(),
            CatalogItemType::ServerWelcome,
        );
        sources
    }

    #[test]
    fn test_activates_tools_from_search_results() {
        let sources = make_catalog_sources();
        let result = json!({
            "content": [{
                "type": "text",
                "text": "Results:\n--- [catalog:filesystem__read_file] ---\nRead file content\n--- [catalog:filesystem__write_file] ---\nWrite file content"
            }]
        });

        let activated = extract_catalog_activations(
            &result,
            &sources,
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
        );

        assert_eq!(activated.len(), 2);
        assert!(activated.contains(&"filesystem__read_file".to_string()));
        assert!(activated.contains(&"filesystem__write_file".to_string()));
    }

    #[test]
    fn test_skips_already_activated_tools() {
        let sources = make_catalog_sources();
        let result = json!({
            "content": [{
                "type": "text",
                "text": "--- [catalog:filesystem__read_file] ---\nContent"
            }]
        });

        let mut activated_tools = HashSet::new();
        activated_tools.insert("filesystem__read_file".to_string());

        let activated = extract_catalog_activations(
            &result,
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
        let result = json!({
            "content": [{
                "type": "text",
                "text": "--- [catalog:db__users] ---\nUser table\n--- [catalog:db__query] ---\nQuery prompt"
            }]
        });

        let activated = extract_catalog_activations(
            &result,
            &sources,
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
        );

        assert_eq!(activated.len(), 2);
        assert!(activated.contains(&"db__users".to_string()));
        assert!(activated.contains(&"db__query".to_string()));
    }

    #[test]
    fn test_server_welcome_not_activated() {
        let sources = make_catalog_sources();
        let result = json!({
            "content": [{
                "type": "text",
                "text": "--- [catalog:filesystem] ---\nServer docs"
            }]
        });

        let activated = extract_catalog_activations(
            &result,
            &sources,
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
        );

        // ServerWelcome items should not be activated (always_active = true in logic)
        assert!(activated.is_empty());
    }

    #[test]
    fn test_ignores_non_catalog_labels() {
        let sources = make_catalog_sources();
        let result = json!({
            "content": [{
                "type": "text",
                "text": "--- [execute:abc123] ---\nSome output\n--- [unknown_label] ---\nOther"
            }]
        });

        let activated = extract_catalog_activations(
            &result,
            &sources,
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
        );

        assert!(activated.is_empty());
    }

    #[test]
    fn test_handles_empty_result() {
        let sources = make_catalog_sources();
        let result = json!({"content": []});

        let activated = extract_catalog_activations(
            &result,
            &sources,
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
        );

        assert!(activated.is_empty());
    }

    #[test]
    fn test_handles_missing_content() {
        let sources = make_catalog_sources();
        let result = json!({});

        let activated = extract_catalog_activations(
            &result,
            &sources,
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
        );

        assert!(activated.is_empty());
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
        // Should not crash, result unchanged
        assert!(result.get("content").is_none());
    }

    // ── build_fallback_ctx_search_tool tests ────────────────────────

    #[test]
    fn test_fallback_tool_structure() {
        let tool = build_fallback_ctx_search_tool(false);
        assert_eq!(tool.name, "ctx_search");
        assert!(tool.description.as_ref().unwrap().contains("Search indexed content"));
        assert!(tool.input_schema["properties"]["queries"].is_object());
        assert!(tool.input_schema["properties"]["source"].is_object());
        assert!(tool.input_schema["properties"]["limit"].is_object());
    }

    #[test]
    fn test_fallback_tool_with_indexing() {
        let tool = build_fallback_ctx_search_tool(true);
        let desc = tool.description.as_ref().unwrap();
        assert!(desc.contains("ctx_execute"));
        assert!(desc.contains("batch:"));
    }

    #[test]
    fn test_fallback_tool_without_indexing() {
        let tool = build_fallback_ctx_search_tool(false);
        let desc = tool.description.as_ref().unwrap();
        assert!(!desc.contains("ctx_execute"));
    }

    // ── ContextModeSessionState tests ───────────────────────────────

    #[test]
    fn test_next_run_id_increments() {
        let mut state = ContextModeSessionState {
            enabled: true,
            indexing_tools_enabled: false,
            transport: Mutex::new(None),
            cached_tools: Mutex::new(None),
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
}
