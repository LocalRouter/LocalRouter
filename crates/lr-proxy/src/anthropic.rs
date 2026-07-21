//! Parsing of the Anthropic Messages wire format for passive monitoring.
//!
//! Claude Code talks to `api.anthropic.com/v1/messages`, which the OpenAI-shaped
//! server routes don't understand — so the proxy parses it here purely for
//! monitor attribution. Nothing in this module mutates traffic; it only reads.

use serde_json::Value;

/// The maximum content-preview length we keep for the monitor UI.
const CONTENT_PREVIEW_LIMIT: usize = 2000;

/// Request-side metadata extracted from an Anthropic Messages request body.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct AnthropicRequestMeta {
    pub model: Option<String>,
    pub stream: bool,
    pub message_count: usize,
    pub has_tools: bool,
}

/// Response-side metadata extracted from an Anthropic Messages response
/// (either a single JSON object or a reconstructed SSE stream).
#[derive(Debug, Default, Clone, PartialEq)]
pub struct AnthropicResponseMeta {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub stop_reason: Option<String>,
    pub content_preview: Option<String>,
}

/// True if a path is the Anthropic Messages endpoint we monitor.
pub fn is_messages_path(path: &str) -> bool {
    // Tolerate query strings and API-version prefixes.
    let path = path.split('?').next().unwrap_or(path);
    path == "/v1/messages" || path.ends_with("/v1/messages")
}

/// Extract request metadata from a parsed Anthropic Messages request body.
pub fn parse_request(body: &Value) -> AnthropicRequestMeta {
    AnthropicRequestMeta {
        model: body
            .get("model")
            .and_then(Value::as_str)
            .map(str::to_string),
        stream: body.get("stream").and_then(Value::as_bool).unwrap_or(false),
        message_count: body
            .get("messages")
            .and_then(Value::as_array)
            .map(Vec::len)
            .unwrap_or(0),
        has_tools: body
            .get("tools")
            .and_then(Value::as_array)
            .is_some_and(|t| !t.is_empty()),
    }
}

/// Extract response metadata from a single (non-streaming) Anthropic response.
pub fn parse_response(body: &Value) -> AnthropicResponseMeta {
    let usage = body.get("usage");
    AnthropicResponseMeta {
        input_tokens: usage
            .and_then(|u| u.get("input_tokens"))
            .and_then(Value::as_u64),
        output_tokens: usage
            .and_then(|u| u.get("output_tokens"))
            .and_then(Value::as_u64),
        stop_reason: body
            .get("stop_reason")
            .and_then(Value::as_str)
            .map(str::to_string),
        content_preview: extract_content_text(body).map(|t| truncate(&t)),
    }
}

/// Concatenate the text of all `content` blocks of type `text`.
fn extract_content_text(body: &Value) -> Option<String> {
    let blocks = body.get("content")?.as_array()?;
    let mut out = String::new();
    for block in blocks {
        if block.get("type").and_then(Value::as_str) == Some("text") {
            if let Some(t) = block.get("text").and_then(Value::as_str) {
                out.push_str(t);
            }
        }
    }
    (!out.is_empty()).then_some(out)
}

