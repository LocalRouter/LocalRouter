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

use super::helpers::{
    check_llm_access_with_state, get_enabled_client, get_enabled_client_from_manager,
};
use super::stream_usage::{finalize_stream, StreamTracker};
use crate::middleware::client_auth::ClientAuthContext;
use crate::middleware::error::{ApiErrorResponse, ApiResult};
use crate::state::{AppState, AuthContext};
use crate::types::{
    CompletionChoice, CompletionChunk, CompletionChunkChoice, CompletionRequest,
    CompletionResponse, PromptInput, TokenUsage,
};
use lr_providers::CompletionRequest as ProviderCompletionRequest;

#[cfg(test)]
use lr_providers::{ChatMessage as ProviderChatMessage, ChatMessageContent};

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
        (status = 400, description = "Bad request", body = crate::types::ErrorResponse),
        (status = 401, description = "Unauthorized", body = crate::types::ErrorResponse),
        (status = 429, description = "Rate limit exceeded", body = crate::types::ErrorResponse),
        (status = 500, description = "Internal server error", body = crate::types::ErrorResponse)
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

    // Generate session ID for correlated monitor events
    let session_id = uuid::Uuid::new_v4().to_string();

    // Emit monitor event for traffic inspection
    let request_json = serde_json::to_value(&request).unwrap_or_default();
    let mut llm_guard = super::monitor_helpers::emit_llm_call(
        &state,
        client_auth.as_ref(),
        Some(&session_id),
        "/v1/completions",
        &request.model,
        request.stream,
        &request_json,
    );

    // Record client activity for connection graph
    state.record_client_activity(&auth.api_key_id);

    // Enforce client mode: block MCP-only clients from LLM endpoints
    {
        let client =
            get_enabled_client(&state, &auth.api_key_id).map_err(|e| llm_guard.capture_err(e))?;
        check_llm_access_with_state(&state, &client).map_err(|e| llm_guard.capture_err(e))?;
    }

    // Run the shared pipeline (validate → access checks → rate
    // limits → secret scan → guardrails → compression → RouteLLM →
    // convert). Legacy `/v1/completions` inherits the same feature
    // set as `/v1/chat/completions` by routing through the canonical
    // entry point with a `CompletionRequest → ChatCompletionRequest`
    // adapter.
    let chat_req =
        legacy_to_chat_completion_request(&request).map_err(|e| llm_guard.capture_err(e))?;
    let turn = super::pipeline::run_turn_pipeline(
        &state,
        &auth,
        client_auth.as_ref(),
        chat_req,
        &mut llm_guard,
        "/v1/completions",
        session_id,
        super::pipeline::PipelineCaps::completions(),
    )
    .await?;
    let super::pipeline::TurnContext {
        provider_request,
        guardrail_handle,
        compression_tokens_saved,
        ..
    } = turn;

    // Log request summary
    {
        let client_id_short = &auth.api_key_id[..8.min(auth.api_key_id.len())];
        let guardrails_active = guardrail_handle.is_some();
        tracing::info!(
            "Completion request: client={}, model={}, stream={}, guardrails={}",
            client_id_short,
            request.model,
            request.stream,
            guardrails_active,
        );
    }

    // Determine if we can run guardrails in parallel with the LLM request
    let config = state.config_manager.get();
    let use_parallel = config.guardrails.parallel_guardrails
        && !has_side_effects(&request)
        && guardrail_handle.is_some();

    if use_parallel {
        // Defuse the guard: parallel handler functions manage their own completion.
        let llm_event_id = llm_guard.into_event_id();
        let guardrail_handle = guardrail_handle.unwrap();

        if request.stream {
            handle_streaming_parallel(
                state,
                auth,
                client_auth,
                request,
                provider_request,
                guardrail_handle,
                compression_tokens_saved,
                llm_event_id,
            )
            .await
        } else {
            handle_non_streaming_parallel(
                state,
                auth,
                client_auth,
                request,
                provider_request,
                guardrail_handle,
                compression_tokens_saved,
                llm_event_id,
            )
            .await
        }
    } else {
        // Sequential mode: if the pipeline spawned a guardrail scan
        // (parallel caps but parallel dispatch disabled by side-
        // effects), await and approve here. If guardrails ran
        // inline already, `guardrail_handle` is `None` and there's
        // nothing to do.
        if let Some(handle) = guardrail_handle {
            let guardrail_result = handle
                .await
                .map_err(|e| {
                    llm_guard.capture_err(ApiErrorResponse::internal_error(format!(
                        "Guardrail check failed: {}",
                        e
                    )))
                })?
                .map_err(|e| llm_guard.capture_err(e))?;

            if let Some(check_result) = guardrail_result {
                super::pipeline::handle_guardrail_approval(
                    &state,
                    client_auth.as_ref().map(|e| &e.0),
                    // Synthesize a throwaway ChatCompletionRequest for
                    // the approval popup — the helper only inspects
                    // `model` and `messages` for display. We'd re-
                    // convert `&request` but the conversion just built
                    // a ChatCompletionRequest moments ago; the minimal
                    // ask here is OK since the popup is cosmetic.
                    &legacy_to_chat_completion_request(&request)
                        .map_err(|e| llm_guard.capture_err(e))?,
                    check_result,
                    "request",
                )
                .await
                .map_err(|e| llm_guard.capture_err(e))?;
            }
        }

        // Now defuse — sub-functions manage their own completion from here
        let llm_event_id = llm_guard.into_event_id();

        if request.stream {
            handle_streaming(
                state,
                auth,
                client_auth,
                request,
                provider_request,
                compression_tokens_saved,
                llm_event_id,
            )
            .await
        } else {
            handle_non_streaming(
                state,
                auth,
                client_auth,
                request,
                provider_request,
                compression_tokens_saved,
                llm_event_id,
            )
            .await
        }
    }
}

