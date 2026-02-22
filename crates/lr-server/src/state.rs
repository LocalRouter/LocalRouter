//! Server state management
//!
//! Shared state for the web server including router, API key manager,
//! rate limiter, and generation tracking.

#![allow(dead_code)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use dashmap::DashMap;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use lr_clients::{ClientManager, TokenStore};
use lr_config::ConfigManager;
use lr_mcp::protocol::{JsonRpcNotification, JsonRpcRequest, JsonRpcResponse};
use lr_mcp::{McpGateway, McpServerManager};
use lr_monitoring::logger::AccessLogger;
use lr_monitoring::mcp_logger::McpAccessLogger;
use lr_monitoring::metrics::MetricsCollector;
use lr_providers::health_cache::HealthCacheManager;
use lr_providers::registry::ProviderRegistry;
use lr_router::{RateLimiterManager, Router};
use lr_types::TokenRecorder;

use super::types::{CostDetails, GenerationDetailsResponse, ProviderHealthSnapshot, TokenUsage};

/// Message types that can be sent through the SSE stream
#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SseMessage {
    /// JSON-RPC response to a request
    Response(JsonRpcResponse),
    /// JSON-RPC notification from server
    Notification(JsonRpcNotification),
    /// JSON-RPC request from server (sampling, elicitation, etc.)
    Request(JsonRpcRequest),
    /// Endpoint information (sent on SSE connection)
    Endpoint { endpoint: String },
}

/// Manages active SSE connections for MCP clients
///
/// Each client can have one active SSE connection. When a client sends
/// a POST request, the response is routed through their SSE connection.
pub struct SseConnectionManager {
    /// Map of client_id -> response sender
    /// Using unbounded channel to avoid blocking POST handlers
    connections: DashMap<String, tokio::sync::mpsc::UnboundedSender<SseMessage>>,
    /// Map of (client_id, request_id) -> response sender for server-initiated requests
    /// Used to match responses from clients to pending server requests
    pending_server_requests: DashMap<String, tokio::sync::oneshot::Sender<JsonRpcResponse>>,
    /// Tauri app handle for emitting connection events (optional)
    app_handle: RwLock<Option<tauri::AppHandle>>,
}

impl SseConnectionManager {
    pub fn new() -> Self {
        Self {
            connections: DashMap::new(),
            pending_server_requests: DashMap::new(),
            app_handle: RwLock::new(None),
        }
    }

    /// Set the Tauri app handle for event emission
    pub fn set_app_handle(&self, handle: tauri::AppHandle) {
        *self.app_handle.write() = Some(handle);
    }

    /// Emit an event if the app handle is available
    fn emit_event(&self, event: &str, payload: impl serde::Serialize + Clone) {
        if let Some(handle) = self.app_handle.read().as_ref() {
            use tauri::Emitter;
            let _ = handle.emit(event, payload);
        }
    }

    /// Get list of all active connection client IDs
    pub fn get_active_connections(&self) -> Vec<String> {
        self.connections.iter().map(|e| e.key().clone()).collect()
    }

    /// Register an SSE connection for a client
    /// Returns a receiver that the SSE handler should listen on
    pub fn register(&self, client_id: &str) -> tokio::sync::mpsc::UnboundedReceiver<SseMessage> {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();

        // If there's an existing connection, it will be replaced
        // The old sender will be dropped, causing the old SSE stream to end
        if let Some(old) = self.connections.insert(client_id.to_string(), tx) {
            tracing::info!("Replaced existing SSE connection for client {}", client_id);
            drop(old);
        }

        let active_connections: Vec<String> =
            self.connections.iter().map(|e| e.key().clone()).collect();
        tracing::info!(
            "Registered SSE connection for client {} (active_connections={:?})",
            client_id,
            active_connections
        );

        // Emit connection opened event
        self.emit_event("sse-connection-opened", client_id.to_string());

        rx
    }

    /// Unregister an SSE connection
    pub fn unregister(&self, client_id: &str) {
        if self.connections.remove(client_id).is_some() {
            tracing::debug!("Unregistered SSE connection for client {}", client_id);
            // Emit connection closed event
            self.emit_event("sse-connection-closed", client_id.to_string());
        }
    }

