//! `POST /v1/responses` (and `/responses`) — the OpenAI Responses API
//! surface, backed by any of our chat-completions providers.
//!
//! ## Scope (v1)
//!
//! This handler implements the minimum viable Responses API needed to
//! unblock clients written against `/responses` (e.g. Codex/ChatGPT
//! apps pointed at LocalRouter instead of `chatgpt.com`). It:
//!
//! - Routes through the existing `state.router`, so strategy routing,
//!   provider overrides (e.g. OpenAI OAuth → `/responses` passthrough
//!   from phase 1), rate limits, and free-tier handling all apply.
//! - Honors `previous_response_id` + `store: true` by persisting the
//!   accumulated message history to `lr-responses-sessions`
//!   (SQLite at `<data_dir>/responses-sessions/sessions.db`).
//! - Emits streaming Responses-API SSE events
//!   (`response.created` → `response.output_text.delta` →
//!   `response.function_call_arguments.delta` →
//!   `response.output_item.done` → `response.completed`) via
//!   `lr_providers::openai_responses::ResponsesEmitter`.
//!
//! The handler reuses the existing chat-completions pipeline helpers
//! (now `pub(crate)` in `chat.rs`) so validation, rate limits,
//! guardrails, secret scanning, and compression all apply verbatim —
//! we translate `CreateResponseRequest` → `ChatCompletionRequest`
//! early, feed the shared helpers, then convert to the provider
//! shape via `convert_to_provider_request`.
//!
//! Known gaps (follow-ups):
//! - MCP-via-LLM orchestration and firewall approval popups for
//!   `localrouter/auto` are still chat-specific. Responses currently
//!   goes through `state.router` directly; adding those requires
//!   hoisting more of the chat.rs orchestrator.
//! - Only `input` arrays with `message` / `function_call` /
//!   `function_call_output` items are handled; reasoning items and
//!   custom tools pass through untouched but aren't actively
//!   interpreted.

use std::convert::Infallible;
use std::time::Instant;

use axum::{
    extract::{Extension, State},
    response::{sse::KeepAlive, IntoResponse, Json as JsonResponse, Response, Sse},
    Json,
};
use chrono::Utc;
use futures::StreamExt;
use lr_providers::openai_responses::{
    completion_to_response_object, ResponsesEmitter, ResponsesSseFrame,
};
use lr_providers::{ChatMessage, Tool};

