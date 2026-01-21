#![allow(dead_code)]

use dashmap::DashMap;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

use crate::mcp::manager::McpServerManager;
use crate::mcp::protocol::{JsonRpcError, JsonRpcNotification, JsonRpcRequest, JsonRpcResponse, McpPrompt, McpResource, McpTool};
use crate::router::Router;
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

    /// Track which servers have global notification handlers registered
    notification_handlers_registered: Arc<DashMap<String, bool>>,

    /// Broadcast sender for client notifications (optional)
    /// Allows external clients to subscribe to real-time notifications from MCP servers
    /// Format: (server_id, notification)
    notification_broadcast: Option<Arc<tokio::sync::broadcast::Sender<(String, JsonRpcNotification)>>>,

    /// Router for LLM provider access (for sampling/createMessage support)
    router: Arc<Router>,
}

impl McpGateway {
    /// Create a new MCP gateway
    pub fn new(server_manager: Arc<McpServerManager>, config: GatewayConfig, router: Arc<Router>) -> Self {
        Self::new_with_broadcast(server_manager, config, router, None)
    }

    /// Create a new MCP gateway with optional broadcast channel for client notifications
    pub fn new_with_broadcast(
        server_manager: Arc<McpServerManager>,
        config: GatewayConfig,
        router: Arc<Router>,
        notification_broadcast: Option<Arc<tokio::sync::broadcast::Sender<(String, JsonRpcNotification)>>>,
    ) -> Self {
        Self {
            sessions: Arc::new(DashMap::new()),
            server_manager,
            config,
            notification_handlers_registered: Arc::new(DashMap::new()),
            notification_broadcast,
            router,
        }
    }

    /// Handle an MCP gateway request
    pub async fn handle_request(
        &self,
        client_id: &str,
        allowed_servers: Vec<String>,
        enable_deferred_loading: bool,
        roots: Vec<crate::mcp::protocol::Root>,
        request: JsonRpcRequest,
    ) -> AppResult<JsonRpcResponse> {
        // Get or create session
        let session: Arc<RwLock<GatewaySession>> = self
            .get_or_create_session(client_id, allowed_servers, enable_deferred_loading, roots)
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
        roots: Vec<crate::mcp::protocol::Root>,
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

        // Create new session (deferred loading will be set up in handle_initialize if supported)
        let ttl = Duration::from_secs(self.config.session_ttl_seconds);
        let session_data = GatewaySession::new(
            client_id.to_string(),
            allowed_servers.clone(),
            ttl,
            self.config.cache_ttl_seconds,
            roots,
            enable_deferred_loading,
        );

        let session = Arc::new(RwLock::new(session_data));

        // Register GLOBAL notification handlers for each server (if not already registered)
        self.register_notification_handlers(&allowed_servers)
            .await;

        self.sessions.insert(client_id.to_string(), session.clone());

        Ok(session)
    }

