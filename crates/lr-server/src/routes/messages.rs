//! Anthropic Messages ingress. Native history stays alongside a scan-only
//! projection through the shared authorization and inference pipeline.
use axum::{
    extract::State,
    http::HeaderMap,
    response::{IntoResponse, Response},
    Extension, Json,
};
use lr_providers::messages::{MessagesContext, MessagesHeaders, MessagesRequest, RESPONSE};
use serde_json::{json, Value};

use super::{
    gateway_json::GatewayJson,
    pipeline::{run_turn_pipeline, PipelineCaps},
};
use crate::{
    middleware::{
        client_auth::ClientAuthContext,
        error::{ApiErrorResponse, ApiResult},
    },
    state::{AppState, AuthContext},
    types::ChatCompletionRequest,
};

#[derive(serde::Serialize, utoipa::ToSchema)]
pub struct MessagesErrorResponse {
    pub r#type: String,
    pub error: MessagesErrorDetail,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub localrouter: Option<Value>,
}
#[derive(serde::Serialize, utoipa::ToSchema)]
pub struct MessagesErrorDetail {
    pub r#type: String,
    pub message: String,
}

/// Normalize handler, extraction and authentication errors into the native SDK
/// envelope while retaining status, retry hints and routing diagnostics.
pub(crate) async fn native_error_response(response: Response) -> Response {
    let (mut parts, body) = response.into_parts();
    let status = parts.status.as_u16();
    let bytes = axum::body::to_bytes(body, 64 * 1024)
        .await
        .unwrap_or_default();
    let value: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    let message = value["error"]["message"]
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| {
            if bytes.is_empty() {
                "Messages request failed".into()
            } else {
                String::from_utf8_lossy(&bytes).into_owned()
            }
        });
    let kind = match status {
        401 => "authentication_error",
        403 => "permission_error",
        404 => "not_found_error",
        413 => "request_too_large",
        429 => "rate_limit_error",
        529 => "overloaded_error",
        400..500 => "invalid_request_error",
        _ => "api_error",
    };
    let mut body = json!({"type":"error","error":{"type":kind,"message":message}});
    if let Some(extensions) = value.get("extensions") {
        body["localrouter"] = json!({"extensions":extensions});
    }
    parts.headers.remove(axum::http::header::CONTENT_LENGTH);
    parts.headers.insert(
        axum::http::header::CONTENT_TYPE,
        axum::http::HeaderValue::from_static("application/json"),
    );
    Response::from_parts(parts, axum::body::Body::from(body.to_string()))
}

#[utoipa::path(post,path="/v1/messages",tag="messages",request_body=MessagesRequest,
    responses((status=200,description="Native Messages response or Messages SSE",body=lr_providers::messages::MessagesResponse),
    (status=400,description="Unsupported or invalid Messages request",body=MessagesErrorResponse),
    (status=401,description="Unauthorized",body=MessagesErrorResponse),
    (status=429,description="Rate limit exceeded",body=MessagesErrorResponse)),
    security(("bearer_auth"=[]),("anthropic_api_key"=[])))]
