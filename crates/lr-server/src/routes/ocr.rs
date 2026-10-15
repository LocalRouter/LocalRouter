//! Explicit native OCR with sequential policy checks and a fixed page budget.
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
use lr_providers::ocr::{OcrRequest, OcrResponse};
#[utoipa::path(post,path="/v1/ocr",tag="documents",request_body=OcrRequest,responses((status=200,description="Untrusted extracted pages with native usage and source provenance",body=OcrResponse),(status=400,description="Invalid pages or unsupported operation/policy",body=crate::types::ErrorResponse),(status=401,description="Unauthorized",body=crate::types::ErrorResponse),(status=429,description="Rate limited",body=crate::types::ErrorResponse),(status=502,description="Native dispatch failed; outcome and charges may be unknown",body=crate::types::ErrorResponse)),security(("bearer_auth"=[])))]
pub async fn ocr(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    client_auth: Option<Extension<ClientAuthContext>>,
    GatewayJson(request): GatewayJson<OcrRequest>,
) -> ApiResult<Response> {
    state
        .router
        .authorize_native_operation(&auth.api_key_id, lr_config::NativeOperation::Ocr)
        .map_err(ApiErrorResponse::from)?;
    request.validate().map_err(ApiErrorResponse::from)?;
    let (provider, model) = request
        .model
        .split_once('/')
        .filter(|(provider, model)| {
            !provider.is_empty() && *provider != "localrouter" && !model.is_empty()
        })
        .ok_or_else(|| {
            ApiErrorResponse::bad_request("OCR requires an explicit qualified provider/model")
        })?;
    let instance = state
        .provider_registry
        .get_provider(provider)
        .ok_or_else(|| ApiErrorResponse::bad_request("OCR provider is unavailable"))?;
    let protocol = instance.ocr_protocol().ok_or_else(|| {
        ApiErrorResponse::bad_request("Provider has no verified native OCR mapping")
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
            "OCR does not support automatic MCP or duplicate gateway hops",
        ));
    }
    crate::documents::enforce_opaque_scan_policy(&state, &auth.api_key_id)?;
    let body = serde_json::to_value(&request)
        .map_err(|_| ApiErrorResponse::bad_request("Invalid OCR request"))?;
    let source = request.source().map_err(ApiErrorResponse::from)?;
    let projection:ChatCompletionRequest=serde_json::from_value(serde_json::json!({"model":request.model,"messages":[{"role":"user","content":format!("[Explicit OCR: {}; MIME: {}; SHA-256: {}; selected pages: {:?}; document bytes are opaque]",source.filename.as_deref().unwrap_or("unnamed"),source.mime,source.sha256,request.pages.indices().map_err(ApiErrorResponse::from)?)}]})).map_err(|_|ApiErrorResponse::bad_request("Invalid OCR policy projection"))?;
    let expected = serde_json::to_value(&projection)
        .map_err(|_| ApiErrorResponse::bad_request("Invalid OCR projection"))?;
    let session = uuid::Uuid::new_v4().to_string();
    let started = std::time::Instant::now();
    let mut guard = monitor_helpers::emit_llm_call(
        &state,
        client_auth.as_ref(),
        Some(&session),
        "/v1/ocr",
        &request.model,
        false,
        &body,
    );
    state.record_client_activity(&auth.api_key_id);
    state.emit_event("llm-request", "ocr");
    let context = run_turn_pipeline(
        &state,
        &auth,
        client_auth.as_ref(),
        projection,
        &mut guard,
        "/v1/ocr",
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
            "OCR model/content rewrites require a new explicit OCR request",
        )));
    }
    let provider = provider.to_owned();
    let model = model.to_owned();
    let response = state
        .router
        .ocr(&auth.api_key_id, request)
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
        Some(lr_types::LlmApi::Ocr),
        None,
    );
    guard.complete(
        &state,
        &provider,
        &model,
        200,
        0,
        0,
        None,
        None,
        started.elapsed().as_millis() as u64,
        Some("complete"),
        &format!("{} OCR pages", response.usage_info.pages_processed),
        false,
    );
    Ok(Json(response).into_response())
}
