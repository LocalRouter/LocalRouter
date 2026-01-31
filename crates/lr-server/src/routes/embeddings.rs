//! POST /v1/embeddings endpoint
//!
//! Convert text to vector embeddings.

use axum::{
    extract::State,
    response::{IntoResponse, Response},
    Extension, Json,
};
use std::time::Instant;
use uuid::Uuid;

use super::helpers::{get_enabled_client, get_enabled_client_from_manager};
use lr_router::UsageInfo;
use crate::middleware::client_auth::ClientAuthContext;
use crate::middleware::error::{ApiErrorResponse, ApiResult};
use crate::state::{AppState, AuthContext};
use crate::types::{
    EmbeddingData, EmbeddingInput, EmbeddingRequest, EmbeddingResponse, EmbeddingVector,
};

/// POST /v1/embeddings
/// Generate embeddings for input text(s)
#[utoipa::path(
    post,
    path = "/v1/embeddings",
    tag = "embeddings",
    request_body = EmbeddingRequest,
    responses(
        (status = 200, description = "Successful response", body = crate::types::EmbeddingResponse),
        (status = 400, description = "Bad request", body = crate::types::ErrorResponse),
        (status = 401, description = "Unauthorized", body = crate::types::ErrorResponse),
        (status = 501, description = "Not implemented yet", body = crate::types::ErrorResponse),
        (status = 500, description = "Internal server error", body = crate::types::ErrorResponse)
    ),
    security(
        ("bearer_auth" = [])
    )
)]
pub async fn embeddings(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    client_auth: Option<Extension<ClientAuthContext>>,
    Json(request): Json<EmbeddingRequest>,
) -> ApiResult<Response> {
    // Emit LLM request event to trigger tray icon indicator
    state.emit_event("llm-request", "embedding");

    // Record client activity for connection graph
    state.record_client_activity(&auth.api_key_id);

    // Validate client is enabled (skip for internal test token)
    if auth.api_key_id != "internal-test" {
        let _client = get_enabled_client(&state, &auth.api_key_id)?;
    }

    // Validate request
    validate_request(&request)?;

    // Validate client provider access (if using client auth)
    validate_client_provider_access(&state, client_auth.as_ref().map(|e| &e.0), &request).await?;

    // Check rate limits
    check_rate_limits(&state, &auth, &request).await?;

    // Generate a unique ID for this request
    let request_id = format!("emb-{}", Uuid::new_v4());
    let started_at = Instant::now();

    // Convert encoding_format from String to EncodingFormat
    let encoding_format = request
        .encoding_format
        .as_ref()
        .and_then(|fmt| match fmt.as_str() {
            "float" => Some(lr_providers::EncodingFormat::Float),
            "base64" => Some(lr_providers::EncodingFormat::Base64),
            _ => None,
        });

    // Convert server EmbeddingInput to provider EmbeddingInput
    let provider_input = match request.input.clone() {
        crate::types::EmbeddingInput::Single(s) => {
            lr_providers::EmbeddingInput::Single(s)
        }
        crate::types::EmbeddingInput::Multiple(v) => {
            lr_providers::EmbeddingInput::Multiple(v)
        }
    };

    // Convert to provider format
    let provider_request = lr_providers::EmbeddingRequest {
        model: request.model.clone(),
        input: provider_input,
        encoding_format,
        dimensions: request.dimensions,
        user: request.user.clone(),
    };

    // Call router to get embeddings
    let response = match state.router.embed(&auth.api_key_id, provider_request).await {
        Ok(resp) => resp,
        Err(e) => {
            // Record failure metrics
            let latency = Instant::now().duration_since(started_at).as_millis() as u64;
            let strategy_id = state
                .client_manager
                .get_client(&auth.api_key_id)
                .map(|c| c.strategy_id.clone())
                .unwrap_or_else(|| "default".to_string());
            state.metrics_collector.record_failure(
                &auth.api_key_id,
                "unknown",
                &request.model,
                &strategy_id,
                latency,
            );

            // Log to access log (persistent storage)
            if let Err(log_err) = state.access_logger.log_failure(
                &auth.api_key_id,
                "unknown",
                &request.model,
                latency,
                &request_id,
                502, // Bad Gateway
            ) {
                tracing::warn!("Failed to write access log: {}", log_err);
            }

            tracing::error!("Embedding request failed: {}", e);
            return Err(ApiErrorResponse::bad_gateway(format!(
                "Provider error: {}",
                e
            )));
        }
    };

    let completed_at = Instant::now();
    let latency_ms = completed_at.duration_since(started_at).as_millis() as u64;

    // Get pricing info for cost calculation
    let provider = response
        .model
        .split('/')
        .next()
        .unwrap_or("unknown")
        .to_string();
    let pricing = match state.provider_registry.get_provider(&provider) {
        Some(p) => p.get_pricing(&response.model).await.ok(),
        None => None,
    }
    .unwrap_or_else(lr_providers::PricingInfo::free);

    let cost = (response.usage.prompt_tokens as f64 / 1000.0) * pricing.input_cost_per_1k;

    // Get client's strategy_id for metrics
    let strategy_id = state
        .client_manager
        .get_client(&auth.api_key_id)
        .map(|c| c.strategy_id.clone())
        .unwrap_or_else(|| "default".to_string());

    // Record success metrics
    state
        .metrics_collector
        .record_success(&lr_monitoring::metrics::RequestMetrics {
            api_key_name: &auth.api_key_id,
            provider: &provider,
            model: &response.model,
            strategy_id: &strategy_id,
            input_tokens: response.usage.prompt_tokens as u64,
            output_tokens: 0, // Embeddings don't have output tokens
            cost_usd: cost,
            latency_ms,
        });

    // Record tokens for tray graph (real-time tracking)
    if let Some(ref tray_graph) = *state.tray_graph_manager.read() {
        tray_graph.record_tokens(response.usage.total_tokens as u64);
    }

    // Log to access log (persistent storage)
    if let Err(e) = state.access_logger.log_success(
        &auth.api_key_id,
        &provider,
        &response.model,
        response.usage.prompt_tokens as u64,
        0, // Embeddings don't have output tokens
        cost,
        latency_ms,
        &request_id,
    ) {
        tracing::warn!("Failed to write access log: {}", e);
    }

    // Convert provider response to API response
    let api_response = EmbeddingResponse {
        object: response.object,
        data: response
            .data
            .into_iter()
            .map(|emb| EmbeddingData {
                object: emb.object,
                embedding: if let Some(vec) = emb.embedding {
                    EmbeddingVector::Float(vec)
                } else {
                    EmbeddingVector::Float(vec![]) // Default to empty if none
                },
                index: emb.index as u32,
            })
            .collect(),
        model: response.model,
        usage: crate::types::EmbeddingUsage {
            prompt_tokens: response.usage.prompt_tokens,
            total_tokens: response.usage.total_tokens,
        },
    };

    // Log success
    tracing::info!(
        "Embedding request completed: id={}, model={}, tokens={}, latency={}ms",
        request_id,
        api_response.model,
        api_response.usage.total_tokens,
        latency_ms
    );

    // Return JSON response
    Ok(Json(api_response).into_response())
}

