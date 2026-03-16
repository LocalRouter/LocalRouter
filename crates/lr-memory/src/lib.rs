//! Persistent memory for LLM conversations via Zillis memsearch.
//!
//! Provides auto-capture of conversations in MCP via LLM and Both modes,
//! with per-client isolation and configurable LLM compaction.

pub mod cli;
pub mod compaction;
pub mod daemon;
pub mod session_manager;
pub mod transcript;

#[cfg(test)]
mod tests;

use std::path::PathBuf;
use std::sync::Arc;

use dashmap::DashMap;
use parking_lot::RwLock;
use tokio::task::JoinHandle;

use lr_config::MemoryConfig;

pub use cli::{MemsearchCli, SearchResult};
pub use session_manager::SessionManager;
pub use transcript::TranscriptWriter;

/// Core memory service — manages per-client memsearch instances,
/// session tracking, and compaction.
pub struct MemoryService {
    pub cli: MemsearchCli,
    pub session_manager: SessionManager,
    pub transcript: TranscriptWriter,
    config: RwLock<MemoryConfig>,
    /// One daemon per client_id (per-client isolation)
    daemons: DashMap<String, daemon::MemsearchDaemon>,
    /// Root memory directory (e.g., ~/.localrouter/memory/)
    memory_dir: PathBuf,
}

impl MemoryService {
    /// Create a new memory service. Does NOT validate memsearch installation —
    /// that's done lazily on first use or via the setup commands.
    pub fn new(config: MemoryConfig, memory_dir: PathBuf) -> Self {
        let session_config = session_manager::SessionConfig {
            inactivity_timeout: std::time::Duration::from_secs(
                config.session_inactivity_minutes * 60,
            ),
            max_duration: std::time::Duration::from_secs(config.max_session_minutes * 60),
        };
        Self {
            cli: MemsearchCli::new(),
            session_manager: SessionManager::new(session_config),
            transcript: TranscriptWriter::new(),
            config: RwLock::new(config),
            daemons: DashMap::new(),
            memory_dir,
        }
    }

    /// Ensure the per-client memory directory exists with proper structure.
    /// Returns the client's memory directory path.
    pub fn ensure_client_dir(&self, client_id: &str) -> std::io::Result<PathBuf> {
        let client_dir = self.memory_dir.join(client_id);
        std::fs::create_dir_all(client_dir.join("sessions"))?;
        std::fs::create_dir_all(client_dir.join("archive"))?;

        // Generate .memsearch.toml if it doesn't exist
        let config_path = client_dir.join(".memsearch.toml");
        if !config_path.exists() {
            let config_content = self.generate_memsearch_config();
            std::fs::write(&config_path, config_content)?;
        }

        Ok(client_dir)
    }

    /// Start the memsearch watch daemon for a client (if not already running).
    pub async fn start_daemon(&self, client_id: &str) -> Result<(), String> {
        if let Some(mut daemon) = self.daemons.get_mut(client_id) {
            if daemon.is_running() {
                return Ok(());
            }
        }

        let client_dir = self
            .ensure_client_dir(client_id)
            .map_err(|e| format!("Failed to create client directory: {}", e))?;

        let sessions_dir = client_dir.join("sessions");
        let mut daemon = daemon::MemsearchDaemon::new();
        daemon.start(&sessions_dir).await?;
        self.daemons.insert(client_id.to_string(), daemon);
        Ok(())
    }

    /// Stop the memsearch watch daemon for a client.
    pub async fn stop_daemon(&self, client_id: &str) {
        if let Some((_, mut daemon)) = self.daemons.remove(client_id) {
            daemon.stop().await;
        }
    }

    /// Stop all running daemons (for shutdown).
    pub async fn stop_all_daemons(&self) {
        let keys: Vec<String> = self.daemons.iter().map(|r| r.key().clone()).collect();
        for key in keys {
            self.stop_daemon(&key).await;
        }
    }

    /// Search memories for a client.
    pub async fn search(
        &self,
        client_id: &str,
        query: &str,
        top_k: usize,
    ) -> Result<Vec<SearchResult>, String> {
        let client_dir = self.memory_dir.join(client_id);
        let sessions_dir = client_dir.join("sessions");
        if !sessions_dir.exists() {
            return Ok(Vec::new());
        }
        self.cli.search(&sessions_dir, query, top_k).await
    }

    /// Update the last activity time for a session file.
    pub fn touch_session(&self, path: &std::path::Path) {
        self.session_manager.touch_by_path(path);
    }

    /// Start the background session monitor task.
    /// Checks every 60 seconds for expired sessions and triggers compaction.
    pub fn start_session_monitor(self: &Arc<Self>) -> JoinHandle<()> {
        let service = Arc::clone(self);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
            loop {
                interval.tick().await;
                let expired = service.session_manager.close_expired_sessions();
                for (client_id, session) in expired {
                    let config = service.config.read().clone();
                    if let Some(ref compaction_config) = config.compaction {
                        if compaction_config.enabled {
                            let client_dir = service.memory_dir.join(&client_id);
                            let archive_dir = client_dir.join("archive");
                            if let Err(e) = compaction::compact_session(
                                &service.cli,
                                &session.file_path,
                                &archive_dir,
                                compaction_config,
                            )
                            .await
                            {
                                tracing::warn!(
                                    "Memory compaction failed for client {}: {}",
                                    &client_id[..8.min(client_id.len())],
                                    e
                                );
                            }
                        }
                    }
                }
            }
        })
    }

    /// Update configuration.
    pub fn update_config(&self, config: MemoryConfig) {
        let session_config = session_manager::SessionConfig {
            inactivity_timeout: std::time::Duration::from_secs(
                config.session_inactivity_minutes * 60,
            ),
            max_duration: std::time::Duration::from_secs(config.max_session_minutes * 60),
        };
        self.session_manager.update_config(session_config);
        *self.config.write() = config;
    }

    /// Get current config.
    pub fn config(&self) -> MemoryConfig {
        self.config.read().clone()
    }

    /// Regenerate `.memsearch.toml` for all existing client directories.
    /// Called after config changes (e.g., switching embedding provider).
    pub fn regenerate_client_configs(&self) {
        if let Ok(entries) = std::fs::read_dir(&self.memory_dir) {
            for entry in entries.flatten() {
                if entry.path().is_dir() {
                    let config_path = entry.path().join(".memsearch.toml");
                    let config_content = self.generate_memsearch_config();
                    if let Err(e) = std::fs::write(&config_path, config_content) {
                        tracing::warn!(
                            "Failed to regenerate .memsearch.toml for {}: {}",
                            entry.path().display(),
                            e
                        );
                    }
                }
            }
        }
    }

    /// Get the root memory directory.
    pub fn memory_dir(&self) -> &std::path::Path {
        &self.memory_dir
    }

    /// Generate .memsearch.toml content based on current config.
    fn generate_memsearch_config(&self) -> String {
        let config = self.config.read();
        match &config.embedding {
            lr_config::MemoryEmbeddingConfig::Onnx => {
                "[embedding]\nprovider = \"onnx\"\n".to_string()
            }
            lr_config::MemoryEmbeddingConfig::Ollama {
                model_name,
                ..
            } => {
                // Default to localhost:11434 — the standard Ollama endpoint.
                // The provider_id references a LocalRouter provider config but
                // we don't have access to the provider registry here.
                format!(
                    "[embedding]\nprovider = \"ollama\"\nmodel = \"{}\"\nbase_url = \"http://localhost:11434\"\n",
                    model_name
                )
            }
        }
    }
}
