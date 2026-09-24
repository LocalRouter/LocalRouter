//! POST /v1/systemone endpoint
//!
//! System One typed decisions (TypeSafe's Jev wire protocol): a `state` plus a
//! map of typed questions (choice / score / noul) in, typed answers with
//! calibrated probabilities out. Served natively by System One providers
//! (TypeSafe, Laya, Kev, compatible servers) or translated onto chat
//! completions for any chat model.
//!
//! The request runs through the same cross-cutting checks as chat where they
//! make sense: client/mode gate, strategy and per-model permissions (with the
//! approval popup), rate limits, secret scanning, guardrails, prompt
//! compression of `state`, free-tier handling, monitoring, metrics, access
//! log and the generation tracker.

use axum::{
    body::Body,
    extract::State,
    http::{header, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    Extension, Json,
};
use chrono::Utc;
use std::time::Instant;
use uuid::Uuid;

use super::helpers::{
    check_llm_access_with_state, check_strategy_permission, get_client_with_strategy,
    get_enabled_client, validate_strategy_model_access,
};
use crate::middleware::client_auth::ClientAuthContext;
use crate::middleware::error::{ApiErrorResponse, ApiResult};
use crate::state::{AppState, AuthContext, GenerationDetails};
use crate::types::{CostDetails, TokenUsage};
use lr_providers::{SystemOneRequest, SystemOneResponse};
use lr_router::UsageInfo;

/// Endpoint label used in monitor events and logs.
const ENDPOINT: &str = "/v1/systemone";

/// Response header naming how the answers were produced
/// (`native`, `letter_logprobs`, or `json`).
pub const BACKEND_HEADER: &str = "x-localrouter-systemone-backend";

/// POST /v1/systemone
///
/// Answer typed questions about a state. Compatible with TypeSafe's
/// `POST /v1/systemone`, so TypeSafe SDKs work by pointing their base URL at
/// LocalRouter.
#[utoipa::path(
    post,
    path = "/v1/systemone",
    tag = "systemone",
    request_body = lr_providers::SystemOneRequest,
    responses(
        (status = 200, description = "Typed answers", body = lr_providers::SystemOneResponse,
            headers(
                ("x-localrouter-systemone-backend" = String, description = "native, letter_logprobs, or json"),
                ("x-typesafe-request-id" = String, description = "Upstream request id, when the provider sends one"),
            )),
        (status = 400, description = "Invalid request", body = crate::types::ErrorResponse),
        (status = 401, description = "Unauthorized", body = crate::types::ErrorResponse),
        (status = 403, description = "Denied by permissions, the firewall, secret scanning or guardrails", body = crate::types::ErrorResponse),
        (status = 422, description = "Upstream validation error (body passed through unchanged)"),
        (status = 429, description = "Rate limited", body = crate::types::ErrorResponse),
        (status = 502, description = "Provider error", body = crate::types::ErrorResponse)
    ),
    security(("bearer_auth" = []))
)]
pub async fn systemone(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    client_auth: Option<Extension<ClientAuthContext>>,
    Json(mut request): Json<SystemOneRequest>,
) -> ApiResult<Response> {
    state.emit_event("llm-request", "systemone");

    let session_id = Uuid::new_v4().to_string();
    let is_duplicate = lr_types::is_duplicate_hop();
    let model_label = request.model.clone().unwrap_or_else(|| "auto".to_string());

    let request_json = serde_json::to_value(&request).unwrap_or_default();
    let mut llm_guard = super::monitor_helpers::emit_llm_call(
        &state,
        client_auth.as_ref(),
        Some(&session_id),
        ENDPOINT,
        &model_label,
        false,
        &request_json,
    );

    state.record_client_activity(&auth.api_key_id);

    // Client must exist, be enabled, and be in gateway mode.
    {
        let client =
            get_enabled_client(&state, &auth.api_key_id).map_err(|e| llm_guard.capture_err(e))?;
        check_llm_access_with_state(&state, &client).map_err(|e| llm_guard.capture_err(e))?;
    }

    if let Err(message) = lr_providers::systemone::validate_systemone_request(&request) {
        let err = ApiErrorResponse::bad_request(message);
        super::monitor_helpers::emit_validation_error(
            &state,
            client_auth.as_ref(),
            Some(&session_id),
            ENDPOINT,
            None,
            &err.error.error.message,
            400,
        );
        return Err(llm_guard.capture_err(err));
    }

    // Model access: auto alias, auto-router approval, strategy permission,
    // per-model firewall.
    apply_access_checks(
        &state,
        &auth,
        client_auth.as_ref(),
        &session_id,
        &mut request,
        &mut llm_guard,
        is_duplicate,
    )
    .await?;

    if let Err(e) = check_rate_limits(&state, &auth, &request_json).await {
        super::monitor_helpers::emit_rate_limit_event(
            &state,
            client_auth.as_ref(),
            Some(&session_id),
            "rate_limit_exceeded",
            ENDPOINT,
            &e.error.error.message,
            429,
            None,
        );
        return Err(llm_guard.capture_err(e));
    }

    // Content checks run on the (possibly popup-edited) request. A duplicate
    // hop was already checked by the first LocalRouter.
    if !is_duplicate {
        let client_id = client_auth
            .as_ref()
            .map(|c| c.0.client_id.clone())
            .unwrap_or_else(|| auth.api_key_id.clone());
        let body = serde_json::to_value(&request).unwrap_or_default();
        let model_for_scan = request.model.clone().unwrap_or_default();

        match super::pipeline::scan_request_for_secrets(&state, &client_id, &model_for_scan, &body)
            .await
        {
            super::pipeline::SecretScanOutcome::Allow => {}
            super::pipeline::SecretScanOutcome::Deny(message) => {
                return Err(llm_guard.capture_err(ApiErrorResponse::forbidden(message)));
            }
        }

        if let Some(result) =
            super::pipeline::guardrails_scan_request(&state, &client_id, &model_for_scan, &body)
                .await
        {
            super::pipeline::handle_guardrail_result(
                &state,
                &client_id,
                &model_for_scan,
                &body,
                result,
                "request",
            )
            .await
            .map_err(|e| llm_guard.capture_err(e))?;
        }

        // Prompt compression of `state` (opt-in, same switches as chat).
        match super::pipeline::run_systemone_state_compression(
            &state,
            client_auth.as_ref().map(|c| &c.0),
            &mut request,
        )
        .await
        {
            Ok(Some(stats)) => {
                let transformed = serde_json::to_value(&request).unwrap_or_default();
                super::monitor_helpers::update_llm_call_transformed(
                    &state,
                    llm_guard.event_id(),
                    &transformed,
                    vec![format!(
                        "prompt compression: state {} → {} words",
                        stats.original_tokens, stats.compressed_tokens
                    )],
                );
            }
            Ok(None) => {}
            Err(e) => tracing::warn!(
                "System One state compression failed, sending original: {}",
                e
            ),
        }
    }

    let request_id = format!("gen-{}", Uuid::new_v4());
    let created_at = Utc::now();
    let started_at = Instant::now();
    tracing::info!(
        "System One request: client={}, model={}, questions={}",
        &auth.api_key_id[..8.min(auth.api_key_id.len())],
        request.model.as_deref().unwrap_or("(default)"),
        request.questions.len(),
    );

    let outcome = match state
        .router
        .systemone(&auth.api_key_id, request.clone())
        .await
    {
        Err(lr_types::AppError::FreeTierFallbackAvailable {
            retry_after_secs,
            exhausted_models,
        }) => {
            super::chat::check_free_tier_fallback(
                &state,
                &auth.api_key_id,
                &exhausted_models,
                retry_after_secs,
            )
            .await
            .map_err(|e| llm_guard.capture_err(e))?;
            state
                .router
                .systemone_with_paid_fallback(&auth.api_key_id, request.clone())
                .await
        }
        other => other,
    };

    let response = match outcome {
        Ok(response) => response,
        Err(e) => {
            let err_text = e.to_string();
            let passthrough = upstream_client_error(&e);
            let api_err = ApiErrorResponse::from(e);
            let status_code = passthrough
                .as_ref()
                .map(|(status, _)| status.as_u16())
                .unwrap_or_else(|| api_err.status.as_u16());
            let latency = started_at.elapsed().as_millis() as u64;
            let strategy_id = strategy_id_for(&state, &auth.api_key_id);
            state.metrics_collector.record_failure(
                &auth.api_key_id,
                "unknown",
                &model_label,
                &strategy_id,
                latency,
            );
            if let Err(log_err) = state.access_logger.log_failure(
                &auth.api_key_id,
                "unknown",
                &model_label,
                latency,
                &request_id,
                status_code,
            ) {
                tracing::warn!("Failed to write access log: {}", log_err);
            }
            llm_guard.complete_error(&state, "unknown", &model_label, status_code, &err_text);
            tracing::warn!("System One request failed: {}", err_text);

            // Upstream 4xx with a JSON body (e.g. TypeSafe's 422 detail) goes
            // back to the client unchanged so SDK error parsing keeps working.
            if let Some((status, body)) = passthrough {
                return Ok(Response::builder()
                    .status(status)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body))
                    .unwrap_or_else(|_| status.into_response()));
            }
            return Err(api_err);
        }
    };

    let completed_at = Instant::now();
    let latency_ms = completed_at.duration_since(started_at).as_millis() as u64;
    finalize_success(
        &state,
        &auth,
        &request_id,
        created_at,
        started_at,
        completed_at,
        latency_ms,
        &response,
        llm_guard,
    );

    let mut http_response = Json(&response).into_response();
    let headers = http_response.headers_mut();
    if let Ok(v) = HeaderValue::from_str(response.backend.as_str()) {
        headers.insert(BACKEND_HEADER, v);
    }
    if let Some(rid) = response
        .request_id
        .as_deref()
        .and_then(|r| HeaderValue::from_str(r).ok())
    {
        headers.insert(lr_providers::systemone::TYPESAFE_REQUEST_ID_HEADER, rid);
    }
    if let Ok(v) = HeaderValue::from_str(&request_id) {
        headers.insert("x-localrouter-generation-id", v);
    }
    Ok(http_response)
}

