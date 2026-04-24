//! Pre-LLM pipeline stages shared by the three LLM HTTP surfaces
//! (`/v1/chat/completions`, `/v1/responses`, `/v1/completions`).
//!
//! Each stage is callable independently today; a higher-level
//! `run_turn_pipeline` entry point lives in future work (Commits
//! 2–4 of the shared-pipeline refactor).
//!
//! Stage order (assembled by `chat_completions` and followed by the
//! other adapters):
//!
//! 1. `validate_request`          — schema + bounds checks
//! 2. `apply_model_access_checks` — strategy / auto-routing / firewall
//! 3. `run_prompt_compression`    — optional message compression
//! 4. `run_guardrails_scan`       — request-side safety scan
//! 5. `run_secret_scan_check`     — outbound secret-leak scan
//! 6. `check_rate_limits`         — per-client quota gate
//! 7. `convert_to_provider_request` — ChatCompletionRequest → CompletionRequest
//!
//! Extracted from `chat.rs` — move only, no logic change.
//! Visibility flips: `apply_firewall_request_edits` and
//! `build_flagged_text_preview` were private in chat.rs; they are
//! `pub(crate)` here only because of the file split.

use axum::Extension;

use super::finalize::estimate_token_count;
use super::helpers::{
    check_llm_access_with_state, check_strategy_permission, get_client_with_strategy,
    validate_strategy_model_access,
};
use crate::middleware::client_auth::ClientAuthContext;
use crate::middleware::error::{ApiErrorResponse, ApiResult};
use crate::state::{AppState, AuthContext};
use crate::types::{ChatCompletionRequest, ChatMessage, MessageContent};
use lr_providers::{
    ChatMessage as ProviderChatMessage, ChatMessageContent as ProviderMessageContent,
    CompletionRequest as ProviderCompletionRequest, ContentPart as ProviderContentPart,
    ImageUrl as ProviderImageUrl, PreComputedRouting,
};
use lr_router::UsageInfo;

