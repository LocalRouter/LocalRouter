#![allow(dead_code)]

use dashmap::DashMap;
use serde_json::{json, Value};
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tokio::sync::RwLock;

use crate::manager::McpServerManager;
use crate::protocol::{JsonRpcError, JsonRpcNotification, JsonRpcRequest, JsonRpcResponse};
use lr_marketplace::MarketplaceService;
use lr_router::Router;
use lr_skills::executor::ScriptExecutor;
use lr_skills::manager::SkillManager;
use lr_types::{AppError, AppResult};

use super::elicitation::ElicitationManager;
use super::firewall::FirewallManager;
use super::merger::{
    build_gateway_instructions, merge_initialize_results, InstructionsContext,
    McpServerInstructionInfo, SkillInfo, UnavailableServerInfo,
};
use super::router::{broadcast_request, separate_results, should_broadcast};
use super::session::GatewaySession;
use super::types::*;

/// MCP Gateway - Unified endpoint for multiple MCP servers
pub struct McpGateway {
    /// Active sessions (client_id -> session)
    pub(crate) sessions: Arc<DashMap<String, Arc<RwLock<GatewaySession>>>>,

    /// MCP server manager
    pub(crate) server_manager: Arc<McpServerManager>,

    /// Gateway configuration
    pub(crate) config: GatewayConfig,

    /// Track which servers have global notification handlers registered
    pub(crate) notification_handlers_registered: Arc<DashMap<String, bool>>,

    /// Broadcast sender for client notifications (optional)
    /// Allows external clients to subscribe to real-time notifications from MCP servers
    /// Format: (server_id, notification)
    pub(crate) notification_broadcast:
        Option<Arc<tokio::sync::broadcast::Sender<(String, JsonRpcNotification)>>>,

    /// Router for LLM provider access (for sampling/createMessage support)
    pub(crate) router: Arc<Router>,

    /// Elicitation manager for handling structured user input requests
    pub(crate) elicitation_manager: Arc<ElicitationManager>,

    /// Firewall manager for tool call approval flow
    pub firewall_manager: Arc<FirewallManager>,

    /// Skill manager (optional, for AgentSkills.io support)
    /// Uses OnceLock so it can be set after Arc construction via &self
    pub(crate) skill_manager: OnceLock<Arc<SkillManager>>,

    /// Script executor for running skill scripts (optional)
    pub(crate) script_executor: OnceLock<Arc<ScriptExecutor>>,

    /// Override for skills async enabled (set via set_skills_async_enabled)
    pub(crate) skills_async_override: OnceLock<bool>,

    /// Marketplace service (optional, for MCP server/skill discovery)
    /// Uses OnceLock so it can be set after Arc construction via &self
    pub(crate) marketplace_service: OnceLock<Arc<MarketplaceService>>,
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

        // Create firewall manager with broadcast support if available
        let firewall_manager = match &notification_broadcast {
            Some(broadcast) => {
                Arc::new(FirewallManager::new_with_broadcast(120, broadcast.clone()))
            }
            None => Arc::new(FirewallManager::default()),
        };

