//! CodingAgentManager — session lifecycle, process management, output buffering.

use crate::types::*;
use dashmap::DashMap;
use lr_config::{CodingAgentType, CodingAgentsConfig, CodingPermissionMode};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::Mutex;
use tracing::{debug, info};

/// Manages all coding agent sessions
pub struct CodingAgentManager {
    /// All sessions, keyed by session ID
    /// Value: (client_id, session) — client_id stored outside Mutex for lockless ownership checks
    sessions: DashMap<SessionId, (String, Arc<Mutex<CodingSession>>)>,
    /// Global config
    config: CodingAgentsConfig,
    /// Max concurrent sessions (atomic so it can be updated without &mut self)
    max_concurrent_sessions: AtomicUsize,
}

impl CodingAgentManager {
    pub fn new(config: CodingAgentsConfig) -> Self {
        let max = config.max_concurrent_sessions;
        Self {
            sessions: DashMap::new(),
            config,
            max_concurrent_sessions: AtomicUsize::new(max),
        }
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
    /// Agents are implicitly enabled when installed — no explicit enable flag.
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

        // Resolve working directory: use provided or create temp dir
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

        // Resolve binary path
        let binary = agent_type.binary_name().to_string();

        // Spawn the agent process
        let process = spawn_agent_process(agent_type, &binary, prompt, &work_dir, &config).await?;

        session.process = Some(process);

        let session_arc = Arc::new(Mutex::new(session));
        self.sessions.insert(
            session_id.clone(),
            (client_id.to_string(), session_arc.clone()),
        );

        // Start background stdout reader
        spawn_output_reader(session_arc);

        info!(
            agent = %agent_type,
            session_id = %session_id,
            "Started coding agent session"
        );

        Ok(StartResponse {
            session_id,
            status: SessionStatus::Active,
        })
    }

    /// Send a message to an existing session
    pub async fn say(
        &self,
        session_id: &str,
        client_id: &str,
        message: &str,
        permission_mode: Option<CodingPermissionMode>,
    ) -> Result<SayResponse, CodingAgentError> {
        let session_arc = self.get_session(session_id, client_id)?;
        let mut session = session_arc.lock().await;

        // If permission mode changed and process is alive, we'd need to restart
        // For now, just update the config for next spawn
        if let Some(mode) = permission_mode {
            session.config.permission_mode = mode;
        }

        match session.status {
            SessionStatus::Active | SessionStatus::AwaitingInput => {
                // Try to write to stdin if available (interactive mode)
                let has_stdin = session
                    .process
                    .as_ref()
                    .and_then(|p| p.stdin.as_ref())
                    .is_some();

                if has_stdin {
                    let process = session.process.as_mut().unwrap();
                    let stdin = process.stdin.as_mut().unwrap();
                    let msg = format!("{}\n", message);
                    stdin
                        .write_all(msg.as_bytes())
                        .await
                        .map_err(|e| CodingAgentError::IoError(e.to_string()))?;
                    stdin
                        .flush()
                        .await
                        .map_err(|e| CodingAgentError::IoError(e.to_string()))?;
                    session.status = SessionStatus::Active;
                    session.last_activity = chrono::Utc::now();
                } else {
                    // No stdin (process was spawned in -p mode with null stdin).
                    // Wait for current process to finish, then auto-resume below.
                    return Err(CodingAgentError::IoError(
                        "Session is still running. Wait for it to complete, then use 'say' to send a follow-up.".to_string()
                    ));
                }
            }
            SessionStatus::Done | SessionStatus::Error | SessionStatus::Interrupted => {
                // Process exited: auto-resume via spawn_follow_up
                let binary = session.agent_type.binary_name().to_string();

                let process = spawn_agent_process(
                    session.agent_type,
                    &binary,
                    message,
                    &session.working_directory,
                    &session.config,
                )
                .await?;

                session.process = Some(process);
                session.status = SessionStatus::Active;
                session.last_activity = chrono::Utc::now();
                session.exit_code = None;
                session.error = None;

                // Need to drop lock before spawning reader
                let session_id_clone = session.id.clone();
                drop(session);

                let session_arc2 = self
                    .sessions
                    .get(&session_id_clone)
                    .map(|r| r.value().1.clone())
                    .ok_or(CodingAgentError::SessionNotFound(session_id_clone))?;
                spawn_output_reader(session_arc2);

                return Ok(SayResponse {
                    session_id: session_id.to_string(),
                    status: SessionStatus::Active,
                });
            }
        }

        let status = session.status.clone();
        Ok(SayResponse {
            session_id: session_id.to_string(),
            status,
        })
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

        let pending = session
            .pending_question
            .as_ref()
            .map(|pq| PendingQuestionInfo {
                id: pq.id.clone(),
                question_type: pq.question_type.clone(),
                questions: pq.questions.clone(),
            });

        Ok(StatusResponse {
            session_id: session_id.to_string(),
            status: session.status.clone(),
            result: session.result.clone(),
            recent_output: session.recent_output(lines),
            pending_question: pending,
            cost_usd: session.cost_usd,
            turn_count: session.turn_count,
        })
    }

