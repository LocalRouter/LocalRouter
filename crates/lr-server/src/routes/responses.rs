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
//! Known gaps documented as TODOs:
//! - Guardrails, MCP-via-LLM orchestration, and firewall interception
//!   are NOT yet integrated (they live inline in chat.rs). Clients
//!   needing those should stay on `/v1/chat/completions` for now; a
//!   follow-up refactor will extract the pipeline into
//!   `routes/shared.rs` so both endpoints pick it up.
//! - Only `input` arrays with `message` / `function_call` /
//!   `function_call_output` items are handled; reasoning items and
//!   custom tools pass through untouched but aren't actively
//!   interpreted.

use std::convert::Infallible;

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
use lr_providers::{
    ChatMessage, ChatMessageContent, CompletionRequest, ContentPart, ImageUrl, ResponseFormat, Tool,
};
use lr_responses_sessions::{
    deserialize_history, serialize_history, ResponsesSession, ResponsesSessionStore,
    RetentionConfig,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::{debug, warn};
use uuid::Uuid;

use crate::middleware::error::{ApiErrorResponse, ApiResult};
use crate::state::{AppState, AuthContext};

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
    Json(req): Json<CreateResponseRequest>,
) -> ApiResult<Response> {
    // Correlate logs/monitor events across the turn.
    let session_id = Uuid::new_v4().to_string();
    let response_id = format!("resp_{}", Uuid::new_v4().simple());
    let created_at = Utc::now().timestamp();

    // Emit request-side monitor event so traffic inspection sees this
    // hit like any other /v1/chat/completions request. (Intentionally
    // shares the `/v1/responses` path string so the monitor UI shows
    // the right endpoint column.)
    let request_json = serde_json::to_value(&req).unwrap_or(Value::Null);
    let mut llm_guard = super::monitor_helpers::emit_llm_call(
        &state,
        None, // client_auth not threaded yet for the responses route
        Some(&session_id),
        "/v1/responses",
        &req.model,
        req.stream,
        &request_json,
    );

    let store_flag = req.store.unwrap_or(true);

    // Load session store lazily so a broken DB doesn't block the whole
    // server. Failures degrade to no-persistence behaviour.
    let session_store = responses_session_store(&state);
    // Pull retention from live config so users can override via
    // settings.yaml without rebuilding.
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
                // Not a hard error — clients legitimately hit stale ids;
                // we treat them as "start fresh" to avoid 404 loops.
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

    // Translate the inbound request into chat-completions shape.
    let provider_request = build_provider_request(&req, prior_messages, prior_tools.clone())?;

    // Merged history the session row will capture on success.
    let merged_messages = provider_request.messages.clone();
    let merged_tools = provider_request.tools.clone();

    if req.stream {
        let stream = match state
            .router
            .stream_complete(&auth.api_key_id, provider_request)
            .await
        {
            Ok((s, _routing_meta)) => s,
            Err(e) => {
                return Err(llm_guard.capture_err(ApiErrorResponse::bad_gateway(format!(
                    "Router error: {}",
                    e
                ))));
            }
        };

        let emit_sse = build_stream_response(
            state.clone(),
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
            },
        );

        // `llm_guard`'s Drop impl records error-on-drop; consume it
        // explicitly since this SSE stream path completes asynchronously
        // and we've already handed off to the stream.
        let _ = llm_guard.into_event_id();
        return Ok(emit_sse);
    }

    // Non-streaming path.
    let (completion, _routing_meta) = state
        .router
        .complete(&auth.api_key_id, provider_request)
        .await
        .map_err(|e| {
            llm_guard.capture_err(ApiErrorResponse::bad_gateway(format!(
                "Router error: {}",
                e
            )))
        })?;

    let response_object = completion_to_response_object(&completion, &response_id, created_at);

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

    let _ = llm_guard.into_event_id();
    Ok(JsonResponse(response_object).into_response())
}

// ============================================================================
// Translation: CreateResponseRequest → CompletionRequest (provider shape)
// ============================================================================

