//! MCP proxy routes
//!
//! Handles proxying JSON-RPC requests from external MCP clients to MCP servers.
//! Routes: POST / (unified gateway), POST /mcp/:server_id (individual server)
//! GET / and GET /mcp return SSE stream if Accept: text/event-stream, otherwise API info.

use axum::{
    extract::{Path, State},
    http::header,
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse, Response,
    },
    Json,
};
use std::convert::Infallible;
use std::time::Instant;

use super::helpers::get_enabled_client_from_manager;
use lr_config::{McpServerAccess, RootConfig};
use lr_mcp::protocol::{JsonRpcRequest, JsonRpcResponse, Root};
use lr_monitoring::mcp_metrics::McpRequestMetrics;
use crate::middleware::client_auth::ClientAuthContext;
use crate::middleware::error::ApiErrorResponse;
use crate::state::{AppState, SseConnectionManager, SseMessage};

/// Send a JSON-RPC response via SSE stream (preferred) or HTTP body (fallback)
///
/// The MCP SDK's SSEClientTransport expects responses via the SSE stream.
/// We send via SSE if a connection exists, and also include in HTTP body as fallback.
fn send_response(
    sse_manager: &SseConnectionManager,
    client_id: &str,
    response: JsonRpcResponse,
) -> Response {
    let response_id = response.id.clone();
    let has_error = response.error.is_some();

    tracing::debug!(
        "send_response called: client_id={}, response_id={:?}, has_error={}",
        client_id,
        response_id,
        has_error
    );

    // Try to send via SSE stream first (required for MCP SDK's SSEClientTransport)
    if sse_manager.send_response(client_id, response.clone()) {
        // Response sent via SSE - return 202 Accepted with empty body
        // The SDK will receive the response on the SSE stream
        tracing::debug!(
            "Response sent via SSE for client {}, returning 202 Accepted",
            client_id
        );
        (axum::http::StatusCode::ACCEPTED, "").into_response()
    } else {
        // No SSE connection - fall back to returning in HTTP body
        // This handles cases where client connects without SSE
        tracing::warn!(
            "No SSE connection for client {}, returning response in HTTP body (response_id={:?})",
            client_id,
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
        (status = 401, description = "Unauthorized", body = crate::types::ErrorResponse),
        (status = 403, description = "Forbidden - no MCP server access", body = crate::types::ErrorResponse)
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
            "LocalRouter - Unified MCP Gateway\n\
             \n\
             This endpoint supports both SSE and JSON-RPC:\n\
               GET  / (Accept: text/event-stream) - SSE notification stream\n\
               POST / - JSON-RPC requests to unified gateway\n\
             \n\
             Individual MCP servers:\n\
               GET  /mcp/{server_id} (Accept: text/event-stream) - SSE for specific server\n\
               POST /mcp/{server_id} - JSON-RPC to specific server\n\
             \n\
             Authentication: Include 'Authorization: Bearer <your-token>' header\n",
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
        tracing::debug!("Internal test client establishing unified SSE connection");
        all_server_ids
    } else {
        // Get enabled client from manager
        let client = match get_enabled_client_from_manager(&state, &client_id) {
            Ok(client) => client,
            Err(e) => return e.into_response(),
        };

        // Check MCP access mode
        if !client.mcp_server_access.has_any_access() {
            return ApiErrorResponse::forbidden(
                "Client has no MCP server access. Configure mcp_server_access in client settings.",
            )
            .into_response();
        }

        // Get allowed servers based on access mode
        match &client.mcp_server_access {
            McpServerAccess::None => vec![],
            McpServerAccess::All => all_server_ids,
            McpServerAccess::Specific(servers) => servers.clone(),
        }
    };

    tracing::debug!(
        "Unified SSE connection established for client {} with {} servers",
        client_id,
        allowed_servers.len()
    );

    // Register with SSE connection manager to receive responses
    let mut response_rx = state.sse_connection_manager.register(&client_id);

    // Subscribe to notification broadcast
    let mut notification_rx = state.mcp_notification_broadcast.subscribe();

    // Clone for cleanup
    let client_id_cleanup = client_id.clone();
    let sse_manager = state.sse_connection_manager.clone();

    // Create SSE stream that forwards both responses and notifications
    let sse_stream = async_stream::stream! {
        // Send endpoint event first (MCP SSE transport spec)
        // The data should be just the endpoint path, not a JSON object
        tracing::info!("SSE stream started for client {}, sending endpoint event", client_id);
        yield Ok::<_, Infallible>(Event::default().event("endpoint").data("/"));

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
                                            tracing::info!(
                                                "SSE stream yielding response for client {}: id={:?}, json_len={}",
                                                client_id,
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
            }
        }

        // Cleanup: unregister from SSE manager when stream ends
        sse_manager.unregister(&client_id_cleanup);
        tracing::debug!("SSE stream ended for client {}", client_id_cleanup);
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
        (status = 403, description = "Forbidden - no MCP server access", body = crate::types::ErrorResponse),
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

    // Record client activity for connection graph
    state.record_client_activity(&client_id);

    // Check for deferred loading header (used by Try it out UI)
    // Only applies to internal-test client for security - external clients use their config
    // Use lowercase header name as that's how browsers/http2 send it
    let deferred_loading_header = headers
        .get("x-deferred-loading")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);

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
        // Create a synthetic client with full MCP access for testing
        let mut test_client = lr_config::Client::new_with_strategy("Internal Test Client".to_string(), "internal-test".to_string());
        test_client.id = "internal-test".to_string();
        test_client.mcp_server_access = McpServerAccess::All;
        test_client.skills_access = lr_config::SkillsAccess::All;
        test_client.mcp_sampling_enabled = true;
        // Apply deferred loading from header for internal test client only
        test_client.mcp_deferred_loading = deferred_loading_header;
        tracing::info!(
            "Internal test client: deferred_loading={}",
            deferred_loading_header
        );
        (test_client, all_server_ids.clone())
    } else {
        // Get enabled client from manager
        let client = match get_enabled_client_from_manager(&state, &client_id) {
            Ok(client) => client,
            Err(e) => return e.into_response(),
        };

        // Check MCP access mode
        if !client.mcp_server_access.has_any_access() {
            return ApiErrorResponse::forbidden(
                "Client has no MCP server access. Configure mcp_server_access in client settings.",
            )
            .into_response();
        }

        // Get allowed servers based on access mode
        let allowed = match &client.mcp_server_access {
            McpServerAccess::None => vec![],
            McpServerAccess::All => all_server_ids.clone(),
            McpServerAccess::Specific(servers) => servers.clone(),
        };

        (client, allowed)
    };

    let request_id = request.id.clone();
    tracing::info!(
        "Gateway POST request from client {}: method={}, request_id={:?}, servers={}",
        client_id,
        request.method,
        request_id,
        allowed_servers.len()
    );

    // Merge global and per-client roots
    let global_roots = state.config_manager.get_roots();
    let roots = merge_roots(&global_roots, client.roots.as_ref());

    // Intercept client capability methods before routing to gateway
    // These are requests FROM backend servers TO gateway (gateway acts as MCP client)
    match request.method.as_str() {
        "sampling/createMessage" => {
            // Check if sampling is enabled for this client
            if !client.mcp_sampling_enabled {
                let error = lr_mcp::protocol::JsonRpcError::custom(
                    -32601,
                    "Sampling is disabled for this client".to_string(),
                    Some(serde_json::json!({
                        "hint": "Contact administrator to enable mcp_sampling_enabled for your client"
                    })),
                );

                let response = lr_mcp::protocol::JsonRpcResponse::error(
                    request.id.unwrap_or(serde_json::Value::Null),
                    error,
                );

                return send_response(&state.sse_connection_manager, &client_id, response);
            }

            // Parse sampling request from params
            let sampling_req: lr_mcp::protocol::SamplingRequest = match request.params.as_ref()
            {
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
                        return send_response(&state.sse_connection_manager, &client_id, response);
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
                    return send_response(&state.sse_connection_manager, &client_id, response);
                }
            };

            // Convert MCP sampling request to provider completion request
            let mut completion_req =
                match lr_mcp::gateway::sampling::convert_sampling_to_chat_request(sampling_req)
                {
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
                        return send_response(&state.sse_connection_manager, &client_id, response);
                    }
                };

            // Default to auto-routing if no specific model requested
            if completion_req.model.is_empty() {
                completion_req.model = "localrouter/auto".to_string();
            }

            // Call router to execute completion
            let completion_resp = match state.router.complete(&client_id, completion_req).await {
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
                    return send_response(&state.sse_connection_manager, &client_id, response);
                }
            };

            // Convert provider response back to MCP sampling response
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
                        return send_response(&state.sse_connection_manager, &client_id, response);
                    }
                };

            // Return success response
            let response = lr_mcp::protocol::JsonRpcResponse::success(
                request.id.unwrap_or(serde_json::Value::Null),
                serde_json::to_value(sampling_resp).unwrap(),
            );

            return send_response(&state.sse_connection_manager, &client_id, response);
        }

        _ => {
            // Continue with normal gateway handling for other methods
        }
    }

    // Handle request via gateway
    match state
        .mcp_gateway
        .handle_request_with_skills(
            &client_id,
            allowed_servers,
            client.mcp_deferred_loading,
            roots,
            client.skills_access.clone(),
            request,
        )
        .await
    {
        Ok(response) => send_response(&state.sse_connection_manager, &client_id, response),
        Err(err) => {
            tracing::error!("Gateway error for client {}: {}", client_id, err);
            ApiErrorResponse::internal_error(format!("Gateway error: {}", err)).into_response()
        }
    }
}

