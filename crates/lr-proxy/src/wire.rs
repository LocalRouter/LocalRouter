//! Wire-format dispatch for intercepted LLM traffic.
//!
//! The proxy forwards *any* host (unrecognized ones are blind-tunneled); this
//! module decides which decrypted requests are LLM API calls we know how to
//! parse for the Monitor/firewall, and routes them to the right parser:
//! Anthropic Messages, OpenAI Chat Completions, or the OpenAI Responses API
//! (used by Codex, both via `api.openai.com` and the ChatGPT Codex backend).

use lr_secret_scanner::request_stream::RequestMetadata as SniffedRequest;
use serde_json::Value;

use crate::{anthropic, ollama, openai, systemone};

/// The request/response encoding of an intercepted LLM call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WireFormat {
    /// Anthropic Messages (`POST /v1/messages`) — Claude Code, Claude SDKs.
    AnthropicMessages,
    /// OpenAI Chat Completions (`POST .../chat/completions`).
    OpenAiChat,
    /// OpenAI Responses API (`POST .../responses`) — Codex CLI.
    OpenAiResponses,
    /// Codex standalone web search (`POST .../alpha/search`), plain JSON.
    CodexSearch,
    /// Image generation and edits (`POST .../images/{generations,edits}`),
    /// JSON from Codex's image tool.
    Images,
    /// Ollama's native API (`POST /api/chat`, `POST /api/generate`), seen by
    /// the reverse proxy when it wraps a local Ollama. Streams NDJSON, not SSE.
    Ollama(ollama::OllamaEndpoint),
    /// System One decisions (`POST .../v1/systemone`): TypeSafe's hosted Jev,
    /// or Laya / Kev behind the reverse proxy. Plain JSON, never streamed.
    SystemOne,
    Decisions,
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
    if crate::codex_search::is_search_path(path) {
        Some(WireFormat::CodexSearch)
    } else if crate::images::is_images_path(path) {
        Some(WireFormat::Images)
    } else if anthropic::is_messages_path(path) {
        Some(WireFormat::AnthropicMessages)
    } else if path.ends_with("/completions") {
        Some(WireFormat::OpenAiChat)
    } else if path.ends_with("/responses")
        || path.ends_with("/responses/compact")
        || response_id(path).is_some()
    {
        Some(WireFormat::OpenAiResponses)
    } else if path.ends_with("/decisions") {
        Some(WireFormat::Decisions)
    } else if systemone::is_systemone_path(path) {
        Some(WireFormat::SystemOne)
    } else {
        ollama::detect(path).map(WireFormat::Ollama)
    }
}

/// A retrieval is authorized through the original response's owner/scope alias.
pub fn response_id(path: &str) -> Option<String> {
    lr_secret_scanner::wire::response_id(path)
}

fn valid_conversation_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 256 && !id.chars().any(char::is_control)
}

/// Explicit caller identity for diagnostics only; never an authorization grant.
pub fn conversation_id(body: &Value) -> Option<String> {
    for path in [
        "/client_metadata/session_id",
        "/client_metadata/thread_id",
        "/metadata/session_id",
    ] {
        if let Some(id) = body
            .pointer(path)
            .and_then(Value::as_str)
            .filter(|id| valid_conversation_id(id))
        {
            return Some(id.to_owned());
        }
    }
    let metadata: Value =
        serde_json::from_str(body.pointer("/metadata/user_id")?.as_str()?).ok()?;
    metadata["session_id"]
        .as_str()
        .filter(|id| valid_conversation_id(id))
        .map(str::to_owned)
}

/// [`conversation_id`] for a body that was sniffed while it streamed. Equal to
/// it unless a field it reads overflowed the sniffer's capture bound.
pub fn conversation_id_from_sniffed(sniffed: &SniffedRequest) -> Option<String> {
    let client = sniffed.client_metadata_json();
    for key in ["session_id", "thread_id"] {
        if let Some(id) = client
            .as_ref()
            .and_then(|client| client.get(key))
            .and_then(Value::as_str)
            .filter(|id| valid_conversation_id(id))
        {
            return Some(id.to_owned());
        }
    }
    if let Some(id) = sniffed
        .metadata_session_id
        .value()
        .filter(|id| valid_conversation_id(id))
    {
        return Some(id.clone());
    }
    let metadata: Value = serde_json::from_str(sniffed.metadata_user_id.value()?).ok()?;
    metadata["session_id"]
        .as_str()
        .filter(|id| valid_conversation_id(id))
        .map(str::to_owned)
}

