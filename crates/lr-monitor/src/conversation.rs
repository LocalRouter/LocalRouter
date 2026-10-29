//! Conservative, bounded local correlation. Hashes are never sent to the UI.
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::io::{self, Write};

pub(crate) struct Correlation {
    explicit: Option<[u8; 32]>,
    reset: bool,
    previous_response: Option<[u8; 32]>,
    history: Vec<[u8; 32]>,
    assistant: Vec<bool>,
    pub title: String,
}

struct HashWriter(Sha256);
impl Write for HashWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.update(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
fn hash(value: &Value) -> [u8; 32] {
    let mut writer = HashWriter(Sha256::new());
    // Values are always serializable; streaming avoids copying long contexts.
    let _ = serde_json::to_writer(&mut writer, value);
    writer.0.finalize().into()
}

impl Correlation {
    pub fn from_body(body: &Value) -> Option<Self> {
        let options = body.pointer("/extensions/localrouter.session");
        let explicit = options
            .and_then(|s| s.get("id"))
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty() && s.len() <= 256 && !s.chars().any(char::is_control))
            .map(|s| hash(&Value::String(s.into())));
        let previous_response = body
            .get("previous_response_id")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty() && s.len() <= 256)
            .map(|s| hash(&Value::String(s.into())));
        let messages = body
            .get("messages")
            .or_else(|| body.get("input"))
            .and_then(Value::as_array);
        // A very large history can still use an explicit ID; never retain unbounded hashes.
        let history = messages.filter(|m| m.len() <= 2048);
        let title = messages
            .and_then(|m| {
                m.iter()
                    .find(|v| v.get("role").and_then(Value::as_str) == Some("user"))
            })
            .and_then(|m| m.get("content"))
            .or_else(|| body.get("input").filter(|v| v.is_string()))
            .map(crate::preview::content)
            .unwrap_or_default()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .chars()
            .take(160)
            .collect();
        if explicit.is_none()
            && previous_response.is_none()
            && !body.get("input").is_some_and(Value::is_string)
            && history.is_none_or(|m| {
                !m.iter()
                    .any(|v| v.get("role").and_then(Value::as_str) == Some("user"))
            })
        {
            return None;
        }
        Some(Self {
            explicit,
            previous_response,
            reset: options
                .and_then(|s| s.get("reset"))
                .and_then(Value::as_bool)
                .unwrap_or(false),
            history: history
                .map(|m| m.iter().map(hash).collect())
                .unwrap_or_default(),
            assistant: history
                .map(|m| {
                    m.iter()
                        .map(|v| {
                            matches!(
                                v.get("role").and_then(Value::as_str),
                                Some("assistant" | "tool")
                            ) || matches!(
                                v.get("type").and_then(Value::as_str),
                                Some("function_call" | "function_call_output")
                            )
                        })
                        .collect()
                })
                .unwrap_or_default(),
            title,
        })
    }
    pub fn is_explicit(&self) -> bool {
        self.explicit.is_some()
    }
    pub fn is_response_chain(&self) -> bool {
        self.previous_response.is_some()
    }
    pub fn references_response(&self, data: &crate::types::MonitorEventData) -> bool {
        if self.reset || self.explicit.is_some() {
            return false;
        }
        let crate::types::MonitorEventData::LlmCall {
            response_body: Some(body),
            ..
        } = data
        else {
            return false;
        };
        self.previous_response
            .is_some_and(|id| body.get("id").is_some_and(|v| hash(v) == id))
    }
    pub fn follows(&self, prior: &Self) -> bool {
        if self.reset {
            return false;
        }
        if let Some(id) = self.explicit {
            return prior.explicit == Some(id);
        }
        // Do not bridge explicit boundaries with a text-based guess. A bare
        // repeated prompt or identical history is not a conversation signal.
        self.previous_response.is_none()
            && prior.explicit.is_none()
            && !prior.history.is_empty()
            && self.history.len() > prior.history.len()
            && self.history.starts_with(&prior.history)
            && self.assistant[prior.history.len()..].iter().any(|v| *v)
    }
    pub fn history_len(&self) -> usize {
        self.history.len()
    }
    pub fn size(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.history.capacity() * 32
            + self.assistant.capacity()
            + self.title.capacity()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn c(body: Value) -> Correlation {
        Correlation::from_body(&body).unwrap()
    }
    #[test]
    fn history_requires_continuation_and_exact_prefix() {
        let first = c(json!({"messages":[{"role":"user","content":"Hello"}]}));
        let repeated = c(json!({"messages":[{"role":"user","content":"Hello"}]}));
        let next = c(
            json!({"messages":[{"role":"user","content":"Hello"},{"role":"assistant","content":"Hi"},{"role":"user","content":"Next"}]}),
        );
        assert!(!repeated.follows(&first));
        assert!(next.follows(&first));
        assert!(!first.follows(&next));
        assert_eq!(next.title, "Hello");
        assert!(!c(json!({"messages":[{"role":"user","content":"Different"},{"role":"assistant","content":"Hi"}]})).follows(&first));
    }
    #[test]
    fn large_histories_are_bounded_and_titles_exclude_system_messages() {
        let messages = vec![json!({"role":"user","content":"Hello"}); 2049];
        assert!(Correlation::from_body(&json!({"messages":messages})).is_none());
        let body = json!({"extensions":{"localrouter.session":{"id":"one"}},"messages":messages});
        assert_eq!(c(body).history_len(), 0);
        let first = c(
            json!({"messages":[{"role":"system","content":"private"},{"role":"user","content":"界".repeat(200)}]}),
        );
        assert_eq!(first.title.chars().count(), 160);
        assert!(!first.title.contains("private"));
    }
    #[test]
    fn explicit_ids_and_reset_are_respected() {
        let first = c(json!({"extensions":{"localrouter.session":{"id":"one"}}}));
        assert!(c(json!({"extensions":{"localrouter.session":{"id":"one"}}})).follows(&first));
        assert!(!c(json!({"extensions":{"localrouter.session":{"id":"other"}}})).follows(&first));
        assert!(
            !c(json!({"extensions":{"localrouter.session":{"id":"one","reset":true}}}))
                .follows(&first)
        );
        assert!(Correlation::from_body(
            &json!({"messages":[{"role":"system","content":"private instructions"}]})
        )
        .is_none());
    }
}
