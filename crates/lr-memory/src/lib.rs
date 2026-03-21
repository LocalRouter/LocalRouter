//! Persistent memory for LLM conversations using native FTS5 search.
//!
//! Each client gets an isolated SQLite FTS5 database on disk.
//! No Python, no embedding models, no external dependencies.

pub mod compaction;
pub mod session_manager;
pub mod transcript;

#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use dashmap::DashMap;
use parking_lot::RwLock;
use tokio::task::JoinHandle;

use lr_config::MemoryConfig;
use lr_context::ContentStore;

pub use session_manager::SessionManager;
pub use transcript::TranscriptWriter;

/// A single search result from memory FTS5 search.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SearchResult {
    pub source: String,
    pub title: String,
    pub content: String,
    pub score: Option<f64>,
}

/// Core memory service — manages per-client transcript writing,
/// session tracking, and FTS5-based search (with optional vector search).
pub struct MemoryService {
    pub session_manager: SessionManager,
    pub transcript: TranscriptWriter,
    config: RwLock<MemoryConfig>,
    /// Root memory directory (e.g., ~/.localrouter/memory/)
    memory_dir: PathBuf,
    /// Per-client FTS5 stores (persistent on disk)
    stores: DashMap<String, ContentStore>,
    /// Optional embedding service for hybrid (FTS5 + vector) search
    embedding_service: Option<Arc<lr_embeddings::EmbeddingService>>,
}

impl MemoryService {
    /// Create a new memory service.
    pub fn new(config: MemoryConfig, memory_dir: PathBuf) -> Self {
        let session_config = session_manager::SessionConfig {
            inactivity_timeout: Duration::from_secs(config.session_inactivity_minutes * 60),
            max_duration: Duration::from_secs(config.max_session_minutes * 60),
        };

        Self {
            session_manager: SessionManager::new(session_config),
            transcript: TranscriptWriter::new(),
            config: RwLock::new(config),
            memory_dir,
            stores: DashMap::new(),
            embedding_service: None,
        }
    }

    /// Create a new memory service with an embedding service for hybrid search.
    pub fn with_embedding_service(
        config: MemoryConfig,
        memory_dir: PathBuf,
        embedding_service: Arc<lr_embeddings::EmbeddingService>,
    ) -> Self {
        let mut service = Self::new(config, memory_dir);
        service.embedding_service = Some(embedding_service);
        service
    }

    /// Set or replace the embedding service (enables hybrid search for new stores).
    pub fn set_embedding_service(&self, service: Arc<lr_embeddings::EmbeddingService>) {
        // Attach to all existing stores and rebuild their vector indices
        for entry in self.stores.iter() {
            entry.value().set_embedding_service(Arc::clone(&service));
            if let Err(e) = entry.value().rebuild_vectors() {
                tracing::warn!(
                    "Failed to rebuild vectors for client {}: {}",
                    entry.key(),
                    e
                );
            }
        }
        // Note: new stores created after this call won't automatically get the service
        // because we can't mutate self. The get_or_create_store method handles this.
    }

    /// Get the embedding service (if any).
    pub fn embedding_service(&self) -> Option<&Arc<lr_embeddings::EmbeddingService>> {
        self.embedding_service.as_ref()
    }

    /// Ensure the per-client memory directory exists with proper structure.
    pub fn ensure_client_dir(&self, client_id: &str) -> std::io::Result<PathBuf> {
        let client_dir = self.memory_dir.join(client_id);
        std::fs::create_dir_all(client_dir.join("sessions"))?;
        std::fs::create_dir_all(client_dir.join("archive"))?;
        Ok(client_dir)
    }

    /// Get or create the FTS5 ContentStore for a client.
    ///
    /// Opens `memory/{client_id}/memory.db` on disk. Creates if needed.
    /// If an embedding service is available, attaches it to the store for hybrid search.
    fn get_or_create_store(
        &self,
        client_id: &str,
    ) -> Result<dashmap::mapref::one::Ref<'_, String, ContentStore>, String> {
        if self.stores.contains_key(client_id) {
            return self
                .stores
                .get(client_id)
                .ok_or_else(|| "Store disappeared".to_string());
        }

        let client_dir = self.memory_dir.join(client_id);
        std::fs::create_dir_all(&client_dir)
            .map_err(|e| format!("Failed to create client dir: {}", e))?;

        let db_path = client_dir.join("memory.db");
        let store = ContentStore::open(&db_path)
            .map_err(|e| format!("Failed to open memory store: {}", e))?;

        // Attach embedding service for hybrid search if available
        if let Some(ref embedding_service) = self.embedding_service {
            store.set_embedding_service(Arc::clone(embedding_service));
            // Rebuild vector index from existing FTS5 content
            if let Err(e) = store.rebuild_vectors() {
                tracing::warn!(
                    "Failed to rebuild vectors for client {}: {}",
                    &client_id[..8.min(client_id.len())],
                    e
                );
            }
        }

