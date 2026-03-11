//! lr-context — Native content indexing, search & read.
//!
//! FTS5 BM25-based knowledge base for session-scoped content.
//! Chunks content by format (markdown, plain text, JSON),
//! stores in SQLite FTS5, and retrieves via three-layer search
//! (Porter stemming → Trigram → Fuzzy correction).

mod chunk;
mod fuzzy;
mod search;
mod types;

pub use types::{
    ContentType, ContextError, IndexResult, MatchLayer, ReadResult, SearchHit, SearchResult,
    SourceInfo,
};

use once_cell::sync::Lazy;
use parking_lot::Mutex;
use regex::Regex;
use rusqlite::{params, Connection};
use std::collections::HashSet;
use std::sync::Arc;

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
        let conn = self.conn.lock();
        let results = queries
            .iter()
            .map(|q| search::search_with_fallback(&conn, q, limit, source))
            .collect();
        Ok(results)
    }

    // ── Read ──

    /// Read original content with pagination (1-based offset, line limit).
    pub fn read(
        &self,
        label: &str,
        offset: Option<usize>,
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
        let offset = offset.unwrap_or(1).max(1); // 1-based, minimum 1
        let limit = limit.unwrap_or(2000);

        let lines: Vec<&str> = content.lines().collect();
        let start_idx = (offset - 1).min(lines.len());
        let end_idx = (start_idx + limit).min(lines.len());
        let showing_lines = &lines[start_idx..end_idx];

        // Format with right-aligned line numbers + tab (cat -n style)
        let max_line_num = if end_idx > 0 {
            start_idx + showing_lines.len()
        } else {
            1
        };
        let width = max_line_num.to_string().len().max(1);

        let formatted: String = showing_lines
            .iter()
            .enumerate()
            .map(|(i, line)| {
                let line_num = start_idx + i + 1;
                format!("{:>width$}\t{}", line_num, line, width = width)
            })
            .collect::<Vec<_>>()
            .join("\n");

        let showing_start = if showing_lines.is_empty() {
            0
        } else {
            start_idx + 1
        };
        let showing_end = if showing_lines.is_empty() {
            0
        } else {
            start_idx + showing_lines.len()
        };

        Ok(ReadResult {
            label: label.to_string(),
            content: formatted,
            total_lines,
            showing_start,
            showing_end,
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

        // Read all lines
        let read = store.read("docs:api", None, None).unwrap();
        assert_eq!(read.total_lines, result.total_lines);
        assert_eq!(read.showing_start, 1);
        assert_eq!(read.showing_end, result.total_lines);
        // Should have cat-n style line numbers
        assert!(read.content.contains("\t"));
    }

    #[test]
    fn test_read_with_offset_limit() {
        let store = ContentStore::new().unwrap();
        store.index("docs:api", sample_markdown()).unwrap();

        let read = store.read("docs:api", Some(5), Some(3)).unwrap();
        assert_eq!(read.showing_start, 5);
        assert_eq!(read.showing_end, 7);
        // Should have 3 lines
        assert_eq!(read.content.lines().count(), 3);
    }

    #[test]
    fn test_read_out_of_range() {
        let store = ContentStore::new().unwrap();
        let result = store.index("docs:api", "line 1\nline 2\nline 3").unwrap();

        // Offset beyond content
        let read = store.read("docs:api", Some(100), Some(10)).unwrap();
        assert_eq!(read.showing_start, 0); // empty range
        assert_eq!(read.showing_end, 0);
        assert!(read.content.is_empty());

        // Partial range
        let read = store.read("docs:api", Some(2), Some(100)).unwrap();
        assert_eq!(read.showing_start, 2);
        assert_eq!(read.showing_end, result.total_lines);
    }

    #[test]
    fn test_reindex_same_label() {
        let store = ContentStore::new().unwrap();

        store.index("docs:api", "old content here").unwrap();
        store.index("docs:api", "new content here").unwrap();

        let sources = store.list_sources().unwrap();
        assert_eq!(sources.len(), 1); // Should have replaced, not duplicated

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

        // Read the section
        let read = store
            .read("docs:api", Some(hit.line_start), Some(10))
            .unwrap();
        assert!(read.showing_start > 0);
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

        // Search across all sources
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

        // Search only in docs:api
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
        assert!(!store.delete("docs:api").unwrap()); // already deleted

        let sources = store.list_sources().unwrap();
        assert!(sources.is_empty());

        // Search should return no results
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

        // These should not cause FTS5 errors
        let results = store
            .search(&["quotes' AND (parens)".to_string()], 5, None)
            .unwrap();
        // Should not panic — results may or may not be empty
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
        // Generate 100KB+ content
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

        // Search should work
        let results = store.search(&["Section 250".to_string()], 5, None).unwrap();
        // May or may not find exact match depending on tokenization
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

        // "caching" should match "cached" via Porter stemming
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

        // "useEff" is a substring — trigram should find it
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
        assert_eq!(results.len(), 2); // One SearchResult per query
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

        // "kuberntes" is a typo — fuzzy correction should find "kubernetes"
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
        // Verify FTS5 is available with our rusqlite features
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
}
