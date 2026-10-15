//! Regex-based secret detection engine with keyword pre-filtering

use std::collections::{HashMap, HashSet};

use aho_corasick::AhoCorasick;
use tracing::warn;

use crate::entropy::shannon_entropy;
use crate::filters::is_placeholder;
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
    pub fn new(entropy_threshold: f32, allowlist: &[String]) -> Result<Self, String> {
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
            entropy_group: pattern.entropy_group,
            entropy_threshold: pattern.entropy,
            keywords: pattern.keywords.iter().map(|s| s.to_string()).collect(),
            category: pattern.category.clone(),
            placeholder: pattern.placeholder,
            validate: pattern.validate,
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
    ///
    /// Each secret value is reported once, however many times it occurs:
    /// agents resend the whole conversation on every turn, so one value
    /// routinely appears in several places of a request (a tool call, its
    /// output, a transcript). Within one text, the first rule in priority
    /// order to match a span claims it, so a vendor token inside a generic
    /// `token = "..."` assignment is reported once, under the vendor rule.
    pub fn scan(
        &self,
        texts: &[ExtractedText],
        force_entropy_threshold: Option<f32>,
    ) -> Vec<SecretFinding> {
        let mut findings: Vec<SecretFinding> = Vec::new();
        // Priority (rule index) of the rule behind each finding.
        let mut ranks: Vec<usize> = Vec::new();
        let mut by_value: HashMap<String, usize> = HashMap::new();

        for text in texts {
            for (rank, finding) in self.scan_text(text, force_entropy_threshold) {
                match by_value.get(&finding.raw_text) {
                    Some(&i) => {
                        // Same value seen before: keep the first location,
                        // but report it under the more specific rule.
                        if rank < ranks[i] {
                            let message_index = findings[i].message_index;
                            findings[i] = SecretFinding {
                                message_index,
                                ..finding
                            };
                            ranks[i] = rank;
                        }
                    }
                    None => {
                        by_value.insert(finding.raw_text.clone(), findings.len());
                        findings.push(finding);
                        ranks.push(rank);
                    }
                }
            }
        }

        findings
    }

    /// Findings in one text, in text order, each paired with its rule's
    /// priority.
    fn scan_text(
        &self,
        text: &ExtractedText,
        force_entropy_threshold: Option<f32>,
    ) -> Vec<(usize, SecretFinding)> {
        // Determine which rules are candidates via keyword pre-filter. The
        // automaton is ASCII case-insensitive, so no lowercased copy is needed.
        let mut candidate_rules: Vec<usize> = if let Some(ref ac) = self.keyword_automaton {
            let mut candidates = HashSet::new();
            // Prefixes overlap (e.g. sk-, sk-proj-, sk-ant-api03-).
            // Non-overlapping matching would suppress the more specific rule.
            for mat in ac.find_overlapping_iter(&text.text) {
                if let Some(rule_indices) = self.keyword_to_rules.get(mat.pattern().as_usize()) {
                    candidates.extend(rule_indices.iter().copied());
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
        // Rule order is priority order: specific rules claim spans first.
        candidate_rules.sort_unstable();

        let mut claimed: Vec<(usize, usize)> = Vec::new();
        let mut accepted: Vec<(usize, usize, SecretFinding)> = Vec::new();

        for rule_idx in candidate_rules {
            let rule = &self.rules[rule_idx];
            for captures in rule.compiled_regex.captures_iter(&text.text) {
                let Some(secret) = captures.get(rule.secret_group) else {
                    continue;
                };
                if secret.as_str().is_empty() {
                    continue;
                }
                let (start, end) = (secret.start(), secret.end());
                if claimed.iter().any(|&(s, e)| start < e && s < end) {
                    continue;
                }
                // Claimed even when the checks below reject it: the specific
                // rule's verdict (e.g. "documentation placeholder") stands, and
                // a looser generic rule must not report the same span.
                claimed.push((start, end));

                if let Some(finding) = self.evaluate(
                    rule,
                    &captures,
                    secret.as_str(),
                    text,
                    force_entropy_threshold,
                ) {
                    accepted.push((start, rule_idx, finding));
                }
            }
        }

        accepted.sort_by_key(|(start, ..)| *start);
        accepted
            .into_iter()
            .map(|(_, rank, finding)| (rank, finding))
            .collect()
    }

    /// Apply a rule's entropy gate, placeholder filter, validator and the
    /// allowlist to one match.
    fn evaluate(
        &self,
        rule: &SecretRule,
        captures: &regex::Captures<'_>,
        secret: &str,
        text: &ExtractedText,
        force_entropy_threshold: Option<f32>,
    ) -> Option<SecretFinding> {
        // Entropy of the secret part only — never of surrounding text.
        let entropy_source = rule
            .entropy_group
            .and_then(|g| captures.get(g))
            .map_or(secret, |m| m.as_str());
        let entropy = shannon_entropy(entropy_source);
        let threshold = force_entropy_threshold
            .unwrap_or_else(|| rule.entropy_threshold.unwrap_or(self.entropy_threshold));
        if entropy < threshold {
            return None;
        }

        if is_placeholder(secret, rule.placeholder) {
            return None;
        }
        if rule.validate.is_some_and(|validate| !validate(captures)) {
            return None;
        }

        if self.is_allowlisted(secret) {
            return None;
        }

        Some(SecretFinding {
            rule_id: rule.id.clone(),
            rule_description: rule.description.clone(),
            category: rule.category.to_string(),
            regex_pattern: rule.compiled_regex.as_str().to_string(),
            keywords: rule.keywords.clone(),
            rule_entropy_threshold: rule.entropy_threshold,
            message_index: text.message_index,
            matched_text: truncate_matched_text(secret, 40),
            raw_text: secret.to_string(),
            entropy,
        })
    }

    /// Check if a matched text is excluded by the allowlist
    fn is_allowlisted(&self, text: &str) -> bool {
        self.allowlist.iter().any(|r| r.is_match(text))
    }
}

/// Truncate matched text for safe display, masking the middle
fn truncate_matched_text(text: &str, max_len: usize) -> String {
    let count = text.chars().count();
    // Slice at character boundaries: generic password rules can match Unicode.
    if count < 10 {
        return text.to_string();
    }
    let prefix: String = text.chars().take(6).collect();
    let suffix: String = text.chars().skip(count - 4).collect();
    if count <= max_len {
        format!("{}{}{}", prefix, "*".repeat((count - 10).min(20)), suffix)
    } else {
        format!("{}...({} chars)...{}", prefix, count, suffix)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Random material for realistic fixtures. Counting sequences like
    /// `abcdef...` or `A1b2C3...` are treated as documentation placeholders.
    const RANDOM_120: &str = "6nmCEa00cbNmH0B4zKPSYOPP5z1oYFR26doJldQrewR7prNzl2bcS0875ZaICBk4aPNQCFdhUe2mIc9CJ4WnEbD86JlLrT00pyi78T0igtccyGw2pANSDDUa";

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
        let texts = make_text("My AWS key is AKIAZ7Q3VJ2M4XK9PLR6 and it works");
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
        let texts = make_text("token: ghp_q8Zr2LmX7vKp4NwT9sYb3HdJ6fGc1eRa0uVi");
        let findings = engine.scan(&texts, None);
        assert!(
            findings.iter().any(|f| f.rule_id == "github-pat"),
            "Should detect GitHub PAT. Findings: {:?}",
            findings
        );
    }

    #[test]
    fn test_allowlist_excludes() {
        let allowlist = vec!["AKIAZ7Q3VJ2M4XK9PLR6".to_string()];
        let engine = RegexEngine::new(3.0, &allowlist).unwrap();
        let texts = make_text("My AWS key is AKIAZ7Q3VJ2M4XK9PLR6");
        let findings = engine.scan(&texts, None);
        assert!(
            !findings.iter().any(|f| f.rule_id == "aws-access-key-id"),
            "Allowlisted key should not be detected"
        );
    }

    /// Findings carry the plaintext for the popup's reveal button, but it must
    /// never serialize — findings are persisted into monitor events.
    #[test]
    fn test_raw_text_captured_but_not_serialized() {
        let engine = RegexEngine::new(3.0, &[]).unwrap();
        let texts = make_text("my key is AKIAZ7Q3VJ2M4XK9PLR6");
        let findings = engine.scan(&texts, None);
        let finding = findings
            .iter()
            .find(|f| f.rule_id == "aws-access-key-id")
            .expect("AWS key detected");

        assert_eq!(finding.raw_text, "AKIAZ7Q3VJ2M4XK9PLR6");
        assert_ne!(finding.matched_text, finding.raw_text, "preview is masked");

        let json = serde_json::to_string(finding).unwrap();
        assert!(
            !json.contains("AKIAZ7Q3VJ2M4XK9PLR6"),
            "serialized finding leaked the plaintext secret: {json}"
        );
    }

    #[test]
    fn overlapping_ai_key_prefixes_do_not_suppress_detection() {
        let engine = RegexEngine::new(3.0, &[]).unwrap();
        for (prefix, rule) in [
            ("sk-proj-", "openai-api-key-v2"),
            ("sk-ant-", "anthropic-api-key-v2"),
            ("sk-ant-api03-", "anthropic-api-key"),
        ] {
            let text = format!("{prefix}{RANDOM_120}");
            let findings = engine.scan(&make_text(&text), None);
            assert!(
                findings.iter().any(|finding| finding.rule_id == rule),
                "missed {rule}"
            );
        }
    }

    #[test]
    fn unicode_secret_previews_are_masked_at_character_boundaries() {
        assert_eq!(truncate_matched_text("🔑密码测试", 40), "🔑密码测试");
        assert_eq!(
            truncate_matched_text("🔑abcdefghi🔑j", 40),
            "🔑abcde**hi🔑j"
        );
        let text = "🔑".repeat(50);
        assert_eq!(
            truncate_matched_text(&text, 40),
            format!("{}...(50 chars)...{}", "🔑".repeat(6), "🔑".repeat(4))
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
