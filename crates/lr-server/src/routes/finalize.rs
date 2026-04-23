//! Post-LLM response finalization helpers shared across the three
//! LLM-facing HTTP surfaces (`/v1/chat/completions`, `/v1/responses`,
//! `/v1/completions`).
//!
//! Every endpoint needs the same bookkeeping after a provider call
//! completes: cost computation, metrics recording, tray graph token
//! push, access log write, `metrics-updated` tray event,
//! `update_llm_call_routing` / `complete_llm_call` /
//! `update_llm_call_response_body` monitor events, and
//! `generation_tracker.record(...)`. This module centralizes those
//! operations so each adapter owns only wire-format conversion.
//!
//! Extracted from `chat.rs` (Commit 1 of the shared-pipeline refactor).

use std::time::Instant;

use chrono::{DateTime, Utc};

use crate::state::{AppState, AuthContext, GenerationDetails};
use crate::types::{ChatCompletionRequest, ChatMessage, CostDetails, MessageContent, TokenUsage};

// ---- utility helpers (moved from chat.rs, now pub(crate)) ---------------

/// Estimate token count from messages (rough estimate: ~4 chars/token).
///
/// Used by rate-limit estimation and incremental-prompt-tokens
/// accounting across the three LLM endpoints.
pub(crate) fn estimate_token_count(messages: &[ChatMessage]) -> u64 {
    messages
        .iter()
        .map(|msg| match &msg.content {
            Some(MessageContent::Text(text)) => (text.len() / 4).max(1) as u64,
            Some(MessageContent::Parts(parts)) => parts.len() as u64 * 100,
            None => 0,
        })
        .sum()
}

/// Repair JSON response content when `response_format` requests it.
///
/// No-op when `response_format` is not JSON, when repair is disabled
/// globally and for this client, or when the content is already valid.
/// Records a `feature_json_repair` metrics event when repair was
/// actually applied.
pub(crate) fn maybe_repair_json_content(
    content: String,
    request: &ChatCompletionRequest,
    state: &AppState,
    auth: &AuthContext,
) -> String {
    let schema = match &request.response_format {
        Some(crate::types::ResponseFormat::JsonObject { .. }) => None,
        Some(crate::types::ResponseFormat::JsonSchema { schema, .. }) => Some(schema),
        _ => return content,
    };

    let config = state.config_manager.get();
    let repair_config = &config.json_repair;

    let client = state.client_manager.get_client(&auth.api_key_id);
    let enabled = client
        .as_ref()
        .and_then(|c| c.json_repair.enabled)
        .unwrap_or(repair_config.enabled);
    if !enabled {
        return content;
    }

    let syntax_repair = client
        .as_ref()
        .and_then(|c| c.json_repair.syntax_repair)
        .unwrap_or(repair_config.syntax_repair);
    let schema_coercion = client
        .as_ref()
        .and_then(|c| c.json_repair.schema_coercion)
        .unwrap_or(repair_config.schema_coercion);

    let options = lr_json_repair::RepairOptions {
        syntax_repair,
        schema_coercion,
        strip_extra_fields: repair_config.strip_extra_fields,
        add_defaults: repair_config.add_defaults,
        normalize_enums: repair_config.normalize_enums,
    };

    let result = lr_json_repair::repair_content(&content, schema, &options);
    if result.was_modified {
        tracing::info!(
            repairs = result.repairs.len(),
            "JSON repair applied {} fix(es) to response",
            result.repairs.len()
        );
        state
            .metrics_collector
            .record_feature_event("feature_json_repair", 0, 0.0);
    }
    result.repaired
}

// ---- finalize_non_streaming -----------------------------------------------

/// Inputs that the caller computes once per turn and passes through
/// the finalize stages. Kept as a struct so the two finalize halves
/// (metrics + wire-format-body update + generation record) agree on
/// the same numbers without the caller juggling many arguments.
pub(crate) struct FinalizeInputs<'a> {
    pub state: &'a AppState,
    pub auth: &'a AuthContext,
    pub llm_event_id: &'a str,
    pub generation_id: &'a str,
    pub started_at: Instant,
    pub created_at: DateTime<Utc>,
    /// Prompt tokens to charge for this turn (incremental for
    /// chat-style history, full prompt tokens for single-shot
    /// completions).
    pub incremental_prompt_tokens: u32,
    /// Compression savings for the optional `feature_compression`
    /// metric event. `0` when compression didn't run or saved
    /// nothing.
    pub compression_tokens_saved: u64,
    /// Routing decision payload for `update_llm_call_routing`. `None`
    /// skips the update.
    pub routing_metadata: Option<&'a serde_json::Value>,
    /// Value of `user` from the original request — attached to
    /// GenerationDetails for per-user cost attribution.
    pub user: Option<String>,
    /// True for streaming endpoints — recorded in
    /// `complete_llm_call` and `GenerationDetails`.
    pub streamed: bool,
}

