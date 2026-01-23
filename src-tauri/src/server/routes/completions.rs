//! POST /v1/completions endpoint
//!
//! Legacy text completion endpoint (converts to chat format internally).

use axum::{
    extract::State,
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse, Response,
    },
    Extension, Json,
};
use chrono::Utc;
use futures::stream::StreamExt;
use std::time::Instant;
use uuid::Uuid;

use super::helpers::get_enabled_client_from_manager;
use crate::providers::{
    ChatMessage as ProviderChatMessage, ChatMessageContent,
    CompletionRequest as ProviderCompletionRequest,
};
use crate::router::UsageInfo;
use crate::server::middleware::client_auth::ClientAuthContext;
use crate::server::middleware::error::{ApiErrorResponse, ApiResult};
use crate::server::state::{AppState, AuthContext, GenerationDetails};
use crate::server::types::{
    CompletionChoice, CompletionChunk, CompletionChunkChoice, CompletionRequest,
    CompletionResponse, PromptInput, TokenUsage,
};

/// POST /v1/completions
/// Legacy completion endpoint - converts prompt to chat format
/// Supports both streaming and non-streaming responses
#[utoipa::path(
    post,
    path = "/v1/completions",
    tag = "completions",
    request_body = CompletionRequest,
    responses(
        (status = 200, description = "Successful response (non-streaming)", body = CompletionResponse),
        (status = 200, description = "Successful response (streaming)", content_type = "text/event-stream"),
        (status = 400, description = "Bad request", body = crate::server::types::ErrorResponse),
        (status = 401, description = "Unauthorized", body = crate::server::types::ErrorResponse),
        (status = 429, description = "Rate limit exceeded", body = crate::server::types::ErrorResponse),
        (status = 500, description = "Internal server error", body = crate::server::types::ErrorResponse)
    ),
    security(
        ("bearer_auth" = [])
    )
)]
pub async fn completions(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    client_auth: Option<Extension<ClientAuthContext>>,
    Json(request): Json<CompletionRequest>,
) -> ApiResult<Response> {
    // Emit LLM request event to trigger tray icon indicator
    state.emit_event("llm-request", "completion");

    // Validate request
    validate_request(&request)?;

    // Validate client provider access (if using client auth)
    validate_client_provider_access(&state, client_auth.as_ref().map(|e| &e.0), &request).await?;

    // Check rate limits
    check_rate_limits(&state, &auth, &request).await?;

    // Convert prompt to chat messages format
    let messages = convert_prompt_to_messages(&request.prompt)?;

    // Create provider request
    let provider_request = ProviderCompletionRequest {
        model: request.model.clone(),
        messages,
        temperature: request.temperature,
        max_tokens: request.max_tokens,
        stream: request.stream,
        top_p: request.top_p,
        frequency_penalty: request.frequency_penalty,
        presence_penalty: request.presence_penalty,
        stop: request.stop.as_ref().map(|s| match s {
            crate::server::types::StopSequence::Single(s) => vec![s.clone()],
            crate::server::types::StopSequence::Multiple(v) => v.clone(),
        }),
        // Extended parameters (not supported in legacy completions endpoint)
        top_k: None,
        seed: None,
        repetition_penalty: None,
        extensions: None,
        // Tool calling (not supported in legacy completions endpoint)
        tools: None,
        tool_choice: None,
        // Response format (not supported in legacy completions endpoint)
        response_format: None,
        // Log probabilities (not supported in legacy completions endpoint)
        logprobs: None,
        top_logprobs: None,
    };

    if request.stream {
        handle_streaming(state, auth, request, provider_request).await
    } else {
        handle_non_streaming(state, auth, request, provider_request).await
    }
}

