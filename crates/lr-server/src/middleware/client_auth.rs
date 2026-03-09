//! Unified client authentication middleware for MCP proxy
//!
//! Supports three authentication methods:
//! 1. Internal test token (for UI testing, bypasses restrictions)
//! 2. OAuth access tokens (short-lived, from /oauth/token endpoint)
//! 3. Direct bearer tokens (client secret used directly)
//!
//! All methods use the same Authorization header format:
//! "Authorization: Bearer <token>"

use axum::{
    extract::Request,
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};

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
/// Returns None if token is missing, empty/whitespace-only, or format is invalid
fn extract_bearer_token(auth_header: &str) -> Option<String> {
    if !auth_header.starts_with("Bearer ") {
        return None;
    }

    auth_header.strip_prefix("Bearer ").and_then(|s| {
        if s.trim().is_empty()
            || s.len() > 256
            || !s
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
        {
            None
        } else {
            Some(s.to_string())
        }
    })
}

/// Extract token from URL query parameter (?token=<value>)
///
/// Fallback authentication for MCP clients (like Claude Code) that don't send
/// custom headers with Streamable HTTP transport.
fn extract_query_token(query: &str) -> Option<String> {
    for pair in query.split('&') {
        if let Some(value) = pair.strip_prefix("token=") {
            let value = value.trim();
            if value.is_empty() || value.len() > 256 {
                return None;
            }
            if !value
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
            {
                return None;
            }
            return Some(value.to_string());
        }
    }
    None
}

/// Return an OAuth-compatible 401 response
///
/// MCP clients (Claude Code) expect RFC 6749 format: {"error": "string", "error_description": "string"}
/// NOT the OpenAI format: {"error": {"message": "...", "type": "..."}}
fn mcp_unauthorized_response(description: &str) -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(serde_json::json!({
            "error": "unauthorized",
            "error_description": description
        })),
    )
        .into_response()
}

/// Client authentication middleware for MCP proxy routes
///
/// Validates bearer tokens from Authorization header or URL query parameter.
/// Supports three token sources:
/// 1. Authorization: Bearer <token> header
/// 2. ?token=<value> query parameter (fallback for MCP HTTP clients)
///
/// And three token types:
/// 1. Internal test token (for UI testing, bypasses restrictions)
/// 2. OAuth access tokens (short-lived, from /oauth/token endpoint)
/// 3. Direct client secrets (validated via ClientManager)
///
/// On success, attaches ClientAuthContext to request extensions.
pub async fn client_auth_middleware(mut req: Request, next: Next) -> Response {
    // Allow unauthenticated GET requests to root info endpoints (non-SSE)
    // These return API documentation and don't require auth
    let path = req.uri().path().to_string();
    let is_get = req.method() == axum::http::Method::GET;
    let accepts_sse = req
        .headers()
        .get(axum::http::header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.contains("text/event-stream"))
        .unwrap_or(false);

    if is_get && !accepts_sse && (path == "/" || path == "/mcp") {
        return next.run(req).await;
    }

    // Extract token: try Authorization header first, then query parameter
    let token = if let Some(auth_header) = req
        .headers()
        .get("Authorization")
        .and_then(|h| h.to_str().ok())
    {
        match extract_bearer_token(auth_header) {
            Some(t) => t,
            None => {
                return mcp_unauthorized_response(
                    "Invalid Authorization header format. Expected: Bearer <token>",
                );
            }
        }
    } else if let Some(t) = req.uri().query().and_then(extract_query_token) {
        t
    } else {
        return mcp_unauthorized_response("Missing authentication. Provide Authorization: Bearer <token> header or ?token=<value> query parameter");
    };

    // Extract state from request extensions
    let state = match req.extensions().get::<crate::state::AppState>() {
        Some(s) => s.clone(),
        None => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "server_error", "error_description": "Missing application state"})),
            ).into_response();
        }
    };

    // Check if this is the internal test token (for UI testing)
    // This allows the Tauri frontend to access MCP servers without a configured client
    if token == state.internal_test_secret.as_str() {
        tracing::debug!(
            "Internal test token detected - bypassing client restrictions for UI MCP testing"
        );
        let auth_context = ClientAuthContext {
            client_id: "internal-test".to_string(),
        };
        req.extensions_mut().insert(auth_context);
        return next.run(req).await;
    }

    // Try OAuth access token first (short-lived tokens from /oauth/token)
    let client_id = if let Some(id) = state.token_store.verify_token(&token) {
        // Token is a valid OAuth access token
        tracing::debug!(
            event = "auth_success",
            client_id = %id,
            method = "oauth_token",
            "Client authenticated via OAuth access token"
        );
        id
    } else {
        // Try direct client secret (long-lived credentials)
        match state.client_manager.verify_secret(&token) {
            Ok(Some(client)) => {
                tracing::debug!(
                    event = "auth_success",
                    client_id = %client.id,
                    method = "client_secret",
                    "Client authenticated via client secret"
                );
                client.id
            }
            Ok(None) => {
                tracing::warn!(
                    event = "auth_failed",
                    reason = "invalid_token",
                    "Authentication failed: invalid bearer token"
                );
                return mcp_unauthorized_response("Invalid bearer token");
            }
            Err(e) => {
                tracing::error!("Error verifying client secret: {}", e);
                return mcp_unauthorized_response("Authentication error");
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

        // Empty token - should be rejected
        let auth = "Bearer ";
        let result = extract_bearer_token(auth);
        assert_eq!(result, None);

        // Whitespace-only token - should also be rejected
        let auth = "Bearer    ";
        let result = extract_bearer_token(auth);
        assert_eq!(result, None);

        // Token too long - should be rejected
        let auth = format!("Bearer {}", "a".repeat(257));
        let result = extract_bearer_token(&auth);
        assert_eq!(result, None);

        // Token with valid length
        let auth = format!("Bearer {}", "a".repeat(256));
        let result = extract_bearer_token(&auth);
        assert!(result.is_some());

        // Token with invalid characters
        let auth = "Bearer abc!@#";
        let result = extract_bearer_token(auth);
        assert_eq!(result, None);

        // Token with allowed special chars
        let auth = "Bearer lr-abc_123.test";
        let result = extract_bearer_token(auth);
        assert_eq!(result, Some("lr-abc_123.test".to_string()));
    }
}
