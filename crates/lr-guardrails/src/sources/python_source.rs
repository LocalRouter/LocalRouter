//! Python file regex pattern extractor
//!
//! Extracts regex patterns from Python source files commonly found in
//! guardrail projects like Presidio and LLM Guard. Handles:
//! - `re.compile(r"pattern")` — single and multi-line
//! - `Pattern("name", r"regex", score)` — Presidio positional args, multi-line
//! - `Pattern(name="...", regex=r"...")` — keyword arg variant
//! - `PATTERN = r"..."` — constant assignments
//! - Triple-quoted raw strings: `r"""..."""` and `r'''...'''`

use crate::types::{GuardrailCategory, GuardrailSeverity, RawRule, ScanDirection};
use lr_types::AppResult;
use tracing::debug;

/// Extract regex patterns from a Python source file
pub fn extract_python_patterns(
    data: &[u8],
    source_id: &str,
    file_path: &str,
    default_category: GuardrailCategory,
    default_direction: ScanDirection,
) -> AppResult<Vec<RawRule>> {
    let text = String::from_utf8_lossy(data);
    let mut rules = Vec::new();
    let mut rule_idx = 0;

    let file_stem = file_path
        .rsplit('/')
        .next()
        .unwrap_or(file_path)
        .trim_end_matches(".py");

    // Extract from whole file (handles multi-line calls)
    let patterns = extract_re_compile_all(&text)
        .into_iter()
        .chain(extract_pattern_class_all(&text))
        .chain(extract_constant_patterns(&text));

    for (name, pattern) in patterns {
        // Skip patterns that are too simple or too broad
        if pattern.len() < 5 || pattern == ".*" || pattern == ".+" {
            continue;
        }

        // Validate the regex compiles
        if regex::Regex::new(&pattern).is_err() {
            continue;
        }

        let rule_name = if name.is_empty() {
            format!("{}_{}", file_stem, rule_idx)
        } else {
            name
        };

        rules.push(RawRule {
            id: format!("{}-py-{}-{}", source_id, file_stem, rule_idx),
            name: rule_name,
            pattern,
            category: default_category.clone(),
            severity: GuardrailSeverity::Medium,
            direction: default_direction.clone(),
            description: format!("Extracted from {}", file_path),
        });

        rule_idx += 1;
    }

    debug!(
        "Extracted {} patterns from Python file '{}' for source '{}'",
        rules.len(),
        file_path,
        source_id
    );

    Ok(rules)
}

/// Extract all `re.compile(...)` patterns from the full file text.
/// Handles single-line and multi-line calls, and triple-quoted strings.
fn extract_re_compile_all(text: &str) -> Vec<(String, String)> {
    let mut results = Vec::new();
    let mut search_from = 0;

    while let Some(pos) = text[search_from..].find("re.compile(") {
        let abs_pos = search_from + pos;
        let after_open = abs_pos + "re.compile(".len();

        // Find the matching close paren, handling nested parens
        if let Some(close) = find_matching_paren(text, after_open) {
            let inner = &text[after_open..close];
            // Extract the first string argument from the inner content
            if let Some(pattern) = extract_first_string_arg(inner) {
                results.push((String::new(), pattern));
            }
            search_from = close + 1;
        } else {
            search_from = after_open;
        }
    }

    results
}

/// Extract all `Pattern(...)` calls from the full file text.
/// Handles Presidio's positional args: `Pattern("name", r"regex", score)`
/// and keyword args: `Pattern(name="...", regex=r"...")`
fn extract_pattern_class_all(text: &str) -> Vec<(String, String)> {
    let mut results = Vec::new();
    let mut search_from = 0;

    while let Some(pos) = text[search_from..].find("Pattern(") {
        let abs_pos = search_from + pos;

        // Make sure this isn't part of a longer word (e.g., "PatternX(")
        if abs_pos > 0 {
            let prev_char = text.as_bytes()[abs_pos - 1];
            if prev_char.is_ascii_alphanumeric() || prev_char == b'_' {
                search_from = abs_pos + "Pattern(".len();
                continue;
            }
        }

        let after_open = abs_pos + "Pattern(".len();

        if let Some(close) = find_matching_paren(text, after_open) {
            let inner = &text[after_open..close];

            // Try keyword args first
            let name = extract_keyword_value(inner, "name=")
                .or_else(|| extract_keyword_value(inner, "label="))
                .unwrap_or_default();
            let pattern = extract_keyword_value(inner, "regex=")
                .or_else(|| extract_keyword_value(inner, "pattern="));

            if let Some(pat) = pattern {
                results.push((name, pat));
            } else {
                // Fall back to positional args: Pattern("name", r"regex", score)
                let args = split_top_level_args(inner);
                if args.len() >= 2 {
                    let name_val = extract_first_string_arg(args[0]).unwrap_or_default();
                    if let Some(pat) = extract_first_string_arg(args[1]) {
                        results.push((name_val, pat));
                    }
                }
            }
            search_from = close + 1;
        } else {
            search_from = abs_pos + "Pattern(".len();
        }
    }

    results
}

