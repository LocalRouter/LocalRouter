//! MCP gateway routes
//!
//! Handles proxying JSON-RPC requests from external MCP clients to MCP servers.
//! All requests go through the unified gateway at POST /.
//! GET / returns SSE stream if Accept: text/event-stream, otherwise API info.

use axum::{
    extract::{Path, Query, State},
    http::header,
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse, Response,
    },
    Json,
};
use std::convert::Infallible;
use uuid::Uuid;

use super::helpers::{check_mcp_access_with_state, get_enabled_client_from_manager};
use crate::middleware::client_auth::ClientAuthContext;
use crate::middleware::error::ApiErrorResponse;
use crate::state::{AppState, SseConnectionManager, SseMessage};
use lr_config::RootConfig;
use lr_mcp::protocol::{JsonRpcRequest, JsonRpcResponse, Root};

/// Query parameters for MCP POST requests
#[derive(serde::Deserialize, Default)]
pub struct McpQueryParams {
    /// Per-connection session ID (sent in the SSE endpoint event)
    #[serde(rename = "sessionId")]
    session_id: Option<String>,
}

/// Send a JSON-RPC response via SSE stream (preferred) or HTTP body (fallback)
///
/// The MCP SDK's SSEClientTransport expects responses via the SSE stream.
/// We send via SSE if a connection exists, and also include in HTTP body as fallback.
///
/// `connection_key` is either a per-connection session UUID (for SSE) or client_id (for non-SSE).
fn send_response(
    sse_manager: &SseConnectionManager,
    connection_key: &str,
    response: JsonRpcResponse,
) -> Response {
    let response_id = response.id.clone();

    // Try to send via SSE stream first (required for MCP SDK's SSEClientTransport)
    if sse_manager.send_response(connection_key, response.clone()) {
        tracing::debug!(
            "Response sent via SSE: connection={}, response_id={:?}",
            &connection_key[..8.min(connection_key.len())],
            response_id
        );
        (axum::http::StatusCode::ACCEPTED, "").into_response()
    } else {
        // No SSE connection - fall back to returning in HTTP body
        tracing::debug!(
            "No SSE connection, returning response in HTTP body: connection={}, response_id={:?}",
            &connection_key[..8.min(connection_key.len())],
            response_id
        );
        Json(response).into_response()
    }
}

