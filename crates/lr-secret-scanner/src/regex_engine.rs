//! Regex-based secret detection engine with keyword pre-filtering

use aho_corasick::AhoCorasick;
use tracing::warn;

use crate::entropy::shannon_entropy;
use crate::patterns::builtin::{builtin_patterns, PatternDef};
use crate::types::{ExtractedText, SecretFinding, SecretRule};

/// Metadata about a compiled rule (for UI display)
#[derive(Debug, Clone, serde::Serialize)]
pub struct RuleMetadata {
    pub id: String,
    pub description: String,
    pub regex: String,
    pub category: String,
    pub entropy_threshold: Option<f32>,
    pub keywords: Vec<String>,
}

/// Compiled regex engine with keyword pre-filtering
pub struct RegexEngine {
    /// All compiled rules
    rules: Vec<SecretRule>,
    /// Aho-Corasick automaton for keyword pre-filtering
    /// Maps keyword index -> list of rule indices that use that keyword
    keyword_automaton: Option<AhoCorasick>,
    keyword_to_rules: Vec<Vec<usize>>,
    /// Global entropy threshold
    entropy_threshold: f32,
    /// Compiled allowlist patterns
    allowlist: Vec<regex::Regex>,
}

impl RegexEngine {
    /// Build a new regex engine from builtin patterns
    pub fn new(
        entropy_threshold: f32,
        allowlist: &[String],
    ) -> Result<Self, String> {
        let mut rules = Vec::new();
        let mut all_keywords: Vec<String> = Vec::new();
        let mut keyword_to_rules: Vec<Vec<usize>> = Vec::new();

        // Compile builtin patterns
        for pattern in builtin_patterns() {
            match Self::compile_pattern_def(&pattern) {
                Ok(rule) => {
                    let rule_idx = rules.len();
                    for kw in pattern.keywords {
                        let kw_lower = (*kw).to_lowercase();
                        if let Some(pos) = all_keywords.iter().position(|k| k == &kw_lower) {
                            keyword_to_rules[pos].push(rule_idx);
                        } else {
                            all_keywords.push(kw_lower);
                            keyword_to_rules.push(vec![rule_idx]);
                        }
                    }
                    rules.push(rule);
                }
                Err(e) => {
                    warn!("Failed to compile builtin pattern '{}': {}", pattern.id, e);
                }
            }
        }

        // Build Aho-Corasick automaton for keyword pre-filtering
        let keyword_automaton = if all_keywords.is_empty() {
            None
        } else {
            match AhoCorasick::builder()
                .ascii_case_insensitive(true)
                .build(&all_keywords)
            {
                Ok(ac) => Some(ac),
                Err(e) => {
                    warn!("Failed to build keyword automaton: {}", e);
                    None
                }
            }
        };

        // Compile allowlist patterns
        let mut compiled_allowlist = Vec::new();
        for pattern in allowlist {
            match regex::Regex::new(pattern) {
                Ok(r) => compiled_allowlist.push(r),
                Err(e) => {
                    warn!("Failed to compile allowlist pattern '{}': {}", pattern, e);
                }
            }
        }

        Ok(Self {
            rules,
            keyword_automaton,
            keyword_to_rules,
            entropy_threshold,
            allowlist: compiled_allowlist,
        })
    }

    fn compile_pattern_def(pattern: &PatternDef) -> Result<SecretRule, String> {
        let compiled = regex::Regex::new(pattern.regex)
            .map_err(|e| format!("Regex compile error for '{}': {}", pattern.id, e))?;

        Ok(SecretRule {
            id: pattern.id.to_string(),
            description: pattern.description.to_string(),
            compiled_regex: compiled,
            secret_group: pattern.secret_group,
            entropy_threshold: pattern.entropy,
            keywords: pattern.keywords.iter().map(|s| s.to_string()).collect(),
            category: pattern.category.clone(),
        })
    }

    /// Number of compiled rules
    pub fn rule_count(&self) -> usize {
        self.rules.len()
    }

    /// Get metadata about all compiled rules (for display in UI)
    pub fn rule_metadata(&self) -> Vec<RuleMetadata> {
        self.rules
            .iter()
            .map(|r| RuleMetadata {
                id: r.id.clone(),
                description: r.description.clone(),
                regex: r.compiled_regex.as_str().to_string(),
                category: r.category.to_string(),
                entropy_threshold: r.entropy_threshold,
                keywords: r.keywords.clone(),
            })
            .collect()
    }

