//! Helper functions for emitting monitor events from route handlers.

use crate::middleware::client_auth::ClientAuthContext;
use crate::state::AppState;
use axum::Extension;
use lr_monitor::{EventStatus, MonitorEventData, MonitorEventType};

/// Emit an LlmRequest monitor event. Returns the event ID for linking to the response.
pub fn emit_llm_request(
    state: &AppState,
    client_auth: Option<&Extension<ClientAuthContext>>,
    endpoint: &str,
    model: &str,
    stream: bool,
    request_body: &serde_json::Value,
) -> String {
    let (client_id, client_name) = resolve_client(state, client_auth);

    let message_count = request_body
        .get("messages")
        .and_then(|m| m.as_array())
        .map(|a| a.len())
        .unwrap_or(0);

    let tools = request_body.get("tools").and_then(|t| t.as_array());
    let has_tools = tools.is_some_and(|t| !t.is_empty());
    let tool_count = tools.map(|t| t.len()).unwrap_or(0);

    state.monitor_store.push(
        MonitorEventType::LlmRequest,
        client_id,
        client_name,
        None,
        MonitorEventData::LlmRequest {
            endpoint: endpoint.to_string(),
            model: model.to_string(),
            stream,
            message_count,
            has_tools,
            tool_count,
            request_body: truncate_json(request_body, 10_000),
        },
        EventStatus::Complete,
        None,
    )
}

/// Emit an LlmRequestTransformed event showing the final request after all
/// transformations (compression, RouteLLM, MCP tool injection, etc.).
pub fn emit_llm_request_transformed(
    state: &AppState,
    client_auth: Option<&Extension<ClientAuthContext>>,
    endpoint: &str,
    model: &str,
    stream: bool,
    request_body: &serde_json::Value,
    transformations: Vec<String>,
) {
    let (client_id, client_name) = resolve_client(state, client_auth);

    let message_count = request_body
        .get("messages")
        .and_then(|m| m.as_array())
        .map(|a| a.len())
        .unwrap_or(0);

    let tools = request_body.get("tools").and_then(|t| t.as_array());
    let has_tools = tools.is_some_and(|t| !t.is_empty());
    let tool_count = tools.map(|t| t.len()).unwrap_or(0);

    state.monitor_store.push(
        MonitorEventType::LlmRequestTransformed,
        client_id,
        client_name,
        None,
        MonitorEventData::LlmRequestTransformed {
            endpoint: endpoint.to_string(),
            model: model.to_string(),
            stream,
            message_count,
            has_tools,
            tool_count,
            request_body: truncate_json(request_body, 10_000),
            transformations_applied: transformations,
        },
        EventStatus::Complete,
        None,
    );
}

/// Emit an LlmResponse monitor event for a completed non-streaming response.
#[allow(clippy::too_many_arguments)]
pub fn emit_llm_response(
    state: &AppState,
    client_auth: Option<&Extension<ClientAuthContext>>,
    request_id: &str,
    provider: &str,
    model: &str,
    status_code: u16,
    input_tokens: u64,
    output_tokens: u64,
    cost_usd: Option<f64>,
    latency_ms: u64,
    finish_reason: Option<&str>,
    content_preview: &str,
    streamed: bool,
) {
    let (client_id, client_name) = resolve_client(state, client_auth);

    state.monitor_store.push(
        MonitorEventType::LlmResponse,
        client_id,
        client_name,
        Some(request_id.to_string()),
        MonitorEventData::LlmResponse {
            provider: provider.to_string(),
            model: model.to_string(),
            status_code,
            input_tokens,
            output_tokens,
            total_tokens: input_tokens + output_tokens,
            cost_usd,
            latency_ms,
            finish_reason: finish_reason.map(|s| s.to_string()),
            content_preview: truncate_string(content_preview, 2000),
            streamed,
        },
        EventStatus::Complete,
        Some(latency_ms),
    );
}

/// Emit a pending LlmResponse for streaming (will be updated when stream completes).
/// Returns the monitor event ID.
pub fn emit_llm_response_pending(
    state: &AppState,
    client_auth: Option<&Extension<ClientAuthContext>>,
    request_id: &str,
    model: &str,
) -> String {
    let (client_id, client_name) = resolve_client(state, client_auth);

    state.monitor_store.push(
        MonitorEventType::LlmResponse,
        client_id,
        client_name,
        Some(request_id.to_string()),
        MonitorEventData::LlmResponse {
            provider: String::new(),
            model: model.to_string(),
            status_code: 0,
            input_tokens: 0,
            output_tokens: 0,
            total_tokens: 0,
            cost_usd: None,
            latency_ms: 0,
            finish_reason: None,
            content_preview: String::new(),
            streamed: true,
        },
        EventStatus::Pending,
        None,
    )
}

