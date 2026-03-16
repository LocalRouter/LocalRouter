//! Persistent memory for LLM conversations via Zillis memsearch.
//!
//! All embedding and LLM calls are routed through LocalRouter's own
//! endpoints using a transient bearer token. No separate model downloads
//! needed — uses whatever providers the user has configured.

pub mod cli;
pub mod compaction;
pub mod session_manager;
pub mod transcript;

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
///
/// Embedding and LLM calls are routed through LocalRouter using a
/// transient bearer token generated at startup.
pub struct MemoryService {
    pub cli: MemsearchCli,
    pub session_manager: SessionManager,
    pub transcript: TranscriptWriter,
    config: RwLock<MemoryConfig>,
    /// Root memory directory (e.g., ~/.localrouter/memory/)
    memory_dir: PathBuf,
    /// Last index time per client_id (for debouncing)
    last_indexed: DashMap<String, Instant>,
    /// Transient bearer token for memsearch to call LocalRouter
    pub memory_secret: String,
}

impl MemoryService {
    /// Create a new memory service.
    ///
    /// `server_port`: LocalRouter's HTTP port (e.g., 3625)
    /// `memory_secret`: transient bearer token for auth
    pub fn new(config: MemoryConfig, memory_dir: PathBuf, server_port: u16, memory_secret: String) -> Self {
        let base_url = format!("http://localhost:{}/v1", server_port);
        let embedding_model = config.embedding_model.clone().unwrap_or_default();

        let session_config = session_manager::SessionConfig {
            inactivity_timeout: Duration::from_secs(config.session_inactivity_minutes * 60),
            max_duration: Duration::from_secs(config.max_session_minutes * 60),
        };

        Self {
            cli: MemsearchCli {
                base_url,
                api_key: memory_secret.clone(),
                embedding_model: parking_lot::RwLock::new(embedding_model),
            },
            session_manager: SessionManager::new(session_config),
            transcript: TranscriptWriter::new(),
            config: RwLock::new(config),
            memory_dir,
            last_indexed: DashMap::new(),
            memory_secret,
        }
    }

    /// Ensure the per-client memory directory exists with proper structure.
    pub fn ensure_client_dir(&self, client_id: &str) -> std::io::Result<PathBuf> {
        let client_dir = self.memory_dir.join(client_id);
        std::fs::create_dir_all(client_dir.join("sessions"))?;
        std::fs::create_dir_all(client_dir.join("archive"))?;
        Ok(client_dir)
    }

    /// Index a client's sessions directory with debouncing.
    pub async fn index_client(&self, client_id: &str) {
        if self.cli.get_embedding_model().is_empty() {
            return; // No embedding model configured
        }

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
        if self.cli.get_embedding_model().is_empty() {
            return Err("No embedding model configured. Select one in Memory settings.".to_string());
        }

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
    pub fn start_session_monitor(self: &Arc<Self>) -> JoinHandle<()> {
        let service = Arc::clone(self);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(60));
            loop {
                interval.tick().await;
                let expired = service.session_manager.close_expired_sessions();
                for (client_id, session) in expired {
                    let config = service.config.read().clone();
                    if let Some(ref compaction_model) = config.compaction_model {
                        let client_dir = service.memory_dir.join(&client_id);
                        let archive_dir = client_dir.join("archive");
                        if let Err(e) = compaction::compact_session(
                            &service.cli,
                            &session.file_path,
                            &archive_dir,
                            compaction_model,
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
        })
    }

    /// Update configuration.
    pub fn update_config(&self, config: MemoryConfig) {
        let session_config = session_manager::SessionConfig {
            inactivity_timeout: Duration::from_secs(config.session_inactivity_minutes * 60),
            max_duration: Duration::from_secs(config.max_session_minutes * 60),
        };
        self.session_manager.update_config(session_config);

        // Update CLI embedding model
        self.cli.set_embedding_model(config.embedding_model.clone().unwrap_or_default());

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
