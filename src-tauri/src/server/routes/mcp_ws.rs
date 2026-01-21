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

use crate::server::middleware::client_auth::ClientAuthContext;
use crate::server::middleware::error::ApiErrorResponse;
use crate::server::state::AppState;

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
/// const ws = new WebSocket('ws://localhost:3625/mcp/ws', {
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
        (status = 401, description = "Unauthorized", body = crate::server::types::ErrorResponse),
        (status = 403, description = "Forbidden - no MCP server access", body = crate::server::types::ErrorResponse)
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

    // Get client and validate
    let client = match state.client_manager.get_client(&client_id) {
        Some(client) => client,
        None => {
            return ApiErrorResponse::unauthorized("Client not found").into_response();
        }
    };

    if !client.enabled {
        return ApiErrorResponse::forbidden("Client is disabled").into_response();
    }

    // Get allowed servers (IMPORTANT: empty list = NO ACCESS)
    let allowed_servers = client.allowed_mcp_servers.clone();

    if allowed_servers.is_empty() {
        return ApiErrorResponse::forbidden(
            "Client has no MCP server access. Configure allowed_mcp_servers in client settings.",
        )
        .into_response();
    }

    tracing::info!(
        "WebSocket connection from client {} with access to {} server(s)",
        client_id,
        allowed_servers.len()
    );

    // Upgrade to WebSocket
    ws.on_upgrade(move |socket| handle_websocket(socket, state, client_id, allowed_servers))
}

/// Handle WebSocket connection for a specific client
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

    // Clone client_id for use in multiple tasks
    let client_id_forward = client_id.clone();
    let client_id_receive = client_id.clone();

    // Task 1: Forward notifications from broadcast to send channel
    let tx_clone = tx.clone();
    let mut forward_task = tokio::spawn(async move {
        while let Ok((server_id, notification)) = notification_rx.recv().await {
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
    });

    // Task 2: Handle incoming messages (ping/pong keepalive)
    let tx_clone = tx.clone();
    let mut receive_task = tokio::spawn(async move {
        while let Some(msg) = receiver.next().await {
            match msg {
                Ok(Message::Text(text)) => {
                    // Handle ping/pong for keepalive
                    if text.trim() == "ping" {
                        let _ = tx_clone.send(Message::Text("pong".to_string()));
                    }
                }
                Ok(Message::Close(_)) => {
                    tracing::debug!("WebSocket close message received from client {}", client_id_receive);
                    break;
                }
                Err(e) => {
                    tracing::debug!("WebSocket receive error: {}", e);
                    break;
                }
                _ => {
                    // Ignore other message types (Binary, Ping, Pong)
                }
            }
        }
    });

    // Task 3: Send messages from channel to WebSocket
    let mut send_task = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if let Err(e) = sender.send(msg).await {
                tracing::debug!("WebSocket send error (client likely disconnected): {}", e);
                break;
            }
        }
    });

    // Wait for any task to complete
    tokio::select! {
        _ = &mut forward_task => {
            tracing::debug!("WebSocket forward task completed for client {}", client_id);
        }
        _ = &mut receive_task => {
            tracing::debug!("WebSocket receive task completed for client {}", client_id);
        }
        _ = &mut send_task => {
            tracing::debug!("WebSocket send task completed for client {}", client_id);
        }
    }

    // Clean up remaining tasks
    forward_task.abort();
    receive_task.abort();
    send_task.abort();

    tracing::info!("WebSocket connection closed for client {}", client_id);
}
