//! Preview exactly the same policy evaluation used by auto routing.
use lr_config::RoutingPolicy;
use lr_providers::{ChatMessage, CompletionRequest, PreComputedRouting};
use lr_server::state::AppState;
use serde::Serialize;
use serde_json::{json, Value};
use std::sync::Arc;
use tauri::State;

#[derive(Serialize)]
pub struct RoutingPolicyPreview {
    pub decision: PreComputedRouting,
    pub context: Value,
}

#[tauri::command]
pub async fn preview_routing_policy(
    strategy_id: String,
    policy: RoutingPolicy,
    prompt: String,
    mode: Option<String>,
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
    let mut request = CompletionRequest::new("localrouter/auto", vec![message]);
    request.metadata = mode.map(|m| [("localrouter.mode".to_string(), m)].into_iter().collect());
    let context = lr_router::decision_routing::routing_context(
        &request.messages,
        request.metadata.as_ref(),
        &policy,
    )
    .unwrap_or_else(|e| json!({"error":e}));
    let decision = state
        .router
        .evaluate_routing_policy(client_id, strategy, &policy, &request)
        .await;
    Ok(RoutingPolicyPreview { decision, context })
}
