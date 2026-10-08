use std::fmt;

use serde::Serialize;

use crate::truncate::smart_truncate;

// ─────────────────────────────────────────────────────────
// Constants
// ─────────────────────────────────────────────────────────

/// Chars before a line is split into sub-chunks in read().
pub(crate) const LONG_LINE_THRESHOLD: usize = 2000;

/// Max bytes for TOC section in index display.
const INDEX_TOC_CAP: usize = 4 * 1024;

/// Max chars for a TOC entry title.
const TOC_TITLE_MAX_CHARS: usize = 120;

/// Max bytes for search output. Use with `format_search_results()`.
pub const SEARCH_OUTPUT_CAP: usize = 64 * 1024;

/// Max bytes for batch output.
pub(crate) const BATCH_OUTPUT_CAP: usize = 64 * 1024;

/// Hits per query when the caller does not ask for a number.
pub const SEARCH_DEFAULT_LIMIT: usize = 5;

/// Most hits per query a caller may ask for.
pub const SEARCH_MAX_LIMIT: usize = 20;

/// Marker appended when output is cut at its byte cap.
fn truncation_marker(cap: usize) -> String {
    format!(
        "\n\u{2026} [output truncated at ~{}KB \u{2014} narrow with source or fewer queries] \u{2026}\n",
        cap / 1024
    )
}

// ─────────────────────────────────────────────────────────
// Tool names used in hints
// ─────────────────────────────────────────────────────────

/// Names of the search and read tools, so hints spell out the exact call.
#[derive(Debug, Clone, Copy)]
pub struct ToolNames<'a> {
    pub search: &'a str,
    pub read: &'a str,
}

impl Default for ToolNames<'static> {
    fn default() -> Self {
        Self {
            search: "IndexSearch",
            read: "IndexRead",
        }
    }
}

// ─────────────────────────────────────────────────────────
// Date range filter
// ─────────────────────────────────────────────────────────

/// Date range for filtering search results by `sources.indexed_at`.
/// Both bounds use `>` / `<` (exclusive). Use `DateRange::default()` for unbounded.
#[derive(Debug, Clone)]
pub struct DateRange {
    /// Lower bound (exclusive). Empty string matches everything.
    pub after: String,
    /// Upper bound (exclusive). Sentinel value matches everything.
    pub before: String,
}

impl DateRange {
    const SENTINEL_AFTER: &'static str = "";
    const SENTINEL_BEFORE: &'static str = "9999-12-31 23:59:59";

    /// Create a new DateRange. `None` values use sentinels that match everything.
    pub fn new(after: Option<String>, before: Option<String>) -> Self {
        Self {
            after: after.unwrap_or_else(|| Self::SENTINEL_AFTER.to_string()),
            before: before.unwrap_or_else(|| Self::SENTINEL_BEFORE.to_string()),
        }
    }

    /// Whether this range matches every date.
    pub fn is_unbounded(&self) -> bool {
        self.after == Self::SENTINEL_AFTER && self.before == Self::SENTINEL_BEFORE
    }
}

impl Default for DateRange {
    fn default() -> Self {
        Self {
            after: Self::SENTINEL_AFTER.to_string(),
            before: Self::SENTINEL_BEFORE.to_string(),
        }
    }
}

// ─────────────────────────────────────────────────────────
// Enums
// ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentType {
    Prose,
    Code,
}

