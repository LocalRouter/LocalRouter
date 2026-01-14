//! Authentication middleware for API key validation

use axum::{
    extract::Request,
    middleware::Next,
    response::Response,
};

use crate::config::ModelSelection;
use crate::server::middleware::error::ApiErrorResponse;
use crate::server::state::{AppState, AuthContext};

/// Authentication middleware
/// Extracts and validates API key from Authorization header
pub async fn auth_middleware(
    mut req: Request,
    next: Next,
) -> Result<Response, ApiErrorResponse> {
    // Get state from request extensions
    let state = req
        .extensions()
        .get::<AppState>()
        .cloned()
        .ok_or_else(|| ApiErrorResponse::internal_error("Missing application state"))?;
    // Extract Authorization header
    let auth_header = req
        .headers()
        .get("Authorization")
        .and_then(|h| h.to_str().ok())
        .ok_or_else(|| ApiErrorResponse::unauthorized("Missing Authorization header"))?;

    // Check if it starts with "Bearer "
    if !auth_header.starts_with("Bearer ") {
        return Err(ApiErrorResponse::unauthorized("Invalid Authorization header format"));
    }

    // Extract the API key
    let api_key = &auth_header[7..]; // Skip "Bearer "

    // Validate the API key
    let api_key_manager = state.api_key_manager.read();
    let api_key_info = api_key_manager
        .verify_key(api_key)
        .ok_or_else(|| ApiErrorResponse::unauthorized("Invalid API key"))?;

    // Note: verify_key already checks if key is enabled, so we don't need to check again

    // Parse model selection
    let model_selection = parse_model_selection(&api_key_info.model_selection);

    // Create auth context
    let auth_context = AuthContext {
        api_key_id: api_key_info.id.clone(),
        model_selection,
    };

    // Insert auth context into request extensions
    req.extensions_mut().insert(auth_context);

    Ok(next.run(req).await)
}

/// Parse model selection from API key config
fn parse_model_selection(selection: &ModelSelection) -> crate::server::state::ModelSelection {
    match selection {
        ModelSelection::DirectModel { provider, model } => {
            crate::server::state::ModelSelection::DirectModel {
                provider: provider.clone(),
                model: model.clone(),
            }
        }
        ModelSelection::Router { router_name } => {
            crate::server::state::ModelSelection::Router {
                router_name: router_name.clone(),
            }
        }
    }
}
