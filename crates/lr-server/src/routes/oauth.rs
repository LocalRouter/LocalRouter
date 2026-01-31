//! OAuth 2.0 endpoints for client authentication
//!
//! Implements the OAuth 2.0 client credentials flow for generating short-lived access tokens.
//! Reference: https://datatracker.ietf.org/doc/html/rfc6749#section-4.4

use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::ToSchema;

use lr_clients::{ClientManager, TokenStore};

/// OAuth 2.0 token request (client credentials flow)
///
/// Can be sent either as:
/// 1. JSON body with client_id and client_secret
/// 2. Form-encoded body with client_id and client_secret
/// 3. Basic Authentication header with client_id:client_secret (base64 encoded)
#[derive(Debug, Deserialize, ToSchema)]
pub struct TokenRequest {
    /// OAuth 2.0 grant type (must be "client_credentials")
    #[schema(example = "client_credentials")]
    pub grant_type: String,

    /// Client ID (optional if using Basic Auth)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,

    /// Client secret (optional if using Basic Auth)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_secret: Option<String>,
}

/// OAuth 2.0 token response
#[derive(Debug, Serialize, ToSchema)]
pub struct TokenResponse {
    /// The access token
    #[schema(example = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9...")]
    pub access_token: String,

    /// Token type (always "Bearer")
    #[schema(example = "Bearer")]
    pub token_type: String,

    /// Token expiration in seconds
    #[schema(example = 3600)]
    pub expires_in: i64,
}

/// OAuth 2.0 error response
#[derive(Debug, Serialize, ToSchema)]
pub struct TokenErrorResponse {
    /// Error code
    #[schema(example = "invalid_client")]
    pub error: String,

    /// Human-readable error description
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_description: Option<String>,
}

impl TokenErrorResponse {
    fn invalid_request(description: &str) -> Self {
        Self {
            error: "invalid_request".to_string(),
            error_description: Some(description.to_string()),
        }
    }

    fn invalid_client() -> Self {
        Self {
            error: "invalid_client".to_string(),
            error_description: Some("Client authentication failed".to_string()),
        }
    }

    fn unsupported_grant_type() -> Self {
        Self {
            error: "unsupported_grant_type".to_string(),
            error_description: Some(
                "Only 'client_credentials' grant type is supported".to_string(),
            ),
        }
    }
}

/// Shared state for OAuth endpoints
#[derive(Clone)]
pub struct OAuthState {
    pub client_manager: Arc<ClientManager>,
    pub token_store: Arc<TokenStore>,
}

/// Extract client credentials from Basic Auth header
///
/// Expected format: "Basic base64(client_id:client_secret)"
fn extract_basic_auth(auth_header: Option<&str>) -> Option<(String, String)> {
    let auth_header = auth_header?;

    // Check if it's a Basic auth header
    if !auth_header.starts_with("Basic ") {
        return None;
    }

    // Extract the base64 part
    let encoded = auth_header.strip_prefix("Basic ")?;

    // Decode base64
    let decoded = BASE64.decode(encoded).ok()?;
    let decoded_str = String::from_utf8(decoded).ok()?;

    // Split on first colon
    let mut parts = decoded_str.splitn(2, ':');
    let client_id = parts.next()?.to_string();
    let client_secret = parts.next()?.to_string();

    Some((client_id, client_secret))
}

