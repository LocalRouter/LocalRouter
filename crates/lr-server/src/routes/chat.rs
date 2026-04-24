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

use super::finalize::{estimate_token_count, maybe_repair_json_content};
use super::helpers::get_client_with_strategy;
use crate::middleware::client_auth::ClientAuthContext;
use crate::middleware::error::{ApiErrorResponse, ApiResult};
use crate::state::{AppState, AuthContext, GenerationDetails};
use crate::types::{
    ChatCompletionChoice, ChatCompletionChunk, ChatCompletionChunkChoice, ChatCompletionLogprobs,
    ChatCompletionRequest, ChatCompletionResponse, ChatCompletionTokenLogprob, ChatMessage,
    ChunkDelta, MessageContent, TokenUsage, TopLogprob,
};
use lr_providers::{CompletionRequest as ProviderCompletionRequest, PreComputedRouting};

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
        (status = 400, description = "Bad request", body = crate::types::ErrorResponse),
        (status = 401, description = "Unauthorized", body = crate::types::ErrorResponse),
        (status = 429, description = "Rate limit exceeded", body = crate::types::ErrorResponse),
        (status = 500, description = "Internal server error", body = crate::types::ErrorResponse)
    ),
    security(
        ("bearer_auth" = [])
    )
)]
pub async fn chat_completions(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    client_auth: Option<Extension<ClientAuthContext>>,
    Json(mut request): Json<ChatCompletionRequest>,
) -> ApiResult<Response> {
    // Emit LLM request event to trigger tray icon indicator
    state.emit_event("llm-request", "chat");

    // Generate session ID for correlating monitor events
    let session_id = uuid::Uuid::new_v4().to_string();

    // Emit monitor event for traffic inspection
    let request_json = serde_json::to_value(&request).unwrap_or_default();
    let mut llm_guard = super::monitor_helpers::emit_llm_call(
        &state,
        client_auth.as_ref(),
        Some(&session_id),
        "/v1/chat/completions",
        &request.model,
        request.stream,
        &request_json,
    );

    // Record client activity for connection graph
    state.record_client_activity(&auth.api_key_id);

    // Validate request
    if let Err(e) = super::pipeline::validate_request(&request) {
        super::monitor_helpers::emit_validation_error(
            &state,
            client_auth.as_ref(),
            Some(&session_id),
            "/v1/chat/completions",
            e.error.error.param.as_deref(),
            &e.error.error.message,
            400,
        );
        return Err(llm_guard.capture_err(e));
    }

    super::pipeline::apply_model_access_checks(
        &state,
        &auth,
        client_auth.as_ref(),
        &session_id,
        &mut request,
        &mut llm_guard,
    )
    .await?;

    // Check rate limits first (reject early before spawning parallel work)
    if let Err(e) = super::pipeline::check_rate_limits(&state, &auth, &request).await {
        super::monitor_helpers::emit_rate_limit_event(
            &state,
            client_auth.as_ref(),
            Some(&session_id),
            "rate_limit_exceeded",
            "/v1/chat/completions",
            &e.error.error.message,
            429,
            None,
        );
        return Err(llm_guard.capture_err(e));
    }

    // Secret scanning: check outbound request for leaked secrets (before guardrails)
    super::pipeline::run_secret_scan_check(&state, client_auth.as_ref().map(|e| &e.0), &request)
        .await
        .map_err(|e| llm_guard.capture_err(e))?;

    // Start guardrail scan in parallel (only if safety engine is available)
    let guardrail_handle = if client_auth.is_some()
        && state
            .safety_engine
            .read()
            .as_ref()
            .is_some_and(|e| e.has_models())
    {
        let state_ref = state.clone();
        let client_ctx = client_auth.as_ref().map(|e| e.0.clone());
        let request_clone = request.clone();
        Some(tokio::spawn(async move {
            super::pipeline::run_guardrails_scan(&state_ref, client_ctx.as_ref(), &request_clone)
                .await
        }))
    } else {
        None
    };

    // Start prompt compression in parallel (only if compression is enabled)
    let compression_handle = if state.compression_service.read().is_some()
        && state.config_manager.get().prompt_compression.enabled
    {
        let state_ref = state.clone();
        let client_ctx = client_auth.as_ref().map(|e| e.0.clone());
        let request_clone = request.clone();
        Some(tokio::spawn(async move {
            super::pipeline::run_prompt_compression(&state_ref, client_ctx.as_ref(), &request_clone)
                .await
        }))
    } else {
        None
    };

    // Start RouteLLM classification in parallel (only for localrouter/auto)
    let routellm_handle = if request.model == "localrouter/auto" {
        spawn_routellm_classification(&state, client_auth.as_ref().map(|e| &e.0), &request)
    } else {
        None
    };

    // Await compression result (must complete before converting to provider format)
    let compression_result = if let Some(handle) = compression_handle {
        handle.await.map_err(|e| {
            llm_guard.capture_err(ApiErrorResponse::internal_error(format!(
                "Compression task failed: {}",
                e
            )))
        })?
    } else {
        Ok(None)
    };

    // Track compression tokens saved for cost calculation after pricing is available
    let mut compression_tokens_saved: u64 = 0;

    // Apply compression: replace request messages if compression succeeded
    if let Ok(Some(compressed)) = &compression_result {
        tracing::info!(
            "Prompt compressed: {} -> {} msgs, ~{:.0}% reduction ({}ms)",
            compressed.original_count,
            compressed.compressed_messages.len(),
            if compressed.original_tokens > 0 {
                100.0
                    - (compressed.compressed_tokens as f64 / compressed.original_tokens as f64
                        * 100.0)
            } else {
                0.0
            },
            compressed.duration_ms,
        );
        // Track tokens saved by compression
        if compressed.original_tokens > compressed.compressed_tokens {
            let saved = (compressed.original_tokens - compressed.compressed_tokens) as u64;
            state
                .metrics_collector
                .record_feature_event("feature_compression", saved, 0.0);
            compression_tokens_saved = saved;
        }
        request.messages = compressed
            .compressed_messages
            .iter()
            .map(|m| ChatMessage {
                role: m.role.clone(),
                content: Some(MessageContent::Text(m.content.clone())),
                name: None,
                tool_calls: None,
                tool_call_id: None,
                reasoning_content: None,
            })
            .collect();
    } else if let Err(e) = &compression_result {
        tracing::warn!("Prompt compression failed (continuing without): {}", e);
    }

    // Await RouteLLM classification result
    let routellm_result = if let Some(handle) = routellm_handle {
        handle.await.map_err(|e| {
            llm_guard.capture_err(ApiErrorResponse::internal_error(format!(
                "RouteLLM task failed: {}",
                e
            )))
        })?
    } else {
        None
    };

    // Convert to provider format (uses possibly-compressed messages)
    let mut provider_request = super::pipeline::convert_to_provider_request(&request)
        .map_err(|e| llm_guard.capture_err(e))?;

    // Inject pre-computed RouteLLM routing into provider request
    if let Some(routing) = routellm_result {
        provider_request.pre_computed_routing = Some(routing);
    }

    // Log request summary with all active features
    {
        let client_id_short = &auth.api_key_id[..8.min(auth.api_key_id.len())];
        let guardrails_active = guardrail_handle.is_some();
        let compression_active = compression_tokens_saved > 0;
        let routellm_active = provider_request.pre_computed_routing.is_some();
        let routellm_tier = provider_request.pre_computed_routing.as_ref().map(|r| {
            if r.is_strong {
                "strong"
            } else {
                "weak"
            }
        });
        let client_mode = client_auth
            .as_ref()
            .and_then(|ext| {
                state
                    .client_manager
                    .get_client(&ext.0.client_id)
                    .map(|c| c.client_mode.clone())
            })
            .unwrap_or_default();
        let is_mcp_via_llm = client_mode == lr_config::ClientMode::McpViaLlm;

        tracing::info!(
            "LLM request: client={}, model={}, stream={}, mode={:?}, guardrails={}, compression={}{}, routellm={}{}, mcp_via_llm={}",
            client_id_short,
            request.model,
            request.stream,
            client_mode,
            guardrails_active,
            compression_active,
            if compression_active { format!(" (saved {} tokens)", compression_tokens_saved) } else { String::new() },
            routellm_active,
            routellm_tier.map(|t| format!(" ({})", t)).unwrap_or_default(),
            is_mcp_via_llm,
        );
    }

    // MCP via LLM: intercept after compression + RouteLLM are applied.
    // Guardrails run in parallel with the LLM call when possible; the orchestrator
    // awaits the guardrail gate before executing tools or returning a response.
    if let Ok((ref client, _)) = get_client_with_strategy(&state, &auth.api_key_id) {
        if client.client_mode == lr_config::ClientMode::McpViaLlm {
            return handle_mcp_via_llm(
                state,
                auth,
                client_auth,
                request,
                provider_request,
                guardrail_handle,
                compression_tokens_saved,
                llm_guard.into_event_id(),
                session_id,
            )
            .await;
        }
    }

    // Emit transformed request event if any transformations were applied
    {
        let mut transformations = Vec::new();
        if compression_tokens_saved > 0 {
            transformations.push(format!(
                "compression (-{} tokens)",
                compression_tokens_saved
            ));
        }
        if provider_request.pre_computed_routing.is_some() {
            let tier = provider_request
                .pre_computed_routing
                .as_ref()
                .map(|r| if r.is_strong { "strong" } else { "weak" })
                .unwrap_or("unknown");
            transformations.push(format!("routellm ({})", tier));
        }
        if guardrail_handle.is_some() {
            transformations.push("guardrails".to_string());
        }
        if !transformations.is_empty() {
            let req_json = serde_json::to_value(&request).unwrap_or_default();
            super::monitor_helpers::update_llm_call_transformed(
                &state,
                llm_guard.event_id(),
                &req_json,
                transformations,
            );
        }
    }

    // Determine if we can run guardrails in parallel with the LLM request.
    // Parallel mode buffers the response until guardrails pass, reducing latency.
    // Falls back to sequential when request may cause side effects.
    let config = state.config_manager.get();
    let use_parallel = config.guardrails.parallel_guardrails && !has_side_effects(&request);

    if use_parallel {
        // Defuse the guard: parallel handler functions manage their own completion.
        let llm_event_id = llm_guard.into_event_id();

        // Parallel mode: pass guardrail handle to response handlers
        // They will start the LLM request immediately and await guardrails concurrently
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
        // Sequential mode: keep guard alive through guardrail checks.
        // If guardrails error, the guard's Drop auto-completes as Error.
        let guardrail_result = if let Some(handle) = guardrail_handle {
            handle
                .await
                .map_err(|e| {
                    llm_guard.capture_err(ApiErrorResponse::internal_error(format!(
                        "Guardrail check failed: {}",
                        e
                    )))
                })?
                .map_err(|e| llm_guard.capture_err(e))?
        } else {
            None
        };

        if let Some(check_result) = guardrail_result {
            super::pipeline::handle_guardrail_approval(
                &state,
                client_auth.as_ref().map(|e| &e.0),
                &request,
                check_result,
                "request",
            )
            .await
            .map_err(|e| llm_guard.capture_err(e))?;
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

/// Spawn RouteLLM classification as a parallel task.
/// Returns None if RouteLLM is not configured/enabled for this client.
fn spawn_routellm_classification(
    state: &AppState,
    client_context: Option<&ClientAuthContext>,
    request: &ChatCompletionRequest,
) -> Option<tokio::task::JoinHandle<Option<PreComputedRouting>>> {
    let client_id = &client_context?.client_id;
    let config = state.config_manager.get();
    let client = config.clients.iter().find(|c| c.id == *client_id)?;
    let strategy = config
        .strategies
        .iter()
        .find(|s| s.id == client.strategy_id)?;
    let auto_config = strategy.auto_config.as_ref()?;
    let routellm_config = auto_config.routellm_config.as_ref().filter(|c| c.enabled)?;
    let service = state.router.get_routellm_service()?.clone();
    let threshold = routellm_config.threshold;
    let request_clone = request.clone();
    let metrics_collector = state.metrics_collector.clone();

    Some(tokio::spawn(async move {
        let prompt = request_clone
            .messages
            .iter()
            .filter_map(|m| match &m.content {
                Some(MessageContent::Text(text)) => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");

        match service.predict_with_threshold(&prompt, threshold).await {
            Ok((is_strong, win_rate)) => {
                tracing::info!(
                    "RouteLLM classification: win_rate={:.3}, threshold={:.3}, selected={}",
                    win_rate,
                    threshold,
                    if is_strong { "strong" } else { "weak" }
                );
                // Track strong/weak classification for dashboard (persisted to metrics DB)
                if is_strong {
                    metrics_collector.record_feature_event("feature_routellm_strong", 0, 0.0);
                } else {
                    metrics_collector.record_feature_event("feature_routellm_weak", 0, 0.0);
                }
                Some(PreComputedRouting {
                    is_strong,
                    win_rate,
                })
            }
            Err(e) => {
                tracing::warn!("RouteLLM classification failed: {}", e);
                None
            }
        }
    }))
}

/// Check whether a request may cause side effects that require sequential guardrails.
///
/// Side effects include non-function tools (e.g. web_search_preview, code_interpreter)
/// and models that inherently perform web searches (e.g. Perplexity Sonar).
/// Used by both the standard flow and MCP via LLM to determine parallel vs sequential mode.
fn has_side_effects(request: &ChatCompletionRequest) -> bool {
    // Non-function tools can trigger real-world actions
    if let Some(tools) = &request.tools {
        if tools.iter().any(|t| t.tool_type != "function") {
            return true;
        }
    }
    // Perplexity Sonar models perform web searches inherently
    let model = request.model.to_lowercase();
    model.contains("sonar")
}

/// Handle MCP via LLM mode: intercept the request and run the agentic orchestrator.
///
/// MCP tools are transparently injected into the LLM request, tool calls are
/// executed server-side, and the conversation loops until the LLM produces a
/// final response. The client speaks only the OpenAI protocol.
///
/// Called after compression and RouteLLM are already applied to the request/provider_request.
/// Guardrails run in parallel with the LLM call when possible (no side effects from the
/// model itself). The orchestrator awaits the guardrail gate before executing any tools
/// or returning a response. Falls back to sequential when the model has side effects.
#[allow(clippy::too_many_arguments)]
async fn handle_mcp_via_llm(
    state: AppState,
    auth: AuthContext,
    client_auth: Option<Extension<ClientAuthContext>>,
    mut request: ChatCompletionRequest,
    provider_request: ProviderCompletionRequest,
    guardrail_handle: GuardrailHandle,
    _compression_tokens_saved: u64,
    llm_event_id: String,
    session_id: String,
) -> ApiResult<Response> {
    // Determine if we can run guardrails in parallel with the LLM call.
    // The LLM call itself is safe, but the orchestrator must await guardrails
    // before executing any tool calls or returning a response.
    // Falls back to sequential when the model has side effects (e.g. Perplexity Sonar).
    let config = state.config_manager.get();
    let use_parallel = config.guardrails.parallel_guardrails && !has_side_effects(&request);

    let guardrail_gate = if use_parallel {
        // Parallel mode: wrap guardrail processing into a gate task.
        // The orchestrator will await this after the LLM call returns.
        guardrail_handle.map(|handle| {
            let state = state.clone();
            let client_auth = client_auth.clone();
            let request = request.clone();
            tokio::spawn(async move {
                let result = handle
                    .await
                    .map_err(|e| format!("Guardrail check failed: {}", e))?
                    .map_err(|e| format!("{:?}", e))?;
                if let Some(check_result) = result {
                    super::pipeline::handle_guardrail_approval(
                        &state,
                        client_auth.as_ref().map(|e| &e.0),
                        &request,
                        check_result,
                        "request",
                    )
                    .await
                    .map_err(|e| format!("{:?}", e))?;
                }
                Ok(())
            })
        })
    } else {
        // Sequential mode: await guardrails before the LLM call starts
        if let Some(handle) = guardrail_handle {
            let guardrail_result = handle.await.map_err(|e| {
                ApiErrorResponse::internal_error(format!("Guardrail check failed: {}", e))
            })??;
            if let Some(check_result) = guardrail_result {
                super::pipeline::handle_guardrail_approval(
                    &state,
                    client_auth.as_ref().map(|e| &e.0),
                    &request,
                    check_result,
                    "request",
                )
                .await?;
            }
        }
        None
    };

    let generation_id = format!("gen-{}", Uuid::new_v4());
    let started_at = Instant::now();
    let created_at = Utc::now();

    // Get the client and strategy
    let (client, mcp_via_llm_strategy) = get_client_with_strategy(&state, &auth.api_key_id)
        .map_err(|_| ApiErrorResponse::internal_error("Client lookup failed"))?;

    // Compute allowed MCP servers respecting client's mcp_permissions
    let all_server_ids: Vec<String> = state
        .config_manager
        .get()
        .mcp_servers
        .iter()
        .map(|s| s.id.clone())
        .collect();

    let allowed_servers: Vec<String> = if client.mcp_permissions.global.is_enabled() {
        all_server_ids
    } else {
        all_server_ids
            .iter()
            .filter(|server_id| client.mcp_permissions.has_any_enabled_for_server(server_id))
            .cloned()
            .collect()
    };

    // Run model firewall with augmented request (MCP tools visible in popup)
    if request.model != "localrouter/auto" {
        // Pre-fetch MCP tool definitions so the firewall popup shows the full augmented request
        // Always wrap in Some so the is_mcp_via_llm flag is set even if pre-fetch fails
        let mcp_tools_json = Some(
            match state
                .mcp_via_llm_manager
                .list_tools_for_preview(state.mcp_gateway.clone(), &client, allowed_servers.clone())
                .await
            {
                Ok(tools) => tools,
                Err(e) => {
                    tracing::warn!(
                        "MCP via LLM: failed to pre-fetch tools for firewall popup: {}",
                        e
                    );
                    serde_json::json!([])
                }
            },
        );

        let mcp_strategy_permission = mcp_via_llm_strategy
            .auto_config
            .as_ref()
            .map(|ac| ac.permission.clone());

        let firewall_edits = super::pipeline::check_model_firewall_permission(
            &state,
            client_auth.as_ref().map(|e| &e.0),
            &request,
            mcp_tools_json,
            mcp_strategy_permission,
        )
        .await?;

        if let Some(edits) = firewall_edits {
            super::pipeline::apply_firewall_request_edits(&mut request, &edits);
        }
    }

    // Streaming: use multi-segment streaming orchestrator
    if request.stream {
        let model = provider_request.model.clone();

        let chunk_stream = state
            .mcp_via_llm_manager
            .handle_streaming_request(
                state.mcp_gateway.clone(),
                state.router.clone(),
                &client,
                provider_request,
                allowed_servers,
                guardrail_gate,
                Some(llm_event_id.clone()),
                Some(session_id.clone()),
                None,
            )
            .await
            .map_err(|e| {
                ApiErrorResponse::bad_gateway(format!("MCP via LLM streaming error: {}", e))
            })?;

        let created_timestamp = created_at.timestamp();
        let gen_id = generation_id.clone();

        // Track content and completion for generation tracking
        use parking_lot::Mutex;
        use std::sync::Arc;
        let content_accumulator = Arc::new(Mutex::new(String::new()));
        let finish_reason = Arc::new(Mutex::new(String::from("stop")));
        let (completion_tx, completion_rx) = tokio::sync::oneshot::channel::<()>();
        let completion_tx = Arc::new(Mutex::new(Some(completion_tx)));

        // Set up streaming JSON repair if enabled and response_format is JSON
        let streaming_repairer = {
            let is_json_format = matches!(
                &request.response_format,
                Some(crate::types::ResponseFormat::JsonObject { .. })
                    | Some(crate::types::ResponseFormat::JsonSchema { .. })
            );
            if is_json_format {
                let config = state.config_manager.get();
                let client_cfg = state.client_manager.get_client(&auth.api_key_id);
                let enabled = client_cfg
                    .as_ref()
                    .and_then(|c| c.json_repair.enabled)
                    .unwrap_or(config.json_repair.enabled);
                let syntax_repair = client_cfg
                    .as_ref()
                    .and_then(|c| c.json_repair.syntax_repair)
                    .unwrap_or(config.json_repair.syntax_repair);
                if enabled && syntax_repair {
                    Some(Arc::new(Mutex::new(
                        lr_json_repair::StreamingJsonRepairer::new(
                            None,
                            lr_json_repair::RepairOptions::default(),
                        ),
                    )))
                } else {
                    None
                }
            } else {
                None
            }
        };
        let streaming_repairer_map = streaming_repairer.clone();

        let content_accumulator_map = content_accumulator.clone();
        let finish_reason_map = finish_reason.clone();
        let completion_tx_map = completion_tx.clone();

        // Clones for generation tracking after stream completes
        let state_clone = state.clone();
        let auth_clone = auth.clone();
        let gen_id_clone = generation_id.clone();
        let model_clone = model.clone();
        let created_at_clone = created_at;
        let request_user = request.user.clone();
        let request_messages = request.messages.clone();
        let compression_tokens_saved = _compression_tokens_saved;

        // Map provider chunks to SSE events, then append [DONE] sentinel
        let data_stream = chunk_stream.map(
            move |chunk_result| -> Result<Event, std::convert::Infallible> {
                match chunk_result {
                    Ok(provider_chunk) => {
                        // Track content for token estimation
                        let is_done = if let Some(choice) = provider_chunk.choices.first() {
                            if let Some(content) = &choice.delta.content {
                                content_accumulator_map.lock().push_str(content);
                            }
                            if let Some(reason) = &choice.finish_reason {
                                *finish_reason_map.lock() = reason.clone();
                                true
                            } else {
                                false
                            }
                        } else {
                            false
                        };

                        if is_done {
                            if let Some(tx) = completion_tx_map.lock().take() {
                                let _ = tx.send(());
                            }
                        }

                        let api_chunk = ChatCompletionChunk {
                            id: gen_id.clone(),
                            object: "chat.completion.chunk".to_string(),
                            created: created_timestamp,
                            model: provider_chunk.model.clone(),
                            choices: {
                                let mut choices: Vec<ChatCompletionChunkChoice> = provider_chunk
                                    .choices
                                    .into_iter()
                                    .map(|c| ChatCompletionChunkChoice {
                                        index: c.index,
                                        delta: ChunkDelta {
                                            role: c.delta.role,
                                            content: c.delta.content,
                                            tool_calls: c.delta.tool_calls.map(|tcs| {
                                                tcs.into_iter()
                                                    .map(|tc| crate::types::ToolCallDelta {
                                                        index: tc.index,
                                                        id: tc.id,
                                                        tool_type: tc.tool_type,
                                                        function: tc.function.map(|f| {
                                                            crate::types::FunctionCallDelta {
                                                                name: f.name,
                                                                arguments: f.arguments,
                                                            }
                                                        }),
                                                    })
                                                    .collect()
                                            }),
                                            reasoning_content: c.delta.reasoning_content,
                                        },
                                        finish_reason: c.finish_reason,
                                    })
                                    .collect();

                                // Apply streaming JSON repair
                                if let Some(ref repairer) = streaming_repairer_map {
                                    for choice in &mut choices {
                                        if let Some(text) = choice.delta.content.take() {
                                            let repaired = repairer.lock().push_content(&text);
                                            if !repaired.is_empty() {
                                                choice.delta.content = Some(repaired);
                                            }
                                        }
                                        if choice.finish_reason.is_some() {
                                            let flushed = repairer.lock().finish();
                                            if !flushed.is_empty() {
                                                let existing =
                                                    choice.delta.content.take().unwrap_or_default();
                                                choice.delta.content =
                                                    Some(format!("{}{}", existing, flushed));
                                            }
                                        }
                                    }
                                }

                                choices
                            },
                            usage: None,
                            system_fingerprint: None,
                            service_tier: None,
                            request_usage_entries: None,
                        };
                        let json = serde_json::to_string(&api_chunk).unwrap_or_default();
                        Ok(Event::default().data(json))
                    }
                    Err(e) => {
                        if let Some(tx) = completion_tx_map.lock().take() {
                            let _ = tx.send(());
                        }
                        let error_response = serde_json::json!({
                            "error": {
                                "message": format!("MCP via LLM streaming error: {}", e),
                                "type": "server_error",
                                "code": "streaming_error"
                            }
                        });
                        Ok(Event::default()
                            .data(serde_json::to_string(&error_response).unwrap_or_default()))
                    }
                }
            },
        );

        // Record generation details after stream completes.
        //
        // Note: `skip_monitor_completion: true` on `FinalizeInputs`
        // tells the shared helper to skip `complete_llm_call` +
        // `update_llm_call_response_body` — the MCP-via-LLM
        // orchestrator (orchestrator_stream.rs) completes those
        // per-iteration with full tool-call metadata, and
        // overwriting them here with a text-only view would lose
        // fidelity. All other telemetry (cost / metrics / tray /
        // access log / `metrics-updated` event / generation
        // tracker) still fires.
        tokio::spawn(async move {
            let _ =
                tokio::time::timeout(tokio::time::Duration::from_secs(300), completion_rx).await;

            let completion_content = content_accumulator.lock().clone();
            let finish_reason_final = finish_reason.lock().clone();

            let prompt_tokens = request_messages
                .last()
                .map(|m| estimate_token_count(std::slice::from_ref(m)) as u32)
                .unwrap_or(0);
            let completion_tokens = (completion_content.len() / 4).max(1) as u32;

            let provider = if let Some((p, _)) = model_clone.split_once('/') {
                p.to_string()
            } else {
                "router".to_string()
            };

            let wire_body = super::monitor_helpers::build_streaming_response_body(
                &gen_id_clone,
                &model_clone,
                &completion_content,
                &finish_reason_final,
                prompt_tokens as u64,
                completion_tokens as u64,
                created_at_clone.timestamp(),
            );

            let finalize_inputs = super::finalize::FinalizeInputs {
                state: &state_clone,
                auth: &auth_clone,
                llm_event_id: &llm_event_id,
                generation_id: &gen_id_clone,
                started_at,
                created_at: created_at_clone,
                incremental_prompt_tokens: prompt_tokens,
                compression_tokens_saved,
                routing_metadata: None,
                user: request_user,
                streamed: true,
                skip_monitor_completion: true,
            };
            super::finalize::finalize_streaming_at_end(
                &finalize_inputs,
                super::finalize::StreamingFinalizeSummary {
                    provider,
                    model: model_clone,
                    prompt_tokens,
                    completion_tokens,
                    reasoning_tokens: None,
                    finish_reason: Some(finish_reason_final),
                    content_preview: completion_content,
                },
                &wire_body,
            )
            .await;
        });

        // Append [DONE] sentinel after all data chunks (required by OpenAI streaming protocol)
        let done_stream = futures::stream::once(async {
            Ok::<Event, std::convert::Infallible>(Event::default().data("[DONE]"))
        });
        let sse_stream = data_stream.chain(done_stream);

        return Ok(Sse::new(sse_stream)
            .keep_alive(KeepAlive::default())
            .into_response());
    }

    // Non-streaming: run the agentic orchestrator. Chat completions
    // don't supply a deterministic session key — the orchestrator
    // falls back to hash-based matching.
    let response = state
        .mcp_via_llm_manager
        .handle_request(
            state.mcp_gateway.clone(),
            &state.router,
            &client,
            provider_request,
            allowed_servers,
            guardrail_gate,
            Some(llm_event_id.clone()),
            Some(session_id.clone()),
            None,
        )
        .await
        .map_err(|e| ApiErrorResponse::bad_gateway(format!("MCP via LLM error: {}", e)))?;

    let completed_at = Instant::now();
    let latency_ms = completed_at.duration_since(started_at).as_millis() as u64;

    // Emit monitor response event
    {
        let content_preview = response
            .choices
            .first()
            .map(|c| match &c.message.content {
                lr_providers::ChatMessageContent::Text(t) => t.clone(),
                lr_providers::ChatMessageContent::Parts(parts) => parts
                    .iter()
                    .filter_map(|p| match p {
                        lr_providers::ContentPart::Text { text } => Some(text.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join(""),
            })
            .unwrap_or_default();
        let finish_reason = response
            .choices
            .first()
            .and_then(|c| c.finish_reason.as_deref());
        let reasoning_tokens = response
            .usage
            .completion_tokens_details
            .as_ref()
            .and_then(|d| d.reasoning_tokens.or(d.thinking_tokens))
            .map(|t| t as u64);
        super::monitor_helpers::complete_llm_call(
            &state,
            &llm_event_id,
            &response.provider,
            &response.model,
            200,
            response.usage.prompt_tokens as u64,
            response.usage.completion_tokens as u64,
            reasoning_tokens,
            None,
            latency_ms,
            finish_reason,
            &content_preview,
            false,
        );
    }

    // Convert provider response to server response
    let api_response = ChatCompletionResponse {
        id: generation_id.clone(),
        object: "chat.completion".to_string(),
        created: created_at.timestamp(),
        model: response.model.clone(),
        choices: response
            .choices
            .iter()
            .map(|choice| {
                let tool_calls = choice.message.tool_calls.as_ref().map(|provider_tools| {
                    provider_tools
                        .iter()
                        .map(|tool_call| crate::types::ToolCall {
                            id: tool_call.id.clone(),
                            tool_type: tool_call.tool_type.clone(),
                            function: crate::types::FunctionCall {
                                name: tool_call.function.name.clone(),
                                arguments: tool_call.function.arguments.clone(),
                            },
                        })
                        .collect()
                });

                let content = match &choice.message.content {
                    lr_providers::ChatMessageContent::Text(t) if t.is_empty() => None,
                    lr_providers::ChatMessageContent::Text(t) => {
                        Some(MessageContent::Text(t.clone()))
                    }
                    lr_providers::ChatMessageContent::Parts(parts) => {
                        let server_parts: Vec<crate::types::ContentPart> = parts
                            .iter()
                            .map(|part| match part {
                                lr_providers::ContentPart::Text { text } => {
                                    crate::types::ContentPart::Text { text: text.clone() }
                                }
                                lr_providers::ContentPart::ImageUrl { image_url } => {
                                    crate::types::ContentPart::ImageUrl {
                                        image_url: crate::types::ImageUrl {
                                            url: image_url.url.clone(),
                                            detail: image_url.detail.clone(),
                                        },
                                    }
                                }
                            })
                            .collect();
                        Some(MessageContent::Parts(server_parts))
                    }
                };

                ChatCompletionChoice {
                    index: choice.index,
                    message: ChatMessage {
                        role: choice.message.role.clone(),
                        content,
                        name: choice.message.name.clone(),
                        tool_calls,
                        tool_call_id: choice.message.tool_call_id.clone(),
                        reasoning_content: choice.message.reasoning_content.clone(),
                    },
                    finish_reason: choice.finish_reason.clone(),
                    logprobs: None,
                }
            })
            .collect(),
        usage: TokenUsage {
            prompt_tokens: response.usage.prompt_tokens,
            completion_tokens: response.usage.completion_tokens,
            total_tokens: response.usage.total_tokens,
            prompt_tokens_details: response.usage.prompt_tokens_details.clone(),
            completion_tokens_details: response.usage.completion_tokens_details.clone(),
        },
        system_fingerprint: response.system_fingerprint.clone(),
        service_tier: response.service_tier.clone(),
        extensions: response.extensions.clone(),
        request_usage_entries: response.request_usage_entries.as_ref().map(|entries| {
            entries
                .iter()
                .map(|e| TokenUsage {
                    prompt_tokens: e.prompt_tokens,
                    completion_tokens: e.completion_tokens,
                    total_tokens: e.total_tokens,
                    prompt_tokens_details: e.prompt_tokens_details.clone(),
                    completion_tokens_details: e.completion_tokens_details.clone(),
                })
                .collect()
        }),
    };

    // Store full response body in monitor event for inspection
    if let Ok(response_json) = serde_json::to_value(&api_response) {
        super::monitor_helpers::update_llm_call_response_body(
            &state,
            &llm_event_id,
            &response_json,
        );
    }

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
        cost: None,
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

    // Record tokens for tray graph
    if let Some(recorder) = state.tray_graph_manager.read().as_ref() {
        recorder.record_tokens(response.usage.total_tokens as u64);
    }

    Ok(Json(api_response).into_response())
}

/// Type alias for a spawned guardrail scan task
type GuardrailHandle =
    Option<tokio::task::JoinHandle<ApiResult<Option<lr_guardrails::SafetyCheckResult>>>>;

/// Handle non-streaming chat completion with parallel guardrails.
/// Starts LLM request immediately and awaits guardrails concurrently.
#[allow(clippy::too_many_arguments)]
async fn handle_non_streaming_parallel(
    state: AppState,
    auth: AuthContext,
    client_auth: Option<Extension<ClientAuthContext>>,
    request: ChatCompletionRequest,
    provider_request: ProviderCompletionRequest,
    guardrail_handle: GuardrailHandle,
    compression_tokens_saved: u64,
    llm_event_id: String,
) -> ApiResult<Response> {
    let generation_id = format!("gen-{}", Uuid::new_v4());
    let started_at = Instant::now();
    let created_at = Utc::now();

    // Start LLM request immediately (don't wait for guardrails)
    // Clone for potential paid fallback retry after free-tier exhaustion
    let provider_request_fallback = provider_request.clone();
    let llm_handle = {
        let router = state.router.clone();
        let api_key_id = auth.api_key_id.clone();
        tokio::spawn(async move { router.complete(&api_key_id, provider_request).await })
    };

    // Wait for guardrail scan (if spawned) and LLM response concurrently
    let (guardrail_result, llm_result) = if let Some(handle) = guardrail_handle {
        let (g, l) = tokio::join!(handle, llm_handle);
        (Some(g), l)
    } else {
        (None, llm_handle.await)
    };

    // Process guardrail result first — if denied, discard LLM response
    if let Some(guardrail_res) = guardrail_result {
        let guardrail_result = guardrail_res.map_err(|e| {
            ApiErrorResponse::internal_error(format!("Guardrail check failed: {}", e))
        })??;

        if let Some(check_result) = guardrail_result {
            super::pipeline::handle_guardrail_approval(
                &state,
                client_auth.as_ref().map(|e| &e.0),
                &request,
                check_result,
                "request",
            )
            .await?;
        }
    }

    // Guardrails passed — unwrap LLM response
    let llm_result = llm_result
        .map_err(|e| ApiErrorResponse::internal_error(format!("LLM request failed: {}", e)))?;

    let (response, routing_metadata) = match llm_result {
        Ok((resp, meta)) => (resp, meta),
        Err(lr_types::AppError::FreeTierFallbackAvailable {
            retry_after_secs,
            exhausted_models,
        }) => {
            check_free_tier_fallback(
                &state,
                &auth.api_key_id,
                &exhausted_models,
                retry_after_secs,
            )
            .await?;
            state
                .router
                .complete_with_paid_fallback(&auth.api_key_id, provider_request_fallback)
                .await
                .map_err(|e| ApiErrorResponse::bad_gateway(format!("Provider error: {}", e)))?
        }
        Err(e) => {
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
                502,
            ) {
                tracing::warn!("Failed to write access log: {}", log_err);
            }
            super::monitor_helpers::complete_llm_call_error(
                &state,
                &llm_event_id,
                "unknown",
                "unknown",
                502,
                &e.to_string(),
            );
            return Err(ApiErrorResponse::bad_gateway(format!(
                "Provider error: {}",
                e
            )));
        }
    };

    // Reuse shared response-building logic
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

/// Check if a free-tier fallback should be allowed, denied, or needs approval.
/// Returns Ok(()) if the request should proceed with paid models.
async fn check_free_tier_fallback(
    state: &AppState,
    client_id: &str,
    exhausted_models: &[(String, String)],
    _retry_after_secs: u64,
) -> ApiResult<()> {
    use lr_mcp::gateway::access_control::{
        check_needs_approval, FirewallCheckContext, FirewallCheckResult,
    };
    use lr_mcp::gateway::firewall::FirewallApprovalAction;

    // Look up strategy to get fallback_mode
    let (_client, strategy) = super::helpers::get_client_with_strategy(state, client_id)
        .map_err(|_| ApiErrorResponse::rate_limited("Free tier exhausted"))?;

    let has_time_based = state
        .free_tier_approval_tracker
        .has_valid_approval(client_id);

    let ctx = FirewallCheckContext::FreeTierFallback {
        fallback_mode: &strategy.free_tier_fallback,
        has_time_based_approval: has_time_based,
    };

    match check_needs_approval(&ctx) {
        FirewallCheckResult::Allow => Ok(()),
        FirewallCheckResult::Deny => Err(ApiErrorResponse::payment_required(
            "Free tier exhausted. Paid fallback is disabled.",
        )),
        FirewallCheckResult::Ask => {
            // Build exhausted summary for popup
            let summary = exhausted_models
                .iter()
                .map(|(p, m)| format!("{}/{}", p, m))
                .collect::<Vec<_>>()
                .join(", ");

            let client_name = state
                .client_manager
                .get_client(client_id)
                .map(|c| c.name.clone())
                .unwrap_or_else(|| client_id.to_string());

            // Get the firewall manager from the MCP gateway
            let firewall_manager = &state.mcp_gateway.firewall_manager;

            let response = firewall_manager
                .request_free_tier_fallback_approval(client_id.to_string(), client_name, summary)
                .await
                .map_err(|e| {
                    ApiErrorResponse::internal_error(format!(
                        "Failed to request free-tier fallback approval: {}",
                        e
                    ))
                })?;

            match response.action {
                FirewallApprovalAction::AllowOnce | FirewallApprovalAction::AllowSession => Ok(()),
                FirewallApprovalAction::Allow1Minute => {
                    state
                        .free_tier_approval_tracker
                        .add_1_minute_approval(client_id);
                    Ok(())
                }
                FirewallApprovalAction::Allow1Hour => {
                    state
                        .free_tier_approval_tracker
                        .add_1_hour_approval(client_id);
                    Ok(())
                }
                FirewallApprovalAction::AllowPermanent => {
                    // Config update (free_tier_fallback = Allow) is handled by
                    // submit_firewall_approval; just allow this request through
                    Ok(())
                }
                _ => Err(ApiErrorResponse::payment_required(
                    "Free tier exhausted. Paid fallback denied by user.",
                )),
            }
        }
    }
}

/// Handle non-streaming chat completion (sequential guardrails — already resolved)
async fn handle_non_streaming(
    state: AppState,
    auth: AuthContext,
    client_auth: Option<Extension<ClientAuthContext>>,
    request: ChatCompletionRequest,
    provider_request: ProviderCompletionRequest,
    compression_tokens_saved: u64,
    llm_event_id: String,
) -> ApiResult<Response> {
    let generation_id = format!("gen-{}", Uuid::new_v4());
    let started_at = Instant::now();
    let created_at = Utc::now();

    // Call router to get completion
    let (response, routing_metadata) = match state
        .router
        .complete(&auth.api_key_id, provider_request.clone())
        .await
    {
        Ok(result) => result,
        Err(lr_types::AppError::FreeTierFallbackAvailable {
            retry_after_secs,
            exhausted_models,
        }) => {
            // Free-tier exhausted but fallback available — check approval
            check_free_tier_fallback(
                &state,
                &auth.api_key_id,
                &exhausted_models,
                retry_after_secs,
            )
            .await?;
            // Approved — retry with paid models
            state
                .router
                .complete_with_paid_fallback(&auth.api_key_id, provider_request)
                .await
                .map_err(|e| ApiErrorResponse::bad_gateway(format!("Provider error: {}", e)))?
        }
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

            // Emit monitor error event
            super::monitor_helpers::complete_llm_call_error(
                &state,
                &llm_event_id,
                "unknown",
                &request.model,
                502,
                &e.to_string(),
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

/// Build the non-streaming response from a completed provider response.
/// Shared by both sequential and parallel handlers.
#[allow(clippy::too_many_arguments)]
async fn build_non_streaming_response(
    state: AppState,
    auth: AuthContext,
    _client_auth: Option<Extension<ClientAuthContext>>,
    request: ChatCompletionRequest,
    response: lr_providers::CompletionResponse,
    generation_id: String,
    started_at: Instant,
    created_at: chrono::DateTime<Utc>,
    compression_tokens_saved: u64,
    llm_event_id: String,
    routing_metadata: Option<serde_json::Value>,
) -> ApiResult<Response> {
    // For chat messages, calculate incremental token count (last message only)
    // instead of cumulative (all conversation history).
    let incremental_prompt_tokens = if let Some(last_msg) = request.messages.last() {
        estimate_token_count(std::slice::from_ref(last_msg)) as u32
    } else {
        response.usage.prompt_tokens
    };

    // Shared finalize: cost, metrics, tray graph, access log,
    // `metrics-updated` tray event, `update_llm_call_routing` +
    // `complete_llm_call`. Lives in `routes/finalize.rs` so the three
    // LLM endpoints (`/v1/chat/completions`, `/v1/responses`,
    // `/v1/completions`) emit identical telemetry.
    let finalize_inputs = super::finalize::FinalizeInputs {
        state: &state,
        auth: &auth,
        llm_event_id: &llm_event_id,
        generation_id: &generation_id,
        started_at,
        created_at,
        incremental_prompt_tokens,
        compression_tokens_saved,
        routing_metadata: routing_metadata.as_ref(),
        user: request.user.clone(),
        streamed: false,
        skip_monitor_completion: false,
    };
    let metrics = super::finalize::finalize_metrics_and_monitor(&finalize_inputs, &response).await;

    // Note: Router already records usage for rate limiting, so we don't need to do it here

    // Convert provider response to API response.
    //
    // We borrow `response` rather than consuming it because the
    // shared finalize tail below still needs it for generation-row
    // metadata (provider / model / usage). Cloning the choices vec
    // is cheap — the heavy fields (content, tool_calls) are typically
    // a few KB each.
    let api_response = ChatCompletionResponse {
        id: generation_id.clone(),
        object: "chat.completion".to_string(),
        created: created_at.timestamp(),
        model: response.model.clone(),
        choices: response
            .choices
            .clone()
            .into_iter()
            .map(|choice| {
                // Convert provider message content to server message content
                let content = match choice.message.content {
                    lr_providers::ChatMessageContent::Text(text) => {
                        if text.is_empty() && choice.message.tool_calls.is_some() {
                            // If content is empty and we have tool calls, content can be None
                            None
                        } else {
                            // Apply JSON repair if enabled and response_format is JSON
                            let text = maybe_repair_json_content(text, &request, &state, &auth);
                            Some(MessageContent::Text(text))
                        }
                    }
                    lr_providers::ChatMessageContent::Parts(parts) => {
                        // Convert provider parts to server parts
                        let server_parts: Vec<crate::types::ContentPart> = parts
                            .into_iter()
                            .map(|part| match part {
                                lr_providers::ContentPart::Text { text } => {
                                    crate::types::ContentPart::Text { text }
                                }
                                lr_providers::ContentPart::ImageUrl { image_url } => {
                                    crate::types::ContentPart::ImageUrl {
                                        image_url: crate::types::ImageUrl {
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
                        .map(|tool_call| crate::types::ToolCall {
                            id: tool_call.id,
                            tool_type: tool_call.tool_type,
                            function: crate::types::FunctionCall {
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
                        reasoning_content: choice.message.reasoning_content,
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
            prompt_tokens_details: response.usage.prompt_tokens_details.clone(),
            completion_tokens_details: response.usage.completion_tokens_details.clone(),
        },
        system_fingerprint: response.system_fingerprint.clone(),
        service_tier: response.service_tier.clone(),
        extensions: None, // Provider-specific extensions (Phase 1)
        request_usage_entries: response.request_usage_entries.as_ref().map(|entries| {
            entries
                .iter()
                .map(|e| TokenUsage {
                    prompt_tokens: e.prompt_tokens,
                    completion_tokens: e.completion_tokens,
                    total_tokens: e.total_tokens,
                    prompt_tokens_details: e.prompt_tokens_details.clone(),
                    completion_tokens_details: e.completion_tokens_details.clone(),
                })
                .collect()
        }),
    };

    // Shared finalize tail: stash the wire-format body on the
    // `LlmCall` monitor event and record a `GenerationDetails` row
    // with pricing-derived cost breakdown.
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

/// Handle streaming chat completion
async fn handle_streaming(
    state: AppState,
    auth: AuthContext,
    _client_auth: Option<Extension<ClientAuthContext>>,
    request: ChatCompletionRequest,
    provider_request: ProviderCompletionRequest,
    compression_tokens_saved: u64,
    llm_event_id: String,
) -> ApiResult<Response> {
    let generation_id = format!("gen-{}", Uuid::new_v4());
    let created_at = Utc::now();
    let started_at = Instant::now();

    // Clone model before moving provider_request
    let model = provider_request.model.clone();

    // Call router to get streaming completion
    let (stream, routing_metadata) = match state
        .router
        .stream_complete(&auth.api_key_id, provider_request.clone())
        .await
    {
        Ok(result) => result,
        Err(lr_types::AppError::FreeTierFallbackAvailable {
            retry_after_secs,
            exhausted_models,
        }) => {
            // Free-tier exhausted but fallback available — check approval
            check_free_tier_fallback(
                &state,
                &auth.api_key_id,
                &exhausted_models,
                retry_after_secs,
            )
            .await?;
            // Approved — retry with paid models
            state
                .router
                .stream_complete_with_paid_fallback(&auth.api_key_id, provider_request)
                .await
                .map_err(|e| ApiErrorResponse::bad_gateway(format!("Provider error: {}", e)))?
        }
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

            // Emit monitor error event
            super::monitor_helpers::complete_llm_call_error(
                &state,
                &llm_event_id,
                "unknown",
                &model,
                502,
                &e.to_string(),
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

    // Update monitor event with routing info if auto-routing was used
    if let Some(ref meta) = routing_metadata {
        super::monitor_helpers::update_llm_call_routing(&state, &llm_event_id, meta);
    }

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

    // Set up streaming JSON repair if enabled and response_format is JSON
    let streaming_repairer = {
        let is_json_format = matches!(
            &request.response_format,
            Some(crate::types::ResponseFormat::JsonObject { .. })
                | Some(crate::types::ResponseFormat::JsonSchema { .. })
        );
        if is_json_format {
            let config = state.config_manager.get();
            let client = state.client_manager.get_client(&auth.api_key_id);
            let enabled = client
                .as_ref()
                .and_then(|c| c.json_repair.enabled)
                .unwrap_or(config.json_repair.enabled);
            let syntax_repair = client
                .as_ref()
                .and_then(|c| c.json_repair.syntax_repair)
                .unwrap_or(config.json_repair.syntax_repair);
            if enabled && syntax_repair {
                Some(Arc::new(Mutex::new(
                    lr_json_repair::StreamingJsonRepairer::new(
                        None,
                        lr_json_repair::RepairOptions::default(),
                    ),
                )))
            } else {
                None
            }
        } else {
            None
        }
    };
    let streaming_repairer_map = streaming_repairer.clone();

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
                        model: provider_chunk.model.clone(),
                        choices: {
                            let mut choices: Vec<ChatCompletionChunkChoice> = provider_chunk
                                .choices
                                .into_iter()
                                .map(|choice| {
                                    // Convert provider tool_calls delta to server tool_calls delta
                                    let tool_calls =
                                        choice.delta.tool_calls.map(|provider_deltas| {
                                            provider_deltas
                                                .into_iter()
                                                .map(|delta| crate::types::ToolCallDelta {
                                                    index: delta.index,
                                                    id: delta.id,
                                                    tool_type: delta.tool_type,
                                                    function: delta.function.map(|f| {
                                                        crate::types::FunctionCallDelta {
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
                                            reasoning_content: choice.delta.reasoning_content,
                                        },
                                        finish_reason: choice.finish_reason,
                                    }
                                })
                                .collect();

                            // Apply streaming JSON repair outside the map closure
                            if let Some(ref repairer) = streaming_repairer_map {
                                for choice in &mut choices {
                                    if let Some(text) = choice.delta.content.take() {
                                        let repaired = repairer.lock().push_content(&text);
                                        if !repaired.is_empty() {
                                            choice.delta.content = Some(repaired);
                                        }
                                    }
                                    if choice.finish_reason.is_some() {
                                        let flushed = repairer.lock().finish();
                                        if !flushed.is_empty() {
                                            let existing =
                                                choice.delta.content.take().unwrap_or_default();
                                            choice.delta.content =
                                                Some(format!("{}{}", existing, flushed));
                                        }
                                    }
                                }
                            }

                            choices
                        },
                        usage: None,
                        system_fingerprint: None,
                        service_tier: None,
                        request_usage_entries: None,
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

        let completion_content = content_accumulator.lock().clone();
        let finish_reason_final = finish_reason.lock().clone();

        // Estimate tokens for this message only (not the entire
        // conversation). Chat stream tokens are rough estimates —
        // ~4 chars / token for the accumulated output, and only the
        // last user message counts for prompt (no cumulative history
        // billing).
        let prompt_tokens = request_messages
            .last()
            .map(|m| estimate_token_count(std::slice::from_ref(m)) as u32)
            .unwrap_or(0);
        let completion_tokens = (completion_content.len() / 4).max(1) as u32;

        let provider = if let Some((p, _)) = model_clone.split_once('/') {
            p.to_string()
        } else {
            "router".to_string()
        };

        let wire_body = super::monitor_helpers::build_streaming_response_body(
            &gen_id_clone,
            &model_clone,
            &completion_content,
            &finish_reason_final,
            prompt_tokens as u64,
            completion_tokens as u64,
            created_at_clone.timestamp(),
        );

        let finalize_inputs = super::finalize::FinalizeInputs {
            state: &state_clone,
            auth: &auth_clone,
            llm_event_id: &llm_event_id,
            generation_id: &gen_id_clone,
            started_at,
            created_at: created_at_clone,
            incremental_prompt_tokens: prompt_tokens,
            compression_tokens_saved,
            routing_metadata: None,
            user: request_user,
            streamed: true,
            skip_monitor_completion: false,
        };
        super::finalize::finalize_streaming_at_end(
            &finalize_inputs,
            super::finalize::StreamingFinalizeSummary {
                provider,
                model: model_clone,
                prompt_tokens,
                completion_tokens,
                reasoning_tokens: None,
                finish_reason: Some(finish_reason_final),
                content_preview: completion_content,
            },
            &wire_body,
        )
        .await;
    });

    Ok(Sse::new(sse_stream)
        .keep_alive(KeepAlive::default())
        .into_response())
}

/// Handle streaming chat completion with parallel guardrails.
/// Buffers SSE events until guardrails resolve, then flushes or aborts.
#[allow(clippy::too_many_arguments)]
async fn handle_streaming_parallel(
    state: AppState,
    auth: AuthContext,
    client_auth: Option<Extension<ClientAuthContext>>,
    request: ChatCompletionRequest,
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

    // Start LLM streaming request immediately
    let (stream, routing_metadata) = match state
        .router
        .stream_complete(&auth.api_key_id, provider_request.clone())
        .await
    {
        Ok(result) => result,
        Err(lr_types::AppError::FreeTierFallbackAvailable {
            retry_after_secs,
            exhausted_models,
        }) => {
            check_free_tier_fallback(
                &state,
                &auth.api_key_id,
                &exhausted_models,
                retry_after_secs,
            )
            .await?;
            state
                .router
                .stream_complete_with_paid_fallback(&auth.api_key_id, provider_request)
                .await
                .map_err(|e| ApiErrorResponse::bad_gateway(format!("Provider error: {}", e)))?
        }
        Err(e) => {
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
                502,
            ) {
                tracing::warn!("Failed to write access log: {}", log_err);
            }
            super::monitor_helpers::complete_llm_call_error(
                &state,
                &llm_event_id,
                "unknown",
                &model,
                502,
                &e.to_string(),
            );
            return Err(ApiErrorResponse::bad_gateway(format!(
                "Provider error: {}",
                e
            )));
        }
    };

    // Update monitor event with routing info if auto-routing was used
    if let Some(ref meta) = routing_metadata {
        super::monitor_helpers::update_llm_call_routing(&state, &llm_event_id, meta);
    }

    // Guardrail gate: signals whether the response should be flushed or denied
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum GuardrailGate {
        Pending,
        Passed,
        Denied,
    }

    let (gate_tx, gate_rx) = watch::channel(GuardrailGate::Pending);

    // Output channel: SSE events sent to the client
    let (event_tx, event_rx) = mpsc::channel::<Result<Event, std::convert::Infallible>>(256);

    // Spawn guardrail resolver
    if let Some(handle) = guardrail_handle {
        let state = state.clone();
        let client_auth = client_auth.clone();
        let request = request.clone();
        tokio::spawn(async move {
            let result = handle.await;
            match result {
                Ok(Ok(None)) => {
                    // Safe — no violations
                    let _ = gate_tx.send(GuardrailGate::Passed);
                }
                Ok(Ok(Some(check_result))) => {
                    // Violations found — request approval
                    match super::pipeline::handle_guardrail_approval(
                        &state,
                        client_auth.as_ref().map(|e| &e.0),
                        &request,
                        check_result,
                        "request",
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
                    // Error in guardrails — fail open
                    tracing::warn!("Guardrail check failed, failing open");
                    let _ = gate_tx.send(GuardrailGate::Passed);
                }
            }
        });
    } else {
        // No guardrails — pass immediately
        let _ = gate_tx.send(GuardrailGate::Passed);
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
        let request_messages = request.messages.clone();
        let mut gate_rx = gate_rx;
        let mut stream = stream;

        // Set up streaming JSON repair if enabled and response_format is JSON
        let parallel_streaming_repairer = {
            let is_json_format = matches!(
                &request.response_format,
                Some(crate::types::ResponseFormat::JsonObject { .. })
                    | Some(crate::types::ResponseFormat::JsonSchema { .. })
            );
            if is_json_format {
                let config = state_clone.config_manager.get();
                let client = state_clone
                    .client_manager
                    .get_client(&auth_clone.api_key_id);
                let enabled = client
                    .as_ref()
                    .and_then(|c| c.json_repair.enabled)
                    .unwrap_or(config.json_repair.enabled);
                let syntax_repair = client
                    .as_ref()
                    .and_then(|c| c.json_repair.syntax_repair)
                    .unwrap_or(config.json_repair.syntax_repair);
                if enabled && syntax_repair {
                    Some(lr_json_repair::StreamingJsonRepairer::new(
                        None,
                        lr_json_repair::RepairOptions::default(),
                    ))
                } else {
                    None
                }
            } else {
                None
            }
        };

        tokio::spawn(async move {
            let mut buffer: Vec<Result<Event, std::convert::Infallible>> = Vec::new();
            let mut gate_resolved = false;
            let mut gate_state = GuardrailGate::Pending;
            let mut content_accumulator = String::new();
            let mut finish_reason_val = String::from("stop");
            let mut stream_done = false;
            let mut streaming_repairer = parallel_streaming_repairer;

            // Helper to convert a provider chunk to an SSE event
            let convert_chunk = |provider_chunk: lr_providers::CompletionChunk,
                                 gen_id: &str,
                                 created_ts: i64,
                                 content_acc: &mut String,
                                 finish_reason: &mut String,
                                 repairer: &mut Option<lr_json_repair::StreamingJsonRepairer>|
             -> (Result<Event, std::convert::Infallible>, bool) {
                let is_done = if let Some(choice) = provider_chunk.choices.first() {
                    if let Some(content) = &choice.delta.content {
                        content_acc.push_str(content);
                    }
                    if let Some(reason) = &choice.finish_reason {
                        *finish_reason = reason.clone();
                        true
                    } else {
                        false
                    }
                } else {
                    false
                };

                let api_chunk = ChatCompletionChunk {
                    id: gen_id.to_string(),
                    object: "chat.completion.chunk".to_string(),
                    created: created_ts,
                    model: provider_chunk.model.clone(),
                    choices: {
                        let mut choices: Vec<ChatCompletionChunkChoice> = provider_chunk
                            .choices
                            .into_iter()
                            .map(|choice| {
                                let tool_calls = choice.delta.tool_calls.map(|provider_deltas| {
                                    provider_deltas
                                        .into_iter()
                                        .map(|delta| crate::types::ToolCallDelta {
                                            index: delta.index,
                                            id: delta.id,
                                            tool_type: delta.tool_type,
                                            function: delta.function.map(|f| {
                                                crate::types::FunctionCallDelta {
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
                                        reasoning_content: choice.delta.reasoning_content,
                                    },
                                    finish_reason: choice.finish_reason,
                                }
                            })
                            .collect();

                        // Apply streaming JSON repair outside the map closure
                        if let Some(ref mut rep) = repairer {
                            for choice in &mut choices {
                                if let Some(text) = choice.delta.content.take() {
                                    let repaired = rep.push_content(&text);
                                    if !repaired.is_empty() {
                                        choice.delta.content = Some(repaired);
                                    }
                                }
                                if choice.finish_reason.is_some() {
                                    let flushed = rep.finish();
                                    if !flushed.is_empty() {
                                        let existing =
                                            choice.delta.content.take().unwrap_or_default();
                                        choice.delta.content =
                                            Some(format!("{}{}", existing, flushed));
                                    }
                                }
                            }
                        }

                        choices
                    },
                    usage: None,
                    system_fingerprint: None,
                    service_tier: None,
                    request_usage_entries: None,
                };

                let json = serde_json::to_string(&api_chunk).unwrap_or_default();
                (Ok(Event::default().data(json)), is_done)
            };

            loop {
                tokio::select! {
                    chunk = stream.next() => {
                        match chunk {
                            Some(Ok(provider_chunk)) => {
                                let (event, is_done) = convert_chunk(
                                    provider_chunk,
                                    &gen_id,
                                    created_timestamp,
                                    &mut content_accumulator,
                                    &mut finish_reason_val,
                                    &mut streaming_repairer,
                                );
                                if is_done {
                                    stream_done = true;
                                }
                                if gate_resolved && gate_state == GuardrailGate::Passed {
                                    if event_tx.send(event).await.is_err() {
                                        break;
                                    }
                                } else if !gate_resolved {
                                    buffer.push(event);
                                }
                                // If gate_state == Denied, silently drop chunks
                                if stream_done {
                                    break;
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
                                    // Flush buffered events
                                    for event in buffer.drain(..) {
                                        if event_tx.send(event).await.is_err() {
                                            break;
                                        }
                                    }
                                }
                                GuardrailGate::Denied => {
                                    // Send error event and close
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
                                    // Don't break yet — let the stream drain
                                }
                                GuardrailGate::Pending => unreachable!(),
                            }
                        }
                    }
                }
            }

            // Stream is done but gate may still be pending (very fast stream, slow guardrails)
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
                        // Should not happen, but fail open
                        for event in buffer.drain(..) {
                            if event_tx.send(event).await.is_err() {
                                break;
                            }
                        }
                    }
                }
            }

            // Shared finalize — matches the `handle_streaming` path.
            let prompt_tokens = request_messages
                .last()
                .map(|m| estimate_token_count(std::slice::from_ref(m)) as u32)
                .unwrap_or(0);
            let completion_tokens = (content_accumulator.len() / 4).max(1) as u32;

            let provider = if let Some((p, _)) = model_clone.split_once('/') {
                p.to_string()
            } else {
                "router".to_string()
            };

            let wire_body = super::monitor_helpers::build_streaming_response_body(
                &gen_id_clone,
                &model_clone,
                &content_accumulator,
                &finish_reason_val,
                prompt_tokens as u64,
                completion_tokens as u64,
                created_at.timestamp(),
            );

            let finalize_inputs = super::finalize::FinalizeInputs {
                state: &state_clone,
                auth: &auth_clone,
                llm_event_id: &llm_event_id,
                generation_id: &gen_id_clone,
                started_at,
                created_at,
                incremental_prompt_tokens: prompt_tokens,
                compression_tokens_saved,
                routing_metadata: None,
                user: request_user,
                streamed: true,
                skip_monitor_completion: false,
            };
            super::finalize::finalize_streaming_at_end(
                &finalize_inputs,
                super::finalize::StreamingFinalizeSummary {
                    provider,
                    model: model_clone,
                    prompt_tokens,
                    completion_tokens,
                    reasoning_tokens: None,
                    finish_reason: Some(finish_reason_val),
                    content_preview: content_accumulator,
                },
                &wire_body,
            )
            .await;
        });
    }

    // Return SSE response immediately (backed by event_rx channel)
    Ok(Sse::new(ReceiverStream::new(event_rx))
        .keep_alive(KeepAlive::default())
        .into_response())
}

// `maybe_repair_json_content` and `estimate_token_count` live in
// `super::finalize` now — imported at the top of this file.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ResponseFormat;
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
                reasoning_content: None,
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
            parallel_tool_calls: None,
            logit_bias: None,
            service_tier: None,
            store: None,
            metadata: None,
            modalities: None,
            audio: None,
            prediction: None,
            reasoning_effort: None,
        };

        assert!(crate::routes::pipeline::validate_request(&request).is_ok());
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
                reasoning_content: None,
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
            parallel_tool_calls: None,
            logit_bias: None,
            service_tier: None,
            store: None,
            metadata: None,
            modalities: None,
            audio: None,
            prediction: None,
            reasoning_effort: None,
        };

        assert!(crate::routes::pipeline::validate_request(&request).is_err());
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
            parallel_tool_calls: None,
            logit_bias: None,
            service_tier: None,
            store: None,
            metadata: None,
            modalities: None,
            audio: None,
            prediction: None,
            reasoning_effort: None,
        };

        assert!(crate::routes::pipeline::validate_request(&request).is_err());
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
                reasoning_content: None,
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
            parallel_tool_calls: None,
            logit_bias: None,
            service_tier: None,
            store: None,
            metadata: None,
            modalities: None,
            audio: None,
            prediction: None,
            reasoning_effort: None,
        };

        assert!(crate::routes::pipeline::validate_request(&request).is_ok());
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
                reasoning_content: None,
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
            parallel_tool_calls: None,
            logit_bias: None,
            service_tier: None,
            store: None,
            metadata: None,
            modalities: None,
            audio: None,
            prediction: None,
            reasoning_effort: None,
        };

        assert!(crate::routes::pipeline::validate_request(&request).is_err());
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
                reasoning_content: None,
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
            parallel_tool_calls: None,
            logit_bias: None,
            service_tier: None,
            store: None,
            metadata: None,
            modalities: None,
            audio: None,
            prediction: None,
            reasoning_effort: None,
        };

        assert!(crate::routes::pipeline::validate_request(&request).is_ok());
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
                reasoning_content: None,
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
            parallel_tool_calls: None,
            logit_bias: None,
            service_tier: None,
            store: None,
            metadata: None,
            modalities: None,
            audio: None,
            prediction: None,
            reasoning_effort: None,
        };

        assert!(crate::routes::pipeline::validate_request(&request).is_err());
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
                reasoning_content: None,
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
            parallel_tool_calls: None,
            logit_bias: None,
            service_tier: None,
            store: None,
            metadata: None,
            modalities: None,
            audio: None,
            prediction: None,
            reasoning_effort: None,
        };

        assert!(crate::routes::pipeline::validate_request(&request).is_ok());
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
                reasoning_content: None,
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
            parallel_tool_calls: None,
            logit_bias: None,
            service_tier: None,
            store: None,
            metadata: None,
            modalities: None,
            audio: None,
            prediction: None,
            reasoning_effort: None,
        };

        assert!(crate::routes::pipeline::validate_request(&request).is_err());
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
                reasoning_content: None,
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
            parallel_tool_calls: None,
            logit_bias: None,
            service_tier: None,
            store: None,
            metadata: None,
            modalities: None,
            audio: None,
            prediction: None,
            reasoning_effort: None,
        };

        assert!(crate::routes::pipeline::validate_request(&request).is_ok());
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
                reasoning_content: None,
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
            parallel_tool_calls: None,
            logit_bias: None,
            service_tier: None,
            store: None,
            metadata: None,
            modalities: None,
            audio: None,
            prediction: None,
            reasoning_effort: None,
        };

        assert!(crate::routes::pipeline::validate_request(&request).is_err());
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
                reasoning_content: None,
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
            parallel_tool_calls: None,
            logit_bias: None,
            service_tier: None,
            store: None,
            metadata: None,
            modalities: None,
            audio: None,
            prediction: None,
            reasoning_effort: None,
        };

        assert!(crate::routes::pipeline::validate_request(&request).is_ok());
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
                reasoning_content: None,
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
            parallel_tool_calls: None,
            logit_bias: None,
            service_tier: None,
            store: None,
            metadata: None,
            modalities: None,
            audio: None,
            prediction: None,
            reasoning_effort: None,
        };

        assert!(crate::routes::pipeline::validate_request(&request).is_err());
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
                reasoning_content: None,
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
            parallel_tool_calls: None,
            logit_bias: None,
            service_tier: None,
            store: None,
            metadata: None,
            modalities: None,
            audio: None,
            prediction: None,
            reasoning_effort: None,
        };

        let result = crate::routes::pipeline::convert_to_provider_request(&request);
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
                reasoning_content: None,
            },
            ChatMessage {
                role: "assistant".to_string(),
                content: Some(MessageContent::Text("I'm doing well!".to_string())), // ~15 chars = 3-4 tokens
                name: None,
                tool_calls: None,
                tool_call_id: None,
                reasoning_content: None,
            },
        ];

        let count = estimate_token_count(&messages);
        assert!(count > 0);
        assert!(count < 100); // Should be reasonable
    }
}
