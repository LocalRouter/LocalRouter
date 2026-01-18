//! POST /v1/chat/completions endpoint
//!
//! The primary endpoint for conversational AI interactions.

use std::time::Instant;

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
use uuid::Uuid;

use crate::providers::{ChatMessage as ProviderChatMessage, CompletionRequest as ProviderCompletionRequest};
use crate::router::UsageInfo;
use crate::server::middleware::error::{ApiErrorResponse, ApiResult};
use crate::server::middleware::client_auth::ClientAuthContext;
use crate::server::state::{AppState, AuthContext, GenerationDetails};
use crate::server::types::{
    ChatCompletionChunk, ChatCompletionChunkChoice, ChatCompletionChoice, ChatCompletionRequest,
    ChatCompletionResponse, ChatMessage, ChunkDelta, MessageContent,
    TokenUsage,
};

/// POST /v1/chat/completions
/// Send a chat completion request
#[utoipa::path(
    post,
    path = "/v1/chat/completions",
    tag = "chat",
    request_body = ChatCompletionRequest,
    responses(
        (status = 200, description = "Successful response (non-streaming)", body = ChatCompletionResponse),
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
pub async fn chat_completions(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    client_auth: Option<Extension<ClientAuthContext>>,
    Json(request): Json<ChatCompletionRequest>,
) -> ApiResult<Response> {
    // Emit LLM request event to trigger tray icon indicator
    state.emit_event("llm-request", "chat");

    // Validate request
    validate_request(&request)?;

    // Validate model access based on API key's model selection
    validate_model_access(&state, &auth, &request).await?;

    // Validate client provider access (if using client auth)
    validate_client_provider_access(&state, client_auth.as_ref().map(|e| &e.0), &request).await?;

    // Check rate limits
    check_rate_limits(&state, &auth, &request).await?;

    // Convert to provider format
    let provider_request = convert_to_provider_request(&request)?;

    if request.stream {
        // Handle streaming response
        handle_streaming(state, auth, request, provider_request).await
    } else {
        // Handle non-streaming response
        handle_non_streaming(state, auth, request, provider_request).await
    }
}

/// Validate the chat completion request
fn validate_request(request: &ChatCompletionRequest) -> ApiResult<()> {
    if request.model.is_empty() {
        return Err(ApiErrorResponse::bad_request("model is required").with_param("model"));
    }

    if request.messages.is_empty() {
        return Err(ApiErrorResponse::bad_request("messages cannot be empty").with_param("messages"));
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

    // Validate top_k (extended parameter)
    if let Some(top_k) = request.top_k {
        if top_k == 0 {
            return Err(
                ApiErrorResponse::bad_request("top_k must be greater than 0").with_param("top_k"),
            );
        }
    }

    // Validate repetition_penalty (extended parameter)
    if let Some(rep_penalty) = request.repetition_penalty {
        if !(0.0..=2.0).contains(&rep_penalty) {
            return Err(
                ApiErrorResponse::bad_request("repetition_penalty must be between 0 and 2")
                    .with_param("repetition_penalty"),
            );
        }
    }

    // Validate response_format if present
    if let Some(ref format) = request.response_format {
        match format {
            crate::server::types::ResponseFormat::JsonObject { r#type } => {
                if r#type != "json_object" {
                    return Err(
                        ApiErrorResponse::bad_request("response_format type must be 'json_object'")
                            .with_param("response_format"),
                    );
                }
            }
            crate::server::types::ResponseFormat::JsonSchema { r#type, schema } => {
                if r#type != "json_schema" {
                    return Err(
                        ApiErrorResponse::bad_request("response_format type must be 'json_schema'")
                            .with_param("response_format"),
                    );
                }
                // Basic validation that schema is an object
                if !schema.is_object() {
                    return Err(
                        ApiErrorResponse::bad_request("response_format schema must be a JSON object")
                            .with_param("response_format"),
                    );
                }
            }
        }
    }

    Ok(())
}