/// Validate completion request
#[cfg(test)]
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

/// Check whether a completions request may cause side effects that require sequential guardrails.
fn has_side_effects(request: &CompletionRequest) -> bool {
    // Legacy completions have no tools; only check model name
    let model = request.model.to_lowercase();
    model.contains("sonar")
}

/// Handle guardrail approval popup for detected violations
async fn handle_guardrail_approval(
    state: &AppState,
    client_context: Option<&ClientAuthContext>,
    request: &CompletionRequest,
    result: lr_guardrails::SafetyCheckResult,
) -> ApiResult<()> {
    use lr_mcp::gateway::firewall::{FirewallApprovalAction, GuardrailApprovalDetails};

    if !result.needs_approval() {
        return Ok(());
    }

    let Some(client_ctx) = client_context else {
        return Ok(());
    };

    // Check for time-based guardrail denial (Deny All for 1 Hour)
    if state
        .guardrail_denial_tracker
        .has_valid_denial(&client_ctx.client_id)
    {
        tracing::info!(
            "Guardrail: auto-denying request for client {} (active denial bypass)",
            client_ctx.client_id
        );
        return Err(ApiErrorResponse::forbidden(
            "Request blocked by safety guardrails (auto-denied)",
        ));
    }

    let client = get_enabled_client_from_manager(state, &client_ctx.client_id)?;

    // Extract the scanned text for display in the approval popup
    let request_json = serde_json::to_value(request).unwrap_or_default();
    let flagged_text = build_flagged_text_preview(
        &lr_guardrails::text_extractor::extract_request_text(&request_json),
    );

    let details = GuardrailApprovalDetails {
        verdicts: result
            .verdicts
            .iter()
            .map(|v| serde_json::to_value(v).unwrap_or_default())
            .collect(),
        actions_required: result
            .actions_required
            .iter()
            .map(|a| serde_json::to_value(a).unwrap_or_default())
            .collect(),
        total_duration_ms: result.total_duration_ms,
        scan_direction: "request".to_string(),
        flagged_text,
    };

    let preview = result
        .actions_required
        .iter()
        .map(|a| format!("{}: {:?}", a.category, a.action))
        .collect::<Vec<_>>()
        .join("\n");

    let response = state
        .mcp_gateway
        .firewall_manager
        .request_guardrail_approval(
            client.id.clone(),
            client.name.clone(),
            request.model.clone(),
            "guardrails".to_string(),
            details,
            preview,
        )
        .await
        .map_err(|e| {
            ApiErrorResponse::internal_error(format!("Guardrail approval failed: {}", e))
        })?;

    match response.action {
        FirewallApprovalAction::AllowOnce
        | FirewallApprovalAction::AllowSession
        | FirewallApprovalAction::Allow1Minute
        | FirewallApprovalAction::Allow1Hour
        | FirewallApprovalAction::AllowPermanent
        | FirewallApprovalAction::AllowCategories => Ok(()),
        FirewallApprovalAction::Deny
        | FirewallApprovalAction::DenySession
        | FirewallApprovalAction::DenyAlways
        | FirewallApprovalAction::BlockCategories
        | FirewallApprovalAction::Deny1Hour
        | FirewallApprovalAction::DisableClient => Err(ApiErrorResponse::forbidden(
            "Request blocked by safety check",
        )),
    }
}

/// Build a truncated text preview from extracted texts for the guardrail approval popup.
fn build_flagged_text_preview(texts: &[lr_guardrails::text_extractor::ExtractedText]) -> String {
    const MAX_LEN: usize = 500;

    // Prefer the last user message as the most relevant context
    let best = texts
        .iter()
        .rev()
        .find(|t| t.label.starts_with("user"))
        .or_else(|| texts.last());

    match best {
        Some(t) => {
            let prefix = format!("[{}] ", t.label);
            let available = MAX_LEN.saturating_sub(prefix.len());
            if t.text.len() <= available {
                format!("{}{}", prefix, t.text)
            } else {
                // Find a safe char boundary to avoid panicking on multi-byte UTF-8
                let mut safe_end = available.saturating_sub(3).min(t.text.len());
                while safe_end > 0 && !t.text.is_char_boundary(safe_end) {
                    safe_end -= 1;
                }
                format!("{}{}...", prefix, &t.text[..safe_end])
            }
        }
        None => String::new(),
    }
}

/// Convert prompt(s) to chat message format
#[cfg(test)]
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
            reasoning_content: None,
        })
        .collect();

    Ok(messages)
}

/// Convert a legacy `/v1/completions` request into a
/// `ChatCompletionRequest` the shared pipeline can drive.
///
/// Each `prompt` entry becomes a user-role `ChatMessage` with
/// `content: Text(...)`. Legacy fields that chat completions
/// doesn't carry (logprobs/top_logprobs for the legacy-text-shape,
/// `n`, `stop`, etc.) are preserved so that
/// `convert_to_provider_request` regenerates an equivalent
/// `ProviderCompletionRequest`.
fn legacy_to_chat_completion_request(
    req: &CompletionRequest,
) -> ApiResult<crate::types::ChatCompletionRequest> {
    let messages = match &req.prompt {
        PromptInput::Single(p) => vec![crate::types::ChatMessage {
            role: "user".to_string(),
            content: Some(crate::types::MessageContent::Text(p.clone())),
            name: None,
            tool_calls: None,
            tool_call_id: None,
            reasoning_content: None,
        }],
        PromptInput::Multiple(ps) => ps
            .iter()
            .map(|p| crate::types::ChatMessage {
                role: "user".to_string(),
                content: Some(crate::types::MessageContent::Text(p.clone())),
                name: None,
                tool_calls: None,
                tool_call_id: None,
                reasoning_content: None,
            })
            .collect(),
    };

    Ok(crate::types::ChatCompletionRequest {
        model: req.model.clone(),
        messages,
        temperature: req.temperature,
        top_p: req.top_p,
        max_tokens: req.max_tokens,
        max_completion_tokens: None,
        n: req.n,
        stop: req.stop.clone(),
        stream: req.stream,
        stream_options: None,
        logprobs: None,
        top_logprobs: None,
        frequency_penalty: req.frequency_penalty,
        presence_penalty: req.presence_penalty,
        top_k: None,
        seed: None,
        repetition_penalty: None,
        response_format: None,
        tools: None,
        tool_choice: None,
        parallel_tool_calls: None,
        logit_bias: None,
        service_tier: None,
        store: None,
        metadata: None,
        modalities: None,
        audio: None,
        prediction: None,
        reasoning_effort: None,
        extensions: None,
        user: req.user.clone(),
    })
}