        self.stores.insert(client_id.to_string(), store);
        self.stores
            .get(client_id)
            .ok_or_else(|| "Store disappeared after insert".to_string())
    }

    /// Index transcript content into the client's FTS5 store.
    ///
    /// `label` should be `"session/{session_id}"` for transcript exchanges.
    /// Content is re-indexed atomically if the label already exists.
    pub fn index_transcript(
        &self,
        client_id: &str,
        session_id: &str,
        content: &str,
    ) -> Result<(), String> {
        if content.trim().is_empty() {
            return Ok(());
        }

        let store = self.get_or_create_store(client_id)?;
        let label = format!("session/{}", session_id);

        // Read existing content for this label, append new content
        let existing = match store.read(&label, None, Some(100_000)) {
            Ok(r) => {
                // Strip line numbers from the read output to get raw content
                r.content
                    .lines()
                    .map(|line| {
                        // Content from read() is in cat -n format: "    N\tcontent"
                        if let Some(idx) = line.find('\t') {
                            &line[idx + 1..]
                        } else {
                            line
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            }
            Err(_) => String::new(),
        };

        let full_content = if existing.trim().is_empty() {
            content.to_string()
        } else {
            format!("{}\n\n{}", existing.trim(), content)
        };

        store
            .index(&label, &full_content)
            .map_err(|e| format!("FTS5 index failed: {}", e))?;

        Ok(())
    }

    /// Search memories for a client using FTS5 full-text search.
    pub fn search(
        &self,
        client_id: &str,
        query: &str,
        top_k: usize,
    ) -> Result<Vec<SearchResult>, String> {
        let client_dir = self.memory_dir.join(client_id);
        if !client_dir.exists() {
            return Ok(Vec::new());
        }

        let store = self.get_or_create_store(client_id)?;
        let results = store
            .search(&[query.to_string()], top_k, None)
            .map_err(|e| format!("FTS5 search failed: {}", e))?;

        let mut out = Vec::new();
        for sr in &results {
            for hit in &sr.hits {
                out.push(SearchResult {
                    source: hit.source.clone(),
                    title: hit.title.clone(),
                    content: hit.content.clone(),
                    score: Some(hit.rank),
                });
            }
        }

        Ok(out)
    }

    /// Search memories using ContentStore's native search (returns full SearchResult
    /// with line numbers for use with read). Supports combined query/queries.
    pub fn search_combined(
        &self,
        client_id: &str,
        query: Option<&str>,
        queries: Option<&[String]>,
        limit: usize,
        source: Option<&str>,
    ) -> Result<Vec<lr_context::SearchResult>, String> {
        let client_dir = self.memory_dir.join(client_id);
        if !client_dir.exists() {
            return Ok(Vec::new());
        }

        let store = self.get_or_create_store(client_id)?;
        store
            .search_combined(query, queries, limit, source)
            .map_err(|e| format!("Search failed: {}", e))
    }

    /// Read a memory source by label with optional offset/limit pagination.
    pub fn read(
        &self,
        client_id: &str,
        label: &str,
        offset: Option<&str>,
        limit: Option<usize>,
    ) -> Result<lr_context::ReadResult, String> {
        let store = self.get_or_create_store(client_id)?;
        store
            .read(label, offset, limit)
            .map_err(|e| format!("Read failed: {}", e))
    }

    /// List all indexed sources for a client (for summary fallback).
    pub fn list_sources(&self, client_id: &str) -> Result<Vec<lr_context::SourceInfo>, String> {
        let client_dir = self.memory_dir.join(client_id);
        if !client_dir.exists() {
            return Ok(Vec::new());
        }

        let store = self.get_or_create_store(client_id)?;
        store
            .list_sources()
            .map_err(|e| format!("List sources failed: {}", e))
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
                    if config.compaction_model.is_some() {
                        let client_dir = service.memory_dir.join(&client_id);
                        let archive_dir = client_dir.join("archive");
                        if let Err(e) =
                            compaction::compact_session(&session.file_path, &archive_dir).await
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

    /// Get compaction statistics for a client.
    pub fn get_compaction_stats(&self, client_id: &str) -> Result<CompactionStats, String> {
        let client_dir = self.memory_dir.join(client_id);
        let sessions_dir = client_dir.join("sessions");
        let archive_dir = client_dir.join("archive");

        let active_path = self.session_manager.active_session_path(client_id);
        let session_files = count_md_files(&sessions_dir);
        let active_sessions = if active_path.is_some() { 1 } else { 0 };
        let pending_compaction = session_files.saturating_sub(active_sessions);
        let archived_sessions = count_md_files(&archive_dir);

        let (indexed_sources, total_lines) = match self.list_sources(client_id) {
            Ok(sources) => {
                let lines: usize = sources.iter().map(|s| s.total_lines).sum();
                (sources.len(), lines)
            }
            Err(_) => (0, 0),
        };

        Ok(CompactionStats {
            active_sessions,
            pending_compaction,
            archived_sessions,
            indexed_sources,
            total_lines,
        })
    }

    /// Force-compact all expired (non-active) sessions for a client.
    ///
    /// Moves all `.md` files in `sessions/` that are not the active session
    /// to the `archive/` directory. Does not require `compaction_model` to be set.
    pub async fn force_compact(&self, client_id: &str) -> Result<CompactResult, String> {
        let client_dir = self.memory_dir.join(client_id);
        let sessions_dir = client_dir.join("sessions");
        let archive_dir = client_dir.join("archive");

        let active_path = self.session_manager.active_session_path(client_id);

        let entries = std::fs::read_dir(&sessions_dir)
            .map_err(|e| format!("Failed to read sessions dir: {}", e))?;

        let mut archived_count = 0;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("md") {
                continue;
            }
            // Skip the active session
            if active_path.as_ref().is_some_and(|active| active == &path) {
                continue;
            }
            if let Err(e) = compaction::compact_session(&path, &archive_dir).await {
                tracing::warn!("Force compact failed for {:?}: {}", path, e);
            } else {
                archived_count += 1;
            }
        }

        Ok(CompactResult { archived_count })
    }

    /// Rebuild the FTS5 index from all session and archive `.md` files on disk.
    ///
    /// Drops the existing store, deletes `memory.db`, and re-indexes everything.
    /// Calls `progress_fn(current, total)` after each file is indexed.
    pub fn reindex(
        &self,
        client_id: &str,
        mut progress_fn: impl FnMut(usize, usize),
    ) -> Result<usize, String> {
        // Remove existing store from cache
        self.stores.remove(client_id);

        let client_dir = self.memory_dir.join(client_id);
        let db_path = client_dir.join("memory.db");

        // Delete existing database files
        for suffix in ["", "-shm", "-wal"] {
            let p = if suffix.is_empty() {
                db_path.clone()
            } else {
                db_path.with_extension(format!("db{}", suffix))
            };
            if p.exists() {
                let _ = std::fs::remove_file(&p);
            }
        }

        // Collect all .md files from sessions/ and archive/
        let sessions_dir = client_dir.join("sessions");
        let archive_dir = client_dir.join("archive");
        let mut files: Vec<PathBuf> = Vec::new();
        for dir in [&sessions_dir, &archive_dir] {
            if let Ok(entries) = std::fs::read_dir(dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.extension().and_then(|e| e.to_str()) == Some("md") {
                        files.push(path);
                    }
                }
            }
        }

        let total = files.len();
        progress_fn(0, total);

        // Re-create the store and index each file
        let store = self.get_or_create_store(client_id)?;
        for (i, file_path) in files.iter().enumerate() {
            let file_name = file_path.file_stem().unwrap_or_default().to_string_lossy();
            let label = format!("session/{}", file_name);

            match std::fs::read_to_string(file_path) {
                Ok(content) if !content.trim().is_empty() => {
                    if let Err(e) = store.index(&label, &content) {
                        tracing::warn!("Reindex failed for {:?}: {}", file_path, e);
                    }
                }
                Ok(_) => {} // empty file, skip
                Err(e) => {
                    tracing::warn!("Failed to read {:?} for reindex: {}", file_path, e);
                }
            }

            progress_fn(i + 1, total);
        }

        Ok(total)
    }

    /// Clear all memory for a client: deletes FTS5 index, session files, and archive.
    pub fn clear_memory(&self, client_id: &str) -> Result<(), String> {
        // Close any active sessions for this client
        self.session_manager.force_close(client_id);

        // Remove the ContentStore from the cache (drops the SQLite connection)
        self.stores.remove(client_id);

        // Delete the entire client directory (sessions, archive, memory.db)
        let client_dir = self.memory_dir.join(client_id);
        if client_dir.exists() {
            std::fs::remove_dir_all(&client_dir)
                .map_err(|e| format!("Failed to delete memory directory: {}", e))?;
        }

        tracing::info!(
            "Cleared all memory for client {}",
            &client_id[..8.min(client_id.len())]
        );
        Ok(())
    }
}

/// Compaction statistics for a client's memory.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CompactionStats {
    pub active_sessions: usize,
    pub pending_compaction: usize,
    pub archived_sessions: usize,
    pub indexed_sources: usize,
    pub total_lines: usize,
}

/// Result of a force-compact operation.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CompactResult {
    pub archived_count: usize,
}

/// Count `.md` files in a directory.
fn count_md_files(dir: &Path) -> usize {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .filter(|e| e.path().extension().and_then(|ext| ext.to_str()) == Some("md"))
                .count()
        })
        .unwrap_or(0)
}
