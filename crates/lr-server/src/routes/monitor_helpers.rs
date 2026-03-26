//! Helper functions for emitting monitor events from route handlers.
//!
//! Combined events follow the emit-then-update pattern:
//! 1. `emit_*()` creates a Pending event with request data, returns an `LlmCallGuard`
//! 2. The guard auto-completes the event as Error on drop if not explicitly completed
//! 3. `complete_*()` updates the event with response data and sets status to Complete/Error

use crate::middleware::client_auth::ClientAuthContext;
use crate::middleware::error::ApiErrorResponse;
use crate::state::AppState;
use axum::Extension;
use lr_monitor::{EventStatus, MonitorEventData, MonitorEventGuard, MonitorEventType};

// ---- LLM Call Guard ----

/// RAII guard for LlmCall monitor events.
///
/// Wraps a `MonitorEventGuard` and provides LLM-specific completion methods.
/// If dropped without calling `complete()`, `complete_error()`, or `into_event_id()`,
/// the event is automatically marked as Error.
pub struct LlmCallGuard {
    inner: MonitorEventGuard,
}

impl LlmCallGuard {
    /// Get the event ID.
    pub fn event_id(&self) -> &str {
        self.inner.event_id()
    }

    /// Complete the LLM call event with success data, consuming the guard.
    #[allow(clippy::too_many_arguments)]
    pub fn complete(
        self,
        state: &AppState,
        provider: &str,
        model: &str,
        status_code: u16,
        input_tokens: u64,
        output_tokens: u64,
        reasoning_tokens_val: Option<u64>,
        cost_usd: Option<f64>,
        latency_ms: u64,
        finish_reason: Option<&str>,
        content_preview: &str,
        streamed: bool,
    ) {
        let event_id = self.inner.defuse();
        complete_llm_call(
            state,
            &event_id,
            provider,
            model,
            status_code,
            input_tokens,
            output_tokens,
            reasoning_tokens_val,
            cost_usd,
            latency_ms,
            finish_reason,
            content_preview,
            streamed,
        );
    }

    /// Complete the LLM call event with an error, consuming the guard.
    pub fn complete_error(
        self,
        state: &AppState,
        provider: &str,
        model: &str,
        status_code: u16,
        error_msg: &str,
    ) {
        let event_id = self.inner.defuse();
        complete_llm_call_error(state, &event_id, provider, model, status_code, error_msg);
    }

    /// Capture error context from an `ApiErrorResponse` onto the guard, then
    /// return the error unchanged.  This lets callers write:
    ///
    /// ```ignore
    /// validate_something().map_err(|e| llm_guard.capture_err(e))?;
    /// // or
    /// return Err(llm_guard.capture_err(ApiErrorResponse::not_found("...")));
    /// ```
    ///
    /// If the guard is dropped without explicit completion, the `Drop` impl
    /// will use this captured status code and message instead of a generic fallback.
    pub fn capture_err(&mut self, err: ApiErrorResponse) -> ApiErrorResponse {
        self.inner
            .set_early_error(err.status.as_u16(), &err.error.error.message);
        err
    }

    /// Extract the event ID, defusing the guard.
    ///
    /// Use this when handing off the event ID to a spawned task (e.g., streaming
    /// response handlers) that manages its own completion calls.
    pub fn into_event_id(self) -> String {
        self.inner.defuse()
    }
}

// ---- LLM Call (combined: request + transform + response/error) ----

