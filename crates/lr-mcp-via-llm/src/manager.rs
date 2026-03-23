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
use crate::session::{
    compute_message_hashes, reconstruct_history, score_session_match, McpViaLlmSession,
    PendingMixedExecution,
};

/// Manages MCP via LLM sessions and orchestrates agentic tool execution
/// Callback type for emitting monitor events from the orchestrator.
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

pub struct McpViaLlmManager {
    /// Sessions indexed by client_id
    pub(crate) sessions_by_client: DashMap<String, Vec<Arc<RwLock<McpViaLlmSession>>>>,
    /// Pending mixed tool executions indexed by client_id
    /// (one pending execution per client at most)
    pub(crate) pending_executions: Arc<DashMap<String, PendingMixedExecution>>,
    /// Configuration
    config: RwLock<McpViaLlmConfig>,
    /// Context management configuration (for client tool indexing)
    context_management_config: RwLock<lr_config::ContextManagementConfig>,
    /// Seen client tools per client (client_id → tool_names)
    seen_client_tools: DashMap<String, std::collections::HashSet<String>>,
    /// Optional memory service for auto-capturing conversation transcripts
    memory_service: RwLock<Option<Arc<lr_memory::MemoryService>>>,
    /// Optional monitor event callback
    pub(crate) monitor_emit: parking_lot::RwLock<Option<MonitorEmitFn>>,
    /// Optional monitor update callback
    pub(crate) monitor_update: parking_lot::RwLock<Option<MonitorUpdateFn>>,
}

impl McpViaLlmManager {
    pub fn new(config: McpViaLlmConfig) -> Self {
        Self {
            sessions_by_client: DashMap::new(),
            pending_executions: Arc::new(DashMap::new()),
            config: RwLock::new(config),
            context_management_config: RwLock::new(lr_config::ContextManagementConfig::default()),
            seen_client_tools: DashMap::new(),
            memory_service: RwLock::new(None),
            monitor_emit: parking_lot::RwLock::new(None),
            monitor_update: parking_lot::RwLock::new(None),
        }
    }

    /// Set the monitor event callback.
    pub fn set_monitor_emit(&self, emit: MonitorEmitFn) {
        *self.monitor_emit.write() = Some(emit);
    }

    /// Set the monitor update callback.
    pub fn set_monitor_update(&self, update: MonitorUpdateFn) {
        *self.monitor_update.write() = Some(update);
    }

    /// Set the memory service for auto-capturing conversation transcripts.
    pub fn set_memory_service(&self, service: Option<Arc<lr_memory::MemoryService>>) {
        *self.memory_service.write() = service;
    }

    /// Get a reference to the memory service (if configured).
    pub fn memory_service(&self) -> Option<Arc<lr_memory::MemoryService>> {
        self.memory_service.read().clone()
    }

    pub fn update_config(&self, config: McpViaLlmConfig) {
        *self.config.write() = config;
    }

    pub fn update_context_management_config(&self, config: lr_config::ContextManagementConfig) {
        *self.context_management_config.write() = config;
    }

    pub fn context_management_config(&self) -> lr_config::ContextManagementConfig {
        self.context_management_config.read().clone()
    }

    pub fn config(&self) -> McpViaLlmConfig {
        self.config.read().clone()
    }

    /// Record a seen client tool for a given client.
    pub fn record_seen_client_tool(&self, client_id: &str, tool_name: &str) {
        self.seen_client_tools
            .entry(client_id.to_string())
            .or_default()
            .insert(tool_name.to_string());
    }

