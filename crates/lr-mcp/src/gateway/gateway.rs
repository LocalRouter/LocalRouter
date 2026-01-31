#![allow(dead_code)]

use dashmap::DashMap;
use serde_json::{json, Value};
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tokio::sync::RwLock;

use crate::manager::McpServerManager;
use crate::protocol::{
    JsonRpcError, JsonRpcNotification, JsonRpcRequest, JsonRpcResponse, McpPrompt, McpResource,
    McpTool,
};
use lr_router::Router;
use lr_skills::executor::ScriptExecutor;
use lr_skills::manager::SkillManager;
use lr_types::{AppError, AppResult};

use super::deferred::{create_search_tool, search_prompts, search_resources, search_tools};
use super::elicitation::ElicitationManager;
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
    notification_broadcast:
        Option<Arc<tokio::sync::broadcast::Sender<(String, JsonRpcNotification)>>>,

    /// Router for LLM provider access (for sampling/createMessage support)
    router: Arc<Router>,

    /// Elicitation manager for handling structured user input requests
    elicitation_manager: Arc<ElicitationManager>,

    /// Skill manager (optional, for AgentSkills.io support)
    /// Uses OnceLock so it can be set after Arc construction via &self
    skill_manager: OnceLock<Arc<SkillManager>>,

    /// Script executor for running skill scripts (optional)
    script_executor: OnceLock<Arc<ScriptExecutor>>,
}

impl McpGateway {
    /// Create a new MCP gateway
    pub fn new(
        server_manager: Arc<McpServerManager>,
        config: GatewayConfig,
        router: Arc<Router>,
    ) -> Self {
        Self::new_with_broadcast(server_manager, config, router, None)
    }

    /// Create a new MCP gateway with optional broadcast channel for client notifications
    pub fn new_with_broadcast(
        server_manager: Arc<McpServerManager>,
        config: GatewayConfig,
        router: Arc<Router>,
        notification_broadcast: Option<
            Arc<tokio::sync::broadcast::Sender<(String, JsonRpcNotification)>>,
        >,
    ) -> Self {
        // Create elicitation manager with broadcast support if available
        let elicitation_manager = match &notification_broadcast {
            Some(broadcast) => Arc::new(ElicitationManager::new_with_broadcast(
                120,
                broadcast.clone(),
            )),
            None => Arc::new(ElicitationManager::default()),
        };

        Self {
            sessions: Arc::new(DashMap::new()),
            server_manager,
            config,
            notification_handlers_registered: Arc::new(DashMap::new()),
            notification_broadcast,
            router,
            elicitation_manager,
            skill_manager: OnceLock::new(),
            script_executor: OnceLock::new(),
        }
    }

    /// Set skill manager and script executor for AgentSkills.io support.
    /// Uses OnceLock so this can be called on `&self` (gateway is behind Arc).
    pub fn set_skill_support(
        &self,
        skill_manager: Arc<SkillManager>,
        script_executor: Arc<ScriptExecutor>,
    ) {
        let _ = self.skill_manager.set(skill_manager);
        let _ = self.script_executor.set(script_executor);
    }

    /// Check if skill support has been configured
    fn has_skill_support(&self) -> bool {
        self.skill_manager.get().is_some() && self.script_executor.get().is_some()
    }

    /// Build a mapping from server ID (UUID) to human-readable server name
    ///
    /// This is used to namespace tools/resources/prompts with readable names
    /// (e.g., "filesystem__read_file") instead of UUIDs.
    fn build_server_id_to_name_mapping(
        &self,
        server_ids: &[String],
    ) -> std::collections::HashMap<String, String> {
        let mut mapping = std::collections::HashMap::new();
        for server_id in server_ids {
            if let Some(config) = self.server_manager.get_config(server_id) {
                mapping.insert(server_id.clone(), config.name);
            }
        }
        mapping
    }

    /// Handle an MCP gateway request
    pub async fn handle_request(
        &self,
        client_id: &str,
        allowed_servers: Vec<String>,
        enable_deferred_loading: bool,
        roots: Vec<crate::protocol::Root>,
        request: JsonRpcRequest,
    ) -> AppResult<JsonRpcResponse> {
        self.handle_request_with_skills(
            client_id,
            allowed_servers,
            enable_deferred_loading,
            roots,
            lr_config::SkillsAccess::None,
            request,
        )
        .await
    }

