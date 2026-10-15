//! Native reranking over explicitly supplied query/candidate text.
use super::{
    gateway_json::GatewayJson,
    monitor_helpers,
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
use axum::{
    extract::State,
    response::{IntoResponse, Response},
    Extension, Json,
};
use lr_providers::rerank::{RerankRequest, RerankResponse};
#[utoipa::path(post,path="/v1/rerank",tag="retrieval",request_body=RerankRequest,responses((status=200,description="Native ranking with original candidate indices and billed units",body=RerankResponse),(status=400,description="Invalid candidates or unsupported operation/policy",body=crate::types::ErrorResponse),(status=401,description="Unauthorized",body=crate::types::ErrorResponse),(status=429,description="Rate limited",body=crate::types::ErrorResponse),(status=502,description="Native dispatch failed; charges may be unknown",body=crate::types::ErrorResponse)),security(("bearer_auth"=[])))]
pub async fn rerank(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    client_auth: Option<Extension<ClientAuthContext>>,
    GatewayJson(request): GatewayJson<RerankRequest>,
) -> ApiResult<Response> {
    state
        .router
        .authorize_native_operation(&auth.api_key_id, lr_config::NativeOperation::Rerank)
        .map_err(ApiErrorResponse::from)?;
    request.validate().map_err(ApiErrorResponse::from)?;
    let (provider, model) = request
        .model
        .split_once('/')
        .filter(|(provider, model)| {
            !provider.is_empty() && *provider != "localrouter" && !model.is_empty()
        })
        .ok_or_else(|| {
            ApiErrorResponse::bad_request("Rerank requires an explicit qualified provider/model")
        })?;
    let instance = state
        .provider_registry
        .get_provider(provider)
        .ok_or_else(|| ApiErrorResponse::bad_request("Rerank provider is unavailable"))?;
    let protocol = instance.rerank_protocol().ok_or_else(|| {
        ApiErrorResponse::bad_request("Provider has no verified native Rerank mapping")
    })?;
    let mut native = request.clone();
    native.model = model.into();
    native.wire_body(protocol).map_err(ApiErrorResponse::from)?;
    if lr_types::is_duplicate_hop()
        || state
            .config_manager
            .get()
            .clients
            .iter()
            .any(|c| c.id == auth.api_key_id && c.is_mcp_via_llm())
    {
        return Err(ApiErrorResponse::bad_request(
            "Rerank does not support automatic MCP or duplicate gateway hops",
        ));
    }
    let body = serde_json::to_value(&request)
        .map_err(|_| ApiErrorResponse::bad_request("Invalid rerank request"))?;
    let projection:ChatCompletionRequest=serde_json::from_value(serde_json::json!({"model":request.model,"messages":std::iter::once(&request.query).chain(request.documents.iter()).map(|text|serde_json::json!({"role":"user","content":text})).collect::<Vec<_>>()})).map_err(|_|ApiErrorResponse::bad_request("Invalid rerank policy projection"))?;
    let expected = serde_json::to_value(&projection)
        .map_err(|_| ApiErrorResponse::bad_request("Invalid rerank projection"))?;
    let session = uuid::Uuid::new_v4().to_string();
    let started = std::time::Instant::now();
    let mut guard = monitor_helpers::emit_llm_call(
        &state,
        client_auth.as_ref(),
        Some(&session),
        "/v1/rerank",
        &request.model,
        false,
        &body,
    );
    state.record_client_activity(&auth.api_key_id);
    state.emit_event("llm-request", "rerank");
    let context = run_turn_pipeline(
        &state,
        &auth,
        client_auth.as_ref(),
        projection,
        &mut guard,
        "/v1/rerank",
        session,
        PipelineCaps {
            allow_compression: false,
            allow_decision_routing: false,
            parallel_guardrails: false,
        },
    )
    .await?;
    if serde_json::to_value(&context.chat_req).ok().as_ref() != Some(&expected) {
        return Err(guard.capture_err(ApiErrorResponse::bad_request(
            "Rerank model/content rewrites require a new explicit Rerank request",
        )));
    }
    let provider = provider.to_owned();
    let model = model.to_owned();
    let response = state
        .router
        .rerank(&auth.api_key_id, request)
        .await
        .map_err(|e| guard.capture_err(ApiErrorResponse::from(e).without_retry_if(true)))?;
    monitor_helpers::update_llm_call_response_body(
        &state,
        guard.event_id(),
        &serde_json::to_value(&response).unwrap_or_default(),
    );
    monitor_helpers::update_llm_call_upstream(
        &state,
        guard.event_id(),
        Some(lr_types::LlmApi::Rerank),
        None,
    );
    let input_tokens = response
        .meta
        .as_ref()
        .and_then(|m| m.get("billed_units"))
        .and_then(|u| u.get("input_tokens"))
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    guard.complete(
        &state,
        &provider,
        &model,
        200,
        input_tokens,
        0,
        None,
        None,
        started.elapsed().as_millis() as u64,
        Some("complete"),
        &format!("{} ranked candidates", response.results.len()),
        false,
    );
    Ok(Json(response).into_response())
}
