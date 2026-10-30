//! MCP server lifecycle management
//!
//! Manages MCP server instances, their lifecycle, and health checks.

#![allow(dead_code)]

use crate::oauth::McpOAuthManager;
use crate::protocol::{JsonRpcNotification, JsonRpcRequest, JsonRpcResponse, StreamingChunk};
use crate::transport::{SseTransport, StdioTransport, Transport, WebSocketTransport};
use dashmap::DashMap;
use futures_util::stream::Stream;
use lr_api_keys::keychain_trait::KeychainStorage;
use lr_config::{McpServerConfig, McpTransportConfig, McpTransportType};
use lr_types::{AppError, AppResult};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;

/// Notification callback type
///
/// Called when a notification is received from an MCP server.
/// Receives the server_id and the notification.
pub type NotificationCallback = Arc<dyn Fn(String, JsonRpcNotification) + Send + Sync>;

/// Callback type for emitting monitor events from the MCP server manager.
/// Returns the assigned event ID.
pub type MonitorEmitFn = Arc<
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
>;

/// Callback type for updating an existing monitor event.
pub type MonitorUpdateFn =
    Arc<dyn Fn(&str, Box<dyn FnOnce(&mut lr_monitor::MonitorEvent) + Send>) + Send + Sync>;

/// Get a clone of the cached shell environment (PATH resolution for subprocess spawning).
///
/// Re-exported from [`lr_utils::binary`], which owns the login-shell PATH
/// probe. It lives there rather than here so `lr-coding-agents` can use it
/// too — this crate depends on that one, so the helper cannot live here
/// without creating a dependency cycle.
pub fn shell_env() -> HashMap<String, String> {
    lr_utils::binary::shell_env()
}

/// Notification handler with unique ID for removal
#[derive(Clone)]
pub struct NotificationHandler {
    /// Unique handler ID
    pub id: u64,
    /// The callback function
    pub callback: NotificationCallback,
}

/// Handler ID for tracking and removal
pub type HandlerId = u64;

/// MCP server manager
///
/// Manages the lifecycle of MCP server instances.
/// Supports STDIO and HTTP-SSE transports.
/// Handles OAuth authentication for servers that require it.
#[derive(Clone)]
pub struct McpServerManager {
    /// Active STDIO transports (server_id -> transport)
    stdio_transports: Arc<DashMap<String, Arc<StdioTransport>>>,

    /// Active SSE transports (server_id -> transport)
    sse_transports: Arc<DashMap<String, Arc<SseTransport>>>,

    /// Active WebSocket transports (server_id -> transport)
    websocket_transports: Arc<DashMap<String, Arc<WebSocketTransport>>>,

    /// Server configurations (server_id -> config)
    configs: Arc<DashMap<String, McpServerConfig>>,

    /// OAuth manager for MCP servers
    oauth_manager: Arc<McpOAuthManager>,

    /// Notification handlers (server_id -> list of handlers with IDs)
    notification_handlers: Arc<DashMap<String, Vec<NotificationHandler>>>,

    /// Next handler ID (atomic counter)
    next_handler_id: Arc<std::sync::atomic::AtomicU64>,

    /// Weak references follow client session lifetime; a separate successful
    /// probe cannot erase a failure on a client's still-broken connection.
    session_failures: Arc<
        DashMap<String, std::sync::Weak<crate::transport::session_transport_set::SessionFailures>>,
    >,
    health_cache:
        Arc<parking_lot::RwLock<Option<Arc<lr_providers::health_cache::HealthCacheManager>>>>,

    /// Optional monitor event callback
    monitor_emit: Arc<parking_lot::RwLock<Option<MonitorEmitFn>>>,
}

/// Health status for an MCP server
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerHealth {
    /// Server ID
    pub server_id: String,

    /// Server name
    pub server_name: String,

    /// Health status
    pub status: HealthStatus,

    /// Latency in milliseconds (for running servers)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<u64>,

    /// Error message (if unhealthy)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,

    /// Last health check timestamp
    pub last_check: chrono::DateTime<chrono::Utc>,
}

/// Health status enum
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum HealthStatus {
    /// Server is running and responding
    Healthy,
    /// An isolated authenticated MCP handshake/discovery succeeded; no global transport is running.
    Ready,
    /// Server is unhealthy or cannot start
    Unhealthy,
    /// Status unknown
    Unknown,
}

impl McpServerManager {
    /// Create a new MCP server manager
    pub fn new() -> Self {
        Self::new_with_oauth_manager(Arc::new(McpOAuthManager::new()))
    }

    /// Login, health probes and client transports share credential caches and
    /// refresh locks, including providers with rotating refresh tokens.
    pub fn new_with_oauth_manager(oauth_manager: Arc<McpOAuthManager>) -> Self {
        Self {
            stdio_transports: Arc::new(DashMap::new()),
            sse_transports: Arc::new(DashMap::new()),
            websocket_transports: Arc::new(DashMap::new()),
            configs: Arc::new(DashMap::new()),
            oauth_manager,
            notification_handlers: Arc::new(DashMap::new()),
            next_handler_id: Arc::new(std::sync::atomic::AtomicU64::new(1)),
            monitor_emit: Arc::new(parking_lot::RwLock::new(None)),
            session_failures: Arc::new(DashMap::new()),
            health_cache: Arc::new(parking_lot::RwLock::new(None)),
        }
    }

    /// Create a new MCP server manager for testing
    ///
    /// Uses in-memory storage (MockKeychain) instead of file/system keychain.
    /// This ensures tests are idempotent and don't depend on disk state.
    pub fn new_for_test() -> Self {
        let mock_keychain = lr_api_keys::MockKeychain::new();
        let cached_keychain = lr_api_keys::CachedKeychain::new(std::sync::Arc::new(mock_keychain));
        let oauth_manager = McpOAuthManager::new_with_keychain(cached_keychain);

        Self {
            stdio_transports: Arc::new(DashMap::new()),
            sse_transports: Arc::new(DashMap::new()),
            websocket_transports: Arc::new(DashMap::new()),
            configs: Arc::new(DashMap::new()),
            oauth_manager: Arc::new(oauth_manager),
            notification_handlers: Arc::new(DashMap::new()),
            next_handler_id: Arc::new(std::sync::atomic::AtomicU64::new(1)),
            monitor_emit: Arc::new(parking_lot::RwLock::new(None)),
            session_failures: Arc::new(DashMap::new()),
            health_cache: Arc::new(parking_lot::RwLock::new(None)),
        }
    }

    pub fn set_health_cache(&self, cache: Arc<lr_providers::health_cache::HealthCacheManager>) {
        *self.health_cache.write() = Some(cache);
    }

    pub(crate) fn report_startup_phase(
        &self,
        server_id: &str,
        phase: &str,
        elapsed_ms: u64,
        succeeded: bool,
    ) {
        let name = self
            .get_config(server_id)
            .map(|c| c.name)
            .unwrap_or_else(|| server_id.to_string());
        let outcome = if succeeded { "completed" } else { "failed" };
        self.emit_monitor_event(
            lr_monitor::MonitorEventType::McpServerEvent,
            None,
            None,
            None,
            lr_monitor::MonitorEventData::McpServerEvent {
                server_id: server_id.into(),
                server_name: Some(name.clone()),
                action: format!("{phase}_{outcome}"),
                message: format!("{name}: MCP {phase} {outcome} after {elapsed_ms}ms"),
            },
            if succeeded {
                lr_monitor::EventStatus::Complete
            } else {
                lr_monitor::EventStatus::Error
            },
            Some(elapsed_ms),
        );
    }

    /// Set the monitor event callback.
    pub fn set_monitor_emit(&self, emit: MonitorEmitFn) {
        *self.monitor_emit.write() = Some(emit);
    }

    /// Emit a monitor event if the callback is set. Returns the event ID.
    fn emit_monitor_event(
        &self,
        event_type: lr_monitor::MonitorEventType,
        client_id: Option<String>,
        client_name: Option<String>,
        session_id: Option<String>,
        data: lr_monitor::MonitorEventData,
        status: lr_monitor::EventStatus,
        duration_ms: Option<u64>,
    ) -> String {
        if let Some(cb) = self.monitor_emit.read().as_ref() {
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
            String::new()
        }
    }

    /// Set a request callback for a STDIO server
    ///
    /// This allows the server to send requests (like sampling/createMessage) to the client.
    /// The callback should process the request and return a response.
    ///
    /// # Arguments
    /// * `server_id` - The server ID to set the callback for
    /// * `callback` - The callback to invoke when requests are received from the server
    ///
    /// # Returns
    /// * `true` if the callback was set, `false` if the server is not a STDIO server
    pub fn set_request_callback(
        &self,
        server_id: &str,
        callback: crate::transport::StdioRequestCallback,
    ) -> bool {
        if let Some(transport) = self.stdio_transports.get(server_id) {
            transport.set_request_callback(callback);
            tracing::info!("Set request callback for STDIO server: {}", server_id);
            true
        } else if let Some(transport) = self.sse_transports.get(server_id) {
            transport.set_request_callback(callback);
            tracing::info!("Set request callback for SSE server: {}", server_id);
            true
        } else {
            tracing::debug!(
                "Skipping request callback - server {} has no active transport",
                server_id
            );
            false
        }
    }

