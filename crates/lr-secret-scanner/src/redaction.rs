//! In-memory redaction plans. Plaintext values never serialize or appear in Debug.
use serde_json::Value;

pub const REDACTION_MESSAGE: &str = "[REDACTED: secret removed by LocalRouter]";

#[derive(Clone, PartialEq, Eq, Default)]
pub struct Redaction {
    secrets: Vec<String>,
}

impl std::fmt::Debug for Redaction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Redaction")
            .field("count", &self.secrets.len())
            .finish()
    }
}

impl Redaction {
    pub fn add(&mut self, secret: &str) {
        if !secret.is_empty() && !self.secrets.iter().any(|s| s == secret) {
            self.secrets.push(secret.to_string());
            // Match longer values first when secrets overlap.
            self.secrets.sort_by_key(|s| std::cmp::Reverse(s.len()));
        }
    }

    pub fn is_empty(&self) -> bool {
        self.secrets.is_empty()
    }

    pub fn extend(&mut self, other: Self) {
        for secret in other.secrets {
            self.add(&secret);
        }
    }

    pub fn apply_text(&self, text: &str) -> String {
        // Merge matched spans, including overlaps; never rescan inserted markers.
        let mut spans = Vec::new();
        for secret in &self.secrets {
            let mut offset = 0;
            while offset < text.len() {
                let Some(relative) = text[offset..].find(secret) else {
                    break;
                };
                let start = offset + relative;
                spans.push((start, start + secret.len()));
                offset = start + text[start..].chars().next().unwrap().len_utf8();
            }
        }
        spans.sort_unstable();
        let mut merged: Vec<(usize, usize)> = Vec::new();
        for (start, end) in spans {
            if let Some(last) = merged.last_mut().filter(|last| start < last.1) {
                last.1 = last.1.max(end);
            } else {
                merged.push((start, end));
            }
        }
        let mut result = String::new();
        let mut offset = 0;
        for (start, end) in merged {
            result.push_str(&text[offset..start]);
            result.push_str(REDACTION_MESSAGE);
            offset = end;
        }
        result.push_str(&text[offset..]);
        result
    }

    /// Replace every occurrence in JSON string values, preserving structure.
    pub fn apply(&self, body: &mut Value) {
        match body {
            Value::String(text) => *text = self.apply_text(text),
            Value::Array(values) => values.iter_mut().for_each(|v| self.apply(v)),
            Value::Object(values) => values.values_mut().for_each(|v| self.apply(v)),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_all_occurrences_and_nested_values_preserving_json() {
        let mut plan = Redaction::default();
        plan.add("key-123");
        plan.add("key-123456");
        plan.add("");
        let mut body = serde_json::json!({"messages":[{"content":"🔑 key-123456 / key-123 / key-123"}],"count":2});
        plan.apply(&mut body);
        assert_eq!(
            body["messages"][0]["content"],
            format!("🔑 {0} / {0} / {0}", REDACTION_MESSAGE)
        );
        assert_eq!(body["count"], 2);
        assert!(!format!("{plan:?}").contains("key-123"));
    }

    #[test]
    fn merges_partial_and_self_overlapping_matches() {
        let mut plan = Redaction::default();
        plan.add("abc");
        plan.add("bcde");
        plan.add("xxx");
        assert_eq!(
            plan.apply_text("abcde xxxx"),
            format!("{0} {0}", REDACTION_MESSAGE)
        );
    }

    #[test]
    fn inserted_markers_are_never_redacted_again() {
        let mut plan = Redaction::default();
        plan.add("secret");
        plan.add("LocalRouter");
        assert_eq!(
            plan.apply_text("secret LocalRouter"),
            format!("{0} {0}", REDACTION_MESSAGE)
        );
    }
}
