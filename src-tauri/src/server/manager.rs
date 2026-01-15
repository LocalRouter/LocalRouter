//! Server manager for controlling the web server lifecycle

use std::sync::Arc;
use parking_lot::RwLock;
use tokio::task::JoinHandle;
use tracing::info;

use crate::api_keys::ApiKeyManager;
use crate::providers::registry::ProviderRegistry;
use crate::router::{RateLimiterManager, Router};
use super::{ServerConfig, start_server, state::AppState};

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
}

impl ServerManager {
    pub fn new() -> Self {
        Self {
            app_state: Arc::new(RwLock::new(None)),
            server_handle: Arc::new(RwLock::new(None)),
            status: Arc::new(RwLock::new(ServerStatus::Stopped)),
        }
    }

    /// Start the web server
    pub async fn start(
        &self,
        config: ServerConfig,
        router: Arc<Router>,
        api_key_manager: ApiKeyManager,
        rate_limiter: Arc<RateLimiterManager>,
        provider_registry: Arc<ProviderRegistry>,
    ) -> anyhow::Result<()> {
        // Check if already running
        if *self.status.read() == ServerStatus::Running {
            info!("Server is already running");
            return Ok(());
        }

        // Stop any existing server first
        self.stop().await;

        let (state, handle) = start_server(
            config,
            router,
            api_key_manager,
            rate_limiter,
            provider_registry,
        )
        .await?;

        *self.app_state.write() = Some(state);
        *self.server_handle.write() = Some(handle);
        *self.status.write() = ServerStatus::Running;

        info!("Server started successfully");
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