fn build_provider_request(
    req: &CreateResponseRequest,
    prior_messages: Vec<ChatMessage>,
    prior_tools: Option<Vec<Tool>>,
) -> ApiResult<CompletionRequest> {
    let mut messages: Vec<ChatMessage> = prior_messages;
    if let Some(instr) = req.instructions.as_deref() {
        if !instr.is_empty() {
            messages.insert(
                0,
                ChatMessage {
                    role: "system".into(),
                    content: ChatMessageContent::Text(instr.to_string()),
                    tool_calls: None,
                    tool_call_id: None,
                    name: None,
                    reasoning_content: None,
                },
            );
        }
    }

    // Translate input items into ChatMessages.
    let new_messages = match &req.input {
        ResponseInput::Text(s) => vec![ChatMessage {
            role: "user".into(),
            content: ChatMessageContent::Text(s.clone()),
            tool_calls: None,
            tool_call_id: None,
            name: None,
            reasoning_content: None,
        }],
        ResponseInput::Items(items) => items
            .iter()
            .filter_map(response_item_to_chat_message)
            .collect(),
    };
    messages.extend(new_messages);

    // Tools: prefer caller's `tools` over any prior ones; if caller
    // didn't specify, fall back to what we stored in the prior turn
    // so chained turns don't lose them.
    let tools: Option<Vec<Tool>> = match req.tools.as_ref() {
        Some(list) => Some(list.iter().filter_map(value_to_tool).collect()),
        None => prior_tools,
    };

    let tool_choice = req.tool_choice.as_ref().and_then(value_to_tool_choice);

    let response_format = req
        .response_format
        .as_ref()
        .and_then(value_to_response_format);

    Ok(CompletionRequest {
        model: req.model.clone(),
        messages,
        temperature: req.temperature,
        max_tokens: req.max_output_tokens,
        stream: req.stream,
        top_p: req.top_p,
        frequency_penalty: None,
        presence_penalty: None,
        stop: None,
        top_k: None,
        seed: None,
        repetition_penalty: None,
        extensions: None,
        tools,
        tool_choice,
        response_format,
        logprobs: None,
        top_logprobs: None,
        n: None,
        logit_bias: None,
        parallel_tool_calls: req.parallel_tool_calls,
        service_tier: None,
        store: req.store,
        metadata: None,
        modalities: None,
        audio: None,
        prediction: None,
        reasoning_effort: req.reasoning.as_ref().and_then(|r| r.effort.clone()),
        pre_computed_routing: None,
    })
}

fn response_item_to_chat_message(item: &Value) -> Option<ChatMessage> {
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
            let content = content_from_response_parts(content);
            Some(ChatMessage {
                role,
                content,
                tool_calls: None,
                tool_call_id: None,
                name: None,
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
            Some(ChatMessage {
                role: "assistant".into(),
                content: ChatMessageContent::Text(String::new()),
                tool_calls: Some(vec![lr_providers::ToolCall {
                    id: call_id,
                    tool_type: "function".into(),
                    function: lr_providers::FunctionCall { name, arguments },
                }]),
                tool_call_id: None,
                name: None,
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
            Some(ChatMessage {
                role: "tool".into(),
                content: ChatMessageContent::Text(output),
                tool_calls: None,
                tool_call_id: Some(call_id),
                name: None,
                reasoning_content: None,
            })
        }
        _ => None, // reasoning, custom tool calls, etc. — dropped for v1
    }
}

fn content_from_response_parts(content: &Value) -> ChatMessageContent {
    // Either a single string or an array of typed parts.
    if let Some(s) = content.as_str() {
        return ChatMessageContent::Text(s.to_string());
    }
    let Some(arr) = content.as_array() else {
        return ChatMessageContent::Text(String::new());
    };
    let mut parts: Vec<ContentPart> = Vec::new();
    for p in arr {
        let obj = match p.as_object() {
            Some(o) => o,
            None => continue,
        };
        match obj.get("type").and_then(|v| v.as_str()) {
            Some("input_text") | Some("output_text") => {
                if let Some(text) = obj.get("text").and_then(|v| v.as_str()) {
                    parts.push(ContentPart::Text {
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
                parts.push(ContentPart::ImageUrl {
                    image_url: ImageUrl { url, detail },
                });
            }
            _ => {}
        }
    }
    if parts.is_empty() {
        ChatMessageContent::Text(String::new())
    } else {
        ChatMessageContent::Parts(parts)
    }
}

fn value_to_tool(v: &Value) -> Option<Tool> {
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
    Some(Tool {
        tool_type,
        function: lr_providers::FunctionDefinition {
            name,
            description,
            parameters,
        },
    })
}

fn value_to_tool_choice(v: &Value) -> Option<lr_providers::ToolChoice> {
    if let Some(s) = v.as_str() {
        return Some(lr_providers::ToolChoice::Auto(s.to_string()));
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
    Some(lr_providers::ToolChoice::Specific {
        tool_type,
        function: lr_providers::FunctionName {
            name: name.to_string(),
        },
    })
}

fn value_to_response_format(v: &Value) -> Option<ResponseFormat> {
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
        return Some(ResponseFormat::JsonSchema {
            format_type,
            schema,
        });
    }
    Some(ResponseFormat::JsonObject { format_type })
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
}

fn build_stream_response(
    _state: AppState,
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
    } = ctx;

    // Produce SSE `Event`s as the chat-completions stream flows in.
    let mut emitter = ResponsesEmitter::new(response_id.clone(), model.clone(), created_at);
    let mut finish_reason: Option<String> = None;
    let mut assistant_text = String::new();
    let mut tool_calls: Vec<lr_providers::ToolCall> = Vec::new();

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
            for frame in emitter.on_chunk(chunk) {
                yield sse_event(frame);
            }
        }

        // Finish frames.
        for frame in emitter.finish(finish_reason.as_deref()) {
            yield sse_event(frame);
        }

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
