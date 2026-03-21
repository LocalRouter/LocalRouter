use rusqlite::{params, Connection};

use crate::fuzzy;
use crate::types::{ContentType, DateRange, MatchLayer, SearchHit, SearchResult};

const SNIPPET_WINDOW: usize = 300; // chars radius per match
pub(crate) const SNIPPET_MAX_LEN: usize = 1500; // default per-hit max
pub(crate) const SNIPPET_BATCH_MAX_LEN: usize = 3000; // batch mode
const SNIPPET_LINE_MAX_CHARS: usize = 200; // max chars per line in snippet

/// Escape SQL LIKE metacharacters (`%`, `_`, `\`) so they match literally.
fn escape_like(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

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
    date_range: &DateRange,
) -> Vec<RawHit> {
    let sanitized = sanitize_fts_query(query, mode);
    if sanitized == "\"\"" {
        return Vec::new();
    }

    let result = if let Some(src) = source {
        let filter = format!("{}%", escape_like(src));
        conn.prepare_cached(
            "SELECT c.title, c.content, c.content_type, s.label,
                    c.line_start, c.line_end,
                    bm25(chunks, 2.0, 1.0) AS rank,
                    highlight(chunks, 1, char(2), char(3)) AS highlighted
             FROM chunks c
             JOIN sources s ON s.id = CAST(c.source_id AS INTEGER)
             WHERE chunks MATCH ?1 AND s.label LIKE ?2 ESCAPE '\\'
               AND s.indexed_at > ?3 AND s.indexed_at < ?4
             ORDER BY rank
             LIMIT ?5",
        )
        .and_then(|mut s| {
            let rows = s.query_map(
                params![
                    sanitized,
                    filter,
                    &date_range.after,
                    &date_range.before,
                    limit as i64
                ],
                map_raw_hit,
            )?;
            Ok(rows.filter_map(|r| r.ok()).collect())
        })
    } else {
        conn.prepare_cached(
            "SELECT c.title, c.content, c.content_type, s.label,
                    c.line_start, c.line_end,
                    bm25(chunks, 2.0, 1.0) AS rank,
                    highlight(chunks, 1, char(2), char(3)) AS highlighted
             FROM chunks c
             JOIN sources s ON s.id = CAST(c.source_id AS INTEGER)
             WHERE chunks MATCH ?1
               AND s.indexed_at > ?2 AND s.indexed_at < ?3
             ORDER BY rank
             LIMIT ?4",
        )
        .and_then(|mut s| {
            let rows = s.query_map(
                params![
                    sanitized,
                    &date_range.after,
                    &date_range.before,
                    limit as i64
                ],
                map_raw_hit,
            )?;
            Ok(rows.filter_map(|r| r.ok()).collect())
        })
    };

    match result {
        Ok(hits) => hits,
        Err(e) => {
            tracing::debug!("Porter search error: {}", e);
            Vec::new()
        }
    }
}

