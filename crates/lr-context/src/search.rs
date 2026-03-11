use rusqlite::{params, Connection};

use crate::fuzzy;
use crate::types::{ContentType, MatchLayer, SearchHit, SearchResult};

const SNIPPET_MAX_CHARS: usize = 300;

// ─────────────────────────────────────────────────────────
// Query sanitization
// ─────────────────────────────────────────────────────────

#[derive(Clone, Copy)]
enum JoinMode {
    And,
    Or,
}

/// Sanitize a query for FTS5 Porter/unicode61 search.
fn sanitize_fts_query(query: &str, mode: JoinMode) -> String {
    let words: Vec<String> = query
        .replace(|c: char| "\"'(){}[]*:^~".contains(c), " ")
        .split_whitespace()
        .filter(|w| {
            !w.is_empty() && !matches!(w.to_uppercase().as_str(), "AND" | "OR" | "NOT" | "NEAR")
        })
        .map(|w| format!("\"{}\"", w))
        .collect();

    if words.is_empty() {
        return "\"\"".to_string();
    }

    let joiner = match mode {
        JoinMode::And => " ",
        JoinMode::Or => " OR ",
    };
    words.join(joiner)
}

/// Sanitize a query for FTS5 trigram search (words must be >= 3 chars).
fn sanitize_trigram_query(query: &str, mode: JoinMode) -> Option<String> {
    let cleaned = query.replace(|c: char| "\"'(){}[]*:^~".contains(c), "");
    let trimmed = cleaned.trim();
    if trimmed.len() < 3 {
        return None;
    }

    let words: Vec<String> = trimmed
        .split_whitespace()
        .filter(|w| w.len() >= 3)
        .map(|w| format!("\"{}\"", w))
        .collect();

    if words.is_empty() {
        return None;
    }

    let joiner = match mode {
        JoinMode::And => " ",
        JoinMode::Or => " OR ",
    };
    Some(words.join(joiner))
}

// ─────────────────────────────────────────────────────────
// Search layers
// ─────────────────────────────────────────────────────────

struct RawHit {
    title: String,
    content: String,
    content_type: String,
    label: String,
    rank: f64,
    highlighted: String,
    line_start: i64,
    line_end: i64,
}

fn search_porter(
    conn: &Connection,
    query: &str,
    limit: usize,
    source: Option<&str>,
    mode: JoinMode,
) -> Vec<RawHit> {
    let sanitized = sanitize_fts_query(query, mode);
    if sanitized == "\"\"" {
        return Vec::new();
    }

    let result = if let Some(src) = source {
        let filter = format!("{}%", src);
        let mut stmt = conn
            .prepare_cached(
                "SELECT c.title, c.content, c.content_type, s.label,
                        c.line_start, c.line_end,
                        bm25(chunks, 2.0, 1.0) AS rank,
                        highlight(chunks, 1, char(2), char(3)) AS highlighted
                 FROM chunks c
                 JOIN sources s ON s.id = CAST(c.source_id AS INTEGER)
                 WHERE chunks MATCH ?1 AND s.label LIKE ?2
                 ORDER BY rank
                 LIMIT ?3",
            )
            .ok();
        stmt.as_mut().and_then(|s| {
            s.query_map(params![sanitized, filter, limit as i64], map_raw_hit)
                .ok()
                .map(|rows| rows.filter_map(|r| r.ok()).collect())
        })
    } else {
        let mut stmt = conn
            .prepare_cached(
                "SELECT c.title, c.content, c.content_type, s.label,
                        c.line_start, c.line_end,
                        bm25(chunks, 2.0, 1.0) AS rank,
                        highlight(chunks, 1, char(2), char(3)) AS highlighted
                 FROM chunks c
                 JOIN sources s ON s.id = CAST(c.source_id AS INTEGER)
                 WHERE chunks MATCH ?1
                 ORDER BY rank
                 LIMIT ?2",
            )
            .ok();
        stmt.as_mut().and_then(|s| {
            s.query_map(params![sanitized, limit as i64], map_raw_hit)
                .ok()
                .map(|rows| rows.filter_map(|r| r.ok()).collect())
        })
    };

    result.unwrap_or_default()
}

