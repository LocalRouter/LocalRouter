//! Parsing of OpenAI-format exchanges observed by the proxy: Chat Completions
//! and the Responses API (what Codex CLI speaks, both against `api.openai.com`
//! and the ChatGPT Codex backend on `chatgpt.com`).
//!
//! Mirrors `anthropic.rs`: extract request/response metadata for the monitor
//! (model, tokens, cost inputs, previews) and reconstruct SSE streams into a
//! full response body so streamed calls are captured like plain ones.

use serde_json::Value;

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

fn str_field(v: Option<&Value>, key: &str) -> Option<String> {
    v.and_then(|v| v.get(key))
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn u64_field(v: Option<&Value>, key: &str) -> Option<u64> {
    v.and_then(|v| v.get(key)).and_then(Value::as_u64)
}

// ---------------------------------------------------------------------------
// Chat Completions
// ---------------------------------------------------------------------------

/// Extract request metadata from a Chat Completions request body.
pub fn parse_chat_request(body: &Value) -> RequestMeta {
    RequestMeta {
        model: str_field(Some(body), "model"),
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

/// Extract response metadata from a non-streaming Chat Completions response.
pub fn parse_chat_response(body: &Value) -> ResponseMeta {
    let usage = body.get("usage");
    let first_choice = body
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|c| c.first());
    let content = first_choice
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(Value::as_str);
    ResponseMeta {
        message_id: str_field(Some(body), "id"),
        model: str_field(Some(body), "model"),
        input_tokens: u64_field(usage, "prompt_tokens"),
        output_tokens: u64_field(usage, "completion_tokens"),
        cache_creation_tokens: None,
        cache_read_tokens: usage
            .and_then(|u| u.get("prompt_tokens_details"))
            .and_then(|d| d.get("cached_tokens"))
            .and_then(Value::as_u64),
        reasoning_tokens: usage
            .and_then(|u| u.get("completion_tokens_details"))
            .and_then(|d| d.get("reasoning_tokens"))
            .and_then(Value::as_u64),
        stop_reason: first_choice
            .and_then(|c| c.get("finish_reason"))
            .and_then(Value::as_str)
            .map(str::to_string),
        content_preview: content.map(truncate),
        reasoning_preview: None,
    }
}

/// Reconstruct a Chat Completions SSE stream into (metadata, assembled body).
///
/// Accumulates `choices[0].delta` chunks (content + tool calls); the final
/// usage chunk (`stream_options.include_usage`) supplies token counts.
pub fn reconstruct_chat_sse(raw: &str) -> (ResponseMeta, Value) {
    let mut meta = ResponseMeta::default();
    let mut content = String::new();
    // Tool calls accumulate by index: (id, name, arguments-json).
    let mut tools: Vec<(Option<String>, String, String)> = Vec::new();

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

        if meta.message_id.is_none() {
            meta.message_id = str_field(Some(&json), "id");
        }
        if meta.model.is_none() {
            meta.model = str_field(Some(&json), "model");
        }
        if let Some(usage) = json.get("usage").filter(|u| !u.is_null()) {
            meta.input_tokens = u64_field(Some(usage), "prompt_tokens");
            meta.output_tokens = u64_field(Some(usage), "completion_tokens");
            meta.cache_read_tokens = usage
                .get("prompt_tokens_details")
                .and_then(|d| d.get("cached_tokens"))
                .and_then(Value::as_u64);
            meta.reasoning_tokens = usage
                .get("completion_tokens_details")
                .and_then(|d| d.get("reasoning_tokens"))
                .and_then(Value::as_u64);
        }

        let Some(choice) = json
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|c| c.first())
        else {
            continue;
        };
        if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
            meta.stop_reason = Some(reason.to_string());
        }
        let Some(delta) = choice.get("delta") else {
            continue;
        };
        if let Some(t) = delta.get("content").and_then(Value::as_str) {
            content.push_str(t);
        }
        if let Some(calls) = delta.get("tool_calls").and_then(Value::as_array) {
            for call in calls {
                let idx = call.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
                if idx >= tools.len() {
                    tools.resize_with(idx + 1, Default::default);
                }
                let slot = &mut tools[idx];
                if let Some(id) = call.get("id").and_then(Value::as_str) {
                    slot.0 = Some(id.to_string());
                }
                if let Some(f) = call.get("function") {
                    if let Some(n) = f.get("name").and_then(Value::as_str) {
                        slot.1.push_str(n);
                    }
                    if let Some(a) = f.get("arguments").and_then(Value::as_str) {
                        slot.2.push_str(a);
                    }
                }
            }
        }
    }

    meta.content_preview = (!content.is_empty()).then(|| truncate(&content));

    // Assemble a non-streaming-shaped body for the monitor's response view.
    let tool_calls: Vec<Value> = tools
        .into_iter()
        .map(|(id, name, args)| {
            serde_json::json!({
                "id": id,
                "type": "function",
                "function": { "name": name, "arguments": args },
            })
        })
        .collect();
    let mut message = serde_json::Map::new();
    message.insert("role".into(), "assistant".into());
    message.insert("content".into(), content.into());
    if !tool_calls.is_empty() {
        message.insert("tool_calls".into(), tool_calls.into());
    }
    let body = serde_json::json!({
        "id": meta.message_id,
        "model": meta.model,
        "choices": [{ "message": message, "finish_reason": meta.stop_reason }],
        "usage": {
            "prompt_tokens": meta.input_tokens,
            "completion_tokens": meta.output_tokens,
        },
    });
    (meta, body)
}

