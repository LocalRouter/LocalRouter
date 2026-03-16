//! Tower-based authentication layer for Tauri HTTP compatibility

use axum::body::Body;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use tauri::http::{Request, Response, StatusCode};
use tower::{Layer, Service};

use crate::middleware::client_auth::ClientAuthContext;
use crate::state::{AppState, AuthContext};
use crate::types::{ApiError, ErrorResponse};

/// Authentication layer that validates API keys
#[derive(Clone)]
pub struct AuthLayer {
    state: AppState,
}

impl AuthLayer {
    pub fn new(state: AppState) -> Self {
        Self { state }
    }
}

impl<S> Layer<S> for AuthLayer {
    type Service = AuthService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        AuthService {
            inner,
            state: self.state.clone(),
        }
    }
}

/// Authentication service that performs API key validation
#[derive(Clone)]
pub struct AuthService<S> {
    inner: S,
    state: AppState,
}

impl<S> Service<Request<Body>> for AuthService<S>
where
    S: Service<Request<Body>, Response = Response<Body>> + Clone + Send + 'static,
    S::Future: Send + 'static,
    S::Error: Into<Box<dyn std::error::Error + Send + Sync>> + 'static,
{
    type Response = Response<Body>;
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    #[allow(deprecated)]
    fn call(&mut self, mut req: Request<Body>) -> Self::Future {
        let state = self.state.clone();
        let mut inner = self.inner.clone();

        Box::pin(async move {
            // Skip authentication for OPTIONS requests (CORS preflight)
            if req.method() == tauri::http::Method::OPTIONS {
                return inner.call(req).await;
            }

            // Check if this is a protected route
            let path = req.uri().path();

            // Protected API routes require Bearer token authentication.
            //
            // All /v1/* routes are protected by the blanket prefix check below.
            // Non-prefixed routes must be listed explicitly because the root namespace
            // is shared with MCP (/mcp/*), OAuth (/oauth/*), and public routes (/health,
            // /openapi.*), each with their own auth logic.
            //
            // IMPORTANT: When adding a new non-prefixed API route in lib.rs, add a
            // corresponding entry here. The test `test_all_api_routes_require_auth`
            // will catch any omissions.
            let is_protected = path.starts_with("/v1/")
                || path == "/chat/completions"
                || path == "/completions"
                || path == "/embeddings"
                || path == "/moderations"
                || path == "/models"
                || path.starts_with("/models/")
                || path.starts_with("/images/")
                || path.starts_with("/audio/")
                || path.starts_with("/generation");

            if !is_protected {
                // Public route - skip authentication
                return inner.call(req).await;
            }

            // Extract Authorization header
            let auth_header = match req.headers().get("Authorization") {
                Some(header) => match header.to_str() {
                    Ok(s) => s,
                    Err(_) => {
                        return Ok(create_error_response(
                            StatusCode::UNAUTHORIZED,
                            "authentication_error",
                            "Invalid Authorization header encoding",
                        ));
                    }
                },
                None => {
                    return Ok(create_error_response(
                        StatusCode::UNAUTHORIZED,
                        "authentication_error",
                        "Missing Authorization header",
                    ));
                }
            };

            // Check if it starts with "Bearer "
            if !auth_header.starts_with("Bearer ") {
                return Ok(create_error_response(
                    StatusCode::UNAUTHORIZED,
                    "authentication_error",
                    "Invalid Authorization header format",
                ));
            }

            // Extract the bearer token (API key or client secret)
            let bearer_token = &auth_header[7..]; // Skip "Bearer "

            // Check if this is the internal test token or memory service token
            let is_internal = bearer_token == state.internal_test_secret.as_str();
            let is_memory = bearer_token == state.memory_secret.as_str();
            tracing::debug!(
                "Auth check: token_len={}, is_internal={}, is_memory={}, memory_secret_prefix={}",
                bearer_token.len(),
                is_internal,
                is_memory,
                &state.memory_secret[..12.min(state.memory_secret.len())],
            );
            if is_internal || is_memory {
                let client_id = if is_memory { "memory-service" } else { "internal-test" };
                tracing::debug!("{} token detected - bypassing API key restrictions", client_id);
                let auth_context = AuthContext {
                    api_key_id: client_id.to_string(),
                    model_selection: None,
                };
                req.extensions_mut().insert(auth_context);
                req.extensions_mut().insert(ClientAuthContext {
                    client_id: client_id.to_string(),
                });
                return inner.call(req).await;
            }

            // Validate bearer token using client manager
            let auth_context = {
                tracing::debug!("Authenticating bearer token with client manager");
                match state.client_manager.verify_secret(bearer_token) {
                    Ok(Some(client)) => {
                        tracing::info!(
                            event = "auth_success",
                            client_id = %client.id,
                            method = "client_secret",
                            "Client authenticated"
                        );

                        // Also inject ClientAuthContext for guardrails and other client-aware features
                        req.extensions_mut().insert(ClientAuthContext {
                            client_id: client.id.clone(),
                        });

                        // Use the client's ID for routing
                        AuthContext {
                            api_key_id: client.id.clone(),
                            model_selection: None,
                        }
                    }
                    Ok(None) => {
                        tracing::warn!(
                            event = "auth_failed",
                            reason = "invalid_token",
                            "Authentication failed: invalid bearer token"
                        );
                        return Ok(create_error_response(
                            StatusCode::UNAUTHORIZED,
                            "authentication_error",
                            "Invalid API key",
                        ));
                    }
                    Err(e) => {
                        tracing::error!("Error verifying client secret: {}", e);
                        return Ok(create_error_response(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            "internal_error",
                            "Authentication error",
                        ));
                    }
                }
            };

            // Insert auth context into request extensions
            req.extensions_mut().insert(auth_context);

            // Call the inner service
            inner.call(req).await
        })
    }
}

/// Helper function to create an error response
fn create_error_response(status: StatusCode, error_type: &str, message: &str) -> Response<Body> {
    let error = ErrorResponse {
        error: ApiError {
            message: message.to_string(),
            error_type: error_type.to_string(),
            param: None,
            code: None,
        },
    };

    let body = match serde_json::to_vec(&error) {
        Ok(json) => Body::from(json),
        Err(_) => Body::from(r#"{"error":{"message":"Internal error","type":"internal_error"}}"#),
    };

    Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .body(body)
        .unwrap()
}
