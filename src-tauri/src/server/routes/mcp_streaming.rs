// ! MCP Streaming Routes
//!
//! SSE streaming endpoints for multiplexing multiple MCP servers into a single stream.

use axum::response::sse::KeepAlive;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response, Sse},
    Extension, Json,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::error;
use utoipa::ToSchema;

use super::helpers::get_enabled_client;
use crate::config::McpServerAccess;
use crate::mcp::protocol::{JsonRpcRequest, Root};
use crate::server::middleware::client_auth::ClientAuthContext;
use crate::server::middleware::error::ApiErrorResponse;
use crate::server::state::AppState;

/// Request to initialize a streaming session
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct InitializeStreamingRequest {
    /// MCP protocol version
    #[serde(rename = "protocolVersion")]
    pub protocol_version: String,

    /// Client capabilities
    #[serde(default)]
    pub capabilities: Value,

    /// Client information
    #[serde(rename = "clientInfo")]
    pub client_info: ClientInfo,
}

/// Client information
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ClientInfo {
    /// Client name
    pub name: String,

    /// Client version
    pub version: String,
}

/// Response when creating a streaming session
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct StreamingSessionInfo {
    /// Unique session ID
    pub session_id: String,

    /// URL to connect for SSE stream
    pub stream_url: String,

    /// URL to send requests
    pub request_url: String,

    /// Successfully initialized servers
    pub initialized_servers: Vec<String>,

    /// Servers that failed to initialize
    pub failed_servers: Vec<String>,
}

/// Response when accepting a request
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RequestAccepted {
    /// Internal request ID for tracking
    pub request_id: String,

    /// Target servers for this request
    pub target_servers: Vec<String>,

    /// Whether this is a broadcast request
    pub broadcast: bool,
}

/// Initialize streaming session
///
/// Creates a new streaming session that multiplexes multiple MCP backend servers.
/// The session automatically initializes all allowed backend servers and returns
/// URLs for the SSE stream and request submission.
#[utoipa::path(
    post,
    path = "/gateway/stream",
    tag = "mcp",
    request_body = InitializeStreamingRequest,
    responses(
        (status = 200, description = "Session created successfully", body = StreamingSessionInfo),
        (status = 401, description = "Unauthorized - invalid or missing credentials"),
        (status = 429, description = "Too many sessions - rate limit exceeded"),
        (status = 500, description = "Failed to initialize session")
    ),
    security(("bearer" = []))
)]
pub async fn initialize_streaming_session(
    State(state): State<AppState>,
    client_auth: Option<Extension<ClientAuthContext>>,
    Json(request): Json<InitializeStreamingRequest>,
) -> Result<Json<StreamingSessionInfo>, ApiErrorResponse> {
    // Verify authentication
    let auth_ctx = client_auth
        .ok_or_else(|| ApiErrorResponse::unauthorized("Missing authentication"))?
        .0;

    // Get config for MCP server list
    let config = state.config_manager.get();

    // Handle internal test client specially (for UI testing)
    let client = if auth_ctx.client_id == "internal-test" {
        tracing::debug!(
            "Internal test client initializing streaming session - granting full access"
        );
        let mut test_client = crate::config::Client::new("Internal Test Client".to_string());
        test_client.id = "internal-test".to_string();
        test_client.mcp_server_access = McpServerAccess::All;
        test_client.mcp_sampling_enabled = true;
        test_client
    } else {
        get_enabled_client(&state, &auth_ctx.client_id)?
    };

    // Check MCP access mode
    if !client.mcp_server_access.has_any_access() {
        return Err(ApiErrorResponse::forbidden(
            "Client has no MCP server access",
        ));
    }

    // Get allowed servers based on access mode
    let allowed_servers: Vec<String> = match &client.mcp_server_access {
        McpServerAccess::None => vec![],
        McpServerAccess::All => config.mcp_servers.iter().map(|s| s.id.clone()).collect(),
        McpServerAccess::Specific(servers) => servers.clone(),
    };

    // Create gateway session
    let roots = config
        .roots
        .iter()
        .map(|r| Root {
            uri: r.uri.clone(),
            name: r.name.clone(),
        })
        .collect();

    let gateway_session = std::sync::Arc::new(tokio::sync::RwLock::new(
        crate::mcp::gateway::session::GatewaySession::new(
            auth_ctx.client_id.clone(),
            allowed_servers.clone(),
            std::time::Duration::from_secs(3600), // 1 hour TTL
            3600,                                 // base cache TTL
            roots,
            client.mcp_deferred_loading,
        ),
    ));

    // Prepare initialization parameters
    let init_params = serde_json::json!({
        "protocolVersion": request.protocol_version,
        "capabilities": request.capabilities,
        "clientInfo": request.client_info,
    });

    // Create streaming session
    let streaming_session = match state
        .streaming_session_manager
        .create_session(
            auth_ctx.client_id.clone(),
            allowed_servers.clone(),
            gateway_session,
            init_params,
        )
        .await
    {
        Ok(session) => session,
        Err(e) => {
            error!("Failed to create streaming session: {}", e);
            return Err(ApiErrorResponse::from(e));
        }
    };

    let session_id = streaming_session.session_id().to_string();

    // Determine which servers initialized successfully
    let initialized_servers = streaming_session.allowed_servers().to_vec();
    let failed_servers: Vec<String> = allowed_servers
        .iter()
        .filter(|s| !initialized_servers.contains(s))
        .cloned()
        .collect();

    Ok(Json(StreamingSessionInfo {
        session_id: session_id.clone(),
        stream_url: format!("/gateway/stream/{}", session_id),
        request_url: format!("/gateway/stream/{}/request", session_id),
        initialized_servers,
        failed_servers,
    }))
}