fn search_trigram(
    conn: &Connection,
    query: &str,
    limit: usize,
    source: Option<&str>,
    mode: JoinMode,
) -> Vec<RawHit> {
    let sanitized = match sanitize_trigram_query(query, mode) {
        Some(q) => q,
        None => return Vec::new(),
    };

    let result = if let Some(src) = source {
        let filter = format!("{}%", src);
        let mut stmt = conn
            .prepare_cached(
                "SELECT c.title, c.content, c.content_type, s.label,
                        c.line_start, c.line_end,
                        bm25(chunks_trigram, 2.0, 1.0) AS rank,
                        highlight(chunks_trigram, 1, char(2), char(3)) AS highlighted
                 FROM chunks_trigram c
                 JOIN sources s ON s.id = CAST(c.source_id AS INTEGER)
                 WHERE chunks_trigram MATCH ?1 AND s.label LIKE ?2
                 ORDER BY rank
                 LIMIT ?3",
            )
            .ok();
        stmt.as_mut().and_then(|s| {
            s.query_map(params![sanitized, filter, limit as i64], map_raw_hit)
                .ok()
                .map(|rows| rows.filter_map(|r| r.ok()).collect())
        })
    } else {
        let mut stmt = conn
            .prepare_cached(
                "SELECT c.title, c.content, c.content_type, s.label,
                        c.line_start, c.line_end,
                        bm25(chunks_trigram, 2.0, 1.0) AS rank,
                        highlight(chunks_trigram, 1, char(2), char(3)) AS highlighted
                 FROM chunks_trigram c
                 JOIN sources s ON s.id = CAST(c.source_id AS INTEGER)
                 WHERE chunks_trigram MATCH ?1
                 ORDER BY rank
                 LIMIT ?2",
            )
            .ok();
        stmt.as_mut().and_then(|s| {
            s.query_map(params![sanitized, limit as i64], map_raw_hit)
                .ok()
                .map(|rows| rows.filter_map(|r| r.ok()).collect())
        })
    };

    result.unwrap_or_default()
}

fn map_raw_hit(row: &rusqlite::Row) -> rusqlite::Result<RawHit> {
    Ok(RawHit {
        title: row.get(0)?,
        content: row.get(1)?,
        content_type: row.get(2)?,
        label: row.get(3)?,
        line_start: row.get(4)?,
        line_end: row.get(5)?,
        rank: row.get(6)?,
        highlighted: row.get(7)?,
    })
}

// ─────────────────────────────────────────────────────────
// Snippet extraction
// ─────────────────────────────────────────────────────────

fn extract_snippet(highlighted: &str, content: &str) -> String {
    let stx = '\x02';
    let etx = '\x03';

    let chars: Vec<char> = highlighted.chars().collect();
    let first_match_pos = chars.iter().position(|c| *c == stx);

    if let Some(match_pos) = first_match_pos {
        let half = SNIPPET_MAX_CHARS / 2;
        let start = match_pos.saturating_sub(half);
        let end = (match_pos + half).min(chars.len());

        let window: String = chars[start..end]
            .iter()
            .filter(|c| **c != stx && **c != etx)
            .collect();

        let prefix = if start > 0 { "..." } else { "" };
        let suffix = if end < chars.len() { "..." } else { "" };

        format!("{}{}{}", prefix, window, suffix)
    } else {
        // No match markers — use first N chars of content
        let truncated: String = content.chars().take(SNIPPET_MAX_CHARS).collect();
        if content.chars().count() > SNIPPET_MAX_CHARS {
            format!("{}...", truncated)
        } else {
            truncated
        }
    }
}