/// Validate completion request
fn validate_request(request: &CompletionRequest) -> ApiResult<()> {
    if request.model.is_empty() {
        return Err(ApiErrorResponse::bad_request("model is required").with_param("model"));
    }

    // Validate temperature
    if let Some(temp) = request.temperature {
        if !(0.0..=2.0).contains(&temp) {
            return Err(
                ApiErrorResponse::bad_request("temperature must be between 0 and 2")
                    .with_param("temperature"),
            );
        }
    }

    // Validate top_p
    if let Some(top_p) = request.top_p {
        if !(0.0..=1.0).contains(&top_p) {
            return Err(
                ApiErrorResponse::bad_request("top_p must be between 0 and 1").with_param("top_p"),
            );
        }
    }

    // Validate n parameter
    if let Some(n) = request.n {
        if n == 0 {
            return Err(ApiErrorResponse::bad_request("n must be at least 1").with_param("n"));
        }
        if n > 128 {
            return Err(ApiErrorResponse::bad_request("n must be at most 128").with_param("n"));
        }
        if n > 1 && request.stream {
            return Err(
                ApiErrorResponse::bad_request("n > 1 is not supported with streaming")
                    .with_param("n"),
            );
        }
    }

    // Validate frequency_penalty (OpenAI range: -2.0 to 2.0)
    if let Some(freq_penalty) = request.frequency_penalty {
        if !(-2.0..=2.0).contains(&freq_penalty) {
            return Err(ApiErrorResponse::bad_request(
                "frequency_penalty must be between -2 and 2",
            )
            .with_param("frequency_penalty"));
        }
    }

    // Validate presence_penalty (OpenAI range: -2.0 to 2.0)
    if let Some(pres_penalty) = request.presence_penalty {
        if !(-2.0..=2.0).contains(&pres_penalty) {
            return Err(
                ApiErrorResponse::bad_request("presence_penalty must be between -2 and 2")
                    .with_param("presence_penalty"),
            );
        }
    }

    Ok(())
}

