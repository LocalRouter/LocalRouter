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
    body::Body,
    extract::{ConnectInfo, Request},
    http::{header, Method, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    routing::{get, post},
    Router,
};
use futures::StreamExt;
use http_body::Body as HttpBody;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use tower_http::cors::{Any, CorsLayer};
use tower_http::limit::RequestBodyLimitLayer;
use tracing::{error, info};

/// Interval for periodic session cleanup tasks (gateway + MCP via LLM).
const SESSION_CLEANUP_INTERVAL: std::time::Duration = std::time::Duration::from_secs(600);

use lr_mcp::McpServerManager;
use lr_providers::registry::ProviderRegistry;
use lr_router::{RateLimiterManager, Router as AppRouter};

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
/// - POST /v1/audio/transcriptions (OpenAI)
/// - POST /v1/audio/translations (OpenAI)
/// - POST /v1/audio/speech (OpenAI)
/// - GET /v1/models (OpenAI)
/// - GET /v1/generation (OpenAI)
/// - POST / (MCP unified gateway)
/// - POST /servers/:server_id (MCP individual server)
///
/// Returns the AppState, JoinHandle, and the actual port used
pub async fn start_server(
    config: ServerConfig,
    router: Arc<AppRouter>,
    mcp_server_manager: Arc<McpServerManager>,
    rate_limiter: Arc<RateLimiterManager>,
    provider_registry: Arc<ProviderRegistry>,
    config_manager: Arc<lr_config::ConfigManager>,
    client_manager: Arc<lr_clients::ClientManager>,
    token_store: Arc<lr_clients::TokenStore>,
    metrics_collector: Arc<lr_monitoring::metrics::MetricsCollector>,
    health_cache: Option<Arc<lr_providers::health_cache::HealthCacheManager>>,
) -> anyhow::Result<(
    AppState,
    tokio::task::JoinHandle<()>,
    u16,
    CancellationToken,
)> {
    info!("Starting web server on {}:{}", config.host, config.port);

    // Shutdown signal: cancelling this stops the accept loop (graceful
    // shutdown) AND kills any in-flight request/stream via the kill-switch
    // middleware below.
    let shutdown = CancellationToken::new();

    // Create shared state
    let state = AppState::new(
        router,
        rate_limiter,
        provider_registry,
        config_manager,
        client_manager,
        token_store,
        metrics_collector,
        health_cache,
    )
    .with_mcp(mcp_server_manager);

    // Build the router with auth layer applied
    let app = build_app(state.clone(), config.enable_cors, shutdown.clone());

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

    // Spawn session cleanup tasks (runs every 10 minutes)
    let gateway_for_cleanup = state.mcp_gateway.clone();
    let mcp_via_llm_for_cleanup = state.mcp_via_llm_manager.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(SESSION_CLEANUP_INTERVAL);
        loop {
            interval.tick().await;
            gateway_for_cleanup.cleanup_expired_sessions().await;
            mcp_via_llm_for_cleanup.cleanup_expired_sessions();
        }
    });

    // Spawn token cleanup task (runs every 5 minutes)
    let token_store_for_cleanup = state.token_store.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(300)); // 5 minutes
        loop {
            interval.tick().await;
            let removed = token_store_for_cleanup.cleanup_expired();
            if removed > 0 {
                info!("Cleaned up {} expired OAuth tokens", removed);
            }
        }
    });

    // Start server. Graceful shutdown stops accepting new connections when the
    // token is cancelled; the kill-switch middleware terminates in-flight
    // requests/streams so Stop is immediate rather than draining.
    let shutdown_for_serve = shutdown.clone();
    let handle = tokio::spawn(async move {
        if let Err(e) = axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .with_graceful_shutdown(async move { shutdown_for_serve.cancelled().await })
        .await
        {
            error!("Server error: {}", e);
        }
    });

    Ok((state_clone, handle, port, shutdown))
}

