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

/// Most messages whose hashes are kept; a longer history only correlates
/// through an explicit ID or a response chain.
const MAX_HISTORY: usize = 2048;

/// What correlation reads from a request's `messages` (or `input`) array,
/// collected one element at a time so a streamed request needs no whole body.
#[derive(Debug, Default, Clone)]
pub struct MessageHistory {
    len: usize,
    hashes: Vec<[u8; 32]>,
    assistant: Vec<bool>,
    has_user: bool,
    /// Content of the first `user` message; `Some(None)` when it has none.
    first_user: Option<Option<String>>,
}

impl MessageHistory {
    /// Add the next element of the array.
    pub fn push(&mut self, message: &Value) {
        self.len += 1;
        let user = message.get("role").and_then(Value::as_str) == Some("user");
        if user && self.first_user.is_none() {
            self.first_user = Some(message.get("content").map(crate::preview::content));
        }
        self.has_user |= user;
        if self.len > MAX_HISTORY {
            // Never retain unbounded hashes.
            self.hashes = Vec::new();
            self.assistant = Vec::new();
            return;
        }
        self.hashes.push(hash(message));
        self.assistant.push(
            matches!(
                message.get("role").and_then(Value::as_str),
                Some("assistant" | "tool")
            ) || matches!(
                message.get("type").and_then(Value::as_str),
                Some("function_call" | "function_call_output")
            ),
        );
    }

    /// Elements added so far.
    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    fn of(messages: &[Value]) -> Self {
        let mut history = Self::default();
        for message in messages {
            history.push(message);
        }
        history
    }
}

/// The parts of a request body correlation reads, for a body that is not
/// held whole. [`CorrelationParts::from_body`] of a whole body gives the
/// parts the streamed path collects.
#[derive(Debug, Default, Clone)]
pub struct CorrelationParts {
    /// `extensions["localrouter.session"]`, as forwarded.
    pub session: Option<Value>,
    /// The top-level `previous_response_id`, when it is a string.
    pub previous_response_id: Option<String>,
    /// The `messages` array, or `input` when there is no `messages` member.
    pub history: Option<MessageHistory>,
    /// The top-level `input`, when it is a string.
    pub input_string: Option<String>,
}

impl CorrelationParts {
    pub fn from_body(body: &Value) -> Self {
        let messages = body
            .get("messages")
            .or_else(|| body.get("input"))
            .and_then(Value::as_array);
        Self {
            session: body.pointer("/extensions/localrouter.session").cloned(),
            previous_response_id: body
                .get("previous_response_id")
                .and_then(Value::as_str)
                .map(str::to_owned),
            history: messages.map(|m| MessageHistory::of(m)),
            input_string: body.get("input").and_then(Value::as_str).map(str::to_owned),
        }
    }
}

impl Correlation {
    #[cfg(test)]
    pub fn from_body(body: &Value) -> Option<Self> {
        Self::from_parts(CorrelationParts::from_body(body))
    }

    pub fn from_parts(parts: CorrelationParts) -> Option<Self> {
        let options = parts.session.as_ref();
        let explicit = options
            .and_then(|s| s.get("id"))
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty() && s.len() <= 256 && !s.chars().any(char::is_control))
            .map(|s| hash(&Value::String(s.into())));
        let previous_response = parts
            .previous_response_id
            .as_deref()
            .filter(|s| !s.is_empty() && s.len() <= 256)
            .map(|s| hash(&Value::String(s.into())));
        let messages = parts.history.as_ref();
        // A very large history can still use an explicit ID; never retain unbounded hashes.
        let history = messages.filter(|m| m.len <= MAX_HISTORY);
        let title = messages
            .and_then(|m| m.first_user.clone())
            .flatten()
            .or_else(|| {
                parts
                    .input_string
                    .as_ref()
                    .map(|s| crate::preview::content(&Value::String(s.clone())))
            })
            .unwrap_or_default()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .chars()
            .take(160)
            .collect();
        if explicit.is_none()
            && previous_response.is_none()
            && parts.input_string.is_none()
            && history.is_none_or(|m| !m.has_user)
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
            history: history.map(|m| m.hashes.clone()).unwrap_or_default(),
            assistant: history.map(|m| m.assistant.clone()).unwrap_or_default(),
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
