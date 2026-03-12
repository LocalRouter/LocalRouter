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

use super::helpers::{check_mcp_access, get_enabled_client_from_manager};
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
        if let Err(e) = check_mcp_access(&client) {
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
                                // Namespace the notification for the unified gateway
                                let namespaced_notification = lr_mcp::protocol::JsonRpcNotification {
                                    jsonrpc: notification.jsonrpc.clone(),
                                    method: format!("{}::{}", server_id, notification.method),
                                    params: notification.params.clone(),
                                };
                                // Send raw JSON-RPC notification (MCP SSE transport spec)
                                if let Ok(json) = serde_json::to_string(&namespaced_notification) {
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

        // Cleanup: unregister from SSE manager when stream ends
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
    query: Option<Query<McpQueryParams>>,
    Json(request): Json<JsonRpcRequest>,
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
    let session_id = query.and_then(|q| q.0.session_id);
    // Connection key for SSE routing: session_id if available, otherwise client_id
    let connection_key = session_id.as_deref().unwrap_or(&client_id).to_string();

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
        if let Err(e) = check_mcp_access(&client) {
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

            // Branch on client mode
            match client.client_mode {
                lr_config::ClientMode::Both | lr_config::ClientMode::McpOnly => {
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

                _ => {
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
                            Ok(resp) => resp,
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
                    request,
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
                request,
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
}