fn search_trigram(
    conn: &Connection,
    query: &str,
    limit: usize,
    source: Option<&str>,
    mode: JoinMode,
    date_range: &DateRange,
) -> Vec<RawHit> {
    let sanitized = match sanitize_trigram_query(query, mode) {
        Some(q) => q,
        None => return Vec::new(),
    };

    let result = if let Some(src) = source {
        let filter = format!("{}%", escape_like(src));
        conn.prepare_cached(
            "SELECT c.title, c.content, c.content_type, s.label,
                    c.line_start, c.line_end,
                    bm25(chunks_trigram, 2.0, 1.0) AS rank,
                    highlight(chunks_trigram, 1, char(2), char(3)) AS highlighted
             FROM chunks_trigram c
             JOIN sources s ON s.id = CAST(c.source_id AS INTEGER)
             WHERE chunks_trigram MATCH ?1 AND s.label LIKE ?2 ESCAPE '\\'
               AND s.indexed_at > ?3 AND s.indexed_at < ?4
             ORDER BY rank
             LIMIT ?5",
        )
        .and_then(|mut s| {
            let rows = s.query_map(
                params![
                    sanitized,
                    filter,
                    &date_range.after,
                    &date_range.before,
                    limit as i64
                ],
                map_raw_hit,
            )?;
            Ok(rows.filter_map(|r| r.ok()).collect())
        })
    } else {
        conn.prepare_cached(
            "SELECT c.title, c.content, c.content_type, s.label,
                    c.line_start, c.line_end,
                    bm25(chunks_trigram, 2.0, 1.0) AS rank,
                    highlight(chunks_trigram, 1, char(2), char(3)) AS highlighted
             FROM chunks_trigram c
             JOIN sources s ON s.id = CAST(c.source_id AS INTEGER)
             WHERE chunks_trigram MATCH ?1
               AND s.indexed_at > ?2 AND s.indexed_at < ?3
             ORDER BY rank
             LIMIT ?4",
        )
        .and_then(|mut s| {
            let rows = s.query_map(
                params![
                    sanitized,
                    &date_range.after,
                    &date_range.before,
                    limit as i64
                ],
                map_raw_hit,
            )?;
            Ok(rows.filter_map(|r| r.ok()).collect())
        })
    };

    match result {
        Ok(hits) => hits,
        Err(e) => {
            tracing::debug!("Trigram search error: {}", e);
            Vec::new()
        }
    }
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
// Multi-match snippet extraction with line numbers
// ─────────────────────────────────────────────────────────

/// Find character positions (in original content, without markers) of STX match starts.
fn find_match_positions(highlighted: &str) -> Vec<usize> {
    let mut positions = Vec::new();
    let mut content_pos = 0;
    for ch in highlighted.chars() {
        match ch {
            '\x02' => {
                positions.push(content_pos);
            }
            '\x03' => {}
            _ => {
                content_pos += 1;
            }
        }
    }
    positions
}

/// Build a table of line start positions (in char indices) for content.
fn build_line_starts(content_chars: &[char]) -> Vec<usize> {
    let mut starts = vec![0];
    for (i, &ch) in content_chars.iter().enumerate() {
        if ch == '\n' {
            starts.push(i + 1);
        }
    }
    starts
}

/// Extract multi-match snippet with line numbers.
fn extract_multi_snippet(
    highlighted: &str,
    content: &str,
    line_start: usize,
    max_len: usize,
) -> String {
    let content_chars: Vec<char> = content.chars().collect();
    let content_len = content_chars.len();

    if content_len == 0 {
        return String::new();
    }

    let match_positions = find_match_positions(highlighted);

    if match_positions.is_empty() {
        // No matches — show first N lines
        return format_first_n_lines(content, line_start, max_len);
    }

    let line_starts = build_line_starts(&content_chars);

    let char_to_line = |pos: usize| -> usize {
        line_starts
            .partition_point(|&ls| ls <= pos)
            .saturating_sub(1)
    };

    // Create char-based windows around each match
    let mut windows: Vec<(usize, usize)> = match_positions
        .iter()
        .map(|&pos| {
            let start = pos.saturating_sub(SNIPPET_WINDOW);
            let end = (pos + SNIPPET_WINDOW).min(content_len);
            (start, end)
        })
        .collect();

    // Sort and merge overlapping
    windows.sort_by_key(|w| w.0);
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for w in windows {
        if let Some(last) = merged.last_mut() {
            if w.0 <= last.1 {
                last.1 = last.1.max(w.1);
                continue;
            }
        }
        merged.push(w);
    }

    // Convert to line-based windows (expand to full lines) and cap
    let content_lines: Vec<&str> = content.lines().collect();
    let total_content_lines = content_lines.len();

    let mut line_windows: Vec<(usize, usize)> = Vec::new(); // (start_line_idx, end_line_idx) inclusive
    let mut total_chars = 0;

    for &(win_start, win_end) in &merged {
        let start_line_idx = char_to_line(win_start);
        let end_line_idx = char_to_line(win_end.saturating_sub(1).max(win_start));
        let end_line_idx = end_line_idx.min(total_content_lines.saturating_sub(1));

        // Estimate chars for this window
        let window_chars: usize = (start_line_idx..=end_line_idx)
            .filter_map(|i| content_lines.get(i))
            .map(|l| l.chars().count().min(SNIPPET_LINE_MAX_CHARS) + 10) // +10 for label + tab + newline
            .sum();

        if total_chars + window_chars > max_len && !line_windows.is_empty() {
            break;
        }

        line_windows.push((start_line_idx, end_line_idx));
        total_chars += window_chars;
    }

    if line_windows.is_empty() {
        return format_first_n_lines(content, line_start, max_len);
    }

    // Filter out degenerate windows where end < start
    let line_windows: Vec<(usize, usize)> =
        line_windows.into_iter().filter(|&(s, e)| e >= s).collect();

    if line_windows.is_empty() {
        return format_first_n_lines(content, line_start, max_len);
    }

    // Calculate max label width from all windows
    let max_abs_line = line_windows
        .iter()
        .map(|&(_, end)| line_start + end)
        .max()
        .unwrap_or(line_start);
    let max_width = max_abs_line.to_string().len().max(1);

    // Format output
    let mut output = String::new();

    for (wi, &(start, end)) in line_windows.iter().enumerate() {
        if wi > 0 {
            output.push_str(&format!("{:>width$}\n", "\u{2026}", width = max_width));
        }

        for (i, line) in content_lines.iter().enumerate().take(end + 1).skip(start) {
            let abs_line = line_start + i;

            // Truncate long lines
            let display_line = if line.chars().count() > SNIPPET_LINE_MAX_CHARS {
                let truncated: String = line.chars().take(SNIPPET_LINE_MAX_CHARS).collect();
                format!("{}\u{2026}", truncated)
            } else {
                line.to_string()
            };

            output.push_str(&format!(
                "{:>width$}\t{}\n",
                abs_line,
                display_line,
                width = max_width
            ));
        }
    }

    output.trim_end_matches('\n').to_string()
}

/// Format the first N lines of content with line numbers (fallback when no matches).
fn format_first_n_lines(content: &str, line_start: usize, max_len: usize) -> String {
    let mut output = String::new();
    let lines: Vec<&str> = content.lines().collect();
    let total = lines.len();
    let max_line = line_start + total.saturating_sub(1);
    let width = max_line.to_string().len().max(1);

    for (i, line) in lines.iter().enumerate() {
        let abs_line = line_start + i;
        let display_line = if line.chars().count() > SNIPPET_LINE_MAX_CHARS {
            let truncated: String = line.chars().take(SNIPPET_LINE_MAX_CHARS).collect();
            format!("{}\u{2026}", truncated)
        } else {
            line.to_string()
        };
        let formatted = format!("{:>width$}\t{}\n", abs_line, display_line, width = width);
        if output.len() + formatted.len() > max_len && !output.is_empty() {
            break;
        }
        output.push_str(&formatted);
    }

    output.trim_end_matches('\n').to_string()
}

fn raw_hits_to_search_hits(
    hits: Vec<RawHit>,
    layer: MatchLayer,
    max_snippet_len: usize,
) -> Vec<SearchHit> {
    hits.into_iter()
        .map(|h| {
            let line_start = h.line_start.max(1) as usize;
            let content =
                extract_multi_snippet(&h.highlighted, &h.content, line_start, max_snippet_len);
            SearchHit {
                title: h.title,
                content,
                source: h.label,
                rank: h.rank,
                content_type: ContentType::parse(&h.content_type),
                match_layer: layer,
                line_start,
                line_end: h.line_end.max(1) as usize,
            }
        })
        .collect()
}

// ─────────────────────────────────────────────────────────
// Fuzzy correction
// ─────────────────────────────────────────────────────────

fn fuzzy_correct_word(conn: &Connection, word: &str) -> Option<String> {
    if word.chars().count() < 3 {
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
        .filter(|w| w.chars().count() >= 3)
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
    max_snippet_len: usize,
    date_range: &DateRange,
) -> SearchResult {
    // Layer 1a: Porter AND
    let hits = search_porter(conn, query, limit, source, JoinMode::And, date_range);
    if !hits.is_empty() {
        return SearchResult {
            query: query.to_string(),
            hits: raw_hits_to_search_hits(hits, MatchLayer::Porter, max_snippet_len),
            corrected_query: None,
        };
    }

    // Layer 1b: Porter OR
    let hits = search_porter(conn, query, limit, source, JoinMode::Or, date_range);
    if !hits.is_empty() {
        return SearchResult {
            query: query.to_string(),
            hits: raw_hits_to_search_hits(hits, MatchLayer::Porter, max_snippet_len),
            corrected_query: None,
        };
    }

    // Layer 2a: Trigram AND
    let hits = search_trigram(conn, query, limit, source, JoinMode::And, date_range);
    if !hits.is_empty() {
        return SearchResult {
            query: query.to_string(),
            hits: raw_hits_to_search_hits(hits, MatchLayer::Trigram, max_snippet_len),
            corrected_query: None,
        };
    }

    // Layer 2b: Trigram OR
    let hits = search_trigram(conn, query, limit, source, JoinMode::Or, date_range);
    if !hits.is_empty() {
        return SearchResult {
            query: query.to_string(),
            hits: raw_hits_to_search_hits(hits, MatchLayer::Trigram, max_snippet_len),
            corrected_query: None,
        };
    }

    // Layer 3: Fuzzy correction + re-search
    if let Some(corrected) = fuzzy_correct_query(conn, query) {
        let hits = search_porter(conn, &corrected, limit, source, JoinMode::And, date_range);
        if !hits.is_empty() {
            return SearchResult {
                query: query.to_string(),
                hits: raw_hits_to_search_hits(hits, MatchLayer::Fuzzy, max_snippet_len),
                corrected_query: Some(corrected),
            };
        }
        let hits = search_porter(conn, &corrected, limit, source, JoinMode::Or, date_range);
        if !hits.is_empty() {
            return SearchResult {
                query: query.to_string(),
                hits: raw_hits_to_search_hits(hits, MatchLayer::Fuzzy, max_snippet_len),
                corrected_query: Some(corrected),
            };
        }

        let hits = search_trigram(conn, &corrected, limit, source, JoinMode::And, date_range);
        if !hits.is_empty() {
            return SearchResult {
                query: query.to_string(),
                hits: raw_hits_to_search_hits(hits, MatchLayer::Fuzzy, max_snippet_len),
                corrected_query: Some(corrected),
            };
        }
        let hits = search_trigram(conn, &corrected, limit, source, JoinMode::Or, date_range);
        if !hits.is_empty() {
            return SearchResult {
                query: query.to_string(),
                hits: raw_hits_to_search_hits(hits, MatchLayer::Fuzzy, max_snippet_len),
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
        assert!(result.contains("hello"));
        assert!(result.contains("world"));
        assert!(result.contains("test"));
    }

    #[test]
    fn test_sanitize_fts_query_fts_keywords() {
        let result = sanitize_fts_query("hello AND world NOT test", JoinMode::And);
        assert!(result.contains("hello"));
        assert!(result.contains("world"));
        assert!(result.contains("test"));
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
        assert_eq!(result, None);
    }

    #[test]
    fn test_sanitize_trigram_query_too_short() {
        let result = sanitize_trigram_query("ab", JoinMode::And);
        assert_eq!(result, None);
    }

    // ── Multi-match snippet tests ──

    #[test]
    fn snippet_single_match_window() {
        let content = "line one\nline two with match_word here\nline three";
        let highlighted = "line one\nline two with \x02match_word\x03 here\nline three";
        let result = extract_multi_snippet(highlighted, content, 1, SNIPPET_MAX_LEN);
        assert!(result.contains("match_word"));
        assert!(result.contains("\t")); // has line numbers
    }

    #[test]
    fn snippet_multiple_matches_merged() {
        let content = "line one\nline two match_a\nline three match_b\nline four";
        let highlighted =
            "line one\nline two \x02match_a\x03\nline three \x02match_b\x03\nline four";
        let result = extract_multi_snippet(highlighted, content, 1, SNIPPET_MAX_LEN);
        // Both matches should be in one window (they're close)
        assert!(result.contains("match_a"));
        assert!(result.contains("match_b"));
        // Should NOT have separator between them
        assert!(!result.contains("\u{2026}\n"));
    }

    #[test]
    fn snippet_respects_max_len() {
        let mut content = String::new();
        for i in 0..100 {
            content.push_str(&format!(
                "Line {} with some content for testing purposes\n",
                i
            ));
        }
        let highlighted = content.replace("Line 50", "\x02Line 50\x03");
        let result = extract_multi_snippet(&highlighted, &content, 1, 500);
        assert!(result.len() < 1000); // Should be bounded
    }

    #[test]
    fn snippet_boundary_markers() {
        let mut content = String::new();
        for i in 0..50 {
            content.push_str(&format!("Line {} of the document content\n", i));
        }
        let highlighted = content.replace("Line 25", "\x02Line 25\x03");
        let result = extract_multi_snippet(&highlighted, &content, 1, SNIPPET_MAX_LEN);
        // Should have content around line 25
        assert!(result.contains("Line 25"));
    }

    #[test]
    fn snippet_no_markers_full_content() {
        let content = "small\ncontent";
        let result = extract_multi_snippet(content, content, 1, SNIPPET_MAX_LEN);
        // No match markers — should show first N lines
        assert!(result.contains("small"));
        assert!(result.contains("content"));
    }

    #[test]
    fn snippet_stx_etx_stripped() {
        let content = "text with keyword here";
        let highlighted = "text with \x02keyword\x03 here";
        let result = extract_multi_snippet(highlighted, content, 1, SNIPPET_MAX_LEN);
        assert!(!result.contains('\x02'));
        assert!(!result.contains('\x03'));
    }

    #[test]
    fn snippet_fallback_no_markers() {
        let content = "a".repeat(500);
        let result = extract_multi_snippet(&content, &content, 1, SNIPPET_MAX_LEN);
        assert!(!result.is_empty());
    }

    // ── Line-numbered output tests ──

    #[test]
    fn search_display_line_numbers() {
        let content = "first line\nsecond line\nthird line";
        let highlighted = "first line\n\x02second\x03 line\nthird line";
        let result = extract_multi_snippet(highlighted, content, 10, SNIPPET_MAX_LEN);
        // Should contain tab-separated line numbers
        assert!(result.contains("10\t"));
        assert!(result.contains("11\t"));
    }

    #[test]
    fn search_display_long_line_truncated() {
        let long_line = "x".repeat(500);
        let content = format!("short\n{}\nshort", long_line);
        let highlighted = format!("short\n\x02{}\x03\nshort", long_line);
        let result = extract_multi_snippet(&highlighted, &content, 1, SNIPPET_MAX_LEN);
        // The long line should be truncated
        assert!(result.contains("\u{2026}"));
    }

    #[test]
    fn search_display_hit_header() {
        let hit = SearchHit {
            title: "Auth > OAuth".to_string(),
            content: "  8\t### OAuth Flow".to_string(),
            source: "docs:api".to_string(),
            rank: -1.5,
            content_type: ContentType::Prose,
            match_layer: MatchLayer::Porter,
            line_start: 8,
            line_end: 14,
        };
        let result = SearchResult {
            query: "OAuth".to_string(),
            hits: vec![hit],
            corrected_query: None,
        };
        let display = result.to_string();
        assert!(display.contains("**[1] docs:api \u{2014} Auth > OAuth** (lines 8-14)"));
    }
}
