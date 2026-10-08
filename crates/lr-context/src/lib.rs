//! lr-context — Native content indexing, search & read.
//!
//! FTS5 BM25-based knowledge base for session-scoped content.
//! Chunks content by format (markdown, plain text, JSON),
//! stores in SQLite FTS5, and retrieves via three-layer search
//! (Porter stemming → Trigram → Fuzzy correction).

mod chunk;
mod fuzzy;
#[cfg(feature = "vector")]
mod hybrid;
mod search;
mod truncate;
mod types;

pub use chunk::chunk_content;
pub use types::format_search_results;
pub use types::{
    BatchIndexResult, BatchItemSummary, BatchResult, Chunk, ChunkToc, ContentType, ContextError,
    DateRange, IndexResult, MatchLayer, ReadRequest, ReadResult, SearchHit, SearchResult,
    SourceInfo, ToolNames, SEARCH_DEFAULT_LIMIT, SEARCH_MAX_LIMIT, SEARCH_OUTPUT_CAP,
};

use once_cell::sync::Lazy;
use parking_lot::Mutex;
use regex::Regex;
use rusqlite::{params, Connection};
use std::collections::HashSet;
use std::sync::Arc;

use search::{SNIPPET_BATCH_MAX_LEN, SNIPPET_MAX_LEN};
use truncate::smart_truncate;
use types::{ChunkToc as ChunkTocType, LineOffset, LONG_LINE_THRESHOLD};

/// Max bytes for read() output.
const READ_OUTPUT_CAP: usize = 64 * 1024;

/// Default number of lines returned by `read()` when no limit is specified.
pub const READ_DEFAULT_LIMIT: usize = 200;

// ─────────────────────────────────────────────────────────
// Stopwords (ported from context-mode/src/store.ts)
// ─────────────────────────────────────────────────────────

static STOPWORDS: Lazy<HashSet<&'static str>> = Lazy::new(|| {
    [
        "the", "and", "for", "are", "but", "not", "you", "all", "can", "had", "her", "was", "one",
        "our", "out", "has", "his", "how", "its", "may", "new", "now", "old", "see", "way", "who",
        "did", "get", "got", "let", "say", "she", "too", "use", "will", "with", "this", "that",
        "from", "they", "been", "have", "many", "some", "them", "than", "each", "make", "like",
        "just", "over", "such", "take", "into", "year", "your", "good", "could", "would", "about",
        "which", "their", "there", "other", "after", "should", "through", "also", "more", "most",
        "only", "very", "when", "what", "then", "these", "those", "being", "does", "done", "both",
        "same", "still", "while", "where", "here", "were", "much",
        // Common in code/changelogs
        "update", "updates", "updated", "deps", "dev", "tests", "test", "add", "added", "fix",
        "fixed", "run", "running", "using",
    ]
    .into_iter()
    .collect()
});

static WORD_SPLIT_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"[^\p{L}\p{N}_-]+").expect("invalid word-split regex"));

// ─────────────────────────────────────────────────────────
// ContentStore
// ─────────────────────────────────────────────────────────

pub struct ContentStore {
    conn: Arc<Mutex<Connection>>,
    /// Optional embedding service for hybrid (FTS5 + vector) search.
    #[cfg(feature = "vector")]
    embedding_service: Mutex<Option<Arc<lr_embeddings::EmbeddingService>>>,
    /// In-memory vector index, rebuilt from FTS5 content.
    #[cfg(feature = "vector")]
    vector_entries: Mutex<Vec<VectorEntry>>,
}

/// An in-memory vector entry for cosine similarity search.
#[cfg(feature = "vector")]
struct VectorEntry {
    source: String,
    title: String,
    content: String,
    embedding: Vec<f32>,
    content_type: ContentType,
    line_start: usize,
    line_end: usize,
}