/// Extract patterns from constant assignments like `PATTERN = r"..."`
fn extract_constant_patterns(text: &str) -> Vec<(String, String)> {
    let mut results = Vec::new();

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        if let Some(eq_pos) = trimmed.find('=') {
            // Make sure this isn't == or !=
            if eq_pos + 1 < trimmed.len() && trimmed.as_bytes()[eq_pos + 1] == b'=' {
                continue;
            }
            if eq_pos > 0 && trimmed.as_bytes()[eq_pos - 1] == b'!' {
                continue;
            }

            let lhs = trimmed[..eq_pos].trim();
            let rhs = trimmed[eq_pos + 1..].trim();

            let is_pattern_var = lhs.to_uppercase().contains("PATTERN")
                || lhs.to_uppercase().contains("REGEX")
                || lhs.to_uppercase().ends_with("_RE");

            if is_pattern_var {
                if let Some(pattern) = extract_first_string_arg(rhs) {
                    results.push((lhs.to_string(), pattern));
                }
            }
        }
    }

    results
}

/// Find the position of the matching closing paren, starting right after '('.
/// Handles nested parens, string literals, and comments.
fn find_matching_paren(text: &str, start: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut depth = 1;
    let mut i = start;

    while i < bytes.len() && depth > 0 {
        match bytes[i] {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            b'#' => {
                // Skip to end of line (Python comment)
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
                continue;
            }
            b'"' | b'\'' => {
                // Skip string literal
                let quote = bytes[i];
                let triple = i + 2 < bytes.len() && bytes[i + 1] == quote && bytes[i + 2] == quote;
                if triple {
                    i += 3;
                    while i + 2 < bytes.len() {
                        if bytes[i] == quote && bytes[i + 1] == quote && bytes[i + 2] == quote {
                            i += 3;
                            break;
                        }
                        i += 1;
                    }
                    continue;
                } else {
                    i += 1;
                    while i < bytes.len() && bytes[i] != quote && bytes[i] != b'\n' {
                        if bytes[i] == b'\\' {
                            i += 1; // skip escaped char
                        }
                        i += 1;
                    }
                    if i < bytes.len() && bytes[i] == quote {
                        i += 1;
                    }
                    continue;
                }
            }
            _ => {}
        }
        i += 1;
    }

    None
}

/// Extract the first string literal value from a Python expression.
/// Handles: r"...", r'...', r"""...""", r'''...''', "...", '...'
fn extract_first_string_arg(s: &str) -> Option<String> {
    let s = s.trim();

    // Try to find r"..." r'...' r"""...""" r'''...''' "..." '...'
    let bytes = s.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        let is_raw = i < bytes.len() && bytes[i] == b'r';
        let str_start = if is_raw { i + 1 } else { i };

        if str_start >= bytes.len() {
            break;
        }

        let quote = bytes[str_start];
        if quote != b'"' && quote != b'\'' {
            i += 1;
            continue;
        }

        // Check for triple quote
        let triple = str_start + 2 < bytes.len()
            && bytes[str_start + 1] == quote
            && bytes[str_start + 2] == quote;

        if triple {
            let content_start = str_start + 3;
            // Find closing """
            let mut j = content_start;
            while j + 2 < bytes.len() {
                if bytes[j] == quote && bytes[j + 1] == quote && bytes[j + 2] == quote {
                    let content = &s[content_start..j];
                    let content = content.trim();
                    if !content.is_empty() {
                        return Some(content.to_string());
                    }
                    break;
                }
                j += 1;
            }
            i = if j + 2 < bytes.len() { j + 3 } else { bytes.len() };
        } else {
            let content_start = str_start + 1;
            // Find closing quote (handle escape sequences)
            let mut j = content_start;
            while j < bytes.len() && bytes[j] != quote && bytes[j] != b'\n' {
                if bytes[j] == b'\\' {
                    j += 1;
                }
                j += 1;
            }
            if j < bytes.len() && bytes[j] == quote {
                let content = &s[content_start..j];
                if !content.is_empty() {
                    return Some(content.to_string());
                }
            }
            i = j + 1;
        }

        // If we found content, we already returned. If empty, keep looking.
    }

    None
}

/// Extract the string value following a keyword like `name=` or `regex=`
fn extract_keyword_value(text: &str, keyword: &str) -> Option<String> {
    let pos = text.find(keyword)?;
    let after = &text[pos + keyword.len()..];
    extract_first_string_arg(after.trim())
}