/// Handle non-streaming completion
/// Type alias for a spawned guardrail scan task
type GuardrailHandle = tokio::task::JoinHandle<ApiResult<Option<lr_guardrails::SafetyCheckResult>>>;

/// Handle non-streaming completion with parallel guardrails.
#[allow(clippy::too_many_arguments)]
async fn handle_non_streaming_parallel(
    state: AppState,
    auth: AuthContext,
    client_auth: Option<Extension<ClientAuthContext>>,
    request: CompletionRequest,
    provider_request: ProviderCompletionRequest,
    guardrail_handle: GuardrailHandle,
    compression_tokens_saved: u64,
    llm_event_id: String,
) -> ApiResult<Response> {
    let generation_id = format!("gen-{}", Uuid::new_v4());
    let started_at = Instant::now();
    let created_at = Utc::now();

    // Start LLM request immediately. Preserve the router's routing
    // metadata so `build_non_streaming_response` can attach it to the
    // monitor event — matches `handle_non_streaming`'s behavior.
    // Run the LLM call as an in-handler future rather than a detached spawn:
    // if the request is aborted (client disconnect or server Stop), the
    // provider call is dropped with it, which closes the upstream connection
    // and cancels generation (local providers like Ollama otherwise keep
    // generating to completion).
    let router = state.router.clone();
    let api_key_id = auth.api_key_id.clone();
    let llm_future = router.complete(&api_key_id, provider_request);

    // Wait for both concurrently
    let (guardrail_result, llm_result) = tokio::join!(guardrail_handle, llm_future);

    // Process guardrail result first
    let guardrail_result = guardrail_result.map_err(|e| {
        ApiErrorResponse::internal_error(format!("Guardrail check failed: {}", e))
    })??;

    if let Some(check_result) = guardrail_result {
        handle_guardrail_approval(
            &state,
            client_auth.as_ref().map(|e| &e.0),
            &request,
            check_result,
        )
        .await?;
    }

    // Unwrap LLM response (and router's routing metadata)
    let (response, routing_metadata) = llm_result.map_err(|e| {
        let err_text = e.to_string();
        let api_err: ApiErrorResponse = e.into();
        let status_code = api_err.status.as_u16();
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
        if let Err(log_err) = state.access_logger.log_failure(
            &auth.api_key_id,
            "unknown",
            "unknown",
            latency,
            &generation_id,
            status_code,
        ) {
            tracing::warn!("Failed to write access log: {}", log_err);
        }

        // Emit monitor error event
        super::monitor_helpers::complete_llm_call_error(
            &state,
            &llm_event_id,
            "unknown",
            &request.model,
            status_code,
            &err_text,
        );

        api_err
    })?;

    build_non_streaming_response(
        state,
        auth,
        client_auth,
        request,
        response,
        generation_id,
        started_at,
        created_at,
        compression_tokens_saved,
        llm_event_id,
        routing_metadata,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn handle_non_streaming(
    state: AppState,
    auth: AuthContext,
    _client_auth: Option<Extension<ClientAuthContext>>,
    request: CompletionRequest,
    provider_request: ProviderCompletionRequest,
    compression_tokens_saved: u64,
    llm_event_id: String,
) -> ApiResult<Response> {
    let generation_id = format!("gen-{}", Uuid::new_v4());
    let started_at = Instant::now();
    let created_at = Utc::now();

    // Call router to get completion. Routing metadata is emitted
    // onto the `LlmCall` monitor event below so auto-routing
    // decisions show up alongside the completion.
    let (response, routing_metadata) = match state
        .router
        .complete(&auth.api_key_id, provider_request)
        .await
    {
        Ok((resp, routing_meta)) => (resp, routing_meta),
        Err(e) => {
            let err_text = e.to_string();
            let api_err: ApiErrorResponse = e.into();
            let status_code = api_err.status.as_u16();
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
                status_code,
            ) {
                tracing::warn!("Failed to write access log: {}", log_err);
            }

            // Emit monitor error event
            super::monitor_helpers::complete_llm_call_error(
                &state,
                &llm_event_id,
                "unknown",
                &request.model,
                status_code,
                &err_text,
            );

            return Err(api_err);
        }
    };

    build_non_streaming_response(
        state,
        auth,
        _client_auth,
        request,
        response,
        generation_id,
        started_at,
        created_at,
        compression_tokens_saved,
        llm_event_id,
        routing_metadata,
    )
    .await
}

/// Build the non-streaming completion response. Shared by both sequential and parallel handlers.
#[allow(clippy::too_many_arguments)]
async fn build_non_streaming_response(
    state: AppState,
    auth: AuthContext,
    _client_auth: Option<Extension<ClientAuthContext>>,
    request: CompletionRequest,
    response: lr_providers::CompletionResponse,
    generation_id: String,
    started_at: Instant,
    created_at: chrono::DateTime<Utc>,
    compression_tokens_saved: u64,
    llm_event_id: String,
    routing_metadata: Option<serde_json::Value>,
) -> ApiResult<Response> {
    // Legacy `/v1/completions` charges the full prompt_tokens from
    // the provider (single-shot prompt — no chat-style incremental
    // accounting). The shared finalize helper handles
    // cost/metrics/tray/access-log/monitor completion using exactly
    // that value.
    let finalize_inputs = super::finalize::FinalizeInputs {
        state: &state,
        auth: &auth,
        llm_event_id: &llm_event_id,
        generation_id: &generation_id,
        started_at,
        created_at,
        prompt_tokens: response.usage.prompt_tokens,
        compression_tokens_saved,
        routing_metadata: routing_metadata.as_ref(),
        user: request.user.clone(),
        streamed: false,
        skip_monitor_completion: false,
    };
    let metrics = super::finalize::finalize_metrics_and_monitor(&finalize_inputs, &response).await;

    // Convert chat completion response to legacy completion response.
    // We clone the fields we thread into the wire body so the shared
    // finalize tail can still borrow `&response` for the generation
    // row.
    let api_response = CompletionResponse {
        id: generation_id.clone(),
        object: "text_completion".to_string(),
        created: created_at.timestamp(),
        model: response.model.clone(),
        choices: response
            .choices
            .iter()
            .map(|choice| CompletionChoice {
                text: choice.message.content.as_text(),
                index: choice.index,
                finish_reason: choice.finish_reason.clone(),
                logprobs: None,
            })
            .collect(),
        usage: TokenUsage {
            prompt_tokens: response.usage.prompt_tokens,
            completion_tokens: response.usage.completion_tokens,
            total_tokens: response.usage.total_tokens,
            prompt_tokens_details: response.usage.prompt_tokens_details.clone(),
            completion_tokens_details: response.usage.completion_tokens_details.clone(),
        },
        request_usage_entries: None,
    };

    // Shared finalize tail: stash wire-format body on the `LlmCall`
    // event and record the `GenerationDetails` row.
    let wire_body = serde_json::to_value(&api_response).unwrap_or(serde_json::Value::Null);
    let finish_reason = api_response
        .choices
        .first()
        .and_then(|c| c.finish_reason.clone());
    super::finalize::update_response_body_and_record_generation(
        &finalize_inputs,
        &response,
        &metrics,
        &wire_body,
        finish_reason,
        api_response.usage.clone(),
    );

    Ok(Json(api_response).into_response())
}

/// The chunk sent before `[DONE]` when the client set
/// `stream_options.include_usage`: empty `choices`, the request's usage.
fn usage_chunk_event(
    id: &str,
    created: i64,
    tracked: &StreamTracker,
    prompt_estimate: u64,
) -> Event {
    let chunk = CompletionChunk {
        id: id.to_string(),
        object: "text_completion".to_string(),
        created,
        choices: Vec::new(),
        usage: Some(tracked.client_usage(prompt_estimate)),
    };
    Event::default().data(serde_json::to_string(&chunk).unwrap_or_default())
}

/// Handle streaming completion
#[allow(clippy::too_many_arguments)]
async fn handle_streaming(
    state: AppState,
    auth: AuthContext,
    _client_auth: Option<Extension<ClientAuthContext>>,
    request: CompletionRequest,
    provider_request: ProviderCompletionRequest,
    compression_tokens_saved: u64,
    llm_event_id: String,
) -> ApiResult<Response> {
    let generation_id = format!("gen-{}", Uuid::new_v4());
    let created_at = Utc::now();
    let started_at = Instant::now();

    // Clone model before moving provider_request
    let model = provider_request.model.clone();
    let prompt_estimate = lr_providers::usage_estimate::prompt_tokens(
        &provider_request.messages,
        provider_request.tools.as_deref(),
    );

    // Call router to get streaming completion. Routing metadata is
    // forwarded into the spawned stream task below so auto-routing
    // decisions show up on the monitor event.
    let (stream, routing_metadata) = match state
        .router
        .stream_complete(&auth.api_key_id, provider_request)
        .await
    {
        Ok((s, routing_meta)) => (s, routing_meta),
        Err(e) => {
            let err_text = e.to_string();
            let api_err: ApiErrorResponse = e.into();
            let status_code = api_err.status.as_u16();
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
                status_code,
            ) {
                tracing::warn!("Failed to write access log: {}", log_err);
            }

            // Emit monitor error event
            super::monitor_helpers::complete_llm_call_error(
                &state,
                &llm_event_id,
                "unknown",
                &model,
                status_code,
                &err_text,
            );

            return Err(api_err);
        }
    };

    // Convert provider stream to SSE stream
    let created_timestamp = created_at.timestamp();
    let gen_id = generation_id.clone();

    // Track what the stream produced; finalize once it ends
    use parking_lot::Mutex;
    use std::sync::Arc;
    let tracker = Arc::new(Mutex::new(StreamTracker::default()));

    // Resolves when the stream finishes, or when it is dropped because the
    // client disconnected (the sender is dropped with it)
    let (completion_tx, completion_rx) = tokio::sync::oneshot::channel::<()>();
    let completion_tx = Arc::new(Mutex::new(Some(completion_tx)));
    let tracker_map = tracker.clone();
    let tracker_end = tracker.clone();
    let gen_id_end = generation_id.clone();
    let include_usage = request
        .stream_options
        .as_ref()
        .is_some_and(|o| o.include_usage);

    // Clone for tracking after stream completes
    let state_clone = state.clone();
    let auth_clone = auth.clone();
    let gen_id_clone = generation_id.clone();
    let model_clone = model.clone();
    let created_at_clone = created_at;
    let request_user = request.user.clone();

    let sse_stream = stream.filter_map(move |chunk_result| {
        std::future::ready(match chunk_result {
            Ok(provider_chunk) => {
                tracker_map.lock().observe(&provider_chunk);
                if StreamTracker::is_usage_only(&provider_chunk) {
                    return std::future::ready(None);
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
                    usage: None,
                };

                let json = serde_json::to_string(&api_chunk).unwrap_or_default();
                Some(Ok::<Event, std::convert::Infallible>(
                    Event::default().data(json),
                ))
            }
            Err(e) => {
                tracing::error!("Error in streaming: {}", e);
                // Return error in SSE format with actual error message
                let error_response = serde_json::json!({
                    "error": {
                        "message": format!("Streaming error: {}", e),
                        "type": "server_error",
                        "code": "streaming_error"
                    }
                });
                Some(Ok(Event::default().data(
                    serde_json::to_string(&error_response)
                        .unwrap_or_else(|_| "[ERROR]".to_string()),
                )))
            }
        })
    });
    // End of stream: optional usage chunk, then [DONE]
    let sse_stream = sse_stream.chain(futures::stream::iter(std::iter::once(()).flat_map(
        move |_| {
            let mut tail = Vec::new();
            if include_usage {
                tail.push(Ok(usage_chunk_event(
                    &gen_id_end,
                    created_timestamp,
                    &tracker_end.lock(),
                    prompt_estimate,
                )));
            }
            tail.push(Ok::<Event, std::convert::Infallible>(
                Event::default().data("[DONE]"),
            ));
            if let Some(tx) = completion_tx.lock().take() {
                let _ = tx.send(());
            }
            tail
        },
    )));

    // Record telemetry after the stream completes via the shared
    // finalize helper. Same output shape as chat.rs streaming and
    // responses.rs streaming — cost, metrics, tray graph, access log,
    // `complete_llm_call`, `update_llm_call_response_body`, and the
    // generation-tracker row.
    lr_types::spawn_traced(async move {
        // Ends normally or when the client disconnects
        let _ = completion_rx.await;
        let tracked = std::mem::take(&mut *tracker.lock());
        finalize_stream(
            super::finalize::FinalizeInputs {
                state: &state_clone,
                auth: &auth_clone,
                llm_event_id: &llm_event_id,
                generation_id: &gen_id_clone,
                started_at,
                created_at: created_at_clone,
                prompt_tokens: 0, // from the stream
                compression_tokens_saved,
                routing_metadata: routing_metadata.as_ref(),
                user: request_user,
                streamed: true,
                skip_monitor_completion: false,
            },
            &model_clone,
            &tracked,
            prompt_estimate,
        )
        .await;
    });

    Ok(Sse::new(sse_stream)
        .keep_alive(KeepAlive::default())
        .into_response())
}

