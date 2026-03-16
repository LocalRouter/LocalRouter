//! Persistent memory for LLM conversations via Zillis memsearch.
//!
//! Provides auto-capture of conversations in MCP via LLM and Both modes,
//! with per-client isolation and configurable LLM compaction.
//!
//! No background daemon — indexing is called directly after each transcript write.

pub mod cli;
pub mod compaction;
pub mod session_manager;
pub mod transcript;

// Daemon module removed — indexing is called directly after each write.

#[cfg(test)]
mod tests;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use dashmap::DashMap;
use parking_lot::RwLock;
use tokio::task::JoinHandle;

use lr_config::MemoryConfig;

pub use cli::{MemsearchCli, SearchResult};
pub use session_manager::SessionManager;
pub use transcript::TranscriptWriter;

/// Minimum interval between index calls per client (debounce).
const INDEX_DEBOUNCE: Duration = Duration::from_secs(3);

/// Core memory service — manages per-client transcript writing,
/// session tracking, indexing, and compaction.
pub struct MemoryService {
    pub cli: MemsearchCli,
    pub session_manager: SessionManager,
    pub transcript: TranscriptWriter,
    config: RwLock<MemoryConfig>,
    /// Root memory directory (e.g., ~/.localrouter/memory/)
    memory_dir: PathBuf,
    /// Last index time per client_id (for debouncing)
    last_indexed: DashMap<String, Instant>,
}

impl MemoryService {
    /// Create a new memory service. Does NOT validate memsearch installation —
    /// that's done lazily on first use or via the setup commands.
    pub fn new(config: MemoryConfig, memory_dir: PathBuf) -> Self {
        let provider = match &config.embedding {
            lr_config::MemoryEmbeddingConfig::Local
            | lr_config::MemoryEmbeddingConfig::Onnx => "local".to_string(),
            lr_config::MemoryEmbeddingConfig::Ollama { .. } => "ollama".to_string(),
        };
        let session_config = session_manager::SessionConfig {
            inactivity_timeout: std::time::Duration::from_secs(
                config.session_inactivity_minutes * 60,
            ),
            max_duration: std::time::Duration::from_secs(config.max_session_minutes * 60),
        };
        Self {
            cli: MemsearchCli::with_provider(provider),
            session_manager: SessionManager::new(session_config),
            transcript: TranscriptWriter::new(),
            config: RwLock::new(config),
            memory_dir,
            last_indexed: DashMap::new(),
        }
    }

    /// Ensure the per-client memory directory exists with proper structure.
    /// Returns the client's memory directory path.
    pub fn ensure_client_dir(&self, client_id: &str) -> std::io::Result<PathBuf> {
        let client_dir = self.memory_dir.join(client_id);
        std::fs::create_dir_all(client_dir.join("sessions"))?;
        std::fs::create_dir_all(client_dir.join("archive"))?;
        Ok(client_dir)
    }

    /// Index a client's sessions directory with debouncing.
    /// Called after each transcript write. Skips if indexed within the last
    /// few seconds to avoid redundant work during rapid exchanges.
    pub async fn index_client(&self, client_id: &str) {
        // Debounce: skip if indexed recently
        if let Some(last) = self.last_indexed.get(client_id) {
            if last.elapsed() < INDEX_DEBOUNCE {
                return;
            }
        }

        let client_dir = self.memory_dir.join(client_id);
        let sessions_dir = client_dir.join("sessions");
        if sessions_dir.exists() {
            if let Err(e) = self.cli.index(&sessions_dir).await {
                tracing::warn!(
                    "Memory index failed for client {}: {}",
                    &client_id[..8.min(client_id.len())],
                    e
                );
                return;
            }
            self.last_indexed
                .insert(client_id.to_string(), Instant::now());
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

    /// Get the root memory directory.
    pub fn memory_dir(&self) -> &std::path::Path {
        &self.memory_dir
    }
}