    /// Register a notification handler for a specific server
    ///
    /// # Arguments
    /// * `server_id` - The server ID to register the handler for
    /// * `callback` - The callback to invoke when notifications are received
    ///
    /// # Returns
    /// * `HandlerId` - A unique ID that can be used to remove this handler later
    ///
    /// Note: Multiple handlers can be registered for the same server.
    pub fn on_notification(&self, server_id: &str, callback: NotificationCallback) -> HandlerId {
        let id = self
            .next_handler_id
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        let handler = NotificationHandler { id, callback };

        self.notification_handlers
            .entry(server_id.to_string())
            .or_default()
            .push(handler);

        id
    }

    /// Remove a notification handler by ID
    ///
    /// # Arguments
    /// * `server_id` - The server ID the handler was registered for
    /// * `handler_id` - The handler ID returned from `on_notification`
    ///
    /// # Returns
    /// * `true` if the handler was found and removed
    /// * `false` if the handler was not found
    pub fn remove_notification_handler(&self, server_id: &str, handler_id: HandlerId) -> bool {
        if let Some(mut handlers) = self.notification_handlers.get_mut(server_id) {
            let initial_len = handlers.len();
            handlers.retain(|h| h.id != handler_id);
            handlers.len() < initial_len
        } else {
            false
        }
    }

    /// Remove all notification handlers for a server
    ///
    /// # Arguments
    /// * `server_id` - The server ID to remove handlers for
    ///
    /// # Returns
    /// * Number of handlers removed
    pub fn clear_notification_handlers(&self, server_id: &str) -> usize {
        if let Some((_, handlers)) = self.notification_handlers.remove(server_id) {
            handlers.len()
        } else {
            0
        }
    }

    /// Dispatch a notification to all registered handlers
    ///
    /// # Arguments
    /// * `server_id` - The server ID that sent the notification
    /// * `notification` - The notification to dispatch
    pub(crate) fn dispatch_notification(&self, server_id: &str, notification: JsonRpcNotification) {
        if let Some(handlers) = self.notification_handlers.get(server_id) {
            for handler in handlers.iter() {
                (handler.callback)(server_id.to_string(), notification.clone());
            }
        }
    }

    /// Get the OAuth manager
    pub fn oauth_manager(&self) -> Arc<McpOAuthManager> {
        self.oauth_manager.clone()
    }

    /// Load server configurations from config
    pub fn load_configs(&self, configs: Vec<McpServerConfig>) {
        for config in configs {
            self.configs.insert(config.id.clone(), config);
        }
    }

    /// Add a server configuration
    pub fn add_config(&self, config: McpServerConfig) {
        self.configs.insert(config.id.clone(), config);
    }

    /// Remove a server configuration
    pub fn remove_config(&self, server_id: &str) {
        self.configs.remove(server_id);
    }

    /// Update a specific field in a server configuration
    pub fn set_config_enabled(&self, server_id: &str, enabled: bool) -> bool {
        if let Some(mut config) = self.configs.get_mut(server_id) {
            config.enabled = enabled;
            true
        } else {
            false
        }
    }

    /// Get a server configuration
    pub fn get_config(&self, server_id: &str) -> Option<McpServerConfig> {
        self.configs.get(server_id).map(|c| c.clone())
    }

    /// List all server configurations
    pub fn list_configs(&self) -> Vec<McpServerConfig> {
        self.configs
            .iter()
            .map(|entry| entry.value().clone())
            .collect()
    }

    /// Start an MCP server
    ///
    /// # Arguments
    /// * `server_id` - The server ID to start
    ///
    /// # Returns
    /// * Ok(()) if the server started successfully
    /// * Err if the server is not configured or failed to start
    pub async fn start_server(&self, server_id: &str) -> AppResult<()> {
        // Get server config
        let config = self
            .configs
            .get(server_id)
            .ok_or_else(|| AppError::Mcp(format!("Server not found: {}", server_id)))?
            .clone();

        if !config.enabled {
            return Err(AppError::Mcp(format!("Server is disabled: {}", server_id)));
        }

        tracing::info!("Starting MCP server: {} ({})", config.name, server_id);

        #[allow(deprecated)]
        let result = match config.transport {
            McpTransportType::Stdio => self.start_stdio_server(server_id, &config).await,
            McpTransportType::Sse | McpTransportType::HttpSse => {
                self.start_sse_server(server_id, &config).await
            }
            McpTransportType::WebSocket => self.start_websocket_server(server_id, &config).await,
        };

        if let Err(ref e) = result {
            self.emit_monitor_event(
                lr_monitor::MonitorEventType::McpServerEvent,
                None,
                None,
                None,
                lr_monitor::MonitorEventData::McpServerEvent {
                    server_id: server_id.to_string(),
                    server_name: Some(config.name.clone()),
                    action: "connection_failed".to_string(),
                    message: format!("{}", e),
                },
                lr_monitor::EventStatus::Error,
                None,
            );
            return result;
        }

        tracing::info!("MCP server started: {}", server_id);
        Ok(())
    }

    /// Start a STDIO MCP server
    async fn start_stdio_server(&self, server_id: &str, config: &McpServerConfig) -> AppResult<()> {
        // Parse STDIO command using shell-words (handles both new single-command and legacy formats)
        let (command, args, config_env) = config
            .transport_config
            .parse_stdio_command()
            .map_err(AppError::Mcp)?;

        // Build environment: shell env (PATH) -> config env -> auth env
        // Later entries override earlier ones, so user config takes precedence
        let mut env = shell_env();

        // Merge config environment variables
        for (key, value) in config_env {
            env.insert(key, value);
        }

        // Merge auth config environment variables (if specified)
        if let Some(lr_config::McpAuthConfig::EnvVars { env_refs }) = &config.auth_config {
            // Resolve env var values from keychain
            let keychain = lr_api_keys::CachedKeychain::auto()
                .unwrap_or_else(|_| lr_api_keys::CachedKeychain::system());
            for (var_name, ref_key) in env_refs {
                match keychain.get(lr_config::MCP_KEYRING_SERVICE, ref_key) {
                    Ok(Some(value)) => {
                        env.insert(var_name.clone(), value);
                    }
                    Ok(None) => {
                        tracing::warn!(
                            "Env var '{}' ref '{}' not found in keychain",
                            var_name,
                            ref_key
                        );
                    }
                    Err(e) => {
                        tracing::warn!(
                            "Failed to read env var '{}' from keychain: {}",
                            var_name,
                            e
                        );
                    }
                }
            }
            tracing::debug!("Applied auth env vars for STDIO server: {}", server_id);
        }

        // Spawn the STDIO transport in the configured working directory
        // (defaults to the user's home directory).
        let cwd = config.transport_config.resolve_stdio_cwd();
        let transport = StdioTransport::spawn_in(command, args, env, cwd).await?;

        // Set up notification callback
        let server_id_for_callback = server_id.to_string();
        let manager_for_callback = self.clone();
        transport.set_notification_callback(Arc::new(move |notification| {
            manager_for_callback.dispatch_notification(&server_id_for_callback, notification);
        }));

        // Store the transport
        self.stdio_transports
            .insert(server_id.to_string(), Arc::new(transport));

        Ok(())
    }

    /// Start an SSE MCP server
    async fn start_sse_server(&self, server_id: &str, config: &McpServerConfig) -> AppResult<()> {
        // Extract SSE config
        let (url, mut headers) = match &config.transport_config {
            McpTransportConfig::Sse { url, headers }
            | McpTransportConfig::HttpSse { url, headers } => (url.clone(), headers.clone()),
            _ => {
                return Err(AppError::Mcp(
                    "Invalid transport config for SSE/HttpSse".to_string(),
                ))
            }
        };

        self.apply_auth_headers(server_id, config, &mut headers)
            .await?;

        // Connect to the SSE server
        let refresher = config
            .auth_config
            .as_ref()
            .map(|_| self.auth_refresher(server_id));
        let transport = SseTransport::connect_with_auth(url, headers, refresher).await?;

        // Set up notification callback
        let server_id_for_callback = server_id.to_string();
        let manager_for_callback = self.clone();
        transport.set_notification_callback(Arc::new(move |notification| {
            manager_for_callback.dispatch_notification(&server_id_for_callback, notification);
        }));

        // Store the transport
        self.sse_transports
            .insert(server_id.to_string(), Arc::new(transport));

        Ok(())
    }

    /// Start a WebSocket MCP server
    async fn start_websocket_server(
        &self,
        server_id: &str,
        config: &McpServerConfig,
    ) -> AppResult<()> {
        // Extract WebSocket config
        let (url, mut headers) = match &config.transport_config {
            McpTransportConfig::WebSocket { url, headers } => (url.clone(), headers.clone()),
            _ => {
                return Err(AppError::Mcp(
                    "Invalid transport config for WebSocket".to_string(),
                ))
            }
        };

        self.apply_auth_headers(server_id, config, &mut headers)
            .await?;

        // Connect to the WebSocket server
        let transport = WebSocketTransport::connect(url, headers).await?;

        // Set up notification callback
        let server_id_for_callback = server_id.to_string();
        let manager_for_callback = self.clone();
        transport.set_notification_callback(Arc::new(move |notification| {
            manager_for_callback.dispatch_notification(&server_id_for_callback, notification);
        }));

        // Store the transport
        self.websocket_transports
            .insert(server_id.to_string(), Arc::new(transport));

        Ok(())
    }