/// Unified MCP gateway with content negotiation
///
/// Returns SSE stream if Accept header contains text/event-stream,
/// otherwise returns API information text.
///
/// SSE mode: Establishes an SSE connection to receive notifications from all
/// allowed MCP servers for this client. Used by MCP SDK's SSEClientTransport.
///
/// Text mode: Returns basic API information.
#[utoipa::path(
    get,
    path = "/",
    tag = "mcp",
    responses(
        (status = 200, description = "SSE event stream or API info", content_type = "text/event-stream"),
        (status = 401, description = "Unauthorized", body = crate::types::ErrorResponse)
    ),
    security(("bearer" = []))
)]
pub async fn mcp_gateway_get_handler(
    State(state): State<AppState>,
    client_auth: Option<axum::Extension<ClientAuthContext>>,
    headers: axum::http::HeaderMap,
) -> Response {
    // Check if client accepts SSE
    let accepts_sse = headers
        .get(header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.contains("text/event-stream"))
        .unwrap_or(false);

    if !accepts_sse {
        // Return API info text
        return (
            axum::http::StatusCode::OK,
            [(header::CONTENT_TYPE, "text/plain")],
            "LocalRouter - OpenAI-Compatible LLM Gateway & Unified MCP Gateway\n\
             \n\
             API Documentation: /openapi.json\n",
        )
            .into_response();
    }

    // SSE mode - return event stream for unified gateway
    let client_id = match client_auth {
        Some(ctx) => ctx.0.client_id.clone(),
        None => {
            return ApiErrorResponse::unauthorized("Missing authentication context")
                .into_response();
        }
    };

    // Get all server IDs
    let all_server_ids: Vec<String> = state
        .config_manager
        .get()
        .mcp_servers
        .iter()
        .map(|s| s.id.clone())
        .collect();

    // Handle internal test client specially (for UI testing)
    let allowed_servers = if client_id == "internal-test" {
        // Read MCP access header to determine which servers to include
        let mcp_access_header = headers
            .get("x-mcp-access")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("all");

        if mcp_access_header.eq_ignore_ascii_case("none") {
            tracing::debug!("Internal test client establishing SSE connection with no MCP servers (skills-only)");
            vec![]
        } else if mcp_access_header.eq_ignore_ascii_case("all") {
            tracing::debug!(
                "Internal test client establishing unified SSE connection with all servers"
            );
            all_server_ids
        } else {
            // Specific server ID
            tracing::debug!(
                "Internal test client establishing SSE connection for server {}",
                mcp_access_header
            );
            vec![mcp_access_header.to_string()]
        }
    } else {
        // Get enabled client from manager
        let client = match get_enabled_client_from_manager(&state, &client_id) {
            Ok(client) => client,
            Err(e) => return e.into_response(),
        };

        // Enforce client mode: block LLM-only and MCP-via-LLM clients from direct MCP access
        if let Err(e) = check_mcp_access_with_state(&state, &client) {
            return e.into_response();
        }

        // Get allowed servers based on mcp_permissions
        // An empty server list is valid — the gateway also serves marketplace,
        // coding agents, and skills which don't require MCP server access.
        if client.mcp_permissions.global.is_enabled() {
            all_server_ids
        } else {
            // Filter to servers with enabled permission at server or sub-item level
            all_server_ids
                .into_iter()
                .filter(|server_id| client.mcp_permissions.has_any_enabled_for_server(server_id))
                .collect()
        }
    };

    // Generate a unique session ID for this SSE connection.
    // This allows multiple simultaneous connections from the same client,
    // each with its own gateway session and ContextMode process.
    let session_id = Uuid::new_v4().to_string();

    tracing::debug!(
        "Unified SSE connection established for client {} (session={}) with {} servers (skills support: {})",
        client_id,
        &session_id[..8],
        allowed_servers.len(),
        state.mcp_gateway.has_skill_support()
    );

    // Register with SSE connection manager using session_id (not client_id)
    let mut response_rx = state.sse_connection_manager.register(&session_id);

    // Subscribe to notification broadcast
    let mut notification_rx = state.mcp_notification_broadcast.subscribe();

    // Subscribe to per-client permission change notifications
    let mut client_notification_rx = state.client_notification_broadcast.subscribe();

    // Clone for cleanup
    let session_id_cleanup = session_id.clone();
    let sse_manager = state.sse_connection_manager.clone();
    let gateway_for_cleanup = state.mcp_gateway.clone();

    // Create SSE stream that forwards both responses and notifications
    let sse_stream = async_stream::stream! {
        // Send endpoint event first (MCP SSE transport spec)
        // Include sessionId in the endpoint URL so POST requests are routed back to this connection
        let endpoint = format!("/?sessionId={}", session_id);
        tracing::debug!("SSE stream started: client={}, session={}", &client_id[..8.min(client_id.len())], &session_id[..8]);
        yield Ok::<_, Infallible>(Event::default().event("endpoint").data(endpoint));

        loop {
            // Use biased select to prioritize responses over notifications
            // This ensures responses are sent immediately when available
            tokio::select! {
                biased;

                // Handle responses from POST requests (high priority)
                msg = response_rx.recv() => {
                    match msg {
                        Some(sse_msg) => {
                            // Send raw JSON-RPC, not wrapped SseMessage (MCP SSE transport spec)
                            match sse_msg {
                                SseMessage::Response(response) => {
                                    let response_id = response.id.clone();
                                    match serde_json::to_string(&response) {
                                        Ok(json) => {
                                            tracing::debug!(
                                                "SSE response: client={}, id={:?}, len={}",
                                                &client_id[..8.min(client_id.len())],
                                                response_id,
                                                json.len()
                                            );
                                            yield Ok::<_, Infallible>(Event::default().event("message").data(json));
                                        }
                                        Err(e) => {
                                            tracing::error!(
                                                "Failed to serialize response for SSE: {} (client={}, id={:?})",
                                                e,
                                                client_id,
                                                response_id
                                            );
                                        }
                                    }
                                }
                                SseMessage::Notification(notification) => {
                                    if let Ok(json) = serde_json::to_string(&notification) {
                                        tracing::debug!("SSE stream yielding notification for client {}", client_id);
                                        yield Ok::<_, Infallible>(Event::default().event("message").data(json));
                                    }
                                }
                                SseMessage::Request(request) => {
                                    // Server-initiated request (sampling, elicitation, roots/list)
                                    match serde_json::to_string(&request) {
                                        Ok(json) => {
                                            tracing::info!(
                                                "SSE stream yielding server-initiated request for client {}: method={}, id={:?}",
                                                client_id,
                                                request.method,
                                                request.id
                                            );
                                            yield Ok::<_, Infallible>(Event::default().event("message").data(json));
                                        }
                                        Err(e) => {
                                            tracing::error!(
                                                "Failed to serialize request for SSE: {} (client={}, method={})",
                                                e,
                                                client_id,
                                                request.method
                                            );
                                        }
                                    }
                                }
                                SseMessage::Endpoint { .. } => {
                                    // Endpoint events handled separately at stream start
                                }
                            }
                        }
                        None => {
                            tracing::debug!("Response channel closed for client {}", client_id);
                            break;
                        }
                    }
                }

                // Handle notifications from MCP servers
                notif_result = notification_rx.recv() => {
                    match notif_result {
                        Ok((server_id, notification)) => {
                            // Only forward notifications for allowed servers
                            if allowed_servers.contains(&server_id) {
                                // Forward with standard MCP method names so SDK clients
                                // can match them (e.g. ToolListChangedNotificationSchema).
                                // For resource update notifications, namespace the URI in params.
                                let forwarded = if notification.method == "notifications/resources/updated" {
                                    // Namespace the resource URI so clients can match it
                                    let params = notification.params.as_ref().map(|p| {
                                        let mut p = p.clone();
                                        if let Some(uri) = p.get("uri").and_then(|v| v.as_str()) {
                                            p["uri"] = serde_json::Value::String(
                                                format!("{}::{}", server_id, uri)
                                            );
                                        }
                                        p
                                    });
                                    lr_mcp::protocol::JsonRpcNotification {
                                        jsonrpc: notification.jsonrpc.clone(),
                                        method: notification.method.clone(),
                                        params,
                                    }
                                } else {
                                    notification
                                };
                                // Send raw JSON-RPC notification (MCP SSE transport spec)
                                if let Ok(json) = serde_json::to_string(&forwarded) {
                                    yield Ok::<_, Infallible>(Event::default().event("message").data(json));
                                }
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                            tracing::warn!("Unified SSE client {} lagged, missed {} notifications", client_id, n);
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                            tracing::debug!("Notification broadcast closed");
                            break;
                        }
                    }
                }

                // Handle per-client permission change notifications
                client_notif_result = client_notification_rx.recv() => {
                    match client_notif_result {
                        Ok((target_client_id, notification)) => {
                            if target_client_id == client_id {
                                if let Ok(json) = serde_json::to_string(&notification) {
                                    tracing::info!(
                                        "SSE: sending permission change notification to client {}: {}",
                                        client_id,
                                        notification.method
                                    );
                                    yield Ok::<_, Infallible>(Event::default().event("message").data(json));
                                }
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                            tracing::warn!("Client notification channel lagged for {}, missed {} messages", client_id, n);
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                            tracing::debug!("Client notification broadcast closed");
                            break;
                        }
                    }
                }
            }
        }

        // Cleanup: terminate gateway session (closes per-session transports)
        // and unregister from SSE manager when stream ends
        let _ = gateway_for_cleanup.terminate_session(&session_id_cleanup).await;
        sse_manager.unregister(&session_id_cleanup);
        tracing::debug!("SSE stream ended for session {}", session_id_cleanup);
    };

    Sse::new(sse_stream)
        .keep_alive(KeepAlive::default())
        .into_response()
}

/// MCP unified gateway handler
///
/// Single endpoint that aggregates multiple MCP servers into one interface.
/// Client is identified via authentication token (no client_id in URL).
/// Tools/resources/prompts are namespaced to avoid collisions.
///
/// # Request Body
/// JSON-RPC 2.0 request
///
/// # Response
/// JSON-RPC 2.0 response with merged results from multiple servers
#[utoipa::path(
    post,
    path = "/",
    tag = "mcp",
    request_body = lr_mcp::protocol::JsonRpcRequest,
    responses(
        (status = 200, description = "JSON-RPC response", body = lr_mcp::protocol::JsonRpcResponse),
        (status = 401, description = "Unauthorized", body = crate::types::ErrorResponse),
        (status = 500, description = "Internal server error", body = crate::types::ErrorResponse)
    ),
    security(
        ("bearer" = [])
    )
)]
pub async fn mcp_gateway_handler(
    State(state): State<AppState>,
    client_auth: Option<axum::Extension<ClientAuthContext>>,
    headers: axum::http::HeaderMap,
    query: Query<McpQueryParams>,
    Json(body): Json<serde_json::Value>,
) -> Response {
    // Extract client_id from auth context (no URL parameter)
    let client_id = match client_auth {
        Some(ctx) => ctx.0.client_id.clone(),
        None => {
            return ApiErrorResponse::unauthorized("Missing authentication context")
                .into_response();
        }
    };

    // Extract per-connection session ID from query params (set by SSE endpoint event)
    let session_id = query.0.session_id;
    // Connection key for SSE routing: session_id if available, otherwise client_id
    let connection_key = session_id.as_deref().unwrap_or(&client_id).to_string();

    // Detect whether this is a JSON-RPC response (from client answering a server-initiated
    // request, e.g. passthrough sampling/elicitation) or a normal JSON-RPC request.
    // Responses have "result" or "error" fields but no "method" field.
    let is_response = !body.get("method").is_some_and(|v| v.is_string())
        && (body.get("result").is_some() || body.get("error").is_some());

    if is_response {
        // This is a JSON-RPC response from the client (e.g. answering a passthrough
        // sampling/createMessage or elicitation/create request we forwarded via SSE).
        let response: JsonRpcResponse = match serde_json::from_value(body) {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!("Failed to parse client JSON-RPC response: {}", e);
                return ApiErrorResponse::bad_request(format!("Invalid JSON-RPC response: {}", e))
                    .into_response();
            }
        };

        let response_id = response.id.clone();

        tracing::info!(
            "Received client JSON-RPC response: connection={}, id={}",
            &connection_key[..8.min(connection_key.len())],
            response_id
        );

        // Route the response to the pending server-initiated request
        if state
            .sse_connection_manager
            .resolve_server_request(&connection_key, response)
        {
            return Json(crate::types::MessageResponse {
                message: "Response accepted".to_string(),
            })
            .into_response();
        } else {
            tracing::warn!(
                "No pending server request matched for response id={} on connection={}",
                response_id,
                &connection_key[..8.min(connection_key.len())]
            );
            // Still return 202 - the response may have been for a request that already
            // timed out or was handled by another path
            return (
                axum::http::StatusCode::ACCEPTED,
                Json(crate::types::MessageResponse {
                    message: "Response accepted (no pending request matched)".to_string(),
                }),
            )
                .into_response();
        }
    }

    // Parse as a JSON-RPC request
    let mut request: JsonRpcRequest = match serde_json::from_value(body) {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("Failed to parse JSON-RPC request: {}", e);
            return ApiErrorResponse::bad_request(format!("Invalid JSON-RPC request: {}", e))
                .into_response();
        }
    };

    // ---- MCP 2026-07-28 transport handling (SEP-2243 / SEP-2575) ----
    // Peers may declare their protocol revision via the MCP-Protocol-Version
    // header. Versions newer than the latest we support are rejected; the
    // stateless revision additionally sends Mcp-Method / Mcp-Name routing
    // headers that must agree with the JSON-RPC body.
    let header_version = headers
        .get("mcp-protocol-version")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);

    if let Some(version) = header_version.as_deref() {
        // ISO-date version strings compare chronologically as strings.
        if version > lr_mcp::protocol::MCP_PROTOCOL_VERSION_STATELESS {
            let error_response = JsonRpcResponse::error(
                request.id.clone().unwrap_or(serde_json::Value::Null),
                lr_mcp::protocol::JsonRpcError::unsupported_protocol_version(version),
            );
            return (axum::http::StatusCode::BAD_REQUEST, Json(error_response)).into_response();
        }
    }

    let stateless_transport = header_version
        .as_deref()
        .map(lr_mcp::protocol::ProtocolRevision::from_peer_version)
        .is_some_and(|r| r.is_stateless());

    if stateless_transport {
        // Mcp-Method must agree with the JSON-RPC body when present
        // (SEP-2243; absence is tolerated during the RC transition window).
        if let Some(header_method) = headers.get("mcp-method").and_then(|v| v.to_str().ok()) {
            if header_method != request.method {
                let error_response = JsonRpcResponse::error(
                    request.id.clone().unwrap_or(serde_json::Value::Null),
                    lr_mcp::protocol::JsonRpcError::header_mismatch(format!(
                        "Mcp-Method header '{}' does not match body method '{}'",
                        header_method, request.method
                    )),
                );
                return (axum::http::StatusCode::BAD_REQUEST, Json(error_response)).into_response();
            }
        } else {
            tracing::debug!(
                "2026-07-28 client omitted Mcp-Method header (method={})",
                request.method
            );
        }

        // Mcp-Name carries the operation target (tool/prompt name or
        // resource URI); when both header and body specify it they must agree.
        if let Some(header_name) = headers.get("mcp-name").and_then(|v| v.to_str().ok()) {
            let body_name = request
                .params
                .as_ref()
                .and_then(|p| p.get("name").or_else(|| p.get("uri")))
                .and_then(|v| v.as_str());
            if let Some(body_name) = body_name {
                if header_name != body_name {
                    let error_response = JsonRpcResponse::error(
                        request.id.clone().unwrap_or(serde_json::Value::Null),
                        lr_mcp::protocol::JsonRpcError::header_mismatch(format!(
                            "Mcp-Name header '{}' does not match body target '{}'",
                            header_name, body_name
                        )),
                    );
                    return (axum::http::StatusCode::BAD_REQUEST, Json(error_response))
                        .into_response();
                }
            }
        }

        // Make the stateless declaration visible to the gateway even when the
        // body omits `_meta` (the session revision has one source of truth).
        let params = request.params.get_or_insert_with(|| serde_json::json!({}));
        if let Some(obj) = params.as_object_mut() {
            let meta = obj.entry("_meta").or_insert_with(|| serde_json::json!({}));
            if let Some(meta_obj) = meta.as_object_mut() {
                meta_obj
                    .entry(lr_mcp::protocol::meta_keys::PROTOCOL_VERSION.to_string())
                    .or_insert_with(|| {
                        serde_json::json!(lr_mcp::protocol::MCP_PROTOCOL_VERSION_STATELESS)
                    });
            }
        }
    }

    // Record client activity for connection graph
    state.record_client_activity(&client_id);

    // Check for headers used by Try it out UI
    // Only applies to internal-test client for security - external clients use their config
    // Use lowercase header name as that's how browsers/http2 send it
    let mcp_access_header = headers
        .get("x-mcp-access")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("all");

    let skills_access_header = headers
        .get("x-skills-access")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.to_string());

    let coding_agent_access_header = headers
        .get("x-coding-agent-access")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.to_string());

    // Get all server IDs for later use
    let all_server_ids: Vec<String> = state
        .config_manager
        .get()
        .mcp_servers
        .iter()
        .map(|s| s.id.clone())
        .collect();

    // Handle internal test client specially (for UI testing)
    let (client, allowed_servers) = if client_id == "internal-test" {
        // Create a synthetic client with access based on headers
        let mut test_client = lr_config::Client::new_with_strategy(
            "Internal Test Client".to_string(),
            "internal-test".to_string(),
        );
        test_client.id = "internal-test".to_string();
        test_client.mcp_sampling_enabled = true;

        // Apply MCP server access from header using mcp_permissions
        let allowed = if mcp_access_header.eq_ignore_ascii_case("none") {
            test_client.mcp_permissions.global = lr_config::PermissionState::Off;
            vec![]
        } else if mcp_access_header.eq_ignore_ascii_case("all") {
            test_client.mcp_permissions.global = lr_config::PermissionState::Allow;
            all_server_ids.clone()
        } else {
            // Specific server ID
            let server_id = mcp_access_header.to_string();
            test_client.mcp_permissions.global = lr_config::PermissionState::Off;
            test_client
                .mcp_permissions
                .servers
                .insert(server_id.clone(), lr_config::PermissionState::Allow);
            vec![server_id]
        };

        // Apply skills access from header using skills_permissions
        match &skills_access_header {
            Some(v) if v.eq_ignore_ascii_case("all") => {
                test_client.skills_permissions.global = lr_config::PermissionState::Allow;
            }
            Some(skill_name) if !skill_name.is_empty() => {
                test_client.skills_permissions.global = lr_config::PermissionState::Off;
                test_client
                    .skills_permissions
                    .skills
                    .insert(skill_name.clone(), lr_config::PermissionState::Allow);
            }
            _ => {
                test_client.skills_permissions.global = lr_config::PermissionState::Off;
            }
        }

        // Determine if this is "All MCPs & Skills" mode vs direct mode
        let is_all_mode = mcp_access_header.eq_ignore_ascii_case("all")
            && skills_access_header
                .as_ref()
                .is_some_and(|s| s.eq_ignore_ascii_case("all"));

        // Marketplace:
        // - "All" mode: enable only if globally enabled
        // - Direct mode (specific server/skill): disabled
        if is_all_mode
            && (state.config_manager.get().marketplace.mcp_enabled
                || state.config_manager.get().marketplace.skills_enabled)
        {
            test_client.marketplace_permission = lr_config::PermissionState::Allow;
        } else {
            test_client.marketplace_permission = lr_config::PermissionState::Off;
        }

        // Coding agents:
        // - "All" mode: enable coding agents if any agent is available
        // - Direct mode with specific agent type: enable that agent
        // - Otherwise: disabled
        match &coding_agent_access_header {
            Some(agent_type_str) if !agent_type_str.is_empty() => {
                if let Ok(agent_type) = serde_json::from_value::<lr_config::CodingAgentType>(
                    serde_json::Value::String(agent_type_str.clone()),
                ) {
                    test_client.coding_agent_permission = lr_config::PermissionState::Allow;
                    test_client.coding_agent_type = Some(agent_type);
                }
            }
            _ => {
                test_client.coding_agent_permission = lr_config::PermissionState::Off;
                test_client.coding_agent_type = None;
            }
        }

        // Context management (indexing):
        // - "All" mode: inherit global setting (None)
        // - Direct mode (specific server/skill): disabled — individual MCP testing
        //   shouldn't use indexing since it adds IndexSearch/IndexRead tools that
        //   aren't relevant when testing a single server
        if !is_all_mode {
            test_client.context_management_enabled = Some(false);
        }

        tracing::info!(
            "Internal test client: mcp_access={}, skills_access={:?}, marketplace={:?}, coding_agent={:?}, is_all_mode={}",
            mcp_access_header,
            skills_access_header,
            test_client.marketplace_permission,
            coding_agent_access_header,
            is_all_mode,
        );
        (test_client, allowed)
    } else {
        // Get enabled client from manager
        let client = match get_enabled_client_from_manager(&state, &client_id) {
            Ok(client) => client,
            Err(e) => return e.into_response(),
        };

        // Enforce client mode: block LLM-only and MCP-via-LLM clients from direct MCP access
        if let Err(e) = check_mcp_access_with_state(&state, &client) {
            return e.into_response();
        }

        // Get allowed servers based on mcp_permissions
        // An empty server list is valid — the gateway also serves marketplace,
        // coding agents, and skills which don't require MCP server access.
        let allowed = if client.mcp_permissions.global.is_enabled() {
            all_server_ids.clone()
        } else {
            // Filter to servers with enabled permission at server or sub-item level
            all_server_ids
                .iter()
                .filter(|server_id| client.mcp_permissions.has_any_enabled_for_server(server_id))
                .cloned()
                .collect()
        };

        (client, allowed)
    };

    tracing::info!(
        "MCP request: client={}, method={}, servers={}",
        &client_id[..8.min(client_id.len())],
        request.method,
        allowed_servers.len()
    );

    // Merge global and per-client roots
    let global_roots = state.config_manager.get_roots();
    let roots = merge_roots(&global_roots, client.roots.as_ref());

    // 2026-07-28 subscriptions/listen (SEP-2575): a long-lived response
    // stream carrying opted-in change notifications; replaces the legacy
    // GET SSE endpoint + resources/subscribe for stateless clients.
    if request.method == "subscriptions/listen" {
        return subscriptions_listen_response(&state, &client_id, allowed_servers, request);
    }

    // Intercept client capability methods before routing to gateway
    // These are requests FROM backend servers TO gateway (gateway acts as MCP client)
    match request.method.as_str() {
        "sampling/createMessage" => {
            // Check sampling permission (re-read current state, may have changed mid-connection)
            let sampling_permission = &client.mcp_sampling_permission;

            if matches!(sampling_permission, lr_config::PermissionState::Off) {
                let error = lr_mcp::protocol::JsonRpcError::custom(
                    -32601,
                    "Sampling is disabled for this client".to_string(),
                    Some(serde_json::json!({
                        "hint": "Contact administrator to set mcp_sampling_permission to 'allow' or 'ask'"
                    })),
                );

                let response = lr_mcp::protocol::JsonRpcResponse::error(
                    request.id.unwrap_or(serde_json::Value::Null),
                    error,
                );

                return send_response(&state.sse_connection_manager, &connection_key, response);
            }

            // Parse sampling request from params
            let sampling_req: lr_mcp::protocol::SamplingRequest = match request.params.as_ref() {
                Some(params) => match serde_json::from_value(params.clone()) {
                    Ok(req) => req,
                    Err(e) => {
                        let error = lr_mcp::protocol::JsonRpcError::invalid_params(format!(
                            "Invalid sampling request: {}",
                            e
                        ));
                        let response = lr_mcp::protocol::JsonRpcResponse::error(
                            request.id.unwrap_or(serde_json::Value::Null),
                            error,
                        );
                        return send_response(
                            &state.sse_connection_manager,
                            &connection_key,
                            response,
                        );
                    }
                },
                None => {
                    let error = lr_mcp::protocol::JsonRpcError::invalid_params(
                        "Missing params for sampling request".to_string(),
                    );
                    let response = lr_mcp::protocol::JsonRpcResponse::error(
                        request.id.unwrap_or(serde_json::Value::Null),
                        error,
                    );
                    return send_response(&state.sse_connection_manager, &connection_key, response);
                }
            };

            // If permission is Ask, request user approval via popup
            if matches!(sampling_permission, lr_config::PermissionState::Ask) {
                let request_id = uuid::Uuid::new_v4().to_string();
                let approval_result = state
                    .sampling_approval_manager
                    .request_approval(
                        request_id,
                        "_gateway".to_string(),
                        sampling_req.clone(),
                        None,
                    )
                    .await;

                match approval_result {
                    Ok(lr_mcp::gateway::sampling_approval::SamplingApprovalAction::Allow) => {
                        // User approved, continue
                    }
                    Ok(lr_mcp::gateway::sampling_approval::SamplingApprovalAction::Deny)
                    | Err(_) => {
                        let error = lr_mcp::protocol::JsonRpcError::custom(
                            -32601,
                            "Sampling request was denied by user".to_string(),
                            None,
                        );
                        let response = lr_mcp::protocol::JsonRpcResponse::error(
                            request.id.unwrap_or(serde_json::Value::Null),
                            error,
                        );
                        return send_response(
                            &state.sse_connection_manager,
                            &connection_key,
                            response,
                        );
                    }
                }
            }

            // Branch on client mode: direct-MCP clients get sampling forwarded to
            // their external client; MCP-via-LLM routes sampling to an LLM provider.
            if client.mcp_direct_enabled() {
                {
                    // Passthrough: forward sampling request to external client via SSE notification
                    let (passthrough_id, passthrough_rx) =
                        state.sampling_passthrough_manager.create_pending(None);

                    // Send notification to external client via SSE broadcast
                    let notification = lr_mcp::protocol::JsonRpcNotification {
                        jsonrpc: "2.0".to_string(),
                        method: "sampling/createMessage".to_string(),
                        params: Some(serde_json::json!({
                            "passthrough_request_id": passthrough_id,
                            "request": request.params,
                        })),
                    };

                    // Broadcast via the MCP notification channel so SSE clients receive it
                    let _ = state
                        .mcp_notification_broadcast
                        .send(("_sampling_passthrough".to_string(), notification));

                    tracing::info!(
                        "Forwarding sampling request to external client (passthrough {})",
                        passthrough_id
                    );

                    // Wait for external client response with timeout
                    match tokio::time::timeout(std::time::Duration::from_secs(120), passthrough_rx)
                        .await
                    {
                        Ok(Ok(response_value)) => {
                            let response = lr_mcp::protocol::JsonRpcResponse::success(
                                request.id.unwrap_or(serde_json::Value::Null),
                                response_value,
                            );
                            return send_response(
                                &state.sse_connection_manager,
                                &connection_key,
                                response,
                            );
                        }
                        Ok(Err(_)) | Err(_) => {
                            let error = lr_mcp::protocol::JsonRpcError::custom(
                                -32603,
                                "Sampling passthrough timed out waiting for client response"
                                    .to_string(),
                                None,
                            );
                            let response = lr_mcp::protocol::JsonRpcResponse::error(
                                request.id.unwrap_or(serde_json::Value::Null),
                                error,
                            );
                            return send_response(
                                &state.sse_connection_manager,
                                &connection_key,
                                response,
                            );
                        }
                    }
                }
            } else {
                {
                    // MCP via LLM (and fallback): route sampling to LLM provider
                    let mut completion_req =
                        match lr_mcp::gateway::sampling::convert_sampling_to_chat_request(
                            sampling_req,
                        ) {
                            Ok(req) => req,
                            Err(e) => {
                                let error = lr_mcp::protocol::JsonRpcError::custom(
                                    -32603,
                                    format!("Failed to convert sampling request: {}", e),
                                    None,
                                );
                                let response = lr_mcp::protocol::JsonRpcResponse::error(
                                    request.id.unwrap_or(serde_json::Value::Null),
                                    error,
                                );
                                return send_response(
                                    &state.sse_connection_manager,
                                    &connection_key,
                                    response,
                                );
                            }
                        };

                    if completion_req.model.is_empty() {
                        completion_req.model = "localrouter/auto".to_string();
                    }

                    let completion_resp =
                        match state.router.complete(&client_id, completion_req).await {
                            Ok((resp, _routing_meta)) => resp,
                            Err(e) => {
                                let error = lr_mcp::protocol::JsonRpcError::custom(
                                    -32603,
                                    format!("LLM completion failed: {}", e),
                                    None,
                                );
                                let response = lr_mcp::protocol::JsonRpcResponse::error(
                                    request.id.unwrap_or(serde_json::Value::Null),
                                    error,
                                );
                                return send_response(
                                    &state.sse_connection_manager,
                                    &connection_key,
                                    response,
                                );
                            }
                        };

                    let sampling_resp =
                        match lr_mcp::gateway::sampling::convert_chat_to_sampling_response(
                            completion_resp,
                        ) {
                            Ok(resp) => resp,
                            Err(e) => {
                                let error = lr_mcp::protocol::JsonRpcError::custom(
                                    -32603,
                                    format!("Failed to convert completion response: {}", e),
                                    None,
                                );
                                let response = lr_mcp::protocol::JsonRpcResponse::error(
                                    request.id.unwrap_or(serde_json::Value::Null),
                                    error,
                                );
                                return send_response(
                                    &state.sse_connection_manager,
                                    &connection_key,
                                    response,
                                );
                            }
                        };

                    let response = lr_mcp::protocol::JsonRpcResponse::success(
                        request.id.unwrap_or(serde_json::Value::Null),
                        serde_json::to_value(sampling_resp).unwrap(),
                    );

                    return send_response(&state.sse_connection_manager, &connection_key, response);
                }
            }
        }

        _ => {
            // Continue with normal gateway handling for other methods
        }
    }

    // Handle request via gateway
    // For SSE clients, process asynchronously and return 202 immediately.
    // This prevents the MCP SDK's request timeout from firing while the gateway
    // processes slow operations (starting servers, broadcasting initialize,
    // indexing catalogs for context management, etc.).
    if state.sse_connection_manager.has_connection(&connection_key) {
        let gateway = state.mcp_gateway.clone();
        let sse_manager = state.sse_connection_manager.clone();
        let client_id_owned = client_id.clone();
        let connection_key_owned = connection_key.clone();
        let session_id_owned = session_id.clone();
        let request_id = request.id.clone();
        let request_method = request.method.clone();
        let is_initialize = request_method == "initialize";

        tracing::debug!(
            "SSE async: client={}, method={}, request_id={:?}",
            &client_id[..8.min(client_id.len())],
            request_method,
            request_id
        );

        let join_handle = tokio::spawn(async move {
            // Overall timeout: must complete before the MCP SDK's client-side timeout (60s)
            let gateway_timeout = tokio::time::Duration::from_secs(15);
            let result = tokio::time::timeout(
                gateway_timeout,
                gateway.handle_request_with_skills(
                    &client_id_owned,
                    session_id_owned.as_deref(),
                    allowed_servers,
                    roots,
                    client.mcp_permissions.clone(),
                    client.skills_permissions.clone(),
                    client.name.clone(),
                    client.marketplace_permission.clone(),
                    client.coding_agent_permission.clone(),
                    client.coding_agent_type,
                    Some(lr_config::ContextManagementOverrides {
                        context_management_enabled: client.context_management_enabled,
                        catalog_compression_enabled: client.catalog_compression_enabled,
                    }),
                    client.mcp_sampling_permission.clone(),
                    client.mcp_elicitation_permission.clone(),
                    client.memory_enabled,
                    client.effective_client_mode(),
                    request,
                    None, // monitor_session_id
                ),
            )
            .await;

            let response = match result {
                Ok(Ok(response)) => response,
                Ok(Err(err)) => {
                    tracing::error!(
                        "Gateway error: client={}, method={}, error={}",
                        client_id_owned,
                        request_method,
                        err
                    );
                    lr_mcp::protocol::JsonRpcResponse::error(
                        request_id.unwrap_or(serde_json::Value::Null),
                        lr_mcp::protocol::JsonRpcError::internal_error(format!(
                            "Gateway error: {}",
                            err
                        )),
                    )
                }
                Err(_) => {
                    tracing::error!(
                        "Gateway timeout ({}s): client={}, method={}",
                        gateway_timeout.as_secs(),
                        client_id_owned,
                        request_method
                    );
                    lr_mcp::protocol::JsonRpcResponse::error(
                        request_id.unwrap_or(serde_json::Value::Null),
                        lr_mcp::protocol::JsonRpcError::internal_error(format!(
                            "Gateway request timed out after {}s",
                            gateway_timeout.as_secs()
                        )),
                    )
                }
            };

            if !sse_manager.send_response(&connection_key_owned, response) {
                tracing::error!(
                    "Failed to send response via SSE: connection_key={}, method={}",
                    connection_key_owned,
                    request_method
                );
            }
        });

        // Register the task handle so it can be aborted if the client
        // sends a new initialize (e.g., after SSE reconnection).
        if is_initialize {
            state
                .sse_connection_manager
                .register_gateway_task(&connection_key, join_handle.abort_handle());
        }

        (axum::http::StatusCode::ACCEPTED, "").into_response()
    } else {
        // Detect stateless (2026-07-28) peers by header or body _meta
        let stateless_peer = stateless_transport
            || lr_mcp::protocol::RequestClientMeta::from_params(request.params.as_ref())
                .revision()
                .is_some_and(|r| r.is_stateless());

        if stateless_peer {
            // Stateless peer without an SSE push channel: run with MRTR
            // support — a backend elicitation that needs this client's input
            // parks the call and returns input_required (SEP-2322).
            return run_stateless_with_mrtr(
                state,
                client_id,
                client,
                session_id,
                allowed_servers,
                roots,
                request,
            )
            .await;
        }

        // No SSE connection — process synchronously and return response in body
        match state
            .mcp_gateway
            .handle_request_with_skills(
                &client_id,
                session_id.as_deref(),
                allowed_servers,
                roots,
                client.mcp_permissions.clone(),
                client.skills_permissions.clone(),
                client.name.clone(),
                client.marketplace_permission.clone(),
                client.coding_agent_permission.clone(),
                client.coding_agent_type,
                Some(lr_config::ContextManagementOverrides {
                    context_management_enabled: client.context_management_enabled,
                    catalog_compression_enabled: client.catalog_compression_enabled,
                }),
                client.mcp_sampling_permission.clone(),
                client.mcp_elicitation_permission.clone(),
                client.memory_enabled,
                client.effective_client_mode(),
                request,
                None, // monitor_session_id
            )
            .await
        {
            Ok(response) => send_response(&state.sse_connection_manager, &connection_key, response),
            Err(err) => {
                tracing::error!("Gateway error for client {}: {}", client_id, err);
                ApiErrorResponse::internal_error(format!("Gateway error: {}", err)).into_response()
            }
        }
    }
}

