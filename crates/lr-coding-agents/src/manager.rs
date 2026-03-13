//! CodingAgentManager — session lifecycle, process management, output buffering.
//!
//! Uses BloopAI/vibe-kanban's `executors` crate for robust process management
//! (kill_on_drop, graduated signal escalation, Claude Code control protocol).

use crate::types::*;
use dashmap::DashMap;
use executors::env::{ExecutionEnv, RepoContext};
use executors::executors::{CodingAgent, SpawnedChild, StandardCodingAgentExecutor};
use lr_config::{CodingAgentApprovalMode, CodingAgentType, CodingAgentsConfig, CodingPermissionMode};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::{broadcast, Mutex};
use tracing::{debug, info, warn};

/// Manages all coding agent sessions
pub struct CodingAgentManager {
    /// All sessions, keyed by session ID
    /// Value: (client_id, session) — client_id stored outside Mutex for lockless ownership checks
    sessions: DashMap<SessionId, (String, Arc<Mutex<CodingSession>>)>,
    /// Global config
    config: CodingAgentsConfig,
    /// Max concurrent sessions (atomic so it can be updated without &mut self)
    max_concurrent_sessions: AtomicUsize,
    /// Broadcast channel for session change notifications
    change_tx: broadcast::Sender<()>,
}

impl CodingAgentManager {
    pub fn new(config: CodingAgentsConfig) -> Self {
        let max = config.max_concurrent_sessions;
        let (change_tx, _) = broadcast::channel(16);
        Self {
            sessions: DashMap::new(),
            config,
            max_concurrent_sessions: AtomicUsize::new(max),
            change_tx,
        }
    }

    /// Subscribe to session change notifications
    pub fn subscribe_changes(&self) -> broadcast::Receiver<()> {
        self.change_tx.subscribe()
    }

    /// Notify that sessions have changed
    fn notify_changed(&self) {
        let _ = self.change_tx.send(());
    }

    /// Update config (called when config changes)
    pub fn update_config(&mut self, config: CodingAgentsConfig) {
        self.max_concurrent_sessions
            .store(config.max_concurrent_sessions, Ordering::Relaxed);
        self.config = config;
    }

    /// Update max concurrent sessions at runtime (0 = unlimited)
    pub fn set_max_concurrent_sessions(&self, max: usize) {
        self.max_concurrent_sessions.store(max, Ordering::Relaxed);
    }

    /// Get config reference
    pub fn config(&self) -> &CodingAgentsConfig {
        &self.config
    }

    /// Check if an agent type is available (binary installed on system).
    pub fn is_agent_enabled(&self, agent_type: CodingAgentType) -> bool {
        which::which(agent_type.binary_name()).is_ok()
    }

    /// Get all available agent types (installed on system)
    pub fn enabled_agents(&self) -> Vec<CodingAgentType> {
        CodingAgentType::all()
            .iter()
            .filter(|t| which::which(t.binary_name()).is_ok())
            .copied()
            .collect()
    }

    /// Detect which agents are installed on the system
    pub fn detect_installed_agents() -> Vec<CodingAgentType> {
        CodingAgentType::all()
            .iter()
            .filter(|t| which::which(t.binary_name()).is_ok())
            .copied()
            .collect()
    }

