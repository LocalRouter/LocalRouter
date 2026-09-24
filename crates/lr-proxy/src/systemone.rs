//! System One decision protocol (`POST .../v1/systemone`, TypeSafe's Jev API
//! and compatible servers such as Laya and Kev).
//!
//! Plain JSON request/response pairs: no streaming, no messages. The monitor
//! records the number of questions and a one-line summary of the answers.

use serde_json::Value;

use crate::wire::{RequestMeta, ResponseMeta};

/// Whether a request path is a System One decision call.
pub fn is_systemone_path(path: &str) -> bool {
    path.split('?')
        .next()
        .unwrap_or(path)
        .trim_end_matches('/')
        .ends_with("/systemone")
}

/// Request metadata: model and number of questions.
pub fn parse_request(body: &Value) -> RequestMeta {
    RequestMeta {
        model: body
            .get("model")
            .and_then(Value::as_str)
            .map(str::to_string),
        stream: false,
        message_count: body
            .get("questions")
            .and_then(Value::as_object)
            .map(|q| q.len())
            .unwrap_or(0),
        has_tools: false,
    }
}

/// Response metadata: model, token usage, and an answers summary such as
/// `dept=billing (0.90), urgent=0.20`.
pub fn parse_response(body: &Value) -> ResponseMeta {
    let usage = body.get("usage");
    let token = |k: &str| usage.and_then(|u| u.get(k)).and_then(Value::as_u64);
    let preview = body
        .get("answers")
        .and_then(Value::as_object)
        .map(|answers| {
            answers
                .iter()
                .map(
                    |(id, a)| match a.get("type").and_then(Value::as_str).unwrap_or_default() {
                        "choice" => format!(
                            "{id}={} ({:.2})",
                            a.get("choice").and_then(Value::as_str).unwrap_or("?"),
                            a.get("confidence").and_then(Value::as_f64).unwrap_or(0.0)
                        ),
                        "score" => format!(
                            "{id}={:.2}",
                            a.get("score").and_then(Value::as_f64).unwrap_or(0.0)
                        ),
                        "noul" => format!(
                            "{id}={:.2}",
                            a.get("noul").and_then(Value::as_f64).unwrap_or(0.0)
                        ),
                        other => format!("{id}={other}"),
                    },
                )
                .collect::<Vec<_>>()
                .join(", ")
        });
    ResponseMeta {
        model: body
            .get("model")
            .and_then(Value::as_str)
            .map(str::to_string),
        input_tokens: token("input_tokens"),
        output_tokens: token("output_tokens"),
        stop_reason: preview.as_ref().map(|_| "stop".to_string()),
        content_preview: preview.filter(|p| !p.is_empty()),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn detects_systemone_paths() {
        assert!(is_systemone_path("/v1/systemone"));
        assert!(is_systemone_path("/v1/systemone/"));
        assert!(is_systemone_path("/api/v1/systemone?x=1"));
        assert!(is_systemone_path("/typesafe/v1/systemone"));
        assert!(!is_systemone_path("/v1/systemone/permute"));
        assert!(!is_systemone_path("/v1/models"));
    }

    #[test]
    fn parses_request_and_response() {
        let req = parse_request(&json!({
            "model": "jev-latest", "state": "s",
            "questions": {"a": {"type": "noul"}, "b": {"type": "choice"}}
        }));
        assert_eq!(req.model.as_deref(), Some("jev-latest"));
        assert_eq!(req.message_count, 2);
        assert!(!req.stream);

        let resp = parse_response(&json!({
            "model": "jev-1.13.0",
            "answers": {
                "dept": {"type": "choice", "choice": "billing", "confidence": 0.9, "probabilities": {}},
                "lvl": {"type": "score", "score": 1.5, "confidence": 0.2},
                "yes": {"type": "noul", "noul": 0.25}
            },
            "usage": {"input_tokens": 312, "output_tokens": 0}
        }));
        assert_eq!(resp.model.as_deref(), Some("jev-1.13.0"));
        assert_eq!(resp.input_tokens, Some(312));
        assert_eq!(resp.output_tokens, Some(0));
        assert_eq!(
            resp.content_preview.as_deref(),
            Some("dept=billing (0.90), lvl=1.50, yes=0.25")
        );
    }

    #[test]
    fn error_bodies_yield_empty_meta() {
        let resp = parse_response(&json!({"detail": [{"msg": "bad"}]}));
        assert!(resp.content_preview.is_none());
        assert!(resp.input_tokens.is_none());
    }
}