/// Provider name the proxy records Anthropic traffic under.
pub const ANTHROPIC_PROVIDER: &str = "anthropic";
/// Provider name the proxy records OpenAI / ChatGPT traffic under.
pub const OPENAI_PROVIDER: &str = "openai";

/// Catalog/metrics provider name for an intercepted host.
pub fn provider_for_host(host: &str) -> &'static str {
    let host = host.to_ascii_lowercase();
    if host.contains("anthropic") {
        ANTHROPIC_PROVIDER
    } else if host.contains("openai") || host.contains("chatgpt") {
        OPENAI_PROVIDER
    } else if host.contains("typesafe") {
        "typesafe"
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
        // connection end instead. Neither does Ollama's native API — the
        // reverse proxy sees plain HTTP request/response pairs.
        WireFormat::OpenAiChat
        | WireFormat::CodexSearch
        | WireFormat::Images
        | WireFormat::Ollama(_)
        | WireFormat::SystemOne
        | WireFormat::Decisions => false,
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
        WireFormat::CodexSearch => crate::codex_search::parse_request(body),
        WireFormat::Images => crate::images::parse_request(body),
        WireFormat::Ollama(endpoint) => ollama::parse_request(endpoint, body),
        WireFormat::SystemOne => systemone::parse_request(body),
        WireFormat::Decisions => crate::decisions::parse_request(body),
    }
}

/// [`parse_request`] for a body that was sniffed while it streamed instead of
/// parsed: the same metadata for Anthropic Messages, Chat Completions and the
/// Responses API. `None` for the other formats, whose metadata comes from
/// fields the sniffer does not collect.
///
/// A `model` longer than the sniffer's capture bound is reported as unset.
pub fn request_meta_from_sniffed(
    format: WireFormat,
    sniffed: &SniffedRequest,
) -> Option<RequestMeta> {
    let message_count = match format {
        WireFormat::AnthropicMessages
        | WireFormat::OpenAiChat
        | WireFormat::Ollama(ollama::OllamaEndpoint::Chat) => sniffed.messages_len(),
        WireFormat::OpenAiResponses => sniffed.input_len(),
        // A streamed generate request has its conversation member, the
        // prompt: one logical message.
        WireFormat::Ollama(ollama::OllamaEndpoint::Generate) => 1,
        _ => return None,
    };
    // Ollama streams unless the caller opts out.
    let stream = match format {
        WireFormat::Ollama(_) => sniffed.stream.value().copied().unwrap_or(true),
        _ => sniffed.is_stream(),
    };
    Some(RequestMeta {
        model: sniffed.model.value().cloned(),
        stream,
        message_count: usize::try_from(message_count).unwrap_or(usize::MAX),
        has_tools: sniffed.has_tools(),
    })
}

/// Extract response metadata from a single (non-streaming) response body.
pub fn parse_response(format: WireFormat, body: &Value) -> ResponseMeta {
    match format {
        WireFormat::AnthropicMessages => anthropic::parse_response(body),
        WireFormat::OpenAiChat => openai::parse_chat_response(body),
        WireFormat::OpenAiResponses => openai::parse_responses_response(body),
        WireFormat::CodexSearch => crate::codex_search::parse_response(body),
        WireFormat::Images => crate::images::parse_response(body),
        WireFormat::Ollama(endpoint) => ollama::parse_response(endpoint, body),
        WireFormat::SystemOne => systemone::parse_response(body),
        WireFormat::Decisions => crate::decisions::parse_response(body),
    }
}

/// Reconstruct an SSE stream into (metadata, assembled response body).
pub fn reconstruct_sse(format: WireFormat, raw: &str) -> (ResponseMeta, Value) {
    match format {
        WireFormat::AnthropicMessages => anthropic::reconstruct_sse(raw),
        WireFormat::OpenAiChat => openai::reconstruct_chat_sse(raw),
        WireFormat::OpenAiResponses => openai::reconstruct_responses_sse(raw),
        // Ollama never sends SSE on its native API; treat a mislabeled stream
        // as NDJSON rather than losing the exchange.
        WireFormat::Ollama(endpoint) => ollama::reconstruct_ndjson(endpoint, raw),
        // System One never streams; if a body was captured as a stream, parse
        // it as the single JSON object it is.
        WireFormat::Decisions => match serde_json::from_str::<Value>(raw.trim()) {
            Ok(body) => (crate::decisions::parse_response(&body), body),
            Err(_) => (ResponseMeta::default(), Value::Null),
        },
        WireFormat::CodexSearch => match serde_json::from_str::<Value>(raw.trim()) {
            Ok(body) => (crate::codex_search::parse_response(&body), body),
            Err(_) => (ResponseMeta::default(), Value::Null),
        },
        WireFormat::Images => match serde_json::from_str::<Value>(raw.trim()) {
            Ok(body) => (crate::images::parse_response(&body), body),
            Err(_) => (ResponseMeta::default(), Value::Null),
        },
        WireFormat::SystemOne => match serde_json::from_str::<Value>(raw.trim()) {
            Ok(body) => (systemone::parse_response(&body), body),
            Err(_) => (ResponseMeta::default(), Value::Null),
        },
    }
}

