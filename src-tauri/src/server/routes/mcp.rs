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
use std::time::Instant;

use crate::mcp::protocol::{JsonRpcRequest, JsonRpcResponse};
use crate::monitoring::mcp_metrics::McpRequestMetrics;
use crate::server::middleware::error::ApiErrorResponse;
use crate::server::middleware::client_auth::ClientAuthContext;
use crate::server::state::{AppState, OAuthContext};

/// Handle MCP request with validation
///
/// Validates client/OAuth context and forwards request to MCP server.
/// Supports both new ClientAuthContext and legacy OAuthContext.
async fn handle_request(
    client_id_param: String,
    server_id: String,
    state: AppState,
    client_context: Option<ClientAuthContext>,
    oauth_context: Option<OAuthContext>,
    request: JsonRpcRequest,
) -> Result<JsonRpcResponse, ApiErrorResponse> {
    // Start timing for metrics
    let start_time = Instant::now();
    let method = request.method.clone();

    // Determine which authentication method is being used and validate access
    if let Some(client_ctx) = client_context {
        // New unified client authentication
        // Verify client_id matches (URL param should match authenticated client)
        if client_id_param != client_ctx.client_id {
            tracing::warn!(
                "Client ID mismatch: URL={}, Auth={}",
                client_id_param,
                client_ctx.client_id
            );
            return Err(ApiErrorResponse::forbidden(
                "Client ID does not match authenticated client",
            ));
        }

        // Get client to check allowed MCP servers
        let client = state
            .client_manager
            .get_client(&client_ctx.client_id)
            .ok_or_else(|| ApiErrorResponse::unauthorized("Client not found"))?;

        // Check if client is enabled
        if !client.enabled {
            return Err(ApiErrorResponse::forbidden("Client is disabled"));
        }

        // Check if client has access to this MCP server
        if !client.allowed_mcp_servers.contains(&server_id) {
            tracing::warn!(
                "Client {} attempted to access unauthorized MCP server {}",
                client_ctx.client_id,
                server_id
            );
            return Err(ApiErrorResponse::forbidden(format!(
                "Access denied: Client is not authorized to access MCP server '{}'. Contact administrator to grant access.",
                server_id
            )));
        }

        tracing::debug!(
            "Client {} authorized for MCP server: {}",
            client_ctx.client_id,
            server_id
        );
    } else if let Some(oauth_ctx) = oauth_context {
        // Legacy OAuth authentication
        // Verify client_id matches OAuth context (URL param should match authenticated client)
        if client_id_param != oauth_ctx.client_id {
            tracing::warn!(
                "Client ID mismatch: URL={}, Auth={}",
                client_id_param,
                oauth_ctx.client_id
            );
            return Err(ApiErrorResponse::forbidden(
                "Client ID does not match authenticated client",
            ));
        }

        // Check if client has access to this server (legacy linked_server_ids)
        if !oauth_ctx.linked_server_ids.contains(&server_id) {
            tracing::warn!(
                "Client {} attempted to access unauthorized server {}",
                oauth_ctx.client_id,
                server_id
            );
            return Err(ApiErrorResponse::forbidden(
                "Client does not have access to this MCP server",
            ));
        }
    } else {
        // No authentication context provided
        return Err(ApiErrorResponse::unauthorized("Missing authentication context"));
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

    // Record metrics
    let latency_ms = start_time.elapsed().as_millis() as u64;
    state.metrics_collector.mcp().record(&McpRequestMetrics {
        client_id: &client_id_param,
        server_id: &server_id,
        method: &method,
        latency_ms,
        success: response.error.is_none(),
        error_code: response.error.as_ref().map(|e| e.code),
    });

    // Determine transport type
    let transport = "unknown"; // TODO: Add transport detection to MCP manager

    // Log to MCP access log (persistent storage)
    let request_id = format!("mcp_{}", uuid::Uuid::new_v4());
    if response.error.is_none() {
        if let Err(e) = state.mcp_access_logger.log_success(
            &client_id_param,
            &server_id,
            &method,
            latency_ms,
            transport,
            &request_id,
        ) {
            tracing::warn!("Failed to write MCP access log: {}", e);
        }
    } else if let Err(e) = state.mcp_access_logger.log_failure(
        &client_id_param,
        &server_id,
        &method,
        500, // Internal Server Error
        response.error.as_ref().map(|e| e.code),
        latency_ms,
        transport,
        &request_id,
    ) {
        tracing::warn!("Failed to write MCP access log: {}", e);
    }

    Ok(response)
}

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
#[utoipa::path(
    post,
    path = "/mcp/{client_id}/{server_id}",
    tag = "mcp",
    params(
        ("client_id" = String, Path, description = "OAuth client ID"),
        ("server_id" = String, Path, description = "MCP server ID")
    ),
    request_body = crate::mcp::protocol::JsonRpcRequest,
    responses(
        (status = 200, description = "JSON-RPC response", body = crate::mcp::protocol::JsonRpcResponse),
        (status = 401, description = "Unauthorized", body = crate::server::types::ErrorResponse),
        (status = 403, description = "Forbidden - no access to server", body = crate::server::types::ErrorResponse),
        (status = 502, description = "Bad gateway - MCP server error", body = crate::server::types::ErrorResponse),
        (status = 500, description = "Internal server error", body = crate::server::types::ErrorResponse)
    ),
    security(
        ("oauth2" = [])
    )
)]
pub async fn mcp_proxy_handler(
    Path((client_id_param, server_id)): Path<(String, String)>,
    State(state): State<AppState>,
    client_auth: Option<axum::Extension<ClientAuthContext>>,
    oauth_context: Option<axum::Extension<OAuthContext>>,
    Json(request): Json<JsonRpcRequest>,
) -> Response {

    // Call the helper and convert result to response
    match handle_request(
        client_id_param,
        server_id,
        state,
        client_auth.map(|e| e.0),
        oauth_context.map(|e| e.0),
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
#[utoipa::path(
    get,
    path = "/mcp/health",
    tag = "mcp",
    responses(
        (status = 200, description = "MCP proxy is healthy", content_type = "text/plain")
    )
)]
pub async fn mcp_health_handler() -> impl IntoResponse {
    (StatusCode::OK, "MCP proxy healthy")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clients::{ClientManager, TokenStore};
    use crate::mcp::McpServerManager;
    use crate::providers::health::HealthCheckManager;
    use crate::providers::registry::ProviderRegistry;
    use crate::router::{RateLimiterManager, Router};
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
        let rate_limiter = Arc::new(RateLimiterManager::new(None));
        let client_manager = Arc::new(ClientManager::new(vec![]));
        let token_store = Arc::new(TokenStore::new());

        let state = AppState::new(
            router,
            rate_limiter,
            provider_registry,
            config_manager.clone(),
            client_manager,
            token_store,
        ).with_mcp(Arc::new(McpServerManager::new()));

        let state_with_oauth = state;

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
            None, // No ClientAuthContext
            Some(oauth_context.0), // OAuth context
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
        let rate_limiter = Arc::new(RateLimiterManager::new(None));
        let client_manager = Arc::new(ClientManager::new(vec![]));
        let token_store = Arc::new(TokenStore::new());

        let state = AppState::new(
            router,
            rate_limiter,
            provider_registry,
            config_manager.clone(),
            client_manager,
            token_store,
        ).with_mcp(Arc::new(McpServerManager::new()));

        let state_with_oauth = state;

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
            None, // No ClientAuthContext
            Some(oauth_context.0), // OAuth context
            request,
        )
        .await;

        // Should fail with forbidden error
        assert!(result.is_err());
    }
}