/// Update a pending streaming LlmResponse with final data.
#[allow(clippy::too_many_arguments)]
pub fn complete_llm_response(
    state: &AppState,
    monitor_event_id: &str,
    provider: &str,
    model: &str,
    input_tokens: u64,
    output_tokens: u64,
    cost_usd: Option<f64>,
    latency_ms: u64,
    finish_reason: Option<&str>,
    content_preview: &str,
) {
    let finish_reason = finish_reason.map(|s| s.to_string());
    let provider = provider.to_string();
    let model = model.to_string();
    let content_preview = truncate_string(content_preview, 2000);

    state.monitor_store.update(monitor_event_id, |event| {
        event.status = EventStatus::Complete;
        event.duration_ms = Some(latency_ms);
        if let MonitorEventData::LlmResponse {
            provider: ref mut p,
            model: ref mut m,
            status_code: ref mut sc,
            input_tokens: ref mut it,
            output_tokens: ref mut ot,
            total_tokens: ref mut tt,
            cost_usd: ref mut cu,
            latency_ms: ref mut lm,
            finish_reason: ref mut fr,
            content_preview: ref mut cp,
            ..
        } = &mut event.data
        {
            *p = provider;
            *m = model;
            *sc = 200;
            *it = input_tokens;
            *ot = output_tokens;
            *tt = input_tokens + output_tokens;
            *cu = cost_usd;
            *lm = latency_ms;
            *fr = finish_reason;
            *cp = content_preview;
        }
    });
}

/// Emit an LlmError monitor event.
pub fn emit_llm_error(
    state: &AppState,
    client_auth: Option<&Extension<ClientAuthContext>>,
    request_id: Option<&str>,
    provider: &str,
    model: &str,
    status_code: u16,
    error: &str,
) {
    let (client_id, client_name) = resolve_client(state, client_auth);

    state.monitor_store.push(
        MonitorEventType::LlmError,
        client_id,
        client_name,
        request_id.map(|s| s.to_string()),
        MonitorEventData::LlmError {
            provider: provider.to_string(),
            model: model.to_string(),
            status_code,
            error: truncate_string(error, 1000),
        },
        EventStatus::Error,
        None,
    );
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
        None,
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
        None,
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
pub fn emit_rate_limit_event(
    state: &AppState,
    client_auth: Option<&Extension<ClientAuthContext>>,
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
        None,
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
        None,
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
pub fn emit_moderation_event(
    state: &AppState,
    reason: &str,
    message: &str,
    status_code: u16,
) {
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
pub fn emit_internal_error(
    state: &AppState,
    error_type: &str,
    message: &str,
    status_code: u16,
) {
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

// ---- Guardrail events ----

/// Emit a GuardrailRequest event before running safety checks.
pub fn emit_guardrail_request(
    state: &AppState,
    client_ctx: Option<&ClientAuthContext>,
    direction: &str,
    text_preview: &str,
    models_used: Vec<String>,
) {
    let (client_id, client_name) = resolve_client_ctx(state, client_ctx);
    state.monitor_store.push(
        MonitorEventType::GuardrailRequest,
        client_id,
        client_name,
        None,
        MonitorEventData::GuardrailRequest {
            direction: direction.to_string(),
            text_preview: truncate_string(text_preview, 500),
            models_used,
        },
        EventStatus::Pending,
        None,
    );
}

/// Emit a GuardrailResponse event after safety check completes.
pub fn emit_guardrail_response(
    state: &AppState,
    client_ctx: Option<&ClientAuthContext>,
    direction: &str,
    result: &str,
    flagged_categories: Vec<lr_monitor::FlaggedCategory>,
    action_taken: &str,
    latency_ms: u64,
) {
    let (client_id, client_name) = resolve_client_ctx(state, client_ctx);
    state.monitor_store.push(
        MonitorEventType::GuardrailResponse,
        client_id,
        client_name,
        None,
        MonitorEventData::GuardrailResponse {
            direction: direction.to_string(),
            result: result.to_string(),
            flagged_categories,
            action_taken: action_taken.to_string(),
            latency_ms,
        },
        EventStatus::Complete,
        Some(latency_ms),
    );
}

// ---- Secret scan events ----

/// Emit a SecretScanRequest event before scanning.
pub fn emit_secret_scan_request(
    state: &AppState,
    client_ctx: Option<&ClientAuthContext>,
    text_preview: &str,
    rules_count: usize,
) {
    let (client_id, client_name) = resolve_client_ctx(state, client_ctx);
    state.monitor_store.push(
        MonitorEventType::SecretScanRequest,
        client_id,
        client_name,
        None,
        MonitorEventData::SecretScanRequest {
            text_preview: truncate_string(text_preview, 500),
            rules_count,
        },
        EventStatus::Pending,
        None,
    );
}

/// Emit a SecretScanResponse event after scanning.
pub fn emit_secret_scan_response(
    state: &AppState,
    client_ctx: Option<&ClientAuthContext>,
    findings_count: usize,
    findings: serde_json::Value,
    action_taken: &str,
    latency_ms: u64,
) {
    let (client_id, client_name) = resolve_client_ctx(state, client_ctx);
    state.monitor_store.push(
        MonitorEventType::SecretScanResponse,
        client_id,
        client_name,
        None,
        MonitorEventData::SecretScanResponse {
            findings_count,
            findings,
            action_taken: action_taken.to_string(),
            latency_ms,
        },
        EventStatus::Complete,
        Some(latency_ms),
    );
}

// ---- Routing events ----

/// Emit a RoutingDecision event when final model routing is determined.
pub fn emit_routing_decision(
    state: &AppState,
    client_ctx: Option<&ClientAuthContext>,
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
        None,
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
pub fn emit_prompt_compression(
    state: &AppState,
    client_ctx: Option<&ClientAuthContext>,
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
        None,
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
        None,
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
fn truncate_json(value: &serde_json::Value, max_bytes: usize) -> serde_json::Value {
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
