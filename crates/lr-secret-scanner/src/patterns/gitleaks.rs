//! Offline, pinned Gitleaks content rules. See data/gitleaks/README.md.
use crate::{
    filters::{self, PlaceholderPolicy},
    types::{SecretCategory, SecretRule},
};
use regex::{Captures, Regex};
use serde::Deserialize;
use std::sync::Arc;

#[derive(Deserialize)]
struct Snapshot {
    version: String,
    rules: Vec<Rule>,
    allowlists: Vec<Allowlist>,
}
#[derive(Deserialize)]
struct Rule {
    id: String,
    description: String,
    regex: String,
    secret_group: Option<usize>,
    whole_match: bool,
    entropy: f32,
    keywords: Vec<String>,
    allowlists: Vec<Allowlist>,
}
#[derive(Deserialize)]
struct Allowlist {
    condition: String,
    target: String,
    regexes: Vec<String>,
    stopwords: Vec<String>,
    requires_path_or_commit: bool,
}

pub struct CompiledAllowlist {
    and: bool,
    target: Target,
    regexes: Vec<Regex>,
    stopwords: Vec<String>,
    requires_path_or_commit: bool,
}
enum Target {
    Secret,
    Match,
    Line,
}
impl CompiledAllowlist {
    fn compile(raw: Allowlist) -> Result<Self, String> {
        let and = match raw.condition.as_str() {
            "AND" => true,
            "OR" => false,
            _ => return Err("Unsupported Gitleaks allowlist condition".into()),
        };
        let target = match raw.target.as_str() {
            "secret" => Target::Secret,
            "match" => Target::Match,
            "line" => Target::Line,
            _ => return Err("Unsupported Gitleaks allowlist target".into()),
        };
        Ok(Self {
            and,
            target,
            regexes: raw
                .regexes
                .iter()
                .map(|r| Regex::new(r).map_err(|e| format!("Invalid Gitleaks allowlist: {e}")))
                .collect::<Result<_, _>>()?,
            stopwords: raw
                .stopwords
                .into_iter()
                .map(|s| s.to_ascii_lowercase())
                .collect(),
            requires_path_or_commit: raw.requires_path_or_commit,
        })
    }
    fn applies(&self, secret: &str, matched: &str, line: &str) -> bool {
        if self.and && self.requires_path_or_commit {
            return false;
        }
        let target = match self.target {
            Target::Secret => secret,
            Target::Match => matched,
            Target::Line => line,
        };
        let regex = !self.regexes.is_empty() && self.regexes.iter().any(|r| r.is_match(target));
        let word = !self.stopwords.is_empty() && {
            let lower = secret.to_ascii_lowercase();
            self.stopwords.iter().any(|w| lower.contains(w))
        };
        if self.and {
            (!self.regexes.is_empty() || !self.stopwords.is_empty())
                && (self.regexes.is_empty() || regex)
                && (self.stopwords.is_empty() || word)
        } else {
            regex || word
        }
    }
}

pub struct RuleChecks {
    pub version: String,
    pub first_capture: bool,
    global: Arc<Vec<CompiledAllowlist>>,
    allowlists: Vec<CompiledAllowlist>,
}
impl RuleChecks {
    pub fn is_allowed(&self, secret: &str, captures: &Captures<'_>, text: &str) -> bool {
        let Some(matched) = captures.get(0) else {
            return false;
        };
        let start = text[..matched.start()].rfind('\n').map_or(0, |i| i + 1);
        let end = text[matched.end()..]
            .find('\n')
            .map_or(text.len(), |i| matched.end() + i);
        let line = &text[start..end];
        self.global
            .iter()
            .chain(&self.allowlists)
            .any(|a| a.applies(secret, matched.as_str(), line))
    }
}