impl ContentType {
    pub fn as_str(&self) -> &'static str {
        match self {
            ContentType::Prose => "prose",
            ContentType::Code => "code",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "code" => ContentType::Code,
            _ => ContentType::Prose,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentFormat {
    Markdown,
    PlainText,
    Json,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchLayer {
    Porter,
    Trigram,
    Fuzzy,
}

// ─────────────────────────────────────────────────────────
// Internal types
// ─────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Chunk {
    pub title: String,
    pub content: String,
    pub content_type: ContentType,
    pub line_start: usize,
    pub line_end: usize,
    /// Offset reference for TOC display: "8" or "8-2" for sub-line splits.
    pub line_ref: String,
}

/// Parsed offset for read(): supports "5" and "5-2" (sub-line) formats.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LineOffset {
    pub line: usize,        // 1-based
    pub sub: Option<usize>, // 1-based sub-chunk index
}

impl LineOffset {
    pub fn parse(s: &str) -> Result<Self, ContextError> {
        if let Some((line_str, sub_str)) = s.split_once('-') {
            let line: usize = line_str
                .parse()
                .map_err(|_| ContextError::InvalidParams(format!("invalid offset: {:?}", s)))?;
            let sub: usize = sub_str
                .parse()
                .map_err(|_| ContextError::InvalidParams(format!("invalid offset: {:?}", s)))?;
            // Clamp to 1-based minimum
            let line = line.max(1);
            let sub = sub.max(1);
            Ok(LineOffset {
                line,
                sub: Some(sub),
            })
        } else {
            let line: usize = s
                .parse()
                .map_err(|_| ContextError::InvalidParams(format!("invalid offset: {:?}", s)))?;
            // Clamp to 1-based minimum
            let line = line.max(1);
            Ok(LineOffset { line, sub: None })
        }
    }

    pub fn to_display(&self) -> String {
        match self.sub {
            Some(sub) => format!("{}-{}", self.line, sub),
            None => format!("{}", self.line),
        }
    }
}

// ─────────────────────────────────────────────────────────
// Result types
// ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct ChunkToc {
    pub title: String,    // full breadcrumb: "API > Auth > OAuth"
    pub line_ref: String, // "8" or "8-2"
    pub depth: usize,     // hierarchy depth (count of " > " separators)
}

#[derive(Debug, Clone, Serialize)]
pub struct IndexResult {
    pub source_id: i64,
    pub label: String,
    pub total_chunks: usize,
    pub code_chunks: usize,
    pub total_lines: usize,
    pub content_bytes: usize,
    pub chunk_titles: Vec<ChunkToc>,
}

impl IndexResult {
    /// First line: `Indexed "label" — N lines, X.XKB, N chunks (N code)`
    pub fn summary(&self) -> String {
        let label_display = if self.label.chars().count() > 200 {
            let truncated: String = self.label.chars().take(200).collect();
            format!("{}…", truncated)
        } else {
            self.label.clone()
        };
        let kb = self.content_bytes as f64 / 1024.0;
        format!(
            "Indexed {:?} \u{2014} {} lines, {:.1}KB, {} chunks ({} code)",
            label_display, self.total_lines, kb, self.total_chunks, self.code_chunks,
        )
    }

    /// `## Contents` block with optional depth filter + search/read hints.
    /// Pass `None` for unlimited depth, `Some(1)` for top-level only, etc.
    pub fn toc(&self, max_depth: Option<usize>) -> String {
        self.toc_with(max_depth, ToolNames::default())
    }

    /// [`Self::toc`] with hints naming the given tools.
    pub fn toc_with(&self, max_depth: Option<usize>, tools: ToolNames<'_>) -> String {
        let mut out = self.outline(max_depth, INDEX_TOC_CAP, tools.search);
        out.push_str(&format!(
            "\nUse {}(queries: [...], source: {:?}) to find specific content.\n\
             Use {}(label: {:?}, offset: \"1\") to read sections.",
            tools.search, self.label, tools.read, self.label
        ));
        out
    }

    /// Just the `## Contents` block (no hints), pruned to `cap` bytes.
    /// Empty when the content has no sections.
    pub fn outline(&self, max_depth: Option<usize>, cap: usize, search_tool: &str) -> String {
        let mut out = String::new();

        let filtered: Vec<&ChunkToc> = if let Some(max_d) = max_depth {
            self.chunk_titles
                .iter()
                .filter(|e| e.depth <= max_d)
                .collect()
        } else {
            self.chunk_titles.iter().collect()
        };

        if !filtered.is_empty() {
            out.push_str("## Contents\n");

            let (kept, depth_pruned, list_truncated) = prune_toc(&filtered, cap);

            for entry in &kept {
                let indent = "  ".repeat(entry.depth);
                let leaf = leaf_title(&entry.title);
                let leaf_display = if leaf.chars().count() > TOC_TITLE_MAX_CHARS {
                    let truncated: String = leaf.chars().take(TOC_TITLE_MAX_CHARS).collect();
                    format!("{}…", truncated)
                } else {
                    leaf.to_string()
                };
                out.push_str(&format!(
                    "{}- [L{}] {}\n",
                    indent, entry.line_ref, leaf_display
                ));
            }

            if depth_pruned > 0 {
                out.push_str(&format!(
                    "  \u{2026} {} deeper sections pruned \u{2014} use {} to discover\n",
                    depth_pruned, search_tool
                ));
            }

            if list_truncated > 0 {
                out.push_str(&format!("  \u{2026} {} more sections\n", list_truncated));
            }
        }
        out
    }
}

