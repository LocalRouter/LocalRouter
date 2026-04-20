//! Wire types for the OpenAI Responses API.
//!
//! Mirrors the subset of types we need from Codex's
//! `codex-rs/codex-api/src/common.rs` + `protocol/src/models.rs`.
//! Kept deliberately minimal: only the fields we send/receive when
//! bridging our chat-completions-shaped `CompletionRequest` through
//! `POST https://chatgpt.com/backend-api/codex/responses` (or
//! `api.openai.com/v1/responses` once we pass through natively).

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Request body for `POST /responses`.
///
/// Fields are ordered to match Codex's canonical form so captured
/// golden fixtures line up verbatim.
#[derive(Debug, Clone, Serialize, PartialEq, Default)]
pub struct ResponsesApiRequest {
    pub model: String,

    /// System/developer prompt hoisted out of the messages array.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub instructions: String,

    /// Ordered conversation history as typed `ResponseItem`s.
    pub input: Vec<ResponseItem>,

    /// Tools passed as JSON verbatim — the `[{ "type": "function",
    /// "name": ..., "parameters": ... }]` form.
    #[serde(default)]
    pub tools: Vec<Value>,

    /// `"auto"`, `"none"`, `"required"`, or a JSON object for a
    /// specific function. We hardcode `"auto"` for ChatGPT-backend.
    pub tool_choice: String,

    pub parallel_tool_calls: bool,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<Reasoning>,

    /// Whether the server should persist the response for retrieval
    /// via `previous_response_id`. `false` for pass-through, `true`
    /// when our own Responses endpoint honors retention.
    pub store: bool,

    pub stream: bool,

    /// Additional fields to include on the response — commonly
    /// `["usage"]` when reasoning is requested.
    #[serde(default)]
    pub include: Vec<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_tier: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_cache_key: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<TextControls>,

    /// When continuing a prior turn, the id returned by the previous
    /// response. Server fills in missing history.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_response_id: Option<String>,
}

/// Controls the server's reasoning pass (o1/gpt-5 family).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Reasoning {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

/// `text` controls — verbosity + optional JSON schema output.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct TextControls {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verbosity: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<TextFormat>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TextFormat {
    #[serde(rename = "type")]
    pub format_type: String,
    pub strict: bool,
    pub schema: Value,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
}

/// A single item in the conversation. Typed enum to match the
/// Responses API `type`-tagged wire format.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ResponseItem {
    /// User/assistant/system text + multimodal message.
    Message {
        role: String,
        content: Vec<ContentItem>,
    },
    /// An assistant-issued tool call emitted as part of input when
    /// replaying history.
    FunctionCall {
        /// Opaque client-issued correlation id the server echoes on
        /// the matching tool-call output.
        call_id: String,
        name: String,
        /// JSON-encoded arguments (string, not an object).
        arguments: String,
    },
    /// The tool's response to a prior `FunctionCall`.
    FunctionCallOutput { call_id: String, output: String },
    /// Reasoning blob replayed in conversation history (o1/gpt-5
    /// chains). Passthrough only — we don't inspect `summary` /
    /// `content`.
    Reasoning {
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        summary: Vec<Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        content: Option<Vec<Value>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        encrypted_content: Option<String>,
    },
}

/// Content parts within a `ResponseItem::Message`. The `_text` /
/// `_image` split mirrors the Responses API's `input_*` / `output_*`
/// wire names; we use `input_*` for request-side items and
/// `output_text` on responses. Serde's `type` tag handles both.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentItem {
    InputText {
        text: String,
    },
    InputImage {
        image_url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
    },
    OutputText {
        text: String,
    },
}

// ============================================================================
// Response (non-streaming) types
// ============================================================================

/// Full response body returned by `POST /responses` when `stream=false`.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct ResponseObject {
    pub id: String,
    #[serde(default)]
    pub object: Option<String>,
    #[serde(default)]
    pub created_at: Option<i64>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub output: Vec<OutputItem>,
    #[serde(default)]
    pub usage: Option<ResponsesUsage>,
}