pub fn rules() -> Result<Vec<SecretRule>, String> {
    let snapshot: Snapshot = serde_json::from_str(include_str!("../../data/gitleaks/rules.json"))
        .map_err(|e| format!("Invalid bundled Gitleaks rules: {e}"))?;
    let global = Arc::new(
        snapshot
            .allowlists
            .into_iter()
            .map(CompiledAllowlist::compile)
            .collect::<Result<Vec<_>, _>>()?,
    );
    snapshot
        .rules
        .into_iter()
        .map(|raw| {
            let regex = Regex::new(&raw.regex)
                .map_err(|e| format!("Invalid Gitleaks rule {}: {e}", raw.id))?;
            let group = raw.secret_group.unwrap_or(0);
            if group >= regex.captures_len() {
                return Err(format!(
                    "Invalid capture group for Gitleaks rule {}",
                    raw.id
                ));
            }
            let validate = match raw.id.as_str() {
                "jwt-base64" => Some(filters::base64_jwt_is_secret as super::builtin::Validator),
                "curl-auth-user" => {
                    Some(filters::curl_user_has_secret as super::builtin::Validator)
                }
                "curl-auth-header" => {
                    Some(filters::curl_header_has_secret as super::builtin::Validator)
                }
                _ => None,
            };
            let placeholder = match raw.id.as_str() {
                "curl-auth-user" => PlaceholderPolicy::Off,
                "curl-auth-header" => PlaceholderPolicy::Generic,
                _ => PlaceholderPolicy::Strict,
            };
            Ok(SecretRule {
                id: raw.id,
                description: raw.description,
                compiled_regex: regex,
                secret_group: group,
                entropy_group: None,
                entropy_threshold: Some(raw.entropy),
                keywords: raw.keywords,
                category: SecretCategory::Generic,
                placeholder,
                validate,
                upstream: Some(RuleChecks {
                    version: snapshot.version.clone(),
                    first_capture: raw.secret_group.is_none() && !raw.whole_match,
                    global: global.clone(),
                    allowlists: raw
                        .allowlists
                        .into_iter()
                        .map(CompiledAllowlist::compile)
                        .collect::<Result<_, _>>()?,
                }),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn all_embedded_rules_compile_and_have_unique_ids_and_source_metadata() {
        let rules = rules().unwrap();
        assert_eq!(rules.len(), 201);
        let ids: std::collections::HashSet<_> = rules.iter().map(|r| &r.id).collect();
        assert_eq!(ids.len(), rules.len());
        assert!(rules
            .iter()
            .all(|r| r.upstream.as_ref().unwrap().version == "v8.30.1"));
    }
    fn allow(
        condition: &str,
        target: &str,
        paths: bool,
        regexes: &[&str],
        stopwords: &[&str],
    ) -> CompiledAllowlist {
        CompiledAllowlist::compile(Allowlist {
            condition: condition.into(),
            target: target.into(),
            regexes: regexes.iter().map(|s| s.to_string()).collect(),
            stopwords: stopwords.iter().map(|s| s.to_string()).collect(),
            requires_path_or_commit: paths,
        })
        .unwrap()
    }
    #[test]
    fn content_allowlists_respect_target_and_or_and_unknown_file_context() {
        assert!(allow("OR", "match", false, &["example"], &[]).applies(
            "value",
            "example=value",
            "other"
        ));
        assert!(!allow("OR", "secret", false, &["example"], &[]).applies(
            "value",
            "example=value",
            "other"
        ));
        assert!(allow("OR", "line", false, &["example"], &[]).applies(
            "value",
            "value",
            "example=value"
        ));
        assert!(!allow("AND", "line", true, &["example"], &[]).applies(
            "value",
            "value",
            "example=value"
        ));
        assert!(!allow("OR", "line", true, &[], &[]).applies("value", "value", "example=value"));
        assert!(
            allow("AND", "secret", false, &["value"], &["VAL"]).applies("value", "value", "value")
        );
        assert!(!allow("AND", "secret", false, &["value"], &["other"])
            .applies("value", "value", "value"));
    }
}