    /// Send a response to a client's SSE stream
    /// Returns true if the message was sent, false if no connection exists
    pub fn send_response(&self, client_id: &str, response: JsonRpcResponse) -> bool {
        let response_id = response.id.clone();
        let active_connections: Vec<String> =
            self.connections.iter().map(|e| e.key().clone()).collect();

        tracing::debug!(
            "SseConnectionManager::send_response: looking for client_id={}, active_connections={:?}",
            client_id,
            active_connections
        );

        if let Some(tx) = self.connections.get(client_id) {
            match tx.send(SseMessage::Response(response)) {
                Ok(_) => {
                    tracing::info!(
                        "Sent response to client {} via SSE channel (response_id={:?})",
                        client_id,
                        response_id
                    );
                    true
                }
                Err(e) => {
                    tracing::warn!(
                        "Failed to send response to client {} - channel closed: {} (response_id={:?})",
                        client_id,
                        e,
                        response_id
                    );
                    false
                }
            }
        } else {
            tracing::warn!(
                "No SSE connection for client {} - cannot send response (response_id={:?}, active_connections={:?})",
                client_id,
                response_id,
                active_connections
            );
            false
        }
    }

    /// Send a notification to a client's SSE stream
    pub fn send_notification(&self, client_id: &str, notification: JsonRpcNotification) -> bool {
        if let Some(tx) = self.connections.get(client_id) {
            tx.send(SseMessage::Notification(notification)).is_ok()
        } else {
            false
        }
    }

    /// Send endpoint information to a client (sent on initial connection)
    pub fn send_endpoint(&self, client_id: &str, endpoint: String) -> bool {
        if let Some(tx) = self.connections.get(client_id) {
            tx.send(SseMessage::Endpoint { endpoint }).is_ok()
        } else {
            false
        }
    }

    /// Send a server-initiated request to a client's SSE stream
    /// Returns a receiver that will receive the response when the client responds
    pub fn send_request(
        &self,
        client_id: &str,
        request: JsonRpcRequest,
    ) -> Option<tokio::sync::oneshot::Receiver<JsonRpcResponse>> {
        let request_id = request.id.clone().unwrap_or(serde_json::Value::Null);
        let key = format!("{}:{}", client_id, request_id);

        if let Some(tx) = self.connections.get(client_id) {
            // Create oneshot channel for the response
            let (response_tx, response_rx) = tokio::sync::oneshot::channel();

            // Store the pending request
            self.pending_server_requests
                .insert(key.clone(), response_tx);

            match tx.send(SseMessage::Request(request)) {
                Ok(_) => {
                    tracing::info!(
                        "Sent server-initiated request to client {} (request_id={:?})",
                        client_id,
                        request_id
                    );
                    Some(response_rx)
                }
                Err(e) => {
                    tracing::warn!(
                        "Failed to send request to client {} - channel closed: {}",
                        client_id,
                        e
                    );
                    // Clean up the pending request
                    self.pending_server_requests.remove(&key);
                    None
                }
            }
        } else {
            tracing::warn!(
                "No SSE connection for client {} - cannot send request",
                client_id
            );
            None
        }
    }

    /// Resolve a pending server-initiated request with a response from the client
    /// Returns true if the response was matched with a pending request
    pub fn resolve_server_request(&self, client_id: &str, response: JsonRpcResponse) -> bool {
        let key = format!("{}:{}", client_id, response.id);

        if let Some((_, tx)) = self.pending_server_requests.remove(&key) {
            match tx.send(response) {
                Ok(_) => {
                    tracing::info!(
                        "Resolved server-initiated request for client {} (request_id={})",
                        client_id,
                        key
                    );
                    true
                }
                Err(_) => {
                    tracing::warn!(
                        "Failed to resolve server request - receiver dropped (key={})",
                        key
                    );
                    false
                }
            }
        } else {
            tracing::debug!(
                "No pending server request for key {} - this might be a client-initiated response",
                key
            );
            false
        }
    }

    /// Check if a client has an active SSE connection
    pub fn has_connection(&self, client_id: &str) -> bool {
        self.connections.contains_key(client_id)
    }
}

impl Default for SseConnectionManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Tracks time-based guardrail bypasses per client
///
/// When a user clicks "Allow for 1 Hour" on a guardrail popup,
/// the bypass is stored here and checked before scanning.
#[derive(Clone, Default)]
pub struct GuardrailApprovalTracker {
    /// Map of client_id -> expiry_instant
    bypasses: Arc<DashMap<String, Instant>>,
}

impl GuardrailApprovalTracker {
    pub fn new() -> Self {
        Self {
            bypasses: Arc::new(DashMap::new()),
        }
    }

    /// Check if a client has a valid time-based guardrail bypass
    pub fn has_valid_bypass(&self, client_id: &str) -> bool {
        if let Some(entry) = self.bypasses.get(client_id) {
            if *entry > Instant::now() {
                return true;
            }
            // Expired, remove it
            drop(entry);
            self.bypasses.remove(client_id);
        }
        false
    }