/// Per-item summary within a batch index.
#[derive(Debug, Clone, Serialize)]
pub struct BatchItemSummary {
    pub subpath: String,
    pub bytes: usize,
    pub chunks: usize,
}

/// Result of batch-indexing multiple items under a shared root path.
#[derive(Debug, Clone, Serialize)]
pub struct BatchIndexResult {
    pub root_path: String,
    pub items_indexed: usize,
    pub total_bytes: usize,
    pub total_lines: usize,
    pub total_chunks: usize,
    pub item_summaries: Vec<BatchItemSummary>,
}

impl BatchIndexResult {
    /// Single summary line: `Indexed N items at "root" — X lines, Y.YKB, Z chunks`
    pub fn summary(&self) -> String {
        let kb = self.total_bytes as f64 / 1024.0;
        format!(
            "Indexed {} items at {:?} \u{2014} {} lines, {:.1}KB, {} chunks",
            self.items_indexed, self.root_path, self.total_lines, kb, self.total_chunks,
        )
    }

    /// TOC listing each indexed item. `max_depth=Some(1)` shows only item names (no sub-entries).
    pub fn toc(&self, max_depth: Option<usize>) -> String {
        self.toc_with(max_depth, ToolNames::default())
    }

    /// [`Self::toc`] with hints naming the given search tool.
    pub fn toc_with(&self, _max_depth: Option<usize>, tools: ToolNames<'_>) -> String {
        let mut out = String::new();
        if !self.item_summaries.is_empty() {
            out.push_str("## Contents\n");
            for item in &self.item_summaries {
                out.push_str(&format!("- {}\n", item.subpath));
            }
        }
        out.push_str(&format!(
            "\nUse {}(queries: [...], source: {:?}) to discover items.",
            tools.search, self.root_path
        ));
        out
    }
}