pub async fn create_message(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    client_auth: Option<Extension<ClientAuthContext>>,
    headers: HeaderMap,
    GatewayJson(mut native): GatewayJson<MessagesRequest>,
) -> ApiResult<Response> {
    native.validate().map_err(ApiErrorResponse::from)?;
    if !native.model.contains('/') {
        native.model = state
            .router
            .resolve_messages_model(&auth.api_key_id, &native.model)
            .await
            .map_err(ApiErrorResponse::from)?;
    }
    let headers = MessagesHeaders::from_headers(&headers).map_err(ApiErrorResponse::from)?;
    if lr_types::is_duplicate_hop() {
        return Err(ApiErrorResponse::bad_request(
            "Native Messages duplicate-hop forwarding is not supported",
        ));
    }
    // Bound histories cannot be classified or moved between providers. Session
    // binding support is added only with an established, freshly allowed pin.
    let strict_session = native
        .extensions
        .as_ref()
        .and_then(|v| v.get(lr_providers::session::EXTENSION))
        .and_then(|v| v.get("require_continuity"))
        .and_then(Value::as_bool)
        == Some(true);
    if native.has_bound_state()
        && ((native.model == "localrouter/auto" && !strict_session) || !native.model.contains('/'))
    {
        return Err(ApiErrorResponse::bad_request(
            "Native tool/thinking history requires an explicit provider/model or an established session with require_continuity",
        ));
    }
    if state
        .client_manager
        .get_client(&auth.api_key_id)
        .is_some_and(|c| c.is_mcp_via_llm())
    {
        if headers.beta.is_some()
            || headers
                .version
                .as_deref()
                .is_some_and(|v| v != "2023-06-01")
        {
            return Err(ApiErrorResponse::bad_request("Native beta/version semantics cannot be translated through automatic MCP execution"));
        }
        let translated = native.translated_chat().map_err(ApiErrorResponse::from)?;
        let request: ChatCompletionRequest = serde_json::from_value(
            serde_json::to_value(translated)
                .map_err(|e| ApiErrorResponse::bad_request(e.to_string()))?,
        )
        .map_err(|e| ApiErrorResponse::bad_request(e.to_string()))?;
        let streaming = request.stream;
        let response = super::chat::chat_completions_with_endpoint(
            state,
            auth,
            client_auth,
            request,
            "/v1/messages",
            Some(serde_json::to_value(&native).unwrap_or_default()),
        )
        .await?;
        if streaming {
            return Ok(super::messages_stream::translate_chat_response(response));
        }
        return translate_non_streaming(response).await;
    }
    let projection: ChatCompletionRequest = serde_json::from_value(native.policy_projection())
        .map_err(|e| {
            ApiErrorResponse::bad_request(format!("Invalid Messages policy projection: {e}"))
        })?;
    let session_id = uuid::Uuid::new_v4().to_string();
    state.emit_event("llm-request", "messages");
    state.record_client_activity(&auth.api_key_id);
    let mut guard = super::monitor_helpers::emit_llm_call(
        &state,
        client_auth.as_ref(),
        Some(&session_id),
        "/v1/messages",
        &native.model,
        native.stream,
        &serde_json::to_value(&native).unwrap_or_default(),
    );
    let mut turn = run_turn_pipeline(
        &state,
        &auth,
        client_auth.as_ref(),
        projection,
        &mut guard,
        "/v1/messages",
        session_id,
        PipelineCaps {
            allow_compression: false,
            allow_decision_routing: false,
            parallel_guardrails: false,
        },
    )
    .await?;
    turn.provider_request.native_messages = Some(Box::new(MessagesContext {
        request: native,
        headers,
    }));
    if turn.chat_req.stream {
        return super::messages_stream::stream_message(
            state,
            auth,
            turn.chat_req,
            turn.provider_request,
            guard.into_event_id(),
        )
        .await;
    }
    let response = super::chat::handle_non_streaming(
        state,
        auth,
        client_auth,
        turn.chat_req,
        turn.provider_request,
        None,
        0,
        guard.into_event_id(),
    )
    .await?;
    translate_non_streaming(response).await
}

async fn translate_non_streaming(response: Response) -> ApiResult<Response> {
    let body = axum::body::to_bytes(response.into_body(), 32 * 1024 * 1024)
        .await
        .map_err(|e| {
            ApiErrorResponse::bad_gateway(format!("Messages response exceeds local limit: {e}"))
        })?;
    let body: Value = serde_json::from_slice(&body)
        .map_err(|e| ApiErrorResponse::bad_gateway(format!("Invalid Messages response: {e}")))?;
    Ok(Json(messages_response(body)?).into_response())
}

#[utoipa::path(post,path="/v1/messages/count_tokens",tag="messages",request_body=lr_providers::messages::CountMessageTokensRequest,
    responses((status=200,description="Provider token-count estimate; no generation",body=lr_providers::messages::CountMessageTokensResponse),
    (status=400,description="Unsupported or invalid token-count request",body=MessagesErrorResponse),
    (status=401,description="Unauthorized",body=MessagesErrorResponse)),security(("bearer_auth"=[]),("anthropic_api_key"=[])))]
