//! McpViaLlmManager - entry point for MCP via LLM mode
//!
//! Manages sessions and dispatches requests to the agentic orchestrator.

use std::sync::Arc;
use std::time::Duration;

use dashmap::DashMap;
use parking_lot::RwLock;

use lr_config::{Client, McpViaLlmConfig};
use lr_mcp::McpGateway;
use lr_providers::CompletionRequest;
use lr_router::Router;

use crate::orchestrator;
use crate::session::McpViaLlmSession;

/// Manages MCP via LLM sessions and orchestrates agentic tool execution
pub struct McpViaLlmManager {
    /// Sessions indexed by client_id
    sessions_by_client: DashMap<String, Vec<Arc<RwLock<McpViaLlmSession>>>>,
    /// Configuration
    config: RwLock<McpViaLlmConfig>,
}

impl McpViaLlmManager {
    pub fn new(config: McpViaLlmConfig) -> Self {
        Self {
            sessions_by_client: DashMap::new(),
            config: RwLock::new(config),
        }
    }

    pub fn update_config(&self, config: McpViaLlmConfig) {
        *self.config.write() = config;
    }

    pub fn config(&self) -> McpViaLlmConfig {
        self.config.read().clone()
    }

    /// Get an existing session or create a new one for this client.
    /// Phase 1: one session per client (simplest matching strategy).
    fn get_or_create_session(&self, client_id: &str) -> Arc<RwLock<McpViaLlmSession>> {
        let ttl = Duration::from_secs(self.config.read().session_ttl_seconds);

        let mut sessions = self
            .sessions_by_client
            .entry(client_id.to_string())
            .or_default();

        // Clean expired sessions
        sessions.retain(|s| !s.read().is_expired(ttl));

        // Return existing session or create new
        if let Some(session) = sessions.first() {
            session.write().touch();
            return session.clone();
        }

        let session_id = uuid::Uuid::new_v4().to_string();
        let session = Arc::new(RwLock::new(McpViaLlmSession::new(
            session_id,
            client_id.to_string(),
        )));
        sessions.push(session.clone());
        session
    }

    /// Handle a chat completion request in MCP via LLM mode.
    ///
    /// Returns a `CompletionResponse` from lr-providers that the caller
    /// (chat.rs) converts to an HTTP response.
    pub async fn handle_request(
        &self,
        gateway: &McpGateway,
        router: &Router,
        client: &Client,
        request: CompletionRequest,
        allowed_servers: Vec<String>,
    ) -> Result<lr_providers::CompletionResponse, McpViaLlmError> {
        let config = self.config();
        let session = self.get_or_create_session(&client.id);

        orchestrator::run_agentic_loop(
            gateway,
            router,
            client,
            session,
            request,
            &config,
            allowed_servers,
        )
        .await
    }

    /// Remove expired sessions (can be called periodically)
    pub fn cleanup_expired_sessions(&self) {
        let ttl = Duration::from_secs(self.config.read().session_ttl_seconds);
        self.sessions_by_client.retain(|_, sessions| {
            sessions.retain(|s| !s.read().is_expired(ttl));
            !sessions.is_empty()
        });
    }
}

#[derive(Debug, thiserror::Error)]
pub enum McpViaLlmError {
    #[error("MCP gateway error: {0}")]
    Gateway(String),

    #[error("Router error: {0}")]
    Router(#[from] lr_types::AppError),

    #[error("Max iterations ({0}) exceeded")]
    MaxIterations(u32),

    #[error("Loop timeout ({0}s) exceeded")]
    Timeout(u64),

    #[error("Tool execution failed: {0}")]
    ToolExecution(String),
}
