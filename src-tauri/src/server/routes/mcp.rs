//! MCP proxy routes
//!
//! Handles proxying JSON-RPC requests from external MCP clients to MCP servers.
//! Route format: POST /mcp/{client_id}/{server_id}

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};

use crate::mcp::protocol::{JsonRpcRequest, JsonRpcResponse};
use crate::server::middleware::error::ApiErrorResponse;
use crate::server::state::{AppState, OAuthContext};

/// MCP proxy handler
///
/// Routes JSON-RPC requests to the appropriate MCP server.
/// Validates that the OAuth client has access to the requested server.
///
/// # Path Parameters
/// * `client_id` - OAuth client ID (from auth context)
/// * `server_id` - MCP server ID to proxy to
///
/// # Request Body
/// JSON-RPC 2.0 request
///
/// # Response
/// JSON-RPC 2.0 response
pub async fn mcp_proxy_handler(
    Path((client_id_param, server_id)): Path<(String, String)>,
    State(state): State<AppState>,
    oauth_context: axum::Extension<OAuthContext>,
    Json(request): Json<JsonRpcRequest>,
) -> Response {
    // Helper function to handle errors
    async fn handle_request(
        client_id_param: String,
        server_id: String,
        state: AppState,
        oauth_context: OAuthContext,
        request: JsonRpcRequest,
    ) -> Result<JsonRpcResponse, ApiErrorResponse> {
        // Verify client_id matches OAuth context (URL param should match authenticated client)
        if client_id_param != oauth_context.client_id {
            tracing::warn!(
                "Client ID mismatch: URL={}, Auth={}",
                client_id_param,
                oauth_context.client_id
            );
            return Err(ApiErrorResponse::forbidden(
                "Client ID does not match authenticated client",
            ));
        }

        // Check if client has access to this server
        if !oauth_context.linked_server_ids.contains(&server_id) {
            tracing::warn!(
                "Client {} attempted to access unauthorized server {}",
                oauth_context.client_id,
                server_id
            );
            return Err(ApiErrorResponse::forbidden(
                "Client does not have access to this MCP server",
            ));
        }

        // Start server if not running
        let mcp_manager = &state.mcp_server_manager;
        if !mcp_manager.is_running(&server_id) {
            tracing::info!("Starting MCP server {} for proxy request", server_id);
            mcp_manager
                .start_server(&server_id)
                .await
                .map_err(|e| {
                    ApiErrorResponse::bad_gateway(format!("Failed to start MCP server: {}", e))
                })?;
        }

        // Forward request to MCP server
        tracing::debug!(
            "Proxying JSON-RPC request to server {}: method={}",
            server_id,
            request.method
        );

        let response = mcp_manager
            .send_request(&server_id, request)
            .await
            .map_err(|e| {
                ApiErrorResponse::bad_gateway(format!("MCP server error: {}", e))
            })?;

        Ok(response)
    }

    // Call the helper and convert result to response
    match handle_request(
        client_id_param,
        server_id,
        state,
        oauth_context.0,
        request,
    )
    .await
    {
        Ok(response) => Json(response).into_response(),
        Err(err) => err.into_response(),
    }
}

/// Health check endpoint for MCP proxy
///
/// Returns 200 OK if the service is running.
pub async fn mcp_health_handler() -> impl IntoResponse {
    (StatusCode::OK, "MCP proxy healthy")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api_keys::ApiKeyManager;
    use crate::mcp::McpServerManager;
    use crate::oauth_clients::OAuthClientManager;
    use crate::providers::health::HealthCheckManager;
    use crate::providers::registry::ProviderRegistry;
    use crate::router::{RateLimiterManager, Router};
    use serde_json::json;
    use std::sync::Arc;

    #[tokio::test]
    async fn test_client_id_mismatch() {
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
        );
        let state_with_oauth = state.with_oauth_and_mcp(
            OAuthClientManager::new(vec![]),
            Arc::new(McpServerManager::new()),
        );

        // Create OAuth context
        let oauth_context = axum::Extension(OAuthContext {
            client_id: "client-123".to_string(),
            linked_server_ids: vec!["server-1".to_string()],
        });

        // Create JSON-RPC request
        let request = JsonRpcRequest::with_id(1, "test_method".to_string(), None);

        // Call handler with mismatched client_id
        let result = handle_request(
            "different-client".to_string(), // Mismatch!
            "server-1".to_string(),
            state_with_oauth,
            oauth_context.0,
            request,
        )
        .await;

        // Should fail with forbidden error
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_unauthorized_server_access() {
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
        );
        let state_with_oauth = state.with_oauth_and_mcp(
            OAuthClientManager::new(vec![]),
            Arc::new(McpServerManager::new()),
        );

        // Create OAuth context with access to server-1 only
        let oauth_context = axum::Extension(OAuthContext {
            client_id: "client-123".to_string(),
            linked_server_ids: vec!["server-1".to_string()],
        });

        // Create JSON-RPC request
        let request = JsonRpcRequest::with_id(1, "test_method".to_string(), None);

        // Try to access server-2 (unauthorized)
        let result = handle_request(
            "client-123".to_string(),
            "server-2".to_string(), // Not in linked_server_ids!
            state_with_oauth,
            oauth_context.0,
            request,
        )
        .await;

        // Should fail with forbidden error
        assert!(result.is_err());
    }
}