// ---------------------------------------------------------------------------
// Responses API (Codex)
// ---------------------------------------------------------------------------

/// Extract request metadata from a Responses API request body.
pub fn parse_responses_request(body: &Value) -> RequestMeta {
    let message_count = match body.get("input") {
        Some(Value::Array(items)) => items.len(),
        Some(Value::String(_)) => 1,
        _ => 0,
    };
    RequestMeta {
        model: str_field(Some(body), "model"),
        stream: body.get("stream").and_then(Value::as_bool).unwrap_or(false),
        message_count,
        has_tools: body
            .get("tools")
            .and_then(Value::as_array)
            .is_some_and(|t| !t.is_empty()),
    }
}

/// Extract response metadata from a non-streaming Responses API response
/// (also used on the final `response.completed` SSE event, which carries the
/// complete response object).
pub fn parse_responses_response(body: &Value) -> ResponseMeta {
    let usage = body.get("usage");
    let output = body.get("output").and_then(Value::as_array);

    // Concatenate output_text across message items, and reasoning summaries.
    let mut text = String::new();
    let mut reasoning = String::new();
    for item in output.into_iter().flatten() {
        match item.get("type").and_then(Value::as_str) {
            Some("message") => {
                for part in item
                    .get("content")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    if part.get("type").and_then(Value::as_str) == Some("output_text") {
                        if let Some(t) = part.get("text").and_then(Value::as_str) {
                            text.push_str(t);
                        }
                    }
                }
            }
            Some("reasoning") => {
                for part in item
                    .get("summary")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    if let Some(t) = part.get("text").and_then(Value::as_str) {
                        reasoning.push_str(t);
                    }
                }
            }
            _ => {}
        }
    }

    ResponseMeta {
        message_id: str_field(Some(body), "id"),
        model: str_field(Some(body), "model"),
        input_tokens: u64_field(usage, "input_tokens"),
        output_tokens: u64_field(usage, "output_tokens"),
        cache_creation_tokens: None,
        cache_read_tokens: usage
            .and_then(|u| u.get("input_tokens_details"))
            .and_then(|d| d.get("cached_tokens"))
            .and_then(Value::as_u64),
        reasoning_tokens: usage
            .and_then(|u| u.get("output_tokens_details"))
            .and_then(|d| d.get("reasoning_tokens"))
            .and_then(Value::as_u64),
        stop_reason: str_field(Some(body), "status"),
        content_preview: (!text.is_empty()).then(|| truncate(&text)),
        reasoning_preview: (!reasoning.is_empty()).then(|| truncate(&reasoning)),
    }
}