/// Emit a pending LlmCall event at the start of a request. Returns an `LlmCallGuard`
/// that auto-completes the event as Error if dropped without explicit completion.
pub fn emit_llm_call(
    state: &AppState,
    client_auth: Option<&Extension<ClientAuthContext>>,
    session_id: Option<&str>,
    endpoint: &str,
    model: &str,
    stream: bool,
    request_body: &serde_json::Value,
) -> LlmCallGuard {
    let (client_id, client_name) = resolve_client(state, client_auth);

    let message_count = request_body
        .get("messages")
        .and_then(|m| m.as_array())
        .map(|a| a.len())
        .unwrap_or(0);

    let tools = request_body.get("tools").and_then(|t| t.as_array());
    let has_tools = tools.is_some_and(|t| !t.is_empty());
    let tool_count = tools.map(|t| t.len()).unwrap_or(0);

    let event_id = state.monitor_store.push(
        MonitorEventType::LlmCall,
        client_id,
        client_name,
        session_id.map(|s| s.to_string()),
        MonitorEventData::LlmCall {
            endpoint: endpoint.to_string(),
            model: model.to_string(),
            stream,
            message_count,
            has_tools,
            tool_count,
            request_body: truncate_json(request_body, 10_000),
            transformed_body: None,
            transformations_applied: None,
            provider: None,
            status_code: None,
            input_tokens: None,
            output_tokens: None,
            total_tokens: None,
            reasoning_tokens: None,
            cost_usd: None,
            latency_ms: None,
            finish_reason: None,
            content_preview: None,
            streamed: None,
            response_body: None,
            error: None,
            routing_info: None,
        },
        EventStatus::Pending,
        None,
    );

    LlmCallGuard {
        inner: MonitorEventGuard::new(
            state.monitor_store.clone(),
            event_id,
            MonitorEventType::LlmCall,
        ),
    }
}

/// Update the LlmCall event with transformation data (compression, MCP tools, etc.).
pub fn update_llm_call_transformed(
    state: &AppState,
    event_id: &str,
    request_body: &serde_json::Value,
    transformations: Vec<String>,
) {
    let body = truncate_json(request_body, 10_000);
    state.monitor_store.update(event_id, |event| {
        if let MonitorEventData::LlmCall {
            transformed_body,
            transformations_applied,
            ..
        } = &mut event.data
        {
            *transformed_body = Some(body);
            *transformations_applied = Some(transformations);
        }
    });
}

/// Complete the LlmCall event with response data (non-streaming or stream finished).
#[allow(clippy::too_many_arguments)]
pub fn complete_llm_call(
    state: &AppState,
    event_id: &str,
    provider: &str,
    model: &str,
    status_code: u16,
    input_tokens: u64,
    output_tokens: u64,
    reasoning_tokens_val: Option<u64>,
    cost_usd: Option<f64>,
    latency_ms: u64,
    finish_reason: Option<&str>,
    content_preview: &str,
    streamed: bool,
) {
    let provider = provider.to_string();
    let model = model.to_string();
    let finish_reason = finish_reason.map(|s| s.to_string());
    let content_preview = truncate_string(content_preview, 2000);

    state.monitor_store.update(event_id, |event| {
        event.status = EventStatus::Complete;
        event.duration_ms = Some(latency_ms);
        if let MonitorEventData::LlmCall {
            model: ref mut m,
            provider: ref mut p,
            status_code: ref mut sc,
            input_tokens: ref mut it,
            output_tokens: ref mut ot,
            total_tokens: ref mut tt,
            reasoning_tokens: ref mut rt,
            cost_usd: ref mut cu,
            latency_ms: ref mut lm,
            finish_reason: ref mut fr,
            content_preview: ref mut cp,
            streamed: ref mut st,
            ..
        } = &mut event.data
        {
            *m = model;
            *p = Some(provider);
            *sc = Some(status_code);
            *it = Some(input_tokens);
            *ot = Some(output_tokens);
            *tt = Some(input_tokens + output_tokens);
            *rt = reasoning_tokens_val;
            *cu = cost_usd;
            *lm = Some(latency_ms);
            *fr = finish_reason;
            *cp = Some(content_preview);
            *st = Some(streamed);
        }
    });
}

/// Complete the LlmCall event with an error.
pub fn complete_llm_call_error(
    state: &AppState,
    event_id: &str,
    provider: &str,
    model: &str,
    status_code: u16,
    error_msg: &str,
) {
    let provider = provider.to_string();
    let model = model.to_string();
    let error_msg = truncate_string(error_msg, 1000);

    // Build a response body representing the error so it's visible in the monitor UI
    let error_body = serde_json::json!({
        "error": {
            "message": &error_msg,
            "type": "provider_error",
            "code": status_code,
        }
    });

    state.monitor_store.update(event_id, |event| {
        event.status = EventStatus::Error;
        if let MonitorEventData::LlmCall {
            model: ref mut m,
            provider: ref mut p,
            status_code: ref mut sc,
            error: ref mut e,
            response_body: ref mut rb,
            ..
        } = &mut event.data
        {
            *m = model;
            *p = Some(provider);
            *sc = Some(status_code);
            *e = Some(error_msg);
            *rb = Some(error_body);
        }
    });
}