    /// Respond to a pending question
    pub async fn respond(
        &self,
        session_id: &str,
        client_id: &str,
        question_id: &str,
        answers: Vec<String>,
    ) -> Result<RespondResponse, CodingAgentError> {
        let session_arc = self.get_session(session_id, client_id)?;
        let mut session = session_arc.lock().await;

        let pending = session
            .pending_question
            .take()
            .ok_or(CodingAgentError::NoPendingQuestion)?;

        if pending.id != question_id {
            // Put it back
            let id = pending.id.clone();
            session.pending_question = Some(pending);
            return Err(CodingAgentError::QuestionIdMismatch {
                expected: id,
                got: question_id.to_string(),
            });
        }

        // Parse answers into an ApprovalResponse
        let response = match pending.question_type {
            QuestionType::ToolApproval | QuestionType::PlanApproval => {
                let answer = answers.first().map(|s| s.as_str()).unwrap_or("deny");
                if answer.starts_with("allow") || answer.starts_with("approve") {
                    ApprovalResponse::Approved
                } else {
                    let reason = answer.split_once(':').map(|(_, r)| r.trim().to_string());
                    ApprovalResponse::Denied { reason }
                }
            }
            QuestionType::Question => ApprovalResponse::Answered { answers },
        };

        // Send response to the waiting executor
        let _ = pending.resolve.send(response);

        session.status = SessionStatus::Active;
        session.last_activity = chrono::Utc::now();

        Ok(RespondResponse {
            session_id: session_id.to_string(),
            status: session.status.clone(),
        })
    }

    /// Interrupt a running session
    pub async fn interrupt(
        &self,
        session_id: &str,
        client_id: &str,
    ) -> Result<InterruptResponse, CodingAgentError> {
        let session_arc = self.get_session(session_id, client_id)?;
        let mut session = session_arc.lock().await;

        if let Some(ref process) = session.process {
            process.cancel.cancel();
        }

        // Try to kill the process group
        if let Some(ref mut process) = session.process {
            let _ = process.child.start_kill();
        }

        session.status = SessionStatus::Interrupted;
        session.last_activity = chrono::Utc::now();

        info!(
            session_id = %session_id,
            "Coding agent session interrupted"
        );

        Ok(InterruptResponse {
            session_id: session_id.to_string(),
            status: SessionStatus::Interrupted,
        })
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

        // Pre-filter by client_id without locking the Mutex
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
                working_directory: session.working_directory.to_string_lossy().to_string(),
                display_text: truncate_prompt(&session.initial_prompt, 80),
                timestamp: session.created_at,
                status: session.status.clone(),
            });
        }
        summaries.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
        summaries
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
            Ok(())
        } else {
            Err(CodingAgentError::SessionNotFound(session_id.to_string()))
        }
    }

    /// Get a session, validating client ownership.
    /// Client ownership is checked against the client_id stored alongside the session
    /// in the DashMap (outside the Mutex), so no lock is needed for validation.
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