impl fmt::Display for BatchIndexResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "{}", self.summary())?;
        if !self.item_summaries.is_empty() {
            writeln!(f)?;
        }
        write!(f, "{}", self.toc(None))?;
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchHit {
    pub title: String,
    pub content: String,
    pub source: String,
    pub rank: f64,
    pub content_type: ContentType,
    pub match_layer: MatchLayer,
    pub line_start: usize,
    pub line_end: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchResult {
    pub query: String,
    pub hits: Vec<SearchHit>,
    pub corrected_query: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReadResult {
    pub label: String,
    pub content: String,
    pub total_lines: usize,
    pub showing_start: String,
    pub showing_end: String,
    /// Offset that continues where this read stopped; `None` at the end.
    pub next_offset: Option<String>,
    /// Lines from `next_offset` to the end (counting a partly shown line).
    pub remaining_lines: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct SourceInfo {
    pub label: String,
    pub total_lines: usize,
    pub chunk_count: usize,
    pub code_chunk_count: usize,
}

#[derive(Debug, Clone)]
pub struct ReadRequest {
    pub label: String,
    pub offset: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct BatchResult {
    pub search_results: Vec<SearchResult>,
    pub read_results: Vec<ReadResult>,
}

// ─────────────────────────────────────────────────────────
// Error type
// ─────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum ContextError {
    #[error("Database error: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("Source not found: {0}")]
    SourceNotFound(String),

    #[error("No indexed source matches {prefix:?}. Indexed sources: {}", known_sources_list(.known))]
    NoMatchingSource { prefix: String, known: Vec<String> },

    #[error("Invalid parameters: {0}")]
    InvalidParams(String),
}

/// Labels listed in a [`ContextError::NoMatchingSource`] message.
const KNOWN_SOURCES_SHOWN: usize = 30;

fn known_sources_list(known: &[String]) -> String {
    if known.is_empty() {
        return "(none)".to_string();
    }
    let mut out = known
        .iter()
        .take(KNOWN_SOURCES_SHOWN)
        .map(|l| format!("{:?}", l))
        .collect::<Vec<_>>()
        .join(", ");
    if known.len() > KNOWN_SOURCES_SHOWN {
        out.push_str(&format!(
            ", \u{2026} {} more",
            known.len() - KNOWN_SOURCES_SHOWN
        ));
    }
    out
}

// ─────────────────────────────────────────────────────────
// Display implementations (LLM-friendly output)
// ─────────────────────────────────────────────────────────

/// Key identifying a shown hit across queries: (source, snippet text). Only
/// an identical snippet is referenced, so a later query never loses text —
/// a different chunk, or another part of the same chunk, is shown in full.
type HitKey = (String, String);

/// `line N` or `lines A-B` (a range never reads backwards).
fn line_span(start: usize, end: usize) -> String {
    let end = end.max(start);
    if start == end {
        format!("line {}", start)
    } else {
        format!("lines {}-{}", start, end)
    }
}

/// Write one query's results. Hits already shown for an earlier query (per
/// `seen`) are listed by reference instead of repeating their snippet.
fn write_search_result(
    out: &mut String,
    result: &SearchResult,
    seen: &mut std::collections::HashMap<HitKey, (usize, usize)>,
    query_no: usize,
) {
    if result.hits.is_empty() {
        out.push_str(&format!("### No results for {:?}\n", result.query));
        return;
    }

    out.push_str(&format!("### Results for {:?}", result.query));
    if let Some(ref corrected) = result.corrected_query {
        out.push_str(&format!(" (corrected to {:?})", corrected));
    }
    out.push_str("\n\n");

    for (i, hit) in result.hits.iter().enumerate() {
        // Annotate memory session sources so the LLM knows if it's
        // reading a raw transcript or a compacted summary
        let source_annotation = if hit.source.starts_with("session/") {
            if hit.source.ends_with("-summary") {
                " `[compacted summary]`"
            } else {
                " `[transcript]`"
            }
        } else {
            ""
        };
        out.push_str(&format!(
            "**[{}] {} \u{2014} {}** ({}){}",
            i + 1,
            hit.source,
            hit.title,
            line_span(hit.line_start, hit.line_end),
            source_annotation,
        ));

        let key = (hit.source.clone(), hit.content.clone());
        if let Some(&(q, n)) = seen.get(&key) {
            out.push_str(&format!(
                " \u{2014} same as query {} hit [{}] above\n\n",
                q, n
            ));
            continue;
        }
        seen.insert(key, (query_no, i + 1));

        // Content already has line numbers from search extraction
        out.push('\n');
        out.push_str(&hit.content);
        out.push_str("\n\n");
    }
}

impl fmt::Display for SearchResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut out = String::new();
        write_search_result(&mut out, self, &mut std::collections::HashMap::new(), 1);
        write!(f, "{}", out.trim_end_matches('\n'))
    }
}

impl fmt::Display for ReadResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let split = self.showing_start.contains('-') || self.showing_end.contains('-');
        if split {
            // "1-1 to 1-14" reads better than "1-1-1-14"
            writeln!(
                f,
                "Source: {} (lines {} to {} of {}; \"N-M\" is part M of long line N)",
                self.label, self.showing_start, self.showing_end, self.total_lines,
            )?;
        } else {
            writeln!(
                f,
                "Source: {} (lines {}-{} of {})",
                self.label, self.showing_start, self.showing_end, self.total_lines,
            )?;
        }
        writeln!(f)?;
        write!(f, "{}", self.content)?;
        if let Some(ref next) = self.next_offset {
            write!(
                f,
                "\n\n[{} more line{} \u{2014} continue with offset=\"{}\"]",
                self.remaining_lines,
                if self.remaining_lines == 1 { "" } else { "s" },
                next
            )?;
        }
        Ok(())
    }
}

impl fmt::Display for IndexResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "{}", self.summary())?;
        if !self.chunk_titles.is_empty() {
            writeln!(f)?;
        }
        write!(f, "{}", self.toc(None))?;
        Ok(())
    }
}