/// Build a JSON HTTP response for a stateless (2026-07-28) peer, echoing the
/// negotiated protocol version header.
fn stateless_json_response(response: JsonRpcResponse) -> Response {
    let mut http_response = Json(response).into_response();
    if let Ok(value) =
        axum::http::HeaderValue::from_str(lr_mcp::protocol::MCP_PROTOCOL_VERSION_STATELESS)
    {
        http_response
            .headers_mut()
            .insert("mcp-protocol-version", value);
    }
    http_response
}

/// Run a stateless (2026-07-28) peer's request with MRTR support (SEP-2322).
///
/// The gateway call runs as a task. If a backend elicitation directed at
/// this client fires while the call is in flight, the task is parked in the
/// [`MrtrManager`] and the client receives `resultType: "input_required"`
/// with the pending input requests and an opaque `requestState`. The client
/// retries the call carrying `inputResponses` + `requestState`; the parked
/// task is resumed after the responses are submitted (schema-validated)
/// through the elicitation manager.
#[allow(clippy::too_many_arguments)]
async fn run_stateless_with_mrtr(
    state: AppState,
    client_id: String,
    client: lr_config::Client,
    session_id: Option<String>,
    allowed_servers: Vec<String>,
    roots: Vec<Root>,
    request: JsonRpcRequest,
) -> Response {
    let elicitation_mgr = state.mcp_gateway.get_elicitation_manager();
    // Gateway session key: per-connection id when present, else the client id
    let session_key = session_id.clone().unwrap_or_else(|| client_id.clone());
    let request_id = request.id.clone().unwrap_or(serde_json::Value::Null);

    let resume_state = request
        .params
        .as_ref()
        .and_then(|p| p.get("requestState"))
        .and_then(|v| v.as_str())
        .map(str::to_string);

    let mut task = if let Some(state_id) = resume_state {
        // Retry of a parked call: submit the provided input responses, then
        // keep waiting on the original task.
        let Some(parked) = state.mrtr_manager.resume(&state_id, &client_id) else {
            return stateless_json_response(JsonRpcResponse::error(
                request_id,
                lr_mcp::protocol::JsonRpcError::invalid_params(format!(
                    "Unknown or expired requestState: {}",
                    state_id
                )),
            ));
        };

        let responses = request
            .params
            .as_ref()
            .and_then(|p| p.get("inputResponses"))
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        for entry in &responses {
            let Some(id) = entry.get("id").and_then(|v| v.as_str()) else {
                continue;
            };
            let response_value = entry
                .get("response")
                .cloned()
                .unwrap_or(serde_json::Value::Null);

            // Client answers use the MCP elicitation result shape
            // { action, content }: accept submits the (schema-validated)
            // content; decline/cancel cancels the pending elicitation.
            let action = response_value
                .get("action")
                .and_then(|v| v.as_str())
                .unwrap_or("accept");
            let outcome = if action == "accept" {
                let data = response_value
                    .get("content")
                    .cloned()
                    .unwrap_or_else(|| response_value.clone());
                elicitation_mgr.submit_response(id, lr_mcp::protocol::ElicitationResponse { data })
            } else {
                elicitation_mgr.cancel_request(id)
            };

            if let Err(e) = outcome {
                // Keep the call parked (same requestState) so the client can
                // retry with corrected input.
                state.mrtr_manager.park(state_id.clone(), parked);
                return stateless_json_response(JsonRpcResponse::error(
                    request_id,
                    lr_mcp::protocol::JsonRpcError::invalid_params(format!(
                        "inputResponse for {} rejected: {}",
                        id, e
                    )),
                ));
            }
        }

        parked.task
    } else {
        // Fresh request: run the gateway call as a task we can park
        let gateway = state.mcp_gateway.clone();
        let client_for_task = client.clone();
        let client_id_for_task = client_id.clone();
        let session_id_for_task = session_id.clone();
        tokio::spawn(async move {
            gateway
                .handle_request_with_skills(
                    &client_id_for_task,
                    session_id_for_task.as_deref(),
                    allowed_servers,
                    roots,
                    client_for_task.mcp_permissions.clone(),
                    client_for_task.skills_permissions.clone(),
                    client_for_task.name.clone(),
                    client_for_task.marketplace_permission.clone(),
                    client_for_task.coding_agent_permission.clone(),
                    client_for_task.coding_agent_type,
                    Some(lr_config::ContextManagementOverrides {
                        context_management_enabled: client_for_task.context_management_enabled,
                        catalog_compression_enabled: client_for_task.catalog_compression_enabled,
                    }),
                    client_for_task.mcp_sampling_permission.clone(),
                    client_for_task.mcp_elicitation_permission.clone(),
                    client_for_task.memory_enabled,
                    client_for_task.effective_client_mode(),
                    request,
                    None, // monitor_session_id
                )
                .await
        })
    };

    // Race the in-flight task against elicitations directed at this session
    let mut events = state.mcp_notification_broadcast.subscribe();
    loop {
        // Catch up on elicitations that fired before (or while) we listened
        let pending = elicitation_mgr.pending_for_session(&session_key);
        if !pending.is_empty() {
            let state_id = Uuid::new_v4().to_string();
            let input_requests: Vec<serde_json::Value> = pending
                .iter()
                .map(|d| {
                    serde_json::json!({
                        "id": d.request_id,
                        "method": "elicitation/create",
                        "params": {
                            "message": d.message,
                            "requestedSchema": d.schema,
                        },
                    })
                })
                .collect();

            tracing::info!(
                "Parking stateless request for client {} ({} pending input(s), requestState={})",
                &client_id[..8.min(client_id.len())],
                input_requests.len(),
                &state_id[..8]
            );
            state.mrtr_manager.park(
                state_id.clone(),
                crate::state::ParkedMrtrCall {
                    task,
                    client_id: client_id.clone(),
                    parked_at: std::time::Instant::now(),
                },
            );

            return stateless_json_response(JsonRpcResponse::success(
                request_id,
                serde_json::json!({
                    "resultType": lr_mcp::protocol::RESULT_TYPE_INPUT_REQUIRED,
                    "requestState": state_id,
                    "inputRequests": input_requests,
                }),
            ));
        }

        tokio::select! {
            res = &mut task => {
                let response = match res {
                    Ok(Ok(response)) => response,
                    Ok(Err(err)) => {
                        tracing::error!("Gateway error for client {}: {}", client_id, err);
                        JsonRpcResponse::error(
                            request_id.clone(),
                            lr_mcp::protocol::JsonRpcError::internal_error(format!(
                                "Gateway error: {}",
                                err
                            )),
                        )
                    }
                    Err(join_err) => JsonRpcResponse::error(
                        request_id.clone(),
                        lr_mcp::protocol::JsonRpcError::internal_error(format!(
                            "Gateway task failed: {}",
                            join_err
                        )),
                    ),
                };
                return stateless_json_response(response);
            }
            event = events.recv() => {
                match event {
                    Ok((source, notification)) => {
                        let for_this_session = source == "_elicitation"
                            && notification
                                .params
                                .as_ref()
                                .and_then(|p| p.get("session_key"))
                                .and_then(|v| v.as_str())
                                == Some(session_key.as_str());
                        if for_this_session {
                            // Loop re-reads pending_for_session
                            continue;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        // Missed events; the pending re-check at loop top
                        // covers anything we lost.
                        continue;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        // Shutdown path: just wait for the task itself.
                        let response = match (&mut task).await {
                            Ok(Ok(response)) => response,
                            Ok(Err(err)) => JsonRpcResponse::error(
                                request_id.clone(),
                                lr_mcp::protocol::JsonRpcError::internal_error(format!(
                                    "Gateway error: {}",
                                    err
                                )),
                            ),
                            Err(join_err) => JsonRpcResponse::error(
                                request_id.clone(),
                                lr_mcp::protocol::JsonRpcError::internal_error(format!(
                                    "Gateway task failed: {}",
                                    join_err
                                )),
                            ),
                        };
                        return stateless_json_response(response);
                    }
                }
            }
        }
    }
}

/// The `subscriptions/listen` opt-in type for a gateway notification method
/// (2026-07-28, SEP-2575). Request-scoped notifications (progress, message)
/// stay on the response stream of the request they relate to.
fn subscription_type_for_method(method: &str) -> Option<&'static str> {
    match method {
        "notifications/tools/list_changed" => Some("toolsListChanged"),
        "notifications/prompts/list_changed" => Some("promptsListChanged"),
        "notifications/resources/list_changed" => Some("resourcesListChanged"),
        "notifications/resources/updated" => Some("resourceSubscriptions"),
        _ => None,
    }
}

