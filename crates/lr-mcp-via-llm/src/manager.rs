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
    compute_message_hashes, reconstruct_history, reconstruct_keyed_history, score_session_match,
    McpViaLlmSession, PendingMixedExecution, PriorTurnTotals,
};

/// Sessions kept per client; the least recently active one goes first.
const MAX_SESSIONS_PER_CLIENT: usize = 64;

/// A turn ready to run: either a fresh pass of the loop, or the resume of
/// a pending mixed execution with the client's tool results.
enum PreparedTurn {
    Run {
        session: Arc<RwLock<McpViaLlmSession>>,
        request: CompletionRequest,
    },
    Resume {
        session: Arc<RwLock<McpViaLlmSession>>,
        pending: Box<PendingMixedExecution>,
        request: CompletionRequest,
        client_tool_results: Vec<ChatMessage>,
    },
}

/// The turn succeeded: from now on the session holds the history of the
/// response named by `options.session_key`.
fn stamp_session_key(session: &Arc<RwLock<McpViaLlmSession>>, options: &TurnOptions) {
    if let Some(key) = &options.session_key {
        session.write().explicit_key = Some(key.clone());
    }
}

/// Swap the stored turn's leading `instructions` message (if `messages`
/// still starts with it) for this turn's own.
fn replace_instructions(
    messages: &mut Vec<ChatMessage>,
    stored: Option<&str>,
    current: Option<&ChatMessage>,
) {
    if let (Some(stored), Some(first)) = (stored, messages.first()) {
        if first.role == "system" && first.content.as_text() == stored {
            messages.remove(0);
        }
    }
    if let Some(current) = current {
        messages.insert(0, current.clone());
    }
}

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

    /// Get all seen client tools for a given client.
    pub fn get_seen_client_tools(&self, client_id: &str) -> Vec<String> {
        self.seen_client_tools
            .get(client_id)
            .map(|s| s.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Find a session for a chat completions turn (or a preview), creating
    /// one when none matches.
    ///
    /// When `incoming_messages` is provided, sessions are matched by
    /// comparing normalized message hashes against stored client-visible
    /// hashes (fuzzy-resilient to whitespace changes, Unicode normalization,
    /// and message truncation). Keyed sessions (Responses API) never match
    /// by hash: they are found only by their key (`keyed_session`).
    ///
    /// When `None` (e.g., preview), falls back to the first available
    /// session.
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
                return self.create_new_session(&mut sessions, client_id, false);
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
            if s.keyed || s.client_message_hashes.is_empty() {
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
        self.create_new_session(&mut sessions, client_id, false)
    }

    /// Session for a keyed (Responses API) turn.
    ///
    /// The session keyed to `resume_key` (the response the turn continues
    /// from) is forked: the turn runs on the copy, and the original keeps
    /// its key and history, so any number of turns can continue from one
    /// response, each from the right history. Without a key, or when no
    /// session carries it (expired, or the turn that would have produced it
    /// failed), the turn starts a new session. The turn's own key is stamped
    /// on the session only once the turn succeeded.
    pub(crate) fn keyed_session(
        &self,
        client_id: &str,
        resume_key: Option<&str>,
    ) -> Arc<RwLock<McpViaLlmSession>> {
        let ttl = Duration::from_secs(self.config.read().session_ttl_seconds);
        let mut sessions = self
            .sessions_by_client
            .entry(client_id.to_string())
            .or_default();
        sessions.retain(|s| !s.read().is_expired(ttl));

        let parent = resume_key.and_then(|key| {
            sessions
                .iter()
                .find(|s| s.read().explicit_key.as_deref() == Some(key))
                .cloned()
        });
        let Some(parent) = parent else {
            return self.create_new_session(&mut sessions, client_id, true);
        };
        let fork = {
            let mut parent = parent.write();
            parent.touch();
            parent.fork()
        };
        tracing::debug!(
            "MCP via LLM: continuing keyed session for client {}",
            &client_id[..8.min(client_id.len())]
        );
        let fork = Arc::new(RwLock::new(fork));
        Self::push_session(&mut sessions, fork.clone());
        fork
    }

    /// Session a turn runs on: keyed when the endpoint names the response
    /// it produces, otherwise matched by message hashes.
    fn session_for_turn(
        &self,
        client_id: &str,
        incoming: &[ChatMessage],
        options: &TurnOptions,
    ) -> Arc<RwLock<McpViaLlmSession>> {
        if options.session_key.is_some() {
            self.keyed_session(client_id, options.resume_session_key.as_deref())
        } else {
            self.get_or_create_session(client_id, Some(incoming))
        }
    }

    /// Create a new session and add it to the sessions list.
    fn create_new_session(
        &self,
        sessions: &mut Vec<Arc<RwLock<McpViaLlmSession>>>,
        client_id: &str,
        keyed: bool,
    ) -> Arc<RwLock<McpViaLlmSession>> {
        let session_id = uuid::Uuid::new_v4().to_string();
        let mut session = McpViaLlmSession::new_with_key(session_id, client_id.to_string(), None);
        session.keyed = keyed;
        let session = Arc::new(RwLock::new(session));
        Self::push_session(sessions, session.clone());
        session
    }

    /// Add a session, evicting the least recently active one when the
    /// client already has `MAX_SESSIONS_PER_CLIENT`. Every keyed turn adds
    /// a session (see `keyed_session`), so without a cap a long Responses
    /// conversation would keep one full history per turn until the TTL.
    fn push_session(
        sessions: &mut Vec<Arc<RwLock<McpViaLlmSession>>>,
        session: Arc<RwLock<McpViaLlmSession>>,
    ) {
        while sessions.len() >= MAX_SESSIONS_PER_CLIENT {
            let oldest = sessions
                .iter()
                .enumerate()
                .min_by_key(|(_, s)| s.read().last_activity)
                .map(|(i, _)| i);
            match oldest {
                Some(i) => {
                    sessions.remove(i);
                }
                None => break,
            }
        }
        sessions.push(session);
    }

    /// Find a session by its id.
    pub(crate) fn find_session_by_id(
        &self,
        client_id: &str,
        session_id: &str,
    ) -> Option<Arc<RwLock<McpViaLlmSession>>> {
        self.sessions_by_client
            .get(client_id)
            .and_then(|sessions| sessions.iter().find(|s| s.read().id == session_id).cloned())
    }

    /// Check if the incoming request contains tool results that match a pending
    /// mixed execution for this client.
    pub(crate) fn take_pending_if_matching(
        &self,
        client_id: &str,
        request: &CompletionRequest,
    ) -> Option<(PendingMixedExecution, Vec<ChatMessage>)> {
        // Check if there's a pending execution for this client
        let dashmap::mapref::entry::Entry::Occupied(pending_entry) =
            self.pending_executions.entry(client_id.to_string())
        else {
            return None;
        };
        let pending_ref = pending_entry.get();
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
            // Match and remove under the same shard lock. Otherwise a new
            // execution installed after lookup could be resumed with old results.
            let pending = pending_entry.remove();
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
    pub async fn handle_request(
        &self,
        gateway: Arc<McpGateway>,
        router: &Router,
        client: &Client,
        request: CompletionRequest,
        allowed_servers: Vec<String>,
        guardrail_gate: Option<GuardrailGate>,
        llm_call_event_id: Option<String>,
        monitor_session_id: Option<String>,
        options: TurnOptions,
    ) -> Result<lr_providers::CompletionResponse, McpViaLlmError> {
        let config = self.config();
        let monitor = self.turn_monitor(llm_call_event_id.clone(), monitor_session_id, &options);
        let turn = self
            .prepare_turn(gateway.as_ref(), client, request, &options, &monitor)
            .await;

        let result = match turn {
            PreparedTurn::Resume {
                session,
                pending,
                request,
                client_tool_results,
            } => {
                let cm_config = self.context_management_config();
                orchestrator::resume_after_mixed(
                    gateway,
                    router,
                    client,
                    session.clone(),
                    *pending,
                    request,
                    client_tool_results,
                    &config,
                    allowed_servers,
                    &cm_config,
                    monitor,
                    options.separate_assistant_messages,
                )
                .await
                .map(|result| (session, result))
            }
            PreparedTurn::Run { session, request } => {
                let on_transformed = self.transformed_request_callback(llm_call_event_id);
                orchestrator::run_agentic_loop(
                    gateway,
                    router,
                    client,
                    session.clone(),
                    request,
                    &config,
                    allowed_servers,
                    guardrail_gate,
                    PriorTurnTotals::default(),
                    self.memory_service(),
                    on_transformed,
                    monitor,
                    options.separate_assistant_messages,
                )
                .await
                .map(|result| (session, result))
            }
        };
        let (session, result) = result?;
        stamp_session_key(&session, &options);
        self.handle_orchestrator_result(&client.id, result)
    }

    /// Monitor wiring for one turn.
    fn turn_monitor(
        &self,
        llm_call_event_id: Option<String>,
        monitor_session_id: Option<String>,
        options: &TurnOptions,
    ) -> orchestrator::TurnMonitor {
        orchestrator::TurnMonitor {
            session_id: monitor_session_id,
            emit: self.monitor_emit.read().clone(),
            update: self.monitor_update.read().clone(),
            llm_call_event_id,
            endpoint: options
                .monitor_endpoint
                .clone()
                .unwrap_or_else(|| "/v1/chat/completions".to_string()),
        }
    }

    /// Monitor callback for the transformed request: updates the turn's
    /// own LlmCall event.
    fn transformed_request_callback(
        &self,
        llm_call_event_id: Option<String>,
    ) -> orchestrator::TransformedRequestCallback {
        let update_fn = self.monitor_update.read().clone();
        match (update_fn, llm_call_event_id) {
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
    }

    /// Everything a turn needs before the loop runs: the session, the
    /// pending mixed execution it resumes (if any), memory transcript
    /// setup, and the request with the hidden history of earlier turns
    /// spliced back in.
    async fn prepare_turn(
        &self,
        gateway: &McpGateway,
        client: &Client,
        mut request: CompletionRequest,
        options: &TurnOptions,
        monitor: &orchestrator::TurnMonitor,
    ) -> PreparedTurn {
        let keyed = options.session_key.is_some();
        // This turn's `instructions` (Responses API): they lead the
        // request and apply to this turn only.
        let current_instructions = (keyed && options.leading_instructions)
            .then(|| request.messages.first().filter(|m| m.role == "system"))
            .flatten()
            .cloned();

        // A resume continues on the session that stopped for the client's
        // tools; anything else gets a session of its own kind.
        let resume = self.take_pending_if_matching(&client.id, &request);
        let session = match &resume {
            Some((pending, _)) => self
                .find_session_by_id(&client.id, &pending.session_id)
                .unwrap_or_else(|| self.session_for_turn(&client.id, &request.messages, options)),
            None => self.session_for_turn(&client.id, &request.messages, options),
        };

        // Store client message hashes for future session matching (BEFORE reconstruction)
        session.write().client_message_hashes = compute_message_hashes(&request.messages);

        // Store monitor session_id so gateway tool calls get grouped
        if let Some(ref sid) = monitor.session_id {
            let gw_key = session.read().gateway_session_key.clone();
            if let Some(gw_session) = gateway.get_session(&gw_key) {
                gw_session.write().await.monitor_session_id = Some(sid.clone());
            }
        }

        self.init_memory_transcript(client, &session, &request)
            .await;

        if let Some((mut pending, client_tool_results)) = resume {
            tracing::info!(
                "MCP via LLM: resuming pending mixed execution for client {} ({} client tool results)",
                &client.id[..8.min(client.id.len())],
                client_tool_results.len()
            );
            if keyed {
                let stored = session.read().instructions_head.clone();
                replace_instructions(
                    &mut pending.messages_before_mixed,
                    stored.as_deref(),
                    current_instructions.as_ref(),
                );
                session.write().instructions_head =
                    current_instructions.as_ref().map(|m| m.content.as_text());
            }
            return PreparedTurn::Resume {
                session,
                pending: Box::new(pending),
                request,
                client_tool_results,
            };
        }

        // Reconstruct history: inject hidden MCP messages from previous turns
        {
            let s = session.read();
            if !s.history.full_messages.is_empty() {
                let reconstructed = if keyed {
                    reconstruct_keyed_history(
                        &s.history.full_messages,
                        &request.messages,
                        s.gateway_instructions.as_deref(),
                        s.instructions_head.as_deref(),
                        current_instructions.is_some(),
                    )
                } else {
                    reconstruct_history(
                        &s.history.full_messages,
                        &request.messages,
                        s.gateway_instructions.as_deref(),
                    )
                };
                drop(s);
                request.messages = reconstructed;
            }
        }
        if keyed {
            session.write().instructions_head =
                current_instructions.as_ref().map(|m| m.content.as_text());
        }
        PreparedTurn::Run { session, request }
    }

    /// Initialize the memory transcript if memory is enabled for this client.
    async fn init_memory_transcript(
        &self,
        client: &Client,
        session: &Arc<RwLock<McpViaLlmSession>>,
        request: &CompletionRequest,
    ) {
        let Some(svc) = self.memory_service() else {
            return;
        };
        if !client.memory_enabled.unwrap_or(false) || session.read().transcript_path.is_some() {
            return;
        }
        let memory_folder = client.memory_folder_name();
        let Ok(client_dir) = svc.ensure_client_dir(memory_folder) else {
            return;
        };
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
    pub async fn handle_streaming_request(
        &self,
        gateway: Arc<McpGateway>,
        router: Arc<Router>,
        client: &Client,
        request: CompletionRequest,
        allowed_servers: Vec<String>,
        guardrail_gate: Option<GuardrailGate>,
        llm_call_event_id: Option<String>,
        monitor_session_id: Option<String>,
        options: TurnOptions,
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
        let monitor = self.turn_monitor(llm_call_event_id.clone(), monitor_session_id, &options);
        let turn = self
            .prepare_turn(gateway.as_ref(), client, request, &options, &monitor)
            .await;

        let (session, request) = match turn {
            PreparedTurn::Resume {
                session,
                pending,
                request,
                client_tool_results,
            } => {
                // Resume: await MCP handles, reconstruct history, then run
                // the rest of the loop and send its result as one chunk.
                let cm_config = self.context_management_config();
                let result = orchestrator::resume_after_mixed(
                    gateway.clone(),
                    &router,
                    client,
                    session.clone(),
                    *pending,
                    request,
                    client_tool_results,
                    &config,
                    allowed_servers.clone(),
                    &cm_config,
                    monitor,
                    options.separate_assistant_messages,
                )
                .await?;
                stamp_session_key(&session, &options);
                let response = self.handle_orchestrator_result(&client.id, result)?;
                let chunk = response_to_chunk(&response);
                return Ok(Box::pin(futures::stream::once(async move { Ok(chunk) })));
            }
            PreparedTurn::Run { session, request } => (session, request),
        };

        let on_transformed_stream = self.transformed_request_callback(llm_call_event_id);
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
            self.memory_service(),
            on_transformed_stream,
            monitor,
            orchestrator_stream::StreamOptions {
                withhold_tool_call_deltas: options.withhold_tool_call_deltas,
                separate_assistant_messages: options.separate_assistant_messages,
                session_key: options.session_key,
            },
        )
        .await
    }

    /// Remove expired sessions (call periodically from background task)
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
                annotations: None,
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
        // The resumed loop's usage and metadata, which a streaming client
        // would otherwise never see (and the endpoint would estimate).
        extensions: response.extensions.clone(),
        usage: Some(response.usage.clone()),
        provider: Some(response.provider.clone()).filter(|p| !p.is_empty()),
    }
}

/// Per-turn options an endpoint adapter passes to the orchestrator.
///
/// The default is what `/v1/chat/completions` uses: sessions are matched by
/// message hashes and the client's tool-call deltas are streamed as they
/// arrive. Calls the gateway executes are never sent to the client either
/// way.
#[derive(Debug, Clone, Default)]
pub struct TurnOptions {
    /// Key of the session the turn continues from. Only used for keyed
    /// turns (`session_key` set). The Responses API passes
    /// `previous_response_id`.
    pub resume_session_key: Option<String>,
    /// Makes the turn keyed: its session is found by `resume_session_key`
    /// only (never by message hashes), and carries this key once the turn
    /// succeeded, so a later turn can name it in `resume_session_key`. The
    /// Responses API passes the id of the response being produced.
    pub session_key: Option<String>,
    /// Streaming only. Hold the client's tool-call deltas back and send the
    /// client's calls once, complete, with the finish reason. For wire
    /// formats where every forwarded tool call delta becomes an event of
    /// an item the client is expected to answer.
    pub withhold_tool_call_deltas: bool,
    /// Keyed turns: the request's first message is this turn's
    /// `instructions`, which apply to this turn only. They are not carried
    /// into the history later turns continue from.
    pub leading_instructions: bool,
    /// Report text the model wrote before gateway-executed tool calls as
    /// assistant messages of their own: streaming sends a
    /// `MESSAGE_BOUNDARY_EXTENSION` chunk where one message ends;
    /// non-streaming lists their text under `PRECEDING_MESSAGES_EXTENSION`.
    pub separate_assistant_messages: bool,
    /// Endpoint label for the orchestrator's monitor events (defaults to
    /// `/v1/chat/completions`).
    pub monitor_endpoint: Option<String>,
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

#[cfg(test)]
mod instruction_tests {
    use super::replace_instructions;
    use lr_providers::{ChatMessage, ChatMessageContent};

    fn msg(role: &str, text: &str) -> ChatMessage {
        ChatMessage {
            annotations: None,
            role: role.into(),
            content: ChatMessageContent::Text(text.into()),
            tool_calls: None,
            tool_call_id: None,
            name: None,
            reasoning_content: None,
        }
    }

    fn texts(messages: &[ChatMessage]) -> Vec<String> {
        messages.iter().map(|m| m.content.as_text()).collect()
    }

    /// A resumed mixed call runs with the resuming request's instructions,
    /// not those of the request that stopped for the client's tools.
    #[test]
    fn resumed_history_swaps_the_stored_instructions() {
        let stored = vec![msg("system", "Old"), msg("user", "q")];

        let mut swapped = stored.clone();
        replace_instructions(&mut swapped, Some("Old"), Some(&msg("system", "New")));
        assert_eq!(texts(&swapped), ["New", "q"]);

        let mut dropped = stored.clone();
        replace_instructions(&mut dropped, Some("Old"), None);
        assert_eq!(texts(&dropped), ["q"]);

        // A leading system message that is not the stored instructions stays.
        let mut kept = vec![msg("system", "Conversation note"), msg("user", "q")];
        replace_instructions(&mut kept, Some("Old"), None);
        assert_eq!(texts(&kept), ["Conversation note", "q"]);
    }
}