/// Update the LlmCall event with auto-routing metadata.
///
/// Called when auto-routing was used, to record which models were tried and why.
pub fn update_llm_call_routing(
    state: &AppState,
    event_id: &str,
    routing_metadata: &serde_json::Value,
) {
    match serde_json::from_value::<lr_monitor::AutoRoutingInfo>(routing_metadata.clone()) {
        Ok(info) => {
            state.monitor_store.update(event_id, |event| {
                if let MonitorEventData::LlmCall {
                    routing_info: ref mut ri,
                    ..
                } = &mut event.data
                {
                    *ri = Some(info);
                }
            });
        }
        Err(e) => {
            tracing::warn!("Failed to deserialize routing metadata: {}", e);
        }
    }
}

/// Update the LlmCall event with the full response body JSON.
///
/// Called after building the API response, so the event already has status/metrics
/// but gains the full response for inspection in the monitor UI.
pub fn update_llm_call_response_body(
    state: &AppState,
    event_id: &str,
    response_body: &serde_json::Value,
) {
    let body = truncate_json(response_body, 10_000);
    state.monitor_store.update(event_id, |event| {
        if let MonitorEventData::LlmCall {
            response_body: ref mut rb,
            ..
        } = &mut event.data
        {
            *rb = Some(body);
        }
    });
}

/// Build a synthetic response body JSON for streaming completions.
///
/// Since streaming doesn't produce a single response object, this reconstructs
/// a response-like JSON from the accumulated data for monitor UI inspection.
pub fn build_streaming_response_body(
    generation_id: &str,
    model: &str,
    content: &str,
    finish_reason: &str,
    prompt_tokens: u64,
    completion_tokens: u64,
    created_timestamp: i64,
) -> serde_json::Value {
    serde_json::json!({
        "id": generation_id,
        "object": "chat.completion",
        "created": created_timestamp,
        "model": model,
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": if content.is_empty() { serde_json::Value::Null } else { serde_json::Value::String(content.to_string()) },
            },
            "finish_reason": finish_reason,
        }],
        "usage": {
            "prompt_tokens": prompt_tokens,
            "completion_tokens": completion_tokens,
            "total_tokens": prompt_tokens + completion_tokens,
        },
        "_streaming": true,
        "_note": "Reconstructed from streaming response chunks",
    })
}

// ---- Auth & Access Control events ----

/// Emit an AuthError monitor event (middleware-level auth failures).
pub fn emit_auth_error(
    state: &AppState,
    error_type: &str,
    endpoint: &str,
    message: &str,
    status_code: u16,
) {
    state.monitor_store.push(
        MonitorEventType::AuthError,
        None,
        None,
        None,
        MonitorEventData::AuthError {
            error_type: error_type.to_string(),
            endpoint: endpoint.to_string(),
            message: message.to_string(),
            status_code,
        },
        EventStatus::Error,
        None,
    );
}

/// Emit an AccessDenied monitor event (route-level access control).
pub fn emit_access_denied(
    state: &AppState,
    client_auth: Option<&Extension<ClientAuthContext>>,
    session_id: Option<&str>,
    reason: &str,
    endpoint: &str,
    message: &str,
    status_code: u16,
) {
    let (client_id, client_name) = resolve_client(state, client_auth);

    state.monitor_store.push(
        MonitorEventType::AccessDenied,
        client_id,
        client_name,
        session_id.map(|s| s.to_string()),
        MonitorEventData::AccessDenied {
            reason: reason.to_string(),
            endpoint: endpoint.to_string(),
            message: message.to_string(),
            status_code,
        },
        EventStatus::Error,
        None,
    );
}

/// Emit an AccessDenied event using client_id directly (for routes without ClientAuthContext).
pub fn emit_access_denied_for_client(
    state: &AppState,
    client_id: &str,
    session_id: Option<&str>,
    reason: &str,
    endpoint: &str,
    message: &str,
    status_code: u16,
) {
    let client_name = state
        .client_manager
        .get_client(client_id)
        .map(|c| c.name.clone());

    state.monitor_store.push(
        MonitorEventType::AccessDenied,
        Some(client_id.to_string()),
        client_name,
        session_id.map(|s| s.to_string()),
        MonitorEventData::AccessDenied {
            reason: reason.to_string(),
            endpoint: endpoint.to_string(),
            message: message.to_string(),
            status_code,
        },
        EventStatus::Error,
        None,
    );
}