/// Reconstruct a newline-delimited JSON stream (Ollama's native streaming
/// encoding) into (metadata, assembled response body). Formats that don't use
/// NDJSON fall back to their SSE reconstruction, which tolerates arbitrary
/// input and simply yields empty metadata.
pub fn reconstruct_ndjson(format: WireFormat, raw: &str) -> (ResponseMeta, Value) {
    match format {
        WireFormat::Ollama(endpoint) => ollama::reconstruct_ndjson(endpoint, raw),
        other => reconstruct_sse(other, raw),
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
        assert_eq!(detect("/v1/systemone"), Some(WireFormat::SystemOne));
        assert_eq!(detect("/v1/decisions"), Some(WireFormat::Decisions));
        assert_eq!(detect("/api/v1/systemone"), Some(WireFormat::SystemOne));
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
        assert_eq!(provider_for_host("api.typesafe.ai"), "typesafe");
        assert_eq!(provider_for_host("example.com"), "unknown");
    }
}

#[cfg(test)]
mod conversation_identity_tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn explicit_client_sessions_are_linked_without_using_account_identity() {
        assert_eq!(conversation_id(&json!({"metadata":{"user_id":json!({"account_uuid":"account","session_id":"first"}).to_string()}})).as_deref(), Some("first"));
        assert_eq!(
            conversation_id(&json!({"client_metadata":{"session_id":"codex-thread"}})).as_deref(),
            Some("codex-thread")
        );
        assert!(conversation_id(
            &json!({"metadata":{"user_id":json!({"account_uuid":"account"}).to_string()}})
        )
        .is_none());
        assert!(conversation_id(&json!({"metadata":{"user_id":"{invalid"}})).is_none());
    }
}

#[cfg(test)]
mod sniffer_parity_tests {
    use super::*;
    use lr_secret_scanner::request_stream::{Lexer, MetadataSniffer};

    const FORMATS: [WireFormat; 4] = [
        WireFormat::AnthropicMessages,
        WireFormat::OpenAiChat,
        WireFormat::OpenAiResponses,
        WireFormat::Ollama(ollama::OllamaEndpoint::Chat),
    ];

    fn sniff(body: &str, chunk: usize) -> SniffedRequest {
        let mut lexer = Lexer::new();
        let mut sniffer = MetadataSniffer::new();
        for piece in body.as_bytes().chunks(chunk) {
            lexer
                .push(piece, |path, _, event| sniffer.observe(path, &event))
                .unwrap();
        }
        lexer
            .finish(|path, _, event| sniffer.observe(path, &event))
            .unwrap();
        sniffer.into_metadata()
    }

    /// Streaming metadata equals batch metadata for every format and chunking.
    fn assert_parity(body: &str) {
        let parsed: Value = serde_json::from_str(body).unwrap();
        for chunk in [usize::MAX, 1, 3, 17] {
            let sniffed = sniff(body, chunk);
            for format in FORMATS {
                assert_eq!(
                    request_meta_from_sniffed(format, &sniffed),
                    Some(parse_request(format, &parsed)),
                    "{format:?}, chunks of {chunk}: {body}"
                );
            }
            assert_eq!(
                conversation_id_from_sniffed(&sniffed),
                conversation_id(&parsed),
                "chunks of {chunk}: {body}"
            );
        }
    }

