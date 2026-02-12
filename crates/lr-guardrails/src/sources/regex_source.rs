//! Generic regex pattern file parser
//!
//! Parses JSON files containing regex patterns from sources like
//! Presidio, PayloadsAllTheThings, and LLM Guard.

use crate::types::{GuardrailCategory, GuardrailSeverity, RawRule, ScanDirection};
use lr_types::AppResult;

/// Parse rules from a JSON array of pattern objects
///
/// Expected format:
/// ```json
/// [
///   {
///     "id": "rule-id",
///     "name": "Rule Name",
///     "pattern": "regex pattern",
///     "category": "prompt_injection",
///     "severity": "high",
///     "direction": "input",
///     "description": "What this detects"
///   }
/// ]
/// ```
pub fn parse_regex_json(data: &[u8], source_id: &str) -> AppResult<Vec<RawRule>> {
    let entries: Vec<serde_json::Value> = serde_json::from_slice(data).map_err(|e| {
        lr_types::AppError::Internal(format!(
            "Failed to parse regex rules JSON from {}: {}",
            source_id, e
        ))
    })?;

    let mut rules = Vec::new();

    for (i, entry) in entries.iter().enumerate() {
        let id = entry
            .get("id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("{}-{}", source_id, i));

        let name = entry
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("Unnamed Rule")
            .to_string();

        let pattern = match entry.get("pattern").and_then(|v| v.as_str()) {
            Some(p) if !p.is_empty() => p.to_string(),
            _ => continue, // skip entries without patterns
        };

        let category = entry
            .get("category")
            .and_then(|v| v.as_str())
            .map(parse_category)
            .unwrap_or(GuardrailCategory::PromptInjection);

        let severity = entry
            .get("severity")
            .and_then(|v| v.as_str())
            .map(GuardrailSeverity::from_str_lenient)
            .unwrap_or(GuardrailSeverity::Medium);

        let direction = entry
            .get("direction")
            .and_then(|v| v.as_str())
            .map(parse_direction)
            .unwrap_or(ScanDirection::Both);

        let description = entry
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        rules.push(RawRule {
            id,
            name,
            pattern,
            category,
            severity,
            direction,
            description,
        });
    }

    Ok(rules)
}

/// Parse a plain text file with one regex pattern per line
///
/// Lines starting with # are comments. Empty lines are skipped.
/// Each pattern becomes an input-direction rule.
pub fn parse_pattern_list(
    data: &[u8],
    source_id: &str,
    category: GuardrailCategory,
    severity: GuardrailSeverity,
    direction: ScanDirection,
) -> AppResult<Vec<RawRule>> {
    let text = String::from_utf8_lossy(data);
    let mut rules = Vec::new();

    for (i, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        rules.push(RawRule {
            id: format!("{}-{}", source_id, i),
            name: format!("Pattern {}", i + 1),
            pattern: regex::escape(line),
            category: category.clone(),
            severity,
            direction: direction.clone(),
            description: String::new(),
        });
    }

    Ok(rules)
}

fn parse_category(s: &str) -> GuardrailCategory {
    match s.to_lowercase().as_str() {
        "prompt_injection" | "injection" => GuardrailCategory::PromptInjection,
        "jailbreak" | "jailbreak_attempt" => GuardrailCategory::JailbreakAttempt,
        "pii" | "pii_leakage" => GuardrailCategory::PiiLeakage,
        "code_injection" | "code" | "sql" | "xss" => GuardrailCategory::CodeInjection,
        "encoded" | "encoded_payload" | "obfuscation" => GuardrailCategory::EncodedPayload,
        "sensitive_data" | "sensitive" => GuardrailCategory::SensitiveData,
        "malicious_output" | "malicious" => GuardrailCategory::MaliciousOutput,
        "data_leakage" | "leakage" | "exfiltration" => GuardrailCategory::DataLeakage,
        _ => GuardrailCategory::PromptInjection,
    }
}

fn parse_direction(s: &str) -> ScanDirection {
    match s.to_lowercase().as_str() {
        "input" | "request" => ScanDirection::Input,
        "output" | "response" => ScanDirection::Output,
        "both" | "all" => ScanDirection::Both,
        _ => ScanDirection::Both,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_regex_json() {
        let json = serde_json::json!([
            {
                "id": "test-1",
                "name": "Test Rule",
                "pattern": "(?i)test\\s+pattern",
                "category": "prompt_injection",
                "severity": "high",
                "direction": "input",
                "description": "A test rule"
            },
            {
                "id": "test-2",
                "name": "PII Rule",
                "pattern": "\\d{3}-\\d{2}-\\d{4}",
                "category": "pii",
                "severity": "critical",
                "direction": "both",
                "description": "SSN pattern"
            }
        ]);

        let data = serde_json::to_vec(&json).unwrap();
        let rules = parse_regex_json(&data, "test").unwrap();

        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0].id, "test-1");
        assert_eq!(rules[0].severity, GuardrailSeverity::High);
        assert_eq!(rules[1].category, GuardrailCategory::PiiLeakage);
        assert_eq!(rules[1].direction, ScanDirection::Both);
    }

    #[test]
    fn test_parse_pattern_list() {
        let data = b"# Comment line\n\nignore previous instructions\nhack the system\n";
        let rules = parse_pattern_list(
            data,
            "test",
            GuardrailCategory::PromptInjection,
            GuardrailSeverity::High,
            ScanDirection::Input,
        )
        .unwrap();

        assert_eq!(rules.len(), 2);
        assert!(rules[0].pattern.contains("ignore previous instructions"));
    }

    #[test]
    fn test_parse_skips_empty_patterns() {
        let json = serde_json::json!([
            {"name": "Good", "pattern": "test"},
            {"name": "Empty", "pattern": ""},
            {"name": "Missing"}
        ]);

        let data = serde_json::to_vec(&json).unwrap();
        let rules = parse_regex_json(&data, "test").unwrap();
        assert_eq!(rules.len(), 1);
    }
}
