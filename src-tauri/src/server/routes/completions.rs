//! POST /v1/completions endpoint
//!
//! Legacy text completion endpoint (converts to chat format internally).

use axum::{
    extract::State,
    response::{IntoResponse, Response},
    Extension, Json,
};
use std::time::Instant;
use chrono::Utc;
use uuid::Uuid;

use crate::providers::{ChatMessage as ProviderChatMessage, CompletionRequest as ProviderCompletionRequest};
use crate::server::middleware::error::{ApiErrorResponse, ApiResult};
use crate::server::state::{AppState, AuthContext, GenerationDetails};
use crate::server::types::{
    CompletionChoice, CompletionRequest, CompletionResponse, PromptInput, TokenUsage,
};

/// POST /v1/completions
/// Legacy completion endpoint - converts prompt to chat format
pub async fn completions(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Json(request): Json<CompletionRequest>,
) -> ApiResult<Response> {
    // Emit LLM request event to trigger tray icon indicator
    state.emit_event("llm-request", "completion");

    // Validate request
    validate_request(&request)?;

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
    };

    if request.stream {
        // For streaming, we'd need to convert the chat chunks to completion chunks
        // For now, return an error
        return Err(ApiErrorResponse::bad_request(
            "Streaming not yet supported for legacy completions endpoint",
        ));
    }

    handle_non_streaming(state, auth, request, provider_request).await
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
            content: p,
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
    let response = state
        .router
        .complete(&auth.api_key_id, provider_request)
        .await
        .map_err(|e| ApiErrorResponse::bad_gateway(format!("Provider error: {}", e)))?;

    let completed_at = Instant::now();

    // Note: Router already records usage for rate limiting, so we don't need to do it here

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
                text: choice.message.content,
                index: choice.index,
                finish_reason: choice.finish_reason,
                logprobs: None,
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
        provider: "router".to_string(), // Using router, not a specific provider
        created_at,
        finish_reason: api_response
            .choices
            .first()
            .and_then(|c| c.finish_reason.clone())
            .unwrap_or_else(|| "unknown".to_string()),
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
