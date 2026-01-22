//! Web server module
//!
//! Provides OpenAI-compatible HTTP API endpoints using Axum.

pub mod manager;
pub mod middleware;
pub mod openapi;
pub mod routes;
pub mod state;
pub mod types;

// Re-export manager types for convenience
pub use manager::{ServerManager, ServerStatus};

use std::net::SocketAddr;
use std::sync::Arc;

use axum::{
    extract::Request,
    http::{header, Method, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    routing::{delete, get, post},
    Router,
};
use tokio::net::TcpListener;
use tower_http::cors::{Any, CorsLayer};
use tracing::{error, info};

use crate::mcp::McpServerManager;
use crate::providers::registry::ProviderRegistry;
use crate::router::{RateLimiterManager, Router as AppRouter};

use self::middleware::auth_layer::AuthLayer;
use self::state::AppState;

/// Web server configuration
#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub enable_cors: bool,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            port: 8080,
            enable_cors: true,
        }
    }
}

/// Start the web server
///
/// This creates an Axum server with unified OpenAI-compatible and MCP endpoints:
/// - POST /v1/chat/completions (OpenAI)
/// - POST /v1/completions (OpenAI)
/// - POST /v1/embeddings (OpenAI)
/// - GET /v1/models (OpenAI)
/// - GET /v1/generation (OpenAI)
/// - POST / (MCP unified gateway)
/// - POST /servers/:server_id (MCP individual server)
///
/// Returns the AppState, JoinHandle, and the actual port used
#[allow(clippy::too_many_arguments)]
pub async fn start_server(
    config: ServerConfig,
    router: Arc<AppRouter>,
    mcp_server_manager: Arc<McpServerManager>,
    rate_limiter: Arc<RateLimiterManager>,
    provider_registry: Arc<ProviderRegistry>,
    config_manager: Arc<crate::config::ConfigManager>,
    client_manager: Arc<crate::clients::ClientManager>,
    token_store: Arc<crate::clients::TokenStore>,
    metrics_collector: Arc<crate::monitoring::metrics::MetricsCollector>,
) -> anyhow::Result<(AppState, tokio::task::JoinHandle<()>, u16)> {
    info!("Starting web server on {}:{}", config.host, config.port);

    // Create shared state
    let state = AppState::new(
        router,
        rate_limiter,
        provider_registry,
        config_manager,
        client_manager,
        token_store,
        metrics_collector,
    )
    .with_mcp(mcp_server_manager);

    // Build the router with auth layer applied
    let app = build_app(state.clone(), config.enable_cors);

    // Try to bind to the configured port, incrementing if necessary
    let host_ip = config.host.parse::<std::net::IpAddr>()?;
    let mut port = config.port;
    let max_attempts = 100; // Try up to 100 ports

    let listener = loop {
        let addr = SocketAddr::from((host_ip, port));

        match TcpListener::bind(addr).await {
            Ok(listener) => {
                if port != config.port {
                    info!(
                        "Port {} was taken, using port {} instead",
                        config.port, port
                    );
                }
                break listener;
            }
            Err(e) => {
                if port - config.port >= max_attempts {
                    return Err(anyhow::anyhow!(
                        "Could not bind to any port between {} and {} (last error: {})",
                        config.port,
                        port,
                        e
                    ));
                }
                tracing::debug!("Port {} is taken, trying next port", port);
                port += 1;
            }
        }
    };

    info!("Web server listening on http://{}:{}", config.host, port);
    info!("OpenAI-compatible endpoints available at:");

    // Clone state to return before starting server (which runs forever)
    let state_clone = state.clone();

    // Spawn gateway session cleanup task (runs every 10 minutes)
    let gateway_for_cleanup = state.mcp_gateway.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(600)); // 10 minutes
        loop {
            interval.tick().await;
            gateway_for_cleanup.cleanup_expired_sessions();
        }
    });

    // Start server (this runs forever)
    let handle = tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            error!("Server error: {}", e);
        }
    });

    Ok((state_clone, handle, port))
}