/// Model-access checks shared with chat: auto-model alias, auto-router
/// approval popup, strategy permission and model allow-list, per-model
/// firewall. Popup edits are applied back onto `request`.
async fn apply_access_checks(
    state: &AppState,
    auth: &AuthContext,
    client_auth: Option<&Extension<ClientAuthContext>>,
    session_id: &str,
    request: &mut SystemOneRequest,
    llm_guard: &mut super::monitor_helpers::LlmCallGuard,
    is_duplicate: bool,
) -> ApiResult<()> {
    let Ok((client, strategy)) = get_client_with_strategy(state, &auth.api_key_id) else {
        // Internal clients (Try It Out direct mode) have no strategy.
        return Ok(());
    };

    // The strategy's custom auto-model name is an alias for localrouter/auto.
    if let (Some(model), Some(ac)) = (request.model.as_deref(), strategy.auto_config.as_ref()) {
        if model != "localrouter/auto" && model == ac.model_name {
            request.model = Some("localrouter/auto".to_string());
        }
    }

    let is_auto = request.model.as_deref() == Some("localrouter/auto");

    if is_auto && !is_duplicate {
        let Some(auto_config) = strategy.auto_config.as_ref() else {
            return Err(llm_guard.capture_err(ApiErrorResponse::not_found(
                "Auto routing is not configured for this client".to_string(),
            )));
        };
        if auto_config.prioritized_models.is_empty() {
            return Err(llm_guard.capture_err(ApiErrorResponse::bad_request(
                "Auto routing has no prioritized models configured".to_string(),
            )));
        }
        if auto_config.permission.is_enabled()
            && (auto_config.permission.requires_approval()
                || state.mcp_gateway.firewall_manager.should_intercept(
                    &client.id,
                    lr_mcp::gateway::firewall::InterceptCategory::Llm,
                ))
        {
            let request_json = serde_json::to_value(&*request).unwrap_or_default();
            if let Some(edits) = super::pipeline::request_auto_router_popup(
                state,
                auth,
                client_auth,
                session_id,
                &client,
                auto_config,
                request_json,
                false,
                ENDPOINT,
                llm_guard,
            )
            .await?
            {
                apply_popup_edits(request, &edits);
            }
        }
    }

    check_strategy_permission(&strategy).map_err(|e| llm_guard.capture_err(e))?;
    let model = request.model.clone();
    if let Some(model) = model.as_deref().filter(|m| *m != "localrouter/auto") {
        validate_strategy_model_access(state, &strategy, model)
            .map_err(|e| llm_guard.capture_err(e))?;

        if !is_duplicate {
            let strategy_permission = strategy
                .auto_config
                .as_ref()
                .map(|ac| ac.permission.clone());
            let snapshot = serde_json::to_value(&*request).unwrap_or_default();
            let edits = super::pipeline::check_model_firewall_for(
                state,
                client_auth.map(|e| &e.0),
                model,
                move || snapshot,
                false,
                strategy_permission,
            )
            .await
            .map_err(|e| llm_guard.capture_err(e))?;
            if let Some(edits) = edits {
                apply_popup_edits(request, &edits);
            }
        }
    }
    Ok(())
}