    /// Add a time-based bypass for a client
    pub fn add_bypass(&self, client_id: &str, duration: Duration) {
        let expiry = Instant::now() + duration;
        self.bypasses.insert(client_id.to_string(), expiry);
        tracing::info!(
            "Added guardrail bypass: client={}, duration={}s",
            client_id,
            duration.as_secs(),
        );
    }

    /// Add a 1-hour bypass
    pub fn add_1_hour_bypass(&self, client_id: &str) {
        self.add_bypass(client_id, Duration::from_secs(3600));
    }

    /// Clean up expired bypasses
    pub fn cleanup_expired(&self) -> usize {
        let now = Instant::now();
        let expired: Vec<_> = self
            .bypasses
            .iter()
            .filter(|entry| *entry.value() <= now)
            .map(|entry| entry.key().clone())
            .collect();

        let count = expired.len();
        for key in expired {
            self.bypasses.remove(&key);
        }
        count
    }
}

/// Tracks time-based guardrail denial bypasses per client
///
/// When a user clicks "Deny All for 1 Hour" on a guardrail popup,
/// the denial is stored here and checked before scanning.
/// While active, flagged content is auto-denied without a popup.
#[derive(Clone, Default)]
pub struct GuardrailDenialTracker {
    /// Map of client_id -> expiry_instant
    denials: Arc<DashMap<String, Instant>>,
}

impl GuardrailDenialTracker {
    pub fn new() -> Self {
        Self {
            denials: Arc::new(DashMap::new()),
        }
    }

    /// Check if a client has a valid time-based guardrail denial
    pub fn has_valid_denial(&self, client_id: &str) -> bool {
        if let Some(entry) = self.denials.get(client_id) {
            if *entry > Instant::now() {
                return true;
            }
            // Expired, remove it
            drop(entry);
            self.denials.remove(client_id);
        }
        false
    }

    /// Add a time-based denial for a client
    pub fn add_denial(&self, client_id: &str, duration: Duration) {
        let expiry = Instant::now() + duration;
        self.denials.insert(client_id.to_string(), expiry);
        tracing::info!(
            "Added guardrail denial: client={}, duration={}s",
            client_id,
            duration.as_secs(),
        );
    }

    /// Add a 1-hour denial
    pub fn add_1_hour_denial(&self, client_id: &str) {
        self.add_denial(client_id, Duration::from_secs(3600));
    }

    /// Clean up expired denials
    pub fn cleanup_expired(&self) -> usize {
        let now = Instant::now();
        let expired: Vec<_> = self
            .denials
            .iter()
            .filter(|entry| *entry.value() <= now)
            .map(|entry| entry.key().clone())
            .collect();

        let count = expired.len();
        for key in expired {
            self.denials.remove(&key);
        }
        count
    }
}

/// Tracks time-based model approvals for the model firewall
///
/// When a user clicks "Allow for 1 Hour" on a model permission popup,
/// the approval is stored here and checked before triggering new popups.
#[derive(Clone, Default)]
pub struct ModelApprovalTracker {
    /// Map of (client_id, provider__model_id) -> expiry_instant
    approvals: Arc<DashMap<(String, String), Instant>>,
}

impl ModelApprovalTracker {
    pub fn new() -> Self {
        Self {
            approvals: Arc::new(DashMap::new()),
        }
    }

    /// Check if a model has a valid time-based approval
    pub fn has_valid_approval(&self, client_id: &str, provider: &str, model_id: &str) -> bool {
        let key = (client_id.to_string(), format!("{}__{}", provider, model_id));
        if let Some(entry) = self.approvals.get(&key) {
            if *entry > Instant::now() {
                return true;
            }
            // Expired, remove it
            drop(entry);
            self.approvals.remove(&key);
        }
        false
    }

    /// Add a time-based approval for a model (default: 1 hour)
    pub fn add_approval(
        &self,
        client_id: &str,
        provider: &str,
        model_id: &str,
        duration: Duration,
    ) {
        let key = (client_id.to_string(), format!("{}__{}", provider, model_id));
        let expiry = Instant::now() + duration;
        self.approvals.insert(key, expiry);
        tracing::info!(
            "Added model approval: client={}, model={}__{}",
            client_id,
            provider,
            model_id,
        );
    }

    /// Add a 1-hour approval
    pub fn add_1_hour_approval(&self, client_id: &str, provider: &str, model_id: &str) {
        self.add_approval(client_id, provider, model_id, Duration::from_secs(3600));
    }

    /// Clean up expired approvals
    pub fn cleanup_expired(&self) -> usize {
        let now = Instant::now();
        let expired: Vec<_> = self
            .approvals
            .iter()
            .filter(|entry| *entry.value() <= now)
            .map(|entry| entry.key().clone())
            .collect();

        let count = expired.len();
        for key in expired {
            self.approvals.remove(&key);
        }
        count
    }
}