    /// Start a new coding session
    pub async fn start_session(
        &self,
        agent_type: CodingAgentType,
        client_id: &str,
        prompt: &str,
        working_directory: Option<PathBuf>,
        model: Option<String>,
        permission_mode: Option<CodingPermissionMode>,
    ) -> Result<StartResponse, CodingAgentError> {
        // Check concurrent session limit (0 = unlimited)
        let max = self.max_concurrent_sessions.load(Ordering::Relaxed);
        if max > 0 && self.sessions.len() >= max {
            return Err(CodingAgentError::TooManySessions { max });
        }

        let work_dir = working_directory.unwrap_or_else(std::env::temp_dir);
        let perm_mode = permission_mode.unwrap_or_default();

        let session_id = uuid::Uuid::new_v4().to_string();
        let config = SessionConfig {
            model,
            permission_mode: perm_mode,
            env: Default::default(),
        };

        let mut session = CodingSession::new(
            session_id.clone(),
            agent_type,
            client_id.to_string(),
            work_dir.clone(),
            config.clone(),
            prompt.to_string(),
            self.config.output_buffer_size,
        );

        // Spawn via executors crate (robust process management)
        let spawned = spawn_via_executor(
            agent_type,
            prompt,
            &work_dir,
            &config,
            self.config.approval_mode,
            None, // no session_id for initial spawn
        )
        .await?;

        let cancel = spawned
            .cancel
            .unwrap_or_else(tokio_util::sync::CancellationToken::new);

        session.process = Some(AgentProcess {
            child: spawned.child,
            stdin: None, // stdin is managed by the executor's ProtocolPeer
            cancel,
        });

        let session_arc = Arc::new(Mutex::new(session));
        self.sessions.insert(
            session_id.clone(),
            (client_id.to_string(), session_arc.clone()),
        );

        // Start background stdout reader
        spawn_output_reader(session_arc, self.change_tx.clone());

        info!(
            agent = %agent_type,
            session_id = %session_id,
            "Started coding agent session"
        );

        self.notify_changed();

        Ok(StartResponse {
            session_id,
            status: SessionStatus::Active,
        })
    }

    /// Combined say + interrupt: send a message, interrupt, or both.
    ///
    /// - `message` only: send to active session or resume done/error session
    /// - `interrupt` only: gracefully stop the session
    /// - `message` + `interrupt`: interrupt, then resume with the new message
    pub async fn say(
        &self,
        session_id: &str,
        client_id: &str,
        message: Option<&str>,
        interrupt: bool,
        permission_mode: Option<CodingPermissionMode>,
    ) -> Result<SayResponse, CodingAgentError> {
        if message.is_none() && !interrupt {
            return Err(CodingAgentError::IoError(
                "Provide a message, set interrupt to true, or both".to_string(),
            ));
        }

        let session_arc = self.get_session(session_id, client_id)?;
        let mut session = session_arc.lock().await;

        // Update permission mode if changed
        if let Some(mode) = permission_mode {
            session.config.permission_mode = mode;
        }

        match session.status {
            SessionStatus::Active | SessionStatus::AwaitingInput => {
                if interrupt {
                    // Graceful interrupt via cancellation token
                    if let Some(ref process) = session.process {
                        process.cancel.cancel();
                    }
                    // Also send kill to process group as fallback
                    if let Some(ref mut process) = session.process {
                        let _ = process.child.start_kill();
                    }
                    session.status = SessionStatus::Interrupted;
                    session.last_activity = chrono::Utc::now();

                    info!(session_id = %session_id, "Coding agent session interrupted");
                    self.notify_changed();

                    if let Some(msg) = message {
                        // Interrupt + message: drop lock, then resume with follow-up
                        let agent_type = session.agent_type;
                        let work_dir = session.working_directory.clone();
                        let config = session.config.clone();
                        let agent_session_id = session.agent_session_id.clone();
                        let sid = session.id.clone();
                        drop(session);

                        // Brief pause for process cleanup
                        tokio::time::sleep(Duration::from_millis(200)).await;

                        self.resume_session(
                            &sid,
                            agent_type,
                            msg,
                            &work_dir,
                            &config,
                            agent_session_id.as_deref(),
                        )
                        .await?;

                        return Ok(SayResponse {
                            session_id: session_id.to_string(),
                            status: SessionStatus::Active,
                            interrupted: Some(true),
                            resumed: agent_session_id.is_some().then_some(true),
                        });
                    }

                    return Ok(SayResponse {
                        session_id: session_id.to_string(),
                        status: SessionStatus::Interrupted,
                        interrupted: Some(true),
                        resumed: None,
                    });
                }

                // Message only on active session — write to stdin if available
                if let Some(msg) = message {
                    let has_stdin = session
                        .process
                        .as_ref()
                        .and_then(|p| p.stdin.as_ref())
                        .is_some();

                    if has_stdin {
                        use tokio::io::AsyncWriteExt;
                        let process = session.process.as_mut().unwrap();
                        let stdin = process.stdin.as_mut().unwrap();
                        let msg_line = format!("{}\n", msg);
                        stdin
                            .write_all(msg_line.as_bytes())
                            .await
                            .map_err(|e| CodingAgentError::IoError(e.to_string()))?;
                        stdin
                            .flush()
                            .await
                            .map_err(|e| CodingAgentError::IoError(e.to_string()))?;
                        session.status = SessionStatus::Active;
                        session.last_activity = chrono::Utc::now();
                    } else {
                        return Err(CodingAgentError::IoError(
                            "Session is still running. Wait for it to complete, then use 'say' to send a follow-up.".to_string()
                        ));
                    }
                }

                let status = session.status.clone();
                Ok(SayResponse {
                    session_id: session_id.to_string(),
                    status,
                    interrupted: None,
                    resumed: None,
                })
            }
            SessionStatus::Done | SessionStatus::Error | SessionStatus::Interrupted => {
                if message.is_none() && interrupt {
                    // Interrupt on already-stopped session — no-op
                    return Ok(SayResponse {
                        session_id: session_id.to_string(),
                        status: session.status.clone(),
                        interrupted: None,
                        resumed: None,
                    });
                }

                // Resume session with follow-up
                let agent_type = session.agent_type;
                let work_dir = session.working_directory.clone();
                let config = session.config.clone();
                let agent_session_id = session.agent_session_id.clone();
                let sid = session.id.clone();
                drop(session);

                let msg = message.unwrap_or("");
                self.resume_session(
                    &sid,
                    agent_type,
                    msg,
                    &work_dir,
                    &config,
                    agent_session_id.as_deref(),
                )
                .await?;

                Ok(SayResponse {
                    session_id: session_id.to_string(),
                    status: SessionStatus::Active,
                    interrupted: interrupt.then_some(true),
                    resumed: agent_session_id.is_some().then_some(true),
                })
            }
        }
    }