/// Handle streaming completion with parallel guardrails.
/// Buffers SSE events until guardrails resolve, then flushes or aborts.
#[allow(clippy::too_many_arguments)]
async fn handle_streaming_parallel(
    state: AppState,
    auth: AuthContext,
    client_auth: Option<Extension<ClientAuthContext>>,
    request: CompletionRequest,
    provider_request: ProviderCompletionRequest,
    guardrail_handle: GuardrailHandle,
    compression_tokens_saved: u64,
    llm_event_id: String,
) -> ApiResult<Response> {
    use tokio::sync::{mpsc, watch};
    use tokio_stream::wrappers::ReceiverStream;

    let generation_id = format!("gen-{}", Uuid::new_v4());
    let created_at = Utc::now();
    let started_at = Instant::now();
    let model = provider_request.model.clone();
    let prompt_estimate = lr_providers::usage_estimate::prompt_tokens(
        &provider_request.messages,
        provider_request.tools.as_deref(),
    );
    let include_usage = request
        .stream_options
        .as_ref()
        .is_some_and(|o| o.include_usage);

    // Start LLM streaming request immediately
    let (stream, routing_metadata) = match state
        .router
        .stream_complete(&auth.api_key_id, provider_request)
        .await
    {
        Ok((s, routing_meta)) => (s, routing_meta),
        Err(e) => {
            let err_text = e.to_string();
            let api_err: ApiErrorResponse = e.into();
            let status_code = api_err.status.as_u16();
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
            if let Err(log_err) = state.access_logger.log_failure(
                &auth.api_key_id,
                "unknown",
                &model,
                latency,
                &generation_id,
                status_code,
            ) {
                tracing::warn!("Failed to write access log: {}", log_err);
            }

            // Emit monitor error event
            super::monitor_helpers::complete_llm_call_error(
                &state,
                &llm_event_id,
                "unknown",
                &model,
                status_code,
                &err_text,
            );

            return Err(api_err);
        }
    };

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum GuardrailGate {
        Pending,
        Passed,
        Denied,
    }

    let (gate_tx, gate_rx) = watch::channel(GuardrailGate::Pending);
    let (event_tx, event_rx) = mpsc::channel::<Result<Event, std::convert::Infallible>>(256);

    // Spawn guardrail resolver
    {
        let state = state.clone();
        let client_auth = client_auth.clone();
        let request = request.clone();
        lr_types::spawn_traced(async move {
            let result = guardrail_handle.await;
            match result {
                Ok(Ok(None)) => {
                    let _ = gate_tx.send(GuardrailGate::Passed);
                }
                Ok(Ok(Some(check_result))) => {
                    match handle_guardrail_approval(
                        &state,
                        client_auth.as_ref().map(|e| &e.0),
                        &request,
                        check_result,
                    )
                    .await
                    {
                        Ok(()) => {
                            let _ = gate_tx.send(GuardrailGate::Passed);
                        }
                        Err(_) => {
                            let _ = gate_tx.send(GuardrailGate::Denied);
                        }
                    }
                }
                Ok(Err(_)) | Err(_) => {
                    tracing::warn!("Guardrail check failed, failing open");
                    let _ = gate_tx.send(GuardrailGate::Passed);
                }
            }
        });
    }

    // Spawn buffer/flush worker
    {
        let created_timestamp = created_at.timestamp();
        let gen_id = generation_id.clone();
        let gen_id_clone = generation_id.clone();
        let model_clone = model.clone();
        let state_clone = state.clone();
        let auth_clone = auth.clone();
        let request_user = request.user.clone();
        let mut gate_rx = gate_rx;
        let mut stream = stream;

        lr_types::spawn_traced(async move {
            let mut buffer: Vec<Result<Event, std::convert::Infallible>> = Vec::new();
            let mut gate_resolved = false;
            let mut gate_state = GuardrailGate::Pending;
            let mut tracker = StreamTracker::default();

            let convert_chunk = |provider_chunk: lr_providers::CompletionChunk,
                                 gen_id: &str,
                                 created_ts: i64|
             -> Result<Event, std::convert::Infallible> {
                let api_chunk = CompletionChunk {
                    id: gen_id.to_string(),
                    object: "text_completion".to_string(),
                    created: created_ts,
                    choices: provider_chunk
                        .choices
                        .into_iter()
                        .map(|choice| CompletionChunkChoice {
                            text: choice.delta.content.unwrap_or_default(),
                            index: choice.index,
                            finish_reason: choice.finish_reason,
                        })
                        .collect(),
                    usage: None,
                };

                let json = serde_json::to_string(&api_chunk).unwrap_or_default();
                Ok(Event::default().data(json))
            };

            loop {
                tokio::select! {
                    // The client is gone — disconnect, or server Stop dropped
                    // the response body via the kill-switch. Stop consuming the
                    // provider stream so it is dropped and the upstream
                    // connection closes, cancelling generation (local
                    // providers like Ollama otherwise keep generating).
                    _ = event_tx.closed() => {
                        break;
                    }
                    chunk = stream.next() => {
                        match chunk {
                            Some(Ok(provider_chunk)) => {
                                // Keep reading past the finish reason: the
                                // upstream's usage often follows it
                                tracker.observe(&provider_chunk);
                                if StreamTracker::is_usage_only(&provider_chunk) {
                                    continue;
                                }
                                let event = convert_chunk(provider_chunk, &gen_id, created_timestamp);
                                if gate_resolved && gate_state == GuardrailGate::Passed {
                                    if event_tx.send(event).await.is_err() {
                                        break;
                                    }
                                } else if !gate_resolved {
                                    buffer.push(event);
                                }
                            }
                            Some(Err(e)) => {
                                tracing::error!("Error in streaming: {}", e);
                                let error_response = serde_json::json!({
                                    "error": {
                                        "message": format!("Streaming error: {}", e),
                                        "type": "server_error",
                                        "code": "streaming_error"
                                    }
                                });
                                let event = Ok(Event::default().data(
                                    serde_json::to_string(&error_response)
                                        .unwrap_or_else(|_| "[ERROR]".to_string()),
                                ));
                                if gate_resolved && gate_state == GuardrailGate::Passed {
                                    let _ = event_tx.send(event).await;
                                } else if !gate_resolved {
                                    buffer.push(event);
                                }
                                break;
                            }
                            None => {
                                break;
                            }
                        }
                    }
                    result = gate_rx.changed(), if !gate_resolved => {
                        if result.is_ok() {
                            gate_resolved = true;
                            gate_state = *gate_rx.borrow();
                            match gate_state {
                                GuardrailGate::Passed => {
                                    for event in buffer.drain(..) {
                                        if event_tx.send(event).await.is_err() {
                                            break;
                                        }
                                    }
                                }
                                GuardrailGate::Denied => {
                                    let error_response = serde_json::json!({
                                        "error": {
                                            "message": "Request blocked by safety guardrails",
                                            "type": "permission_error",
                                            "code": "guardrails_denied"
                                        }
                                    });
                                    let _ = event_tx.send(Ok(Event::default().data(
                                        serde_json::to_string(&error_response)
                                            .unwrap_or_else(|_| "[ERROR]".to_string()),
                                    ))).await;
                                    // Drop the provider stream immediately —
                                    // no point letting the model keep
                                    // generating a response nobody will see.
                                    break;
                                }
                                GuardrailGate::Pending => unreachable!(),
                            }
                        }
                    }
                }
            }

            // Stream done, gate may still be pending
            if !gate_resolved {
                let _ = gate_rx.changed().await;
                gate_state = *gate_rx.borrow();
                match gate_state {
                    GuardrailGate::Passed => {
                        for event in buffer.drain(..) {
                            if event_tx.send(event).await.is_err() {
                                break;
                            }
                        }
                    }
                    GuardrailGate::Denied => {
                        let error_response = serde_json::json!({
                            "error": {
                                "message": "Request blocked by safety guardrails",
                                "type": "permission_error",
                                "code": "guardrails_denied"
                            }
                        });
                        let _ = event_tx
                            .send(Ok(Event::default().data(
                                serde_json::to_string(&error_response)
                                    .unwrap_or_else(|_| "[ERROR]".to_string()),
                            )))
                            .await;
                    }
                    GuardrailGate::Pending => {
                        for event in buffer.drain(..) {
                            if event_tx.send(event).await.is_err() {
                                break;
                            }
                        }
                    }
                }
            }

            // End of stream for a client that is still there and allowed
            // to see it: optional usage chunk, then [DONE]
            if gate_state != GuardrailGate::Denied && !event_tx.is_closed() {
                if include_usage {
                    let _ = event_tx
                        .send(Ok(usage_chunk_event(
                            &gen_id,
                            created_timestamp,
                            &tracker,
                            prompt_estimate,
                        )))
                        .await;
                }
                let _ = event_tx.send(Ok(Event::default().data("[DONE]"))).await;
            }

            // Shared finalize at stream end — same path as
            // `handle_streaming` above.
            finalize_stream(
                super::finalize::FinalizeInputs {
                    state: &state_clone,
                    auth: &auth_clone,
                    llm_event_id: &llm_event_id,
                    generation_id: &gen_id_clone,
                    started_at,
                    created_at,
                    prompt_tokens: 0, // from the stream
                    compression_tokens_saved,
                    routing_metadata: routing_metadata.as_ref(),
                    user: request_user,
                    streamed: true,
                    skip_monitor_completion: false,
                },
                &model_clone,
                &tracker,
                prompt_estimate,
            )
            .await;
        });
    }

    Ok(Sse::new(ReceiverStream::new(event_rx))
        .keep_alive(KeepAlive::default())
        .into_response())
}

