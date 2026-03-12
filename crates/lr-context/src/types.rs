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
pub const SEARCH_OUTPUT_CAP: usize = 40 * 1024;

/// Max bytes for batch output.
pub(crate) const BATCH_OUTPUT_CAP: usize = 40 * 1024;

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
pub(crate) struct Chunk {
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

    #[error("Invalid parameters: {0}")]
    InvalidParams(String),
}

// ─────────────────────────────────────────────────────────
// Display implementations (LLM-friendly output)
// ─────────────────────────────────────────────────────────

impl fmt::Display for SearchResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.hits.is_empty() {
            writeln!(f, "### No results for {:?}", self.query)?;
            return Ok(());
        }

        write!(f, "### Results for {:?}", self.query)?;
        if let Some(ref corrected) = self.corrected_query {
            write!(f, " (corrected to {:?})", corrected)?;
        }
        writeln!(f)?;
        writeln!(f)?;

        for (i, hit) in self.hits.iter().enumerate() {
            writeln!(
                f,
                "**[{}] {} \u{2014} {}** (lines {}-{})",
                i + 1,
                hit.source,
                hit.title,
                hit.line_start,
                hit.line_end,
            )?;

            // Content already has line numbers from search extraction
            writeln!(f, "{}", hit.content)?;
            writeln!(f)?;
        }

        writeln!(f, "---")?;
        write!(f, "*Use read(source, offset, limit) for full context.*")?;
        Ok(())
    }
}

impl fmt::Display for ReadResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "Source: {} (lines {}-{} of {})",
            self.label, self.showing_start, self.showing_end, self.total_lines,
        )?;
        writeln!(f)?;
        write!(f, "{}", self.content)?;
        Ok(())
    }
}

impl fmt::Display for IndexResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label_display = if self.label.chars().count() > 200 {
            let truncated: String = self.label.chars().take(200).collect();
            format!("{}…", truncated)
        } else {
            self.label.clone()
        };
        let kb = self.content_bytes as f64 / 1024.0;
        writeln!(
            f,
            "Indexed {:?} \u{2014} {} lines, {:.1}KB, {} chunks ({} code)",
            label_display, self.total_lines, kb, self.total_chunks, self.code_chunks,
        )?;

        if !self.chunk_titles.is_empty() {
            writeln!(f)?;
            writeln!(f, "## Contents")?;

            let (kept, depth_pruned, list_truncated) = prune_toc(&self.chunk_titles, INDEX_TOC_CAP);

            for entry in &kept {
                let indent = "  ".repeat(entry.depth);
                let leaf = leaf_title(&entry.title);
                let leaf_display = if leaf.chars().count() > TOC_TITLE_MAX_CHARS {
                    let truncated: String = leaf.chars().take(TOC_TITLE_MAX_CHARS).collect();
                    format!("{}…", truncated)
                } else {
                    leaf.to_string()
                };
                writeln!(f, "{}- [L{}] {}", indent, entry.line_ref, leaf_display)?;
            }

            if depth_pruned > 0 {
                writeln!(
                    f,
                    "  \u{2026} {} deeper sections pruned \u{2014} use search() to discover",
                    depth_pruned
                )?;
            }

            if list_truncated > 0 {
                writeln!(f, "  \u{2026} {} more sections", list_truncated)?;
            }
        }

        writeln!(f)?;
        writeln!(f, "Use search(queries: [...]) to find specific content.")?;
        write!(
            f,
            "Use read(source: {:?}, offset: \"1\") to read sections.",
            self.label
        )?;
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
                    buf.push_str("\n\u{2026} [output truncated at ~40KB] \u{2026}\n");
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
                    buf.push_str("\n\u{2026} [output truncated at ~40KB] \u{2026}\n");
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
pub fn format_search_results(results: &[SearchResult], cap: usize) -> String {
    let mut output = String::new();
    for result in results {
        let formatted = result.to_string();
        if output.len() + formatted.len() > cap && !output.is_empty() {
            output.push_str("\n\u{2026} [output truncated at ~40KB] \u{2026}\n");
            break;
        }
        if !output.is_empty() {
            output.push('\n');
        }
        output.push_str(&formatted);
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
fn prune_toc(entries: &[ChunkToc], max_bytes: usize) -> (Vec<&ChunkToc>, usize, usize) {
    let mut kept: Vec<&ChunkToc> = entries.iter().collect();
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
        assert!(display.contains("read(source, offset, limit)"));
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
        let output = format_search_results(&results, SEARCH_OUTPUT_CAP);
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
        let output = format_search_results(&results, SEARCH_OUTPUT_CAP);
        assert!(output.len() <= SEARCH_OUTPUT_CAP + 200); // small slack
    }
}