/// Spawn an agent process with appropriate CLI arguments
async fn spawn_agent_process(
    agent_type: CodingAgentType,
    binary: &str,
    prompt: &str,
    working_dir: &Path,
    config: &SessionConfig,
) -> Result<AgentProcess, CodingAgentError> {
    use command_group::AsyncCommandGroup;
    use std::process::Stdio;

    let mut cmd = tokio::process::Command::new(binary);
    cmd.current_dir(working_dir);
    // Use null stdin for -p mode: the prompt is passed as CLI args, and piped stdin
    // blocks Claude Code from proceeding. The `say` command handles follow-up messages
    // by spawning a new process when the session has ended.
    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    // Set env vars from config
    for (k, v) in &config.env {
        cmd.env(k, v);
    }

    // Clear env vars that prevent nested sessions (e.g., when spawned from inside Claude Code)
    cmd.env_remove("CLAUDECODE");
    cmd.env_remove("CLAUDE_CODE_ENTRYPOINT");
    cmd.env_remove("CLAUDE_CODE_SESSION_ACCESS_TOKEN");

    // Build agent-specific CLI args
    match agent_type {
        CodingAgentType::ClaudeCode => {
            cmd.arg("-p").arg(prompt);
            cmd.arg("--output-format").arg("stream-json");
            if let Some(ref model) = config.model {
                cmd.arg("--model").arg(model);
            }
            match config.permission_mode {
                CodingPermissionMode::Auto => {
                    cmd.arg("--dangerously-skip-permissions");
                }
                CodingPermissionMode::Plan => {
                    // Claude Code doesn't have a plan-only flag directly
                    // Use system prompt to enforce plan mode
                }
                CodingPermissionMode::Supervised => {
                    // Default behavior
                }
            }
        }
        CodingAgentType::GeminiCli => {
            cmd.arg("-p").arg(prompt);
            if let Some(ref model) = config.model {
                cmd.arg("--model").arg(model);
            }
        }
        CodingAgentType::Codex => {
            cmd.arg(prompt);
            if let Some(ref model) = config.model {
                cmd.arg("--model").arg(model);
            }
            match config.permission_mode {
                CodingPermissionMode::Auto => {
                    cmd.arg("--approval-mode").arg("full-auto");
                }
                CodingPermissionMode::Supervised => {
                    cmd.arg("--approval-mode").arg("suggest");
                }
                CodingPermissionMode::Plan => {
                    cmd.arg("--approval-mode").arg("suggest");
                }
            }
        }
        CodingAgentType::Amp => {
            cmd.arg("--prompt").arg(prompt);
        }
        CodingAgentType::Aider => {
            cmd.arg("--message").arg(prompt);
            cmd.arg("--no-auto-commits");
            if let Some(ref model) = config.model {
                cmd.arg("--model").arg(model);
            }
        }
        CodingAgentType::Cursor => {
            cmd.arg(prompt);
        }
        CodingAgentType::Opencode => {
            cmd.arg("-p").arg(prompt);
            if let Some(ref model) = config.model {
                cmd.arg("--model").arg(model);
            }
        }
        CodingAgentType::QwenCode => {
            cmd.arg(prompt);
        }
        CodingAgentType::Copilot => {
            cmd.arg(prompt);
        }
        CodingAgentType::Droid => {
            cmd.arg(prompt);
        }
    }

    let cancel = tokio_util::sync::CancellationToken::new();

    let group_child: command_group::AsyncGroupChild =
        cmd.group_spawn()
            .map_err(|e: std::io::Error| CodingAgentError::SpawnFailed {
                agent: agent_type.display_name().to_string(),
                reason: e.to_string(),
            })?;

    Ok(AgentProcess {
        child: group_child,
        stdin: None,
        cancel,
    })
}

