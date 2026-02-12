//! Compiled rule set wrapping regex::RegexSet for fast matching

use regex::RegexSet;
use tracing::warn;

use crate::types::{GuardrailCategory, GuardrailSeverity, RawRule, ScanDirection};

/// Metadata for a single compiled rule (indexed alongside RegexSet patterns)
#[derive(Debug, Clone)]
pub struct RuleMetadata {
    pub id: String,
    pub name: String,
    pub source_id: String,
    pub source_label: String,
    pub category: GuardrailCategory,
    pub severity: GuardrailSeverity,
    pub direction: ScanDirection,
    pub description: String,
    /// Original pattern string (for extracting matched text)
    pub pattern: String,
}

/// A compiled set of rules from a single source, optimized for fast matching
#[derive(Debug)]
pub struct CompiledRuleSet {
    /// Source identifier
    pub source_id: String,
    /// Source display name
    pub source_label: String,
    /// Compiled regex set for input rules
    pub input_regex_set: Option<RegexSet>,
    /// Compiled regex set for output rules
    pub output_regex_set: Option<RegexSet>,
    /// Metadata for input rules (indexed to match input_regex_set)
    pub input_metadata: Vec<RuleMetadata>,
    /// Metadata for output rules (indexed to match output_regex_set)
    pub output_metadata: Vec<RuleMetadata>,
    /// Total number of rules in this set
    pub rule_count: usize,
}

impl CompiledRuleSet {
    /// Compile a list of raw rules into an optimized rule set
    pub fn compile(source_id: &str, source_label: &str, rules: &[RawRule]) -> Self {
        let mut input_patterns = Vec::new();
        let mut input_metadata = Vec::new();
        let mut output_patterns = Vec::new();
        let mut output_metadata = Vec::new();

        for rule in rules {
            let meta = RuleMetadata {
                id: rule.id.clone(),
                name: rule.name.clone(),
                source_id: source_id.to_string(),
                source_label: source_label.to_string(),
                category: rule.category.clone(),
                severity: rule.severity,
                direction: rule.direction.clone(),
                description: rule.description.clone(),
                pattern: rule.pattern.clone(),
            };

            if rule.direction.matches_input() {
                input_patterns.push(rule.pattern.clone());
                input_metadata.push(meta.clone());
            }
            if rule.direction.matches_output() {
                output_patterns.push(rule.pattern.clone());
                output_metadata.push(meta);
            }
        }

        let input_regex_set = if input_patterns.is_empty() {
            None
        } else {
            match RegexSet::new(&input_patterns) {
                Ok(set) => Some(set),
                Err(e) => {
                    warn!(
                        "Failed to compile input regex set for source '{}': {}",
                        source_id, e
                    );
                    // Try compiling individual patterns, skip broken ones
                    let (valid_patterns, valid_meta) =
                        filter_valid_patterns(&input_patterns, &input_metadata);
                    input_metadata = valid_meta;
                    if valid_patterns.is_empty() {
                        None
                    } else {
                        RegexSet::new(&valid_patterns).ok()
                    }
                }
            }
        };

        let output_regex_set = if output_patterns.is_empty() {
            None
        } else {
            match RegexSet::new(&output_patterns) {
                Ok(set) => Some(set),
                Err(e) => {
                    warn!(
                        "Failed to compile output regex set for source '{}': {}",
                        source_id, e
                    );
                    let (valid_patterns, valid_meta) =
                        filter_valid_patterns(&output_patterns, &output_metadata);
                    output_metadata = valid_meta;
                    if valid_patterns.is_empty() {
                        None
                    } else {
                        RegexSet::new(&valid_patterns).ok()
                    }
                }
            }
        };

        let rule_count = input_metadata.len() + output_metadata.len();

        Self {
            source_id: source_id.to_string(),
            source_label: source_label.to_string(),
            input_regex_set,
            output_regex_set,
            input_metadata,
            output_metadata,
            rule_count,
        }
    }
}

/// Filter out patterns that fail to compile individually
fn filter_valid_patterns(
    patterns: &[String],
    metadata: &[RuleMetadata],
) -> (Vec<String>, Vec<RuleMetadata>) {
    let mut valid_patterns = Vec::new();
    let mut valid_metadata = Vec::new();

    for (pattern, meta) in patterns.iter().zip(metadata.iter()) {
        match regex::Regex::new(pattern) {
            Ok(_) => {
                valid_patterns.push(pattern.clone());
                valid_metadata.push(meta.clone());
            }
            Err(e) => {
                warn!(
                    "Skipping invalid regex pattern '{}' from rule '{}': {}",
                    pattern, meta.id, e
                );
            }
        }
    }

    (valid_patterns, valid_metadata)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::*;

    #[test]
    fn test_compile_empty() {
        let set = CompiledRuleSet::compile("test", "Test", &[]);
        assert_eq!(set.rule_count, 0);
        assert!(set.input_regex_set.is_none());
        assert!(set.output_regex_set.is_none());
    }

    #[test]
    fn test_compile_input_rules() {
        let rules = vec![RawRule {
            id: "test-1".to_string(),
            name: "Test Rule".to_string(),
            pattern: r"(?i)ignore\s+previous\s+instructions".to_string(),
            category: GuardrailCategory::PromptInjection,
            severity: GuardrailSeverity::High,
            direction: ScanDirection::Input,
            description: "Prompt injection attempt".to_string(),
        }];

        let set = CompiledRuleSet::compile("test", "Test", &rules);
        assert_eq!(set.rule_count, 1);
        assert!(set.input_regex_set.is_some());
        assert!(set.output_regex_set.is_none());

        let regex_set = set.input_regex_set.unwrap();
        assert!(regex_set.is_match("please ignore previous instructions"));
        assert!(!regex_set.is_match("normal message"));
    }

    #[test]
    fn test_compile_both_direction() {
        let rules = vec![RawRule {
            id: "test-1".to_string(),
            name: "SSN Pattern".to_string(),
            pattern: r"\d{3}-\d{2}-\d{4}".to_string(),
            category: GuardrailCategory::PiiLeakage,
            severity: GuardrailSeverity::Critical,
            direction: ScanDirection::Both,
            description: "Social Security Number".to_string(),
        }];

        let set = CompiledRuleSet::compile("test", "Test", &rules);
        // Both direction rules appear in both sets
        assert_eq!(set.rule_count, 2);
        assert!(set.input_regex_set.is_some());
        assert!(set.output_regex_set.is_some());
    }

    #[test]
    fn test_compile_with_invalid_pattern() {
        let rules = vec![
            RawRule {
                id: "valid".to_string(),
                name: "Valid".to_string(),
                pattern: r"test".to_string(),
                category: GuardrailCategory::PromptInjection,
                severity: GuardrailSeverity::Low,
                direction: ScanDirection::Input,
                description: "Valid".to_string(),
            },
            RawRule {
                id: "invalid".to_string(),
                name: "Invalid".to_string(),
                pattern: r"[invalid".to_string(),
                category: GuardrailCategory::PromptInjection,
                severity: GuardrailSeverity::Low,
                direction: ScanDirection::Input,
                description: "Invalid regex".to_string(),
            },
        ];

        let set = CompiledRuleSet::compile("test", "Test", &rules);
        // Should still compile with valid rules
        assert!(set.input_regex_set.is_some());
        assert_eq!(set.input_metadata.len(), 1);
        assert_eq!(set.input_metadata[0].id, "valid");
    }
}
