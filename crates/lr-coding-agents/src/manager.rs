//! CodingAgentManager — session lifecycle, process management, output buffering.
//!
//! Uses BloopAI/vibe-kanban's `executors` crate for robust process management
//! (kill_on_drop, graduated signal escalation, Claude Code control protocol).

use crate::types::*;
use dashmap::DashMap;
use executors::approvals::ExecutorApprovalService;
use executors::env::{ExecutionEnv, RepoContext};
use executors::executors::{CodingAgent, SpawnedChild, StandardCodingAgentExecutor};
use lr_config::{
    CodingAgentApprovalMode, CodingAgentType, CodingAgentsConfig, CodingPermissionMode,
};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::{broadcast, Mutex};
use tracing::{debug, info, warn};

/// Per-session approval-service factory. Returning a fresh service per
/// spawn lets callers scope approval-pending state to a specific session
/// — Direktor uses this to publish `agent.approval_required` events
/// keyed by Direktor's own session id while the upstream
/// `executors::ExecutorApprovalService` future is held.
pub type ApprovalServiceFactory =
    Arc<dyn Fn(&SessionId) -> Arc<dyn ExecutorApprovalService> + Send + Sync>;

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
    /// Optional factory for a custom [`ExecutorApprovalService`]. When
    /// set, the manager calls it for every spawned session and installs
    /// the returned service via [`StandardCodingAgentExecutor::use_approvals`]
    /// — overriding whatever the default approval flow (popup / noop)
    /// would otherwise install. Direktor uses this to route approval
    /// requests onto its EventBus instead of LocalRouter's popup UI.
    ///
    /// Held behind a `RwLock` so callers that already hold an
    /// `Arc<CodingAgentManager>` can install / replace it after
    /// construction (the Direktor adapter constructs the manager and
    /// the factory in different orders depending on the lifecycle).
    approval_service_factory: std::sync::RwLock<Option<ApprovalServiceFactory>>,
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
            approval_service_factory: std::sync::RwLock::new(None),
        }
    }

    /// Install a per-session [`ExecutorApprovalService`] factory at
    /// construction time.
    ///
    /// When set, the factory is invoked once per spawned session
    /// (initial start *and* every resume / follow-up) and the returned
    /// service is attached via the executor's `use_approvals` hook —
    /// taking precedence over the popup / noop default the
    /// `CodingAgentApprovalMode` config would otherwise pick.
    ///
    /// Builder-style; returns `Self` so it composes with
    /// [`CodingAgentManager::new`]. For runtime install on an
    /// already-constructed manager (held behind an `Arc`) use
    /// [`Self::set_approval_service_factory`] which only requires
    /// `&self`.
    pub fn with_approval_service_factory(self, factory: ApprovalServiceFactory) -> Self {
        *self
            .approval_service_factory
            .write()
            .unwrap_or_else(|p| p.into_inner()) = Some(factory);
        self
    }

    /// Replace the approval-service factory after construction. `None`
    /// reverts to the default mode-driven approval flow. Takes `&self`
    /// (interior mutability via `RwLock`) so callers holding an `Arc`
    /// can swap factories at runtime.
    pub fn set_approval_service_factory(&self, factory: Option<ApprovalServiceFactory>) {
        *self
            .approval_service_factory
            .write()
            .unwrap_or_else(|p| p.into_inner()) = factory;
    }

    /// Whether a custom approval-service factory has been installed.
    pub fn has_custom_approval_service(&self) -> bool {
        self.approval_service_factory
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .is_some()
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

    /// The user's explicit `binary_path` override for an agent, if it is set
    /// and still points at something that exists.
    ///
    /// Deliberately separate from [`Self::resolve_binary`]: spawning only
    /// diverts from an agent's normal launch command when the user asked for
    /// it, never because auto-detection happened to find a binary.
    pub fn configured_binary_path(&self, agent_type: CodingAgentType) -> Option<PathBuf> {
        let configured = self
            .config
            .agents
            .iter()
            .find(|a| a.agent_type == agent_type)
            .and_then(|a| a.binary_path.as_ref())
            .filter(|p| !p.trim().is_empty())?;

        let path = PathBuf::from(configured.trim());
        if path.exists() {
            return Some(path);
        }

        // An override pointing at something that no longer exists is worth
        // surfacing: silently falling back would make the override look like
        // it took effect.
        tracing::warn!(
            "Configured binary_path for {} does not exist: {configured}",
            agent_type.display_name()
        );
        None
    }

    /// Resolve the executable for an agent, honouring a configured override.
    ///
    /// A user-set `binary_path` wins outright — it is the escape hatch for
    /// installs we cannot discover (wrappers, non-standard names). Otherwise
    /// we search the way the user's terminal does; see
    /// [`lr_utils::binary::find_binary`] for why the process PATH alone is not
    /// enough in a GUI app.
    pub fn resolve_binary(&self, agent_type: CodingAgentType) -> Option<PathBuf> {
        self.configured_binary_path(agent_type)
            .or_else(|| lr_utils::binary::find_binary(agent_type.binary_name()))
    }

    /// Check if an agent type is available (binary installed on system).
    pub fn is_agent_enabled(&self, agent_type: CodingAgentType) -> bool {
        self.resolve_binary(agent_type).is_some()
    }

    /// Get all available agent types (installed on system)
    pub fn enabled_agents(&self) -> Vec<CodingAgentType> {
        CodingAgentType::all()
            .iter()
            .filter(|t| self.is_agent_enabled(**t))
            .copied()
            .collect()
    }

    /// Detect which agents are installed on the system
    ///
    /// Associated function, so it cannot consult per-agent `binary_path`
    /// overrides; prefer [`Self::enabled_agents`] when a manager is at hand.
    pub fn detect_installed_agents() -> Vec<CodingAgentType> {
        CodingAgentType::all()
            .iter()
            .filter(|t| lr_utils::binary::find_binary(t.binary_name()).is_some())
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

        // Spawn via executors crate (robust process management).
        // Materialise the approval service before the await so the
        // RwLockReadGuard isn't held across the .await boundary
        // (it's !Send).
        let approval_override = {
            let guard = self
                .approval_service_factory
                .read()
                .unwrap_or_else(|p| p.into_inner());
            guard.as_ref().map(|factory| factory(&session_id))
        };
        let spawned = spawn_via_executor(
            agent_type,
            prompt,
            &work_dir,
            &config,
            self.config.approval_mode,
            None, // no session_id for initial spawn
            approval_override,
            self.configured_binary_path(agent_type),
        )
        .await?;

        let cancel = spawned
            .cancel
            .unwrap_or_else(tokio_util::sync::CancellationToken::new);

        session.process = Some(AgentProcess {
            child: spawned.child,
            stdin: None, // stdin is managed by the executor's ProtocolPeer
            cancel,
            reader_cancel: tokio_util::sync::CancellationToken::new(),
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
            SessionStatus::Active => {
                if interrupt {
                    // Graceful interrupt via cancellation token first
                    // (triggers ProtocolPeer to send interrupt via control protocol)
                    if let Some(ref process) = session.process {
                        process.cancel.cancel();
                    }
                    session.status = SessionStatus::Interrupted;
                    session.last_activity = chrono::Utc::now();

                    info!(session_id = %session_id, "Coding agent session interrupted");
                    self.notify_changed();

                    if let Some(msg) = message {
                        // Interrupt + message: drop lock, wait for graceful shutdown, then resume
                        let agent_type = session.agent_type;
                        let work_dir = session.working_directory.clone();
                        let config = session.config.clone();
                        let agent_session_id = session.agent_session_id.clone();
                        let sid = session.id.clone();
                        drop(session);

                        // Wait for the process to exit gracefully after interrupt.
                        // The cancellation token triggers the control protocol interrupt;
                        // we give it time to finish before spawning the follow-up.
                        tokio::time::sleep(Duration::from_millis(500)).await;

                        // If process hasn't exited yet, force kill as fallback
                        if let Some(entry) = self.sessions.get(&sid) {
                            let mut s = entry.value().1.lock().await;
                            if let Some(ref mut process) = s.process {
                                let _ = process.child.start_kill();
                            }
                        }

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

                    // Interrupt only (no message): kill process after grace period
                    let sid = session.id.clone();
                    drop(session);

                    tokio::time::sleep(Duration::from_millis(500)).await;
                    if let Some(entry) = self.sessions.get(&sid) {
                        let mut s = entry.value().1.lock().await;
                        if let Some(ref mut process) = s.process {
                            let _ = process.child.start_kill();
                        }
                    }

                    return Ok(SayResponse {
                        session_id: session_id.to_string(),
                        status: SessionStatus::Interrupted,
                        interrupted: Some(true),
                        resumed: None,
                    });
                }

                // Message only on active session.
                // The executor's ProtocolPeer manages stdin internally — we cannot
                // write to it directly. For active sessions, the user must wait for
                // completion, then send a follow-up (which uses spawn_follow_up).
                if message.is_some() {
                    return Err(CodingAgentError::IoError(
                        "Session is still running. Wait for it to complete or use interrupt=true, then send a follow-up message.".to_string()
                    ));
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
                if message.is_none() {
                    // No message — interrupt on stopped session is a no-op
                    return Ok(SayResponse {
                        session_id: session_id.to_string(),
                        status: session.status.clone(),
                        interrupted: None,
                        resumed: None,
                    });
                }

                // Resume session with follow-up (message is guaranteed Some here)
                let msg = message.unwrap();
                let agent_type = session.agent_type;
                let work_dir = session.working_directory.clone();
                let config = session.config.clone();
                let agent_session_id = session.agent_session_id.clone();
                let sid = session.id.clone();
                drop(session);

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

    /// Resume a done/error/interrupted session by spawning a new process.
    ///
    /// Cancels the old output reader before replacing the process to avoid
    /// two concurrent readers appending to the same buffer.
    async fn resume_session(
        &self,
        session_id: &str,
        agent_type: CodingAgentType,
        message: &str,
        work_dir: &Path,
        config: &SessionConfig,
        agent_session_id: Option<&str>,
    ) -> Result<(), CodingAgentError> {
        let session_arc = self
            .sessions
            .get(session_id)
            .map(|r| r.value().1.clone())
            .ok_or_else(|| CodingAgentError::SessionNotFound(session_id.to_string()))?;

        // Cancel the old output reader before spawning a new process.
        // This prevents two concurrent readers from racing on the buffer.
        {
            let session = session_arc.lock().await;
            if let Some(ref process) = session.process {
                process.reader_cancel.cancel();
            }
        }

        // Brief yield to let the old reader task notice cancellation
        tokio::task::yield_now().await;

        let spawned = spawn_via_executor(
            agent_type,
            message,
            work_dir,
            config,
            self.config.approval_mode,
            agent_session_id,
            {
                let guard = self
                    .approval_service_factory
                    .read()
                    .unwrap_or_else(|p| p.into_inner());
                guard
                    .as_ref()
                    .map(|factory| factory(&session_id.to_string()))
            },
            self.configured_binary_path(agent_type),
        )
        .await?;

        let cancel = spawned
            .cancel
            .unwrap_or_else(tokio_util::sync::CancellationToken::new);

        {
            let mut session = session_arc.lock().await;
            session.process = Some(AgentProcess {
                child: spawned.child,
                stdin: None,
                cancel,
                reader_cancel: tokio_util::sync::CancellationToken::new(),
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

        summaries.sort_by_key(|s| std::cmp::Reverse(s.timestamp));
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
        summaries.sort_by_key(|s| std::cmp::Reverse(s.timestamp));
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

    /// End a session (admin).
    ///
    /// Kills the process and cancels the output reader BEFORE removing from
    /// the session map, so cleanup completes while the session is still findable.
    pub async fn end_session(&self, session_id: &str) -> Result<(), CodingAgentError> {
        // First, kill the process while the session is still in the map
        let session_arc = self
            .sessions
            .get(session_id)
            .map(|entry| entry.value().1.clone())
            .ok_or_else(|| CodingAgentError::SessionNotFound(session_id.to_string()))?;

        {
            let mut session = session_arc.lock().await;
            if let Some(ref process) = session.process {
                process.reader_cancel.cancel();
                process.cancel.cancel();
            }
            if let Some(ref mut process) = session.process {
                let _ = process.child.start_kill();
            }
        }

        // Now remove from the map
        self.sessions.remove(session_id);
        info!(session_id = %session_id, "Coding agent session ended by admin");
        self.notify_changed();
        Ok(())
    }

    /// Enumerate sessions visible on disk, augmented with cheap
    /// live-process detection. See [`crate::discovery`] for per-agent
    /// feasibility (Claude Code + ACP-harness agents return real
    /// entries; everything else returns empty).
    ///
    /// Best-effort throughout — IO errors per file are logged and
    /// skipped, missing roots yield empty, and a failure to snapshot
    /// running processes degrades each entry's `liveness` to
    /// [`crate::discovery::Liveness::Unknown`].
    pub async fn discover_disk_sessions(
        &self,
        filter: crate::discovery::DiscoverFilter,
    ) -> Result<Vec<crate::discovery::DiscoveredSession>, CodingAgentError> {
        let agents: Vec<CodingAgentType> = filter
            .agents
            .clone()
            .unwrap_or_else(|| CodingAgentType::all().to_vec());

        // Per-agent disk scan. Spawned to a blocking pool so std::fs
        // calls don't pin the async runtime — fast in practice but
        // proper hygiene.
        let cwd = filter.working_directory.clone();
        let session_id = filter.session_id.clone();
        let mut all = tokio::task::spawn_blocking(move || {
            let mut combined = Vec::new();
            for agent in agents {
                let d = crate::discovery::discoverer_for(agent);
                combined.extend(d.discover(cwd.as_deref(), session_id.as_deref()));
            }
            combined
        })
        .await
        .map_err(|e| CodingAgentError::IoError(format!("discovery join: {e}")))?;

        // Sort newest-first; truncate to limit.
        all.sort_by_key(|s| std::cmp::Reverse(s.last_active_at));
        let limit = if filter.limit == 0 {
            crate::discovery::DEFAULT_LIMIT
        } else {
            filter.limit
        };
        all.truncate(limit);

        // Live-process scan — single sweep across the whole result set.
        // Each session contributes two needles: the raw session id
        // (matches Claude Code's `--resume <id>` argv token) and
        // `<id>.jsonl` (matches the ACP harness's session-file path on
        // argv). We can't tell from outside which form is on the target
        // process; LivePid from either promotes the entry.
        let needles_owned: Vec<String> = all
            .iter()
            .flat_map(|s| {
                vec![
                    s.agent_session_id.clone(),
                    format!("{}.jsonl", s.agent_session_id),
                ]
            })
            .collect();
        let needle_count = needles_owned.len();
        let liveness_pairs = tokio::task::spawn_blocking(move || {
            let refs: Vec<&str> = needles_owned.iter().map(|s| s.as_str()).collect();
            crate::discovery::detect_live_pids(&refs)
        })
        .await
        .unwrap_or_else(|_| {
            // sysinfo failed → every entry stays Unknown.
            vec![crate::discovery::Liveness::Unknown; needle_count]
        });

        // Each session contributes two needles in the order
        // [agent_session_id, "<id>.jsonl"]. Promote LivePid from either.
        for (idx, sess) in all.iter_mut().enumerate() {
            let a = liveness_pairs
                .get(idx * 2)
                .cloned()
                .unwrap_or(crate::discovery::Liveness::Unknown);
            let b = liveness_pairs
                .get(idx * 2 + 1)
                .cloned()
                .unwrap_or(crate::discovery::Liveness::Unknown);
            sess.liveness = match (a, b) {
                (crate::discovery::Liveness::LivePid { pid }, _)
                | (_, crate::discovery::Liveness::LivePid { pid }) => {
                    crate::discovery::Liveness::LivePid { pid }
                }
                (crate::discovery::Liveness::NotFound, _)
                | (_, crate::discovery::Liveness::NotFound) => crate::discovery::Liveness::NotFound,
                _ => crate::discovery::Liveness::Unknown,
            };
        }

        Ok(all)
    }

    /// Spawn a new tracked process for a session this manager didn't
    /// originally create, resuming via the agent's native `--resume`
    /// (or equivalent). The session enters the manager's map so
    /// subsequent `say` / `status` / `end_session` calls all work.
    ///
    /// Returns the **manager's** session id — distinct from
    /// `agent_session_id`, which is the agent's own internal id stashed
    /// on the new entry.
    pub async fn start_session_with_resume(
        &self,
        agent_type: CodingAgentType,
        client_id: &str,
        prompt: &str,
        working_directory: PathBuf,
        model: Option<String>,
        permission_mode: Option<CodingPermissionMode>,
        agent_session_id: String,
    ) -> Result<StartResponse, CodingAgentError> {
        let max = self.max_concurrent_sessions.load(Ordering::Relaxed);
        if max > 0 && self.sessions.len() >= max {
            return Err(CodingAgentError::TooManySessions { max });
        }

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
            working_directory.clone(),
            config.clone(),
            prompt.to_string(),
            self.config.output_buffer_size,
        );
        // Pre-record the agent's own id so the resume path threads
        // `--resume <agent_session_id>` immediately.
        session.agent_session_id = Some(agent_session_id.clone());

        let approval_override = {
            let guard = self
                .approval_service_factory
                .read()
                .unwrap_or_else(|p| p.into_inner());
            guard.as_ref().map(|factory| factory(&session_id))
        };
        let spawned = spawn_via_executor(
            agent_type,
            prompt,
            &working_directory,
            &config,
            self.config.approval_mode,
            Some(&agent_session_id),
            approval_override,
            self.configured_binary_path(agent_type),
        )
        .await?;

        let cancel = spawned
            .cancel
            .unwrap_or_else(tokio_util::sync::CancellationToken::new);

        session.process = Some(AgentProcess {
            child: spawned.child,
            stdin: None,
            cancel,
            reader_cancel: tokio_util::sync::CancellationToken::new(),
        });

        let session_arc = Arc::new(Mutex::new(session));
        self.sessions.insert(
            session_id.clone(),
            (client_id.to_string(), session_arc.clone()),
        );

        spawn_output_reader(session_arc, self.change_tx.clone());

        info!(
            agent = %agent_type,
            session_id = %session_id,
            agent_session_id = %agent_session_id,
            "Attached to existing coding-agent session"
        );

        self.notify_changed();

        Ok(StartResponse {
            session_id,
            status: SessionStatus::Active,
        })
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
    approval_override: Option<Arc<dyn ExecutorApprovalService>>,
    configured_binary: Option<PathBuf>,
) -> Result<SpawnedChild, CodingAgentError> {
    let mut env = ExecutionEnv::new(
        RepoContext::new(work_dir.to_path_buf(), Vec::new()),
        false,
        String::new(),
    );

    // Merge session-specific environment variables
    env.merge(&config.env);

    // Clear env vars that prevent nested sessions (e.g., when LocalRouter itself
    // is running inside a Claude Code session). Setting to empty effectively removes
    // them since ExecutionEnv applies these via Command::env() which overrides inherited vars.
    env.insert("CLAUDECODE", "");
    env.insert("CLAUDE_CODE_ENTRYPOINT", "");
    env.insert("CLAUDE_CODE_SESSION_ACCESS_TOKEN", "");

    // Agents the executors crate has no support for are spawned directly.
    // Borrowing another agent's executor does not work: each one hard-codes
    // the flags of the CLI it was written for and `base_command_override`
    // replaces only the base command, not those flags.
    if let Some(args) = direct_cli_args(agent_type, prompt, config, resume_session_id) {
        return spawn_direct(agent_type, &args, work_dir, &env, configured_binary).await;
    }

    let mut executor = build_executor(agent_type, config, approval_mode, configured_binary)?;
    if let Some(svc) = approval_override {
        // Plug an externally supplied approval service into whichever
        // executor variant `build_executor` produced. The default impl
        // on `StandardCodingAgentExecutor` is a no-op, so calling this
        // for agents that don't implement approvals is harmless.
        executor.use_approvals(svc);
    }

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

/// Command-line arguments for agents we spawn ourselves, or `None` for
/// agents the executors crate handles.
///
/// Both CLIs here take the prompt as a flag value and emit plain text on
/// stdout, which is exactly what the session output reader consumes.
///
/// Flag sets were verified against the installed binaries (agy 1.1.13,
/// aider) rather than taken from documentation.
fn direct_cli_args(
    agent_type: CodingAgentType,
    prompt: &str,
    config: &SessionConfig,
    resume_session_id: Option<&str>,
) -> Option<Vec<String>> {
    let is_auto = matches!(config.permission_mode, CodingPermissionMode::Auto);
    let is_plan = matches!(config.permission_mode, CodingPermissionMode::Plan);
    let mut args: Vec<String> = Vec::new();

    match agent_type {
        CodingAgentType::Antigravity => {
            // `text` is agy's default, but state it so a future change to
            // that default cannot silently turn the output into JSON that
            // the line reader would surface as noise.
            args.extend(["--output-format".into(), "text".into()]);

            if let Some(model) = &config.model {
                args.extend(["--model".into(), model.clone()]);
            }
            if is_auto {
                args.push("--dangerously-skip-permissions".into());
            }
            if is_plan {
                args.extend(["--mode".into(), "plan".into()]);
            }
            if let Some(sid) = resume_session_id {
                args.extend(["--conversation".into(), sid.to_string()]);
            }

            // `--print` takes the prompt as its value and must come last so
            // the prompt cannot be parsed as a flag.
            args.extend(["--print".into(), prompt.to_string()]);
        }
        CodingAgentType::Aider => {
            // Aider is interactive by default; --message runs one prompt and
            // exits.
            //
            // --no-pretty and --no-stream matter for correctness, not
            // cosmetics: pretty mode redraws lines with ANSI escapes, which
            // the line-oriented output reader would record as garbage.
            //
            // --no-auto-commits keeps aider from committing to the user's
            // repository behind their back.
            args.extend([
                "--no-pretty".into(),
                "--no-stream".into(),
                "--no-auto-commits".into(),
                "--no-check-update".into(),
            ]);

            if let Some(model) = &config.model {
                args.extend(["--model".into(), model.clone()]);
            }
            if is_auto {
                args.push("--yes-always".into());
            }
            // Aider has no resume-by-id concept; a follow-up starts fresh.

            args.extend(["--message".into(), prompt.to_string()]);
        }
        _ => return None,
    }

    Some(args)
}

/// Spawn a CLI agent directly, bypassing the executors crate.
async fn spawn_direct(
    agent_type: CodingAgentType,
    args: &[String],
    work_dir: &Path,
    env: &ExecutionEnv,
    configured_binary: Option<PathBuf>,
) -> Result<SpawnedChild, CodingAgentError> {
    use std::process::Stdio;
    use workspace_utils::command_ext::GroupSpawnNoWindowExt;

    // An explicit binary_path wins; otherwise resolve to an absolute path,
    // since the app's own PATH may not contain it.
    let binary = configured_binary
        .or_else(|| lr_utils::binary::find_binary(agent_type.binary_name()))
        .ok_or_else(|| CodingAgentError::SpawnFailed {
            agent: agent_type.display_name().to_string(),
            reason: format!("{} not found on PATH", agent_type.binary_name()),
        })?;

    // Collect every environment variable before building the command. Inside
    // a Flatpak sandbox these are folded into `flatpak-spawn --env=` flags, so
    // they must all be known up front rather than applied to the Command.
    let mut spawn_env: Vec<(String, String)> = Vec::new();
    // These CLIs shell out to their own toolchain (aider is a Python entry
    // point, agy runs git), so hand them the user's real PATH. Under Flatpak
    // `shell_path()` reports the *host's* login-shell PATH, which is what the
    // host-side process needs.
    //
    // ExecutionEnv wins if it sets its own PATH, matching the previous
    // ordering where apply_to_command ran after this. Skipping the push
    // rather than emitting both and relying on last-wins matters under
    // Flatpak: duplicates would become two conflicting `--env=PATH=` flags.
    if !env.contains_key("PATH") {
        if let Some(path) = lr_utils::binary::shell_path() {
            spawn_env.push(("PATH".to_string(), path));
        }
    }
    spawn_env.extend(env.vars.iter().map(|(k, v)| (k.clone(), v.clone())));

    let invocation =
        lr_utils::sandbox::host_invocation(&binary.to_string_lossy(), spawn_env, Some(work_dir));

    let mut command = tokio::process::Command::new(&invocation.program);
    command
        .kill_on_drop(true)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .args(&invocation.leading_args)
        .args(args);
    for (key, value) in &invocation.envs {
        command.env(key, value);
    }
    // When proxying, the working directory travels as `--directory=`; the host
    // path means nothing to the `flatpak-spawn` process itself.
    if !lr_utils::sandbox::current().needs_host_proxy() {
        command.current_dir(work_dir);
    }

    let mut child = command
        .group_spawn_no_window()
        .map_err(|e| CodingAgentError::SpawnFailed {
            agent: agent_type.display_name().to_string(),
            reason: format!("failed to start {}: {e}", binary.display()),
        })?;

    // Drain stderr. The session reader only consumes stdout, and an unread
    // pipe would eventually fill and block the child — so this is required
    // for correctness, not just for the log line it produces.
    if let Some(stderr) = child.inner().stderr.take() {
        let agent = agent_type.display_name().to_string();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if !line.trim().is_empty() {
                    warn!(agent = %agent, "stderr: {line}");
                }
            }
        });
    }

    Ok(child.into())
}

/// Build a CodingAgent executor for the given agent type and config.
///
/// Executor structs are constructed via JSON deserialization since their
/// fields are partially private (e.g., `approvals_service`).
///
/// `Auto` mode wires each agent's native most-AI-permissive flag rather
/// than a flat skip-all. The choice per agent follows each CLI's own
/// notion of "auto":
///
/// - **Claude Code**: `--permission-mode=auto` (the AI-classifier mode
///   added in late 2025 — auto-approves common operations and bubbles
///   up risky ones via the existing `--permission-prompt-tool=stdio`
///   bridge). Achieved by leaving `approvals: true` (which makes the
///   executors crate add `--permission-prompt-tool=stdio
///   --permission-mode=bypassPermissions`) and then appending
///   `--permission-mode=auto` via `cmd.additional_params` —
///   argparse "last flag wins" overrides the bypass with auto, while
///   the prompt-tool stays wired so bubble-ups reach the host.
/// - **Codex**: `--full-auto` semantics —
///   `ask_for_approval=OnRequest` + `sandbox=workspace_write`. Lets
///   Codex run freely inside the workspace and bubble up sandbox
///   escapes.
/// - **Gemini CLI**: `--yolo` (set via `yolo: true`).
/// - **Opencode**: `auto_approve: true` (executors flag).
/// - **Cursor**: `force: true` ("Force allow commands unless
///   explicitly denied").
/// - **Amp**: `dangerously_allow_all: true`.
/// - **Copilot**: `allow_all_tools: true`.
/// - **QwenCode**: `yolo: true`.
/// - **Droid**: `autonomy: "high"`.
/// - **Aider**: appends `--yes` to the Amp-override `additional_params`.
fn build_executor(
    agent_type: CodingAgentType,
    config: &SessionConfig,
    approval_mode: CodingAgentApprovalMode,
    configured_binary: Option<PathBuf>,
) -> Result<CodingAgent, CodingAgentError> {
    // Every arm below funnels through `deser_executor`, so applying the
    // user's binary_path override there covers all of them at once rather
    // than needing the same line repeated per agent.
    let configured_binary = configured_binary.as_deref();
    // `is_auto` must reflect the **session's** declared `permission_mode`,
    // not the server-wide `approval_mode`. The doc comment block above
    // (Claude Code, Gemini CLI, …) describes per-session intent —
    // "session wants its agent free-running" — but the historical check
    // looked at the global `approval_mode` (which defaults to Ask/Elicit
    // and is rarely set to `Allow`). Result: callers like Direktor that
    // pass `config.permission_mode = Auto` saw the auto wiring silently
    // skipped (no `--permission-mode=auto` for Claude Code, no `--yolo`
    // for Gemini, …) and then `say()`'s runtime mode-flip got rejected
    // by Claude Code with `"Cannot set permission mode to
    // bypassPermissions because the session was not launched with
    // --dangerously-skip-permissions"`. The session ended up running in
    // whatever Claude Code's local default happened to be.
    //
    // The previous parameter is intentionally still accepted (some
    // callers may want a global-policy gate later); for now, prefer the
    // session value, which matches every other branch in this function
    // (`is_plan`, `is_supervised`, …).
    let _ = approval_mode;
    let is_auto = matches!(config.permission_mode, CodingPermissionMode::Auto);

    match agent_type {
        CodingAgentType::ClaudeCode => {
            let is_plan = matches!(config.permission_mode, CodingPermissionMode::Plan);
            let is_supervised = matches!(config.permission_mode, CodingPermissionMode::Supervised);

            // Approvals must stay wired in `auto` so bubble-up
            // requests reach the host's `ExecutorApprovalService`.
            // The `--permission-mode=auto` override is appended via
            // additional_params so argparse picks it up after the
            // executors-crate-supplied bypass/permissions flags.
            let approvals_on = is_auto || is_supervised;
            let mut json = serde_json::json!({
                "plan": is_plan,
                "approvals": approvals_on,
                "dangerously_skip_permissions": false,
            });
            if is_auto {
                json["cmd"] = serde_json::json!({
                    "additional_params": ["--permission-mode=auto"],
                });
            }

            if let Some(ref model) = config.model {
                json["model"] = serde_json::Value::String(model.clone());
            }

            let executor = deser_executor(json, "Claude Code", configured_binary)?;
            Ok(CodingAgent::ClaudeCode(executor))
        }
        CodingAgentType::GeminiCli => {
            let mut json = build_model_json(config);
            if is_auto {
                json["yolo"] = serde_json::Value::Bool(true);
            }
            let executor = deser_executor(json, "Gemini", configured_binary)?;
            Ok(CodingAgent::Gemini(executor))
        }
        CodingAgentType::Codex => {
            let mut json = build_model_json(config);
            if is_auto {
                // `--full-auto` semantics: workspace-write sandbox,
                // ask-for-approval on-request (so out-of-sandbox
                // operations still bubble up).
                json["sandbox"] = serde_json::Value::String("workspace_write".into());
                json["ask_for_approval"] = serde_json::Value::String("on_request".into());
            }
            let executor = deser_executor(json, "Codex", configured_binary)?;
            Ok(CodingAgent::Codex(executor))
        }
        CodingAgentType::Amp => {
            let json = if is_auto {
                serde_json::json!({ "dangerously_allow_all": true })
            } else {
                serde_json::json!({})
            };
            let executor = deser_executor(json, "Amp", configured_binary)?;
            Ok(CodingAgent::Amp(executor))
        }
        CodingAgentType::Cursor => {
            let json = if is_auto {
                serde_json::json!({ "force": true })
            } else {
                serde_json::json!({})
            };
            let executor = deser_executor(json, "Cursor", configured_binary)?;
            Ok(CodingAgent::CursorAgent(executor))
        }
        CodingAgentType::Opencode => {
            let mut json = build_model_json(config);
            // Opencode's `auto_approve` defaults to true upstream,
            // but spell it out for both branches so behaviour is
            // explicit and visible in logs.
            json["auto_approve"] = serde_json::Value::Bool(is_auto);
            let executor = deser_executor(json, "Opencode", configured_binary)?;
            Ok(CodingAgent::Opencode(executor))
        }
        CodingAgentType::QwenCode => {
            let json = if is_auto {
                serde_json::json!({ "yolo": true })
            } else {
                serde_json::json!({})
            };
            let executor = deser_executor(json, "QwenCode", configured_binary)?;
            Ok(CodingAgent::QwenCode(executor))
        }
        CodingAgentType::Copilot => {
            let json = if is_auto {
                serde_json::json!({ "allow_all_tools": true })
            } else {
                serde_json::json!({})
            };
            let executor = deser_executor(json, "Copilot", configured_binary)?;
            Ok(CodingAgent::Copilot(executor))
        }
        CodingAgentType::Droid => {
            let json = if is_auto {
                serde_json::json!({ "autonomy": "high" })
            } else {
                serde_json::json!({})
            };
            let executor = deser_executor(json, "Droid", configured_binary)?;
            Ok(CodingAgent::Droid(executor))
        }
        // Spawned directly by `spawn_direct`, which runs before this
        // function is reached. Kept as explicit arms so adding a future
        // direct-spawn agent still has to make a deliberate choice here.
        CodingAgentType::Antigravity | CodingAgentType::Aider => {
            Err(CodingAgentError::SpawnFailed {
                agent: agent_type.display_name().to_string(),
                reason: "internal: agent is spawned directly, not via the executors crate"
                    .to_string(),
            })
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

/// Helper to deserialize an executor from JSON, returning CodingAgentError on failure.
fn deser_executor<T: serde::de::DeserializeOwned>(
    mut json: serde_json::Value,
    agent_name: &str,
    configured_binary: Option<&Path>,
) -> Result<T, CodingAgentError> {
    // Point the executor at the user's binary instead of its built-in launch
    // command (usually an `npx ...` invocation). Every executor honours
    // `base_command_override` via CmdOverrides.
    if let Some(binary) = configured_binary {
        if let Some(obj) = json.as_object_mut() {
            obj.insert(
                "base_command_override".to_string(),
                serde_json::Value::String(quote_base_command(binary)),
            );
        }
    }

    serde_json::from_value(json).map_err(|e| CodingAgentError::SpawnFailed {
        agent: agent_name.to_string(),
        reason: format!("Failed to build executor: {}", e),
    })
}

/// Render a path so the executors crate's shell-style splitter reads it back
/// as a single argument.
///
/// `CommandBuilder` runs its base string through `shlex::split` (Unix) or
/// `winsplit::split` (Windows), so an unquoted path containing spaces — say
/// `/Users/me/My Tools/claude` — would be torn into two arguments and the
/// spawn would fail with a confusing "not found".
fn quote_base_command(path: &Path) -> String {
    let raw = path.to_string_lossy();

    #[cfg(windows)]
    {
        // winsplit follows the MSVCRT rules: double quotes group, and an
        // embedded quote is escaped by doubling it.
        format!("\"{}\"", raw.replace('"', "\"\""))
    }

    #[cfg(not(windows))]
    {
        shlex::try_quote(raw.as_ref())
            .map(|q| q.into_owned())
            .unwrap_or_else(|_| raw.into_owned())
    }
}

/// Spawn a background task that reads stdout and appends to the session buffer.
/// Also parses Claude Code's session ID from stream-json output.
///
/// The reader respects the `reader_cancel` token on the `AgentProcess` — when
/// the session is resumed with a new process, the old reader stops cleanly.
fn spawn_output_reader(session: Arc<Mutex<CodingSession>>, change_tx: broadcast::Sender<()>) {
    tokio::spawn(async move {
        // Take stdout and reader_cancel from the process
        let (stdout, reader_cancel) = {
            let mut s = session.lock().await;
            let stdout = s
                .process
                .as_mut()
                .and_then(|p| p.child.inner().stdout.take());
            let cancel = s
                .process
                .as_ref()
                .map(|p| p.reader_cancel.clone())
                .unwrap_or_else(tokio_util::sync::CancellationToken::new);
            (stdout, cancel)
        };

        let Some(stdout) = stdout else {
            return;
        };

        let reader = BufReader::new(stdout);
        let mut lines = reader.lines();

        loop {
            tokio::select! {
                biased;
                _ = reader_cancel.cancelled() => {
                    debug!("Output reader cancelled (session being resumed)");
                    break;
                }
                line_result = lines.next_line() => {
                    match line_result {
                        Ok(Some(line)) => {
                            let mut s = session.lock().await;
                            if s.agent_session_id.is_none() {
                                if let Some(sid) = extract_session_id(&line) {
                                    debug!(session_id = %sid, "Captured agent session ID");
                                    s.agent_session_id = Some(sid);
                                }
                            }
                            s.append_output(line);
                        }
                        Ok(None) => {
                            // EOF — process exited. Only update status if we haven't been cancelled
                            // (if cancelled, resume_session handles the status transition).
                            if reader_cancel.is_cancelled() {
                                break;
                            }
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
                                            s.error =
                                                Some(format!("Failed to get exit status: {}", e));
                                        }
                                    }
                                }
                            }
                            break;
                        }
                        Err(e) => {
                            if !reader_cancel.is_cancelled() {
                                let mut s = session.lock().await;
                                s.append_output(format!("[error reading output: {}]", e));
                            }
                            break;
                        }
                    }
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
mod direct_cli_tests {
    use super::*;

    fn cfg(mode: CodingPermissionMode, model: Option<&str>) -> SessionConfig {
        SessionConfig {
            model: model.map(str::to_string),
            permission_mode: mode,
            env: Default::default(),
        }
    }

    /// Flags that belong to other agents' executors and that both CLIs
    /// reject. This is the defect that made the previous implementation
    /// unusable, so it is asserted directly.
    const FOREIGN_FLAGS: &[&str] = &["--execute", "--stream-json", "--experimental-acp", "--acp"];

    #[test]
    fn executor_agents_are_not_spawned_directly() {
        for agent in [
            CodingAgentType::ClaudeCode,
            CodingAgentType::Codex,
            CodingAgentType::Opencode,
            CodingAgentType::GeminiCli,
        ] {
            assert!(
                direct_cli_args(
                    agent,
                    "hi",
                    &cfg(CodingPermissionMode::Supervised, None),
                    None
                )
                .is_none(),
                "{agent} should use the executors crate"
            );
        }
    }

    #[test]
    fn direct_agents_never_get_another_agents_flags() {
        for agent in [CodingAgentType::Antigravity, CodingAgentType::Aider] {
            for mode in [
                CodingPermissionMode::Auto,
                CodingPermissionMode::Supervised,
                CodingPermissionMode::Plan,
            ] {
                let args = direct_cli_args(agent, "hi", &cfg(mode, Some("m")), None).unwrap();
                for bad in FOREIGN_FLAGS {
                    assert!(
                        !args.iter().any(|a| a == bad),
                        "{agent} args contain foreign flag {bad}: {args:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn antigravity_prompt_is_the_last_argument() {
        // agy's --print consumes the next argument, so anything appended
        // after the prompt would be swallowed as part of it.
        let args = direct_cli_args(
            CodingAgentType::Antigravity,
            "do the thing",
            &cfg(CodingPermissionMode::Auto, Some("gemini-3-pro")),
            Some("conv-1"),
        )
        .unwrap();

        assert_eq!(args.last().unwrap(), "do the thing");
        assert_eq!(args[args.len() - 2], "--print");
        assert!(args.windows(2).any(|w| w == ["--model", "gemini-3-pro"]));
        assert!(args.windows(2).any(|w| w == ["--conversation", "conv-1"]));
        assert!(args.iter().any(|a| a == "--dangerously-skip-permissions"));
    }

    #[test]
    fn antigravity_plan_mode_uses_mode_flag_not_skip_permissions() {
        let args = direct_cli_args(
            CodingAgentType::Antigravity,
            "hi",
            &cfg(CodingPermissionMode::Plan, None),
            None,
        )
        .unwrap();
        assert!(args.windows(2).any(|w| w == ["--mode", "plan"]));
        assert!(!args.iter().any(|a| a == "--dangerously-skip-permissions"));
    }

    #[test]
    fn supervised_mode_grants_nothing_extra() {
        for agent in [CodingAgentType::Antigravity, CodingAgentType::Aider] {
            let args = direct_cli_args(
                agent,
                "hi",
                &cfg(CodingPermissionMode::Supervised, None),
                None,
            )
            .unwrap();
            assert!(!args.iter().any(|a| a == "--dangerously-skip-permissions"));
            assert!(!args.iter().any(|a| a == "--yes-always"));
        }
    }

    #[test]
    fn aider_runs_one_shot_without_touching_the_repo() {
        let args = direct_cli_args(
            CodingAgentType::Aider,
            "fix the bug",
            &cfg(CodingPermissionMode::Auto, Some("gpt-4o")),
            None,
        )
        .unwrap();

        assert_eq!(args.last().unwrap(), "fix the bug");
        assert_eq!(args[args.len() - 2], "--message");
        // Committing to the user's repo unasked would be destructive.
        assert!(args.iter().any(|a| a == "--no-auto-commits"));
        // ANSI redraws would be recorded verbatim by the line reader.
        assert!(args.iter().any(|a| a == "--no-pretty"));
        assert!(args.iter().any(|a| a == "--no-stream"));
        assert!(args.iter().any(|a| a == "--yes-always"));
        assert!(args.windows(2).any(|w| w == ["--model", "gpt-4o"]));
    }

    fn manager_with_binary_path(agent: CodingAgentType, path: &str) -> CodingAgentManager {
        CodingAgentManager::new(CodingAgentsConfig {
            agents: vec![lr_config::CodingAgentConfig {
                agent_type: agent,
                working_directory: None,
                model_id: None,
                permission_mode: CodingPermissionMode::Supervised,
                env: Default::default(),
                binary_path: Some(path.to_string()),
            }],
            ..Default::default()
        })
    }

    #[test]
    fn configured_binary_path_is_honoured_when_it_exists() {
        // Any real file works; the check is existence, not executability.
        let real = if cfg!(windows) {
            "C:\\Windows\\System32\\cmd.exe"
        } else {
            "/bin/sh"
        };
        let mgr = manager_with_binary_path(CodingAgentType::Aider, real);

        assert_eq!(
            mgr.configured_binary_path(CodingAgentType::Aider),
            Some(PathBuf::from(real))
        );
        assert_eq!(
            mgr.resolve_binary(CodingAgentType::Aider),
            Some(real.into())
        );
    }

    #[test]
    fn stale_override_falls_back_instead_of_breaking_detection() {
        let mgr = manager_with_binary_path(
            CodingAgentType::Aider,
            "/nonexistent/path/to/aider-that-was-uninstalled",
        );
        // The override is ignored...
        assert!(mgr.configured_binary_path(CodingAgentType::Aider).is_none());
        // ...and detection still behaves as if it were never set, rather
        // than reporting the agent as missing.
        assert_eq!(
            mgr.resolve_binary(CodingAgentType::Aider),
            lr_utils::binary::find_binary("aider")
        );
    }

    #[test]
    fn blank_override_is_treated_as_unset() {
        for blank in ["", "   "] {
            let mgr = manager_with_binary_path(CodingAgentType::Aider, blank);
            assert!(mgr.configured_binary_path(CodingAgentType::Aider).is_none());
        }
    }

    #[test]
    fn override_is_not_invented_for_agents_without_one() {
        let mgr = manager_with_binary_path(CodingAgentType::Aider, "/bin/sh");
        // Configuring one agent must not divert another.
        assert!(mgr
            .configured_binary_path(CodingAgentType::ClaudeCode)
            .is_none());
    }

    #[test]
    fn base_command_quoting_survives_spaces() {
        // The executors crate splits its base string shell-style, so an
        // unquoted path with spaces would be torn into two arguments.
        let quoted = quote_base_command(Path::new("/Users/me/My Tools/claude"));
        let parts = shlex::split(&quoted).expect("quoted path must re-parse");
        assert_eq!(parts, vec!["/Users/me/My Tools/claude"]);
    }

    #[test]
    fn base_command_quoting_leaves_plain_paths_readable() {
        assert_eq!(
            shlex::split(&quote_base_command(Path::new("/usr/local/bin/claude"))).unwrap(),
            vec!["/usr/local/bin/claude"]
        );
    }

    /// Round-trip the built executor through serde to see what it actually
    /// holds — `base_command_override` lives on a flattened `CmdOverrides`,
    /// so asserting on the JSON is the way to prove it survived.
    fn executor_json(agent: CodingAgentType, binary: Option<&str>) -> serde_json::Value {
        let executor = build_executor(
            agent,
            &cfg(CodingPermissionMode::Supervised, None),
            CodingAgentApprovalMode::default(),
            binary.map(PathBuf::from),
        )
        .unwrap_or_else(|e| panic!("{agent} executor should build: {e}"));

        // CodingAgent is externally tagged, so the executor's own fields sit
        // one level under the variant key (e.g. {"CLAUDE_CODE": { .. }}).
        let value = serde_json::to_value(&executor).expect("executor should serialize");
        value
            .as_object()
            .and_then(|o| o.values().next())
            .cloned()
            .unwrap_or(value)
    }

    #[test]
    fn executor_agents_get_base_command_override_only_when_configured() {
        // Every executor-backed agent must accept the override, since the
        // user can set binary_path for any of them.
        for agent in [
            CodingAgentType::ClaudeCode,
            CodingAgentType::GeminiCli,
            CodingAgentType::Codex,
            CodingAgentType::Amp,
            CodingAgentType::Cursor,
            CodingAgentType::Opencode,
            CodingAgentType::QwenCode,
            CodingAgentType::Copilot,
            CodingAgentType::Droid,
        ] {
            let plain = executor_json(agent, None).to_string();
            assert!(
                !plain.contains("base_command_override"),
                "{agent} diverted its launch command without being asked"
            );

            let overridden = executor_json(agent, Some("/opt/custom/bin/tool")).to_string();
            assert!(
                overridden.contains("/opt/custom/bin/tool"),
                "{agent} dropped the configured binary_path: {overridden}"
            );
        }
    }

    #[test]
    fn overridden_base_command_with_spaces_reaches_the_executor_quoted() {
        let json = executor_json(CodingAgentType::ClaudeCode, Some("/opt/My Tools/claude"));
        let base = json
            .get("base_command_override")
            .and_then(|v| v.as_str())
            .expect("override should be present");
        assert_eq!(
            shlex::split(base).unwrap(),
            vec!["/opt/My Tools/claude"],
            "stored base must re-parse as one argument"
        );
    }

    #[test]
    fn model_is_omitted_when_unset() {
        for agent in [CodingAgentType::Antigravity, CodingAgentType::Aider] {
            let args =
                direct_cli_args(agent, "hi", &cfg(CodingPermissionMode::Auto, None), None).unwrap();
            assert!(!args.iter().any(|a| a == "--model"), "{agent}: {args:?}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use executors::approvals::{ExecutorApprovalError, ExecutorApprovalService};
    use lr_config::CodingAgentsConfig;
    use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
    use tokio_util::sync::CancellationToken;
    use workspace_utils::approvals::{ApprovalStatus, QuestionStatus};

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

    /// Counts factory invocations + records the session ids it received,
    /// so the test can assert the manager actually delegates per spawn.
    struct CountingApprovals {
        calls: AtomicUsize,
    }

    #[async_trait]
    impl ExecutorApprovalService for CountingApprovals {
        async fn create_tool_approval(
            &self,
            _tool_name: &str,
            _tool_input: Option<&serde_json::Value>,
        ) -> Result<String, ExecutorApprovalError> {
            self.calls.fetch_add(1, AtomicOrdering::SeqCst);
            Ok(String::new())
        }
        async fn create_question_approval(
            &self,
            _tool_name: &str,
            _question_count: usize,
        ) -> Result<String, ExecutorApprovalError> {
            self.calls.fetch_add(1, AtomicOrdering::SeqCst);
            Ok(String::new())
        }
        async fn wait_tool_approval(
            &self,
            _approval_id: &str,
            _cancel: CancellationToken,
        ) -> Result<ApprovalStatus, ExecutorApprovalError> {
            Ok(ApprovalStatus::Approved)
        }
        async fn wait_question_answer(
            &self,
            _approval_id: &str,
            _cancel: CancellationToken,
        ) -> Result<QuestionStatus, ExecutorApprovalError> {
            Ok(QuestionStatus::Answered {
                answers: Vec::new(),
            })
        }
    }

    #[test]
    fn approval_service_factory_round_trip() {
        let manager = CodingAgentManager::new(test_config());
        assert!(!manager.has_custom_approval_service());

        let factory: ApprovalServiceFactory = Arc::new(|_session_id: &SessionId| {
            Arc::new(CountingApprovals {
                calls: AtomicUsize::new(0),
            }) as Arc<dyn ExecutorApprovalService>
        });
        let manager = manager.with_approval_service_factory(factory);
        assert!(manager.has_custom_approval_service());
    }

    #[test]
    fn set_approval_service_factory_replaces_and_clears() {
        let manager = CodingAgentManager::new(test_config());
        let factory: ApprovalServiceFactory = Arc::new(|_| {
            Arc::new(CountingApprovals {
                calls: AtomicUsize::new(0),
            }) as Arc<dyn ExecutorApprovalService>
        });
        manager.set_approval_service_factory(Some(factory));
        assert!(manager.has_custom_approval_service());
        manager.set_approval_service_factory(None);
        assert!(!manager.has_custom_approval_service());
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
        assert_eq!(
            truncate_prompt("a long prompt that exceeds", 10),
            "a long ..."
        );
        assert_eq!(truncate_prompt("line1\nline2", 20), "line1");
    }
}