// ---- Rate Limiting events ----

/// Emit a RateLimitEvent monitor event.
#[allow(clippy::too_many_arguments)]
pub fn emit_rate_limit_event(
    state: &AppState,
    client_auth: Option<&Extension<ClientAuthContext>>,
    session_id: Option<&str>,
    reason: &str,
    endpoint: &str,
    message: &str,
    status_code: u16,
    retry_after_secs: Option<u64>,
) {
    let (client_id, client_name) = resolve_client(state, client_auth);

    state.monitor_store.push(
        MonitorEventType::RateLimitEvent,
        client_id,
        client_name,
        session_id.map(|s| s.to_string()),
        MonitorEventData::RateLimitEvent {
            reason: reason.to_string(),
            endpoint: endpoint.to_string(),
            message: message.to_string(),
            status_code,
            retry_after_secs,
        },
        EventStatus::Error,
        None,
    );
}

// ---- Validation events ----

/// Emit a ValidationError monitor event.
pub fn emit_validation_error(
    state: &AppState,
    client_auth: Option<&Extension<ClientAuthContext>>,
    session_id: Option<&str>,
    endpoint: &str,
    field: Option<&str>,
    message: &str,
    status_code: u16,
) {
    let (client_id, client_name) = resolve_client(state, client_auth);

    state.monitor_store.push(
        MonitorEventType::ValidationError,
        client_id,
        client_name,
        session_id.map(|s| s.to_string()),
        MonitorEventData::ValidationError {
            endpoint: endpoint.to_string(),
            field: field.map(|f| f.to_string()),
            message: message.to_string(),
            status_code,
        },
        EventStatus::Error,
        None,
    );
}

// ---- Moderation events ----

/// Emit a ModerationEvent monitor event.
pub fn emit_moderation_event(state: &AppState, reason: &str, message: &str, status_code: u16) {
    state.monitor_store.push(
        MonitorEventType::ModerationEvent,
        None,
        None,
        None,
        MonitorEventData::ModerationEvent {
            reason: reason.to_string(),
            message: message.to_string(),
            status_code,
        },
        EventStatus::Error,
        None,
    );
}

// ---- OAuth events ----

/// Emit an OAuthEvent monitor event.
pub fn emit_oauth_event(
    state: &AppState,
    action: &str,
    client_id_hint: Option<&str>,
    message: &str,
    status_code: u16,
) {
    state.monitor_store.push(
        MonitorEventType::OAuthEvent,
        client_id_hint.map(|s| s.to_string()),
        None,
        None,
        MonitorEventData::OAuthEvent {
            action: action.to_string(),
            client_id_hint: client_id_hint.map(|s| s.to_string()),
            message: message.to_string(),
            status_code,
        },
        EventStatus::Error,
        None,
    );
}

// ---- Internal error events ----

/// Emit an InternalError monitor event.
pub fn emit_internal_error(state: &AppState, error_type: &str, message: &str, status_code: u16) {
    state.monitor_store.push(
        MonitorEventType::InternalError,
        None,
        None,
        None,
        MonitorEventData::InternalError {
            error_type: error_type.to_string(),
            message: truncate_string(message, 1000),
            status_code,
        },
        EventStatus::Error,
        None,
    );
}

// ---- Guardrail events (combined: request + response) ----

/// Emit a pending GuardrailScan event before running input safety checks. Returns event ID.
pub fn emit_guardrail_scan(
    state: &AppState,
    client_ctx: Option<&ClientAuthContext>,
    session_id: Option<&str>,
    direction: &str,
    text_preview: &str,
    models_used: Vec<String>,
) -> String {
    let (client_id, client_name) = resolve_client_ctx(state, client_ctx);
    state.monitor_store.push(
        MonitorEventType::GuardrailScan,
        client_id,
        client_name,
        session_id.map(|s| s.to_string()),
        MonitorEventData::GuardrailScan {
            direction: direction.to_string(),
            text_preview: truncate_string(text_preview, 500),
            models_used,
            result: None,
            flagged_categories: None,
            action_taken: None,
            latency_ms: None,
        },
        EventStatus::Pending,
        None,
    )
}