/// One assistant-produced item in the response.
///
/// We only decode the two kinds we care about:
///  - `message` → text (+ any tool citations inline)
///  - `function_call` → a tool call the assistant wants invoked
///
/// Reasoning items etc. are represented as `Other` so unknown
/// variants don't break deserialization.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OutputItem {
    Message {
        #[serde(default)]
        id: Option<String>,
        #[serde(default)]
        role: Option<String>,
        #[serde(default)]
        content: Vec<ContentItem>,
    },
    FunctionCall {
        #[serde(default)]
        id: Option<String>,
        call_id: String,
        name: String,
        arguments: String,
    },
    #[serde(other)]
    Other,
}

/// Token-usage returned with the final response / `response.completed`
/// streaming event.
#[derive(Debug, Clone, Deserialize, PartialEq, Default)]
pub struct ResponsesUsage {
    #[serde(default)]
    pub input_tokens: u32,
    #[serde(default)]
    pub output_tokens: u32,
    #[serde(default)]
    pub total_tokens: u32,
    #[serde(default)]
    pub output_tokens_details: Option<OutputTokensDetails>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Default)]
pub struct OutputTokensDetails {
    #[serde(default)]
    pub reasoning_tokens: u32,
}

// ============================================================================
// Streaming SSE event envelope
// ============================================================================

/// A raw SSE payload from `/responses` before we classify it.
///
/// We decode the envelope to read the `type` discriminator; the
/// payload-specific fields are picked out by the streaming adapter.
#[derive(Debug, Clone, Deserialize)]
pub struct ResponsesSseEnvelope {
    #[serde(rename = "type")]
    pub event_type: String,
    #[serde(default)]
    pub delta: Option<String>,
    #[serde(default)]
    pub item: Option<serde_json::Value>,
    #[serde(default)]
    pub response: Option<serde_json::Value>,
    #[serde(default)]
    pub item_id: Option<String>,
    #[serde(default)]
    pub output_index: Option<i64>,
    #[serde(default)]
    pub error: Option<ResponsesSseError>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ResponsesSseError {
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub code: Option<String>,
    #[serde(rename = "type", default)]
    pub error_type: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_plain_chat_request() {
        let req = ResponsesApiRequest {
            model: "gpt-4o".into(),
            instructions: "Be helpful.".into(),
            input: vec![ResponseItem::Message {
                role: "user".into(),
                content: vec![ContentItem::InputText { text: "hi".into() }],
            }],
            tools: vec![],
            tool_choice: "auto".into(),
            parallel_tool_calls: true,
            reasoning: None,
            store: false,
            stream: true,
            include: vec![],
            service_tier: None,
            prompt_cache_key: None,
            text: None,
            previous_response_id: None,
        };
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["instructions"], "Be helpful.");
        assert_eq!(json["input"][0]["role"], "user");
        assert_eq!(json["input"][0]["content"][0]["type"], "input_text");
        assert_eq!(json["tool_choice"], "auto");
        assert_eq!(json["parallel_tool_calls"], true);
    }

    #[test]
    fn function_call_roundtrip_item() {
        let item = ResponseItem::FunctionCall {
            call_id: "call_abc".into(),
            name: "search".into(),
            arguments: r#"{"q":"rust"}"#.into(),
        };
        let v = serde_json::to_value(&item).unwrap();
        assert_eq!(v["type"], "function_call");
        assert_eq!(v["call_id"], "call_abc");
        let back: ResponseItem = serde_json::from_value(v).unwrap();
        assert_eq!(back, item);
    }

    #[test]
    fn response_object_parses_with_unknown_output_variants() {
        let payload = serde_json::json!({
            "id": "resp_1",
            "status": "completed",
            "output": [
                {"type": "reasoning", "summary": []},
                {"type": "message", "role": "assistant",
                 "content": [{"type": "output_text", "text": "hi"}]}
            ],
            "usage": {"input_tokens": 3, "output_tokens": 1, "total_tokens": 4}
        });
        let r: ResponseObject = serde_json::from_value(payload).unwrap();
        assert_eq!(r.id, "resp_1");
        assert_eq!(r.output.len(), 2);
        assert!(matches!(r.output[0], OutputItem::Other));
        let OutputItem::Message { content, .. } = &r.output[1] else {
            panic!("expected message");
        };
        assert!(matches!(&content[0], ContentItem::OutputText { text } if text == "hi"));
    }
}
