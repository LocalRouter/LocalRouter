use dashmap::DashMap;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

use crate::mcp::manager::McpServerManager;
use crate::mcp::protocol::{JsonRpcRequest, JsonRpcResponse, McpPrompt, McpResource, McpTool};
use crate::utils::errors::{AppError, AppResult};

use super::deferred::{create_search_tool, search_prompts, search_resources, search_tools};
use super::merger::{merge_initialize_results, merge_prompts, merge_resources, merge_tools};
use super::router::{broadcast_request, separate_results, should_broadcast};
use super::session::GatewaySession;
use super::types::*;

/// MCP Gateway - Unified endpoint for multiple MCP servers
pub struct McpGateway {
    /// Active sessions (client_id -> session)
    sessions: Arc<DashMap<String, Arc<RwLock<GatewaySession>>>>,

    /// MCP server manager
    server_manager: Arc<McpServerManager>,

    /// Gateway configuration
    config: GatewayConfig,
}

impl McpGateway {
    /// Create a new MCP gateway
    pub fn new(server_manager: Arc<McpServerManager>, config: GatewayConfig) -> Self {
        Self {
            sessions: Arc::new(DashMap::new()),
            server_manager,
            config,
        }
    }

    /// Handle an MCP gateway request
    pub async fn handle_request(
        &self,
        client_id: &str,
        allowed_servers: Vec<String>,
        enable_deferred_loading: bool,
        request: JsonRpcRequest,
    ) -> AppResult<JsonRpcResponse> {
        // Get or create session
        let session: Arc<RwLock<GatewaySession>> = self
            .get_or_create_session(client_id, allowed_servers, enable_deferred_loading)
            .await?;

        // Update last activity
        {
            let mut session_write = session.write().await;
            session_write.touch();
        }

        // Route based on method
        if should_broadcast(&request.method) {
            self.handle_broadcast_request(session, request).await
        } else {
            self.handle_direct_request(session, request).await
        }
    }

    /// Get or create a session
    async fn get_or_create_session(
        &self,
        client_id: &str,
        allowed_servers: Vec<String>,
        enable_deferred_loading: bool,
    ) -> AppResult<Arc<RwLock<GatewaySession>>> {
        // Check if session exists
        if let Some(session) = self.sessions.get(client_id) {
            let session_read = session.read().await;

            // Check if expired
            if session_read.is_expired() {
                drop(session_read);
                drop(session);

                // Remove expired session
                self.sessions.remove(client_id);
            } else {
                drop(session_read);
                return Ok(session.clone());
            }
        }

        // Create new session
        let ttl = Duration::from_secs(self.config.session_ttl_seconds);
        let mut session_data =
            GatewaySession::new(client_id.to_string(), allowed_servers.clone(), ttl);

        // Initialize deferred loading if enabled
        if enable_deferred_loading {
            // Ensure servers are started
            for server_id in &allowed_servers {
                if !self.server_manager.is_running(server_id) {
                    self.server_manager.start_server(server_id).await?;
                }
            }

            // Fetch full catalog from all servers
            let tools = self
                .fetch_and_merge_tools(
                    &allowed_servers,
                    JsonRpcRequest::new(Some(serde_json::json!(1)), "tools/list".to_string(), None),
                )
                .await
                .unwrap_or_default();

            let resources = self
                .fetch_and_merge_resources(
                    &allowed_servers,
                    JsonRpcRequest::new(
                        Some(serde_json::json!(2)),
                        "resources/list".to_string(),
                        None,
                    ),
                )
                .await
                .unwrap_or_default();

            let prompts = self
                .fetch_and_merge_prompts(
                    &allowed_servers,
                    JsonRpcRequest::new(
                        Some(serde_json::json!(3)),
                        "prompts/list".to_string(),
                        None,
                    ),
                )
                .await
                .unwrap_or_default();

            session_data.deferred_loading = Some(DeferredLoadingState {
                enabled: true,
                activated_tools: std::collections::HashSet::new(),
                full_catalog: tools,
                activated_resources: std::collections::HashSet::new(),
                full_resource_catalog: resources,
                activated_prompts: std::collections::HashSet::new(),
                full_prompt_catalog: prompts,
            });

            tracing::info!(
                "Initialized deferred loading for client {}: {} tools, {} resources, {} prompts",
                client_id,
                session_data
                    .deferred_loading
                    .as_ref()
                    .map(|d| d.full_catalog.len())
                    .unwrap_or(0),
                session_data
                    .deferred_loading
                    .as_ref()
                    .map(|d| d.full_resource_catalog.len())
                    .unwrap_or(0),
                session_data
                    .deferred_loading
                    .as_ref()
                    .map(|d| d.full_prompt_catalog.len())
                    .unwrap_or(0),
            );
        }

        let session = Arc::new(RwLock::new(session_data));

        // Register notification handlers for each server
        self.register_notification_handlers(&session, &allowed_servers)
            .await;

        self.sessions.insert(client_id.to_string(), session.clone());

        Ok(session)
    }