    #[test]
    fn sniffed_metadata_matches_parse_request_for_each_wire_shape() {
        for body in [
            // Anthropic Messages: Claude Code's order, then `model` last with
            // `system` and `tools` after `messages` and `role` after `content`.
            r#"{"model":"claude-sonnet-4-5","max_tokens":1024,"messages":[{"role":"user","content":"hi"},{"role":"assistant","content":[{"type":"thinking","thinking":"t","signature":"s"},{"type":"text","text":"a"}]},{"role":"user","content":[{"type":"tool_result","content":[{"type":"text","text":"r"}]}]}],"system":[{"type":"text","text":"s"}],"tools":[{"name":"bash","input_schema":{"type":"object"}}],"metadata":{"user_id":"{\"account_uuid\":\"a\",\"session_id\":\"claude-session\"}"},"stream":true}"#,
            r#"{"messages":[{"content":[{"text":"a","type":"text"}],"role":"user"}],"system":"s","tools":[],"stream":false,"max_tokens":1,"model":"claude-opus-4-5"}"#,
            // Chat Completions.
            r#"{"model":"gpt-4o","messages":[{"role":"system","content":"s"},{"role":"user","content":[{"type":"text","text":"q"},{"type":"image_url","image_url":{"url":"data:image/png;base64,AAAA"}}]}],"tools":[{"type":"function","function":{"name":"f","parameters":{}}},{"type":"function","function":{"name":"g"}}],"stream":true,"stream_options":{"include_usage":true}}"#,
            r#"{"stream":true,"messages":[{"content":"q","role":"user"}],"model":"gpt-4o-mini"}"#,
            r#"{"model":"gpt-4o","messages":[]}"#,
            // Responses API (Codex): array input, string input, key order.
            r#"{"model":"gpt-5-codex","instructions":"i","input":[{"type":"message","role":"user","content":[{"type":"input_text","text":"x"}]},{"type":"reasoning","encrypted_content":"opaque","summary":[]},{"type":"function_call","call_id":"c","name":"shell","arguments":"{\"model\":\"inner\",\"stream\":true}"},{"type":"function_call_output","call_id":"c","output":"ok"}],"tools":[{"type":"function","name":"shell"}],"tool_choice":"auto","stream":true,"previous_response_id":"resp_1","client_metadata":{"session_id":"codex-thread","thread_id":"t"},"store":false}"#,
            r#"{"input":"plain question","stream":false,"model":"gpt-5"}"#,
            r#"{"input":[],"tools":[{"type":"web_search"}],"model":"gpt-5","client_metadata":{"thread_id":"only-thread"}}"#,
            r#"{"type":"response.create","input":[{"type":"message"}],"model":"gpt-5","metadata":{"session_id":"explicit"}}"#,
            // Wrong types, absent fields, non-object roots.
            r#"{"model":null,"stream":"yes","messages":{"0":1},"tools":"t","input":7,"metadata":"m","client_metadata":["session_id"]}"#,
            r#"{"model":7,"stream":1,"messages":"m","tools":{"a":[1]},"input":{"a":1},"metadata":{"user_id":7,"session_id":null}}"#,
            r#"{"model":"","stream":null,"messages":null,"tools":null,"input":null}"#,
            r#"{"model":["m"],"stream":[true],"messages":true,"tools":[null],"input":false}"#,
            "{}",
            "[]",
            r#"[{"model":"m","messages":[1],"stream":true}]"#,
            r#""model""#,
            "12",
            "null",
            // Same names below the top level are not the request's fields.
            r#"{"nested":{"model":"no","stream":true,"messages":[1],"tools":[1],"input":[1],"client_metadata":{"session_id":"no"}},"model":"yes","messages":[{"model":"no","messages":[1,2,3],"tools":[1]}]}"#,
            // Repeated keys: the last one wins.
            r#"{"model":"a","model":"b","messages":[1],"messages":[1,2,3],"tools":[1],"tools":[],"stream":true,"stream":false,"input":"s","input":[1,2],"metadata":{"session_id":"one"},"metadata":{"user_id":"{\"session_id\":\"two\"}"}}"#,
            r#"{"model":"a","model":1,"stream":false,"stream":true,"tools":[],"tools":[1],"input":[1],"input":"s","client_metadata":{"session_id":"one"},"client_metadata":{"session_id":""}}"#,
            // Escaped spellings, whitespace, control characters in ids.
            "{ \"\\u006dodel\" : \"esc\\u0061ped \\ud83d\\udd11 \\\"q\\\"\" ,\n\t\"stream\":true ,\"messages\" : [ ] , \"tools\":[ {} ] }",
            r#"{"client_metadata":{"session_id":"bad\nid","thread_id":"good"},"metadata":{"session_id":"unused"}}"#,
            r#"{"client_metadata":{"session_id":7},"metadata":{"session_id":"","user_id":"{\"session_id\":\"from-user\"}"}}"#,
            r#"{"metadata":{"user_id":"{invalid"}}"#,
            r#"{"metadata":{"user_id":"{\"session_id\":7}"}}"#,
        ] {
            assert_parity(body);
        }
        // A session id at the length limit, and one byte over.
        for len in [256, 257] {
            assert_parity(&format!(
                r#"{{"client_metadata":{{"session_id":"{}"}}}}"#,
                "s".repeat(len)
            ));
        }
    }