/// Server state shared across all handlers
#[derive(Clone)]
pub struct AppState {
    /// Router for intelligent model selection and routing
    pub router: Arc<Router>,

    /// Unified client manager for authentication (replaces api_key_manager and oauth_client_manager)
    pub client_manager: Arc<ClientManager>,

    /// OAuth token store for short-lived access tokens
    pub token_store: Arc<TokenStore>,

    /// MCP server manager
    pub mcp_server_manager: Arc<McpServerManager>,

    /// MCP unified gateway
    pub mcp_gateway: Arc<McpGateway>,

    /// Rate limiter manager
    pub rate_limiter: Arc<RateLimiterManager>,

    /// Provider registry for listing models
    pub provider_registry: Arc<ProviderRegistry>,

    /// Configuration manager for accessing client routing configs
    pub config_manager: Arc<ConfigManager>,

    /// Generation tracking for /v1/generation endpoint
    pub generation_tracker: Arc<GenerationTracker>,

    /// Metrics collector for tracking usage
    pub metrics_collector: Arc<MetricsCollector>,

    /// Access logger for persistent request logging
    pub access_logger: Arc<AccessLogger>,

    /// MCP access logger for persistent MCP request logging
    pub mcp_access_logger: Arc<McpAccessLogger>,

    /// Tauri app handle for emitting events (set after initialization)
    pub app_handle: Arc<RwLock<Option<tauri::AppHandle>>>,

    /// Transient secret for internal UI testing (never persisted, regenerated on startup)
    /// Used to allow the Tauri frontend to bypass API key restrictions when testing models
    pub internal_test_secret: Arc<String>,

    /// RouteLLM intelligent routing service
    pub routellm_service: Option<Arc<lr_routellm::RouteLLMService>>,

    /// Tray graph manager for real-time token visualization (optional, only in UI mode)
    /// Behind RwLock to allow setting it after AppState creation during Tauri setup
    pub tray_graph_manager: Arc<RwLock<Option<Arc<dyn TokenRecorder>>>>,

    /// Broadcast channel for MCP server notifications
    /// Allows multiple clients to subscribe to real-time notifications from MCP servers
    /// Format: (server_id, notification)
    pub mcp_notification_broadcast:
        Arc<tokio::sync::broadcast::Sender<(String, JsonRpcNotification)>>,

    /// Broadcast channel for per-client permission change notifications
    /// Used to notify connected MCP clients when their permissions change
    /// Format: (client_id, notification)
    pub client_notification_broadcast:
        Arc<tokio::sync::broadcast::Sender<(String, JsonRpcNotification)>>,

    /// SSE connection manager for MCP HTTP+SSE transport
    /// Tracks active SSE connections and routes responses to the correct stream
    pub sse_connection_manager: Arc<SseConnectionManager>,

    /// Track which MCP servers have notification handlers registered (to prevent duplicates)
    pub mcp_notification_handlers_registered: Arc<DashMap<String, bool>>,

    /// Centralized health cache for providers and MCP servers
    pub health_cache: Arc<HealthCacheManager>,

    /// Time-based model approval tracker for model firewall
    pub model_approval_tracker: Arc<ModelApprovalTracker>,

    /// Time-based guardrail bypass tracker (allow for 1 hour)
    pub guardrail_approval_tracker: Arc<GuardrailApprovalTracker>,

    /// Time-based guardrail denial tracker (deny for 1 hour)
    pub guardrail_denial_tracker: Arc<GuardrailDenialTracker>,

    /// Safety engine for LLM-based content inspection (swappable at runtime)
    pub safety_engine: Arc<RwLock<Option<Arc<lr_guardrails::SafetyEngine>>>>,
}

