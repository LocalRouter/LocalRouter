//! MCP WebSocket notification routes
//!
//! Provides real-time notification forwarding from MCP servers to external clients.
//! Clients connect via WebSocket and receive notifications for servers they have access to.

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    response::{IntoResponse, Response},
};
use futures::{sink::SinkExt, stream::StreamExt};
use serde_json::json;

use super::helpers::get_enabled_client_from_manager;
use crate::middleware::client_auth::ClientAuthContext;
use crate::middleware::error::ApiErrorResponse;
use crate::state::AppState;
use lr_config::McpServerAccess;

/// WebSocket upgrade handler for MCP notifications
///
/// Establishes a WebSocket connection for receiving real-time notifications
/// from MCP servers. Clients are authenticated and only receive notifications
/// from servers they have access to.
///
/// # WebSocket Protocol
/// - Server → Client: JSON-RPC notification messages
/// - Format: `{"server_id": "...", "notification": {...}}`
/// - Client → Server: Ping/Pong for keepalive (text "ping" expects "pong")
///
/// # Authentication
/// Uses bearer token authentication (same as MCP gateway)
///
/// # Example
/// ```javascript
/// const ws = new WebSocket('ws://localhost:3625/ws', {
///   headers: { 'Authorization': 'Bearer lr-your-token' }
/// });
/// ws.onmessage = (event) => {
///   const {server_id, notification} = JSON.parse(event.data);
///   console.log(`Notification from ${server_id}:`, notification);
/// };
/// ```
#[utoipa::path(
    get,
    path = "/ws",
    tag = "mcp",
    responses(
        (status = 101, description = "WebSocket upgrade successful"),
        (status = 401, description = "Unauthorized", body = crate::types::ErrorResponse),
        (status = 403, description = "Forbidden - no MCP server access", body = crate::types::ErrorResponse)
    ),
    security(
        ("bearer" = [])
    )
)]
pub async fn mcp_websocket_handler(
    State(state): State<AppState>,
    client_auth: Option<axum::Extension<ClientAuthContext>>,
    ws: WebSocketUpgrade,
) -> Response {
    // Extract client_id from auth context
    let client_id = match client_auth {
        Some(ctx) => ctx.0.client_id.clone(),
        None => {
            return ApiErrorResponse::unauthorized("Missing authentication context")
                .into_response();
        }
    };

    // Get enabled client
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
    let all_server_ids: Vec<String> = state
        .config_manager
        .get()
        .mcp_servers
        .iter()
        .map(|s| s.id.clone())
        .collect();

    let allowed_servers: Vec<String> = match &client.mcp_server_access {
        McpServerAccess::None => vec![],
        McpServerAccess::All => all_server_ids,
        McpServerAccess::Specific(servers) => servers.clone(),
    };

    tracing::info!(
        "WebSocket connection from client {} with access to {} server(s)",
        client_id,
        allowed_servers.len()
    );

    // Upgrade to WebSocket
    ws.on_upgrade(move |socket| handle_websocket(socket, state, client_id, allowed_servers))
}

/// Handle WebSocket connection for a specific client
///
/// Uses graceful shutdown via broadcast channel to avoid abrupt task cancellation.
async fn handle_websocket(
    socket: WebSocket,
    state: AppState,
    client_id: String,
    allowed_servers: Vec<String>,
) {
    let (mut sender, mut receiver) = socket.split();

    // Subscribe to broadcast channel
    let mut notification_rx = state.mcp_notification_broadcast.subscribe();

    // Create channel for sending messages from multiple tasks
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Message>();

    // Shutdown signal for graceful termination
    let (shutdown_tx, mut shutdown_rx1) = tokio::sync::broadcast::channel::<()>(1);
    let mut shutdown_rx2 = shutdown_tx.subscribe();
    let mut shutdown_rx3 = shutdown_tx.subscribe();

    // Clone client_id for use in multiple tasks
    let client_id_forward = client_id.clone();
    let client_id_receive = client_id.clone();

    // Task 1: Forward notifications from broadcast to send channel
    let tx_clone = tx.clone();
    let forward_task = tokio::spawn(async move {
        loop {
            tokio::select! {
                biased;
                _ = shutdown_rx1.recv() => {
                    tracing::debug!("Forward task received shutdown signal for client {}", client_id_forward);
                    break;
                }
                result = notification_rx.recv() => {
                    match result {
                        Ok((server_id, notification)) => {
                            // Filter: only forward notifications from servers this client has access to
                            if !allowed_servers.contains(&server_id) {
                                continue;
                            }

                            // Create notification message
                            let message = json!({
                                "server_id": server_id,
                                "notification": notification,
                            });

                            let text = match serde_json::to_string(&message) {
                                Ok(t) => t,
                                Err(e) => {
                                    tracing::error!("Failed to serialize notification: {}", e);
                                    continue;
                                }
                            };

                            // Send to channel
                            if tx_clone.send(Message::Text(text)).is_err() {
                                tracing::debug!("Send channel closed for client {}", client_id_forward);
                                break;
                            }
                        }
                        Err(_) => {
                            // Broadcast channel closed
                            break;
                        }
                    }
                }
            }
        }
    });

    // Task 2: Handle incoming messages (ping/pong keepalive)
    let tx_clone = tx.clone();
    let shutdown_tx_clone = shutdown_tx.clone();
    let receive_task = tokio::spawn(async move {
        loop {
            tokio::select! {
                biased;
                _ = shutdown_rx2.recv() => {
                    tracing::debug!("Receive task received shutdown signal for client {}", client_id_receive);
                    break;
                }
                msg_opt = receiver.next() => {
                    match msg_opt {
                        Some(Ok(Message::Text(text))) => {
                            // Handle ping/pong for keepalive
                            if text.trim() == "ping" {
                                let _ = tx_clone.send(Message::Text("pong".to_string()));
                            }
                        }
                        Some(Ok(Message::Close(_))) => {
                            tracing::debug!(
                                "WebSocket close message received from client {}",
                                client_id_receive
                            );
                            // Signal other tasks to shutdown gracefully
                            let _ = shutdown_tx_clone.send(());
                            break;
                        }
                        Some(Err(e)) => {
                            tracing::debug!("WebSocket receive error: {}", e);
                            let _ = shutdown_tx_clone.send(());
                            break;
                        }
                        None => {
                            // Stream ended
                            let _ = shutdown_tx_clone.send(());
                            break;
                        }
                        _ => {
                            // Ignore other message types (Binary, Ping, Pong)
                        }
                    }
                }
            }
        }
    });

    // Task 3: Send messages from channel to WebSocket
    let shutdown_tx_clone = shutdown_tx.clone();
    let send_task = tokio::spawn(async move {
        loop {
            tokio::select! {
                biased;
                _ = shutdown_rx3.recv() => {
                    tracing::debug!("Send task received shutdown signal");
                    // Try to send close frame gracefully
                    let _ = sender.send(Message::Close(None)).await;
                    break;
                }
                msg_opt = rx.recv() => {
                    match msg_opt {
                        Some(msg) => {
                            if let Err(e) = sender.send(msg).await {
                                tracing::debug!("WebSocket send error (client likely disconnected): {}", e);
                                let _ = shutdown_tx_clone.send(());
                                break;
                            }
                        }
                        None => {
                            // Channel closed
                            break;
                        }
                    }
                }
            }
        }
    });

    // Wait for all tasks to complete gracefully
    let _ = tokio::join!(forward_task, receive_task, send_task);

    tracing::info!("WebSocket connection closed for client {}", client_id);
}