/// Reconstruct a Responses API SSE stream into (metadata, assembled body).
///
/// The final `response.completed` event carries the entire response object, so
/// when present we parse that directly; otherwise we fall back to accumulating
/// `response.output_text.delta` / reasoning-summary deltas.
pub fn reconstruct_responses_sse(raw: &str) -> (ResponseMeta, Value) {
    let mut completed: Option<Value> = None;
    let mut text = String::new();
    let mut reasoning = String::new();
    let mut meta = ResponseMeta::default();

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
        match json.get("type").and_then(Value::as_str) {
            Some("response.created") => {
                let r = json.get("response");
                meta.message_id = str_field(r, "id");
                meta.model = str_field(r, "model");
            }
            Some("response.output_text.delta") => {
                if let Some(t) = json.get("delta").and_then(Value::as_str) {
                    text.push_str(t);
                }
            }
            Some("response.reasoning_summary_text.delta") => {
                if let Some(t) = json.get("delta").and_then(Value::as_str) {
                    reasoning.push_str(t);
                }
            }
            Some("response.completed") | Some("response.incomplete") | Some("response.failed") => {
                if let Some(r) = json.get("response") {
                    completed = Some(r.clone());
                }
            }
            _ => {}
        }
    }

    if let Some(body) = completed {
        let meta = parse_responses_response(&body);
        return (meta, body);
    }

    // Stream ended without a terminal event (disconnect): salvage the deltas.
    meta.content_preview = (!text.is_empty()).then(|| truncate(&text));
    meta.reasoning_preview = (!reasoning.is_empty()).then(|| truncate(&reasoning));
    let body = serde_json::json!({
        "id": meta.message_id,
        "model": meta.model,
        "output": [{
            "type": "message",
            "content": [{ "type": "output_text", "text": text }],
        }],
    });
    (meta, body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_chat_request_and_response() {
        let req = parse_chat_request(&json!({
            "model": "gpt-5.2",
            "stream": true,
            "messages": [{"role": "user", "content": "hi"}],
            "tools": [{"type": "function", "function": {"name": "t"}}],
        }));
        assert_eq!(req.model.as_deref(), Some("gpt-5.2"));
        assert!(req.stream);
        assert_eq!(req.message_count, 1);
        assert!(req.has_tools);

        let resp = parse_chat_response(&json!({
            "id": "chatcmpl-1",
            "model": "gpt-5.2",
            "choices": [{"message": {"role": "assistant", "content": "hello"}, "finish_reason": "stop"}],
            "usage": {
                "prompt_tokens": 10,
                "completion_tokens": 4,
                "prompt_tokens_details": {"cached_tokens": 6},
                "completion_tokens_details": {"reasoning_tokens": 2},
            },
        }));
        assert_eq!(resp.message_id.as_deref(), Some("chatcmpl-1"));
        assert_eq!(resp.input_tokens, Some(10));
        assert_eq!(resp.output_tokens, Some(4));
        assert_eq!(resp.cache_read_tokens, Some(6));
        assert_eq!(resp.reasoning_tokens, Some(2));
        assert_eq!(resp.stop_reason.as_deref(), Some("stop"));
        assert_eq!(resp.content_preview.as_deref(), Some("hello"));
    }

    #[test]
    fn reconstructs_chat_sse() {
        let raw = concat!(
            "data: {\"id\":\"c1\",\"model\":\"gpt-5.2\",\"choices\":[{\"delta\":{\"role\":\"assistant\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"Hel\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"lo\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
            "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":7,\"completion_tokens\":2}}\n\n",
            "data: [DONE]\n\n",
        );
        let (meta, body) = reconstruct_chat_sse(raw);
        assert_eq!(meta.message_id.as_deref(), Some("c1"));
        assert_eq!(meta.content_preview.as_deref(), Some("Hello"));
        assert_eq!(meta.stop_reason.as_deref(), Some("stop"));
        assert_eq!(meta.input_tokens, Some(7));
        assert_eq!(meta.output_tokens, Some(2));
        assert_eq!(body["choices"][0]["message"]["content"], "Hello");
    }

    #[test]
    fn reconstructs_chat_sse_tool_calls() {
        let raw = concat!(
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_1\",\"function\":{\"name\":\"get_weather\",\"arguments\":\"{\\\"ci\"}}]}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"ty\\\":\\\"SF\\\"}\"}}]}}]}\n\n",
        );
        let (_, body) = reconstruct_chat_sse(raw);
        let call = &body["choices"][0]["message"]["tool_calls"][0];
        assert_eq!(call["function"]["name"], "get_weather");
        assert_eq!(call["function"]["arguments"], "{\"city\":\"SF\"}");
    }

    #[test]
    fn parses_responses_request_and_response() {
        let req = parse_responses_request(&json!({
            "model": "gpt-5.1-codex",
            "stream": true,
            "input": [{"role": "user", "content": [{"type": "input_text", "text": "hi"}]}],
            "tools": [{"type": "function", "name": "shell"}],
        }));
        assert_eq!(req.model.as_deref(), Some("gpt-5.1-codex"));
        assert_eq!(req.message_count, 1);
        assert!(req.has_tools);

        let resp = parse_responses_response(&json!({
            "id": "resp_1",
            "model": "gpt-5.1-codex",
            "status": "completed",
            "output": [
                {"type": "reasoning", "summary": [{"type": "summary_text", "text": "thinking…"}]},
                {"type": "message", "content": [{"type": "output_text", "text": "done"}]},
            ],
            "usage": {
                "input_tokens": 100,
                "output_tokens": 20,
                "input_tokens_details": {"cached_tokens": 80},
                "output_tokens_details": {"reasoning_tokens": 12},
            },
        }));
        assert_eq!(resp.message_id.as_deref(), Some("resp_1"));
        assert_eq!(resp.input_tokens, Some(100));
        assert_eq!(resp.output_tokens, Some(20));
        assert_eq!(resp.cache_read_tokens, Some(80));
        assert_eq!(resp.reasoning_tokens, Some(12));
        assert_eq!(resp.stop_reason.as_deref(), Some("completed"));
        assert_eq!(resp.content_preview.as_deref(), Some("done"));
        assert_eq!(resp.reasoning_preview.as_deref(), Some("thinking…"));
    }

    #[test]
    fn responses_sse_uses_completed_event() {
        let raw = concat!(
            "event: response.created\n",
            "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_9\",\"model\":\"gpt-5.1-codex\"}}\n\n",
            "event: response.output_text.delta\n",
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"par\"}\n\n",
            "event: response.completed\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_9\",\"model\":\"gpt-5.1-codex\",\"status\":\"completed\",\"output\":[{\"type\":\"message\",\"content\":[{\"type\":\"output_text\",\"text\":\"partial done\"}]}],\"usage\":{\"input_tokens\":5,\"output_tokens\":3}}}\n\n",
        );
        let (meta, body) = reconstruct_responses_sse(raw);
        assert_eq!(meta.message_id.as_deref(), Some("resp_9"));
        assert_eq!(meta.content_preview.as_deref(), Some("partial done"));
        assert_eq!(meta.input_tokens, Some(5));
        assert_eq!(body["status"], "completed");
    }

    #[test]
    fn responses_sse_salvages_deltas_without_terminal_event() {
        let raw = concat!(
            "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_2\",\"model\":\"m\"}}\n\n",
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"cut o\"}\n\n",
        );
        let (meta, _) = reconstruct_responses_sse(raw);
        assert_eq!(meta.message_id.as_deref(), Some("resp_2"));
        assert_eq!(meta.content_preview.as_deref(), Some("cut o"));
    }
}