    /// Register notification handlers for a session
    async fn register_notification_handlers(
        &self,
        session: &Arc<RwLock<GatewaySession>>,
        allowed_servers: &[String],
    ) {
        for server_id in allowed_servers {
            let session_clone = session.clone();
            let server_id_clone = server_id.clone();

            // Register notification handler
            self.server_manager.on_notification(
                server_id,
                Arc::new(move |_, notification| {
                    // Handle cache invalidation notifications
                    match notification.method.as_str() {
                        "notifications/tools/list_changed" => {
                            tracing::info!(
                                "Received tools/list_changed notification from server: {}",
                                server_id_clone
                            );
                            // Invalidate tools cache
                            if let Ok(mut session_write) = session_clone.try_write() {
                                session_write.cached_tools = None;
                                tracing::debug!("Invalidated tools cache for session");
                            }
                        }
                        "notifications/resources/list_changed" => {
                            tracing::info!(
                                "Received resources/list_changed notification from server: {}",
                                server_id_clone
                            );
                            // Invalidate resources cache
                            if let Ok(mut session_write) = session_clone.try_write() {
                                session_write.cached_resources = None;
                                tracing::debug!("Invalidated resources cache for session");
                            }
                        }
                        "notifications/prompts/list_changed" => {
                            tracing::info!(
                                "Received prompts/list_changed notification from server: {}",
                                server_id_clone
                            );
                            // Invalidate prompts cache
                            if let Ok(mut session_write) = session_clone.try_write() {
                                session_write.cached_prompts = None;
                                tracing::debug!("Invalidated prompts cache for session");
                            }
                        }
                        other_method => {
                            tracing::debug!(
                                "Received notification from server {}: {}",
                                server_id_clone,
                                other_method
                            );
                            // Other notifications are logged but not acted upon
                        }
                    }
                }),
            );
        }
    }

    /// Handle broadcast request (initialize, tools/list, etc.)
    async fn handle_broadcast_request(
        &self,
        session: Arc<RwLock<GatewaySession>>,
        request: JsonRpcRequest,
    ) -> AppResult<JsonRpcResponse> {
        match request.method.as_str() {
            "initialize" => self.handle_initialize(session, request).await,
            "tools/list" => self.handle_tools_list(session, request).await,
            "resources/list" => self.handle_resources_list(session, request).await,
            "prompts/list" => self.handle_prompts_list(session, request).await,
            "logging/setLevel" | "ping" => {
                // These are broadcast but we don't merge results
                self.broadcast_and_return_first(session, request).await
            }
            _ => Err(AppError::Mcp(format!(
                "Unknown broadcast method: {}",
                request.method
            ))),
        }
    }