/// Validate that the requested model is allowed by the API key's model selection
async fn validate_model_access(
    state: &AppState,
    auth: &AuthContext,
    request: &ChatCompletionRequest,
) -> ApiResult<()> {
    // If no model selection is configured, allow all models
    let Some(ref selection) = auth.model_selection else {
        return Ok(());
    };

    // Parse the model string to extract provider and model ID
    // The model can be in format "provider/model" or just "model"
    if let Some((provider, model_id)) = request.model.split_once('/') {
        // Provider specified in request
        if !selection.is_model_allowed(provider, model_id) {
            return Err(ApiErrorResponse::forbidden(format!(
                "Model '{}' is not accessible with this API key. Check your API key's model selection settings.",
                request.model
            ))
            .with_param("model"));
        }
    } else {
        // No provider specified - need to find which provider has this model
        let all_models = state
            .provider_registry
            .list_all_models()
            .await
            .map_err(|e| ApiErrorResponse::internal_error(format!("Failed to list models: {}", e)))?;

        let matching_model = all_models
            .iter()
            .find(|m| m.id.eq_ignore_ascii_case(&request.model))
            .ok_or_else(|| {
                ApiErrorResponse::bad_request(format!("Model not found: {}", request.model))
                    .with_param("model")
            })?;

        // Check if allowed
        if !selection.is_model_allowed(&matching_model.provider, &matching_model.id) {
            return Err(ApiErrorResponse::forbidden(format!(
                "Model '{}' is not accessible with this API key. Check your API key's model selection settings.",
                request.model
            ))
            .with_param("model"));
        }
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
    request: &ChatCompletionRequest,
) -> ApiResult<()> {
    // If no client context, skip validation (using API key auth)
    let Some(client_ctx) = client_context else {
        return Ok(());
    };

    // Get the client to check allowed providers
    let client = state
        .client_manager
        .get_client(&client_ctx.client_id)
        .ok_or_else(|| ApiErrorResponse::unauthorized("Client not found"))?;

    // If client is disabled, deny access
    if !client.enabled {
        return Err(ApiErrorResponse::forbidden("Client is disabled"));
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
            .map_err(|e| ApiErrorResponse::internal_error(format!("Failed to list models: {}", e)))?;

        let matching_model = all_models
            .iter()
            .find(|m| m.id.eq_ignore_ascii_case(&request.model))
            .ok_or_else(|| {
                ApiErrorResponse::bad_request(format!("Model not found: {}", request.model))
                    .with_param("model")
            })?;

        matching_model.provider.clone()
    };

    // Check if provider is in allowed list
    if !client.allowed_llm_providers.contains(&provider) {
        tracing::warn!(
            "Client {} attempted to access unauthorized LLM provider: {}",
            client.client_id,
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
        client.client_id,
        provider
    );

    Ok(())
}

/// Check rate limits before processing request
async fn check_rate_limits(
    state: &AppState,
    auth: &AuthContext,
    request: &ChatCompletionRequest,
) -> ApiResult<()> {
    // Estimate usage for rate limit check (rough estimate)
    let estimated_tokens = estimate_token_count(&request.messages);
    let usage_estimate = UsageInfo {
        input_tokens: estimated_tokens,
        output_tokens: request.max_tokens.unwrap_or(100) as u64,
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
            error.error = error.error.with_code(format!("retry_after_{}", retry_after));
        }

        return Err(error);
    }

    Ok(())
}

/// Convert API request to provider request format
fn convert_to_provider_request(
    request: &ChatCompletionRequest,
) -> ApiResult<ProviderCompletionRequest> {
    let messages = request
        .messages
        .iter()
        .map(|msg| {
            let content = match &msg.content {
                Some(MessageContent::Text(text)) => text.clone(),
                Some(MessageContent::Parts(_)) => {
                    // For now, extract text from parts
                    // Full multimodal support would require more complex handling
                    return Err(ApiErrorResponse::bad_request(
                        "Multimodal content not yet fully supported",
                    ));
                }
                None => String::new(),
            };

            Ok(ProviderChatMessage {
                role: msg.role.clone(),
                content,
            })
        })
        .collect::<ApiResult<Vec<_>>>()?;

    Ok(ProviderCompletionRequest {
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
        // Extended parameters
        top_k: request.top_k,
        seed: request.seed,
        repetition_penalty: request.repetition_penalty,
        extensions: request.extensions.clone(),
    })
}

/// Handle non-streaming chat completion
async fn handle_non_streaming(
    state: AppState,
    auth: AuthContext,
    request: ChatCompletionRequest,
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
            state.metrics_collector.record_failure(
                &auth.api_key_id,
                "unknown",
                "unknown",
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

            return Err(ApiErrorResponse::bad_gateway(format!("Provider error: {}", e)));
        }
    };

    let completed_at = Instant::now();

    // Calculate cost from router (get pricing info)
    let pricing = match state.provider_registry.get_provider(&response.provider) {
        Some(p) => p.get_pricing(&response.model).await.ok(),
        None => None,
    }.unwrap_or_else(crate::providers::PricingInfo::free);

    let cost = {
        let input_cost = (response.usage.prompt_tokens as f64 / 1000.0) * pricing.input_cost_per_1k;
        let output_cost = (response.usage.completion_tokens as f64 / 1000.0) * pricing.output_cost_per_1k;
        input_cost + output_cost
    };

    // Record success metrics for all four tiers
    let latency_ms = completed_at.duration_since(started_at).as_millis() as u64;
    state.metrics_collector.record_success(&crate::monitoring::metrics::RequestMetrics {
        api_key_name: &auth.api_key_id,
        provider: &response.provider,
        model: &response.model,
        input_tokens: response.usage.prompt_tokens as u64,
        output_tokens: response.usage.completion_tokens as u64,
        cost_usd: cost,
        latency_ms,
    });

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

    // Emit event for real-time UI updates
    state.emit_event("metrics-updated", &serde_json::json!({
        "timestamp": created_at.to_rfc3339(),
    }).to_string());

    // Note: Router already records usage for rate limiting, so we don't need to do it here

    // Convert provider response to API response
    let api_response = ChatCompletionResponse {
        id: generation_id.clone(),
        object: "chat.completion".to_string(),
        created: created_at.timestamp(),
        model: response.model.clone(),
        choices: response
            .choices
            .into_iter()
            .map(|choice| ChatCompletionChoice {
                index: choice.index,
                message: ChatMessage {
                    role: choice.message.role,
                    content: Some(MessageContent::Text(choice.message.content)),
                    name: None,
                },
                finish_reason: choice.finish_reason,
            })
            .collect(),
        usage: TokenUsage {
            prompt_tokens: response.usage.prompt_tokens,
            completion_tokens: response.usage.completion_tokens,
            total_tokens: response.usage.total_tokens,
            prompt_tokens_details: response.usage.prompt_tokens_details,
            completion_tokens_details: response.usage.completion_tokens_details,
        },
        extensions: None, // Provider-specific extensions (Phase 1)
    };

    // Track generation details
    let generation_details = GenerationDetails {
        id: generation_id,
        model: response.model.clone(),
        provider: response.provider.clone(),
        created_at,
        finish_reason: api_response.choices.first().and_then(|c| c.finish_reason.clone()).unwrap_or_else(|| "unknown".to_string()),
        tokens: api_response.usage.clone(),
        cost: Some(crate::server::types::CostDetails {
            prompt_cost: (response.usage.prompt_tokens as f64 / 1000.0) * pricing.input_cost_per_1k,
            completion_cost: (response.usage.completion_tokens as f64 / 1000.0) * pricing.output_cost_per_1k,
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

/// Handle streaming chat completion
async fn handle_streaming(
    state: AppState,
    auth: AuthContext,
    request: ChatCompletionRequest,
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
            state.metrics_collector.record_failure(
                &auth.api_key_id,
                "unknown",
                &model,
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

            return Err(ApiErrorResponse::bad_gateway(format!("Provider error: {}", e)));
        }
    };

    // Convert provider stream to SSE stream
    let created_timestamp = created_at.timestamp();
    let gen_id = generation_id.clone();

    // Track token usage across stream
    use std::sync::Arc;
    use parking_lot::Mutex;
    let content_accumulator = Arc::new(Mutex::new(String::new())); // Track completion content
    let finish_reason = Arc::new(Mutex::new(String::from("stop")));

    // Clone for the stream.map closure
    let content_accumulator_map = content_accumulator.clone();
    let finish_reason_map = finish_reason.clone();

    // Clone for tracking after stream completes
    let state_clone = state.clone();
    let auth_clone = auth.clone();
    let gen_id_clone = generation_id.clone();
    let model_clone = model.clone();
    let created_at_clone = created_at;
    let request_user = request.user.clone();
    let request_messages = request.messages.clone();

    let sse_stream = stream.map(move |chunk_result| -> Result<Event, std::convert::Infallible> {
        match chunk_result {
            Ok(provider_chunk) => {
                // Track content for token estimation
                if let Some(choice) = provider_chunk.choices.first() {
                    if let Some(content) = &choice.delta.content {
                        content_accumulator_map.lock().push_str(content);
                    }

                    // Track finish reason
                    if let Some(reason) = &choice.finish_reason {
                        *finish_reason_map.lock() = reason.clone();
                    }
                }

                let api_chunk = ChatCompletionChunk {
                    id: gen_id.clone(),
                    object: "chat.completion.chunk".to_string(),
                    created: created_timestamp,
                    model: model.clone(),
                    choices: provider_chunk
                        .choices
                        .into_iter()
                        .map(|choice| ChatCompletionChunkChoice {
                            index: choice.index,
                            delta: ChunkDelta {
                                role: choice.delta.role,
                                content: choice.delta.content,
                            },
                            finish_reason: choice.finish_reason,
                        })
                        .collect(),
                    usage: None, // Not available in streaming chunks
                };

                let json = serde_json::to_string(&api_chunk).unwrap_or_default();
                Ok(Event::default().data(json))
            }
            Err(e) => {
                tracing::error!("Error in streaming: {}", e);
                Ok(Event::default().data("[ERROR]"))
            }
        }
    });

    // Record generation details after stream completes
    tokio::spawn(async move {
        // Wait a bit to ensure stream has completed and tokens were accumulated
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

        let completed_at = Instant::now();
        let completion_content = content_accumulator.lock().clone();
        let finish_reason_final = finish_reason.lock().clone();

        // Estimate tokens (rough estimate: ~4 chars per token)
        let prompt_tokens = estimate_token_count(&request_messages) as u32;
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
        }.unwrap_or_else(crate::providers::PricingInfo::free);

        let cost = {
            let input_cost = (prompt_tokens as f64 / 1000.0) * pricing.input_cost_per_1k;
            let output_cost = (completion_tokens as f64 / 1000.0) * pricing.output_cost_per_1k;
            input_cost + output_cost
        };

        // Record success metrics for streaming (with estimated tokens)
        let latency_ms = completed_at.duration_since(started_at).as_millis() as u64;
        state_clone.metrics_collector.record_success(&crate::monitoring::metrics::RequestMetrics {
            api_key_name: &auth_clone.api_key_id,
            provider: &provider,
            model: &model_clone,
            input_tokens: prompt_tokens as u64,
            output_tokens: completion_tokens as u64,
            cost_usd: cost,
            latency_ms,
        });

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
        state_clone.emit_event("metrics-updated", &serde_json::json!({
            "timestamp": created_at_clone.to_rfc3339(),
        }).to_string());

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

/// Estimate token count from messages (rough estimate)
fn estimate_token_count(messages: &[ChatMessage]) -> u64 {
    messages
        .iter()
        .map(|msg| {
            match &msg.content {
                Some(MessageContent::Text(text)) => {
                    // Rough estimate: ~4 chars per token
                    (text.len() / 4).max(1) as u64
                }
                Some(MessageContent::Parts(parts)) => {
                    parts.len() as u64 * 100 // Very rough estimate
                }
                None => 0,
            }
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::types::ResponseFormat;
    use serde_json::json;

    #[test]
    fn test_validate_request_basic() {
        let request = ChatCompletionRequest {
            model: "gpt-4".to_string(),
            messages: vec![ChatMessage {
                role: "user".to_string(),
                content: Some(MessageContent::Text("Hello".to_string())),
                name: None,
            }],
            temperature: None,
            max_tokens: None,
            stream: false,
            top_p: None,
            frequency_penalty: None,
            presence_penalty: None,
            stop: None,
            top_k: None,
            seed: None,
            repetition_penalty: None,
            response_format: None,
            tools: None,
            tool_choice: None,
            extensions: None,
            user: None,
        };

        assert!(validate_request(&request).is_ok());
    }

    #[test]
    fn test_validate_request_empty_model() {
        let request = ChatCompletionRequest {
            model: "".to_string(),
            messages: vec![ChatMessage {
                role: "user".to_string(),
                content: Some(MessageContent::Text("Hello".to_string())),
                name: None,
            }],
            temperature: None,
            max_tokens: None,
            stream: false,
            top_p: None,
            frequency_penalty: None,
            presence_penalty: None,
            stop: None,
            top_k: None,
            seed: None,
            repetition_penalty: None,
            response_format: None,
            tools: None,
            tool_choice: None,
            extensions: None,
            user: None,
        };

        assert!(validate_request(&request).is_err());
    }

    #[test]
    fn test_validate_request_empty_messages() {
        let request = ChatCompletionRequest {
            model: "gpt-4".to_string(),
            messages: vec![],
            temperature: None,
            max_tokens: None,
            stream: false,
            top_p: None,
            frequency_penalty: None,
            presence_penalty: None,
            stop: None,
            top_k: None,
            seed: None,
            repetition_penalty: None,
            response_format: None,
            tools: None,
            tool_choice: None,
            extensions: None,
            user: None,
        };

        assert!(validate_request(&request).is_err());
    }

    #[test]
    fn test_validate_temperature_valid() {
        let request = ChatCompletionRequest {
            model: "gpt-4".to_string(),
            messages: vec![ChatMessage {
                role: "user".to_string(),
                content: Some(MessageContent::Text("Hello".to_string())),
                name: None,
            }],
            temperature: Some(0.7),
            max_tokens: None,
            stream: false,
            top_p: None,
            frequency_penalty: None,
            presence_penalty: None,
            stop: None,
            top_k: None,
            seed: None,
            repetition_penalty: None,
            response_format: None,
            tools: None,
            tool_choice: None,
            extensions: None,
            user: None,
        };

        assert!(validate_request(&request).is_ok());
    }

    #[test]
    fn test_validate_temperature_invalid_high() {
        let request = ChatCompletionRequest {
            model: "gpt-4".to_string(),
            messages: vec![ChatMessage {
                role: "user".to_string(),
                content: Some(MessageContent::Text("Hello".to_string())),
                name: None,
            }],
            temperature: Some(2.5),
            max_tokens: None,
            stream: false,
            top_p: None,
            frequency_penalty: None,
            presence_penalty: None,
            stop: None,
            top_k: None,
            seed: None,
            repetition_penalty: None,
            response_format: None,
            tools: None,
            tool_choice: None,
            extensions: None,
            user: None,
        };

        assert!(validate_request(&request).is_err());
    }

    #[test]
    fn test_validate_top_k_valid() {
        let request = ChatCompletionRequest {
            model: "gpt-4".to_string(),
            messages: vec![ChatMessage {
                role: "user".to_string(),
                content: Some(MessageContent::Text("Hello".to_string())),
                name: None,
            }],
            temperature: None,
            max_tokens: None,
            stream: false,
            top_p: None,
            frequency_penalty: None,
            presence_penalty: None,
            stop: None,
            top_k: Some(40),
            seed: None,
            repetition_penalty: None,
            response_format: None,
            tools: None,
            tool_choice: None,
            extensions: None,
            user: None,
        };

        assert!(validate_request(&request).is_ok());
    }

    #[test]
    fn test_validate_top_k_invalid_zero() {
        let request = ChatCompletionRequest {
            model: "gpt-4".to_string(),
            messages: vec![ChatMessage {
                role: "user".to_string(),
                content: Some(MessageContent::Text("Hello".to_string())),
                name: None,
            }],
            temperature: None,
            max_tokens: None,
            stream: false,
            top_p: None,
            frequency_penalty: None,
            presence_penalty: None,
            stop: None,
            top_k: Some(0),
            seed: None,
            repetition_penalty: None,
            response_format: None,
            tools: None,
            tool_choice: None,
            extensions: None,
            user: None,
        };

        assert!(validate_request(&request).is_err());
    }

    #[test]
    fn test_validate_repetition_penalty_valid() {
        let request = ChatCompletionRequest {
            model: "gpt-4".to_string(),
            messages: vec![ChatMessage {
                role: "user".to_string(),
                content: Some(MessageContent::Text("Hello".to_string())),
                name: None,
            }],
            temperature: None,
            max_tokens: None,
            stream: false,
            top_p: None,
            frequency_penalty: None,
            presence_penalty: None,
            stop: None,
            top_k: None,
            seed: None,
            repetition_penalty: Some(1.1),
            response_format: None,
            tools: None,
            tool_choice: None,
            extensions: None,
            user: None,
        };

        assert!(validate_request(&request).is_ok());
    }

    #[test]
    fn test_validate_repetition_penalty_invalid() {
        let request = ChatCompletionRequest {
            model: "gpt-4".to_string(),
            messages: vec![ChatMessage {
                role: "user".to_string(),
                content: Some(MessageContent::Text("Hello".to_string())),
                name: None,
            }],
            temperature: None,
            max_tokens: None,
            stream: false,
            top_p: None,
            frequency_penalty: None,
            presence_penalty: None,
            stop: None,
            top_k: None,
            seed: None,
            repetition_penalty: Some(2.5),
            response_format: None,
            tools: None,
            tool_choice: None,
            extensions: None,
            user: None,
        };

        assert!(validate_request(&request).is_err());
    }

    #[test]
    fn test_validate_response_format_json_object_valid() {
        let request = ChatCompletionRequest {
            model: "gpt-4".to_string(),
            messages: vec![ChatMessage {
                role: "user".to_string(),
                content: Some(MessageContent::Text("Hello".to_string())),
                name: None,
            }],
            temperature: None,
            max_tokens: None,
            stream: false,
            top_p: None,
            frequency_penalty: None,
            presence_penalty: None,
            stop: None,
            top_k: None,
            seed: None,
            repetition_penalty: None,
            response_format: Some(ResponseFormat::JsonObject {
                r#type: "json_object".to_string(),
            }),
            tools: None,
            tool_choice: None,
            extensions: None,
            user: None,
        };

        assert!(validate_request(&request).is_ok());
    }

    #[test]
    fn test_validate_response_format_json_object_invalid_type() {
        let request = ChatCompletionRequest {
            model: "gpt-4".to_string(),
            messages: vec![ChatMessage {
                role: "user".to_string(),
                content: Some(MessageContent::Text("Hello".to_string())),
                name: None,
            }],
            temperature: None,
            max_tokens: None,
            stream: false,
            top_p: None,
            frequency_penalty: None,
            presence_penalty: None,
            stop: None,
            top_k: None,
            seed: None,
            repetition_penalty: None,
            response_format: Some(ResponseFormat::JsonObject {
                r#type: "invalid_type".to_string(),
            }),
            tools: None,
            tool_choice: None,
            extensions: None,
            user: None,
        };

        assert!(validate_request(&request).is_err());
    }

    #[test]
    fn test_validate_response_format_json_schema_valid() {
        let request = ChatCompletionRequest {
            model: "gpt-4".to_string(),
            messages: vec![ChatMessage {
                role: "user".to_string(),
                content: Some(MessageContent::Text("Hello".to_string())),
                name: None,
            }],
            temperature: None,
            max_tokens: None,
            stream: false,
            top_p: None,
            frequency_penalty: None,
            presence_penalty: None,
            stop: None,
            top_k: None,
            seed: None,
            repetition_penalty: None,
            response_format: Some(ResponseFormat::JsonSchema {
                r#type: "json_schema".to_string(),
                schema: json!({
                    "type": "object",
                    "properties": {
                        "name": {"type": "string"}
                    }
                }),
            }),
            tools: None,
            tool_choice: None,
            extensions: None,
            user: None,
        };

        assert!(validate_request(&request).is_ok());
    }

    #[test]
    fn test_validate_response_format_json_schema_invalid_schema() {
        let request = ChatCompletionRequest {
            model: "gpt-4".to_string(),
            messages: vec![ChatMessage {
                role: "user".to_string(),
                content: Some(MessageContent::Text("Hello".to_string())),
                name: None,
            }],
            temperature: None,
            max_tokens: None,
            stream: false,
            top_p: None,
            frequency_penalty: None,
            presence_penalty: None,
            stop: None,
            top_k: None,
            seed: None,
            repetition_penalty: None,
            response_format: Some(ResponseFormat::JsonSchema {
                r#type: "json_schema".to_string(),
                schema: json!("not an object"),
            }),
            tools: None,
            tool_choice: None,
            extensions: None,
            user: None,
        };

        assert!(validate_request(&request).is_err());
    }

    #[test]
    fn test_convert_to_provider_request_with_extended_params() {
        let request = ChatCompletionRequest {
            model: "gpt-4".to_string(),
            messages: vec![ChatMessage {
                role: "user".to_string(),
                content: Some(MessageContent::Text("Hello".to_string())),
                name: None,
            }],
            temperature: Some(0.7),
            max_tokens: Some(100),
            stream: false,
            top_p: Some(0.9),
            frequency_penalty: Some(0.5),
            presence_penalty: Some(0.3),
            stop: None,
            top_k: Some(40),
            seed: Some(12345),
            repetition_penalty: Some(1.1),
            response_format: None,
            tools: None,
            tool_choice: None,
            extensions: None,
            user: None,
        };

        let result = convert_to_provider_request(&request);
        assert!(result.is_ok());

        let provider_request = result.unwrap();
        assert_eq!(provider_request.model, "gpt-4");
        assert_eq!(provider_request.temperature, Some(0.7));
        assert_eq!(provider_request.top_k, Some(40));
        assert_eq!(provider_request.seed, Some(12345));
        assert_eq!(provider_request.repetition_penalty, Some(1.1));
    }

    #[test]
    fn test_estimate_token_count() {
        let messages = vec![
            ChatMessage {
                role: "user".to_string(),
                content: Some(MessageContent::Text("Hello, how are you?".to_string())), // ~20 chars = 5 tokens
                name: None,
            },
            ChatMessage {
                role: "assistant".to_string(),
                content: Some(MessageContent::Text("I'm doing well!".to_string())), // ~15 chars = 3-4 tokens
                name: None,
            },
        ];

        let count = estimate_token_count(&messages);
        assert!(count > 0);
        assert!(count < 100); // Should be reasonable
    }
}