/// Build the Axum app with all routes and middleware.
///
/// `shutdown` drives the kill-switch middleware: when cancelled (server Stop),
/// in-flight requests are aborted and streaming response bodies stop emitting,
/// instead of being allowed to run to completion.
fn build_app(state: AppState, enable_cors: bool, shutdown: CancellationToken) -> Router {
    // Build MCP routes with client auth middleware
    // MCP routes: unified gateway at root (/) and /mcp
    // GET returns SSE if Accept: text/event-stream, otherwise API info
    let mcp_routes = Router::new()
        .route(
            "/",
            get(routes::mcp_gateway_get_handler).post(routes::mcp_gateway_handler),
        ) // Unified MCP gateway: GET for SSE/info, POST for JSON-RPC
        .route(
            "/mcp",
            get(routes::mcp_gateway_get_handler).post(routes::mcp_gateway_handler),
        ) // Alias: /mcp also serves unified gateway
        .route("/ws", get(routes::mcp_websocket_handler)) // WebSocket notifications
        .route("/mcp/ws", get(routes::mcp_websocket_handler)) // Alias: /mcp/ws
        .route(
            "/mcp/elicitation/respond/{request_id}",
            post(routes::elicitation_response_handler), // Submit elicitation responses
        )
        .route(
            "/mcp/sampling/respond/{request_id}",
            post(routes::sampling_passthrough_response_handler), // Submit sampling passthrough responses
        )
        .layer(axum::middleware::from_fn(
            middleware::client_auth::client_auth_middleware,
        ))
        .layer(axum::Extension(state.clone()))
        .with_state(state.clone());

    // Build OAuth routes (no auth required - these ARE the auth endpoints)
    let oauth_state = routes::oauth::OAuthState {
        client_manager: state.client_manager.clone(),
        token_store: state.token_store.clone(),
        token_rate_limiter: Arc::new(dashmap::DashMap::new()),
        monitor_store: state.monitor_store.clone(),
    };

    let oauth_routes = Router::new()
        .route("/oauth/token", post(routes::token_endpoint))
        .with_state(oauth_state);

    // Build the Axum router with all routes
    // Support both /v1 prefix and without for OpenAI compatibility
    // Note: GET / is handled by mcp_gateway_get_handler in mcp_routes (content negotiation)
    let mut router = Router::new()
        .route("/health", get(health_check))
        // OpenAPI specification endpoints
        .route("/openapi.json", get(serve_openapi_json))
        .route("/openapi.yaml", get(serve_openapi_yaml))
        // Routes with /v1 prefix (all protected by blanket `/v1/` check in auth layer)
        .route("/v1/chat/completions", post(routes::chat_completions))
        .route("/v1/completions", post(routes::completions))
        .route("/v1/responses", post(routes::create_response))
        .route("/v1/embeddings", post(routes::embeddings))
        .route("/v1/moderations", post(routes::moderations))
        .route("/v1/images/generations", post(routes::image_generations))
        .route("/v1/audio/speech", post(routes::audio_speech))
        .route("/v1/models", get(routes::list_models))
        .route("/v1/models/{id}", get(routes::get_model))
        .route(
            "/v1/models/{provider}/{model}/pricing",
            get(routes::get_model_pricing),
        )
        .route("/v1/generation", get(routes::get_generation))
        // Routes without /v1 prefix (for compatibility)
        // IMPORTANT: Each non-prefixed route MUST also be added to the auth layer's
        // `is_protected` check in middleware/auth_layer.rs — the /v1/ blanket match
        // does NOT cover these. The test `test_all_api_routes_require_auth` will catch
        // any missing entries.
        .route("/chat/completions", post(routes::chat_completions))
        .route("/completions", post(routes::completions))
        .route("/responses", post(routes::create_response))
        .route("/embeddings", post(routes::embeddings))
        .route("/moderations", post(routes::moderations))
        .route("/images/generations", post(routes::image_generations))
        .route("/audio/speech", post(routes::audio_speech))
        .route("/models", get(routes::list_models))
        .route("/models/{id}", get(routes::get_model))
        .route(
            "/models/{provider}/{model}/pricing",
            get(routes::get_model_pricing),
        )
        .route("/generation", get(routes::get_generation))
        .with_state(state.clone());

    // Apply auth layer (checks all API routes with or without /v1 prefix)
    router = router.layer(AuthLayer::new(state.clone()));

    // Apply 16MB body limit to main routes BEFORE merging audio routes
    // (audio routes need 25MB for file uploads — see below)
    router = router.layer(RequestBodyLimitLayer::new(16 * 1024 * 1024));

    // Audio upload routes with 25MB body limit (audio files can be up to 25MB per OpenAI spec)
    // Merged AFTER the 16MB limit so the global limit doesn't override the audio-specific one.
    // NOTE: Non-prefixed /audio/* routes are covered by the `starts_with("/audio/")` check
    // in middleware/auth_layer.rs — no additional entry needed for new /audio/* sub-routes.
    let audio_upload_routes = Router::new()
        .route(
            "/v1/audio/transcriptions",
            post(routes::audio_transcriptions),
        )
        .route("/v1/audio/translations", post(routes::audio_translations))
        .route("/audio/transcriptions", post(routes::audio_transcriptions))
        .route("/audio/translations", post(routes::audio_translations))
        .with_state(state.clone())
        .layer(AuthLayer::new(state.clone()))
        .layer(RequestBodyLimitLayer::new(25 * 1024 * 1024));

    router = router.merge(audio_upload_routes);

    // Merge OAuth routes (no auth required - these ARE the auth endpoints)
    router = router.merge(oauth_routes);

    // Merge MCP routes (these use OAuth auth, not API key auth)
    router = router.merge(mcp_routes);

    // Add logging middleware
    router = router.layer(axum::middleware::from_fn(logging_middleware));
    router = router.layer(axum::middleware::from_fn(security_headers_middleware));

    // Add DNS rebinding protection
    router = router.layer(axum::middleware::from_fn(host_validation_middleware));

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

    // Kill-switch (outermost layer): when `shutdown` is cancelled (server Stop),
    // abort in-flight requests so they don't run to completion. Added last so it
    // wraps every other layer/route.
    router = router.layer(axum::middleware::from_fn_with_state(shutdown, kill_switch));

    router
}

