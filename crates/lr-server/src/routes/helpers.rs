//! Shared helper functions for route handlers
//!
//! Provides common validation and lookup functions used across multiple endpoints.

use crate::middleware::error::ApiErrorResponse;
use crate::state::AppState;
use lr_config::{Client, Strategy};

/// Emit an AccessDenied monitor event from helpers.
fn emit_access_denied(
    state: &AppState,
    client_id: &str,
    reason: &str,
    message: &str,
    status_code: u16,
) {
    let client_name = state
        .client_manager
        .get_client(client_id)
        .map(|c| c.name.clone());
    state.monitor_store.push(
        lr_monitor::MonitorEventType::AccessDenied,
        Some(client_id.to_string()),
        client_name,
        None,
        lr_monitor::MonitorEventData::AccessDenied {
            reason: reason.to_string(),
            endpoint: String::new(),
            message: message.to_string(),
            status_code,
        },
        lr_monitor::EventStatus::Error,
        None,
    );
}

/// Result type for helper functions
pub type HelperResult<T> = Result<T, ApiErrorResponse>;

/// Check if a client_id is a transient internal token (not a real persisted client).
/// These bypass all client validation — they route directly to provider/model.
pub fn is_internal_client(client_id: &str) -> bool {
    client_id == "internal-test"
}

/// Create a synthetic client for internal tokens (no persisted config).
fn synthetic_internal_client(client_id: &str) -> Client {
    Client::new_with_strategy(client_id.to_string(), "internal".to_string())
}

/// Get an enabled client by ID, returning appropriate errors if not found or disabled.
///
/// This is the standard way to validate a client for any endpoint:
/// 1. Looks up client by ID in config
/// 2. Checks if client is enabled
/// 3. Returns 401 if not found, 403 if disabled
///
/// # Arguments
/// * `state` - Application state containing config manager
/// * `client_id` - The client ID to look up (from auth context)
///
/// # Example
/// ```ignore
/// let client = get_enabled_client(&state, &auth.api_key_id)?;
/// ```
pub fn get_enabled_client(state: &AppState, client_id: &str) -> HelperResult<Client> {
    if is_internal_client(client_id) {
        return Ok(synthetic_internal_client(client_id));
    }

    let config = state.config_manager.get();
    let client = config
        .clients
        .iter()
        .find(|c| c.id == client_id)
        .ok_or_else(|| {
            emit_access_denied(
                state,
                client_id,
                "client_not_found",
                "Client not found",
                401,
            );
            ApiErrorResponse::unauthorized("Client not found")
        })?
        .clone();

    if !client.enabled {
        emit_access_denied(
            state,
            client_id,
            "client_disabled",
            "Client is disabled",
            403,
        );
        return Err(ApiErrorResponse::forbidden("Client is disabled"));
    }

    Ok(client)
}

/// Get an enabled client along with its associated routing strategy.
///
/// This combines client validation with strategy lookup, which is needed
/// for endpoints that need to check model access (models, chat, completions).
///
/// # Arguments
/// * `state` - Application state containing config manager
/// * `client_id` - The client ID to look up (from auth context)
///
/// # Returns
/// A tuple of (Client, Strategy) if both are found and client is enabled.
///
/// # Example
/// ```ignore
/// let (client, strategy) = get_client_with_strategy(&state, &auth.api_key_id)?;
/// if strategy.is_model_allowed(&provider, &model) { ... }
/// ```
pub fn get_client_with_strategy(
    state: &AppState,
    client_id: &str,
) -> HelperResult<(Client, Strategy)> {
    if is_internal_client(client_id) {
        return Ok((
            synthetic_internal_client(client_id),
            Strategy::new(client_id.to_string()),
        ));
    }

    let config = state.config_manager.get();

    let client = config
        .clients
        .iter()
        .find(|c| c.id == client_id)
        .ok_or_else(|| {
            emit_access_denied(
                state,
                client_id,
                "client_not_found",
                "Client not found",
                401,
            );
            ApiErrorResponse::unauthorized("Client not found")
        })?
        .clone();

    if !client.enabled {
        emit_access_denied(
            state,
            client_id,
            "client_disabled",
            "Client is disabled",
            403,
        );
        return Err(ApiErrorResponse::forbidden("Client is disabled"));
    }

    let strategy = config
        .strategies
        .iter()
        .find(|s| s.id == client.strategy_id)
        .ok_or_else(|| {
            emit_access_denied(
                state,
                client_id,
                "strategy_not_found",
                &format!("Strategy '{}' not found", client.strategy_id),
                500,
            );
            ApiErrorResponse::internal_error(format!(
                "Strategy '{}' not found for client '{}'",
                client.strategy_id, client.id
            ))
        })?
        .clone();

    Ok((client, strategy))
}

