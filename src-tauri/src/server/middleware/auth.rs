//! Authentication middleware for API key validation

use axum::{
    extract::Request,
    middleware::Next,
    response::{IntoResponse, Response},
};

use crate::config::ModelSelection;
use crate::server::middleware::error::ApiErrorResponse;
use crate::server::state::{AppState, AuthContext};

/// Authentication middleware
/// Extracts and validates API key from Authorization header
pub async fn auth_middleware(
    req: Request,
    next: Next,
) -> Response {
    // Helper function to handle errors and convert to responses
    async fn handle_request(
        mut req: Request,
        next: Next,
    ) -> Result<Response, ApiErrorResponse> {
        // Extract state from request extensions
        let state = req
            .extensions()
            .get::<AppState>()
            .ok_or_else(|| ApiErrorResponse::internal_error("Missing application state"))?
            .clone();

        // Extract Authorization header
        let auth_header = req
            .headers()
            .get("Authorization")
            .and_then(|h| h.to_str().ok())
            .ok_or_else(|| ApiErrorResponse::unauthorized("Missing Authorization header"))?;

        // Check if it starts with "Bearer "
        if !auth_header.starts_with("Bearer ") {
            return Err(ApiErrorResponse::unauthorized(
                "Invalid Authorization header format",
            ));
        }

        // Extract the API key
        let api_key = &auth_header[7..]; // Skip "Bearer "

        // Validate the API key and extract needed info
        let (api_key_id, model_selection) = {
            let api_key_manager = state.api_key_manager.read();
            let api_key_info = api_key_manager
                .verify_key(api_key)
                .ok_or_else(|| ApiErrorResponse::unauthorized("Invalid API key"))?;

            // Note: verify_key already checks if key is enabled, so we don't need to check again

            // Parse model selection and extract data before dropping the lock
            let model_selection = parse_model_selection(&api_key_info.model_selection);
            (api_key_info.id.clone(), model_selection)
        }; // Lock is dropped here

        // Create auth context
        let auth_context = AuthContext {
            api_key_id,
            model_selection,
        };

        // Insert auth context into request extensions
        req.extensions_mut().insert(auth_context);

        Ok(next.run(req).await)
    }

    // Call the helper and convert errors to responses
    match handle_request(req, next).await {
        Ok(response) => response,
        Err(err) => err.into_response(),
    }
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