/// Per-turn numbers computed by `finalize_metrics_and_monitor`. The
/// caller threads these into the subsequent wire-format conversion
/// and `record_generation` step so both halves agree on the cost /
/// latency figures.
pub(crate) struct FinalizeMetrics {
    pub cost: f64,
    /// Not read by callers today but surfaced here so future streaming
    /// finalize paths can reuse the same figure without recomputing.
    #[allow(dead_code)]
    pub latency_ms: u64,
    pub pricing: lr_providers::PricingInfo,
    pub completed_at: Instant,
}

/// Record cost / metrics / tray graph / access log / monitor
/// completion events for a finished (non-streaming) LLM turn. The
/// caller feeds this the already-resolved `CompletionResponse` from
/// the router or MCP-via-LLM orchestrator.
///
/// The wire-format response is emitted by the caller *after* this
/// returns — `update_response_body_and_record_generation` then wraps
/// up the remaining monitor-event and generation-tracker writes.
pub(crate) async fn finalize_metrics_and_monitor(
    inputs: &FinalizeInputs<'_>,
    response: &lr_providers::CompletionResponse,
) -> FinalizeMetrics {
    let FinalizeInputs {
        state,
        auth,
        llm_event_id,
        generation_id,
        started_at,
        created_at,
        incremental_prompt_tokens,
        compression_tokens_saved,
        routing_metadata,
        streamed,
        ..
    } = *inputs;

    let completed_at = Instant::now();

    let pricing = match state.provider_registry.get_provider(&response.provider) {
        Some(p) => p.get_pricing(&response.model).await.ok(),
        None => None,
    }
    .unwrap_or_else(lr_providers::PricingInfo::free);

    if compression_tokens_saved > 0 && pricing.input_cost_per_1k > 0.0 {
        let cost_saved = (compression_tokens_saved as f64 / 1000.0) * pricing.input_cost_per_1k;
        state
            .metrics_collector
            .record_feature_event("feature_compression", 0, cost_saved);
    }

    let input_cost = (incremental_prompt_tokens as f64 / 1000.0) * pricing.input_cost_per_1k;
    let output_cost =
        (response.usage.completion_tokens as f64 / 1000.0) * pricing.output_cost_per_1k;
    let cost = input_cost + output_cost;

    let strategy_id = state
        .client_manager
        .get_client(&auth.api_key_id)
        .map(|c| c.strategy_id.clone())
        .unwrap_or_else(|| "default".to_string());

    let latency_ms = completed_at.duration_since(started_at).as_millis() as u64;
    state
        .metrics_collector
        .record_success(&lr_monitoring::metrics::RequestMetrics {
            api_key_name: &auth.api_key_id,
            provider: &response.provider,
            model: &response.model,
            strategy_id: &strategy_id,
            input_tokens: incremental_prompt_tokens as u64,
            output_tokens: response.usage.completion_tokens as u64,
            cost_usd: cost,
            latency_ms,
        });

    if let Some(ref tray_graph) = *state.tray_graph_manager.read() {
        tray_graph
            .record_tokens((incremental_prompt_tokens + response.usage.completion_tokens) as u64);
    }

    if let Err(e) = state.access_logger.log_success(
        &auth.api_key_id,
        &response.provider,
        &response.model,
        incremental_prompt_tokens as u64,
        response.usage.completion_tokens as u64,
        cost,
        latency_ms,
        generation_id,
    ) {
        tracing::warn!("Failed to write access log: {}", e);
    }

    state.emit_event(
        "metrics-updated",
        &serde_json::json!({
            "timestamp": created_at.to_rfc3339(),
        })
        .to_string(),
    );

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

    if let Some(meta) = routing_metadata {
        super::monitor_helpers::update_llm_call_routing(state, llm_event_id, meta);
    }
    super::monitor_helpers::complete_llm_call(
        state,
        llm_event_id,
        &response.provider,
        &response.model,
        200,
        incremental_prompt_tokens as u64,
        response.usage.completion_tokens as u64,
        reasoning_tokens,
        Some(cost),
        latency_ms,
        finish_reason,
        &content_preview,
        streamed,
    );

    FinalizeMetrics {
        cost,
        latency_ms,
        pricing,
        completed_at,
    }
}

/// Minimum streaming-turn state the caller assembles from SSE chunks
/// so the shared finalize path can run against it at end-of-stream.
/// The fields mirror the pieces `complete_llm_call` + access log +
/// metrics consume; each adapter feeds what its chunks expose.
#[allow(dead_code)]
pub(crate) struct StreamingFinalizeSummary {
    pub provider: String,
    pub model: String,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub reasoning_tokens: Option<u64>,
    pub finish_reason: Option<String>,
    pub content_preview: String,
}

