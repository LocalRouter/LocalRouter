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
    let client_id = strategy.parent.as_deref().unwrap_or("internal-test");
    let message: ChatMessage = serde_json::from_value(json!({"role":"user","content":prompt}))
        .map_err(|e| e.to_string())?;
    let native_endpoint = match endpoint.as_deref().unwrap_or("chat") {
        "chat" => false,
        "systemone" | "decisions" => true,
        _ => return Err("Unsupported preview endpoint".into()),
    };
    let mut request = if let Some(raw) = request_json {
        let value: Value = serde_json::from_str(&raw).map_err(|error| error.to_string())?;
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
    let context = lr_router::decision_routing::routing_context(
        &request.messages,
        request.metadata.as_ref(),
        &policy,
    );
    let decision = if execute_classifier.unwrap_or(false) {
        Some(
            state
                .router
                .evaluate_routing_policy(client_id, strategy, &policy, &request)
                .await,
        )
    } else if policy.enabled {
        state
            .router
            .deterministic_routing_decision(strategy, &policy, &request)
    } else {
        None
    };
    let explanation = state.router.routing_explanation_for_endpoint(
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
    Ok(RoutingPolicyPreview {
        decision,
        explanation,
        context,
    })
}
