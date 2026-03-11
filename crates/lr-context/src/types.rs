use std::fmt;

use serde::Serialize;

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
// Internal chunk type (not public)
// ─────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub(crate) struct Chunk {
    pub title: String,
    pub content: String,
    pub content_type: ContentType,
    pub line_start: usize,
    pub line_end: usize,
}

// ─────────────────────────────────────────────────────────
// Result types
// ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct IndexResult {
    pub source_id: i64,
    pub label: String,
    pub total_chunks: usize,
    pub code_chunks: usize,
    pub total_lines: usize,
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
    pub showing_start: usize,
    pub showing_end: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct SourceInfo {
    pub label: String,
    pub total_lines: usize,
    pub chunk_count: usize,
    pub code_chunk_count: usize,
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

            // Blockquote the snippet
            for line in hit.content.lines() {
                writeln!(f, "> {}", line)?;
            }
            writeln!(f)?;
        }

        writeln!(f, "---")?;
        write!(f, "*Use read(source, offset, limit) for more context.*")?;
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
        write!(
            f,
            "Indexed {} ({} chunks, {} code, {} lines)",
            self.label, self.total_chunks, self.code_chunks, self.total_lines,
        )
    }
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

    #[test]
    fn test_search_result_display_with_hits() {
        let result = SearchResult {
            query: "oauth flow".to_string(),
            hits: vec![SearchHit {
                title: "Auth > OAuth".to_string(),
                content: "The OAuth flow requires a client_id...".to_string(),
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
        assert!(display.contains("> The OAuth flow"));
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
                content: "kubernetes cluster setup".to_string(),
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
            showing_start: 45,
            showing_end: 46,
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
        };
        let display = result.to_string();
        assert!(display.contains("Indexed docs:api"));
        assert!(display.contains("15 chunks"));
        assert!(display.contains("3 code"));
    }
}