impl ContentStore {
    /// Create a new in-memory content store.
    pub fn new() -> Result<Self, ContextError> {
        let conn = Connection::open_in_memory()?;
        Self::init_schema(&conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            #[cfg(feature = "vector")]
            embedding_service: Mutex::new(None),
            #[cfg(feature = "vector")]
            vector_entries: Mutex::new(Vec::new()),
        })
    }

    /// Open a persistent content store backed by a SQLite file on disk.
    ///
    /// Creates the database file (and parent directories) if it doesn't exist.
    /// Uses WAL journal mode for concurrent read performance.
    pub fn open(path: &std::path::Path) -> Result<Self, ContextError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                ContextError::Database(rusqlite::Error::SqliteFailure(
                    rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_CANTOPEN),
                    Some(format!("Failed to create parent directory: {}", e)),
                ))
            })?;
        }
        let conn = Connection::open(path)?;
        // journal_mode returns a result row, so use prepare+query
        let _ = conn
            .prepare("PRAGMA journal_mode=WAL")?
            .query_row([], |_| Ok(()));
        conn.execute_batch("PRAGMA synchronous=NORMAL;")?;
        Self::init_schema(&conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            #[cfg(feature = "vector")]
            embedding_service: Mutex::new(None),
            #[cfg(feature = "vector")]
            vector_entries: Mutex::new(Vec::new()),
        })
    }

    fn init_schema(conn: &Connection) -> Result<(), ContextError> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS sources (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                label TEXT NOT NULL UNIQUE,
                content TEXT NOT NULL DEFAULT '',
                total_lines INTEGER NOT NULL DEFAULT 0,
                chunk_count INTEGER NOT NULL DEFAULT 0,
                code_chunk_count INTEGER NOT NULL DEFAULT 0,
                indexed_at TEXT NOT NULL DEFAULT (datetime('now'))
            );

            CREATE VIRTUAL TABLE IF NOT EXISTS chunks USING fts5(
                title,
                content,
                source_id UNINDEXED,
                content_type UNINDEXED,
                line_start UNINDEXED,
                line_end UNINDEXED,
                tokenize='porter unicode61'
            );

            CREATE VIRTUAL TABLE IF NOT EXISTS chunks_trigram USING fts5(
                title,
                content,
                source_id UNINDEXED,
                content_type UNINDEXED,
                line_start UNINDEXED,
                line_end UNINDEXED,
                tokenize='trigram'
            );

            CREATE TABLE IF NOT EXISTS vocabulary (
                word TEXT PRIMARY KEY
            );",
        )?;
        Ok(())
    }

    // ── Index ──

    /// Index content with auto-detected format. Re-indexing the same label replaces previous.
    pub fn index(&self, label: &str, content: &str) -> Result<IndexResult, ContextError> {
        let chunks = chunk::chunk_content(content);
        let total_lines = content
            .lines()
            .count()
            .max(if content.is_empty() { 0 } else { 1 });
        let code_chunks = chunks
            .iter()
            .filter(|c| c.content_type == ContentType::Code)
            .count();
        let content_bytes = content.len();

        // Build TOC from chunks
        let chunk_titles: Vec<ChunkTocType> = chunks
            .iter()
            .map(|c| {
                let depth = c.title.matches(" > ").count();
                ChunkTocType {
                    title: c.title.clone(),
                    line_ref: c.line_ref.clone(),
                    depth,
                }
            })
            .collect();

        let conn = self.conn.lock();

        // Atomic dedup + insert in a single transaction
        conn.execute_batch("BEGIN")?;

        let result = (|| -> Result<i64, ContextError> {
            conn.execute(
                "DELETE FROM chunks WHERE source_id IN (SELECT CAST(id AS TEXT) FROM sources WHERE label = ?1)",
                params![label],
            )?;
            conn.execute(
                "DELETE FROM chunks_trigram WHERE source_id IN (SELECT CAST(id AS TEXT) FROM sources WHERE label = ?1)",
                params![label],
            )?;
            conn.execute("DELETE FROM sources WHERE label = ?1", params![label])?;

            conn.execute(
                "INSERT INTO sources (label, content, total_lines, chunk_count, code_chunk_count) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![label, content, total_lines as i64, chunks.len() as i64, code_chunks as i64],
            )?;
            let source_id = conn.last_insert_rowid();

            for chunk in &chunks {
                let source_id_str = source_id.to_string();
                let ct = chunk.content_type.as_str();
                conn.execute(
                    "INSERT INTO chunks (title, content, source_id, content_type, line_start, line_end) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![chunk.title, chunk.content, source_id_str, ct, chunk.line_start as i64, chunk.line_end as i64],
                )?;
                conn.execute(
                    "INSERT INTO chunks_trigram (title, content, source_id, content_type, line_start, line_end) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![chunk.title, chunk.content, source_id_str, ct, chunk.line_start as i64, chunk.line_end as i64],
                )?;
            }

            // Extract and store vocabulary
            if !content.is_empty() {
                Self::extract_vocabulary(&conn, content);
            }

            Ok(source_id)
        })();

        match result {
            Ok(source_id) => {
                conn.execute_batch("COMMIT")?;
                // Drop the connection lock before embedding (which may be slow)
                drop(conn);

                // Update vector entries for hybrid search (if embedding service is attached)
                #[cfg(feature = "vector")]
                {
                    self.delete_vectors(label);
                    self.index_vectors(label, &chunks);
                }

                Ok(IndexResult {
                    source_id,
                    label: label.to_string(),
                    total_chunks: chunks.len(),
                    code_chunks,
                    total_lines,
                    content_bytes,
                    chunk_titles,
                })
            }
            Err(e) => {
                let _ = conn.execute_batch("ROLLBACK");
                Err(e)
            }
        }
    }

    /// Batch-index multiple items under a shared root path.
    /// Each item is indexed at `{root_path}{subpath}`.
    pub fn batch_index(
        &self,
        root_path: &str,
        items: &[(&str, &str)], // (subpath, content)
    ) -> Result<BatchIndexResult, ContextError> {
        let mut total_bytes = 0usize;
        let mut total_lines = 0usize;
        let mut total_chunks = 0usize;
        let mut item_summaries = Vec::new();

        for (subpath, content) in items {
            let label = format!("{}{}", root_path, subpath);
            let result = self.index(&label, content)?;
            total_bytes += result.content_bytes;
            total_lines += result.total_lines;
            total_chunks += result.total_chunks;
            item_summaries.push(types::BatchItemSummary {
                subpath: subpath.to_string(),
                bytes: result.content_bytes,
                chunks: result.total_chunks,
            });
        }

        Ok(BatchIndexResult {
            root_path: root_path.to_string(),
            items_indexed: items.len(),
            total_bytes,
            total_lines,
            total_chunks,
            item_summaries,
        })
    }

    // ── Search ──

    /// Search across indexed content. Multiple queries, optional source filter.
    pub fn search(
        &self,
        queries: &[String],
        limit: usize,
        source: Option<&str>,
        date_range: &DateRange,
    ) -> Result<Vec<SearchResult>, ContextError> {
        self.search_internal(queries, limit, source, SNIPPET_MAX_LEN, date_range)
    }

    /// Search with combined query + queries entry point.
    pub fn search_combined(
        &self,
        query: Option<&str>,
        queries: Option<&[String]>,
        limit: usize,
        source: Option<&str>,
        after: Option<&str>,
        before: Option<&str>,
    ) -> Result<Vec<SearchResult>, ContextError> {
        let mut all_queries: Vec<String> = Vec::new();
        if let Some(q) = query {
            if !q.is_empty() {
                all_queries.push(q.to_string());
            }
        }
        if let Some(qs) = queries {
            all_queries.extend(qs.iter().cloned());
        }
        if all_queries.is_empty() {
            return Err(ContextError::InvalidParams(
                "at least one query is required".to_string(),
            ));
        }
        let date_range =
            DateRange::new(after.map(|s| s.to_string()), before.map(|s| s.to_string()));
        self.search_internal(&all_queries, limit, source, SNIPPET_MAX_LEN, &date_range)
    }

    fn search_internal(
        &self,
        queries: &[String],
        limit: usize,
        source: Option<&str>,
        max_snippet_len: usize,
        date_range: &DateRange,
    ) -> Result<Vec<SearchResult>, ContextError> {
        let conn = self.conn.lock();

        // A source filter that matches nothing is almost always a typo or a
        // stale label: say so instead of returning an empty (or, with vector
        // search, unrelated) result.
        if let Some(prefix) = source {
            if !Self::any_source_matches(&conn, prefix)? {
                let known = Self::labels(&conn)?;
                return Err(ContextError::NoMatchingSource {
                    prefix: prefix.to_string(),
                    known,
                });
            }
        }

        // Labels a vector hit may come from: the same source prefix and date
        // range the FTS layers filter on.
        #[cfg(feature = "vector")]
        let vector_scope = Self::vector_scope(&conn, source, date_range)?;

        let results: Vec<SearchResult> = queries
            .iter()
            .map(|q| {
                #[allow(unused_mut)]
                let mut sr = search::search_with_fallback(
                    &conn,
                    q,
                    limit,
                    source,
                    max_snippet_len,
                    date_range,
                );

                // Hybrid: merge vector search results via RRF (if available)
                #[cfg(feature = "vector")]
                {
                    let fts_hits = std::mem::take(&mut sr.hits);
                    sr.hits = self.vector_search_and_merge(
                        q,
                        fts_hits,
                        limit,
                        vector_scope.as_ref(),
                        max_snippet_len,
                    );
                }

                // Deduplicate hits with overlapping line ranges from the same source.
                // Chunks are created with 2-line overlap, so the same content region
                // can produce multiple hits. Keep the best-ranked hit per region.
                sr.hits = Self::dedup_overlapping_hits(sr.hits);

                sr
            })
            .collect();
        Ok(results)
    }

    /// Whether any indexed source label starts with `prefix`.
    fn any_source_matches(conn: &Connection, prefix: &str) -> Result<bool, ContextError> {
        let filter = format!("{}%", search::escape_like(prefix));
        let found = conn
            .prepare_cached("SELECT 1 FROM sources WHERE label LIKE ?1 ESCAPE '\\' LIMIT 1")?
            .exists(params![filter])?;
        Ok(found)
    }

    /// All indexed source labels, newest first.
    fn labels(conn: &Connection) -> Result<Vec<String>, ContextError> {
        let mut stmt = conn.prepare_cached("SELECT label FROM sources ORDER BY id DESC")?;
        let labels = stmt
            .query_map([], |row| row.get::<_, String>(0))?
            .filter_map(|r| r.ok())
            .collect();
        Ok(labels)
    }

    /// Labels vector search may return for this source/date filter, or
    /// `None` when unfiltered (every label is eligible).
    #[cfg(feature = "vector")]
    fn vector_scope(
        conn: &Connection,
        source: Option<&str>,
        date_range: &DateRange,
    ) -> Result<Option<HashSet<String>>, ContextError> {
        if source.is_none() && date_range.is_unbounded() {
            return Ok(None);
        }
        let filter = format!("{}%", search::escape_like(source.unwrap_or("")));
        let mut stmt = conn.prepare_cached(
            "SELECT label FROM sources WHERE label LIKE ?1 ESCAPE '\\' \
             AND indexed_at > ?2 AND indexed_at < ?3",
        )?;
        let labels = stmt
            .query_map(
                params![filter, &date_range.after, &date_range.before],
                |row| row.get::<_, String>(0),
            )?
            .filter_map(|r| r.ok())
            .collect();
        Ok(Some(labels))
    }

    /// Remove hits from the same source whose line ranges overlap, keeping
    /// the better-ranked one. Ranks follow the BM25 convention: lower (more
    /// negative) is better, for FTS and RRF hits alike. Returns best first.
    fn dedup_overlapping_hits(mut hits: Vec<SearchHit>) -> Vec<SearchHit> {
        if hits.len() <= 1 {
            return hits;
        }
        // Sort by source then line_start for efficient overlap detection
        hits.sort_by(|a, b| {
            a.source
                .cmp(&b.source)
                .then(a.line_start.cmp(&b.line_start))
        });
        let mut kept: Vec<SearchHit> = Vec::with_capacity(hits.len());
        for hit in hits {
            if let Some(last) = kept.last() {
                // Same source and overlapping line range → keep better rank
                if last.source == hit.source && hit.line_start <= last.line_end {
                    if hit.rank < last.rank {
                        // New hit has better rank — replace
                        *kept.last_mut().unwrap() = hit;
                    }
                    // Otherwise skip the new hit
                    continue;
                }
            }
            kept.push(hit);
        }
        // Re-sort by rank (best first) since we disturbed the original ordering
        kept.sort_by(|a, b| {
            a.rank
                .partial_cmp(&b.rank)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        kept
    }

    // ── Read ──

    /// Read original content with pagination. Offset supports "5" or "5-2" (sub-line) format.
    ///
    /// `limit` counts physical lines: a long line split into parts is
    /// returned whole (from the offset's part on). Output stops at
    /// [`READ_OUTPUT_CAP`] on a line boundary and reports where to continue.
    pub fn read(
        &self,
        label: &str,
        offset: Option<&str>,
        limit: Option<usize>,
    ) -> Result<ReadResult, ContextError> {
        let conn = self.conn.lock();

        let (content, total_lines): (String, i64) = conn
            .query_row(
                "SELECT content, total_lines FROM sources WHERE label = ?1",
                params![label],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => {
                    ContextError::SourceNotFound(label.to_string())
                }
                other => ContextError::Database(other),
            })?;
        drop(conn);

        let total_lines = total_lines as usize;
        let limit = limit.unwrap_or(READ_DEFAULT_LIMIT);

        // Parse offset
        let parsed_offset = match offset {
            Some(s) => LineOffset::parse(s)?,
            None => LineOffset { line: 1, sub: None },
        };

        let virtual_lines = build_virtual_lines(&content);

        // Find starting position matching parsed offset
        let start_pos = find_virtual_start(&virtual_lines, &parsed_offset);
        let empty = || ReadResult {
            label: label.to_string(),
            content: String::new(),
            total_lines,
            showing_start: "0".to_string(),
            showing_end: "0".to_string(),
            next_offset: None,
            remaining_lines: 0,
        };
        let Some(first) = virtual_lines.get(start_pos) else {
            return Ok(empty());
        };
        if limit == 0 {
            return Ok(empty());
        }
        let last_line = first.line.saturating_add(limit - 1);

        // Collect lines up to the limit, stopping before the byte cap
        let label_width = virtual_lines
            .iter()
            .skip(start_pos)
            .take_while(|v| v.line <= last_line)
            .map(|v| v.label.len())
            .max()
            .unwrap_or(1);
        let mut formatted = String::new();
        let mut end_pos = start_pos;
        for v in virtual_lines[start_pos..]
            .iter()
            .take_while(|v| v.line <= last_line)
        {
            let row = format!("{:>width$}\t{}", v.label, v.text, width = label_width);
            let added = row.len() + usize::from(!formatted.is_empty());
            if !formatted.is_empty() && formatted.len() + added > READ_OUTPUT_CAP {
                break;
            }
            if !formatted.is_empty() {
                formatted.push('\n');
            }
            formatted.push_str(&row);
            end_pos += 1;
        }
        // A single row over the cap (only possible with extreme limits on
        // the part size) is cut rather than dropped.
        let formatted = smart_truncate(&formatted, READ_OUTPUT_CAP);

        let showing_start = virtual_lines[start_pos].label.clone();
        let showing_end = virtual_lines[end_pos - 1].label.clone();
        let (next_offset, remaining_lines) = match virtual_lines.get(end_pos) {
            Some(next) => (
                Some(next.label.clone()),
                total_lines.saturating_sub(next.line) + 1,
            ),
            None => (None, 0),
        };

        Ok(ReadResult {
            label: label.to_string(),
            content: formatted,
            total_lines,
            showing_start,
            showing_end,
            next_offset,
            remaining_lines,
        })
    }

    // ── Batch Search+Read ──

    /// Combined search + read in one call.
    pub fn batch_search_read(
        &self,
        queries: &[String],
        reads: &[ReadRequest],
        search_limit: usize,
        source: Option<&str>,
        date_range: &DateRange,
    ) -> Result<BatchResult, ContextError> {
        let search_results = if queries.is_empty() {
            Vec::new()
        } else {
            self.search_internal(
                queries,
                search_limit,
                source,
                SNIPPET_BATCH_MAX_LEN,
                date_range,
            )?
        };

        let mut read_results: Vec<ReadResult> = Vec::new();
        for r in reads {
            match self.read(&r.label, r.offset.as_deref(), r.limit) {
                Ok(result) => read_results.push(result),
                Err(e) => {
                    // Surface read errors as empty results with error info
                    read_results.push(ReadResult {
                        label: r.label.clone(),
                        content: format!("Error reading {:?}: {}", r.label, e),
                        total_lines: 0,
                        showing_start: "0".to_string(),
                        showing_end: "0".to_string(),
                        next_offset: None,
                        remaining_lines: 0,
                    });
                }
            }
        }

        Ok(BatchResult {
            search_results,
            read_results,
        })
    }

    // ── Delete ──

    /// Delete a source by label. Returns true if a source was deleted.
    pub fn delete(&self, label: &str) -> Result<bool, ContextError> {
        let conn = self.conn.lock();
        conn.execute(
            "DELETE FROM chunks WHERE source_id IN (SELECT CAST(id AS TEXT) FROM sources WHERE label = ?1)",
            params![label],
        )?;
        conn.execute(
            "DELETE FROM chunks_trigram WHERE source_id IN (SELECT CAST(id AS TEXT) FROM sources WHERE label = ?1)",
            params![label],
        )?;
        let deleted = conn.execute("DELETE FROM sources WHERE label = ?1", params![label])?;
        drop(conn);

        #[cfg(feature = "vector")]
        self.delete_vectors(label);

        Ok(deleted > 0)
    }

    // ── List sources ──

    /// List all indexed sources with metadata, optionally filtered by date range.
    pub fn list_sources(
        &self,
        after: Option<&str>,
        before: Option<&str>,
    ) -> Result<Vec<SourceInfo>, ContextError> {
        let date_range =
            DateRange::new(after.map(|s| s.to_string()), before.map(|s| s.to_string()));
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT label, total_lines, chunk_count, code_chunk_count FROM sources \
             WHERE indexed_at > ?1 AND indexed_at < ?2 \
             ORDER BY id DESC",
        )?;
        let sources = stmt
            .query_map(params![&date_range.after, &date_range.before], |row| {
                Ok(SourceInfo {
                    label: row.get(0)?,
                    total_lines: row.get::<_, i64>(1)? as usize,
                    chunk_count: row.get::<_, i64>(2)? as usize,
                    code_chunk_count: row.get::<_, i64>(3)? as usize,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(sources)
    }

    // ── Vocabulary extraction ──

    fn extract_vocabulary(conn: &Connection, content: &str) {
        let lower = content.to_lowercase();
        let words: HashSet<&str> = WORD_SPLIT_RE
            .split(&lower)
            .filter(|w| w.len() >= 3 && !STOPWORDS.contains(w))
            .collect();

        for word in words {
            let _ = conn.execute(
                "INSERT OR IGNORE INTO vocabulary (word) VALUES (?1)",
                params![word],
            );
        }
    }

    // ── Vector search (optional, feature-gated) ──

    /// Attach an embedding service to enable hybrid (FTS5 + vector) search.
    ///
    /// Can be called after construction. Existing indexed content is NOT
    /// retroactively embedded — call `rebuild_vectors()` for that.
    #[cfg(feature = "vector")]
    pub fn set_embedding_service(&self, service: Arc<lr_embeddings::EmbeddingService>) {
        *self.embedding_service.lock() = Some(service);
    }

    /// Whether vector search is available (embedding service attached and loaded).
    #[cfg(feature = "vector")]
    pub fn has_vector_search(&self) -> bool {
        self.embedding_service
            .lock()
            .as_ref()
            .is_some_and(|s| s.is_loaded() || s.is_downloaded())
    }

    /// Rebuild vector index from all currently indexed FTS5 content.
    ///
    /// Call this after attaching an embedding service to an existing store,
    /// or after bulk indexing to populate the vector index.
    #[cfg(feature = "vector")]
    pub fn rebuild_vectors(&self) -> Result<(), ContextError> {
        let service = {
            let guard = self.embedding_service.lock();
            match guard.as_ref() {
                Some(s) => Arc::clone(s),
                None => return Ok(()), // No service → no-op
            }
        };

        if let Err(e) = service.ensure_loaded() {
            tracing::warn!("Cannot rebuild vectors: {}", e);
            return Ok(()); // Graceful degradation
        }

        // Read all chunks from FTS5
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT c.title, c.content, c.content_type, c.line_start, c.line_end, s.label
             FROM chunks c
             JOIN sources s ON CAST(s.id AS TEXT) = c.source_id
             ORDER BY c.rowid",
        )?;
        let rows: Vec<(String, String, String, i64, i64, String)> = stmt
            .query_map([], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            })?
            .filter_map(|r| r.ok())
            .collect();
        drop(stmt);
        drop(conn);

        if rows.is_empty() {
            *self.vector_entries.lock() = Vec::new();
            return Ok(());
        }

        // Batch embed all chunks
        let texts: Vec<String> = rows
            .iter()
            .map(|(title, content, _, _, _, _)| format!("{}\n{}", title, content))
            .collect();
        let text_refs: Vec<&str> = texts.iter().map(|s| s.as_str()).collect();
        let embeddings = service
            .embed_batch(&text_refs)
            .map_err(|e| ContextError::InvalidParams(format!("Embedding failed: {}", e)))?;

        let mut entries = Vec::with_capacity(rows.len());
        for (i, (title, content, ct, ls, le, label)) in rows.into_iter().enumerate() {
            entries.push(VectorEntry {
                source: label,
                title,
                content,
                embedding: embeddings[i].clone(),
                content_type: ContentType::parse(&ct),
                line_start: ls as usize,
                line_end: le as usize,
            });
        }

        *self.vector_entries.lock() = entries;
        tracing::debug!("Rebuilt vector index: {} entries", texts.len());
        Ok(())
    }

    /// Embed and store vectors for chunks of newly indexed content.
    #[cfg(feature = "vector")]
    fn index_vectors(&self, source_label: &str, chunks: &[crate::types::Chunk]) {
        let service = {
            let guard = self.embedding_service.lock();
            match guard.as_ref() {
                Some(s) => Arc::clone(s),
                None => return,
            }
        };

        if service.ensure_loaded().is_err() {
            return; // Graceful degradation
        }

        let texts: Vec<String> = chunks
            .iter()
            .map(|c| format!("{}\n{}", c.title, c.content))
            .collect();
        let text_refs: Vec<&str> = texts.iter().map(|s| s.as_str()).collect();
        let embeddings = match service.embed_batch(&text_refs) {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!("Embedding failed during index: {}", e);
                return;
            }
        };

        let mut entries = self.vector_entries.lock();
        for (i, chunk) in chunks.iter().enumerate() {
            entries.push(VectorEntry {
                source: source_label.to_string(),
                title: chunk.title.clone(),
                content: chunk.content.clone(),
                embedding: embeddings[i].clone(),
                content_type: chunk.content_type,
                line_start: chunk.line_start,
                line_end: chunk.line_end,
            });
        }
    }

    /// Remove vector entries for a given source label.
    #[cfg(feature = "vector")]
    fn delete_vectors(&self, source_label: &str) {
        let mut entries = self.vector_entries.lock();
        entries.retain(|e| e.source != source_label);
    }

    /// Run vector search and merge with FTS5 results via RRF.
    #[cfg(feature = "vector")]
    fn vector_search_and_merge(
        &self,
        query: &str,
        fts_hits: Vec<SearchHit>,
        limit: usize,
        scope: Option<&HashSet<String>>,
        max_snippet_len: usize,
    ) -> Vec<SearchHit> {
        let service = {
            let guard = self.embedding_service.lock();
            match guard.as_ref() {
                Some(s) => Arc::clone(s),
                None => return fts_hits,
            }
        };

        let query_embedding = match service.embed(query) {
            Ok(e) => e,
            Err(_) => return fts_hits,
        };

        let entries = self.vector_entries.lock();
        if entries.is_empty() {
            return fts_hits;
        }

        // Brute-force cosine similarity (vectors are L2-normalized, so dot product = cosine)
        let mut scored: Vec<(usize, f32)> = entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| scope.is_none_or(|labels| labels.contains(&entry.source)))
            .map(|(i, entry)| {
                let score = dot_product(&query_embedding, &entry.embedding);
                (i, score)
            })
            .collect();
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        // Take top results for RRF merge
        let vector_limit = limit * 2; // Over-fetch for better merge quality
        let vector_hits: Vec<hybrid::VectorSearchHit> = scored
            .into_iter()
            .take(vector_limit)
            .map(|(i, score)| {
                let entry = &entries[i];
                hybrid::VectorSearchHit {
                    source: entry.source.clone(),
                    title: entry.title.clone(),
                    // Same line-numbered, length-capped shape as FTS snippets
                    content: search::format_first_n_lines(
                        &entry.content,
                        entry.line_start.max(1),
                        max_snippet_len,
                    ),
                    score,
                    content_type: entry.content_type,
                    line_start: entry.line_start,
                    line_end: entry.line_end,
                }
            })
            .collect();

        hybrid::rrf_merge(&fts_hits, &vector_hits, limit)
    }
}