/// Check rate limits before processing request
async fn check_rate_limits(
    state: &AppState,
    auth: &AuthContext,
    request: &CompletionRequest,
) -> ApiResult<()> {
    // Estimate usage for rate limit check (rough estimate)
    let estimated_tokens = estimate_prompt_tokens(&request.prompt);
    let max_output_tokens = request.max_tokens.unwrap_or(100);
    let usage_estimate = UsageInfo {
        input_tokens: estimated_tokens,
        output_tokens: max_output_tokens as u64,
        cost_usd: 0.0, // Can't estimate cost without knowing provider
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

/// Convert prompt(s) to chat message format
fn convert_prompt_to_messages(prompt: &PromptInput) -> ApiResult<Vec<ProviderChatMessage>> {
    let prompts = match prompt {
        PromptInput::Single(p) => vec![p.clone()],
        PromptInput::Multiple(ps) => ps.clone(),
    };

    // Convert each prompt to a user message
    let messages = prompts
        .into_iter()
        .map(|p| ProviderChatMessage {
            role: "user".to_string(),
            content: ChatMessageContent::Text(p),
            tool_calls: None,
            tool_call_id: None,
            name: None,
        })
        .collect();

    Ok(messages)
}

/// Handle non-streaming completion
async fn handle_non_streaming(
    state: AppState,
    auth: AuthContext,
    request: CompletionRequest,
    provider_request: ProviderCompletionRequest,
) -> ApiResult<Response> {
    let generation_id = format!("gen-{}", Uuid::new_v4());
    let started_at = Instant::now();
    let created_at = Utc::now();

    // Call router to get completion
    let response = match state
        .router
        .complete(&auth.api_key_id, provider_request)
        .await
    {
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
                "unknown",
                &strategy_id,
                latency,
            );

            // Log to access log (persistent storage)
            if let Err(log_err) = state.access_logger.log_failure(
                &auth.api_key_id,
                "unknown",
                "unknown",
                latency,
                &generation_id,
                502, // Bad Gateway
            ) {
                tracing::warn!("Failed to write access log: {}", log_err);
            }

            return Err(ApiErrorResponse::bad_gateway(format!(
                "Provider error: {}",
                e
            )));
        }
    };

    let completed_at = Instant::now();

    // Calculate cost from router (get pricing info)
    let pricing = match state.provider_registry.get_provider(&response.provider) {
        Some(p) => p.get_pricing(&response.model).await.ok(),
        None => None,
    }
    .unwrap_or_else(crate::providers::PricingInfo::free);

    let cost = {
        let input_cost = (response.usage.prompt_tokens as f64 / 1000.0) * pricing.input_cost_per_1k;
        let output_cost =
            (response.usage.completion_tokens as f64 / 1000.0) * pricing.output_cost_per_1k;
        input_cost + output_cost
    };

    // Get client's strategy_id for metrics
    let strategy_id = state
        .client_manager
        .get_client(&auth.api_key_id)
        .map(|c| c.strategy_id.clone())
        .unwrap_or_else(|| "default".to_string());

    // Record success metrics for all five tiers
    let latency_ms = completed_at.duration_since(started_at).as_millis() as u64;
    state
        .metrics_collector
        .record_success(&crate::monitoring::metrics::RequestMetrics {
            api_key_name: &auth.api_key_id,
            provider: &response.provider,
            model: &response.model,
            strategy_id: &strategy_id,
            input_tokens: response.usage.prompt_tokens as u64,
            output_tokens: response.usage.completion_tokens as u64,
            cost_usd: cost,
            latency_ms,
        });

    // Record tokens for tray graph (real-time tracking for Fast/Medium modes)
    if let Some(ref tray_graph) = *state.tray_graph_manager.read() {
        tray_graph.record_tokens(
            (response.usage.prompt_tokens + response.usage.completion_tokens) as u64,
        );
    }

    // Log to access log (persistent storage)
    if let Err(e) = state.access_logger.log_success(
        &auth.api_key_id,
        &response.provider,
        &response.model,
        response.usage.prompt_tokens as u64,
        response.usage.completion_tokens as u64,
        cost,
        latency_ms,
        &generation_id,
    ) {
        tracing::warn!("Failed to write access log: {}", e);
    }

    // Convert chat completion response to legacy completion response
    let api_response = CompletionResponse {
        id: generation_id.clone(),
        object: "text_completion".to_string(),
        created: created_at.timestamp(),
        model: response.model.clone(),
        choices: response
            .choices
            .into_iter()
            .map(|choice| CompletionChoice {
                text: choice.message.content.as_text(),
                index: choice.index,
                finish_reason: choice.finish_reason,
                logprobs: None,
            })
            .collect(),
        usage: TokenUsage {
            prompt_tokens: response.usage.prompt_tokens,
            completion_tokens: response.usage.completion_tokens,
            total_tokens: response.usage.total_tokens,
            prompt_tokens_details: response.usage.prompt_tokens_details,
            completion_tokens_details: response.usage.completion_tokens_details,
        },
    };

    // Track generation details
    let generation_details = GenerationDetails {
        id: generation_id,
        model: response.model.clone(),
        provider: response.provider.clone(), // Use actual provider, not "router"
        created_at,
        finish_reason: api_response
            .choices
            .first()
            .and_then(|c| c.finish_reason.clone())
            .unwrap_or_else(|| "unknown".to_string()),
        tokens: api_response.usage.clone(),
        cost: Some(crate::server::types::CostDetails {
            prompt_cost: (response.usage.prompt_tokens as f64 / 1000.0) * pricing.input_cost_per_1k,
            completion_cost: (response.usage.completion_tokens as f64 / 1000.0)
                * pricing.output_cost_per_1k,
            total_cost: cost,
            currency: "USD".to_string(),
        }),
        started_at,
        completed_at,
        provider_health: None,
        api_key_id: auth.api_key_id,
        user: request.user,
        stream: false,
    };

    state
        .generation_tracker
        .record(generation_details.id.clone(), generation_details);

    Ok(Json(api_response).into_response())
}