/// Validate embedding request
fn validate_request(request: &EmbeddingRequest) -> ApiResult<()> {
    if request.model.is_empty() {
        return Err(ApiErrorResponse::bad_request("model is required").with_param("model"));
    }

    // Validate input is not empty
    match &request.input {
        EmbeddingInput::Single(s) => {
            if s.is_empty() {
                return Err(
                    ApiErrorResponse::bad_request("input cannot be empty").with_param("input")
                );
            }
        }
        EmbeddingInput::Multiple(v) => {
            if v.is_empty() {
                return Err(ApiErrorResponse::bad_request("input array cannot be empty")
                    .with_param("input"));
            }
            // Also check that individual strings in the array are not empty
            if v.iter().any(|s| s.is_empty()) {
                return Err(
                    ApiErrorResponse::bad_request("input array contains empty strings")
                        .with_param("input"),
                );
            }
        }
    }

    // Validate encoding format
    if let Some(format) = &request.encoding_format {
        if format != "float" && format != "base64" {
            return Err(ApiErrorResponse::bad_request(
                "encoding_format must be 'float' or 'base64'",
            )
            .with_param("encoding_format"));
        }
    }

    // Validate dimensions if provided
    if let Some(dimensions) = request.dimensions {
        if dimensions == 0 {
            return Err(
                ApiErrorResponse::bad_request("dimensions must be greater than 0")
                    .with_param("dimensions"),
            );
        }
    }

    Ok(())
}

