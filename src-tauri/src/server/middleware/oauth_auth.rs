//! OAuth authentication middleware for MCP proxy
//!
//! Validates OAuth 2.0 Client Credentials (client_id + client_secret)
//! and checks access to requested MCP servers.

use axum::{
    extract::Request,
    middleware::Next,
    response::{IntoResponse, Response},
};

use crate::server::middleware::error::ApiErrorResponse;
use crate::server::state::{AppState, OAuthContext};

/// OAuth authentication middleware for MCP proxy routes
///
/// Extracts and validates OAuth credentials from Authorization header.
/// Expected format: "Authorization: Basic {base64(client_id:client_secret)}"
///
/// On success, attaches OAuthContext to request extensions.
pub async fn oauth_auth_middleware(
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

        // Validate OAuth credentials
        let (client_id, linked_server_ids, enabled) = {
            let oauth_manager = state.oauth_client_manager.read();
            let client_config = oauth_manager
                .verify_credentials(auth_header)
                .ok_or_else(|| ApiErrorResponse::unauthorized("Invalid OAuth credentials"))?;

            // Extract data before dropping the lock
            (
                client_config.id.clone(),
                client_config.linked_server_ids.clone(),
                client_config.enabled,
            )
        }; // Lock is dropped here

        // Double-check enabled status (verify_credentials already checks, but be explicit)
        if !enabled {
            return Err(ApiErrorResponse::unauthorized("OAuth client is disabled"));
        }

        // Create OAuth context
        let oauth_context = OAuthContext {
            client_id,
            linked_server_ids,
        };

        // Insert OAuth context into request extensions
        req.extensions_mut().insert(oauth_context);

        Ok(next.run(req).await)
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
    use axum::{body::Body, http::Request as AxumRequest, middleware};
    use base64::{engine::general_purpose::STANDARD, Engine};
    use std::sync::Arc;
    use tower::ServiceExt;

    async fn test_handler() -> &'static str {
        "OK"
    }

    #[tokio::test]
    async fn test_missing_authorization_header() {
        use crate::api_keys::ApiKeyManager;
        use crate::mcp::McpServerManager;
        use crate::oauth_clients::OAuthClientManager;
        use crate::providers::registry::ProviderRegistry;
        use crate::providers::health::HealthCheckManager;
        use crate::router::{Router, RateLimiterManager};
        use crate::clients::{ClientManager, TokenStore};

        // Create test state
        let health_manager = Arc::new(HealthCheckManager::default());
        let provider_registry = Arc::new(ProviderRegistry::new(health_manager));
        let config_manager = Arc::new(crate::config::ConfigManager::new(
            crate::config::AppConfig::default(),
            std::path::PathBuf::from("/tmp/test_config.yaml"),
        ));
        let router = Arc::new(Router::new(
            config_manager.clone(),
            provider_registry.clone(),
            Arc::new(RateLimiterManager::new(None)),
        ));
        let state = AppState::new(
            router,
            ApiKeyManager::new(vec![]),
            Arc::new(RateLimiterManager::new(None)),
            provider_registry,
            Arc::new(ClientManager::new(vec![])),
            Arc::new(TokenStore::new()),
        );
        let state_with_oauth = state.with_oauth_and_mcp(
            OAuthClientManager::new(vec![]),
            Arc::new(McpServerManager::new()),
        );

        // Create app with middleware
        let app = axum::Router::new()
            .route("/", axum::routing::get(test_handler))
            .layer(middleware::from_fn(oauth_auth_middleware))
            .layer(axum::Extension(state_with_oauth));

        // Make request without Authorization header
        let request = AxumRequest::builder()
            .uri("/")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();

        // Should return 401 Unauthorized
        assert_eq!(response.status(), 401);
    }

    #[tokio::test]
    async fn test_invalid_credentials() {
        use crate::api_keys::ApiKeyManager;
        use crate::mcp::McpServerManager;
        use crate::oauth_clients::OAuthClientManager;
        use crate::providers::registry::ProviderRegistry;
        use crate::providers::health::HealthCheckManager;
        use crate::router::{Router, RateLimiterManager};
        use crate::clients::{ClientManager, TokenStore};

        // Create test state with OAuth manager
        let health_manager = Arc::new(HealthCheckManager::default());
        let provider_registry = Arc::new(ProviderRegistry::new(health_manager));
        let config_manager = Arc::new(crate::config::ConfigManager::new(
            crate::config::AppConfig::default(),
            std::path::PathBuf::from("/tmp/test_config.yaml"),
        ));
        let router = Arc::new(Router::new(
            config_manager.clone(),
            provider_registry.clone(),
            Arc::new(RateLimiterManager::new(None)),
        ));
        let state = AppState::new(
            router,
            ApiKeyManager::new(vec![]),
            Arc::new(RateLimiterManager::new(None)),
            provider_registry,
            Arc::new(ClientManager::new(vec![])),
            Arc::new(TokenStore::new()),
        );
        let state_with_oauth = state.with_oauth_and_mcp(
            OAuthClientManager::new(vec![]),
            Arc::new(McpServerManager::new()),
        );

        // Create app with middleware
        let app = axum::Router::new()
            .route("/", axum::routing::get(test_handler))
            .layer(middleware::from_fn(oauth_auth_middleware))
            .layer(axum::Extension(state_with_oauth));

        // Create invalid Basic Auth header
        let credentials = "invalid-id:invalid-secret";
        let encoded = STANDARD.encode(credentials.as_bytes());
        let auth_header = format!("Basic {}", encoded);

        let request = AxumRequest::builder()
            .uri("/")
            .header("Authorization", auth_header)
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();

        // Should return 401 Unauthorized
        assert_eq!(response.status(), 401);
    }
}
