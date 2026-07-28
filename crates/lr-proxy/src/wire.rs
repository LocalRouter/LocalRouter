//! Wire-format dispatch for intercepted LLM traffic.
//!
//! The proxy forwards *any* host (unrecognized ones are blind-tunneled); this
//! module decides which decrypted requests are LLM API calls we know how to
//! parse for the Monitor/firewall, and routes them to the right parser:
//! Anthropic Messages, OpenAI Chat Completions, or the OpenAI Responses API
//! (used by Codex, both via `api.openai.com` and the ChatGPT Codex backend).

use serde_json::Value;

use crate::{anthropic, openai};

/// The request/response encoding of an intercepted LLM call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WireFormat {
    /// Anthropic Messages (`POST /v1/messages`) — Claude Code, Claude SDKs.
    AnthropicMessages,
    /// OpenAI Chat Completions (`POST .../chat/completions`).
    OpenAiChat,
    /// OpenAI Responses API (`POST .../responses`) — Codex CLI.
    OpenAiResponses,
}

/// Request-side metadata extracted from an intercepted LLM request body.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct RequestMeta {
    pub model: Option<String>,
    pub stream: bool,
    pub message_count: usize,
    pub has_tools: bool,
}

/// Response-side metadata extracted from an intercepted LLM response
/// (either a single JSON object or a reconstructed SSE stream).
#[derive(Debug, Default, Clone, PartialEq)]
pub struct ResponseMeta {
    pub message_id: Option<String>,
    pub model: Option<String>,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    /// Tokens written to the prompt cache (Anthropic-only; billed ~1.25x input).
    pub cache_creation_tokens: Option<u64>,
    /// Tokens read from the prompt cache.
    pub cache_read_tokens: Option<u64>,
    /// Reasoning/thinking output tokens.
    pub reasoning_tokens: Option<u64>,
    pub stop_reason: Option<String>,
    pub content_preview: Option<String>,
    /// Concatenated reasoning text (Anthropic `thinking`, OpenAI reasoning summary).
    pub reasoning_preview: Option<String>,
}

/// Identify the wire format of a decrypted request path, or `None` if it is not
/// an LLM call we record (auth, telemetry, model listings, … pass untouched).
pub fn detect(path: &str) -> Option<WireFormat> {
    let path = path.split('?').next().unwrap_or(path);
    if anthropic::is_messages_path(path) {
        Some(WireFormat::AnthropicMessages)
    } else if path.ends_with("/chat/completions") {
        Some(WireFormat::OpenAiChat)
    } else if path.ends_with("/responses") {
        Some(WireFormat::OpenAiResponses)
    } else {
        None
    }
}

/// Catalog/metrics provider name for an intercepted host.
pub fn provider_for_host(host: &str) -> &'static str {
    let host = host.to_ascii_lowercase();
    if host.contains("anthropic") {
        "anthropic"
    } else if host.contains("openai") || host.contains("chatgpt") {
        "openai"
    } else {
        "unknown"
    }
}

/// Split a raw SSE stream into its JSON `data:` payloads, shared by every
/// wire format so resilience fixes land in one place.
///
/// Spec-correct where it matters for robustness: tolerates CRLF line endings
/// and leading whitespace, accumulates multi-line `data:` fields into one
/// payload, ignores `event:`/`id:`/`retry:` fields and comments, and skips
/// `[DONE]` markers and payloads that aren't valid JSON (e.g. a final line
/// truncated by the capture cap).
pub fn sse_json_events(raw: &str) -> Vec<Value> {
    fn flush(data: &mut String, events: &mut Vec<Value>) {
        let payload = std::mem::take(data);
        let payload = payload.trim();
        if payload.is_empty() || payload == "[DONE]" {
            return;
        }
        if let Ok(json) = serde_json::from_str::<Value>(payload) {
            events.push(json);
        }
    }

    let mut events = Vec::new();
    let mut data = String::new();
    for line in raw.lines() {
        let line = line.strip_suffix('\r').unwrap_or(line).trim_start();
        if line.is_empty() {
            flush(&mut data, &mut events);
            continue;
        }
        if let Some(rest) = line.strip_prefix("data:") {
            if !data.is_empty() {
                data.push('\n');
            }
            data.push_str(rest.strip_prefix(' ').unwrap_or(rest));
        }
    }
    flush(&mut data, &mut events);
    events
}

/// True when a stream event marks the end of one request/response cycle.
/// Used by the websocket relay, where one connection carries many cycles and
/// there is no HTTP response boundary to lean on.
pub fn is_terminal_event(format: WireFormat, event: &Value) -> bool {
    let kind = event
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    match format {
        WireFormat::AnthropicMessages => matches!(kind, "message_stop" | "error"),
        WireFormat::OpenAiResponses => matches!(
            kind,
            "response.completed" | "response.failed" | "response.incomplete" | "error"
        ),
        // Chat Completions has no websocket transport; cycles close on
        // connection end instead.
        WireFormat::OpenAiChat => false,
    }
}