use super::finalize::{
    finalize_metrics_and_monitor, update_response_body_and_record_generation, FinalizeInputs,
};
use lr_responses_sessions::{
    deserialize_history, serialize_history, ResponsesSession, ResponsesSessionStore,
    RetentionConfig,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::{debug, warn};
use uuid::Uuid;

use crate::middleware::client_auth::ClientAuthContext;
use crate::middleware::error::{ApiErrorResponse, ApiResult};
use crate::state::{AppState, AuthContext};
use crate::types::{
    ChatCompletionRequest, ChatMessage as ServerChatMessage, ContentPart as ServerContentPart,
    FunctionCall as ServerFunctionCall, FunctionDefinition as ServerFunctionDefinition,
    FunctionName as ServerFunctionName, ImageUrl as ServerImageUrl,
    MessageContent as ServerMessageContent, ResponseFormat as ServerResponseFormat,
    Tool as ServerTool, ToolCall as ServerToolCall, ToolChoice as ServerToolChoice,
};

// ============================================================================
// Request / Response wire types
// ============================================================================

/// Client-facing POST /v1/responses body. Only the fields our
/// chat-completions-backed implementation currently honors. Extra
/// fields on the wire are preserved via `#[serde(deny_unknown_fields)]
/// = false` (serde default) so adding support later is non-breaking.
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct CreateResponseRequest {
    pub model: String,
    /// Either a single string (shorthand for one user message) or an
    /// array of typed `ResponseItem`s.
    pub input: ResponseInput,
    #[serde(default)]
    pub instructions: Option<String>,
    #[serde(default)]
    pub previous_response_id: Option<String>,
    #[serde(default)]
    pub stream: bool,
    #[serde(default)]
    pub store: Option<bool>,
    #[serde(default)]
    pub tools: Option<Vec<Value>>,
    #[serde(default)]
    pub tool_choice: Option<Value>,
    #[serde(default)]
    pub parallel_tool_calls: Option<bool>,
    #[serde(default)]
    pub temperature: Option<f32>,
    #[serde(default)]
    pub top_p: Option<f32>,
    #[serde(default)]
    pub max_output_tokens: Option<u32>,
    #[serde(default)]
    pub reasoning: Option<ReasoningRequest>,
    #[serde(default)]
    pub response_format: Option<Value>,
    #[serde(default)]
    pub metadata: Option<Value>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(untagged)]
pub enum ResponseInput {
    Text(String),
    Items(Vec<Value>),
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ReasoningRequest {
    #[serde(default)]
    pub effort: Option<String>,
}

// ============================================================================
// Handler entry point
// ============================================================================

pub async fn create_response(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    client_auth: Option<Extension<ClientAuthContext>>,
    Json(req): Json<CreateResponseRequest>,
) -> ApiResult<Response> {
    // Correlate logs/monitor events across the turn.
    let session_id = Uuid::new_v4().to_string();
    let response_id = format!("resp_{}", Uuid::new_v4().simple());
    let started_at = Instant::now();
    let created_at_dt = Utc::now();
    let created_at = created_at_dt.timestamp();

    let request_json = serde_json::to_value(&req).unwrap_or(Value::Null);
    let mut llm_guard = super::monitor_helpers::emit_llm_call(
        &state,
        client_auth.as_ref(),
        Some(&session_id),
        "/v1/responses",
        &req.model,
        req.stream,
        &request_json,
    );

    let store_flag = req.store.unwrap_or(true);
    let session_store = responses_session_store(&state);
    let retention = {
        let cfg = state.config_manager.get();
        RetentionConfig {
            retention_days: cfg.responses.retention_days as i64,
            active_window_hours: cfg.responses.active_window_hours as i64,
        }
    };

    // If continuing a prior turn, reload its messages + tools.
    let (prior_messages, prior_tools) = if let Some(prev_id) = req.previous_response_id.as_deref() {
        match session_store
            .as_ref()
            .and_then(|s| s.get_active(prev_id, &retention).ok().flatten())
        {
            Some(sess) => deserialize_history(&sess.messages_json, sess.tools_json.as_deref()),
            None => {
                // Clients legitimately hit stale ids; we treat them as
                // "start fresh" rather than 404 so chained turns still
                // compose reasonably.
                debug!(
                    "previous_response_id {} not found / expired; continuing without history",
                    prev_id
                );
                (Vec::new(), None)
            }
        }
    } else {
        (Vec::new(), None)
    };

    // Build a server-side `ChatCompletionRequest` that the shared
    // chat-completions pipeline already knows how to validate, rate-
    // limit, guardrail-scan, and secret-scan. This is the same shape
    // `/v1/chat/completions` receives — any hardening applied there
    // now applies here with no duplicated orchestration.
    let mut chat_req = build_chat_completion_request(&req, prior_messages, prior_tools.clone())?;

    // Auto-routing firewall + strategy access checks (normalizes the
    // model name, validates strategy permissions, shows the firewall
    // approval popup for `localrouter/auto`, enforces MCP-only client
    // mode). Identical to what `/v1/chat/completions` runs — factored
    // out behind `super::chat::apply_model_access_checks` so the
    // hardening stays in a single place.
    super::chat::apply_model_access_checks(
        &state,
        &auth,
        client_auth.as_ref(),
        &session_id,
        &mut chat_req,
        &mut llm_guard,
    )
    .await?;

    // Run the shared pipeline (fail-fast order mirrors chat.rs).
    if let Err(e) = super::chat::validate_request(&chat_req) {
        return Err(llm_guard.capture_err(e));
    }
    if let Err(e) = super::chat::check_rate_limits(&state, &auth, &chat_req).await {
        return Err(llm_guard.capture_err(e));
    }

    // Guardrails scan on the request (run serially before the LLM
    // call; blocks here if a category action denies the request).
    if let Some(result) =
        super::chat::run_guardrails_scan(&state, client_auth.as_ref().map(|e| &e.0), &chat_req)
            .await?
    {
        super::chat::handle_guardrail_approval(
            &state,
            client_auth.as_ref().map(|e| &e.0),
            &chat_req,
            result,
            "request",
        )
        .await
        .map_err(|e| llm_guard.capture_err(e))?;
    }

    // Secret-scan the request. The helper internally triggers the
    // approval popup on a hit and returns `Err` if the user denies or
    // the finding is `Block`-classified.
    super::chat::run_secret_scan_check(&state, client_auth.as_ref().map(|e| &e.0), &chat_req)
        .await
        .map_err(|e| llm_guard.capture_err(e))?;

    // Prompt compression (no-op when disabled / model not configured).
    let compression_result =
        super::chat::run_prompt_compression(&state, client_auth.as_ref().map(|e| &e.0), &chat_req)
            .await
            .ok()
            .flatten();

    // Now translate to the provider shape. This is the same helper
    // `chat_completions` uses, so the conversion is lossless.
    let provider_request = super::chat::convert_to_provider_request(&chat_req)
        .map_err(|e| llm_guard.capture_err(e))?;

    // Merged history the session row will capture on success (server
    // -> provider conversion drops the `name` field etc., but the
    // provider-shaped messages are what we already store everywhere
    // else in the session schema).
    let merged_messages = provider_request.messages.clone();
    let merged_tools = provider_request.tools.clone();

    // Decide routing backend: the MCP-via-LLM orchestrator produces
    // chat-completions-shape responses just like the router does, so
    // we branch here and the Responses-emitter stays oblivious.
    let client_for_routing =
        super::helpers::get_client_with_strategy(&state, &auth.api_key_id).ok();
    let use_mcp_via_llm = client_for_routing
        .as_ref()
        .map(|(c, _)| c.client_mode == lr_config::ClientMode::McpViaLlm)
        .unwrap_or(false);

    if req.stream {
        let (stream, routing_metadata) = if use_mcp_via_llm {
            let (client, _strategy) = client_for_routing
                .as_ref()
                .cloned()
                .expect("client presence already verified above");
            let allowed_servers = compute_allowed_mcp_servers(&state, &client);
            match state
                .mcp_via_llm_manager
                .handle_streaming_request(
                    state.mcp_gateway.clone(),
                    state.router.clone(),
                    &client,
                    provider_request,
                    allowed_servers,
                    None,
                    Some(llm_guard.event_id().to_string()),
                    Some(session_id.clone()),
                    // Deterministic session key: reuses the orchestrator
                    // session that produced the turn named by
                    // `previous_response_id`, bypassing hash-matching.
                    req.previous_response_id.clone(),
                )
                .await
            {
                Ok(s) => (s, None),
                Err(e) => {
                    return Err(llm_guard.capture_err(ApiErrorResponse::bad_gateway(format!(
                        "MCP-via-LLM error: {}",
                        e
                    ))));
                }
            }
        } else {
            match state
                .router
                .stream_complete(&auth.api_key_id, provider_request)
                .await
            {
                Ok((s, routing_meta)) => (s, routing_meta),
                Err(e) => {
                    return Err(llm_guard.capture_err(ApiErrorResponse::bad_gateway(format!(
                        "Router error: {}",
                        e
                    ))));
                }
            }
        };

        let incremental_prompt_tokens = chat_req
            .messages
            .last()
            .map(|msg| super::finalize::estimate_token_count(std::slice::from_ref(msg)) as u32)
            .unwrap_or(0);

        // Consume the guard BEFORE handing off to the spawned stream:
        // we record completion ourselves at stream-end via
        // `finalize_streaming_at_end`.
        let llm_event_id = llm_guard.into_event_id();

        let emit_sse = build_stream_response(
            state.clone(),
            auth.clone(),
            session_store.clone(),
            stream,
            StreamWrapperCtx {
                response_id,
                model: req.model.clone(),
                created_at,
                store_flag,
                previous_response_id: req.previous_response_id.clone(),
                merged_messages,
                merged_tools,
                api_key_id: auth.api_key_id.clone(),
                metadata_json: req
                    .metadata
                    .as_ref()
                    .and_then(|v| serde_json::to_string(v).ok()),
                llm_event_id,
                started_at,
                created_at_dt,
                incremental_prompt_tokens,
                compression_tokens_saved: compression_result
                    .as_ref()
                    .filter(|r| r.original_tokens > r.compressed_tokens)
                    .map(|r| (r.original_tokens - r.compressed_tokens) as u64)
                    .unwrap_or(0),
                routing_metadata,
            },
        );

        return Ok(emit_sse);
    }

    // Non-streaming path: same MCP-via-LLM vs router branch as above.
    // We also capture the router's routing metadata (if any) so the
    // finalize step can attach it to the monitor event.
    let (completion, routing_metadata) = if use_mcp_via_llm {
        let (client, _strategy) = client_for_routing
            .as_ref()
            .cloned()
            .expect("client presence already verified above");
        let allowed_servers = compute_allowed_mcp_servers(&state, &client);
        let c = state
            .mcp_via_llm_manager
            .handle_request(
                state.mcp_gateway.clone(),
                &state.router,
                &client,
                provider_request,
                allowed_servers,
                None,
                Some(llm_guard.event_id().to_string()),
                Some(session_id.clone()),
                // See streaming variant above — deterministic
                // session key via `previous_response_id`.
                req.previous_response_id.clone(),
            )
            .await
            .map_err(|e| {
                llm_guard.capture_err(ApiErrorResponse::bad_gateway(format!(
                    "MCP-via-LLM error: {}",
                    e
                )))
            })?;
        (c, None)
    } else {
        let (completion, routing_meta) = state
            .router
            .complete(&auth.api_key_id, provider_request)
            .await
            .map_err(|e| {
                llm_guard.capture_err(ApiErrorResponse::bad_gateway(format!(
                    "Router error: {}",
                    e
                )))
            })?;
        (completion, routing_meta)
    };

    // Compute the incremental prompt-tokens the same way chat.rs does
    // — only the last message, since history is accumulated across
    // turns via `previous_response_id`.
    let incremental_prompt_tokens = chat_req
        .messages
        .last()
        .map(|msg| super::finalize::estimate_token_count(std::slice::from_ref(msg)) as u32)
        .unwrap_or(completion.usage.prompt_tokens);

    let finalize_inputs = FinalizeInputs {
        state: &state,
        auth: &auth,
        llm_event_id: llm_guard.event_id(),
        generation_id: &response_id,
        started_at,
        created_at: created_at_dt,
        incremental_prompt_tokens,
        compression_tokens_saved: compression_result
            .as_ref()
            .filter(|r| r.original_tokens > r.compressed_tokens)
            .map(|r| (r.original_tokens - r.compressed_tokens) as u64)
            .unwrap_or(0),
        routing_metadata: routing_metadata.as_ref(),
        user: None,
        streamed: false,
    };
    let metrics = finalize_metrics_and_monitor(&finalize_inputs, &completion).await;

    // Native pass-through: when the provider is ChatGPT Plus (or any
    // future native Responses backend), `OpenAIProvider` stashes the
    // verbatim upstream JSON on `completion.extensions` under
    // `NATIVE_RESPONSES_API_EXT_KEY`. We prefer that over the lossy
    // `completion_to_response_object` translation so reasoning items,
    // encrypted content carry-over, and built-in tool results reach
    // the client intact. Falls back to the translator for every other
    // provider.
    let response_object = select_wire_body(&completion, &response_id, created_at);
    let wire_body = serde_json::to_value(&response_object).unwrap_or(Value::Null);
    let finish_reason = completion
        .choices
        .first()
        .and_then(|c| c.finish_reason.clone());
    let tokens = crate::types::TokenUsage {
        prompt_tokens: incremental_prompt_tokens,
        completion_tokens: completion.usage.completion_tokens,
        total_tokens: incremental_prompt_tokens + completion.usage.completion_tokens,
        prompt_tokens_details: completion.usage.prompt_tokens_details.clone(),
        completion_tokens_details: completion.usage.completion_tokens_details.clone(),
    };
    update_response_body_and_record_generation(
        &finalize_inputs,
        &completion,
        &metrics,
        &wire_body,
        finish_reason,
        tokens,
    );

    // Persist the turn if requested.
    if store_flag {
        if let Some(ref store) = session_store {
            let mut merged_all: Vec<ChatMessage> = merged_messages.clone();
            if let Some(choice) = completion.choices.first() {
                merged_all.push(choice.message.clone());
            }
            let (messages_json, tools_json) =
                serialize_history(&merged_all, merged_tools.as_deref());
            let session = ResponsesSession {
                id: response_id.clone(),
                client_id: auth.api_key_id.clone(),
                previous_response_id: req.previous_response_id.clone(),
                model: req.model.clone(),
                created_at,
                last_activity: Utc::now().timestamp(),
                store: true,
                metadata_json: req
                    .metadata
                    .as_ref()
                    .and_then(|v| serde_json::to_string(v).ok()),
                messages_json,
                tools_json,
                final_response_json: serde_json::to_string(&response_object).ok(),
            };
            if let Err(e) = store.insert(&session) {
                warn!(
                    "Failed to persist /responses session {}: {}",
                    response_id, e
                );
            }
        }
    }

    // Guard's `into_event_id` defuses the auto-error-on-drop — we've
    // already completed the event above via `finalize_metrics_and_monitor`.
    let _ = llm_guard.into_event_id();
    Ok(JsonResponse(response_object).into_response())
}

/// Resolve the MCP servers this client is allowed to invoke via the
/// orchestrator. Mirrors the equivalent block in chat.rs's
/// `handle_mcp_via_llm`; kept inline here to avoid another helper
/// extraction.
fn compute_allowed_mcp_servers(state: &AppState, client: &lr_config::Client) -> Vec<String> {
    let all_server_ids: Vec<String> = state
        .config_manager
        .get()
        .mcp_servers
        .iter()
        .map(|s| s.id.clone())
        .collect();
    if client.mcp_permissions.global.is_enabled() {
        all_server_ids
    } else {
        all_server_ids
            .iter()
            .filter(|id| client.mcp_permissions.has_any_enabled_for_server(id))
            .cloned()
            .collect()
    }
}

// ============================================================================
// Translation: CreateResponseRequest → server-side ChatCompletionRequest
// ============================================================================
//
// We produce the same `ChatCompletionRequest` shape the chat-completions
// route receives, so the shared pipeline helpers (validate, rate limits,
// guardrails, secret scan, compression) can run without any
// /responses-specific branching.

fn build_chat_completion_request(
    req: &CreateResponseRequest,
    prior_messages: Vec<ChatMessage>,
    prior_tools: Option<Vec<lr_providers::Tool>>,
) -> ApiResult<ChatCompletionRequest> {
    // Prior messages come from the session store as provider-shape
    // `lr_providers::ChatMessage`; convert to server-shape messages.
    let mut messages: Vec<ServerChatMessage> =
        prior_messages.iter().map(provider_msg_to_server).collect();

    if let Some(instr) = req.instructions.as_deref() {
        if !instr.is_empty() {
            messages.insert(
                0,
                ServerChatMessage {
                    role: "system".into(),
                    content: Some(ServerMessageContent::Text(instr.to_string())),
                    name: None,
                    tool_calls: None,
                    tool_call_id: None,
                    reasoning_content: None,
                },
            );
        }
    }

    // Translate input items into ChatMessages.
    let new_messages = match &req.input {
        ResponseInput::Text(s) => vec![ServerChatMessage {
            role: "user".into(),
            content: Some(ServerMessageContent::Text(s.clone())),
            name: None,
            tool_calls: None,
            tool_call_id: None,
            reasoning_content: None,
        }],
        ResponseInput::Items(items) => items
            .iter()
            .filter_map(response_item_to_server_message)
            .collect(),
    };
    messages.extend(new_messages);

    // Tools: prefer caller's `tools` over any prior ones.
    let tools: Option<Vec<ServerTool>> = match req.tools.as_ref() {
        Some(list) => Some(list.iter().filter_map(value_to_server_tool).collect()),
        None => prior_tools.map(|ts| ts.iter().map(provider_tool_to_server).collect::<Vec<_>>()),
    };

    let tool_choice = req
        .tool_choice
        .as_ref()
        .and_then(value_to_server_tool_choice);

    let response_format = req
        .response_format
        .as_ref()
        .and_then(value_to_server_response_format);

    Ok(ChatCompletionRequest {
        model: req.model.clone(),
        messages,
        temperature: req.temperature,
        top_p: req.top_p,
        max_tokens: None,
        max_completion_tokens: req.max_output_tokens,
        n: None,
        stop: None,
        stream: req.stream,
        logprobs: None,
        top_logprobs: None,
        frequency_penalty: None,
        presence_penalty: None,
        top_k: None,
        seed: None,
        repetition_penalty: None,
        response_format,
        tools,
        tool_choice,
        parallel_tool_calls: req.parallel_tool_calls,
        logit_bias: None,
        service_tier: None,
        store: req.store,
        metadata: None,
        modalities: None,
        audio: None,
        prediction: None,
        reasoning_effort: req.reasoning.as_ref().and_then(|r| r.effort.clone()),
        extensions: None,
        user: None,
    })
}

/// Downconvert `lr_providers::ChatMessage` (from the session store)
/// to the server-side `ChatMessage` the pipeline helpers expect.
/// Used only for historical replay — never the reverse.
fn provider_msg_to_server(m: &ChatMessage) -> ServerChatMessage {
    let content = match &m.content {
        lr_providers::ChatMessageContent::Text(t) => {
            if t.is_empty() {
                None
            } else {
                Some(ServerMessageContent::Text(t.clone()))
            }
        }
        lr_providers::ChatMessageContent::Parts(parts) => {
            let mapped: Vec<ServerContentPart> = parts
                .iter()
                .map(|p| match p {
                    lr_providers::ContentPart::Text { text } => {
                        ServerContentPart::Text { text: text.clone() }
                    }
                    lr_providers::ContentPart::ImageUrl { image_url } => {
                        ServerContentPart::ImageUrl {
                            image_url: ServerImageUrl {
                                url: image_url.url.clone(),
                                detail: image_url.detail.clone(),
                            },
                        }
                    }
                })
                .collect();
            Some(ServerMessageContent::Parts(mapped))
        }
    };
    let tool_calls = m.tool_calls.as_ref().map(|tcs| {
        tcs.iter()
            .map(|tc| ServerToolCall {
                id: tc.id.clone(),
                tool_type: tc.tool_type.clone(),
                function: ServerFunctionCall {
                    name: tc.function.name.clone(),
                    arguments: tc.function.arguments.clone(),
                },
            })
            .collect()
    });
    ServerChatMessage {
        role: m.role.clone(),
        content,
        name: m.name.clone(),
        tool_calls,
        tool_call_id: m.tool_call_id.clone(),
        reasoning_content: m.reasoning_content.clone(),
    }
}

fn provider_tool_to_server(t: &lr_providers::Tool) -> ServerTool {
    ServerTool {
        tool_type: t.tool_type.clone(),
        function: ServerFunctionDefinition {
            name: t.function.name.clone(),
            description: t.function.description.clone(),
            parameters: t.function.parameters.clone(),
        },
    }
}

fn response_item_to_server_message(item: &Value) -> Option<ServerChatMessage> {
    let obj = item.as_object()?;
    let kind = obj.get("type").and_then(|v| v.as_str()).unwrap_or("");
    match kind {
        "message" => {
            let role = obj
                .get("role")
                .and_then(|v| v.as_str())
                .unwrap_or("user")
                .to_string();
            let content = obj.get("content")?;
            let content = Some(content_from_response_parts(content));
            Some(ServerChatMessage {
                role,
                content,
                name: None,
                tool_calls: None,
                tool_call_id: None,
                reasoning_content: None,
            })
        }
        "function_call" => {
            let call_id = obj.get("call_id").and_then(|v| v.as_str())?.to_string();
            let name = obj.get("name").and_then(|v| v.as_str())?.to_string();
            let arguments = obj
                .get("arguments")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            Some(ServerChatMessage {
                role: "assistant".into(),
                content: None,
                name: None,
                tool_calls: Some(vec![ServerToolCall {
                    id: call_id,
                    tool_type: "function".into(),
                    function: ServerFunctionCall { name, arguments },
                }]),
                tool_call_id: None,
                reasoning_content: None,
            })
        }
        "function_call_output" => {
            let call_id = obj.get("call_id").and_then(|v| v.as_str())?.to_string();
            let output = obj
                .get("output")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            Some(ServerChatMessage {
                role: "tool".into(),
                content: Some(ServerMessageContent::Text(output)),
                name: None,
                tool_calls: None,
                tool_call_id: Some(call_id),
                reasoning_content: None,
            })
        }
        _ => None, // reasoning, custom tool calls — dropped for v1
    }
}

fn content_from_response_parts(content: &Value) -> ServerMessageContent {
    if let Some(s) = content.as_str() {
        return ServerMessageContent::Text(s.to_string());
    }
    let Some(arr) = content.as_array() else {
        return ServerMessageContent::Text(String::new());
    };
    let mut parts: Vec<ServerContentPart> = Vec::new();
    for p in arr {
        let obj = match p.as_object() {
            Some(o) => o,
            None => continue,
        };
        match obj.get("type").and_then(|v| v.as_str()) {
            Some("input_text") | Some("output_text") => {
                if let Some(text) = obj.get("text").and_then(|v| v.as_str()) {
                    parts.push(ServerContentPart::Text {
                        text: text.to_string(),
                    });
                }
            }
            Some("input_image") => {
                let url = obj
                    .get("image_url")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let detail = obj
                    .get("detail")
                    .and_then(|v| v.as_str())
                    .map(str::to_string);
                parts.push(ServerContentPart::ImageUrl {
                    image_url: ServerImageUrl { url, detail },
                });
            }
            _ => {}
        }
    }
    if parts.is_empty() {
        ServerMessageContent::Text(String::new())
    } else {
        ServerMessageContent::Parts(parts)
    }
}

fn value_to_server_tool(v: &Value) -> Option<ServerTool> {
    let obj = v.as_object()?;
    let tool_type = obj
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("function")
        .to_string();
    let name = obj.get("name").and_then(|v| v.as_str())?.to_string();
    let description = obj
        .get("description")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let parameters = obj.get("parameters").cloned().unwrap_or(Value::Null);
    Some(ServerTool {
        tool_type,
        function: ServerFunctionDefinition {
            name,
            description,
            parameters,
        },
    })
}

fn value_to_server_tool_choice(v: &Value) -> Option<ServerToolChoice> {
    if let Some(s) = v.as_str() {
        return Some(ServerToolChoice::Auto(s.to_string()));
    }
    let obj = v.as_object()?;
    let tool_type = obj
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("function")
        .to_string();
    let name = obj
        .get("function")
        .and_then(|f| f.get("name"))
        .and_then(|v| v.as_str())?;
    Some(ServerToolChoice::Specific {
        tool_type,
        function: ServerFunctionName {
            name: name.to_string(),
        },
    })
}

fn value_to_server_response_format(v: &Value) -> Option<ServerResponseFormat> {
    let obj = v.as_object()?;
    let format_type = obj
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if format_type == "json_schema" {
        let schema = obj
            .get("schema")
            .or_else(|| obj.get("json_schema").and_then(|j| j.get("schema")))
            .cloned()
            .unwrap_or(Value::Null);
        return Some(ServerResponseFormat::JsonSchema {
            r#type: format_type,
            schema,
        });
    }
    Some(ServerResponseFormat::JsonObject {
        r#type: format_type,
    })
}

// ============================================================================
// Streaming wrapper: router stream → Responses-API SSE
// ============================================================================

struct StreamWrapperCtx {
    response_id: String,
    model: String,
    created_at: i64,
    store_flag: bool,
    previous_response_id: Option<String>,
    merged_messages: Vec<ChatMessage>,
    merged_tools: Option<Vec<Tool>>,
    api_key_id: String,
    metadata_json: Option<String>,
    llm_event_id: String,
    started_at: Instant,
    created_at_dt: chrono::DateTime<Utc>,
    incremental_prompt_tokens: u32,
    compression_tokens_saved: u64,
    routing_metadata: Option<serde_json::Value>,
}

fn build_stream_response(
    state: AppState,
    auth: AuthContext,
    session_store: Option<ResponsesSessionStore>,
    upstream: std::pin::Pin<
        Box<dyn futures::Stream<Item = lr_types::AppResult<lr_providers::CompletionChunk>> + Send>,
    >,
    ctx: StreamWrapperCtx,
) -> Response {
    let StreamWrapperCtx {
        response_id,
        model,
        created_at,
        store_flag,
        previous_response_id,
        merged_messages,
        merged_tools,
        api_key_id,
        metadata_json,
        llm_event_id,
        started_at,
        created_at_dt,
        incremental_prompt_tokens,
        compression_tokens_saved,
        routing_metadata,
    } = ctx;

    // Produce SSE `Event`s as the chat-completions stream flows in.
    let mut emitter = ResponsesEmitter::new(response_id.clone(), model.clone(), created_at);
    let mut finish_reason: Option<String> = None;
    let mut assistant_text = String::new();
    let mut tool_calls: Vec<lr_providers::ToolCall> = Vec::new();
    let mut completion_tokens_observed: u32 = 0;
    let mut prompt_tokens_observed: u32 = 0;
    let mut reasoning_tokens_observed: Option<u64> = None;
    let provider_name_observed: Option<String> = None;
    let mut model_name_observed: Option<String> = None;
    // True once we see any chunk carrying a raw Responses SSE
    // envelope — signals we're in native pass-through mode and should
    // not emit the emitter's synthesized finish frames (upstream
    // already sent `response.completed`).
    let mut saw_native_envelope = false;

    let emitted = async_stream::stream! {
        // Opening event.
        for frame in emitter.start() {
            yield sse_event(frame);
        }

        // Main loop: translate each chunk into zero-or-more SSE frames.
        let mut stream = upstream;
        while let Some(chunk_result) = stream.next().await {
            let chunk = match chunk_result {
                Ok(c) => c,
                Err(e) => {
                    // Emit an error frame and terminate.
                    let frame = ResponsesSseFrame {
                        event: "response.failed".into(),
                        data: serde_json::json!({
                            "type": "response.failed",
                            "response": {
                                "id": response_id,
                                "status": "failed",
                                "error": { "message": e.to_string() },
                            },
                        }),
                    };
                    yield sse_event(frame);
                    return;
                }
            };
            // Capture finish_reason / text / tool_calls for the
            // persisted session row.
            if model_name_observed.is_none() && !chunk.model.is_empty() {
                model_name_observed = Some(chunk.model.clone());
            }
            for choice in &chunk.choices {
                if let Some(c) = &choice.delta.content {
                    assistant_text.push_str(c);
                }
                if let Some(tcs) = &choice.delta.tool_calls {
                    accumulate_tool_calls(&mut tool_calls, tcs);
                }
                if let Some(fr) = &choice.finish_reason {
                    finish_reason = Some(fr.clone());
                }
            }
            // If the provider shipped usage inside `extensions.usage`
            // (OpenAI / Gemini both do this on the final chunk), pick
            // it up for accurate metrics rather than estimating.
            if let Some(ext) = chunk.extensions.as_ref() {
                if let Some(usage) = ext.get("usage").and_then(|v| v.as_object()) {
                    if let Some(pt) = usage.get("prompt_tokens").and_then(|v| v.as_u64()) {
                        prompt_tokens_observed = pt as u32;
                    }
                    if let Some(ct) = usage.get("completion_tokens").and_then(|v| v.as_u64()) {
                        completion_tokens_observed = ct as u32;
                    }
                    if let Some(rt) = usage
                        .get("completion_tokens_details")
                        .and_then(|d| d.get("reasoning_tokens"))
                        .and_then(|v| v.as_u64())
                    {
                        reasoning_tokens_observed = Some(rt);
                    }
                }
            }
            // Native pass-through: if the upstream Responses API
            // emitted raw SSE envelopes (e.g. ChatGPT Plus, where the
            // translator stashes them via NATIVE_RESPONSES_SSE_EXT_KEY),
            // forward each envelope verbatim so reasoning deltas,
            // encrypted-content carry-over, and built-in tool events
            // reach the client intact. Otherwise fall back to the
            // `ResponsesEmitter` re-serialization.
            let native_envelope = chunk.extensions.as_ref().and_then(|ext| {
                ext.get(lr_providers::openai_responses::NATIVE_RESPONSES_SSE_EXT_KEY)
                    .cloned()
            });
            if let Some(envelope) = native_envelope {
                saw_native_envelope = true;
                let event_type = envelope
                    .get("type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("message")
                    .to_string();
                // Rewrite top-level `response.id` to LocalRouter's
                // response_id, same as the non-streaming path — this
                // keeps `previous_response_id` continuations pointing
                // at our session store, not the upstream handle.
                let envelope = rewrite_envelope_response_id(envelope, &response_id);
                yield sse_event(ResponsesSseFrame {
                    event: event_type,
                    data: envelope,
                });
                // Still advance the emitter's internal counters so
                // `final_response_object` reflects the same shape for
                // the telemetry wire body.
                let _ = emitter.on_chunk(chunk);
            } else {
                for frame in emitter.on_chunk(chunk) {
                    yield sse_event(frame);
                }
            }
        }

        // Finish frames: skip when upstream was native Responses SSE
        // (it already emitted `response.completed` itself). Otherwise
        // emit the synthesized finish frames.
        if !saw_native_envelope {
            for frame in emitter.finish(finish_reason.as_deref()) {
                yield sse_event(frame);
            }
        }

        // Finalize telemetry: cost, metrics, tray graph, access log,
        // `complete_llm_call`, `update_llm_call_response_body`, and
        // the generation-tracker row. Falls back to text-length-based
        // token estimation when the upstream stream didn't surface a
        // `usage` object.
        let completion_tokens = if completion_tokens_observed > 0 {
            completion_tokens_observed
        } else {
            (assistant_text.len() / 4).max(1) as u32
        };
        let prompt_tokens = if prompt_tokens_observed > 0 {
            prompt_tokens_observed
        } else {
            incremental_prompt_tokens
        };
        let provider_for_finalize = provider_name_observed
            .clone()
            .or_else(|| model.split_once('/').map(|(p, _)| p.to_string()))
            .unwrap_or_else(|| "router".to_string());
        let model_for_finalize = model_name_observed.clone().unwrap_or_else(|| model.clone());
        let final_body = emitter.final_response_object(
            if finish_reason.as_deref() == Some("tool_calls") { "incomplete" } else { "completed" },
        );
        let wire_body = serde_json::to_value(&final_body).unwrap_or(Value::Null);
        let finalize_inputs = crate::routes::finalize::FinalizeInputs {
            state: &state,
            auth: &auth,
            llm_event_id: &llm_event_id,
            generation_id: &response_id,
            started_at,
            created_at: created_at_dt,
            incremental_prompt_tokens: prompt_tokens,
            compression_tokens_saved,
            routing_metadata: routing_metadata.as_ref(),
            user: None,
            streamed: true,
        };
        crate::routes::finalize::finalize_streaming_at_end(
            &finalize_inputs,
            crate::routes::finalize::StreamingFinalizeSummary {
                provider: provider_for_finalize,
                model: model_for_finalize,
                prompt_tokens,
                completion_tokens,
                reasoning_tokens: reasoning_tokens_observed,
                finish_reason: finish_reason.clone(),
                content_preview: assistant_text.clone(),
            },
            &wire_body,
        ).await;

        // Persist the session on success (best-effort — don't fail
        // the stream if the write fails).
        if store_flag {
            if let Some(store) = session_store.as_ref() {
                let mut merged_all = merged_messages;
                let assistant_msg = lr_providers::ChatMessage {
                    role: "assistant".into(),
                    content: lr_providers::ChatMessageContent::Text(assistant_text),
                    tool_calls: if tool_calls.is_empty() { None } else { Some(tool_calls) },
                    tool_call_id: None,
                    name: None,
                    reasoning_content: None,
                };
                merged_all.push(assistant_msg);
                let (messages_json, tools_json) = serialize_history(&merged_all, merged_tools.as_deref());
                let status = if finish_reason.as_deref() == Some("tool_calls") {
                    "incomplete"
                } else {
                    "completed"
                };
                let session = ResponsesSession {
                    id: response_id.clone(),
                    client_id: api_key_id,
                    previous_response_id,
                    model,
                    created_at,
                    last_activity: Utc::now().timestamp(),
                    store: true,
                    metadata_json,
                    messages_json,
                    tools_json,
                    final_response_json: serde_json::to_string(
                        &emitter.final_response_object(status),
                    )
                    .ok(),
                };
                if let Err(e) = store.insert(&session) {
                    warn!("Failed to persist streaming /responses session: {}", e);
                }
            }
        }
    };

    Sse::new(emitted)
        .keep_alive(KeepAlive::default())
        .into_response()
}

fn sse_event(frame: ResponsesSseFrame) -> Result<axum::response::sse::Event, Infallible> {
    let data = serde_json::to_string(&frame.data).unwrap_or_else(|_| "{}".to_string());
    Ok(axum::response::sse::Event::default()
        .event(frame.event)
        .data(data))
}

/// Fold streaming `ToolCallDelta`s into a cumulative `ToolCall` list,
/// so the persisted session row captures the complete assistant turn
/// (name + assembled arguments) instead of just the last delta.
fn accumulate_tool_calls(
    sink: &mut Vec<lr_providers::ToolCall>,
    deltas: &[lr_providers::ToolCallDelta],
) {
    for d in deltas {
        let idx = d.index as usize;
        while sink.len() <= idx {
            sink.push(lr_providers::ToolCall {
                id: String::new(),
                tool_type: "function".into(),
                function: lr_providers::FunctionCall {
                    name: String::new(),
                    arguments: String::new(),
                },
            });
        }
        let entry = &mut sink[idx];
        if let Some(id) = &d.id {
            if entry.id.is_empty() {
                entry.id = id.clone();
            }
        }
        if let Some(tt) = &d.tool_type {
            entry.tool_type = tt.clone();
        }
        if let Some(func) = &d.function {
            if let Some(name) = &func.name {
                if entry.function.name.is_empty() {
                    entry.function.name = name.clone();
                }
            }
            if let Some(args) = &func.arguments {
                entry.function.arguments.push_str(args);
            }
        }
    }
}

// ============================================================================
// Session store accessor
// ============================================================================

/// Lazily open the SQLite session DB on first use and spin up the
/// background retention sweeper at the same time. Wiring the store
/// into `AppState::new()` would cascade through every call site —
/// this preserves the same single-instance semantics without the
/// churn, and degrades to `None` on IO errors so the route still
/// functions (just without persistence).
fn responses_session_store(state: &AppState) -> Option<ResponsesSessionStore> {
    use std::sync::OnceLock;
    static STORE: OnceLock<Option<ResponsesSessionStore>> = OnceLock::new();
    STORE
        .get_or_init(|| {
            let path = dirs::home_dir()?
                .join(".localrouter")
                .join("responses-sessions")
                .join("sessions.db");
            match ResponsesSessionStore::open(&path) {
                Ok(s) => {
                    // Periodic retention sweep: every hour, drop rows
                    // older than `retention_days`. The sweeper lives
                    // for the app lifetime; when the store handle is
                    // dropped the task exits naturally.
                    let swept = s.clone();
                    let cfg_manager = state.config_manager.clone();
                    tokio::spawn(async move {
                        let mut interval =
                            tokio::time::interval(std::time::Duration::from_secs(3600));
                        // Skip the first tick — it fires immediately
                        // and we just opened the DB.
                        interval.tick().await;
                        loop {
                            interval.tick().await;
                            let cfg = cfg_manager.get();
                            let retention = RetentionConfig {
                                retention_days: cfg.responses.retention_days as i64,
                                active_window_hours: cfg.responses.active_window_hours as i64,
                            };
                            if let Err(e) = swept.sweep_expired(&retention) {
                                warn!("Responses sessions sweep failed: {}", e);
                            }
                        }
                    });
                    Some(s)
                }
                Err(e) => {
                    warn!(
                        "Failed to open responses sessions DB at {}: {}",
                        path.display(),
                        e
                    );
                    None
                }
            }
        })
        .clone()
}

// ============================================================================
// Wire-body selection: native Responses JSON (when the provider
// produced it) vs. `completion_to_response_object` translation.
// ============================================================================

/// Overwrite the top-level `response.id` inside a Responses API SSE
/// envelope with LocalRouter's own response_id, leaving everything
/// else verbatim. Used in the streaming native pass-through path.
///
/// Envelopes come in two shapes, both handled here:
///   - Events wrapping a full response object: `{type, response: {...}}`
///   - Delta events referencing a response by id: `{type, response_id, ...}`
fn rewrite_envelope_response_id(mut envelope: Value, response_id: &str) -> Value {
    if let Some(obj) = envelope.as_object_mut() {
        // Shape 1: `{type, response: {id, ...}}`
        if let Some(resp) = obj.get_mut("response").and_then(|v| v.as_object_mut()) {
            resp.insert("id".to_string(), Value::String(response_id.to_string()));
        }
        // Shape 2: `{type, response_id: "..."}`
        if obj.contains_key("response_id") {
            obj.insert(
                "response_id".to_string(),
                Value::String(response_id.to_string()),
            );
        }
    }
    envelope
}

/// Pick the best wire-format body for the `/v1/responses` reply.
///
/// When the provider (today: ChatGPT Plus via `OpenAIProvider`)
/// stashed the verbatim upstream Responses API JSON on
/// `completion.extensions[NATIVE_RESPONSES_API_EXT_KEY]`, we use it
/// directly — this is the native pass-through path that preserves
/// reasoning items, encrypted-content carry-over, and built-in tool
/// results that `response_to_completion` silently drops.
///
/// Otherwise we fall back to `completion_to_response_object`, which
/// synthesizes a minimal Responses-shaped body from the
/// ChatCompletion result.
///
/// In both cases we overwrite the top-level `id` with LocalRouter's
/// own `response_id` so clients' `previous_response_id` round-trips
/// hit our session store, not the upstream's opaque handle.
fn select_wire_body(
    completion: &lr_providers::CompletionResponse,
    response_id: &str,
    created_at: i64,
) -> Value {
    let native_raw = completion
        .extensions
        .as_ref()
        .and_then(|ext| {
            ext.get(lr_providers::openai_responses::NATIVE_RESPONSES_API_EXT_KEY)
                .cloned()
        })
        .map(|mut v| {
            if let Some(obj) = v.as_object_mut() {
                obj.insert("id".to_string(), Value::String(response_id.to_string()));
            }
            v
        });
    native_raw.unwrap_or_else(|| completion_to_response_object(completion, response_id, created_at))
}

#[cfg(test)]
mod tests {
    use super::*;
    use lr_providers::{
        ChatMessage as ProvMsg, ChatMessageContent as ProvContent, CompletionChoice,
        CompletionResponse, TokenUsage as ProvUsage,
    };

    fn make_completion(
        ext_payload: Option<serde_json::Value>,
        content: &str,
    ) -> CompletionResponse {
        let mut completion = CompletionResponse {
            id: "upstream-id".into(),
            object: "chat.completion".into(),
            created: 0,
            provider: "openai".into(),
            model: "gpt-5.4".into(),
            choices: vec![CompletionChoice {
                index: 0,
                message: ProvMsg {
                    role: "assistant".into(),
                    content: ProvContent::Text(content.into()),
                    tool_calls: None,
                    tool_call_id: None,
                    name: None,
                    reasoning_content: None,
                },
                finish_reason: Some("stop".into()),
                logprobs: None,
            }],
            usage: ProvUsage {
                prompt_tokens: 0,
                completion_tokens: 0,
                total_tokens: 0,
                prompt_tokens_details: None,
                completion_tokens_details: None,
            },
            system_fingerprint: None,
            service_tier: None,
            extensions: None,
            routellm_win_rate: None,
            request_usage_entries: None,
        };
        if let Some(payload) = ext_payload {
            let mut map = std::collections::HashMap::new();
            map.insert(
                lr_providers::openai_responses::NATIVE_RESPONSES_API_EXT_KEY.to_string(),
                payload,
            );
            completion.extensions = Some(map);
        }
        completion
    }

    #[test]
    fn select_wire_body_prefers_native_when_present() {
        // Native JSON contains a reasoning output item that the
        // ChatCompletion translator drops. We expect it to reach the
        // wire body verbatim, with only the `id` overwritten.
        let native = serde_json::json!({
            "id": "chatgpt-opaque-id",
            "object": "response",
            "output": [
                {
                    "type": "reasoning",
                    "summary": [{"type": "summary_text", "text": "let me think"}],
                    "encrypted_content": "BASE64-opaque-reasoning"
                },
                {
                    "type": "message",
                    "role": "assistant",
                    "content": [{"type": "output_text", "text": "answer"}]
                }
            ],
            "status": "completed"
        });
        let completion = make_completion(Some(native.clone()), "answer");

        let wire = select_wire_body(&completion, "resp_lr_123", 42);

        assert_eq!(
            wire["id"].as_str(),
            Some("resp_lr_123"),
            "LocalRouter id must overwrite upstream id"
        );
        // Reasoning output item preserved — this is the whole point.
        assert_eq!(wire["output"][0]["type"].as_str(), Some("reasoning"));
        assert_eq!(
            wire["output"][0]["encrypted_content"].as_str(),
            Some("BASE64-opaque-reasoning")
        );
        // Message item still there after reasoning.
        assert_eq!(wire["output"][1]["type"].as_str(), Some("message"));
    }

    #[test]
    fn select_wire_body_falls_back_to_translator_when_native_missing() {
        // No extension key → use `completion_to_response_object` which
        // synthesizes a Responses-shaped body from the ChatCompletion.
        let completion = make_completion(None, "hello from fallback");

        let wire = select_wire_body(&completion, "resp_lr_fb", 100);

        assert_eq!(wire["id"].as_str(), Some("resp_lr_fb"));
        assert!(
            wire["output"].is_array(),
            "translator always produces an output array: {wire}"
        );
        // Message content must contain the assistant's text.
        let text = wire["output"][0]["content"][0]["text"].as_str().unwrap();
        assert_eq!(text, "hello from fallback");
    }

    #[test]
    fn rewrite_envelope_handles_both_shapes() {
        // Shape 1: event carrying a full response object under
        // `response`. Typical of `response.created` / `completed`.
        let shape1 = serde_json::json!({
            "type": "response.created",
            "response": {
                "id": "chatgpt-opaque-id",
                "status": "in_progress"
            }
        });
        let rewritten = rewrite_envelope_response_id(shape1, "resp_lr_1");
        assert_eq!(rewritten["response"]["id"].as_str(), Some("resp_lr_1"));
        assert_eq!(
            rewritten["response"]["status"].as_str(),
            Some("in_progress")
        );

        // Shape 2: delta event referencing a response by id. Typical
        // of `response.output_text.delta`.
        let shape2 = serde_json::json!({
            "type": "response.output_text.delta",
            "response_id": "chatgpt-opaque-id",
            "delta": "hello"
        });
        let rewritten = rewrite_envelope_response_id(shape2, "resp_lr_2");
        assert_eq!(rewritten["response_id"].as_str(), Some("resp_lr_2"));
        assert_eq!(rewritten["delta"].as_str(), Some("hello"));

        // Envelope with neither field — pass-through untouched.
        let shape_other = serde_json::json!({
            "type": "response.in_progress",
            "foo": "bar"
        });
        let rewritten = rewrite_envelope_response_id(shape_other.clone(), "resp_lr_3");
        assert_eq!(rewritten, shape_other);
    }

    #[test]
    fn select_wire_body_rewrites_id_even_if_upstream_omits_it() {
        // Edge case: upstream JSON object has no `id` key at all.
        // Insertion path still works (serde_json inserts, doesn't
        // require pre-existing key).
        let native = serde_json::json!({
            "object": "response",
            "output": [],
            "status": "completed"
        });
        let completion = make_completion(Some(native), "");

        let wire = select_wire_body(&completion, "resp_lr_new", 0);
        assert_eq!(wire["id"].as_str(), Some("resp_lr_new"));
    }
}