    /// Resume a done/error/interrupted session by spawning a new process
    async fn resume_session(
        &self,
        session_id: &str,
        agent_type: CodingAgentType,
        message: &str,
        work_dir: &Path,
        config: &SessionConfig,
        agent_session_id: Option<&str>,
    ) -> Result<(), CodingAgentError> {
        let spawned = spawn_via_executor(
            agent_type,
            message,
            work_dir,
            config,
            self.config.approval_mode,
            agent_session_id,
        )
        .await?;

        let cancel = spawned
            .cancel
            .unwrap_or_else(tokio_util::sync::CancellationToken::new);

        let session_arc = self
            .sessions
            .get(session_id)
            .map(|r| r.value().1.clone())
            .ok_or_else(|| CodingAgentError::SessionNotFound(session_id.to_string()))?;

        {
            let mut session = session_arc.lock().await;
            session.process = Some(AgentProcess {
                child: spawned.child,
                stdin: None,
                cancel,
            });
            session.status = SessionStatus::Active;
            session.last_activity = chrono::Utc::now();
            session.exit_code = None;
            session.error = None;
        }

        spawn_output_reader(session_arc, self.change_tx.clone());
        self.notify_changed();
        Ok(())
    }

    /// Get session status
    pub async fn status(
        &self,
        session_id: &str,
        client_id: &str,
        output_lines: Option<usize>,
    ) -> Result<StatusResponse, CodingAgentError> {
        let session_arc = self.get_session(session_id, client_id)?;
        let session = session_arc.lock().await;
        let lines = output_lines.unwrap_or(50);

        Ok(StatusResponse {
            session_id: session_id.to_string(),
            status: session.status.clone(),
            result: session.result.clone(),
            recent_output: session.recent_output(lines),
            cost_usd: session.cost_usd,
            turn_count: session.turn_count,
        })
    }