/// Split top-level comma-separated arguments (respects nested parens/strings).
fn split_top_level_args(s: &str) -> Vec<&str> {
    let mut args = Vec::new();
    let mut depth = 0;
    let mut last_start = 0;
    let bytes = s.as_bytes();
    let mut i = 0;
    let mut in_string: Option<u8> = None;
    let mut triple_string = false;

    while i < bytes.len() {
        if let Some(q) = in_string {
            if triple_string {
                if i + 2 < bytes.len() && bytes[i] == q && bytes[i + 1] == q && bytes[i + 2] == q {
                    in_string = None;
                    triple_string = false;
                    i += 3;
                    continue;
                }
            } else if bytes[i] == q && (i == 0 || bytes[i - 1] != b'\\') {
                in_string = None;
            }
            i += 1;
            continue;
        }

        match bytes[i] {
            b'"' | b'\'' => {
                let q = bytes[i];
                if i + 2 < bytes.len() && bytes[i + 1] == q && bytes[i + 2] == q {
                    in_string = Some(q);
                    triple_string = true;
                    i += 3;
                    continue;
                }
                in_string = Some(q);
            }
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => {
                if depth > 0 {
                    depth -= 1;
                }
            }
            b',' if depth == 0 => {
                args.push(&s[last_start..i]);
                last_start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }

    if last_start < s.len() {
        args.push(&s[last_start..]);
    }

    args
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_re_compile_single_line() {
        let text = r#"regex = re.compile(r"\b\d{3}-\d{2}-\d{4}\b")"#;
        let patterns = extract_re_compile_all(text);
        assert_eq!(patterns.len(), 1);
        assert_eq!(patterns[0].1, r"\b\d{3}-\d{2}-\d{4}\b");
    }

    #[test]
    fn test_re_compile_single_quotes() {
        let text = r"p = re.compile(r'\d{3}-\d{2}-\d{4}')";
        let patterns = extract_re_compile_all(text);
        assert_eq!(patterns.len(), 1);
        assert_eq!(patterns[0].1, r"\d{3}-\d{2}-\d{4}");
    }

    #[test]
    fn test_re_compile_multi_line() {
        let text = "return [\n    re.compile(\n        r\"(?:ghp|gho)_[A-Za-z0-9]{36}\"\n    ),\n]";
        let patterns = extract_re_compile_all(text);
        assert_eq!(patterns.len(), 1);
        assert_eq!(patterns[0].1, "(?:ghp|gho)_[A-Za-z0-9]{36}");
    }

    #[test]
    fn test_re_compile_triple_quoted() {
        let text = r#"re.compile(r"""(?i)\b(sk-[a-zA-Z0-9]{20}T3BlbkFJ[a-zA-Z0-9]{20})(?:['|\"|\n|\r|\s|\x60|;]|$)""")"#;
        let patterns = extract_re_compile_all(text);
        assert_eq!(patterns.len(), 1);
        assert!(patterns[0].1.contains("sk-"));
    }

    #[test]
    fn test_re_compile_triple_quoted_multi_line() {
        let text = "re.compile(\n    r\"\"\"\n    (?i)test_pattern\n    \"\"\"\n)";
        let patterns = extract_re_compile_all(text);
        assert_eq!(patterns.len(), 1);
        assert!(patterns[0].1.contains("test_pattern"));
    }

    #[test]
    fn test_pattern_class_keyword_args() {
        let text =
            r#"Pattern(name="SSN", regex=r"\b\d{3}-\d{2}-\d{4}\b", score=0.8)"#;
        let patterns = extract_pattern_class_all(text);
        assert_eq!(patterns.len(), 1);
        assert_eq!(patterns[0].0, "SSN");
        assert_eq!(patterns[0].1, r"\b\d{3}-\d{2}-\d{4}\b");
    }

    #[test]
    fn test_pattern_class_positional_args() {
        // Presidio format: Pattern("name", r"regex", score)
        let text = r#"Pattern("All Credit Cards", r"\b\d{4}[\s-]?\d{4}[\s-]?\d{4}[\s-]?\d{4}\b", 0.3)"#;
        let patterns = extract_pattern_class_all(text);
        assert_eq!(patterns.len(), 1);
        assert_eq!(patterns[0].0, "All Credit Cards");
        assert!(patterns[0].1.contains("\\d{4}"));
    }

    #[test]
    fn test_pattern_class_positional_multi_line() {
        // Presidio's actual multi-line format
        let text = "PATTERNS = [\n    Pattern(\n        \"IPv4\",\n        r\"\\b(?:25[0-5]|2[0-4]\\d)\\b\",\n        0.6,\n    ),\n]";
        let patterns = extract_pattern_class_all(text);
        assert_eq!(patterns.len(), 1);
        assert_eq!(patterns[0].0, "IPv4");
        assert!(patterns[0].1.contains("25[0-5]"));
    }

    #[test]
    fn test_constant_pattern() {
        let text = r#"SSN_PATTERN = r"\d{3}-\d{2}-\d{4}""#;
        let patterns = extract_constant_patterns(text);
        assert_eq!(patterns.len(), 1);
        assert_eq!(patterns[0].1, r"\d{3}-\d{2}-\d{4}");
    }

    #[test]
    fn test_non_pattern_constant_ignored() {
        let patterns = extract_constant_patterns("MAX_LENGTH = 100");
        assert!(patterns.is_empty());
    }

    #[test]
    fn test_extract_python_patterns_full() {
        let python_code = br#"
import re

# SSN recognizer
SSN_PATTERN = r"\b\d{3}-\d{2}-\d{4}\b"
compiled = re.compile(r"\b[A-Z]{2}\d{6}\b")

class MyRecognizer:
    patterns = [
        Pattern(name="US Phone", regex=r"\b\d{3}[-.]?\d{3}[-.]?\d{4}\b", score=0.7),
    ]
"#;

        let rules = extract_python_patterns(
            python_code,
            "test",
            "recognizers/us_ssn.py",
            GuardrailCategory::PiiLeakage,
            ScanDirection::Both,
        )
        .unwrap();

        assert!(
            rules.len() >= 2,
            "Expected at least 2 rules, got {}",
            rules.len()
        );
    }

    #[test]
    fn test_skip_simple_patterns() {
        let python_code = br#"
p = re.compile(r".*")
q = re.compile(r".+")
r_short = re.compile(r"\d")
good = re.compile(r"\b\d{3}-\d{2}-\d{4}\b")
"#;

        let rules = extract_python_patterns(
            python_code,
            "test",
            "test.py",
            GuardrailCategory::PiiLeakage,
            ScanDirection::Both,
        )
        .unwrap();

        assert_eq!(rules.len(), 1);
    }

    #[test]
    fn test_presidio_multi_line_patterns() {
        // Simulates actual Presidio credit_card_recognizer.py format
        let python_code = br#"
from presidio_analyzer import Pattern

PATTERNS = [
    Pattern(
        "All Credit Cards (weak)",
        r"\b(?:\d[ -]*?){13,16}\b",
        0.3,
    ),
    Pattern(
        "All Credit Cards (medium)",
        r"\b(?:4[0-9]{12}(?:[0-9]{3})?|5[1-5][0-9]{14})\b",
        0.5,
    ),
]
"#;

        let rules = extract_python_patterns(
            python_code,
            "test",
            "credit_card_recognizer.py",
            GuardrailCategory::PiiLeakage,
            ScanDirection::Both,
        )
        .unwrap();

        assert_eq!(rules.len(), 2, "Expected 2 credit card patterns, got {}", rules.len());
        assert!(rules[0].name.contains("Credit Cards (weak)"));
        assert!(rules[1].name.contains("Credit Cards (medium)"));
    }

    #[test]
    fn test_llm_guard_denylist_format() {
        // Simulates LLM Guard secrets plugin format
        let python_code = br#"
import re

class GitHubTokenDetector:
    @property
    def denylist(self):
        return [
            re.compile(r"(?:ghp|gho|ghu|ghs|ghr)_[A-Za-z0-9_]{36}"),
            re.compile(r"github_pat_[0-9a-zA-Z_]{82}"),
        ]
"#;

        let rules = extract_python_patterns(
            python_code,
            "test",
            "github_token.py",
            GuardrailCategory::DataLeakage,
            ScanDirection::Both,
        )
        .unwrap();

        assert_eq!(rules.len(), 2, "Expected 2 GitHub token patterns, got {}", rules.len());
        assert!(rules[0].pattern.contains("ghp"));
    }

    #[test]
    fn test_llm_guard_triple_quoted_multi_line() {
        // LLM Guard sometimes uses triple-quoted strings across lines
        let python_code = br#"
class OpenAIDetector:
    @property
    def denylist(self):
        return [
            re.compile(
                r"""(?i)\b(sk-[a-zA-Z0-9]{20}T3BlbkFJ[a-zA-Z0-9]{20})\b"""
            ),
        ]
"#;

        let rules = extract_python_patterns(
            python_code,
            "test",
            "openai_key.py",
            GuardrailCategory::DataLeakage,
            ScanDirection::Both,
        )
        .unwrap();

        assert_eq!(rules.len(), 1, "Expected 1 OpenAI key pattern, got {}", rules.len());
        assert!(rules[0].pattern.contains("sk-"));
    }
}