impl AppState {
    pub fn new(
        router: Arc<Router>,
        rate_limiter: Arc<RateLimiterManager>,
        provider_registry: Arc<ProviderRegistry>,
        config_manager: Arc<ConfigManager>,
        client_manager: Arc<ClientManager>,
        token_store: Arc<TokenStore>,
        metrics_collector: Arc<MetricsCollector>,
    ) -> Self {
        // Generate a random bearer token for internal UI testing
        // Format: lr-internal-<uuid> to match standard API key format
        // This is regenerated on every app start and never persisted
        let internal_test_secret = format!("lr-internal-{}", Uuid::new_v4().simple());
        tracing::info!("Generated transient internal test bearer token for UI model testing");

        // Get logging config (retention and enabled status)
        let logging_config = &config_manager.get().logging;
        let retention_days = logging_config.retention_days;
        let access_log_enabled = logging_config.enable_access_log;

        // Initialize access logger with configured retention and enabled status
        let access_logger =
            AccessLogger::new(retention_days, access_log_enabled).unwrap_or_else(|e| {
                tracing::error!("Failed to initialize access logger: {}", e);
                panic!("Access logger initialization failed");
            });

        // Initialize MCP access logger with configured retention and enabled status
        let mcp_access_logger = McpAccessLogger::new(retention_days, access_log_enabled)
            .unwrap_or_else(|e| {
                tracing::error!("Failed to initialize MCP access logger: {}", e);
                panic!("MCP access logger initialization failed");
            });

        // Create broadcast channel for MCP notifications
        // Capacity of 1000 messages - old messages dropped if no subscribers are reading fast enough
        let (notification_tx, _rx) = tokio::sync::broadcast::channel(1000);

        // Create broadcast channel for per-client permission change notifications
        let (client_notification_tx, _) = tokio::sync::broadcast::channel(100);

        // Create placeholder MCP manager and gateway (will be replaced by with_mcp)
        let mcp_server_manager = Arc::new(McpServerManager::new());
        let mcp_gateway = Arc::new(McpGateway::new(
            mcp_server_manager.clone(),
            lr_mcp::gateway::GatewayConfig::default(),
            router.clone(),
        ));

        Self {
            router,
            client_manager,
            token_store,
            mcp_server_manager,
            mcp_gateway,
            rate_limiter,
            provider_registry,
            config_manager,
            generation_tracker: Arc::new(GenerationTracker::new()),
            metrics_collector,
            access_logger: Arc::new(access_logger),
            mcp_access_logger: Arc::new(mcp_access_logger),
            app_handle: Arc::new(RwLock::new(None)),
            internal_test_secret: Arc::new(internal_test_secret),
            routellm_service: None,
            tray_graph_manager: Arc::new(RwLock::new(None)),
            mcp_notification_broadcast: Arc::new(notification_tx),
            client_notification_broadcast: Arc::new(client_notification_tx),
            sse_connection_manager: Arc::new(SseConnectionManager::new()),
            mcp_notification_handlers_registered: Arc::new(DashMap::new()),
            health_cache: Arc::new(HealthCacheManager::new()),
            model_approval_tracker: Arc::new(ModelApprovalTracker::new()),
            guardrail_approval_tracker: Arc::new(GuardrailApprovalTracker::new()),
            guardrail_denial_tracker: Arc::new(GuardrailDenialTracker::new()),
            safety_engine: Arc::new(RwLock::new(None)),
        }
    }

    /// Set the safety engine
    pub fn with_safety_engine(self, engine: Arc<lr_guardrails::SafetyEngine>) -> Self {
        *self.safety_engine.write() = Some(engine);
        self
    }

    /// Replace the safety engine at runtime (e.g. after downloading models)
    pub fn replace_safety_engine(&self, engine: Arc<lr_guardrails::SafetyEngine>) {
        *self.safety_engine.write() = Some(engine);
    }

    /// Add MCP manager to the state
    pub fn with_mcp(self, mcp_server_manager: Arc<McpServerManager>) -> Self {
        // Create gateway with the actual MCP server manager and notification broadcast
        let mcp_gateway = Arc::new(McpGateway::new_with_broadcast(
            mcp_server_manager.clone(),
            lr_mcp::gateway::GatewayConfig::default(),
            self.router.clone(),
            Some(self.mcp_notification_broadcast.clone()),
        ));

        Self {
            mcp_server_manager,
            mcp_gateway,
            ..self
        }
    }

    /// Get the internal test secret for UI testing
    /// Only accessible via Tauri IPC, not exposed over HTTP
    pub fn get_internal_test_secret(&self) -> String {
        (*self.internal_test_secret).clone()
    }

    /// Set the Tauri app handle (called after Tauri initialization)
    pub fn set_app_handle(&self, handle: tauri::AppHandle) {
        *self.app_handle.write() = Some(handle.clone());

        // Also set app handle on loggers for event emission
        self.access_logger.set_app_handle(handle.clone());
        self.mcp_access_logger.set_app_handle(handle.clone());
        // Also set app handle on health cache for event emission
        self.health_cache.set_app_handle(handle.clone());
        // Also set app handle on SSE connection manager for connection events
        self.sse_connection_manager.set_app_handle(handle);
    }

    /// Set the tray graph manager (called after Tauri initialization when it's created)
    pub fn set_tray_graph_manager(&self, manager: Arc<dyn TokenRecorder>) {
        *self.tray_graph_manager.write() = Some(manager);
    }

    /// Initialize RouteLLM service with settings from config
    pub fn with_routellm(
        mut self,
        routellm_service: Option<Arc<lr_routellm::RouteLLMService>>,
    ) -> Self {
        self.routellm_service = routellm_service;
        self
    }