/// Individual MCP server handler (auth-based routing)
///
/// Routes JSON-RPC requests to a specific MCP server.
/// Client is identified via authentication token (no client_id in URL).
///
/// # Path Parameters
/// * `server_id` - MCP server ID to proxy to
///
/// # Request Body
/// JSON-RPC 2.0 request
///
/// # Response
/// JSON-RPC 2.0 response
#[utoipa::path(
    post,
    path = "/mcp/{server_id}",
    tag = "mcp",
    params(
        ("server_id" = String, Path, description = "MCP server ID")
    ),
    request_body = lr_mcp::protocol::JsonRpcRequest,
    responses(
        (status = 200, description = "JSON-RPC response", body = lr_mcp::protocol::JsonRpcResponse),
        (status = 401, description = "Unauthorized", body = crate::types::ErrorResponse),
        (status = 403, description = "Forbidden - no access to server", body = crate::types::ErrorResponse),
        (status = 502, description = "Bad gateway - MCP server error", body = crate::types::ErrorResponse),
        (status = 500, description = "Internal server error", body = crate::types::ErrorResponse)
    ),
    security(
        ("bearer" = [])
    )
)]
pub async fn mcp_server_handler(
    Path(server_id): Path<String>,
    State(state): State<AppState>,
    client_auth: Option<axum::Extension<ClientAuthContext>>,
    Json(message): Json<serde_json::Value>,
) -> Response {
    // Extract client_id from auth context (no URL parameter)
    let client_id = match client_auth {
        Some(ctx) => ctx.0.client_id.clone(),
        None => {
            return ApiErrorResponse::unauthorized("Missing authentication context")
                .into_response();
        }
    };

    // Use composite key (client_id:server_id) for SSE connection to support
    // same client connecting to multiple proxied servers simultaneously
    let sse_connection_key = format!("{}:{}", client_id, server_id);

    // Check if this is a response to a server-initiated request
    // Responses have "result" or "error" field but no "method" field
    if message.get("method").is_none()
        && (message.get("result").is_some() || message.get("error").is_some())
    {
        // This is a response to a server-initiated request
        match serde_json::from_value::<JsonRpcResponse>(message) {
            Ok(response) => {
                tracing::info!(
                    "Received response to server-initiated request: client={}, id={:?}",
                    client_id,
                    response.id
                );

                // Try to resolve a pending server request
                if state
                    .sse_connection_manager
                    .resolve_server_request(&sse_connection_key, response)
                {
                    return (axum::http::StatusCode::ACCEPTED, "").into_response();
                } else {
                    tracing::warn!(
                        "No pending server request found for response: client={}, server={}",
                        client_id,
                        server_id
                    );
                    return ApiErrorResponse::bad_request("No pending request for this response")
                        .into_response();
                }
            }
            Err(e) => {
                tracing::error!("Failed to parse response: {}", e);
                return ApiErrorResponse::bad_request(format!("Invalid response format: {}", e))
                    .into_response();
            }
        }
    }

    // Parse as a request
    let request: JsonRpcRequest = match serde_json::from_value(message) {
        Ok(req) => req,
        Err(e) => {
            tracing::error!("Failed to parse request: {}", e);
            return ApiErrorResponse::bad_request(format!("Invalid request format: {}", e))
                .into_response();
        }
    };

    // Handle internal test client specially (for UI testing)
    let client = if client_id == "internal-test" {
        // Create a synthetic client with full MCP access for testing
        let mut test_client = lr_config::Client::new_with_strategy("Internal Test Client".to_string(), "internal-test".to_string());
        test_client.id = "internal-test".to_string();
        test_client.mcp_server_access = McpServerAccess::All;
        test_client.mcp_sampling_enabled = true;
        test_client
    } else {
        // Get enabled client from manager
        match get_enabled_client_from_manager(&state, &client_id) {
            Ok(client) => client,
            Err(e) => return e.into_response(),
        }
    };

    // Check if client has access to this MCP server
    if !client.mcp_server_access.can_access(&server_id) {
        tracing::warn!(
            "Client {} attempted to access unauthorized MCP server {}",
            client_id,
            server_id
        );
        return ApiErrorResponse::forbidden(format!(
            "Access denied: Client is not authorized to access MCP server '{}'. Contact administrator to grant access.",
            server_id
        ))
        .into_response();
    }

    // Start server if not running
    if !state.mcp_server_manager.is_running(&server_id) {
        tracing::info!("Starting MCP server {} for request", server_id);
        if let Err(e) = state.mcp_server_manager.start_server(&server_id).await {
            return ApiErrorResponse::bad_gateway(format!("Failed to start MCP server: {}", e))
                .into_response();
        }
    }

    // Intercept client capability methods (handle by gateway, not backend server)
    // These are requests FROM backend server TO gateway (gateway acts as MCP client)
    match request.method.as_str() {
        "roots/list" => {
            // Return configured roots for this client
            let global_roots = state.config_manager.get_roots();
            let roots = merge_roots(&global_roots, client.roots.as_ref());

            let result = serde_json::json!({
                "roots": roots
            });

            let response = lr_mcp::protocol::JsonRpcResponse::success(
                request.id.unwrap_or(serde_json::Value::Null),
                result,
            );

            return send_response(&state.sse_connection_manager, &sse_connection_key, response);
        }

        "sampling/createMessage" => {
            // Check if sampling is enabled for this client
            if !client.mcp_sampling_enabled {
                let error = lr_mcp::protocol::JsonRpcError::custom(
                    -32601,
                    "Sampling is disabled for this client".to_string(),
                    Some(serde_json::json!({
                        "hint": "Contact administrator to enable mcp_sampling_enabled for your client"
                    })),
                );

                let response = lr_mcp::protocol::JsonRpcResponse::error(
                    request.id.unwrap_or(serde_json::Value::Null),
                    error,
                );

                return send_response(&state.sse_connection_manager, &sse_connection_key, response);
            }

            // Parse sampling request from params
            let sampling_req: lr_mcp::protocol::SamplingRequest =
                match request.params.as_ref() {
                    Some(params) => match serde_json::from_value(params.clone()) {
                        Ok(req) => req,
                        Err(e) => {
                            let error = lr_mcp::protocol::JsonRpcError::invalid_params(
                                format!("Invalid sampling request: {}", e),
                            );
                            let response = lr_mcp::protocol::JsonRpcResponse::error(
                                request.id.unwrap_or(serde_json::Value::Null),
                                error,
                            );
                            return send_response(
                                &state.sse_connection_manager,
                                &sse_connection_key,
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
                        return send_response(
                            &state.sse_connection_manager,
                            &sse_connection_key,
                            response,
                        );
                    }
                };

            // Convert MCP sampling request to provider completion request
            let mut completion_req =
                match lr_mcp::gateway::sampling::convert_sampling_to_chat_request(sampling_req)
                {
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
                            &sse_connection_key,
                            response,
                        );
                    }
                };

            // Default to auto-routing if no specific model requested
            if completion_req.model.is_empty() {
                completion_req.model = "localrouter/auto".to_string();
            }

            // Call router to execute completion
            let completion_resp = match state.router.complete(&client_id, completion_req).await {
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
                        &sse_connection_key,
                        response,
                    );
                }
            };

            // Convert provider response back to MCP sampling response
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
                            &sse_connection_key,
                            response,
                        );
                    }
                };

            // Return success response
            let response = lr_mcp::protocol::JsonRpcResponse::success(
                request.id.unwrap_or(serde_json::Value::Null),
                serde_json::to_value(sampling_resp).unwrap(),
            );

            return send_response(&state.sse_connection_manager, &sse_connection_key, response);
        }

        "elicitation/requestInput" => {
            // Parse elicitation request from params
            let elicitation_req: lr_mcp::protocol::ElicitationRequest =
                match request.params.as_ref() {
                    Some(params) => match serde_json::from_value(params.clone()) {
                        Ok(req) => req,
                        Err(e) => {
                            let error = lr_mcp::protocol::JsonRpcError::invalid_params(
                                format!("Invalid elicitation request: {}", e),
                            );
                            let response = lr_mcp::protocol::JsonRpcResponse::error(
                                request.id.unwrap_or(serde_json::Value::Null),
                                error,
                            );
                            return send_response(
                                &state.sse_connection_manager,
                                &sse_connection_key,
                                response,
                            );
                        }
                    },
                    None => {
                        let error = lr_mcp::protocol::JsonRpcError::invalid_params(
                            "Missing params for elicitation request".to_string(),
                        );
                        let response = lr_mcp::protocol::JsonRpcResponse::error(
                            request.id.unwrap_or(serde_json::Value::Null),
                            error,
                        );
                        return send_response(
                            &state.sse_connection_manager,
                            &sse_connection_key,
                            response,
                        );
                    }
                };

            // Get the elicitation manager from gateway
            // For now, return a helpful message that elicitation needs WebSocket support
            let error = lr_mcp::protocol::JsonRpcError::custom(
                -32601,
                "Elicitation requires WebSocket notification infrastructure".to_string(),
                Some(serde_json::json!({
                    "status": "partial",
                    "hint": "Elicitation module created but needs WebSocket event emission to notify external clients",
                    "message": elicitation_req.message,
                    "requires": "WebSocket connection for user interaction"
                })),
            );

            let response = lr_mcp::protocol::JsonRpcResponse::error(
                request.id.unwrap_or(serde_json::Value::Null),
                error,
            );

            return send_response(&state.sse_connection_manager, &sse_connection_key, response);
        }

        _ => {
            // Normal request - forward to backend server
        }
    }

    // Forward request to MCP server
    let start_time = Instant::now();
    let method = request.method.clone();

    // Log detailed info for initialize requests to help debug capability forwarding
    if method == "initialize" {
        tracing::info!(
            "Initialize request to server {}: full_params={}",
            server_id,
            serde_json::to_string(&request.params).unwrap_or_else(|_| "null".to_string())
        );
        if let Some(params) = &request.params {
            if let Some(caps) = params.get("capabilities") {
                let has_sampling = caps.get("sampling").is_some();
                let has_elicitation = caps.get("elicitation").is_some();
                let has_roots = caps.get("roots").is_some();
                tracing::info!(
                    "Proxying initialize to server {}: sampling={}, elicitation={}, roots={}",
                    server_id,
                    has_sampling,
                    has_elicitation,
                    has_roots
                );
            } else {
                tracing::warn!(
                    "Initialize request to server {} has params but NO capabilities field",
                    server_id
                );
            }
        } else {
            tracing::warn!(
                "Initialize request to server {} has NO params at all",
                server_id
            );
        }
    }

    tracing::info!(
        "Proxying JSON-RPC request to server {}: method={}, client={}",
        server_id,
        request.method,
        client_id
    );

    let response = match state
        .mcp_server_manager
        .send_request(&server_id, request)
        .await
    {
        Ok(response) => {
            // Log detailed info for initialize responses
            if method == "initialize" {
                if let Some(result) = &response.result {
                    if let Some(caps) = result.get("capabilities") {
                        tracing::info!(
                            "Backend server {} initialize response - server capabilities: {}",
                            server_id,
                            caps
                        );
                    }
                }
            }
            tracing::info!(
                "Received response from backend server {}: id={:?}, has_error={}",
                server_id,
                response.id,
                response.error.is_some()
            );
            response
        }
        Err(e) => {
            tracing::error!(
                "Backend server {} returned error: {} (client={})",
                server_id,
                e,
                client_id
            );
            return ApiErrorResponse::bad_gateway(format!("MCP server error: {}", e))
                .into_response();
        }
    };

    // Record metrics
    let latency_ms = start_time.elapsed().as_millis() as u64;
    state.metrics_collector.mcp().record(&McpRequestMetrics {
        client_id: &client_id,
        server_id: &server_id,
        method: &method,
        latency_ms,
        success: response.error.is_none(),
        error_code: response.error.as_ref().map(|e| e.code),
    });

    // Log to MCP access log
    let request_id = format!("mcp_{}", uuid::Uuid::new_v4());
    let transport = "unknown"; // TODO: Add transport detection

    if response.error.is_none() {
        if let Err(e) = state.mcp_access_logger.log_success(
            &client_id,
            &server_id,
            &method,
            latency_ms,
            transport,
            &request_id,
        ) {
            tracing::warn!("Failed to write MCP access log: {}", e);
        }
    } else if let Err(e) = state.mcp_access_logger.log_failure(
        &client_id,
        &server_id,
        &method,
        500,
        response.error.as_ref().map(|e| e.code),
        latency_ms,
        transport,
        &request_id,
    ) {
        tracing::warn!("Failed to write MCP access log: {}", e);
    }

    // sse_connection_key already defined at start of function
    send_response(&state.sse_connection_manager, &sse_connection_key, response)
}