/// Build the Axum app with all routes and middleware
fn build_app(state: AppState, enable_cors: bool) -> Router {
    // Build MCP routes with client auth middleware
    // MCP routes: unified gateway at root (/), individual servers under /mcp
    let mcp_routes = Router::new()
        .route("/", post(routes::mcp_gateway_handler)) // Unified MCP gateway at root (POST /)
        .route("/mcp/:server_id", post(routes::mcp_server_handler)) // Individual MCP server proxy
        .route(
            "/mcp/:server_id/stream",
            post(routes::mcp_server_streaming_handler), // MCP streaming endpoint (SSE)
        )
        .route("/ws", get(routes::mcp_websocket_handler)) // WebSocket notifications
        .route(
            "/mcp/elicitation/respond/:request_id",
            post(routes::elicitation_response_handler), // Submit elicitation responses
        )
        // SSE streaming gateway endpoints
        .route("/gateway/stream", post(routes::initialize_streaming_session)) // Initialize SSE session
        .route("/gateway/stream/:session_id", get(routes::streaming_event_handler)) // SSE event stream
        .route("/gateway/stream/:session_id/request", post(routes::send_streaming_request)) // Send request
        .route("/gateway/stream/:session_id", delete(routes::close_streaming_session)) // Close session
        .layer(axum::middleware::from_fn(
            middleware::client_auth::client_auth_middleware,
        ))
        .layer(axum::Extension(state.clone()))
        .with_state(state.clone());

    // Build OAuth routes (no auth required - these ARE the auth endpoints)
    let oauth_state = routes::oauth::OAuthState {
        client_manager: state.client_manager.clone(),
        token_store: state.token_store.clone(),
    };

    let oauth_routes = Router::new()
        .route("/oauth/token", post(routes::token_endpoint))
        .with_state(oauth_state);

    // Build the Axum router with all routes
    // Support both /v1 prefix and without for OpenAI compatibility
    let mut router = Router::new()
        .route("/health", get(health_check))
        .route("/", get(root_handler))
        // OpenAPI specification endpoints
        .route("/openapi.json", get(serve_openapi_json))
        .route("/openapi.yaml", get(serve_openapi_yaml))
        // Routes with /v1 prefix
        .route("/v1/chat/completions", post(routes::chat_completions))
        .route("/v1/completions", post(routes::completions))
        .route("/v1/embeddings", post(routes::embeddings))
        .route("/v1/models", get(routes::list_models))
        .route("/v1/models/:id", get(routes::get_model))
        .route(
            "/v1/models/:provider/:model/pricing",
            get(routes::get_model_pricing),
        )
        .route("/v1/generation", get(routes::get_generation))
        // Routes without /v1 prefix (for compatibility)
        .route("/chat/completions", post(routes::chat_completions))
        .route("/completions", post(routes::completions))
        .route("/embeddings", post(routes::embeddings))
        .route("/models", get(routes::list_models))
        .route("/models/:id", get(routes::get_model))
        .route(
            "/models/:provider/:model/pricing",
            get(routes::get_model_pricing),
        )
        .route("/generation", get(routes::get_generation))
        .with_state(state.clone());

    // Apply auth layer (checks all API routes with or without /v1 prefix)
    router = router.layer(AuthLayer::new(state.clone()));

    // Merge OAuth routes (no auth required - these ARE the auth endpoints)
    router = router.merge(oauth_routes);

    // Merge MCP routes (these use OAuth auth, not API key auth)
    router = router.merge(mcp_routes);

    // Add logging middleware
    router = router.layer(axum::middleware::from_fn(logging_middleware));

    // Add CORS if enabled
    if enable_cors {
        let cors = CorsLayer::new()
            .allow_origin(Any)
            .allow_methods([
                Method::GET,
                Method::POST,
                Method::PUT,
                Method::DELETE,
                Method::PATCH,
                Method::OPTIONS,
                Method::HEAD,
            ])
            .allow_headers(Any) // Allow all headers for OpenAI SDK compatibility
            .expose_headers(Any) // Expose all headers in responses
            .allow_credentials(false);

        router = router.layer(cors);
    }

    router
}

