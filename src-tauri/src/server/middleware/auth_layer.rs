//! Tower-based authentication layer for Tauri HTTP compatibility

use axum::body::Body;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use tauri::http::{Request, Response, StatusCode};
use tower::{Layer, Service};

use crate::server::state::{AppState, AuthContext};
use crate::server::types::{ApiError, ErrorResponse};

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

            // Protected routes (with or without /v1 prefix)
            let is_protected = path.starts_with("/v1/")
                || path == "/chat/completions"
                || path == "/completions"
                || path == "/embeddings"
                || path == "/models"
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

            // Check if this is the internal test token
            if bearer_token == state.internal_test_secret.as_str() {
                tracing::debug!(
                    "Internal test token detected - bypassing API key restrictions for UI testing"
                );
                let auth_context = AuthContext {
                    api_key_id: "internal-test".to_string(),
                    model_selection: None,
                    routing_config: None,
                };
                req.extensions_mut().insert(auth_context);
                return inner.call(req).await;
            }

            // Validate bearer token using client manager
            let auth_context = {
                tracing::debug!("Authenticating bearer token with client manager");
                match state.client_manager.verify_secret(bearer_token) {
                    Ok(Some(client)) => {
                        tracing::debug!("Authenticated as client: {}", client.id);

                        // Load routing config from config manager
                        let routing_config = {
                            let config = state.config_manager.get();
                            config
                                .clients
                                .iter()
                                .find(|c| c.id == client.id)
                                .and_then(|c| c.routing_config.clone())
                        };

                        tracing::debug!(
                            "Client {} routing config: {:?}",
                            client.id,
                            routing_config.as_ref().map(|c| &c.active_strategy)
                        );

                        // Use the client's ID for routing with routing config
                        AuthContext {
                            api_key_id: client.id.clone(),
                            model_selection: None,
                            routing_config,
                        }
                    }
                    Ok(None) => {
                        tracing::warn!("Invalid bearer token - client not found");
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