    /// Handle direct request (tools/call, resources/read, etc.)
    async fn handle_direct_request(
        &self,
        session: Arc<RwLock<GatewaySession>>,
        request: JsonRpcRequest,
    ) -> AppResult<JsonRpcResponse> {
        match request.method.as_str() {
            "tools/call" => self.handle_tools_call(session, request).await,
            "resources/read" => self.handle_resources_read(session, request).await,
            "prompts/get" => self.handle_prompts_get(session, request).await,
            _ => {
                // For unknown methods, send to first available server and return response
                // This allows clients to use custom/future MCP methods
                let session_read = session.read().await;
                let allowed_servers = session_read.allowed_servers.clone();
                drop(session_read);

                if allowed_servers.is_empty() {
                    return Err(AppError::Mcp(
                        "No servers available to handle method".to_string(),
                    ));
                }

                // Send to first server in the list
                let server_id = &allowed_servers[0];

                // Ensure server is started
                if !self.server_manager.is_running(server_id) {
                    self.server_manager.start_server(server_id).await?;
                }

                // Send request and return response (including errors)
                self.server_manager.send_request(server_id, request).await
            }
        }
    }

    /// Handle initialize request
    async fn handle_initialize(
        &self,
        session: Arc<RwLock<GatewaySession>>,
        request: JsonRpcRequest,
    ) -> AppResult<JsonRpcResponse> {
        let session_read = session.read().await;
        let allowed_servers = session_read.allowed_servers.clone();
        drop(session_read);

        // Ensure all servers are started
        for server_id in &allowed_servers {
            if !self.server_manager.is_running(server_id) {
                self.server_manager.start_server(server_id).await?;
            }
        }

        // Broadcast initialize to all servers
        let timeout = Duration::from_secs(self.config.server_timeout_seconds);
        let max_retries = self.config.max_retry_attempts;

        let results = broadcast_request(
            &allowed_servers,
            request.clone(),
            &self.server_manager,
            timeout,
            max_retries,
        )
        .await;

        // Separate successes and failures
        let (successes, failures) = separate_results(results);

        // Parse initialize results
        let init_results: Vec<(String, InitializeResult)> = successes
            .into_iter()
            .filter_map(|(server_id, value)| {
                serde_json::from_value::<InitializeResult>(value)
                    .ok()
                    .map(|result| (server_id, result))
            })
            .collect();

        // Merge results
        let merged = merge_initialize_results(init_results, failures);

        // Store in session
        {
            let mut session_write = session.write().await;
            session_write.merged_capabilities = Some(merged.clone());
        }

        // Build response
        let result = json!({
            "protocolVersion": merged.protocol_version,
            "capabilities": merged.capabilities,
            "serverInfo": merged.server_info,
        });

        Ok(JsonRpcResponse::success(
            request.id.unwrap_or(Value::Null),
            result,
        ))
    }

    /// Handle tools/list request
    async fn handle_tools_list(
        &self,
        session: Arc<RwLock<GatewaySession>>,
        request: JsonRpcRequest,
    ) -> AppResult<JsonRpcResponse> {
        let session_read = session.read().await;

        // Check for deferred loading
        if let Some(deferred) = &session_read.deferred_loading {
            if deferred.enabled {
                // Return only search tool + activated tools
                let mut tools = vec![create_search_tool()];

                for tool_name in &deferred.activated_tools {
                    if let Some(tool) = deferred.full_catalog.iter().find(|t| t.name == *tool_name)
                    {
                        tools.push(tool.clone());
                    }
                }

                drop(session_read);

                return Ok(JsonRpcResponse::success(
                    request.id.unwrap_or(Value::Null),
                    json!({"tools": tools}),
                ));
            }
        }

        // Check cache
        if let Some(cached) = &session_read.cached_tools {
            if cached.is_valid() {
                let tools = cached.data.clone();
                drop(session_read);

                return Ok(JsonRpcResponse::success(
                    request.id.unwrap_or(Value::Null),
                    json!({"tools": tools}),
                ));
            }
        }

        let allowed_servers = session_read.allowed_servers.clone();
        drop(session_read);

        // Fetch from servers
        let tools = self
            .fetch_and_merge_tools(&allowed_servers, request.clone())
            .await?;

        // Update session mappings and cache
        {
            let mut session_write = session.write().await;
            session_write.update_tool_mappings(&tools);

            let cache_ttl = Duration::from_secs(self.config.cache_ttl_seconds);
            session_write.cached_tools = Some(CachedList::new(tools.clone(), cache_ttl));
        }

        Ok(JsonRpcResponse::success(
            request.id.unwrap_or(Value::Null),
            json!({"tools": tools}),
        ))
    }