/// Health check endpoint
#[utoipa::path(
    get,
    path = "/health",
    tag = "system",
    responses(
        (status = 200, description = "Server is healthy")
    )
)]
async fn health_check() -> StatusCode {
    StatusCode::OK
}

/// Root handler
#[utoipa::path(
    get,
    path = "/",
    tag = "system",
    responses(
        (status = 200, description = "API information", content_type = "text/plain")
    )
)]
async fn root_handler() -> &'static str {
    "LocalRouter AI - Unified OpenAI & MCP API Gateway\n\
     \n\
     OpenAI Endpoints (both /v1 prefix and without are supported):\n\
       POST /v1/chat/completions or /chat/completions\n\
       POST /v1/completions or /completions\n\
       POST /v1/embeddings or /embeddings\n\
       GET  /v1/models or /models\n\
       GET  /v1/models/{id} or /models/{id}\n\
       GET  /v1/models/{provider}/{model}/pricing or /models/{provider}/{model}/pricing\n\
       GET  /v1/generation?id={id} or /generation?id={id}\n\
     \n\
     MCP Endpoints:\n\
       POST /                       - Unified MCP gateway (all servers)\n\
       POST /mcp/{server_id}        - Individual MCP server proxy\n\
       POST /mcp/{server_id}/stream - Streaming MCP endpoint (SSE)\n\
       GET  /ws                     - WebSocket real-time notifications\n\
     \n\
     Documentation:\n\
       GET  /openapi.json - OpenAPI specification (JSON)\n\
       GET  /openapi.yaml - OpenAPI specification (YAML)\n\
     \n\
     Authentication: Include 'Authorization: Bearer <your-token>' header\n"
}

/// Serve OpenAPI specification as JSON
///
/// Returns the complete OpenAPI 3.1 specification in JSON format.
/// This can be used with tools like Swagger UI, Postman, or code generators.
#[utoipa::path(
    get,
    path = "/openapi.json",
    tag = "system",
    responses(
        (status = 200, description = "OpenAPI specification in JSON format", content_type = "application/json"),
        (status = 500, description = "Failed to generate specification")
    )
)]
async fn serve_openapi_json() -> impl IntoResponse {
    match openapi::get_openapi_json() {
        Ok(json) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "application/json")],
            json,
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to generate OpenAPI spec: {}", e),
        )
            .into_response(),
    }
}

/// Serve OpenAPI specification as YAML
///
/// Returns the complete OpenAPI 3.1 specification in YAML format.
/// This can be used with tools like Swagger UI, Redoc, or for documentation.
#[utoipa::path(
    get,
    path = "/openapi.yaml",
    tag = "system",
    responses(
        (status = 200, description = "OpenAPI specification in YAML format", content_type = "application/yaml"),
        (status = 500, description = "Failed to generate specification")
    )
)]
async fn serve_openapi_yaml() -> impl IntoResponse {
    match openapi::get_openapi_yaml() {
        Ok(yaml) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "application/yaml")],
            yaml,
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to generate OpenAPI spec: {}", e),
        )
            .into_response(),
    }
}

/// Logging middleware to log all requests
async fn logging_middleware(req: Request, next: Next) -> Response {
    let method = req.method().clone();
    let uri = req.uri().clone();
    let start = std::time::Instant::now();

    let response = next.run(req).await;

    let elapsed = start.elapsed();
    let status = response.status();

    if status.is_success() {
        info!("{} {} - {} ({:?})", method, uri, status, elapsed);
    } else {
        error!("{} {} - {} ({:?})", method, uri, status, elapsed);
    }

    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_server_config_default() {
        let config = ServerConfig::default();
        assert_eq!(config.host, "127.0.0.1");
        assert_eq!(config.port, 8080);
        assert!(config.enable_cors);
    }
}