/// SSE event stream
///
/// Connects to an existing streaming session and receives real-time events
/// from all backend MCP servers. Events include responses, notifications,
/// streaming chunks, and errors.
#[utoipa::path(
    get,
    path = "/gateway/stream/{session_id}",
    tag = "mcp",
    params(
        ("session_id" = String, Path, description = "Streaming session ID")
    ),
    responses(
        (status = 200, description = "SSE event stream", content_type = "text/event-stream"),
        (status = 401, description = "Unauthorized - invalid or missing credentials"),
        (status = 403, description = "Forbidden - not your session"),
        (status = 404, description = "Session not found")
    ),
    security(("bearer" = []))
)]
pub async fn streaming_event_handler(
    Path(session_id): Path<String>,
    State(state): State<AppState>,
    client_auth: Option<Extension<ClientAuthContext>>,
) -> Response {
    // Verify authentication
    let auth_ctx = match client_auth {
        Some(ctx) => ctx.0,
        None => return ApiErrorResponse::unauthorized("Missing authentication").into_response(),
    };

    // Get session
    let session = match state.streaming_session_manager.get_session(&session_id) {
        Some(s) => s,
        None => return ApiErrorResponse::not_found("Session not found").into_response(),
    };

    // Verify ownership
    if session.client_id() != auth_ctx.client_id {
        return ApiErrorResponse::forbidden("Not your session").into_response();
    }

    // Start event stream
    let stream = session.start_event_stream().await;

    Sse::new(stream)
        .keep_alive(KeepAlive::default())
        .into_response()
}

/// Send request through streaming session
///
/// Submits a JSON-RPC request through the streaming session. The request
/// is routed to the appropriate backend server(s) based on the method name.
/// Responses are delivered asynchronously through the SSE stream.
///
/// Method routing:
/// - Namespaced methods (e.g., "filesystem__tools/call") route to specific server
/// - Broadcast methods ("tools/list", "resources/list", "prompts/list") route to all servers
#[utoipa::path(
    post,
    path = "/gateway/stream/{session_id}/request",
    tag = "mcp",
    params(
        ("session_id" = String, Path, description = "Streaming session ID")
    ),
    request_body = JsonRpcRequest,
    responses(
        (status = 200, description = "Request accepted", body = RequestAccepted),
        (status = 400, description = "Invalid request - bad routing or parameters"),
        (status = 401, description = "Unauthorized - invalid or missing credentials"),
        (status = 403, description = "Forbidden - not your session"),
        (status = 404, description = "Session not found")
    ),
    security(("bearer" = []))
)]
pub async fn send_streaming_request(
    Path(session_id): Path<String>,
    State(state): State<AppState>,
    client_auth: Option<Extension<ClientAuthContext>>,
    Json(request): Json<JsonRpcRequest>,
) -> Result<Json<RequestAccepted>, ApiErrorResponse> {
    // Verify authentication
    let auth_ctx = client_auth
        .ok_or_else(|| ApiErrorResponse::unauthorized("Missing authentication"))?
        .0;

    // Get session
    let session = state
        .streaming_session_manager
        .get_session(&session_id)
        .ok_or_else(|| ApiErrorResponse::not_found("Session not found"))?;

    // Verify ownership
    if session.client_id() != auth_ctx.client_id {
        return Err(ApiErrorResponse::forbidden("Not your session"));
    }

    // Determine routing
    let is_broadcast = matches!(
        request.method.as_str(),
        "tools/list" | "resources/list" | "prompts/list"
    );

    let target_servers = if let Some((server_id, _)) = request.method.split_once("__") {
        vec![server_id.to_string()]
    } else if is_broadcast {
        session.allowed_servers().to_vec()
    } else {
        return Err(ApiErrorResponse::bad_request(
            "Method requires server namespace (e.g., 'filesystem__tools/call')",
        ));
    };

    // Submit request
    let request_id = session.handle_request(request).await.map_err(|e| {
        error!("Failed to handle request: {}", e);
        ApiErrorResponse::from(e)
    })?;

    Ok(Json(RequestAccepted {
        request_id,
        target_servers,
        broadcast: is_broadcast,
    }))
}

/// Close streaming session
///
/// Closes an active streaming session and releases all associated resources.
/// Any pending requests will be cancelled and the SSE stream will be terminated.
#[utoipa::path(
    delete,
    path = "/gateway/stream/{session_id}",
    tag = "mcp",
    params(
        ("session_id" = String, Path, description = "Streaming session ID")
    ),
    responses(
        (status = 204, description = "Session closed successfully"),
        (status = 401, description = "Unauthorized - invalid or missing credentials"),
        (status = 403, description = "Forbidden - not your session"),
        (status = 404, description = "Session not found")
    ),
    security(("bearer" = []))
)]
pub async fn close_streaming_session(
    Path(session_id): Path<String>,
    State(state): State<AppState>,
    client_auth: Option<Extension<ClientAuthContext>>,
) -> Response {
    // Verify authentication
    let auth_ctx = match client_auth {
        Some(ctx) => ctx.0,
        None => return ApiErrorResponse::unauthorized("Missing authentication").into_response(),
    };

    // Get session
    let session = match state.streaming_session_manager.get_session(&session_id) {
        Some(s) => s,
        None => return StatusCode::NOT_FOUND.into_response(),
    };

    // Verify ownership
    if session.client_id() != auth_ctx.client_id {
        return ApiErrorResponse::forbidden("Not your session").into_response();
    }

    // Close session
    state
        .streaming_session_manager
        .close_session(&session_id)
        .await;

    StatusCode::NO_CONTENT.into_response()
}
