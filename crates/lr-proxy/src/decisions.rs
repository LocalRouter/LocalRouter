//! Passive metadata extraction for the non-streaming Decisions API.
use crate::wire::{RequestMeta, ResponseMeta};
use serde_json::Value;

pub fn parse_request(body: &Value) -> RequestMeta {
    if body.get("questions").is_some_and(Value::is_object) {
        return crate::systemone::parse_request(body);
    }
    RequestMeta {
        model: body
            .get("model")
            .and_then(Value::as_str)
            .map(str::to_string),
        stream: false,
        message_count: body
            .get("questions")
            .and_then(Value::as_array)
            .map_or(0, Vec::len),
        has_tools: false,
    }
}
pub fn parse_response(body: &Value) -> ResponseMeta {
    let mut response = crate::systemone::parse_response(body);
    if body.get("answers").is_some_and(Value::is_object) {
        return response;
    }
    response.content_preview = body
        .get("answers")
        .and_then(Value::as_array)
        .map(|answers| {
            answers
                .iter()
                .enumerate()
                .map(|(i, a)| {
                    let name = a
                        .get("name")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                        .unwrap_or_else(|| format!("Question {}", i + 1));
                    let value = match a.get("type").and_then(Value::as_str) {
                        Some("predicate") => a.get("probability").map(Value::to_string),
                        Some("choice") => a.get("choice").map(Value::to_string),
                        Some("score") => a.get("score").map(Value::to_string),
                        Some("refusal") => Some("refused".into()),
                        _ => None,
                    }
                    .unwrap_or_else(|| "?".into());
                    format!("{name}={value}")
                })
                .collect::<Vec<_>>()
                .join(", ")
        })
        .filter(|s| !s.is_empty());
    response.stop_reason = response.content_preview.as_ref().map(|_| "stop".into());
    response
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn preserves_typed_values_and_refusals() {
        assert_eq!(
            parse_request(&json!({"questions":[{},{}]})).message_count,
            2
        );
        let response = parse_response(
            &json!({"model":"m","answers":[{"type":"choice","name":"a","choice":true},{"type":"refusal","name":null}],"usage":{"input_tokens":10,"output_tokens":0}}),
        );
        assert_eq!(
            response.content_preview.as_deref(),
            Some("a=true, Question 2=refused")
        );
        assert_eq!(response.input_tokens, Some(10));
        assert!(parse_response(&json!({"error":{}}))
            .content_preview
            .is_none());
    }
}
