//! Cached-only policy preview; classifier execution is an explicit opt-in test.
use lr_config::RoutingPolicy;
use lr_providers::gateway::GatewayPreferences;
use lr_providers::{ChatMessage, CompletionRequest, PreComputedRouting};
use lr_router::explanation::RoutingExplanation;
use lr_server::state::AppState;
use serde::Serialize;
use serde_json::{json, Value};
use std::sync::Arc;
use tauri::State;

#[derive(Serialize)]
pub struct RoutingPolicyPreview {
    pub decision: Option<PreComputedRouting>,
    pub explanation: RoutingExplanation,
    pub context: Value,
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn preview_routing_policy(
    strategy_id: String,
    client_id: Option<String>,
    policy: RoutingPolicy,
    prompt: String,
    mode: Option<String>,
    execute_classifier: Option<bool>,
    request_json: Option<String>,
    gateway_options: Option<GatewayPreferences>,
    endpoint: Option<String>,
    state: State<'_, Arc<AppState>>,
) -> Result<RoutingPolicyPreview, String> {
    policy.validate()?;
    let config = state.config_manager.get();
    let strategy = config
        .strategies
        .iter()
        .find(|s| s.id == strategy_id)
        .ok_or("Strategy not found")?;
    let client_id = client_id
        .as_deref()
        .or(strategy.parent.as_deref())
        .unwrap_or("internal-test");
    if client_id != "internal-test"
        && !config
            .clients
            .iter()
            .any(|c| c.id == client_id && c.strategy_id == strategy_id)
    {
        return Err("Preview client must use this strategy".into());
    }
    let mut effective_strategy = strategy.clone();
    if let Some(auto) = effective_strategy.auto_config.as_mut() {
        auto.routing_policy = Some(policy.clone());
    }
    let strategy = &effective_strategy;
    let message: ChatMessage = serde_json::from_value(json!({"role":"user","content":prompt}))
        .map_err(|e| e.to_string())?;
    let native_endpoint = match endpoint.as_deref().unwrap_or("chat") {
        "chat" => false,
        "systemone" | "decisions" => true,
        _ => return Err("Unsupported preview endpoint".into()),
    };
    let mut request = if let Some(raw) = request_json {
        let mut value: Value = serde_json::from_str(&raw).map_err(|error| error.to_string())?;
        lr_providers::session::normalize_ingress(&mut value, None)
            .map_err(|error| error.to_string())?;
        lr_providers::gateway::reject_raw_controls(&value).map_err(|error| error.to_string())?;
        if endpoint.as_deref() == Some("decisions") {
            let decision: lr_providers::systemone::decisions::DecisionRequest =
                serde_json::from_value(value).map_err(|error| error.to_string())?;
            decision.validate().map_err(|error| error.to_string())?;
            let native = decision.into_systemone();
            state.router.decision_routing_request(&native)
        } else if native_endpoint {
            let native: lr_providers::SystemOneRequest =
                serde_json::from_value(value).map_err(|error| error.to_string())?;
            state.router.decision_routing_request(&native)
        } else {
            serde_json::from_value(value).map_err(|error| error.to_string())?
        }
    } else {
        CompletionRequest::new("localrouter/auto", vec![message])
    };
    request.model = "localrouter/auto".into();
    if let Some(options) = gateway_options {
        options.validate().map_err(|error| error.to_string())?;
        request
            .extensions
            .get_or_insert_with(Default::default)
            .insert(lr_providers::gateway::EXTENSION.into(), json!(options));
    }
    if let Some(mode) = mode {
        request
            .metadata
            .get_or_insert_with(Default::default)
            .insert("localrouter.mode".into(), mode);
    }
    let endpoint_type = if native_endpoint {
        lr_providers::EndpointType::SystemOne
    } else {
        lr_providers::EndpointType::Chat
    };
    state
        .router
        .prepare_privacy_request(client_id, &mut request)
        .map_err(|error| error.to_string())?;
    let context = lr_router::decision_routing::routing_context(
        &request.messages,
        request.metadata.as_ref(),
        &policy,
    );
    let decision = if execute_classifier.unwrap_or(false) {
        Some(
            state
                .router
                .evaluate_routing_policy_for_endpoint(
                    client_id,
                    strategy,
                    &policy,
                    &request,
                    endpoint_type,
                )
                .await,
        )
    } else if policy.enabled {
        state
            .router
            .deterministic_routing_decision(strategy, &policy, &request)
    } else {
        None
    };
    let mut explanation = state.router.routing_explanation_for_endpoint(
        strategy,
        &policy,
        &request,
        decision.as_ref(),
        if native_endpoint {
            lr_providers::EndpointType::SystemOne
        } else {
            lr_providers::EndpointType::Chat
        },
    );
    state
        .router
        .apply_session_preview(
            client_id,
            strategy,
            &request,
            endpoint_type,
            &mut explanation,
        )
        .map_err(|error| error.to_string())?;
    if client_id == "internal-test"
        && lr_providers::session::SessionOptions::from_request(&request)
            .map_err(|e| e.to_string())?
            .is_some()
    {
        explanation.session =
            Some(json!({"status":"open_client_tab_to_preview_its_session","read_only":true}));
    }
    Ok(RoutingPolicyPreview {
        decision,
        explanation,
        context,
    })
}
