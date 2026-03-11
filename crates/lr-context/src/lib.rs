//! lr-context — Native content indexing, search & read.
//!
//! FTS5 BM25-based knowledge base for session-scoped content.
//! Chunks content by format (markdown, plain text, JSON),
//! stores in SQLite FTS5, and retrieves via three-layer search
//! (Porter stemming → Trigram → Fuzzy correction).

mod chunk;
mod fuzzy;
mod search;
mod truncate;
mod types;

pub use types::format_search_results;
pub use types::{
    BatchResult, ChunkToc, ContentType, ContextError, IndexResult, MatchLayer, ReadRequest,
    ReadResult, SearchHit, SearchResult, SourceInfo, SEARCH_OUTPUT_CAP,
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
const READ_OUTPUT_CAP: usize = 40 * 1024;

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
}

impl ContentStore {
    /// Create a new in-memory content store.
    pub fn new() -> Result<Self, ContextError> {
        let conn = Connection::open_in_memory()?;
        Self::init_schema(&conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
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

    // ── Search ──

    /// Search across indexed content. Multiple queries, optional source filter.
    pub fn search(
        &self,
        queries: &[String],
        limit: usize,
        source: Option<&str>,
    ) -> Result<Vec<SearchResult>, ContextError> {
        self.search_internal(queries, limit, source, SNIPPET_MAX_LEN)
    }

    /// Search with combined query + queries entry point.
    pub fn search_combined(
        &self,
        query: Option<&str>,
        queries: Option<&[String]>,
        limit: usize,
        source: Option<&str>,
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
        self.search_internal(&all_queries, limit, source, SNIPPET_MAX_LEN)
    }

    fn search_internal(
        &self,
        queries: &[String],
        limit: usize,
        source: Option<&str>,
        max_snippet_len: usize,
    ) -> Result<Vec<SearchResult>, ContextError> {
        let conn = self.conn.lock();
        let results = queries
            .iter()
            .map(|q| search::search_with_fallback(&conn, q, limit, source, max_snippet_len))
            .collect();
        Ok(results)
    }

    // ── Read ──

    /// Read original content with pagination. Offset supports "5" or "5-2" (sub-line) format.
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

        let total_lines = total_lines as usize;
        let limit = limit.unwrap_or(2000);

        // Parse offset
        let parsed_offset = match offset {
            Some(s) => LineOffset::parse(s)?,
            None => LineOffset { line: 1, sub: None },
        };

        // Build virtual line list: split long lines into sub-chunks
        let lines: Vec<&str> = content.lines().collect();
        let mut virtual_lines: Vec<(String, &str)> = Vec::new(); // (label, text)

        for (i, line) in lines.iter().enumerate() {
            let line_num = i + 1; // 1-based
            let char_count = line.chars().count();

            if char_count > LONG_LINE_THRESHOLD {
                let chars: Vec<char> = line.chars().collect();
                let sub_count = char_count.div_ceil(LONG_LINE_THRESHOLD);
                for sub_idx in 0..sub_count {
                    let start = sub_idx * LONG_LINE_THRESHOLD;
                    let end = ((sub_idx + 1) * LONG_LINE_THRESHOLD).min(char_count);
                    let label = format!("{}-{}", line_num, sub_idx + 1);
                    // We need to convert char range to byte range for the slice
                    let byte_start: usize = chars[..start].iter().map(|c| c.len_utf8()).sum();
                    let byte_end: usize = chars[..end].iter().map(|c| c.len_utf8()).sum();
                    let text = &line[byte_start..byte_end];
                    virtual_lines.push((label, text));
                }
            } else {
                virtual_lines.push((format!("{}", line_num), line));
            }
        }

        // Find starting position matching parsed offset
        let start_pos = find_virtual_start(&virtual_lines, &parsed_offset);
        let end_pos = (start_pos + limit).min(virtual_lines.len());
        let showing = &virtual_lines[start_pos..end_pos];

        if showing.is_empty() {
            return Ok(ReadResult {
                label: label.to_string(),
                content: String::new(),
                total_lines,
                showing_start: "0".to_string(),
                showing_end: "0".to_string(),
            });
        }

        // Format with right-aligned labels + tab (cat -n style)
        let max_width = showing.iter().map(|(lbl, _)| lbl.len()).max().unwrap_or(1);

        let formatted: String = showing
            .iter()
            .map(|(lbl, text)| format!("{:>width$}\t{}", lbl, text, width = max_width))
            .collect::<Vec<_>>()
            .join("\n");

        let showing_start = showing[0].0.clone();
        let showing_end = showing[showing.len() - 1].0.clone();

        // Apply output cap
        let formatted = smart_truncate(&formatted, READ_OUTPUT_CAP);

        Ok(ReadResult {
            label: label.to_string(),
            content: formatted,
            total_lines,
            showing_start,
            showing_end,
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
    ) -> Result<BatchResult, ContextError> {
        let search_results = if queries.is_empty() {
            Vec::new()
        } else {
            self.search_internal(queries, search_limit, source, SNIPPET_BATCH_MAX_LEN)?
        };

        let read_results: Vec<ReadResult> = reads
            .iter()
            .filter_map(|r| self.read(&r.label, r.offset.as_deref(), r.limit).ok())
            .collect();

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
        Ok(deleted > 0)
    }

    // ── List sources ──

    /// List all indexed sources with metadata.
    pub fn list_sources(&self) -> Result<Vec<SourceInfo>, ContextError> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT label, total_lines, chunk_count, code_chunk_count FROM sources ORDER BY id DESC",
        )?;
        let sources = stmt
            .query_map([], |row| {
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
}

/// Find the starting position in the virtual line list for the given offset.
fn find_virtual_start(virtual_lines: &[(String, &str)], offset: &LineOffset) -> usize {
    let target = offset.to_display();
    // Find exact match first
    if let Some(pos) = virtual_lines.iter().position(|(lbl, _)| *lbl == target) {
        return pos;
    }
    // If offset has no sub, find the first entry for that line number
    if offset.sub.is_none() {
        let line_str = format!("{}", offset.line);
        let line_dash = format!("{}-", offset.line);
        if let Some(pos) = virtual_lines
            .iter()
            .position(|(lbl, _)| *lbl == line_str || lbl.starts_with(&line_dash))
        {
            return pos;
        }
    }
    // Fallback: past end
    virtual_lines.len()
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

        let sources = store.list_sources().unwrap();
        assert_eq!(sources.len(), 1);

        let read = store.read("docs:api", None, None).unwrap();
        assert!(read.content.contains("new content"));
    }

    #[test]
    fn test_index_search_read_workflow() {
        let store = ContentStore::new().unwrap();
        store.index("docs:api", sample_markdown()).unwrap();

        let results = store.search(&["OAuth flow".to_string()], 5, None).unwrap();
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

        let sources = store.list_sources().unwrap();
        assert_eq!(sources.len(), 3);

        let results = store
            .search(&["authentication".to_string()], 5, None)
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
            .search(&["authentication".to_string()], 5, Some("docs:api"))
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

        let sources = store.list_sources().unwrap();
        assert!(sources.is_empty());

        let results = store.search(&["API".to_string()], 5, None).unwrap();
        assert!(results[0].hits.is_empty());
    }

    #[test]
    fn test_list_sources() {
        let store = ContentStore::new().unwrap();
        store.index("src:main", "fn main() {}").unwrap();
        store.index("src:lib", "pub mod utils;").unwrap();

        let sources = store.list_sources().unwrap();
        assert_eq!(sources.len(), 2);
        assert!(sources.iter().any(|s| s.label == "src:main"));
        assert!(sources.iter().any(|s| s.label == "src:lib"));
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
            .search(&["quotes' AND (parens)".to_string()], 5, None)
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

        let results = store.search(&["Section 250".to_string()], 5, None).unwrap();
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_display_formatting_search() {
        let store = ContentStore::new().unwrap();
        store.index("docs:api", sample_markdown()).unwrap();

        let results = store.search(&["OAuth".to_string()], 3, None).unwrap();
        if !results[0].hits.is_empty() {
            let display = results[0].to_string();
            assert!(display.contains("### Results for"));
            assert!(display.contains("**[1]"));
            assert!(display.contains("read(source, offset, limit)"));
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

        let results = store.search(&["cached".to_string()], 5, None).unwrap();
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

        let results = store.search(&["useEffect".to_string()], 5, None).unwrap();
        assert!(!results[0].hits.is_empty(), "Should find useEffect");
    }

    #[test]
    fn test_multi_query_search() {
        let store = ContentStore::new().unwrap();
        store.index("docs:api", sample_markdown()).unwrap();

        let results = store
            .search(&["OAuth".to_string(), "endpoints".to_string()], 5, None)
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

        let results = store.search(&["kuberntes".to_string()], 5, None).unwrap();
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
            .search(&["zzzznonexistentzzz".to_string()], 5, None)
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
    fn read_sub_chunks_count_toward_limit() {
        let store = ContentStore::new().unwrap();
        let long_line = "x".repeat(5000);
        let content = format!("short\n{}\nend", long_line);
        store.index("test", &content).unwrap();

        let read = store.read("test", None, Some(3)).unwrap();
        // limit=3 should return 3 virtual entries
        assert_eq!(read.content.lines().count(), 3);
    }

    #[test]
    fn read_resume_from_sub_offset() {
        let store = ContentStore::new().unwrap();
        let long_line = "y".repeat(5000);
        let content = format!("short\n{}\nend", long_line);
        store.index("test", &content).unwrap();

        let read = store.read("test", Some("2-2"), Some(2)).unwrap();
        assert_eq!(read.showing_start, "2-2");
        assert_eq!(read.content.lines().count(), 2);
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
        assert!(display.contains("search(queries: [...])"));
        assert!(display.contains("read(source:"));
    }

    // ── Search combined ──

    #[test]
    fn search_combined_single_query() {
        let store = ContentStore::new().unwrap();
        store.index("docs:api", sample_markdown()).unwrap();

        let results = store.search_combined(Some("OAuth"), None, 5, None).unwrap();
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
            )
            .unwrap();
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn search_combined_both_merged() {
        let store = ContentStore::new().unwrap();
        store.index("docs:api", sample_markdown()).unwrap();

        let results = store
            .search_combined(Some("OAuth"), Some(&["endpoints".to_string()]), 5, None)
            .unwrap();
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn search_combined_both_none_errors() {
        let store = ContentStore::new().unwrap();
        store.index("docs:api", sample_markdown()).unwrap();

        let err = store.search_combined(None, None, 5, None).unwrap_err();
        assert!(matches!(err, ContextError::InvalidParams(_)));
    }

    // ── Batch search+read ──

    #[test]
    fn batch_search_only() {
        let store = ContentStore::new().unwrap();
        store.index("docs:api", sample_markdown()).unwrap();

        let result = store
            .batch_search_read(&["OAuth".to_string()], &[], 5, None)
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
            .batch_search_read(&["OAuth".to_string()], &[], 5, None)
            .unwrap();
        // Just verify it works — snippet length is internal
        assert!(!result.search_results.is_empty());
    }

    // ── Integration / Edge Cases ──

    #[test]
    fn roundtrip_index_search_read() {
        let store = ContentStore::new().unwrap();
        store.index("docs:api", sample_markdown()).unwrap();

        let results = store.search(&["OAuth".to_string()], 5, None).unwrap();
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

        let results = store.search(&["test".to_string()], 5, None).unwrap();
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
            .search(&["\u{4e2d}\u{6587}".to_string()], 5, None)
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

        let results = store.search(&["Section 250".to_string()], 5, None).unwrap();
        let output = format_search_results(&results, types::SEARCH_OUTPUT_CAP);
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
        let results = store.search(&["xxx".to_string()], 5, None).unwrap();
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
            )
            .unwrap();
        let output = format_search_results(&results, types::SEARCH_OUTPUT_CAP);
        assert!(
            output.len() <= types::SEARCH_OUTPUT_CAP + 500,
            "Search output too large: {} bytes",
            output.len()
        );
    }
}