#[cfg(test)]
mod tests {
    use super::*;
    use lr_guardrails::text_extractor::ExtractedText;

    // === build_flagged_text_preview tests ===

    #[test]
    fn test_build_flagged_text_preview_ascii() {
        let texts = vec![ExtractedText {
            text: "a".repeat(600),
            message_index: Some(0),
            label: "user".to_string(),
            role: "user".to_string(),
        }];
        let result = build_flagged_text_preview(&texts);
        assert!(result.len() <= 500);
        assert!(result.ends_with("..."));
        assert!(result.starts_with("[user] "));
    }

    #[test]
    fn test_build_flagged_text_preview_empty() {
        let result = build_flagged_text_preview(&[]);
        assert_eq!(result, "");
    }

    #[test]
    fn test_build_flagged_text_preview_multibyte_utf8() {
        // Each Chinese character is 3 bytes. Fill with enough to exceed MAX_LEN.
        let text = "你好世界".repeat(200); // 4 chars * 200 = 800 chars, 2400 bytes
        let texts = vec![ExtractedText {
            text,
            message_index: Some(0),
            label: "user".to_string(),
            role: "user".to_string(),
        }];
        let result = build_flagged_text_preview(&texts);
        // Must not panic and must be valid UTF-8
        assert!(result.ends_with("..."));
        assert!(result.starts_with("[user] "));
    }