    /// Wait for a session to leave the `Active` state, then return its status.
    pub async fn wait_for_non_active(
        &self,
        session_id: &str,
        client_id: &str,
        timeout: Duration,
        output_lines: Option<usize>,
    ) -> Result<StatusResponse, CodingAgentError> {
        // Check current status — return immediately if already non-active
        {
            let session_arc = self.get_session(session_id, client_id)?;
            let session = session_arc.lock().await;
            if session.status != SessionStatus::Active {
                let lines = output_lines.unwrap_or(50);
                return Ok(StatusResponse {
                    session_id: session_id.to_string(),
                    status: session.status.clone(),
                    result: session.result.clone(),
                    recent_output: session.recent_output(lines),
                    cost_usd: session.cost_usd,
                    turn_count: session.turn_count,
                });
            }
        }

        // Subscribe to change notifications and wait
        let mut rx = self.subscribe_changes();
        let deadline = tokio::time::Instant::now() + timeout;

        loop {
            tokio::select! {
                _ = tokio::time::sleep_until(deadline) => {
                    return self.status(session_id, client_id, output_lines).await;
                }
                recv = rx.recv() => {
                    match recv {
                        Ok(()) | Err(broadcast::error::RecvError::Lagged(_)) => {
                            let session_arc = match self.get_session(session_id, client_id) {
                                Ok(arc) => arc,
                                Err(e) => return Err(e),
                            };
                            let session = session_arc.lock().await;
                            if session.status != SessionStatus::Active {
                                let lines = output_lines.unwrap_or(50);
                                return Ok(StatusResponse {
                                    session_id: session_id.to_string(),
                                    status: session.status.clone(),
                                    result: session.result.clone(),
                                    recent_output: session.recent_output(lines),
                                    cost_usd: session.cost_usd,
                                    turn_count: session.turn_count,
                                });
                            }
                        }
                        Err(broadcast::error::RecvError::Closed) => {
                            return self.status(session_id, client_id, output_lines).await;
                        }
                    }
                }
            }
        }
    }

    /// List sessions for a client
    pub async fn list_sessions(
        &self,
        client_id: &str,
        agent_type: Option<CodingAgentType>,
        limit: Option<usize>,
    ) -> Vec<SessionSummary> {
        let limit = limit.unwrap_or(50);
        let mut summaries = Vec::new();

        let matching_sessions: Vec<_> = self
            .sessions
            .iter()
            .filter(|entry| entry.value().0 == client_id)
            .map(|entry| entry.value().1.clone())
            .collect();

        for session_arc in matching_sessions {
            let session = session_arc.lock().await;
            if let Some(at) = agent_type {
                if session.agent_type != at {
                    continue;
                }
            }
            summaries.push(SessionSummary {
                session_id: session.id.clone(),
                agent_type: session.agent_type,
                client_id: session.client_id.clone(),
                working_directory: session.working_directory.to_string_lossy().to_string(),
                display_text: truncate_prompt(&session.initial_prompt, 80),
                timestamp: session.created_at,
                status: session.status.clone(),
            });
            if summaries.len() >= limit {
                break;
            }
        }

        summaries.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
        summaries
    }

    /// List all sessions (admin)
    pub async fn list_all_sessions(&self) -> Vec<SessionSummary> {
        let mut summaries = Vec::new();
        let all_sessions: Vec<_> = self
            .sessions
            .iter()
            .map(|entry| entry.value().1.clone())
            .collect();

        for session_arc in all_sessions {
            let session = session_arc.lock().await;
            summaries.push(SessionSummary {
                session_id: session.id.clone(),
                agent_type: session.agent_type,
                client_id: session.client_id.clone(),
                working_directory: session.working_directory.to_string_lossy().to_string(),
                display_text: truncate_prompt(&session.initial_prompt, 80),
                timestamp: session.created_at,
                status: session.status.clone(),
            });
        }
        summaries.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
        summaries
    }