/// Get an enabled client by ID from the client manager (for MCP routes).
///
/// This is the standard way to validate a client for MCP endpoints that use
/// ClientAuthContext. Uses the client_manager for direct lookup.
///
/// # Arguments
/// * `state` - Application state containing client manager
/// * `client_id` - The client ID to look up (from ClientAuthContext)
///
/// # Example
/// ```ignore
/// let client = get_enabled_client_from_manager(&state, &client_ctx.client_id)?;
/// ```
pub fn get_enabled_client_from_manager(state: &AppState, client_id: &str) -> HelperResult<Client> {
    if is_internal_client(client_id) {
        return Ok(synthetic_internal_client(client_id));
    }

    let client = state.client_manager.get_client(client_id).ok_or_else(|| {
        emit_access_denied(
            state,
            client_id,
            "client_not_found",
            "Client not found",
            401,
        );
        ApiErrorResponse::unauthorized("Client not found")
    })?;

    if !client.enabled {
        emit_access_denied(
            state,
            client_id,
            "client_disabled",
            "Client is disabled",
            403,
        );
        return Err(ApiErrorResponse::forbidden("Client is disabled"));
    }

    Ok(client)
}

/// Check that a client is allowed to access LLM endpoints.
/// Returns 403 if client mode is `McpOnly`.
pub fn check_llm_access_with_state(state: &AppState, client: &Client) -> HelperResult<()> {
    if client.client_mode == lr_config::ClientMode::McpOnly {
        emit_access_denied(
            state,
            &client.id,
            "mcp_only_client_llm",
            "Client is in MCP-only mode and cannot access LLM endpoints",
            403,
        );
        return Err(ApiErrorResponse::forbidden(
            "Client is in MCP-only mode and cannot access LLM endpoints",
        ));
    }
    Ok(())
}

/// Validate that the requested model is in the strategy's allowed models list.
///
/// This ensures the model whitelist configured in the strategy is enforced at request time,
/// not just at listing time (/v1/models). Returns 403 if the model is not enabled.
pub fn validate_strategy_model_access(
    state: &AppState,
    strategy: &Strategy,
    model: &str,
) -> HelperResult<()> {
    // If selected_all is true, all models are allowed
    if strategy.allowed_models.selected_all {
        return Ok(());
    }

    // Parse model string: "provider/model" or just "model"
    if let Some((provider, model_id)) = model.split_once('/') {
        if !strategy.is_model_allowed(provider, model_id) {
            return Err(ApiErrorResponse::forbidden(format!(
                "Model '{}' is not enabled for this client. Enable it in the model selection settings.",
                model
            )));
        }
    } else {
        // No provider specified - check if any provider has this model allowed
        let all_models = state.provider_registry.list_all_models_instant();
        let is_allowed = all_models.iter().any(|m| {
            m.id.eq_ignore_ascii_case(model) && strategy.is_model_allowed(&m.provider, &m.id)
        });
        if !is_allowed {
            return Err(ApiErrorResponse::forbidden(format!(
                "Model '{}' is not enabled for this client. Enable it in the model selection settings.",
                model
            )));
        }
    }

    Ok(())
}

/// Check that auto_config.permission allows model access.
///
/// Returns 403 if permission is Off (all model access disabled).
/// Used as a master switch before individual model checks.
pub fn check_strategy_permission(strategy: &Strategy) -> HelperResult<()> {
    if let Some(ref auto_config) = strategy.auto_config {
        if !auto_config.permission.is_enabled() {
            return Err(ApiErrorResponse::forbidden(
                "Model access is disabled for this client. Contact administrator to grant access.",
            ));
        }
    }
    Ok(())
}

/// Compatibility wrapper — does not emit monitor events.
pub fn check_llm_access(client: &Client) -> HelperResult<()> {
    if client.client_mode == lr_config::ClientMode::McpOnly {
        return Err(ApiErrorResponse::forbidden(
            "Client is in MCP-only mode and cannot access LLM endpoints",
        ));
    }
    Ok(())
}

/// Check that a client is allowed to access MCP endpoints directly.
/// Returns 403 if client mode is `LlmOnly` or `McpViaLlm`.
pub fn check_mcp_access_with_state(state: &AppState, client: &Client) -> HelperResult<()> {
    match client.client_mode {
        lr_config::ClientMode::LlmOnly => {
            emit_access_denied(
                state,
                &client.id,
                "llm_only_client_mcp",
                "Client is in LLM-only mode and cannot access MCP endpoints",
                403,
            );
            Err(ApiErrorResponse::forbidden(
                "Client is in LLM-only mode and cannot access MCP endpoints",
            ))
        }
        lr_config::ClientMode::McpViaLlm => {
            emit_access_denied(
                state,
                &client.id,
                "mcp_via_llm_direct_mcp",
                "Client is in MCP-via-LLM mode. MCP tools are available through LLM chat completions, not direct MCP access",
                403,
            );
            Err(ApiErrorResponse::forbidden(
                "Client is in MCP-via-LLM mode. MCP tools are available through LLM chat completions, not direct MCP access",
            ))
        }
        _ => Ok(()),
    }
}

/// Compatibility wrapper — does not emit monitor events.
pub fn check_mcp_access(client: &Client) -> HelperResult<()> {
    match client.client_mode {
        lr_config::ClientMode::LlmOnly => Err(ApiErrorResponse::forbidden(
            "Client is in LLM-only mode and cannot access MCP endpoints",
        )),
        lr_config::ClientMode::McpViaLlm => Err(ApiErrorResponse::forbidden(
            "Client is in MCP-via-LLM mode. MCP tools are available through LLM chat completions, not direct MCP access",
        )),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    // Tests would go here with mock state
}