/// SSE event stream handler for MCP server
///
/// Establishes an SSE connection to receive notifications and responses from the MCP server.
/// Used by the MCP SDK's SSEClientTransport for the server→client message channel.
///
/// # Path Parameters
/// * `server_id` - MCP server ID to connect to
///
/// # Response
/// Server-Sent Events stream with JSON-RPC notifications
#[utoipa::path(
    get,
    path = "/mcp/{server_id}",
    tag = "mcp",
    params(
        ("server_id" = String, Path, description = "MCP server ID")
    ),
    responses(
        (status = 200, description = "SSE event stream", content_type = "text/event-stream"),
        (status = 401, description = "Unauthorized", body = crate::types::ErrorResponse),
        (status = 403, description = "Forbidden - no access to server", body = crate::types::ErrorResponse),
        (status = 502, description = "Bad gateway - MCP server error", body = crate::types::ErrorResponse)
    ),
    security(("bearer" = []))
)]
pub async fn mcp_server_sse_handler(
    Path(server_id): Path<String>,
    State(state): State<AppState>,
    client_auth: Option<axum::Extension<ClientAuthContext>>,
) -> Response {
    // Extract client_id from auth context
    let client_id = match client_auth {
        Some(ctx) => ctx.0.client_id.clone(),
        None => {
            return ApiErrorResponse::unauthorized("Missing authentication context")
                .into_response();
        }
    };

    // Handle internal test client specially (for UI testing)
    let client = if client_id == "internal-test" {
        tracing::debug!(
            "Internal test client establishing SSE connection to MCP server {}",
            server_id
        );
        let mut test_client = lr_config::Client::new_with_strategy("Internal Test Client".to_string(), "internal-test".to_string());
        test_client.id = "internal-test".to_string();
        test_client.mcp_server_access = McpServerAccess::All;
        test_client.mcp_sampling_enabled = true;
        test_client
    } else {
        match get_enabled_client_from_manager(&state, &client_id) {
            Ok(client) => client,
            Err(e) => return e.into_response(),
        }
    };

    // Check if client has access to this MCP server
    if !client.mcp_server_access.can_access(&server_id) {
        tracing::warn!(
            "Client {} attempted SSE connection to unauthorized MCP server {}",
            client_id,
            server_id
        );
        return ApiErrorResponse::forbidden(format!(
            "Access denied: Client is not authorized to access MCP server '{}'.",
            server_id
        ))
        .into_response();
    }

    // Start server if not running
    if !state.mcp_server_manager.is_running(&server_id) {
        tracing::info!("Starting MCP server {} for SSE connection", server_id);
        if let Err(e) = state.mcp_server_manager.start_server(&server_id).await {
            return ApiErrorResponse::bad_gateway(format!("Failed to start MCP server: {}", e))
                .into_response();
        }
    }

    // Register notification handler to forward notifications to the broadcast channel
    // Only register once per server to avoid duplicate notifications
    if !state
        .mcp_notification_handlers_registered
        .contains_key(&server_id)
    {
        state
            .mcp_notification_handlers_registered
            .insert(server_id.clone(), true);

        let broadcast_tx = state.mcp_notification_broadcast.clone();
        let server_id_for_handler = server_id.clone();
        state.mcp_server_manager.on_notification(
            &server_id,
            std::sync::Arc::new(move |srv_id, notification| {
                let payload = (srv_id, notification);
                if let Err(e) = broadcast_tx.send(payload) {
                    tracing::trace!(
                        "No SSE clients subscribed to notifications from server {}: {}",
                        server_id_for_handler,
                        e
                    );
                }
            }),
        );
        tracing::debug!(
            "Registered notification handler for MCP server {}",
            server_id
        );
    }

    // For proxied servers, use a composite key (client_id:server_id) to allow
    // the same client to have separate SSE connections to different servers
    let sse_connection_key = format!("{}:{}", client_id, server_id);

    // Set up request callback for server-initiated requests (sampling, elicitation)
    // This forwards requests from the backend MCP server to the frontend client via SSE
    let sse_manager_for_requests = state.sse_connection_manager.clone();
    let sse_connection_key_for_requests = sse_connection_key.clone();
    let server_id_for_requests = server_id.clone();
    state.mcp_server_manager.set_request_callback(
        &server_id,
        std::sync::Arc::new(move |request| {
            let sse_manager = sse_manager_for_requests.clone();
            let connection_key = sse_connection_key_for_requests.clone();
            let srv_id = server_id_for_requests.clone();
            Box::pin(async move {
                tracing::info!(
                    "Forwarding server-initiated request to client: server={}, method={}, id={:?}",
                    srv_id,
                    request.method,
                    request.id
                );

                // Send the request to the client via SSE and wait for response
                if let Some(response_rx) = sse_manager.send_request(&connection_key, request.clone()) {
                    // Wait for the response with a timeout
                    match tokio::time::timeout(
                        std::time::Duration::from_secs(300), // 5 minute timeout for user interaction
                        response_rx,
                    )
                    .await
                    {
                        Ok(Ok(response)) => {
                            tracing::info!(
                                "Received response for server-initiated request: server={}, id={:?}",
                                srv_id,
                                response.id
                            );
                            response
                        }
                        Ok(Err(_)) => {
                            tracing::warn!(
                                "Response channel closed for server request: server={}",
                                srv_id
                            );
                            JsonRpcResponse::error(
                                request.id.unwrap_or(serde_json::Value::Null),
                                lr_mcp::protocol::JsonRpcError::custom(-32000, "Client connection closed", None),
                            )
                        }
                        Err(_) => {
                            tracing::warn!(
                                "Timeout waiting for response to server request: server={}",
                                srv_id
                            );
                            JsonRpcResponse::error(
                                request.id.unwrap_or(serde_json::Value::Null),
                                lr_mcp::protocol::JsonRpcError::custom(-32000, "Request timeout", None),
                            )
                        }
                    }
                } else {
                    tracing::warn!(
                        "No SSE connection for client to forward request: server={}",
                        srv_id
                    );
                    JsonRpcResponse::error(
                        request.id.unwrap_or(serde_json::Value::Null),
                        lr_mcp::protocol::JsonRpcError::custom(-32000, "No client connection", None),
                    )
                }
            })
        }),
    );

    tracing::debug!(
        "SSE connection established for client {} to MCP server {} (key={})",
        client_id,
        server_id,
        sse_connection_key
    );

    // Register with SSE connection manager to receive responses
    let mut response_rx = state.sse_connection_manager.register(&sse_connection_key);

    // Subscribe to notification broadcast
    let mut notification_rx = state.mcp_notification_broadcast.subscribe();
    let target_server_id = server_id.clone();

    // Clone for cleanup (use composite key for proxied servers)
    let sse_connection_key_cleanup = sse_connection_key.clone();
    let sse_manager = state.sse_connection_manager.clone();

    // Create SSE stream that forwards both responses and notifications
    let sse_stream = async_stream::stream! {
        // Send endpoint event first (MCP SSE transport spec)
        // The data should be just the endpoint path, not a JSON object
        let endpoint_path = format!("/mcp/{}", target_server_id);
        tracing::info!("SSE stream started for client {} to server {}, sending endpoint event: {}", client_id, target_server_id, endpoint_path);
        yield Ok::<_, Infallible>(Event::default().event("endpoint").data(endpoint_path));
        tracing::info!("SSE endpoint event yielded for client {} to server {}, entering select loop", client_id, target_server_id);

        loop {
            tracing::debug!("SSE select loop iteration for client {} to server {}", client_id, target_server_id);
            // Use biased select to prioritize responses over notifications
            tokio::select! {
                biased;

                // Handle responses from POST requests (high priority)
                msg = response_rx.recv() => {
                    tracing::debug!("SSE received message on response_rx for client {} to server {}", client_id, target_server_id);
                    match msg {
                        Some(sse_msg) => {
                            // Send raw JSON-RPC, not wrapped SseMessage (MCP SSE transport spec)
                            match sse_msg {
                                SseMessage::Response(response) => {
                                    let response_id = response.id.clone();
                                    match serde_json::to_string(&response) {
                                        Ok(json) => {
                                            tracing::info!(
                                                "SSE stream yielding response for client {} to server {}: id={:?}, json_len={}",
                                                client_id,
                                                target_server_id,
                                                response_id,
                                                json.len()
                                            );
                                            yield Ok::<_, Infallible>(Event::default().event("message").data(json));
                                        }
                                        Err(e) => {
                                            tracing::error!(
                                                "Failed to serialize response for SSE: {} (client={}, server={}, id={:?})",
                                                e,
                                                client_id,
                                                target_server_id,
                                                response_id
                                            );
                                        }
                                    }
                                }
                                SseMessage::Notification(notification) => {
                                    if let Ok(json) = serde_json::to_string(&notification) {
                                        tracing::debug!("SSE stream yielding notification for client {} to server {}", client_id, target_server_id);
                                        yield Ok::<_, Infallible>(Event::default().event("message").data(json));
                                    }
                                }
                                SseMessage::Request(request) => {
                                    // Server-initiated request (sampling, elicitation, roots/list)
                                    match serde_json::to_string(&request) {
                                        Ok(json) => {
                                            tracing::info!(
                                                "SSE stream yielding server-initiated request for client {} to server {}: method={}, id={:?}",
                                                client_id,
                                                target_server_id,
                                                request.method,
                                                request.id
                                            );
                                            yield Ok::<_, Infallible>(Event::default().event("message").data(json));
                                        }
                                        Err(e) => {
                                            tracing::error!(
                                                "Failed to serialize request for SSE: {} (client={}, server={}, method={})",
                                                e,
                                                client_id,
                                                target_server_id,
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
                            tracing::debug!("Response channel closed for client {} to server {}", client_id, target_server_id);
                            break;
                        }
                    }
                }

                // Handle notifications from MCP servers
                notif_result = notification_rx.recv() => {
                    match notif_result {
                        Ok((notif_server_id, notification)) => {
                            // Only forward notifications for our target server
                            // Send raw JSON-RPC notification (MCP SSE transport spec)
                            if notif_server_id == target_server_id {
                                if let Ok(json) = serde_json::to_string(&notification) {
                                    yield Ok::<_, Infallible>(Event::default().event("message").data(json));
                                }
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                            tracing::warn!("SSE client {} lagged, missed {} notifications", client_id, n);
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                            tracing::debug!("Notification broadcast closed");
                            break;
                        }
                    }
                }
            }
        }

        // Cleanup: unregister from SSE manager when stream ends
        sse_manager.unregister(&sse_connection_key_cleanup);
        tracing::debug!("SSE stream ended for client {} to server {} (key={})", client_id, target_server_id, sse_connection_key_cleanup);
    };

    Sse::new(sse_stream)
        .keep_alive(KeepAlive::default())
        .into_response()
}

/// Streaming MCP server handler
///
/// Routes streaming JSON-RPC requests to a specific MCP server.
/// Returns Server-Sent Events (SSE) stream with chunks.
/// Request must include "stream": true in params.
///
/// # Path Parameters
/// * `server_id` - MCP server ID to proxy to
///
/// # Request Body
/// JSON-RPC 2.0 request with "stream": true in params
///
/// # Response
/// Server-Sent Events stream with StreamingChunk data
#[utoipa::path(
    post,
    path = "/mcp/{server_id}/stream",
    tag = "mcp",
    params(
        ("server_id" = String, Path, description = "MCP server ID")
    ),
    request_body = lr_mcp::protocol::JsonRpcRequest,
    responses(
        (status = 200, description = "SSE stream of chunks", content_type = "text/event-stream"),
        (status = 401, description = "Unauthorized", body = crate::types::ErrorResponse),
        (status = 403, description = "Forbidden - no access to server", body = crate::types::ErrorResponse),
        (status = 400, description = "Bad request - streaming not supported", body = crate::types::ErrorResponse),
        (status = 502, description = "Bad gateway - MCP server error", body = crate::types::ErrorResponse),
        (status = 500, description = "Internal server error", body = crate::types::ErrorResponse)
    ),
    security(
        ("bearer" = [])
    )
)]
pub async fn mcp_server_streaming_handler(
    Path(server_id): Path<String>,
    State(state): State<AppState>,
    client_auth: Option<axum::Extension<ClientAuthContext>>,
    Json(mut request): Json<JsonRpcRequest>,
) -> Response {
    // Extract client context
    let client_id = match client_auth {
        Some(ctx) => ctx.0.client_id.clone(),
        None => {
            return ApiErrorResponse::unauthorized("Missing authentication context")
                .into_response();
        }
    };

    // Handle internal test client specially (for UI testing)
    let client = if client_id == "internal-test" {
        let mut test_client = lr_config::Client::new_with_strategy("Internal Test Client".to_string(), "internal-test".to_string());
        test_client.id = "internal-test".to_string();
        test_client.mcp_server_access = McpServerAccess::All;
        test_client.mcp_sampling_enabled = true;
        test_client
    } else {
        match get_enabled_client_from_manager(&state, &client_id) {
            Ok(client) => client,
            Err(e) => return e.into_response(),
        }
    };

    // Check if client has access to this MCP server
    if !client.mcp_server_access.can_access(&server_id) {
        return ApiErrorResponse::forbidden(format!(
            "Access denied: Client is not authorized to access MCP server '{}'. Contact administrator to grant access.",
            server_id
        ))
        .into_response();
    }

    // Check if server supports streaming
    if !state.mcp_server_manager.supports_streaming(&server_id) {
        return ApiErrorResponse::bad_request(
            "Streaming not supported for this server's transport type",
        )
        .into_response();
    }

    // Ensure "stream": true is in params
    if let Some(params) = request.params.as_mut() {
        if let Some(obj) = params.as_object_mut() {
            obj.insert("stream".to_string(), serde_json::json!(true));
        }
    }

    let method = request.method.clone();
    tracing::debug!(
        "Streaming request from client {} to server {}: method={}",
        client_id,
        server_id,
        method
    );

    // Get stream from manager
    let chunk_stream = match state
        .mcp_server_manager
        .stream_request(&server_id, request)
        .await
    {
        Ok(stream) => stream,
        Err(err) => {
            tracing::error!(
                "Streaming request failed for client {} to server {}: {}",
                client_id,
                server_id,
                err
            );
            return ApiErrorResponse::internal_error(format!("Streaming request failed: {}", err))
                .into_response();
        }
    };

    // Convert to SSE stream
    let sse_stream = async_stream::stream! {
        use futures_util::StreamExt;

        let mut pinned_stream = Box::pin(chunk_stream);

        while let Some(chunk_result) = pinned_stream.next().await {
            match chunk_result {
                Ok(chunk) => {
                    // Serialize chunk to JSON
                    match serde_json::to_string(&chunk) {
                        Ok(json) => {
                            yield Ok::<_, Infallible>(Event::default().data(json));
                        }
                        Err(e) => {
                            tracing::error!("Failed to serialize chunk: {}", e);
                            break;
                        }
                    }
                }
                Err(e) => {
                    tracing::error!("Stream error: {}", e);
                    // Send error event
                    let error_event = serde_json::json!({
                        "error": e.to_string()
                    });
                    if let Ok(json) = serde_json::to_string(&error_event) {
                        yield Ok::<_, Infallible>(Event::default().event("error").data(json));
                    }
                    break;
                }
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
