//! Tower-based authentication layer for Tauri HTTP compatibility

use axum::body::Body;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use tauri::http::{Request, Response, StatusCode};
use tower::{Layer, Service};

use crate::middleware::client_auth::{ClientAuthContext, LoggedClientId};
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
    S::Error: Into<Box<dyn std::error::Error + Send + Sync>> + Send + 'static,
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
        // Call the instance whose readiness was polled. A clone may have its
        // own readiness state (for example a concurrency-limit permit).
        let replacement = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, replacement);

        Box::pin(async move {
            let native = matches!(
                req.uri().path(),
                "/v1/messages" | "/v1/messages/count_tokens"
            );
            let result:Result<Response<Body>,S::Error>=async move {
            // Skip authentication for OPTIONS requests (CORS preflight)
            if req.method() == tauri::http::Method::OPTIONS {
                return inner.call(req).await;
            }

            // Anthropic SDKs use x-api-key. Normalize it into the existing
            // client-secret validator only on native Messages routes; this is
            // never an upstream provider credential.
            if matches!(
                req.uri().path(),
                "/v1/messages" | "/v1/messages/count_tokens"
            ) {
                if req.headers().get_all("x-api-key").iter().count() > 1
                    || req.headers().get_all("authorization").iter().count() > 1
                {
                    return Ok(create_error_response(
                        StatusCode::UNAUTHORIZED,
                        "authentication_error",
                        "Repeated authentication headers",
                    ));
                }
                if !req.headers().contains_key("authorization") {
                    if let Some(key) = req.headers().get("x-api-key") {
                        let bearer = key
                            .to_str()
                            .ok()
                            .filter(|v| {
                                !v.is_empty() && !v.bytes().any(|b| b.is_ascii_whitespace())
                            })
                            .and_then(|v| {
                                tauri::http::HeaderValue::from_str(&format!("Bearer {v}")).ok()
                            });
                        let Some(bearer) = bearer else {
                            return Ok(create_error_response(
                                StatusCode::UNAUTHORIZED,
                                "authentication_error",
                                "Invalid x-api-key header",
                            ));
                        };
                        req.headers_mut()
                            .insert(tauri::http::header::AUTHORIZATION, bearer);
                    }
                }
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
                || path == "/responses"
                || path == "/search" || path == "/ocr" || path == "/rerank" || path == "/videos" || path.starts_with("/videos/") || path == "/embeddings"
                || path == "/moderations"
                || path == "/systemone"
                || path == "/decisions"
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
                        state.monitor_store.push(
                            lr_monitor::MonitorEventType::AuthError,
                            None,
                            None,
                            None,
                            lr_monitor::MonitorEventData::AuthError {
                                error_type: "invalid_encoding".to_string(),
                                endpoint: path.to_string(),
                                message: "Invalid Authorization header encoding".to_string(),
                                status_code: 401,
                            },
                            lr_monitor::EventStatus::Error,
                            None,
                        );
                        return Ok(create_error_response(
                            StatusCode::UNAUTHORIZED,
                            "authentication_error",
                            "Invalid Authorization header encoding",
                        ));
                    }
                },
                None => {
                    state.monitor_store.push(
                        lr_monitor::MonitorEventType::AuthError,
                        None,
                        None,
                        None,
                        lr_monitor::MonitorEventData::AuthError {
                            error_type: "missing_header".to_string(),
                            endpoint: path.to_string(),
                            message: "Missing Authorization header".to_string(),
                            status_code: 401,
                        },
                        lr_monitor::EventStatus::Error,
                        None,
                    );
                    return Ok(create_error_response(
                        StatusCode::UNAUTHORIZED,
                        "authentication_error",
                        "Missing Authorization header",
                    ));
                }
            };

            // Check if it starts with "Bearer "
            if !auth_header.starts_with("Bearer ") {
                state.monitor_store.push(
                    lr_monitor::MonitorEventType::AuthError,
                    None,
                    None,
                    None,
                    lr_monitor::MonitorEventData::AuthError {
                        error_type: "invalid_format".to_string(),
                        endpoint: path.to_string(),
                        message: "Invalid Authorization header format".to_string(),
                        status_code: 401,
                    },
                    lr_monitor::EventStatus::Error,
                    None,
                );
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
                tracing::debug!("internal-test token detected - bypassing API key restrictions");
                let auth_context = AuthContext {
                    credential_scope: None,
                    api_key_id: "internal-test".to_string(),
                    model_selection: None,
                };
                req.extensions_mut().insert(auth_context);
                req.extensions_mut().insert(ClientAuthContext {
                    client_id: "internal-test".to_string(),
                });
                let mut resp = inner.call(req).await?;
                resp.extensions_mut()
                    .insert(LoggedClientId("internal-test".to_string()));
                return Ok(resp);
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

                        let credential_scope = lr_router::explanation::fingerprint(
                            serde_json::json!({"process":state.internal_test_secret.as_str(),"credential":bearer_token}),
                        );
                        // Also inject ClientAuthContext for guardrails and other client-aware features
                        req.extensions_mut().insert(ClientAuthContext {
                            client_id: client.id.clone(),
                        });

                        // Use the client's ID for routing
                        AuthContext {
                            credential_scope: Some(credential_scope),
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
                        state.monitor_store.push(
                            lr_monitor::MonitorEventType::AuthError,
                            None,
                            None,
                            None,
                            lr_monitor::MonitorEventData::AuthError {
                                error_type: "invalid_key".to_string(),
                                endpoint: path.to_string(),
                                message: "Invalid API key".to_string(),
                                status_code: 401,
                            },
                            lr_monitor::EventStatus::Error,
                            None,
                        );
                        return Ok(create_error_response(
                            StatusCode::UNAUTHORIZED,
                            "authentication_error",
                            "Invalid API key",
                        ));
                    }
                    Err(e) => {
                        tracing::error!("Error verifying client secret: {}", e);
                        state.monitor_store.push(
                            lr_monitor::MonitorEventType::AuthError,
                            None,
                            None,
                            None,
                            lr_monitor::MonitorEventData::AuthError {
                                error_type: "verification_error".to_string(),
                                endpoint: path.to_string(),
                                message: format!("Authentication error: {}", e),
                                status_code: 500,
                            },
                            lr_monitor::EventStatus::Error,
                            None,
                        );
                        return Ok(create_error_response(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            "internal_error",
                            "Authentication error",
                        ));
                    }
                }
            };

            // Insert auth context into request extensions
            let logged_id = auth_context.api_key_id.clone();
            if let Err(error)=crate::privacy::check_ingress(&state,&auth_context.api_key_id,req.uri().path()) {return Ok(axum::response::IntoResponse::into_response(error));}
            req.extensions_mut().insert(auth_context);

            // Call the inner service
            let mut resp = inner.call(req).await?;
            resp.extensions_mut().insert(LoggedClientId(logged_id));
            Ok(resp)
            }.await;
            let response = match result {
                Ok(response) => response,
                Err(error) => return Err(error),
            };
            if native
                && (response.status().is_client_error() || response.status().is_server_error())
            {
                Ok(crate::routes::messages::native_error_response(response).await)
            } else {
                Ok(response)
            }
        })
    }
}

/// Helper function to create an error response
fn create_error_response(status: StatusCode, error_type: &str, message: &str) -> Response<Body> {
    let error = ErrorResponse {
        extensions: None,
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
