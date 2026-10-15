//! Current privacy policy gates before auxiliary inference or persistent writes.
use crate::{
    middleware::error::{ApiErrorResponse, ApiResult},
    state::AppState,
};
use lr_config::LlmMode;

pub fn check_ingress(state: &AppState, client_id: &str, path: &str) -> ApiResult<()> {
    let policy = state.config_manager.client_privacy(client_id);
    if !policy.restricted() {
        return Ok(());
    }
    if path.starts_with("/mcp") {
        return Err(ApiErrorResponse::forbidden("Restricted privacy profiles currently support native LLM gateway traffic; use a separately configured client for MCP"));
    }
    let client = state
        .client_manager
        .get_client(client_id)
        .ok_or_else(|| ApiErrorResponse::forbidden("Client not found"))?;
    if client.llm_mode != LlmMode::Gateway || client.is_mcp_via_llm() {
        return Err(ApiErrorResponse::forbidden(
            "Restricted privacy profiles do not allow proxy or automatic MCP execution",
        ));
    }
    if lr_types::is_duplicate_hop() {
        return Err(ApiErrorResponse::forbidden(
            "Restricted privacy profiles do not allow duplicate-hop bypass",
        ));
    }
    if path.contains("/audio/") || path.contains("/images/") || path.ends_with("/moderations") {
        return Err(ApiErrorResponse::forbidden(
            "This modality has no verified restricted-privacy mapping",
        ));
    }
    let api_path = path.strip_prefix("/v1").unwrap_or(path);
    if matches!(
        api_path,
        "/chat/completions"
            | "/completions"
            | "/responses"
            | "/messages"
            | "/systemone"
            | "/decisions"
    ) {
        check_auxiliaries(state, client_id)?;
    }
    Ok(())
}
/// Direct safety executors snapshot credentials and do not inherit gateway
/// controls. Reject the unsupported combination rather than skip a safeguard.
pub fn check_auxiliaries(state: &AppState, client_id: &str) -> ApiResult<()> {
    let policy = state.config_manager.client_privacy(client_id);
    if !policy.restricted() {
        return Ok(());
    }
    let config = state.config_manager.get();
    let Some(client) = state.client_manager.get_client(client_id) else {
        return Ok(());
    };
    let actions = crate::routes::pipeline::merge_guardrail_category_actions(
        client.guardrails.category_actions.as_deref(),
        &config.guardrails.category_actions,
    );
    if config.guardrails.scan_requests
        && !actions.is_empty()
        && !crate::routes::pipeline::guardrails_allow_everything(&actions)
        && state
            .safety_engine
            .read()
            .as_ref()
            .is_some_and(|engine| engine.has_models())
    {
        return Err(ApiErrorResponse::forbidden("Direct safety-model execution has no verified strict privacy mapping; this request cannot run with the selected privacy and safety configuration"));
    }
    Ok(())
}