    /// Handle an MCP gateway request with skill access
    pub async fn handle_request_with_skills(
        &self,
        client_id: &str,
        allowed_servers: Vec<String>,
        enable_deferred_loading: bool,
        roots: Vec<crate::protocol::Root>,
        skills_access: lr_config::SkillsAccess,
        request: JsonRpcRequest,
    ) -> AppResult<JsonRpcResponse> {
        let method = request.method.clone();
        let request_id = request.id.clone();
        let is_broadcast = should_broadcast(&method);

        tracing::info!(
            "Gateway handle_request: client_id={}, method={}, request_id={:?}, is_broadcast={}, servers={}",
            client_id,
            method,
            request_id,
            is_broadcast,
            allowed_servers.len()
        );

        // Get or create session
        let session: Arc<RwLock<GatewaySession>> = self
            .get_or_create_session(client_id, allowed_servers, enable_deferred_loading, roots)
            .await?;

        // Update skills access on session
        if skills_access.has_any_access() {
            let mut session_write = session.write().await;
            session_write.skills_access = skills_access;
        }

        // Update last activity
        {
            let mut session_write = session.write().await;
            session_write.touch();
        }

        // Route based on method
        let result = if is_broadcast {
            self.handle_broadcast_request(session, request).await
        } else {
            self.handle_direct_request(session, request).await
        };

        match &result {
            Ok(response) => {
                tracing::info!(
                    "Gateway handle_request completed: client_id={}, method={}, response_id={:?}, has_error={}",
                    client_id,
                    method,
                    response.id,
                    response.error.is_some()
                );
            }
            Err(e) => {
                tracing::error!(
                    "Gateway handle_request failed: client_id={}, method={}, error={}",
                    client_id,
                    method,
                    e
                );
            }
        }

        result
    }

    /// Get or create a session
    async fn get_or_create_session(
        &self,
        client_id: &str,
        allowed_servers: Vec<String>,
        enable_deferred_loading: bool,
        roots: Vec<crate::protocol::Root>,
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
                // Update deferred loading setting if it changed
                // This allows the Try it out UI to toggle deferred loading between connections
                let current_deferred = session_read.deferred_loading_requested;
                drop(session_read);

                if current_deferred != enable_deferred_loading {
                    let mut session_write = session.write().await;
                    session_write.deferred_loading_requested = enable_deferred_loading;
                    // Reset deferred loading state so it can be re-initialized
                    session_write.deferred_loading = None;
                    tracing::info!(
                        "Updated deferred loading setting for session {}: {} -> {}",
                        client_id,
                        current_deferred,
                        enable_deferred_loading
                    );
                }

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
        self.register_notification_handlers(&allowed_servers).await;

        self.sessions.insert(client_id.to_string(), session.clone());

        Ok(session)
    }

