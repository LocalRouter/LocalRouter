//! Server manager for controlling the web server lifecycle

use parking_lot::RwLock;
use std::sync::Arc;
use tokio::task::JoinHandle;
use tracing::info;

use super::{start_server, state::AppState, ServerConfig};
use crate::mcp::McpServerManager;
use crate::providers::registry::ProviderRegistry;
use crate::router::{RateLimiterManager, Router};

/// Dependencies needed to start the server
pub struct ServerDependencies {
    pub router: Arc<Router>,
    pub mcp_server_manager: Arc<McpServerManager>,
    pub rate_limiter: Arc<RateLimiterManager>,
    pub provider_registry: Arc<ProviderRegistry>,
    pub config_manager: Arc<crate::config::ConfigManager>,
    pub client_manager: Arc<crate::clients::ClientManager>,
    pub token_store: Arc<crate::clients::TokenStore>,
}

/// Server status
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerStatus {
    Stopped,
    Running,
}

/// Manages the web server task
pub struct ServerManager {
    app_state: Arc<RwLock<Option<AppState>>>,
    server_handle: Arc<RwLock<Option<JoinHandle<()>>>>,
    status: Arc<RwLock<ServerStatus>>,
    actual_port: Arc<RwLock<Option<u16>>>,
}

impl ServerManager {
    pub fn new() -> Self {
        Self {
            app_state: Arc::new(RwLock::new(None)),
            server_handle: Arc::new(RwLock::new(None)),
            status: Arc::new(RwLock::new(ServerStatus::Stopped)),
            actual_port: Arc::new(RwLock::new(None)),
        }
    }

    /// Get the actual port the server is running on
    pub fn get_actual_port(&self) -> Option<u16> {
        *self.actual_port.read()
    }

    /// Start the web server
    pub async fn start(
        &self,
        config: ServerConfig,
        deps: ServerDependencies,
    ) -> anyhow::Result<()> {
        // Stop any existing server first
        if *self.status.read() == ServerStatus::Running {
            info!("Stopping existing server before restart");
            self.stop().await;

            // Give the OS time to release the port
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        }

        // Start the new server
        let (state, handle, actual_port) = start_server(
            config,
            deps.router,
            deps.mcp_server_manager,
            deps.rate_limiter,
            deps.provider_registry,
            deps.config_manager,
            deps.client_manager,
            deps.token_store,
        )
        .await?;

        // Update to the new server
        *self.app_state.write() = Some(state);
        *self.server_handle.write() = Some(handle);
        *self.actual_port.write() = Some(actual_port);
        *self.status.write() = ServerStatus::Running;

        info!("Server started successfully on port {}", actual_port);
        Ok(())
    }

    /// Stop the web server
    pub async fn stop(&self) {
        if *self.status.read() == ServerStatus::Stopped {
            info!("Server is already stopped");
            return;
        }

        info!("Stopping server...");

        // Abort the server task
        if let Some(handle) = self.server_handle.write().take() {
            handle.abort();
        }

        // Clear the app state
        *self.app_state.write() = None;
        *self.actual_port.write() = None;
        *self.status.write() = ServerStatus::Stopped;

        info!("Server stopped");
    }

    /// Get the server status
    pub fn get_status(&self) -> ServerStatus {
        *self.status.read()
    }

    /// Get the app state
    pub fn get_state(&self) -> Option<AppState> {
        self.app_state.read().clone()
    }
}

impl Default for ServerManager {
    fn default() -> Self {
        Self::new()
    }
}