/// Finalize a streaming turn once the upstream chunk stream is
/// exhausted. Synthesizes a shim `CompletionResponse` from the
/// caller's accumulated `StreamingFinalizeSummary` and runs the same
/// cost / metrics / monitor-event / access-log writes as the non-
/// streaming path.
///
/// The caller passes the wire-format response body separately via
/// `wire_body` (e.g. the final `response.completed` envelope for
/// `/v1/responses`, or a reconstructed chat-completion JSON for chat
/// / completions). Generation-tracker recording also happens here
/// so every endpoint's streams show up in `/v1/generation/{id}`.
#[allow(dead_code)]
pub(crate) async fn finalize_streaming_at_end(
    inputs: &FinalizeInputs<'_>,
    summary: StreamingFinalizeSummary,
    wire_body: &serde_json::Value,
) -> FinalizeMetrics {
    let synthetic = lr_providers::CompletionResponse {
        id: inputs.generation_id.to_string(),
        object: "chat.completion".to_string(),
        created: inputs.created_at.timestamp(),
        provider: summary.provider.clone(),
        model: summary.model.clone(),
        choices: vec![lr_providers::CompletionChoice {
            index: 0,
            message: lr_providers::ChatMessage {
                role: "assistant".to_string(),
                content: lr_providers::ChatMessageContent::Text(summary.content_preview.clone()),
                tool_calls: None,
                tool_call_id: None,
                name: None,
                reasoning_content: None,
            },
            finish_reason: summary.finish_reason.clone(),
            logprobs: None,
        }],
        usage: lr_providers::TokenUsage {
            prompt_tokens: summary.prompt_tokens,
            completion_tokens: summary.completion_tokens,
            total_tokens: summary.prompt_tokens + summary.completion_tokens,
            prompt_tokens_details: None,
            completion_tokens_details: summary.reasoning_tokens.map(|rt| {
                lr_providers::CompletionTokensDetails {
                    reasoning_tokens: Some(rt as u32),
                    thinking_tokens: None,
                    audio_tokens: None,
                }
            }),
        },
        system_fingerprint: None,
        service_tier: None,
        extensions: None,
        routellm_win_rate: None,
        request_usage_entries: None,
    };

    let metrics = finalize_metrics_and_monitor(inputs, &synthetic).await;

    let tokens = crate::types::TokenUsage {
        prompt_tokens: inputs.incremental_prompt_tokens,
        completion_tokens: summary.completion_tokens,
        total_tokens: inputs.incremental_prompt_tokens + summary.completion_tokens,
        prompt_tokens_details: None,
        completion_tokens_details: None,
    };
    update_response_body_and_record_generation(
        inputs,
        &synthetic,
        &metrics,
        wire_body,
        summary.finish_reason,
        tokens,
    );
    metrics
}

/// After the wire-format API response has been built, attach the
/// serialized body to the `LlmCall` monitor event and record a
/// `GenerationDetails` row for the generation endpoint.
///
/// Each adapter owns its own wire format, so the caller serializes
/// their response to `serde_json::Value` and hands it here.
pub(crate) fn update_response_body_and_record_generation(
    inputs: &FinalizeInputs<'_>,
    response: &lr_providers::CompletionResponse,
    metrics: &FinalizeMetrics,
    wire_body: &serde_json::Value,
    finish_reason: Option<String>,
    tokens: TokenUsage,
) {
    let FinalizeInputs {
        state,
        auth,
        llm_event_id,
        generation_id,
        started_at,
        created_at,
        incremental_prompt_tokens,
        streamed,
        ..
    } = *inputs;

    super::monitor_helpers::update_llm_call_response_body(state, llm_event_id, wire_body);

    let prompt_cost =
        (incremental_prompt_tokens as f64 / 1000.0) * metrics.pricing.input_cost_per_1k;
    let completion_cost =
        (response.usage.completion_tokens as f64 / 1000.0) * metrics.pricing.output_cost_per_1k;

    let details = GenerationDetails {
        id: generation_id.to_string(),
        model: response.model.clone(),
        provider: response.provider.clone(),
        created_at,
        finish_reason: finish_reason.unwrap_or_else(|| "unknown".to_string()),
        tokens,
        cost: Some(CostDetails {
            prompt_cost,
            completion_cost,
            reasoning_cost: None,
            total_cost: metrics.cost,
            currency: "USD".to_string(),
        }),
        started_at,
        completed_at: metrics.completed_at,
        provider_health: None,
        api_key_id: auth.api_key_id.clone(),
        user: inputs.user.clone(),
        stream: streamed,
    };

    state.generation_tracker.record(details.id.clone(), details);
}