/// All opt-in types a client may subscribe to.
const ALL_SUBSCRIPTION_TYPES: &[&str] = &[
    "toolsListChanged",
    "promptsListChanged",
    "resourcesListChanged",
    "resourceSubscriptions",
];

/// Transform a backend notification for delivery on a `subscriptions/listen`
/// stream: filter by opted-in type, namespace resource URIs, and tag with the
/// subscription id. Returns `None` when the notification isn't subscribed.
fn build_subscription_notification(
    server_id: &str,
    notification: &lr_mcp::protocol::JsonRpcNotification,
    requested_types: &std::collections::HashSet<String>,
    subscription_id: &str,
) -> Option<lr_mcp::protocol::JsonRpcNotification> {
    let sub_type = subscription_type_for_method(&notification.method)?;
    if !requested_types.contains(sub_type) {
        return None;
    }

    let mut params = notification
        .params
        .clone()
        .unwrap_or_else(|| serde_json::json!({}));

    // Namespace resource URIs the same way the legacy SSE stream does
    if notification.method == "notifications/resources/updated" {
        if let Some(uri) = params.get("uri").and_then(|v| v.as_str()) {
            let namespaced = format!("{}::{}", server_id, uri);
            params["uri"] = serde_json::Value::String(namespaced);
        }
    }

    if let Some(obj) = params.as_object_mut() {
        let meta = obj.entry("_meta").or_insert_with(|| serde_json::json!({}));
        if let Some(meta_obj) = meta.as_object_mut() {
            meta_obj.insert(
                lr_mcp::protocol::meta_keys::SUBSCRIPTION_ID.to_string(),
                serde_json::json!(subscription_id),
            );
        }
    }

    Some(lr_mcp::protocol::JsonRpcNotification {
        jsonrpc: "2.0".to_string(),
        method: notification.method.clone(),
        params: Some(params),
    })
}