/// Apply a user's edits from an approval popup. The popup edits the whole
/// request body; anything that no longer parses as a System One request is
/// ignored, keeping the original.
fn apply_popup_edits(request: &mut SystemOneRequest, edits: &serde_json::Value) {
    let mut merged = serde_json::to_value(&*request).unwrap_or_default();
    if let (Some(target), Some(changes)) = (merged.as_object_mut(), edits.as_object()) {
        for (k, v) in changes {
            target.insert(k.clone(), v.clone());
        }
    }
    match serde_json::from_value::<SystemOneRequest>(merged) {
        Ok(edited) if lr_providers::systemone::validate_systemone_request(&edited).is_ok() => {
            *request = edited;
        }
        _ => tracing::warn!("Ignoring System One popup edits that do not form a valid request"),
    }
}

/// An upstream client error whose body should reach the caller verbatim.
fn upstream_client_error(e: &lr_types::AppError) -> Option<(StatusCode, String)> {
    match e {
        lr_types::AppError::ProviderStatus { status, message }
            if (400..500).contains(status)
                && serde_json::from_str::<serde_json::Value>(message).is_ok() =>
        {
            StatusCode::from_u16(*status)
                .ok()
                .map(|s| (s, message.clone()))
        }
        _ => None,
    }
}

fn strategy_id_for(state: &AppState, client_id: &str) -> String {
    state
        .client_manager
        .get_client(client_id)
        .map(|c| c.strategy_id.clone())
        .unwrap_or_else(|| "default".to_string())
}

