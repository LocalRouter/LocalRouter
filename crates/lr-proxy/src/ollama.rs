//! Parsing of Ollama's **native** API exchanges (`/api/chat`, `/api/generate`).
//!
//! Reached through the reverse proxy, where LocalRouter wraps a local Ollama
//! and sees whatever dialect the app speaks. Apps that use Ollama's
//! OpenAI-compatible `/v1/*` surface are parsed by [`crate::openai`] instead.
//!
//! Two shapes differ from the OpenAI/Anthropic parsers:
//! - **NDJSON, not SSE.** A streamed response is one JSON object per line, no
//!   `data:` framing; the final object carries the token counts.
//! - **Counts, not a `usage` block.** `prompt_eval_count` / `eval_count` are
//!   top-level fields on the final object.

use serde_json::{json, Value};

use crate::wire::{RequestMeta, ResponseMeta};

const PREVIEW_CAP: usize = 4000;

fn truncate(s: &str) -> String {
    if s.len() <= PREVIEW_CAP {
        return s.to_string();
    }
    let mut end = PREVIEW_CAP;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &s[..end])
}

/// Which native endpoint an Ollama request targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OllamaEndpoint {
    /// `POST /api/chat` — messages in, `message.content` out.
    Chat,
    /// `POST /api/generate` — a prompt in, `response` text out.
    Generate,
}

/// Identify a native Ollama LLM path (ignoring any query string).
/// Non-inference endpoints (`/api/tags`, `/api/pull`, …) return `None`: they
/// are forwarded and served normally, they just aren't monitor events.
pub fn detect(path: &str) -> Option<OllamaEndpoint> {
    match path.split('?').next().unwrap_or(path).trim_end_matches('/') {
        "/api/chat" => Some(OllamaEndpoint::Chat),
        "/api/generate" => Some(OllamaEndpoint::Generate),
        _ => None,
    }
}

/// Extract request metadata from a native Ollama request body.
///
/// Note the `stream` default: unlike OpenAI, Ollama streams **unless** the
/// caller opts out, so an absent field means `true`.
pub fn parse_request(endpoint: OllamaEndpoint, body: &Value) -> RequestMeta {
    RequestMeta {
        model: body
            .get("model")
            .and_then(Value::as_str)
            .map(str::to_string),
        stream: body.get("stream").and_then(Value::as_bool).unwrap_or(true),
        message_count: match endpoint {
            OllamaEndpoint::Chat => body
                .get("messages")
                .and_then(Value::as_array)
                .map(Vec::len)
                .unwrap_or(0),
            // A prompt is one logical message.
            OllamaEndpoint::Generate => usize::from(body.get("prompt").is_some()),
        },
        has_tools: body
            .get("tools")
            .and_then(Value::as_array)
            .is_some_and(|t| !t.is_empty()),
    }
}

/// Pull the fields a monitor event needs out of one Ollama response object
/// (the whole body when not streaming, the final line when streaming).
fn meta_from_object(endpoint: OllamaEndpoint, body: &Value, content: String) -> ResponseMeta {
    ResponseMeta {
        message_id: None,
        model: body
            .get("model")
            .and_then(Value::as_str)
            .map(str::to_string),
        input_tokens: body.get("prompt_eval_count").and_then(Value::as_u64),
        output_tokens: body.get("eval_count").and_then(Value::as_u64),
        cache_creation_tokens: None,
        cache_read_tokens: None,
        reasoning_tokens: None,
        stop_reason: body
            .get("done_reason")
            .and_then(Value::as_str)
            .map(str::to_string),
        content_preview: (!content.is_empty()).then(|| truncate(&content)),
        reasoning_preview: match endpoint {
            OllamaEndpoint::Chat => body
                .get("message")
                .and_then(|m| m.get("thinking"))
                .and_then(Value::as_str)
                .map(truncate),
            OllamaEndpoint::Generate => body.get("thinking").and_then(Value::as_str).map(truncate),
        },
    }
}

/// Text content of one response object (streamed chunk or complete body).
fn content_of(endpoint: OllamaEndpoint, obj: &Value) -> &str {
    match endpoint {
        OllamaEndpoint::Chat => obj
            .get("message")
            .and_then(|m| m.get("content"))
            .and_then(Value::as_str)
            .unwrap_or_default(),
        OllamaEndpoint::Generate => obj
            .get("response")
            .and_then(Value::as_str)
            .unwrap_or_default(),
    }
}

/// Extract response metadata from a non-streaming Ollama response body.
pub fn parse_response(endpoint: OllamaEndpoint, body: &Value) -> ResponseMeta {
    let content = content_of(endpoint, body).to_string();
    meta_from_object(endpoint, body, content)
}

