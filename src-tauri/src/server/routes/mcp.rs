//! MCP proxy routes
//!
//! Handles proxying JSON-RPC requests from external MCP clients to MCP servers.
//! Routes: POST / (unified gateway), POST /mcp/:server_id (individual server)

use axum::{
    extract::{Path, State},
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse, Response,
    },
    Json,
};
use std::convert::Infallible;
use std::time::Instant;

use super::helpers::get_enabled_client_from_manager;
use crate::config::{McpServerAccess, RootConfig};
use crate::mcp::protocol::{JsonRpcRequest, Root};
use crate::monitoring::mcp_metrics::McpRequestMetrics;
use crate::server::middleware::client_auth::ClientAuthContext;
use crate::server::middleware::error::ApiErrorResponse;
use crate::server::state::AppState;

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
    request_body = crate::mcp::protocol::JsonRpcRequest,
    responses(
        (status = 200, description = "JSON-RPC response", body = crate::mcp::protocol::JsonRpcResponse),
        (status = 401, description = "Unauthorized", body = crate::server::types::ErrorResponse),
        (status = 403, description = "Forbidden - no MCP server access", body = crate::server::types::ErrorResponse),
        (status = 500, description = "Internal server error", body = crate::server::types::ErrorResponse)
    ),
    security(
        ("bearer" = [])
    )
)]
pub async fn mcp_gateway_handler(
    State(state): State<AppState>,
    client_auth: Option<axum::Extension<ClientAuthContext>>,
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
        let mut test_client = crate::config::Client::new("Internal Test Client".to_string());
        test_client.id = "internal-test".to_string();
        test_client.mcp_server_access = McpServerAccess::All;
        test_client.mcp_sampling_enabled = true;
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

    tracing::debug!(
        "Gateway request from client {} for {} servers: method={}, deferred_loading={}",
        client_id,
        allowed_servers.len(),
        request.method,
        client.mcp_deferred_loading
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
                let error = crate::mcp::protocol::JsonRpcError::custom(
                    -32601,
                    "Sampling is disabled for this client".to_string(),
                    Some(serde_json::json!({
                        "hint": "Contact administrator to enable mcp_sampling_enabled for your client"
                    })),
                );

                let response = crate::mcp::protocol::JsonRpcResponse::error(
                    request.id.unwrap_or(serde_json::Value::Null),
                    error,
                );

                return Json(response).into_response();
            }

            // Parse sampling request from params
            let sampling_req: crate::mcp::protocol::SamplingRequest =
                match request.params.as_ref() {
                    Some(params) => match serde_json::from_value(params.clone()) {
                        Ok(req) => req,
                        Err(e) => {
                            let error = crate::mcp::protocol::JsonRpcError::invalid_params(
                                format!("Invalid sampling request: {}", e),
                            );
                            let response = crate::mcp::protocol::JsonRpcResponse::error(
                                request.id.unwrap_or(serde_json::Value::Null),
                                error,
                            );
                            return Json(response).into_response();
                        }
                    },
                    None => {
                        let error = crate::mcp::protocol::JsonRpcError::invalid_params(
                            "Missing params for sampling request".to_string(),
                        );
                        let response = crate::mcp::protocol::JsonRpcResponse::error(
                            request.id.unwrap_or(serde_json::Value::Null),
                            error,
                        );
                        return Json(response).into_response();
                    }
                };

            // Convert MCP sampling request to provider completion request
            let mut completion_req =
                match crate::mcp::gateway::sampling::convert_sampling_to_chat_request(sampling_req)
                {
                    Ok(req) => req,
                    Err(e) => {
                        let error = crate::mcp::protocol::JsonRpcError::custom(
                            -32603,
                            format!("Failed to convert sampling request: {}", e),
                            None,
                        );
                        let response = crate::mcp::protocol::JsonRpcResponse::error(
                            request.id.unwrap_or(serde_json::Value::Null),
                            error,
                        );
                        return Json(response).into_response();
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
                    let error = crate::mcp::protocol::JsonRpcError::custom(
                        -32603,
                        format!("LLM completion failed: {}", e),
                        None,
                    );
                    let response = crate::mcp::protocol::JsonRpcResponse::error(
                        request.id.unwrap_or(serde_json::Value::Null),
                        error,
                    );
                    return Json(response).into_response();
                }
            };

            // Convert provider response back to MCP sampling response
            let sampling_resp =
                match crate::mcp::gateway::sampling::convert_chat_to_sampling_response(
                    completion_resp,
                ) {
                    Ok(resp) => resp,
                    Err(e) => {
                        let error = crate::mcp::protocol::JsonRpcError::custom(
                            -32603,
                            format!("Failed to convert completion response: {}", e),
                            None,
                        );
                        let response = crate::mcp::protocol::JsonRpcResponse::error(
                            request.id.unwrap_or(serde_json::Value::Null),
                            error,
                        );
                        return Json(response).into_response();
                    }
                };

            // Return success response
            let response = crate::mcp::protocol::JsonRpcResponse::success(
                request.id.unwrap_or(serde_json::Value::Null),
                serde_json::to_value(sampling_resp).unwrap(),
            );

            return Json(response).into_response();
        }

        _ => {
            // Continue with normal gateway handling for other methods
        }
    }

    // Handle request via gateway
    match state
        .mcp_gateway
        .handle_request(
            &client_id,
            allowed_servers,
            client.mcp_deferred_loading,
            roots,
            request,
        )
        .await
    {
        Ok(response) => Json(response).into_response(),
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
    request_body = crate::mcp::protocol::JsonRpcRequest,
    responses(
        (status = 200, description = "JSON-RPC response", body = crate::mcp::protocol::JsonRpcResponse),
        (status = 401, description = "Unauthorized", body = crate::server::types::ErrorResponse),
        (status = 403, description = "Forbidden - no access to server", body = crate::server::types::ErrorResponse),
        (status = 502, description = "Bad gateway - MCP server error", body = crate::server::types::ErrorResponse),
        (status = 500, description = "Internal server error", body = crate::server::types::ErrorResponse)
    ),
    security(
        ("bearer" = [])
    )
)]
pub async fn mcp_server_handler(
    Path(server_id): Path<String>,
    State(state): State<AppState>,
    client_auth: Option<axum::Extension<ClientAuthContext>>,
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

    // Handle internal test client specially (for UI testing)
    let client = if client_id == "internal-test" {
        // Create a synthetic client with full MCP access for testing
        let mut test_client = crate::config::Client::new("Internal Test Client".to_string());
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

            let response = crate::mcp::protocol::JsonRpcResponse::success(
                request.id.unwrap_or(serde_json::Value::Null),
                result,
            );

            return Json(response).into_response();
        }

        "sampling/createMessage" => {
            // Check if sampling is enabled for this client
            if !client.mcp_sampling_enabled {
                let error = crate::mcp::protocol::JsonRpcError::custom(
                    -32601,
                    "Sampling is disabled for this client".to_string(),
                    Some(serde_json::json!({
                        "hint": "Contact administrator to enable mcp_sampling_enabled for your client"
                    })),
                );

                let response = crate::mcp::protocol::JsonRpcResponse::error(
                    request.id.unwrap_or(serde_json::Value::Null),
                    error,
                );

                return Json(response).into_response();
            }

            // Parse sampling request from params
            let sampling_req: crate::mcp::protocol::SamplingRequest =
                match request.params.as_ref() {
                    Some(params) => match serde_json::from_value(params.clone()) {
                        Ok(req) => req,
                        Err(e) => {
                            let error = crate::mcp::protocol::JsonRpcError::invalid_params(
                                format!("Invalid sampling request: {}", e),
                            );
                            let response = crate::mcp::protocol::JsonRpcResponse::error(
                                request.id.unwrap_or(serde_json::Value::Null),
                                error,
                            );
                            return Json(response).into_response();
                        }
                    },
                    None => {
                        let error = crate::mcp::protocol::JsonRpcError::invalid_params(
                            "Missing params for sampling request".to_string(),
                        );
                        let response = crate::mcp::protocol::JsonRpcResponse::error(
                            request.id.unwrap_or(serde_json::Value::Null),
                            error,
                        );
                        return Json(response).into_response();
                    }
                };

            // Convert MCP sampling request to provider completion request
            let mut completion_req =
                match crate::mcp::gateway::sampling::convert_sampling_to_chat_request(sampling_req)
                {
                    Ok(req) => req,
                    Err(e) => {
                        let error = crate::mcp::protocol::JsonRpcError::custom(
                            -32603,
                            format!("Failed to convert sampling request: {}", e),
                            None,
                        );
                        let response = crate::mcp::protocol::JsonRpcResponse::error(
                            request.id.unwrap_or(serde_json::Value::Null),
                            error,
                        );
                        return Json(response).into_response();
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
                    let error = crate::mcp::protocol::JsonRpcError::custom(
                        -32603,
                        format!("LLM completion failed: {}", e),
                        None,
                    );
                    let response = crate::mcp::protocol::JsonRpcResponse::error(
                        request.id.unwrap_or(serde_json::Value::Null),
                        error,
                    );
                    return Json(response).into_response();
                }
            };

            // Convert provider response back to MCP sampling response
            let sampling_resp =
                match crate::mcp::gateway::sampling::convert_chat_to_sampling_response(
                    completion_resp,
                ) {
                    Ok(resp) => resp,
                    Err(e) => {
                        let error = crate::mcp::protocol::JsonRpcError::custom(
                            -32603,
                            format!("Failed to convert completion response: {}", e),
                            None,
                        );
                        let response = crate::mcp::protocol::JsonRpcResponse::error(
                            request.id.unwrap_or(serde_json::Value::Null),
                            error,
                        );
                        return Json(response).into_response();
                    }
                };

            // Return success response
            let response = crate::mcp::protocol::JsonRpcResponse::success(
                request.id.unwrap_or(serde_json::Value::Null),
                serde_json::to_value(sampling_resp).unwrap(),
            );

            return Json(response).into_response();
        }

        "elicitation/requestInput" => {
            // Parse elicitation request from params
            let elicitation_req: crate::mcp::protocol::ElicitationRequest =
                match request.params.as_ref() {
                    Some(params) => match serde_json::from_value(params.clone()) {
                        Ok(req) => req,
                        Err(e) => {
                            let error = crate::mcp::protocol::JsonRpcError::invalid_params(
                                format!("Invalid elicitation request: {}", e),
                            );
                            let response = crate::mcp::protocol::JsonRpcResponse::error(
                                request.id.unwrap_or(serde_json::Value::Null),
                                error,
                            );
                            return Json(response).into_response();
                        }
                    },
                    None => {
                        let error = crate::mcp::protocol::JsonRpcError::invalid_params(
                            "Missing params for elicitation request".to_string(),
                        );
                        let response = crate::mcp::protocol::JsonRpcResponse::error(
                            request.id.unwrap_or(serde_json::Value::Null),
                            error,
                        );
                        return Json(response).into_response();
                    }
                };

            // Get the elicitation manager from gateway
            // For now, return a helpful message that elicitation needs WebSocket support
            let error = crate::mcp::protocol::JsonRpcError::custom(
                -32601,
                "Elicitation requires WebSocket notification infrastructure".to_string(),
                Some(serde_json::json!({
                    "status": "partial",
                    "hint": "Elicitation module created but needs WebSocket event emission to notify external clients",
                    "message": elicitation_req.message,
                    "requires": "WebSocket connection for user interaction"
                })),
            );

            let response = crate::mcp::protocol::JsonRpcResponse::error(
                request.id.unwrap_or(serde_json::Value::Null),
                error,
            );

            return Json(response).into_response();
        }

        _ => {
            // Normal request - forward to backend server
        }
    }

    // Forward request to MCP server
    let start_time = Instant::now();
    let method = request.method.clone();

    tracing::debug!(
        "Proxying JSON-RPC request to server {}: method={}",
        server_id,
        request.method
    );

    let response = match state
        .mcp_server_manager
        .send_request(&server_id, request)
        .await
    {
        Ok(response) => response,
        Err(e) => {
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

    Json(response).into_response()
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
    request_body = crate::mcp::protocol::JsonRpcRequest,
    responses(
        (status = 200, description = "SSE stream of chunks", content_type = "text/event-stream"),
        (status = 401, description = "Unauthorized", body = crate::server::types::ErrorResponse),
        (status = 403, description = "Forbidden - no access to server", body = crate::server::types::ErrorResponse),
        (status = 400, description = "Bad request - streaming not supported", body = crate::server::types::ErrorResponse),
        (status = 502, description = "Bad gateway - MCP server error", body = crate::server::types::ErrorResponse),
        (status = 500, description = "Internal server error", body = crate::server::types::ErrorResponse)
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
        let mut test_client = crate::config::Client::new("Internal Test Client".to_string());
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
    request_body = crate::mcp::protocol::ElicitationResponse,
    responses(
        (status = 200, description = "Response submitted successfully", body = crate::server::types::MessageResponse),
        (status = 400, description = "Invalid request or request not found", body = crate::server::types::ErrorResponse),
        (status = 401, description = "Unauthorized", body = crate::server::types::ErrorResponse),
    ),
    security(
        ("bearer" = [])
    )
)]
pub async fn elicitation_response_handler(
    State(state): State<AppState>,
    Path(request_id): Path<String>,
    client_auth: Option<axum::Extension<ClientAuthContext>>,
    Json(response): Json<crate::mcp::protocol::ElicitationResponse>,
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
            Json(crate::server::types::MessageResponse {
                message: "Response submitted successfully".to_string(),
            })
            .into_response()
        }
        Err(e) => {
            tracing::warn!("Failed to submit elicitation response: {}", e);
            ApiErrorResponse::bad_request(&format!("Failed to submit response: {}", e))
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
