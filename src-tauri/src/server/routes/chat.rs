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
use crate::server::state::{AppState, AuthContext, GenerationDetails};
use crate::server::types::{
    ChatCompletionChunk, ChatCompletionChunkChoice, ChatCompletionChoice, ChatCompletionRequest,
    ChatCompletionResponse, ChatMessage, ChunkDelta, MessageContent,
    TokenUsage,
};

/// POST /v1/chat/completions
/// Send a chat completion request
pub async fn chat_completions(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Json(request): Json<ChatCompletionRequest>,
) -> ApiResult<Response> {
    // Validate request
    validate_request(&request)?;

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
    let response = state
        .router
        .complete(&auth.api_key_id, provider_request)
        .await
        .map_err(|e| ApiErrorResponse::bad_gateway(format!("Provider error: {}", e)))?;

    let completed_at = Instant::now();

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
        },
    };

    // Track generation details
    let generation_details = GenerationDetails {
        id: generation_id,
        model: response.model.clone(),
        provider: "router".to_string(), // Router determines actual provider
        created_at,
        finish_reason: api_response.choices.first().and_then(|c| c.finish_reason.clone()).unwrap_or_else(|| "unknown".to_string()),
        tokens: api_response.usage.clone(),
        cost: None, // TODO: Calculate cost
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
    _request: ChatCompletionRequest,
    provider_request: ProviderCompletionRequest,
) -> ApiResult<Response> {
    let generation_id = format!("gen-{}", Uuid::new_v4());
    let created_at = Utc::now();

    // Clone model before moving provider_request
    let model = provider_request.model.clone();

    // Call router to get streaming completion
    let stream = state
        .router
        .stream_complete(&auth.api_key_id, provider_request)
        .await
        .map_err(|e| ApiErrorResponse::bad_gateway(format!("Provider error: {}", e)))?;

    // Convert provider stream to SSE stream
    let created_timestamp = created_at.timestamp();
    let gen_id = generation_id.clone();

    let sse_stream = stream.map(move |chunk_result| -> Result<Event, std::convert::Infallible> {
        match chunk_result {
            Ok(provider_chunk) => {
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
                    usage: None, // Usage sent in final chunk
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
