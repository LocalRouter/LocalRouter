use dashmap::DashMap;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

use crate::manager::McpServerManager;
use crate::protocol::{JsonRpcError, JsonRpcNotification, JsonRpcRequest, JsonRpcResponse};
use lr_router::Router;
use lr_types::{AppError, AppResult};

use super::elicitation::ElicitationManager;
use super::firewall::FirewallManager;
use super::merger::{
    build_full_server_content, build_gateway_instructions, compute_catalog_compression_plan,
    compute_item_definition_sizes, format_prompt_as_markdown, format_resource_as_markdown,
    format_tool_as_markdown, merge_initialize_results, InstructionsContext,
    McpServerInstructionInfo, UnavailableServerInfo,
};
use super::router::{broadcast_request, separate_results, should_broadcast};
use super::session::GatewaySession;
use super::types::*;
use super::virtual_server::VirtualMcpServer;

/// MCP Gateway - Unified endpoint for multiple MCP servers
#[allow(clippy::type_complexity)]
pub struct McpGateway {
    /// Active sessions (session_key -> session)
    /// For SSE connections, session_key is a per-connection UUID.
    /// For non-SSE connections, session_key defaults to client_id.
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
    #[allow(dead_code)]
    pub(crate) router: Arc<Router>,

    /// Elicitation manager for handling structured user input requests
    pub(crate) elicitation_manager: Arc<ElicitationManager>,

    /// Firewall manager for tool call approval flow
    pub firewall_manager: Arc<FirewallManager>,

    /// Coding agent approval manager for agent tool/question approvals
    pub coding_agent_approval_manager: Arc<super::coding_agent_approval::CodingAgentApprovalManager>,

    /// Virtual MCP servers (skills, marketplace, coding agents)
    pub(crate) virtual_servers: parking_lot::RwLock<Vec<Arc<dyn VirtualMcpServer>>>,