pub async fn count_tokens(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    client_auth: Option<Extension<ClientAuthContext>>,
    headers: HeaderMap,
    GatewayJson(request): GatewayJson<lr_providers::messages::CountMessageTokensRequest>,
) -> ApiResult<Response> {
    let mut native = request.into_messages();
    native.validate().map_err(ApiErrorResponse::from)?;
    if !native.model.contains('/') {
        native.model = state
            .router
            .resolve_messages_model(&auth.api_key_id, &native.model)
            .await
            .map_err(ApiErrorResponse::from)?;
    }
    if lr_types::is_duplicate_hop() {
        return Err(ApiErrorResponse::bad_request(
            "Native token counting duplicate-hop forwarding is not supported",
        ));
    }
    if native.model == "localrouter/auto" || !native.model.contains('/') {
        return Err(ApiErrorResponse::bad_request(
            "Native token counting requires explicit provider/model",
        ));
    }
    let headers = MessagesHeaders::from_headers(&headers).map_err(ApiErrorResponse::from)?;
    let projection: ChatCompletionRequest = serde_json::from_value(native.policy_projection())
        .map_err(|e| ApiErrorResponse::bad_request(e.to_string()))?;
    let session_id = uuid::Uuid::new_v4().to_string();
    let started = std::time::Instant::now();
    let mut guard = super::monitor_helpers::emit_llm_call(
        &state,
        client_auth.as_ref(),
        Some(&session_id),
        "/v1/messages/count_tokens",
        &native.model,
        false,
        &serde_json::to_value(&native).unwrap_or_default(),
    );
    let mut turn = run_turn_pipeline(
        &state,
        &auth,
        client_auth.as_ref(),
        projection,
        &mut guard,
        "/v1/messages/count_tokens",
        session_id,
        PipelineCaps {
            allow_compression: false,
            allow_decision_routing: false,
            parallel_guardrails: false,
        },
    )
    .await?;
    turn.provider_request.native_messages = Some(Box::new(MessagesContext {
        request: native,
        headers,
    }));
    let body = state
        .router
        .count_message_tokens(&auth.api_key_id, turn.provider_request)
        .await
        .map_err(|e| guard.capture_err(ApiErrorResponse::from(e)))?;
    let event_id = guard.into_event_id();
    let (provider, model) = turn
        .chat_req
        .model
        .split_once('/')
        .unwrap_or(("unknown", &turn.chat_req.model));
    super::monitor_helpers::complete_llm_call(
        &state,
        &event_id,
        provider,
        model,
        200,
        0,
        0,
        None,
        Some(0.0),
        started.elapsed().as_millis() as u64,
        Some("token_count"),
        "Provider token-count estimate; no generation",
        false,
    );
    super::monitor_helpers::update_llm_call_response_body(&state, &event_id, &body);
    Ok(Json(body).into_response())
}

fn messages_response(mut body: Value) -> ApiResult<Value> {
    if let Some(native) = body
        .get_mut("extensions")
        .and_then(Value::as_object_mut)
        .and_then(|v| v.remove(RESPONSE))
    {
        return Ok(native);
    }
    let choice = body["choices"]
        .as_array()
        .filter(|v| v.len() == 1)
        .and_then(|v| v.first())
        .ok_or_else(|| {
            ApiErrorResponse::bad_gateway("Messages translation requires one completion")
        })?;
    let mut content = Vec::new();
    if let Some(text) = choice["message"]["content"]
        .as_str()
        .filter(|s| !s.is_empty())
    {
        content.push(json!({"type":"text","text":text}));
    }
    if let Some(tools) = choice["message"]["tool_calls"].as_array() {
        for tool in tools {
            let arguments = tool["function"]["arguments"].as_str().ok_or_else(|| {
                ApiErrorResponse::bad_gateway("Invalid translated tool arguments")
            })?;
            let input: Value = serde_json::from_str(arguments).map_err(|_| {
                ApiErrorResponse::bad_gateway("Translated tool arguments are not valid JSON")
            })?;
            if !input.is_object() {
                return Err(ApiErrorResponse::bad_gateway(
                    "Translated tool arguments must be an object",
                ));
            }
            content.push(json!({"type":"tool_use","id":tool["id"],"name":tool["function"]["name"],"input":input}));
        }
    }
    let usage = &body["usage"];
    let read = usage["prompt_tokens_details"]["cache_read_tokens"]
        .as_u64()
        .or_else(|| usage["prompt_tokens_details"]["cached_tokens"].as_u64())
        .unwrap_or(0);
    let write = usage["prompt_tokens_details"]["cache_creation_tokens"]
        .as_u64()
        .unwrap_or(0);
    let finish = match choice["finish_reason"].as_str() {
        Some("stop") => "end_turn",
        Some("length") => "max_tokens",
        Some("tool_calls") => "tool_use",
        _ => {
            return Err(ApiErrorResponse::bad_gateway(
                "Unsupported translated Messages finish reason",
            ))
        }
    };
    Ok(
        json!({"id":format!("msg_{}",uuid::Uuid::new_v4().simple()),"type":"message","role":"assistant","model":body["model"],"content":content,"stop_reason":finish,"stop_sequence":null,
        "usage":{"input_tokens":usage["prompt_tokens"].as_u64().unwrap_or(0).saturating_sub(read).saturating_sub(write),"output_tokens":usage["completion_tokens"],"cache_read_input_tokens":read,"cache_creation_input_tokens":write},
        "localrouter":{"transport":"chat_translation","extensions":body.get("extensions")}}),
    )
}