/// Reconstruct an NDJSON stream into (metadata, assembled response body).
///
/// Concatenates the per-chunk text and folds the final object's counts in, so a
/// streamed call is captured exactly like a non-streamed one. A truncated final
/// line (capture cap hit) simply yields no counts rather than failing.
pub fn reconstruct_ndjson(endpoint: OllamaEndpoint, raw: &str) -> (ResponseMeta, Value) {
    let mut content = String::new();
    let mut thinking = String::new();
    let mut last_complete: Option<Value> = None;
    let mut any: Option<Value> = None;

    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(obj) = serde_json::from_str::<Value>(line) else {
            // Only the tail can be truncated mid-object; skip it.
            continue;
        };
        content.push_str(content_of(endpoint, &obj));
        let think = match endpoint {
            OllamaEndpoint::Chat => obj
                .get("message")
                .and_then(|m| m.get("thinking"))
                .and_then(Value::as_str),
            OllamaEndpoint::Generate => obj.get("thinking").and_then(Value::as_str),
        };
        if let Some(t) = think {
            thinking.push_str(t);
        }
        if obj.get("done").and_then(Value::as_bool).unwrap_or(false) {
            last_complete = Some(obj.clone());
        }
        any = Some(obj);
    }

    // Prefer the terminal object (it carries the counts); fall back to the last
    // chunk seen so a cut-off stream still reports its model.
    let summary = last_complete.or(any).unwrap_or(Value::Null);
    let mut meta = meta_from_object(endpoint, &summary, content.clone());
    if !thinking.is_empty() {
        meta.reasoning_preview = Some(truncate(&thinking));
    }

    // Assemble a body shaped like the non-streaming response.
    let body = match endpoint {
        OllamaEndpoint::Chat => json!({
            "model": meta.model,
            "message": { "role": "assistant", "content": content },
            "done": true,
            "done_reason": meta.stop_reason,
            "prompt_eval_count": meta.input_tokens,
            "eval_count": meta.output_tokens,
        }),
        OllamaEndpoint::Generate => json!({
            "model": meta.model,
            "response": content,
            "done": true,
            "done_reason": meta.stop_reason,
            "prompt_eval_count": meta.input_tokens,
            "eval_count": meta.output_tokens,
        }),
    };
    (meta, body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_native_inference_paths_only() {
        assert_eq!(detect("/api/chat"), Some(OllamaEndpoint::Chat));
        assert_eq!(detect("/api/generate?x=1"), Some(OllamaEndpoint::Generate));
        assert_eq!(detect("/api/tags"), None);
        assert_eq!(detect("/api/pull"), None);
        assert_eq!(detect("/v1/chat/completions"), None);
    }

    #[test]
    fn request_streams_by_default() {
        let body = json!({"model": "llama3", "messages": [{"role": "user", "content": "hi"}]});
        let meta = parse_request(OllamaEndpoint::Chat, &body);
        assert_eq!(meta.model.as_deref(), Some("llama3"));
        assert!(meta.stream, "Ollama streams unless the caller opts out");
        assert_eq!(meta.message_count, 1);
        assert!(!meta.has_tools);

        let explicit = json!({"model": "llama3", "messages": [], "stream": false});
        assert!(!parse_request(OllamaEndpoint::Chat, &explicit).stream);
    }

    #[test]
    fn parses_non_streaming_chat_response() {
        let body = json!({
            "model": "llama3",
            "message": {"role": "assistant", "content": "Hello!"},
            "done": true,
            "done_reason": "stop",
            "prompt_eval_count": 26,
            "eval_count": 8,
        });
        let meta = parse_response(OllamaEndpoint::Chat, &body);
        assert_eq!(meta.input_tokens, Some(26));
        assert_eq!(meta.output_tokens, Some(8));
        assert_eq!(meta.stop_reason.as_deref(), Some("stop"));
        assert_eq!(meta.content_preview.as_deref(), Some("Hello!"));
    }

    #[test]
    fn reconstructs_streamed_chat() {
        let raw = concat!(
            "{\"model\":\"llama3\",\"message\":{\"role\":\"assistant\",\"content\":\"Hel\"},\"done\":false}\n",
            "{\"model\":\"llama3\",\"message\":{\"role\":\"assistant\",\"content\":\"lo\"},\"done\":false}\n",
            "{\"model\":\"llama3\",\"message\":{\"role\":\"assistant\",\"content\":\"\"},\"done\":true,\"done_reason\":\"stop\",\"prompt_eval_count\":26,\"eval_count\":8}\n",
        );
        let (meta, body) = reconstruct_ndjson(OllamaEndpoint::Chat, raw);
        assert_eq!(meta.content_preview.as_deref(), Some("Hello"));
        assert_eq!(meta.input_tokens, Some(26));
        assert_eq!(meta.output_tokens, Some(8));
        assert_eq!(meta.model.as_deref(), Some("llama3"));
        assert_eq!(body["message"]["content"], "Hello");
    }

    #[test]
    fn reconstructs_streamed_generate_with_truncated_tail() {
        // Capture cap cut the final object in half: content still assembles,
        // counts are simply absent rather than the parse failing.
        let raw = concat!(
            "{\"model\":\"llama3\",\"response\":\"a\",\"done\":false}\n",
            "{\"model\":\"llama3\",\"response\":\"b\",\"done\":false}\n",
            "{\"model\":\"llama3\",\"response\":\"\",\"done\":tr",
        );
        let (meta, body) = reconstruct_ndjson(OllamaEndpoint::Generate, raw);
        assert_eq!(meta.content_preview.as_deref(), Some("ab"));
        assert_eq!(meta.input_tokens, None);
        assert_eq!(body["response"], "ab");
    }

    #[test]
    fn captures_thinking_text() {
        let raw = concat!(
            "{\"message\":{\"thinking\":\"hmm \",\"content\":\"\"},\"done\":false}\n",
            "{\"message\":{\"thinking\":\"ok\",\"content\":\"Hi\"},\"done\":true,\"eval_count\":3}\n",
        );
        let (meta, _) = reconstruct_ndjson(OllamaEndpoint::Chat, raw);
        assert_eq!(meta.reasoning_preview.as_deref(), Some("hmm ok"));
        assert_eq!(meta.content_preview.as_deref(), Some("Hi"));
    }
}
