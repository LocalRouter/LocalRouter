//! Tower-based authentication layer for Tauri HTTP compatibility

use std::task::{Context, Poll};
use std::future::Future;
use std::pin::Pin;
use tower::{Layer, Service};
use tauri::http::{Request, Response, StatusCode};
use axum::body::Body;

use crate::config::ModelSelection;
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

            // Extract the API key
            let api_key = &auth_header[7..]; // Skip "Bearer "

            // Validate the API key and extract needed data
            let auth_context = {
                let api_key_manager = state.api_key_manager.read();
                let api_key_info = match api_key_manager.verify_key(api_key) {
                    Some(info) => info,
                    None => {
                        return Ok(create_error_response(
                            StatusCode::UNAUTHORIZED,
                            "authentication_error",
                            "Invalid API key",
                        ));
                    }
                };

                // Parse model selection and clone data before lock is released
                let model_selection = api_key_info.model_selection.as_ref().map(|sel| match sel {
                    ModelSelection::All => crate::server::state::ModelSelection::All,
                    ModelSelection::Custom {
                        all_provider_models,
                        individual_models,
                    } => crate::server::state::ModelSelection::Custom {
                        all_provider_models: all_provider_models.clone(),
                        individual_models: individual_models.clone(),
                    },
                    #[allow(deprecated)]
                    ModelSelection::DirectModel { provider, model } => {
                        crate::server::state::ModelSelection::DirectModel {
                            provider: provider.clone(),
                            model: model.clone(),
                        }
                    }
                    #[allow(deprecated)]
                    ModelSelection::Router { router_name } => {
                        crate::server::state::ModelSelection::Router {
                            router_name: router_name.clone(),
                        }
                    }
                });

                // Get routing config
                let routing_config = api_key_info.get_routing_config();

                // Create auth context with cloned data
                AuthContext {
                    api_key_id: api_key_info.id.clone(),
                    model_selection,
                    routing_config,
                }
            }; // Lock is automatically dropped here

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