    /// Emit an event if the app handle is available
    pub fn emit_event(&self, event: &str, payload: &str) {
        if let Some(handle) = self.app_handle.read().as_ref() {
            use tauri::Emitter;
            let _ = handle.emit(event, payload);
        }
    }

    /// Record client activity (HTTP request, SSE connection, etc.)
    /// Emits a "client-activity" event with the client ID
    pub fn record_client_activity(&self, client_id: &str) {
        if let Some(handle) = self.app_handle.read().as_ref() {
            use tauri::Emitter;
            tracing::debug!("Emitting client-activity event for client: {}", client_id);
            let _ = handle.emit("client-activity", client_id.to_string());
        } else {
            tracing::warn!("Cannot emit client-activity: app_handle not set");
        }
    }
}

/// Tracks generation details for the /v1/generation endpoint
pub struct GenerationTracker {
    /// Map of generation ID to generation details
    generations: DashMap<String, GenerationDetails>,

    /// Retention period in seconds (default: 7 days)
    retention_period_secs: i64,
}

/// Aggregate statistics across all tracked generations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AggregateStats {
    pub total_requests: u64,
    pub total_tokens: u64,
    pub total_cost: f64,
    pub successful_requests: u64,
}

impl GenerationTracker {
    pub fn new() -> Self {
        Self {
            generations: DashMap::new(),
            retention_period_secs: 7 * 24 * 60 * 60, // 7 days
        }
    }

    /// Record a new generation
    pub fn record(&self, id: String, details: GenerationDetails) {
        self.generations.insert(id, details);

        // Clean up old generations (simple approach)
        self.cleanup();
    }

    /// Get generation details by ID
    pub fn get(&self, id: &str) -> Option<GenerationDetailsResponse> {
        self.generations.get(id).map(|entry| entry.to_response())
    }

    /// Get aggregate statistics
    pub fn get_stats(&self) -> AggregateStats {
        let mut total_requests = 0u64;
        let mut total_tokens = 0u64;
        let mut total_cost = 0.0f64;

        for entry in self.generations.iter() {
            let details = entry.value();
            total_requests += 1;
            total_tokens += details.tokens.total_tokens as u64;
            if let Some(cost) = &details.cost {
                total_cost += cost.total_cost;
            }
        }

        AggregateStats {
            total_requests,
            successful_requests: total_requests, // GenerationTracker only tracks successful requests
            total_tokens,
            total_cost,
        }
    }

    /// Remove expired generations
    fn cleanup(&self) {
        let now = Utc::now();
        let cutoff = now.timestamp() - self.retention_period_secs;

        self.generations
            .retain(|_, details| details.created_at.timestamp() > cutoff);
    }
}

impl Default for GenerationTracker {
    fn default() -> Self {
        Self::new()
    }
}

/// Internal generation details
pub struct GenerationDetails {
    pub id: String,
    pub model: String,
    pub provider: String,
    pub created_at: DateTime<Utc>,
    pub finish_reason: String,
    pub tokens: TokenUsage,
    pub cost: Option<CostDetails>,
    pub started_at: Instant,
    pub completed_at: Instant,
    pub provider_health: Option<ProviderHealthSnapshot>,
    pub api_key_id: String,
    pub user: Option<String>,
    pub stream: bool,
}

impl GenerationDetails {
    pub fn to_response(&self) -> GenerationDetailsResponse {
        let latency_ms = self
            .completed_at
            .duration_since(self.started_at)
            .as_millis() as u64;

        GenerationDetailsResponse {
            id: self.id.clone(),
            model: self.model.clone(),
            provider: self.provider.clone(),
            created: self.created_at.timestamp(),
            finish_reason: self.finish_reason.clone(),
            tokens: self.tokens.clone(),
            cost: self.cost.clone(),
            latency_ms,
            provider_health: self.provider_health.clone(),
            api_key_id: mask_api_key(&self.api_key_id),
            user: self.user.clone(),
            stream: self.stream,
        }
    }
}

/// Mask API key for display (show first 3 and last 3 chars)
fn mask_api_key(key: &str) -> String {
    if key.len() <= 6 {
        return "*".repeat(key.len());
    }

    let prefix = &key[..3];
    let suffix = &key[key.len() - 3..];
    format!("{}***{}", prefix, suffix)
}

/// Authenticated request context
/// This is attached to requests after authentication middleware
#[derive(Clone)]
pub struct AuthContext {
    pub api_key_id: String,
    pub model_selection: Option<ModelSelection>, // Legacy, kept for backward compatibility
}

