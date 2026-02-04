//! MCP gateway routes
//!
//! Handles proxying JSON-RPC requests from external MCP clients to MCP servers.
//! All requests go through the unified gateway at POST /.
//! GET / returns SSE stream if Accept: text/event-stream, otherwise API info.

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

use super::helpers::get_enabled_client_from_manager;
use crate::middleware::client_auth::ClientAuthContext;
use crate::middleware::error::ApiErrorResponse;
use crate::state::{AppState, SseConnectionManager, SseMessage};
use lr_config::RootConfig;
use lr_mcp::protocol::{JsonRpcRequest, JsonRpcResponse, Root};

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
             Authentication: Include 'Authorization: Bearer <your-token>' header\n\
             Use X-MCP-Access header to control server access: 'all', 'none', or a specific server ID\n",
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

        // Check MCP access using mcp_permissions (hierarchical)
        if !client.mcp_permissions.global.is_enabled() && client.mcp_permissions.servers.is_empty() {
            return ApiErrorResponse::forbidden(
                "Client has no MCP server access. Configure mcp_permissions in client settings.",
            )
            .into_response();
        }

        // Get allowed servers based on mcp_permissions
        // If global is enabled, allow all servers; otherwise filter by server-level permissions
        if client.mcp_permissions.global.is_enabled() {
            all_server_ids
        } else {
            // Filter to only servers with explicit Allow/Ask permission
            all_server_ids
                .into_iter()
                .filter(|server_id| client.mcp_permissions.resolve_server(server_id).is_enabled())
                .collect()
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

    // Check for headers used by Try it out UI
    // Only applies to internal-test client for security - external clients use their config
    // Use lowercase header name as that's how browsers/http2 send it
    let deferred_loading_header = headers
        .get("x-deferred-loading")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);

    let mcp_access_header = headers
        .get("x-mcp-access")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("all");

    let skills_access_header = headers
        .get("x-skills-access")
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
        // Apply deferred loading from header for internal test client only
        test_client.mcp_deferred_loading = deferred_loading_header;

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
            test_client.mcp_permissions.servers.insert(server_id.clone(), lr_config::PermissionState::Allow);
            vec![server_id]
        };

        // Apply skills access from header using skills_permissions
        match &skills_access_header {
            Some(v) if v.eq_ignore_ascii_case("all") => {
                test_client.skills_permissions.global = lr_config::PermissionState::Allow;
            }
            Some(skill_name) if !skill_name.is_empty() => {
                test_client.skills_permissions.global = lr_config::PermissionState::Off;
                test_client.skills_permissions.skills.insert(skill_name.clone(), lr_config::PermissionState::Allow);
            }
            _ => {
                test_client.skills_permissions.global = lr_config::PermissionState::Off;
            }
        }

        tracing::info!(
            "Internal test client: deferred_loading={}, mcp_access={}, skills_access={:?}",
            deferred_loading_header,
            mcp_access_header,
            skills_access_header,
        );
        (test_client, allowed)
    } else {
        // Get enabled client from manager
        let client = match get_enabled_client_from_manager(&state, &client_id) {
            Ok(client) => client,
            Err(e) => return e.into_response(),
        };

        // Check MCP access using mcp_permissions (hierarchical)
        if !client.mcp_permissions.global.is_enabled() && client.mcp_permissions.servers.is_empty() {
            return ApiErrorResponse::forbidden(
                "Client has no MCP server access. Configure mcp_permissions in client settings.",
            )
            .into_response();
        }

        // Get allowed servers based on mcp_permissions
        let allowed = if client.mcp_permissions.global.is_enabled() {
            all_server_ids.clone()
        } else {
            // Filter to only servers with explicit Allow/Ask permission
            all_server_ids
                .iter()
                .filter(|server_id| client.mcp_permissions.resolve_server(server_id).is_enabled())
                .cloned()
                .collect()
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
                match lr_mcp::gateway::sampling::convert_sampling_to_chat_request(sampling_req) {
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
                match lr_mcp::gateway::sampling::convert_chat_to_sampling_response(completion_resp)
                {
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
            client.skills_permissions.clone(),
            client.firewall.clone(),
            client.name.clone(),
            client.marketplace_permission.clone(),
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