/// Validate that the client has access to the requested LLM provider
///
/// This enforces the allowed_llm_providers access control list for clients.
/// Returns 403 Forbidden if the client doesn't have access to the provider.
async fn validate_client_provider_access(
    state: &AppState,
    client_context: Option<&ClientAuthContext>,
    request: &CompletionRequest,
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

/// Handle streaming completion
async fn handle_streaming(
    state: AppState,
    auth: AuthContext,
    request: CompletionRequest,
    provider_request: ProviderCompletionRequest,
) -> ApiResult<Response> {
    let generation_id = format!("gen-{}", Uuid::new_v4());
    let created_at = Utc::now();
    let started_at = Instant::now();

    // Clone model before moving provider_request
    let model = provider_request.model.clone();

    // Call router to get streaming completion
    let stream = match state
        .router
        .stream_complete(&auth.api_key_id, provider_request)
        .await
    {
        Ok(s) => s,
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
                &model,
                &strategy_id,
                latency,
            );

            // Log to access log (persistent storage)
            if let Err(log_err) = state.access_logger.log_failure(
                &auth.api_key_id,
                "unknown",
                &model,
                latency,
                &generation_id,
                502, // Bad Gateway
            ) {
                tracing::warn!("Failed to write access log: {}", log_err);
            }

            return Err(ApiErrorResponse::bad_gateway(format!(
                "Provider error: {}",
                e
            )));
        }
    };

    // Convert provider stream to SSE stream
    let created_timestamp = created_at.timestamp();
    let gen_id = generation_id.clone();

    // Track token usage across stream
    use parking_lot::Mutex;
    use std::sync::Arc;
    let content_accumulator = Arc::new(Mutex::new(String::new())); // Track completion content
    let finish_reason = Arc::new(Mutex::new(String::from("stop")));

    // Use a oneshot channel to signal stream completion instead of fixed delay
    let (completion_tx, completion_rx) = tokio::sync::oneshot::channel::<()>();
    let completion_tx = Arc::new(Mutex::new(Some(completion_tx)));

    // Clone for the stream.map closure
    let content_accumulator_map = content_accumulator.clone();
    let finish_reason_map = finish_reason.clone();
    let completion_tx_map = completion_tx.clone();

    // Clone for tracking after stream completes
    let state_clone = state.clone();
    let auth_clone = auth.clone();
    let gen_id_clone = generation_id.clone();
    let model_clone = model.clone();
    let created_at_clone = created_at;
    let request_user = request.user.clone();
    let request_prompt = request.prompt.clone();

    let sse_stream = stream.map(
        move |chunk_result| -> Result<Event, std::convert::Infallible> {
            match chunk_result {
                Ok(provider_chunk) => {
                    // Track content for token estimation
                    let is_done = if let Some(choice) = provider_chunk.choices.first() {
                        if let Some(content) = &choice.delta.content {
                            content_accumulator_map.lock().push_str(content);
                        }

                        // Track finish reason and check if stream is done
                        if let Some(reason) = &choice.finish_reason {
                            *finish_reason_map.lock() = reason.clone();
                            true
                        } else {
                            false
                        }
                    } else {
                        false
                    };

                    // Signal completion when stream is done
                    if is_done {
                        if let Some(tx) = completion_tx_map.lock().take() {
                            let _ = tx.send(());
                        }
                    }

                    // Convert chat completion chunk to legacy completion chunk
                    let api_chunk = CompletionChunk {
                        id: gen_id.clone(),
                        object: "text_completion".to_string(),
                        created: created_timestamp,
                        choices: provider_chunk
                            .choices
                            .into_iter()
                            .map(|choice| CompletionChunkChoice {
                                text: choice.delta.content.unwrap_or_default(),
                                index: choice.index,
                                finish_reason: choice.finish_reason,
                            })
                            .collect(),
                    };

                    let json = serde_json::to_string(&api_chunk).unwrap_or_default();
                    Ok(Event::default().data(json))
                }
                Err(e) => {
                    tracing::error!("Error in streaming: {}", e);
                    // Signal completion on error as well
                    if let Some(tx) = completion_tx_map.lock().take() {
                        let _ = tx.send(());
                    }
                    // Return error in SSE format with actual error message
                    let error_response = serde_json::json!({
                        "error": {
                            "message": format!("Streaming error: {}", e),
                            "type": "server_error",
                            "code": "streaming_error"
                        }
                    });
                    Ok(Event::default().data(
                        serde_json::to_string(&error_response)
                            .unwrap_or_else(|_| "[ERROR]".to_string()),
                    ))
                }
            }
        },
    );

    // Record generation details after stream completes
    tokio::spawn(async move {
        // Wait for stream completion signal with a timeout fallback
        let _ = tokio::time::timeout(
            tokio::time::Duration::from_secs(300), // 5 minute timeout for long completions
            completion_rx,
        )
        .await;

        let completed_at = Instant::now();
        let completion_content = content_accumulator.lock().clone();
        let finish_reason_final = finish_reason.lock().clone();

        // Estimate tokens (rough estimate: ~4 chars per token)
        let prompt_tokens = estimate_prompt_tokens(&request_prompt) as u32;
        let completion_tokens = (completion_content.len() / 4).max(1) as u32;
        let total_tokens = prompt_tokens + completion_tokens;

        // Infer provider from model name (format: "provider/model" or just "model")
        let provider = if let Some((p, _)) = model_clone.split_once('/') {
            p.to_string()
        } else {
            "router".to_string()
        };

        // Estimate cost (using approximation since streaming doesn't return exact counts)
        let pricing = match state_clone.provider_registry.get_provider(&provider) {
            Some(p) => p.get_pricing(&model_clone).await.ok(),
            None => None,
        }
        .unwrap_or_else(crate::providers::PricingInfo::free);

        let cost = {
            let input_cost = (prompt_tokens as f64 / 1000.0) * pricing.input_cost_per_1k;
            let output_cost = (completion_tokens as f64 / 1000.0) * pricing.output_cost_per_1k;
            input_cost + output_cost
        };

        // Get client's strategy_id for metrics
        let strategy_id = state_clone
            .client_manager
            .get_client(&auth_clone.api_key_id)
            .map(|c| c.strategy_id.clone())
            .unwrap_or_else(|| "default".to_string());

        // Record success metrics for streaming (with estimated tokens)
        let latency_ms = completed_at.duration_since(started_at).as_millis() as u64;
        state_clone
            .metrics_collector
            .record_success(&crate::monitoring::metrics::RequestMetrics {
                api_key_name: &auth_clone.api_key_id,
                provider: &provider,
                model: &model_clone,
                strategy_id: &strategy_id,
                input_tokens: prompt_tokens as u64,
                output_tokens: completion_tokens as u64,
                cost_usd: cost,
                latency_ms,
            });

        // Record tokens for tray graph
        if let Some(ref tray_graph) = *state_clone.tray_graph_manager.read() {
            tray_graph.record_tokens((prompt_tokens + completion_tokens) as u64);
        }

        // Log to access log (persistent storage)
        if let Err(e) = state_clone.access_logger.log_success(
            &auth_clone.api_key_id,
            &provider,
            &model_clone,
            prompt_tokens as u64,
            completion_tokens as u64,
            cost,
            latency_ms,
            &gen_id_clone,
        ) {
            tracing::warn!("Failed to write access log: {}", e);
        }

        // Emit event for real-time UI updates
        state_clone.emit_event(
            "metrics-updated",
            &serde_json::json!({
                "timestamp": created_at_clone.to_rfc3339(),
            })
            .to_string(),
        );

        let generation_details = GenerationDetails {
            id: gen_id_clone,
            model: model_clone,
            provider: provider.clone(),
            created_at: created_at_clone,
            finish_reason: finish_reason_final,
            tokens: TokenUsage {
                prompt_tokens,
                completion_tokens,
                total_tokens,
                prompt_tokens_details: None,
                completion_tokens_details: None,
            },
            cost: Some(crate::server::types::CostDetails {
                prompt_cost: (prompt_tokens as f64 / 1000.0) * pricing.input_cost_per_1k,
                completion_cost: (completion_tokens as f64 / 1000.0) * pricing.output_cost_per_1k,
                total_cost: cost,
                currency: "USD".to_string(),
            }),
            started_at,
            completed_at,
            provider_health: None,
            api_key_id: auth_clone.api_key_id,
            user: request_user,
            stream: true,
        };

        state_clone
            .generation_tracker
            .record(generation_details.id.clone(), generation_details);
    });

    Ok(Sse::new(sse_stream)
        .keep_alive(KeepAlive::default())
        .into_response())
}

/// Estimate token count from prompt (rough estimate)
fn estimate_prompt_tokens(prompt: &PromptInput) -> u64 {
    let text = match prompt {
        PromptInput::Single(s) => s.as_str(),
        PromptInput::Multiple(v) => {
            // Rough estimate for multiple prompts: sum of all lengths
            return v.iter().map(|s| (s.len() / 4).max(1) as u64).sum();
        }
    };

    (text.len() / 4).max(1) as u64
}