    /// Get detailed session info (admin — no client ownership check)
    pub async fn get_session_detail(
        &self,
        session_id: &str,
    ) -> Result<crate::types::SessionDetail, CodingAgentError> {
        let session_arc = self
            .sessions
            .get(session_id)
            .map(|entry| entry.value().1.clone())
            .ok_or_else(|| CodingAgentError::SessionNotFound(session_id.to_string()))?;

        let session = session_arc.lock().await;
        Ok(crate::types::SessionDetail {
            session_id: session.id.clone(),
            agent_type: session.agent_type,
            client_id: session.client_id.clone(),
            working_directory: session.working_directory.to_string_lossy().to_string(),
            display_text: truncate_prompt(&session.initial_prompt, 80),
            status: session.status.clone(),
            created_at: session.created_at,
            recent_output: session.recent_output(200),
            cost_usd: session.cost_usd,
            turn_count: session.turn_count,
            result: session.result.clone(),
            error: session.error.clone(),
            exit_code: session.exit_code,
        })
    }

    /// End a session (admin)
    pub async fn end_session(&self, session_id: &str) -> Result<(), CodingAgentError> {
        if let Some((_, (_, session_arc))) = self.sessions.remove(session_id) {
            let mut session = session_arc.lock().await;
            if let Some(ref process) = session.process {
                process.cancel.cancel();
            }
            if let Some(ref mut process) = session.process {
                let _ = process.child.start_kill();
            }
            info!(session_id = %session_id, "Coding agent session ended by admin");
            self.notify_changed();
            Ok(())
        } else {
            Err(CodingAgentError::SessionNotFound(session_id.to_string()))
        }
    }

    /// Get a session, validating client ownership.
    fn get_session(
        &self,
        session_id: &str,
        client_id: &str,
    ) -> Result<Arc<Mutex<CodingSession>>, CodingAgentError> {
        let entry = self
            .sessions
            .get(session_id)
            .ok_or_else(|| CodingAgentError::SessionNotFound(session_id.to_string()))?;

        let (owner, session_arc) = entry.value();
        if owner != client_id {
            return Err(CodingAgentError::ClientMismatch);
        }

        Ok(session_arc.clone())
    }
}

// ── Executor-based process spawning ──

/// Create an executor instance and spawn the agent process.
///
/// For agents with control protocol support (Claude Code), the executor handles
/// stdin/stdout JSON messaging, approval routing, and graceful interrupts.
/// For all agents, the executor provides kill_on_drop and proper process group management.
async fn spawn_via_executor(
    agent_type: CodingAgentType,
    prompt: &str,
    work_dir: &Path,
    config: &SessionConfig,
    approval_mode: CodingAgentApprovalMode,
    resume_session_id: Option<&str>,
) -> Result<SpawnedChild, CodingAgentError> {
    let env = ExecutionEnv::new(
        RepoContext::new(work_dir.to_path_buf(), Vec::new()),
        false,
        String::new(),
    );

    let executor = build_executor(agent_type, config, approval_mode);

    let spawned = if let Some(sid) = resume_session_id {
        executor
            .spawn_follow_up(work_dir, prompt, sid, None, &env)
            .await
    } else {
        executor.spawn(work_dir, prompt, &env).await
    };

    spawned.map_err(|e| CodingAgentError::SpawnFailed {
        agent: agent_type.display_name().to_string(),
        reason: e.to_string(),
    })
}