/// OAuth authenticated request context for MCP proxy
/// This is attached to requests after OAuth authentication middleware
#[derive(Clone)]
pub struct OAuthContext {
    /// OAuth client ID
    pub client_id: String,
    /// MCP servers this client can access
    pub linked_server_ids: Vec<String>,
}

/// Model selection mode for an API key
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ModelSelection {
    /// All models from all providers (including future models)
    All,

    /// Custom selection of providers and/or individual models
    Custom {
        /// If true, all models are allowed (including future models)
        #[serde(default)]
        selected_all: bool,
        /// Providers where ALL models are selected (including future models)
        selected_providers: Vec<String>,
        /// Individual models selected as (provider, model) pairs
        selected_models: Vec<(String, String)>,
    },

    /// Legacy: Direct model selection (deprecated, use Custom instead)
    #[deprecated(note = "Use ModelSelection::Custom instead")]
    DirectModel { provider: String, model: String },

    /// Legacy: Router-based selection (deprecated)
    #[deprecated(note = "Router-based selection is deprecated")]
    Router {
        #[allow(dead_code)]
        router_name: String,
    },
}

impl ModelSelection {
    /// Check if a model is allowed by this selection
    pub fn is_model_allowed(&self, provider_name: &str, model_id: &str) -> bool {
        match self {
            ModelSelection::All => true,
            ModelSelection::Custom {
                selected_all,
                selected_providers,
                selected_models,
            } => {
                // If all are selected, everything is allowed
                if *selected_all {
                    return true;
                }

                // Check if the provider is in the selected_providers list
                if selected_providers
                    .iter()
                    .any(|p| p.eq_ignore_ascii_case(provider_name))
                {
                    return true;
                }

                // Check if the specific (provider, model) pair is in selected_models
                selected_models.iter().any(|(p, m)| {
                    p.eq_ignore_ascii_case(provider_name) && m.eq_ignore_ascii_case(model_id)
                })
            }
            #[allow(deprecated)]
            ModelSelection::DirectModel { provider, model } => {
                provider.eq_ignore_ascii_case(provider_name) && model.eq_ignore_ascii_case(model_id)
            }
            #[allow(deprecated)]
            ModelSelection::Router { .. } => {
                // Router-based selection is deprecated
                // For now, allow all models (will be handled by router logic)
                true
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mask_api_key() {
        assert_eq!(mask_api_key("sk-1234567890"), "sk-***890");
        assert_eq!(mask_api_key("lr-abc123def456"), "lr-***456");
        assert_eq!(mask_api_key("short"), "*****");
    }

    #[test]
    fn test_generation_tracker() {
        let tracker = GenerationTracker::new();

        let details = GenerationDetails {
            id: "gen-123".to_string(),
            model: "gpt-4".to_string(),
            provider: "openai".to_string(),
            created_at: Utc::now(),
            finish_reason: "stop".to_string(),
            tokens: TokenUsage {
                prompt_tokens: 10,
                completion_tokens: 20,
                total_tokens: 30,
                prompt_tokens_details: None,
                completion_tokens_details: None,
            },
            cost: Some(CostDetails {
                prompt_cost: 0.0001,
                completion_cost: 0.0002,
                total_cost: 0.0003,
                currency: "USD".to_string(),
            }),
            started_at: Instant::now(),
            completed_at: Instant::now(),
            provider_health: None,
            api_key_id: "lr-test123".to_string(),
            user: None,
            stream: false,
        };

        tracker.record("gen-123".to_string(), details);

        let result = tracker.get("gen-123");
        assert!(result.is_some());

        let response = result.unwrap();
        assert_eq!(response.id, "gen-123");
        assert_eq!(response.model, "gpt-4");
        assert_eq!(response.api_key_id, "lr-***123");
    }

    #[test]
    fn test_guardrail_approval_tracker_new() {
        let tracker = GuardrailApprovalTracker::new();
        assert!(!tracker.has_valid_bypass("client-1"));
    }

    #[test]
    fn test_guardrail_approval_tracker_1_hour_bypass() {
        let tracker = GuardrailApprovalTracker::new();

        tracker.add_1_hour_bypass("client-1");
        assert!(tracker.has_valid_bypass("client-1"));
        assert!(!tracker.has_valid_bypass("client-2"));
    }

    #[test]
    fn test_guardrail_approval_tracker_custom_duration() {
        let tracker = GuardrailApprovalTracker::new();

        tracker.add_bypass("client-1", Duration::from_secs(60));
        assert!(tracker.has_valid_bypass("client-1"));
    }

    #[test]
    fn test_guardrail_approval_tracker_expired_bypass() {
        let tracker = GuardrailApprovalTracker::new();

        // Add a bypass that expires immediately
        tracker.add_bypass("client-1", Duration::from_secs(0));
        std::thread::sleep(Duration::from_millis(10));
        assert!(!tracker.has_valid_bypass("client-1"));
    }

    #[test]
    fn test_guardrail_approval_tracker_cleanup() {
        let tracker = GuardrailApprovalTracker::new();

        tracker.add_bypass("client-1", Duration::from_secs(0));
        tracker.add_1_hour_bypass("client-2");
        std::thread::sleep(Duration::from_millis(10));

        let cleaned = tracker.cleanup_expired();
        assert_eq!(cleaned, 1); // client-1 expired

        assert!(!tracker.has_valid_bypass("client-1"));
        assert!(tracker.has_valid_bypass("client-2"));
    }

    #[test]
    fn test_guardrail_denial_tracker_new() {
        let tracker = GuardrailDenialTracker::new();
        assert!(!tracker.has_valid_denial("client-1"));
    }

    #[test]
    fn test_guardrail_denial_tracker_1_hour_denial() {
        let tracker = GuardrailDenialTracker::new();

        tracker.add_1_hour_denial("client-1");
        assert!(tracker.has_valid_denial("client-1"));
        assert!(!tracker.has_valid_denial("client-2"));
    }

    #[test]
    fn test_guardrail_denial_tracker_custom_duration() {
        let tracker = GuardrailDenialTracker::new();

        tracker.add_denial("client-1", Duration::from_secs(120));
        assert!(tracker.has_valid_denial("client-1"));
    }

    #[test]
    fn test_guardrail_denial_tracker_expired_denial() {
        let tracker = GuardrailDenialTracker::new();

        // Add a denial that expires immediately
        tracker.add_denial("client-1", Duration::from_secs(0));
        std::thread::sleep(Duration::from_millis(10));
        assert!(!tracker.has_valid_denial("client-1"));
    }

    #[test]
    fn test_guardrail_denial_tracker_cleanup() {
        let tracker = GuardrailDenialTracker::new();

        tracker.add_denial("client-1", Duration::from_secs(0));
        tracker.add_1_hour_denial("client-2");
        std::thread::sleep(Duration::from_millis(10));

        let cleaned = tracker.cleanup_expired();
        assert_eq!(cleaned, 1); // client-1 expired

        assert!(!tracker.has_valid_denial("client-1"));
        assert!(tracker.has_valid_denial("client-2"));
    }

    #[test]
    fn test_guardrail_denial_tracker_multiple_clients() {
        let tracker = GuardrailDenialTracker::new();

        tracker.add_1_hour_denial("client-1");
        tracker.add_1_hour_denial("client-2");
        tracker.add_1_hour_denial("client-3");

        assert!(tracker.has_valid_denial("client-1"));
        assert!(tracker.has_valid_denial("client-2"));
        assert!(tracker.has_valid_denial("client-3"));
        assert!(!tracker.has_valid_denial("client-4"));
    }

    #[test]
    fn test_guardrail_denial_tracker_overwrite() {
        let tracker = GuardrailDenialTracker::new();

        // Add a short denial, then overwrite with a longer one
        tracker.add_denial("client-1", Duration::from_secs(0));
        std::thread::sleep(Duration::from_millis(10));

        // Should be expired now
        assert!(!tracker.has_valid_denial("client-1"));

        // Overwrite with a longer one
        tracker.add_1_hour_denial("client-1");
        assert!(tracker.has_valid_denial("client-1"));
    }

    #[test]
    fn test_guardrail_trackers_independent() {
        // Approval and denial trackers should be independent
        let approval_tracker = GuardrailApprovalTracker::new();
        let denial_tracker = GuardrailDenialTracker::new();

        approval_tracker.add_1_hour_bypass("client-1");
        denial_tracker.add_1_hour_denial("client-2");

        // Approval tracker only knows about client-1
        assert!(approval_tracker.has_valid_bypass("client-1"));
        assert!(!approval_tracker.has_valid_bypass("client-2"));

        // Denial tracker only knows about client-2
        assert!(!denial_tracker.has_valid_denial("client-1"));
        assert!(denial_tracker.has_valid_denial("client-2"));
    }

    #[test]
    fn test_model_approval_tracker() {
        let tracker = ModelApprovalTracker::new();

        assert!(!tracker.has_valid_approval("client-1", "openai", "gpt-4"));

        tracker.add_1_hour_approval("client-1", "openai", "gpt-4");

        assert!(tracker.has_valid_approval("client-1", "openai", "gpt-4"));
        assert!(!tracker.has_valid_approval("client-1", "openai", "gpt-3.5"));
        assert!(!tracker.has_valid_approval("client-2", "openai", "gpt-4"));
    }
}