    /// Callback to record context management bytes saved (wired to metrics DB)
    pub(crate) on_context_saved: parking_lot::RwLock<Option<Arc<dyn Fn(u64) + Send + Sync>>>,
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
            Some(broadcast) => Arc::new(FirewallManager::new_with_broadcast(
                86400,
                broadcast.clone(),
            )),
            None => Arc::new(FirewallManager::default()),
        };

        // Create coding agent approval manager with broadcast support if available
        let coding_agent_approval_manager = match &notification_broadcast {
            Some(broadcast) => {
                Arc::new(super::coding_agent_approval::CodingAgentApprovalManager::new_with_broadcast(
                    120,
                    broadcast.clone(),
                ))
            }
            None => Arc::new(super::coding_agent_approval::CodingAgentApprovalManager::new(120)),
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
            coding_agent_approval_manager,
            virtual_servers: parking_lot::RwLock::new(Vec::new()),
            on_context_saved: parking_lot::RwLock::new(None),
        }
    }

    /// Set callback to record context management bytes saved to the metrics database
    pub fn set_on_context_saved<F: Fn(u64) + Send + Sync + 'static>(&self, callback: F) {
        *self.on_context_saved.write() = Some(Arc::new(callback));
    }

    /// Register a virtual MCP server (skills, marketplace, coding agents, etc.)
    pub fn register_virtual_server(&self, server: Arc<dyn VirtualMcpServer>) {
        self.virtual_servers.write().push(server);
    }

    /// Check if skill support has been configured (via virtual server)
    pub fn has_skill_support(&self) -> bool {
        self.virtual_servers
            .read()
            .iter()
            .any(|vs| vs.id() == "_skills")
    }

    /// List virtual server indexing info for UI display.
    ///
    /// Returns id, display_name, and tools with indexable flag for each virtual server.
    pub fn list_virtual_server_indexing_info(&self) -> Vec<VirtualServerIndexingInfo> {
        self.virtual_servers
            .read()
            .iter()
            .map(|vs| {
                let tool_names = vs.all_tool_names();
                let tools: Vec<VirtualToolIndexingInfo> = tool_names
                    .into_iter()
                    .map(|name| {
                        let indexable = vs.is_tool_indexable(&name);
                        VirtualToolIndexingInfo { name, indexable }
                    })
                    .collect();
                VirtualServerIndexingInfo {
                    id: vs.id().to_string(),
                    display_name: vs.display_name().to_string(),
                    tools,
                }
            })
            .collect()
    }

    /// Collect instructions from all registered virtual servers.
    ///
    /// For each virtual server, populates `tool_names` from `list_tools()`.
    /// If `build_instructions()` returns `None` but tools exist, creates a
    /// minimal `VirtualInstructions` with just the display name and tool names.
    async fn collect_virtual_instructions(
        &self,
        session: &Arc<RwLock<GatewaySession>>,
    ) -> Vec<super::virtual_server::VirtualInstructions> {
        let session_read = session.read().await;
        let mut result = Vec::new();

        for vs in self.virtual_servers.read().iter() {
            let state = match session_read.virtual_server_state.get(vs.id()) {
                Some(s) => s,
                None => continue,
            };

            let tool_names: Vec<String> = vs
                .list_tools(state.as_ref())
                .into_iter()
                .map(|t| t.name)
                .collect();

            if let Some(mut instructions) = vs.build_instructions(state.as_ref()) {
                instructions.tool_names = tool_names;
                result.push(instructions);
            } else if !tool_names.is_empty() {
                // Virtual server has tools but no instructions — create minimal entry
                result.push(super::virtual_server::VirtualInstructions {
                    section_title: vs.display_name().to_string(),
                    content: String::new(),
                    tool_names,
                    priority: 50,
                });
            }
        }

        result.sort_by_key(|v| v.priority);
        result
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
    ///
    /// Uses Allow-all permissions since no explicit permissions are provided.
    /// For permission-controlled access, use `handle_request_with_skills`.
    pub async fn handle_request(
        &self,
        client_id: &str,
        allowed_servers: Vec<String>,
        roots: Vec<crate::protocol::Root>,
        request: JsonRpcRequest,
    ) -> AppResult<JsonRpcResponse> {
        // Default to Allow-all MCP permissions when no explicit permissions are provided
        let mcp_permissions = lr_config::McpPermissions {
            global: lr_config::PermissionState::Allow,
            ..Default::default()
        };
        self.handle_request_with_skills(
            client_id,
            None, // no session_id, uses client_id as session key
            allowed_servers,
            roots,
            mcp_permissions,
            lr_config::SkillsPermissions::default(),
            String::new(),
            lr_config::PermissionState::Off, // marketplace_permission
            lr_config::PermissionState::Off, // coding_agent_permission
            None,                            // coding_agent_type
            None,                            // context_management_overrides
            lr_config::PermissionState::default(), // mcp_sampling_permission
            lr_config::PermissionState::default(), // mcp_elicitation_permission
            request,
        )
        .await
    }

    /// Handle an MCP gateway request with skill access
    ///
    /// `session_id` is an optional per-connection identifier (UUID). When provided,
    /// it is used as the session key, allowing multiple simultaneous connections
    /// from the same client. When `None`, `client_id` is used as the session key.
    #[allow(clippy::too_many_arguments)]
    pub async fn handle_request_with_skills(
        &self,
        client_id: &str,
        session_id: Option<&str>,
        allowed_servers: Vec<String>,
        roots: Vec<crate::protocol::Root>,
        mcp_permissions: lr_config::McpPermissions,
        skills_permissions: lr_config::SkillsPermissions,
        client_name: String,
        marketplace_permission: lr_config::PermissionState,
        coding_agent_permission: lr_config::PermissionState,
        coding_agent_type: Option<lr_config::CodingAgentType>,
        context_management_overrides: Option<lr_config::ContextManagementOverrides>,
        mcp_sampling_permission: lr_config::PermissionState,
        mcp_elicitation_permission: lr_config::PermissionState,
        request: JsonRpcRequest,
    ) -> AppResult<JsonRpcResponse> {
        let method = request.method.clone();
        let _request_id = request.id.clone();
        let is_broadcast = should_broadcast(&method);
        let session_key = session_id.unwrap_or(client_id);

        tracing::debug!(
            "Gateway handle_request: client_id={}, session_key={}, method={}, is_broadcast={}, servers={}",
            &client_id[..8.min(client_id.len())],
            &session_key[..8.min(session_key.len())],
            method,
            is_broadcast,
            allowed_servers.len()
        );

        // For initialize requests, always start with a fresh session.
        // This prevents lock contention with any previous task that may still
        // be running on the old session (e.g., from a replaced SSE connection).
        // The old task continues on its orphaned Arc but won't block new tasks.
        if method == "initialize" && self.sessions.remove(session_key).is_some() {
            tracing::info!(
                "Gateway: removed stale session for session_key={} before re-initialize",
                session_key
            );
        }

        // Get or create session
        let session: Arc<RwLock<GatewaySession>> = self
            .get_or_create_session(session_key, client_id, allowed_servers, roots)
            .await?;

        // Build a synthetic Client for virtual server state updates
        let synthetic_client = {
            let mut c = lr_config::Client::new_with_strategy(client_name.clone(), String::new());
            c.mcp_permissions = mcp_permissions.clone();
            c.skills_permissions = skills_permissions.clone();
            c.marketplace_permission = marketplace_permission.clone();
            c.coding_agent_permission = coding_agent_permission.clone();
            c.coding_agent_type = coding_agent_type;
            if let Some(ref overrides) = context_management_overrides {
                c.context_management_enabled = overrides.context_management_enabled;
                c.catalog_compression_enabled = overrides.catalog_compression_enabled;
            }
            c
        };

        // Update permissions and virtual server states on session
        {
            let mut session_write = session.write().await;
            session_write.mcp_permissions = mcp_permissions;
            session_write.skills_permissions = skills_permissions;
            session_write.client_name = client_name;
            session_write.mcp_sampling_permission = mcp_sampling_permission;
            session_write.mcp_elicitation_permission = mcp_elicitation_permission;

            // Invalidate tools cache if context management overrides changed
            if session_write.context_management_overrides != context_management_overrides {
                if session_write.context_management_overrides.is_some() {
                    tracing::info!(
                        "Context management overrides changed for session {}, invalidating tools cache",
                        session_key
                    );
                    session_write.invalidate_tools_cache();
                }
                session_write.context_management_overrides = context_management_overrides.clone();
            }

            // Update virtual server states
            for vs in self.virtual_servers.read().iter() {
                if let Some(state) = session_write.virtual_server_state.get_mut(vs.id()) {
                    vs.update_session_state(state.as_mut(), &synthetic_client);
                } else {
                    let state = vs.create_session_state(&synthetic_client);
                    session_write
                        .virtual_server_state
                        .insert(vs.id().to_string(), state);
                }
            }
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
                tracing::debug!(
                    "Gateway request completed: client={}, method={}, has_error={}",
                    &client_id[..8.min(client_id.len())],
                    method,
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
    ///
    /// `session_key` is used as the DashMap key (UUID for SSE, client_id for non-SSE).
    /// `client_id` is stored in the session for identification and permission matching.
    async fn get_or_create_session(
        &self,
        session_key: &str,
        client_id: &str,
        allowed_servers: Vec<String>,
        roots: Vec<crate::protocol::Root>,
    ) -> AppResult<Arc<RwLock<GatewaySession>>> {
        // Check if session exists
        if let Some(session) = self.sessions.get(session_key) {
            let session_read = session.read().await;

            // Check if expired
            if session_read.is_expired() {
                drop(session_read);
                drop(session);

                // Remove expired session
                self.sessions.remove(session_key);
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
                        session_key,
                    );
                    drop(session);
                    self.sessions.remove(session_key);
                    // Fall through to create new session below
                } else {
                    return Ok(session.clone());
                }
            }
        }

        // Create new session
        let ttl = Duration::from_secs(self.config.session_ttl_seconds);
        let session_data = GatewaySession::new(
            client_id.to_string(),
            allowed_servers.clone(),
            ttl,
            self.config.cache_ttl_seconds,
            roots,
        );

        let session = Arc::new(RwLock::new(session_data));

        // Register GLOBAL notification handlers for each server (if not already registered)
        self.register_notification_handlers(&allowed_servers).await;

        self.sessions
            .insert(session_key.to_string(), session.clone());

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
        session: Arc<RwLock<GatewaySession>>,
        request: JsonRpcRequest,
    ) -> AppResult<JsonRpcResponse> {
        // Check elicitation permission
        let elicitation_permission = session.read().await.mcp_elicitation_permission.clone();

        if matches!(elicitation_permission, lr_config::PermissionState::Off) {
            return Ok(JsonRpcResponse::error(
                request.id.unwrap_or(Value::Null),
                JsonRpcError::custom(
                    -32601,
                    "Elicitation is disabled for this client".to_string(),
                    None,
                ),
            ));
        }

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

        // Start all servers in parallel, collecting failures
        // We continue even if some servers fail to start
        let start_timeout = tokio::time::Duration::from_secs(15);
        let mut start_futures = Vec::new();

        for server_id in &allowed_servers {
            let server_name = self
                .server_manager
                .get_config(server_id)
                .map(|c| c.name.clone())
                .unwrap_or_else(|| server_id.clone());

            if self.server_manager.is_running(server_id) {
                tracing::debug!("Server '{}' already running, skipping start", server_name);
                continue;
            }
            let server_id = server_id.clone();
            let manager = self.server_manager.clone();
            start_futures.push(async move {
                tracing::info!("Starting MCP server '{}' for gateway init...", server_name);
                let start = std::time::Instant::now();
                let result =
                    tokio::time::timeout(start_timeout, manager.start_server(&server_id)).await;
                tracing::info!(
                    "Server '{}' start completed in {:?}: {}",
                    server_name,
                    start.elapsed(),
                    match &result {
                        Ok(Ok(())) => "success".to_string(),
                        Ok(Err(e)) => format!("error: {}", e),
                        Err(_) => "TIMEOUT".to_string(),
                    }
                );
                (server_id, result)
            });
        }

        // Already-running servers are immediately successful
        let mut started_servers: Vec<String> = allowed_servers
            .iter()
            .filter(|id| self.server_manager.is_running(id))
            .cloned()
            .collect();
        let mut start_failures: Vec<ServerFailure> = Vec::new();

        let results = futures::future::join_all(start_futures).await;
        for (server_id, result) in results {
            match result {
                Ok(Ok(())) => {
                    started_servers.push(server_id);
                }
                Ok(Err(e)) => {
                    start_failures.push(ServerFailure {
                        server_id,
                        error: e.to_string(),
                    });
                }
                Err(_) => {
                    start_failures.push(ServerFailure {
                        server_id,
                        error: format!(
                            "Server startup timed out after {}s",
                            start_timeout.as_secs()
                        ),
                    });
                }
            }
        }

        // If no servers could be started, proceed without MCP servers.
        // The gateway still serves other features (marketplace, coding agents, skills).
        if started_servers.is_empty() {
            // Only error if servers were attempted and ALL failed — not if there were none to start
            if !start_failures.is_empty() && allowed_servers.is_empty() {
                // Shouldn't happen, but guard against it
            }

            if !start_failures.is_empty() {
                tracing::warn!(
                    "All {} MCP servers failed to start, proceeding without MCP servers",
                    start_failures.len()
                );
            }

            let virtual_instructions = self.collect_virtual_instructions(&session).await;
            let unavailable = self.build_unavailable_server_infos(&start_failures);
            let instructions = build_gateway_instructions(&InstructionsContext {
                servers: Vec::new(),
                unavailable_servers: unavailable,
                context_management_enabled: false,
                catalog_compression: None,
                virtual_instructions,
                ..Default::default()
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
                    name: "LocalRouter MCP Gateway".to_string(),
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

        // Filter client capabilities based on permission settings before broadcasting
        let broadcast_request_msg = {
            let session_read = session.read().await;
            let sampling_perm = &session_read.mcp_sampling_permission;
            let elicitation_perm = &session_read.mcp_elicitation_permission;
            let mut modified_request = request.clone();

            if let Some(ref mut params) = modified_request.params {
                if let Some(caps) = params.get_mut("capabilities") {
                    // Remove sampling capability if permission is Off
                    if matches!(sampling_perm, lr_config::PermissionState::Off) {
                        if let Some(obj) = caps.as_object_mut() {
                            obj.remove("sampling");
                        }
                    } else if !caps
                        .as_object()
                        .map_or(false, |o| o.contains_key("sampling"))
                    {
                        // Ensure sampling capability exists if permission is Allow/Ask
                        if let Some(obj) = caps.as_object_mut() {
                            obj.insert("sampling".to_string(), serde_json::json!({}));
                        }
                    }

                    // Remove elicitation capability if permission is Off
                    if matches!(elicitation_perm, lr_config::PermissionState::Off) {
                        if let Some(obj) = caps.as_object_mut() {
                            obj.remove("elicitation");
                        }
                    } else if !caps
                        .as_object()
                        .map_or(false, |o| o.contains_key("elicitation"))
                    {
                        // Ensure elicitation capability exists if permission is Allow/Ask
                        if let Some(obj) = caps.as_object_mut() {
                            obj.insert("elicitation".to_string(), serde_json::json!({}));
                        }
                    }
                }
            }
            drop(session_read);
            modified_request
        };

        // Broadcast initialize to all successfully started servers
        let timeout = Duration::from_secs(self.config.server_timeout_seconds);
        let max_retries = self.config.max_retry_attempts;

        let results = broadcast_request(
            &servers_to_initialize,
            broadcast_request_msg,
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

        // If all servers failed, check if virtual servers can serve as fallback
        if init_results.is_empty() && !failures.is_empty() {
            let has_virtual = !self.virtual_servers.read().is_empty();

            if has_virtual {
                tracing::info!(
                    "All MCP servers failed to initialize, but virtual servers are configured — proceeding in fallback mode"
                );

                let virtual_instructions = self.collect_virtual_instructions(&session).await;
                let unavailable = self.build_unavailable_server_infos(&failures);
                let instructions = build_gateway_instructions(&InstructionsContext {
                    servers: Vec::new(),
                    unavailable_servers: unavailable,
                    context_management_enabled: false,
                    catalog_compression: None,
                    virtual_instructions,
                    ..Default::default()
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

        // Build gateway instructions based on the full context
        let (active_server_ids, client_id_for_log) = {
            let session_read = session.read().await;
            (
                session_read.allowed_servers.clone(),
                session_read.client_id.clone(),
            )
        };

        let virtual_instructions = self.collect_virtual_instructions(&session).await;

        // Check if context management is enabled for this session
        let (cm_enabled, cm_catalog_threshold, cm_search_tool_name) = {
            let session_read = session.read().await;
            if let Some(state) = session_read.virtual_server_state.get("_context_mode") {
                if let Some(cm_state) = state
                    .as_any()
                    .downcast_ref::<super::context_mode::ContextModeSessionState>()
                {
                    (
                        cm_state.enabled,
                        cm_state.catalog_threshold_bytes,
                        cm_state.search_tool_name.clone(),
                    )
                } else {
                    (false, 0, "IndexSearch".to_string())
                }
            } else {
                (false, 0, "IndexSearch".to_string())
            }
        };

        // Fetch the full catalogs for building instructions.
        let tools_catalog = self
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

        let resources_catalog = self
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

        let prompts_catalog = self
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

        let server_infos = self.build_server_instruction_infos(
            &init_results_for_instructions,
            &tools_catalog,
            &resources_catalog,
            &prompts_catalog,
        );
        let unavailable = self.build_unavailable_server_infos(&merged.failures);

        // Context management: index catalog into native FTS5 store synchronously
        // (before computing compression plan, which needs index results).
        // Also compute item definition sizes for the threshold calculation.
        let item_definition_sizes = if cm_enabled {
            compute_item_definition_sizes(&tools_catalog, &resources_catalog, &prompts_catalog)
        } else {
            std::collections::HashMap::new()
        };

        if cm_enabled {
            let session_bg = session.clone();
            let server_infos_bg = server_infos.clone();
            let tools_catalog_bg = tools_catalog.clone();
            let resources_catalog_bg = resources_catalog.clone();
            let prompts_catalog_bg = prompts_catalog.clone();
            let client_id_bg = client_id_for_log.clone();

            // Get ContentStore and gateway indexing permissions
            let (store, gateway_indexing) = {
                let session_read = session_bg.read().await;
                if let Some(state) = session_read.virtual_server_state.get("_context_mode") {
                    if let Some(cm) = state
                        .as_any()
                        .downcast_ref::<super::context_mode::ContextModeSessionState>()
                    {
                        (Some(cm.store.clone()), cm.gateway_indexing.clone())
                    } else {
                        (None, lr_config::GatewayIndexingPermissions::default())
                    }
                } else {
                    (None, lr_config::GatewayIndexingPermissions::default())
                }
            };

            if let Some(store) = store {
                // Index synchronously using spawn_blocking (ContentStore uses parking_lot::Mutex)
                let server_infos_idx = server_infos_bg.clone();
                let tools_idx = tools_catalog_bg.clone();
                let resources_idx = resources_catalog_bg.clone();
                let prompts_idx = prompts_catalog_bg.clone();
                let store_idx = store.clone();
                let client_id_idx = client_id_bg.clone();
                let gateway_perms = gateway_indexing.clone();

                let indexing_result = tokio::task::spawn_blocking(move || {
                    let mut indexed_count = 0u32;
                    let mut skipped_count = 0u32;

                    for info in &server_infos_idx {
                        let slug = super::types::slugify(&info.name);
                        if !gateway_perms.is_server_eligible(&slug) {
                            skipped_count += 1;
                            continue;
                        }
                        let content = build_full_server_content(info);
                        if let Err(e) = store_idx.index(&format!("mcp/{}", slug), &content) {
                            tracing::warn!("Failed to index catalog for server {}: {}", slug, e);
                        }
                        indexed_count += 1;
                    }

                    for tool in &tools_idx {
                        let (server_slug, original_name) = match tool.name.split_once("__") {
                            Some((s, t)) => (s, t),
                            None => (tool.name.as_str(), tool.name.as_str()),
                        };
                        if !gateway_perms.is_tool_eligible(server_slug, original_name) {
                            skipped_count += 1;
                            continue;
                        }
                        let content = format_tool_as_markdown(tool);
                        if let Err(e) = store_idx
                            .index(&format!("mcp/{}/tool/{}", server_slug, tool.name), &content)
                        {
                            tracing::warn!("Failed to index tool {}: {}", tool.name, e);
                        }
                        indexed_count += 1;
                    }

                    for resource in &resources_idx {
                        let (server_slug, _) = match resource.name.split_once("__") {
                            Some((s, t)) => (s, t),
                            None => (resource.name.as_str(), resource.name.as_str()),
                        };
                        let content = format_resource_as_markdown(resource);
                        if let Err(e) = store_idx.index(
                            &format!("mcp/{}/resource/{}", server_slug, resource.name),
                            &content,
                        ) {
                            tracing::warn!("Failed to index resource {}: {}", resource.name, e);
                        }
                        indexed_count += 1;
                    }

                    for prompt in &prompts_idx {
                        let (server_slug, _) = match prompt.name.split_once("__") {
                            Some((s, t)) => (s, t),
                            None => (prompt.name.as_str(), prompt.name.as_str()),
                        };
                        let content = format_prompt_as_markdown(prompt);
                        if let Err(e) = store_idx.index(
                            &format!("mcp/{}/prompt/{}", server_slug, prompt.name),
                            &content,
                        ) {
                            tracing::warn!("Failed to index prompt {}: {}", prompt.name, e);
                        }
                        indexed_count += 1;
                    }

                    tracing::info!(
                        "Context management catalog indexed for client {}: {} indexed, {} skipped",
                        &client_id_idx[..8.min(client_id_idx.len())],
                        indexed_count,
                        skipped_count,
                    );
                })
                .await;

                if let Err(e) = indexing_result {
                    tracing::warn!(
                        "Context management catalog indexing panicked for client {}: {}",
                        &client_id_bg[..8.min(client_id_bg.len())],
                        e,
                    );
                }

                // Store catalog state for activation tracking
                {
                    let mut session_write = session_bg.write().await;
                    if let Some(state) = session_write.virtual_server_state.get_mut("_context_mode")
                    {
                        if let Some(cm_state) = state
                            .as_any_mut()
                            .downcast_mut::<super::context_mode::ContextModeSessionState>(
                        ) {
                            for info in &server_infos_bg {
                                let slug = super::types::slugify(&info.name);
                                cm_state.catalog_sources.insert(
                                    format!("mcp/{}", slug),
                                    super::context_mode::CatalogItemType::ServerWelcome,
                                );
                                for name in &info.tool_names {
                                    let (s, _) = name.split_once("__").unwrap_or((name, name));
                                    cm_state.catalog_sources.insert(
                                        format!("mcp/{}/tool/{}", s, name),
                                        super::context_mode::CatalogItemType::Tool,
                                    );
                                }
                                for name in &info.resource_names {
                                    let (s, _) = name.split_once("__").unwrap_or((name, name));
                                    cm_state.catalog_sources.insert(
                                        format!("mcp/{}/resource/{}", s, name),
                                        super::context_mode::CatalogItemType::Resource,
                                    );
                                }
                                for name in &info.prompt_names {
                                    let (s, _) = name.split_once("__").unwrap_or((name, name));
                                    cm_state.catalog_sources.insert(
                                        format!("mcp/{}/prompt/{}", s, name),
                                        super::context_mode::CatalogItemType::Prompt,
                                    );
                                }
                            }

                            cm_state.full_tool_catalog = tools_catalog_bg;
                            cm_state.full_resource_catalog = resources_catalog_bg;
                            cm_state.full_prompt_catalog = prompts_catalog_bg;
                        }
                    }
                }
            }
        }

        // Build instructions context
        let mut instructions_ctx = InstructionsContext {
            servers: server_infos,
            unavailable_servers: unavailable,
            context_management_enabled: cm_enabled,
            catalog_compression: None,
            virtual_instructions,
            search_tool_name: cm_search_tool_name,
            item_definition_sizes,
        };

        // Context management: compute compression plan
        if cm_enabled {
            // Context-mode provides its own discovery via ctx_search, so always
            // enable deferral when it's active — don't require client listChanged support.
            let supports_tools_changed = true;
            let supports_resources_changed = true;
            let supports_prompts_changed = true;

            instructions_ctx.catalog_compression = Some(compute_catalog_compression_plan(
                &instructions_ctx,
                cm_catalog_threshold,
                supports_tools_changed,
                supports_resources_changed,
                supports_prompts_changed,
            ));
        }

        let instructions = build_gateway_instructions(&instructions_ctx);

        // Store instructions, compression plan, and context snapshot in session
        {
            let mut session_write = session.write().await;
            if instructions.is_some() {
                if let Some(ref mut mc) = session_write.merged_capabilities {
                    mc.instructions = instructions.clone();
                }
            }
            // Store compression plan so tools/resources/prompts list handlers can filter deferred items
            session_write.catalog_compression = instructions_ctx.catalog_compression.take();
            // Store context snapshot (without compression) for the compression preview UI
            let mut ctx_snapshot = instructions_ctx.clone();
            ctx_snapshot.catalog_compression = None;
            session_write.instructions_context = Some(ctx_snapshot);
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
            let session_key = entry.key().clone();
            let session = entry.value().clone();

            // Try to acquire read lock with timeout
            let is_expired = if let Ok(session_read) = session.try_read() {
                session_read.is_expired()
            } else {
                false
            };

            if is_expired {
                to_remove.push(session_key);
            }
        }

        for session_key in to_remove {
            self.sessions.remove(&session_key);
            tracing::info!("Removed expired gateway session: {}", session_key);
        }
    }

    /// Invalidate tools cache for all sessions belonging to a client
    pub fn invalidate_tools_cache(&self, client_id: &str) {
        for entry in self.sessions.iter() {
            if let Ok(mut session_write) = entry.value().try_write() {
                if session_write.client_id == client_id {
                    session_write.invalidate_tools_cache();
                }
            }
        }
    }

    /// Invalidate resources cache for all sessions belonging to a client
    pub fn invalidate_resources_cache(&self, client_id: &str) {
        for entry in self.sessions.iter() {
            if let Ok(mut session_write) = entry.value().try_write() {
                if session_write.client_id == client_id {
                    session_write.invalidate_resources_cache();
                }
            }
        }
    }

    /// Invalidate prompts cache for all sessions belonging to a client
    pub fn invalidate_prompts_cache(&self, client_id: &str) {
        for entry in self.sessions.iter() {
            if let Ok(mut session_write) = entry.value().try_write() {
                if session_write.client_id == client_id {
                    session_write.invalidate_prompts_cache();
                }
            }
        }
    }

    /// Invalidate all caches for all sessions belonging to a client
    pub fn invalidate_all_caches(&self, client_id: &str) {
        for entry in self.sessions.iter() {
            if let Ok(mut session_write) = entry.value().try_write() {
                if session_write.client_id == client_id {
                    session_write.invalidate_all_caches();
                }
            }
        }
    }

    /// Check all active sessions for permission changes and notify clients.
    ///
    /// Compares stored permission snapshots with current client config.
    /// For each session with changed permissions, invalidates relevant caches,
    /// updates the stored snapshot, and calls the `notify` callback.
    ///
    /// # Arguments
    /// * `clients` - Current client configs from the config manager
    /// * `all_enabled_server_ids` - All enabled MCP server IDs (for computing allowed servers)
    /// * `notify` - Callback called with (client_id, tools_changed, resources_changed, prompts_changed)
    pub fn check_and_notify_permission_changes(
        &self,
        clients: &[lr_config::Client],
        all_enabled_server_ids: &[String],
        notify: impl Fn(&str, bool, bool, bool),
    ) {
        for entry in self.sessions.iter() {
            let session = entry.value();

            // Try to acquire write lock (non-blocking to avoid deadlocks)
            let Ok(mut session_write) = session.try_write() else {
                tracing::debug!(
                    "Could not acquire session lock for permission check: {}",
                    entry.key()
                );
                continue;
            };

            let client_id = session_write.client_id.clone();

            // Find matching client in config
            let Some(client) = clients.iter().find(|c| c.id == client_id) else {
                continue; // Client may have been deleted
            };

            let old_mcp = &session_write.mcp_permissions;
            let old_skills = &session_write.skills_permissions;
            let new_mcp = &client.mcp_permissions;
            let new_skills = &client.skills_permissions;

            // Check if anything changed
            if old_mcp == new_mcp && old_skills == new_skills {
                continue;
            }

            tracing::info!(
                "Permission change detected for client {}, computing notifications",
                client_id
            );

            // Determine what changed
            let tools_changed = old_mcp.global != new_mcp.global
                || old_mcp.servers != new_mcp.servers
                || old_mcp.tools != new_mcp.tools
                || old_skills != new_skills;

            let resources_changed = old_mcp.global != new_mcp.global
                || old_mcp.servers != new_mcp.servers
                || old_mcp.resources != new_mcp.resources;

            let prompts_changed = old_mcp.global != new_mcp.global
                || old_mcp.servers != new_mcp.servers
                || old_mcp.prompts != new_mcp.prompts;

            // Invalidate relevant caches
            if tools_changed {
                session_write.invalidate_tools_cache();
            }
            if resources_changed {
                session_write.invalidate_resources_cache();
            }
            if prompts_changed {
                session_write.invalidate_prompts_cache();
            }

            // Update allowed_servers based on new permissions
            let new_allowed: Vec<String> = if new_mcp.global.is_enabled() {
                all_enabled_server_ids.to_vec()
            } else {
                all_enabled_server_ids
                    .iter()
                    .filter(|sid| new_mcp.has_any_enabled_for_server(sid))
                    .cloned()
                    .collect()
            };
            session_write.allowed_servers = new_allowed;

            // Update stored snapshots
            session_write.mcp_permissions = new_mcp.clone();
            session_write.skills_permissions = new_skills.clone();
            session_write.mcp_sampling_permission = client.mcp_sampling_permission.clone();
            session_write.mcp_elicitation_permission = client.mcp_elicitation_permission.clone();

            // Call notify callback
            notify(
                &client_id,
                tools_changed,
                resources_changed,
                prompts_changed,
            );
        }
    }

    /// Get the elicitation manager (for submitting responses from external clients)
    pub fn get_elicitation_manager(&self) -> Arc<ElicitationManager> {
        self.elicitation_manager.clone()
    }

    /// Get a session by session key (direct DashMap lookup)
    pub fn get_session(&self, session_key: &str) -> Option<Arc<RwLock<GatewaySession>>> {
        self.sessions.get(session_key).map(|s| s.clone())
    }

    /// Get all sessions for a client ID (iterates all sessions)
    pub fn get_sessions_for_client(
        &self,
        client_id: &str,
    ) -> Vec<(String, Arc<RwLock<GatewaySession>>)> {
        self.sessions
            .iter()
            .filter_map(|entry| {
                if let Ok(session_read) = entry.value().try_read() {
                    if session_read.client_id == client_id {
                        return Some((entry.key().clone(), entry.value().clone()));
                    }
                }
                None
            })
            .collect()
    }

    /// List all active sessions with stats for the UI.
    pub async fn list_active_sessions(&self) -> Vec<ActiveSessionInfo> {
        let mut sessions = Vec::new();
        for entry in self.sessions.iter() {
            let session_key = entry.key().clone();
            let session = entry.value().read().await;
            if session.is_expired() {
                continue;
            }

            let duration_secs = session.created_at.elapsed().as_secs();

            // Extract context management stats
            let (
                cm_enabled,
                cm_indexed_sources,
                cm_activated_tools,
                cm_total_tools,
                cm_catalog_threshold_bytes,
            ) = {
                if let Some(state) = session.virtual_server_state.get("_context_mode") {
                    if let Some(cm) = state
                        .as_any()
                        .downcast_ref::<super::context_mode::ContextModeSessionState>()
                    {
                        (
                            cm.enabled,
                            cm.catalog_sources.len(),
                            cm.activated_tools.len(),
                            cm.full_tool_catalog.len(),
                            cm.catalog_threshold_bytes,
                        )
                    } else {
                        (false, 0, 0, 0, 0)
                    }
                } else {
                    (false, 0, 0, 0, 0)
                }
            };

            let initialized_servers = session.get_initialized_servers().len();
            let failed_servers = session.get_failed_servers().len();
            let total_tools = session.tool_mapping.len();

            sessions.push(ActiveSessionInfo {
                session_id: session_key,
                client_id: session.client_id.clone(),
                client_name: session.client_name.clone(),
                duration_secs,
                initialized_servers,
                failed_servers,
                total_tools,
                context_management_enabled: cm_enabled,
                cm_indexed_sources,
                cm_activated_tools,
                cm_total_tools,
                cm_catalog_threshold_bytes,
            });
        }
        sessions
    }

    /// Terminate a session by session key (direct lookup).
    pub async fn terminate_session(&self, session_key: &str) -> Result<(), String> {
        if self.sessions.remove(session_key).is_some() {
            tracing::info!(
                "Gateway: terminated session for session_key={}",
                session_key
            );
            Ok(())
        } else {
            Err(format!("Session not found: {session_key}"))
        }
    }

    /// Terminate all sessions for a given client ID.
    pub async fn terminate_sessions_for_client(&self, client_id: &str) -> usize {
        let keys_to_remove: Vec<String> = self
            .sessions
            .iter()
            .filter_map(|entry| {
                if let Ok(session_read) = entry.value().try_read() {
                    if session_read.client_id == client_id {
                        return Some(entry.key().clone());
                    }
                }
                None
            })
            .collect();

        let count = keys_to_remove.len();
        for key in keys_to_remove {
            self.sessions.remove(&key);
        }
        if count > 0 {
            tracing::info!(
                "Gateway: terminated {} session(s) for client_id={}",
                count,
                client_id
            );
        }
        count
    }

    /// Get catalog sources for a specific session by session key.
    pub async fn get_session_context_sources(
        &self,
        session_key: &str,
    ) -> Result<Vec<CatalogSourceEntry>, String> {
        let session_arc = self
            .sessions
            .get(session_key)
            .ok_or_else(|| format!("Session not found: {session_key}"))?
            .clone();
        let session = session_arc.read().await;

        let state = session
            .virtual_server_state
            .get("_context_mode")
            .ok_or("Context management not available for this session")?;
        let cm = state
            .as_any()
            .downcast_ref::<super::context_mode::ContextModeSessionState>()
            .ok_or("Invalid context mode state")?;

        if !cm.enabled {
            return Err("Context management is not enabled for this session".to_string());
        }

        let mut entries: Vec<CatalogSourceEntry> = cm
            .catalog_sources
            .iter()
            .map(|(label, item_type)| {
                let name = label.strip_prefix("catalog:").unwrap_or(label);
                let activated = match item_type {
                    super::context_mode::CatalogItemType::Tool => cm.activated_tools.contains(name),
                    super::context_mode::CatalogItemType::Resource => {
                        cm.activated_resources.contains(name)
                    }
                    super::context_mode::CatalogItemType::Prompt => {
                        cm.activated_prompts.contains(name)
                    }
                    super::context_mode::CatalogItemType::ServerWelcome => true,
                };
                CatalogSourceEntry {
                    source_label: label.clone(),
                    item_type: format!("{:?}", item_type),
                    activated,
                }
            })
            .collect();
        entries.sort_by(|a, b| a.source_label.cmp(&b.source_label));
        Ok(entries)
    }

    /// Get context stats for a specific session from the native ContentStore.
    pub async fn get_session_context_stats(
        &self,
        session_key: &str,
    ) -> Result<serde_json::Value, String> {
        let session_arc = self
            .sessions
            .get(session_key)
            .ok_or_else(|| format!("Session not found: {session_key}"))?
            .clone();
        let session = session_arc.read().await;

        let state = session
            .virtual_server_state
            .get("_context_mode")
            .ok_or("Context management not available for this session")?;
        let cm = state
            .as_any()
            .downcast_ref::<super::context_mode::ContextModeSessionState>()
            .ok_or("Invalid context mode state")?;

        if !cm.enabled {
            return Err("Context management is not enabled for this session".to_string());
        }

        // Return basic stats from the native ContentStore
        Ok(serde_json::json!({
            "content": [{
                "type": "text",
                "text": format!(
                    "Context store active. Indexed sources: {}, Activated tools: {}",
                    cm.catalog_sources.len(),
                    cm.activated_tools.len(),
                ),
            }]
        }))
    }

    /// Query the context index for a specific session using native search.
    pub async fn query_session_context_index(
        &self,
        session_key: &str,
        query: &str,
        source: Option<&str>,
    ) -> Result<serde_json::Value, String> {
        let session_arc = self
            .sessions
            .get(session_key)
            .ok_or_else(|| format!("Session not found: {session_key}"))?
            .clone();

        // Extract the Arc<ContentStore> under a brief async lock, then release it
        // before performing the blocking search.
        let store = {
            let session = session_arc.read().await;
            let state = session
                .virtual_server_state
                .get("_context_mode")
                .ok_or("Context management not available for this session")?;
            let cm = state
                .as_any()
                .downcast_ref::<super::context_mode::ContextModeSessionState>()
                .ok_or("Invalid context mode state")?;
            if !cm.enabled {
                return Err("Context management is not enabled for this session".to_string());
            }
            cm.store.clone()
        };

        // Run the blocking FTS5 search off the async executor
        let queries = vec![query.to_string()];
        let source_owned = source.map(|s| s.to_string());
        let result =
            tokio::task::spawn_blocking(move || store.search(&queries, 5, source_owned.as_deref()))
                .await
                .map_err(|e| format!("Search task panicked: {}", e))?;

        match result {
            Ok(results) => {
                let formatted =
                    lr_context::format_search_results(&results, lr_context::SEARCH_OUTPUT_CAP);
                Ok(serde_json::json!({
                    "content": [{
                        "type": "text",
                        "text": formatted,
                    }]
                }))
            }
            Err(e) => Err(format!("Search failed: {}", e)),
        }
    }

    /// Get the instructions context snapshot for a session (for compression preview UI).
    pub async fn get_session_instructions_context(
        &self,
        session_key: &str,
    ) -> Result<super::merger::InstructionsContext, String> {
        let session_arc = self
            .sessions
            .get(session_key)
            .ok_or_else(|| format!("Session not found: {session_key}"))?
            .clone();
        let session = session_arc.read().await;
        session
            .instructions_context
            .clone()
            .ok_or_else(|| "Session has no instructions context (not yet initialized)".to_string())
    }

    /// Get the instructions context for a client by finding their active session.
    pub async fn get_client_instructions_context(
        &self,
        client_id: &str,
    ) -> Result<super::merger::InstructionsContext, String> {
        for entry in self.sessions.iter() {
            let session = entry.value().read().await;
            if session.client_id == client_id && !session.is_expired() {
                if let Some(ctx) = &session.instructions_context {
                    return Ok(ctx.clone());
                }
            }
        }
        Err(format!(
            "No active session for client {client_id}. Connect the client first to preview its compression."
        ))
    }

    /// Build a preview InstructionsContext for a client without requiring an active session.
    ///
    /// Tries in order:
    /// 1. The client's own active session
    /// 2. Any active session (servers serve the same tools regardless of client)
    /// 3. Build from running servers by querying their tool catalogs directly
    pub async fn get_or_build_preview_context(
        &self,
        client_id: &str,
        allowed_server_ids: Vec<String>,
    ) -> Result<InstructionsContext, String> {
        // 1. Try this client's active session
        for entry in self.sessions.iter() {
            let session = entry.value().read().await;
            if session.client_id == client_id && !session.is_expired() {
                if let Some(ctx) = &session.instructions_context {
                    return Ok(ctx.clone());
                }
            }
        }

        // 2. Try any active session with context data
        for entry in self.sessions.iter() {
            let session = entry.value().read().await;
            if !session.is_expired() {
                if let Some(ctx) = &session.instructions_context {
                    return Ok(ctx.clone());
                }
            }
        }

        if allowed_server_ids.is_empty() {
            return Err(
                "Client has no MCP servers configured. Add servers in the client's MCP permissions."
                    .to_string(),
            );
        }

        // 3. Start servers on demand if not already running
        let start_timeout = tokio::time::Duration::from_secs(15);
        let mut start_futures = Vec::new();

        for server_id in &allowed_server_ids {
            if self.server_manager.is_running(server_id) {
                continue;
            }
            let sid = server_id.clone();
            let manager = self.server_manager.clone();
            start_futures.push(async move {
                let result = tokio::time::timeout(start_timeout, manager.start_server(&sid)).await;
                (sid, result)
            });
        }

        let mut running_ids: Vec<String> = allowed_server_ids
            .iter()
            .filter(|id| self.server_manager.is_running(id))
            .cloned()
            .collect();
        let mut unavailable = Vec::new();

        if !start_futures.is_empty() {
            let results = futures::future::join_all(start_futures).await;
            for (server_id, result) in results {
                match result {
                    Ok(Ok(())) => {
                        running_ids.push(server_id);
                    }
                    Ok(Err(e)) => {
                        let name = self
                            .server_manager
                            .get_config(&server_id)
                            .map(|c| c.name.clone())
                            .unwrap_or_else(|| server_id.clone());
                        unavailable.push(UnavailableServerInfo {
                            name,
                            error: e.to_string(),
                        });
                    }
                    Err(_) => {
                        let name = self
                            .server_manager
                            .get_config(&server_id)
                            .map(|c| c.name.clone())
                            .unwrap_or_else(|| server_id.clone());
                        unavailable.push(UnavailableServerInfo {
                            name,
                            error: format!(
                                "Server startup timed out after {}s",
                                start_timeout.as_secs()
                            ),
                        });
                    }
                }
            }
        }

        // Fetch tools, resources, prompts from running servers
        let tools = self
            .fetch_and_merge_tools(
                &running_ids,
                JsonRpcRequest::new(
                    Some(json!("_preview_tools")),
                    "tools/list".to_string(),
                    None,
                ),
            )
            .await
            .map(|(t, _)| t)
            .unwrap_or_default();

        let resources = self
            .fetch_and_merge_resources(
                &running_ids,
                JsonRpcRequest::new(
                    Some(json!("_preview_resources")),
                    "resources/list".to_string(),
                    None,
                ),
            )
            .await
            .map(|(r, _)| r)
            .unwrap_or_default();

        let prompts = self
            .fetch_and_merge_prompts(
                &running_ids,
                JsonRpcRequest::new(
                    Some(json!("_preview_prompts")),
                    "prompts/list".to_string(),
                    None,
                ),
            )
            .await
            .map(|(p, _)| p)
            .unwrap_or_default();

        // Build server infos from the fetched data (no init results, so no description/instructions)
        let server_infos: Vec<McpServerInstructionInfo> = running_ids
            .iter()
            .map(|id| {
                let name = self
                    .server_manager
                    .get_config(id)
                    .map(|c| c.name.clone())
                    .unwrap_or_else(|| id.clone());
                McpServerInstructionInfo {
                    name,
                    instructions: None,
                    description: None,
                    tool_names: tools
                        .iter()
                        .filter(|t| t.server_id == *id)
                        .map(|t| t.name.clone())
                        .collect(),
                    resource_names: resources
                        .iter()
                        .filter(|r| r.server_id == *id)
                        .map(|r| r.name.clone())
                        .collect(),
                    prompt_names: prompts
                        .iter()
                        .filter(|p| p.server_id == *id)
                        .map(|p| p.name.clone())
                        .collect(),
                }
            })
            .collect();

        Ok(InstructionsContext {
            servers: server_infos,
            unavailable_servers: unavailable,
            context_management_enabled: false,
            catalog_compression: None,
            virtual_instructions: Vec::new(),
            ..Default::default()
        })
    }

    /// Fetch tool/resource/prompt catalogs from all running servers for preview detail display.
    pub async fn fetch_preview_catalogs(
        &self,
    ) -> (
        Vec<NamespacedTool>,
        Vec<NamespacedResource>,
        Vec<NamespacedPrompt>,
    ) {
        let running_ids: Vec<String> = self
            .server_manager
            .list_configs()
            .iter()
            .filter(|c| c.enabled && self.server_manager.is_running(&c.id))
            .map(|c| c.id.clone())
            .collect();

        if running_ids.is_empty() {
            return (Vec::new(), Vec::new(), Vec::new());
        }

        let tools = self
            .fetch_and_merge_tools(
                &running_ids,
                JsonRpcRequest::new(
                    Some(json!("_preview_catalog_tools")),
                    "tools/list".to_string(),
                    None,
                ),
            )
            .await
            .map(|(t, _)| t)
            .unwrap_or_default();

        let resources = self
            .fetch_and_merge_resources(
                &running_ids,
                JsonRpcRequest::new(
                    Some(json!("_preview_catalog_resources")),
                    "resources/list".to_string(),
                    None,
                ),
            )
            .await
            .map(|(r, _)| r)
            .unwrap_or_default();

        let prompts = self
            .fetch_and_merge_prompts(
                &running_ids,
                JsonRpcRequest::new(
                    Some(json!("_preview_catalog_prompts")),
                    "prompts/list".to_string(),
                    None,
                ),
            )
            .await
            .map(|(p, _)| p)
            .unwrap_or_default();

        (tools, resources, prompts)
    }
}

/// Info about an active gateway session (for UI display).
#[derive(serde::Serialize, Clone)]
pub struct ActiveSessionInfo {
    pub session_id: String,
    pub client_id: String,
    pub client_name: String,
    pub duration_secs: u64,
    pub initialized_servers: usize,
    pub failed_servers: usize,
    pub total_tools: usize,
    pub context_management_enabled: bool,
    pub cm_indexed_sources: usize,
    pub cm_activated_tools: usize,
    pub cm_total_tools: usize,
    pub cm_catalog_threshold_bytes: usize,
}

/// A single catalog source entry (for UI display).
#[derive(serde::Serialize, Clone)]
pub struct CatalogSourceEntry {
    pub source_label: String,
    pub item_type: String,
    pub activated: bool,
}
