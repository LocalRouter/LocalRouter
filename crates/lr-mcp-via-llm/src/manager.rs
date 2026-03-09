//! McpViaLlmManager - entry point for MCP via LLM mode
//!
//! Manages sessions and dispatches requests to the agentic orchestrator.
//! Handles pending mixed tool executions where MCP tools run in the background.

use std::sync::Arc;
use std::time::Duration;

use dashmap::DashMap;
use parking_lot::RwLock;

use lr_config::{Client, McpViaLlmConfig};
use lr_mcp::McpGateway;
use lr_providers::{ChatMessage, CompletionRequest};
use lr_router::Router;

use crate::orchestrator::{self, OrchestratorResult};
use crate::orchestrator_stream;
use crate::session::{McpViaLlmSession, PendingMixedExecution};

/// Manages MCP via LLM sessions and orchestrates agentic tool execution
pub struct McpViaLlmManager {
    /// Sessions indexed by client_id
    sessions_by_client: DashMap<String, Vec<Arc<RwLock<McpViaLlmSession>>>>,
    /// Pending mixed tool executions indexed by client_id
    /// (one pending execution per client at most)
    pending_executions: DashMap<String, PendingMixedExecution>,
    /// Configuration
    config: RwLock<McpViaLlmConfig>,
}

impl McpViaLlmManager {
    pub fn new(config: McpViaLlmConfig) -> Self {
        Self {
            sessions_by_client: DashMap::new(),
            pending_executions: DashMap::new(),
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

    /// Check if the incoming request contains tool results that match a pending
    /// mixed execution for this client.
    fn take_pending_if_matching(
        &self,
        client_id: &str,
        request: &CompletionRequest,
    ) -> Option<(PendingMixedExecution, Vec<ChatMessage>)> {
        // Check if there's a pending execution for this client
        let pending_ref = self.pending_executions.get(client_id)?;
        let pending_client_ids = &pending_ref.client_tool_call_ids;

        // Look for tool result messages in the request that match the pending client tool call IDs
        let client_tool_results: Vec<ChatMessage> = request
            .messages
            .iter()
            .filter(|msg| {
                msg.role == "tool"
                    && msg
                        .tool_call_id
                        .as_ref()
                        .is_some_and(|id| pending_client_ids.contains(id))
            })
            .cloned()
            .collect();

        // If we found at least one matching tool result, this is a resume
        if !client_tool_results.is_empty() {
            drop(pending_ref); // Release read reference before removing
            let (_, pending) = self.pending_executions.remove(client_id)?;
            Some((pending, client_tool_results))
        } else {
            None
        }
    }

    /// Handle a chat completion request in MCP via LLM mode.
    ///
    /// Returns a `CompletionResponse` from lr-providers that the caller
    /// (chat.rs) converts to an HTTP response.
    pub async fn handle_request(
        &self,
        gateway: Arc<McpGateway>,
        router: &Router,
        client: &Client,
        request: CompletionRequest,
        allowed_servers: Vec<String>,
    ) -> Result<lr_providers::CompletionResponse, McpViaLlmError> {
        let config = self.config();
        let session = self.get_or_create_session(&client.id);

        // Check if this is a resume from a pending mixed execution
        if let Some((pending, client_tool_results)) =
            self.take_pending_if_matching(&client.id, &request)
        {
            tracing::info!(
                "MCP via LLM: resuming pending mixed execution for client {} ({} client tool results)",
                &client.id[..8.min(client.id.len())],
                client_tool_results.len()
            );

            let result = orchestrator::resume_after_mixed(
                gateway,
                router,
                client,
                session,
                pending,
                request,
                client_tool_results,
                &config,
                allowed_servers,
            )
            .await?;

            return self.handle_orchestrator_result(&client.id, result);
        }

        // Normal flow: run the agentic loop
        let result = orchestrator::run_agentic_loop(
            gateway,
            router,
            client,
            session,
            request,
            &config,
            allowed_servers,
        )
        .await?;

        self.handle_orchestrator_result(&client.id, result)
    }

    /// Process an orchestrator result, storing pending state if needed.
    fn handle_orchestrator_result(
        &self,
        client_id: &str,
        result: OrchestratorResult,
    ) -> Result<lr_providers::CompletionResponse, McpViaLlmError> {
        match result {
            OrchestratorResult::Complete(response) => Ok(response),
            OrchestratorResult::PendingMixed {
                client_response,
                pending,
            } => {
                tracing::info!(
                    "MCP via LLM: storing pending mixed execution for client {} ({} MCP tasks in background)",
                    &client_id[..8.min(client_id.len())],
                    pending.mcp_handles.len()
                );
                self.pending_executions
                    .insert(client_id.to_string(), pending);
                Ok(client_response)
            }
        }
    }

    /// Handle a streaming chat completion request in MCP via LLM mode.
    ///
    /// Returns a stream of `CompletionChunk`s that the caller wraps in SSE.
    /// Multiple LLM iterations are streamed through a single connection.
    pub async fn handle_streaming_request(
        &self,
        gateway: Arc<McpGateway>,
        router: Arc<Router>,
        client: &Client,
        request: CompletionRequest,
        allowed_servers: Vec<String>,
    ) -> Result<
        std::pin::Pin<
            Box<
                dyn futures::Stream<Item = lr_types::AppResult<lr_providers::CompletionChunk>>
                    + Send,
            >,
        >,
        McpViaLlmError,
    > {
        let config = self.config();
        let session = self.get_or_create_session(&client.id);

        orchestrator_stream::run_agentic_loop_streaming(
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

    /// Start a background task that periodically cleans up expired sessions.
    /// Returns a handle that can be used to abort the task.
    pub fn start_cleanup_task(self: &Arc<Self>) -> tokio::task::JoinHandle<()> {
        let manager = Arc::clone(self);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
            loop {
                interval.tick().await;
                manager.cleanup_expired_sessions();
            }
        })
    }

    /// Remove expired sessions (can be called periodically)
    pub fn cleanup_expired_sessions(&self) {
        let ttl = Duration::from_secs(self.config.read().session_ttl_seconds);
        self.sessions_by_client.retain(|_, sessions| {
            sessions.retain(|s| !s.read().is_expired(ttl));
            !sessions.is_empty()
        });

        // Also clean up pending executions that have been waiting too long
        let timeout = Duration::from_secs(self.config.read().max_loop_timeout_seconds);
        self.pending_executions
            .retain(|_, pending| pending.started_at.elapsed() < timeout);
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
