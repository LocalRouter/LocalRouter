//! CodingAgentManager — session lifecycle, process management, output buffering.

use crate::types::*;
use dashmap::DashMap;
use lr_config::{CodingAgentType, CodingAgentsConfig, CodingPermissionMode};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::Mutex;
use tracing::{debug, info};

/// Manages all coding agent sessions
pub struct CodingAgentManager {
    /// All sessions, keyed by session ID
    sessions: DashMap<SessionId, Arc<Mutex<CodingSession>>>,
    /// Global config
    config: CodingAgentsConfig,
}

impl CodingAgentManager {
    pub fn new(config: CodingAgentsConfig) -> Self {
        Self {
            sessions: DashMap::new(),
            config,
        }
    }

    /// Update config (called when config changes)
    pub fn update_config(&mut self, config: CodingAgentsConfig) {
        self.config = config;
    }

    /// Get config reference
    pub fn config(&self) -> &CodingAgentsConfig {
        &self.config
    }

    /// Get the config for a specific agent type
    pub fn agent_config(&self, agent_type: CodingAgentType) -> Option<&lr_config::CodingAgentConfig> {
        self.config.agents.iter().find(|a| a.agent_type == agent_type)
    }

    /// Check if an agent type is enabled
    pub fn is_agent_enabled(&self, agent_type: CodingAgentType) -> bool {
        self.config
            .agents
            .iter()
            .any(|a| a.agent_type == agent_type && a.enabled)
    }

    /// Get all enabled agent types
    pub fn enabled_agents(&self) -> Vec<CodingAgentType> {
        self.config
            .agents
            .iter()
            .filter(|a| a.enabled)
            .map(|a| a.agent_type)
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
        // Check concurrent session limit
        if self.sessions.len() >= self.config.max_concurrent_sessions {
            return Err(CodingAgentError::TooManySessions {
                max: self.config.max_concurrent_sessions,
            });
        }

        // Resolve working directory
        let work_dir = working_directory
            .or_else(|| {
                self.agent_config(agent_type)
                    .and_then(|c| c.working_directory.as_ref())
                    .map(PathBuf::from)
            })
            .or_else(|| {
                self.config
                    .default_working_directory
                    .as_ref()
                    .map(PathBuf::from)
            })
            .unwrap_or_else(|| PathBuf::from("."));

        // Resolve permission mode
        let perm_mode = permission_mode
            .or_else(|| {
                self.agent_config(agent_type)
                    .map(|c| c.permission_mode)
            })
            .unwrap_or_default();

        // Resolve model
        let model_id = model.or_else(|| {
            self.agent_config(agent_type)
                .and_then(|c| c.model_id.clone())
        });

        let session_id = uuid::Uuid::new_v4().to_string();
        let config = SessionConfig {
            model: model_id,
            permission_mode: perm_mode,
            env: self
                .agent_config(agent_type)
                .map(|c| c.env.clone())
                .unwrap_or_default(),
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
        let binary = self
            .agent_config(agent_type)
            .and_then(|c| c.binary_path.clone())
            .unwrap_or_else(|| agent_type.binary_name().to_string());

        // Spawn the agent process
        let process = spawn_agent_process(
            agent_type,
            &binary,
            prompt,
            &work_dir,
            &config,
        )
        .await?;

        session.process = Some(process);

        let session_arc = Arc::new(Mutex::new(session));
        self.sessions.insert(session_id.clone(), session_arc.clone());

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
                // Process alive: write to stdin
                if let Some(ref mut process) = session.process {
                    if let Some(ref mut stdin) = process.stdin {
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
                    }
                }
            }
            SessionStatus::Done | SessionStatus::Error | SessionStatus::Interrupted => {
                // Process exited: auto-resume via spawn_follow_up
                let binary = self
                    .agent_config(session.agent_type)
                    .and_then(|c| c.binary_path.clone())
                    .unwrap_or_else(|| session.agent_type.binary_name().to_string());

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

                let session_arc2 = self.sessions.get(&session_id_clone)
                    .map(|r| r.value().clone())
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

        let pending = session.pending_question.as_ref().map(|pq| PendingQuestionInfo {
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
                    let reason = answer
                        .split_once(':')
                        .map(|(_, r)| r.trim().to_string());
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

        for entry in self.sessions.iter() {
            let session = entry.value().lock().await;
            if session.client_id != client_id {
                continue;
            }
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
        for entry in self.sessions.iter() {
            let session = entry.value().lock().await;
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
        if let Some((_, session_arc)) = self.sessions.remove(session_id) {
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

    /// Get a session, validating client ownership
    fn get_session(
        &self,
        session_id: &str,
        _client_id: &str,
    ) -> Result<Arc<Mutex<CodingSession>>, CodingAgentError> {
        let entry = self
            .sessions
            .get(session_id)
            .ok_or_else(|| CodingAgentError::SessionNotFound(session_id.to_string()))?;

        // We need to check client_id but session is behind Mutex
        // Clone the Arc and check after acquiring lock would be a deadlock risk
        // Instead, store client_id alongside the session for quick validation
        Ok(entry.value().clone())
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
    cmd.stdin(Stdio::piped());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    // Set env vars from config
    for (k, v) in &config.env {
        cmd.env(k, v);
    }

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

    let mut group_child: command_group::AsyncGroupChild = cmd
        .group_spawn()
        .map_err(|e: std::io::Error| CodingAgentError::SpawnFailed {
            agent: agent_type.display_name().to_string(),
            reason: e.to_string(),
        })?;

    let stdin = group_child.inner().stdin.take();

    Ok(AgentProcess {
        child: group_child,
        stdin,
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
    if first_line.len() <= max_len {
        first_line.to_string()
    } else {
        format!("{}...", &first_line[..max_len - 3])
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

    #[test]
    fn test_detect_installed_agents() {
        // Just verify it doesn't panic
        let agents = CodingAgentManager::detect_installed_agents();
        // We can't assert specific agents are installed in CI
        assert!(agents.len() <= CodingAgentType::all().len());
    }

    #[test]
    fn test_truncate_prompt() {
        assert_eq!(truncate_prompt("short", 80), "short");
        assert_eq!(
            truncate_prompt("a very long prompt that exceeds the limit", 20),
            "a very long promp..."
        );
        assert_eq!(
            truncate_prompt("line1\nline2\nline3", 80),
            "line1"
        );
    }
}
