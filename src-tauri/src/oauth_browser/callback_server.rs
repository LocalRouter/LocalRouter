//! Multi-port OAuth callback server manager
//!
//! Manages local HTTP servers for OAuth callback redirects with support for
//! multiple concurrent flows on different ports or sharing the same port.

use crate::oauth_browser::FlowId;
use crate::utils::errors::{AppError, AppResult};
use axum::{
    extract::Query,
    http::StatusCode,
    response::{Html, IntoResponse},
    Router,
};
use parking_lot::Mutex;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::oneshot;
use tracing::{error, info, warn};

/// Query parameters for OAuth callback
#[derive(Debug, Deserialize)]
struct OAuthCallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

/// Result of an OAuth callback
#[derive(Debug, Clone)]
pub struct CallbackResult {
    /// Authorization code
    pub code: String,
    /// State parameter (for verification)
    pub state: String,
}

/// A registered callback listener for a specific flow
struct CallbackListener {
    /// Flow ID
    #[allow(dead_code)]
    flow_id: FlowId,
    /// Expected CSRF state parameter
    expected_state: String,
    /// Channel to send result
    result_tx: Option<oneshot::Sender<AppResult<CallbackResult>>>,
}

/// Active server on a specific port
struct ActiveServer {
    /// Port number
    #[allow(dead_code)]
    port: u16,
    /// Registered listeners by flow ID
    listeners: HashMap<FlowId, CallbackListener>,
    /// Shutdown signal
    _shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
}

/// Manager for OAuth callback servers
///
/// Manages multiple callback servers on different ports and routes callbacks
/// to the appropriate flow based on the CSRF state parameter.
pub struct CallbackServerManager {
    /// Active servers by port
    active_servers: Arc<Mutex<HashMap<u16, Arc<Mutex<ActiveServer>>>>>,
}