    #[test]
    fn test_build_flagged_text_preview_emoji() {
        // Emoji are 4 bytes each
        let text = "😀".repeat(200);
        let texts = vec![ExtractedText {
            text,
            message_index: Some(0),
            label: "user".to_string(),
            role: "user".to_string(),
        }];
        let result = build_flagged_text_preview(&texts);
        assert!(result.ends_with("..."));
    }

    #[test]
    fn test_build_flagged_text_preview_short_text() {
        let texts = vec![ExtractedText {
            text: "Hello".to_string(),
            message_index: Some(0),
            label: "user".to_string(),
            role: "user".to_string(),
        }];
        let result = build_flagged_text_preview(&texts);
        assert_eq!(result, "[user] Hello");
        assert!(!result.ends_with("..."));
    }

    #[test]
    fn test_build_flagged_text_preview_prefers_user() {
        let texts = vec![
            ExtractedText {
                text: "system prompt".to_string(),
                message_index: Some(0),
                label: "system".to_string(),
                role: "system".to_string(),
            },
            ExtractedText {
                text: "user message".to_string(),
                message_index: Some(1),
                label: "user".to_string(),
                role: "user".to_string(),
            },
        ];
        let result = build_flagged_text_preview(&texts);
        assert_eq!(result, "[user] user message");
    }