/// Build a CodingAgent executor for the given agent type and config.
///
/// Executor structs are constructed via JSON deserialization since their
/// fields are partially private (e.g., `approvals_service`).
fn build_executor(
    agent_type: CodingAgentType,
    config: &SessionConfig,
    approval_mode: CodingAgentApprovalMode,
) -> CodingAgent {
    let is_auto = matches!(approval_mode, CodingAgentApprovalMode::Allow);

    match agent_type {
        CodingAgentType::ClaudeCode => {
            let is_plan = matches!(config.permission_mode, CodingPermissionMode::Plan);
            let is_supervised = matches!(config.permission_mode, CodingPermissionMode::Supervised);

            let mut json = serde_json::json!({
                "plan": is_plan,
                "approvals": is_supervised && !is_auto,
                "dangerously_skip_permissions": is_auto,
            });

            if let Some(ref model) = config.model {
                json["model"] = serde_json::Value::String(model.clone());
            }

            let executor: executors::executors::claude::ClaudeCode =
                serde_json::from_value(json).expect("Failed to build ClaudeCode executor");
            CodingAgent::ClaudeCode(executor)
        }
        CodingAgentType::GeminiCli => {
            let json = build_model_json(config);
            let executor: executors::executors::gemini::Gemini =
                serde_json::from_value(json).expect("Failed to build Gemini executor");
            CodingAgent::Gemini(executor)
        }
        CodingAgentType::Codex => {
            let json = build_model_json(config);
            let executor: executors::executors::codex::Codex =
                serde_json::from_value(json).expect("Failed to build Codex executor");
            CodingAgent::Codex(executor)
        }
        CodingAgentType::Amp => {
            let executor: executors::executors::amp::Amp =
                serde_json::from_value(serde_json::json!({})).expect("Failed to build Amp executor");
            CodingAgent::Amp(executor)
        }
        CodingAgentType::Cursor => {
            let executor: executors::executors::cursor::CursorAgent =
                serde_json::from_value(serde_json::json!({})).expect("Failed to build Cursor executor");
            CodingAgent::CursorAgent(executor)
        }
        CodingAgentType::Opencode => {
            let json = build_model_json(config);
            let executor: executors::executors::opencode::Opencode =
                serde_json::from_value(json).expect("Failed to build Opencode executor");
            CodingAgent::Opencode(executor)
        }
        CodingAgentType::QwenCode => {
            let executor: executors::executors::qwen::QwenCode =
                serde_json::from_value(serde_json::json!({})).expect("Failed to build QwenCode executor");
            CodingAgent::QwenCode(executor)
        }
        CodingAgentType::Copilot => {
            let executor: executors::executors::copilot::Copilot =
                serde_json::from_value(serde_json::json!({})).expect("Failed to build Copilot executor");
            CodingAgent::Copilot(executor)
        }
        CodingAgentType::Droid => {
            let executor: executors::executors::droid::Droid =
                serde_json::from_value(serde_json::json!({})).expect("Failed to build Droid executor");
            CodingAgent::Droid(executor)
        }
        CodingAgentType::Aider => {
            // Aider is not in the executors crate — use ClaudeCode with base command override
            warn!("Aider not supported by executors crate, using base command override");
            let mut json = serde_json::json!({
                "base_command_override": "aider",
                "additional_params": ["--no-auto-commits"],
            });
            if let Some(ref model) = config.model {
                json["model"] = serde_json::Value::String(model.clone());
            }
            let executor: executors::executors::claude::ClaudeCode =
                serde_json::from_value(json).expect("Failed to build Aider executor");
            CodingAgent::ClaudeCode(executor)
        }
    }
}

/// Helper to build JSON with optional model field.
fn build_model_json(config: &SessionConfig) -> serde_json::Value {
    let mut json = serde_json::json!({});
    if let Some(ref model) = config.model {
        json["model"] = serde_json::Value::String(model.clone());
    }
    json
}

