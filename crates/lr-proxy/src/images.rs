//! Image generation and edits (`POST .../images/generations`,
//! `POST .../images/edits`), as sent by Codex's image tool.
//!
//! The request is JSON: a text `prompt`, the model and options, and for edits
//! an `images` list of data URLs or file ids. The prompt is the only text to
//! scan. OpenAI's public edits endpoint also accepts multipart form bodies,
//! which are not JSON; [`is_json_request`] lets the caller tell them apart.
use serde_json::Value;

use crate::wire::{RequestMeta, ResponseMeta};

pub fn is_images_path(path: &str) -> bool {
    let path = path
        .split(['?', '#'])
        .next()
        .unwrap_or(path)
        .trim_end_matches('/');
    path.ends_with("/images/generations") || path.ends_with("/images/edits")
}

/// Whether the body is the JSON request shape this module understands.
pub fn is_json_request(body: &Value) -> bool {
    body.get("prompt").is_some_and(Value::is_string)
}

pub fn parse_request(body: &Value) -> RequestMeta {
    RequestMeta {
        model: body.get("model").and_then(Value::as_str).map(str::to_owned),
        stream: body.get("stream").and_then(Value::as_bool).unwrap_or(false),
        message_count: usize::from(is_json_request(body)),
        has_tools: false,
    }
}

pub fn parse_response(body: &Value) -> ResponseMeta {
    let usage = body.get("usage");
    let tokens = |key: &str| usage.and_then(|u| u.get(key)).and_then(Value::as_u64);
    ResponseMeta {
        model: body.get("model").and_then(Value::as_str).map(str::to_owned),
        input_tokens: tokens("input_tokens"),
        output_tokens: tokens("output_tokens"),
        content_preview: body
            .get("data")
            .and_then(Value::as_array)
            .map(|images| format!("{} image(s)", images.len())),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn matches_generation_and_edit_routes_only() {
        for path in [
            "/backend-api/codex/images/edits",
            "/backend-api/codex/images/generations",
            "/v1/images/generations?x=1",
            "/v1/images/edits/",
        ] {
            assert!(is_images_path(path), "{path}");
        }
        for path in [
            "/v1/images/variations",
            "/v1/images",
            "/images/edits/other",
            "/oauth?next=/images/edits",
        ] {
            assert!(!is_images_path(path), "{path}");
        }
    }

    #[test]
    fn parses_codex_edit_requests_and_responses() {
        let request = json!({"images":[{"image_url":"data:image/png;base64,Zm9v"},{"file_id":"file-image"}],"prompt":"add a red hat","model":"gpt-image-1.5"});
        assert!(is_json_request(&request));
        assert_eq!(
            parse_request(&request),
            RequestMeta {
                model: Some("gpt-image-1.5".into()),
                stream: false,
                message_count: 1,
                has_tools: false
            }
        );
        let response = json!({"created":1,"data":[{"b64_json":"Zm9v"},{"b64_json":"YmFy"}],"usage":{"input_tokens":10,"output_tokens":20}});
        let meta = parse_response(&response);
        assert_eq!(meta.content_preview.as_deref(), Some("2 image(s)"));
        assert_eq!(meta.input_tokens, Some(10));
        assert_eq!(meta.output_tokens, Some(20));
    }

    #[test]
    fn multipart_bodies_are_not_json_requests() {
        assert!(!is_json_request(&Value::Null));
        assert!(!is_json_request(&json!({"model":"gpt-image-1"})));
    }
}