/// Kill-switch middleware: terminates an in-flight request the moment the
/// server's `shutdown` token is cancelled (server Stop), instead of letting it
/// run to completion / drain.
///
/// - Pre-response: races the handler against cancellation, returning 503 and
///   dropping the handler future if Stop wins.
/// - Streaming responses (unknown-length bodies, e.g. SSE / streamed
///   completions): wraps the body so it stops emitting on cancel. Buffered
///   responses (known exact length) pass through untouched, preserving their
///   Content-Length.
async fn kill_switch(
    axum::extract::State(shutdown): axum::extract::State<CancellationToken>,
    req: Request,
    next: Next,
) -> Response {
    let response = tokio::select! {
        biased;
        _ = shutdown.cancelled() => {
            return (StatusCode::SERVICE_UNAVAILABLE, "Server stopping").into_response();
        }
        resp = next.run(req) => resp,
    };

    let (parts, body) = response.into_parts();
    if body.size_hint().exact().is_some() {
        return Response::from_parts(parts, body);
    }

    let mut data = body.into_data_stream();
    let guarded = async_stream::stream! {
        loop {
            tokio::select! {
                biased;
                _ = shutdown.cancelled() => break,
                chunk = data.next() => match chunk {
                    Some(item) => yield item,
                    None => break,
                }
            }
        }
    };
    Response::from_parts(parts, Body::from_stream(guarded))
}

/// Health check endpoint
#[utoipa::path(
    get,
    path = "/health",
    tag = "system",
    responses(
        (status = 200, description = "Server is healthy", body = String)
    )
)]
async fn health_check() -> &'static str {
    "ok"
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