/// Validate the chat completion request
pub(crate) fn validate_request(request: &ChatCompletionRequest) -> ApiResult<()> {
    if request.model.is_empty() {
        return Err(ApiErrorResponse::bad_request("model is required").with_param("model"));
    }

    if request.messages.is_empty() {
        return Err(
            ApiErrorResponse::bad_request("messages cannot be empty").with_param("messages")
        );
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

    // Validate top_k (LocalRouter extension, not in OpenAI API)
    if let Some(top_k) = request.top_k {
        if top_k == 0 {
            return Err(
                ApiErrorResponse::bad_request("top_k must be greater than 0").with_param("top_k"),
            );
        }
    }

    // Validate repetition_penalty (LocalRouter extension, not in OpenAI API)
    // Range: 0.0-2.0 (LocalRouter-specific constraint)
    if let Some(rep_penalty) = request.repetition_penalty {
        if !(0.0..=2.0).contains(&rep_penalty) {
            return Err(ApiErrorResponse::bad_request(
                "repetition_penalty must be between 0 and 2",
            )
            .with_param("repetition_penalty"));
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
        if n > 1 {
            // Note: Currently n > 1 is accepted but only the first completion will be generated
            // This is a limitation that will be fixed in a future update
            tracing::warn!("n > 1 requested but only first completion will be generated (not yet fully supported)");
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

    // Validate top_logprobs (requires logprobs to be true)
    if let Some(top_logprobs) = request.top_logprobs {
        if request.logprobs != Some(true) {
            return Err(
                ApiErrorResponse::bad_request("top_logprobs requires logprobs to be true")
                    .with_param("top_logprobs"),
            );
        }
        if top_logprobs > 20 {
            return Err(
                ApiErrorResponse::bad_request("top_logprobs must be between 0 and 20")
                    .with_param("top_logprobs"),
            );
        }
    }

    // Validate max_tokens and max_completion_tokens are not both set
    if request.max_tokens.is_some() && request.max_completion_tokens.is_some() {
        return Err(ApiErrorResponse::bad_request(
            "Cannot specify both max_tokens and max_completion_tokens",
        ));
    }

    // Validate response_format if present
    if let Some(ref format) = request.response_format {
        match format {
            crate::types::ResponseFormat::JsonObject { r#type } => {
                if r#type != "json_object" {
                    return Err(ApiErrorResponse::bad_request(
                        "response_format type must be 'json_object'",
                    )
                    .with_param("response_format"));
                }
            }
            crate::types::ResponseFormat::JsonSchema { r#type, schema } => {
                if r#type != "json_schema" {
                    return Err(ApiErrorResponse::bad_request(
                        "response_format type must be 'json_schema'",
                    )
                    .with_param("response_format"));
                }
                // Basic validation that schema is an object
                if !schema.is_object() {
                    return Err(ApiErrorResponse::bad_request(
                        "response_format schema must be a JSON object",
                    )
                    .with_param("response_format"));
                }
            }
        }
    }

    Ok(())
}

/// Apply user-edited fields from the firewall approval popup to the request.
///
/// Supports model params (temperature, max_tokens, etc.) and messages.
/// Used by both the model firewall and auto-router approval flows.
pub(crate) fn apply_firewall_request_edits(
    request: &mut ChatCompletionRequest,
    edits: &serde_json::Value,
) {
    if let Some(model) = edits.get("model").and_then(|v| v.as_str()) {
        request.model = model.to_string();
    }
    if let Some(v) = edits.get("temperature") {
        request.temperature = if v.is_null() {
            None
        } else {
            v.as_f64().map(|f| f as f32)
        };
    }
    if let Some(v) = edits.get("max_tokens") {
        request.max_tokens = if v.is_null() {
            None
        } else {
            v.as_u64().and_then(|n| u32::try_from(n).ok())
        };
    }
    if let Some(v) = edits.get("max_completion_tokens") {
        request.max_completion_tokens = if v.is_null() {
            None
        } else {
            v.as_u64().and_then(|n| u32::try_from(n).ok())
        };
    }
    if let Some(v) = edits.get("top_p") {
        request.top_p = if v.is_null() {
            None
        } else {
            v.as_f64().map(|f| f as f32)
        };
    }
    if let Some(v) = edits.get("frequency_penalty") {
        request.frequency_penalty = if v.is_null() {
            None
        } else {
            v.as_f64().map(|f| f as f32)
        };
    }
    if let Some(v) = edits.get("presence_penalty") {
        request.presence_penalty = if v.is_null() {
            None
        } else {
            v.as_f64().map(|f| f as f32)
        };
    }
    if let Some(v) = edits.get("seed") {
        request.seed = if v.is_null() { None } else { v.as_i64() };
    }
    // Messages editing
    if let Some(messages) = edits.get("messages") {
        if let Ok(parsed) = serde_json::from_value::<Vec<ChatMessage>>(messages.clone()) {
            request.messages = parsed;
        }
    }
    // Stop sequences
    if let Some(v) = edits.get("stop") {
        if !v.is_null() {
            request.stop = serde_json::from_value(v.clone()).ok();
        }
    }
}

/// All pre-LLM model-access checks, shared between `/v1/chat/completions`
/// and `/v1/responses`:
///
/// 1. Normalize the custom auto-router model name to `localrouter/auto`.
/// 2. Prioritized-models sanity + auto-router firewall approval popup
///    (captures user edits and an optional specific-model override).
/// 3. Strategy-level permission switch + model-whitelist check.
/// 4. Client-mode enforcement (blocks MCP-only clients from LLM endpoints).
/// 5. Per-model firewall permission (edits applied to `request`).
///
/// Mutates `request` in-place when the user edits the request via the
/// firewall popup; returns `Err` if access is denied.
pub(crate) async fn apply_model_access_checks(
    state: &AppState,
    auth: &AuthContext,
    client_auth: Option<&Extension<ClientAuthContext>>,
    session_id: &str,
    request: &mut ChatCompletionRequest,
    llm_guard: &mut super::monitor_helpers::LlmCallGuard,
) -> ApiResult<()> {
    // Normalize auto model name: bare "auto" or custom model_name →
    // "localrouter/auto" so all downstream hardcoded checks work
    // consistently.
    if request.model != "localrouter/auto" {
        if let Ok((_, ref strategy)) = get_client_with_strategy(state, &auth.api_key_id) {
            if let Some(ref ac) = strategy.auto_config {
                if request.model == ac.model_name {
                    request.model = "localrouter/auto".to_string();
                }
            }
        }
    }

    // Auto-routing firewall check — only for explicit localrouter/auto requests.
    if request.model == "localrouter/auto" {
        if let Ok((client, strategy)) = get_client_with_strategy(state, &auth.api_key_id) {
            if let Some(auto_config) = &strategy.auto_config {
                if auto_config.prioritized_models.is_empty() {
                    return Err(llm_guard.capture_err(ApiErrorResponse::bad_request(
                        "Auto routing has no prioritized models configured".to_string(),
                    )));
                }

                // Show approval popup if permission is Ask, or if monitor
                // intercept overrides Allow → Ask.
                if auto_config.permission.is_enabled()
                    && (auto_config.permission.requires_approval()
                        || state.mcp_gateway.firewall_manager.should_intercept(
                            &client.id,
                            lr_mcp::gateway::firewall::InterceptCategory::Llm,
                        ))
                {
                    use lr_mcp::gateway::firewall::FirewallApprovalAction;

                    let is_mcp_via_llm = client.client_mode == lr_config::ClientMode::McpViaLlm;

                    let mut request_json = serde_json::to_value(&request)
                        .map(|mut v| {
                            if let Some(obj) = v.as_object_mut() {
                                obj.remove("stream");
                            }
                            v
                        })
                        .unwrap_or_default();

                    if is_mcp_via_llm {
                        let all_ids: Vec<String> = state
                            .config_manager
                            .get()
                            .mcp_servers
                            .iter()
                            .map(|s| s.id.clone())
                            .collect();
                        let allowed = if client.mcp_permissions.global.is_enabled() {
                            all_ids.clone()
                        } else {
                            all_ids
                                .iter()
                                .filter(|id| client.mcp_permissions.has_any_enabled_for_server(id))
                                .cloned()
                                .collect()
                        };
                        match state
                            .mcp_via_llm_manager
                            .list_tools_for_preview(state.mcp_gateway.clone(), &client, allowed)
                            .await
                        {
                            Ok(tools) => {
                                if let Some(obj) = request_json.as_object_mut() {
                                    obj.insert("tools".to_string(), tools);
                                }
                            }
                            Err(e) => {
                                tracing::warn!(
                                    "Auto-router firewall: failed to pre-fetch MCP tools: {}",
                                    e,
                                );
                            }
                        }
                    }

                    let models_preview = auto_config
                        .prioritized_models
                        .iter()
                        .take(5)
                        .map(|(p, m)| format!("{}/{}", p, m))
                        .collect::<Vec<_>>()
                        .join(", ");

                    let auto_full_args = serde_json::json!({
                        "candidate_models": auto_config.prioritized_models.iter()
                            .map(|(p, m)| format!("{}/{}", p, m))
                            .collect::<Vec<_>>(),
                        "request": request_json,
                    });

                    let response = state
                        .mcp_gateway
                        .firewall_manager
                        .request_auto_router_approval(
                            client.id.clone(),
                            client.name.clone(),
                            models_preview,
                            Some(auto_full_args),
                            is_mcp_via_llm,
                        )
                        .await
                        .map_err(|e| {
                            ApiErrorResponse::internal_error(format!(
                                "Auto-router approval failed: {}",
                                e
                            ))
                        })
                        .map_err(|e| llm_guard.capture_err(e))?;

                    let ar_action_str = format!("{:?}", response.action);
                    super::monitor_helpers::emit_firewall_decision(
                        state,
                        client_auth.map(|e| &e.0),
                        Some(session_id),
                        "auto_router",
                        &auto_config.model_name,
                        &ar_action_str,
                        None,
                    );

                    match response.action {
                        FirewallApprovalAction::AllowOnce
                        | FirewallApprovalAction::AllowSession
                        | FirewallApprovalAction::Allow1Minute
                        | FirewallApprovalAction::Allow1Hour
                        | FirewallApprovalAction::AllowPermanent => {
                            if let Some(ref edits) = response.edited_arguments {
                                let req_edits = edits.get("request").unwrap_or(edits);
                                apply_firewall_request_edits(request, req_edits);

                                let selected_model =
                                    req_edits.get("model").and_then(|v| v.as_str());
                                if let Some(model) = selected_model {
                                    if model != "localrouter/auto" {
                                        tracing::info!(
                                            "Auto-routing overridden by user: using model '{}'",
                                            model
                                        );
                                        request.model = model.to_string();
                                    }
                                }
                            }
                        }
                        _ => {
                            super::monitor_helpers::emit_access_denied_for_client(
                                state,
                                &auth.api_key_id,
                                Some(session_id),
                                "auto_routing_denied",
                                "/v1/chat/completions",
                                "Auto-routing denied by user",
                                403,
                            );
                            return Err(llm_guard.capture_err(ApiErrorResponse::forbidden(
                                "Auto-routing denied by user",
                            )));
                        }
                    }
                }
            } else {
                return Err(llm_guard.capture_err(ApiErrorResponse::not_found(
                    "Auto routing is not configured for this client".to_string(),
                )));
            }
        }
    }

    // Strategy-level model access checks.
    if let Ok((_, ref strategy)) = get_client_with_strategy(state, &auth.api_key_id) {
        check_strategy_permission(strategy).map_err(|e| llm_guard.capture_err(e))?;

        let is_auto_model = request.model == "localrouter/auto"
            || strategy
                .auto_config
                .as_ref()
                .is_some_and(|ac| request.model == ac.model_name);

        if !is_auto_model {
            validate_strategy_model_access(state, strategy, &request.model)
                .map_err(|e| llm_guard.capture_err(e))?;
        }
    }

    // Enforce client mode: block MCP-only clients from LLM endpoints.
    if let Ok((ref client, _)) = get_client_with_strategy(state, &auth.api_key_id) {
        check_llm_access_with_state(state, client).map_err(|e| llm_guard.capture_err(e))?;
    }

    // Per-model firewall permission (skipped for auto-routing and
    // for MCP-via-LLM clients — those get their own firewall pass
    // once MCP tools are injected into the augmented request).
    if request.model != "localrouter/auto" {
        let is_mcp_via_llm_client = client_auth
            .as_ref()
            .and_then(|ext| state.client_manager.get_client(&ext.0.client_id))
            .is_some_and(|c| c.client_mode == lr_config::ClientMode::McpViaLlm);

        let strategy_permission = get_client_with_strategy(state, &auth.api_key_id)
            .ok()
            .and_then(|(_, s)| s.auto_config.map(|ac| ac.permission));

        if !is_mcp_via_llm_client {
            let firewall_edits = check_model_firewall_permission(
                state,
                client_auth.map(|e| &e.0),
                request,
                None,
                strategy_permission,
            )
            .await
            .map_err(|e| llm_guard.capture_err(e))?;

            if let Some(edits) = firewall_edits {
                apply_firewall_request_edits(request, &edits);
            }
        }
    }

    Ok(())
}

/// Check model firewall permission for LLM access
///
/// This enforces the model_permissions firewall for clients. When a model
/// has "Ask" permission, the request is held pending user approval.
pub(crate) async fn check_model_firewall_permission(
    state: &AppState,
    client_context: Option<&ClientAuthContext>,
    request: &ChatCompletionRequest,
    mcp_via_llm_tools: Option<serde_json::Value>,
    strategy_permission: Option<lr_config::PermissionState>,
) -> ApiResult<Option<serde_json::Value>> {
    use lr_mcp::gateway::access_control;
    use lr_mcp::gateway::firewall::FirewallApprovalAction;

    // If no client context, skip firewall (using API key auth without client)
    let Some(client_ctx) = client_context else {
        return Ok(None);
    };

    // Get enabled client (skip firewall for clients not in manager, e.g. internal-test)
    let Some(client) = state.client_manager.get_client(&client_ctx.client_id) else {
        return Ok(None);
    };
    if !client.enabled {
        return Err(ApiErrorResponse::forbidden("Client is disabled"));
    }

    // Skip firewall for localrouter/auto (handled during routing)
    if request.model == "localrouter/auto" {
        return Ok(None);
    }

    // Extract provider and model from request
    let (provider, model_id) = if let Some((prov, model)) = request.model.split_once('/') {
        (prov.to_string(), model.to_string())
    } else {
        // No provider specified - need to find which provider has this model
        let all_models = state.provider_registry.list_all_models_instant();

        // Collect all matching models to handle duplicates across providers
        let matching_models: Vec<_> = all_models
            .iter()
            .filter(|m| m.id.eq_ignore_ascii_case(&request.model))
            .collect();

        // Get model_permissions from the client's strategy for provider disambiguation
        let strat_perms = state
            .config_manager
            .get()
            .strategies
            .iter()
            .find(|s| s.parent.as_deref() == Some(&client.id))
            .map(|s| s.model_permissions.clone())
            .unwrap_or_default();

        // Prefer a model from a provider where the strategy has permission
        let matching_model = matching_models
            .iter()
            .find(|m| strat_perms.resolve_model(&m.provider, &m.id).is_enabled())
            .or(matching_models.first())
            .ok_or_else(|| {
                ApiErrorResponse::not_found(format!("Model not found: {}", request.model))
                    .with_param("model")
            })?;

        (matching_model.provider.clone(), matching_model.id.clone())
    };

    // Get model_permissions from the client's strategy
    let strategy_model_permissions = state
        .config_manager
        .get()
        .strategies
        .iter()
        .find(|s| s.parent.as_deref() == Some(&client.id))
        .map(|s| s.model_permissions.clone())
        .unwrap_or_default();

    // Use unified check_needs_approval
    use lr_mcp::gateway::access_control::{FirewallCheckContext, FirewallCheckResult};

    let ctx = FirewallCheckContext::Model {
        permissions: &strategy_model_permissions,
        provider: &provider,
        model_id: &model_id,
        has_time_based_approval: state
            .model_approval_tracker
            .has_valid_approval(&client.id, &provider, &model_id),
    };

    // Override resolution with strategy-level auto_config.permission (Ask forces popup)
    // Then monitor intercept can further override Allow → Ask
    let result = {
        let mut r = access_control::check_needs_approval(&ctx);

        // Strategy permission "Ask" overrides Allow → Ask for all models
        if r == FirewallCheckResult::Allow {
            if let Some(ref sp) = strategy_permission {
                if sp.requires_approval() {
                    tracing::info!(
                        "Strategy permission override: Allow → Ask for model {} (client={})",
                        request.model,
                        client.id
                    );
                    r = FirewallCheckResult::Ask;
                }
            }
        }

        // Monitor intercept: override Allow → Ask if intercept rule matches
        if r == FirewallCheckResult::Allow
            && state.mcp_gateway.firewall_manager.should_intercept(
                &client.id,
                lr_mcp::gateway::firewall::InterceptCategory::Llm,
            )
        {
            tracing::info!(
                "Monitor intercept: overriding Allow → Ask for model {} (client={})",
                request.model,
                client.id
            );
            FirewallCheckResult::Ask
        } else {
            r
        }
    };

    match result {
        FirewallCheckResult::Allow => {
            tracing::debug!(
                "Model firewall: {} allowed for client {}",
                request.model,
                client.id
            );
            Ok(None)
        }
        FirewallCheckResult::Deny => {
            tracing::warn!(
                "Model firewall: {} denied for client {}",
                request.model,
                client.id
            );
            Err(ApiErrorResponse::forbidden(format!(
                "Access denied: Model '{}' is not allowed for this client",
                request.model
            ))
            .with_param("model"))
        }
        FirewallCheckResult::Ask => {
            tracing::info!(
                "Model firewall: {} requires approval for client {}",
                request.model,
                client.id
            );

            // Capture the full request for the edit mode popup
            let is_mcp_via_llm = mcp_via_llm_tools.is_some();
            let mut full_request =
                serde_json::to_value(request).unwrap_or_else(|_| serde_json::json!({}));
            if let Some(obj) = full_request.as_object_mut() {
                obj.remove("stream"); // not user-editable
                                      // Merge MCP via LLM tools into the request so the popup shows the augmented request
                if let Some(tools) = mcp_via_llm_tools {
                    obj.insert("tools".to_string(), tools);
                }
            }

            // Request approval from the firewall manager
            let response = state
                .mcp_gateway
                .firewall_manager
                .request_model_approval(
                    client.id.clone(),
                    client.name.clone(),
                    model_id.clone(),
                    provider.clone(),
                    Some(120),
                    Some(full_request),
                    is_mcp_via_llm,
                )
                .await
                .map_err(|e| {
                    ApiErrorResponse::internal_error(format!("Firewall approval failed: {}", e))
                })?;

            let edited_arguments = response.edited_arguments;

            match response.action {
                FirewallApprovalAction::AllowOnce
                | FirewallApprovalAction::AllowSession
                | FirewallApprovalAction::Allow1Minute
                | FirewallApprovalAction::Allow1Hour
                | FirewallApprovalAction::AllowPermanent
                | FirewallApprovalAction::AllowCategories => {
                    tracing::info!(
                        "Model firewall: {} approved ({:?}) for client {}",
                        request.model,
                        response.action,
                        client.id
                    );
                    Ok(edited_arguments)
                }
                FirewallApprovalAction::Deny
                | FirewallApprovalAction::DenySession
                | FirewallApprovalAction::DenyAlways
                | FirewallApprovalAction::BlockCategories
                | FirewallApprovalAction::Deny1Hour
                | FirewallApprovalAction::DisableClient => {
                    tracing::warn!(
                        "Model firewall: {} denied by user for client {}",
                        request.model,
                        client.id
                    );
                    Err(ApiErrorResponse::forbidden(format!(
                        "Access denied: Model '{}' was denied by user",
                        request.model
                    ))
                    .with_param("model"))
                }
            }
        }
    }
}

/// Run guardrails scan on request content using safety engine
///
/// Returns Some(SafetyCheckResult) if violations were found that need action,
/// or None if no scan is needed or content is safe.
/// Run prompt compression if enabled for this client.
/// Returns compressed messages or None if compression is not needed/enabled.
pub(crate) async fn run_prompt_compression(
    state: &AppState,
    client_context: Option<&ClientAuthContext>,
    request: &ChatCompletionRequest,
) -> Result<Option<lr_compression::CompressionResult>, String> {
    // Need compression engine
    let engine = state.compression_service.read().clone();
    let Some(engine) = engine else {
        return Ok(None);
    };

    let config = state.config_manager.get();
    if !config.prompt_compression.enabled {
        return Ok(None);
    }

    // Check per-client enabled override (None=inherit global, Some(false)=off)
    if let Some(client_ctx) = client_context {
        if let Some(client) = state.client_manager.get_client(&client_ctx.client_id) {
            if let Some(false) = client.prompt_compression.enabled {
                return Ok(None); // Client explicitly disabled compression
            }
        } else {
            return Ok(None); // Unknown client
        }
    }

    // Use global settings for all compression parameters
    let min_messages = config.prompt_compression.min_messages;
    let preserve_recent = config.prompt_compression.preserve_recent;
    let rate = config.prompt_compression.default_rate;
    let compress_system = config.prompt_compression.compress_system_prompt;
    let min_message_words = config.prompt_compression.min_message_words;
    let preserve_quoted = config.prompt_compression.preserve_quoted_text;
    let compression_notice = config.prompt_compression.compression_notice;

    // Check minimum message count
    if request.messages.len() < min_messages as usize {
        return Ok(None);
    }

    // Convert request messages to CompressedMessage format
    let messages: Vec<lr_compression::CompressedMessage> = request
        .messages
        .iter()
        .map(|m| lr_compression::CompressedMessage {
            role: m.role.clone(),
            content: match &m.content {
                Some(MessageContent::Text(t)) => t.clone(),
                Some(MessageContent::Parts(parts)) => parts
                    .iter()
                    .filter_map(|p| {
                        if let crate::types::ContentPart::Text { text } = p {
                            Some(text.as_str())
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\n"),
                None => String::new(),
            },
        })
        .collect();

    let comp_start = std::time::Instant::now();
    let result = engine
        .compress_messages(
            &messages,
            rate,
            preserve_recent,
            compress_system,
            min_message_words,
            preserve_quoted,
            compression_notice,
        )
        .await?;
    let comp_duration = comp_start.elapsed().as_millis() as u64;

    // Emit prompt compression monitor event
    let reduction_pct = if result.original_tokens > 0 {
        (1.0 - (result.compressed_tokens as f64 / result.original_tokens as f64)) * 100.0
    } else {
        0.0
    };
    super::monitor_helpers::emit_prompt_compression(
        state,
        client_context,
        None,
        result.original_tokens as u64,
        result.compressed_tokens as u64,
        reduction_pct,
        comp_duration,
        "llmlingua",
    );

    Ok(Some(result))
}

pub(crate) async fn run_guardrails_scan(
    state: &AppState,
    client_context: Option<&ClientAuthContext>,
    request: &ChatCompletionRequest,
) -> ApiResult<Option<lr_guardrails::SafetyCheckResult>> {
    // Need client context and safety engine
    let Some(client_ctx) = client_context else {
        return Ok(None);
    };
    let engine = state.safety_engine.read().clone();
    let Some(engine) = engine else {
        return Ok(None);
    };

    if !engine.has_models() {
        return Ok(None);
    }

    let config = state.config_manager.get();
    let client = match state.client_manager.get_client(&client_ctx.client_id) {
        Some(c) if c.enabled => c,
        Some(_) => return Ok(None),
        None => {
            // Client not found (e.g. internal-test) — no guardrails
            return Ok(None);
        }
    };

    if !config.guardrails.scan_requests {
        return Ok(None);
    }

    // Resolve effective category actions: per-client override > global default
    let effective_category_actions = client
        .guardrails
        .category_actions
        .as_deref()
        .unwrap_or(&config.guardrails.category_actions);

    if effective_category_actions.is_empty() {
        return Ok(None);
    }

    // Check for time-based guardrail bypass (unless monitor intercept overrides)
    if state
        .guardrail_approval_tracker
        .has_valid_bypass(&client.id)
        && !state.mcp_gateway.firewall_manager.should_intercept(
            &client.id,
            lr_mcp::gateway::firewall::InterceptCategory::Guardrails,
        )
    {
        tracing::debug!(
            "Guardrail check skipped: client {} has active bypass",
            client.id
        );
        return Ok(None);
    }

    let request_json = serde_json::to_value(request).unwrap_or_default();

    // Emit guardrail request event
    let model_names: Vec<String> = vec!["guardrails".to_string()];
    let text_preview = lr_guardrails::text_extractor::extract_request_text(&request_json)
        .first()
        .map(|t| t.text.clone())
        .unwrap_or_default();
    let guardrail_event_id = super::monitor_helpers::emit_guardrail_scan(
        state,
        client_context,
        None,
        "request",
        &text_preview,
        model_names,
    );

    let started = std::time::Instant::now();
    let result = engine.check_input(&request_json).await;
    let latency_ms = started.elapsed().as_millis() as u64;

    if result.is_safe {
        super::monitor_helpers::complete_guardrail_scan(
            state,
            &guardrail_event_id,
            "pass",
            vec![],
            "none",
            latency_ms,
        );
        return Ok(None);
    }

    // Apply category action overrides
    let overrides: Vec<(String, lr_guardrails::CategoryAction)> = effective_category_actions
        .iter()
        .filter_map(|entry| {
            let action: lr_guardrails::CategoryAction =
                serde_json::from_value(serde_json::Value::String(entry.action.clone())).ok()?;
            Some((entry.category.clone(), action))
        })
        .collect();
    let result = result.apply_client_category_overrides(&overrides);
    if result.is_safe {
        super::monitor_helpers::complete_guardrail_scan(
            state,
            &guardrail_event_id,
            "pass",
            vec![],
            "none",
            latency_ms,
        );
        return Ok(None);
    }

    // Emit flagged guardrail response
    let flagged_cats: Vec<lr_monitor::FlaggedCategory> = result
        .actions_required
        .iter()
        .map(|a| lr_monitor::FlaggedCategory {
            category: a.category.to_string(),
            confidence: a.confidence.unwrap_or(0.0) as f64,
            action: format!("{:?}", a.action),
        })
        .collect();
    super::monitor_helpers::complete_guardrail_scan(
        state,
        &guardrail_event_id,
        "flagged",
        flagged_cats,
        "ask",
        latency_ms,
    );

    tracing::info!(
        "Safety check: {} flagged categories for client {} (model: {})",
        result.actions_required.len(),
        client.id,
        request.model,
    );

    Ok(Some(result))
}

/// Handle guardrail approval popup for detected violations
pub(crate) async fn handle_guardrail_approval(
    state: &AppState,
    client_context: Option<&ClientAuthContext>,
    request: &ChatCompletionRequest,
    result: lr_guardrails::SafetyCheckResult,
    scan_direction: &str,
) -> ApiResult<()> {
    use lr_mcp::gateway::firewall::{FirewallApprovalAction, GuardrailApprovalDetails};

    // If all flagged categories are "block", silently deny without popup
    if result.all_blocked() {
        tracing::info!(
            "Guardrail: silently blocking request (all flagged categories set to block)"
        );
        return Err(ApiErrorResponse::forbidden(
            "Request blocked by safety guardrails".to_string(),
        ));
    }

    // If only notifications are needed (no Ask actions), don't block
    if !result.needs_approval() {
        return Ok(());
    }

    let Some(client_ctx) = client_context else {
        return Ok(());
    };

    // Use unified check for time-based bypass/denial
    use lr_mcp::gateway::access_control::{FirewallCheckContext, FirewallCheckResult};

    let client = state.client_manager.get_client(&client_ctx.client_id);
    let client_id = client
        .as_ref()
        .map(|c| c.id.as_str())
        .unwrap_or(&client_ctx.client_id);

    let ctx = FirewallCheckContext::Guardrail {
        has_time_based_bypass: state
            .guardrail_approval_tracker
            .has_valid_bypass(&client_ctx.client_id),
        has_time_based_denial: state
            .guardrail_denial_tracker
            .has_valid_denial(&client_ctx.client_id),
        category_actions_empty: client
            .as_ref()
            .map(|c| {
                c.guardrails
                    .category_actions
                    .as_ref()
                    .is_none_or(|a| a.is_empty())
                    && state
                        .config_manager
                        .get()
                        .guardrails
                        .category_actions
                        .is_empty()
            })
            .unwrap_or(true),
    };

    // Monitor intercept: override Allow → Ask for guardrails
    let guardrail_result = {
        let r = lr_mcp::gateway::access_control::check_needs_approval(&ctx);
        if r == FirewallCheckResult::Allow
            && state.mcp_gateway.firewall_manager.should_intercept(
                client_id,
                lr_mcp::gateway::firewall::InterceptCategory::Guardrails,
            )
        {
            tracing::info!(
                "Monitor intercept: overriding Allow → Ask for guardrails (client={})",
                client_id
            );
            FirewallCheckResult::Ask
        } else {
            r
        }
    };

    match guardrail_result {
        FirewallCheckResult::Allow => {
            tracing::debug!(
                "Guardrail: bypassed for client {} (time-based or empty categories)",
                client_id
            );
            return Ok(());
        }
        FirewallCheckResult::Deny => {
            tracing::info!(
                "Guardrail: auto-denying request for client {} (active denial)",
                client_id
            );
            return Err(ApiErrorResponse::forbidden(
                "Request blocked by safety guardrails (auto-denied)",
            ));
        }
        FirewallCheckResult::Ask => {
            // Fall through to popup
        }
    }

    let client_name = client
        .as_ref()
        .map(|c| c.name.clone())
        .unwrap_or_else(|| client_ctx.client_id.clone());

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
        scan_direction: scan_direction.to_string(),
        flagged_text,
    };

    let preview = result
        .actions_required
        .iter()
        .map(|a| {
            format!(
                "[{:?}] {} (confidence: {:.2})",
                a.action,
                a.category,
                a.confidence.unwrap_or(0.0)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    let response = state
        .mcp_gateway
        .firewall_manager
        .request_guardrail_approval(
            client_id.to_string(),
            client_name,
            request.model.clone(),
            "guardrails".to_string(),
            details,
            preview,
        )
        .await
        .map_err(|e| {
            ApiErrorResponse::internal_error(format!("Guardrail approval failed: {}", e))
        })?;

    let action_str = format!("{:?}", response.action);
    super::monitor_helpers::emit_firewall_decision(
        state,
        client_context,
        None,
        "guardrail",
        &request.model,
        &action_str,
        None,
    );

    match response.action {
        FirewallApprovalAction::AllowOnce
        | FirewallApprovalAction::AllowSession
        | FirewallApprovalAction::Allow1Minute
        | FirewallApprovalAction::Allow1Hour
        | FirewallApprovalAction::AllowPermanent
        | FirewallApprovalAction::AllowCategories => {
            tracing::info!("Guardrail: request approved for client {}", client_id);
            Ok(())
        }
        FirewallApprovalAction::Deny
        | FirewallApprovalAction::DenySession
        | FirewallApprovalAction::DenyAlways
        | FirewallApprovalAction::BlockCategories
        | FirewallApprovalAction::Deny1Hour
        | FirewallApprovalAction::DisableClient => {
            tracing::warn!("Guardrail: request denied for client {}", client_id);
            Err(ApiErrorResponse::forbidden(
                "Request blocked by safety check",
            ))
        }
    }
}

/// Build a truncated text preview from extracted texts for the guardrail approval popup.
/// Shows the last user message (most relevant) truncated to a reasonable size.
pub(crate) fn build_flagged_text_preview(
    texts: &[lr_guardrails::text_extractor::ExtractedText],
) -> String {
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

/// Check outbound request for leaked secrets and handle based on configured action.
///
/// Runs synchronously before guardrails since regex+entropy is sub-millisecond.
/// If action=Ask, blocks the request and shows a popup for user decision.
/// If action=Notify, allows the request but emits a notification event.
pub(crate) async fn run_secret_scan_check(
    state: &AppState,
    client_ctx: Option<&ClientAuthContext>,
    request: &ChatCompletionRequest,
) -> ApiResult<()> {
    let Some(client_ctx) = client_ctx else {
        return Ok(());
    };
    let config = state.config_manager.get();
    let client = state.client_manager.get_client(&client_ctx.client_id);

    // Resolve effective action: per-client override > global
    let effective_action = client
        .as_ref()
        .and_then(|c| c.secret_scanning.action.as_ref())
        .unwrap_or(&config.secret_scanning.action);

    if *effective_action == lr_config::SecretScanAction::Off {
        return Ok(());
    }

    // Check time-based bypass (unless monitor intercept overrides)
    if state
        .secret_scan_approval_tracker
        .has_valid_bypass(&client_ctx.client_id)
        && !state.mcp_gateway.firewall_manager.should_intercept(
            &client_ctx.client_id,
            lr_mcp::gateway::firewall::InterceptCategory::SecretScan,
        )
    {
        return Ok(());
    }

    // Check if scanner is initialized and has rules
    // Clone Arc out of the RwLock to avoid holding the guard across await points
    let scanner = {
        let guard = state.secret_scanner.read();
        guard.as_ref().cloned()
    };
    let Some(scanner) = scanner else {
        return Ok(());
    };
    if !scanner.has_rules() {
        return Ok(());
    }

    // Extract text from request using guardrails text extractor
    let request_json = serde_json::to_value(request).unwrap_or_default();
    let guardrail_texts = lr_guardrails::text_extractor::extract_request_text(&request_json);

    // Convert to secret scanner's ExtractedText type
    let texts: Vec<lr_secret_scanner::ExtractedText> = guardrail_texts
        .iter()
        .enumerate()
        .map(|(i, t)| lr_secret_scanner::ExtractedText {
            label: t.label.clone(),
            text: t.text.clone(),
            message_index: i,
        })
        .collect();

    // Emit secret scan request event
    let scan_text_preview = texts.first().map(|t| t.text.as_str()).unwrap_or("");
    let rules_count = if scanner.has_rules() {
        scanner.rule_metadata().len()
    } else {
        0
    };
    let secret_scan_event_id = super::monitor_helpers::emit_secret_scan(
        state,
        Some(client_ctx),
        None,
        scan_text_preview,
        rules_count,
    );

    let scan_start = std::time::Instant::now();
    let result = scanner.scan(&texts);
    let scan_latency = scan_start.elapsed().as_millis() as u64;

    if result.findings.is_empty() {
        super::monitor_helpers::complete_secret_scan(
            state,
            &secret_scan_event_id,
            0,
            serde_json::json!([]),
            "pass",
            scan_latency,
        );
        return Ok(());
    }

    tracing::info!(
        "Secret scan found {} potential secrets in request from client {}",
        result.findings.len(),
        client_ctx.client_id
    );

    let findings_json = serde_json::to_value(&result.findings).unwrap_or(serde_json::json!([]));
    let action_name = format!("{:?}", effective_action).to_lowercase();
    super::monitor_helpers::complete_secret_scan(
        state,
        &secret_scan_event_id,
        result.findings.len(),
        findings_json,
        &action_name,
        scan_latency,
    );

    match effective_action {
        lr_config::SecretScanAction::Notify => {
            // Emit event to UI, allow request to proceed
            let payload =
                serde_json::to_string(&result.findings).unwrap_or_else(|_| "[]".to_string());
            state.emit_event("secret-scan-notify", &payload);
            Ok(())
        }
        lr_config::SecretScanAction::Ask => {
            handle_secret_scan_approval(state, client_ctx, request, result).await
        }
        lr_config::SecretScanAction::Off => Ok(()),
    }
}

/// Handle a secret scan detection that requires user approval (Ask action).
///
/// Blocks the request and shows a popup via the FirewallManager.
pub(crate) async fn handle_secret_scan_approval(
    state: &AppState,
    client_ctx: &ClientAuthContext,
    request: &ChatCompletionRequest,
    result: lr_secret_scanner::ScanResult,
) -> ApiResult<()> {
    use lr_mcp::gateway::firewall::{
        FirewallApprovalAction, SecretFindingSummary, SecretScanApprovalDetails,
    };

    let client = state.client_manager.get_client(&client_ctx.client_id);
    let client_name = client
        .as_ref()
        .map(|c| c.name.clone())
        .unwrap_or_else(|| client_ctx.client_id.clone());

    let details = SecretScanApprovalDetails {
        findings: result
            .findings
            .iter()
            .map(|f| SecretFindingSummary {
                rule_id: f.rule_id.clone(),
                rule_description: f.rule_description.clone(),
                category: f.category.clone(),
                matched_text: f.matched_text.clone(),
                entropy: f.entropy,
            })
            .collect(),
        scan_duration_ms: result.scan_duration_ms,
    };

    let preview = result
        .findings
        .iter()
        .map(|f| {
            format!(
                "[{}] {} (entropy: {:.2})",
                f.category, f.rule_description, f.entropy
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    let response = state
        .mcp_gateway
        .firewall_manager
        .request_secret_scan_approval(
            client_ctx.client_id.clone(),
            client_name,
            request.model.clone(),
            details,
            preview,
        )
        .await
        .map_err(|e| {
            ApiErrorResponse::internal_error(format!("Secret scan approval failed: {}", e))
        })?;

    let action_str = format!("{:?}", response.action);
    super::monitor_helpers::emit_firewall_decision(
        state,
        Some(client_ctx),
        None,
        "secret_scan",
        &request.model,
        &action_str,
        None,
    );

    match response.action {
        FirewallApprovalAction::AllowOnce
        | FirewallApprovalAction::AllowSession
        | FirewallApprovalAction::Allow1Minute
        | FirewallApprovalAction::Allow1Hour
        | FirewallApprovalAction::AllowPermanent => {
            tracing::info!(
                "Secret scan: request approved for client {}",
                client_ctx.client_id
            );
            Ok(())
        }
        _ => {
            tracing::warn!(
                "Secret scan: request denied for client {}",
                client_ctx.client_id
            );
            Err(ApiErrorResponse::forbidden(
                "Request blocked: potential secrets detected in outbound request",
            ))
        }
    }
}

/// Check rate limits before processing request
pub(crate) async fn check_rate_limits(
    state: &AppState,
    auth: &AuthContext,
    request: &ChatCompletionRequest,
) -> ApiResult<()> {
    // Estimate usage for rate limit check (rough estimate)
    let estimated_tokens = estimate_token_count(&request.messages);
    let max_output_tokens = request
        .max_completion_tokens
        .or(request.max_tokens)
        .unwrap_or(100);
    let usage_estimate = UsageInfo {
        input_tokens: estimated_tokens,
        output_tokens: max_output_tokens as u64,
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
            error.error = error
                .error
                .with_code(format!("retry_after_{}", retry_after));
        }

        return Err(error);
    }

    Ok(())
}

/// Convert API request to provider request format
pub(crate) fn convert_to_provider_request(
    request: &ChatCompletionRequest,
) -> ApiResult<ProviderCompletionRequest> {
    let messages = request
        .messages
        .iter()
        .map(|msg| {
            let content = match &msg.content {
                Some(MessageContent::Text(text)) => ProviderMessageContent::Text(text.clone()),
                Some(MessageContent::Parts(parts)) => {
                    // Convert server content parts to provider content parts
                    let provider_parts: Vec<ProviderContentPart> = parts
                        .iter()
                        .map(|part| match part {
                            crate::types::ContentPart::Text { text } => {
                                ProviderContentPart::Text { text: text.clone() }
                            }
                            crate::types::ContentPart::ImageUrl { image_url } => {
                                ProviderContentPart::ImageUrl {
                                    image_url: ProviderImageUrl {
                                        url: image_url.url.clone(),
                                        detail: image_url.detail.clone(),
                                    },
                                }
                            }
                        })
                        .collect();
                    ProviderMessageContent::Parts(provider_parts)
                }
                None => ProviderMessageContent::Text(String::new()),
            };

            // Convert tool_calls from server types to provider types
            let tool_calls = msg.tool_calls.as_ref().map(|tcs| {
                tcs.iter()
                    .map(|tc| lr_providers::ToolCall {
                        id: tc.id.clone(),
                        tool_type: tc.tool_type.clone(),
                        function: lr_providers::FunctionCall {
                            name: tc.function.name.clone(),
                            arguments: tc.function.arguments.clone(),
                        },
                    })
                    .collect()
            });

            Ok(ProviderChatMessage {
                role: msg.role.clone(),
                content,
                tool_calls,
                tool_call_id: msg.tool_call_id.clone(),
                name: msg.name.clone(),
                reasoning_content: None,
            })
        })
        .collect::<ApiResult<Vec<_>>>()?;

    // Prefer max_completion_tokens over max_tokens (for o-series models)
    let max_tokens = request.max_completion_tokens.or(request.max_tokens);

    // Convert tools from server types to provider types
    let tools = request.tools.as_ref().map(|server_tools| {
        server_tools
            .iter()
            .map(|tool| lr_providers::Tool {
                tool_type: tool.tool_type.clone(),
                function: lr_providers::FunctionDefinition {
                    name: tool.function.name.clone(),
                    description: tool.function.description.clone(),
                    parameters: tool.function.parameters.clone(),
                },
            })
            .collect()
    });

    // Convert tool_choice from server types to provider types
    let tool_choice = request.tool_choice.as_ref().map(|choice| match choice {
        crate::types::ToolChoice::Auto(s) => lr_providers::ToolChoice::Auto(s.clone()),
        crate::types::ToolChoice::Specific {
            tool_type,
            function,
        } => lr_providers::ToolChoice::Specific {
            tool_type: tool_type.clone(),
            function: lr_providers::FunctionName {
                name: function.name.clone(),
            },
        },
    });

    // Convert response_format from server types to provider types (Bug #7 fix)
    let response_format = request.response_format.as_ref().map(|format| match format {
        crate::types::ResponseFormat::JsonObject { r#type } => {
            lr_providers::ResponseFormat::JsonObject {
                format_type: r#type.clone(),
            }
        }
        crate::types::ResponseFormat::JsonSchema { r#type, schema } => {
            lr_providers::ResponseFormat::JsonSchema {
                format_type: r#type.clone(),
                schema: schema.clone(),
            }
        }
    });

    Ok(ProviderCompletionRequest {
        model: request.model.clone(),
        messages,
        temperature: request.temperature,
        max_tokens,
        stream: request.stream,
        top_p: request.top_p,
        frequency_penalty: request.frequency_penalty,
        presence_penalty: request.presence_penalty,
        stop: request.stop.as_ref().map(|s| match s {
            crate::types::StopSequence::Single(s) => vec![s.clone()],
            crate::types::StopSequence::Multiple(v) => v.clone(),
        }),
        // Extended parameters
        top_k: request.top_k,
        seed: request.seed,
        repetition_penalty: request.repetition_penalty,
        extensions: request.extensions.clone(),
        // Tool calling (Bug #4 fix)
        tools,
        tool_choice,
        // Response format (Bug #7 fix)
        response_format,
        // Log probabilities (Bug #6 fix)
        logprobs: request.logprobs,
        top_logprobs: request.top_logprobs,
        // Additional OpenAI-compatible parameters (pass-through)
        n: request.n,
        logit_bias: request.logit_bias.clone(),
        parallel_tool_calls: request.parallel_tool_calls,
        service_tier: request.service_tier.clone(),
        store: request.store,
        metadata: request.metadata.clone(),
        modalities: request.modalities.clone(),
        audio: request.audio.clone(),
        prediction: request.prediction.clone(),
        reasoning_effort: request.reasoning_effort.clone(),
        pre_computed_routing: None,
    })
}

/// Spawn a RouteLLM classification for `localrouter/auto` requests.
///
/// Returns `None` when the client has no RouteLLM config, no
/// RouteLLM service is loaded, or the client/strategy resolves to
/// nothing. When spawned, the task returns an `Option<PreComputedRouting>`
/// the caller stamps onto the provider request so the router can skip
/// its own classification step.
pub(crate) fn spawn_routellm_classification(
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

// ============================================================================
// Shared pipeline entry point: `run_turn_pipeline`
// ============================================================================

/// Per-endpoint capability flags. Adapters opt into the features
/// they want to run for a given turn. Each stage in
/// `run_turn_pipeline` consults the corresponding flag.
#[derive(Debug, Clone, Copy)]
pub(crate) struct PipelineCaps {
    /// Run prompt compression (mutates the request's messages
    /// in-place when the compressor produces a shorter version).
    pub allow_compression: bool,
    /// Resolve `localrouter/auto` requests via RouteLLM classification
    /// and bake the decision into `provider_request.pre_computed_routing`.
    pub allow_routellm: bool,
    /// Spawn the guardrail scan as a parallel `JoinHandle` (returned
    /// on `TurnContext`). When `false`, the scan runs sequentially
    /// inside `run_turn_pipeline` and any result is handled via
    /// `handle_guardrail_approval` before the helper returns.
    pub parallel_guardrails: bool,
}

impl PipelineCaps {
    /// Defaults for `/v1/chat/completions` — every stage enabled,
    /// guardrails spawn as a parallel handle so the caller can
    /// dispatch the LLM call while the scan runs.
    #[allow(dead_code)] // preset for future chat.rs migration to run_turn_pipeline
    pub(crate) fn chat() -> Self {
        Self {
            allow_compression: true,
            allow_routellm: true,
            parallel_guardrails: true,
        }
    }

    /// Defaults for `/v1/responses` — compression runs, RouteLLM
    /// does not (the adapter never sees `localrouter/auto` in its
    /// model field because `/responses` resolves models differently),
    /// guardrails run sequentially (simpler; no parallelism win for
    /// the typical single-turn pattern).
    pub(crate) fn responses() -> Self {
        Self {
            allow_compression: true,
            allow_routellm: false,
            parallel_guardrails: false,
        }
    }
}

/// Aggregated output of `run_turn_pipeline`. The caller drives the
/// LLM dispatch with this in hand, then threads it into the
/// `finalize::*` helpers so cost / metrics / monitor events stay
/// consistent across endpoints.
#[derive(Debug)]
pub(crate) struct TurnContext {
    /// Possibly-mutated chat request (compression may have replaced
    /// `messages`).
    pub chat_req: ChatCompletionRequest,
    /// Provider-shape request ready for `router.complete` /
    /// `router.stream_complete` or the MCP-via-LLM orchestrator.
    pub provider_request: ProviderCompletionRequest,
    /// Session ID threaded onto monitor events for this turn.
    #[allow(dead_code)]
    pub session_id: String,
    /// Stage-counted endpoint label (e.g. `/v1/chat/completions`)
    /// — echoed on any error monitor events the shared helper emits.
    #[allow(dead_code)]
    pub endpoint: &'static str,
    /// Number of prompt tokens compression removed from the original
    /// message payload. Passed to the finalize helper for the
    /// `feature_compression` cost-saved metric.
    pub compression_tokens_saved: u64,
    /// When `caps.parallel_guardrails = true` and a scan was spawned,
    /// this is the in-flight task. Caller awaits it before mutating
    /// side effects (tool calls, etc.) and feeds the result into
    /// `handle_guardrail_approval`. Responses.rs (sequential mode)
    /// sees `None` here and doesn't need to await.
    #[allow(dead_code)]
    pub guardrail_handle:
        Option<tokio::task::JoinHandle<ApiResult<Option<lr_guardrails::SafetyCheckResult>>>>,
}

/// Canonical pre-LLM pipeline. Runs validate → access checks → rate
/// limits → secret scan → guardrails → compression → RouteLLM →
/// provider-request conversion in order, gated by `caps`.
///
/// Emits validation / rate-limit monitor events with the supplied
/// `endpoint` label. Returns a `TurnContext` the caller threads
/// into dispatch and finalize.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_turn_pipeline(
    state: &AppState,
    auth: &AuthContext,
    client_auth: Option<&Extension<ClientAuthContext>>,
    mut chat_req: ChatCompletionRequest,
    llm_guard: &mut super::monitor_helpers::LlmCallGuard,
    endpoint: &'static str,
    session_id: String,
    caps: PipelineCaps,
) -> ApiResult<TurnContext> {
    // Stage 1: validate
    if let Err(e) = validate_request(&chat_req) {
        super::monitor_helpers::emit_validation_error(
            state,
            client_auth,
            Some(&session_id),
            endpoint,
            e.error.error.param.as_deref(),
            &e.error.error.message,
            400,
        );
        return Err(llm_guard.capture_err(e));
    }

    // Stage 2: access checks (strategy / auto-routing firewall / MCP mode gate)
    apply_model_access_checks(
        state,
        auth,
        client_auth,
        &session_id,
        &mut chat_req,
        llm_guard,
    )
    .await?;

    // Stage 3: rate limits
    if let Err(e) = check_rate_limits(state, auth, &chat_req).await {
        super::monitor_helpers::emit_rate_limit_event(
            state,
            client_auth,
            Some(&session_id),
            "rate_limit_exceeded",
            endpoint,
            &e.error.error.message,
            429,
            None,
        );
        return Err(llm_guard.capture_err(e));
    }

    // Stage 4: secret scan
    run_secret_scan_check(state, client_auth.map(|e| &e.0), &chat_req)
        .await
        .map_err(|e| llm_guard.capture_err(e))?;

    // Stage 5: guardrails (parallel or sequential per caps)
    let guardrail_handle = if caps.parallel_guardrails
        && client_auth.is_some()
        && state
            .safety_engine
            .read()
            .as_ref()
            .is_some_and(|e| e.has_models())
    {
        let state_ref = state.clone();
        let client_ctx = client_auth.map(|e| e.0.clone());
        let request_clone = chat_req.clone();
        Some(tokio::spawn(async move {
            run_guardrails_scan(&state_ref, client_ctx.as_ref(), &request_clone).await
        }))
    } else {
        // Sequential path: await + block here if the scan denies.
        if let Some(result) =
            run_guardrails_scan(state, client_auth.map(|e| &e.0), &chat_req).await?
        {
            handle_guardrail_approval(
                state,
                client_auth.map(|e| &e.0),
                &chat_req,
                result,
                "request",
            )
            .await
            .map_err(|e| llm_guard.capture_err(e))?;
        }
        None
    };

    // Stage 6: compression (awaited synchronously — compression output
    // mutates `chat_req.messages`; downstream stages need the final
    // form).
    let mut compression_tokens_saved: u64 = 0;
    if caps.allow_compression {
        let compression_result =
            run_prompt_compression(state, client_auth.map(|e| &e.0), &chat_req).await;
        if let Ok(Some(compressed)) = compression_result {
            if compressed.original_tokens > compressed.compressed_tokens {
                let saved = (compressed.original_tokens - compressed.compressed_tokens) as u64;
                state
                    .metrics_collector
                    .record_feature_event("feature_compression", saved, 0.0);
                compression_tokens_saved = saved;
            }
            chat_req.messages = compressed
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
        }
    }

    // Stage 7: RouteLLM classification (before convert, since the
    // result is baked onto `provider_request.pre_computed_routing`).
    // Runs concurrently with compression today because compression
    // has already awaited above, but the helper returns the spawned
    // handle's result for `localrouter/auto` requests only — other
    // models skip.
    let routellm_routing = if caps.allow_routellm && chat_req.model == "localrouter/auto" {
        if let Some(handle) =
            spawn_routellm_classification(state, client_auth.map(|e| &e.0), &chat_req)
        {
            handle.await.map_err(|e| {
                llm_guard.capture_err(ApiErrorResponse::internal_error(format!(
                    "RouteLLM task failed: {}",
                    e
                )))
            })?
        } else {
            None
        }
    } else {
        None
    };

    // Stage 8: convert to provider request (with routing stamped on)
    let mut provider_request =
        convert_to_provider_request(&chat_req).map_err(|e| llm_guard.capture_err(e))?;
    if let Some(routing) = routellm_routing {
        provider_request.pre_computed_routing = Some(routing);
    }

    Ok(TurnContext {
        chat_req,
        provider_request,
        session_id,
        endpoint,
        compression_tokens_saved,
        guardrail_handle,
    })
}