/// Spawn a background task that reads stdout and appends to the session buffer
fn spawn_output_reader(session: Arc<Mutex<CodingSession>>) {
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
                    let mut s = session.lock().await;
                    s.append_output(line);
                }
                Ok(None) => {
                    // EOF — process exited
                    let mut s = session.lock().await;
                    // Try to get exit code
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
                                // Process still running somehow? Shouldn't happen if stdout closed
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
    });
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

    #[error("No pending question to respond to")]
    NoPendingQuestion,

    #[error("Question ID mismatch: expected {expected}, got {got}")]
    QuestionIdMismatch { expected: String, got: String },

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
        assert!(agents.len() <= CodingAgentType::all().len());
    }

    #[test]
    fn test_truncate_prompt() {
        assert_eq!(truncate_prompt("short", 80), "short");
        assert_eq!(
            truncate_prompt("a very long prompt that exceeds the limit", 20),
            "a very long promp..."
        );
        assert_eq!(truncate_prompt("line1\nline2\nline3", 80), "line1");
    }

    #[test]
    fn test_truncate_prompt_multibyte_utf8() {
        // Should not panic on multi-byte UTF-8 characters
        let emoji_prompt = "🚀🔥💻 This is a prompt with emojis that is quite long";
        let result = truncate_prompt(emoji_prompt, 10);
        assert!(result.ends_with("..."));
        // Should contain valid UTF-8
        assert!(result.chars().count() <= 10);
    }

    #[test]
    fn test_truncate_prompt_exact_boundary() {
        assert_eq!(truncate_prompt("12345", 5), "12345");
        assert_eq!(truncate_prompt("123456", 5), "12...");
    }

    #[test]
    fn test_truncate_prompt_cjk_characters() {
        let cjk = "这是一个很长的中文提示符号";
        let result = truncate_prompt(cjk, 8);
        assert!(result.ends_with("..."));
        assert!(result.chars().count() <= 8);
    }

    #[test]
    fn test_manager_new() {
        let config = test_config();
        let manager = CodingAgentManager::new(config);
        assert_eq!(manager.config().max_concurrent_sessions, 5);
        assert_eq!(manager.config().output_buffer_size, 100);
    }

    #[test]
    fn test_is_agent_enabled_checks_installation() {
        let config = test_config();
        let manager = CodingAgentManager::new(config);
        // is_agent_enabled checks if binary is on PATH
        // We can't guarantee which agents are installed in CI,
        // but the method should not panic for any agent type
        for agent_type in CodingAgentType::all() {
            let _ = manager.is_agent_enabled(*agent_type);
        }
    }

    #[test]
    fn test_enabled_agents_matches_installed() {
        let config = test_config();
        let manager = CodingAgentManager::new(config);
        let enabled = manager.enabled_agents();
        let installed = CodingAgentManager::detect_installed_agents();
        assert_eq!(enabled, installed);
    }

    #[test]
    fn test_update_config() {
        let config = test_config();
        let mut manager = CodingAgentManager::new(config);
        assert_eq!(manager.config().max_concurrent_sessions, 5);

        let mut new_config = test_config();
        new_config.max_concurrent_sessions = 20;
        manager.update_config(new_config);
        assert_eq!(manager.config().max_concurrent_sessions, 20);
    }

    #[tokio::test]
    async fn test_get_session_not_found() {
        let manager = CodingAgentManager::new(test_config());
        let result = manager.get_session("nonexistent", "client-1");
        assert!(result.is_err());
        let err = result.err().unwrap();
        assert!(matches!(err, CodingAgentError::SessionNotFound(ref id) if id == "nonexistent"));
    }

    #[tokio::test]
    async fn test_get_session_client_mismatch() {
        let manager = CodingAgentManager::new(test_config());

        // Manually insert a session to test ownership
        let session = CodingSession::new(
            "test-session".to_string(),
            CodingAgentType::ClaudeCode,
            "client-A".to_string(),
            PathBuf::from("/tmp"),
            SessionConfig {
                model: None,
                permission_mode: CodingPermissionMode::Supervised,
                env: Default::default(),
            },
            "test prompt".to_string(),
            100,
        );
        manager.sessions.insert(
            "test-session".to_string(),
            ("client-A".to_string(), Arc::new(Mutex::new(session))),
        );

        // Owner can access
        let result = manager.get_session("test-session", "client-A");
        assert!(result.is_ok());

        // Different client cannot access
        let result = manager.get_session("test-session", "client-B");
        assert!(result.is_err());
        assert!(matches!(
            result.err().unwrap(),
            CodingAgentError::ClientMismatch
        ));
    }

    #[tokio::test]
    async fn test_list_sessions_filters_by_client() {
        let manager = CodingAgentManager::new(test_config());

        // Insert sessions for different clients
        for (id, client) in [("s1", "client-A"), ("s2", "client-B"), ("s3", "client-A")] {
            let session = CodingSession::new(
                id.to_string(),
                CodingAgentType::ClaudeCode,
                client.to_string(),
                PathBuf::from("/tmp"),
                SessionConfig {
                    model: None,
                    permission_mode: CodingPermissionMode::Supervised,
                    env: Default::default(),
                },
                format!("prompt for {}", id),
                100,
            );
            manager.sessions.insert(
                id.to_string(),
                (client.to_string(), Arc::new(Mutex::new(session))),
            );
        }

        let client_a_sessions = manager.list_sessions("client-A", None, None).await;
        assert_eq!(client_a_sessions.len(), 2);

        let client_b_sessions = manager.list_sessions("client-B", None, None).await;
        assert_eq!(client_b_sessions.len(), 1);

        let client_c_sessions = manager.list_sessions("client-C", None, None).await;
        assert_eq!(client_c_sessions.len(), 0);
    }

    #[tokio::test]
    async fn test_list_sessions_filters_by_agent_type() {
        let manager = CodingAgentManager::new(test_config());

        let session1 = CodingSession::new(
            "s1".to_string(),
            CodingAgentType::ClaudeCode,
            "client-A".to_string(),
            PathBuf::from("/tmp"),
            SessionConfig {
                model: None,
                permission_mode: CodingPermissionMode::Supervised,
                env: Default::default(),
            },
            "claude prompt".to_string(),
            100,
        );
        let session2 = CodingSession::new(
            "s2".to_string(),
            CodingAgentType::Codex,
            "client-A".to_string(),
            PathBuf::from("/tmp"),
            SessionConfig {
                model: None,
                permission_mode: CodingPermissionMode::Auto,
                env: Default::default(),
            },
            "codex prompt".to_string(),
            100,
        );

        manager.sessions.insert(
            "s1".to_string(),
            ("client-A".to_string(), Arc::new(Mutex::new(session1))),
        );
        manager.sessions.insert(
            "s2".to_string(),
            ("client-A".to_string(), Arc::new(Mutex::new(session2))),
        );

        let claude_sessions = manager
            .list_sessions("client-A", Some(CodingAgentType::ClaudeCode), None)
            .await;
        assert_eq!(claude_sessions.len(), 1);
        assert_eq!(claude_sessions[0].session_id, "s1");

        let codex_sessions = manager
            .list_sessions("client-A", Some(CodingAgentType::Codex), None)
            .await;
        assert_eq!(codex_sessions.len(), 1);
        assert_eq!(codex_sessions[0].session_id, "s2");
    }

    #[tokio::test]
    async fn test_list_sessions_limit() {
        let manager = CodingAgentManager::new(test_config());

        for i in 0..10 {
            let id = format!("s{}", i);
            let session = CodingSession::new(
                id.clone(),
                CodingAgentType::ClaudeCode,
                "client-A".to_string(),
                PathBuf::from("/tmp"),
                SessionConfig {
                    model: None,
                    permission_mode: CodingPermissionMode::Supervised,
                    env: Default::default(),
                },
                format!("prompt {}", i),
                100,
            );
            manager
                .sessions
                .insert(id, ("client-A".to_string(), Arc::new(Mutex::new(session))));
        }

        let sessions = manager.list_sessions("client-A", None, Some(3)).await;
        assert_eq!(sessions.len(), 3);
    }

    #[tokio::test]
    async fn test_list_all_sessions() {
        let manager = CodingAgentManager::new(test_config());

        for (id, client) in [("s1", "client-A"), ("s2", "client-B")] {
            let session = CodingSession::new(
                id.to_string(),
                CodingAgentType::ClaudeCode,
                client.to_string(),
                PathBuf::from("/tmp"),
                SessionConfig {
                    model: None,
                    permission_mode: CodingPermissionMode::Supervised,
                    env: Default::default(),
                },
                "prompt".to_string(),
                100,
            );
            manager.sessions.insert(
                id.to_string(),
                (client.to_string(), Arc::new(Mutex::new(session))),
            );
        }

        let all = manager.list_all_sessions().await;
        assert_eq!(all.len(), 2);
    }

    #[tokio::test]
    async fn test_end_session() {
        let manager = CodingAgentManager::new(test_config());

        let session = CodingSession::new(
            "s1".to_string(),
            CodingAgentType::ClaudeCode,
            "client-A".to_string(),
            PathBuf::from("/tmp"),
            SessionConfig {
                model: None,
                permission_mode: CodingPermissionMode::Supervised,
                env: Default::default(),
            },
            "prompt".to_string(),
            100,
        );
        manager.sessions.insert(
            "s1".to_string(),
            ("client-A".to_string(), Arc::new(Mutex::new(session))),
        );

        assert!(manager.end_session("s1").await.is_ok());
        assert!(manager.sessions.is_empty());

        // Ending non-existent session should error
        let result = manager.end_session("nonexistent").await;
        assert!(matches!(
            result.unwrap_err(),
            CodingAgentError::SessionNotFound(_)
        ));
    }

    #[tokio::test]
    async fn test_respond_no_pending_question() {
        let manager = CodingAgentManager::new(test_config());

        let session = CodingSession::new(
            "s1".to_string(),
            CodingAgentType::ClaudeCode,
            "client-A".to_string(),
            PathBuf::from("/tmp"),
            SessionConfig {
                model: None,
                permission_mode: CodingPermissionMode::Supervised,
                env: Default::default(),
            },
            "prompt".to_string(),
            100,
        );
        manager.sessions.insert(
            "s1".to_string(),
            ("client-A".to_string(), Arc::new(Mutex::new(session))),
        );

        let result = manager
            .respond("s1", "client-A", "q1", vec!["allow".to_string()])
            .await;
        assert!(matches!(
            result.unwrap_err(),
            CodingAgentError::NoPendingQuestion
        ));
    }

    #[tokio::test]
    async fn test_respond_question_id_mismatch() {
        let manager = CodingAgentManager::new(test_config());

        let (tx, _rx) = tokio::sync::oneshot::channel();
        let mut session = CodingSession::new(
            "s1".to_string(),
            CodingAgentType::ClaudeCode,
            "client-A".to_string(),
            PathBuf::from("/tmp"),
            SessionConfig {
                model: None,
                permission_mode: CodingPermissionMode::Supervised,
                env: Default::default(),
            },
            "prompt".to_string(),
            100,
        );
        session.pending_question = Some(PendingQuestion {
            id: "correct-id".to_string(),
            question_type: QuestionType::ToolApproval,
            questions: vec![QuestionItem {
                question: "Allow?".to_string(),
                options: vec!["allow".to_string(), "deny".to_string()],
            }],
            resolve: tx,
        });

        manager.sessions.insert(
            "s1".to_string(),
            ("client-A".to_string(), Arc::new(Mutex::new(session))),
        );

        let result = manager
            .respond("s1", "client-A", "wrong-id", vec!["allow".to_string()])
            .await;
        assert!(matches!(
            result.unwrap_err(),
            CodingAgentError::QuestionIdMismatch { .. }
        ));
    }

    #[tokio::test]
    async fn test_respond_approve() {
        let manager = CodingAgentManager::new(test_config());

        let (tx, rx) = tokio::sync::oneshot::channel();
        let mut session = CodingSession::new(
            "s1".to_string(),
            CodingAgentType::ClaudeCode,
            "client-A".to_string(),
            PathBuf::from("/tmp"),
            SessionConfig {
                model: None,
                permission_mode: CodingPermissionMode::Supervised,
                env: Default::default(),
            },
            "prompt".to_string(),
            100,
        );
        session.status = SessionStatus::AwaitingInput;
        session.pending_question = Some(PendingQuestion {
            id: "q1".to_string(),
            question_type: QuestionType::ToolApproval,
            questions: vec![QuestionItem {
                question: "Allow Edit?".to_string(),
                options: vec!["allow".to_string(), "deny".to_string()],
            }],
            resolve: tx,
        });

        manager.sessions.insert(
            "s1".to_string(),
            ("client-A".to_string(), Arc::new(Mutex::new(session))),
        );

        let result = manager
            .respond("s1", "client-A", "q1", vec!["allow".to_string()])
            .await;
        assert!(result.is_ok());
        let resp = result.unwrap();
        assert_eq!(resp.status, SessionStatus::Active);

        // Verify the approval was sent through the channel
        let approval = rx.await.unwrap();
        assert!(matches!(approval, ApprovalResponse::Approved));
    }

    #[tokio::test]
    async fn test_respond_deny_with_reason() {
        let manager = CodingAgentManager::new(test_config());

        let (tx, rx) = tokio::sync::oneshot::channel();
        let mut session = CodingSession::new(
            "s1".to_string(),
            CodingAgentType::ClaudeCode,
            "client-A".to_string(),
            PathBuf::from("/tmp"),
            SessionConfig {
                model: None,
                permission_mode: CodingPermissionMode::Supervised,
                env: Default::default(),
            },
            "prompt".to_string(),
            100,
        );
        session.pending_question = Some(PendingQuestion {
            id: "q1".to_string(),
            question_type: QuestionType::ToolApproval,
            questions: vec![QuestionItem {
                question: "Allow Edit?".to_string(),
                options: vec!["allow".to_string(), "deny".to_string()],
            }],
            resolve: tx,
        });

        manager.sessions.insert(
            "s1".to_string(),
            ("client-A".to_string(), Arc::new(Mutex::new(session))),
        );

        let result = manager
            .respond(
                "s1",
                "client-A",
                "q1",
                vec!["deny: too dangerous".to_string()],
            )
            .await;
        assert!(result.is_ok());

        let approval = rx.await.unwrap();
        match approval {
            ApprovalResponse::Denied { reason } => {
                assert_eq!(reason, Some("too dangerous".to_string()));
            }
            _ => panic!("Expected Denied"),
        }
    }

    #[tokio::test]
    async fn test_respond_question_answers() {
        let manager = CodingAgentManager::new(test_config());

        let (tx, rx) = tokio::sync::oneshot::channel();
        let mut session = CodingSession::new(
            "s1".to_string(),
            CodingAgentType::ClaudeCode,
            "client-A".to_string(),
            PathBuf::from("/tmp"),
            SessionConfig {
                model: None,
                permission_mode: CodingPermissionMode::Supervised,
                env: Default::default(),
            },
            "prompt".to_string(),
            100,
        );
        session.pending_question = Some(PendingQuestion {
            id: "q1".to_string(),
            question_type: QuestionType::Question,
            questions: vec![QuestionItem {
                question: "Which auth method?".to_string(),
                options: vec!["OAuth".to_string(), "JWT".to_string()],
            }],
            resolve: tx,
        });

        manager.sessions.insert(
            "s1".to_string(),
            ("client-A".to_string(), Arc::new(Mutex::new(session))),
        );

        let result = manager
            .respond("s1", "client-A", "q1", vec!["OAuth".to_string()])
            .await;
        assert!(result.is_ok());

        let approval = rx.await.unwrap();
        match approval {
            ApprovalResponse::Answered { answers } => {
                assert_eq!(answers, vec!["OAuth".to_string()]);
            }
            _ => panic!("Expected Answered"),
        }
    }

    #[tokio::test]
    async fn test_status_returns_session_data() {
        let manager = CodingAgentManager::new(test_config());

        let mut session = CodingSession::new(
            "s1".to_string(),
            CodingAgentType::ClaudeCode,
            "client-A".to_string(),
            PathBuf::from("/tmp"),
            SessionConfig {
                model: None,
                permission_mode: CodingPermissionMode::Supervised,
                env: Default::default(),
            },
            "test prompt".to_string(),
            100,
        );
        session.append_output("line 1".to_string());
        session.append_output("line 2".to_string());
        session.append_output("line 3".to_string());

        manager.sessions.insert(
            "s1".to_string(),
            ("client-A".to_string(), Arc::new(Mutex::new(session))),
        );

        let result = manager.status("s1", "client-A", Some(2)).await;
        assert!(result.is_ok());
        let status = result.unwrap();
        assert_eq!(status.status, SessionStatus::Active);
        assert_eq!(status.recent_output.len(), 2);
        assert_eq!(status.recent_output[0], "line 2");
        assert_eq!(status.recent_output[1], "line 3");
    }

    #[test]
    fn test_coding_agent_error_display() {
        let err = CodingAgentError::SessionNotFound("abc".to_string());
        assert_eq!(err.to_string(), "Session not found: abc");
        assert_eq!(err.to_mcp_error(), "Session not found: abc");

        let err = CodingAgentError::ClientMismatch;
        assert_eq!(err.to_string(), "Session belongs to a different client");

        let err = CodingAgentError::TooManySessions { max: 10 };
        assert!(err.to_string().contains("10"));
    }
}