/// Metrics, access log, monitor completion and generation tracking for a
/// successful request.
#[allow(clippy::too_many_arguments)]
fn finalize_success(
    state: &AppState,
    auth: &AuthContext,
    request_id: &str,
    created_at: chrono::DateTime<Utc>,
    started_at: Instant,
    completed_at: Instant,
    latency_ms: u64,
    response: &SystemOneResponse,
    llm_guard: super::monitor_helpers::LlmCallGuard,
) {
    let provider = if response.provider.is_empty() {
        "unknown".to_string()
    } else {
        response.provider.clone()
    };
    let input_tokens = response.usage.input_tokens.unwrap_or(0);
    let output_tokens = response.usage.output_tokens.unwrap_or(0);
    let cost = response.cost_usd.unwrap_or(0.0);
    let strategy_id = strategy_id_for(state, &auth.api_key_id);

    state
        .metrics_collector
        .record_success(&lr_monitoring::metrics::RequestMetrics {
            api_key_name: &auth.api_key_id,
            provider: &provider,
            model: &response.model,
            strategy_id: &strategy_id,
            input_tokens,
            output_tokens,
            cost_usd: cost,
            latency_ms,
        });

    if let Err(e) = state.access_logger.log_success(
        &auth.api_key_id,
        &provider,
        &response.model,
        input_tokens,
        output_tokens,
        cost,
        latency_ms,
        request_id,
    ) {
        tracing::warn!("Failed to write access log: {}", e);
    }

    state.emit_event(
        "metrics-updated",
        &serde_json::json!({ "timestamp": created_at.to_rfc3339() }).to_string(),
    );

    let event_id = llm_guard.event_id().to_string();
    let response_json = serde_json::to_value(response).unwrap_or_default();
    llm_guard.complete(
        state,
        &provider,
        &response.model,
        200,
        input_tokens,
        output_tokens,
        None,
        Some(cost),
        latency_ms,
        Some("stop"),
        &answers_preview(response),
        false,
    );
    super::monitor_helpers::update_llm_call_response_body(state, &event_id, &response_json);

    state.generation_tracker.record(
        request_id.to_string(),
        GenerationDetails {
            id: request_id.to_string(),
            model: response.model.clone(),
            provider,
            created_at,
            finish_reason: "stop".to_string(),
            tokens: TokenUsage {
                prompt_tokens: input_tokens as u32,
                completion_tokens: output_tokens as u32,
                total_tokens: (input_tokens + output_tokens) as u32,
                prompt_tokens_details: None,
                completion_tokens_details: None,
            },
            cost: Some(CostDetails {
                prompt_cost: cost,
                completion_cost: 0.0,
                reasoning_cost: None,
                total_cost: cost,
                currency: "USD".to_string(),
            }),
            started_at,
            completed_at,
            provider_health: None,
            api_key_id: auth.api_key_id.clone(),
            user: None,
            stream: false,
        },
    );
}

