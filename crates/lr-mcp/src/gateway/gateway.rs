use dashmap::DashMap;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

use crate::manager::McpServerManager;
use crate::protocol::{JsonRpcError, JsonRpcNotification, JsonRpcRequest, JsonRpcResponse};
use crate::transport::SessionTransportSet;
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
use super::push_relay::{
    PushMethod, PushRelayService, PushRelaySessionContext, RememberedPushSource, SnapshotRecorder,
    UpstreamPushNotification,
};
use super::router::{broadcast_request, separate_results, should_broadcast};
use super::session::GatewaySession;
use super::types::*;
use super::virtual_server::VirtualMcpServer;

/// Freshness hint (SEP-2549 `ttlMs`) attached to list/read results for
/// 2026-07-28 peers. Invalidation is still signaled via listChanged
/// notifications; this only bounds client-side polling.
const LIST_RESULT_TTL_MS: u64 = 60_000;

fn session_initialization_results(
    state: &GatewaySession,
) -> (Vec<(String, InitializeResult)>, Vec<ServerFailure>) {
    let mut successes = Vec::new();
    let mut failures = Vec::new();
    for (id, status) in &state.server_init_status {
        match status {
            InitStatus::Completed(result) => successes.push((id.clone(), result.clone())),
            InitStatus::Failed { error, .. } => failures.push(ServerFailure {
                server_id: id.clone(),
                error: error.clone(),
            }),
            _ => {}
        }
    }
    (successes, failures)
}

/// MCP Gateway - Unified endpoint for multiple MCP servers
#[allow(clippy::type_complexity)]
#[derive(Clone)]
pub struct McpGateway {
    /// Active sessions (session_key -> session)
    /// For SSE connections, session_key is a per-connection UUID.
    /// For non-SSE connections, session_key defaults to client_id.
    pub(crate) sessions: Arc<DashMap<String, Arc<RwLock<GatewaySession>>>>,

    /// MCP server manager
    pub(crate) server_manager: Arc<McpServerManager>,

    /// Cached protocol revision per backend server id, learned from the
    /// `server/discover` probe. Known-legacy backends skip re-probing on
    /// every new session; entries live for the gateway's lifetime.
    pub(crate) backend_revision_cache: Arc<DashMap<String, crate::protocol::ProtocolRevision>>,

    /// Gateway configuration
    pub(crate) config: GatewayConfig,

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
    pub coding_agent_approval_manager:
        Arc<super::coding_agent_approval::CodingAgentApprovalManager>,

    /// Sampling approval manager for user approval flow (permission=Ask).
    /// Wrapped in RwLock so it can be replaced with a shared instance from AppState.
    pub(crate) sampling_approval_manager:
        Arc<parking_lot::RwLock<Arc<super::sampling_approval::SamplingApprovalManager>>>,

    /// Sampling passthrough manager for forwarding to external clients.
    /// Wrapped in RwLock so it can be replaced with a shared instance from AppState.
    pub(crate) sampling_passthrough_manager:
        Arc<parking_lot::RwLock<Arc<super::sampling_approval::SamplingPassthroughManager>>>,

    /// Callback to send a server-initiated request to a connected external client.
    /// Returns a oneshot receiver for the response. Used for passthrough mode.
    #[allow(clippy::type_complexity)]
    pub(crate) client_request_sender: Arc<
        parking_lot::RwLock<
            Option<
                Arc<
                    dyn Fn(
                            &str,           // client_id
                            JsonRpcRequest, // request to forward
                        )
                            -> Option<tokio::sync::oneshot::Receiver<JsonRpcResponse>>
                        + Send
                        + Sync,
                >,
            >,
        >,
    >,

    /// Live-updateable sampling behavior (reads config changes without restart)
    pub(crate) live_sampling: Arc<parking_lot::RwLock<lr_config::SamplingBehavior>>,

    /// Live-updateable elicitation mode
    pub(crate) live_elicitation_mode: Arc<parking_lot::RwLock<lr_config::ElicitationMode>>,

    /// Virtual MCP servers (skills, marketplace, coding agents)
    pub(crate) virtual_servers: Arc<parking_lot::RwLock<Vec<Arc<dyn VirtualMcpServer>>>>,

    /// Callback to record context management bytes saved (wired to metrics DB)
    pub(crate) on_context_saved: Arc<parking_lot::RwLock<Option<Arc<dyn Fn(u64) + Send + Sync>>>>,

    /// Callback to emit monitor events (wired to MonitorEventStore). Returns event ID.
    #[allow(clippy::type_complexity)]
    pub(crate) on_monitor_event: Arc<
        parking_lot::RwLock<
            Option<
                Arc<
                    dyn Fn(
                            lr_monitor::MonitorEventType,
                            Option<String>, // client_id
                            Option<String>, // client_name
                            Option<String>, // session_id
                            lr_monitor::MonitorEventData,
                            lr_monitor::EventStatus,
                            Option<u64>, // duration_ms
                        ) -> String
                        + Send
                        + Sync,
                >,
            >,
        >,
    >,

    /// Callback to update an existing monitor event (wired to MonitorEventStore)
    pub(crate) on_monitor_update:
        Arc<parking_lot::RwLock<Option<super::super::manager::MonitorUpdateFn>>>,

    /// Push Relay, when registered (see [`Self::set_push_relay`]).
    pub(crate) push_relay: Arc<parking_lot::RwLock<Option<Arc<dyn PushRelayService>>>>,

    /// Live Push Relay settings (the master switch).
    pub(crate) push_relay_config: Arc<parking_lot::RwLock<lr_config::PushRelayConfig>>,

    /// How long discovery waits for a server with no current capability
    /// snapshot ([`super::push_relay::limits::FIRST_DISCOVERY_WAIT`]).
    pub(crate) first_discovery_wait: Arc<parking_lot::RwLock<Duration>>,

    /// Session keys whose requests come through the STDIO bridge, recorded by
    /// the HTTP route before each request (see [`Self::set_stdio_bridge_session`]).
    pub(crate) stdio_bridge_sessions: Arc<dashmap::DashSet<String>>,

    /// Session keys that opted into the Push Relay test source, recorded by
    /// the HTTP route before each request (see [`Self::set_push_test_session`]).
    pub(crate) push_test_sessions: Arc<dashmap::DashSet<String>>,

    /// Remembers upstream capabilities in the server configs.
    pub(crate) snapshot_recorder: Arc<SnapshotRecorder>,