    // === validate_request tests ===

    fn make_request(model: &str) -> CompletionRequest {
        CompletionRequest {
            model: model.to_string(),
            prompt: PromptInput::Single("test".to_string()),
            max_tokens: None,
            temperature: None,
            top_p: None,
            n: None,
            stream: false,
            stream_options: None,
            stop: None,
            frequency_penalty: None,
            presence_penalty: None,
            logprobs: None,
            user: None,
        }
    }

    #[test]
    fn test_validate_request_valid() {
        let request = make_request("gpt-4");
        assert!(validate_request(&request).is_ok());
    }

    #[test]
    fn test_validate_request_empty_model() {
        let request = make_request("");
        assert!(validate_request(&request).is_err());
    }

    #[test]
    fn test_validate_request_temperature_out_of_range() {
        let mut request = make_request("gpt-4");
        request.temperature = Some(2.5);
        assert!(validate_request(&request).is_err());
    }

    #[test]
    fn test_validate_request_nan_temperature() {
        let mut request = make_request("gpt-4");
        request.temperature = Some(f32::NAN);
        assert!(validate_request(&request).is_err());
    }

    #[test]
    fn test_validate_request_n_zero() {
        let mut request = make_request("gpt-4");
        request.n = Some(0);
        assert!(validate_request(&request).is_err());
    }

    #[test]
    fn test_validate_request_n_streaming() {
        let mut request = make_request("gpt-4");
        request.n = Some(2);
        request.stream = true;
        assert!(validate_request(&request).is_err());
    }

    #[test]
    fn test_validate_request_nan_frequency_penalty() {
        let mut request = make_request("gpt-4");
        request.frequency_penalty = Some(f32::NAN);
        assert!(validate_request(&request).is_err());
    }

    #[test]
    fn test_validate_request_nan_presence_penalty() {
        let mut request = make_request("gpt-4");
        request.presence_penalty = Some(f32::NAN);
        assert!(validate_request(&request).is_err());
    }

    // === convert_prompt_to_messages tests ===