    /// Scan extracted texts for secrets.
    /// If `force_entropy_threshold` is Some, it overrides both the global and per-rule thresholds.
    pub fn scan(
        &self,
        texts: &[ExtractedText],
        force_entropy_threshold: Option<f32>,
    ) -> Vec<SecretFinding> {
        let mut findings = Vec::new();

        for text in texts {
            let text_lower = text.text.to_lowercase();

            // Determine which rules are candidates via keyword pre-filter
            let candidate_rules: Vec<usize> = if let Some(ref ac) = self.keyword_automaton {
                let mut candidates = std::collections::HashSet::new();
                for mat in ac.find_iter(&text_lower) {
                    if let Some(rule_indices) = self.keyword_to_rules.get(mat.pattern().as_usize())
                    {
                        for &idx in rule_indices {
                            candidates.insert(idx);
                        }
                    }
                }
                // Also include rules with no keywords (they always run)
                for (idx, rule) in self.rules.iter().enumerate() {
                    if rule.keywords.is_empty() {
                        candidates.insert(idx);
                    }
                }
                candidates.into_iter().collect()
            } else {
                // No keyword automaton: all rules are candidates
                (0..self.rules.len()).collect()
            };

            // Run candidate rules
            for rule_idx in candidate_rules {
                let rule = &self.rules[rule_idx];

                for captures in rule.compiled_regex.captures_iter(&text.text) {
                    let matched = if rule.secret_group > 0 {
                        captures
                            .get(rule.secret_group)
                            .map(|m| m.as_str())
                            .unwrap_or("")
                    } else {
                        captures.get(0).map(|m| m.as_str()).unwrap_or("")
                    };

                    if matched.is_empty() {
                        continue;
                    }

                    // Check entropy threshold
                    let entropy = shannon_entropy(matched);
                    let threshold = force_entropy_threshold.unwrap_or_else(|| {
                        rule.entropy_threshold.unwrap_or(self.entropy_threshold)
                    });
                    if entropy < threshold {
                        continue;
                    }

                    // Check allowlist
                    if self.is_allowlisted(matched) {
                        continue;
                    }

                    // Truncate matched text for display
                    let display_text = truncate_matched_text(matched, 40);

                    findings.push(SecretFinding {
                        rule_id: rule.id.clone(),
                        rule_description: rule.description.clone(),
                        category: rule.category.to_string(),
                        regex_pattern: rule.compiled_regex.as_str().to_string(),
                        keywords: rule.keywords.clone(),
                        rule_entropy_threshold: rule.entropy_threshold,
                        message_index: text.message_index,
                        matched_text: display_text,
                        entropy,
                    });
                }
            }
        }

        findings
    }

    /// Check if a matched text is excluded by the allowlist
    fn is_allowlisted(&self, text: &str) -> bool {
        self.allowlist.iter().any(|r| r.is_match(text))
    }
}

/// Truncate matched text for safe display, masking the middle
fn truncate_matched_text(text: &str, max_len: usize) -> String {
    // Need at least 10 chars (6 prefix + 4 suffix) to mask the middle
    if text.len() < 10 {
        return text.to_string();
    }
    let prefix = &text[..6];
    let suffix = &text[text.len() - 4..];
    if text.len() <= max_len {
        // Show first 6 chars, mask middle, show last 4 chars
        let masked_len = text.len() - 10;
        format!("{}{}{}", prefix, "*".repeat(masked_len.min(20)), suffix)
    } else {
        // Longer than max_len: show prefix, char count, suffix
        format!("{}...({} chars)...{}", prefix, text.len(), suffix)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_text(text: &str) -> Vec<ExtractedText> {
        vec![ExtractedText {
            label: "test".to_string(),
            text: text.to_string(),
            message_index: 0,
        }]
    }

    #[test]
    fn test_empty_engine() {
        let engine = RegexEngine::new(3.5, &[]).unwrap();
        assert!(engine.rule_count() > 0); // builtin rules
    }

    #[test]
    fn test_aws_key_detection() {
        let engine = RegexEngine::new(3.0, &[]).unwrap();
        let texts = make_text("My AWS key is AKIAIOSFODNN7EXAMPLE and it works");
        let findings = engine.scan(&texts, None);
        assert!(
            findings.iter().any(|f| f.rule_id == "aws-access-key-id"),
            "Should detect AWS key. Findings: {:?}",
            findings
        );
    }

    #[test]
    fn test_github_pat_detection() {
        let engine = RegexEngine::new(3.0, &[]).unwrap();
        let texts = make_text("token: ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghij");
        let findings = engine.scan(&texts, None);
        assert!(
            findings.iter().any(|f| f.rule_id == "github-pat"),
            "Should detect GitHub PAT. Findings: {:?}",
            findings
        );
    }

    #[test]
    fn test_allowlist_excludes() {
        let allowlist = vec!["AKIAIOSFODNN7EXAMPLE".to_string()];
        let engine = RegexEngine::new(3.0, &allowlist).unwrap();
        let texts = make_text("My AWS key is AKIAIOSFODNN7EXAMPLE");
        let findings = engine.scan(&texts, None);
        assert!(
            !findings.iter().any(|f| f.rule_id == "aws-access-key-id"),
            "Allowlisted key should not be detected"
        );
    }

    #[test]
    fn test_truncate_matched_text() {
        assert_eq!(truncate_matched_text("short", 40), "short");
        let long = "AKIAIOSFODNN7EXAMPLEKEY";
        let truncated = truncate_matched_text(long, 40);
        assert!(truncated.contains("AKIAIO"));
        assert!(truncated.contains("EKEY"));
    }
}