fn raw_hits_to_search_hits(hits: Vec<RawHit>, layer: MatchLayer) -> Vec<SearchHit> {
    hits.into_iter()
        .map(|h| SearchHit {
            title: h.title,
            content: extract_snippet(&h.highlighted, &h.content),
            source: h.label,
            rank: h.rank,
            content_type: ContentType::parse(&h.content_type),
            match_layer: layer,
            line_start: h.line_start.max(0) as usize,
            line_end: h.line_end.max(0) as usize,
        })
        .collect()
}

// ─────────────────────────────────────────────────────────
// Fuzzy correction
// ─────────────────────────────────────────────────────────

fn fuzzy_correct_word(conn: &Connection, word: &str) -> Option<String> {
    if word.len() < 3 {
        return None;
    }

    let lower = word.to_lowercase();
    let max_dist = fuzzy::max_edit_distance(lower.chars().count());
    let min_len = lower.chars().count().saturating_sub(max_dist);
    let max_len = lower.chars().count() + max_dist;

    let mut stmt = conn
        .prepare_cached("SELECT word FROM vocabulary WHERE length(word) BETWEEN ?1 AND ?2")
        .ok()?;

    let candidates: Vec<String> = stmt
        .query_map(params![min_len as i64, max_len as i64], |row| {
            row.get::<_, String>(0)
        })
        .ok()?
        .filter_map(|r| r.ok())
        .collect();

    fuzzy::find_best_correction(&lower, &candidates)
}

fn fuzzy_correct_query(conn: &Connection, query: &str) -> Option<String> {
    let lower_query = query.to_lowercase();
    let words: Vec<&str> = lower_query
        .split_whitespace()
        .filter(|w| w.len() >= 3)
        .collect();

    if words.is_empty() {
        return None;
    }

    let original = words.join(" ");
    let corrected_words: Vec<String> = words
        .iter()
        .map(|w| fuzzy_correct_word(conn, w).unwrap_or_else(|| w.to_string()))
        .collect();
    let corrected = corrected_words.join(" ");

    if corrected != original {
        Some(corrected)
    } else {
        None
    }
}

// ─────────────────────────────────────────────────────────
// Three-layer fallback search
// ─────────────────────────────────────────────────────────

