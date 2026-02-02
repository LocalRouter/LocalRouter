//! Shared helper functions for route handlers
//!
//! Provides common validation and lookup functions used across multiple endpoints.

use crate::middleware::error::ApiErrorResponse;
use crate::state::AppState;
use lr_config::{Client, Strategy};

/// Result type for helper functions
pub type HelperResult<T> = Result<T, ApiErrorResponse>;

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
    let config = state.config_manager.get();
    let client = config
        .clients
        .iter()
        .find(|c| c.id == client_id)
        .ok_or_else(|| ApiErrorResponse::unauthorized("Client not found"))?
        .clone();

    if !client.enabled {
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
    let config = state.config_manager.get();

    let client = config
        .clients
        .iter()
        .find(|c| c.id == client_id)
        .ok_or_else(|| ApiErrorResponse::unauthorized("Client not found"))?
        .clone();

    if !client.enabled {
        return Err(ApiErrorResponse::forbidden("Client is disabled"));
    }

    let strategy = config
        .strategies
        .iter()
        .find(|s| s.id == client.strategy_id)
        .ok_or_else(|| {
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
    let client = state
        .client_manager
        .get_client(client_id)
        .ok_or_else(|| ApiErrorResponse::unauthorized("Client not found"))?;

    if !client.enabled {
        return Err(ApiErrorResponse::forbidden("Client is disabled"));
    }

    Ok(client)
}

#[cfg(test)]
mod tests {
    // Tests would go here with mock state
}