impl fmt::Display for BatchResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Build output into a buffer, then apply smart_truncate as safety net
        let mut buf = String::new();
        let mut total = 0;

        if !self.search_results.is_empty() {
            buf.push_str("# Search Results\n\n");
            for result in &self.search_results {
                let formatted = result.to_string();
                total += formatted.len();
                if total > BATCH_OUTPUT_CAP {
                    buf.push_str(&truncation_marker(BATCH_OUTPUT_CAP));
                    let truncated = smart_truncate(&buf, BATCH_OUTPUT_CAP);
                    return write!(f, "{}", truncated);
                }
                buf.push_str(&formatted);
                buf.push('\n');
            }
        }

        if !self.read_results.is_empty() {
            if !self.search_results.is_empty() {
                buf.push('\n');
            }
            buf.push_str("# Read Results\n\n");
            for result in &self.read_results {
                let formatted = result.to_string();
                total += formatted.len();
                if total > BATCH_OUTPUT_CAP {
                    buf.push_str(&truncation_marker(BATCH_OUTPUT_CAP));
                    let truncated = smart_truncate(&buf, BATCH_OUTPUT_CAP);
                    return write!(f, "{}", truncated);
                }
                buf.push_str(&formatted);
                buf.push('\n');
            }
        }

        write!(f, "{}", buf)
    }
}

// ─────────────────────────────────────────────────────────
// Search output formatting with cap
// ─────────────────────────────────────────────────────────

/// Format multiple search results with an output byte cap.
///
/// A hit already shown for an earlier query is listed by reference, and one
/// footer tells the model how to read around a hit with `read_tool`.
pub fn format_search_results(results: &[SearchResult], cap: usize, read_tool: &str) -> String {
    let mut output = String::new();
    let mut seen = std::collections::HashMap::new();
    let mut any_hits = false;
    for (i, result) in results.iter().enumerate() {
        let mut formatted = String::new();
        write_search_result(&mut formatted, result, &mut seen, i + 1);
        let formatted = formatted.trim_end_matches('\n');
        any_hits |= !result.hits.is_empty();
        if output.len() + formatted.len() > cap && !output.is_empty() {
            output.push_str(&truncation_marker(cap));
            break;
        }
        if !output.is_empty() {
            output.push_str("\n\n");
        }
        output.push_str(formatted);
    }
    if any_hits {
        output.push_str(&format!(
            "\n\n---\n*Hits are ranked best-first. Read around a hit with \
             {}(label=\"<source>\", offset=\"<line>\").*",
            read_tool
        ));
    }
    // Apply smart_truncate as final safety net
    smart_truncate(&output, cap)
}

// ─────────────────────────────────────────────────────────
// TOC helpers
// ─────────────────────────────────────────────────────────

/// Extract the leaf title (last segment after " > ").
fn leaf_title(title: &str) -> &str {
    title.rsplit(" > ").next().unwrap_or(title)
}

/// Prune TOC entries to fit within max_bytes.
/// Returns (kept entries, depth_pruned count, list_truncated count).
fn prune_toc<'a>(entries: &[&'a ChunkToc], max_bytes: usize) -> (Vec<&'a ChunkToc>, usize, usize) {
    let mut kept: Vec<&ChunkToc> = entries.to_vec();
    let mut depth_pruned = 0;
    let mut list_truncated = 0;

    loop {
        let estimated = estimate_toc_size(&kept);
        if estimated <= max_bytes || kept.is_empty() {
            break;
        }

        let max_depth = kept.iter().map(|e| e.depth).max().unwrap_or(0);
        if max_depth == 0 {
            // Can't prune further by depth — truncate the list
            while estimate_toc_size(&kept) > max_bytes && kept.len() > 1 {
                kept.pop();
                list_truncated += 1;
            }
            break;
        }

        let before = kept.len();
        kept.retain(|e| e.depth < max_depth);
        depth_pruned += before - kept.len();
    }

    (kept, depth_pruned, list_truncated)
}

