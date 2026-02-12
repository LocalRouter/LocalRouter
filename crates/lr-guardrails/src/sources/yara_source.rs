//! YARA rule file parser
//!
//! Extracts regex patterns from YARA .yar files for use as guardrail rules.
//! This is a simplified parser that extracts string definitions and condition patterns.

use crate::types::{GuardrailCategory, GuardrailSeverity, RawRule, ScanDirection};
use lr_types::AppResult;
use tracing::debug;

/// Parse YARA rules from a .yar file and extract regex patterns
///
/// YARA files have this structure:
/// ```yara
/// rule rule_name {
///     meta:
///         description = "..."
///         severity = "high"
///     strings:
///         $s1 = "literal string"
///         $s2 = /regex pattern/
///     condition:
///         any of them
/// }
/// ```
///
/// We extract:
/// - String literals → escaped regex
/// - Regex patterns → direct regex
/// - Meta fields → severity, description, category
pub fn parse_yara_rules(
    data: &[u8],
    source_id: &str,
    default_category: GuardrailCategory,
    default_direction: ScanDirection,
) -> AppResult<Vec<RawRule>> {
    let text = String::from_utf8_lossy(data);
    let mut rules = Vec::new();
    let mut rule_idx = 0;

    // Simple state machine parser
    let mut lines = text.lines().peekable();

    while let Some(line) = lines.next() {
        let trimmed = line.trim();

        // Look for rule declarations
        if trimmed.starts_with("rule ") && trimmed.contains('{') {
            let rule_name = trimmed
                .strip_prefix("rule ")
                .and_then(|s| s.split_whitespace().next())
                .unwrap_or("unknown")
                .to_string();

            let mut description = String::new();
            let mut severity = GuardrailSeverity::Medium;
            let mut patterns: Vec<String> = Vec::new();

            // Parse rule body
            let mut in_meta = false;
            let mut in_strings = false;

            for inner_line in lines.by_ref() {
                let inner = inner_line.trim();

                if inner == "}" {
                    break; // End of rule
                }

                if inner.starts_with("meta:") {
                    in_meta = true;
                    in_strings = false;
                    continue;
                }
                if inner.starts_with("strings:") {
                    in_meta = false;
                    in_strings = true;
                    continue;
                }
                if inner.starts_with("condition:") {
                    in_meta = false;
                    in_strings = false;
                    continue;
                }

                if in_meta {
                    if let Some(desc) = extract_meta_string(inner, "description") {
                        description = desc;
                    }
                    if let Some(sev) = extract_meta_string(inner, "severity") {
                        severity = GuardrailSeverity::from_str_lenient(&sev);
                    }
                }

                if in_strings {
                    if let Some(pattern) = extract_yara_string(inner) {
                        patterns.push(pattern);
                    }
                }
            }

            // Create rules from extracted patterns
            for (pat_idx, pattern) in patterns.into_iter().enumerate() {
                rules.push(RawRule {
                    id: format!("{}-{}-{}", source_id, rule_idx, pat_idx),
                    name: rule_name.clone(),
                    pattern,
                    category: default_category.clone(),
                    severity,
                    direction: default_direction.clone(),
                    description: description.clone(),
                });
            }

            rule_idx += 1;
        }
    }

    debug!(
        "Parsed {} rules from YARA source '{}'",
        rules.len(),
        source_id
    );
    Ok(rules)
}

/// Extract a meta field value from a YARA meta line
fn extract_meta_string(line: &str, key: &str) -> Option<String> {
    let line = line.trim();
    if line.starts_with(key) {
        // Format: key = "value"
        if let Some(pos) = line.find('=') {
            let value = line[pos + 1..].trim();
            let value = value.trim_matches('"');
            return Some(value.to_string());
        }
    }
    None
}

/// Extract a regex or literal pattern from a YARA strings line
fn extract_yara_string(line: &str) -> Option<String> {
    let line = line.trim();

    // Skip comments
    if line.starts_with("//") || line.starts_with("/*") {
        return None;
    }

    // Format: $identifier = "literal" or $identifier = /regex/
    if !line.starts_with('$') {
        return None;
    }

    if let Some(eq_pos) = line.find('=') {
        let value_part = line[eq_pos + 1..].trim();

        // Regex pattern: /pattern/ or /pattern/i
        if let Some(stripped) = value_part.strip_prefix('/') {
            // Find closing /
            if let Some(end_slash) = stripped.rfind('/') {
                let pattern = &stripped[..end_slash];
                if !pattern.is_empty() {
                    // Check for case-insensitive flag
                    let flags = &stripped[end_slash + 1..];
                    if flags.contains('i') {
                        return Some(format!("(?i){}", pattern));
                    }
                    return Some(pattern.to_string());
                }
            }
        }

        // Literal string: "text" [nocase]
        if let Some(stripped) = value_part.strip_prefix('"') {
            // Find closing quote
            if let Some(close_quote) = stripped.find('"') {
                let value = &stripped[..close_quote];
                let after_quote = &stripped[close_quote + 1..];
                if !value.is_empty() {
                    if after_quote.contains("nocase") {
                        return Some(format!("(?i){}", regex::escape(value)));
                    }
                    return Some(regex::escape(value));
                }
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_yara() {
        let yara = br#"
rule detect_injection {
    meta:
        description = "Detects prompt injection"
        severity = "high"
    strings:
        $s1 = "ignore previous" nocase
        $s2 = /override\s+safety/i
    condition:
        any of them
}
"#;

        let rules = parse_yara_rules(
            yara,
            "test",
            GuardrailCategory::PromptInjection,
            ScanDirection::Input,
        )
        .unwrap();

        assert_eq!(rules.len(), 2);
        assert!(rules[0].pattern.contains("(?i)"));
        assert!(rules[1].pattern.contains("(?i)"));
    }

    #[test]
    fn test_extract_meta_string() {
        assert_eq!(
            extract_meta_string(r#"description = "test desc""#, "description"),
            Some("test desc".to_string())
        );
        assert_eq!(
            extract_meta_string(r#"severity = "critical""#, "severity"),
            Some("critical".to_string())
        );
        assert_eq!(extract_meta_string("unrelated = 42", "description"), None);
    }

    #[test]
    fn test_extract_yara_regex() {
        assert_eq!(
            extract_yara_string(r#"$re = /test\s+pattern/i"#),
            Some("(?i)test\\s+pattern".to_string())
        );
    }

    #[test]
    fn test_extract_yara_literal() {
        assert_eq!(
            extract_yara_string(r#"$s1 = "hello world""#),
            Some("hello world".to_string())
        );
    }
}