impl CallbackServerManager {
    /// Create a new callback server manager
    pub fn new() -> Self {
        Self {
            active_servers: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Register a callback listener for a flow
    ///
    /// If a server is not already running on the specified port, one will be started.
    /// Multiple flows can share the same port; callbacks are routed by state parameter.
    ///
    /// # Arguments
    /// * `flow_id` - Unique identifier for this flow
    /// * `port` - Port number for callback server (e.g., 8080, 1455, 1456)
    /// * `expected_state` - CSRF state parameter to validate
    ///
    /// # Returns
    /// * Receiver channel that will receive the callback result or error
    pub async fn register_listener(
        &self,
        flow_id: FlowId,
        port: u16,
        expected_state: String,
    ) -> AppResult<oneshot::Receiver<AppResult<CallbackResult>>> {
        let (tx, rx) = oneshot::channel();

        let listener = CallbackListener {
            flow_id,
            expected_state: expected_state.clone(),
            result_tx: Some(tx),
        };

        // Get or create server for this port
        let server_exists = {
            let servers = self.active_servers.lock();
            servers.get(&port).is_some()
        };

        if server_exists {
            // Server already exists, add listener
            info!(
                "Adding listener for flow {} to existing server on port {}",
                flow_id, port
            );
            let servers = self.active_servers.lock();
            if let Some(server) = servers.get(&port) {
                server.lock().listeners.insert(flow_id, listener);
            }
        } else {
            // Start new server
            info!("Starting new OAuth callback server on port {}", port);

            let server_state = Arc::new(Mutex::new(ActiveServer {
                port,
                listeners: HashMap::from([(flow_id, listener)]),
                _shutdown_tx: None,
            }));

            // Start the HTTP server
            self.start_server(port, Arc::clone(&server_state)).await?;

            // Insert into map after await
            let mut servers = self.active_servers.lock();
            servers.insert(port, server_state);
        }

        Ok(rx)
    }

    /// Start an HTTP server on the specified port
    async fn start_server(
        &self,
        port: u16,
        server_state: Arc<Mutex<ActiveServer>>,
    ) -> AppResult<()> {
        // Create callback handler
        let callback_handler = {
            let server_state = Arc::clone(&server_state);

            move |Query(params): Query<OAuthCallbackQuery>| {
                let server_state = Arc::clone(&server_state);

                async move {
                    // Check for OAuth error response
                    if let Some(error) = params.error {
                        let description = params
                            .error_description
                            .unwrap_or_else(|| "Unknown error".to_string());
                        error!("OAuth authorization failed: {} - {}", error, description);

                        return (
                            StatusCode::BAD_REQUEST,
                            Html(format!(
                                r#"
                                <html>
                                    <head><title>Authorization Failed</title></head>
                                    <body style="font-family: sans-serif; text-align: center; padding: 50px;">
                                        <h1>❌ Authorization Failed</h1>
                                        <p>Error: {}</p>
                                        <p>Description: {}</p>
                                        <p>You can close this window and return to LocalRouter AI.</p>
                                    </body>
                                </html>
                                "#,
                                error, description
                            )),
                        )
                            .into_response();
                    }

                    // Extract authorization code
                    let code = match params.code {
                        Some(c) => c,
                        None => {
                            return (
                                StatusCode::BAD_REQUEST,
                                Html(
                                    r#"<html><body style="font-family: sans-serif; text-align: center; padding: 50px;">
                                        <h1>❌ Error</h1>
                                        <p>No authorization code received</p>
                                    </body></html>"#,
                                ),
                            )
                                .into_response();
                        }
                    };

                    // Extract and validate state
                    let state = match params.state {
                        Some(s) => s,
                        None => {
                            return (
                                StatusCode::BAD_REQUEST,
                                Html(
                                    r#"<html><body style="font-family: sans-serif; text-align: center; padding: 50px;">
                                        <h1>❌ Error</h1>
                                        <p>No state parameter received</p>
                                    </body></html>"#,
                                ),
                            )
                                .into_response();
                        }
                    };

                    // Find matching listener by state
                    let mut server = server_state.lock();
                    let matching_listener = server
                        .listeners
                        .iter_mut()
                        .find(|(_, listener)| listener.expected_state == state);

                    match matching_listener {
                        Some((flow_id, listener)) => {
                            info!("Matched callback to flow {}", flow_id);

                            // Send result through channel
                            if let Some(sender) = listener.result_tx.take() {
                                let result = CallbackResult {
                                    code: code.clone(),
                                    state: state.clone(),
                                };

                                if sender.send(Ok(result)).is_err() {
                                    error!(
                                        "Failed to send OAuth callback result for flow {}",
                                        flow_id
                                    );
                                }
                            }

                            // Return success page
                            (
                                StatusCode::OK,
                                Html(
                                    r#"
                                    <html>
                                        <head><title>Authorization Successful</title></head>
                                        <body style="font-family: sans-serif; text-align: center; padding: 50px;">
                                            <h1>✅ Authorization Successful!</h1>
                                            <p>You have successfully authorized the application.</p>
                                            <p>You can close this window and return to LocalRouter AI.</p>
                                            <script>
                                                setTimeout(function() { window.close(); }, 3000);
                                            </script>
                                        </body>
                                    </html>
                                    "#,
                                ),
                            )
                                .into_response()
                        }
                        None => {
                            warn!("Received callback with unknown state: {}", state);
                            (
                                StatusCode::BAD_REQUEST,
                                Html(
                                    r#"<html><body style="font-family: sans-serif; text-align: center; padding: 50px;">
                                        <h1>❌ Error</h1>
                                        <p>Invalid state parameter (CSRF protection)</p>
                                        <p>This may indicate a security issue or an expired authorization request.</p>
                                    </body></html>"#,
                                ),
                            )
                                .into_response()
                        }
                    }
                }
            }
        };

        // Build router
        let app = Router::new().route("/callback", axum::routing::get(callback_handler));

        // Start server
        let addr = format!("127.0.0.1:{}", port);
        info!("Binding OAuth callback server to http://{}/callback", addr);

        let listener = tokio::net::TcpListener::bind(&addr).await.map_err(|e| {
            AppError::OAuthBrowser(format!(
                "Failed to bind callback server on port {}: {}",
                port, e
            ))
        })?;

        // Spawn server in background
        tokio::spawn(async move {
            if let Err(e) = axum::serve(listener, app).await {
                error!("OAuth callback server error on port {}: {}", port, e);
            }
        });

        info!(
            "OAuth callback server started successfully on port {}",
            port
        );
        Ok(())
    }

    /// Cancel a specific flow
    ///
    /// Removes the listener for the specified flow. If this was the last listener
    /// on the port, the server continues running (it's lightweight and stateless).
    pub fn cancel_flow(&self, flow_id: FlowId, port: u16) {
        let mut servers = self.active_servers.lock();

        if let Some(server) = servers.get(&port) {
            let mut server = server.lock();
            if server.listeners.remove(&flow_id).is_some() {
                info!("Cancelled listener for flow {} on port {}", flow_id, port);
            }

            // Clean up empty servers
            if server.listeners.is_empty() {
                drop(server);
                servers.remove(&port);
                info!("Removed empty server on port {}", port);
            }
        }
    }

    /// Get count of active servers
    #[allow(dead_code)]
    pub fn active_server_count(&self) -> usize {
        self.active_servers.lock().len()
    }

    /// Get count of active listeners across all servers
    #[allow(dead_code)]
    pub fn active_listener_count(&self) -> usize {
        self.active_servers
            .lock()
            .values()
            .map(|server| server.lock().listeners.len())
            .sum()
    }
}

impl Default for CallbackServerManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_callback_server_manager_creation() {
        let manager = CallbackServerManager::new();
        assert_eq!(manager.active_server_count(), 0);
        assert_eq!(manager.active_listener_count(), 0);
    }

    #[tokio::test]
    async fn test_register_listener() {
        let manager = CallbackServerManager::new();
        let flow_id = FlowId::new();
        let port = 8888;
        let state = "test_state".to_string();

        let _rx = manager
            .register_listener(flow_id, port, state)
            .await
            .expect("Failed to register listener");

        assert_eq!(manager.active_server_count(), 1);
        assert_eq!(manager.active_listener_count(), 1);
    }

    #[tokio::test]
    async fn test_multiple_listeners_same_port() {
        let manager = CallbackServerManager::new();
        let flow_id1 = FlowId::new();
        let flow_id2 = FlowId::new();
        let port = 8889;

        let _rx1 = manager
            .register_listener(flow_id1, port, "state1".to_string())
            .await
            .expect("Failed to register listener 1");

        let _rx2 = manager
            .register_listener(flow_id2, port, "state2".to_string())
            .await
            .expect("Failed to register listener 2");

        assert_eq!(manager.active_server_count(), 1);
        assert_eq!(manager.active_listener_count(), 2);
    }

    #[tokio::test]
    async fn test_cancel_flow() {
        let manager = CallbackServerManager::new();
        let flow_id = FlowId::new();
        let port = 8890;

        let _rx = manager
            .register_listener(flow_id, port, "test_state".to_string())
            .await
            .expect("Failed to register listener");

        assert_eq!(manager.active_listener_count(), 1);

        manager.cancel_flow(flow_id, port);

        assert_eq!(manager.active_listener_count(), 0);
        assert_eq!(manager.active_server_count(), 0);
    }
}