    /// Register GLOBAL notification handlers for servers (one handler per server, shared across sessions)
    /// This prevents memory leaks from per-session handlers
    async fn register_notification_handlers(
        &self,
        allowed_servers: &[String],
    ) {
        for server_id in allowed_servers {
            // Check if handler already registered for this server
            if self.notification_handlers_registered.contains_key(server_id) {
                continue;
            }

            // Mark as registered (prevent duplicate registration)
            self.notification_handlers_registered.insert(server_id.clone(), true);

            let sessions_clone = self.sessions.clone();
            let server_id_clone = server_id.clone();
            let broadcast_clone = self.notification_broadcast.clone();

            // Register GLOBAL notification handler (one per server, not per session)
            self.server_manager.on_notification(
                server_id,
                Arc::new(move |_, notification| {
                    let sessions_inner = sessions_clone.clone();
                    let server_id_inner = server_id_clone.clone();
                    let broadcast_inner = broadcast_clone.clone();

                    tokio::spawn(async move {
                        match notification.method.as_str() {
                            "notifications/tools/list_changed" => {
                                tracing::info!(
                                    "Received tools/list_changed notification from server: {}",
                                    server_id_inner
                                );
                                // Invalidate tools cache for ALL sessions that include this server
                                for entry in sessions_inner.iter() {
                                    let session = entry.value();
                                    if let Ok(mut session_write) = session.try_write() {
                                        if session_write.allowed_servers.contains(&server_id_inner) {
                                            session_write.cache_ttl_manager.record_invalidation();
                                            session_write.cached_tools = None;
                                        }
                                    }
                                }
                                tracing::debug!("Invalidated tools cache for all sessions using server: {}", server_id_inner);
                            }
                            "notifications/resources/list_changed" => {
                                tracing::info!(
                                    "Received resources/list_changed notification from server: {}",
                                    server_id_inner
                                );
                                // Invalidate resources cache for ALL sessions that include this server
                                for entry in sessions_inner.iter() {
                                    let session = entry.value();
                                    if let Ok(mut session_write) = session.try_write() {
                                        if session_write.allowed_servers.contains(&server_id_inner) {
                                            session_write.cache_ttl_manager.record_invalidation();
                                            session_write.cached_resources = None;
                                        }
                                    }
                                }
                                tracing::debug!("Invalidated resources cache for all sessions using server: {}", server_id_inner);
                            }
                            "notifications/prompts/list_changed" => {
                                tracing::info!(
                                    "Received prompts/list_changed notification from server: {}",
                                    server_id_inner
                                );
                                // Invalidate prompts cache for ALL sessions that include this server
                                for entry in sessions_inner.iter() {
                                    let session = entry.value();
                                    if let Ok(mut session_write) = session.try_write() {
                                        if session_write.allowed_servers.contains(&server_id_inner) {
                                            session_write.cache_ttl_manager.record_invalidation();
                                            session_write.cached_prompts = None;
                                        }
                                    }
                                }
                                tracing::debug!("Invalidated prompts cache for all sessions using server: {}", server_id_inner);
                            }
                            other_method => {
                                tracing::debug!(
                                    "Received notification from server {}: {}",
                                    server_id_inner,
                                    other_method
                                );
                                // Other notifications are logged but not acted upon
                            }
                        }

                        // Forward notification to external clients (if broadcast channel exists)
                        if let Some(broadcast) = broadcast_inner.as_ref() {
                            let payload = (server_id_inner.clone(), notification.clone());
                            match broadcast.send(payload) {
                                Ok(receiver_count) => {
                                    tracing::debug!(
                                        "Forwarded notification from server {} to {} client(s)",
                                        server_id_inner,
                                        receiver_count
                                    );
                                }
                                Err(_) => {
                                    // No active receivers - this is normal when no clients are connected
                                    tracing::trace!(
                                        "No clients subscribed to notifications from server {}",
                                        server_id_inner
                                    );
                                }
                            }
                        }
                    });
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
            _ => {
                // Return JSON-RPC error for unknown methods
                Ok(JsonRpcResponse::error(
                    request.id.unwrap_or(Value::Null),
                    JsonRpcError::method_not_found(&request.method),
                ))
            }
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

            // MCP client capabilities - return helpful errors
            "completion/complete" => {
                Ok(JsonRpcResponse::error(
                    request.id.unwrap_or(Value::Null),
                    JsonRpcError::custom(
                        -32601,
                        "completion/complete is a client capability. Servers request this from clients, not gateways.".to_string(),
                        Some(json!({
                            "method_type": "client_capability",
                            "direction": "server_to_client",
                            "hint": "This method should be implemented by your LLM client, not the MCP gateway"
                        }))
                    )
                ))
            }

            "sampling/createMessage" => {
                // TODO: Full implementation requires provider manager and router integration
                Ok(JsonRpcResponse::error(
                    request.id.unwrap_or(Value::Null),
                    JsonRpcError::custom(
                        -32601,
                        "sampling/createMessage not yet fully implemented. Protocol types and conversion logic are ready.".to_string(),
                        Some(json!({
                            "status": "partial",
                            "hint": "Sampling support infrastructure is in place but requires provider/router integration"
                        }))
                    )
                ))
            }

            "roots/list" => self.handle_roots_list(session, request).await,

            // Future MCP features - return not implemented with workarounds
            "resources/subscribe" => {
                Ok(JsonRpcResponse::error(
                    request.id.unwrap_or(Value::Null),
                    JsonRpcError::custom(
                        -32601,
                        "resources/subscribe not yet implemented. Use resources/list with notifications/resources/list_changed for updates.".to_string(),
                        Some(json!({
                            "workaround": "poll_resources_list",
                            "hint": "Call resources/list periodically, or rely on notifications/resources/list_changed"
                        }))
                    )
                ))
            }

            "resources/unsubscribe" => {
                Ok(JsonRpcResponse::error(
                    request.id.unwrap_or(Value::Null),
                    JsonRpcError::custom(
                        -32601,
                        "resources/unsubscribe not yet implemented.".to_string(),
                        None
                    )
                ))
            }

            _ => {
                // Return JSON-RPC error for unknown methods
                Ok(JsonRpcResponse::error(
                    request.id.unwrap_or(Value::Null),
                    JsonRpcError::method_not_found(&request.method),
                ))
            }
        }
    }

    /// Handle roots/list request (MCP client capability)
    ///
    /// Returns the filesystem roots configured for this session.
    /// Roots are advisory boundaries, not security boundaries.
    async fn handle_roots_list(
        &self,
        session: Arc<RwLock<GatewaySession>>,
        request: JsonRpcRequest,
    ) -> AppResult<JsonRpcResponse> {
        let session_read = session.read().await;
        let roots = session_read.roots.clone();
        drop(session_read);

        // Convert Root to MCP protocol format
        let roots_value: Vec<Value> = roots
            .iter()
            .map(|root| {
                let mut obj = serde_json::Map::new();
                obj.insert("uri".to_string(), json!(root.uri));
                if let Some(name) = &root.name {
                    obj.insert("name".to_string(), json!(name));
                }
                Value::Object(obj)
            })
            .collect();

        let result = json!({
            "roots": roots_value
        });

        Ok(JsonRpcResponse::success(
            request.id.unwrap_or(Value::Null),
            result,
        ))
    }

    /// Handle initialize request
    async fn handle_initialize(
        &self,
        session: Arc<RwLock<GatewaySession>>,
        request: JsonRpcRequest,
    ) -> AppResult<JsonRpcResponse> {
        // Extract client capabilities from request params
        let client_capabilities = request
            .params
            .as_ref()
            .and_then(|params| params.get("capabilities"))
            .and_then(|caps| serde_json::from_value::<ClientCapabilities>(caps.clone()).ok());

        if let Some(ref caps) = client_capabilities {
            tracing::debug!("Client capabilities received: {:?}", caps);
        } else {
            tracing::warn!("Client did not provide capabilities in initialize request");
        }

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

        // If all servers failed, return error
        if init_results.is_empty() && !failures.is_empty() {
            let error_summary = failures
                .iter()
                .map(|f| format!("{}: {}", f.server_id, f.error))
                .collect::<Vec<_>>()
                .join("; ");
            return Err(AppError::Mcp(format!(
                "All servers failed to initialize: {}",
                error_summary
            )));
        }

        // Merge results
        let merged = merge_initialize_results(init_results, failures);

        // Store in session
        {
            let mut session_write = session.write().await;
            session_write.merged_capabilities = Some(merged.clone());
            session_write.client_capabilities = client_capabilities.clone();
        }

        // Check if we should enable deferred loading
        // Deferred loading requires that at least one server supports listChanged notifications
        // and that the client can receive these notifications (implied by MCP spec)
        let session_read = session.read().await;
        let should_enable_deferred = session_read.deferred_loading_requested;
        let client_id_for_log = session_read.client_id.clone();
        let allowed_servers_for_deferred = session_read.allowed_servers.clone();
        drop(session_read);

        if should_enable_deferred {
            // Check if any server supports listChanged for tools
            let any_server_supports_list_changed = merged
                .capabilities
                .tools
                .as_ref()
                .and_then(|t| t.list_changed)
                .unwrap_or(false)
                || merged
                    .capabilities
                    .resources
                    .as_ref()
                    .and_then(|r| r.list_changed)
                    .unwrap_or(false)
                || merged
                    .capabilities
                    .prompts
                    .as_ref()
                    .and_then(|p| p.list_changed)
                    .unwrap_or(false);

            if any_server_supports_list_changed {
                tracing::info!(
                    "Setting up deferred loading for client {}: servers support listChanged",
                    client_id_for_log
                );

                // Ensure servers are started
                for server_id in &allowed_servers_for_deferred {
                    if !self.server_manager.is_running(server_id) {
                        self.server_manager.start_server(server_id).await?;
                    }
                }

                // Fetch full catalog from all servers
                let (tools, _) = self
                    .fetch_and_merge_tools(
                        &allowed_servers_for_deferred,
                        JsonRpcRequest::new(
                            Some(serde_json::json!(1)),
                            "tools/list".to_string(),
                            None,
                        ),
                    )
                    .await
                    .unwrap_or_default();

                let (resources, _) = self
                    .fetch_and_merge_resources(
                        &allowed_servers_for_deferred,
                        JsonRpcRequest::new(
                            Some(serde_json::json!(2)),
                            "resources/list".to_string(),
                            None,
                        ),
                    )
                    .await
                    .unwrap_or_default();

                let (prompts, _) = self
                    .fetch_and_merge_prompts(
                        &allowed_servers_for_deferred,
                        JsonRpcRequest::new(
                            Some(serde_json::json!(3)),
                            "prompts/list".to_string(),
                            None,
                        ),
                    )
                    .await
                    .unwrap_or_default();

                let mut session_write = session.write().await;
                session_write.deferred_loading = Some(DeferredLoadingState {
                    enabled: true,
                    activated_tools: std::collections::HashSet::new(),
                    full_catalog: tools.clone(),
                    activated_resources: std::collections::HashSet::new(),
                    full_resource_catalog: resources.clone(),
                    activated_prompts: std::collections::HashSet::new(),
                    full_prompt_catalog: prompts.clone(),
                });

                tracing::info!(
                    "Deferred loading enabled for client {}: {} tools, {} resources, {} prompts",
                    client_id_for_log,
                    tools.len(),
                    resources.len(),
                    prompts.len(),
                );
            } else {
                tracing::warn!(
                    "Deferred loading requested for client {} but no servers support listChanged notifications. Falling back to normal mode.",
                    client_id_for_log
                );
            }
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
        let (tools, failures) = self
            .fetch_and_merge_tools(&allowed_servers, request.clone())
            .await?;

        // Update session mappings, cache, and failures
        {
            let mut session_write = session.write().await;
            session_write.update_tool_mappings(&tools);
            session_write.last_broadcast_failures = failures;

            let cache_ttl = session_write.cache_ttl_manager.get_ttl();
            session_write.cached_tools = Some(CachedList::new(tools.clone(), cache_ttl));
        }

        // Check if there were any failures during fetch
        let session_read = session.read().await;
        let has_failures = !session_read.last_broadcast_failures.is_empty();
        let failures = if has_failures {
            Some(session_read.last_broadcast_failures.clone())
        } else {
            None
        };
        drop(session_read);

        let mut result = json!({"tools": tools});
        if let Some(failures) = failures {
            result["_meta"] = json!({
                "partial_failure": true,
                "failures": failures
            });
        }

        Ok(JsonRpcResponse::success(
            request.id.unwrap_or(Value::Null),
            result,
        ))
    }

    /// Fetch and merge tools from servers
    async fn fetch_and_merge_tools(
        &self,
        server_ids: &[String],
        request: JsonRpcRequest,
    ) -> AppResult<(Vec<NamespacedTool>, Vec<ServerFailure>)> {
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

        // If all servers failed, return error
        if successes.is_empty() && !failures.is_empty() {
            let error_summary = failures
                .iter()
                .map(|f| format!("{}: {}", f.server_id, f.error))
                .collect::<Vec<_>>()
                .join("; ");
            return Err(AppError::Mcp(format!(
                "All servers failed to respond: {}",
                error_summary
            )));
        }

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

        Ok((merge_tools(server_tools, &failures), failures))
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
        let (resources, failures) = self
            .fetch_and_merge_resources(&allowed_servers, request.clone())
            .await?;

        // Update session mappings, cache, failures, and mark as fetched
        {
            let mut session_write = session.write().await;
            session_write.update_resource_mappings(&resources);
            session_write.last_broadcast_failures = failures.clone();
            session_write.resources_list_fetched = true;

            let cache_ttl = session_write.cache_ttl_manager.get_ttl();
            session_write.cached_resources = Some(CachedList::new(resources.clone(), cache_ttl));
        }

        let mut result = json!({"resources": resources});
        if !failures.is_empty() {
            result["_meta"] = json!({
                "partial_failure": true,
                "failures": failures
            });
        }

        Ok(JsonRpcResponse::success(
            request.id.unwrap_or(Value::Null),
            result,
        ))
    }

    /// Fetch and merge resources from servers
    async fn fetch_and_merge_resources(
        &self,
        server_ids: &[String],
        request: JsonRpcRequest,
    ) -> AppResult<(Vec<NamespacedResource>, Vec<ServerFailure>)> {
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

        // If all servers failed, return error
        if successes.is_empty() && !failures.is_empty() {
            let error_summary = failures
                .iter()
                .map(|f| format!("{}: {}", f.server_id, f.error))
                .collect::<Vec<_>>()
                .join("; ");
            return Err(AppError::Mcp(format!(
                "All servers failed to respond: {}",
                error_summary
            )));
        }

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

        Ok((merge_resources(server_resources, &failures), failures))
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
        let (prompts, failures) = self
            .fetch_and_merge_prompts(&allowed_servers, request.clone())
            .await?;

        // Update session mappings, cache, and failures
        {
            let mut session_write = session.write().await;
            session_write.update_prompt_mappings(&prompts);
            session_write.last_broadcast_failures = failures.clone();

            let cache_ttl = session_write.cache_ttl_manager.get_ttl();
            session_write.cached_prompts = Some(CachedList::new(prompts.clone(), cache_ttl));
        }

        let mut result = json!({"prompts": prompts});
        if !failures.is_empty() {
            result["_meta"] = json!({
                "partial_failure": true,
                "failures": failures
            });
        }

        Ok(JsonRpcResponse::success(
            request.id.unwrap_or(Value::Null),
            result,
        ))
    }

    /// Fetch and merge prompts from servers
    async fn fetch_and_merge_prompts(
        &self,
        server_ids: &[String],
        request: JsonRpcRequest,
    ) -> AppResult<(Vec<NamespacedPrompt>, Vec<ServerFailure>)> {
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

        // If all servers failed, return error
        if successes.is_empty() && !failures.is_empty() {
            let error_summary = failures
                .iter()
                .map(|f| format!("{}: {}", f.server_id, f.error))
                .collect::<Vec<_>>()
                .join("; ");
            return Err(AppError::Mcp(format!(
                "All servers failed to respond: {}",
                error_summary
            )));
        }

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

        Ok((merge_prompts(server_prompts, &failures), failures))
    }

    /// Handle tools/call request
    async fn handle_tools_call(
        &self,
        session: Arc<RwLock<GatewaySession>>,
        request: JsonRpcRequest,
    ) -> AppResult<JsonRpcResponse> {
        // Extract tool name from params
        let tool_name = match request
            .params
            .as_ref()
            .and_then(|p| p.get("name"))
            .and_then(|n| n.as_str())
        {
            Some(name) => name,
            None => {
                return Ok(JsonRpcResponse::error(
                    request.id.unwrap_or(Value::Null),
                    JsonRpcError::invalid_params("Missing tool name in params"),
                ));
            }
        };

        // Check if it's the virtual search tool
        if tool_name == "search" {
            return self.handle_search_tool(session, request).await;
        }

        // Parse namespace
        let (server_id, original_name) = match parse_namespace(tool_name) {
            Some((id, name)) => (id, name),
            None => {
                return Ok(JsonRpcResponse::error(
                    request.id.unwrap_or(Value::Null),
                    JsonRpcError::invalid_params(format!("Invalid namespaced tool: {}", tool_name)),
                ));
            }
        };

        // Verify mapping exists in session
        let session_read = session.read().await;
        if !session_read.tool_mapping.contains_key(tool_name) {
            drop(session_read);
            return Ok(JsonRpcResponse::error(
                request.id.unwrap_or(Value::Null),
                JsonRpcError::tool_not_found(tool_name),
            ));
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

        let deferred = match session_write.deferred_loading.as_mut() {
            Some(d) => d,
            None => {
                return Ok(JsonRpcResponse::error(
                    request.id.unwrap_or(Value::Null),
                    JsonRpcError::invalid_params("Deferred loading not enabled"),
                ));
            }
        };

        // Extract arguments from params
        // MCP tools/call format: params.arguments contains the tool arguments
        let params = match request.params.as_ref() {
            Some(p) => p,
            None => {
                return Ok(JsonRpcResponse::error(
                    request.id.unwrap_or(Value::Null),
                    JsonRpcError::invalid_params("Missing params"),
                ));
            }
        };

        let arguments = match params.get("arguments") {
            Some(a) => a,
            None => {
                return Ok(JsonRpcResponse::error(
                    request.id.unwrap_or(Value::Null),
                    JsonRpcError::invalid_params("Missing arguments in params"),
                ));
            }
        };

        let query = match arguments.get("query").and_then(|q| q.as_str()) {
            Some(q) => q,
            None => {
                return Ok(JsonRpcResponse::error(
                    request.id.unwrap_or(Value::Null),
                    JsonRpcError::invalid_params("Missing query parameter"),
                ));
            }
        };

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
        let params = match request.params.as_ref() {
            Some(p) => p,
            None => {
                return Ok(JsonRpcResponse::error(
                    request.id.unwrap_or(Value::Null),
                    JsonRpcError::invalid_params("Missing params"),
                ));
            }
        };

        // Try to get resource name first (preferred for namespaced routing)
        let resource_name = params.get("name").and_then(|n| n.as_str());

        let (server_id, original_name) = if let Some(name) = resource_name {
            // Prefer namespaced name routing
            match parse_namespace(name) {
                Some((id, n)) => (id, n),
                None => {
                    return Ok(JsonRpcResponse::error(
                        request.id.unwrap_or(Value::Null),
                        JsonRpcError::invalid_params(format!("Invalid namespaced resource: {}", name)),
                    ));
                }
            }
        } else {
            // Fallback: route by URI
            let uri = match params.get("uri").and_then(|u| u.as_str()) {
                Some(u) => u,
                None => {
                    return Ok(JsonRpcResponse::error(
                        request.id.unwrap_or(Value::Null),
                        JsonRpcError::invalid_params("Missing resource name or URI"),
                    ));
                }
            };

            // Look up URI in session mapping
            let session_read = session.read().await;
            let mapping = session_read.resource_uri_mapping.get(uri).cloned();
            let resources_list_fetched = session_read.resources_list_fetched;
            let allowed_servers = session_read.allowed_servers.clone();
            drop(session_read);

            // If URI not found and we haven't tried fetching resources/list yet, do so
            if mapping.is_none() && !resources_list_fetched {
                tracing::info!(
                    "Resource URI not in mapping and resources/list not yet fetched, fetching now"
                );

                // Fetch resources/list to populate the URI mapping (only once per session)
                let (resources, _failures) = self
                    .fetch_and_merge_resources(
                        &allowed_servers,
                        JsonRpcRequest::new(
                            Some(serde_json::json!("auto")),
                            "resources/list".to_string(),
                            None,
                        ),
                    )
                    .await?;

                // Update session mappings and mark as fetched
                let mut session_write = session.write().await;
                session_write.update_resource_mappings(&resources);
                session_write.resources_list_fetched = true;
                let new_mapping = session_write.resource_uri_mapping.get(uri).cloned();
                drop(session_write);

                // Try again with populated mapping
                match new_mapping {
                    Some(m) => m,
                    None => {
                        return Ok(JsonRpcResponse::error(
                            request.id.unwrap_or(Value::Null),
                            JsonRpcError::resource_not_found(format!(
                                "Resource URI not found after fetching resources/list: {}",
                                uri
                            )),
                        ));
                    }
                }
            } else {
                match mapping {
                    Some(m) => m,
                    None => {
                        return Ok(JsonRpcResponse::error(
                            request.id.unwrap_or(Value::Null),
                            JsonRpcError::resource_not_found(format!(
                                "Resource URI not found: {}",
                                uri
                            )),
                        ));
                    }
                }
            }
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
        let prompt_name = match request
            .params
            .as_ref()
            .and_then(|p| p.get("name"))
            .and_then(|n| n.as_str())
        {
            Some(name) => name,
            None => {
                return Ok(JsonRpcResponse::error(
                    request.id.unwrap_or(Value::Null),
                    JsonRpcError::invalid_params("Missing prompt name in params"),
                ));
            }
        };

        // Parse namespace
        let (server_id, original_name) = match parse_namespace(prompt_name) {
            Some((id, name)) => (id, name),
            None => {
                return Ok(JsonRpcResponse::error(
                    request.id.unwrap_or(Value::Null),
                    JsonRpcError::invalid_params(format!("Invalid namespaced prompt: {}", prompt_name)),
                ));
            }
        };

        // Verify mapping exists
        let session_read = session.read().await;
        if !session_read.prompt_mapping.contains_key(prompt_name) {
            drop(session_read);
            return Ok(JsonRpcResponse::error(
                request.id.unwrap_or(Value::Null),
                JsonRpcError::prompt_not_found(prompt_name),
            ));
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
