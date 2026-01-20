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
pub async fn client_auth_middleware(mut req: Request, next: Next) -> Response {
    // Extract Authorization header
    let auth_header = match req
        .headers()
        .get("Authorization")
        .and_then(|h| h.to_str().ok())
    {
        Some(h) => h,
        None => {
            return ApiErrorResponse::unauthorized("Missing Authorization header").into_response();
        }
    };

    // Extract bearer token
    let token = match extract_bearer_token(auth_header) {
        Some(t) => t,
        None => {
            return ApiErrorResponse::unauthorized(
                "Invalid Authorization header format. Expected: Bearer <token>",
            )
            .into_response();
        }
    };

    // Extract state from request extensions
    let state = match req.extensions().get::<crate::server::state::AppState>() {
        Some(s) => s.clone(),
        None => {
            return ApiErrorResponse::internal_error("Missing application state").into_response();
        }
    };

    // Try OAuth access token first (short-lived tokens from /oauth/token)
    let client_id = if let Some(id) = state.token_store.verify_token(&token) {
        // Token is a valid OAuth access token
        tracing::debug!("Client authenticated via OAuth access token: {}", id);
        id
    } else {
        // Try direct client secret (long-lived credentials)
        match state.client_manager.verify_secret(&token) {
            Ok(Some(client)) => {
                tracing::debug!("Client authenticated via client secret: {}", client.id);
                client.id
            }
            Ok(None) => {
                tracing::warn!("Invalid bearer token provided");
                return ApiErrorResponse::unauthorized("Invalid bearer token").into_response();
            }
            Err(e) => {
                tracing::error!("Error verifying client secret: {}", e);
                return ApiErrorResponse::internal_error("Authentication error").into_response();
            }
        }
    };

    // Create auth context
    let auth_context = ClientAuthContext { client_id };

    // Insert auth context into request extensions
    req.extensions_mut().insert(auth_context);

    // Continue to next middleware/handler
    next.run(req).await
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