    #[test]
    fn test_convert_prompt_to_messages_single() {
        let prompt = PromptInput::Single("Hello".to_string());
        let messages = convert_prompt_to_messages(&prompt).unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].role, "user");
        assert_eq!(messages[0].content.as_text(), "Hello");
    }

    #[test]
    fn test_convert_prompt_to_messages_multiple() {
        let prompt = PromptInput::Multiple(vec!["Hello".to_string(), "World".to_string()]);
        let messages = convert_prompt_to_messages(&prompt).unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, "user");
        assert_eq!(messages[1].role, "user");
        assert_eq!(messages[0].content.as_text(), "Hello");
        assert_eq!(messages[1].content.as_text(), "World");
    }

    // === legacy_to_chat_completion_request tests ===
    //
    // These guard the CompletionRequest → ChatCompletionRequest
    // adapter that drives the shared pipeline for `/v1/completions`.
    // Regressions here would silently strip fields from legacy
    // requests after the migration to run_turn_pipeline.

    #[test]
    fn test_legacy_to_chat_preserves_sampling_params() {
        let req = CompletionRequest {
            model: "gpt-4".to_string(),
            prompt: PromptInput::Single("hi".to_string()),
            temperature: Some(0.7),
            top_p: Some(0.9),
            max_tokens: Some(256),
            n: Some(2),
            stream: false,
            stream_options: None,
            stop: Some(crate::types::StopSequence::Single("\n".to_string())),
            frequency_penalty: Some(0.5),
            presence_penalty: Some(-0.5),
            logprobs: Some(3),
            user: Some("user-123".to_string()),
        };
        let chat = legacy_to_chat_completion_request(&req).unwrap();
        assert_eq!(chat.model, "gpt-4");
        assert_eq!(chat.temperature, Some(0.7));
        assert_eq!(chat.top_p, Some(0.9));
        assert_eq!(chat.max_tokens, Some(256));
        assert_eq!(chat.n, Some(2));
        assert_eq!(chat.frequency_penalty, Some(0.5));
        assert_eq!(chat.presence_penalty, Some(-0.5));
        assert_eq!(chat.user.as_deref(), Some("user-123"));
        // legacy logprobs (Option<u32>) is intentionally dropped —
        // ChatCompletionRequest.logprobs is Option<bool> and the
        // legacy provider request didn't carry token-count logprobs
        // either. Document the deliberate gap.
        assert_eq!(chat.logprobs, None);
        assert_eq!(chat.top_logprobs, None);
    }

    #[test]
    fn test_legacy_to_chat_single_prompt_becomes_one_user_message() {
        let req = CompletionRequest {
            model: "m".to_string(),
            prompt: PromptInput::Single("answer this".to_string()),
            temperature: None,
            top_p: None,
            max_tokens: None,
            n: None,
            stream: false,
            stream_options: None,
            stop: None,
            frequency_penalty: None,
            presence_penalty: None,
            logprobs: None,
            user: None,
        };
        let chat = legacy_to_chat_completion_request(&req).unwrap();
        assert_eq!(chat.messages.len(), 1);
        assert_eq!(chat.messages[0].role, "user");
        assert!(matches!(
            &chat.messages[0].content,
            Some(crate::types::MessageContent::Text(t)) if t == "answer this"
        ));
    }

    #[test]
    fn test_legacy_to_chat_multi_prompt_becomes_multiple_messages() {
        // Note: this preserves the original behavior of
        // `convert_prompt_to_messages` — each prompt entry maps to
        // its own user message rather than a single multi-prompt
        // batched call. Behavior may not match OpenAI's legacy
        // semantics for `n` parallel completions over an array
        // prompt; matches what the pre-migration handler did.
        let req = CompletionRequest {
            model: "m".to_string(),
            prompt: PromptInput::Multiple(vec!["a".to_string(), "b".to_string(), "c".to_string()]),
            temperature: None,
            top_p: None,
            max_tokens: None,
            n: None,
            stream: false,
            stream_options: None,
            stop: None,
            frequency_penalty: None,
            presence_penalty: None,
            logprobs: None,
            user: None,
        };
        let chat = legacy_to_chat_completion_request(&req).unwrap();
        assert_eq!(chat.messages.len(), 3);
        for (i, expected) in ["a", "b", "c"].iter().enumerate() {
            assert_eq!(chat.messages[i].role, "user");
            assert!(matches!(
                &chat.messages[i].content,
                Some(crate::types::MessageContent::Text(t)) if t == expected
            ));
        }
    }

    #[test]
    fn test_legacy_to_chat_zeros_chat_only_fields() {
        // Fields present on ChatCompletionRequest but not legacy
        // CompletionRequest must be `None` after conversion. A
        // regression that defaulted these to `Some(...)` would
        // accidentally enable features the legacy client never asked
        // for (e.g. tools, response_format, top_k).
        let req = CompletionRequest {
            model: "m".to_string(),
            prompt: PromptInput::Single("x".to_string()),
            temperature: None,
            top_p: None,
            max_tokens: None,
            n: None,
            stream: false,
            stream_options: None,
            stop: None,
            frequency_penalty: None,
            presence_penalty: None,
            logprobs: None,
            user: None,
        };
        let chat = legacy_to_chat_completion_request(&req).unwrap();
        assert!(chat.tools.is_none());
        assert!(chat.tool_choice.is_none());
        assert!(chat.response_format.is_none());
        assert!(chat.top_k.is_none());
        assert!(chat.seed.is_none());
        assert!(chat.repetition_penalty.is_none());
        assert!(chat.parallel_tool_calls.is_none());
        assert!(chat.logit_bias.is_none());
        assert!(chat.service_tier.is_none());
        assert!(chat.store.is_none());
        assert!(chat.metadata.is_none());
        assert!(chat.modalities.is_none());
        assert!(chat.audio.is_none());
        assert!(chat.prediction.is_none());
        assert!(chat.reasoning_effort.is_none());
        assert!(chat.extensions.is_none());
        assert!(chat.max_completion_tokens.is_none());
    }
}