fn estimate_toc_size(entries: &[&ChunkToc]) -> usize {
    entries
        .iter()
        .map(|e| {
            let leaf = leaf_title(&e.title);
            let leaf_char_count = leaf.chars().count();
            // If leaf exceeds TOC_TITLE_MAX_CHARS, it gets truncated + "…" (3 bytes)
            let leaf_bytes = if leaf_char_count > TOC_TITLE_MAX_CHARS {
                leaf.chars()
                    .take(TOC_TITLE_MAX_CHARS)
                    .map(|c| c.len_utf8())
                    .sum::<usize>()
                    + 3
            } else {
                leaf.len()
            };
            // "  " * depth + "- [L" + line_ref + "] " + leaf + "\n"
            e.depth * 2 + 4 + e.line_ref.len() + 2 + leaf_bytes + 1
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_content_type_roundtrip() {
        assert_eq!(ContentType::parse("code"), ContentType::Code);
        assert_eq!(ContentType::parse("prose"), ContentType::Prose);
        assert_eq!(ContentType::parse("unknown"), ContentType::Prose);
        assert_eq!(ContentType::Code.as_str(), "code");
        assert_eq!(ContentType::Prose.as_str(), "prose");
    }

    // ── LineOffset tests ──

    #[test]
    fn parse_simple_line() {
        let lo = LineOffset::parse("5").unwrap();
        assert_eq!(lo.line, 5);
        assert_eq!(lo.sub, None);
    }

    #[test]
    fn parse_sub_line() {
        let lo = LineOffset::parse("5-2").unwrap();
        assert_eq!(lo.line, 5);
        assert_eq!(lo.sub, Some(2));
    }

    #[test]
    fn parse_zero_clamps_to_one() {
        let lo = LineOffset::parse("0").unwrap();
        assert_eq!(lo.line, 1);
        assert_eq!(lo.sub, None);
    }

    #[test]
    fn parse_sub_zero_clamps_to_one() {
        let lo = LineOffset::parse("5-0").unwrap();
        assert_eq!(lo.line, 5);
        assert_eq!(lo.sub, Some(1));
    }

    #[test]
    fn parse_invalid_string() {
        assert!(LineOffset::parse("abc").is_err());
    }

    #[test]
    fn parse_negative_rejected() {
        assert!(LineOffset::parse("-1").is_err());
    }

    #[test]
    fn display_roundtrip() {
        let lo = LineOffset::parse("5").unwrap();
        assert_eq!(lo.to_display(), "5");
        let lo = LineOffset::parse("5-2").unwrap();
        assert_eq!(lo.to_display(), "5-2");
    }

    // ── Display tests ──

    #[test]
    fn test_search_result_display_with_hits() {
        let result = SearchResult {
            query: "oauth flow".to_string(),
            hits: vec![SearchHit {
                title: "Auth > OAuth".to_string(),
                content: "   8\tThe OAuth flow requires a client_id...".to_string(),
                source: "docs:api".to_string(),
                rank: -1.5,
                content_type: ContentType::Prose,
                match_layer: MatchLayer::Porter,
                line_start: 45,
                line_end: 62,
            }],
            corrected_query: None,
        };
        let display = result.to_string();
        assert!(display.contains("Results for \"oauth flow\""));
        assert!(display.contains("**[1] docs:api"));
        assert!(display.contains("(lines 45-62)"));
        assert!(display.contains("The OAuth flow"));

        let formatted = format_search_results(&[result], SEARCH_OUTPUT_CAP, "MemoryRead");
        assert!(formatted.ends_with(
            "*Hits are ranked best-first. Read around a hit with \
             MemoryRead(label=\"<source>\", offset=\"<line>\").*"
        ));
    }

    #[test]
    fn different_chunks_on_the_same_line_are_both_shown() {
        // Minified JSON: every chunk is on line 1
        let hit = |title: &str, content: &str| SearchHit {
            title: title.to_string(),
            content: content.to_string(),
            source: "jira__getIssue:2".to_string(),
            rank: -1.0,
            content_type: ContentType::Prose,
            match_layer: MatchLayer::Porter,
            line_start: 1,
            line_end: 1,
        };
        let results = [
            SearchResult {
                query: "labels".to_string(),
                hits: vec![hit("data > fields > labels", "[\"monitor\"]")],
                corrected_query: None,
            },
            SearchResult {
                query: "transition".to_string(),
                hits: vec![hit("data > fields > status", "{\"name\": \"Done\"}")],
                corrected_query: None,
            },
            SearchResult {
                query: "monitor".to_string(),
                hits: vec![hit("data > fields > labels", "[\"monitor\"]")],
                corrected_query: None,
            },
        ];
        let out = format_search_results(&results, SEARCH_OUTPUT_CAP, "IndexRead");
        assert!(out.contains("{\"name\": \"Done\"}"), "{out}");
        assert_eq!(out.matches("same as query").count(), 1, "{out}");
        assert!(out.contains("same as query 1 hit [1] above"), "{out}");
    }

    #[test]
    fn line_span_never_reads_backwards() {
        assert_eq!(line_span(2, 1), "line 2");
        assert_eq!(line_span(3, 3), "line 3");
        assert_eq!(line_span(3, 9), "lines 3-9");
    }

    #[test]
    fn no_footer_when_nothing_found() {
        let empty = SearchResult {
            query: "x".to_string(),
            hits: vec![],
            corrected_query: None,
        };
        let formatted = format_search_results(&[empty], SEARCH_OUTPUT_CAP, "IndexRead");
        assert_eq!(formatted, "### No results for \"x\"");
    }

    #[test]
    fn test_search_result_display_empty() {
        let result = SearchResult {
            query: "nonexistent".to_string(),
            hits: vec![],
            corrected_query: None,
        };
        let display = result.to_string();
        assert!(display.contains("No results for \"nonexistent\""));
    }

    #[test]
    fn test_search_result_display_with_correction() {
        let result = SearchResult {
            query: "kuberntes".to_string(),
            hits: vec![SearchHit {
                title: "Deployment".to_string(),
                content: "   1\tkubernetes cluster setup".to_string(),
                source: "docs:k8s".to_string(),
                rank: -1.0,
                content_type: ContentType::Prose,
                match_layer: MatchLayer::Fuzzy,
                line_start: 1,
                line_end: 10,
            }],
            corrected_query: Some("kubernetes".to_string()),
        };
        let display = result.to_string();
        assert!(display.contains("corrected to \"kubernetes\""));
    }

    #[test]
    fn test_read_result_display() {
        let result = ReadResult {
            label: "docs:api".to_string(),
            content: "    45\tline one\n    46\tline two\n".to_string(),
            total_lines: 120,
            showing_start: "45".to_string(),
            showing_end: "46".to_string(),
            next_offset: None,
            remaining_lines: 0,
        };
        let display = result.to_string();
        assert!(display.contains("Source: docs:api (lines 45-46 of 120)"));
        assert!(display.contains("45\tline one"));
    }

    #[test]
    fn test_index_result_display() {
        let result = IndexResult {
            source_id: 1,
            label: "docs:api".to_string(),
            total_chunks: 15,
            code_chunks: 3,
            total_lines: 200,
            content_bytes: 15565,
            chunk_titles: vec![
                ChunkToc {
                    title: "API Documentation".to_string(),
                    line_ref: "1".to_string(),
                    depth: 0,
                },
                ChunkToc {
                    title: "API Documentation > Authentication".to_string(),
                    line_ref: "5".to_string(),
                    depth: 1,
                },
                ChunkToc {
                    title: "API Documentation > Authentication > OAuth Flow".to_string(),
                    line_ref: "8".to_string(),
                    depth: 2,
                },
            ],
        };
        let display = result.to_string();
        assert!(display.contains("Indexed \"docs:api\""));
        assert!(display.contains("200 lines"));
        assert!(display.contains("15 chunks"));
        assert!(display.contains("3 code"));
        assert!(display.contains("## Contents"));
        assert!(display.contains("[L1] API Documentation"));
        assert!(display.contains("[L5] Authentication"));
        assert!(display.contains("[L8] OAuth Flow"));
    }

    #[test]
    fn index_display_toc_pruning() {
        // Create a TOC with many deep entries that exceed 4KB
        let mut entries = Vec::new();
        for i in 0..200 {
            entries.push(ChunkToc {
                title: format!("Root > Section {} > Subsection {}", i / 10, i),
                line_ref: format!("{}", i + 1),
                depth: 2,
            });
        }
        let result = IndexResult {
            source_id: 1,
            label: "big:doc".to_string(),
            total_chunks: 200,
            code_chunks: 0,
            total_lines: 2000,
            content_bytes: 100_000,
            chunk_titles: entries,
        };
        let display = result.to_string();
        assert!(display.contains("pruned"));
    }

    #[test]
    fn index_display_toc_title_truncated() {
        let long_title = "A".repeat(200);
        let result = IndexResult {
            source_id: 1,
            label: "test".to_string(),
            total_chunks: 1,
            code_chunks: 0,
            total_lines: 10,
            content_bytes: 500,
            chunk_titles: vec![ChunkToc {
                title: long_title,
                line_ref: "1".to_string(),
                depth: 0,
            }],
        };
        let display = result.to_string();
        // Title should be truncated at TOC_TITLE_MAX_CHARS
        assert!(display.contains("…"));
    }

    #[test]
    fn test_batch_result_display() {
        let batch = BatchResult {
            search_results: vec![SearchResult {
                query: "test".to_string(),
                hits: vec![],
                corrected_query: None,
            }],
            read_results: vec![ReadResult {
                label: "test".to_string(),
                content: "1\tline one".to_string(),
                total_lines: 1,
                showing_start: "1".to_string(),
                showing_end: "1".to_string(),
                next_offset: None,
                remaining_lines: 0,
            }],
        };
        let display = batch.to_string();
        assert!(display.contains("# Search Results"));
        assert!(display.contains("# Read Results"));
    }

    #[test]
    fn search_output_small_no_truncation() {
        let results = vec![SearchResult {
            query: "test".to_string(),
            hits: vec![],
            corrected_query: None,
        }];
        let output = format_search_results(&results, SEARCH_OUTPUT_CAP, "IndexRead");
        assert!(!output.contains("truncated"));
    }

    #[test]
    fn index_display_unicode_label_no_panic() {
        // Labels with multi-byte chars must not panic on truncation
        let long_label: String = "\u{4f60}\u{597d}".repeat(200); // 400 CJK chars
        let result = IndexResult {
            source_id: 1,
            label: long_label,
            total_chunks: 1,
            code_chunks: 0,
            total_lines: 1,
            content_bytes: 10,
            chunk_titles: vec![],
        };
        // Should not panic
        let display = result.to_string();
        assert!(display.contains("…")); // label truncated
    }

    #[test]
    fn index_display_unicode_toc_title_no_panic() {
        // TOC titles with multi-byte chars must not panic on truncation
        let long_title: String = "\u{4f60}\u{597d}".repeat(200); // 400 CJK chars
        let result = IndexResult {
            source_id: 1,
            label: "test".to_string(),
            total_chunks: 1,
            code_chunks: 0,
            total_lines: 10,
            content_bytes: 500,
            chunk_titles: vec![ChunkToc {
                title: long_title,
                line_ref: "1".to_string(),
                depth: 0,
            }],
        };
        // Should not panic
        let display = result.to_string();
        assert!(display.contains("…"));
    }

    #[test]
    fn batch_result_single_huge_result_capped() {
        // A single enormous result should still be capped by smart_truncate
        let batch = BatchResult {
            search_results: vec![],
            read_results: vec![ReadResult {
                label: "big".to_string(),
                content: "x".repeat(60_000),
                total_lines: 1,
                showing_start: "1".to_string(),
                showing_end: "1".to_string(),
                next_offset: None,
                remaining_lines: 0,
            }],
        };
        let display = batch.to_string();
        // The output should be bounded
        assert!(
            display.len() <= BATCH_OUTPUT_CAP + 1000,
            "Batch display should be capped, got {} bytes",
            display.len()
        );
    }

    #[test]
    fn search_output_many_results_capped() {
        let results: Vec<SearchResult> = (0..100)
            .map(|i| SearchResult {
                query: format!("query_{}", i),
                hits: vec![SearchHit {
                    title: "Title".to_string(),
                    content: "x".repeat(1000),
                    source: "src".to_string(),
                    rank: -1.0,
                    content_type: ContentType::Prose,
                    match_layer: MatchLayer::Porter,
                    line_start: 1,
                    line_end: 100,
                }],
                corrected_query: None,
            })
            .collect();
        let output = format_search_results(&results, SEARCH_OUTPUT_CAP, "IndexRead");
        assert!(output.len() <= SEARCH_OUTPUT_CAP + 200); // small slack
    }
}