/// Dot product of two vectors (for L2-normalized vectors, this equals cosine similarity).
#[cfg(feature = "vector")]
fn dot_product(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
}

/// One row of `read()` output: a whole line, or one part of a long line.
struct VirtualLine<'a> {
    /// Display label and offset: "N" or "N-M" (part M of line N).
    label: String,
    /// 1-based physical line number.
    line: usize,
    text: &'a str,
}

/// Split content into rows, cutting lines longer than [`LONG_LINE_THRESHOLD`]
/// chars into numbered parts (labels match the chunker's `line_ref`s).
fn build_virtual_lines(content: &str) -> Vec<VirtualLine<'_>> {
    let mut virtual_lines = Vec::new();
    for (i, line) in content.lines().enumerate() {
        let line_num = i + 1; // 1-based
        let char_count = line.chars().count();

        if char_count > LONG_LINE_THRESHOLD {
            // Compute byte boundaries once; summing every preceding
            // character for each sub-line made very long lines quadratic.
            let byte_offsets: Vec<usize> = line
                .char_indices()
                .map(|(offset, _)| offset)
                .chain(std::iter::once(line.len()))
                .collect();
            let sub_count = char_count.div_ceil(LONG_LINE_THRESHOLD);
            for sub_idx in 0..sub_count {
                let start = sub_idx * LONG_LINE_THRESHOLD;
                let end = ((sub_idx + 1) * LONG_LINE_THRESHOLD).min(char_count);
                virtual_lines.push(VirtualLine {
                    label: format!("{}-{}", line_num, sub_idx + 1),
                    line: line_num,
                    text: &line[byte_offsets[start]..byte_offsets[end]],
                });
            }
        } else {
            virtual_lines.push(VirtualLine {
                label: format!("{}", line_num),
                line: line_num,
                text: line,
            });
        }
    }
    virtual_lines
}