    /// Deterministic generator (xorshift64*), reproducible from the seed.
    struct Rng(u64);

    impl Rng {
        fn below(&mut self, bound: usize) -> usize {
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            (self.0.wrapping_mul(0x2545_F491_4F6C_DD1D) % bound as u64) as usize
        }
    }

    fn random_value(rng: &mut Rng, depth: usize) -> String {
        match rng.below(if depth > 2 { 6 } else { 9 }) {
            0 => "true".into(),
            1 => "false".into(),
            2 => "null".into(),
            3 => rng.below(1000).to_string(),
            4 => format!("\"s{}\"", rng.below(100)),
            5 => "\"{\\\"session_id\\\":\\\"nested-id\\\"}\"".into(),
            6 | 7 => {
                let items: Vec<String> = (0..rng.below(5))
                    .map(|_| random_value(rng, depth + 1))
                    .collect();
                format!("[{}]", items.join(","))
            }
            _ => {
                let members: Vec<String> = (0..rng.below(4))
                    .map(|index| {
                        let key =
                            ["session_id", "thread_id", "user_id", "model", "k"][rng.below(5)];
                        // The index keeps most keys distinct; repeats of the
                        // bare names are intended.
                        let key = if rng.below(3) == 0 {
                            key.to_string()
                        } else {
                            format!("{key}{index}")
                        };
                        format!("\"{key}\":{}", random_value(rng, depth + 1))
                    })
                    .collect();
                format!("{{{}}}", members.join(","))
            }
        }
    }

    #[test]
    fn sniffed_metadata_matches_parse_request_for_random_bodies() {
        const KEYS: [&str; 10] = [
            "model",
            "stream",
            "messages",
            "input",
            "tools",
            "metadata",
            "client_metadata",
            "type",
            "previous_response_id",
            "other",
        ];
        for seed in 1..=1500u64 {
            let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
            // Any subset, in any order, with occasional repeats.
            let members: Vec<String> = (0..rng.below(12))
                .map(|_| {
                    let key = KEYS[rng.below(KEYS.len())];
                    let value = match (key, rng.below(3)) {
                        ("model", 0) => "\"gpt-5\"".into(),
                        ("stream", 0) => "true".into(),
                        ("metadata" | "client_metadata", 0) => {
                            format!("{{\"session_id\":\"id{}\"}}", rng.below(9))
                        }
                        _ => random_value(&mut rng, 0),
                    };
                    format!("\"{key}\":{value}")
                })
                .collect();
            assert_parity(&format!("{{{}}}", members.join(",")));
        }
    }

    #[test]
    fn other_wire_formats_are_not_derived_from_the_sniffer() {
        let sniffed = sniff(r#"{"model":"m","messages":[1]}"#, usize::MAX);
        for format in [
            WireFormat::CodexSearch,
            WireFormat::Images,
            WireFormat::SystemOne,
            WireFormat::Decisions,
        ] {
            assert_eq!(request_meta_from_sniffed(format, &sniffed), None);
        }
    }

    /// Ollama streams unless told otherwise; a streamed generate request
    /// has its prompt, one message.
    #[test]
    fn ollama_metadata_is_derived_from_the_sniffer() {
        let generate = WireFormat::Ollama(ollama::OllamaEndpoint::Generate);
        let body = r#"{"model":"m","prompt":"p"}"#;
        let parsed: Value = serde_json::from_str(body).unwrap();
        assert_eq!(
            request_meta_from_sniffed(generate, &sniff(body, 3)),
            Some(parse_request(generate, &parsed))
        );
        let body = r#"{"model":"m","prompt":"p","stream":false,"tools":[{}]}"#;
        let parsed: Value = serde_json::from_str(body).unwrap();
        assert_eq!(
            request_meta_from_sniffed(generate, &sniff(body, 1)),
            Some(parse_request(generate, &parsed))
        );
    }

    #[test]
    fn a_model_beyond_the_capture_bound_is_reported_as_unset() {
        let body = format!(r#"{{"model":"{}","messages":[1,2]}}"#, "m".repeat(70_000));
        let meta = request_meta_from_sniffed(WireFormat::OpenAiChat, &sniff(&body, 4096)).unwrap();
        assert_eq!(meta.model, None);
        assert_eq!(meta.message_count, 2);
    }
}