/// Reconstruct response metadata from a raw Anthropic SSE stream body.
///
/// Anthropic streams a sequence of `event:`/`data:` line pairs. We only need:
/// - `message_start` → `message.usage.input_tokens`
/// - `content_block_delta` → `delta.text` (accumulated into the preview)
/// - `message_delta` → `usage.output_tokens`, `delta.stop_reason`
pub fn reconstruct_sse(raw: &str) -> AnthropicResponseMeta {
    let mut meta = AnthropicResponseMeta::default();
    let mut preview = String::new();

    for line in raw.lines() {
        let line = line.trim_start();
        let Some(data) = line.strip_prefix("data:") else {
            continue;
        };
        let data = data.trim();
        if data.is_empty() || data == "[DONE]" {
            continue;
        }
        let Ok(json) = serde_json::from_str::<Value>(data) else {
            continue;
        };

        match json.get("type").and_then(Value::as_str) {
            Some("message_start") => {
                if let Some(u) = json.get("message").and_then(|m| m.get("usage")) {
                    if let Some(v) = u.get("input_tokens").and_then(Value::as_u64) {
                        meta.input_tokens = Some(v);
                    }
                }
            }
            Some("content_block_delta") => {
                if let Some(t) = json
                    .get("delta")
                    .and_then(|d| d.get("text"))
                    .and_then(Value::as_str)
                {
                    if preview.len() < CONTENT_PREVIEW_LIMIT {
                        preview.push_str(t);
                    }
                }
            }
            Some("message_delta") => {
                if let Some(v) = json
                    .get("usage")
                    .and_then(|u| u.get("output_tokens"))
                    .and_then(Value::as_u64)
                {
                    meta.output_tokens = Some(v);
                }
                if let Some(s) = json
                    .get("delta")
                    .and_then(|d| d.get("stop_reason"))
                    .and_then(Value::as_str)
                {
                    meta.stop_reason = Some(s.to_string());
                }
            }
            _ => {}
        }
    }

    if !preview.is_empty() {
        meta.content_preview = Some(truncate(&preview));
    }
    meta
}

fn truncate(s: &str) -> String {
    if s.len() <= CONTENT_PREVIEW_LIMIT {
        return s.to_string();
    }
    // Respect char boundaries when cutting.
    let mut end = CONTENT_PREVIEW_LIMIT;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &s[..end])
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_request_metadata() {
        let body = json!({
            "model": "claude-sonnet-4-20250514",
            "stream": true,
            "messages": [{"role": "user", "content": "hi"}, {"role": "assistant", "content": "yo"}],
            "tools": [{"name": "get_weather"}]
        });
        let meta = parse_request(&body);
        assert_eq!(meta.model.as_deref(), Some("claude-sonnet-4-20250514"));
        assert!(meta.stream);
        assert_eq!(meta.message_count, 2);
        assert!(meta.has_tools);
    }

    #[test]
    fn request_defaults_when_fields_absent() {
        let meta = parse_request(&json!({"model": "x"}));
        assert!(!meta.stream);
        assert_eq!(meta.message_count, 0);
        assert!(!meta.has_tools);
    }

    #[test]
    fn parses_non_streaming_response() {
        let body = json!({
            "content": [
                {"type": "text", "text": "Hello "},
                {"type": "text", "text": "world"},
                {"type": "tool_use", "name": "x"}
            ],
            "stop_reason": "end_turn",
            "usage": {"input_tokens": 12, "output_tokens": 7}
        });
        let meta = parse_response(&body);
        assert_eq!(meta.input_tokens, Some(12));
        assert_eq!(meta.output_tokens, Some(7));
        assert_eq!(meta.stop_reason.as_deref(), Some("end_turn"));
        assert_eq!(meta.content_preview.as_deref(), Some("Hello world"));
    }

    #[test]
    fn reconstructs_streaming_response() {
        let raw = "\
event: message_start
data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":25}}}

event: content_block_delta
data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"Hel\"}}

event: content_block_delta
data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"lo\"}}

event: message_delta
data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":9}}

event: message_stop
data: {\"type\":\"message_stop\"}
";
        let meta = reconstruct_sse(raw);
        assert_eq!(meta.input_tokens, Some(25));
        assert_eq!(meta.output_tokens, Some(9));
        assert_eq!(meta.stop_reason.as_deref(), Some("end_turn"));
        assert_eq!(meta.content_preview.as_deref(), Some("Hello"));
    }

    #[test]
    fn ignores_malformed_sse_lines() {
        let raw = "data: not-json\n\ndata: {\"type\":\"message_delta\",\"usage\":{\"output_tokens\":3}}\n";
        let meta = reconstruct_sse(raw);
        assert_eq!(meta.output_tokens, Some(3));
    }

    #[test]
    fn recognizes_messages_path() {
        assert!(is_messages_path("/v1/messages"));
        assert!(is_messages_path("/v1/messages?beta=true"));
        assert!(!is_messages_path("/v1/complete"));
    }
}
