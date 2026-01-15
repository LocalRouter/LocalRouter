//! Web server module
//!
//! Provides OpenAI-compatible HTTP API endpoints using Axum.

pub mod middleware;
pub mod routes;
pub mod state;
pub mod types;

use std::net::SocketAddr;
use std::sync::Arc;

use axum::{
    extract::Request,
    http::{header, Method, StatusCode},
    middleware::Next,
    response::Response,
    routing::{get, post},
    Router,
};
use tokio::net::TcpListener;
use tower_http::cors::{Any, CorsLayer};
use tracing::{error, info};

use crate::api_keys::ApiKeyManager;
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
/// This creates an Axum server with all OpenAI-compatible endpoints:
/// - POST /v1/chat/completions
/// - POST /v1/completions
/// - POST /v1/embeddings
/// - GET /v1/models
/// - GET /v1/generation
pub async fn start_server(
    config: ServerConfig,
    router: Arc<AppRouter>,
    api_key_manager: ApiKeyManager,
    rate_limiter: Arc<RateLimiterManager>,
    provider_registry: Arc<ProviderRegistry>,
) -> anyhow::Result<()> {
    info!("Starting web server on {}:{}", config.host, config.port);

    // Create shared state
    let state = AppState::new(router, api_key_manager, rate_limiter, provider_registry);

    // Build the router with auth layer applied
    let app = build_app(state, config.enable_cors);

    // Create TCP listener
    let addr = SocketAddr::from((
        config.host.parse::<std::net::IpAddr>()?,
        config.port,
    ));
    let listener = TcpListener::bind(addr).await?;

    info!("Web server listening on http://{}", addr);
    info!("OpenAI-compatible endpoints available at:");

    // Start server
    axum::serve(listener, app)
        .await
        .map_err(|e| anyhow::anyhow!("Server error: {}", e))?;

    Ok(())
}

/// Build the Axum app with all routes and middleware
fn build_app(state: AppState, enable_cors: bool) -> Router {
    // Build the Axum router with all routes
    let mut router = Router::new()
        .route("/health", get(health_check))
        .route("/", get(root_handler))
        .route("/v1/chat/completions", post(routes::chat_completions))
        .route("/v1/completions", post(routes::completions))
        .route("/v1/embeddings", post(routes::embeddings))
        .route("/v1/models", get(routes::list_models))
        .route("/v1/generation", get(routes::get_generation))
        .with_state(state.clone());

    // Apply auth layer (checks /v1/* routes)
    router = router.layer(AuthLayer::new(state));

    // Add logging middleware
    router = router.layer(axum::middleware::from_fn(logging_middleware));

    // Add CORS if enabled
    if enable_cors {
        let cors = CorsLayer::new()
            .allow_origin(Any)
            .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
            .allow_headers([
                header::CONTENT_TYPE,
                header::AUTHORIZATION,
            ])
            .allow_credentials(false);

        router = router.layer(cors);
    }

    router
}

/// Health check endpoint
async fn health_check() -> StatusCode {
    StatusCode::OK
}

/// Root handler
async fn root_handler() -> &'static str {
    "LocalRouter AI - OpenAI-compatible API Gateway\n\
     \n\
     Endpoints:\n\
       POST /v1/chat/completions\n\
       POST /v1/completions\n\
       POST /v1/embeddings\n\
       GET  /v1/models\n\
       GET  /v1/generation?id={id}\n\
     \n\
     Authentication: Include 'Authorization: Bearer <your-api-key>' header\n"
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