    /// Fetch and merge tools from servers
    async fn fetch_and_merge_tools(
        &self,
        server_ids: &[String],
        request: JsonRpcRequest,
    ) -> AppResult<Vec<NamespacedTool>> {
        let timeout = Duration::from_secs(self.config.server_timeout_seconds);
        let max_retries = self.config.max_retry_attempts;

        let results = broadcast_request(
            server_ids,
            request,
            &self.server_manager,
            timeout,
            max_retries,
        )
        .await;

        let (successes, failures) = separate_results(results);

        // Parse tools from results
        let server_tools: Vec<(String, Vec<McpTool>)> = successes
            .into_iter()
            .filter_map(|(server_id, value)| {
                value
                    .get("tools")
                    .and_then(|tools| serde_json::from_value::<Vec<McpTool>>(tools.clone()).ok())
                    .map(|tools| (server_id, tools))
            })
            .collect();

        Ok(merge_tools(server_tools, &failures))
    }

    /// Handle resources/list request
    async fn handle_resources_list(
        &self,
        session: Arc<RwLock<GatewaySession>>,
        request: JsonRpcRequest,
    ) -> AppResult<JsonRpcResponse> {
        let session_read = session.read().await;

        // Check cache
        if let Some(cached) = &session_read.cached_resources {
            if cached.is_valid() {
                let resources = cached.data.clone();
                drop(session_read);

                return Ok(JsonRpcResponse::success(
                    request.id.unwrap_or(Value::Null),
                    json!({"resources": resources}),
                ));
            }
        }

        let allowed_servers = session_read.allowed_servers.clone();
        drop(session_read);

        // Fetch from servers
        let resources = self
            .fetch_and_merge_resources(&allowed_servers, request.clone())
            .await?;

        // Update session mappings and cache
        {
            let mut session_write = session.write().await;
            session_write.update_resource_mappings(&resources);

            let cache_ttl = Duration::from_secs(self.config.cache_ttl_seconds);
            session_write.cached_resources = Some(CachedList::new(resources.clone(), cache_ttl));
        }

        Ok(JsonRpcResponse::success(
            request.id.unwrap_or(Value::Null),
            json!({"resources": resources}),
        ))
    }

    /// Fetch and merge resources from servers
    async fn fetch_and_merge_resources(
        &self,
        server_ids: &[String],
        request: JsonRpcRequest,
    ) -> AppResult<Vec<NamespacedResource>> {
        let timeout = Duration::from_secs(self.config.server_timeout_seconds);
        let max_retries = self.config.max_retry_attempts;

        let results = broadcast_request(
            server_ids,
            request,
            &self.server_manager,
            timeout,
            max_retries,
        )
        .await;

        let (successes, failures) = separate_results(results);

        // Parse resources from results
        let server_resources: Vec<(String, Vec<McpResource>)> = successes
            .into_iter()
            .filter_map(|(server_id, value)| {
                value
                    .get("resources")
                    .and_then(|r| serde_json::from_value::<Vec<McpResource>>(r.clone()).ok())
                    .map(|resources| (server_id, resources))
            })
            .collect();

        Ok(merge_resources(server_resources, &failures))
    }