    /// Create a transport for a server (does NOT store globally or set callbacks).
    ///
    /// Returns an `Arc<dyn Transport>` suitable for per-session ownership.
    /// Callers are responsible for setting notification/request callbacks and closing
    /// the transport when done.
    pub async fn create_transport(&self, server_id: &str) -> AppResult<Arc<dyn Transport>> {
        self.create_transport_snapshot(server_id)
            .await
            .map(|(transport, _)| transport)
    }

    pub(crate) async fn create_transport_snapshot(
        &self,
        server_id: &str,
    ) -> AppResult<(Arc<dyn Transport>, McpServerConfig)> {
        let config = self
            .configs
            .get(server_id)
            .ok_or_else(|| AppError::Mcp(format!("Server not found: {}", server_id)))?
            .clone();

        if !config.enabled {
            return Err(AppError::Mcp(format!("Server is disabled: {}", server_id)));
        }

        #[allow(deprecated)]
        let transport = match config.transport {
            McpTransportType::Stdio => self.create_stdio_transport_impl(server_id, &config).await,
            McpTransportType::Sse | McpTransportType::HttpSse => {
                self.create_sse_transport_impl(server_id, &config).await
            }
            McpTransportType::WebSocket => {
                self.create_websocket_transport_impl(server_id, &config)
                    .await
            }
        }?;
        Ok((transport, config))
    }

    /// Create transports for multiple servers in parallel.
    ///
    /// Returns a `SessionTransportSet` with all successfully created transports,
    /// plus a list of failures.
    pub async fn create_transports(
        &self,
        server_ids: &[String],
        timeout: std::time::Duration,
    ) -> (
        crate::transport::SessionTransportSet,
        Vec<crate::gateway::types::ServerFailure>,
    ) {
        use crate::gateway::types::ServerFailure;
        use crate::transport::SessionTransportSet;

        let transport_set = SessionTransportSet::new();
        self.session_failures
            .retain(|_, state| state.strong_count() > 0);
        self.session_failures.insert(
            uuid::Uuid::new_v4().to_string(),
            Arc::downgrade(&transport_set.failures),
        );
        let cache = self.health_cache.clone();
        let names: HashMap<String, String> = self
            .list_configs()
            .into_iter()
            .map(|c| (c.id, c.name))
            .collect();
        let sessions = self.session_failures.clone();
        transport_set.set_failure_callback(Arc::new(move |id, error| {
            if let Some(cache) = cache.read().as_ref() {
                let error = error.map(str::to_string).or_else(|| {
                    sessions.iter().find_map(|entry| {
                        entry.value().upgrade().and_then(|state| {
                            state
                                .read()
                                .get(id)
                                .and_then(|methods| methods.values().next().cloned())
                        })
                    })
                });
                let name = names.get(id).cloned().unwrap_or_else(|| id.to_string());
                let item = match error {
                    Some(error) => lr_providers::health_cache::ItemHealth::unhealthy(name, error),
                    None => lr_providers::health_cache::ItemHealth::healthy(name, None),
                };
                cache.update_mcp_server(id, item);
            }
        }));
        for id in server_ids {
            transport_set.record_failure(
                id,
                "_connect",
                "MCP connection is still initializing; client tools are not ready yet",
            );
        }
        let mut failures = Vec::new();

        let futures: Vec<_> = server_ids
            .iter()
            .map(|server_id| {
                let server_id = server_id.clone();
                let manager = self.clone();
                let server_name = self
                    .get_config(&server_id)
                    .map(|c| c.name.clone())
                    .unwrap_or_else(|| server_id.clone());
                async move {
                    tracing::info!("Creating transport for MCP server '{}'...", server_name);
                    let start = std::time::Instant::now();
                    let result = tokio::time::timeout(
                        timeout,
                        manager.create_transport_snapshot(&server_id),
                    )
                    .await;
                    manager.report_startup_phase(
                        &server_id,
                        "transport_connect",
                        start.elapsed().as_millis() as u64,
                        matches!(&result, Ok(Ok(_))),
                    );
                    tracing::info!(
                        "Transport creation for '{}' completed in {:?}: {}",
                        server_name,
                        start.elapsed(),
                        match &result {
                            Ok(Ok(_)) => "success".to_string(),
                            Ok(Err(e)) => format!("error: {}", e),
                            Err(_) => "TIMEOUT".to_string(),
                        }
                    );
                    (server_id, result)
                }
            })
            .collect();

        let results = futures::future::join_all(futures).await;

        for (server_id, result) in results {
            match result {
                Ok(Ok((transport, config))) => {
                    transport_set.insert_configured(server_id, transport, config);
                }
                Ok(Err(e)) => {
                    failures.push(ServerFailure {
                        server_id,
                        error: e.to_string(),
                    });
                }
                Err(_) => {
                    failures.push(ServerFailure {
                        server_id,
                        error: format!("Server startup timed out after {}s", timeout.as_secs()),
                    });
                }
            }
        }

        for failure in &failures {
            transport_set.record_failure(
                &failure.server_id,
                "_connect",
                "MCP connection could not start; check authentication and server availability",
            );
        }
        (transport_set, failures)
    }

    /// Internal: create a STDIO transport without storing or setting callbacks.
    async fn create_stdio_transport_impl(
        &self,
        server_id: &str,
        config: &McpServerConfig,
    ) -> AppResult<Arc<dyn Transport>> {
        let (command, args, config_env) = config
            .transport_config
            .parse_stdio_command()
            .map_err(AppError::Mcp)?;

        let mut env = shell_env();
        for (key, value) in config_env {
            env.insert(key, value);
        }

        if let Some(lr_config::McpAuthConfig::EnvVars { env_refs }) = &config.auth_config {
            let keychain = lr_api_keys::CachedKeychain::auto()
                .unwrap_or_else(|_| lr_api_keys::CachedKeychain::system());
            for (var_name, ref_key) in env_refs {
                match keychain.get(lr_config::MCP_KEYRING_SERVICE, ref_key) {
                    Ok(Some(value)) => {
                        env.insert(var_name.clone(), value);
                    }
                    Ok(None) => {
                        tracing::warn!(
                            "Env var '{}' ref '{}' not found in keychain",
                            var_name,
                            ref_key
                        );
                    }
                    Err(e) => {
                        tracing::warn!(
                            "Failed to read env var '{}' from keychain: {}",
                            var_name,
                            e
                        );
                    }
                }
            }
            tracing::debug!("Applied auth env vars for STDIO server: {}", server_id);
        }

        let cwd = config.transport_config.resolve_stdio_cwd();
        let transport = StdioTransport::spawn_in(command, args, env, cwd).await?;
        Ok(Arc::new(transport))
    }

    /// Internal: create an SSE transport without storing or setting callbacks.
    async fn create_sse_transport_impl(
        &self,
        server_id: &str,
        config: &McpServerConfig,
    ) -> AppResult<Arc<dyn Transport>> {
        let (url, mut headers) = match &config.transport_config {
            McpTransportConfig::Sse { url, headers }
            | McpTransportConfig::HttpSse { url, headers } => (url.clone(), headers.clone()),
            _ => {
                return Err(AppError::Mcp(
                    "Invalid transport config for SSE/HttpSse".to_string(),
                ))
            }
        };

        self.apply_auth_headers(server_id, config, &mut headers)
            .await?;

        let refresher = config
            .auth_config
            .as_ref()
            .map(|_| self.auth_refresher(server_id));
        let transport = SseTransport::connect_with_auth(url, headers, refresher).await?;
        Ok(Arc::new(transport))
    }

    /// Internal: create a WebSocket transport without storing or setting callbacks.
    async fn create_websocket_transport_impl(
        &self,
        server_id: &str,
        config: &McpServerConfig,
    ) -> AppResult<Arc<dyn Transport>> {
        let (url, mut headers) = match &config.transport_config {
            McpTransportConfig::WebSocket { url, headers } => (url.clone(), headers.clone()),
            _ => {
                return Err(AppError::Mcp(
                    "Invalid transport config for WebSocket".to_string(),
                ))
            }
        };

        self.apply_auth_headers(server_id, config, &mut headers)
            .await?;

        let transport = WebSocketTransport::connect(url, headers).await?;
        Ok(Arc::new(transport))
    }

    /// Apply auth config to HTTP headers (shared by SSE and WebSocket transports).
    async fn apply_auth_headers(
        &self,
        server_id: &str,
        config: &McpServerConfig,
        headers: &mut HashMap<String, String>,
    ) -> AppResult<()> {
        self.apply_auth_headers_with(server_id, config, headers, false)
            .await
    }

