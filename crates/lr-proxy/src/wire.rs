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
    fn maps_hosts_to_providers() {
        assert_eq!(provider_for_host("api.anthropic.com"), "anthropic");
        assert_eq!(provider_for_host("api.openai.com"), "openai");
        assert_eq!(provider_for_host("chatgpt.com"), "openai");
        assert_eq!(provider_for_host("example.com"), "unknown");
    }
}
