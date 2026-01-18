//! Unified client authentication middleware for MCP proxy
//!
//! Supports two authentication methods:
//! 1. OAuth access tokens (short-lived, from /oauth/token endpoint)
//! 2. Direct bearer tokens (client secret used directly)
//!
//! Both methods use the same Authorization header format:
//! "Authorization: Bearer <token>"

use axum::{
    extract::Request,
    middleware::Next,
    response::{IntoResponse, Response},
};

use crate::server::middleware::error::ApiErrorResponse;

/// Client authentication context
///
/// Attached to request extensions after successful authentication.
/// Contains the authenticated client_id for access control checks.
#[derive(Debug, Clone)]
pub struct ClientAuthContext {
    /// The authenticated client ID
    pub client_id: String,
}

/// Extract Bearer token from Authorization header
///
/// Expected format: "Bearer <token>"
fn extract_bearer_token(auth_header: &str) -> Option<String> {
    if !auth_header.starts_with("Bearer ") {
        return None;
    }

    auth_header.strip_prefix("Bearer ").map(|s| s.to_string())
}

/// Client authentication middleware for MCP proxy routes
///
/// Validates bearer tokens from Authorization header.
/// Supports two token types:
/// 1. OAuth access tokens (validated via TokenStore)
/// 2. Direct client secrets (validated via ClientManager)
///
/// On success, attaches ClientAuthContext to request extensions.
pub async fn client_auth_middleware(
    req: Request,
    next: Next,
) -> Response {
    // Helper function to handle errors and convert to responses
    async fn handle_request(
        req: Request,
        _next: Next,
    ) -> Result<Response, ApiErrorResponse> {
        // Extract Authorization header
        let auth_header = req
            .headers()
            .get("Authorization")
            .and_then(|h| h.to_str().ok())
            .ok_or_else(|| ApiErrorResponse::unauthorized("Missing Authorization header"))?;

        // Extract bearer token
        let _token = extract_bearer_token(auth_header)
            .ok_or_else(|| ApiErrorResponse::unauthorized("Invalid Authorization header format. Expected: Bearer <token>"))?;

        // TODO: Get ClientManager and TokenStore from AppState
        // For now, we'll use the old OAuth approach until we wire up the new system

        // Try to verify as OAuth access token first (short-lived)
        // If that fails, try to verify as direct client secret (long-lived)

        // TEMPORARY: Use old OAuth system
        // This will be replaced once we wire up ClientManager and TokenStore in AppState
        Err(ApiErrorResponse::internal_error(
            "Client authentication not yet wired up - in progress"
        ))

        // TODO: Replace with this once wired up:
        /*
        // Extract state from request extensions
        let state = req
            .extensions()
            .get::<AppState>()
            .ok_or_else(|| ApiErrorResponse::internal_error("Missing application state"))?
            .clone();

        // Try OAuth access token first
        let client_id = if let Some(id) = state.token_store.verify_token(&token) {
            // Token is a valid OAuth access token
            id
        } else {
            // Try direct client secret
            match state.client_manager.verify_secret(&token) {
                Ok(Some(client)) => client.client_id,
                Ok(None) => {
                    return Err(ApiErrorResponse::unauthorized("Invalid bearer token"));
                }
                Err(e) => {
                    tracing::error!("Error verifying client secret: {}", e);
                    return Err(ApiErrorResponse::internal_error("Authentication error"));
                }
            }
        };

        // Create auth context
        let auth_context = ClientAuthContext { client_id };

        // Insert auth context into request extensions
        req.extensions_mut().insert(auth_context);

        Ok(next.run(req).await)
        */
    }

    // Call the helper and convert errors to responses
    match handle_request(req, next).await {
        Ok(response) => response,
        Err(err) => err.into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_bearer_token() {
        // Valid Bearer token
        let auth = "Bearer abc123xyz";
        let result = extract_bearer_token(auth);
        assert_eq!(result, Some("abc123xyz".to_string()));

        // Invalid format (not Bearer)
        let auth = "Basic abc123";
        let result = extract_bearer_token(auth);
        assert_eq!(result, None);

        // No space after Bearer
        let auth = "Bearerabc123";
        let result = extract_bearer_token(auth);
        assert_eq!(result, None);

        // Empty token
        let auth = "Bearer ";
        let result = extract_bearer_token(auth);
        assert_eq!(result, Some("".to_string()));

        // Token with spaces (should include everything after "Bearer ")
        let auth = "Bearer token with spaces";
        let result = extract_bearer_token(auth);
        assert_eq!(result, Some("token with spaces".to_string()));
    }
}