    /// Response streams of downstream `events/stream` requests.
    pub(crate) push_stream_sinks: Arc<super::push_relay::PushStreamSinks>,
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
            Some(broadcast) => Arc::new(
                super::coding_agent_approval::CodingAgentApprovalManager::new_with_broadcast(
                    120,
                    broadcast.clone(),
                ),
            ),
            None => Arc::new(super::coding_agent_approval::CodingAgentApprovalManager::new(120)),
        };

        // Create sampling approval manager with broadcast support if available
        let sampling_approval_manager = match &notification_broadcast {
            Some(broadcast) => Arc::new(
                super::sampling_approval::SamplingApprovalManager::new_with_broadcast(
                    120,
                    broadcast.clone(),
                ),
            ),
            None => Arc::new(super::sampling_approval::SamplingApprovalManager::new(120)),
        };

        // Create sampling passthrough manager
        let sampling_passthrough_manager = Arc::new(
            super::sampling_approval::SamplingPassthroughManager::new(120),
        );

        Self {
            sessions: Arc::new(DashMap::new()),
            server_manager,
            backend_revision_cache: Arc::new(DashMap::new()),
            client_request_sender: Arc::new(parking_lot::RwLock::new(None)),
            live_sampling: Arc::new(parking_lot::RwLock::new(config.sampling.clone())),
            live_elicitation_mode: Arc::new(parking_lot::RwLock::new(
                config.elicitation_mode.clone(),
            )),
            config,
            notification_broadcast,
            router,
            elicitation_manager,
            firewall_manager,
            coding_agent_approval_manager,
            sampling_approval_manager: Arc::new(parking_lot::RwLock::new(
                sampling_approval_manager,
            )),
            sampling_passthrough_manager: Arc::new(parking_lot::RwLock::new(
                sampling_passthrough_manager,
            )),
            virtual_servers: Arc::new(parking_lot::RwLock::new(Vec::new())),
            on_context_saved: Arc::new(parking_lot::RwLock::new(None)),
            on_monitor_event: Arc::new(parking_lot::RwLock::new(None)),
            on_monitor_update: Arc::new(parking_lot::RwLock::new(None)),
            push_relay: Arc::new(parking_lot::RwLock::new(None)),
            push_relay_config: Arc::new(parking_lot::RwLock::new(
                lr_config::PushRelayConfig::default(),
            )),
            first_discovery_wait: Arc::new(parking_lot::RwLock::new(
                super::push_relay::limits::FIRST_DISCOVERY_WAIT,
            )),
            stdio_bridge_sessions: Arc::new(dashmap::DashSet::new()),
            push_test_sessions: Arc::new(dashmap::DashSet::new()),
            snapshot_recorder: Arc::new(SnapshotRecorder::new()),
            push_stream_sinks: Arc::new(super::push_relay::PushStreamSinks::default()),
        }
    }

    /// Set callback to record context management bytes saved to the metrics database
    pub fn set_on_context_saved<F: Fn(u64) + Send + Sync + 'static>(&self, callback: F) {
        *self.on_context_saved.write() = Some(Arc::new(callback));
    }

    /// Set callback to emit monitor events to the MonitorEventStore. Returns event ID.
    #[allow(clippy::type_complexity)]
    pub fn set_on_monitor_event<
        F: Fn(
                lr_monitor::MonitorEventType,
                Option<String>,
                Option<String>,
                Option<String>,
                lr_monitor::MonitorEventData,
                lr_monitor::EventStatus,
                Option<u64>,
            ) -> String
            + Send
            + Sync
            + 'static,
    >(
        &self,
        callback: F,
    ) {
        *self.on_monitor_event.write() = Some(Arc::new(callback));
    }

    /// Set callback to update existing monitor events.
    pub fn set_on_monitor_update(&self, callback: super::super::manager::MonitorUpdateFn) {
        *self.on_monitor_update.write() = Some(callback);
    }

    /// Emit a monitor event if the callback is set. Returns the event ID.
    pub(crate) fn emit_monitor_event(
        &self,
        event_type: lr_monitor::MonitorEventType,
        client_id: Option<String>,
        client_name: Option<String>,
        session_id: Option<String>,
        data: lr_monitor::MonitorEventData,
        status: lr_monitor::EventStatus,
        duration_ms: Option<u64>,
    ) -> String {
        if let Some(cb) = self.on_monitor_event.read().as_ref() {
            tracing::debug!(
                "Gateway emitting monitor event: {:?} for client {:?}",
                event_type,
                client_id
            );
            cb(
                event_type,
                client_id,
                client_name,
                session_id,
                data,
                status,
                duration_ms,
            )
        } else {
            tracing::warn!(
                "Gateway monitor callback not set, dropping event: {:?}",
                event_type
            );
            String::new()
        }
    }

    /// Update an existing monitor event by ID.
    pub(crate) fn update_monitor_event(
        &self,
        event_id: &str,
        updater: Box<dyn FnOnce(&mut lr_monitor::MonitorEvent) + Send>,
    ) {
        if let Some(cb) = self.on_monitor_update.read().as_ref() {
            cb(event_id, updater);
        }
    }

    /// Set the callback for sending server-initiated requests to connected external clients.
    /// Used for passthrough sampling/elicitation.
    #[allow(clippy::type_complexity)]
    pub fn set_client_request_sender<
        F: Fn(&str, JsonRpcRequest) -> Option<tokio::sync::oneshot::Receiver<JsonRpcResponse>>
            + Send
            + Sync
            + 'static,
    >(
        &self,
        sender: F,
    ) {
        *self.client_request_sender.write() = Some(Arc::new(sender));
    }

    /// Replace the sampling approval manager with a shared instance.
    /// Call this to share the same manager between gateway and server response endpoints.
    pub fn set_sampling_approval_manager(
        &self,
        manager: Arc<super::sampling_approval::SamplingApprovalManager>,
    ) {
        *self.sampling_approval_manager.write() = manager;
    }

    /// Update the live gateway settings (sampling/elicitation modes and permissions).
    /// Called when the user changes global MCP gateway settings in the UI.
    pub fn update_gateway_settings(&self, settings: &lr_config::McpGatewaySettings) {
        *self.live_sampling.write() = settings.sampling.clone();
        *self.live_elicitation_mode.write() = settings.elicitation_mode.clone();
    }

    /// Replace the sampling passthrough manager with a shared instance.
    /// Call this to share the same manager between gateway and server response endpoints.
    pub fn set_sampling_passthrough_manager(
        &self,
        manager: Arc<super::sampling_approval::SamplingPassthroughManager>,
    ) {
        *self.sampling_passthrough_manager.write() = manager;
    }

    /// Register a virtual MCP server (skills, marketplace, coding agents, etc.)
    /// Register the Push Relay. From then on downstream `events/*` requests,
    /// push capabilities and instructions, and upstream push notifications
    /// go through it.
    pub fn set_push_relay(&self, service: Arc<dyn PushRelayService>) {
        *self.push_relay.write() = Some(service);
    }

    /// The registered Push Relay, if any.
    pub fn push_relay(&self) -> Option<Arc<dyn PushRelayService>> {
        self.push_relay.read().clone()
    }

    /// Per-request response streams for downstream `events/stream`.
    pub fn push_stream_sinks(&self) -> Arc<super::push_relay::PushStreamSinks> {
        self.push_stream_sinks.clone()
    }

    /// Apply changed Push Relay settings.
    pub fn update_push_relay_config(&self, config: &lr_config::PushRelayConfig) {
        *self.push_relay_config.write() = config.clone();
    }

    /// Current Push Relay settings.
    pub fn push_relay_config(&self) -> lr_config::PushRelayConfig {
        self.push_relay_config.read().clone()
    }

    /// Install the callback that writes remembered capabilities to the config
    /// file (debounced batches of changed snapshots only).
    pub fn set_capability_snapshot_persister(
        &self,
        persister: super::push_relay::CapabilitySnapshotPersister,
    ) {
        self.snapshot_recorder.set_persister(persister);
    }

    /// Override how long changed snapshots are batched before persisting.
    pub fn set_capability_snapshot_debounce(&self, debounce: Duration) {
        self.snapshot_recorder.set_debounce(debounce);
    }

    /// Override the first-discovery wait (tests reach its bound with it).
    #[doc(hidden)]
    pub fn set_first_discovery_wait(&self, wait: Duration) {
        *self.first_discovery_wait.write() = wait;
    }

    /// Record whether requests for a session arrive through the STDIO bridge.
    /// The HTTP route calls this before every request, from the
    /// `X-LocalRouter-Bridge` header.
    pub fn set_stdio_bridge_session(&self, session_key: &str, is_stdio_bridge: bool) {
        if is_stdio_bridge {
            self.stdio_bridge_sessions.insert(session_key.to_string());
        } else {
            self.stdio_bridge_sessions.remove(session_key);
        }
    }

    /// Record whether a session opted into the Push Relay test source. The
    /// HTTP route calls this before every request, from the
    /// `X-LocalRouter-Push-Test` header.
    pub fn set_push_test_session(&self, session_key: &str, opted_in: bool) {
        if opted_in {
            self.push_test_sessions.insert(session_key.to_string());
        } else {
            self.push_test_sessions.remove(session_key);
        }
    }

    /// The Push Relay context of a live session.
    pub async fn push_session_context(&self, session_key: &str) -> Option<PushRelaySessionContext> {
        let session = self.get_session(session_key)?;
        let state = session.read().await;
        Some(push_context(&state))
    }

    /// Each upstream's declared capabilities for a live session (which
    /// servers have events, channels, ...). `None` until it initialised.
    pub async fn session_capability_providers(
        &self,
        session_key: &str,
    ) -> Option<CapabilityProviders> {
        let session = self.get_session(session_key)?;
        let state = session.read().await;
        state
            .merged_capabilities
            .as_ref()
            .map(|merged| merged.providers.clone())
    }

    /// Servers among `server_ids` that are enabled and whose remembered
    /// capabilities (still matching their configuration) include push.
    /// Resource updates count only when `include_resource_updates` is set.
    pub fn remembered_push_sources(
        &self,
        server_ids: &[String],
        include_resource_updates: bool,
    ) -> Vec<RememberedPushSource> {
        server_ids
            .iter()
            .filter_map(|id| {
                let config = self.server_manager.get_config(id)?;
                if !config.enabled {
                    return None;
                }
                let snapshot = config.current_capability_snapshot()?.clone();
                snapshot
                    .push
                    .has_consumable_push(include_resource_updates)
                    .then(|| RememberedPushSource {
                        server_id: id.clone(),
                        server_name: config.name.clone(),
                        snapshot,
                    })
            })
            .collect()
    }

    /// [`Self::remembered_push_sources`] for the servers a session's client
    /// may see.
    pub async fn remembered_push_sources_for_session(
        &self,
        session_key: &str,
        include_resource_updates: bool,
    ) -> Vec<RememberedPushSource> {
        let Some(session) = self.get_session(session_key) else {
            return Vec::new();
        };
        let visible = session.read().await.requested_servers.clone();
        self.remembered_push_sources(&visible, include_resource_updates)
    }

    /// Send an arbitrary request (e.g. `events/list`, `events/poll`) to one
    /// upstream over a session's own connection, under the normal request
    /// timeout.
    pub async fn upstream_request(
        &self,
        session_key: &str,
        server_id: &str,
        method: &str,
        params: Option<Value>,
    ) -> AppResult<JsonRpcResponse> {
        let transports = self.session_transports_for(session_key, server_id).await?;
        let request = JsonRpcRequest::new(
            Some(json!(format!("_lr_push_{}", uuid::Uuid::new_v4()))),
            method.to_string(),
            params,
        );
        transports.send_request(server_id, request).await
    }

    /// Open a long-lived request (e.g. `events/stream`) on one upstream over
    /// a session's own connection. See [`crate::transport::RpcStream`].
    pub async fn open_upstream_stream(
        &self,
        session_key: &str,
        server_id: &str,
        method: &str,
        params: Option<Value>,
        buffer: usize,
    ) -> AppResult<crate::transport::RpcStream> {
        let transports = self.session_transports_for(session_key, server_id).await?;
        let request = JsonRpcRequest::new(
            Some(json!(format!("_lr_push_{}", uuid::Uuid::new_v4()))),
            method.to_string(),
            params,
        );
        transports.open_rpc_stream(server_id, request, buffer).await
    }

    async fn session_transports_for(
        &self,
        session_key: &str,
        server_id: &str,
    ) -> AppResult<Arc<SessionTransportSet>> {
        let session = self
            .get_session(session_key)
            .ok_or_else(|| AppError::Mcp(format!("Session not found: {session_key}")))?;
        let state = session.read().await;
        if !state.requested_servers.iter().any(|id| id == server_id) {
            return Err(AppError::Mcp(format!(
                "Server {server_id} is not visible to this session"
            )));
        }
        state
            .transports
            .clone()
            .filter(|transports| transports.is_running(server_id))
            .ok_or_else(|| AppError::Mcp(format!("Server not running: {server_id}")))
    }

    /// Deliver a notification to one session's client (its SSE / WebSocket
    /// / `subscriptions/listen` connection). Returns whether anyone listened.
    pub fn send_session_notification(
        &self,
        session_key: &str,
        notification: JsonRpcNotification,
    ) -> bool {
        self.notification_broadcast
            .as_ref()
            .is_some_and(|broadcast| {
                broadcast
                    .send((session_notification_key(session_key), notification))
                    .is_ok()
            })
    }

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

        if let Some(service) = self.push_relay() {
            if let Some(content) = service.instructions(&push_context(&session_read)) {
                result.push(super::virtual_server::VirtualInstructions {
                    section_title: "Events".to_string(),
                    content,
                    tool_names: Vec::new(),
                    priority: 25,
                });
            }
        }

        result.sort_by_key(|v| v.priority);
        result
    }

    /// Add what the Push Relay advertises for this session to serialised
    /// capabilities. No-op without a registered relay.
    fn apply_push_advertisement(&self, state: &GatewaySession, capabilities: &mut Value) {
        if let Some(service) = self.push_relay() {
            service
                .advertisement(&push_context(state))
                .apply_to(capabilities);
        }
    }

    /// Route a downstream `events/*` request to the Push Relay, or answer
    /// method-not-found when none is registered. Never forwarded upstream.
    async fn handle_push_request(
        &self,
        session: Arc<RwLock<GatewaySession>>,
        method: PushMethod,
        request: JsonRpcRequest,
    ) -> AppResult<JsonRpcResponse> {
        let Some(service) = self.push_relay() else {
            return Ok(JsonRpcResponse::error(
                request.id.unwrap_or(Value::Null),
                JsonRpcError::method_not_found(&request.method),
            ));
        };
        let ctx = push_context(&*session.read().await);
        match method {
            PushMethod::List => service.events_list(&ctx, request).await,
            PushMethod::Poll => service.events_poll(&ctx, request).await,
            PushMethod::Stream => service.events_stream(&ctx, request).await,
            PushMethod::Subscribe => service.events_subscribe(&ctx, request).await,
            PushMethod::Unsubscribe => service.events_unsubscribe(&ctx, request).await,
        }
    }

    /// Visible, enabled servers with no remembered capabilities (or ones that
    /// no longer match their configuration). Empty unless a Push Relay is
    /// registered and enabled: only it reads the snapshots.
    fn servers_awaiting_first_discovery(&self, servers: &[String]) -> Vec<String> {
        if self.push_relay().is_none() || !self.push_relay_config.read().enabled {
            return Vec::new();
        }
        servers
            .iter()
            .filter(|id| {
                self.server_manager.get_config(id).is_some_and(|config| {
                    config.enabled && config.current_capability_snapshot().is_none()
                })
            })
            .cloned()
            .collect()
    }

    /// Wait until every server in `servers` has remembered capabilities or
    /// has failed to start, or until `deadline`.
    async fn wait_for_first_discovery(
        &self,
        session: &Arc<RwLock<GatewaySession>>,
        servers: &[String],
        deadline: tokio::time::Instant,
    ) {
        let mut ticker = tokio::time::interval(Duration::from_millis(50));
        loop {
            let settled = {
                let state = session.read().await;
                let backends_done = !state.initialization_pending && state.transports.is_some();
                servers.iter().all(|id| {
                    let remembered = self
                        .server_manager
                        .get_config(id)
                        .is_some_and(|c| c.current_capability_snapshot().is_some());
                    remembered
                        || matches!(
                            state.server_init_status.get(id),
                            Some(InitStatus::Failed { .. })
                        )
                        || (backends_done && !state.allowed_servers.contains(id))
                })
            };
            if settled || tokio::time::Instant::now() >= deadline {
                return;
            }
            tokio::select! {
                _ = ticker.tick() => {}
                _ = tokio::time::sleep_until(deadline) => return,
            }
        }
    }

    /// Remember what each freshly initialised upstream offers, in the
    /// background so the handshake never waits on it.
    fn observe_upstream_capabilities(
        &self,
        transports: &Arc<SessionTransportSet>,
        results: &[(String, InitializeResult)],
    ) {
        for (server_id, init) in results {
            spawn_capability_observation(
                self.server_manager.clone(),
                self.snapshot_recorder.clone(),
                transports.clone(),
                server_id.clone(),
                init.clone(),
            );
        }
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
    pub(crate) fn build_unavailable_server_infos(
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
            None,                            // memory_enabled
            None,                            // memory_folder
            lr_config::ClientMode::default(), // client_mode
            lr_config::ClientPushSettings::default(),
            request,
            None, // monitor_session_id
        )
        .await
    }

    /// Handle an MCP gateway request with skill access
    ///
    /// `session_id` is an optional per-connection identifier (UUID). When provided,
    /// it is used as the session key, allowing multiple simultaneous connections
    /// from the same client. When `None`, `client_id` is used as the session key.
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
        memory_enabled: Option<bool>,
        memory_folder: Option<String>,
        client_mode: lr_config::ClientMode,
        push_settings: lr_config::ClientPushSettings,
        request: JsonRpcRequest,
        monitor_session_id: Option<String>,
    ) -> AppResult<JsonRpcResponse> {
        let method = request.method.clone();
        let _request_id = request.id.clone();
        let is_broadcast = should_broadcast(&method);
        let session_key = session_id.unwrap_or(client_id);

        // Stateless (2026-07-28) support: clients declare protocol version,
        // identity and capabilities in `_meta` on every request. A declared
        // version we don't support is rejected up front; requests without the
        // declaration are legacy (2025-11-25 and earlier) and flow unchanged.
        let client_meta = crate::protocol::RequestClientMeta::from_params(request.params.as_ref());
        if let Some(version) = client_meta.protocol_version.as_deref() {
            if !crate::protocol::SUPPORTED_PROTOCOL_VERSIONS.contains(&version) {
                return Ok(JsonRpcResponse::error(
                    request.id.clone().unwrap_or(Value::Null),
                    JsonRpcError::unsupported_protocol_version(version),
                ));
            }
        }
        let declared_revision = client_meta.revision();

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
        if method == "initialize" {
            if let Some((_, old_session)) = self.sessions.remove(session_key) {
                // Close the old session's transports in the background: a
                // task still running on it may hold its lock for a while.
                tokio::spawn(async move { close_session_transports(&old_session).await });
                tracing::info!(
                    "Gateway: removed stale session for session_key={} before re-initialize",
                    session_key
                );
            }
        }

        // Get or create session
        let session: Arc<RwLock<GatewaySession>> = self
            .get_or_create_session(session_key, client_id, allowed_servers, roots)
            .await?;

        // Build a synthetic Client for virtual server state updates
        let synthetic_client = {
            let mut c = lr_config::Client::new_with_strategy(client_name.clone(), String::new());
            // Virtual servers key per-client state (e.g. the memory folder) on
            // the real client identity
            c.id = client_id.to_string();
            c.memory_folder = memory_folder;
            // MCP-via-LLM sessions have no shell of their own; virtual
            // servers adapt (e.g. skills show virtual instead of disk paths)
            if client_mode == lr_config::ClientMode::McpViaLlm {
                c.mcp_mode = lr_config::McpMode::ViaLlm;
            }
            c.mcp_permissions = mcp_permissions.clone();
            c.skills_permissions = skills_permissions.clone();
            c.marketplace_permission = marketplace_permission.clone();
            c.coding_agent_permission = coding_agent_permission.clone();
            c.coding_agent_type = coding_agent_type;
            if let Some(ref overrides) = context_management_overrides {
                c.context_management_enabled = overrides.context_management_enabled;
                c.catalog_compression_enabled = overrides.catalog_compression_enabled;
            }
            c.memory_enabled = memory_enabled;
            push_settings.apply_to(&mut c);
            c
        };

        // Update permissions and virtual server states on session
        let list_changes = {
            let mut session_write = session.write().await;
            let list_changes = session_write.apply_listing_snapshot(
                super::session::ClientListingSnapshot::from_client(&synthetic_client),
            );
            session_write.client_name = client_name;
            session_write.is_stdio_bridge = self.stdio_bridge_sessions.contains(session_key);
            session_write.push_test_source = self.push_test_sessions.contains(session_key);
            session_write.mcp_sampling_permission = mcp_sampling_permission;
            session_write.mcp_elicitation_permission = mcp_elicitation_permission;
            session_write.client_mode = client_mode;
            session_write.context_management_overrides = context_management_overrides.clone();

            // Update virtual server states
            let session_context = push_context(&session_write);
            for vs in self.virtual_servers.read().iter() {
                if let Some(state) = session_write.virtual_server_state.get_mut(vs.id()) {
                    vs.update_session_state(state.as_mut(), &synthetic_client);
                    vs.update_session_context(state.as_mut(), &session_context);
                } else {
                    let mut state = vs.create_session_state(&synthetic_client);
                    vs.update_session_context(state.as_mut(), &session_context);
                    session_write
                        .virtual_server_state
                        .insert(vs.id().to_string(), state);
                }
            }
            list_changes
        };

        // Settings changed since this session's previous request (and the
        // config-change hook didn't get to it first): tell the client to
        // re-list. Session-scoped, so only this connection receives it.
        if list_changes.any() {
            tracing::info!(
                "Client settings changed for session {}: {:?}",
                session_key,
                list_changes
            );
            self.send_list_changed(&session_notification_key(session_key), list_changes);
        }

        // Update last activity and set monitor session_id
        {
            let mut session_write = session.write().await;
            session_write.touch();
            if monitor_session_id.is_some() {
                session_write.monitor_session_id = monitor_session_id;
            }
            if let Some(revision) = declared_revision {
                session_write.protocol_revision = revision;
            }
            if client_meta.log_level.is_some() {
                session_write.log_level = client_meta.log_level.clone();
            }
        }

        // Handle client notifications (fire-and-forget, no meaningful response)
        if method.starts_with("notifications/") {
            tracing::debug!("Gateway received client notification: {}", method);

            // Forward notifications/cancelled to the correct upstream server
            if method == "notifications/cancelled" {
                if let Some(params) = &request.params {
                    if let (Some(request_id), Some(service)) =
                        (params.get("requestId"), self.push_relay())
                    {
                        service.on_client_cancelled(session_key, request_id);
                    }
                    if let Some(request_id) = params.get("requestId") {
                        let request_id_str = request_id
                            .as_str()
                            .map(|s| s.to_string())
                            .unwrap_or_else(|| request_id.to_string());

                        let server_id = {
                            let session_read = session.read().await;
                            session_read.pending_requests.get(&request_id_str).cloned()
                        };

                        if let Some(server_id) = server_id {
                            let notification = JsonRpcRequest::new(
                                None,
                                "notifications/cancelled".to_string(),
                                Some(params.clone()),
                            );
                            // Use session transports to forward cancellation
                            let transports = session.read().await.transports.clone();
                            let send_result = if let Some(transports) = transports {
                                transports.send_request(&server_id, notification).await
                            } else {
                                Err(AppError::Mcp(
                                    "Session not initialized, cannot forward cancellation".into(),
                                ))
                            };
                            if let Err(e) = send_result {
                                tracing::warn!(
                                    "Failed to forward cancellation to server {}: {}",
                                    server_id,
                                    e
                                );
                            } else {
                                tracing::info!(
                                    "Forwarded cancellation for request {} to server {}",
                                    request_id_str,
                                    server_id
                                );
                            }
                        }
                    }
                }
            }

            // notifications/initialized is already sent to servers during handle_initialize.
            // Other notifications are silently consumed.
            return Ok(JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                id: Value::Null,
                result: Some(Value::Null),
                error: None,
            });
        }

        // Stateless (2026-07-28) clients never send `initialize`; materialize
        // backend transports lazily on the first request that needs them.
        // `server/discover` is exempt so a capability probe stays cheap.
        // Local skill reads must not depend on remote services initializing.
        // Permissions and session state are still checked by virtual dispatch.
        let local_skill_call = method == "tools/call"
            && request
                .params
                .as_ref()
                .and_then(|p| p.get("name"))
                .and_then(Value::as_str)
                .is_some_and(|name| {
                    self.virtual_servers
                        .read()
                        .iter()
                        .any(|vs| vs.id() == "_skills" && vs.owns_tool(name))
                });
        if method != "initialize"
            && method != "server/discover"
            && method != "ping"
            && !local_skill_call
        {
            let needs_backends = {
                let state = session.read().await;
                state.transports.is_none() && !state.initialization_pending
            };
            if needs_backends {
                self.initialize_session_backends(&session, &client_meta)
                    .await?;
            }
        }

        self.recover_backends_for_request(&session, &request).await;

        // Keep pending discovery on the normal response path so stateless
        // clients receive result and private-cache metadata as usual.
        let mut result = if let Some(response) = self
            .pending_initialization_response(&session, &request)
            .await
        {
            Ok(response)
        } else if is_broadcast {
            self.handle_broadcast_request(session.clone(), request)
                .await
        } else {
            self.handle_direct_request(session.clone(), request).await
        };

        // Tag responses for stateless peers: every result carries
        // `resultType`, and list/read results carry cache metadata.
        // Per-client filtering means responses are never shared-cacheable.
        if session.read().await.protocol_revision.is_stateless() {
            if let Ok(response) = &mut result {
                if let Some(result_value) = &mut response.result {
                    crate::protocol::tag_result_complete(result_value);
                    if let Some(obj) = result_value.as_object_mut() {
                        let meta = obj.entry("_meta").or_insert_with(|| json!({}));
                        if let Some(meta) = meta.as_object_mut() {
                            meta.insert(crate::protocol::meta_keys::SERVER_INFO.to_string(), json!({
                                "name": "LocalRouter MCP Gateway", "version": env!("CARGO_PKG_VERSION"),
                            }));
                        }
                    }
                    if matches!(
                        method.as_str(),
                        "tools/list"
                            | "prompts/list"
                            | "resources/list"
                            | "resources/templates/list"
                            | "resources/read"
                    ) {
                        crate::protocol::tag_cacheable(
                            result_value,
                            LIST_RESULT_TTL_MS,
                            crate::protocol::CACHE_SCOPE_PRIVATE,
                        );
                    }
                }
            }
        }

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

    /// Materialize backend transports for a session that has none, by
    /// running the initialize flow with a synthetic request.
    ///
    /// Stateless (2026-07-28) clients removed the `initialize` handshake, so
    /// the first real request triggers this. The synthetic initialize speaks
    /// the legacy revision to backends (upstream stateless support is
    /// negotiated separately per backend).
    async fn initialize_session_backends(
        &self,
        session: &Arc<RwLock<GatewaySession>>,
        client_meta: &crate::protocol::RequestClientMeta,
    ) -> AppResult<()> {
        // Serialize concurrent lazy initializations: without this, two
        // simultaneous first requests would both create backend transports
        // and the losing set's stdio processes would leak.
        let init_lock = session.read().await.lazy_init_lock.clone();
        let _guard = init_lock.lock().await;
        {
            let state = session.read().await;
            if state.transports.is_some() || state.initialization_pending {
                // Another request started initialization while we waited.
                return Ok(());
            }
        }

        let params = json!({
            "protocolVersion": crate::protocol::MCP_PROTOCOL_VERSION,
            "capabilities": client_meta
                .client_capabilities
                .clone()
                .unwrap_or_else(|| json!({})),
            "clientInfo": client_meta.client_info.clone().unwrap_or_else(|| json!({
                "name": "LocalRouter MCP Gateway",
                "version": env!("CARGO_PKG_VERSION"),
            })),
        });
        let synthetic = JsonRpcRequest::new(
            Some(json!("_lazy_init")),
            "initialize".to_string(),
            Some(params),
        );

        tracing::info!(
            "Lazily initializing backend transports for session {} (stateless client)",
            session.read().await.session_key
        );

        let response = self.handle_initialize(session.clone(), synthetic).await?;
        if let Some(error) = response.error {
            return Err(AppError::Mcp(format!(
                "Lazy backend initialization failed: {} (code {})",
                error.message, error.code
            )));
        }
        Ok(())
    }

    /// Probe a backend with `server/discover` (2026-07-28).
    ///
    /// Returns a synthesized `InitializeResult` when the backend supports the
    /// stateless revision, so it can be merged alongside legacy handshake
    /// results. Any error, timeout, or missing 2026-07-28 support means the
    /// backend is treated as legacy.
    async fn probe_server_discover(
        &self,
        transport_set: &Arc<SessionTransportSet>,
        server_id: &str,
        timeout: Duration,
    ) -> Option<InitializeResult> {
        let discover = JsonRpcRequest::with_id(
            0,
            "server/discover".to_string(),
            Some(json!({
                "_meta": {
                    crate::protocol::meta_keys::PROTOCOL_VERSION:
                        crate::protocol::MCP_PROTOCOL_VERSION_STATELESS,
                    crate::protocol::meta_keys::CLIENT_CAPABILITIES: {},
                    crate::protocol::meta_keys::CLIENT_INFO: {
                        "name": "LocalRouter MCP Gateway",
                        "version": env!("CARGO_PKG_VERSION"),
                    },
                }
            })),
        );

        let response =
            tokio::time::timeout(timeout, transport_set.send_request(server_id, discover))
                .await
                .ok()?
                .ok()?;
        if response.error.is_some() {
            return None;
        }
        let value = response.result?;

        // Some SDKs (rmcp) name the list `supportedVersions`
        let supports_stateless = value
            .get("protocolVersions")
            .or_else(|| value.get("supportedVersions"))
            .and_then(|v| v.as_array())
            .is_some_and(|arr| {
                arr.iter()
                    .any(|v| v.as_str() == Some(crate::protocol::MCP_PROTOCOL_VERSION_STATELESS))
            });
        if !supports_stateless {
            return None;
        }

        let capabilities = value
            .get("capabilities")
            .cloned()
            .and_then(|c| serde_json::from_value(c).ok())
            .unwrap_or_default();
        let server_info = value
            .get("_meta")
            .and_then(|meta| meta.get(crate::protocol::meta_keys::SERVER_INFO))
            .or_else(|| value.get("serverInfo"))
            .cloned()
            .and_then(|s| serde_json::from_value(s).ok())
            .unwrap_or(ServerInfo {
                name: server_id.to_string(),
                version: String::new(),
                description: None,
            });

        Some(InitializeResult {
            // Reported downstream via the min() of backend versions: keep the
            // legacy string so legacy clients never see a revision they can't
            // speak. The backend's stateless revision is tracked separately
            // on the transport set.
            protocol_version: crate::protocol::MCP_PROTOCOL_VERSION.to_string(),
            capabilities,
            server_info,
            instructions: value
                .get("instructions")
                .and_then(|i| i.as_str())
                .map(str::to_string),
        })
    }

    /// Handle `server/discover` (2026-07-28): advertise supported protocol
    /// versions, server identity, and capabilities without requiring any
    /// backend to be started.
    async fn handle_server_discover(
        &self,
        session: Arc<RwLock<GatewaySession>>,
        request: JsonRpcRequest,
    ) -> AppResult<JsonRpcResponse> {
        // A server never seen before (or reconfigured) is started and waited
        // for, bounded, so its capabilities can be remembered before this
        // client is told what it may use.
        let awaiting_discovery = {
            let requested = session.read().await.requested_servers.clone();
            self.servers_awaiting_first_discovery(&requested)
        };
        if !awaiting_discovery.is_empty() {
            let deadline = tokio::time::Instant::now() + *self.first_discovery_wait.read();
            let needs_backends = {
                let state = session.read().await;
                state.transports.is_none() && !state.initialization_pending
            };
            if needs_backends {
                let gateway = self.clone();
                let background = session.clone();
                let client_meta =
                    crate::protocol::RequestClientMeta::from_params(request.params.as_ref());
                tokio::spawn(async move {
                    if let Err(e) = gateway
                        .initialize_session_backends(&background, &client_meta)
                        .await
                    {
                        tracing::debug!("Backend start for first discovery failed: {}", e);
                    }
                });
            }
            self.wait_for_first_discovery(&session, &awaiting_discovery, deadline)
                .await;
        }

        // Use merged capabilities when backends are already running;
        // otherwise advertise the gateway's full capability surface (a
        // superset — list endpoints simply return fewer items when a
        // capability has no providing backend).
        let merged = session.read().await.merged_capabilities.clone();
        let mut capabilities = match merged {
            Some(m) => serde_json::to_value(&m.capabilities)
                .unwrap_or_else(|_| json!({ "tools": { "listChanged": true } })),
            None => json!({
                "tools": { "listChanged": true },
                "resources": { "listChanged": true, "subscribe": true },
                "prompts": { "listChanged": true },
                "completions": {}
            }),
        };
        self.apply_push_advertisement(&*session.read().await, &mut capabilities);

        let result = crate::protocol::ServerDiscoverResult {
            protocol_versions: crate::protocol::SUPPORTED_PROTOCOL_VERSIONS
                .iter()
                .map(|s| s.to_string())
                .collect(),
            server_info: json!({
                "name": "LocalRouter MCP Gateway",
                "version": env!("CARGO_PKG_VERSION"),
            }),
            capabilities,
        };

        Ok(JsonRpcResponse::success(
            request.id.unwrap_or(Value::Null),
            serde_json::to_value(&result)?,
        ))
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
        // Check if session exists (clone the Arc out: holding the DashMap
        // entry across the awaits below would block the shard)
        let existing = self.sessions.get(session_key).map(|s| s.value().clone());
        if let Some(session) = existing {
            let session_read = session.read().await;

            // Check if expired
            if session_read.is_expired() {
                // Close per-session transports before removing
                let transports = session_read.transports.clone();
                if let Some(task) = &session_read.initialization_abort {
                    task.abort();
                }
                drop(session_read);
                drop(session);
                if let Some(transports) = transports {
                    transports.close_all().await;
                }

                // Remove expired session
                self.sessions.remove(session_key);
            } else {
                // Check if allowed servers changed (e.g. switching between direct/all modes)
                // If so, drop the stale session and create a fresh one since cached state
                // (tool mappings, init statuses, etc.) is tied to the server list
                // Compare against the requested set, not `allowed_servers`:
                // the latter drops servers that failed to start, and comparing
                // it would rebuild the session (wiping tool mappings) on every
                // request while any server is down.
                let mut servers_sorted = allowed_servers.clone();
                servers_sorted.sort();
                let servers_changed = servers_sorted != session_read.requested_servers;
                // Clone transports before dropping the read lock
                let transports = if servers_changed {
                    if let Some(task) = &session_read.initialization_abort {
                        task.abort();
                    }
                    session_read.transports.clone()
                } else {
                    None
                };
                drop(session_read);

                if servers_changed {
                    tracing::info!(
                        "Allowed servers changed for session {} - recreating session",
                        session_key,
                    );
                    // Close per-session transports before removing
                    if let Some(transports) = transports {
                        transports.close_all().await;
                    }
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
        let mut session_data = GatewaySession::new(
            client_id.to_string(),
            allowed_servers.clone(),
            ttl,
            self.config.cache_ttl_seconds,
            roots,
        );
        session_data.session_key = session_key.to_string();

        let session = Arc::new(RwLock::new(session_data));

        // Note: notification and request handlers are now registered per-session in
        // handle_initialize, after session transports are created. The old global
        // register_notification_handlers method is retained but no longer called here.

        self.sessions
            .insert(session_key.to_string(), session.clone());

        Ok(session)
    }

    /// Register request callbacks on per-session transports.
    ///
    /// Same logic as `register_request_handlers` but targets the session's
    /// `SessionTransportSet` instead of the global server_manager transports.
    fn register_request_handlers_on_transports(
        &self,
        server_ids: &[String],
        client_id: &str,
        session_key: &str,
        client_mode: lr_config::ClientMode,
        transport_set: &Arc<SessionTransportSet>,
    ) {
        for server_id in server_ids {
            let transport = match transport_set.get(server_id) {
                Some(t) => t,
                None => continue,
            };

            let server_id_clone = server_id.clone();
            let elicitation_manager = self.elicitation_manager.clone();
            let sampling_approval_manager = self.sampling_approval_manager.read().clone();
            let client_request_sender = self.client_request_sender.read().clone();
            let router = self.router.clone();
            let client_id = client_id.to_string();
            let session_key = session_key.to_string();
            let live_sampling = self.live_sampling.clone();
            let live_elicitation_mode = self.live_elicitation_mode.clone();
            let firewall_manager = self.firewall_manager.clone();

            transport.set_request_callback(std::sync::Arc::new(move |request| {
                let server_id = server_id_clone.clone();
                let elicitation_mgr = elicitation_manager.clone();
                let sampling_approval_mgr = sampling_approval_manager.clone();
                let client_request_sender = client_request_sender.clone();
                let router = router.clone();
                let client_id = client_id.clone();
                let session_key = session_key.clone();
                let client_mode = client_mode;
                let firewall_mgr = firewall_manager.clone();
                let sampling_behavior = live_sampling.read().clone();
                let elicitation_mode = live_elicitation_mode.read().clone();

                Box::pin(async move {
                    tracing::info!(
                        "Server {} sent request: method={}, id={:?}",
                        server_id,
                        request.method,
                        request.id,
                    );

                    let request_id = request.id.unwrap_or(Value::Null);
                    match request.method.as_str() {
                        "roots/list" => {
                            JsonRpcResponse::success(
                                request_id,
                                json!({ "roots": [] }),
                            )
                        }

                        "elicitation/create" | "elicitation/requestInput" => {
                            if matches!(elicitation_mode, lr_config::ElicitationMode::Off) {
                                return JsonRpcResponse::success(
                                    request_id,
                                    json!({ "action": "decline" }),
                                );
                            }

                            if matches!(client_mode, lr_config::ClientMode::LlmOnly) {
                                return JsonRpcResponse::success(
                                    request_id,
                                    json!({ "action": "decline" }),
                                );
                            }

                            let effective_mode = match (&elicitation_mode, &client_mode) {
                                (lr_config::ElicitationMode::Passthrough, lr_config::ClientMode::McpViaLlm) => {
                                    lr_config::ElicitationMode::Direct
                                }
                                (mode, _) => mode.clone(),
                            };

                            let effective_mode = if matches!(effective_mode, lr_config::ElicitationMode::Off)
                                && firewall_mgr.should_intercept(
                                    &client_id,
                                    super::firewall::InterceptCategory::Elicitation,
                                )
                            {
                                tracing::info!(
                                    "Monitor intercept: overriding Off → Direct for elicitation (client={})",
                                    client_id
                                );
                                lr_config::ElicitationMode::Direct
                            } else {
                                effective_mode
                            };

                            tracing::info!(
                                "Elicitation handler: mode={:?}, effective={:?}, client_mode={:?}",
                                elicitation_mode, effective_mode, client_mode
                            );

                            match effective_mode {
                                lr_config::ElicitationMode::Passthrough => {
                                    if let Some(sender) = &client_request_sender {
                                        let forward_req = crate::protocol::JsonRpcRequest {
                                            jsonrpc: "2.0".to_string(),
                                            id: Some(json!(uuid::Uuid::new_v4().to_string())),
                                            method: "elicitation/create".to_string(),
                                            params: request.params.clone(),
                                        };

                                        tracing::info!(
                                            "Passthrough: forwarding elicitation request to external client for server {}",
                                            server_id
                                        );

                                        if let Some(response_rx) = sender(&session_key, forward_req) {
                                            match tokio::time::timeout(
                                                std::time::Duration::from_secs(120),
                                                response_rx,
                                            ).await {
                                                Ok(Ok(response)) => {
                                                    JsonRpcResponse::success(
                                                        request_id,
                                                        response.result.unwrap_or(json!({ "action": "decline" })),
                                                    )
                                                }
                                                Ok(Err(_)) | Err(_) => {
                                                    tracing::warn!("Elicitation passthrough timed out");
                                                    JsonRpcResponse::success(
                                                        request_id,
                                                        json!({ "action": "decline" }),
                                                    )
                                                }
                                            }
                                        } else {
                                            tracing::warn!("No SSE connection for client - falling back to Direct elicitation");
                                            Self::handle_elicitation_direct(
                                                &elicitation_mgr, server_id, Some(session_key.clone()), request.params, request_id
                                            ).await
                                        }
                                    } else {
                                        Self::handle_elicitation_direct(
                                            &elicitation_mgr, server_id, Some(session_key.clone()), request.params, request_id
                                        ).await
                                    }
                                }
                                lr_config::ElicitationMode::Direct => {
                                    Self::handle_elicitation_direct(
                                        &elicitation_mgr, server_id, Some(session_key.clone()), request.params, request_id
                                    ).await
                                }
                                lr_config::ElicitationMode::Off => {
                                    JsonRpcResponse::success(
                                        request_id,
                                        json!({ "action": "decline" }),
                                    )
                                }
                            }
                        }

                        "sampling/createMessage" => {
                            if matches!(sampling_behavior, lr_config::SamplingBehavior::Off) {
                                return JsonRpcResponse::error(
                                    request_id,
                                    JsonRpcError::custom(
                                        -32601,
                                        "Sampling is disabled".to_string(),
                                        None,
                                    ),
                                );
                            }

                            if matches!(client_mode, lr_config::ClientMode::LlmOnly) {
                                return JsonRpcResponse::error(
                                    request_id,
                                    JsonRpcError::custom(
                                        -32601,
                                        "Sampling is unavailable for LLM-only clients".to_string(),
                                        None,
                                    ),
                                );
                            }

                            let effective = match (&sampling_behavior, &client_mode) {
                                (lr_config::SamplingBehavior::Passthrough, lr_config::ClientMode::McpViaLlm) => {
                                    lr_config::SamplingBehavior::DirectAllow
                                }
                                (lr_config::SamplingBehavior::DirectAllow | lr_config::SamplingBehavior::DirectAsk, lr_config::ClientMode::McpOnly) => {
                                    lr_config::SamplingBehavior::Passthrough
                                }
                                (b, _) => b.clone(),
                            };

                            let effective = if matches!(effective, lr_config::SamplingBehavior::DirectAllow)
                                && firewall_mgr.should_intercept(
                                    &client_id,
                                    super::firewall::InterceptCategory::Sampling,
                                )
                            {
                                tracing::info!(
                                    "Monitor intercept: overriding DirectAllow → DirectAsk for sampling (client={})",
                                    client_id
                                );
                                lr_config::SamplingBehavior::DirectAsk
                            } else {
                                effective
                            };

                            if matches!(effective, lr_config::SamplingBehavior::DirectAsk) {
                                let sampling_req_for_approval = request.params.as_ref()
                                    .and_then(|p| serde_json::from_value::<crate::protocol::SamplingRequest>(p.clone()).ok());

                                let approval_req = sampling_req_for_approval.unwrap_or_else(|| {
                                    crate::protocol::SamplingRequest {
                                        messages: Vec::new(),
                                        system_prompt: None,
                                        temperature: None,
                                        max_tokens: None,
                                        stop_sequences: None,
                                        metadata: None,
                                        model_preferences: None,
                                        tools: None,
                                        tool_choice: None,
                                    }
                                });

                                let approval_id = uuid::Uuid::new_v4().to_string();
                                match sampling_approval_mgr.request_approval(
                                    approval_id,
                                    server_id.clone(),
                                    approval_req,
                                    None,
                                ).await {
                                    Ok(super::sampling_approval::SamplingApprovalAction::Allow) => {
                                        // User approved, continue to direct
                                    }
                                    Ok(super::sampling_approval::SamplingApprovalAction::Deny) | Err(_) => {
                                        return JsonRpcResponse::error(
                                            request_id,
                                            JsonRpcError::custom(
                                                -32601,
                                                "Sampling request was denied by user".to_string(),
                                                None,
                                            ),
                                        );
                                    }
                                }
                            }

                            match effective {
                                lr_config::SamplingBehavior::Passthrough => {
                                    if let Some(sender) = &client_request_sender {
                                        let forward_req = crate::protocol::JsonRpcRequest {
                                            jsonrpc: "2.0".to_string(),
                                            id: Some(json!(uuid::Uuid::new_v4().to_string())),
                                            method: "sampling/createMessage".to_string(),
                                            params: request.params.clone(),
                                        };

                                        tracing::info!(
                                            "Passthrough: forwarding sampling request to external client"
                                        );

                                        if let Some(response_rx) = sender(&session_key, forward_req) {
                                            match tokio::time::timeout(
                                                std::time::Duration::from_secs(120),
                                                response_rx,
                                            ).await {
                                                Ok(Ok(response)) => {
                                                    if let Some(err) = response.error {
                                                        JsonRpcResponse::error(
                                                            request_id,
                                                            err,
                                                        )
                                                    } else {
                                                        JsonRpcResponse::success(
                                                            request_id,
                                                            response.result.unwrap_or(json!({})),
                                                        )
                                                    }
                                                }
                                                Ok(Err(_)) | Err(_) => {
                                                    JsonRpcResponse::error(
                                                        request_id,
                                                        JsonRpcError::custom(
                                                            -32603,
                                                            "Sampling passthrough timed out waiting for client response".to_string(),
                                                            None,
                                                        ),
                                                    )
                                                }
                                            }
                                        } else {
                                            JsonRpcResponse::error(
                                                request_id,
                                                JsonRpcError::custom(
                                                    -32603,
                                                    "No SSE connection for client - cannot passthrough sampling request".to_string(),
                                                    None,
                                                ),
                                            )
                                        }
                                    } else {
                                        JsonRpcResponse::error(
                                            request_id,
                                            JsonRpcError::custom(
                                                -32603,
                                                "Sampling passthrough unavailable: no client request sender configured".to_string(),
                                                None,
                                            ),
                                        )
                                    }
                                }
                                lr_config::SamplingBehavior::DirectAllow | lr_config::SamplingBehavior::DirectAsk => {
                                    Self::handle_sampling_direct(
                                        &router, &client_id, request.params, request_id
                                    ).await
                                }
                                lr_config::SamplingBehavior::Off => {
                                    JsonRpcResponse::error(
                                        request_id,
                                        JsonRpcError::custom(-32601, "Sampling is disabled".to_string(), None),
                                    )
                                }
                            }
                        }

                        _ => {
                            JsonRpcResponse::error(
                                request_id,
                                JsonRpcError::method_not_found(&request.method),
                            )
                        }
                    }
                })
            }));
        }
    }

    /// Retry only relevant unavailable backends, under the existing initialization
    /// lock. Healthy tools and local virtual calls never wait for unrelated servers.
    async fn recover_backends_for_request(
        &self,
        session: &Arc<RwLock<GatewaySession>>,
        request: &JsonRpcRequest,
    ) {
        let (transports, requested, prefix, retry_after) = {
            let state = session.read().await;
            if state.initialization_pending {
                return;
            }
            let Some(transports) = state.transports.clone() else {
                return;
            };
            let prefix = match request.method.as_str() {
                "tools/list" | "resources/list" | "prompts/list" => None,
                "tools/call" | "prompts/get" => {
                    let Some(name) = request
                        .params
                        .as_ref()
                        .and_then(|p| p.get("name"))
                        .and_then(Value::as_str)
                    else {
                        return;
                    };
                    let mapped = if request.method == "tools/call" {
                        state.tool_mapping.get(name)
                    } else {
                        state.prompt_mapping.get(name)
                    };
                    if let Some((id, _)) = mapped {
                        self.server_manager.get_config(id).map(|c| slugify(&c.name))
                    } else {
                        let Some((prefix, _)) = parse_namespace(name) else {
                            return;
                        };
                        Some(prefix)
                    }
                }
                "resources/read" => {
                    let Some(params) = request.params.as_ref() else {
                        return;
                    };
                    let mapped = params
                        .get("name")
                        .and_then(Value::as_str)
                        .and_then(|name| state.resource_mapping.get(name))
                        .or_else(|| {
                            params
                                .get("uri")
                                .and_then(Value::as_str)
                                .and_then(|uri| state.resource_uri_mapping.get(uri))
                        });
                    let Some((id, _)) = mapped else {
                        return;
                    };
                    self.server_manager.get_config(id).map(|c| slugify(&c.name))
                }
                _ => return,
            };
            (
                transports,
                state.requested_servers.clone(),
                prefix,
                state.backend_retry_after.clone(),
            )
        };
        let mut candidates = Vec::new();
        for id in requested {
            let Some(config) = self.server_manager.get_config(&id).filter(|c| c.enabled) else {
                continue;
            };
            if prefix.as_ref().is_some_and(|p| *p != slugify(&config.name)) {
                continue;
            }
            if retry_after
                .get(&id)
                .is_some_and(|at| *at > std::time::Instant::now())
            {
                continue;
            }
            let failed = !matches!(
                session.read().await.server_init_status.get(&id),
                Some(InitStatus::Completed(_))
            );
            if failed
                || !transports.configuration_matches(&id, &config)
                || transports.needs_reconnect(&id).await
            {
                candidates.push(id);
            }
        }
        if candidates.is_empty() {
            return;
        }
        if request.method.ends_with("/list") {
            // Discovery and local tools stay responsive while an unavailable
            // remote reconnects. Reuse the session's existing abort handle so
            // teardown cancels recovery exactly like initial warm-up.
            let mut state = session.write().await;
            if state
                .initialization_abort
                .as_ref()
                .is_some_and(|task| !task.is_finished())
            {
                return;
            }
            let gateway = self.clone();
            let background_session = session.clone();
            let task = tokio::spawn(async move {
                gateway
                    .recover_session_servers(&background_session, candidates)
                    .await;
                background_session.write().await.initialization_abort = None;
            });
            state.initialization_abort = Some(task.abort_handle());
        } else {
            self.recover_session_servers(session, candidates).await;
        }
    }

    pub(super) async fn recover_session_servers(
        &self,
        session: &Arc<RwLock<GatewaySession>>,
        candidates: Vec<String>,
    ) {
        let (transports, lock) = {
            let state = session.read().await;
            let Some(transports) = state.transports.clone() else {
                return;
            };
            (transports, state.lazy_init_lock.clone())
        };
        let _guard = lock.lock().await;
        let mut retry = Vec::new();
        for id in candidates {
            let failed = !matches!(
                session.read().await.server_init_status.get(&id),
                Some(InitStatus::Completed(_))
            );
            let config_changed = self
                .server_manager
                .get_config(&id)
                .is_some_and(|config| !transports.configuration_matches(&id, &config));
            if failed || config_changed || transports.needs_reconnect(&id).await {
                let mut state = session.write().await;
                if !state.requested_servers.contains(&id) || state.initialization_pending {
                    continue;
                }
                if state
                    .backend_retry_after
                    .get(&id)
                    .is_some_and(|at| *at > std::time::Instant::now())
                {
                    continue;
                }
                state.backend_retry_after.insert(
                    id.clone(),
                    std::time::Instant::now() + Duration::from_secs(30),
                );
                retry.push(id);
            }
        }
        if !retry.is_empty() {
            self.attach_servers_to_session(session, retry).await;
        }
    }

    /// Use the same handshake for newly granted and recovering servers. Rebuild
    /// just their transports; never replay the operation whose outcome was lost.
    pub(crate) async fn attach_servers_to_session(
        &self,
        session: &Arc<RwLock<GatewaySession>>,
        server_ids: Vec<String>,
    ) {
        let (transports, client_id, session_key, client_mode, capabilities) = {
            let state = session.read().await;
            (
                state.transports.clone(),
                state.client_id.clone(),
                state.session_key.clone(),
                state.client_mode,
                state
                    .client_capabilities
                    .as_ref()
                    .and_then(|c| serde_json::to_value(c).ok())
                    .unwrap_or_else(|| json!({})),
            )
        };
        let Some(transports) = transports else {
            return;
        };
        let timeout = Duration::from_secs(self.config.server_timeout_seconds.max(15));
        {
            let mut state = session.write().await;
            for id in &server_ids {
                state
                    .server_init_status
                    .insert(id.clone(), InitStatus::NotStarted);
            }
        }
        let results = futures::future::join_all(server_ids.iter().map(|server_id| {
            let transports = transports.clone();
            let capabilities = capabilities.clone();
            let client_id = client_id.clone();
            let session_key = session_key.clone();
            async move {
                // An enabled check here also protects attachment after a settings
                // change while a failed connection was waiting for its retry.
                let result = async {
                    if !self.server_manager.get_config(server_id).is_some_and(|c| c.enabled) {
                        return Err(AppError::Mcp("MCP server is disabled or removed".into()));
                    }
                    if !self.sessions.get(&session_key).is_some_and(|entry| Arc::ptr_eq(entry.value(), session)) {
                        return Err(AppError::Mcp("MCP client session ended during reconnection".into()));
                    }
                    transports.close_server(server_id).await;
                    let (transport, config) = self.server_manager.create_transport_snapshot(server_id).await?;
                    transports.insert_configured(server_id.clone(), transport, config);
                    transports.mark_connecting(server_id);
                    let ids = vec![server_id.clone()];
                    self.register_request_handlers_on_transports(&ids, &client_id, &session_key, client_mode, &transports);
                    self.register_notification_handlers_on_transports(&ids, session, &transports);
                    // Re-probe during recovery: the endpoint or protocol may
                    // have changed since the cached legacy classification.
                    if let Some(result) = self.probe_server_discover(&transports, server_id, Duration::from_secs(2)).await {
                        transports.set_revision(server_id, crate::protocol::ProtocolRevision::V2026_07_28);
                        self.backend_revision_cache.insert(server_id.clone(), crate::protocol::ProtocolRevision::V2026_07_28);
                        return Ok(result);
                    }
                    let init = JsonRpcRequest::new(Some(json!("_attach_init")), "initialize".into(), Some(json!({
                        "protocolVersion": crate::protocol::MCP_PROTOCOL_VERSION,
                        "capabilities": capabilities,
                        "clientInfo": {"name":"LocalRouter MCP Gateway", "version":env!("CARGO_PKG_VERSION")}
                    })));
                    let response = transports.send_request(server_id, init).await?;
                    if response.error.is_some() { return Err(AppError::Mcp("MCP initialize rejected".into())); }
                    let result: InitializeResult = serde_json::from_value(response.result.unwrap_or(Value::Null))
                        .map_err(|_| AppError::Mcp("Invalid MCP initialize response".into()))?;
                    transports.send_request(server_id, JsonRpcRequest::new(None, "notifications/initialized".into(), Some(json!({})))).await?;
                    self.backend_revision_cache.insert(server_id.clone(), crate::protocol::ProtocolRevision::V2025_11_25);
                    Ok(result)
                };
                let result = match tokio::time::timeout(timeout, result).await {
                    Ok(result) => result,
                    Err(_) => Err(AppError::Mcp("MCP backend reconnection timed out".into())),
                };
                let result = if !self.sessions.get(&session_key).is_some_and(|entry| Arc::ptr_eq(entry.value(), session)) {
                    Err(AppError::Mcp("MCP client session ended during reconnection".into()))
                } else { result };
                if result.is_err() { transports.close_server(server_id).await; }
                (server_id.clone(), result)
            }
        })).await;
        let mut changed = false;
        let mut recovered = Vec::new();
        {
            let mut state = session.write().await;
            for (id, result) in results {
                match result {
                    Ok(result) => {
                        recovered.push((id.clone(), result.clone()));
                        state
                            .server_init_status
                            .insert(id.clone(), InitStatus::Completed(result));
                        if !state.allowed_servers.contains(&id) {
                            state.allowed_servers.push(id.clone());
                        }
                        state.backend_retry_after.remove(&id);
                        state.last_broadcast_failures.retain(|f| f.server_id != id);
                        changed = true;
                    }
                    Err(error) => {
                        tracing::warn!(server_id = %id, "MCP reconnection failed: {error}");
                        let error = "MCP upstream connection failed; check authentication and server availability".to_string();
                        transports.record_failure(&id, "_connect", &error);
                        state.allowed_servers.retain(|server| server != &id);
                        state.server_init_status.insert(
                            id,
                            InitStatus::Failed {
                                error,
                                retry_count: 0,
                            },
                        );
                    }
                }
            }
            let (successes, failures) = session_initialization_results(&state);
            state.merged_capabilities = Some(merge_initialize_results(successes, failures));
            if !changed {
                state.invalidate_all_caches();
            }
        }
        if !self
            .sessions
            .get(&session_key)
            .is_some_and(|entry| Arc::ptr_eq(entry.value(), session))
        {
            transports.close_all().await;
            return;
        }
        if changed {
            self.observe_upstream_capabilities(&transports, &recovered);
            let (successes, failures) = {
                let state = session.read().await;
                session_initialization_results(&state)
            };
            self.refresh_session_catalogs(session, successes, failures)
                .await;
            self.send_list_changed(
                &session_notification_key(&session_key),
                super::session::ListChanges {
                    tools: true,
                    resources: true,
                    prompts: true,
                },
            );
        }
    }

    /// Register per-session notification callbacks on transports.
    ///
    /// Replaces the global `register_notification_handlers` for sessions that
    /// own their own transport set. Notifications invalidate session caches and
    /// are forwarded to SSE clients.
    fn register_notification_handlers_on_transports(
        &self,
        server_ids: &[String],
        session: &Arc<RwLock<GatewaySession>>,
        transport_set: &Arc<SessionTransportSet>,
    ) {
        use crate::transport::NotificationCallback;

        for server_id in server_ids {
            let transport = match transport_set.get(server_id) {
                Some(t) => t,
                None => continue,
            };

            // Weak: the session owns the transport that owns this callback
            let session_weak = Arc::downgrade(session);
            let server_id_clone = server_id.clone();
            let broadcast_clone = self.notification_broadcast.clone();
            let push_relay = self.push_relay.clone();
            let server_manager = self.server_manager.clone();
            let snapshot_recorder = self.snapshot_recorder.clone();

            let callback: NotificationCallback =
                Arc::new(move |notification: JsonRpcNotification| {
                    let session_weak = session_weak.clone();
                    let server_id_inner = server_id_clone.clone();
                    let broadcast_inner = broadcast_clone.clone();
                    let push_relay = push_relay.clone();
                    let server_manager = server_manager.clone();
                    let snapshot_recorder = snapshot_recorder.clone();

                    tokio::spawn(async move {
                        let Some(session_inner) = session_weak.upgrade() else {
                            return; // session already closed
                        };
                        let method = notification.method.as_str();
                        if method == super::push_relay::EVENTS_LIST_CHANGED_NOTIFICATION {
                            let (transports, init) = {
                                let state = session_inner.read().await;
                                let init = match state.server_init_status.get(&server_id_inner) {
                                    Some(InitStatus::Completed(init)) => Some(init.clone()),
                                    _ => None,
                                };
                                (state.transports.clone(), init)
                            };
                            if let (Some(transports), Some(init)) = (transports, init) {
                                spawn_capability_observation(
                                    server_manager,
                                    snapshot_recorder,
                                    transports,
                                    server_id_inner.clone(),
                                    init,
                                );
                            }
                        }
                        if method == super::push_relay::CLAUDE_CHANNEL_NOTIFICATION
                            || method.starts_with(super::push_relay::EVENTS_NOTIFICATION_PREFIX)
                        {
                            let service = push_relay.read().clone();
                            if let Some(service) = service {
                                let (session_key, client_id) = {
                                    let state = session_inner.read().await;
                                    (state.session_key.clone(), state.client_id.clone())
                                };
                                if service.on_upstream_notification(UpstreamPushNotification {
                                    server_id: server_id_inner.clone(),
                                    session_key,
                                    client_id,
                                    notification: notification.clone(),
                                }) {
                                    return;
                                }
                            }
                        }
                        match notification.method.as_str() {
                            "notifications/tools/list_changed" => {
                                tracing::info!(
                                    "Received tools/list_changed notification from server: {}",
                                    server_id_inner
                                );
                                {
                                    let mut session_write = session_inner.write().await;
                                    session_write.cache_ttl_manager.record_invalidation();
                                    session_write.cached_tools = None;
                                }
                            }
                            "notifications/resources/list_changed" => {
                                tracing::info!(
                                    "Received resources/list_changed notification from server: {}",
                                    server_id_inner
                                );
                                {
                                    let mut session_write = session_inner.write().await;
                                    session_write.cache_ttl_manager.record_invalidation();
                                    session_write.cached_resources = None;
                                }
                            }
                            "notifications/prompts/list_changed" => {
                                tracing::info!(
                                    "Received prompts/list_changed notification from server: {}",
                                    server_id_inner
                                );
                                {
                                    let mut session_write = session_inner.write().await;
                                    session_write.cache_ttl_manager.record_invalidation();
                                    session_write.cached_prompts = None;
                                }
                            }
                            other_method => {
                                tracing::debug!(
                                    "Received notification from server {}: {}",
                                    server_id_inner,
                                    other_method
                                );
                            }
                        }

                        // Forward to the client of this session only: the transport
                        // belongs to the session, so progress tokens, log messages and
                        // resource updates are this client's own and pass through unchanged.
                        if let Some(broadcast) = broadcast_inner.as_ref() {
                            let session_key = session_inner.read().await.session_key.clone();
                            let payload = (
                                super::types::session_server_notification_key(
                                    &session_key,
                                    &server_id_inner,
                                ),
                                notification,
                            );
                            match broadcast.send(payload) {
                                Ok(receiver_count) => {
                                    tracing::debug!(
                                        "Forwarded notification from server {} to {} client(s)",
                                        server_id_inner,
                                        receiver_count
                                    );
                                }
                                Err(_) => {
                                    tracing::trace!(
                                        "No clients subscribed to notifications from server {}",
                                        server_id_inner
                                    );
                                }
                            }
                        }
                    });
                });

            transport.set_notification_callback(callback);
        }
    }

    /// Handle elicitation via Direct mode (gateway popup)
    async fn handle_elicitation_direct(
        elicitation_mgr: &super::elicitation::ElicitationManager,
        server_id: String,
        session_key: Option<String>,
        params: Option<Value>,
        request_id: Value,
    ) -> JsonRpcResponse {
        let params = params.unwrap_or(json!({}));
        let message = params
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("Server requests input")
            .to_string();
        let schema = params
            .get("requestedSchema")
            .or_else(|| params.get("schema"))
            .cloned()
            .unwrap_or(json!({}));

        let elicitation_req = crate::protocol::ElicitationRequest { message, schema };

        match elicitation_mgr
            .request_input_for_session(server_id, session_key, elicitation_req, None)
            .await
        {
            Ok(response) => {
                // Build MCP-spec-compliant response: { action, content }
                JsonRpcResponse::success(
                    request_id,
                    json!({
                        "action": "accept",
                        "content": response.data,
                    }),
                )
            }
            Err(e) => {
                tracing::warn!("Elicitation failed: {}", e);
                JsonRpcResponse::success(request_id, json!({ "action": "decline" }))
            }
        }
    }

    /// Handle sampling via Direct mode (route through LLM provider)
    async fn handle_sampling_direct(
        router: &Router,
        client_id: &str,
        params: Option<Value>,
        request_id: Value,
    ) -> JsonRpcResponse {
        let sampling_req = match params.as_ref().and_then(|p| {
            serde_json::from_value::<crate::protocol::SamplingRequest>(p.clone()).ok()
        }) {
            Some(req) => req,
            None => {
                return JsonRpcResponse::error(
                    request_id,
                    JsonRpcError::invalid_params("Invalid sampling request".to_string()),
                );
            }
        };

        let mut completion_req =
            match super::sampling::convert_sampling_to_chat_request(sampling_req) {
                Ok(req) => req,
                Err(e) => {
                    return JsonRpcResponse::error(
                        request_id,
                        JsonRpcError::custom(
                            -32603,
                            format!("Failed to convert sampling request: {}", e),
                            None,
                        ),
                    );
                }
            };

        if completion_req.model.is_empty() {
            completion_req.model = "localrouter/auto".to_string();
        }

        match router.complete(client_id, completion_req).await {
            Ok((resp, _routing_meta)) => {
                match super::sampling::convert_chat_to_sampling_response(resp) {
                    Ok(sampling_resp) => {
                        // Build MCP-spec-compliant response
                        let content = match &sampling_resp.content {
                            crate::protocol::SamplingContent::Text(text) => {
                                json!({ "type": "text", "text": text })
                            }
                            crate::protocol::SamplingContent::Structured(v) => v.clone(),
                        };
                        JsonRpcResponse::success(
                            request_id,
                            json!({
                                "model": sampling_resp.model,
                                "stopReason": sampling_resp.stop_reason,
                                "role": sampling_resp.role,
                                "content": content,
                            }),
                        )
                    }
                    Err(e) => JsonRpcResponse::error(
                        request_id,
                        JsonRpcError::custom(
                            -32603,
                            format!("Failed to convert sampling response: {}", e),
                            None,
                        ),
                    ),
                }
            }
            Err(e) => JsonRpcResponse::error(
                request_id,
                JsonRpcError::custom(-32603, format!("LLM completion failed: {}", e), None),
            ),
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
            "resources/templates/list" => {
                self.handle_resource_templates_list(session, request).await
            }
            "prompts/list" => self.handle_prompts_list(session, request).await,
            method if PushMethod::from_method(method).is_some() => {
                let method = PushMethod::from_method(method).expect("matched above");
                self.handle_push_request(session, method, request).await
            }
            "ping" if !session.read().await.protocol_revision.is_stateless() => Ok(
                JsonRpcResponse::success(request.id.unwrap_or(Value::Null), json!({})),
            ),
            "logging/setLevel" | "ping" => {
                // Removed in 2026-07-28: stateless peers set the log level
                // per request via _meta and have no ping. Legacy peers keep
                // the old broadcast behavior.
                if session.read().await.protocol_revision.is_stateless() {
                    return Ok(JsonRpcResponse::error(
                        request.id.unwrap_or(Value::Null),
                        JsonRpcError::method_not_found(&request.method),
                    ));
                }
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

            "server/discover" => self.handle_server_discover(session, request).await,

            "completion/complete" => self.handle_completion_complete(session, request).await,

            "sampling/createMessage" => {
                // Sampling is handled in route handlers (mcp.rs) not in gateway
                // This shouldn't be called from unified gateway - use individual server proxy instead
                Ok(JsonRpcResponse::error(
                    request.id.unwrap_or(Value::Null),
                    JsonRpcError::custom(
                        -32601,
                        "sampling/createMessage should use individual server proxy endpoint"
                            .to_string(),
                        Some(json!({
                            "hint": "Use POST /mcp/:server_id for sampling requests from backend servers"
                        })),
                    ),
                ))
            }

            "elicitation/requestInput" => self.handle_elicitation_request(session, request).await,

            "roots/list" => self.handle_roots_list(session, request).await,

            // Resource subscription methods
            "resources/subscribe" => self.handle_resources_subscribe(session, request).await,

            "resources/unsubscribe" => self.handle_resources_unsubscribe(session, request).await,

            // MCP Events (SEP-3415): only the Push Relay may answer these;
            // they are never broadcast or forwarded upstream.
            method if PushMethod::from_method(method).is_some() => {
                let method = PushMethod::from_method(method).expect("matched above");
                self.handle_push_request(session, method, request).await
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

    /// Handle completion/complete request
    ///
    /// Forwards argument auto-completion requests to the appropriate upstream server.
    /// The `ref` field identifies the prompt or resource being completed (namespaced),
    /// which we un-namespace before forwarding.
    async fn handle_completion_complete(
        &self,
        session: Arc<RwLock<GatewaySession>>,
        request: JsonRpcRequest,
    ) -> AppResult<JsonRpcResponse> {
        let request_id = request.id.clone().unwrap_or(Value::Null);
        let params = match request.params.as_ref() {
            Some(p) => p.clone(),
            None => {
                return Ok(JsonRpcResponse::error(
                    request_id,
                    JsonRpcError::invalid_params("Missing params"),
                ));
            }
        };

        // Extract ref object and its name
        let ref_obj = match params.get("ref") {
            Some(r) => r,
            None => {
                return Ok(JsonRpcResponse::error(
                    request_id,
                    JsonRpcError::invalid_params("Missing 'ref' in params"),
                ));
            }
        };

        let ref_name = match ref_obj.get("name").and_then(|v| v.as_str()) {
            Some(name) => name,
            None => {
                return Ok(JsonRpcResponse::error(
                    request_id,
                    JsonRpcError::invalid_params("Missing 'name' in ref"),
                ));
            }
        };

        // Parse namespace from ref name to determine target server
        let (server_id, original_name) = match parse_namespace(ref_name) {
            Some(parsed) => parsed,
            None => {
                return Ok(JsonRpcResponse::error(
                    request_id,
                    JsonRpcError::invalid_params(format!(
                        "Could not determine server from ref name '{}'. Expected namespaced format (e.g., 'server__name').",
                        ref_name
                    )),
                ));
            }
        };

        // Build modified params with un-namespaced ref name
        let mut modified_params = params.clone();
        if let Some(ref_mut) = modified_params.get_mut("ref") {
            if let Some(obj) = ref_mut.as_object_mut() {
                obj.insert("name".to_string(), json!(original_name));
            }
        }

        // Forward to the specific upstream server via session transports
        let forward_request = JsonRpcRequest::new(
            Some(request_id.clone()),
            "completion/complete".to_string(),
            Some(modified_params),
        );

        let transports = session
            .read()
            .await
            .transports
            .clone()
            .ok_or_else(|| AppError::Mcp("Session not initialized".into()))?;

        let timeout = Duration::from_secs(self.config.server_timeout_seconds);
        match tokio::time::timeout(
            timeout,
            transports.send_request(&server_id, forward_request),
        )
        .await
        {
            Ok(Ok(response)) => Ok(response),
            Ok(Err(e)) => Ok(JsonRpcResponse::error(
                request_id,
                JsonRpcError::internal_error(format!(
                    "completion/complete failed for server '{}': {}",
                    server_id, e
                )),
            )),
            Err(_) => Ok(JsonRpcResponse::error(
                request_id,
                JsonRpcError::internal_error(format!(
                    "completion/complete timed out for server '{}'",
                    server_id
                )),
            )),
        }
    }

    /// Handle initialize request
    async fn handle_initialize(
        &self,
        session: Arc<RwLock<GatewaySession>>,
        request: JsonRpcRequest,
    ) -> AppResult<JsonRpcResponse> {
        let request_id = request.id.clone().unwrap_or(Value::Null);
        let started = tokio::time::Instant::now();
        let awaiting_discovery = {
            let requested = session.read().await.requested_servers.clone();
            self.servers_awaiting_first_discovery(&requested)
        };
        let gateway = self.clone();
        let background_session = session.clone();
        let (sender, mut receiver) = tokio::sync::oneshot::channel();
        {
            let mut state = session.write().await;
            // Local skill search must be ready before any remote startup. The
            // normal full catalog pass will add upstream entries afterwards.
            let skill_entries = self
                .virtual_servers
                .read()
                .iter()
                .find(|vs| vs.id() == "_skills")
                .and_then(|vs| {
                    state
                        .virtual_server_state
                        .get(vs.id())
                        .map(|s| vs.catalog_index_entries(s.as_ref()))
                })
                .unwrap_or_default();
            if let Some(cm) = state
                .virtual_server_state
                .get_mut("_context_mode")
                .and_then(|s| {
                    s.as_any_mut()
                        .downcast_mut::<super::context_mode::ContextModeSessionState>()
                })
            {
                if cm.catalog_compression_enabled {
                    for (label, content) in skill_entries {
                        match cm.store.index(&label, &content) {
                            Ok(_) => {
                                cm.catalog_sources
                                    .insert(label, super::context_mode::CatalogItemType::Skill);
                            }
                            Err(error) => tracing::warn!(
                                "Could not index local skill before MCP warm-up: {error}"
                            ),
                        }
                    }
                }
            }
            state.initialization_pending = true;
            let task = tokio::spawn(async move {
                let start = std::time::Instant::now();
                let result = gateway
                    .initialize_backends(background_session.clone(), request)
                    .await;
                let (key, deferred) = {
                    let mut state = background_session.write().await;
                    state.initialization_pending = false;
                    state.initialization_abort = None;
                    (state.session_key.clone(), state.initialization_deferred)
                };
                tracing::info!(session_key = %key, elapsed_ms = start.elapsed().as_millis(),
                    "MCP backend warm-up finished");
                if deferred {
                    gateway.send_list_changed(
                        &session_notification_key(&key),
                        super::session::ListChanges {
                            tools: true,
                            resources: true,
                            prompts: true,
                        },
                    );
                }
                let _ = sender.send(result);
            });
            state.initialization_abort = Some(task.abort_handle());
        }
        let mut outcome = tokio::time::timeout(Duration::from_secs(2), &mut receiver).await;
        if outcome.is_err() && !awaiting_discovery.is_empty() {
            // A server never seen before (or reconfigured) gets longer: its
            // capabilities decide what this session can be offered.
            let deadline = started + *self.first_discovery_wait.read();
            tokio::select! {
                result = &mut receiver => outcome = Ok(result),
                _ = self.wait_for_first_discovery(&session, &awaiting_discovery, deadline) => {}
            }
        }
        match outcome {
            Ok(Ok(Ok(mut response))) if !awaiting_discovery.is_empty() => {
                // The handshake can finish before the new server's
                // capabilities are remembered (they are read in the
                // background): wait for them within the same bound, then
                // advertise what they allow.
                let deadline = started + *self.first_discovery_wait.read();
                self.wait_for_first_discovery(&session, &awaiting_discovery, deadline)
                    .await;
                if let Some(capabilities) = response
                    .result
                    .as_mut()
                    .and_then(|result| result.get_mut("capabilities"))
                {
                    let state = session.read().await;
                    self.apply_push_advertisement(&state, capabilities);
                }
                Ok(response)
            }
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(AppError::Mcp("MCP initialization task stopped".into())),
            Err(_) => {
                let (completed, key) = {
                    let mut state = session.write().await;
                    state.initialization_deferred = true;
                    (!state.initialization_pending, state.session_key.clone())
                };
                // Completion can race the foreground deadline. If it already
                // passed the deferred check, publish the change here instead.
                if completed {
                    self.send_list_changed(
                        &session_notification_key(&key),
                        super::session::ListChanges {
                            tools: true,
                            resources: true,
                            prompts: true,
                        },
                    );
                }
                let virtual_instructions = self.collect_virtual_instructions(&session).await;
                let state = session.read().await;
                let unavailable = state.requested_servers.iter().map(|id| UnavailableServerInfo {
                    name: self.server_manager.get_config(id).map(|c| c.name).unwrap_or_else(|| id.clone()),
                    error: "Still connecting in the background; tools will appear after list_changed".into(),
                }).collect();
                let instructions = build_gateway_instructions(&InstructionsContext {
                    unavailable_servers: unavailable,
                    virtual_instructions,
                    ..Default::default()
                });
                let mut value = json!({
                    "protocolVersion": crate::protocol::MCP_PROTOCOL_VERSION,
                    "capabilities": {
                        "tools": {"listChanged":true},
                        "resources": {"listChanged":true, "subscribe":true},
                        "prompts": {"listChanged":true},
                        "logging": {},
                        "completions": {}
                    },
                    "serverInfo":{"name":"LocalRouter MCP Gateway","version":env!("CARGO_PKG_VERSION")},
                    "_meta":{"initializing":state.initialization_pending}
                });
                if let Some(instructions) = instructions {
                    value["instructions"] = json!(instructions);
                }
                self.apply_push_advertisement(&state, &mut value["capabilities"]);
                Ok(JsonRpcResponse::success(request_id, value))
            }
        }
    }

    /// Local discovery remains usable while remote authentication/handshakes run.
    async fn pending_initialization_response(
        &self,
        session: &Arc<RwLock<GatewaySession>>,
        request: &JsonRpcRequest,
    ) -> Option<JsonRpcResponse> {
        let state = session.read().await;
        if !state.initialization_pending {
            return None;
        }
        let key = match request.method.as_str() {
            "tools/list" => "tools",
            "resources/list" => "resources",
            "prompts/list" => "prompts",
            "resources/templates/list" => "resourceTemplates",
            "tools/call" | "resources/read" | "prompts/get" => {
                let local = request.method == "tools/call"
                    && request
                        .params
                        .as_ref()
                        .and_then(|p| p.get("name"))
                        .and_then(Value::as_str)
                        .is_some_and(|name| {
                            self.virtual_servers
                                .read()
                                .iter()
                                .any(|vs| vs.owns_tool(name))
                        });
                if local {
                    return None;
                }
                return Some(JsonRpcResponse::error(request.id.clone().unwrap_or(Value::Null), JsonRpcError::custom(
                    -32000, "Upstream MCP servers are still initializing; retry after tools/list_changed", None)));
            }
            _ => return None,
        };
        let mut items = Vec::new();
        if key == "tools" {
            self.append_virtual_server_tools(&mut items, &state);
        }
        let failures: Vec<_> = state.requested_servers.iter().map(|id| json!({
            "server_id":id, "error":"Connection is still initializing; upstream tools are not available yet"
        })).collect();
        Some(JsonRpcResponse::success(
            request.id.clone().unwrap_or(Value::Null),
            json!({
                key:items, "_meta":{"initializing":true,"partial_failure":!failures.is_empty(),"failures":failures}
            }),
        ))
    }

    async fn initialize_backends(
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
        {
            let mut state = session.write().await;
            state.client_capabilities = client_capabilities.clone();
            state.client_capabilities_raw = raw_capabilities.cloned();
        }

        // Create per-session transports for all allowed servers
        let start_timeout = tokio::time::Duration::from_secs(15);
        let (transport_set, start_failures) = self
            .server_manager
            .create_transports(&allowed_servers, start_timeout)
            .await;
        let transport_set = Arc::new(transport_set);
        let started_servers = transport_set.running_server_ids();

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
                protocol_version: crate::protocol::MCP_PROTOCOL_VERSION.to_string(),
                capabilities: ServerCapabilities {
                    tools: Some(ToolsCapability {
                        list_changed: Some(true),
                    }),
                    resources: None,
                    prompts: None,
                    logging: None,
                    completions: None,
                    ..Default::default()
                },
                server_info: ServerInfo {
                    name: "LocalRouter MCP Gateway".to_string(),
                    version: env!("CARGO_PKG_VERSION").to_string(),
                    description: None,
                },
                failures: start_failures,
                instructions: instructions.clone(),
                providers: CapabilityProviders::default(),
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
                session_write.transports = Some(transport_set.clone());
                self.apply_push_advertisement(&session_write, &mut response_value["capabilities"]);
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

        // Filter client capabilities based on global gateway settings before broadcasting
        let broadcast_request_msg = {
            let sampling_behavior = self.live_sampling.read().clone();
            let elicitation_mode = self.live_elicitation_mode.read().clone();
            let mut modified_request = request.clone();

            if let Some(ref mut params) = modified_request.params {
                if let Some(caps) = params.get_mut("capabilities") {
                    // Remove sampling capability if behavior is Off
                    if matches!(sampling_behavior, lr_config::SamplingBehavior::Off) {
                        if let Some(obj) = caps.as_object_mut() {
                            obj.remove("sampling");
                        }
                    } else if !caps.as_object().is_some_and(|o| o.contains_key("sampling")) {
                        // Ensure sampling capability exists if permission is Allow/Ask
                        if let Some(obj) = caps.as_object_mut() {
                            obj.insert("sampling".to_string(), serde_json::json!({}));
                        }
                    }

                    // Remove elicitation capability if mode is Off
                    if matches!(elicitation_mode, lr_config::ElicitationMode::Off) {
                        if let Some(obj) = caps.as_object_mut() {
                            obj.remove("elicitation");
                        }
                    } else {
                        // For Direct mode, always advertise even if client doesn't support it
                        // For Passthrough mode, ensure it's present if client declared support
                        if !caps
                            .as_object()
                            .is_some_and(|o| o.contains_key("elicitation"))
                        {
                            if let Some(obj) = caps.as_object_mut() {
                                obj.insert("elicitation".to_string(), serde_json::json!({}));
                            }
                        }
                    }
                }
            }
            modified_request
        };

        // Declare the (filtered) client capabilities to stateless backends:
        // they travel in `_meta` on every request instead of an initialize.
        if let Some(caps) = broadcast_request_msg
            .params
            .as_ref()
            .and_then(|p| p.get("capabilities"))
        {
            transport_set.set_client_capabilities(caps.clone());
        }

        // Probe backends for 2026-07-28 stateless support (server/discover)
        // before falling back to the legacy initialize handshake. Backends
        // already known to be legacy skip the probe.
        let mut discovered_results: Vec<(String, InitializeResult)> = Vec::new();
        let mut servers_needing_initialize: Vec<String> = Vec::new();
        let probe_timeout = Duration::from_secs(5);
        // Probe independently: one slow backend must not add five seconds
        // for every configured server before any legacy handshake can begin.
        let probes = futures::future::join_all(servers_to_initialize.iter().map(|server_id| {
            let transport_set = transport_set.clone();
            async move {
                let known_legacy = self
                    .backend_revision_cache
                    .get(server_id)
                    .is_some_and(|entry| !entry.value().is_stateless());
                let discovered = if known_legacy {
                    None
                } else {
                    let start = std::time::Instant::now();
                    let discovered = self
                        .probe_server_discover(&transport_set, server_id, probe_timeout)
                        .await;
                    // Unsupported discovery is an expected legacy fallback.
                    self.server_manager.report_startup_phase(
                        server_id,
                        "protocol_probe",
                        start.elapsed().as_millis() as u64,
                        true,
                    );
                    discovered
                };
                (server_id.clone(), discovered)
            }
        }))
        .await;
        for (server_id, discovered) in probes {
            if let Some(init_result) = discovered {
                self.backend_revision_cache.insert(
                    server_id.clone(),
                    crate::protocol::ProtocolRevision::V2026_07_28,
                );
                transport_set
                    .set_revision(&server_id, crate::protocol::ProtocolRevision::V2026_07_28);
                discovered_results.push((server_id, init_result));
            } else {
                self.backend_revision_cache.insert(
                    server_id.clone(),
                    crate::protocol::ProtocolRevision::V2025_11_25,
                );
                servers_needing_initialize.push(server_id);
            }
        }

        // Broadcast initialize to the legacy backends
        let timeout = Duration::from_secs(self.config.server_timeout_seconds);
        let max_retries = self.config.max_retry_attempts;

        let results =
            futures::future::join_all(servers_needing_initialize.iter().map(|server_id| {
                let request = broadcast_request_msg.clone();
                let transports = transport_set.clone();
                async move {
                    let start = std::time::Instant::now();
                    let mut responses = broadcast_request(
                        std::slice::from_ref(server_id),
                        request,
                        &transports,
                        timeout,
                        max_retries,
                    )
                    .await;
                    let response = responses.pop().expect("one server produces one result");
                    self.server_manager.report_startup_phase(
                        server_id,
                        "initialize",
                        start.elapsed().as_millis() as u64,
                        response.1.is_ok(),
                    );
                    response
                }
            }))
            .await;

        // Separate successes and failures
        let (successes, mut failures) = separate_results(results);

        // Include servers that failed to start in the failures list
        failures.extend(start_failures);

        // Parse initialize results from the legacy handshake
        let mut legacy_init_results = Vec::new();
        for (server_id, value) in successes {
            match serde_json::from_value::<InitializeResult>(value) {
                Ok(result) => legacy_init_results.push((server_id, result)),
                Err(_) => {
                    transport_set.record_failure(
                        &server_id,
                        "initialize",
                        "Invalid MCP initialize response",
                    );
                    failures.push(ServerFailure {
                        server_id,
                        error: "Invalid MCP initialize response".into(),
                    });
                }
            }
        }

        // Only legacy-handshake backends expect notifications/initialized
        let legacy_server_ids: Vec<String> = legacy_init_results
            .iter()
            .map(|(id, _)| id.clone())
            .collect();

        // Combine legacy and discover-probed backends for merging and handlers
        let mut init_results = legacy_init_results;
        init_results.extend(discovered_results);
        {
            let mut state = session.write().await;
            for (id, result) in &init_results {
                state
                    .server_init_status
                    .insert(id.clone(), InitStatus::Completed(result.clone()));
            }
            for failure in &failures {
                state.server_init_status.insert(
                    failure.server_id.clone(),
                    InitStatus::Failed {
                        error: failure.error.clone(),
                        retry_count: 0,
                    },
                );
            }
        }

        // Register per-session request callbacks on transports before completing
        // the handshake. Servers may send requests (elicitation, sampling, roots/list)
        // from their oninitialized callback which fires when they receive the notification.
        let initialized_server_ids: Vec<String> =
            init_results.iter().map(|(id, _)| id.clone()).collect();
        let (client_id_for_callbacks, session_key_for_callbacks, client_mode_for_callbacks) = {
            let s = session.read().await;
            (s.client_id.clone(), s.session_key.clone(), s.client_mode)
        };
        self.register_request_handlers_on_transports(
            &initialized_server_ids,
            &client_id_for_callbacks,
            &session_key_for_callbacks,
            client_mode_for_callbacks,
            &transport_set,
        );

        // Register per-session notification callbacks on transports
        self.register_notification_handlers_on_transports(
            &initialized_server_ids,
            &session,
            &transport_set,
        );

        // Send notifications/initialized to legacy-handshake servers via
        // session transports. This completes the MCP handshake — servers fire
        // oninitialized callbacks which may register conditional tools based on
        // client capabilities. Stateless (2026-07-28) backends have no
        // handshake to complete.
        for server_id in &legacy_server_ids {
            let notification = JsonRpcRequest::new(
                None,
                "notifications/initialized".to_string(),
                Some(json!({})),
            );
            if let Err(e) = transport_set.send_request(server_id, notification).await {
                tracing::warn!(
                    "Failed to send notifications/initialized to server {}: {}",
                    server_id,
                    e
                );
            }
        }

        // Store the transport set on the session
        {
            let mut session_write = session.write().await;
            session_write.transports = Some(transport_set.clone());
        }

        self.observe_upstream_capabilities(&transport_set, &init_results);

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
                    protocol_version: crate::protocol::MCP_PROTOCOL_VERSION.to_string(),
                    capabilities: ServerCapabilities {
                        tools: Some(ToolsCapability {
                            list_changed: Some(true),
                        }),
                        resources: None,
                        prompts: None,
                        logging: None,
                        completions: None,
                        ..Default::default()
                    },
                    server_info: ServerInfo {
                        name: "LocalRouter MCP Gateway (skills-only)".to_string(),
                        version: env!("CARGO_PKG_VERSION").to_string(),
                        description: None,
                    },
                    failures,
                    instructions: instructions.clone(),
                    providers: CapabilityProviders::default(),
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
                    session_write.transports = Some(transport_set.clone());
                    self.apply_push_advertisement(
                        &session_write,
                        &mut response_value["capabilities"],
                    );
                }

                return Ok(JsonRpcResponse::success(
                    request.id.unwrap_or(Value::Null),
                    response_value,
                ));
            }

            // Close transports since all initializations failed and no virtual servers
            transport_set.close_all().await;
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

        let instructions = self
            .refresh_session_catalogs(
                &session,
                init_results_for_instructions,
                merged.failures.clone(),
            )
            .await;

        // Build response
        let mut result = json!({
            "protocolVersion": merged.protocol_version,
            "capabilities": merged.capabilities,
            "serverInfo": merged.server_info,
        });
        if let Some(inst) = &instructions {
            result["instructions"] = json!(inst);
        }
        self.apply_push_advertisement(&*session.read().await, &mut result["capabilities"]);

        Ok(JsonRpcResponse::success(
            request.id.unwrap_or(Value::Null),
            result,
        ))
    }

    /// Rebuild discovery and indexing after initial startup or a repaired
    /// backend, retaining the client's activations, approvals and response index.
    async fn refresh_session_catalogs(
        &self,
        session: &Arc<RwLock<GatewaySession>>,
        init_results_for_instructions: Vec<(String, InitializeResult)>,
        failures: Vec<ServerFailure>,
    ) -> Option<String> {
        let transport_set = session.read().await.transports.clone()?;
        // Build gateway instructions based on the full context
        let client_id_for_log = session.read().await.client_id.clone();

        let virtual_instructions = self.collect_virtual_instructions(session).await;

        // Catalog indexing and deferral run only when MCP Catalog Indexing
        // (catalog compression) is on; Tool Responses Indexing alone leaves
        // the catalog fully visible.
        let (catalog_enabled, cm_catalog_threshold, cm_search_tool_name, cm_read_tool_name) = {
            let session_read = session.read().await;
            if let Some(state) = session_read.virtual_server_state.get("_context_mode") {
                if let Some(cm_state) = state
                    .as_any()
                    .downcast_ref::<super::context_mode::ContextModeSessionState>()
                {
                    (
                        cm_state.catalog_compression_enabled,
                        cm_state.catalog_threshold_bytes,
                        cm_state.search_tool_name.clone(),
                        cm_state.read_tool_name.clone(),
                    )
                } else {
                    (false, 0, "IndexSearch".to_string(), "IndexRead".to_string())
                }
            } else {
                (false, 0, "IndexSearch".to_string(), "IndexRead".to_string())
            }
        };

        // Catalog families are independent. Serial timeouts used to keep
        // the whole gateway (including Skills) waiting for a broken remote.
        let tools_request = JsonRpcRequest::with_id(1, "tools/list".into(), None);
        let resources_request = JsonRpcRequest::with_id(2, "resources/list".into(), None);
        let prompts_request = JsonRpcRequest::with_id(3, "prompts/list".into(), None);
        let tools_servers: Vec<String> = init_results_for_instructions
            .iter()
            .filter(|(_, r)| r.capabilities.tools.is_some())
            .map(|(id, _)| id.clone())
            .collect();
        let resource_servers: Vec<String> = init_results_for_instructions
            .iter()
            .filter(|(_, r)| r.capabilities.resources.is_some())
            .map(|(id, _)| id.clone())
            .collect();
        let prompt_servers: Vec<String> = init_results_for_instructions
            .iter()
            .filter(|(_, r)| r.capabilities.prompts.is_some())
            .map(|(id, _)| id.clone())
            .collect();
        for (ids, method) in [
            (&tools_servers, "tools/list"),
            (&resource_servers, "resources/list"),
            (&prompt_servers, "prompts/list"),
        ] {
            for id in ids {
                transport_set.record_failure(
                    id,
                    method,
                    "MCP catalog is still loading; client tools are not ready yet",
                );
            }
        }
        let (tools_result, resources_result, prompts_result) = tokio::join!(
            self.fetch_and_merge_tools(&tools_servers, tools_request, &transport_set),
            self.fetch_and_merge_resources(&resource_servers, resources_request, &transport_set),
            self.fetch_and_merge_prompts(&prompt_servers, prompts_request, &transport_set),
        );
        let (mut tools_catalog, tool_failures) = tools_result.unwrap_or_default();
        let (mut resources_catalog, resource_failures) = resources_result.unwrap_or_default();
        let (mut prompts_catalog, prompt_failures) = prompts_result.unwrap_or_default();
        {
            let mut state = session.write().await;
            // Preserve known catalogs for unrelated backends that fail a fresh
            // probe during recovery, just as their routing mappings are kept.
            if let Some(cache) = &state.cached_tools {
                tools_catalog.extend(
                    cache
                        .data
                        .iter()
                        .filter(|t| tool_failures.iter().any(|f| f.server_id == t.server_id))
                        .cloned(),
                );
            }
            if let Some(cache) = &state.cached_resources {
                resources_catalog.extend(
                    cache
                        .data
                        .iter()
                        .filter(|r| resource_failures.iter().any(|f| f.server_id == r.server_id))
                        .cloned(),
                );
            }
            if let Some(cache) = &state.cached_prompts {
                prompts_catalog.extend(
                    cache
                        .data
                        .iter()
                        .filter(|p| prompt_failures.iter().any(|f| f.server_id == p.server_id))
                        .cloned(),
                );
            }
            // Initial startup leaves catalog fetch/caching to the normal list
            // handlers. Recovery must replace existing mappings and caches now.
            if !state.initialization_pending {
                state.update_tool_mappings(&tools_catalog, &tool_failures);
                state.update_resource_mappings(&resources_catalog, &resource_failures);
                state.update_prompt_mappings(&prompts_catalog, &prompt_failures);
                let ttl = state.cache_ttl_manager.get_ttl();
                state.cached_tools = Some(
                    CachedList::new(tools_catalog.clone(), ttl)
                        .with_failures(tool_failures.clone()),
                );
                state.cached_resources = Some(
                    CachedList::new(resources_catalog.clone(), ttl)
                        .with_failures(resource_failures),
                );
                state.cached_prompts = Some(
                    CachedList::new(prompts_catalog.clone(), ttl).with_failures(prompt_failures),
                );
                state.last_broadcast_failures = tool_failures;
            }
        }

        let server_infos = self.build_server_instruction_infos(
            &init_results_for_instructions,
            &tools_catalog,
            &resources_catalog,
            &prompts_catalog,
        );
        let unavailable = self.build_unavailable_server_infos(&failures);

        // Context management: index catalog into native FTS5 store synchronously
        // (before computing compression plan, which needs index results).
        // Also compute item definition sizes for the threshold calculation.
        let item_definition_sizes = if catalog_enabled {
            compute_item_definition_sizes(&tools_catalog, &resources_catalog, &prompts_catalog)
        } else {
            std::collections::HashMap::new()
        };

        if catalog_enabled {
            let session_bg = session.clone();
            let server_infos_bg = server_infos.clone();
            let tools_catalog_bg = tools_catalog.clone();
            let resources_catalog_bg = resources_catalog.clone();
            let prompts_catalog_bg = prompts_catalog.clone();
            let client_id_bg = client_id_for_log.clone();

            // Get ContentStore, gateway indexing permissions, and virtual catalog entries
            let (store, gateway_indexing, virtual_catalog_entries) = {
                let session_read = session_bg.read().await;
                let cm_info = session_read
                    .virtual_server_state
                    .get("_context_mode")
                    .and_then(|state| {
                        state
                            .as_any()
                            .downcast_ref::<super::context_mode::ContextModeSessionState>()
                            .map(|cm| (cm.store.clone(), cm.gateway_indexing.clone()))
                    });

                // Collect catalog entries from virtual servers (e.g. skills)
                let mut virt_entries: Vec<(String, String)> = Vec::new();
                for vs in self.virtual_servers.read().iter() {
                    if let Some(state) = session_read.virtual_server_state.get(vs.id()) {
                        virt_entries.extend(vs.catalog_index_entries(state.as_ref()));
                    }
                }

                match cm_info {
                    Some((store, perms)) => (Some(store), perms, virt_entries),
                    None => (
                        None,
                        lr_config::GatewayIndexingPermissions::default(),
                        virt_entries,
                    ),
                }
            };

            if let Some(store) = store {
                // Index synchronously using spawn_blocking (ContentStore uses parking_lot::Mutex)
                let server_infos_idx = server_infos_bg.clone();
                let tools_idx = tools_catalog_bg.clone();
                let resources_idx = resources_catalog_bg.clone();
                let prompts_idx = prompts_catalog_bg.clone();
                let virtual_idx = virtual_catalog_entries.clone();
                let store_idx = store.clone();
                let client_id_idx = client_id_bg.clone();
                let gateway_perms = gateway_indexing.clone();
                let stale_catalog_sources: Vec<String> = {
                    let state = session.read().await;
                    state
                        .virtual_server_state
                        .get("_context_mode")
                        .and_then(|s| {
                            s.as_any()
                                .downcast_ref::<super::context_mode::ContextModeSessionState>()
                        })
                        .map(|cm| {
                            cm.catalog_sources
                                .keys()
                                .filter(|label| label.starts_with("mcp/"))
                                .cloned()
                                .collect()
                        })
                        .unwrap_or_default()
                };

                let indexing_result = tokio::task::spawn_blocking(move || {
                    let mut indexed_count = 0u32;
                    let mut skipped_count = 0u32;
                    for source in stale_catalog_sources {
                        let _ = store_idx.delete(&source);
                    }

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

                    // Index virtual server catalog entries (skills, etc.)
                    for (label, content) in &virtual_idx {
                        if let Err(e) = store_idx.index(label, content) {
                            tracing::warn!(
                                "Failed to index virtual catalog entry {}: {}",
                                label,
                                e
                            );
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
                            cm_state
                                .catalog_sources
                                .retain(|label, _| !label.starts_with("mcp/"));
                            cm_state
                                .activated_tools
                                .retain(|name| tools_catalog_bg.iter().any(|t| &t.name == name));
                            cm_state.activated_resources.retain(|name| {
                                resources_catalog_bg.iter().any(|r| &r.name == name)
                            });
                            cm_state
                                .activated_prompts
                                .retain(|name| prompts_catalog_bg.iter().any(|p| &p.name == name));
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

                            // Register virtual catalog sources (skills, etc.)
                            for (label, _) in &virtual_catalog_entries {
                                cm_state.catalog_sources.insert(
                                    label.clone(),
                                    super::context_mode::CatalogItemType::Skill,
                                );
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
            context_management_enabled: catalog_enabled,
            catalog_compression: None,
            virtual_instructions,
            search_tool_name: cm_search_tool_name,
            read_tool_name: cm_read_tool_name,
            item_definition_sizes,
        };

        // Catalog compression: compute deferral plan
        if catalog_enabled {
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

        instructions
    }

    /// Broadcast and return first successful response (for ping, logging/setLevel)
    async fn broadcast_and_return_first(
        &self,
        session: Arc<RwLock<GatewaySession>>,
        request: JsonRpcRequest,
    ) -> AppResult<JsonRpcResponse> {
        let (mut allowed_servers, transports) = {
            let session_read = session.read().await;
            (
                session_read.allowed_servers.clone(),
                session_read
                    .transports
                    .clone()
                    .ok_or_else(|| AppError::Mcp("Session not initialized".into()))?,
            )
        };

        // ping and logging/setLevel were removed in 2026-07-28: don't send
        // them to stateless backends (they would only answer
        // method_not_found and pollute their logs).
        if matches!(request.method.as_str(), "ping" | "logging/setLevel") {
            allowed_servers.retain(|server_id| !transports.revision(server_id).is_stateless());
            if allowed_servers.is_empty() {
                // Nothing legacy to forward to; acknowledge locally.
                return Ok(JsonRpcResponse::success(
                    request.id.unwrap_or(Value::Null),
                    json!({}),
                ));
            }
        }

        let timeout = Duration::from_secs(self.config.server_timeout_seconds);
        let max_retries = self.config.max_retry_attempts;

        let results = broadcast_request(
            &allowed_servers,
            request.clone(),
            &transports,
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
    pub async fn cleanup_expired_sessions(&self) {
        let mut to_remove = Vec::new();

        for entry in self.sessions.iter() {
            let session_key = entry.key().clone();
            let session = entry.value().clone();

            let is_expired = if let Ok(session_read) = session.try_read() {
                session_read.is_expired()
            } else {
                false
            };

            if is_expired {
                to_remove.push((session_key, session));
            }
        }

        for (session_key, session) in to_remove {
            self.sessions.remove(&session_key);
            self.stdio_bridge_sessions.remove(&session_key);
            self.push_test_sessions.remove(&session_key);
            close_session_transports(&session).await;
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

    /// Check all active sessions for client setting changes and notify clients.
    ///
    /// Compares each session's listing snapshot with the current client
    /// config. For each changed session, invalidates affected caches, stops
    /// routing to servers that lost access, and calls `notify`. Waits for
    /// busy sessions rather than skipping them.
    ///
    /// # Arguments
    /// * `clients` - Current client configs from the config manager
    /// * `all_enabled_server_ids` - All enabled MCP server IDs (for computing allowed servers)
    /// * `notify` - Callback called with (client_id, tools_changed, resources_changed, prompts_changed)
    pub async fn check_and_notify_permission_changes(
        &self,
        clients: &[lr_config::Client],
        all_enabled_server_ids: &[String],
        notify: impl Fn(&str, bool, bool, bool),
    ) {
        // Snapshot first: awaiting session locks while iterating would hold
        // DashMap shard locks across the await.
        let sessions: Vec<Arc<RwLock<GatewaySession>>> =
            self.sessions.iter().map(|e| e.value().clone()).collect();

        for session in sessions {
            let mut session_write = session.write().await;
            let client_id = session_write.client_id.clone();

            // Find matching client in config
            let Some(client) = clients.iter().find(|c| c.id == client_id) else {
                continue; // Client may have been deleted
            };

            // Sessions that never served a request have nothing to compare
            if session_write.listing_snapshot.is_none() {
                continue;
            }
            let changes = session_write
                .apply_listing_snapshot(super::session::ClientListingSnapshot::from_client(client));
            if !changes.any() {
                continue;
            }

            tracing::info!(
                "Settings change detected for client {}: {:?}",
                client_id,
                changes
            );

            // Stop routing to servers that lost access. Servers that gained
            // access have no transport in this session yet; the next request
            // carries the wider server list and rebuilds the session.
            let new_mcp = client.mcp_permissions.clone();
            let still_allowed = |sid: &String| {
                all_enabled_server_ids.contains(sid) && new_mcp.has_any_enabled_for_server(sid)
            };
            session_write.allowed_servers.retain(still_allowed);
            session_write.requested_servers.retain(still_allowed);

            session_write.mcp_sampling_permission = client.mcp_sampling_permission.clone();
            session_write.mcp_elicitation_permission = client.mcp_elicitation_permission.clone();
            drop(session_write);

            notify(
                &client_id,
                changes.tools,
                changes.resources,
                changes.prompts,
            );
        }
    }

    /// Rebuild each permitted session's skill search catalog before telling
    /// that client to re-list. Never hold a DashMap entry across awaits.
    pub async fn notify_skills_changed(&self) {
        let skills = self
            .virtual_servers
            .read()
            .iter()
            .find(|vs| vs.id() == "_skills")
            .cloned();
        let Some(skills) = skills else {
            return;
        };
        let sessions: Vec<_> = self
            .sessions
            .iter()
            .map(|entry| (entry.key().clone(), entry.value().clone()))
            .collect();
        for (key, session) in sessions {
            let mut session = session.write().await;
            if !session.skills_permissions.has_any_access() {
                continue;
            }
            let entries = session
                .virtual_server_state
                .get("_skills")
                .map(|state| skills.catalog_index_entries(state.as_ref()))
                .unwrap_or_default();
            session.cached_tools = None;
            if let Some(cm) = session
                .virtual_server_state
                .get_mut("_context_mode")
                .and_then(|state| {
                    state
                        .as_any_mut()
                        .downcast_mut::<super::context_mode::ContextModeSessionState>()
                })
            {
                let old: Vec<_> = cm
                    .catalog_sources
                    .keys()
                    .filter(|label| label.starts_with("catalog:skills/"))
                    .cloned()
                    .collect();
                for label in old {
                    cm.catalog_sources.remove(&label);
                    if let Err(error) = cm.store.delete(&label) {
                        tracing::warn!("Could not remove stale skill index entry: {error}");
                    }
                }
                if cm.catalog_compression_enabled {
                    for (label, content) in entries {
                        match cm.store.index(&label, &content) {
                            Ok(_) => {
                                cm.catalog_sources
                                    .insert(label, super::context_mode::CatalogItemType::Skill);
                            }
                            Err(error) => {
                                tracing::warn!("Could not refresh skill catalog index: {error}")
                            }
                        }
                    }
                }
            }
            drop(session);
            self.send_list_changed(
                &session_notification_key(&key),
                super::session::ListChanges {
                    tools: true,
                    resources: false,
                    prompts: false,
                },
            );
        }
    }

    /// Broadcast `list_changed` notifications for the changed lists.
    fn send_list_changed(&self, key: &str, changes: super::session::ListChanges) {
        let Some(broadcast) = &self.notification_broadcast else {
            return;
        };
        for (changed, method) in [
            (changes.tools, "notifications/tools/list_changed"),
            (changes.resources, "notifications/resources/list_changed"),
            (changes.prompts, "notifications/prompts/list_changed"),
        ] {
            if changed {
                let _ = broadcast.send((
                    key.to_string(),
                    JsonRpcNotification {
                        jsonrpc: "2.0".to_string(),
                        method: method.to_string(),
                        params: None,
                    },
                ));
            }
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
    /// The Monitor request id a session's MCP events are recorded under
    /// (`GatewaySession::monitor_session_id`), if it has one. Never waits: a
    /// session locked for writing right now reports none.
    pub fn monitor_session_id(&self, session_key: &str) -> Option<String> {
        self.get_session(session_key)?
            .try_read()
            .ok()?
            .monitor_session_id
            .clone()
    }

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
    /// Closes all per-session transports before removing.
    pub async fn terminate_session(&self, session_key: &str) -> Result<(), String> {
        self.stdio_bridge_sessions.remove(session_key);
        self.push_test_sessions.remove(session_key);
        if let Some((_, session)) = self.sessions.remove(session_key) {
            close_session_transports(&session).await;
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
        // Snapshot first: awaiting session locks while iterating would hold
        // DashMap shard locks across the await.
        let all_sessions: Vec<(String, Arc<RwLock<GatewaySession>>)> = self
            .sessions
            .iter()
            .map(|entry| (entry.key().clone(), entry.value().clone()))
            .collect();
        let mut keys_to_remove = Vec::new();
        for (key, session) in all_sessions {
            if session.read().await.client_id == client_id {
                keys_to_remove.push(key);
            }
        }

        let count = keys_to_remove.len();
        for key in &keys_to_remove {
            if let Some((_, session)) = self.sessions.remove(key) {
                close_session_transports(&session).await;
            }
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
                    super::context_mode::CatalogItemType::ServerWelcome
                    | super::context_mode::CatalogItemType::Skill => true,
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
        let (store, read_tool_name) = {
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
            (cm.store.clone(), cm.read_tool_name.clone())
        };

        // Run the blocking FTS5 search off the async executor
        let queries = vec![query.to_string()];
        let source_owned = source.map(|s| s.to_string());
        let result = tokio::task::spawn_blocking(move || {
            store.search(
                &queries,
                5,
                source_owned.as_deref(),
                &lr_context::DateRange::default(),
            )
        })
        .await
        .map_err(|e| format!("Search task panicked: {}", e))?;

        match result {
            Ok(results) => {
                let formatted = lr_context::format_search_results(
                    &results,
                    lr_context::SEARCH_OUTPUT_CAP,
                    &read_tool_name,
                );
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

        // 3. Create ephemeral transports for preview
        let start_timeout = tokio::time::Duration::from_secs(15);
        let (preview_transports, preview_failures) = self
            .server_manager
            .create_transports(&allowed_server_ids, start_timeout)
            .await;
        let preview_transports = Arc::new(preview_transports);
        let running_ids = preview_transports.running_server_ids();
        let unavailable: Vec<UnavailableServerInfo> = preview_failures
            .into_iter()
            .map(|f| {
                let name = self
                    .server_manager
                    .get_config(&f.server_id)
                    .map(|c| c.name.clone())
                    .unwrap_or_else(|| f.server_id.clone());
                UnavailableServerInfo {
                    name,
                    error: f.error,
                }
            })
            .collect();

        // Send initialize to running servers to get instructions/description
        let init_request = JsonRpcRequest::new(
            Some(json!("_preview_init")),
            "initialize".to_string(),
            Some(json!({
                "protocolVersion": crate::protocol::MCP_PROTOCOL_VERSION,
                "capabilities": {
                    "sampling": {},
                    "elicitation": { "form": {} },
                    "roots": { "listChanged": true }
                },
                "clientInfo": {
                    "name": "LocalRouter",
                    "version": env!("CARGO_PKG_VERSION")
                }
            })),
        );
        let timeout = Duration::from_secs(self.config.server_timeout_seconds);
        let init_results_raw = broadcast_request(
            &running_ids,
            init_request,
            &preview_transports,
            timeout,
            0, // no retries for preview
        )
        .await;
        let (init_successes, _) = separate_results(init_results_raw);
        let init_results: std::collections::HashMap<String, InitializeResult> = init_successes
            .into_iter()
            .filter_map(|(server_id, value)| {
                serde_json::from_value::<InitializeResult>(value)
                    .ok()
                    .map(|result| (server_id, result))
            })
            .collect();

        // Send notifications/initialized to complete MCP handshake for preview
        for server_id in init_results.keys() {
            let notification = JsonRpcRequest::new(
                None,
                "notifications/initialized".to_string(),
                Some(json!({})),
            );
            let _ = preview_transports
                .send_request(server_id, notification)
                .await;
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
                &preview_transports,
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
                &preview_transports,
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
                &preview_transports,
            )
            .await
            .map(|(p, _)| p)
            .unwrap_or_default();

        // Build server infos using init results for instructions/description
        let server_infos: Vec<McpServerInstructionInfo> = running_ids
            .iter()
            .map(|id| {
                let name = self
                    .server_manager
                    .get_config(id)
                    .map(|c| c.name.clone())
                    .unwrap_or_else(|| id.clone());
                let init = init_results.get(id);
                McpServerInstructionInfo {
                    name,
                    instructions: init.and_then(|r| r.instructions.clone()),
                    description: init.and_then(|r| r.server_info.description.clone()),
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

        let ctx = InstructionsContext {
            servers: server_infos,
            unavailable_servers: unavailable,
            context_management_enabled: false,
            catalog_compression: None,
            virtual_instructions: Vec::new(),
            ..Default::default()
        };

        // Close ephemeral preview transports — they are not needed after catalog fetch
        preview_transports.close_all().await;

        // Cache the context in a lightweight preview session so subsequent
        // calls (e.g. threshold slider changes) don't rebuild from scratch.
        let preview_session_id = format!("_preview_{}", client_id);
        let session = Arc::new(RwLock::new(GatewaySession::new(
            client_id.to_string(),
            running_ids,
            Duration::from_secs(300), // 5 min TTL for preview sessions
            0,
            Vec::new(),
        )));
        {
            let mut session_write = session.write().await;
            session_write.instructions_context = Some(ctx.clone());
        }
        self.sessions.insert(preview_session_id, session);

        Ok(ctx)
    }

    /// Fetch tool/resource/prompt catalogs from all enabled servers for preview detail display.
    ///
    /// Creates ephemeral transports, fetches data, then tears them down.
    pub async fn fetch_preview_catalogs(
        &self,
    ) -> (
        Vec<NamespacedTool>,
        Vec<NamespacedResource>,
        Vec<NamespacedPrompt>,
    ) {
        let server_ids: Vec<String> = self
            .server_manager
            .list_configs()
            .iter()
            .filter(|c| c.enabled)
            .map(|c| c.id.clone())
            .collect();

        if server_ids.is_empty() {
            return (Vec::new(), Vec::new(), Vec::new());
        }

        let start_timeout = tokio::time::Duration::from_secs(15);
        let (ephemeral_transports, _) = self
            .server_manager
            .create_transports(&server_ids, start_timeout)
            .await;
        let ephemeral_transports = Arc::new(ephemeral_transports);
        let running_ids = ephemeral_transports.running_server_ids();

        if running_ids.is_empty() {
            ephemeral_transports.close_all().await;
            return (Vec::new(), Vec::new(), Vec::new());
        }

        // Send initialize to get servers ready
        let init_request = JsonRpcRequest::new(
            Some(json!("_preview_catalog_init")),
            "initialize".to_string(),
            Some(json!({
                "protocolVersion": crate::protocol::MCP_PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": { "name": "LocalRouter", "version": env!("CARGO_PKG_VERSION") }
            })),
        );
        let timeout = Duration::from_secs(self.config.server_timeout_seconds);
        let _ = broadcast_request(
            &running_ids,
            init_request,
            &ephemeral_transports,
            timeout,
            0,
        )
        .await;

        // Complete MCP handshake: send notifications/initialized so servers
        // register conditional tools in their oninitialized callback
        for server_id in &running_ids {
            let notification = JsonRpcRequest::new(
                None,
                "notifications/initialized".to_string(),
                Some(json!({})),
            );
            let _ = ephemeral_transports
                .send_request(server_id, notification)
                .await;
        }

        let tools = self
            .fetch_and_merge_tools(
                &running_ids,
                JsonRpcRequest::new(
                    Some(json!("_preview_catalog_tools")),
                    "tools/list".to_string(),
                    None,
                ),
                &ephemeral_transports,
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
                &ephemeral_transports,
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
                &ephemeral_transports,
            )
            .await
            .map(|(p, _)| p)
            .unwrap_or_default();

        ephemeral_transports.close_all().await;
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

/// Close a removed session's backend transports (stdio processes, HTTP
/// streams). Waits briefly for the session lock instead of skipping a busy
/// session, which would leak its processes.
async fn close_session_transports(session: &Arc<RwLock<GatewaySession>>) {
    let transports = match tokio::time::timeout(Duration::from_secs(5), session.read()).await {
        Ok(session_read) => {
            if let Some(task) = &session_read.initialization_abort {
                task.abort();
            }
            session_read.transports.clone()
        }
        Err(_) => {
            tracing::warn!("Timed out waiting for session lock; closing transports skipped");
            None
        }
    };
    if let Some(transports) = transports {
        transports.close_all().await;
    }
}

/// The Push Relay's view of a session.
fn push_context(state: &GatewaySession) -> PushRelaySessionContext {
    PushRelaySessionContext {
        client_id: state.client_id.clone(),
        session_key: state.session_key.clone(),
        client_name: state.client_name.clone(),
        is_stdio_bridge: state.is_stdio_bridge,
        push_test_source: state.push_test_source,
        protocol_revision: state.protocol_revision,
        client_capabilities: state.client_capabilities_raw.clone(),
        visible_servers: state.requested_servers.clone(),
        running_servers: state.allowed_servers.clone(),
        notification_key: session_notification_key(&state.session_key),
    }
}

/// Snapshot one upstream's capabilities (and event catalogue) in the
/// background and remember them if they changed.
fn spawn_capability_observation(
    manager: Arc<McpServerManager>,
    recorder: Arc<SnapshotRecorder>,
    transports: Arc<SessionTransportSet>,
    server_id: String,
    init: InitializeResult,
) {
    tokio::spawn(async move {
        if let Some(snapshot) =
            super::push_relay::observe_upstream(&manager, &transports, &server_id, &init).await
        {
            if recorder.record(&manager, &server_id, snapshot) {
                tracing::info!(
                    "Remembered changed capabilities of MCP server {}",
                    server_id
                );
            }
        }
    });
}