    /// Handle prompts/list request
    async fn handle_prompts_list(
        &self,
        session: Arc<RwLock<GatewaySession>>,
        request: JsonRpcRequest,
    ) -> AppResult<JsonRpcResponse> {
        let session_read = session.read().await;

        // Check cache
        if let Some(cached) = &session_read.cached_prompts {
            if cached.is_valid() {
                let prompts = cached.data.clone();
                drop(session_read);

                return Ok(JsonRpcResponse::success(
                    request.id.unwrap_or(Value::Null),
                    json!({"prompts": prompts}),
                ));
            }
        }

        let allowed_servers = session_read.allowed_servers.clone();
        drop(session_read);

        // Fetch from servers
        let prompts = self
            .fetch_and_merge_prompts(&allowed_servers, request.clone())
            .await?;

        // Update session mappings and cache
        {
            let mut session_write = session.write().await;
            session_write.update_prompt_mappings(&prompts);

            let cache_ttl = Duration::from_secs(self.config.cache_ttl_seconds);
            session_write.cached_prompts = Some(CachedList::new(prompts.clone(), cache_ttl));
        }

        Ok(JsonRpcResponse::success(
            request.id.unwrap_or(Value::Null),
            json!({"prompts": prompts}),
        ))
    }

    /// Fetch and merge prompts from servers
    async fn fetch_and_merge_prompts(
        &self,
        server_ids: &[String],
        request: JsonRpcRequest,
    ) -> AppResult<Vec<NamespacedPrompt>> {
        let timeout = Duration::from_secs(self.config.server_timeout_seconds);
        let max_retries = self.config.max_retry_attempts;

        let results = broadcast_request(
            server_ids,
            request,
            &self.server_manager,
            timeout,
            max_retries,
        )
        .await;

        let (successes, failures) = separate_results(results);

        // Parse prompts from results
        let server_prompts: Vec<(String, Vec<McpPrompt>)> = successes
            .into_iter()
            .filter_map(|(server_id, value)| {
                value
                    .get("prompts")
                    .and_then(|p| serde_json::from_value::<Vec<McpPrompt>>(p.clone()).ok())
                    .map(|prompts| (server_id, prompts))
            })
            .collect();

        Ok(merge_prompts(server_prompts, &failures))
    }

    /// Handle tools/call request
    async fn handle_tools_call(
        &self,
        session: Arc<RwLock<GatewaySession>>,
        request: JsonRpcRequest,
    ) -> AppResult<JsonRpcResponse> {
        // Extract tool name from params
        let tool_name = request
            .params
            .as_ref()
            .and_then(|p| p.get("name"))
            .and_then(|n| n.as_str())
            .ok_or_else(|| AppError::Mcp("Missing tool name in params".to_string()))?;

        // Check if it's the virtual search tool
        if tool_name == "search" {
            return self.handle_search_tool(session, request).await;
        }

        // Parse namespace
        let (server_id, original_name) = parse_namespace(tool_name)
            .ok_or_else(|| AppError::Mcp(format!("Invalid namespaced tool: {}", tool_name)))?;

        // Verify mapping exists in session
        let session_read = session.read().await;
        if !session_read.tool_mapping.contains_key(tool_name) {
            drop(session_read);
            return Err(AppError::Mcp(format!("Unknown tool: {}", tool_name)));
        }
        drop(session_read);

        // Transform request: Strip namespace
        let mut transformed_request = request.clone();
        if let Some(params) = transformed_request.params.as_mut() {
            if let Some(obj) = params.as_object_mut() {
                obj.insert("name".to_string(), json!(original_name));
            }
        }

        // Route to server
        self.server_manager
            .send_request(&server_id, transformed_request)
            .await
    }