    /// Register GLOBAL notification handlers for servers (one handler per server, shared across sessions)
    /// This prevents memory leaks from per-session handlers
    async fn register_notification_handlers(&self, allowed_servers: &[String]) {
        for server_id in allowed_servers {
            // Check if handler already registered for this server
            if self
                .notification_handlers_registered
                .contains_key(server_id)
            {
                continue;
            }

            // Mark as registered (prevent duplicate registration)
            self.notification_handlers_registered
                .insert(server_id.clone(), true);

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
                                        if session_write.allowed_servers.contains(&server_id_inner)
                                        {
                                            session_write.cache_ttl_manager.record_invalidation();
                                            session_write.cached_tools = None;
                                        }
                                    }
                                }
                                tracing::debug!(
                                    "Invalidated tools cache for all sessions using server: {}",
                                    server_id_inner
                                );
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
                                        if session_write.allowed_servers.contains(&server_id_inner)
                                        {
                                            session_write.cache_ttl_manager.record_invalidation();
                                            session_write.cached_resources = None;
                                        }
                                    }
                                }
                                tracing::debug!(
                                    "Invalidated resources cache for all sessions using server: {}",
                                    server_id_inner
                                );
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
                                        if session_write.allowed_servers.contains(&server_id_inner)
                                        {
                                            session_write.cache_ttl_manager.record_invalidation();
                                            session_write.cached_prompts = None;
                                        }
                                    }
                                }
                                tracing::debug!(
                                    "Invalidated prompts cache for all sessions using server: {}",
                                    server_id_inner
                                );
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
                // Sampling is handled in route handlers (mcp.rs) not in gateway
                // This shouldn't be called from unified gateway - use individual server proxy instead
                Ok(JsonRpcResponse::error(
                    request.id.unwrap_or(Value::Null),
                    JsonRpcError::custom(
                        -32601,
                        "sampling/createMessage should use individual server proxy endpoint".to_string(),
                        Some(json!({
                            "hint": "Use POST /mcp/:server_id for sampling requests from backend servers"
                        }))
                    )
                ))
            }

            "elicitation/requestInput" => {
                self.handle_elicitation_request(session, request).await
            }

            "roots/list" => self.handle_roots_list(session, request).await,

            // Resource subscription methods
            "resources/subscribe" => {
                self.handle_resources_subscribe(session, request).await
            }

            "resources/unsubscribe" => {
                self.handle_resources_unsubscribe(session, request).await
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

    /// Handle resources/subscribe request
    ///
    /// Subscribes to change notifications for a specific resource.
    /// When the resource changes, the backend server sends notifications/resources/updated.
    async fn handle_resources_subscribe(
        &self,
        session: Arc<RwLock<GatewaySession>>,
        request: JsonRpcRequest,
    ) -> AppResult<JsonRpcResponse> {
        // Extract URI from params
        let uri = match request.params.as_ref().and_then(|p| p.get("uri")) {
            Some(Value::String(uri)) => uri.clone(),
            _ => {
                return Ok(JsonRpcResponse::error(
                    request.id.unwrap_or(Value::Null),
                    JsonRpcError::invalid_params("Missing or invalid 'uri' parameter".to_string()),
                ));
            }
        };

        // Look up the server that owns this resource
        let server_id = {
            let session_read = session.read().await;

            // First try resource_uri_mapping
            if let Some((server_id, _)) = session_read.resource_uri_mapping.get(&uri) {
                server_id.clone()
            } else {
                // If not found, we need to determine which server owns this resource
                // Try to match by URI prefix or pattern
                // For now, return an error if not found in mapping
                return Ok(JsonRpcResponse::error(
                    request.id.unwrap_or(Value::Null),
                    JsonRpcError::custom(
                        -32602,
                        format!("Resource URI not found: {}. Call resources/list first to populate mappings.", uri),
                        None,
                    ),
                ));
            }
        };

        // Check if server supports subscriptions
        {
            let session_read = session.read().await;
            if let Some(caps) = &session_read.merged_capabilities {
                let supports_subscribe = caps
                    .capabilities
                    .resources
                    .as_ref()
                    .and_then(|r| r.subscribe)
                    .unwrap_or(false);

                if !supports_subscribe {
                    return Ok(JsonRpcResponse::error(
                        request.id.unwrap_or(Value::Null),
                        JsonRpcError::custom(
                            -32601,
                            "Resource subscriptions not supported by backend servers".to_string(),
                            Some(json!({
                                "workaround": "Use notifications/resources/list_changed for general updates"
                            })),
                        ),
                    ));
                }
            }
        }

        // Forward subscription request to the backend server
        let backend_request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: request.id.clone(),
            method: "resources/subscribe".to_string(),
            params: Some(json!({ "uri": uri })),
        };

        match self
            .server_manager
            .send_request(&server_id, backend_request)
            .await
        {
            Ok(response) => {
                // If successful, track the subscription in the session
                if response.error.is_none() {
                    let mut session_write = session.write().await;
                    session_write.subscribe_resource(uri.clone(), server_id.clone());
                    tracing::info!("Subscribed to resource {} on server {}", uri, server_id);
                }
                Ok(response)
            }
            Err(e) => Ok(JsonRpcResponse::error(
                request.id.unwrap_or(Value::Null),
                JsonRpcError::custom(-32603, format!("Subscription failed: {}", e), None),
            )),
        }
    }

    /// Handle resources/unsubscribe request
    ///
    /// Unsubscribes from change notifications for a specific resource.
    async fn handle_resources_unsubscribe(
        &self,
        session: Arc<RwLock<GatewaySession>>,
        request: JsonRpcRequest,
    ) -> AppResult<JsonRpcResponse> {
        // Extract URI from params
        let uri = match request.params.as_ref().and_then(|p| p.get("uri")) {
            Some(Value::String(uri)) => uri.clone(),
            _ => {
                return Ok(JsonRpcResponse::error(
                    request.id.unwrap_or(Value::Null),
                    JsonRpcError::invalid_params("Missing or invalid 'uri' parameter".to_string()),
                ));
            }
        };

        // Check if we're subscribed and get the server_id
        let server_id = {
            let session_read = session.read().await;
            match session_read.subscribed_resources.get(&uri) {
                Some(server_id) => server_id.clone(),
                None => {
                    // Not subscribed - return success anyway (idempotent)
                    return Ok(JsonRpcResponse::success(
                        request.id.unwrap_or(Value::Null),
                        json!({}),
                    ));
                }
            }
        };

        // Forward unsubscribe request to the backend server
        let backend_request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: request.id.clone(),
            method: "resources/unsubscribe".to_string(),
            params: Some(json!({ "uri": uri })),
        };

        match self
            .server_manager
            .send_request(&server_id, backend_request)
            .await
        {
            Ok(response) => {
                // Remove the subscription from session tracking
                let mut session_write = session.write().await;
                session_write.unsubscribe_resource(&uri);
                tracing::info!("Unsubscribed from resource {} on server {}", uri, server_id);
                Ok(response)
            }
            Err(e) => {
                // Even on error, remove from local tracking
                let mut session_write = session.write().await;
                session_write.unsubscribe_resource(&uri);
                Ok(JsonRpcResponse::error(
                    request.id.unwrap_or(Value::Null),
                    JsonRpcError::custom(-32603, format!("Unsubscribe failed: {}", e), None),
                ))
            }
        }
    }

    /// Handle elicitation/requestInput
    async fn handle_elicitation_request(
        &self,
        _session: Arc<RwLock<GatewaySession>>,
        request: JsonRpcRequest,
    ) -> AppResult<JsonRpcResponse> {
        // Parse elicitation request from params
        let elicitation_req: crate::protocol::ElicitationRequest = match request.params.as_ref() {
            Some(params) => serde_json::from_value(params.clone()).map_err(|e| {
                AppError::InvalidParams(format!("Invalid elicitation request: {}", e))
            })?,
            None => {
                return Ok(JsonRpcResponse::error(
                    request.id.unwrap_or(Value::Null),
                    JsonRpcError::invalid_params(
                        "Missing params for elicitation request".to_string(),
                    ),
                ));
            }
        };

        // Get the server ID that initiated this request
        // Note: In the MCP spec, elicitation/requestInput is sent BY servers TO clients,
        // not the other way around. This handler exists for cases where a client explicitly
        // sends this request to the gateway (uncommon). For proper backend→client elicitation,
        // the notification callback mechanism should be used instead.
        //
        // Try to get server_id from params, fall back to "_gateway" to indicate
        // the request came through the unified gateway rather than a specific backend.
        let server_id = request
            .params
            .as_ref()
            .and_then(|p| p.get("server_id"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| "_gateway".to_string());

        // Request input from user via elicitation manager
        match self
            .elicitation_manager
            .request_input(
                server_id,
                elicitation_req,
                None, // Use default timeout
            )
            .await
        {
            Ok(response) => Ok(JsonRpcResponse::success(
                request.id.unwrap_or(Value::Null),
                serde_json::to_value(response)?,
            )),
            Err(e) => Ok(JsonRpcResponse::error(
                request.id.unwrap_or(Value::Null),
                JsonRpcError::custom(-32603, format!("Elicitation failed: {}", e), None),
            )),
        }
    }

    /// Handle initialize request
    async fn handle_initialize(
        &self,
        session: Arc<RwLock<GatewaySession>>,
        request: JsonRpcRequest,
    ) -> AppResult<JsonRpcResponse> {
        // Extract client capabilities from request params
        // NOTE: Per MCP spec, all clients must handle notifications including listChanged.
        // We store capabilities for potential future features (sampling, roots/listChanged, etc.)
        // but don't gate deferred loading on them since notification support is mandatory.
        let client_capabilities = request
            .params
            .as_ref()
            .and_then(|params| params.get("capabilities"))
            .and_then(|caps| serde_json::from_value::<ClientCapabilities>(caps.clone()).ok());

        // Log raw capabilities JSON for debugging (before struct parsing)
        let raw_capabilities = request
            .params
            .as_ref()
            .and_then(|params| params.get("capabilities"));

        if let Some(raw_caps) = raw_capabilities {
            tracing::info!(
                "Gateway received initialize with raw capabilities: {}",
                raw_caps
            );
        }

        if let Some(ref caps) = client_capabilities {
            tracing::info!(
                "Client capabilities parsed: sampling={}, elicitation={}, roots_listChanged={}",
                caps.supports_sampling(),
                caps.supports_elicitation(),
                caps.roots
                    .as_ref()
                    .and_then(|r| r.list_changed)
                    .unwrap_or(false)
            );
        } else {
            tracing::warn!(
                "Client did not provide capabilities in initialize request (optional per MCP spec)"
            );
        }

        let session_read = session.read().await;
        let allowed_servers = session_read.allowed_servers.clone();
        drop(session_read);

        // Try to start all servers, collecting failures
        // We continue even if some servers fail to start
        let mut start_failures: Vec<ServerFailure> = Vec::new();
        let mut started_servers: Vec<String> = Vec::new();

        for server_id in &allowed_servers {
            if self.server_manager.is_running(server_id) {
                started_servers.push(server_id.clone());
                continue;
            }

            match self.server_manager.start_server(server_id).await {
                Ok(()) => {
                    started_servers.push(server_id.clone());
                }
                Err(e) => {
                    tracing::warn!(
                        "Failed to start MCP server {} during gateway initialization: {}",
                        server_id,
                        e
                    );
                    start_failures.push(ServerFailure {
                        server_id: server_id.clone(),
                        error: e.to_string(),
                    });
                }
            }
        }

        // If no servers could be started, check if skills are available as fallback
        if started_servers.is_empty() {
            // Check if this session has skills access and the gateway has skill support
            let session_read = session.read().await;
            let has_skills =
                session_read.skills_access.has_any_access() && self.has_skill_support();
            drop(session_read);

            if has_skills {
                tracing::info!(
                    "No MCP servers available, but skills are configured — proceeding in skills-only mode"
                );

                let merged = MergedCapabilities {
                    protocol_version: "2024-11-05".to_string(),
                    capabilities: ServerCapabilities {
                        tools: Some(ToolsCapability {
                            list_changed: Some(true),
                        }),
                        resources: None,
                        prompts: None,
                        logging: None,
                    },
                    server_info: ServerInfo {
                        name: "LocalRouter MCP Gateway (skills-only)".to_string(),
                        version: env!("CARGO_PKG_VERSION").to_string(),
                        description: None,
                    },
                    failures: start_failures,
                };

                let response_value = json!({
                    "protocolVersion": merged.protocol_version,
                    "capabilities": {
                        "tools": { "listChanged": true }
                    },
                    "serverInfo": {
                        "name": merged.server_info.name,
                        "version": merged.server_info.version
                    }
                });

                {
                    let mut session_write = session.write().await;
                    session_write.merged_capabilities = Some(merged);
                }

                return Ok(JsonRpcResponse::success(
                    request.id.unwrap_or(Value::Null),
                    response_value,
                ));
            }

            let error_summary = start_failures
                .iter()
                .map(|f| format!("{}: {}", f.server_id, f.error))
                .collect::<Vec<_>>()
                .join("; ");
            return Err(AppError::Mcp(format!(
                "All MCP servers failed to start: {}",
                error_summary
            )));
        }

        // Update session to only include successfully started servers
        if !start_failures.is_empty() {
            let mut session_write = session.write().await;
            session_write.allowed_servers = started_servers.clone();
            // Remove init status for failed servers
            for failure in &start_failures {
                session_write.server_init_status.remove(&failure.server_id);
            }
            drop(session_write);
            tracing::info!(
                "Gateway proceeding with {} of {} servers ({} failed to start)",
                started_servers.len(),
                allowed_servers.len(),
                start_failures.len()
            );
        }

        // Use only the successfully started servers for broadcast
        let servers_to_initialize = started_servers;

        // Broadcast initialize to all successfully started servers
        let timeout = Duration::from_secs(self.config.server_timeout_seconds);
        let max_retries = self.config.max_retry_attempts;

        let results = broadcast_request(
            &servers_to_initialize,
            request.clone(),
            &self.server_manager,
            timeout,
            max_retries,
        )
        .await;

        // Separate successes and failures
        let (successes, mut failures) = separate_results(results);

        // Include servers that failed to start in the failures list
        failures.extend(start_failures);

        // Parse initialize results
        let init_results: Vec<(String, InitializeResult)> = successes
            .into_iter()
            .filter_map(|(server_id, value)| {
                serde_json::from_value::<InitializeResult>(value)
                    .ok()
                    .map(|result| (server_id, result))
            })
            .collect();

        // If all servers failed, check if skills can serve as fallback
        if init_results.is_empty() && !failures.is_empty() {
            let session_read = session.read().await;
            let has_skills =
                session_read.skills_access.has_any_access() && self.has_skill_support();
            drop(session_read);

            if has_skills {
                tracing::info!(
                    "All MCP servers failed to initialize, but skills are configured — proceeding in skills-only mode"
                );

                let merged = MergedCapabilities {
                    protocol_version: "2024-11-05".to_string(),
                    capabilities: ServerCapabilities {
                        tools: Some(ToolsCapability {
                            list_changed: Some(true),
                        }),
                        resources: None,
                        prompts: None,
                        logging: None,
                    },
                    server_info: ServerInfo {
                        name: "LocalRouter MCP Gateway (skills-only)".to_string(),
                        version: env!("CARGO_PKG_VERSION").to_string(),
                        description: None,
                    },
                    failures,
                };

                let response_value = json!({
                    "protocolVersion": merged.protocol_version,
                    "capabilities": {
                        "tools": { "listChanged": true }
                    },
                    "serverInfo": {
                        "name": merged.server_info.name,
                        "version": merged.server_info.version
                    }
                });

                {
                    let mut session_write = session.write().await;
                    session_write.merged_capabilities = Some(merged);
                }

                return Ok(JsonRpcResponse::success(
                    request.id.unwrap_or(Value::Null),
                    response_value,
                ));
            }

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

        // Merge results (includes both start failures and initialize failures)
        let merged = merge_initialize_results(init_results, failures);

        // Store capabilities in session
        {
            let mut session_write = session.write().await;
            session_write.merged_capabilities = Some(merged.clone());
            session_write.client_capabilities = client_capabilities.clone();
        }

        // Enable deferred loading if requested AND client supports listChanged notifications
        // The unified MCP gateway acts as a server to the client and sends listChanged notifications
        // when tools are activated via the search tool. Backend MCP servers don't need to support
        // listChanged - they just provide their tools normally.
        let session_read = session.read().await;
        let should_enable_deferred = session_read.deferred_loading_requested;
        let client_id_for_log = session_read.client_id.clone();
        let allowed_servers_for_deferred = session_read.allowed_servers.clone();
        drop(session_read);

        if should_enable_deferred {
            // Check if client supports receiving listChanged notifications
            // For internal-test client (Try it out UI), we skip this check since we know it can handle notifications
            let is_internal_test = client_id_for_log == "internal-test";
            let client_supports_notifications = is_internal_test
                || client_capabilities
                    .as_ref()
                    .map(|caps| caps.supports_tools_list_changed())
                    .unwrap_or(false);

            if !client_supports_notifications {
                tracing::warn!(
                    "Deferred loading requested for client {} but client does not support tools.listChanged notifications. \
                     Falling back to normal mode. Client must declare {{ tools: {{ listChanged: true }} }} in initialize capabilities.",
                    client_id_for_log
                );
            } else {
                tracing::info!(
                    "Setting up deferred loading for client {}: client supports listChanged, fetching full catalog from backend servers",
                    client_id_for_log
                );

                // Note: Servers should already be started from the initialization phase above.
                // The allowed_servers_for_deferred list only contains successfully started servers.
                // We skip any that may have stopped in the meantime.
                let mut active_servers = Vec::new();
                for server_id in &allowed_servers_for_deferred {
                    if self.server_manager.is_running(server_id) {
                        active_servers.push(server_id.clone());
                    } else {
                        // Try to restart if needed, but don't fail the whole operation
                        match self.server_manager.start_server(server_id).await {
                            Ok(()) => active_servers.push(server_id.clone()),
                            Err(e) => {
                                tracing::warn!(
                                    "Failed to start server {} for deferred loading catalog: {}",
                                    server_id,
                                    e
                                );
                            }
                        }
                    }
                }

                // Fetch full catalog from active backend servers
                let (tools, _) = self
                    .fetch_and_merge_tools(
                        &active_servers,
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
                        &active_servers,
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
                        &active_servers,
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
                "Deferred loading enabled for client {}: {} tools, {} resources, {} prompts in catalog",
                client_id_for_log,
                tools.len(),
                resources.len(),
                prompts.len(),
            );
            } // end of if client_supports_notifications
        } // end of if should_enable_deferred

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
                // Return only search tool + activated tools + skill tools
                let mut tools: Vec<serde_json::Value> =
                    vec![serde_json::to_value(create_search_tool()).unwrap_or_default()];

                for tool_name in &deferred.activated_tools {
                    if let Some(tool) = deferred.full_catalog.iter().find(|t| t.name == *tool_name)
                    {
                        tools.push(serde_json::to_value(tool).unwrap_or_default());
                    }
                }

                let skills_access = session_read.skills_access.clone();
                drop(session_read);

                self.append_skill_tools(&mut tools, &skills_access);

                return Ok(JsonRpcResponse::success(
                    request.id.unwrap_or(Value::Null),
                    json!({"tools": tools}),
                ));
            }
        }

        // Check cache
        if let Some(cached) = &session_read.cached_tools {
            if cached.is_valid() {
                let mut tools: Vec<serde_json::Value> = cached
                    .data
                    .iter()
                    .map(|t| serde_json::to_value(t).unwrap_or_default())
                    .collect();

                let skills_access = session_read.skills_access.clone();
                drop(session_read);

                self.append_skill_tools(&mut tools, &skills_access);

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
        let skills_access = session_read.skills_access.clone();
        drop(session_read);

        let mut all_tools: Vec<serde_json::Value> = tools
            .iter()
            .map(|t| serde_json::to_value(t).unwrap_or_default())
            .collect();

        self.append_skill_tools(&mut all_tools, &skills_access);

        let mut result = json!({"tools": all_tools});
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

        // Build server ID to human-readable name mapping
        let name_mapping = self.build_server_id_to_name_mapping(server_ids);

        Ok((
            merge_tools(server_tools, &failures, Some(&name_mapping)),
            failures,
        ))
    }

    /// Handle resources/list request
    async fn handle_resources_list(
        &self,
        session: Arc<RwLock<GatewaySession>>,
        request: JsonRpcRequest,
    ) -> AppResult<JsonRpcResponse> {
        let request_id = request.id.clone();
        let session_read = session.read().await;

        // Check cache
        if let Some(cached) = &session_read.cached_resources {
            if cached.is_valid() {
                let resources = cached.data.clone();
                drop(session_read);

                tracing::debug!(
                    "resources/list returning {} cached resources (request_id={:?})",
                    resources.len(),
                    request_id
                );

                return Ok(JsonRpcResponse::success(
                    request.id.unwrap_or(Value::Null),
                    json!({"resources": resources}),
                ));
            }
        }

        let allowed_servers = session_read.allowed_servers.clone();
        drop(session_read);

        tracing::info!(
            "resources/list fetching from {} servers (request_id={:?})",
            allowed_servers.len(),
            request_id
        );

        // Fetch from servers
        let (resources, failures) = self
            .fetch_and_merge_resources(&allowed_servers, request.clone())
            .await?;

        tracing::info!(
            "resources/list fetched {} resources with {} failures (request_id={:?})",
            resources.len(),
            failures.len(),
            request_id
        );

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

        // Build server ID to human-readable name mapping
        let name_mapping = self.build_server_id_to_name_mapping(server_ids);

        Ok((
            merge_resources(server_resources, &failures, Some(&name_mapping)),
            failures,
        ))
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

        // Build server ID to human-readable name mapping
        let name_mapping = self.build_server_id_to_name_mapping(server_ids);

        Ok((
            merge_prompts(server_prompts, &failures, Some(&name_mapping)),
            failures,
        ))
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
            Some(name) => name.to_string(),
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

        // Check if it's a skill tool
        if self.is_skill_tool(&tool_name) {
            return self
                .handle_skill_tool_call(session, &tool_name, request)
                .await;
        }

        // Look up tool in session mapping to get server_id (UUID) and original_name
        // The mapping stores: namespaced_name -> (server_id, original_name)
        // where namespaced_name uses human-readable server name but server_id is the UUID for routing
        let session_read = session.read().await;
        let (server_id, original_name) = match session_read.tool_mapping.get(&tool_name) {
            Some((id, name)) => (id.clone(), name.clone()),
            None => {
                drop(session_read);
                return Ok(JsonRpcResponse::error(
                    request.id.unwrap_or(Value::Null),
                    JsonRpcError::tool_not_found(&tool_name),
                ));
            }
        };
        drop(session_read);

        // Transform request: Strip namespace
        let mut transformed_request = request.clone();
        if let Some(params) = transformed_request.params.as_mut() {
            if let Some(obj) = params.as_object_mut() {
                obj.insert("name".to_string(), json!(original_name));
            }
        }

        tracing::info!(
            "Gateway routing tools/call to server {}: tool={}, request_id={:?}",
            server_id,
            original_name,
            transformed_request.id
        );

        // Route to server
        let result = self
            .server_manager
            .send_request(&server_id, transformed_request)
            .await;

        match &result {
            Ok(response) => {
                tracing::info!(
                    "Gateway received response from server {}: response_id={:?}, has_error={}",
                    server_id,
                    response.id,
                    response.error.is_some()
                );
            }
            Err(e) => {
                tracing::error!(
                    "Gateway failed to get response from server {}: {}",
                    server_id,
                    e
                );
            }
        }

        result
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

        let search_type = arguments
            .get("type")
            .and_then(|t| t.as_str())
            .unwrap_or("all");

        let limit = arguments
            .get("limit")
            .and_then(|l| l.as_u64())
            .unwrap_or(10) as usize;

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
            // Look up resource in session mapping to get server_id (UUID) and original_name
            let session_read = session.read().await;
            match session_read.resource_mapping.get(name) {
                Some((id, orig)) => {
                    let result = (id.clone(), orig.clone());
                    drop(session_read);
                    result
                }
                None => {
                    drop(session_read);
                    return Ok(JsonRpcResponse::error(
                        request.id.unwrap_or(Value::Null),
                        JsonRpcError::resource_not_found(format!("Resource not found: {}", name)),
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

        // Look up prompt in session mapping to get server_id (UUID) and original_name
        let session_read = session.read().await;
        let (server_id, original_name) = match session_read.prompt_mapping.get(prompt_name) {
            Some((id, name)) => (id.clone(), name.clone()),
            None => {
                drop(session_read);
                return Ok(JsonRpcResponse::error(
                    request.id.unwrap_or(Value::Null),
                    JsonRpcError::prompt_not_found(prompt_name),
                ));
            }
        };
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

    /// Get the elicitation manager (for submitting responses from external clients)
    pub fn get_elicitation_manager(&self) -> Arc<ElicitationManager> {
        self.elicitation_manager.clone()
    }

    /// Append skill tools to a tools list if the client has skills access
    fn append_skill_tools(
        &self,
        tools: &mut Vec<serde_json::Value>,
        access: &lr_config::SkillsAccess,
    ) {
        if access.has_any_access() {
            if let Some(sm) = self.skill_manager.get() {
                let skill_tools = lr_skills::mcp_tools::build_skill_tools(sm, access);
                for st in skill_tools {
                    tools.push(serde_json::to_value(&st).unwrap_or_default());
                }
            }
        }
    }

    /// Check if a tool name matches a skill tool pattern
    fn is_skill_tool(&self, tool_name: &str) -> bool {
        tool_name.starts_with("show-skill_")
            || tool_name == "get-skill-resource"
            || tool_name == "run-skill-script"
            || tool_name == "get-skill-script-run"
    }

    /// Handle a skill tool call
    async fn handle_skill_tool_call(
        &self,
        session: Arc<RwLock<GatewaySession>>,
        tool_name: &str,
        request: JsonRpcRequest,
    ) -> AppResult<JsonRpcResponse> {
        let (skill_manager, script_executor) =
            match (self.skill_manager.get(), self.script_executor.get()) {
                (Some(sm), Some(se)) => (sm, se),
                _ => {
                    return Ok(JsonRpcResponse::error(
                        request.id.unwrap_or(Value::Null),
                        JsonRpcError::custom(
                            -32601,
                            "Skills support is not configured".to_string(),
                            None,
                        ),
                    ));
                }
            };

        // Get skills access from session
        let session_read = session.read().await;
        let skills_access = session_read.skills_access.clone();
        drop(session_read);

        // Extract arguments from params
        let arguments = request
            .params
            .as_ref()
            .and_then(|p| p.get("arguments"))
            .cloned()
            .unwrap_or(json!({}));

        match lr_skills::mcp_tools::handle_skill_tool_call(
            tool_name,
            &arguments,
            skill_manager,
            script_executor,
            &skills_access,
        )
        .await
        {
            Ok(Some(response)) => Ok(JsonRpcResponse::success(
                request.id.unwrap_or(Value::Null),
                response,
            )),
            Ok(None) => Ok(JsonRpcResponse::error(
                request.id.unwrap_or(Value::Null),
                JsonRpcError::tool_not_found(tool_name),
            )),
            Err(e) => Ok(JsonRpcResponse::success(
                request.id.unwrap_or(Value::Null),
                json!({
                    "content": [{
                        "type": "text",
                        "text": format!("Error: {}", e)
                    }],
                    "isError": true
                }),
            )),
        }
    }

    /// Get a session by client ID
    pub fn get_session(&self, client_id: &str) -> Option<Arc<RwLock<GatewaySession>>> {
        self.sessions.get(client_id).map(|s| s.clone())
    }
}