/// Check rate limits before processing request
async fn check_rate_limits(
    state: &AppState,
    auth: &AuthContext,
    request: &EmbeddingRequest,
) -> ApiResult<()> {
    // Estimate input tokens based on input text length
    let estimated_tokens = match &request.input {
        EmbeddingInput::Single(s) => (s.len() / 4).max(1) as u64,
        EmbeddingInput::Multiple(v) => v.iter().map(|s| (s.len() / 4).max(1) as u64).sum(),
    };

    let usage_estimate = UsageInfo {
        input_tokens: estimated_tokens,
        output_tokens: 0, // Embeddings don't have output tokens
        cost_usd: 0.0,    // Can't estimate cost without knowing provider
    };

    let rate_limit_result = state
        .rate_limiter
        .check_api_key(&auth.api_key_id, &usage_estimate)
        .await
        .map_err(|e| ApiErrorResponse::internal_error(format!("Rate limit check failed: {}", e)))?;

    if !rate_limit_result.allowed {
        let mut error = ApiErrorResponse::rate_limited(format!(
            "Rate limit exceeded: {}/{} used",
            rate_limit_result.current_usage, rate_limit_result.limit
        ));

        if let Some(retry_after) = rate_limit_result.retry_after_secs {
            error.error = error
                .error
                .with_code(format!("retry_after_{}", retry_after));
        }

        return Err(error);
    }

    Ok(())
}

/// Validate that the client has access to the requested LLM provider
///
/// This enforces the allowed_llm_providers access control list for clients.
/// Returns 403 Forbidden if the client doesn't have access to the provider.
async fn validate_client_provider_access(
    state: &AppState,
    client_context: Option<&ClientAuthContext>,
    request: &EmbeddingRequest,
) -> ApiResult<()> {
    // If no client context, skip validation (using API key auth)
    let Some(client_ctx) = client_context else {
        return Ok(());
    };

    // Get enabled client
    let client = get_enabled_client_from_manager(state, &client_ctx.client_id)?;

    // Special case: localrouter/auto is a virtual model that routes to actual providers
    // The actual provider access will be checked by the router during auto-routing
    if request.model == "localrouter/auto" {
        tracing::debug!(
            "Client {} using localrouter/auto - provider access will be checked during routing",
            client.id
        );
        return Ok(());
    }

    // Extract provider from model string
    // Format can be "provider/model" or just "model"
    let provider = if let Some((prov, _model)) = request.model.split_once('/') {
        prov.to_string()
    } else {
        // No provider specified - need to find which provider has this model
        let all_models = state
            .provider_registry
            .list_all_models()
            .await
            .map_err(|e| {
                ApiErrorResponse::internal_error(format!("Failed to list models: {}", e))
            })?;

        let matching_model = all_models
            .iter()
            .find(|m| m.id.eq_ignore_ascii_case(&request.model))
            .ok_or_else(|| {
                ApiErrorResponse::bad_request(format!("Model not found: {}", request.model))
                    .with_param("model")
            })?;

        matching_model.provider.clone()
    };

    // Check if provider is in allowed list (case-insensitive comparison)
    let is_allowed = client
        .allowed_llm_providers
        .iter()
        .any(|p| p.eq_ignore_ascii_case(&provider));

    if !is_allowed {
        tracing::warn!(
            "Client {} attempted to access unauthorized LLM provider: {}",
            client.id,
            provider
        );

        return Err(ApiErrorResponse::forbidden(format!(
            "Access denied: Client is not authorized to use provider '{}'. Contact administrator to grant access.",
            provider
        ))
        .with_param("model"));
    }

    tracing::debug!(
        "Client {} authorized for LLM provider: {}",
        client.id,
        provider
    );

    Ok(())
}