    /// Build a callback that re-resolves a server's auth headers after a
    /// 401, forcing a browser-OAuth token refresh (the stored token was
    /// rejected even if it looks unexpired).
    fn auth_refresher(&self, server_id: &str) -> crate::transport::sse::AuthRefresher {
        let manager = self.clone();
        let server_id = server_id.to_string();
        Arc::new(move |force_refresh| {
            let manager = manager.clone();
            let server_id = server_id.clone();
            Box::pin(async move {
                let config = manager
                    .get_config(&server_id)
                    .ok_or_else(|| AppError::Mcp(format!("Server not found: {}", server_id)))?;
                if !force_refresh
                    && !matches!(
                        config.auth_config,
                        Some(lr_config::McpAuthConfig::OAuthBrowser { .. })
                    )
                {
                    return Ok(HashMap::new());
                }
                let mut headers = HashMap::new();
                manager
                    .apply_auth_headers_with(&server_id, &config, &mut headers, force_refresh)
                    .await?;
                Ok(headers)
            })
        })
    }

    async fn apply_auth_headers_with(
        &self,
        server_id: &str,
        config: &McpServerConfig,
        headers: &mut HashMap<String, String>,
        force_refresh: bool,
    ) -> AppResult<()> {
        let Some(auth_config) = &config.auth_config else {
            return Ok(());
        };

        match auth_config {
            lr_config::McpAuthConfig::BearerToken { token_ref } => {
                let keychain = lr_api_keys::CachedKeychain::auto()
                    .unwrap_or_else(|_| lr_api_keys::CachedKeychain::system());
                let account_name = lr_config::mcp_bearer_token_account(&config.id);
                let mut token = keychain
                    .get(lr_config::MCP_KEYRING_SERVICE, &account_name)
                    .ok()
                    .flatten();
                // Marketplace installs before 0.0.155 stored the token under
                // the bare server id.
                if token.is_none() && token_ref == lr_config::LEGACY_PENDING_BEARER_TOKEN_REF {
                    token = keychain
                        .get(lr_config::MCP_KEYRING_SERVICE, &config.id)
                        .ok()
                        .flatten();
                }
                if let Some(token) = token {
                    headers.insert("Authorization".to_string(), format!("Bearer {}", token));
                    tracing::debug!("Applied bearer token auth for server: {}", server_id);
                } else {
                    tracing::warn!("Bearer token not found in keychain for MCP server");
                }
            }
            lr_config::McpAuthConfig::CustomHeaders { header_refs } => {
                let keychain = lr_api_keys::CachedKeychain::auto()
                    .unwrap_or_else(|_| lr_api_keys::CachedKeychain::system());
                for (header_name, ref_key) in header_refs {
                    match keychain.get(lr_config::MCP_KEYRING_SERVICE, ref_key) {
                        Ok(Some(value)) => {
                            headers.insert(header_name.clone(), value);
                        }
                        Ok(None) => {
                            tracing::warn!(
                                "Header '{}' ref '{}' not found in keychain",
                                header_name,
                                ref_key
                            );
                        }
                        Err(e) => {
                            tracing::warn!(
                                "Failed to read header '{}' from keychain: {}",
                                header_name,
                                e
                            );
                        }
                    }
                }
                tracing::debug!("Applied custom headers auth for server: {}", server_id);
            }
            lr_config::McpAuthConfig::OAuth {
                client_id,
                client_secret_ref,
                token_url,
                scopes,
                ..
            } => {
                let keychain = lr_api_keys::CachedKeychain::auto()
                    .unwrap_or_else(|_| lr_api_keys::CachedKeychain::system());

                let client_secret = match keychain
                    .get(lr_config::MCP_KEYRING_SERVICE, client_secret_ref)
                {
                    Ok(Some(secret)) => secret,
                    Ok(None) => {
                        let msg =
                            format!("OAuth client secret not found for server: {}", server_id);
                        tracing::warn!("OAuth client secret not found in keychain for MCP server");
                        return Err(AppError::Mcp(msg));
                    }
                    Err(e) => {
                        let msg = format!("Failed to retrieve OAuth client secret: {}", e);
                        tracing::error!("{}", msg);
                        return Err(e);
                    }
                };

                let client = reqwest::Client::builder()
                    .connect_timeout(std::time::Duration::from_secs(10))
                    .timeout(std::time::Duration::from_secs(30))
                    .redirect(reqwest::redirect::Policy::none())
                    .build()
                    .expect("MCP OAuth token client");
                let mut form_params = vec![
                    ("grant_type", "client_credentials"),
                    ("client_id", client_id.as_str()),
                    ("client_secret", client_secret.as_str()),
                ];
                let scopes_str = scopes.join(" ");
                if !scopes.is_empty() {
                    form_params.push(("scope", scopes_str.as_str()));
                }

                let token_response = client
                    .post(token_url)
                    .form(&form_params)
                    .send()
                    .await
                    .map_err(|e| AppError::Mcp(format!("OAuth token request failed: {}", e)))?;

                if !token_response.status().is_success() {
                    let status = token_response.status();
                    let body = token_response.text().await.unwrap_or_default();
                    return Err(AppError::Mcp(format!(
                        "OAuth token request failed with status {}: {}",
                        status, body
                    )));
                }

                let token_json: serde_json::Value = token_response.json().await.map_err(|e| {
                    AppError::Mcp(format!("Failed to parse OAuth token response: {}", e))
                })?;

                let access_token = token_json
                    .get("access_token")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        AppError::Mcp("OAuth response missing access_token".to_string())
                    })?;

                headers.insert(
                    "Authorization".to_string(),
                    format!("Bearer {}", access_token),
                );
                tracing::info!("Applied OAuth token for server: {}", server_id);
            }
            auth @ lr_config::McpAuthConfig::OAuthBrowser { .. } => {
                let resource_url = match &config.transport_config {
                    McpTransportConfig::Sse { url, .. }
                    | McpTransportConfig::HttpSse { url, .. } => url,
                    _ => {
                        return Err(AppError::Mcp(
                            "Browser OAuth requires HTTP transport".into(),
                        ))
                    }
                };
                let token = if force_refresh {
                    self.oauth_manager
                        .refresh_browser_token(&config.id, auth, resource_url)
                        .await?
                } else {
                    self.oauth_manager
                        .get_browser_token(&config.id, auth, resource_url)
                        .await?
                };
                headers.insert("Authorization".to_string(), format!("Bearer {token}"));
            }
            _ => {
                tracing::debug!("No applicable auth config for server: {}", server_id);
            }
        }

        Ok(())
    }

    /// Stop an MCP server
    ///
    /// # Arguments
    /// * `server_id` - The server ID to stop
    ///
    /// # Returns
    /// * Ok(()) if the server stopped successfully
    /// * Err if the server is not running or failed to stop
    pub async fn stop_server(&self, server_id: &str) -> AppResult<()> {
        tracing::info!("Stopping MCP server: {}", server_id);

        // Try to stop STDIO transport
        if let Some((_, transport)) = self.stdio_transports.remove(server_id) {
            transport.close().await?;
            tracing::info!("MCP STDIO server stopped: {}", server_id);
            return Ok(());
        }

        // Try to stop SSE transport
        if let Some((_, transport)) = self.sse_transports.remove(server_id) {
            transport.close().await?;
            tracing::info!("MCP SSE server stopped: {}", server_id);
            return Ok(());
        }

        // Try to stop WebSocket transport
        if let Some((_, transport)) = self.websocket_transports.remove(server_id) {
            transport.close().await?;
            tracing::info!("MCP WebSocket server stopped: {}", server_id);
            return Ok(());
        }

        // Server not running
        Err(AppError::Mcp(format!("Server not running: {}", server_id)))
    }

    /// Send a JSON-RPC request to an MCP server
    ///
    /// # Arguments
    /// * `server_id` - The server ID to send the request to
    /// * `request` - The JSON-RPC request
    ///
    /// # Returns
    /// * The JSON-RPC response from the server
    pub async fn send_request(
        &self,
        server_id: &str,
        request: JsonRpcRequest,
    ) -> AppResult<JsonRpcResponse> {
        // Check STDIO transport
        if let Some(transport) = self.stdio_transports.get(server_id) {
            return transport.send_request(request).await;
        }

        // Check SSE transport
        if let Some(transport) = self.sse_transports.get(server_id) {
            return transport.send_request(request).await;
        }

        // Check WebSocket transport
        if let Some(transport) = self.websocket_transports.get(server_id) {
            return transport.send_request(request).await;
        }

        Err(AppError::Mcp(format!("Server not running: {}", server_id)))
    }

    /// Check if a server's transport supports streaming
    ///
    /// # Arguments
    /// * `server_id` - The server ID to check
    ///
    /// # Returns
    /// * `true` if the transport supports streaming, `false` otherwise
    pub fn supports_streaming(&self, server_id: &str) -> bool {
        // Check STDIO transport
        if let Some(transport) = self.stdio_transports.get(server_id) {
            return transport.supports_streaming();
        }

        // Check SSE transport
        if let Some(transport) = self.sse_transports.get(server_id) {
            return transport.supports_streaming();
        }

        // Check WebSocket transport
        if let Some(transport) = self.websocket_transports.get(server_id) {
            return transport.supports_streaming();
        }

        false
    }

    /// Send a streaming request to an MCP server
    ///
    /// # Arguments
    /// * `server_id` - The server ID to send the request to
    /// * `request` - The JSON-RPC request to send
    ///
    /// # Returns
    /// * A stream of chunks representing the response
    pub async fn stream_request(
        &self,
        server_id: &str,
        request: JsonRpcRequest,
    ) -> AppResult<Pin<Box<dyn Stream<Item = AppResult<StreamingChunk>> + Send>>> {
        // Check STDIO transport
        if let Some(transport) = self.stdio_transports.get(server_id) {
            return transport.stream_request(request).await;
        }

        // Check SSE transport
        if let Some(transport) = self.sse_transports.get(server_id) {
            return transport.stream_request(request).await;
        }

        // Check WebSocket transport
        if let Some(transport) = self.websocket_transports.get(server_id) {
            return transport.stream_request(request).await;
        }

        Err(AppError::Mcp(format!("Server not running: {}", server_id)))
    }

    /// Get the health status of an MCP server
    ///
    /// # Arguments
    /// * `server_id` - The server ID to check
    ///
    /// # Returns
    /// * The health status
    pub async fn get_server_health(&self, server_id: &str) -> McpServerHealth {
        let config = self.get_config(server_id);

        // Check if there's a UI-managed global transport still alive
        let has_live_global_transport = self
            .stdio_transports
            .get(server_id)
            .is_some_and(|t| t.is_alive())
            || self
                .sse_transports
                .get(server_id)
                .is_some_and(|t| t.is_healthy())
            || self
                .websocket_transports
                .get(server_id)
                .is_some_and(|t| t.is_healthy());

        // A transport that exists but is no longer alive means a running server
        // went away, as opposed to a server that simply hasn't been started.
        let had_dead_transport = !has_live_global_transport
            && (self.stdio_transports.contains_key(server_id)
                || self.sse_transports.contains_key(server_id)
                || self.websocket_transports.contains_key(server_id));

        // Server health describes current availability, not historical failures
        // on independent client sessions. Always probe the server, even when a
        // client previously failed to initialize. Session diagnostics and their
        // per-operation recovery remain owned by SessionTransportSet.
        // A live process/socket alone does not prove protocol availability.
        let (status, latency_ms, error) = if has_live_global_transport {
            let probe = async {
                let response = self
                    .send_request(
                        server_id,
                        JsonRpcRequest::with_id(0, "tools/list".into(), None),
                    )
                    .await?;
                if response.error.as_ref().is_some_and(|e| e.code == -32601) {
                    // A resource/prompt-only MCP server can legitimately have no tools.
                    let ping = self
                        .send_request(server_id, JsonRpcRequest::with_id(1, "ping".into(), None))
                        .await?;
                    if ping.error.is_none() {
                        return Ok(());
                    }
                } else if response.error.is_none()
                    && response
                        .result
                        .as_ref()
                        .is_some_and(|v| v.get("tools").is_some_and(serde_json::Value::is_array))
                {
                    return Ok(());
                }
                Err::<(), AppError>(AppError::Mcp("Running MCP catalog request failed".into()))
            };
            match tokio::time::timeout(std::time::Duration::from_secs(10), probe).await {
                Ok(Ok(())) => (HealthStatus::Healthy, None, None),
                _ => (HealthStatus::Unhealthy, None, Some("Running MCP connection failed protocol/tool discovery; check authentication and server availability".into())),
            }
        } else {
            // Clean up any dead global transports before the readiness check
            if self
                .stdio_transports
                .get(server_id)
                .is_some_and(|t| !t.is_alive())
            {
                self.stdio_transports.remove(server_id);
            }
            if self
                .sse_transports
                .get(server_id)
                .is_some_and(|t| !t.is_healthy())
            {
                self.sse_transports.remove(server_id);
            }
            if self
                .websocket_transports
                .get(server_id)
                .is_some_and(|t| !t.is_healthy())
            {
                self.websocket_transports.remove(server_id);
            }
            self.check_server_readiness(&config).await
        };

        let server_name = config
            .map(|c| c.name)
            .unwrap_or_else(|| "Unknown".to_string());

        // Emit a monitor event when the server can't be reached or started
        if status == HealthStatus::Unhealthy {
            if let Some(ref err_msg) = error {
                let (action, message) = if had_dead_transport {
                    (
                        "disconnected",
                        format!(
                            "{server_name} was running but stopped responding, and restarting it failed: {err_msg}"
                        ),
                    )
                } else {
                    (
                        "health_check_failed",
                        format!("Health check could not start {server_name}. {err_msg}"),
                    )
                };
                self.emit_monitor_event(
                    lr_monitor::MonitorEventType::McpServerEvent,
                    None,
                    None,
                    None,
                    lr_monitor::MonitorEventData::McpServerEvent {
                        server_id: server_id.to_string(),
                        server_name: Some(server_name.clone()),
                        action: action.to_string(),
                        message,
                    },
                    lr_monitor::EventStatus::Error,
                    None,
                );
            }
        }

        McpServerHealth {
            server_id: server_id.to_string(),
            server_name,
            status,
            latency_ms,
            error,
            last_check: chrono::Utc::now(),
        }
    }

    /// Check a non-running server through an isolated authenticated MCP handshake.
    /// Arbitrary process output or HTTP reachability alone is insufficient.
    async fn check_server_readiness(
        &self,
        config: &Option<McpServerConfig>,
    ) -> (HealthStatus, Option<u64>, Option<String>) {
        let Some(config) = config else {
            return (
                HealthStatus::Unknown,
                None,
                Some("Server not configured".to_string()),
            );
        };

        self.check_protocol_readiness(&config.id).await
    }

    /// Try to spawn a command and wait for it to produce output
    ///
    /// This verifies the command can actually run (including npx downloads).
    /// Returns Ok(latency_ms) if the command starts and produces output, Err(message) otherwise.
    async fn try_spawn_command(
        command: &str,
        args: &[String],
        env: &std::collections::HashMap<String, String>,
        cwd: Option<&std::path::Path>,
    ) -> Result<u64, String> {
        let start = std::time::Instant::now();
        const TIMEOUT_SECS: u64 = 60; // Allow time for npx downloads

        if let Some(dir) = cwd {
            if !dir.is_dir() {
                return Err(format!(
                    "Working directory does not exist: {}",
                    dir.display()
                ));
            }
        }

        // Build the command
        let mut cmd = tokio::process::Command::new(command);
        cmd.args(args)
            .envs(env.iter())
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        if let Some(dir) = cwd {
            cmd.current_dir(dir);
        }

        // Try to spawn
        let mut child = match cmd.spawn() {
            Ok(child) => child,
            Err(e) => {
                return Err(match e.kind() {
                    std::io::ErrorKind::NotFound => format!("Command not found: {}", command),
                    std::io::ErrorKind::PermissionDenied => {
                        format!("Permission denied: {}", command)
                    }
                    _ => format!("Failed to spawn: {}", e),
                });
            }
        };

        let stdout = child.stdout.take();
        let stderr = child.stderr.take();

        // Hold stdin open for the whole probe. `Child::wait` closes a stdin it
        // still owns before waiting, and stdio MCP servers exit (often with an
        // error such as "connection closed: initialize request") when stdin
        // closes before the handshake. Sending an initialize request also gives
        // a well-behaved server something to answer on stdout right away.
        let mut stdin = child.stdin.take();
        if let Some(ref mut pipe) = stdin {
            use tokio::io::AsyncWriteExt;
            let _ = pipe.write_all(PROBE_INITIALIZE_REQUEST.as_bytes()).await;
            let _ = pipe.flush().await;
        }

        // Wait for either:
        // 1. Any output on stdout (MCP server is running)
        // 2. Any output on stderr (could be error or npx progress)
        // 3. Process exit
        // 4. Timeout
        let result = tokio::time::timeout(
            tokio::time::Duration::from_secs(TIMEOUT_SECS),
            Self::wait_for_output_or_exit(&mut child, stdout, stderr),
        )
        .await;

        let latency = start.elapsed().as_millis() as u64;

        // Always try to kill the child process
        let _ = child.kill().await;
        drop(stdin);

        match result {
            Ok(Ok(())) => {
                // Got output - server can start
                Ok(latency)
            }
            Ok(Err(e)) => {
                // Process exited with error
                Err(e)
            }
            Err(_) => {
                // Timeout - process is still running but no output
                // This is unusual but not necessarily an error
                Err(format!(
                    "Timeout after {}s waiting for response",
                    TIMEOUT_SECS
                ))
            }
        }
    }

    /// Wait for stdout output or process exit to determine health.
    ///
    /// Stderr is drained in a background task to prevent pipe buffer deadlock
    /// but is NOT used as a health signal — only as diagnostic context when the
    /// process actually exits with an error code. This avoids false negatives
    /// from MCP servers that log warnings (e.g. "connection closed: initialize
    /// request") to stderr when they don't receive an immediate initialize request.
    async fn wait_for_output_or_exit(
        child: &mut tokio::process::Child,
        stdout: Option<tokio::process::ChildStdout>,
        stderr: Option<tokio::process::ChildStderr>,
    ) -> Result<(), String> {
        use tokio::io::AsyncBufReadExt;

        let stdout_reader = stdout.map(tokio::io::BufReader::new);

        // Drain stderr in background to prevent pipe buffer deadlock.
        // Captured output is only used for diagnostics when the process exits with failure.
        let stderr_capture = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        let _stderr_task = if let Some(stderr) = stderr {
            let capture = stderr_capture.clone();
            Some(tokio::spawn(async move {
                let mut reader = tokio::io::BufReader::new(stderr);
                let mut line = String::new();
                loop {
                    line.clear();
                    match reader.read_line(&mut line).await {
                        Ok(0) => break,
                        Ok(_) => {
                            if let Ok(mut buf) = capture.lock() {
                                buf.push_str(&line);
                                if buf.len() > 4096 {
                                    break;
                                }
                            }
                        }
                        Err(_) => break,
                    }
                }
            }))
        } else {
            None
        };

        enum Outcome {
            Output,
            StdoutClosed,
            Exited(std::io::Result<std::process::ExitStatus>),
            StillRunning,
        }

        let outcome = tokio::select! {
            // Wait for stdout line (positive signal: server is producing output)
            result = async {
                if let Some(mut reader) = stdout_reader {
                    let mut line = String::new();
                    reader.read_line(&mut line).await
                } else {
                    std::future::pending::<std::io::Result<usize>>().await
                }
            } => {
                match result {
                    Ok(0) => Outcome::StdoutClosed,
                    Ok(_) => Outcome::Output,
                    Err(e) => return Err(format!("Failed to read stdout: {}", e)),
                }
            }

            // Wait for process exit
            result = child.wait() => Outcome::Exited(result),

            // If the process is still alive after a brief delay, consider it healthy.
            // MCP stdio servers won't produce stdout output until they receive a
            // JSON-RPC initialize request on stdin, so staying alive without errors
            // is sufficient for a health check.
            _ = tokio::time::sleep(tokio::time::Duration::from_secs(3)) => Outcome::StillRunning,
        };

        let exit = match outcome {
            // Got output on stdout - server is running!
            Outcome::Output | Outcome::StillRunning => return Ok(()),
            // Stdout closes when the process exits, so let the exit status decide.
            // A process that closed stdout but keeps running is still ready.
            Outcome::StdoutClosed => {
                match tokio::time::timeout(tokio::time::Duration::from_secs(1), child.wait()).await
                {
                    Ok(result) => result,
                    Err(_) => return Ok(()),
                }
            }
            Outcome::Exited(result) => result,
        };

        match exit {
            // Exited successfully (unusual for MCP servers)
            Ok(status) if status.success() => Ok(()),
            Ok(status) => {
                // Process exited with error - use captured stderr for context
                tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
                let captured = stderr_capture
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .clone();
                Err(describe_early_exit(&status.to_string(), &captured))
            }
            Err(e) => Err(format!("Failed to wait for process: {}", e)),
        }
    }

    /// Authenticate and perform a real MCP handshake and catalog request.
    /// HEAD (even HTTP 401/500) used to count as Ready without credentials.
    async fn check_protocol_readiness(
        &self,
        server_id: &str,
    ) -> (HealthStatus, Option<u64>, Option<String>) {
        let start = std::time::Instant::now();
        // Owning the temporary transport in a set guarantees cleanup even
        // when the outer deadline cancels the handshake/discovery future.
        let transports = crate::transport::SessionTransportSet::new();
        let probe = async {
            let transport = self.create_transport(server_id).await?;
            transports.insert(server_id.to_string(), transport);
            let initialize = JsonRpcRequest::with_id(
                0,
                "initialize".into(),
                Some(serde_json::json!({
                    "protocolVersion": crate::protocol::MCP_PROTOCOL_VERSION,
                    "capabilities": {},
                    "clientInfo": {"name": "localrouter-health-check", "version": "1"}
                })),
            );
            let response = transports.send_request(server_id, initialize).await?;
            let capabilities = if response.error.is_some() {
                // Stateless backends reject initialize. Negotiate discovery
                // before deciding they are down, as the gateway does.
                let discover = JsonRpcRequest::with_id(
                    1,
                    "server/discover".into(),
                    Some(serde_json::json!({
                        "_meta": {crate::protocol::meta_keys::PROTOCOL_VERSION: crate::protocol::MCP_PROTOCOL_VERSION_STATELESS}
                    })),
                );
                let discovered = transports.send_request(server_id, discover).await?;
                let value = discovered
                    .result
                    .ok_or_else(|| AppError::Mcp("MCP handshake rejected".into()))?;
                if !value
                    .get("protocolVersions")
                    .and_then(|v| v.as_array())
                    .is_some_and(|versions| {
                        versions.iter().any(|v| {
                            v.as_str() == Some(crate::protocol::MCP_PROTOCOL_VERSION_STATELESS)
                        })
                    })
                {
                    return Err(AppError::Mcp(
                        "MCP discovery did not negotiate a supported protocol".into(),
                    ));
                }
                transports.set_revision(server_id, crate::protocol::ProtocolRevision::V2026_07_28);
                serde_json::from_value::<crate::gateway::types::ServerCapabilities>(
                    value.get("capabilities").cloned().unwrap_or_default(),
                )
                .map_err(|_| AppError::Mcp("Invalid MCP discovery capabilities".into()))?
            } else {
                let initialized: crate::gateway::types::InitializeResult =
                    serde_json::from_value(response.result.unwrap_or_default())
                        .map_err(|_| AppError::Mcp("Invalid MCP initialize response".into()))?;
                transports
                    .send_request(
                        server_id,
                        JsonRpcRequest::new(None, "notifications/initialized".into(), None),
                    )
                    .await?;
                initialized.capabilities
            };
            if capabilities.tools.is_some() {
                let response = transports
                    .send_request(
                        server_id,
                        JsonRpcRequest::with_id(1, "tools/list".into(), None),
                    )
                    .await?;
                if response.error.is_some()
                    || !response
                        .result
                        .as_ref()
                        .is_some_and(|v| v.get("tools").is_some_and(serde_json::Value::is_array))
                {
                    return Err(AppError::Mcp(
                        "MCP tools/list failed; check authentication and permissions".into(),
                    ));
                }
            }
            Ok::<(), AppError>(())
        };
        let result = tokio::time::timeout(std::time::Duration::from_secs(20), probe).await;
        transports.close_all().await;
        match result {
            Ok(Ok(())) => (
                HealthStatus::Ready,
                Some(start.elapsed().as_millis() as u64),
                Some("MCP handshake verified; client sessions connect independently".into()),
            ),
            // Do not surface arbitrary upstream response bodies or credential-bearing URLs.
            Ok(Err(error)) => (
                HealthStatus::Unhealthy,
                None,
                Some({
                    let description = error.to_string().to_lowercase();
                    if description.contains("not found") || description.contains("no such file") {
                        "MCP server executable or endpoint not found".into()
                    } else {
                        "Authenticated MCP handshake or tool discovery failed; check server authentication and availability".into()
                    }
                }),
            ),
            Err(_) => (
                HealthStatus::Unhealthy,
                None,
                Some("Authenticated MCP health check timed out after 20s".into()),
            ),
        }
    }

    /// Get health status for all servers
    pub async fn get_all_health(&self) -> Vec<McpServerHealth> {
        let mut health_statuses = Vec::new();

        for entry in self.configs.iter() {
            let server_id = entry.key();
            let health = self.get_server_health(server_id).await;
            health_statuses.push(health);
        }

        health_statuses
    }

    /// Check if a server is running
    pub fn is_running(&self, server_id: &str) -> bool {
        self.stdio_transports.contains_key(server_id)
            || self.sse_transports.contains_key(server_id)
            || self.websocket_transports.contains_key(server_id)
    }

    /// Get the transport type for a running server
    ///
    /// # Arguments
    /// * `server_id` - The server ID to check
    ///
    /// # Returns
    /// * The transport type as a string ("stdio", "http-sse", "websocket", or "unknown")
    pub fn get_transport_type(&self, server_id: &str) -> &'static str {
        if self.stdio_transports.contains_key(server_id) {
            "stdio"
        } else if self.sse_transports.contains_key(server_id) {
            "http-sse"
        } else if self.websocket_transports.contains_key(server_id) {
            "websocket"
        } else {
            "unknown"
        }
    }

    /// Shutdown all servers
    pub async fn shutdown_all(&self) {
        tracing::info!("Shutting down all MCP servers");

        // Collect all server IDs from all transport types
        let mut server_ids = Vec::new();

        for entry in self.stdio_transports.iter() {
            server_ids.push(entry.key().clone());
        }
        for entry in self.sse_transports.iter() {
            server_ids.push(entry.key().clone());
        }
        for entry in self.websocket_transports.iter() {
            server_ids.push(entry.key().clone());
        }

        // Stop all servers
        for server_id in server_ids {
            if let Err(e) = self.stop_server(&server_id).await {
                tracing::error!("Failed to stop server {}: {}", server_id, e);
                let server_name = self.get_config(&server_id).map(|c| c.name);
                self.emit_monitor_event(
                    lr_monitor::MonitorEventType::McpServerEvent,
                    None,
                    None,
                    None,
                    lr_monitor::MonitorEventData::McpServerEvent {
                        server_id: server_id.clone(),
                        server_name,
                        action: "stop_failed".to_string(),
                        message: format!("Failed to stop server during shutdown: {}", e),
                    },
                    lr_monitor::EventStatus::Error,
                    None,
                );
            }
        }
    }
}