/// HTTP-ish status for a monitor event closed by a terminal stream event:
/// success terminals map to 200, error terminals carry their own status when
/// the event includes one (Codex wraps upstream HTTP errors that way).
pub fn terminal_status(_format: WireFormat, event: &Value) -> u16 {
    let kind = event
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    match kind {
        "response.completed" | "response.incomplete" | "message_stop" => 200,
        _ => event
            .get("status")
            .or_else(|| event.get("status_code"))
            .and_then(Value::as_u64)
            .and_then(|s| u16::try_from(s).ok())
            .unwrap_or(502),
    }
}

/// Extract request metadata from a parsed request body.
pub fn parse_request(format: WireFormat, body: &Value) -> RequestMeta {
    match format {
        WireFormat::AnthropicMessages => anthropic::parse_request(body),
        WireFormat::OpenAiChat => openai::parse_chat_request(body),
        WireFormat::OpenAiResponses => openai::parse_responses_request(body),
    }
}

/// Extract response metadata from a single (non-streaming) response body.
pub fn parse_response(format: WireFormat, body: &Value) -> ResponseMeta {
    match format {
        WireFormat::AnthropicMessages => anthropic::parse_response(body),
        WireFormat::OpenAiChat => openai::parse_chat_response(body),
        WireFormat::OpenAiResponses => openai::parse_responses_response(body),
    }
}

/// Reconstruct an SSE stream into (metadata, assembled response body).
pub fn reconstruct_sse(format: WireFormat, raw: &str) -> (ResponseMeta, Value) {
    match format {
        WireFormat::AnthropicMessages => anthropic::reconstruct_sse(raw),
        WireFormat::OpenAiChat => openai::reconstruct_chat_sse(raw),
        WireFormat::OpenAiResponses => openai::reconstruct_responses_sse(raw),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_known_llm_paths() {
        assert_eq!(
            detect("/v1/messages?beta=true"),
            Some(WireFormat::AnthropicMessages)
        );
        assert_eq!(detect("/v1/chat/completions"), Some(WireFormat::OpenAiChat));
        assert_eq!(detect("/v1/responses"), Some(WireFormat::OpenAiResponses));
        // Codex's ChatGPT-subscription backend.
        assert_eq!(
            detect("/backend-api/codex/responses"),
            Some(WireFormat::OpenAiResponses)
        );
        assert_eq!(detect("/v1/models"), None);
        assert_eq!(detect("/api/auth/session"), None);
    }

    #[test]
    fn sse_splitter_handles_crlf_multiline_and_noise() {
        let raw = concat!(
            ": comment\r\n",
            "event: message_start\r\n",
            "data: {\"a\":\r\n",
            "data:  1}\r\n",
            "\r\n",
            "data: not-json\n",
            "\n",
            "data: [DONE]\n",
            "\n",
            "data: {\"b\":2}\n",
        );
        let events = sse_json_events(raw);
        assert_eq!(events.len(), 2);
        // Multi-line data fields join with a newline per the SSE spec.
        assert_eq!(events[0]["a"], 1);
        // A final event without a trailing blank line still flushes.
        assert_eq!(events[1]["b"], 2);
    }

    #[test]
    fn terminal_events_per_format() {
        use serde_json::json;
        let done = json!({"type": "response.completed"});
        let failed = json!({"type": "response.failed"});
        let err = json!({"type": "error", "status": 429});
        assert!(is_terminal_event(WireFormat::OpenAiResponses, &done));
        assert!(is_terminal_event(WireFormat::OpenAiResponses, &failed));
        assert!(is_terminal_event(WireFormat::OpenAiResponses, &err));
        assert!(!is_terminal_event(
            WireFormat::OpenAiResponses,
            &json!({"type": "response.output_text.delta"})
        ));
        assert!(is_terminal_event(
            WireFormat::AnthropicMessages,
            &json!({"type": "message_stop"})
        ));
        assert_eq!(terminal_status(WireFormat::OpenAiResponses, &done), 200);
        assert_eq!(terminal_status(WireFormat::OpenAiResponses, &err), 429);
        assert_eq!(terminal_status(WireFormat::OpenAiResponses, &failed), 502);
    }

    #[test]
    fn maps_hosts_to_providers() {
        assert_eq!(provider_for_host("api.anthropic.com"), "anthropic");
        assert_eq!(provider_for_host("api.openai.com"), "openai");
        assert_eq!(provider_for_host("chatgpt.com"), "openai");
        assert_eq!(provider_for_host("example.com"), "unknown");
    }
}