/// Complete a GuardrailScan event with the scan result.
pub fn complete_guardrail_scan(
    state: &AppState,
    event_id: &str,
    result: &str,
    flagged_categories: Vec<lr_monitor::FlaggedCategory>,
    action_taken: &str,
    latency_ms: u64,
) {
    state.monitor_store.update(event_id, |event| {
        event.status = EventStatus::Complete;
        event.duration_ms = Some(latency_ms);
        if let MonitorEventData::GuardrailScan {
            result: ref mut r,
            flagged_categories: ref mut fc,
            action_taken: ref mut at,
            latency_ms: ref mut lm,
            ..
        } = &mut event.data
        {
            *r = Some(result.to_string());
            *fc = Some(flagged_categories);
            *at = Some(action_taken.to_string());
            *lm = Some(latency_ms);
        }
    });
}

/// Emit a pending GuardrailResponseScan event for output safety checks. Returns event ID.
pub fn emit_guardrail_response_scan(
    state: &AppState,
    client_ctx: Option<&ClientAuthContext>,
    session_id: Option<&str>,
    direction: &str,
    text_preview: &str,
    models_used: Vec<String>,
) -> String {
    let (client_id, client_name) = resolve_client_ctx(state, client_ctx);
    state.monitor_store.push(
        MonitorEventType::GuardrailResponseScan,
        client_id,
        client_name,
        session_id.map(|s| s.to_string()),
        MonitorEventData::GuardrailResponseScan {
            direction: direction.to_string(),
            text_preview: truncate_string(text_preview, 500),
            models_used,
            result: None,
            flagged_categories: None,
            action_taken: None,
            latency_ms: None,
        },
        EventStatus::Pending,
        None,
    )
}

/// Complete a GuardrailResponseScan event with the scan result.
pub fn complete_guardrail_response_scan(
    state: &AppState,
    event_id: &str,
    result: &str,
    flagged_categories: Vec<lr_monitor::FlaggedCategory>,
    action_taken: &str,
    latency_ms: u64,
) {
    state.monitor_store.update(event_id, |event| {
        event.status = EventStatus::Complete;
        event.duration_ms = Some(latency_ms);
        if let MonitorEventData::GuardrailResponseScan {
            result: ref mut r,
            flagged_categories: ref mut fc,
            action_taken: ref mut at,
            latency_ms: ref mut lm,
            ..
        } = &mut event.data
        {
            *r = Some(result.to_string());
            *fc = Some(flagged_categories);
            *at = Some(action_taken.to_string());
            *lm = Some(latency_ms);
        }
    });
}

// ---- Secret scan events (combined: request + response) ----

/// Emit a pending SecretScan event before scanning. Returns event ID.
pub fn emit_secret_scan(
    state: &AppState,
    client_ctx: Option<&ClientAuthContext>,
    session_id: Option<&str>,
    text_preview: &str,
    rules_count: usize,
) -> String {
    let (client_id, client_name) = resolve_client_ctx(state, client_ctx);
    state.monitor_store.push(
        MonitorEventType::SecretScan,
        client_id,
        client_name,
        session_id.map(|s| s.to_string()),
        MonitorEventData::SecretScan {
            text_preview: truncate_string(text_preview, 500),
            rules_count,
            findings_count: None,
            findings: None,
            action_taken: None,
            latency_ms: None,
        },
        EventStatus::Pending,
        None,
    )
}

/// Complete a SecretScan event with scan results.
pub fn complete_secret_scan(
    state: &AppState,
    event_id: &str,
    findings_count: usize,
    findings: serde_json::Value,
    action_taken: &str,
    latency_ms: u64,
) {
    state.monitor_store.update(event_id, |event| {
        event.status = EventStatus::Complete;
        event.duration_ms = Some(latency_ms);
        if let MonitorEventData::SecretScan {
            findings_count: ref mut fc,
            findings: ref mut f,
            action_taken: ref mut at,
            latency_ms: ref mut lm,
            ..
        } = &mut event.data
        {
            *fc = Some(findings_count);
            *f = Some(findings);
            *at = Some(action_taken.to_string());
            *lm = Some(latency_ms);
        }
    });
}

// ---- Routing events ----