/// MCP initialize request written to a stdio server during a readiness probe
const PROBE_INITIALIZE_REQUEST: &str = concat!(
    r#"{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2025-06-18","#,
    r#""capabilities":{},"clientInfo":{"name":"localrouter-health-check","version":"1"}}}"#,
    "\n"
);

/// Remove ANSI escape sequences (colored log output) from a line
fn strip_ansi(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();
                // CSI sequence: parameters, then a final byte in @..~
                for c in chars.by_ref() {
                    if ('@'..='~').contains(&c) {
                        break;
                    }
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

/// Explain a stdio server that exited during the readiness probe, quoting the
/// most relevant stderr line.
fn describe_early_exit(status: &str, stderr: &str) -> String {
    let lines: Vec<String> = stderr
        .lines()
        .map(|l| strip_ansi(l).trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();
    let reason = lines
        .iter()
        .find(|l| {
            let lower = l.to_lowercase();
            lower.contains("error")
                || lower.contains("fatal")
                || lower.contains("failed")
                || lower.contains("not found")
        })
        .or_else(|| lines.last());
    let mut message =
        format!("The server process stopped on its own ({status}) before it could be used");
    if let Some(reason) = reason {
        let reason = if reason.chars().count() > 160 {
            format!("{}...", reason.chars().take(160).collect::<String>())
        } else {
            reason.clone()
        };
        message.push_str(&format!(". It reported: {reason}"));
    }
    message
}

impl Default for McpServerManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lr_config::McpServerConfig;
    use std::collections::HashMap;

    async fn health_fixture() -> (
        McpServerManager,
        String,
        Arc<std::sync::atomic::AtomicBool>,
        tokio::task::JoinHandle<()>,
    ) {
        use axum::{
            http::{HeaderMap, StatusCode},
            routing::get,
            Json, Router,
        };
        let broken = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let state = broken.clone();
        let app = Router::new().route("/mcp", get(|| async { StatusCode::METHOD_NOT_ALLOWED }).post(
            move |headers: HeaderMap, Json(request): Json<JsonRpcRequest>| {
                let state = state.clone();
                async move {
                    use axum::response::IntoResponse;
                    if headers.get("authorization").and_then(|h| h.to_str().ok()) != Some("Bearer health-fixture") {
                        return StatusCode::UNAUTHORIZED.into_response();
                    }
                    let result = match request.method.as_str() {
                        "initialize" => serde_json::json!({"protocolVersion":"2025-11-25", "capabilities":{"tools":{}}, "serverInfo":{"name":"fixture","version":"1"}}),
                        "tools/list" if state.load(std::sync::atomic::Ordering::Relaxed) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
                        "tools/list" => serde_json::json!({"tools":[]}),
                        _ => serde_json::json!({}),
                    };
                    Json(JsonRpcResponse::success(request.id.unwrap_or_default(), result)).into_response()
                }
            }
        ));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/mcp", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let manager = McpServerManager::new_for_test();
        let config = McpServerConfig::new(
            "fixture".into(),
            McpTransportType::HttpSse,
            McpTransportConfig::HttpSse {
                url,
                headers: HashMap::from([("Authorization".into(), "Bearer health-fixture".into())]),
            },
        );
        let id = config.id.clone();
        manager.add_config(config);
        (manager, id, broken, task)
    }

    #[tokio::test]
    async fn remote_health_requires_authenticated_mcp_discovery() {
        let (manager, id, broken, task) = health_fixture().await;
        assert_eq!(
            manager.get_server_health(&id).await.status,
            HealthStatus::Ready
        );
        broken.store(true, std::sync::atomic::Ordering::Relaxed);
        let failed = manager.get_server_health(&id).await;
        assert_eq!(failed.status, HealthStatus::Unhealthy);
        assert!(!failed.error.unwrap().contains("health-fixture"));
        broken.store(false, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(
            manager.get_server_health(&id).await.status,
            HealthStatus::Ready
        );
        let mut config = manager.get_config(&id).unwrap();
        if let McpTransportConfig::HttpSse { headers, .. } = &mut config.transport_config {
            headers.clear();
        }
        manager.add_config(config);
        assert_eq!(
            manager.get_server_health(&id).await.status,
            HealthStatus::Unhealthy
        );
        task.abort();
    }

    #[tokio::test]
    async fn open_global_socket_does_not_hide_broken_tools() {
        let (manager, id, broken, task) = health_fixture().await;
        manager.start_server(&id).await.unwrap();
        assert_eq!(
            manager.get_server_health(&id).await.status,
            HealthStatus::Healthy
        );
        broken.store(true, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(
            manager.get_server_health(&id).await.status,
            HealthStatus::Unhealthy
        );
        manager.stop_server(&id).await.unwrap();
        task.abort();
    }

    #[tokio::test]
    async fn protocol_health_accepts_stateless_backends_without_initialize() {
        use axum::{http::StatusCode, routing::get, Json, Router};
        let app = Router::new().route("/mcp", get(|| async { StatusCode::METHOD_NOT_ALLOWED }).post(
            |Json(request): Json<JsonRpcRequest>| async move {
                let id = request.id.clone().unwrap_or_default();
                Json(match request.method.as_str() {
                    "initialize" => JsonRpcResponse::error(id, crate::protocol::JsonRpcError::method_not_found("initialize")),
                    "server/discover" => JsonRpcResponse::success(id, serde_json::json!({"protocolVersions":["2026-07-28"], "capabilities":{"tools":{}}})),
                    "tools/list" => {
                        assert_eq!(request.params.unwrap()["_meta"][crate::protocol::meta_keys::PROTOCOL_VERSION], "2026-07-28");
                        JsonRpcResponse::success(id, serde_json::json!({"tools":[]}))
                    },
                    method => panic!("unexpected method {method}"),
                })
            }
        ));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/mcp", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let manager = McpServerManager::new_for_test();
        let config = McpServerConfig::new(
            "stateless".into(),
            McpTransportType::HttpSse,
            McpTransportConfig::HttpSse {
                url,
                headers: HashMap::new(),
            },
        );
        let id = config.id.clone();
        manager.add_config(config);
        assert_eq!(
            manager.get_server_health(&id).await.status,
            HealthStatus::Ready
        );
        task.abort();
    }

    #[tokio::test]
    async fn server_health_refresh_does_not_reuse_or_clear_client_failures() {
        use lr_providers::health_cache::{HealthCacheManager, ItemHealthStatus};
        let (manager, id, broken, task) = health_fixture().await;
        let cache = Arc::new(HealthCacheManager::new());
        manager.set_health_cache(cache.clone());
        let (transports, failures) = manager
            .create_transports(std::slice::from_ref(&id), std::time::Duration::from_secs(2))
            .await;
        assert!(failures.is_empty());
        assert_eq!(
            cache.get().mcp_servers[&id].status,
            ItemHealthStatus::Unhealthy
        );
        // Creating a socket is not readiness; complete this client's handshake.
        transports
            .send_request(&id, JsonRpcRequest::with_id(0, "initialize".into(), None))
            .await
            .unwrap();
        broken.store(true, std::sync::atomic::Ordering::Relaxed);
        let request = JsonRpcRequest::with_id(1, "tools/list".into(), None);
        assert!(transports.send_request(&id, request.clone()).await.is_err());
        assert_eq!(
            cache.get().mcp_servers[&id].status,
            ItemHealthStatus::Unhealthy
        );
        assert_eq!(
            manager.get_server_health(&id).await.status,
            HealthStatus::Unhealthy
        );
        broken.store(false, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(
            manager.check_protocol_readiness(&id).await.0,
            HealthStatus::Ready
        );
        assert_eq!(
            manager.get_server_health(&id).await.status,
            HealthStatus::Ready
        );
        transports
            .send_request(&id, JsonRpcRequest::with_id(2, "prompts/list".into(), None))
            .await
            .unwrap();
        assert_eq!(
            manager.get_server_health(&id).await.status,
            HealthStatus::Ready
        );
        // A successful server probe must not erase this client's discovery failure.
        assert!(transports.failures.read()[&id].contains_key("tools/list"));
        assert_eq!(
            cache.get().mcp_servers[&id].status,
            ItemHealthStatus::Unhealthy
        );
        transports.send_request(&id, request).await.unwrap();
        assert_eq!(
            cache.get().mcp_servers[&id].status,
            ItemHealthStatus::Healthy
        );
        assert_eq!(
            manager.get_server_health(&id).await.status,
            HealthStatus::Ready
        );
        transports.close_all().await;
        task.abort();
    }

    #[tokio::test]
    async fn failed_client_initialize_cannot_latch_server_health() {
        let (manager, id, broken, task) = health_fixture().await;
        let (transports, failures) = manager
            .create_transports(std::slice::from_ref(&id), std::time::Duration::from_secs(2))
            .await;
        assert!(failures.is_empty());
        // Keep the failed session alive, as happens after partial gateway startup.
        transports.record_failure(
            &id,
            "initialize",
            "MCP connection failed during initialize; check authentication and server availability",
        );
        assert_eq!(
            manager.get_server_health(&id).await.status,
            HealthStatus::Ready
        );
        assert_eq!(
            manager.get_server_health(&id).await.status,
            HealthStatus::Ready
        );
        assert!(transports.failures.read()[&id].contains_key("initialize"));
        broken.store(true, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(
            manager.get_server_health(&id).await.status,
            HealthStatus::Unhealthy
        );
        broken.store(false, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(
            manager.get_server_health(&id).await.status,
            HealthStatus::Ready
        );
        transports.close_all().await;
        task.abort();
    }

    #[test]
    fn test_manager_creation() {
        let manager = McpServerManager::new();
        assert_eq!(manager.list_configs().len(), 0);
    }

    #[test]
    fn test_add_remove_config() {
        let manager = McpServerManager::new();

        let config = McpServerConfig::new(
            "Test Server".to_string(),
            McpTransportType::Stdio,
            McpTransportConfig::Stdio {
                command: "echo".to_string(),
                args: vec![],
                env: HashMap::new(),
                cwd: None,
            },
        );

        let server_id = config.id.clone();

        manager.add_config(config.clone());
        assert_eq!(manager.list_configs().len(), 1);
        assert!(manager.get_config(&server_id).is_some());

        manager.remove_config(&server_id);
        assert_eq!(manager.list_configs().len(), 0);
        assert!(manager.get_config(&server_id).is_none());
    }

    #[tokio::test]
    async fn non_mcp_executable_is_not_reported_ready() {
        let manager = McpServerManager::new();

        // echo is a valid command that exits immediately
        let config = McpServerConfig::new(
            "Test Server".to_string(),
            McpTransportType::Stdio,
            McpTransportConfig::Stdio {
                command: "echo".to_string(),
                args: vec!["hello".to_string()],
                env: HashMap::new(),
                cwd: None,
            },
        );

        let server_id = config.id.clone();
        manager.add_config(config);

        let health = manager.get_server_health(&server_id).await;
        // Process output alone is not a successful MCP handshake.
        assert_eq!(health.status, HealthStatus::Unhealthy);
    }

    #[tokio::test]
    async fn test_health_check_invalid_command() {
        let manager = McpServerManager::new();

        // nonexistent command should fail
        let config = McpServerConfig::new(
            "Test Server".to_string(),
            McpTransportType::Stdio,
            McpTransportConfig::Stdio {
                command: "this-command-definitely-does-not-exist-12345".to_string(),
                args: vec![],
                env: HashMap::new(),
                cwd: None,
            },
        );

        let server_id = config.id.clone();
        manager.add_config(config);

        let health = manager.get_server_health(&server_id).await;
        assert_eq!(health.status, HealthStatus::Unhealthy);
        assert!(health.error.is_some());
        assert!(health.error.unwrap().contains("not found"));
    }

    #[tokio::test]
    async fn test_try_spawn_command_valid() {
        // Test with a simple command that produces output
        let result = McpServerManager::try_spawn_command(
            "echo",
            &["test output".to_string()],
            &HashMap::new(),
            None,
        )
        .await;

        assert!(result.is_ok(), "echo should succeed: {:?}", result);
    }

    /// The readiness check must run in the configured working directory, so a
    /// server pinned to a directory that no longer exists reports unhealthy
    /// instead of silently starting somewhere else.
    #[tokio::test]
    async fn test_try_spawn_command_rejects_missing_cwd() {
        let missing = std::path::Path::new("/definitely/not/a/real/directory-12345");
        let result = McpServerManager::try_spawn_command(
            "echo",
            &["hi".to_string()],
            &HashMap::new(),
            Some(missing),
        )
        .await;

        let err = result.unwrap_err();
        assert!(
            err.contains("Working directory does not exist"),
            "unexpected error: {}",
            err
        );
    }

    #[tokio::test]
    async fn test_try_spawn_command_accepts_existing_cwd() {
        let dir = std::env::temp_dir();
        let result = McpServerManager::try_spawn_command(
            "echo",
            &["hi".to_string()],
            &HashMap::new(),
            Some(dir.as_path()),
        )
        .await;

        assert!(
            result.is_ok(),
            "echo in temp dir should succeed: {:?}",
            result
        );
    }

    /// A STDIO server with no configured `cwd` resolves to the home directory
    /// rather than inheriting LocalRouter's own (which is `/` on a GUI launch).
    #[test]
    fn test_stdio_config_defaults_to_home_dir() {
        let config = McpServerConfig::new(
            "Test Server".to_string(),
            McpTransportType::Stdio,
            McpTransportConfig::Stdio {
                command: "echo".to_string(),
                args: vec![],
                env: HashMap::new(),
                cwd: None,
            },
        );

        assert_eq!(
            config.transport_config.resolve_stdio_cwd(),
            lr_config::default_mcp_working_dir()
        );
    }

    #[tokio::test]
    async fn test_try_spawn_command_not_found() {
        // Test with a nonexistent command
        let result = McpServerManager::try_spawn_command(
            "this-command-definitely-does-not-exist-xyz",
            &[],
            &HashMap::new(),
            None,
        )
        .await;

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.contains("not found") || err.contains("No such file"),
            "Error should mention command not found: {}",
            err
        );
    }

    #[tokio::test]
    #[ignore] // This test requires npx/npm to be installed
    async fn test_try_spawn_npx_valid_package() {
        // Test with a real MCP server package
        // This will download the package if not cached, so it may take a while
        let result = McpServerManager::try_spawn_command(
            "npx",
            &[
                "-y".to_string(),
                "@modelcontextprotocol/server-everything".to_string(),
            ],
            &HashMap::new(),
            None,
        )
        .await;

        assert!(
            result.is_ok(),
            "npx @modelcontextprotocol/server-everything should succeed: {:?}",
            result
        );
    }

    #[tokio::test]
    #[ignore] // This test requires npx/npm to be installed
    async fn test_try_spawn_npx_invalid_package() {
        // Test with a nonexistent npm package
        let result = McpServerManager::try_spawn_command(
            "npx",
            &[
                "-y".to_string(),
                "@nonexistent/package-that-does-not-exist-12345".to_string(),
            ],
            &HashMap::new(),
            None,
        )
        .await;

        assert!(
            result.is_err(),
            "npx with invalid package should fail: {:?}",
            result
        );
    }

    #[tokio::test]
    async fn test_try_spawn_command_no_stdout_output() {
        // A command that exits immediately without producing stdout output.
        // This simulates stdio MCP servers (like `netget --mcp`) that close
        // when they don't receive an initialize request on stdin.
        // `true` exits with code 0 but produces no output.
        let result = McpServerManager::try_spawn_command("true", &[], &HashMap::new(), None).await;

        assert!(
            result.is_ok(),
            "command that exits cleanly without output should be considered ready: {:?}",
            result
        );
    }

    #[tokio::test]
    async fn test_try_spawn_command_keeps_stdin_open_for_handshake() {
        // Like many stdio MCP servers: read the initialize request from stdin
        // and fail if stdin closes first.
        let result = McpServerManager::try_spawn_command(
            "sh",
            &[
                "-c".to_string(),
                "read -r line || { echo 'Error: connection closed: initialize request' >&2; exit 1; }; echo \"$line\"".to_string(),
            ],
            &std::collections::HashMap::new(),
            None,
        )
        .await;
        assert!(
            result.is_ok(),
            "probe closed stdin before the handshake: {result:?}"
        );
    }

    #[tokio::test]
    async fn test_try_spawn_command_explains_early_exit() {
        let result = McpServerManager::try_spawn_command(
            "sh",
            &[
                "-c".to_string(),
                "printf '\\033[31mfatal: missing API key\\033[0m\\n' >&2; exit 3".to_string(),
            ],
            &std::collections::HashMap::new(),
            None,
        )
        .await;
        let err = result.unwrap_err();
        assert!(err.contains("stopped on its own"), "{err}");
        assert!(err.contains("missing API key"), "{err}");
    }

    #[test]
    fn test_describe_early_exit() {
        let stderr = "\u{1b}[2m2026-10-13T18:03:04Z\u{1b}[0m \u{1b}[32m INFO\u{1b}[0m Starting\n\
                      Error: connection closed: initialize request\n";
        assert_eq!(
            describe_early_exit("exit status: 1", stderr),
            "The server process stopped on its own (exit status: 1) before it could be used. \
             It reported: Error: connection closed: initialize request"
        );
        // No stderr: just the status
        assert_eq!(
            describe_early_exit("exit status: 2", ""),
            "The server process stopped on its own (exit status: 2) before it could be used"
        );
        // Without an error-looking line, the last line is quoted
        assert!(
            describe_early_exit("exit status: 1", "starting\nbye\n").ends_with("It reported: bye")
        );
    }

    #[test]
    fn test_strip_ansi() {
        assert_eq!(strip_ansi("\u{1b}[33m WARN\u{1b}[0m x"), " WARN x");
        assert_eq!(strip_ansi("plain"), "plain");
    }
}
