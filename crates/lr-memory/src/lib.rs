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
use lr_monitor::MonitorEventStore;

pub use compaction::CompactionLlm;
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
    /// Optional LLM for compaction summarization
    compaction_llm: RwLock<Option<Arc<dyn CompactionLlm>>>,
    /// Optional monitor store for emitting compaction events
    monitor_store: RwLock<Option<Arc<MonitorEventStore>>>,
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
            compaction_llm: RwLock::new(None),
            monitor_store: RwLock::new(None),
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

    /// Set the compaction LLM (enables LLM-based summarization during compaction).
    pub fn set_compaction_llm(&self, llm: Arc<dyn CompactionLlm>) {
        *self.compaction_llm.write() = Some(llm);
    }

    /// Set the monitor event store (enables compaction events in monitor).
    pub fn set_monitor_store(&self, store: Arc<MonitorEventStore>) {
        *self.monitor_store.write() = Some(store);
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
            .search(
                &[query.to_string()],
                top_k,
                None,
                &lr_context::DateRange::default(),
            )
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
    #[allow(clippy::too_many_arguments)]
    pub fn search_combined(
        &self,
        client_id: &str,
        query: Option<&str>,
        queries: Option<&[String]>,
        limit: usize,
        source: Option<&str>,
        after: Option<&str>,
        before: Option<&str>,
    ) -> Result<Vec<lr_context::SearchResult>, String> {
        let client_dir = self.memory_dir.join(client_id);
        if !client_dir.exists() {
            return Ok(Vec::new());
        }

        let store = self.get_or_create_store(client_id)?;
        store
            .search_combined(query, queries, limit, source, after, before)
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

    /// List all indexed sources for a client, optionally filtered by date range.
    pub fn list_sources(
        &self,
        client_id: &str,
        after: Option<&str>,
        before: Option<&str>,
    ) -> Result<Vec<lr_context::SourceInfo>, String> {
        let client_dir = self.memory_dir.join(client_id);
        if !client_dir.exists() {
            return Ok(Vec::new());
        }

        let store = self.get_or_create_store(client_id)?;
        store
            .list_sources(after, before)
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
                    let client_dir = service.memory_dir.join(&client_id);
                    let archive_dir = client_dir.join("archive");

                    // Only provide LLM when compaction is explicitly enabled + model configured
                    let (llm_arc, model) =
                        if config.compaction_enabled && config.compaction_model.is_some() {
                            (
                                service.compaction_llm.read().clone(),
                                config.compaction_model.clone(),
                            )
                        } else {
                            (None, None)
                        };

                    let session_id = session
                        .file_path
                        .file_stem()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string();
                    let short_id = &session_id[..8.min(session_id.len())];

                    // Read transcript size for monitor event (before archiving moves the file)
                    let transcript_bytes = tokio::fs::metadata(&session.file_path)
                        .await
                        .map(|m| m.len())
                        .unwrap_or(0);

                    // Compute file paths for monitor event
                    let transcript_rel_path = format!(
                        "{}/archive/{}.md",
                        &client_id, session_id
                    );
                    let summary_rel_path = format!(
                        "{}/archive/{}-summary.md",
                        &client_id, session_id
                    );

                    // Emit monitor event if LLM compaction will happen
                    let monitor_event_id = if model.is_some() {
                        emit_compaction_event(
                            &service.monitor_store,
                            short_id,
                            model.as_deref().unwrap_or(""),
                            transcript_bytes,
                            &client_id,
                            Some(&transcript_rel_path),
                        )
                    } else {
                        None
                    };

                    let started = std::time::Instant::now();

                    // Always archive, optionally summarize with LLM
                    match compaction::compact_session(
                        &session.file_path,
                        &archive_dir,
                        llm_arc.as_deref(),
                        model.as_deref(),
                    )
                    .await
                    {
                        Ok(outcome) => {
                            // Update FTS5 index based on outcome
                            if let Ok(store) = service.get_or_create_store(&client_id) {
                                match outcome {
                                    compaction::CompactionOutcome::ArchivedAndSummarized(
                                        result,
                                    ) => {
                                        // Index the summary
                                        let summary_label =
                                            format!("session/{}-summary", session_id);
                                        let _ =
                                            store.index(&summary_label, &result.summary);

                                        // Complete monitor event with full metadata
                                        if let Some(event_id) = &monitor_event_id {
                                            complete_compaction_event(
                                                &service.monitor_store,
                                                event_id,
                                                &result,
                                                transcript_bytes,
                                                &summary_rel_path,
                                                started.elapsed().as_millis() as u64,
                                            );
                                        }

                                        // Remove raw transcript from index
                                        let raw_label = format!("session/{}", session_id);
                                        let _ = store.delete(&raw_label);
                                    }
                                    compaction::CompactionOutcome::ArchivedEmptyResponse(
                                        result,
                                    ) => {
                                        // LLM responded but summary was empty — preserve metadata for debugging
                                        if let Some(event_id) = &monitor_event_id {
                                            error_compaction_event_with_result(
                                                &service.monitor_store,
                                                event_id,
                                                "LLM returned empty summary",
                                                &result,
                                                transcript_bytes,
                                                started.elapsed().as_millis() as u64,
                                            );
                                        }
                                    }
                                    compaction::CompactionOutcome::ArchivedOnly => {
                                        // Raw transcript stays indexed as-is.
                                        // If we expected summarization, mark as error
                                        // (LLM failed gracefully inside compact_session)
                                        if let Some(event_id) = &monitor_event_id {
                                            error_compaction_event(
                                                &service.monitor_store,
                                                event_id,
                                                "LLM summarization failed (archived without summary)",
                                                started.elapsed().as_millis() as u64,
                                            );
                                        }
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            // Error monitor event
                            if let Some(event_id) = &monitor_event_id {
                                error_compaction_event(
                                    &service.monitor_store,
                                    event_id,
                                    &e,
                                    started.elapsed().as_millis() as u64,
                                );
                            }
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
        let archived_sessions = count_raw_archive_files(&archive_dir);
        let summarized_sessions = count_summary_files(&archive_dir);

        let (indexed_sources, total_lines) = match self.list_sources(client_id, None, None) {
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
            summarized_sessions,
            indexed_sources,
            total_lines,
        })
    }

    /// Force-compact all expired (non-active) sessions for a client.
    ///
    /// Archives sessions and optionally summarizes them with the configured LLM.
    /// Calls `progress_fn(current, total)` after each session is processed.
    pub async fn force_compact(
        &self,
        client_id: &str,
        mut progress_fn: impl FnMut(usize, usize),
    ) -> Result<CompactResult, String> {
        let client_dir = self.memory_dir.join(client_id);
        let sessions_dir = client_dir.join("sessions");
        let archive_dir = client_dir.join("archive");

        let active_path = self.session_manager.active_session_path(client_id);

        // Collect files to compact
        let entries = std::fs::read_dir(&sessions_dir)
            .map_err(|e| format!("Failed to read sessions dir: {}", e))?;

        let mut files_to_compact: Vec<PathBuf> = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("md") {
                continue;
            }
            if active_path.as_ref().is_some_and(|active| active == &path) {
                continue;
            }
            files_to_compact.push(path);
        }

        let total = files_to_compact.len();
        progress_fn(0, total);

        // Clone Arc to avoid holding RwLock across await
        let config = self.config.read().clone();
        let (llm_arc, model) =
            if config.compaction_enabled && config.compaction_model.is_some() {
                (
                    self.compaction_llm.read().clone(),
                    config.compaction_model.clone(),
                )
            } else {
                (None, None)
            };

        let mut archived_count = 0;
        let mut summarized_count = 0;

        for (i, path) in files_to_compact.iter().enumerate() {
            let session_id = path
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            let short_id = &session_id[..8.min(session_id.len())];

            // Read transcript size for monitor event
            let transcript_bytes = tokio::fs::metadata(path)
                .await
                .map(|m| m.len())
                .unwrap_or(0);

            // Compute file paths for monitor event
            let transcript_rel_path = format!(
                "{}/archive/{}.md",
                client_id, session_id
            );
            let summary_rel_path = format!(
                "{}/archive/{}-summary.md",
                client_id, session_id
            );

            // Emit monitor event if LLM compaction will happen
            let monitor_event_id = if model.is_some() {
                emit_compaction_event(
                    &self.monitor_store,
                    short_id,
                    model.as_deref().unwrap_or(""),
                    transcript_bytes,
                    client_id,
                    Some(&transcript_rel_path),
                )
            } else {
                None
            };

            let started = std::time::Instant::now();

            match compaction::compact_session(path, &archive_dir, llm_arc.as_deref(), model.as_deref()).await {
                Ok(outcome) => {
                    archived_count += 1;

                    // Update FTS5 index based on outcome
                    if let Ok(store) = self.get_or_create_store(client_id) {
                        match outcome {
                            compaction::CompactionOutcome::ArchivedAndSummarized(result) => {
                                summarized_count += 1;
                                // Index the summary
                                let summary_label =
                                    format!("session/{}-summary", session_id);
                                let _ = store.index(&summary_label, &result.summary);

                                // Complete monitor event with full metadata
                                if let Some(event_id) = &monitor_event_id {
                                    complete_compaction_event(
                                        &self.monitor_store,
                                        event_id,
                                        &result,
                                        transcript_bytes,
                                        &summary_rel_path,
                                        started.elapsed().as_millis() as u64,
                                    );
                                }

                                // Remove raw transcript from index
                                let raw_label = format!("session/{}", session_id);
                                let _ = store.delete(&raw_label);
                            }
                            compaction::CompactionOutcome::ArchivedEmptyResponse(result) => {
                                if let Some(event_id) = &monitor_event_id {
                                    error_compaction_event_with_result(
                                        &self.monitor_store,
                                        event_id,
                                        "LLM returned empty summary",
                                        &result,
                                        transcript_bytes,
                                        started.elapsed().as_millis() as u64,
                                    );
                                }
                            }
                            compaction::CompactionOutcome::ArchivedOnly => {
                                // Raw transcript stays indexed as-is.
                                // If we expected summarization, mark as error
                                if let Some(event_id) = &monitor_event_id {
                                    error_compaction_event(
                                        &self.monitor_store,
                                        event_id,
                                        "LLM summarization failed (archived without summary)",
                                        started.elapsed().as_millis() as u64,
                                    );
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    // Error monitor event
                    if let Some(event_id) = &monitor_event_id {
                        error_compaction_event(
                            &self.monitor_store,
                            event_id,
                            &e,
                            started.elapsed().as_millis() as u64,
                        );
                    }
                    tracing::warn!("Force compact failed for {:?}: {}", path, e);
                }
            }

            progress_fn(i + 1, total);
        }

        Ok(CompactResult {
            archived_count,
            summarized_count,
        })
    }

    /// Re-compact all archived sessions for a client by regenerating LLM summaries.
    ///
    /// Calls `progress_fn(current, total)` after each session is processed.
    pub async fn recompact_all(
        &self,
        client_id: &str,
        mut progress_fn: impl FnMut(usize, usize),
    ) -> Result<RecompactResult, String> {
        let config = self.config.read().clone();
        if !config.compaction_enabled {
            return Err("Compaction is not enabled".to_string());
        }
        let model_str = config
            .compaction_model
            .clone()
            .ok_or("No compaction model configured")?;

        // Clone Arc to avoid holding RwLock across await
        let llm_arc = self.compaction_llm.read().clone();
        let llm = llm_arc.as_deref().ok_or("Compaction LLM not available")?;

        let client_dir = self.memory_dir.join(client_id);
        let archive_dir = client_dir.join("archive");

        // Collect raw transcript files (exclude *-summary.md)
        let raw_files = collect_raw_archive_files(&archive_dir)?;

        let total = raw_files.len();
        progress_fn(0, total);

        let mut recompacted_count = 0;
        let mut failed_count = 0;

        for (i, session_id) in raw_files.iter().enumerate() {
            let short_id = &session_id[..8.min(session_id.len())];

            // Read transcript size for monitor event
            let raw_path = archive_dir.join(format!("{}.md", session_id));
            let transcript_bytes = std::fs::metadata(&raw_path)
                .map(|m| m.len())
                .unwrap_or(0);

            // Compute file paths for monitor event
            let transcript_rel_path = format!(
                "{}/archive/{}.md",
                client_id, session_id
            );
            let summary_rel_path = format!(
                "{}/archive/{}-summary.md",
                client_id, session_id
            );

            let monitor_event_id = emit_compaction_event(
                &self.monitor_store,
                short_id,
                &model_str,
                transcript_bytes,
                client_id,
                Some(&transcript_rel_path),
            );

            let started = std::time::Instant::now();

            match compaction::recompact_session(session_id, &archive_dir, llm, &model_str).await {
                Ok(result) => {
                    recompacted_count += 1;

                    // Update FTS5 index: index summary, delete raw
                    if let Ok(store) = self.get_or_create_store(client_id) {
                        let summary_label =
                            format!("session/{}-summary", session_id);
                        let _ = store.index(&summary_label, &result.summary);

                        if let Some(event_id) = &monitor_event_id {
                            complete_compaction_event(
                                &self.monitor_store,
                                event_id,
                                &result,
                                transcript_bytes,
                                &summary_rel_path,
                                started.elapsed().as_millis() as u64,
                            );
                        }

                        // Remove raw transcript from index (if it was indexed)
                        let raw_label = format!("session/{}", session_id);
                        let _ = store.delete(&raw_label);
                    }
                }
                Err(e) => {
                    failed_count += 1;
                    if let Some(event_id) = &monitor_event_id {
                        error_compaction_event(
                            &self.monitor_store,
                            event_id,
                            &e,
                            started.elapsed().as_millis() as u64,
                        );
                    }
                    tracing::warn!(
                        "Recompact failed for session {}: {}",
                        short_id,
                        e,
                    );
                }
            }

            progress_fn(i + 1, total);
        }

        Ok(RecompactResult {
            recompacted_count,
            failed_count,
        })
    }

    /// Rebuild the FTS5 index from all session and archive `.md` files on disk.
    ///
    /// Drops the existing store, deletes `memory.db`, and re-indexes everything.
    /// When a summary file exists alongside a raw transcript in archive/,
    /// only the summary is indexed.
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

        // Collect files to index from sessions/
        let sessions_dir = client_dir.join("sessions");
        let archive_dir = client_dir.join("archive");

        // (label, file_path) pairs to index
        let mut to_index: Vec<(String, PathBuf)> = Vec::new();

        // Sessions dir: index all .md files normally
        if let Ok(entries) = std::fs::read_dir(&sessions_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("md") {
                    let file_name = path.file_stem().unwrap_or_default().to_string_lossy();
                    let label = format!("session/{}", file_name);
                    to_index.push((label, path));
                }
            }
        }

        // Archive dir: prefer summary files over raw transcripts
        if let Ok(entries) = std::fs::read_dir(&archive_dir) {
            // First pass: collect all files
            let mut raw_files: Vec<(String, PathBuf)> = Vec::new(); // (session_id, path)
            let mut summary_files: Vec<(String, PathBuf)> = Vec::new(); // (session_id, path)

            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("md") {
                    continue;
                }
                let file_name = path
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();

                if let Some(session_id) = file_name.strip_suffix("-summary") {
                    summary_files.push((session_id.to_string(), path));
                } else {
                    raw_files.push((file_name, path));
                }
            }

            // Build a set of session IDs that have summaries
            let summarized_ids: std::collections::HashSet<&str> =
                summary_files.iter().map(|(id, _)| id.as_str()).collect();

            // Index summary files with their summary label
            for (session_id, path) in &summary_files {
                let label = format!("session/{}-summary", session_id);
                to_index.push((label, path.clone()));
            }

            // Index raw files only if no summary exists
            for (session_id, path) in &raw_files {
                if !summarized_ids.contains(session_id.as_str()) {
                    let label = format!("session/{}", session_id);
                    to_index.push((label, path.clone()));
                }
            }
        }

        let total = to_index.len();
        progress_fn(0, total);

        // Re-create the store and index each file
        let store = self.get_or_create_store(client_id)?;
        for (i, (label, file_path)) in to_index.iter().enumerate() {
            match std::fs::read_to_string(file_path) {
                Ok(content) if !content.trim().is_empty() => {
                    if let Err(e) = store.index(label, &content) {
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
    pub summarized_sessions: usize,
    pub indexed_sources: usize,
    pub total_lines: usize,
}

/// Result of a force-compact operation.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CompactResult {
    pub archived_count: usize,
    pub summarized_count: usize,
}

/// Result of a re-compact operation.
#[derive(Debug, Clone, serde::Serialize)]
pub struct RecompactResult {
    pub recompacted_count: usize,
    pub failed_count: usize,
}

// ─────────────────────────────────────────────────────────
// Monitor event helpers
// ─────────────────────────────────────────────────────────

/// Emit a Pending MemoryCompaction monitor event. Returns the event ID if emitted.
fn emit_compaction_event(
    monitor_store: &RwLock<Option<Arc<MonitorEventStore>>>,
    short_session_id: &str,
    model: &str,
    transcript_bytes: u64,
    client_id: &str,
    transcript_path: Option<&str>,
) -> Option<String> {
    let store = monitor_store.read().clone()?;
    Some(store.push(
        lr_monitor::MonitorEventType::MemoryCompaction,
        Some(client_id.to_string()),
        None,
        None,
        lr_monitor::MonitorEventData::MemoryCompaction {
            session_id: short_session_id.to_string(),
            model: model.to_string(),
            transcript_bytes,
            transcript_path: transcript_path.map(|s| s.to_string()),
            request_body: None,
            summary_bytes: None,
            summary_path: None,
            compression_ratio: None,
            input_tokens: None,
            output_tokens: None,
            reasoning_tokens: None,
            finish_reason: None,
            response_body: None,
            content_preview: None,
            error: None,
        },
        lr_monitor::EventStatus::Pending,
        None,
    ))
}

/// Update a MemoryCompaction monitor event to Complete with full response metadata.
fn complete_compaction_event(
    monitor_store: &RwLock<Option<Arc<MonitorEventStore>>>,
    event_id: &str,
    result: &compaction::CompactionResult,
    transcript_bytes: u64,
    summary_path: &str,
    duration_ms: u64,
) {
    let Some(store) = monitor_store.read().clone() else {
        return;
    };
    let summary_bytes = result.summary.len() as u64;
    let ratio = if transcript_bytes > 0 {
        (1.0 - (summary_bytes as f64 / transcript_bytes as f64)) * 100.0
    } else {
        0.0
    };
    let content_preview = if result.summary.len() > 10_000 {
        Some(format!("{}...", &result.summary[..10_000]))
    } else {
        Some(result.summary.clone())
    };
    store.update(event_id, |event| {
        event.status = lr_monitor::EventStatus::Complete;
        event.duration_ms = Some(duration_ms);
        if let lr_monitor::MonitorEventData::MemoryCompaction {
            summary_bytes: sb,
            summary_path: sp,
            compression_ratio: cr,
            input_tokens: it,
            output_tokens: ot,
            reasoning_tokens: rt,
            finish_reason: fr,
            request_body: req,
            response_body: resp,
            content_preview: cp,
            ..
        } = &mut event.data
        {
            *sb = Some(summary_bytes);
            *sp = Some(summary_path.to_string());
            *cr = Some(ratio);
            *it = Some(result.input_tokens as u64);
            *ot = Some(result.output_tokens as u64);
            *rt = result.reasoning_tokens.map(|t| t as u64);
            *fr = result.finish_reason.clone();
            *req = result.request_body.clone();
            *resp = result.response_body.clone();
            *cp = content_preview.clone();
        }
    });
}

/// Update a MemoryCompaction monitor event to Error.
fn error_compaction_event(
    monitor_store: &RwLock<Option<Arc<MonitorEventStore>>>,
    event_id: &str,
    error_msg: &str,
    duration_ms: u64,
) {
    let Some(store) = monitor_store.read().clone() else {
        return;
    };
    store.update(event_id, |event| {
        event.status = lr_monitor::EventStatus::Error;
        event.duration_ms = Some(duration_ms);
        if let lr_monitor::MonitorEventData::MemoryCompaction { error, .. } = &mut event.data {
            *error = Some(error_msg.to_string());
        }
    });
}

/// Update a MemoryCompaction monitor event to Error, preserving the LLM response metadata.
///
/// Used when the LLM responded but the summary was empty — we still want the full
/// request/response bodies and token counts for debugging.
fn error_compaction_event_with_result(
    monitor_store: &RwLock<Option<Arc<MonitorEventStore>>>,
    event_id: &str,
    error_msg: &str,
    result: &compaction::CompactionResult,
    transcript_bytes: u64,
    duration_ms: u64,
) {
    let Some(store) = monitor_store.read().clone() else {
        return;
    };
    let ratio = if transcript_bytes > 0 {
        (1.0 - (result.summary.len() as f64 / transcript_bytes as f64)) * 100.0
    } else {
        0.0
    };
    store.update(event_id, |event| {
        event.status = lr_monitor::EventStatus::Error;
        event.duration_ms = Some(duration_ms);
        if let lr_monitor::MonitorEventData::MemoryCompaction {
            summary_bytes: sb,
            compression_ratio: cr,
            input_tokens: it,
            output_tokens: ot,
            reasoning_tokens: rt,
            finish_reason: fr,
            request_body: req,
            response_body: resp,
            error: err,
            ..
        } = &mut event.data
        {
            *sb = Some(result.summary.len() as u64);
            *cr = Some(ratio);
            *it = Some(result.input_tokens as u64);
            *ot = Some(result.output_tokens as u64);
            *rt = result.reasoning_tokens.map(|t| t as u64);
            *fr = result.finish_reason.clone();
            *req = result.request_body.clone();
            *resp = result.response_body.clone();
            *err = Some(error_msg.to_string());
        }
    });
}

/// Count `.md` files in a directory (all markdown files).
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

/// Count raw archive files (`.md` files NOT ending in `-summary.md`).
fn count_raw_archive_files(dir: &Path) -> usize {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .filter(|e| {
                    let path = e.path();
                    path.extension().and_then(|ext| ext.to_str()) == Some("md")
                        && !path
                            .file_stem()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .ends_with("-summary")
                })
                .count()
        })
        .unwrap_or(0)
}

/// Count summary files (`*-summary.md`) in a directory.
fn count_summary_files(dir: &Path) -> usize {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .filter(|e| {
                    let path = e.path();
                    path.extension().and_then(|ext| ext.to_str()) == Some("md")
                        && path
                            .file_stem()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .ends_with("-summary")
                })
                .count()
        })
        .unwrap_or(0)
}

/// Collect session IDs of raw archive files (excluding summary files).
fn collect_raw_archive_files(archive_dir: &Path) -> Result<Vec<String>, String> {
    let entries =
        std::fs::read_dir(archive_dir).map_err(|e| format!("Failed to read archive dir: {}", e))?;

    let mut session_ids = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let file_name = path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        if !file_name.ends_with("-summary") {
            session_ids.push(file_name);
        }
    }

    Ok(session_ids)
}
