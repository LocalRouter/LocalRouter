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

use super::helpers::get_enabled_client_from_manager;
use crate::providers::{
    ChatMessage as ProviderChatMessage, ChatMessageContent as ProviderMessageContent,
    CompletionRequest as ProviderCompletionRequest, ContentPart as ProviderContentPart,
    ImageUrl as ProviderImageUrl,
};
use crate::router::UsageInfo;
use crate::server::middleware::client_auth::ClientAuthContext;
use crate::server::middleware::error::{ApiErrorResponse, ApiResult};
use crate::server::state::{AppState, AuthContext, GenerationDetails};
use crate::server::types::{
    ChatCompletionChoice, ChatCompletionChunk, ChatCompletionChunkChoice, ChatCompletionLogprobs,
    ChatCompletionRequest, ChatCompletionResponse, ChatCompletionTokenLogprob, ChatMessage,
    ChunkDelta, MessageContent, TokenUsage, TopLogprob,
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
        return Err(
            ApiErrorResponse::bad_request("messages cannot be empty").with_param("messages")
        );
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

    // Validate top_k (LocalRouter extension, not in OpenAI API)
    if let Some(top_k) = request.top_k {
        if top_k == 0 {
            return Err(
                ApiErrorResponse::bad_request("top_k must be greater than 0").with_param("top_k"),
            );
        }
    }

    // Validate repetition_penalty (LocalRouter extension, not in OpenAI API)
    // Range: 0.0-2.0 (LocalRouter-specific constraint)
    if let Some(rep_penalty) = request.repetition_penalty {
        if !(0.0..=2.0).contains(&rep_penalty) {
            return Err(ApiErrorResponse::bad_request(
                "repetition_penalty must be between 0 and 2",
            )
            .with_param("repetition_penalty"));
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
        if n > 1 {
            // Note: Currently n > 1 is accepted but only the first completion will be generated
            // This is a limitation that will be fixed in a future update
            tracing::warn!("n > 1 requested but only first completion will be generated (not yet fully supported)");
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

    // Validate top_logprobs (requires logprobs to be true)
    if let Some(top_logprobs) = request.top_logprobs {
        if request.logprobs != Some(true) {
            return Err(
                ApiErrorResponse::bad_request("top_logprobs requires logprobs to be true")
                    .with_param("top_logprobs"),
            );
        }
        if top_logprobs > 20 {
            return Err(
                ApiErrorResponse::bad_request("top_logprobs must be between 0 and 20")
                    .with_param("top_logprobs"),
            );
        }
    }

    // Validate max_tokens and max_completion_tokens are not both set
    if request.max_tokens.is_some() && request.max_completion_tokens.is_some() {
        return Err(ApiErrorResponse::bad_request(
            "Cannot specify both max_tokens and max_completion_tokens",
        ));
    }

    // Validate response_format if present
    if let Some(ref format) = request.response_format {
        match format {
            crate::server::types::ResponseFormat::JsonObject { r#type } => {
                if r#type != "json_object" {
                    return Err(ApiErrorResponse::bad_request(
                        "response_format type must be 'json_object'",
                    )
                    .with_param("response_format"));
                }
            }
            crate::server::types::ResponseFormat::JsonSchema { r#type, schema } => {
                if r#type != "json_schema" {
                    return Err(ApiErrorResponse::bad_request(
                        "response_format type must be 'json_schema'",
                    )
                    .with_param("response_format"));
                }
                // Basic validation that schema is an object
                if !schema.is_object() {
                    return Err(ApiErrorResponse::bad_request(
                        "response_format schema must be a JSON object",
                    )
                    .with_param("response_format"));
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

/// Check rate limits before processing request
async fn check_rate_limits(
    state: &AppState,
    auth: &AuthContext,
    request: &ChatCompletionRequest,
) -> ApiResult<()> {
    // Estimate usage for rate limit check (rough estimate)
    let estimated_tokens = estimate_token_count(&request.messages);
    let max_output_tokens = request
        .max_completion_tokens
        .or(request.max_tokens)
        .unwrap_or(100);
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

/// Convert API request to provider request format
fn convert_to_provider_request(
    request: &ChatCompletionRequest,
) -> ApiResult<ProviderCompletionRequest> {
    let messages = request
        .messages
        .iter()
        .map(|msg| {
            let content = match &msg.content {
                Some(MessageContent::Text(text)) => ProviderMessageContent::Text(text.clone()),
                Some(MessageContent::Parts(parts)) => {
                    // Convert server content parts to provider content parts
                    let provider_parts: Vec<ProviderContentPart> = parts
                        .iter()
                        .map(|part| match part {
                            crate::server::types::ContentPart::Text { text } => {
                                ProviderContentPart::Text { text: text.clone() }
                            }
                            crate::server::types::ContentPart::ImageUrl { image_url } => {
                                ProviderContentPart::ImageUrl {
                                    image_url: ProviderImageUrl {
                                        url: image_url.url.clone(),
                                        detail: image_url.detail.clone(),
                                    },
                                }
                            }
                        })
                        .collect();
                    ProviderMessageContent::Parts(provider_parts)
                }
                None => ProviderMessageContent::Text(String::new()),
            };

            Ok(ProviderChatMessage {
                role: msg.role.clone(),
                content,
                tool_calls: None,   // Input messages don't have tool_calls initially
                tool_call_id: None, // Only for tool role messages
                name: msg.name.clone(),
            })
        })
        .collect::<ApiResult<Vec<_>>>()?;

    // Prefer max_completion_tokens over max_tokens (for o-series models)
    let max_tokens = request.max_completion_tokens.or(request.max_tokens);

    // Convert tools from server types to provider types
    let tools = request.tools.as_ref().map(|server_tools| {
        server_tools
            .iter()
            .map(|tool| crate::providers::Tool {
                tool_type: tool.tool_type.clone(),
                function: crate::providers::FunctionDefinition {
                    name: tool.function.name.clone(),
                    description: tool.function.description.clone(),
                    parameters: tool.function.parameters.clone(),
                },
            })
            .collect()
    });

    // Convert tool_choice from server types to provider types
    let tool_choice = request.tool_choice.as_ref().map(|choice| match choice {
        crate::server::types::ToolChoice::Auto(s) => crate::providers::ToolChoice::Auto(s.clone()),
        crate::server::types::ToolChoice::Specific {
            tool_type,
            function,
        } => crate::providers::ToolChoice::Specific {
            tool_type: tool_type.clone(),
            function: crate::providers::FunctionName {
                name: function.name.clone(),
            },
        },
    });

    // Convert response_format from server types to provider types (Bug #7 fix)
    let response_format = request.response_format.as_ref().map(|format| match format {
        crate::server::types::ResponseFormat::JsonObject { r#type } => {
            crate::providers::ResponseFormat::JsonObject {
                format_type: r#type.clone(),
            }
        }
        crate::server::types::ResponseFormat::JsonSchema { r#type, schema } => {
            crate::providers::ResponseFormat::JsonSchema {
                format_type: r#type.clone(),
                schema: schema.clone(),
            }
        }
    });

    Ok(ProviderCompletionRequest {
        model: request.model.clone(),
        messages,
        temperature: request.temperature,
        max_tokens,
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
        // Tool calling (Bug #4 fix)
        tools,
        tool_choice,
        // Response format (Bug #7 fix)
        response_format,
        // Log probabilities (Bug #6 fix)
        logprobs: request.logprobs,
        top_logprobs: request.top_logprobs,
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

    // For chat messages, calculate incremental token count (last message only)
    // instead of cumulative (all conversation history)
    let incremental_prompt_tokens = if let Some(last_msg) = request.messages.last() {
        estimate_token_count(&[last_msg.clone()]) as u32
    } else {
        response.usage.prompt_tokens
    };

    let cost = {
        let input_cost = (incremental_prompt_tokens as f64 / 1000.0) * pricing.input_cost_per_1k;
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
            input_tokens: incremental_prompt_tokens as u64,
            output_tokens: response.usage.completion_tokens as u64,
            cost_usd: cost,
            latency_ms,
        });

    // Record tokens for tray graph (real-time tracking for Fast/Medium modes)
    if let Some(ref tray_graph) = *state.tray_graph_manager.read() {
        tray_graph
            .record_tokens((incremental_prompt_tokens + response.usage.completion_tokens) as u64);
    }

    // Log to access log (persistent storage)
    if let Err(e) = state.access_logger.log_success(
        &auth.api_key_id,
        &response.provider,
        &response.model,
        incremental_prompt_tokens as u64,
        response.usage.completion_tokens as u64,
        cost,
        latency_ms,
        &generation_id,
    ) {
        tracing::warn!("Failed to write access log: {}", e);
    }

    // Emit event for real-time UI updates
    state.emit_event(
        "metrics-updated",
        &serde_json::json!({
            "timestamp": created_at.to_rfc3339(),
        })
        .to_string(),
    );

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
            .map(|choice| {
                // Convert provider message content to server message content
                let content = match choice.message.content {
                    crate::providers::ChatMessageContent::Text(text) => {
                        if text.is_empty() && choice.message.tool_calls.is_some() {
                            // If content is empty and we have tool calls, content can be None
                            None
                        } else {
                            Some(MessageContent::Text(text))
                        }
                    }
                    crate::providers::ChatMessageContent::Parts(parts) => {
                        // Convert provider parts to server parts
                        let server_parts: Vec<crate::server::types::ContentPart> = parts
                            .into_iter()
                            .map(|part| match part {
                                crate::providers::ContentPart::Text { text } => {
                                    crate::server::types::ContentPart::Text { text }
                                }
                                crate::providers::ContentPart::ImageUrl { image_url } => {
                                    crate::server::types::ContentPart::ImageUrl {
                                        image_url: crate::server::types::ImageUrl {
                                            url: image_url.url,
                                            detail: image_url.detail,
                                        },
                                    }
                                }
                            })
                            .collect();
                        Some(MessageContent::Parts(server_parts))
                    }
                };

                // Convert provider tool_calls to server tool_calls
                let tool_calls = choice.message.tool_calls.map(|provider_tools| {
                    provider_tools
                        .into_iter()
                        .map(|tool_call| crate::server::types::ToolCall {
                            id: tool_call.id,
                            tool_type: tool_call.tool_type,
                            function: crate::server::types::FunctionCall {
                                name: tool_call.function.name,
                                arguments: tool_call.function.arguments,
                            },
                        })
                        .collect()
                });

                ChatCompletionChoice {
                    index: choice.index,
                    message: ChatMessage {
                        role: choice.message.role,
                        content,
                        name: choice.message.name,
                        tool_calls,
                        tool_call_id: choice.message.tool_call_id,
                    },
                    finish_reason: choice.finish_reason,
                    logprobs: choice
                        .logprobs
                        .map(|provider_logprobs| ChatCompletionLogprobs {
                            content: provider_logprobs.content.map(|tokens| {
                                tokens
                                    .into_iter()
                                    .map(|token| ChatCompletionTokenLogprob {
                                        token: token.token,
                                        logprob: token.logprob,
                                        bytes: token.bytes,
                                        top_logprobs: token
                                            .top_logprobs
                                            .into_iter()
                                            .map(|top| TopLogprob {
                                                token: top.token,
                                                logprob: top.logprob,
                                                bytes: top.bytes,
                                            })
                                            .collect(),
                                    })
                                    .collect()
                            }),
                        }),
                }
            })
            .collect(),
        usage: TokenUsage {
            prompt_tokens: incremental_prompt_tokens,
            completion_tokens: response.usage.completion_tokens,
            total_tokens: incremental_prompt_tokens + response.usage.completion_tokens,
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
        finish_reason: api_response
            .choices
            .first()
            .and_then(|c| c.finish_reason.clone())
            .unwrap_or_else(|| "unknown".to_string()),
        tokens: api_response.usage.clone(),
        cost: Some(crate::server::types::CostDetails {
            prompt_cost: (incremental_prompt_tokens as f64 / 1000.0) * pricing.input_cost_per_1k,
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
    let request_messages = request.messages.clone();

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

                    let api_chunk = ChatCompletionChunk {
                        id: gen_id.clone(),
                        object: "chat.completion.chunk".to_string(),
                        created: created_timestamp,
                        model: model.clone(),
                        choices: provider_chunk
                            .choices
                            .into_iter()
                            .map(|choice| {
                                // Convert provider tool_calls delta to server tool_calls delta
                                let tool_calls = choice.delta.tool_calls.map(|provider_deltas| {
                                    provider_deltas
                                        .into_iter()
                                        .map(|delta| crate::server::types::ToolCallDelta {
                                            index: delta.index,
                                            id: delta.id,
                                            tool_type: delta.tool_type,
                                            function: delta.function.map(|f| {
                                                crate::server::types::FunctionCallDelta {
                                                    name: f.name,
                                                    arguments: f.arguments,
                                                }
                                            }),
                                        })
                                        .collect()
                                });

                                ChatCompletionChunkChoice {
                                    index: choice.index,
                                    delta: ChunkDelta {
                                        role: choice.delta.role,
                                        content: choice.delta.content,
                                        tool_calls,
                                    },
                                    finish_reason: choice.finish_reason,
                                }
                            })
                            .collect(),
                        usage: None, // Not available in streaming chunks
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

        // Estimate tokens for this message only (not the entire conversation)
        // Count only the last user message (the new message that was just sent)
        let last_user_message_tokens = if let Some(last_msg) = request_messages.last() {
            estimate_token_count(&[last_msg.clone()])
        } else {
            0
        };
        let prompt_tokens = last_user_message_tokens as u32;
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
                tool_calls: None,
                tool_call_id: None,
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
            max_completion_tokens: None,
            n: None,
            logprobs: None,
            top_logprobs: None,
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
                tool_calls: None,
                tool_call_id: None,
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
            max_completion_tokens: None,
            n: None,
            logprobs: None,
            top_logprobs: None,
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
            max_completion_tokens: None,
            n: None,
            logprobs: None,
            top_logprobs: None,
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
                tool_calls: None,
                tool_call_id: None,
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
            max_completion_tokens: None,
            n: None,
            logprobs: None,
            top_logprobs: None,
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
                tool_calls: None,
                tool_call_id: None,
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
            max_completion_tokens: None,
            n: None,
            logprobs: None,
            top_logprobs: None,
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
                tool_calls: None,
                tool_call_id: None,
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
            max_completion_tokens: None,
            n: None,
            logprobs: None,
            top_logprobs: None,
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
                tool_calls: None,
                tool_call_id: None,
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
            max_completion_tokens: None,
            n: None,
            logprobs: None,
            top_logprobs: None,
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
                tool_calls: None,
                tool_call_id: None,
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
            max_completion_tokens: None,
            n: None,
            logprobs: None,
            top_logprobs: None,
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
                tool_calls: None,
                tool_call_id: None,
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
            max_completion_tokens: None,
            n: None,
            logprobs: None,
            top_logprobs: None,
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
                tool_calls: None,
                tool_call_id: None,
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
            max_completion_tokens: None,
            n: None,
            logprobs: None,
            top_logprobs: None,
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
                tool_calls: None,
                tool_call_id: None,
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
            max_completion_tokens: None,
            n: None,
            logprobs: None,
            top_logprobs: None,
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
                tool_calls: None,
                tool_call_id: None,
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
            max_completion_tokens: None,
            n: None,
            logprobs: None,
            top_logprobs: None,
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
                tool_calls: None,
                tool_call_id: None,
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
            max_completion_tokens: None,
            n: None,
            logprobs: None,
            top_logprobs: None,
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
                tool_calls: None,
                tool_call_id: None,
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
            max_completion_tokens: None,
            n: None,
            logprobs: None,
            top_logprobs: None,
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
                tool_calls: None,
                tool_call_id: None,
            },
            ChatMessage {
                role: "assistant".to_string(),
                content: Some(MessageContent::Text("I'm doing well!".to_string())), // ~15 chars = 3-4 tokens
                name: None,
                tool_calls: None,
                tool_call_id: None,
            },
        ];

        let count = estimate_token_count(&messages);
        assert!(count > 0);
        assert!(count < 100); // Should be reasonable
    }
}