/// Emit a RoutingDecision event when final model routing is determined.
#[allow(clippy::too_many_arguments)]
pub fn emit_routing_decision(
    state: &AppState,
    client_ctx: Option<&ClientAuthContext>,
    session_id: Option<&str>,
    routing_type: &str,
    original_model: &str,
    final_model: &str,
    candidate_models: Option<Vec<String>>,
    firewall_action: Option<&str>,
) {
    let (client_id, client_name) = resolve_client_ctx(state, client_ctx);
    state.monitor_store.push(
        MonitorEventType::RoutingDecision,
        client_id,
        client_name,
        session_id.map(|s| s.to_string()),
        MonitorEventData::RoutingDecision {
            routing_type: routing_type.to_string(),
            original_model: original_model.to_string(),
            final_model: final_model.to_string(),
            candidate_models,
            firewall_action: firewall_action.map(|s| s.to_string()),
        },
        EventStatus::Complete,
        None,
    );
}

// ---- Prompt compression events ----

/// Emit a PromptCompression event after compression completes.
#[allow(clippy::too_many_arguments)]
pub fn emit_prompt_compression(
    state: &AppState,
    client_ctx: Option<&ClientAuthContext>,
    session_id: Option<&str>,
    original_tokens: u64,
    compressed_tokens: u64,
    reduction_percent: f64,
    duration_ms: u64,
    method: &str,
) {
    let (client_id, client_name) = resolve_client_ctx(state, client_ctx);
    state.monitor_store.push(
        MonitorEventType::PromptCompression,
        client_id,
        client_name,
        session_id.map(|s| s.to_string()),
        MonitorEventData::PromptCompression {
            original_tokens,
            compressed_tokens,
            reduction_percent,
            duration_ms,
            method: method.to_string(),
        },
        EventStatus::Complete,
        Some(duration_ms),
    );
}

// ---- Firewall decision events ----

/// Emit a FirewallDecision event when a popup is shown and the user responds.
pub fn emit_firewall_decision(
    state: &AppState,
    client_ctx: Option<&ClientAuthContext>,
    session_id: Option<&str>,
    firewall_type: &str,
    item_name: &str,
    action: &str,
    duration: Option<&str>,
) {
    let (client_id, client_name) = resolve_client_ctx(state, client_ctx);
    state.monitor_store.push(
        MonitorEventType::FirewallDecision,
        client_id,
        client_name,
        session_id.map(|s| s.to_string()),
        MonitorEventData::FirewallDecision {
            firewall_type: firewall_type.to_string(),
            item_name: item_name.to_string(),
            action: action.to_string(),
            duration: duration.map(|s| s.to_string()),
        },
        EventStatus::Complete,
        None,
    );
}

/// Resolve client ID and name from auth context.
fn resolve_client(
    state: &AppState,
    client_auth: Option<&Extension<ClientAuthContext>>,
) -> (Option<String>, Option<String>) {
    match client_auth {
        Some(ext) => {
            let client_id = ext.0.client_id.clone();
            let client_name = state
                .client_manager
                .get_client(&client_id)
                .map(|c| c.name.clone());
            (Some(client_id), client_name)
        }
        None => (None, None),
    }
}

/// Resolve client ID and name from direct ClientAuthContext (non-Extension).
fn resolve_client_ctx(
    state: &AppState,
    client_ctx: Option<&ClientAuthContext>,
) -> (Option<String>, Option<String>) {
    match client_ctx {
        Some(ctx) => {
            let client_id = ctx.client_id.clone();
            let client_name = state
                .client_manager
                .get_client(&client_id)
                .map(|c| c.name.clone());
            (Some(client_id), client_name)
        }
        None => (None, None),
    }
}

/// Truncate a JSON value to approximately max_bytes by converting to string,
/// truncating, and wrapping in a descriptive object if too large.
pub fn truncate_json(value: &serde_json::Value, max_bytes: usize) -> serde_json::Value {
    let serialized = serde_json::to_string(value).unwrap_or_default();
    if serialized.len() <= max_bytes {
        value.clone()
    } else {
        // Return truncated version with a note
        serde_json::json!({
            "_truncated": true,
            "_original_size": serialized.len(),
            "_preview": &serialized[..max_bytes.min(serialized.len())],
        })
    }
}

/// Truncate a string to max_len characters (UTF-8 safe).
fn truncate_string(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        let mut end = max_len.saturating_sub(3);
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}...", &s[..end])
    }
}