    /// Handle search tool call (for deferred loading)
    async fn handle_search_tool(
        &self,
        session: Arc<RwLock<GatewaySession>>,
        request: JsonRpcRequest,
    ) -> AppResult<JsonRpcResponse> {
        let mut session_write = session.write().await;

        let deferred = session_write
            .deferred_loading
            .as_mut()
            .ok_or_else(|| AppError::Mcp("Deferred loading not enabled".to_string()))?;

        // Extract arguments from params
        // MCP tools/call format: params.arguments contains the tool arguments
        let params = request
            .params
            .as_ref()
            .ok_or_else(|| AppError::Mcp("Missing params".to_string()))?;

        let arguments = params
            .get("arguments")
            .ok_or_else(|| AppError::Mcp("Missing arguments in params".to_string()))?;

        let query = arguments
            .get("query")
            .and_then(|q| q.as_str())
            .ok_or_else(|| AppError::Mcp("Missing query parameter".to_string()))?;

        let search_type = arguments.get("type").and_then(|t| t.as_str()).unwrap_or("all");

        let limit = arguments.get("limit").and_then(|l| l.as_u64()).unwrap_or(10) as usize;

        // Search based on type
        let mut activated_names = Vec::new();
        let mut all_matches = Vec::new();

        if search_type == "tools" || search_type == "all" {
            let matches = search_tools(query, &deferred.full_catalog, limit);
            for (tool, _score) in &matches {
                deferred.activated_tools.insert(tool.name.clone());
                activated_names.push(tool.name.clone());
            }
            all_matches.extend(matches.into_iter().map(|(tool, score)| {
                json!({
                    "type": "tool",
                    "name": tool.name,
                    "relevance": score,
                    "description": tool.description,
                })
            }));
        }

        if search_type == "resources" || search_type == "all" {
            let matches = search_resources(query, &deferred.full_resource_catalog, limit);
            for (resource, _score) in &matches {
                deferred.activated_resources.insert(resource.name.clone());
                activated_names.push(resource.name.clone());
            }
            all_matches.extend(matches.into_iter().map(|(resource, score)| {
                json!({
                    "type": "resource",
                    "name": resource.name,
                    "relevance": score,
                    "description": resource.description,
                })
            }));
        }

        if search_type == "prompts" || search_type == "all" {
            let matches = search_prompts(query, &deferred.full_prompt_catalog, limit);
            for (prompt, _score) in &matches {
                deferred.activated_prompts.insert(prompt.name.clone());
                activated_names.push(prompt.name.clone());
            }
            all_matches.extend(matches.into_iter().map(|(prompt, score)| {
                json!({
                    "type": "prompt",
                    "name": prompt.name,
                    "relevance": score,
                    "description": prompt.description,
                })
            }));
        }

        drop(session_write);

        // Return search results
        Ok(JsonRpcResponse::success(
            request.id.unwrap_or(Value::Null),
            json!({
                "activated": activated_names,
                "message": format!("Activated {} items. Use tools/list, resources/list, or prompts/list to see them.", activated_names.len()),
                "matches": all_matches,
            }),
        ))
    }

    /// Handle resources/read request
    async fn handle_resources_read(
        &self,
        session: Arc<RwLock<GatewaySession>>,
        request: JsonRpcRequest,
    ) -> AppResult<JsonRpcResponse> {
        // Extract resource URI or name from params
        let params = request
            .params
            .as_ref()
            .ok_or_else(|| AppError::Mcp("Missing params".to_string()))?;

        // Try to get resource name first (preferred for namespaced routing)
        let resource_name = params.get("name").and_then(|n| n.as_str());

        let (server_id, original_name) = if let Some(name) = resource_name {
            // Prefer namespaced name routing
            parse_namespace(name)
                .ok_or_else(|| AppError::Mcp(format!("Invalid namespaced resource: {}", name)))?
        } else {
            // Fallback: route by URI
            let uri = params
                .get("uri")
                .and_then(|u| u.as_str())
                .ok_or_else(|| AppError::Mcp("Missing resource name or URI".to_string()))?;

            // Look up URI in session mapping
            let session_read = session.read().await;
            let mapping = session_read.resource_uri_mapping.get(uri).cloned();
            drop(session_read);

            mapping.ok_or_else(|| {
                AppError::Mcp(format!(
                    "Resource URI not found: {}. Make sure to call resources/list first.",
                    uri
                ))
            })?
        };

        // Transform request based on routing method
        let mut transformed_request = request.clone();
        if resource_name.is_some() {
            // Routed by namespaced name - strip namespace from name parameter
            if let Some(params) = transformed_request.params.as_mut() {
                if let Some(obj) = params.as_object_mut() {
                    obj.insert("name".to_string(), json!(original_name));
                }
            }
        }
        // If routed by URI, leave parameters unchanged - backend will handle its own URIs

        // Route to server
        self.server_manager
            .send_request(&server_id, transformed_request)
            .await
    }

