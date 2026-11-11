//! Codex standalone web search (`POST .../alpha/search`).
//! The request has optional Responses-style input plus search commands. Its
//! JSON response contains plaintext output and provider-owned encrypted state.
use serde_json::Value;

use crate::wire::{RequestMeta, ResponseMeta};

pub fn is_search_path(path: &str) -> bool {
    path.split(['?', '#'])
        .next()
        .unwrap_or(path)
        .trim_end_matches('/')
        .ends_with("/alpha/search")
}

pub fn parse_request(body: &Value) -> RequestMeta {
    let mut meta = crate::openai::parse_responses_request(body);
    meta.stream = false;
    meta.has_tools = false;
    if let Some(commands) = body.get("commands").and_then(Value::as_object) {
        meta.message_count = commands
            .values()
            .filter_map(Value::as_array)
            .map(Vec::len)
            .sum();
    }
    meta
}

pub fn parse_response(body: &Value) -> ResponseMeta {
    ResponseMeta {
        model: body.get("model").and_then(Value::as_str).map(str::to_owned),
        content_preview: body.get("output").and_then(Value::as_str).map(|s| {
            let mut preview: String = s.chars().take(4000).collect();
            if s.chars().nth(4000).is_some() {
                preview.push('…');
            }
            preview
        }),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn matches_only_standalone_search_routes() {
        for path in [
            "/backend-api/codex/alpha/search",
            "/v1/alpha/search?x=1",
            "/alpha/search/",
        ] {
            assert!(is_search_path(path), "{path}");
        }
        for path in [
            "/v1/search",
            "/alpha/searching",
            "/alpha/search/other",
            "/oauth?next=/alpha/search",
        ] {
            assert!(!is_search_path(path), "{path}");
        }
    }

    #[test]
    fn parses_search_without_treating_encrypted_state_as_an_answer_or_response_id() {
        let request = json!({"id":"search-session","model":"test","commands":{"search_query":[{"q":"hello"}],"open":[{"ref_id":"turn0search0"}]}});
        assert_eq!(
            parse_request(&request),
            RequestMeta {
                model: Some("test".into()),
                stream: false,
                message_count: 2,
                has_tools: false
            }
        );
        let response = json!({"encrypted_output":"opaque-provider-state","output":"Search answer","results":[{"type":"text_result","ref_id":"turn0search0"}]});
        let meta = parse_response(&response);
        assert_eq!(meta.content_preview.as_deref(), Some("Search answer"));
        assert!(meta.message_id.is_none());
        assert!(parse_response(&json!({"encrypted_output":"opaque"}))
            .content_preview
            .is_none());
        assert!(parse_response(&json!({"output":"🔎".repeat(4001)}))
            .content_preview
            .unwrap()
            .ends_with('…'));
    }
}
