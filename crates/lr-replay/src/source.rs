//! Validate selected input without recovering missing/redacted content or executing tools.
use crate::types::*;
use serde_json::{json, Value};
use uuid::Uuid;
pub const MAX_CASES: usize = 64;
pub const MAX_TARGETS: usize = 16;
pub const MAX_BODY_BYTES: usize = 1024 * 1024;
pub fn validate_input(input: &ReplayInput) -> Result<(), String> {
    if serde_json::to_vec(&input.body)
        .map_err(|e| e.to_string())?
        .len()
        > MAX_BODY_BYTES
    {
        return Err("Replay input exceeds 1 MiB".into());
    }
    let object = input
        .body
        .as_object()
        .ok_or("Replay input must be a JSON object")?;
    fn incomplete(value: &Value, depth: usize) -> bool {
        if depth > 64 {
            return true;
        }
        match value {
            Value::Object(fields) => {
                fields.get("_truncated") == Some(&Value::Bool(true))
                    || fields.get("_redacted") == Some(&Value::Bool(true))
                    || fields.get("type").and_then(Value::as_str) == Some("redacted_thinking")
                    || fields.iter().any(|(key, v)| {
                        (matches!(
                            key.to_ascii_lowercase().as_str(),
                            "encrypted_content"
                                | "encryptedcontent"
                                | "thought_signature"
                                | "thoughtsignature"
                        ) || matches!(
                            fields.get("type").and_then(Value::as_str),
                            Some("thinking" | "redacted_thinking" | "reasoning")
                        ) && matches!(key.as_str(), "signature"))
                            && v.as_str().is_some_and(|s| !s.is_empty())
                            || incomplete(v, depth + 1)
                    })
            }
            Value::Array(items) => items.iter().any(|v| incomplete(v, depth + 1)),
            _ => false,
        }
    }
    if incomplete(&input.body, 0) {
        return Err("Captured input is truncated, redacted or contains provider-bound state. Import or edit a complete standalone request".into());
    }
    for key in ["previous_response_id", "conversation", "conversation_id"] {
        if object.get(key).is_some_and(|v| !v.is_null()) {
            return Err("Continuation handles cannot be replayed as a standalone request. Supply the full context explicitly".into());
        }
    }
    let required = match input.endpoint {
        ReplayEndpoint::ChatCompletions => "messages",
        ReplayEndpoint::Completions => "prompt",
        ReplayEndpoint::Responses => "input",
        ReplayEndpoint::Decisions => "input",
        ReplayEndpoint::SystemOne => "state",
    };
    if object.get(required).is_none_or(Value::is_null) {
        return Err(format!("Replay input needs a complete '{required}' field"));
    }
    let mut typed = input.body.clone();
    typed["model"] = json!("localrouter/auto");
    typed["stream"] = Value::Bool(false);
    let extensions = typed
        .as_object_mut()
        .unwrap()
        .entry("extensions")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or("extensions must be an object")?;
    extensions.insert(
        lr_providers::replay::EXTENSION.into(),
        json!({"tools":"disabled"}),
    );
    lr_providers::replay::normalize_ingress(&mut typed).map_err(|e| e.to_string())?;
    match input.endpoint {
        ReplayEndpoint::ChatCompletions => {
            serde_json::from_value::<lr_server::types::ChatCompletionRequest>(typed)
                .map_err(|e| e.to_string())?;
        }
        ReplayEndpoint::Completions => {
            serde_json::from_value::<lr_server::types::CompletionRequest>(typed)
                .map_err(|e| e.to_string())?;
        }
        ReplayEndpoint::Responses => {
            serde_json::from_value::<lr_server::routes::responses::CreateResponseRequest>(typed)
                .map_err(|e| e.to_string())?;
        }
        ReplayEndpoint::Decisions => {
            serde_json::from_value::<lr_providers::systemone::decisions::DecisionRequest>(typed)
                .map_err(|e| e.to_string())?
                .validate()
                .map_err(|e| e.to_string())?;
        }
        ReplayEndpoint::SystemOne => {
            serde_json::from_value::<lr_providers::SystemOneRequest>(typed)
                .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}
pub fn validate_cases(cases: &[ReplayCase]) -> Result<(), String> {
    if serde_json::to_vec(cases).map_err(|e| e.to_string())?.len() > 8 * 1024 * 1024 {
        return Err("Selected cases and checks exceed 8 MiB".into());
    }

    if cases.is_empty() || cases.len() > MAX_CASES {
        return Err(format!("Choose 1–{MAX_CASES} cases"));
    }
    let mut ids = std::collections::HashSet::new();
    for case in cases {
        if case.id.is_empty() || case.id.len() > 128 || !ids.insert(&case.id) {
            return Err("Case ids must be nonempty, unique and at most 128 bytes".into());
        }
        if case.name.trim().is_empty() || case.name.len() > 256 {
            return Err("Case names must contain 1–256 bytes".into());
        }
        validate_input(&case.input)?;
        if case.checks.len() > 16 {
            return Err("A case can have at most 16 checks".into());
        }
        for check in &case.checks {
            match check {
                TaskCheck::Contains { text, .. } if text.is_empty() || text.len() > 4096 => {
                    return Err("Contains checks need 1–4096 bytes".into())
                }
                TaskCheck::Regex { pattern } => {
                    if pattern.len() > 4096 {
                        return Err("Regex checks are limited to 4096 bytes".into());
                    }
                    regex::Regex::new(pattern).map_err(|e| e.to_string())?;
                }
                TaskCheck::JsonPointer { pointer, .. }
                    if pointer.len() > 1024
                        || (!pointer.is_empty() && !pointer.starts_with('/')) =>
                {
                    return Err("Invalid JSON pointer".into())
                }
                TaskCheck::ExpectedRoute { route } if route.is_empty() || route.len() > 64 => {
                    return Err("Expected routes need 1–64 bytes".into())
                }
                _ => {}
            }
        }
    }
    Ok(())
}
pub fn validate_run(request: &ReplayRunRequest) -> Result<(), String> {
    if serde_json::to_vec(&request.targets)
        .map_err(|e| e.to_string())?
        .len()
        > 2 * 1024 * 1024
    {
        return Err("Target policy snapshots exceed 2 MiB".into());
    }

    validate_cases(&request.cases)?;
    if request.targets.is_empty() || request.targets.len() > MAX_TARGETS {
        return Err(format!("Choose 1–{MAX_TARGETS} targets"));
    }
    if !(1..=32768).contains(&request.max_output_tokens) {
        return Err("Output token limit must be between 1 and 32768".into());
    }
    let mut labels = std::collections::HashSet::new();
    for target in &request.targets {
        let label = target.label();
        if label.trim().is_empty() || label.len() > 512 || !labels.insert(label) {
            return Err("Targets need unique, nonempty labels of at most 512 bytes".into());
        }
        if let ReplayTarget::Policy { policy, .. } = target {
            policy.validate()?;
        }
    }
    Ok(())
}
/// Remove transport identity and per-turn state before retaining a selected case.
pub fn sanitize_input(input: &ReplayInput) -> Result<ReplayInput, String> {
    validate_input(input)?;
    if let Some(options) = input.body.pointer("/extensions/localrouter.gateway") {
        let options: lr_providers::gateway::GatewayPreferences =
            serde_json::from_value(options.clone()).map_err(|e| e.to_string())?;
        options.validate().map_err(|e| e.to_string())?;
    }
    let mut input = input.clone();
    let object = input.body.as_object_mut().unwrap();
    object.retain(|key, _| {
        !matches!(
            key.to_ascii_lowercase().as_str(),
            "api_key"
                | "apikey"
                | "authorization"
                | "headers"
                | "base_url"
                | "baseurl"
                | "prompt_cache_key"
                | "promptcachekey"
                | "user"
                | "access_token"
                | "client_secret"
        )
    });
    if let Some(metadata) = object.get_mut("metadata").and_then(Value::as_object_mut) {
        metadata.retain(|key, _| key == "localrouter.mode");
    }
    if let Some(extensions) = object.get_mut("extensions").and_then(Value::as_object_mut) {
        extensions.retain(|key, _| {
            (!key.starts_with("localrouter.") || key == lr_providers::gateway::EXTENSION)
                && !matches!(
                    key.to_ascii_lowercase().as_str(),
                    "api_key"
                        | "apikey"
                        | "authorization"
                        | "headers"
                        | "access_token"
                        | "client_secret"
                )
        });
    }
    Ok(input)
}
pub fn prepare_body(
    input: &ReplayInput,
    model: &str,
    run_id: &str,
    max_output_tokens: u32,
) -> Result<Value, String> {
    let input = sanitize_input(input)?;
    let mut body = input.body.clone();
    let object = body.as_object_mut().unwrap();
    for key in [
        "api_key",
        "apiKey",
        "authorization",
        "Authorization",
        "headers",
        "base_url",
        "baseURL",
    ] {
        object.remove(key);
    }
    object.insert("model".into(), Value::String(model.into()));
    if matches!(
        input.endpoint,
        ReplayEndpoint::Decisions | ReplayEndpoint::SystemOne
    ) {
        object.remove("stream");
    } else {
        object.insert("stream".into(), Value::Bool(false));
    }
    object.remove("stream_options");
    object.remove("prompt_cache_key");
    object.remove("store");
    if matches!(
        input.endpoint,
        ReplayEndpoint::ChatCompletions | ReplayEndpoint::Completions
    ) {
        object.insert("max_tokens".into(), json!(max_output_tokens));
        object.remove("max_completion_tokens");
    }
    if matches!(
        input.endpoint,
        ReplayEndpoint::Responses | ReplayEndpoint::ChatCompletions
    ) {
        object.insert("store".into(), Value::Bool(false));
    }
    if input.endpoint == ReplayEndpoint::Responses {
        object.insert("max_output_tokens".into(), json!(max_output_tokens));
    }
    let extensions = object
        .entry("extensions")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or("extensions must be an object")?;
    extensions.retain(|key, _| {
        !key.starts_with("localrouter.") || key == lr_providers::gateway::EXTENSION
    });
    extensions.insert(
        lr_providers::replay::EXTENSION.into(),
        json!({"tools":"disabled"}),
    );
    extensions.insert(
        lr_providers::session::EXTENSION.into(),
        json!({"id":format!("replay-{run_id}-{}",Uuid::new_v4()),"reclassify":true}),
    );
    lr_providers::replay::normalize_ingress(&mut body).map_err(|e| e.to_string())?;
    Ok(body)
}
pub fn output_text(body: &Value) -> String {
    if let Some(text) = body
        .pointer("/choices/0/message/content")
        .and_then(Value::as_str)
        .or_else(|| body.pointer("/choices/0/text").and_then(Value::as_str))
        .or_else(|| body.get("output_text").and_then(Value::as_str))
    {
        return text.into();
    }
    if let Some(output) = body.get("output").and_then(Value::as_array) {
        return output
            .iter()
            .filter_map(|item| item.get("content").and_then(Value::as_array))
            .flatten()
            .filter_map(|part| part.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n");
    }
    if let Some(answers) = body.get("answers") {
        return serde_json::to_string(answers).unwrap_or_default();
    }
    String::new()
}
pub fn evaluate_checks(checks: &[TaskCheck], body: &Value) -> Vec<CheckResult> {
    let text = output_text(body);
    let parsed = serde_json::from_str::<Value>(&text).ok();
    checks
        .iter()
        .map(|check| {
            let (kind, passed) = match check {
                TaskCheck::Contains {
                    text: expected,
                    case_sensitive,
                } => (
                    "contains",
                    if *case_sensitive {
                        text.contains(expected)
                    } else {
                        text.to_lowercase().contains(&expected.to_lowercase())
                    },
                ),
                TaskCheck::Regex { pattern } => (
                    "regex",
                    regex::Regex::new(pattern).is_ok_and(|re| re.is_match(&text)),
                ),
                TaskCheck::JsonValid => ("json_valid", parsed.is_some()),
                TaskCheck::JsonPointer { pointer, expected } => (
                    "json_pointer",
                    parsed.as_ref().unwrap_or(body).pointer(pointer) == Some(expected),
                ),
                TaskCheck::ExpectedRoute { route } => (
                    "expected_route",
                    body.pointer("/decision/route")
                        .or_else(|| {
                            body.pointer("/extensions/localrouter.routing/decision_routing/route")
                        })
                        .and_then(Value::as_str)
                        == Some(route.as_str()),
                ),
            };
            CheckResult {
                kind: kind.into(),
                passed,
                detail: None,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn input() -> ReplayInput {
        ReplayInput {
            endpoint: ReplayEndpoint::ChatCompletions,
            body: json!({"messages":[{"role":"user","content":"Hello"}]}),
            source_event_id: None,
        }
    }
    #[test]
    fn rejects_incomplete_and_bound_state_without_reconstructing_it() {
        for extra in [
            json!({"_truncated":true}),
            json!({"_redacted":true}),
            json!({"type":"reasoning","encrypted_content":"opaque"}),
        ] {
            let mut v = input();
            v.body["messages"][0]["content"] = json!([extra]);
            assert!(validate_input(&v).is_err());
        }
        let mut v = input();
        v.body["previous_response_id"] = json!("resp_live");
        assert!(validate_input(&v).is_err());
        v.body = json!({"_preview":"hello","_truncated":true});
        assert!(sanitize_input(&v).is_err());
    }
    #[test]
    fn fresh_session_no_credentials_or_callable_tools_and_preserved_policy() {
        let mut v = input();
        v.body["api_key"] = json!("secret");
        v.body["metadata"] = json!({"session-id":"live","localrouter.mode":"deep"});
        v.body["extensions"] = json!({"localrouter.session":{"id":"live"},"localrouter.session_control_token":"private","localrouter.gateway":{"zdr":true}});
        v.body["tools"] = json!([{"type":"function"}]);
        v.body["tool_choice"] = json!("required");
        let a = prepare_body(&v, "p/m", "test", 12).unwrap();
        let b = prepare_body(&v, "p/m", "test", 12).unwrap();
        assert!(a.get("api_key").is_none());
        assert!(a.get("tools").is_none());
        assert!(a.get("tool_choice").is_none());
        assert!(a["extensions"]
            .get("localrouter.session_control_token")
            .is_none());
        assert!(a["metadata"].get("session-id").is_none());
        assert_eq!(a["metadata"]["localrouter.mode"], "deep");
        assert_eq!(a["extensions"]["localrouter.gateway"]["zdr"], true);
        assert_ne!(
            a["extensions"]["localrouter.session"]["id"],
            b["extensions"]["localrouter.session"]["id"]
        );
        assert_eq!(a["max_tokens"], 12);
        assert_eq!(a["stream"], false);
    }
    #[test]
    fn checks_are_deterministic_and_do_not_execute_inference() {
        let checks = vec![
            TaskCheck::JsonValid,
            TaskCheck::JsonPointer {
                pointer: "/ok".into(),
                expected: json!(true),
            },
            TaskCheck::Contains {
                text: "TRUE".into(),
                case_sensitive: false,
            },
            TaskCheck::Regex {
                pattern: "ok".into(),
            },
        ];
        assert!(evaluate_checks(
            &checks,
            &json!({"choices":[{"message":{"content":"{\"ok\":true}"}}]})
        )
        .iter()
        .all(|r| r.passed));
        assert!(
            evaluate_checks(
                &[TaskCheck::ExpectedRoute {
                    route: "fast".into()
                }],
                &json!({"decision":{"route":"fast"}})
            )[0]
            .passed
        );
    }
    #[test]
    fn validates_endpoint_shape_and_unique_cases() {
        let case = ReplayCase {
            id: "a".into(),
            name: "A".into(),
            input: input(),
            checks: vec![],
        };
        assert!(validate_cases(&[case.clone(), case]).is_err());
        let mut v = input();
        v.body["messages"] = json!("not messages");
        assert!(validate_input(&v).is_err());
    }
}

#[cfg(test)]
mod decision_replay_tests {
    use super::*;
    #[test]
    fn native_decisions_have_no_chat_only_stream_parameter() {
        let input = ReplayInput {
            endpoint: ReplayEndpoint::SystemOne,
            body: json!({"state":"hello","questions":{"route":{"type":"choice","instructions":"Choose a route","criteria":{"a":"A","b":"B"}}}}),
            source_event_id: None,
        };
        let body = prepare_body(&input, "native/model", "run", 32).unwrap();
        assert!(body.get("stream").is_none());
        assert!(body.get("max_tokens").is_none());
        assert_eq!(body["state"], "hello");
    }
    #[test]
    fn strips_transport_identity_case_insensitively_and_rejects_gateway_secrets() {
        let mut input = ReplayInput {
            endpoint: ReplayEndpoint::ChatCompletions,
            body: json!({"messages":[{"role":"user","content":"literal user input"}],"AUTHORIZATION":"secret","APIKEY":"secret","access_token":"secret"}),
            source_event_id: None,
        };
        let clean = sanitize_input(&input).unwrap();
        assert!(!serde_json::to_string(&clean).unwrap().contains("secret"));
        input.body["extensions"] = json!({"localrouter.gateway":{"api_key":"secret"}});
        assert!(sanitize_input(&input).is_err());
    }
}

#[cfg(test)]
mod opaque_state_tests {
    use super::*;
    #[test]
    fn arbitrary_decision_state_cannot_smuggle_provider_bound_opaque_blocks() {
        for state in [
            json!({"type":"redacted_thinking","data":"opaque"}),
            json!({"parts":[{"thoughtSignature":"bound-to-provider"}]}),
            json!({"encryptedContent":"opaque"}),
        ] {
            let input = ReplayInput {
                endpoint: ReplayEndpoint::SystemOne,
                body: json!({"state":state,"questions":{"route":{"type":"choice","instructions":"Choose a route","criteria":{"a":"A","b":"B"}}}}),
                source_event_id: None,
            };
            assert!(validate_input(&input)
                .unwrap_err()
                .contains("provider-bound"));
        }
    }
}
