//! Authenticated ranked retrieval through the existing sequential policy pipeline.
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
use lr_providers::search::{SearchRequest, SearchResponse};
#[utoipa::path(post,path="/v1/search",tag="search",request_body=SearchRequest,
    responses((status=200,description="Ranked page results; missing request charges remain unknown",body=SearchResponse),(status=400,description="Invalid request or unsupported policy",body=crate::types::ErrorResponse),(status=401,description="Unauthorized",body=crate::types::ErrorResponse),(status=429,description="Rate limit exceeded",body=crate::types::ErrorResponse)),security(("bearer_auth"=[])))]
pub async fn search(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    client_auth: Option<Extension<ClientAuthContext>>,
    GatewayJson(mut request): GatewayJson<SearchRequest>,
) -> ApiResult<Response> {
    request.validate().map_err(ApiErrorResponse::from)?;
    let policy = state
        .router
        .authorize_ranked_search(&auth.api_key_id)
        .map_err(ApiErrorResponse::from)?;
    if request.model.is_none() {
        request.model = policy.default_search_model;
    }
    let model = request.model.clone().ok_or_else(|| {
        ApiErrorResponse::bad_request(
            "Provide a qualified search model or configure a client search default",
        )
    })?;
    let (provider, upstream_model) = model
        .split_once('/')
        .map(|(provider, model)| (provider.to_owned(), model.to_owned()))
        .ok_or_else(|| {
            ApiErrorResponse::bad_request("Ranked search requires a qualified provider/model")
        })?;
    if provider.is_empty() || provider == "localrouter" || upstream_model.is_empty() {
        return Err(ApiErrorResponse::bad_request(
            "Ranked search requires an explicit provider/model",
        ));
    }
    let instance = state
        .provider_registry
        .get_provider(&provider)
        .ok_or_else(|| ApiErrorResponse::bad_request("Search provider is unavailable"))?;
    let protocol = instance
        .search_protocol()
        .ok_or_else(|| ApiErrorResponse::bad_request("Provider has no native ranked Search API"))?;
    let mut wire = request.clone();
    wire.model = Some(upstream_model.clone());
    wire.wire_body(protocol).map_err(ApiErrorResponse::from)?;
    if lr_types::is_duplicate_hop()
        || state
            .config_manager
            .get()
            .clients
            .iter()
            .any(|client| client.id == auth.api_key_id && client.is_mcp_via_llm())
    {
        return Err(ApiErrorResponse::bad_request(
            "Ranked search does not support automatic MCP or duplicate gateway hops",
        ));
    }
    let body = serde_json::to_value(&request)
        .map_err(|_| ApiErrorResponse::internal_error("Invalid search request"))?;
    let session_id = uuid::Uuid::new_v4().to_string();
    let started = std::time::Instant::now();
    let mut guard = monitor_helpers::emit_llm_call(
        &state,
        client_auth.as_ref(),
        Some(&session_id),
        "/v1/search",
        &model,
        false,
        &body,
    );
    state.record_client_activity(&auth.api_key_id);
    state.emit_event("llm-request", "search");
    let projection:ChatCompletionRequest=serde_json::from_value(serde_json::json!({"model":model,"messages":request.query.queries().into_iter().map(|query|serde_json::json!({"role":"user","content":query})).collect::<Vec<_>>()})).map_err(|_|ApiErrorResponse::bad_request("Invalid search query projection"))?;
    let expected = serde_json::to_value(&projection)
        .map_err(|_| ApiErrorResponse::bad_request("Invalid search projection"))?;
    let context = run_turn_pipeline(
        &state,
        &auth,
        client_auth.as_ref(),
        projection,
        &mut guard,
        "/v1/search",
        session_id,
        PipelineCaps {
            allow_compression: false,
            allow_decision_routing: false,
            parallel_guardrails: false,
        },
    )
    .await?;
    // Chat-specific approval edits cannot silently diverge from the query that
    // will actually be sent. Ask for a new explicit Search request instead.
    if serde_json::to_value(&context.chat_req).ok().as_ref() != Some(&expected) {
        return Err(guard.capture_err(ApiErrorResponse::bad_request(
            "Search query rewrites require resubmitting an explicit Search request",
        )));
    }
    let response = match state.router.ranked_search(&auth.api_key_id, request).await {
        Ok(response) => response,
        Err(error) => {
            let mut response = guard
                .capture_err(ApiErrorResponse::from(error))
                .into_response();
            // OpenAI-compatible SDKs honor this hint. An upstream request may
            // already have been billed when its response is lost or invalid.
            response.headers_mut().insert(
                "x-should-retry",
                axum::http::HeaderValue::from_static("false"),
            );
            return Ok(response);
        }
    };
    let value = serde_json::to_value(&response)
        .map_err(|_| ApiErrorResponse::internal_error("Invalid search response"))?;
    monitor_helpers::update_llm_call_response_body(&state, guard.event_id(), &value);
    monitor_helpers::update_llm_call_upstream(
        &state,
        guard.event_id(),
        Some(lr_monitor::LlmApi::Search),
        None,
    );
    let preview = response
        .results
        .iter()
        .map(|result| result.title.as_str())
        .collect::<Vec<_>>()
        .join("; ");
    guard.complete(
        &state,
        &provider,
        &upstream_model,
        200,
        0,
        0,
        None,
        None,
        started.elapsed().as_millis() as u64,
        Some("complete"),
        &preview,
        false,
    );
    Ok(Json(response).into_response())
}