    /// Handle prompts/get request
    async fn handle_prompts_get(
        &self,
        session: Arc<RwLock<GatewaySession>>,
        request: JsonRpcRequest,
    ) -> AppResult<JsonRpcResponse> {
        // Extract prompt name from params
        let prompt_name = request
            .params
            .as_ref()
            .and_then(|p| p.get("name"))
            .and_then(|n| n.as_str())
            .ok_or_else(|| AppError::Mcp("Missing prompt name in params".to_string()))?;

        // Parse namespace
        let (server_id, original_name) = parse_namespace(prompt_name)
            .ok_or_else(|| AppError::Mcp(format!("Invalid namespaced prompt: {}", prompt_name)))?;

        // Verify mapping exists
        let session_read = session.read().await;
        if !session_read.prompt_mapping.contains_key(prompt_name) {
            drop(session_read);
            return Err(AppError::Mcp(format!("Unknown prompt: {}", prompt_name)));
        }
        drop(session_read);

        // Transform request: Strip namespace
        let mut transformed_request = request.clone();
        if let Some(params) = transformed_request.params.as_mut() {
            if let Some(obj) = params.as_object_mut() {
                obj.insert("name".to_string(), json!(original_name));
            }
        }

        // Route to server
        self.server_manager
            .send_request(&server_id, transformed_request)
            .await
    }

    /// Broadcast and return first successful response (for ping, logging/setLevel)
    async fn broadcast_and_return_first(
        &self,
        session: Arc<RwLock<GatewaySession>>,
        request: JsonRpcRequest,
    ) -> AppResult<JsonRpcResponse> {
        let session_read = session.read().await;
        let allowed_servers = session_read.allowed_servers.clone();
        drop(session_read);

        let timeout = Duration::from_secs(self.config.server_timeout_seconds);
        let max_retries = self.config.max_retry_attempts;

        let results = broadcast_request(
            &allowed_servers,
            request.clone(),
            &self.server_manager,
            timeout,
            max_retries,
        )
        .await;

        let (successes, _failures) = separate_results(results);

        if successes.is_empty() {
            return Err(AppError::Mcp("All servers failed".to_string()));
        }

        // Return first successful result
        let (_, value) = successes.into_iter().next().unwrap();

        Ok(JsonRpcResponse::success(
            request.id.unwrap_or(Value::Null),
            value,
        ))
    }

    /// Cleanup expired sessions (call periodically from background task)
    pub fn cleanup_expired_sessions(&self) {
        let mut to_remove = Vec::new();

        for entry in self.sessions.iter() {
            let client_id = entry.key().clone();
            let session = entry.value().clone();

            // Try to acquire read lock with timeout
            let is_expired = if let Ok(session_read) = session.try_read() {
                session_read.is_expired()
            } else {
                false
            };

            if is_expired {
                to_remove.push(client_id);
            }
        }

        for client_id in to_remove {
            self.sessions.remove(&client_id);
            tracing::info!("Removed expired gateway session for client: {}", client_id);
        }
    }

    /// Invalidate tools cache for a session (for notification handling)
    pub fn invalidate_tools_cache(&self, client_id: &str) {
        if let Some(session) = self.sessions.get(client_id) {
            if let Ok(mut session_write) = session.try_write() {
                session_write.invalidate_tools_cache();
            }
        }
    }

    /// Invalidate resources cache for a session
    pub fn invalidate_resources_cache(&self, client_id: &str) {
        if let Some(session) = self.sessions.get(client_id) {
            if let Ok(mut session_write) = session.try_write() {
                session_write.invalidate_resources_cache();
            }
        }
    }
}