        Self {
            sessions: Arc::new(DashMap::new()),
            server_manager,
            config,
            notification_handlers_registered: Arc::new(DashMap::new()),
            notification_broadcast,
            router,
            elicitation_manager,
            firewall_manager,
            skill_manager: OnceLock::new(),
            script_executor: OnceLock::new(),
            skills_async_override: OnceLock::new(),
            marketplace_service: OnceLock::new(),
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

    /// Enable or disable async skill script execution.
    ///
    /// Updates the gateway config's `skills_async_enabled` flag.
    /// This is applied to new sessions and existing sessions on their next request.
    pub fn set_skills_async_enabled(&self, enabled: bool) {
        // Note: config is not behind a lock since it's set at construction time.
        // This is a best-effort update for the async flag; new sessions will
        // pick it up via handle_request_with_skills.
        // For existing sessions, it's propagated when skills_access is set.
        //
        // Since GatewayConfig is in a plain field (not Arc/RwLock), we can't
        // mutate it after construction. Instead, we store the flag on the gateway
        // struct itself and check it in handle_request_with_skills.
        let _ = self.skills_async_override.set(enabled);
    }

    /// Set marketplace service for MCP server/skill discovery.
    /// Uses OnceLock so this can be called on `&self` (gateway is behind Arc).
    pub fn set_marketplace_service(&self, service: Arc<MarketplaceService>) {
        let _ = self.marketplace_service.set(service);
    }

    /// Check if skill support has been configured
    fn has_skill_support(&self) -> bool {
        self.skill_manager.get().is_some() && self.script_executor.get().is_some()
    }

    /// Collect skill info for building gateway instructions.
    /// Returns skill metadata for all skills accessible to the given access level.
    fn collect_skill_info(&self, access: &lr_config::SkillsAccess) -> Vec<SkillInfo> {
        if !access.has_any_access() {
            return Vec::new();
        }
        let Some(sm) = self.skill_manager.get() else {
            return Vec::new();
        };
        let all_skills = sm.get_all();
        all_skills
            .iter()
            .filter(|s| s.enabled && access.can_access_by_name(&s.metadata.name))
            .map(|s| {
                let sname = lr_skills::types::sanitize_name(&s.metadata.name);
                SkillInfo {
                    name: s.metadata.name.clone(),
                    description: s.metadata.description.clone(),
                    get_info_tool: format!("skill_{}_get_info", sname),
                }
            })
            .collect()
    }

    /// Build `McpServerInstructionInfo` list from init results and catalogs.
    ///
    /// Maps server UUIDs to human-readable names and groups tools/resources/prompts by server.
    fn build_server_instruction_infos(
        &self,
        init_results: &[(String, InitializeResult)],
        tools: &[NamespacedTool],
        resources: &[NamespacedResource],
        prompts: &[NamespacedPrompt],
    ) -> Vec<McpServerInstructionInfo> {
        let name_mapping = self.build_server_id_to_name_mapping(
            &init_results
                .iter()
                .map(|(id, _)| id.clone())
                .collect::<Vec<_>>(),
        );

        init_results
            .iter()
            .map(|(server_id, result)| {
                let name = name_mapping
                    .get(server_id)
                    .cloned()
                    .unwrap_or_else(|| server_id.clone());

                let tool_names: Vec<String> = tools
                    .iter()
                    .filter(|t| t.server_id == *server_id)
                    .map(|t| t.name.clone())
                    .collect();

                let resource_names: Vec<String> = resources
                    .iter()
                    .filter(|r| r.server_id == *server_id)
                    .map(|r| r.name.clone())
                    .collect();

                let prompt_names: Vec<String> = prompts
                    .iter()
                    .filter(|p| p.server_id == *server_id)
                    .map(|p| p.name.clone())
                    .collect();

                McpServerInstructionInfo {
                    name,
                    instructions: result.instructions.clone(),
                    description: result.server_info.description.clone(),
                    tool_names,
                    resource_names,
                    prompt_names,
                }
            })
            .collect()
    }

    /// Build `UnavailableServerInfo` list from server failures.
    fn build_unavailable_server_infos(
        &self,
        failures: &[ServerFailure],
    ) -> Vec<UnavailableServerInfo> {
        failures
            .iter()
            .map(|f| {
                let name = self
                    .server_manager
                    .get_config(&f.server_id)
                    .map(|c| crate::gateway::types::slugify(&c.name))
                    .unwrap_or_else(|| f.server_id.clone());
                UnavailableServerInfo {
                    name,
                    error: f.error.clone(),
                }
            })
            .collect()
    }

    /// Build a mapping from server ID (UUID) to slugified server name
    ///
    /// This is used to namespace tools/resources/prompts with readable names
    /// (e.g., "filesystem__read_file") instead of UUIDs. The name is slugified
    /// so the same form is used for tool prefixes and XML instruction tags.
    pub(crate) fn build_server_id_to_name_mapping(
        &self,
        server_ids: &[String],
    ) -> std::collections::HashMap<String, String> {
        let mut mapping = std::collections::HashMap::new();
        for server_id in server_ids {
            if let Some(config) = self.server_manager.get_config(server_id) {
                mapping.insert(
                    server_id.clone(),
                    crate::gateway::types::slugify(&config.name),
                );
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
            lr_config::FirewallRules::default(),
            String::new(),
            false, // marketplace_enabled
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
        firewall_rules: lr_config::FirewallRules,
        client_name: String,
        marketplace_enabled: bool,
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

        // Update skills access and async config on session
        if skills_access.has_any_access() {
            let async_enabled = self
                .skills_async_override
                .get()
                .copied()
                .unwrap_or(self.config.skills_async_enabled);
            let mut session_write = session.write().await;
            session_write.skills_access = skills_access;
            session_write.skills_async_enabled = async_enabled;
        }

        // Update firewall rules, client name, and marketplace access on session (always refresh from config)
        {
            let mut session_write = session.write().await;
            session_write.firewall_rules = firewall_rules;
            session_write.client_name = client_name;
            session_write.marketplace_enabled = marketplace_enabled;
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
                // Check if allowed servers changed (e.g. switching between direct/all modes)
                // If so, drop the stale session and create a fresh one since cached state
                // (tool mappings, init statuses, etc.) is tied to the server list
                let mut servers_sorted = allowed_servers.clone();
                servers_sorted.sort();
                let mut existing_sorted = session_read.allowed_servers.clone();
                existing_sorted.sort();
                let servers_changed = servers_sorted != existing_sorted;
                drop(session_read);

                if servers_changed {
                    tracing::info!(
                        "Allowed servers changed for session {} - recreating session",
                        client_id,
                    );
                    drop(session);
                    self.sessions.remove(client_id);
                    // Fall through to create new session below
                } else {
                    // Update deferred loading setting if it changed
                    // This allows the Try it out UI to toggle deferred loading between connections
                    let session_read = session.read().await;
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

                let session_read = session.read().await;
                let skills_access = session_read.skills_access.clone();
                drop(session_read);

                let skill_infos = self.collect_skill_info(&skills_access);
                let unavailable = self.build_unavailable_server_infos(&start_failures);
                let instructions = build_gateway_instructions(&InstructionsContext {
                    servers: Vec::new(),
                    unavailable_servers: unavailable,
                    skills: skill_infos,
                    deferred_loading: false,
                });

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
                    instructions: instructions.clone(),
                };

                let mut response_value = json!({
                    "protocolVersion": merged.protocol_version,
                    "capabilities": {
                        "tools": { "listChanged": true }
                    },
                    "serverInfo": {
                        "name": merged.server_info.name,
                        "version": merged.server_info.version
                    }
                });
                if let Some(inst) = &instructions {
                    response_value["instructions"] = json!(inst);
                }

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

                let session_read = session.read().await;
                let skills_access = session_read.skills_access.clone();
                drop(session_read);

                let skill_infos = self.collect_skill_info(&skills_access);
                let unavailable = self.build_unavailable_server_infos(&failures);
                let instructions = build_gateway_instructions(&InstructionsContext {
                    servers: Vec::new(),
                    unavailable_servers: unavailable,
                    skills: skill_infos,
                    deferred_loading: false,
                });

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
                    instructions: instructions.clone(),
                };

                let mut response_value = json!({
                    "protocolVersion": merged.protocol_version,
                    "capabilities": {
                        "tools": { "listChanged": true }
                    },
                    "serverInfo": {
                        "name": merged.server_info.name,
                        "version": merged.server_info.version
                    }
                });
                if let Some(inst) = &instructions {
                    response_value["instructions"] = json!(inst);
                }

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
        // Keep a clone of init_results for building instructions later
        let init_results_for_instructions = init_results.clone();
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

                // Check per-type capabilities for resources and prompts
                let resources_deferred = is_internal_test
                    || client_capabilities
                        .as_ref()
                        .map(|caps| caps.supports_resources_list_changed())
                        .unwrap_or(false);
                let prompts_deferred = is_internal_test
                    || client_capabilities
                        .as_ref()
                        .map(|caps| caps.supports_prompts_list_changed())
                        .unwrap_or(false);

                let mut session_write = session.write().await;
                session_write.deferred_loading = Some(DeferredLoadingState {
                    enabled: true,
                    resources_deferred,
                    prompts_deferred,
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

        // Build gateway instructions based on the full context
        let session_read = session.read().await;
        let skills_access = session_read.skills_access.clone();
        let deferred_state = session_read.deferred_loading.clone();
        let active_server_ids = session_read.allowed_servers.clone();
        drop(session_read);

        let has_skills = skills_access.has_any_access() && self.has_skill_support();
        let skill_infos = if has_skills {
            self.collect_skill_info(&skills_access)
        } else {
            Vec::new()
        };

        let deferred_enabled = deferred_state.as_ref().map(|d| d.enabled).unwrap_or(false);

        // Get the full catalogs for building instructions.
        // In deferred mode, we already have them; in normal mode, fetch now.
        let (tools_catalog, resources_catalog, prompts_catalog) =
            if let Some(ref ds) = deferred_state {
                (
                    ds.full_catalog.clone(),
                    ds.full_resource_catalog.clone(),
                    ds.full_prompt_catalog.clone(),
                )
            } else {
                let tools = self
                    .fetch_and_merge_tools(
                        &active_server_ids,
                        JsonRpcRequest::new(
                            Some(serde_json::json!("_instructions_tools")),
                            "tools/list".to_string(),
                            None,
                        ),
                    )
                    .await
                    .map(|(t, _)| t)
                    .unwrap_or_default();

                let resources = self
                    .fetch_and_merge_resources(
                        &active_server_ids,
                        JsonRpcRequest::new(
                            Some(serde_json::json!("_instructions_resources")),
                            "resources/list".to_string(),
                            None,
                        ),
                    )
                    .await
                    .map(|(r, _)| r)
                    .unwrap_or_default();

                let prompts = self
                    .fetch_and_merge_prompts(
                        &active_server_ids,
                        JsonRpcRequest::new(
                            Some(serde_json::json!("_instructions_prompts")),
                            "prompts/list".to_string(),
                            None,
                        ),
                    )
                    .await
                    .map(|(p, _)| p)
                    .unwrap_or_default();

                (tools, resources, prompts)
            };

        let server_infos = self.build_server_instruction_infos(
            &init_results_for_instructions,
            &tools_catalog,
            &resources_catalog,
            &prompts_catalog,
        );
        let unavailable = self.build_unavailable_server_infos(&merged.failures);

        let instructions = build_gateway_instructions(&InstructionsContext {
            servers: server_infos,
            unavailable_servers: unavailable,
            skills: skill_infos,
            deferred_loading: deferred_enabled,
        });

        // Store instructions in merged capabilities
        if instructions.is_some() {
            let mut session_write = session.write().await;
            if let Some(ref mut mc) = session_write.merged_capabilities {
                mc.instructions = instructions.clone();
            }
        }

        // Build response
        let mut result = json!({
            "protocolVersion": merged.protocol_version,
            "capabilities": merged.capabilities,
            "serverInfo": merged.server_info,
        });
        if let Some(inst) = &instructions {
            result["instructions"] = json!(inst);
        }

        Ok(JsonRpcResponse::success(
            request.id.unwrap_or(Value::Null),
            result,
        ))
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

    /// Get a session by client ID
    pub fn get_session(&self, client_id: &str) -> Option<Arc<RwLock<GatewaySession>>> {
        self.sessions.get(client_id).map(|s| s.clone())
    }
}
