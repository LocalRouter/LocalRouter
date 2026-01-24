//! MCP server lifecycle management
//!
//! Manages MCP server instances, their lifecycle, and health checks.

#![allow(dead_code)]

use crate::api_keys::keychain_trait::KeychainStorage;
use crate::config::{McpServerConfig, McpTransportConfig, McpTransportType};
use crate::mcp::oauth::McpOAuthManager;
use crate::mcp::protocol::{JsonRpcNotification, JsonRpcRequest, JsonRpcResponse, StreamingChunk};
use crate::mcp::transport::{SseTransport, StdioTransport, Transport, WebSocketTransport};
use crate::utils::errors::{AppError, AppResult};
use dashmap::DashMap;
use futures_util::stream::Stream;
use serde::{Deserialize, Serialize};
use std::pin::Pin;
use std::sync::Arc;

/// Notification callback type
///
/// Called when a notification is received from an MCP server.
/// Receives the server_id and the notification.
pub type NotificationCallback = Arc<dyn Fn(String, JsonRpcNotification) + Send + Sync>;

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
    Healthy,
    Unhealthy,
    Unknown,
}

impl McpServerManager {
    /// Create a new MCP server manager
    pub fn new() -> Self {
        Self {
            stdio_transports: Arc::new(DashMap::new()),
            sse_transports: Arc::new(DashMap::new()),
            websocket_transports: Arc::new(DashMap::new()),
            configs: Arc::new(DashMap::new()),
            oauth_manager: Arc::new(McpOAuthManager::new()),
            notification_handlers: Arc::new(DashMap::new()),
            next_handler_id: Arc::new(std::sync::atomic::AtomicU64::new(1)),
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
    pub fn set_request_callback(&self, server_id: &str, callback: crate::mcp::transport::StdioRequestCallback) -> bool {
        if let Some(transport) = self.stdio_transports.get(server_id) {
            transport.set_request_callback(callback);
            tracing::info!("Set request callback for STDIO server: {}", server_id);
            true
        } else {
            tracing::warn!(
                "Cannot set request callback - server {} is not a STDIO transport",
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
        match config.transport {
            McpTransportType::Stdio => {
                self.start_stdio_server(server_id, &config).await?;
            }
            McpTransportType::Sse | McpTransportType::HttpSse => {
                self.start_sse_server(server_id, &config).await?;
            }
            McpTransportType::WebSocket => {
                self.start_websocket_server(server_id, &config).await?;
            }
        }

        tracing::info!("MCP server started: {}", server_id);
        Ok(())
    }

    /// Start a STDIO MCP server
    async fn start_stdio_server(&self, server_id: &str, config: &McpServerConfig) -> AppResult<()> {
        // Extract STDIO config
        let (command, args, mut env) = match &config.transport_config {
            McpTransportConfig::Stdio { command, args, env } => {
                (command.clone(), args.clone(), env.clone())
            }
            _ => {
                return Err(AppError::Mcp(
                    "Invalid transport config for STDIO".to_string(),
                ))
            }
        };

        // Merge auth config environment variables (if specified)
        if let Some(crate::config::McpAuthConfig::EnvVars { env: auth_env }) = &config.auth_config {
            // Merge auth env vars with base env vars
            // Auth env vars override base env vars
            for (key, value) in auth_env {
                env.insert(key.clone(), value.clone());
            }
            tracing::debug!("Applied auth env vars for STDIO server: {}", server_id);
        }

        // Spawn the STDIO transport
        let transport = StdioTransport::spawn(command, args, env).await?;

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

        // Apply auth config (if specified)
        if let Some(auth_config) = &config.auth_config {
            match auth_config {
                crate::config::McpAuthConfig::BearerToken { token_ref: _ } => {
                    // Retrieve token from keychain
                    let keychain = crate::api_keys::CachedKeychain::auto()
                        .unwrap_or_else(|_| crate::api_keys::CachedKeychain::system());
                    // Token is stored with account name: {server_id}_bearer_token
                    let account_name = format!("{}_bearer_token", config.id);
                    if let Ok(Some(token)) = keychain.get("LocalRouter-McpServers", &account_name) {
                        headers.insert("Authorization".to_string(), format!("Bearer {}", token));
                        tracing::debug!("Applied bearer token auth for SSE server: {}", server_id);
                    } else {
                        tracing::warn!(
                            "Bearer token not found in keychain for server: {} (tried account: {})",
                            server_id,
                            account_name
                        );
                    }
                }
                crate::config::McpAuthConfig::CustomHeaders {
                    headers: auth_headers,
                } => {
                    // Merge custom auth headers with base headers
                    // Auth headers override base headers
                    for (key, value) in auth_headers {
                        headers.insert(key.clone(), value.clone());
                    }
                    tracing::debug!("Applied custom headers auth for SSE server: {}", server_id);
                }
                crate::config::McpAuthConfig::OAuth {
                    client_id,
                    client_secret_ref,
                    token_url,
                    scopes,
                    ..
                } => {
                    // Get keychain
                    let keychain = crate::api_keys::CachedKeychain::auto()
                        .unwrap_or_else(|_| crate::api_keys::CachedKeychain::system());

                    // Get client secret from keychain
                    let client_secret = match keychain
                        .get("LocalRouter-McpServers", client_secret_ref)
                    {
                        Ok(Some(secret)) => secret,
                        Ok(None) => {
                            tracing::warn!(
                                "OAuth client secret not found in keychain for server: {} (account: {})",
                                server_id,
                                client_secret_ref
                            );
                            return Err(AppError::Mcp(format!(
                                "OAuth client secret not found for server: {}",
                                server_id
                            )));
                        }
                        Err(e) => {
                            tracing::error!("Failed to retrieve OAuth client secret: {}", e);
                            return Err(e);
                        }
                    };

                    // Acquire OAuth token via Client Credentials flow
                    tracing::debug!("Acquiring OAuth token for SSE server: {}", server_id);

                    // Build token request
                    let client = reqwest::Client::new();
                    let mut form_params = vec![
                        ("grant_type", "client_credentials"),
                        ("client_id", client_id.as_str()),
                        ("client_secret", client_secret.as_str()),
                    ];

                    // Add scopes if provided
                    let scopes_str = scopes.join(" ");
                    if !scopes.is_empty() {
                        form_params.push(("scope", scopes_str.as_str()));
                    }

                    // Send token request
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

                    // Parse token response
                    let token_json: serde_json::Value =
                        token_response.json().await.map_err(|e| {
                            AppError::Mcp(format!("Failed to parse OAuth token response: {}", e))
                        })?;

                    let access_token = token_json
                        .get("access_token")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| {
                            AppError::Mcp("OAuth response missing access_token".to_string())
                        })?;

                    // Add Authorization header with token
                    headers.insert(
                        "Authorization".to_string(),
                        format!("Bearer {}", access_token),
                    );

                    tracing::info!("Applied OAuth token for SSE server: {}", server_id);
                }
                crate::config::McpAuthConfig::OAuthBrowser { .. } => {
                    // OAuth browser flow - token should already be stored in keychain
                    // by the McpOAuthBrowserManager after successful authentication
                    let keychain = crate::api_keys::CachedKeychain::auto()
                        .unwrap_or_else(|_| crate::api_keys::CachedKeychain::system());

                    // Try to get access token from keychain
                    let account_name = format!("{}_access_token", config.id);
                    match keychain.get("LocalRouter-McpServerTokens", &account_name) {
                        Ok(Some(token)) => {
                            headers
                                .insert("Authorization".to_string(), format!("Bearer {}", token));
                            tracing::debug!(
                                "Applied OAuth browser token for SSE server: {}",
                                server_id
                            );
                        }
                        Ok(None) => {
                            tracing::warn!(
                                "OAuth browser token not found in keychain for server: {}. User must authenticate via browser first.",
                                server_id
                            );
                            return Err(AppError::Mcp(format!(
                                "OAuth browser authentication required for server: {}. Please complete browser authentication first.",
                                server_id
                            )));
                        }
                        Err(e) => {
                            tracing::error!("Failed to retrieve OAuth browser token: {}", e);
                            return Err(e);
                        }
                    }
                }
                _ => {
                    // None or EnvVars (not applicable for SSE)
                    tracing::debug!("No applicable auth config for SSE server: {}", server_id);
                }
            }
        }

        // Connect to the SSE server
        let transport = SseTransport::connect(url, headers).await?;

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

        // Apply auth config (if specified)
        if let Some(auth_config) = &config.auth_config {
            match auth_config {
                crate::config::McpAuthConfig::BearerToken { token_ref: _ } => {
                    // Retrieve token from keychain
                    let keychain = crate::api_keys::CachedKeychain::auto()
                        .unwrap_or_else(|_| crate::api_keys::CachedKeychain::system());
                    let account_name = format!("{}_bearer_token", config.id);
                    if let Ok(Some(token)) = keychain.get("LocalRouter-McpServers", &account_name) {
                        headers.insert("Authorization".to_string(), format!("Bearer {}", token));
                        tracing::debug!(
                            "Applied bearer token auth for WebSocket server: {}",
                            server_id
                        );
                    } else {
                        tracing::warn!(
                            "Bearer token not found in keychain for server: {} (tried account: {})",
                            server_id,
                            account_name
                        );
                    }
                }
                crate::config::McpAuthConfig::CustomHeaders {
                    headers: auth_headers,
                } => {
                    for (key, value) in auth_headers {
                        headers.insert(key.clone(), value.clone());
                    }
                    tracing::debug!(
                        "Applied custom headers auth for WebSocket server: {}",
                        server_id
                    );
                }
                crate::config::McpAuthConfig::OAuth {
                    client_id,
                    client_secret_ref,
                    token_url,
                    scopes,
                    ..
                } => {
                    // Get keychain
                    let keychain = crate::api_keys::CachedKeychain::auto()
                        .unwrap_or_else(|_| crate::api_keys::CachedKeychain::system());

                    // Get client secret from keychain
                    let client_secret = match keychain
                        .get("LocalRouter-McpServers", client_secret_ref)
                    {
                        Ok(Some(secret)) => secret,
                        Ok(None) => {
                            tracing::warn!(
                                "OAuth client secret not found in keychain for server: {} (account: {})",
                                server_id,
                                client_secret_ref
                            );
                            return Err(AppError::Mcp(format!(
                                "OAuth client secret not found for server: {}",
                                server_id
                            )));
                        }
                        Err(e) => {
                            tracing::error!("Failed to retrieve OAuth client secret: {}", e);
                            return Err(e);
                        }
                    };

                    // Acquire OAuth token
                    tracing::debug!("Acquiring OAuth token for WebSocket server: {}", server_id);

                    let client = reqwest::Client::new();
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

                    let token_json: serde_json::Value =
                        token_response.json().await.map_err(|e| {
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

                    tracing::info!("Applied OAuth token for WebSocket server: {}", server_id);
                }
                _ => {
                    tracing::debug!(
                        "No applicable auth config for WebSocket server: {}",
                        server_id
                    );
                }
            }
        }

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

        let (status, error) = if let Some(transport) = self.stdio_transports.get(server_id) {
            if transport.is_alive() {
                (HealthStatus::Healthy, None)
            } else {
                (
                    HealthStatus::Unhealthy,
                    Some("Process not running".to_string()),
                )
            }
        } else if let Some(transport) = self.sse_transports.get(server_id) {
            if transport.is_healthy() {
                (HealthStatus::Healthy, None)
            } else {
                (
                    HealthStatus::Unhealthy,
                    Some("SSE connection lost".to_string()),
                )
            }
        } else if let Some(transport) = self.websocket_transports.get(server_id) {
            if transport.is_healthy() {
                (HealthStatus::Healthy, None)
            } else {
                (
                    HealthStatus::Unhealthy,
                    Some("WebSocket connection lost".to_string()),
                )
            }
        } else {
            (HealthStatus::Unhealthy, Some("Not started".to_string()))
        };

        McpServerHealth {
            server_id: server_id.to_string(),
            server_name: config
                .map(|c| c.name)
                .unwrap_or_else(|| "Unknown".to_string()),
            status,
            error,
            last_check: chrono::Utc::now(),
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
            }
        }
    }
}

impl Default for McpServerManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::McpServerConfig;
    use std::collections::HashMap;

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
    async fn test_health_check_not_running() {
        let manager = McpServerManager::new();

        let config = McpServerConfig::new(
            "Test Server".to_string(),
            McpTransportType::Stdio,
            McpTransportConfig::Stdio {
                command: "echo".to_string(),
                args: vec![],
                env: HashMap::new(),
            },
        );

        let server_id = config.id.clone();
        manager.add_config(config);

        let health = manager.get_server_health(&server_id).await;
        assert_eq!(health.status, HealthStatus::Unhealthy);
        assert!(health.error.is_some());
    }
}