/// DNS rebinding protection middleware
///
/// Validates the Host header against expected localhost values to prevent
/// DNS rebinding attacks where an attacker's domain resolves to 127.0.0.1.
async fn host_validation_middleware(req: Request, next: Next) -> Response {
    use middleware::error::ApiErrorResponse;

    if let Some(host) = req
        .headers()
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
    {
        let host_without_port = host.split(':').next().unwrap_or(host);
        match host_without_port {
            "localhost" | "127.0.0.1" | "[::1]" => {} // OK
            _ => {
                return ApiErrorResponse::forbidden("Invalid Host header").into_response();
            }
        }
    }
    // If no Host header, allow (some clients don't send it)
    next.run(req).await
}

/// Security headers middleware
async fn security_headers_middleware(req: Request, next: Next) -> Response {
    let mut response = next.run(req).await;
    let headers = response.headers_mut();
    headers.insert(
        axum::http::header::HeaderName::from_static("x-content-type-options"),
        axum::http::header::HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        axum::http::header::HeaderName::from_static("x-frame-options"),
        axum::http::header::HeaderValue::from_static("DENY"),
    );
    headers.insert(
        axum::http::header::HeaderName::from_static("referrer-policy"),
        axum::http::header::HeaderValue::from_static("strict-origin-when-cross-origin"),
    );
    headers.insert(
        axum::http::header::HeaderName::from_static("cache-control"),
        axum::http::header::HeaderValue::from_static("no-store"),
    );
    response
}

/// Logging middleware to log all requests
async fn logging_middleware(req: Request, next: Next) -> Response {
    use crate::middleware::client_auth::LoggedClientId;

    let method = req.method().clone();
    let uri = req.uri().clone();
    let peer = req
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ci| ci.0);
    let start = std::time::Instant::now();

    let response = next.run(req).await;

    let elapsed = start.elapsed();
    let status = response.status();

    let peer_str = peer
        .map(|a| a.to_string())
        .unwrap_or_else(|| "unknown".into());
    let client = response
        .extensions()
        .get::<LoggedClientId>()
        .map(|id| id.0.as_str())
        .unwrap_or("none");

    if status.is_success() {
        info!(
            "{} {} - {} ({:?}) [{}] client={}",
            method, uri, status, elapsed, peer_str, client
        );
    } else {
        error!(
            "{} {} - {} ({:?}) [{}] client={}",
            method, uri, status, elapsed, peer_str, client
        );
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

#[cfg(test)]
mod kill_switch_tests {
    use super::*;
    use axum::body::Body;
    use axum::routing::get;
    use axum::Router;
    use tower::ServiceExt; // for `oneshot`

    fn test_app(token: CancellationToken) -> Router {
        Router::new()
            .route("/buffered", get(|| async { "ok" }))
            .route(
                "/stream",
                get(|| async {
                    // Endless stream — only ends if the kill-switch terminates it.
                    let s = async_stream::stream! {
                        loop {
                            yield Ok::<_, std::io::Error>(
                                axum::body::Bytes::from_static(b"chunk\n"),
                            );
                            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                        }
                    };
                    Body::from_stream(s)
                }),
            )
            .layer(axum::middleware::from_fn_with_state(token, kill_switch))
    }

    #[tokio::test]
    async fn passes_through_when_not_cancelled() {
        let app = test_app(CancellationToken::new());
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/buffered")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn returns_503_when_already_cancelled() {
        let token = CancellationToken::new();
        token.cancel();
        let app = test_app(token);
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/buffered")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn streaming_body_terminates_on_cancel() {
        let token = CancellationToken::new();
        let app = test_app(token.clone());
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/stream")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        // Cancel shortly after the stream starts; the guarded body must then end
        // (otherwise `to_bytes` would hang on the endless stream).
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(30)).await;
            token.cancel();
        });

        let collected = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            axum::body::to_bytes(resp.into_body(), usize::MAX),
        )
        .await;
        assert!(
            collected.is_ok(),
            "streaming body did not terminate after the kill-switch fired"
        );
    }
}
