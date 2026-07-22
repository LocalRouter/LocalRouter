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
    pub message_id: Option<String>,
    pub model: Option<String>,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    /// Tokens written to the prompt cache (billed ~1.25x input on Anthropic).
    pub cache_creation_tokens: Option<u64>,
    /// Tokens read from the prompt cache (billed ~0.1x input on Anthropic).
    pub cache_read_tokens: Option<u64>,
    /// Extended-thinking output tokens (`output_tokens_details.thinking_tokens`).
    pub reasoning_tokens: Option<u64>,
    pub stop_reason: Option<String>,
    pub content_preview: Option<String>,
    /// Concatenated `thinking` text (the model's reasoning).
    pub reasoning_preview: Option<String>,
}

fn usage_u64(usage: Option<&Value>, key: &str) -> Option<u64> {
    usage.and_then(|u| u.get(key)).and_then(Value::as_u64)
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
    let content = body.get("content");
    AnthropicResponseMeta {
        message_id: body.get("id").and_then(Value::as_str).map(str::to_string),
        model: body
            .get("model")
            .and_then(Value::as_str)
            .map(str::to_string),
        input_tokens: usage_u64(usage, "input_tokens"),
        output_tokens: usage_u64(usage, "output_tokens"),
        cache_creation_tokens: usage_u64(usage, "cache_creation_input_tokens"),
        cache_read_tokens: usage_u64(usage, "cache_read_input_tokens"),
        reasoning_tokens: usage
            .and_then(|u| u.get("output_tokens_details"))
            .and_then(|d| d.get("thinking_tokens"))
            .and_then(Value::as_u64),
        stop_reason: body
            .get("stop_reason")
            .and_then(Value::as_str)
            .map(str::to_string),
        content_preview: extract_block_text(content, "text").map(|t| truncate(&t)),
        reasoning_preview: extract_block_text(content, "thinking").map(|t| truncate(&t)),
    }
}

/// Concatenate the text of all content blocks of the given `block_type`, reading
/// the matching field (`text` for text blocks, `thinking` for thinking blocks).
fn extract_block_text(content: Option<&Value>, block_type: &str) -> Option<String> {
    let blocks = content?.as_array()?;
    let field = if block_type == "thinking" {
        "thinking"
    } else {
        "text"
    };
    let mut out = String::new();
    for block in blocks {
        if block.get("type").and_then(Value::as_str) == Some(block_type) {
            if let Some(t) = block.get(field).and_then(Value::as_str) {
                out.push_str(t);
            }
        }
    }
    (!out.is_empty()).then_some(out)
}