/// POST /oauth/token - OAuth 2.0 token endpoint
///
/// Implements the client credentials grant flow:
/// 1. Client sends credentials (either in body or Basic Auth)
/// 2. Server verifies credentials
/// 3. Server generates and returns a short-lived access token (1 hour)
///
/// # Request
/// ```json
/// {
///   "grant_type": "client_credentials",
///   "client_id": "lr-abc123",
///   "client_secret": "secret123"
/// }
/// ```
///
/// Or using Basic Authentication:
/// ```
/// POST /oauth/token
/// Authorization: Basic base64(client_id:client_secret)
/// Content-Type: application/x-www-form-urlencoded
///
/// grant_type=client_credentials
/// ```
///
/// # Response
/// ```json
/// {
///   "access_token": "token123",
///   "token_type": "Bearer",
///   "expires_in": 3600
/// }
/// ```
#[utoipa::path(
    post,
    path = "/oauth/token",
    tag = "oauth",
    request_body = TokenRequest,
    responses(
        (status = 200, description = "Access token generated successfully", body = TokenResponse),
        (status = 400, description = "Bad request - invalid grant type or missing credentials", body = TokenErrorResponse),
        (status = 401, description = "Unauthorized - invalid client credentials", body = TokenErrorResponse),
        (status = 500, description = "Internal server error", body = TokenErrorResponse)
    )
)]
pub async fn token_endpoint(
    State(state): State<OAuthState>,
    headers: axum::http::HeaderMap,
    Json(request): Json<TokenRequest>,
) -> Response {
    // Verify grant type
    if request.grant_type != "client_credentials" {
        return (
            StatusCode::BAD_REQUEST,
            Json(TokenErrorResponse::unsupported_grant_type()),
        )
            .into_response();
    }

    // Extract credentials from either body or Basic Auth header
    let (client_id, client_secret) =
        if let (Some(id), Some(secret)) = (request.client_id, request.client_secret) {
            // Credentials in body
            (id, secret)
        } else if let Some(auth) = headers.get("authorization") {
            // Try Basic Auth
            match extract_basic_auth(auth.to_str().ok()) {
                Some((id, secret)) => (id, secret),
                None => {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(TokenErrorResponse::invalid_request(
                            "Invalid Authorization header format",
                        )),
                    )
                        .into_response();
                }
            }
        } else {
            return (
                StatusCode::BAD_REQUEST,
                Json(TokenErrorResponse::invalid_request(
                    "Missing client credentials",
                )),
            )
                .into_response();
        };

    // Verify client credentials
    let client = match state
        .client_manager
        .verify_credentials(&client_id, &client_secret)
    {
        Ok(Some(client)) => client,
        Ok(None) => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(TokenErrorResponse::invalid_client()),
            )
                .into_response();
        }
        Err(e) => {
            tracing::error!("Error verifying client credentials: {}", e);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(TokenErrorResponse::invalid_request("Internal server error")),
            )
                .into_response();
        }
    };

    // Generate access token
    match state.token_store.generate_token(client.id.clone()) {
        Ok((access_token, expires_in)) => {
            tracing::info!("Generated OAuth token for client: {}", client.id);

            (
                StatusCode::OK,
                Json(TokenResponse {
                    access_token,
                    token_type: "Bearer".to_string(),
                    expires_in,
                }),
            )
                .into_response()
        }
        Err(e) => {
            tracing::error!("Error generating access token: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(TokenErrorResponse::invalid_request(
                    "Failed to generate token",
                )),
            )
                .into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_basic_auth() {
        // Valid Basic Auth
        let auth = "Basic bHItYWJjMTIzOnNlY3JldDEyMw=="; // lr-abc123:secret123
        let result = extract_basic_auth(Some(auth));
        assert_eq!(
            result,
            Some(("lr-abc123".to_string(), "secret123".to_string()))
        );

        // Invalid format (not Basic)
        let auth = "Bearer token123";
        let result = extract_basic_auth(Some(auth));
        assert_eq!(result, None);

        // Invalid base64
        let auth = "Basic !!!invalid!!!";
        let result = extract_basic_auth(Some(auth));
        assert_eq!(result, None);

        // Missing colon
        let auth = "Basic bm9jb2xvbg=="; // "nocolon"
        let result = extract_basic_auth(Some(auth));
        assert_eq!(result, None);

        // Empty
        let result = extract_basic_auth(None);
        assert_eq!(result, None);
    }

    #[test]
    fn test_basic_auth_with_colon_in_secret() {
        // client_id:secret:with:colons
        let auth = "Basic bHItYWJjOnNlY3JldDp3aXRoOmNvbG9ucw==";
        let result = extract_basic_auth(Some(auth));
        assert_eq!(
            result,
            Some(("lr-abc".to_string(), "secret:with:colons".to_string()))
        );
    }
}