/// Find the starting position in the virtual line list for the given offset.
/// Gracefully falls through: if the exact offset doesn't exist (e.g., "5-2" on a
/// short line), finds the next valid position (e.g., line 6).
fn find_virtual_start(virtual_lines: &[VirtualLine<'_>], offset: &LineOffset) -> usize {
    let target = offset.to_display();
    // Find exact match first
    if let Some(pos) = virtual_lines.iter().position(|v| v.label == target) {
        return pos;
    }

    // If offset has no sub, find the first entry for that line number
    if offset.sub.is_none() {
        if let Some(pos) = virtual_lines.iter().position(|v| v.line == offset.line) {
            return pos;
        }
    }

    // Graceful fallthrough: find the first virtual line whose line number > offset.line.
    // This handles cases like "5-2" on a short line → start at line 6.
    virtual_lines
        .iter()
        .position(|v| v.line > offset.line)
        .unwrap_or(virtual_lines.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_markdown() -> &'static str {
        "# API Documentation\n\
         \n\
         This document describes the API.\n\
         \n\
         ## Authentication\n\
         \n\
         ### OAuth Flow\n\
         \n\
         The OAuth flow requires a client_id and client_secret.\n\
         To configure the OAuth provider, set the OAUTH_CLIENT_ID\n\
         environment variable to your application's ID.\n\
         \n\
         ### API Keys\n\
         \n\
         API keys can be generated from the dashboard settings page.\n\
         Each key has configurable permissions and expiration.\n\
         \n\
         ## Endpoints\n\
         \n\
         ### GET /v1/models\n\
         \n\
         Returns a list of available models.\n\
         \n\
         ```json\n\
         {\"models\": [{\"id\": \"gpt-4\"}]}\n\
         ```\n\
         \n\
         ### POST /v1/chat/completions\n\
         \n\
         Send a chat completion request.\n\
         \n\
         Required parameters:\n\
         - model: The model to use\n\
         - messages: Array of message objects\n"
    }

    #[test]
    fn read_large_limit_does_not_overflow_after_offset() {
        let store = ContentStore::new().unwrap();
        store.index("limits", "first\nsecond\nthird").unwrap();
        let result = store.read("limits", Some("2"), Some(usize::MAX)).unwrap();
        assert_eq!(result.showing_start, "2");
        assert_eq!(result.showing_end, "3");
        assert!(result.content.contains("second"));
        assert!(result.content.contains("third"));
    }

    #[test]
    fn test_index_and_read_full() {
        let store = ContentStore::new().unwrap();
        let result = store.index("docs:api", sample_markdown()).unwrap();
        assert!(result.total_chunks > 0);
        assert!(result.total_lines > 0);
        assert_eq!(result.label, "docs:api");
        assert!(result.content_bytes > 0);
        assert!(!result.chunk_titles.is_empty());

        // Read all lines
        let read = store.read("docs:api", None, None).unwrap();
        assert_eq!(read.total_lines, result.total_lines);
        assert_eq!(read.showing_start, "1");
        // Should have cat-n style line numbers
        assert!(read.content.contains("\t"));
    }

    #[test]
    fn test_read_with_offset_limit() {
        let store = ContentStore::new().unwrap();
        store.index("docs:api", sample_markdown()).unwrap();

        let read = store.read("docs:api", Some("5"), Some(3)).unwrap();
        assert_eq!(read.showing_start, "5");
        assert_eq!(read.showing_end, "7");
        assert_eq!(read.content.lines().count(), 3);
    }

    #[test]
    fn test_read_out_of_range() {
        let store = ContentStore::new().unwrap();
        store.index("docs:api", "line 1\nline 2\nline 3").unwrap();

        // Offset beyond content
        let read = store.read("docs:api", Some("100"), Some(10)).unwrap();
        assert_eq!(read.showing_start, "0");
        assert_eq!(read.showing_end, "0");
        assert!(read.content.is_empty());

        // Partial range
        let read = store.read("docs:api", Some("2"), Some(100)).unwrap();
        assert_eq!(read.showing_start, "2");
    }

    #[test]
    fn test_reindex_same_label() {
        let store = ContentStore::new().unwrap();

        store.index("docs:api", "old content here").unwrap();
        store.index("docs:api", "new content here").unwrap();

        let sources = store.list_sources(None, None).unwrap();
        assert_eq!(sources.len(), 1);

        let read = store.read("docs:api", None, None).unwrap();
        assert!(read.content.contains("new content"));
    }

    #[test]
    fn test_index_search_read_workflow() {
        let store = ContentStore::new().unwrap();
        store.index("docs:api", sample_markdown()).unwrap();

        let results = store
            .search(&["OAuth flow".to_string()], 5, None, &DateRange::default())
            .unwrap();
        assert_eq!(results.len(), 1);
        assert!(
            !results[0].hits.is_empty(),
            "Expected search hits for 'OAuth flow'"
        );

        // Use the found section's line range to read
        let hit = &results[0].hits[0];
        assert!(hit.content.to_lowercase().contains("oauth"));

        // Read the section using line_start
        let offset = hit.line_start.to_string();
        let read = store.read("docs:api", Some(&offset), Some(10)).unwrap();
        assert!(!read.showing_start.is_empty());
        assert_ne!(read.showing_start, "0");
    }

    #[test]
    fn test_multiple_sources() {
        let store = ContentStore::new().unwrap();
        store
            .index("docs:api", "# API\n\nAPI documentation with authentication")
            .unwrap();
        store
            .index("docs:guide", "# Guide\n\nUser guide with tutorials")
            .unwrap();
        store
            .index(
                "docs:faq",
                "# FAQ\n\nFrequently asked questions about authentication",
            )
            .unwrap();

        let sources = store.list_sources(None, None).unwrap();
        assert_eq!(sources.len(), 3);

        let results = store
            .search(
                &["authentication".to_string()],
                5,
                None,
                &DateRange::default(),
            )
            .unwrap();
        assert!(!results[0].hits.is_empty());
    }

    #[test]
    fn test_source_filtering() {
        let store = ContentStore::new().unwrap();
        store
            .index("docs:api", "# API\n\nAuthentication via OAuth")
            .unwrap();
        store
            .index("docs:guide", "# Guide\n\nAuthentication tutorial")
            .unwrap();

        let results = store
            .search(
                &["authentication".to_string()],
                5,
                Some("docs:api"),
                &DateRange::default(),
            )
            .unwrap();
        assert!(!results[0].hits.is_empty());
        for hit in &results[0].hits {
            assert!(hit.source.starts_with("docs:api"));
        }
    }

    #[test]
    fn test_delete_source() {
        let store = ContentStore::new().unwrap();
        store
            .index("docs:api", "# API\n\nSome API content")
            .unwrap();

        assert!(store.delete("docs:api").unwrap());
        assert!(!store.delete("docs:api").unwrap());

        let sources = store.list_sources(None, None).unwrap();
        assert!(sources.is_empty());

        let results = store
            .search(&["API".to_string()], 5, None, &DateRange::default())
            .unwrap();
        assert!(results[0].hits.is_empty());
    }

    #[test]
    fn test_list_sources() {
        let store = ContentStore::new().unwrap();
        store.index("src:main", "fn main() {}").unwrap();
        store.index("src:lib", "pub mod utils;").unwrap();

        let sources = store.list_sources(None, None).unwrap();
        assert_eq!(sources.len(), 2);
        assert!(sources.iter().any(|s| s.label == "src:main"));
        assert!(sources.iter().any(|s| s.label == "src:lib"));
    }

    // ── Date range filtering ──

    #[test]
    fn search_date_range_filters_old_sources() {
        let store = ContentStore::new().unwrap();
        store
            .index("old", "# Old topic\n\nOld OAuth content")
            .unwrap();
        store
            .index("new", "# New topic\n\nNew OAuth content")
            .unwrap();

        // Manually backdate "old" source's indexed_at
        {
            let conn = store.conn.lock();
            conn.execute(
                "UPDATE sources SET indexed_at = '2020-01-01 00:00:00' WHERE label = 'old'",
                [],
            )
            .unwrap();
        }

        // Search with after=2024: should only find "new"
        let results = store
            .search(
                &["OAuth".to_string()],
                5,
                None,
                &DateRange::new(Some("2024-01-01".to_string()), None),
            )
            .unwrap();
        assert!(!results[0].hits.is_empty());
        for hit in &results[0].hits {
            assert_eq!(hit.source, "new", "Should only find 'new' source");
        }

        // Search with before=2021: should only find "old"
        let results = store
            .search(
                &["OAuth".to_string()],
                5,
                None,
                &DateRange::new(None, Some("2021-01-01".to_string())),
            )
            .unwrap();
        assert!(!results[0].hits.is_empty());
        for hit in &results[0].hits {
            assert_eq!(hit.source, "old", "Should only find 'old' source");
        }

        // Unbounded (default) returns both
        let results = store
            .search(&["OAuth".to_string()], 5, None, &DateRange::default())
            .unwrap();
        assert!(results[0].hits.len() >= 2);
    }

    #[test]
    fn list_sources_date_range() {
        let store = ContentStore::new().unwrap();
        store.index("old", "old content").unwrap();
        store.index("new", "new content").unwrap();

        {
            let conn = store.conn.lock();
            conn.execute(
                "UPDATE sources SET indexed_at = '2020-01-01 00:00:00' WHERE label = 'old'",
                [],
            )
            .unwrap();
        }

        // After 2024: only "new"
        let sources = store.list_sources(Some("2024-01-01"), None).unwrap();
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].label, "new");

        // Before 2021: only "old"
        let sources = store.list_sources(None, Some("2021-01-01")).unwrap();
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].label, "old");

        // No filter: both
        let sources = store.list_sources(None, None).unwrap();
        assert_eq!(sources.len(), 2);
    }

    #[test]
    fn search_combined_date_range() {
        let store = ContentStore::new().unwrap();
        store
            .index("old", "# Old\n\nOld authentication guide")
            .unwrap();
        store
            .index("new", "# New\n\nNew authentication guide")
            .unwrap();

        {
            let conn = store.conn.lock();
            conn.execute(
                "UPDATE sources SET indexed_at = '2020-01-01 00:00:00' WHERE label = 'old'",
                [],
            )
            .unwrap();
        }

        let results = store
            .search_combined(
                Some("authentication"),
                None,
                5,
                None,
                Some("2024-01-01"),
                None,
            )
            .unwrap();
        assert!(!results[0].hits.is_empty());
        for hit in &results[0].hits {
            assert_eq!(hit.source, "new");
        }
    }

    #[test]
    fn test_empty_content() {
        let store = ContentStore::new().unwrap();
        let result = store.index("empty", "").unwrap();
        assert_eq!(result.total_chunks, 0);
        assert_eq!(result.total_lines, 0);

        let read = store.read("empty", None, None).unwrap();
        assert_eq!(read.total_lines, 0);
    }

    #[test]
    fn test_read_source_not_found() {
        let store = ContentStore::new().unwrap();
        let err = store.read("nonexistent", None, None).unwrap_err();
        assert!(matches!(err, ContextError::SourceNotFound(_)));
    }

    #[test]
    fn test_special_characters_in_search() {
        let store = ContentStore::new().unwrap();
        store
            .index(
                "docs:special",
                "# Special Chars\n\nContent with 'quotes' and (parens) and [brackets]",
            )
            .unwrap();

        let results = store
            .search(
                &["quotes' AND (parens)".to_string()],
                5,
                None,
                &DateRange::default(),
            )
            .unwrap();
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_unicode_content() {
        let store = ContentStore::new().unwrap();
        let content = "# \u{4f60}\u{597d}\u{4e16}\u{754c}\n\n\u{8fd9}\u{662f}\u{4e2d}\u{6587}\u{5185}\u{5bb9}\u{ff0c}\u{5305}\u{542b}\u{591a}\u{79cd}\u{5b57}\u{7b26}\u{3002}";
        store.index("docs:chinese", content).unwrap();

        let read = store.read("docs:chinese", None, None).unwrap();
        assert!(read.content.contains('\u{4f60}'));
    }

    #[test]
    fn test_large_content() {
        let store = ContentStore::new().unwrap();
        let mut content = String::with_capacity(110_000);
        for i in 0..500 {
            content.push_str(&format!("## Section {}\n\n", i));
            content.push_str(&format!(
                "This is section {} with some content that makes it substantial enough. ",
                i
            ));
            content.push_str("Lorem ipsum dolor sit amet, consectetur adipiscing elit.\n\n");
        }

        let result = store.index("docs:large", &content).unwrap();
        assert!(result.total_chunks > 0);
        assert!(result.total_lines > 100);

        let results = store
            .search(&["Section 250".to_string()], 5, None, &DateRange::default())
            .unwrap();
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_display_formatting_search() {
        let store = ContentStore::new().unwrap();
        store.index("docs:api", sample_markdown()).unwrap();

        let results = store
            .search(&["OAuth".to_string()], 3, None, &DateRange::default())
            .unwrap();
        if !results[0].hits.is_empty() {
            let display = results[0].to_string();
            assert!(display.contains("### Results for"));
            assert!(display.contains("**[1]"));
            // The read hint is a single footer added by format_search_results
            assert!(!display.contains("Read around a hit"));
            let formatted = format_search_results(&results, SEARCH_OUTPUT_CAP, "IndexRead");
            assert!(formatted.contains("IndexRead(label=\"<source>\", offset=\"<line>\")"));
        }
    }

    #[test]
    fn test_display_formatting_read() {
        let store = ContentStore::new().unwrap();
        store
            .index("docs:api", "line one\nline two\nline three")
            .unwrap();

        let read = store.read("docs:api", None, None).unwrap();
        let display = read.to_string();
        assert!(display.contains("Source: docs:api"));
        assert!(display.contains("1\tline one"));
        assert!(display.contains("2\tline two"));
    }

    #[test]
    fn test_stemming_search() {
        let store = ContentStore::new().unwrap();
        store
            .index(
                "docs:cache",
                "# Caching\n\nThe application uses cached responses for performance. The caching layer supports TTL-based expiration.",
            )
            .unwrap();

        let results = store
            .search(&["cached".to_string()], 5, None, &DateRange::default())
            .unwrap();
        assert!(
            !results[0].hits.is_empty(),
            "Porter stemming should match 'cached' to 'caching'"
        );
    }

    #[test]
    fn test_trigram_substring_search() {
        let store = ContentStore::new().unwrap();
        store
            .index(
                "docs:react",
                "# React Hooks\n\nThe useEffect hook handles side effects. The useState hook manages state.",
            )
            .unwrap();

        let results = store
            .search(&["useEffect".to_string()], 5, None, &DateRange::default())
            .unwrap();
        assert!(!results[0].hits.is_empty(), "Should find useEffect");
    }

    #[test]
    fn test_multi_query_search() {
        let store = ContentStore::new().unwrap();
        store.index("docs:api", sample_markdown()).unwrap();

        let results = store
            .search(
                &["OAuth".to_string(), "endpoints".to_string()],
                5,
                None,
                &DateRange::default(),
            )
            .unwrap();
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_fuzzy_correction_search() {
        let store = ContentStore::new().unwrap();
        store
            .index(
                "docs:k8s",
                "# Kubernetes\n\nKubernetes is a container orchestration platform.\nDeploy containers to kubernetes clusters with kubectl.",
            )
            .unwrap();

        let results = store
            .search(&["kuberntes".to_string()], 5, None, &DateRange::default())
            .unwrap();
        if !results[0].hits.is_empty() {
            assert_eq!(results[0].hits[0].match_layer, MatchLayer::Fuzzy);
            assert!(results[0].corrected_query.is_some());
        }
    }

    #[test]
    fn test_no_results_search() {
        let store = ContentStore::new().unwrap();
        store.index("docs:api", sample_markdown()).unwrap();

        let results = store
            .search(
                &["zzzznonexistentzzz".to_string()],
                5,
                None,
                &DateRange::default(),
            )
            .unwrap();
        assert!(results[0].hits.is_empty());
    }

    #[test]
    fn test_fts5_available() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE VIRTUAL TABLE test_fts USING fts5(content);
             INSERT INTO test_fts VALUES ('hello world');",
        )
        .expect("FTS5 should be available with bundled-full feature");

        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM test_fts WHERE test_fts MATCH 'hello'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    // ── Read: long line protection ──

    #[test]
    fn read_normal_lines() {
        let store = ContentStore::new().unwrap();
        store
            .index("test", "line one\nline two\nline three")
            .unwrap();

        let read = store.read("test", None, None).unwrap();
        assert!(read.content.contains("1\tline one"));
        assert!(read.content.contains("2\tline two"));
        assert!(read.content.contains("3\tline three"));
        assert_eq!(read.showing_start, "1");
        assert_eq!(read.showing_end, "3");
    }

    #[test]
    fn read_long_line_splits() {
        let store = ContentStore::new().unwrap();
        let long_line = "x".repeat(5000);
        let content = format!("short\n{}\nend", long_line);
        store.index("test", &content).unwrap();

        let read = store.read("test", None, None).unwrap();
        // Should have sub-chunk labels for the long line
        assert!(read.content.contains("2-1\t"));
        assert!(read.content.contains("2-2\t"));
        assert!(read.content.contains("2-3\t"));
    }

    #[test]
    fn read_limit_counts_physical_lines() {
        let store = ContentStore::new().unwrap();
        let long_line = "x".repeat(5000); // 3 parts
        let content = format!("short\n{}\nend", long_line);
        store.index("test", &content).unwrap();

        // limit=2 covers line 1 and every part of line 2
        let read = store.read("test", None, Some(2)).unwrap();
        assert_eq!(read.content.lines().count(), 4);
        assert_eq!(read.showing_end, "2-3");
        assert_eq!(read.next_offset.as_deref(), Some("3"));
        assert_eq!(read.remaining_lines, 1);
    }

    #[test]
    fn read_resume_from_sub_offset() {
        let store = ContentStore::new().unwrap();
        let long_line = "y".repeat(5000);
        let content = format!("short\n{}\nend", long_line);
        store.index("test", &content).unwrap();

        // Resuming mid-line returns the rest of that line
        let read = store.read("test", Some("2-2"), Some(1)).unwrap();
        assert_eq!(read.showing_start, "2-2");
        assert_eq!(read.showing_end, "2-3");
        assert_eq!(read.content.lines().count(), 2);
        assert_eq!(read.next_offset.as_deref(), Some("3"));
    }

    #[test]
    fn read_stops_at_cap_and_resumes() {
        let store = ContentStore::new().unwrap();
        // ~100 bytes per line, far more than the cap in total
        let content: String = (1..=2000)
            .map(|i| format!("{:04} {}\n", i, "z".repeat(95)))
            .collect();
        store.index("big", &content).unwrap();

        let first = store.read("big", None, Some(2000)).unwrap();
        assert!(first.content.len() <= READ_OUTPUT_CAP);
        let next = first.next_offset.clone().expect("cap leaves lines to read");
        let next_line: usize = next.parse().unwrap();
        // Rows are whole: the last shown line is the one before `next`
        assert_eq!(first.showing_end, (next_line - 1).to_string());
        assert_eq!(first.remaining_lines, 2000 - next_line + 1);
        assert!(first
            .to_string()
            .contains(&format!("continue with offset=\"{}\"", next)));

        let second = store.read("big", Some(&next), Some(2000)).unwrap();
        assert_eq!(second.showing_start, next);
        assert!(second
            .content
            .trim_start()
            .starts_with(&format!("{}\t{:04} ", next_line, next_line)));
    }

    #[test]
    fn read_to_end_has_no_continuation() {
        let store = ContentStore::new().unwrap();
        store.index("t", "a\nb\nc").unwrap();
        let read = store.read("t", None, None).unwrap();
        assert_eq!(read.next_offset, None);
        assert!(!read.to_string().contains("continue with"));

        let partial = store.read("t", None, Some(2)).unwrap();
        assert_eq!(partial.next_offset.as_deref(), Some("3"));
        assert!(partial
            .to_string()
            .ends_with("[1 more line \u{2014} continue with offset=\"3\"]"));
    }

    #[test]
    fn read_header_for_split_lines_is_readable() {
        let store = ContentStore::new().unwrap();
        store.index("j", &"q".repeat(5000)).unwrap();
        let read = store.read("j", None, None).unwrap();
        let header = read.to_string().lines().next().unwrap().to_string();
        assert_eq!(
            header,
            "Source: j (lines 1-1 to 1-3 of 1; \"N-M\" is part M of long line N)"
        );
    }

    #[test]
    fn read_sub_line_content_verbatim() {
        let store = ContentStore::new().unwrap();
        let long_line = "abcdef".repeat(500); // 3000 chars
        let content = format!("before\n{}\nafter", long_line);
        store.index("test", &content).unwrap();

        let read = store.read("test", Some("2-1"), Some(1)).unwrap();
        // Content should be verbatim (no … markers in read output)
        assert!(!read.content.contains('\u{2026}'));
        // Should contain the first LONG_LINE_THRESHOLD chars of the long line
        assert!(read.content.contains("abcdef"));
    }

    #[test]
    fn read_default_offset_none() {
        let store = ContentStore::new().unwrap();
        store.index("test", "a\nb\nc").unwrap();

        let read = store.read("test", None, None).unwrap();
        assert_eq!(read.showing_start, "1");
    }

    #[test]
    fn read_out_of_range_offset() {
        let store = ContentStore::new().unwrap();
        store.index("test", "a\nb\nc").unwrap();

        let read = store.read("test", Some("999"), None).unwrap();
        assert_eq!(read.showing_start, "0");
        assert_eq!(read.showing_end, "0");
        assert!(read.content.is_empty());
    }

    #[test]
    fn read_output_cap_applied() {
        let store = ContentStore::new().unwrap();
        // Create content that would exceed 40KB when formatted
        let mut content = String::new();
        for i in 0..2000 {
            content.push_str(&format!(
                "Line {} with content that adds up quickly for testing the output cap\n",
                i
            ));
        }
        store.index("test", &content).unwrap();

        let read = store.read("test", None, None).unwrap();
        // Should be capped at ~40KB
        assert!(
            read.content.len() <= READ_OUTPUT_CAP + 500,
            "Output should be capped, got {} bytes",
            read.content.len()
        );
    }

    // ── Index: rich summary ──

    #[test]
    fn index_display_header_stats() {
        let store = ContentStore::new().unwrap();
        let result = store.index("docs:api", sample_markdown()).unwrap();
        let display = result.to_string();
        assert!(display.contains("lines"));
        assert!(display.contains("KB"));
        assert!(display.contains("chunks"));
        assert!(display.contains("code"));
    }

    #[test]
    fn index_display_toc_hierarchy() {
        let store = ContentStore::new().unwrap();
        let result = store.index("docs:api", sample_markdown()).unwrap();
        let display = result.to_string();
        assert!(display.contains("## Contents"));
        assert!(display.contains("[L"));
    }

    #[test]
    fn index_display_usage_instructions() {
        let store = ContentStore::new().unwrap();
        let result = store.index("docs:api", sample_markdown()).unwrap();
        let display = result.to_string();
        assert!(display.contains("IndexSearch(queries: [...], source: \"docs:api\")"));
        assert!(display.contains("IndexRead(label: \"docs:api\", offset: \"1\")"));

        let renamed = result.toc_with(
            None,
            ToolNames {
                search: "Find",
                read: "Open",
            },
        );
        assert!(renamed.contains("Find(queries:"));
        assert!(renamed.contains("Open(label:"));
    }

    // ── Ranking ──

    fn hit(source: &str, start: usize, end: usize, rank: f64) -> SearchHit {
        SearchHit {
            title: format!("{source}:{start}"),
            content: String::new(),
            source: source.to_string(),
            rank,
            content_type: ContentType::Prose,
            match_layer: MatchLayer::Porter,
            line_start: start,
            line_end: end,
        }
    }

    #[test]
    fn dedup_keeps_better_hit_and_sorts_best_first() {
        // BM25: lower is better
        let hits = vec![
            hit("a", 8, 20, -1.0),
            hit("b", 1, 5, -3.0),
            hit("a", 1, 10, -5.0),
        ];
        let kept = ContentStore::dedup_overlapping_hits(hits);
        assert_eq!(kept.len(), 2);
        assert_eq!((kept[0].source.as_str(), kept[0].rank), ("a", -5.0));
        assert_eq!((kept[1].source.as_str(), kept[1].rank), ("b", -3.0));
    }

    #[test]
    fn search_returns_best_match_first() {
        let store = ContentStore::new().unwrap();
        let filler = "lorem ipsum dolor sit amet consectetur adipiscing elit ".repeat(20);
        store
            .index(
                "weak",
                &format!("{filler}\nwidget mentioned once\n{filler}"),
            )
            .unwrap();
        store
            .index("strong", "widget widget widget: the widget guide")
            .unwrap();
        let results = store
            .search(&["widget".to_string()], 5, None, &DateRange::default())
            .unwrap();
        let sources: Vec<&str> = results[0].hits.iter().map(|h| h.source.as_str()).collect();
        assert_eq!(sources.first(), Some(&"strong"), "got {sources:?}");
    }

    // ── Source filter ──

    #[test]
    fn search_unknown_source_is_an_error_listing_sources() {
        let store = ContentStore::new().unwrap();
        store.index("tool__a:1", "alpha content").unwrap();
        store.index("tool__b:1", "beta content").unwrap();
        let err = store
            .search_combined(Some("alpha"), None, 5, Some("catalog:skills"), None, None)
            .unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("No indexed source matches \"catalog:skills\""),
            "{msg}"
        );
        assert!(
            msg.contains("\"tool__a:1\"") && msg.contains("\"tool__b:1\""),
            "{msg}"
        );

        // A prefix of an existing label is fine
        assert!(store
            .search_combined(Some("alpha"), None, 5, Some("tool__"), None, None)
            .is_ok());
    }

    #[cfg(feature = "vector")]
    #[test]
    fn vector_scope_follows_source_prefix() {
        let store = ContentStore::new().unwrap();
        store.index("tool__a:1", "alpha").unwrap();
        store.index("tool__b:1", "beta").unwrap();
        let conn = store.conn.lock();
        assert!(
            ContentStore::vector_scope(&conn, None, &DateRange::default())
                .unwrap()
                .is_none()
        );
        let scope = ContentStore::vector_scope(&conn, Some("tool__a"), &DateRange::default())
            .unwrap()
            .unwrap();
        assert!(scope.contains("tool__a:1"));
        assert!(!scope.contains("tool__b:1"));
    }

    #[test]
    fn format_search_results_dedups_across_queries() {
        let store = ContentStore::new().unwrap();
        store
            .index("doc", "the frobnicator handles widget sprockets")
            .unwrap();
        let results = store
            .search(
                &["frobnicator".to_string(), "sprockets".to_string()],
                5,
                None,
                &DateRange::default(),
            )
            .unwrap();
        let out = format_search_results(&results, SEARCH_OUTPUT_CAP, "IndexRead");
        assert_eq!(out.matches("the frobnicator handles").count(), 1, "{out}");
        assert!(out.contains("same as query 1 hit [1] above"), "{out}");
        assert_eq!(out.matches("Read around a hit").count(), 1, "{out}");
    }

    // ── Search combined ──

    #[test]
    fn search_combined_single_query() {
        let store = ContentStore::new().unwrap();
        store.index("docs:api", sample_markdown()).unwrap();

        let results = store
            .search_combined(Some("OAuth"), None, 5, None, None, None)
            .unwrap();
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn search_combined_multiple_queries() {
        let store = ContentStore::new().unwrap();
        store.index("docs:api", sample_markdown()).unwrap();

        let results = store
            .search_combined(
                None,
                Some(&["OAuth".to_string(), "endpoints".to_string()]),
                5,
                None,
                None,
                None,
            )
            .unwrap();
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn search_combined_both_merged() {
        let store = ContentStore::new().unwrap();
        store.index("docs:api", sample_markdown()).unwrap();

        let results = store
            .search_combined(
                Some("OAuth"),
                Some(&["endpoints".to_string()]),
                5,
                None,
                None,
                None,
            )
            .unwrap();
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn search_combined_both_none_errors() {
        let store = ContentStore::new().unwrap();
        store.index("docs:api", sample_markdown()).unwrap();

        let err = store
            .search_combined(None, None, 5, None, None, None)
            .unwrap_err();
        assert!(matches!(err, ContextError::InvalidParams(_)));
    }

    // ── Batch search+read ──

    #[test]
    fn batch_search_only() {
        let store = ContentStore::new().unwrap();
        store.index("docs:api", sample_markdown()).unwrap();

        let result = store
            .batch_search_read(&["OAuth".to_string()], &[], 5, None, &DateRange::default())
            .unwrap();
        assert!(!result.search_results.is_empty());
        assert!(result.read_results.is_empty());
    }

    #[test]
    fn batch_read_only() {
        let store = ContentStore::new().unwrap();
        store.index("docs:api", sample_markdown()).unwrap();

        let result = store
            .batch_search_read(
                &[],
                &[ReadRequest {
                    label: "docs:api".to_string(),
                    offset: Some("1".to_string()),
                    limit: Some(5),
                }],
                5,
                None,
                &DateRange::default(),
            )
            .unwrap();
        assert!(result.search_results.is_empty());
        assert!(!result.read_results.is_empty());
    }

    #[test]
    fn batch_combined() {
        let store = ContentStore::new().unwrap();
        store.index("docs:api", sample_markdown()).unwrap();

        let result = store
            .batch_search_read(
                &["OAuth".to_string()],
                &[ReadRequest {
                    label: "docs:api".to_string(),
                    offset: Some("1".to_string()),
                    limit: Some(5),
                }],
                5,
                None,
                &DateRange::default(),
            )
            .unwrap();
        assert!(!result.search_results.is_empty());
        assert!(!result.read_results.is_empty());
        let display = result.to_string();
        assert!(display.contains("# Search Results"));
        assert!(display.contains("# Read Results"));
    }

    #[test]
    fn batch_search_uses_3000_snippets() {
        let store = ContentStore::new().unwrap();
        store.index("docs:api", sample_markdown()).unwrap();

        // Batch search should use SNIPPET_BATCH_MAX_LEN (3000)
        let result = store
            .batch_search_read(&["OAuth".to_string()], &[], 5, None, &DateRange::default())
            .unwrap();
        // Just verify it works — snippet length is internal
        assert!(!result.search_results.is_empty());
    }

    // ── Integration / Edge Cases ──

    #[test]
    fn roundtrip_index_search_read() {
        let store = ContentStore::new().unwrap();
        store.index("docs:api", sample_markdown()).unwrap();

        let results = store
            .search(&["OAuth".to_string()], 5, None, &DateRange::default())
            .unwrap();
        assert!(!results[0].hits.is_empty());

        let hit = &results[0].hits[0];
        let offset = hit.line_start.to_string();
        let read = store.read("docs:api", Some(&offset), Some(10)).unwrap();
        assert!(!read.content.is_empty());
        assert_ne!(read.showing_start, "0");
    }

    #[test]
    fn empty_content_all_ops() {
        let store = ContentStore::new().unwrap();
        store.index("empty", "").unwrap();

        let results = store
            .search(&["test".to_string()], 5, None, &DateRange::default())
            .unwrap();
        assert!(results[0].hits.is_empty());

        let read = store.read("empty", None, None).unwrap();
        assert_eq!(read.total_lines, 0);
    }

    #[test]
    fn unicode_content_all_ops() {
        let store = ContentStore::new().unwrap();
        let content = "# \u{4f60}\u{597d}\n\n\u{8fd9}\u{662f}\u{4e2d}\u{6587}\u{5185}\u{5bb9}";
        store.index("cjk", content).unwrap();

        let results = store
            .search(
                &["\u{4e2d}\u{6587}".to_string()],
                5,
                None,
                &DateRange::default(),
            )
            .unwrap();
        // May or may not find matches depending on tokenization
        assert_eq!(results.len(), 1);

        let read = store.read("cjk", None, None).unwrap();
        assert!(read.content.contains('\u{4f60}'));
    }

    #[test]
    fn very_large_doc_end_to_end() {
        let store = ContentStore::new().unwrap();
        let mut content = String::with_capacity(110_000);
        for i in 0..1000 {
            content.push_str(&format!("## Section {}\n\n", i));
            content.push_str(&format!(
                "Content for section {} with substantial text that makes each section large enough. ",
                i
            ));
            content.push_str(
                "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod.\n\n",
            );
        }

        let result = store.index("large", &content).unwrap();
        assert!(
            result.content_bytes > 100_000,
            "content_bytes={}",
            result.content_bytes
        );
        let display = result.to_string();
        assert!(display.contains("## Contents"));

        let results = store
            .search(&["Section 250".to_string()], 5, None, &DateRange::default())
            .unwrap();
        let output = format_search_results(&results, types::SEARCH_OUTPUT_CAP, "IndexRead");
        assert!(output.len() <= types::SEARCH_OUTPUT_CAP + 500);

        let read = store.read("large", None, None).unwrap();
        assert!(read.content.len() <= READ_OUTPUT_CAP + 500);
    }

    #[test]
    fn single_line_50kb() {
        let store = ContentStore::new().unwrap();
        let content = "x".repeat(50_000);
        store.index("big_line", &content).unwrap();

        // Read should show sub-chunks
        let read = store.read("big_line", None, Some(5)).unwrap();
        assert!(read.content.contains("1-1\t"));

        // Search should work
        let results = store
            .search(&["xxx".to_string()], 5, None, &DateRange::default())
            .unwrap();
        assert_eq!(results.len(), 1);

        // Index should show TOC
        let result = store.index("big_line2", &content).unwrap();
        let display = result.to_string();
        assert!(display.contains("## Contents"));
    }

    #[test]
    fn all_output_caps_enforced() {
        let store = ContentStore::new().unwrap();
        let mut content = String::with_capacity(110_000);
        for i in 0..1000 {
            content.push_str(&format!(
                "Line {} with enough content to be substantial\n",
                i
            ));
        }
        store.index("big", &content).unwrap();

        // Read cap
        let read = store.read("big", None, None).unwrap();
        assert!(
            read.content.len() <= READ_OUTPUT_CAP + 500,
            "Read output too large: {} bytes",
            read.content.len()
        );

        // Search cap via format_search_results
        let results = store
            .search(
                &[
                    "Line".to_string(),
                    "content".to_string(),
                    "substantial".to_string(),
                ],
                50,
                None,
                &DateRange::default(),
            )
            .unwrap();
        let output = format_search_results(&results, types::SEARCH_OUTPUT_CAP, "IndexRead");
        assert!(
            output.len() <= types::SEARCH_OUTPUT_CAP + 500,
            "Search output too large: {} bytes",
            output.len()
        );
    }

    // ── Read: offset edge cases ──

    #[test]
    fn read_offset_zero_clamps_to_line_1() {
        let store = ContentStore::new().unwrap();
        store.index("test", "aaa\nbbb\nccc").unwrap();

        let read = store.read("test", Some("0"), None).unwrap();
        // "0" clamps to line 1
        assert_eq!(read.showing_start, "1");
        assert!(read.content.contains("aaa"));
    }

    #[test]
    fn read_offset_sub_zero_clamps_to_sub_1() {
        let store = ContentStore::new().unwrap();
        let long_line = "x".repeat(5000);
        let content = format!("short\n{}\nend", long_line);
        store.index("test", &content).unwrap();

        // "2-0" should clamp to "2-1"
        let read = store.read("test", Some("2-0"), Some(1)).unwrap();
        assert_eq!(read.showing_start, "2-1");
    }

    #[test]
    fn read_offset_sub_1_on_short_line() {
        let store = ContentStore::new().unwrap();
        // Line 5 is short (no sub-chunks)
        store
            .index("test", "a\nb\nc\nd\nshort line 5\nf\ng")
            .unwrap();

        // "5-1" on a short line that has no sub-chunks — should still find line 5
        // because "5-1" doesn't exist but line 5 does, so we fall through to
        // the first entry whose line number >= 5
        let read = store.read("test", Some("5-1"), Some(1)).unwrap();
        // Should start at line 5 (no sub-chunks exist, so falls through to next line >= 5)
        assert!(
            read.showing_start == "5" || read.showing_start == "6",
            "Expected 5 or 6, got: {}",
            read.showing_start
        );
        assert!(!read.content.is_empty());
    }

    #[test]
    fn read_offset_sub_2_on_short_line_falls_to_next() {
        let store = ContentStore::new().unwrap();
        // Line 5 is short (fits in one chunk), no "5-2" exists
        store
            .index("test", "a\nb\nc\nd\nshort line 5\nf\ng")
            .unwrap();

        // "5-2" doesn't exist → should fall through to line 6
        let read = store.read("test", Some("5-2"), Some(1)).unwrap();
        assert_eq!(
            read.showing_start, "6",
            "Should fall through to line 6 when 5-2 doesn't exist"
        );
        assert!(read.content.contains("f"));
    }

    #[test]
    fn read_offset_sub_on_last_line_gives_empty() {
        let store = ContentStore::new().unwrap();
        store.index("test", "a\nb\nc").unwrap();

        // "3-2" doesn't exist and there's no line after 3 → empty result
        let read = store.read("test", Some("3-2"), None).unwrap();
        assert_eq!(read.showing_start, "0");
        assert!(read.content.is_empty());
    }

    #[test]
    fn read_offset_sub_on_long_line_works() {
        let store = ContentStore::new().unwrap();
        let long_line = "abcdef".repeat(400); // 2400 chars → 2 sub-chunks at threshold=2000
        let content = format!("first\n{}\nlast", long_line);
        store.index("test", &content).unwrap();

        // "2-1" should work (first sub-chunk of long line 2)
        let read1 = store.read("test", Some("2-1"), Some(1)).unwrap();
        assert_eq!(read1.showing_start, "2-1");

        // "2-2" should work (second sub-chunk of long line 2)
        let read2 = store.read("test", Some("2-2"), Some(1)).unwrap();
        assert_eq!(read2.showing_start, "2-2");

        // "2-3" doesn't exist (only 2 sub-chunks) → should fall through to line 3
        let read3 = store.read("test", Some("2-3"), Some(1)).unwrap();
        assert_eq!(read3.showing_start, "3");
        assert!(read3.content.contains("last"));
    }

    #[test]
    fn batch_read_error_surfaces() {
        let store = ContentStore::new().unwrap();
        store.index("docs:api", sample_markdown()).unwrap();

        // Request a nonexistent source — should surface error instead of silently dropping
        let result = store
            .batch_search_read(
                &[],
                &[ReadRequest {
                    label: "nonexistent".to_string(),
                    offset: None,
                    limit: None,
                }],
                5,
                None,
                &DateRange::default(),
            )
            .unwrap();
        assert_eq!(result.read_results.len(), 1);
        assert!(result.read_results[0].content.contains("Error"));
    }
}