    /// Get all seen client tools for a given client.
    pub fn get_seen_client_tools(&self, client_id: &str) -> Vec<String> {
        self.seen_client_tools
            .get(client_id)
            .map(|s| s.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Get the gateway session key for a client (if a session exists).
    pub fn get_gateway_session_key(&self, client_id: &str) -> Option<String> {
        let ttl = Duration::from_secs(self.config.read().session_ttl_seconds);
        self.sessions_by_client.get(client_id).and_then(|sessions| {
            sessions
                .iter()
                .find(|s| !s.read().is_expired(ttl))
                .map(|s| s.read().gateway_session_key.clone())
        })
    }

    /// Get an existing session or create a new one for this client.
    ///
    /// When `incoming_messages` is provided, sessions are matched by comparing
    /// normalized message hashes against stored client-visible hashes (fuzzy-resilient
    /// to whitespace changes, Unicode normalization, and message truncation).
    ///
    /// When `None` (e.g., preview), falls back to the first available session.
    pub(crate) fn get_or_create_session(
        &self,
        client_id: &str,
        incoming_messages: Option<&[ChatMessage]>,
    ) -> Arc<RwLock<McpViaLlmSession>> {
        let ttl = Duration::from_secs(self.config.read().session_ttl_seconds);

        let mut sessions = self
            .sessions_by_client
            .entry(client_id.to_string())
            .or_default();

        // Clean expired sessions
        sessions.retain(|s| !s.read().is_expired(ttl));

        // If no messages provided (preview) or empty, return first available or create new
        let incoming_messages = match incoming_messages {
            Some(msgs) if !msgs.is_empty() => msgs,
            _ => {
                if let Some(session) = sessions.first() {
                    session.write().touch();
                    return session.clone();
                }
                return self.create_new_session(&mut sessions, client_id);
            }
        };

        // Compute incoming hashes for matching
        let incoming_hashes = compute_message_hashes(incoming_messages);

        // Score each session and find best match
        const MATCH_THRESHOLD: f64 = 0.5;
        let mut best_score = 0.0f64;
        let mut best_idx: Option<usize> = None;

        for (i, session) in sessions.iter().enumerate() {
            let s = session.read();
            if s.client_message_hashes.is_empty() {
                continue;
            }
            let score = score_session_match(&s.client_message_hashes, &incoming_hashes);
            if score > best_score || (score == best_score && score >= MATCH_THRESHOLD) {
                best_score = score;
                best_idx = Some(i);
            }
        }

        if best_score >= MATCH_THRESHOLD {
            if let Some(idx) = best_idx {
                let session = sessions[idx].clone();
                session.write().touch();
                tracing::debug!(
                    "MCP via LLM: matched session for client {} (score={:.2}, {} stored hashes, {} incoming)",
                    &client_id[..8.min(client_id.len())],
                    best_score,
                    sessions[idx].read().client_message_hashes.len(),
                    incoming_hashes.len()
                );
                return session;
            }
        }

        // No match — create new session
        tracing::info!(
            "MCP via LLM: creating new session for client {} (best score={:.2}, {} existing sessions)",
            &client_id[..8.min(client_id.len())],
            best_score,
            sessions.len()
        );
        self.create_new_session(&mut sessions, client_id)
    }

    /// Create a new session and add it to the sessions list.
    fn create_new_session(
        &self,
        sessions: &mut Vec<Arc<RwLock<McpViaLlmSession>>>,
        client_id: &str,
    ) -> Arc<RwLock<McpViaLlmSession>> {
        let session_id = uuid::Uuid::new_v4().to_string();
        let session = Arc::new(RwLock::new(McpViaLlmSession::new(
            session_id,
            client_id.to_string(),
        )));
        sessions.push(session.clone());
        session
    }

    /// Find a session by its gateway session key.
    pub(crate) fn find_session_by_gateway_key(
        &self,
        client_id: &str,
        gateway_key: &str,
    ) -> Option<Arc<RwLock<McpViaLlmSession>>> {
        self.sessions_by_client.get(client_id).and_then(|sessions| {
            sessions
                .iter()
                .find(|s| s.read().gateway_session_key == gateway_key)
                .cloned()
        })
    }

    /// Check if the incoming request contains tool results that match a pending
    /// mixed execution for this client.
    pub(crate) fn take_pending_if_matching(
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

    /// Pre-fetch MCP tool definitions for preview purposes (e.g., firewall popup).
    ///
    /// Initializes the gateway session if needed and returns tool definitions
    /// as a JSON array in OpenAI function tool format.
    pub async fn list_tools_for_preview(
        &self,
        gateway: Arc<McpGateway>,
        client: &Client,
        allowed_servers: Vec<String>,
    ) -> Result<serde_json::Value, McpViaLlmError> {
        let session = self.get_or_create_session(&client.id, None);

        let (gateway_session_key, gateway_initialized) = {
            let s = session.read();
            (s.gateway_session_key.clone(), s.gateway_initialized)
        };

        let gw_client = crate::gateway_client::GatewayClient::new(
            &gateway,
            client,
            gateway_session_key,
            allowed_servers,
        );

        if !gateway_initialized {
            let instructions = gw_client.initialize().await?;
            let mut s = session.write();
            s.gateway_initialized = true;
            s.pending_gateway_instructions = instructions;
        }

        let mcp_tools = gw_client.list_tools().await?;

        // Convert to OpenAI function tool format
        let tools_json: Vec<serde_json::Value> = mcp_tools
            .iter()
            .map(|t| {
                serde_json::json!({
                    "type": "function",
                    "function": {
                        "name": t.name,
                        "description": t.description,
                        "parameters": t.input_schema,
                    }
                })
            })
            .collect();

        Ok(serde_json::Value::Array(tools_json))
    }

    /// Handle a chat completion request in MCP via LLM mode.
    ///
    /// Returns a `CompletionResponse` from lr-providers that the caller
    /// (chat.rs) converts to an HTTP response.
    ///
    /// If `guardrail_gate` is provided, the orchestrator will await it after the
    /// first LLM call returns but before executing any tools or returning a response.
    #[allow(clippy::too_many_arguments)]
    pub async fn handle_request(
        &self,
        gateway: Arc<McpGateway>,
        router: &Router,
        client: &Client,
        mut request: CompletionRequest,
        allowed_servers: Vec<String>,
        guardrail_gate: Option<GuardrailGate>,
        llm_call_event_id: Option<String>,
        monitor_session_id: Option<String>,
    ) -> Result<lr_providers::CompletionResponse, McpViaLlmError> {
        let config = self.config();
        let session = self.get_or_create_session(&client.id, Some(&request.messages));
        let memory_svc = self.memory_service();

        // Store client message hashes for future session matching (BEFORE reconstruction)
        let incoming_hashes = compute_message_hashes(&request.messages);
        session.write().client_message_hashes = incoming_hashes;

        // Store monitor session_id so gateway tool calls get grouped
        if let Some(ref sid) = monitor_session_id {
            let gw_key = session.read().gateway_session_key.clone();
            if let Some(gw_session) = gateway.get_session(&gw_key) {
                gw_session.write().await.monitor_session_id = Some(sid.clone());
            }
        }

        // Initialize memory transcript if enabled for this client
        if let Some(ref svc) = memory_svc {
            if client.memory_enabled.unwrap_or(false) {
                let needs_init = session.read().transcript_path.is_none();
                if needs_init {
                    let memory_folder = client.memory_folder_name();
                    if let Ok(client_dir) = svc.ensure_client_dir(memory_folder) {
                        let active_dir = client_dir.join("active");
                        let content_hint = request
                            .messages
                            .iter()
                            .rev()
                            .find(|m| m.role == "user")
                            .map(|m| m.content.as_text())
                            .unwrap_or_default();
                        let (file_path, is_new) = svc.session_manager.get_or_create_session(
                            &client.id,
                            &active_dir,
                            &content_hint,
                            memory_folder,
                        );
                        if is_new {
                            if let Err(e) = svc.transcript.create_session_file(&file_path).await {
                                tracing::warn!("Failed to create memory transcript: {}", e);
                            }
                        }
                        let mut s = session.write();
                        s.transcript_path = Some(file_path);
                        s.memory_folder = Some(memory_folder.to_string());
                    }
                }
            }
        }

        // Check if this is a resume from a pending mixed execution
        if let Some((pending, client_tool_results)) =
            self.take_pending_if_matching(&client.id, &request)
        {
            // Use the session that created the pending execution
            let resume_session = self
                .find_session_by_gateway_key(&client.id, &pending.gateway_session_key)
                .unwrap_or_else(|| session.clone());

            tracing::info!(
                "MCP via LLM: resuming pending mixed execution for client {} ({} client tool results)",
                &client.id[..8.min(client.id.len())],
                client_tool_results.len()
            );

            let cm_config = self.context_management_config();
            let result = orchestrator::resume_after_mixed(
                gateway,
                router,
                client,
                resume_session,
                pending,
                request,
                client_tool_results,
                &config,
                allowed_servers,
                &cm_config,
            )
            .await?;

            return self.handle_orchestrator_result(&client.id, result);
        }

        // Reconstruct history: inject hidden MCP messages from previous turns
        {
            let s = session.read();
            if !s.history.full_messages.is_empty() {
                let reconstructed = reconstruct_history(
                    &s.history.full_messages,
                    &request.messages,
                    s.gateway_instructions.as_deref(),
                );
                drop(s);
                request.messages = reconstructed;
            }
        }

        // Build monitor callback for transformed request — updates the parent LlmCall event
        let on_transformed: orchestrator::TransformedRequestCallback = {
            let update_fn = self.monitor_update.read().clone();
            let event_id = llm_call_event_id.clone();
            match (update_fn, event_id) {
                (Some(update_fn), Some(event_id)) => Some(Box::new(
                    move |request_body: serde_json::Value, transformations: Vec<String>| {
                        update_fn(
                            &event_id,
                            Box::new(move |event| {
                                if let lr_monitor::MonitorEventData::LlmCall {
                                    transformed_body,
                                    transformations_applied,
                                    ..
                                } = &mut event.data
                                {
                                    *transformed_body = Some(request_body);
                                    *transformations_applied = Some(transformations);
                                }
                            }),
                        );
                    },
                )
                    as Box<dyn FnOnce(serde_json::Value, Vec<String>) + Send>),
                _ => None,
            }
        };

        // Normal flow: run the agentic loop
        let monitor_emit = self.monitor_emit.read().clone();
        let monitor_update = self.monitor_update.read().clone();
        let result = orchestrator::run_agentic_loop(
            gateway,
            router,
            client,
            session,
            request,
            &config,
            allowed_servers,
            guardrail_gate,
            None,
            memory_svc,
            on_transformed,
            monitor_session_id,
            monitor_emit,
            monitor_update,
            llm_call_event_id,
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
                // Insert replaces any existing entry; Drop impl on PendingMixedExecution
                // aborts old background tasks automatically.
                if self.pending_executions.contains_key(client_id) {
                    tracing::warn!(
                        "MCP via LLM: replacing existing pending execution for client {} — previous background tasks will be aborted",
                        &client_id[..8.min(client_id.len())]
                    );
                }
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
    ///
    /// If `guardrail_gate` is provided, the orchestrator will await it after the
    /// first LLM stream completes but before executing any tools or sending the finish chunk.
    #[allow(clippy::too_many_arguments)]
    pub async fn handle_streaming_request(
        &self,
        gateway: Arc<McpGateway>,
        router: Arc<Router>,
        client: &Client,
        mut request: CompletionRequest,
        allowed_servers: Vec<String>,
        guardrail_gate: Option<GuardrailGate>,
        llm_call_event_id: Option<String>,
        monitor_session_id: Option<String>,
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
        let session = self.get_or_create_session(&client.id, Some(&request.messages));
        let memory_svc = self.memory_service();

        // Store client message hashes for future session matching (BEFORE reconstruction)
        let incoming_hashes = compute_message_hashes(&request.messages);
        session.write().client_message_hashes = incoming_hashes;

        // Initialize memory transcript for streaming (same logic as non-streaming)
        if let Some(ref svc) = memory_svc {
            if client.memory_enabled.unwrap_or(false) {
                let needs_init = session.read().transcript_path.is_none();
                if needs_init {
                    let memory_folder = client.memory_folder_name();
                    if let Ok(client_dir) = svc.ensure_client_dir(memory_folder) {
                        let active_dir = client_dir.join("active");
                        let content_hint = request
                            .messages
                            .iter()
                            .rev()
                            .find(|m| m.role == "user")
                            .map(|m| m.content.as_text())
                            .unwrap_or_default();
                        let (file_path, is_new) = svc.session_manager.get_or_create_session(
                            &client.id,
                            &active_dir,
                            &content_hint,
                            memory_folder,
                        );
                        if is_new {
                            if let Err(e) = svc.transcript.create_session_file(&file_path).await {
                                tracing::warn!("Failed to create memory transcript: {}", e);
                            }
                        }
                        let mut s = session.write();
                        s.transcript_path = Some(file_path);
                        s.memory_folder = Some(memory_folder.to_string());
                    }
                }
            }
        }

        // Check if this is a resume from a pending mixed execution (streaming variant)
        if let Some((pending, client_tool_results)) =
            self.take_pending_if_matching(&client.id, &request)
        {
            // Use the session that created the pending execution
            let resume_session = self
                .find_session_by_gateway_key(&client.id, &pending.gateway_session_key)
                .unwrap_or_else(|| session.clone());

            tracing::info!(
                "MCP via LLM streaming: resuming pending mixed execution for client {} ({} client tool results)",
                &client.id[..8.min(client.id.len())],
                client_tool_results.len()
            );

            // Resume: await MCP handles, reconstruct history, then stream
            let cm_config = self.context_management_config();
            let result = orchestrator::resume_after_mixed(
                gateway.clone(),
                &router,
                client,
                resume_session,
                pending,
                request,
                client_tool_results,
                &config,
                allowed_servers.clone(),
                &cm_config,
            )
            .await?;

            match result {
                OrchestratorResult::Complete(response) => {
                    // Wrap completed response as a single-chunk stream
                    let chunk = response_to_chunk(&response);
                    return Ok(Box::pin(futures::stream::once(async move { Ok(chunk) })));
                }
                OrchestratorResult::PendingMixed {
                    client_response,
                    pending,
                } => {
                    // Another mixed result after resume — store and return client tools as stream
                    self.pending_executions.insert(client.id.clone(), pending);
                    let chunk = response_to_chunk(&client_response);
                    return Ok(Box::pin(futures::stream::once(async move { Ok(chunk) })));
                }
            }
        }

        // Reconstruct history: inject hidden MCP messages from previous turns
        {
            let s = session.read();
            if !s.history.full_messages.is_empty() {
                let reconstructed = reconstruct_history(
                    &s.history.full_messages,
                    &request.messages,
                    s.gateway_instructions.as_deref(),
                );
                drop(s);
                request.messages = reconstructed;
            }
        }

        // Build monitor callback for transformed request — updates the parent LlmCall event
        let on_transformed_stream: orchestrator::TransformedRequestCallback = {
            let update_fn = self.monitor_update.read().clone();
            let event_id = llm_call_event_id.clone();
            match (update_fn, event_id) {
                (Some(update_fn), Some(event_id)) => Some(Box::new(
                    move |request_body: serde_json::Value, transformations: Vec<String>| {
                        update_fn(
                            &event_id,
                            Box::new(move |event| {
                                if let lr_monitor::MonitorEventData::LlmCall {
                                    transformed_body,
                                    transformations_applied,
                                    ..
                                } = &mut event.data
                                {
                                    *transformed_body = Some(request_body);
                                    *transformations_applied = Some(transformations);
                                }
                            }),
                        );
                    },
                )
                    as Box<dyn FnOnce(serde_json::Value, Vec<String>) + Send>),
                _ => None,
            }
        };

        let monitor_emit = self.monitor_emit.read().clone();
        let monitor_update = self.monitor_update.read().clone();
        orchestrator_stream::run_agentic_loop_streaming(
            gateway,
            router,
            client,
            session,
            request,
            &config,
            allowed_servers,
            self.pending_executions.clone(),
            guardrail_gate,
            memory_svc,
            on_transformed_stream,
            monitor_session_id,
            monitor_emit,
            monitor_update,
            llm_call_event_id,
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
        // Drop impl on PendingMixedExecution will abort background tasks.
        let timeout = Duration::from_secs(self.config.read().max_loop_timeout_seconds);
        self.pending_executions.retain(|client_id, pending| {
            let expired = pending.started_at.elapsed() >= timeout;
            if expired {
                tracing::warn!(
                    "MCP via LLM: cleaning up timed-out pending execution for client {} ({} background tasks aborted, waited {:.1}s)",
                    &client_id[..8.min(client_id.len())],
                    pending.mcp_handles.len(),
                    pending.started_at.elapsed().as_secs_f64()
                );
            }
            !expired
        });
    }
}

/// Convert a CompletionResponse to a single CompletionChunk (for streaming resume)
fn response_to_chunk(response: &lr_providers::CompletionResponse) -> lr_providers::CompletionChunk {
    let choice = response.choices.first();
    lr_providers::CompletionChunk {
        id: response.id.clone(),
        object: "chat.completion.chunk".to_string(),
        created: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64,
        model: response.model.clone(),
        choices: vec![lr_providers::ChunkChoice {
            index: 0,
            delta: lr_providers::ChunkDelta {
                role: Some("assistant".to_string()),
                content: choice.and_then(|c| match &c.message.content {
                    lr_providers::ChatMessageContent::Text(t) if !t.is_empty() => Some(t.clone()),
                    _ => None,
                }),
                tool_calls: choice.and_then(|c| {
                    c.message.tool_calls.as_ref().map(|tcs| {
                        tcs.iter()
                            .enumerate()
                            .map(|(i, tc)| lr_providers::ToolCallDelta {
                                index: i as u32,
                                id: Some(tc.id.clone()),
                                tool_type: Some(tc.tool_type.clone()),
                                function: Some(lr_providers::FunctionCallDelta {
                                    name: Some(tc.function.name.clone()),
                                    arguments: Some(tc.function.arguments.clone()),
                                }),
                            })
                            .collect()
                    })
                }),
                reasoning_content: choice.and_then(|c| c.message.reasoning_content.clone()),
            },
            finish_reason: choice.and_then(|c| c.finish_reason.clone()),
        }],
        extensions: None,
    }
}

/// A gate that must resolve before the orchestrator may execute tools or return a response.
/// Resolves to Ok(()) if guardrails passed, Err(message) if denied.
pub type GuardrailGate = tokio::task::JoinHandle<Result<(), String>>;

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

    #[error("Guardrail denied: {0}")]
    GuardrailDenied(String),
}