/// Reconstruct an Anthropic SSE stream into (metadata, assembled message body).
///
/// Walks the `event:`/`data:` pairs, assembling content blocks by index
/// (text / thinking / tool_use) so the full response — including reasoning and
/// tool calls — is captured just like a non-streaming response.
pub fn reconstruct_sse(raw: &str) -> (AnthropicResponseMeta, Value) {
    // Per-index accumulators: (type, text/thinking buffer, tool name, tool id, partial json).
    #[derive(Default)]
    struct Block {
        kind: String,
        text: String,
        name: Option<String>,
        id: Option<String>,
        partial_json: String,
    }
    fn block_at(blocks: &mut Vec<Block>, idx: usize) -> &mut Block {
        if idx >= blocks.len() {
            blocks.resize_with(idx + 1, Block::default);
        }
        &mut blocks[idx]
    }

    let mut blocks: Vec<Block> = Vec::new();
    let mut message = serde_json::Map::new();
    let mut usage = serde_json::Map::new();
    let mut meta = AnthropicResponseMeta::default();

    for line in raw.lines() {
        let Some(data) = line.trim_start().strip_prefix("data:") else {
            continue;
        };
        let data = data.trim();
        if data.is_empty() || data == "[DONE]" {
            continue;
        }
        let Ok(json) = serde_json::from_str::<Value>(data) else {
            continue;
        };
        let idx = json.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;

        match json.get("type").and_then(Value::as_str) {
            Some("message_start") => {
                let m = json.get("message");
                meta.message_id = m
                    .and_then(|m| m.get("id"))
                    .and_then(Value::as_str)
                    .map(str::to_string);
                meta.model = m
                    .and_then(|m| m.get("model"))
                    .and_then(Value::as_str)
                    .map(str::to_string);
                let u = m.and_then(|m| m.get("usage"));
                meta.input_tokens = usage_u64(u, "input_tokens");
                meta.cache_creation_tokens = usage_u64(u, "cache_creation_input_tokens");
                meta.cache_read_tokens = usage_u64(u, "cache_read_input_tokens");
            }
            Some("content_block_start") => {
                if let Some(cb) = json.get("content_block") {
                    let b = block_at(&mut blocks, idx);
                    b.kind = cb
                        .get("type")
                        .and_then(Value::as_str)
                        .unwrap_or("text")
                        .to_string();
                    b.name = cb.get("name").and_then(Value::as_str).map(str::to_string);
                    b.id = cb.get("id").and_then(Value::as_str).map(str::to_string);
                }
            }
            Some("content_block_delta") => {
                let delta = json.get("delta");
                let b = block_at(&mut blocks, idx);
                if let Some(t) = delta.and_then(|d| d.get("text")).and_then(Value::as_str) {
                    b.text.push_str(t);
                } else if let Some(t) = delta
                    .and_then(|d| d.get("thinking"))
                    .and_then(Value::as_str)
                {
                    b.text.push_str(t);
                } else if let Some(j) = delta
                    .and_then(|d| d.get("partial_json"))
                    .and_then(Value::as_str)
                {
                    b.partial_json.push_str(j);
                }
            }
            Some("message_delta") => {
                if let Some(v) = usage_u64(json.get("usage"), "output_tokens") {
                    meta.output_tokens = Some(v);
                    usage.insert("output_tokens".into(), v.into());
                }
                if let Some(v) = json
                    .get("usage")
                    .and_then(|u| u.get("output_tokens_details"))
                    .and_then(|d| d.get("thinking_tokens"))
                    .and_then(Value::as_u64)
                {
                    meta.reasoning_tokens = Some(v);
                }
                if let Some(s) = json
                    .get("delta")
                    .and_then(|d| d.get("stop_reason"))
                    .and_then(Value::as_str)
                {
                    meta.stop_reason = Some(s.to_string());
                    message.insert("stop_reason".into(), s.into());
                }
            }
            _ => {}
        }
    }

    // Build previews + the assembled content array.
    let mut content_preview = String::new();
    let mut reasoning_preview = String::new();
    let content: Vec<Value> = blocks
        .into_iter()
        .map(|b| match b.kind.as_str() {
            "thinking" => {
                reasoning_preview.push_str(&b.text);
                serde_json::json!({ "type": "thinking", "thinking": b.text })
            }
            "tool_use" => {
                let input = serde_json::from_str::<Value>(&b.partial_json)
                    .unwrap_or_else(|_| serde_json::json!({}));
                serde_json::json!({ "type": "tool_use", "id": b.id, "name": b.name, "input": input })
            }
            _ => {
                content_preview.push_str(&b.text);
                serde_json::json!({ "type": "text", "text": b.text })
            }
        })
        .collect();

    if !content_preview.is_empty() {
        meta.content_preview = Some(truncate(&content_preview));
    }
    if !reasoning_preview.is_empty() {
        meta.reasoning_preview = Some(truncate(&reasoning_preview));
    }

    // Assemble a response body shaped like a non-streaming Anthropic message.
    if let Some(v) = meta.input_tokens {
        usage.insert("input_tokens".into(), v.into());
    }
    if let Some(v) = meta.cache_creation_tokens {
        usage.insert("cache_creation_input_tokens".into(), v.into());
    }
    if let Some(v) = meta.cache_read_tokens {
        usage.insert("cache_read_input_tokens".into(), v.into());
    }
    if let Some(ref id) = meta.message_id {
        message.insert("id".into(), id.clone().into());
    }
    if let Some(ref m) = meta.model {
        message.insert("model".into(), m.clone().into());
    }
    message.insert("type".into(), "message".into());
    message.insert("role".into(), "assistant".into());
    message.insert("content".into(), Value::Array(content));
    message.insert("usage".into(), Value::Object(usage));

    (meta, Value::Object(message))
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
        let (meta, body) = reconstruct_sse(raw);
        assert_eq!(meta.input_tokens, Some(25));
        assert_eq!(meta.output_tokens, Some(9));
        assert_eq!(meta.stop_reason.as_deref(), Some("end_turn"));
        assert_eq!(meta.content_preview.as_deref(), Some("Hello"));
        // The assembled body carries the reconstructed content + usage.
        assert_eq!(body["content"][0]["text"], "Hello");
        assert_eq!(body["usage"]["output_tokens"], 9);
    }

    #[test]
    fn reconstructs_thinking_and_cache_tokens() {
        // Mirrors real Claude Code streaming: thinking block + cache + thinking_tokens.
        let raw = "\
data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_1\",\"model\":\"claude-fable-5\",\"usage\":{\"input_tokens\":2,\"cache_creation_input_tokens\":39325,\"cache_read_input_tokens\":0}}}

data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"thinking\",\"thinking\":\"\"}}

data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"thinking_delta\",\"thinking\":\"I should be brief.\"}}

data: {\"type\":\"content_block_stop\",\"index\":0}

data: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}

data: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hey!\"}}

data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":28,\"output_tokens_details\":{\"thinking_tokens\":15}}}
";
        let (meta, body) = reconstruct_sse(raw);
        assert_eq!(meta.message_id.as_deref(), Some("msg_1"));
        assert_eq!(meta.model.as_deref(), Some("claude-fable-5"));
        assert_eq!(meta.cache_creation_tokens, Some(39325));
        assert_eq!(meta.reasoning_tokens, Some(15));
        assert_eq!(meta.content_preview.as_deref(), Some("Hey!"));
        assert_eq!(
            meta.reasoning_preview.as_deref(),
            Some("I should be brief.")
        );
        // Assembled body preserves both blocks in order.
        assert_eq!(body["content"][0]["type"], "thinking");
        assert_eq!(body["content"][0]["thinking"], "I should be brief.");
        assert_eq!(body["content"][1]["text"], "Hey!");
        assert_eq!(body["usage"]["cache_creation_input_tokens"], 39325);
    }

    #[test]
    fn parses_cache_and_reasoning_tokens_non_streaming() {
        let body = json!({
            "usage": {
                "input_tokens": 2, "output_tokens": 28,
                "cache_creation_input_tokens": 39325, "cache_read_input_tokens": 10,
                "output_tokens_details": {"thinking_tokens": 15}
            },
            "content": [{"type":"thinking","thinking":"hmm"},{"type":"text","text":"hi"}]
        });
        let meta = parse_response(&body);
        assert_eq!(meta.cache_creation_tokens, Some(39325));
        assert_eq!(meta.cache_read_tokens, Some(10));
        assert_eq!(meta.reasoning_tokens, Some(15));
        assert_eq!(meta.reasoning_preview.as_deref(), Some("hmm"));
    }

    #[test]
    fn ignores_malformed_sse_lines() {
        let raw = "data: not-json\n\ndata: {\"type\":\"message_delta\",\"usage\":{\"output_tokens\":3}}\n";
        let (meta, _) = reconstruct_sse(raw);
        assert_eq!(meta.output_tokens, Some(3));
    }

    #[test]
    fn recognizes_messages_path() {
        assert!(is_messages_path("/v1/messages"));
        assert!(is_messages_path("/v1/messages?beta=true"));
        assert!(!is_messages_path("/v1/complete"));
    }
}
