//! Sequential shared policy for one native row. Only explicitly supported
//! edits are applied to the exact raw wire body; nothing is silently dropped.
use super::rows;
use crate::{
    middleware::{
        client_auth::ClientAuthContext,
        error::{ApiErrorResponse, ApiResult},
    },
    routes::{
        monitor_helpers::LlmCallGuard,
        pipeline::{run_turn_pipeline, PipelineCaps},
    },
    state::{AppState, AuthContext},
    types::ChatCompletionRequest,
};
use axum::Extension;
use lr_batches::BatchJob;
use serde_json::{json, Value};

pub(crate) async fn validate_row(
    state: &AppState,
    auth: &AuthContext,
    job: &BatchJob,
    body: &Value,
    row_id: &str,
) -> ApiResult<Value> {
    if lr_types::is_duplicate_hop() || auth.credential_scope.is_none() {
        return Err(ApiErrorResponse::forbidden(
            "Native batch policy does not allow internal or duplicate-hop bypass",
        ));
    }
    rows::validate_body(&job.endpoint, body)?;
    let qualified = format!(
        "{}/{}",
        job.provider_instance,
        job.options
            .permission_model
            .as_deref()
            .unwrap_or(&job.model)
    );
    let mut projection = match job.endpoint.as_str() {
        "/v1/chat/completions" => body.clone(),
        "/v1/responses" => {
            let mut value = json!({"messages":rows::response_messages(body)?,"max_tokens":body["max_output_tokens"]});
            for key in ["temperature", "top_p"] {
                if let Some(v) = body.get(key) {
                    value[key] = v.clone();
                }
            }
            value
        }
        "/v1/embeddings" => {
            let inputs = match &body["input"] {
                Value::String(_) => vec![body["input"].clone()],
                Value::Array(items) => items.clone(),
                _ => return Err(ApiErrorResponse::bad_request("Unsupported embedding input")),
            };
            json!({"messages":inputs.into_iter().map(|v|json!({"role":"user","content":v})).collect::<Vec<_>>()})
        }
        _ => {
            return Err(ApiErrorResponse::bad_request(
                "Unsupported batch policy endpoint",
            ))
        }
    };
    projection["model"] = json!(qualified);
    projection["stream"] = json!(false);
    // The ordinary text is already in messages. Scan every other outbound
    // string too, including schemas and batch-level metadata. Excluding the
    // messages here lets approved text redactions work without stale copies.
    let mut options = body.clone();
    let map = options
        .as_object_mut()
        .ok_or_else(|| ApiErrorResponse::bad_request("Invalid batch body"))?;
    for key in ["input", "instructions"] {
        map.remove(key);
    }
    if let Some(messages) = map.get_mut("messages").and_then(Value::as_array_mut) {
        for message in messages {
            if let Some(message) = message.as_object_mut() {
                message.remove("content");
            }
        }
    }
    let scan_marker = json!({"role":"user","content":serde_json::to_string(&json!({"native_options":options,"batch_metadata":job.options.metadata,"provider_only":job.options.provider_only,"custom_id":row_id})).map_err(|_|ApiErrorResponse::bad_request("Invalid native scan projection"))?});
    projection["messages"]
        .as_array_mut()
        .ok_or_else(|| ApiErrorResponse::bad_request("Invalid batch scan projection"))?
        .push(scan_marker.clone());
    let projection: ChatCompletionRequest = serde_json::from_value(projection)
        .map_err(|_| ApiErrorResponse::bad_request("Invalid batch policy projection"))?;
    let mut before = serde_json::to_value(&projection)
        .map_err(|_| ApiErrorResponse::bad_request("Invalid batch projection"))?;
    let mut guard = LlmCallGuard::validation_only(state);
    let client = Extension(ClientAuthContext {
        client_id: auth.api_key_id.clone(),
    });
    let turn = run_turn_pipeline(
        state,
        auth,
        Some(&client),
        projection,
        &mut guard,
        "/v1/batches",
        job.id.clone(),
        PipelineCaps {
            allow_compression: false,
            allow_decision_routing: false,
            parallel_guardrails: false,
        },
    )
    .await?;
    if turn.is_duplicate || turn.chat_req.model != qualified {
        return Err(ApiErrorResponse::bad_request(
            "A native batch row cannot change model or bypass policy",
        ));
    }
    let mut after = serde_json::to_value(turn.chat_req)
        .map_err(|_| ApiErrorResponse::bad_request("Invalid edited batch projection"))?;
    let original_marker = before["messages"].as_array_mut().and_then(Vec::pop);
    let edited_marker = after["messages"].as_array_mut().and_then(Vec::pop);
    if original_marker != edited_marker || original_marker.as_ref() != Some(&scan_marker) {
        return Err(ApiErrorResponse::bad_request(
            "Batch option/schema/metadata edits require creating a revised input",
        ));
    }
    if before.get("extensions") != after.get("extensions")
        || before.get("stream") != after.get("stream")
    {
        return Err(ApiErrorResponse::bad_request(
            "Native batch routing/stream edits require a revised batch request",
        ));
    }
    let mut result = body.clone();
    result["model"] = json!(job.model);
    let object = result
        .as_object_mut()
        .ok_or_else(|| ApiErrorResponse::bad_request("Invalid batch body"))?;
    match job.endpoint.as_str() {
        "/v1/chat/completions" => {
            let message_metadata = |value: &Value| {
                let mut messages = value["messages"].clone();
                if let Some(messages) = messages.as_array_mut() {
                    for message in messages {
                        if let Some(message) = message.as_object_mut() {
                            message.remove("content");
                        }
                    }
                }
                messages
            };
            if message_metadata(&before) != message_metadata(&after) {
                return Err(ApiErrorResponse::bad_request("Native chat batch edits must preserve message roles, tool calls and identifiers"));
            }
            let mut keys = std::collections::HashSet::new();
            for key in before
                .as_object()
                .into_iter()
                .flat_map(|v| v.keys())
                .chain(after.as_object().into_iter().flat_map(|v| v.keys()))
            {
                keys.insert(key.clone());
            }
            for key in keys {
                if ["model", "stream", "extensions"].contains(&key.as_str()) {
                    continue;
                }
                if before.get(&key) != after.get(&key) {
                    if ![
                        "messages",
                        "temperature",
                        "top_p",
                        "max_tokens",
                        "max_completion_tokens",
                        "frequency_penalty",
                        "presence_penalty",
                        "seed",
                        "logprobs",
                        "top_logprobs",
                        "n",
                        "parallel_tool_calls",
                    ]
                    .contains(&key.as_str())
                    {
                        return Err(ApiErrorResponse::bad_request(
                            "Native batch schema/tool/metadata edits require a revised input",
                        ));
                    }
                    if let Some(value) = after.get(&key) {
                        object.insert(key, value.clone());
                    } else {
                        object.remove(&key);
                    }
                }
            }
        }
        "/v1/responses" => {
            if before["messages"] != after["messages"] || before.get("tools") != after.get("tools")
            {
                return Err(ApiErrorResponse::bad_request("Native Responses batch history/tool edits are unsupported; revise the input explicitly"));
            }
            for (from, to) in [
                ("temperature", "temperature"),
                ("top_p", "top_p"),
                ("max_tokens", "max_output_tokens"),
            ] {
                if before.get(from) != after.get(from) {
                    if let Some(value) = after.get(from) {
                        object.insert(to.into(), value.clone());
                    } else {
                        object.remove(to);
                    }
                }
            }
            for key in after.as_object().into_iter().flat_map(|v| v.keys()) {
                if ![
                    "model",
                    "messages",
                    "stream",
                    "temperature",
                    "top_p",
                    "max_tokens",
                ]
                .contains(&key.as_str())
                    && before.get(key) != after.get(key)
                {
                    return Err(ApiErrorResponse::bad_request(
                        "Unsupported native Responses batch edit",
                    ));
                }
            }
        }
        "/v1/embeddings" => {
            let original = before["messages"]
                .as_array()
                .ok_or_else(|| ApiErrorResponse::bad_request("Invalid embedding projection"))?;
            let edited = after["messages"]
                .as_array()
                .filter(|m| m.len() == original.len())
                .ok_or_else(|| {
                    ApiErrorResponse::bad_request("Embedding edits cannot change input cardinality")
                })?;
            let mut inputs = Vec::new();
            for message in edited {
                if message["role"] != "user" || !message["content"].is_string() {
                    return Err(ApiErrorResponse::bad_request(
                        "Embedding edits must remain text inputs",
                    ));
                }
                inputs.push(message["content"].clone());
            }
            if body["input"].is_string() {
                object.insert("input".into(), inputs.remove(0));
            } else {
                object.insert("input".into(), Value::Array(inputs));
            }
            for key in after.as_object().into_iter().flat_map(|v| v.keys()) {
                if !["model", "messages", "stream"].contains(&key.as_str())
                    && before.get(key) != after.get(key)
                {
                    return Err(ApiErrorResponse::bad_request(
                        "Embedding generation-option edits are unsupported",
                    ));
                }
            }
        }
        _ => unreachable!(),
    }
    if job.endpoint != "/v1/embeddings" {
        result["store"] = json!(false);
    }
    rows::validate_body(&job.endpoint, &result)?;
    Ok(result)
}