pub(crate) fn search_with_fallback(
    conn: &Connection,
    query: &str,
    limit: usize,
    source: Option<&str>,
) -> SearchResult {
    // Layer 1a: Porter AND
    let hits = search_porter(conn, query, limit, source, JoinMode::And);
    if !hits.is_empty() {
        return SearchResult {
            query: query.to_string(),
            hits: raw_hits_to_search_hits(hits, MatchLayer::Porter),
            corrected_query: None,
        };
    }

    // Layer 1b: Porter OR
    let hits = search_porter(conn, query, limit, source, JoinMode::Or);
    if !hits.is_empty() {
        return SearchResult {
            query: query.to_string(),
            hits: raw_hits_to_search_hits(hits, MatchLayer::Porter),
            corrected_query: None,
        };
    }

    // Layer 2a: Trigram AND
    let hits = search_trigram(conn, query, limit, source, JoinMode::And);
    if !hits.is_empty() {
        return SearchResult {
            query: query.to_string(),
            hits: raw_hits_to_search_hits(hits, MatchLayer::Trigram),
            corrected_query: None,
        };
    }

    // Layer 2b: Trigram OR
    let hits = search_trigram(conn, query, limit, source, JoinMode::Or);
    if !hits.is_empty() {
        return SearchResult {
            query: query.to_string(),
            hits: raw_hits_to_search_hits(hits, MatchLayer::Trigram),
            corrected_query: None,
        };
    }

    // Layer 3: Fuzzy correction + re-search
    if let Some(corrected) = fuzzy_correct_query(conn, query) {
        // Re-run Porter AND/OR with corrected query
        let hits = search_porter(conn, &corrected, limit, source, JoinMode::And);
        if !hits.is_empty() {
            return SearchResult {
                query: query.to_string(),
                hits: raw_hits_to_search_hits(hits, MatchLayer::Fuzzy),
                corrected_query: Some(corrected),
            };
        }
        let hits = search_porter(conn, &corrected, limit, source, JoinMode::Or);
        if !hits.is_empty() {
            return SearchResult {
                query: query.to_string(),
                hits: raw_hits_to_search_hits(hits, MatchLayer::Fuzzy),
                corrected_query: Some(corrected),
            };
        }

        // Re-run Trigram AND/OR with corrected query
        let hits = search_trigram(conn, &corrected, limit, source, JoinMode::And);
        if !hits.is_empty() {
            return SearchResult {
                query: query.to_string(),
                hits: raw_hits_to_search_hits(hits, MatchLayer::Fuzzy),
                corrected_query: Some(corrected),
            };
        }
        let hits = search_trigram(conn, &corrected, limit, source, JoinMode::Or);
        if !hits.is_empty() {
            return SearchResult {
                query: query.to_string(),
                hits: raw_hits_to_search_hits(hits, MatchLayer::Fuzzy),
                corrected_query: Some(corrected),
            };
        }
    }

    // No results at all
    SearchResult {
        query: query.to_string(),
        hits: Vec::new(),
        corrected_query: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_fts_query_basic() {
        let result = sanitize_fts_query("hello world", JoinMode::And);
        assert_eq!(result, "\"hello\" \"world\"");
    }

    #[test]
    fn test_sanitize_fts_query_or_mode() {
        let result = sanitize_fts_query("hello world", JoinMode::Or);
        assert_eq!(result, "\"hello\" OR \"world\"");
    }

    #[test]
    fn test_sanitize_fts_query_special_chars() {
        let result = sanitize_fts_query("hello's \"world\" (test)", JoinMode::And);
        // Special chars should be stripped
        assert!(result.contains("hello"));
        assert!(result.contains("world"));
        assert!(result.contains("test"));
    }

    #[test]
    fn test_sanitize_fts_query_fts_keywords() {
        let result = sanitize_fts_query("hello AND world NOT test", JoinMode::And);
        // AND, NOT should be filtered out
        assert!(result.contains("hello"));
        assert!(result.contains("world"));
        assert!(result.contains("test"));
        // The words AND, NOT should not appear as search terms
        assert!(!result.contains("\"AND\""));
        assert!(!result.contains("\"NOT\""));
    }

    #[test]
    fn test_sanitize_fts_query_empty() {
        let result = sanitize_fts_query("", JoinMode::And);
        assert_eq!(result, "\"\"");
    }

    #[test]
    fn test_sanitize_trigram_query_basic() {
        let result = sanitize_trigram_query("useEffect", JoinMode::And);
        assert_eq!(result, Some("\"useEffect\"".to_string()));
    }

    #[test]
    fn test_sanitize_trigram_query_short_words_filtered() {
        let result = sanitize_trigram_query("ab cd", JoinMode::And);
        assert_eq!(result, None); // all words < 3 chars
    }

    #[test]
    fn test_sanitize_trigram_query_too_short() {
        let result = sanitize_trigram_query("ab", JoinMode::And);
        assert_eq!(result, None);
    }

    #[test]
    fn test_extract_snippet_with_markers() {
        let highlighted = "some text \x02matching\x03 content here";
        let content = "some text matching content here";
        let snippet = extract_snippet(highlighted, content);
        assert!(snippet.contains("matching"));
        assert!(!snippet.contains('\x02'));
        assert!(!snippet.contains('\x03'));
    }

    #[test]
    fn test_extract_snippet_no_markers() {
        let content = "short content";
        let snippet = extract_snippet(content, content);
        assert_eq!(snippet, "short content");
    }

    #[test]
    fn test_extract_snippet_long_content_truncated() {
        let content = "a".repeat(500);
        let snippet = extract_snippet(&content, &content);
        assert!(snippet.len() <= SNIPPET_MAX_CHARS + 10); // +10 for "..."
        assert!(snippet.ends_with("..."));
    }
}