/// One-line summary of the answers for the monitor list, e.g.
/// `dept=billing (0.91), urgent=0.20`.
pub(crate) fn answers_preview(response: &SystemOneResponse) -> String {
    response
        .answers
        .iter()
        .map(|(id, answer)| match answer {
            lr_providers::SystemOneAnswer::Choice {
                choice, confidence, ..
            } => format!("{id}={choice} ({confidence:.2})"),
            lr_providers::SystemOneAnswer::Score { score, .. } => format!("{id}={score:.2}"),
            lr_providers::SystemOneAnswer::Noul { noul, .. } => format!("{id}={noul:.2}"),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Pre-flight rate-limit check with an estimated token count (body bytes / 4).
async fn check_rate_limits(
    state: &AppState,
    auth: &AuthContext,
    request_json: &serde_json::Value,
) -> ApiResult<()> {
    let estimated_tokens = (request_json.to_string().len() / 4).max(1) as u64;
    let usage_estimate = UsageInfo {
        input_tokens: estimated_tokens,
        output_tokens: 0,
        cost_usd: 0.0,
    };
    let result = state
        .rate_limiter
        .check_api_key(&auth.api_key_id, &usage_estimate)
        .await
        .map_err(|e| ApiErrorResponse::internal_error(format!("Rate limit check failed: {}", e)))?;
    if !result.allowed {
        let mut error = ApiErrorResponse::rate_limited(format!(
            "Rate limit exceeded: {}/{} used",
            result.current_usage, result.limit
        ));
        if let Some(retry_after) = result.retry_after_secs {
            error.error = error
                .error
                .with_code(format!("retry_after_{}", retry_after));
        }
        return Err(error);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn request() -> SystemOneRequest {
        serde_json::from_value(json!({
            "model": "laya/english",
            "state": "s",
            "questions": {"q": {"type": "noul", "instructions": "i"}}
        }))
        .unwrap()
    }

    #[test]
    fn popup_edits_replace_valid_fields() {
        let mut r = request();
        apply_popup_edits(
            &mut r,
            &json!({"model": "kev/kev-latest", "state": "edited"}),
        );
        assert_eq!(r.model.as_deref(), Some("kev/kev-latest"));
        assert_eq!(r.state, json!("edited"));
    }

    #[test]
    fn popup_edits_that_break_the_request_are_ignored() {
        let mut r = request();
        apply_popup_edits(&mut r, &json!({"questions": {}}));
        assert_eq!(r.questions.len(), 1);
        apply_popup_edits(&mut r, &json!({"questions": "nonsense"}));
        assert_eq!(r.questions.len(), 1);
    }

    #[test]
    fn upstream_json_client_errors_pass_through() {
        let e = lr_types::AppError::ProviderStatus {
            status: 422,
            message: r#"{"detail":[{"loc":["body","questions"],"msg":"bad"}]}"#.into(),
        };
        let (status, body) = upstream_client_error(&e).unwrap();
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(body.contains("detail"));

        let text = lr_types::AppError::ProviderStatus {
            status: 400,
            message: "API error (400): plain text".into(),
        };
        assert!(upstream_client_error(&text).is_none());
        assert!(upstream_client_error(&lr_types::AppError::Unauthorized).is_none());
    }

    #[test]
    fn preview_lists_answers() {
        let resp: SystemOneResponse = serde_json::from_value(json!({
            "model": "m",
            "answers": {
                "dept": {"type": "choice", "choice": "billing", "confidence": 0.9, "probabilities": {"billing": 0.95, "tech": 0.05}},
                "urgent": {"type": "noul", "noul": 0.2}
            },
            "usage": {"input_tokens": 1, "output_tokens": 0}
        }))
        .unwrap();
        assert_eq!(answers_preview(&resp), "dept=billing (0.90), urgent=0.20");
    }
}