/// Spawn a background task that reads stdout and appends to the session buffer.
/// Also parses Claude Code's session ID from stream-json output.
fn spawn_output_reader(session: Arc<Mutex<CodingSession>>, change_tx: broadcast::Sender<()>) {
    tokio::spawn(async move {
        // Take stdout from the process
        let stdout = {
            let mut s = session.lock().await;
            s.process
                .as_mut()
                .and_then(|p| p.child.inner().stdout.take())
        };

        let Some(stdout) = stdout else {
            return;
        };

        let reader = BufReader::new(stdout);
        let mut lines = reader.lines();

        loop {
            match lines.next_line().await {
                Ok(Some(line)) => {
                    // Try to extract agent session ID from Claude Code's stream-json
                    if let Some(sid) = extract_session_id(&line) {
                        let mut s = session.lock().await;
                        if s.agent_session_id.is_none() {
                            debug!(session_id = %sid, "Captured agent session ID");
                            s.agent_session_id = Some(sid);
                        }
                    }

                    let mut s = session.lock().await;
                    s.append_output(line);
                }
                Ok(None) => {
                    // EOF — process exited
                    let mut s = session.lock().await;
                    if let Some(ref mut process) = s.process {
                        match process.child.try_wait() {
                            Ok(Some(status)) => {
                                s.exit_code = status.code();
                                if status.success() {
                                    if s.status == SessionStatus::Active {
                                        s.status = SessionStatus::Done;
                                    }
                                } else if s.status == SessionStatus::Active {
                                    s.status = SessionStatus::Error;
                                    s.error = Some(format!(
                                        "Process exited with code {}",
                                        status.code().unwrap_or(-1)
                                    ));
                                }
                            }
                            Ok(None) => {
                                if s.status == SessionStatus::Active {
                                    s.status = SessionStatus::Done;
                                }
                            }
                            Err(e) => {
                                if s.status == SessionStatus::Active {
                                    s.status = SessionStatus::Error;
                                    s.error = Some(format!("Failed to get exit status: {}", e));
                                }
                            }
                        }
                    }
                    break;
                }
                Err(e) => {
                    let mut s = session.lock().await;
                    s.append_output(format!("[error reading output: {}]", e));
                    break;
                }
            }
        }

        debug!("Output reader finished for session");
        let _ = change_tx.send(());
    });
}

/// Extract Claude Code's session ID from stream-json output.
/// Claude Code emits `{"type":"system","session_id":"..."}` early in the stream.
fn extract_session_id(line: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    if v.get("type")?.as_str()? == "system" {
        v.get("session_id")?.as_str().map(String::from)
    } else {
        None
    }
}

fn truncate_prompt(prompt: &str, max_len: usize) -> String {
    let first_line = prompt.lines().next().unwrap_or(prompt);
    if first_line.chars().count() <= max_len {
        first_line.to_string()
    } else {
        let truncated: String = first_line.chars().take(max_len.saturating_sub(3)).collect();
        format!("{}...", truncated)
    }
}

/// Errors from coding agent operations
#[derive(Debug, thiserror::Error)]
pub enum CodingAgentError {
    #[error("Session not found: {0}")]
    SessionNotFound(String),

    #[error("Session belongs to a different client")]
    ClientMismatch,

    #[error("Too many concurrent sessions (max: {max})")]
    TooManySessions { max: usize },

    #[error("Failed to spawn {agent}: {reason}")]
    SpawnFailed { agent: String, reason: String },

    #[error("I/O error: {0}")]
    IoError(String),

    #[error("Agent not enabled: {0}")]
    AgentNotEnabled(String),
}

impl CodingAgentError {
    pub fn to_mcp_error(&self) -> String {
        self.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lr_config::CodingAgentsConfig;

    fn test_config() -> CodingAgentsConfig {
        CodingAgentsConfig {
            max_concurrent_sessions: 5,
            output_buffer_size: 100,
            ..Default::default()
        }
    }

    #[test]
    fn test_detect_installed_agents() {
        let agents = CodingAgentManager::detect_installed_agents();
        // Just verify it doesn't panic
        assert!(agents.len() <= CodingAgentType::all().len());
    }

    #[test]
    fn test_manager_config() {
        let config = test_config();
        let manager = CodingAgentManager::new(config.clone());
        assert_eq!(manager.config().max_concurrent_sessions, 5);
    }

    #[test]
    fn test_extract_session_id() {
        assert_eq!(
            extract_session_id(r#"{"type":"system","session_id":"abc-123"}"#),
            Some("abc-123".to_string())
        );
        assert_eq!(
            extract_session_id(r#"{"type":"assistant","content":"hello"}"#),
            None
        );
        assert_eq!(extract_session_id("not json"), None);
        assert_eq!(extract_session_id(""), None);
    }

    #[test]
    fn test_truncate_prompt() {
        assert_eq!(truncate_prompt("short", 10), "short");
        assert_eq!(truncate_prompt("a long prompt that exceeds", 10), "a long ...");
        assert_eq!(truncate_prompt("line1\nline2", 20), "line1");
    }
}