/// Handle `subscriptions/listen` (2026-07-28): acknowledge the subscription,
/// then keep the POST response stream open, delivering opted-in change
/// notifications tagged with the subscription id.
fn subscriptions_listen_response(
    state: &AppState,
    client_id: &str,
    allowed_servers: Vec<String>,
    request: JsonRpcRequest,
) -> Response {
    // Opt-in types; an omitted list subscribes to everything.
    let requested_types: std::collections::HashSet<String> = request
        .params
        .as_ref()
        .and_then(|p| p.get("types"))
        .and_then(|t| t.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_else(|| {
            ALL_SUBSCRIPTION_TYPES
                .iter()
                .map(|s| s.to_string())
                .collect()
        });

    let subscription_id = Uuid::new_v4().to_string();
    let mut notification_rx = state.mcp_notification_broadcast.subscribe();
    let request_id = request.id.clone().unwrap_or(serde_json::Value::Null);
    let client_id = client_id.to_string();

    tracing::info!(
        "subscriptions/listen opened: client={}, subscription={}, types={:?}",
        &client_id[..8.min(client_id.len())],
        &subscription_id[..8],
        requested_types
    );

    let sse_stream = async_stream::stream! {
        // Acknowledge the subscription first
        let ack = JsonRpcResponse::success(
            request_id,
            serde_json::json!({
                "resultType": "complete",
                "subscriptionId": subscription_id,
                "types": requested_types.iter().collect::<Vec<_>>(),
            }),
        );
        if let Ok(json) = serde_json::to_string(&ack) {
            yield Ok::<_, Infallible>(Event::default().event("message").data(json));
        }

        loop {
            match notification_rx.recv().await {
                Ok((server_id, notification)) => {
                    if !allowed_servers.contains(&server_id) {
                        continue;
                    }
                    if let Some(tagged) = build_subscription_notification(
                        &server_id,
                        &notification,
                        &requested_types,
                        &subscription_id,
                    ) {
                        if let Ok(json) = serde_json::to_string(&tagged) {
                            yield Ok::<_, Infallible>(Event::default().event("message").data(json));
                        }
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(
                        "subscriptions/listen for client {} lagged, missed {} notifications",
                        client_id,
                        n
                    );
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    };

    Sse::new(sse_stream)
        .keep_alive(KeepAlive::default())
        .into_response()
}

/// Merge global and per-client roots
///
/// If client has custom roots configured, use those exclusively.
/// Otherwise, use global roots from AppConfig.
fn merge_roots(global_roots: &[RootConfig], client_roots: Option<&Vec<RootConfig>>) -> Vec<Root> {
    let roots_to_use = if let Some(client_roots) = client_roots {
        // Client has custom roots - use them exclusively
        client_roots
    } else {
        // Use global roots
        global_roots
    };

    // Convert RootConfig to Root and filter enabled
    roots_to_use
        .iter()
        .filter(|r| r.enabled)
        .map(|r| Root {
            uri: r.uri.clone(),
            name: r.name.clone(),
        })
        .collect()
}

/// Submit a response to a pending elicitation request
///
/// External clients use this endpoint to submit user responses to elicitation requests
/// they received via WebSocket notifications.
#[utoipa::path(
    post,
    path = "/mcp/elicitation/respond/{request_id}",
    tag = "mcp",
    request_body = lr_mcp::protocol::ElicitationResponse,
    responses(
        (status = 200, description = "Response submitted successfully", body = crate::types::MessageResponse),
        (status = 400, description = "Invalid request or request not found", body = crate::types::ErrorResponse),
        (status = 401, description = "Unauthorized", body = crate::types::ErrorResponse),
    ),
    security(
        ("bearer" = [])
    )
)]
pub async fn elicitation_response_handler(
    State(state): State<AppState>,
    Path(request_id): Path<String>,
    client_auth: Option<axum::Extension<ClientAuthContext>>,
    Json(response): Json<lr_mcp::protocol::ElicitationResponse>,
) -> Response {
    // Verify authentication
    if client_auth.is_none() {
        return ApiErrorResponse::unauthorized("Missing authentication").into_response();
    }

    // Submit response to elicitation manager
    match state
        .mcp_gateway
        .get_elicitation_manager()
        .submit_response(&request_id, response)
    {
        Ok(()) => {
            tracing::info!("Elicitation response submitted for request {}", request_id);
            Json(crate::types::MessageResponse {
                message: "Response submitted successfully".to_string(),
            })
            .into_response()
        }
        Err(e) => {
            tracing::warn!("Failed to submit elicitation response: {}", e);
            ApiErrorResponse::bad_request(format!("Failed to submit response: {}", e))
                .into_response()
        }
    }
}

/// Handle sampling passthrough response from external client.
/// Endpoint: POST /mcp/sampling/respond/:request_id
pub async fn sampling_passthrough_response_handler(
    State(state): State<AppState>,
    Path(request_id): Path<String>,
    client_auth: Option<axum::Extension<ClientAuthContext>>,
    Json(response): Json<serde_json::Value>,
) -> Response {
    if client_auth.is_none() {
        return ApiErrorResponse::unauthorized("Missing authentication").into_response();
    }

    match state
        .sampling_passthrough_manager
        .submit_response(&request_id, response)
    {
        Ok(()) => {
            tracing::info!(
                "Sampling passthrough response submitted for request {}",
                request_id
            );
            Json(crate::types::MessageResponse {
                message: "Response submitted successfully".to_string(),
            })
            .into_response()
        }
        Err(e) => {
            tracing::warn!("Failed to submit sampling passthrough response: {}", e);
            ApiErrorResponse::bad_request(format!("Failed to submit response: {}", e))
                .into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_merge_roots_uses_global_when_no_client_override() {
        let global_roots = vec![
            RootConfig {
                uri: "file:///global/path1".to_string(),
                name: Some("Global 1".to_string()),
                enabled: true,
            },
            RootConfig {
                uri: "file:///global/path2".to_string(),
                name: None,
                enabled: true,
            },
        ];

        let result = merge_roots(&global_roots, None);

        assert_eq!(result.len(), 2);
        assert_eq!(result[0].uri, "file:///global/path1");
        assert_eq!(result[0].name, Some("Global 1".to_string()));
        assert_eq!(result[1].uri, "file:///global/path2");
        assert_eq!(result[1].name, None);
    }

    #[test]
    fn test_merge_roots_uses_client_override_exclusively() {
        let global_roots = vec![RootConfig {
            uri: "file:///global/path".to_string(),
            name: Some("Global".to_string()),
            enabled: true,
        }];

        let client_roots = vec![RootConfig {
            uri: "file:///client/path".to_string(),
            name: Some("Client".to_string()),
            enabled: true,
        }];

        let result = merge_roots(&global_roots, Some(&client_roots));

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].uri, "file:///client/path");
        assert_eq!(result[0].name, Some("Client".to_string()));
    }

    #[test]
    fn test_merge_roots_filters_disabled() {
        let global_roots = vec![
            RootConfig {
                uri: "file:///enabled".to_string(),
                name: Some("Enabled".to_string()),
                enabled: true,
            },
            RootConfig {
                uri: "file:///disabled".to_string(),
                name: Some("Disabled".to_string()),
                enabled: false,
            },
        ];

        let result = merge_roots(&global_roots, None);

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].uri, "file:///enabled");
    }

    fn notification(
        method: &str,
        params: serde_json::Value,
    ) -> lr_mcp::protocol::JsonRpcNotification {
        lr_mcp::protocol::JsonRpcNotification {
            jsonrpc: "2.0".to_string(),
            method: method.to_string(),
            params: Some(params),
        }
    }

    fn all_types() -> std::collections::HashSet<String> {
        ALL_SUBSCRIPTION_TYPES
            .iter()
            .map(|s| s.to_string())
            .collect()
    }

    #[test]
    fn test_subscription_notification_tagged_and_delivered() {
        let n = notification("notifications/tools/list_changed", serde_json::json!({}));
        let tagged = build_subscription_notification("srv1", &n, &all_types(), "sub-123")
            .expect("subscribed type is delivered");

        assert_eq!(tagged.method, "notifications/tools/list_changed");
        let meta = &tagged.params.unwrap()["_meta"];
        assert_eq!(meta["io.modelcontextprotocol/subscriptionId"], "sub-123");
    }

    #[test]
    fn test_subscription_notification_filters_unrequested_types() {
        let n = notification("notifications/tools/list_changed", serde_json::json!({}));
        let only_resources: std::collections::HashSet<String> =
            ["resourcesListChanged".to_string()].into_iter().collect();

        assert!(build_subscription_notification("srv1", &n, &only_resources, "sub").is_none());
    }

    #[test]
    fn test_subscription_notification_ignores_request_scoped() {
        // Progress/log notifications belong to their request's response
        // stream, never the subscriptions/listen stream.
        for method in ["notifications/progress", "notifications/message"] {
            let n = notification(method, serde_json::json!({}));
            assert!(build_subscription_notification("srv1", &n, &all_types(), "sub").is_none());
        }
    }

    #[test]
    fn test_subscription_notification_namespaces_resource_uri() {
        let n = notification(
            "notifications/resources/updated",
            serde_json::json!({ "uri": "file:///a.txt" }),
        );
        let tagged = build_subscription_notification("srv1", &n, &all_types(), "sub").unwrap();
        let params = tagged.params.unwrap();
        assert_eq!(params["uri"], "srv1::file:///a.txt");
        assert_eq!(
            params["_meta"]["io.modelcontextprotocol/subscriptionId"],
            "sub"
        );
    }
}
