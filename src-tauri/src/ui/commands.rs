//! Tauri command handlers
//!
//! Functions exposed to the frontend via Tauri IPC.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use lr_config::{ConfigManager, SkillsAccess, SkillsConfig};
use lr_monitoring::logger::AccessLogger;
use lr_oauth::clients::OAuthClientManager;
use lr_server::ServerManager;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};

// Re-export submodules for backward compatibility with main.rs
pub use crate::ui::commands_clients::*;
pub use crate::ui::commands_mcp::*;
pub use crate::ui::commands_providers::*;

/// Get current configuration
#[tauri::command]
pub async fn get_config(
    config_manager: State<'_, ConfigManager>,
) -> Result<serde_json::Value, String> {
    let config = config_manager.get();
    serde_json::to_value(config).map_err(|e| e.to_string())
}

/// Manually reload configuration from disk
///
/// Forces a reload of the configuration file.
/// The file watcher will automatically reload on external changes, but this command
/// can be used to force a reload on demand.
///
/// Emits "config-changed" event to all frontend listeners.
#[tauri::command]
pub async fn reload_config(config_manager: State<'_, ConfigManager>) -> Result<(), String> {
    config_manager.reload().await.map_err(|e| e.to_string())
}

// ============================================================================
// Server Configuration Commands
// ============================================================================

/// Get server configuration (host and port)
#[tauri::command]
pub async fn get_server_config(
    config_manager: State<'_, ConfigManager>,
    server_manager: State<'_, Arc<ServerManager>>,
) -> Result<ServerConfigInfo, String> {
    let config = config_manager.get();
    let actual_port = server_manager.get_actual_port();
    Ok(ServerConfigInfo {
        host: config.server.host.clone(),
        port: config.server.port,
        actual_port,
        enable_cors: config.server.enable_cors,
    })
}

/// Update server configuration
///
/// # Arguments
/// * `host` - Host/interface to listen on (e.g., "127.0.0.1", "0.0.0.0")
/// * `port` - Port number to listen on
/// * `enable_cors` - Whether to enable CORS
///
/// # Note
/// Changes are saved to configuration file but the server needs to be restarted for them to take effect.
/// Use `restart_server` command after calling this.
#[tauri::command]
pub async fn update_server_config(
    host: Option<String>,
    port: Option<u16>,
    enable_cors: Option<bool>,
    config_manager: State<'_, ConfigManager>,
) -> Result<(), String> {
    config_manager
        .update(|config| {
            if let Some(host) = host {
                config.server.host = host;
            }
            if let Some(port) = port {
                config.server.port = port;
            }
            if let Some(enable_cors) = enable_cors {
                config.server.enable_cors = enable_cors;
            }
        })
        .map_err(|e| e.to_string())?;

    // Persist to disk
    config_manager.save().await.map_err(|e| e.to_string())
}

/// Restart the web server
///
/// Stops the current server and starts a new one with the current configuration.
/// This is needed after changing server host/port settings.
#[tauri::command]
pub async fn restart_server(app: tauri::AppHandle) -> Result<(), String> {
    // Emit an event to trigger server restart
    // The main.rs will listen for this event and restart the server
    app.emit("server-restart-requested", ())
        .map_err(|e| format!("Failed to emit restart event: {}", e))?;

    Ok(())
}

/// Server configuration info for display
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfigInfo {
    pub host: String,
    pub port: u16,
    pub actual_port: Option<u16>,
    pub enable_cors: bool,
}

// ============================================================================
// Monitoring & Statistics Commands
// ============================================================================

/// Get aggregate statistics (requests, tokens, cost)
///
/// Returns statistics computed from persistent metrics database (last 90 days).
#[tauri::command]
pub async fn get_aggregate_stats(
    server_manager: State<'_, Arc<lr_server::ServerManager>>,
) -> Result<lr_server::state::AggregateStats, String> {
    // Get the app state from server manager
    let app_state = server_manager
        .get_state()
        .ok_or_else(|| "Server is not running".to_string())?;

    // Get persistent metrics from the last 90 days (all stored data)
    let now = chrono::Utc::now();
    let start = now - chrono::Duration::days(90);
    let data_points = app_state.metrics_collector.get_global_range(start, now);

    // Aggregate the metrics
    let mut total_requests = 0u64;
    let mut successful_requests = 0u64;
    let mut total_tokens = 0u64;
    let mut total_cost = 0.0f64;

    for point in data_points {
        total_requests += point.requests;
        successful_requests += point.successful_requests;
        total_tokens += point.total_tokens;
        total_cost += point.cost_usd;
    }

    Ok(lr_server::state::AggregateStats {
        total_requests,
        successful_requests,
        total_tokens,
        total_cost,
    })
}

// ============================================================================
// Network Interface Commands
// ============================================================================

/// Network interface information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkInterface {
    pub name: String,
    pub ip: String,
    pub is_loopback: bool,
}

/// Get list of network interfaces
///
/// Returns a list of all network interfaces on the system, including loopback.
/// Always includes "0.0.0.0" (all interfaces) and "127.0.0.1" (loopback) as options.
#[tauri::command]
pub async fn get_network_interfaces() -> Result<Vec<NetworkInterface>, String> {
    let mut interfaces = vec![
        NetworkInterface {
            name: "All Interfaces".to_string(),
            ip: "0.0.0.0".to_string(),
            is_loopback: false,
        },
        NetworkInterface {
            name: "Loopback".to_string(),
            ip: "127.0.0.1".to_string(),
            is_loopback: true,
        },
    ];

    // Try to get system interfaces
    if let Ok(addrs) = if_addrs::get_if_addrs() {
        for iface in addrs {
            if iface.is_loopback() {
                continue; // Skip loopback, we already added it
            }

            let ip = iface.ip().to_string();

            // Only include IPv4 addresses
            if iface.ip().is_ipv4() {
                interfaces.push(NetworkInterface {
                    name: iface.name.clone(),
                    ip,
                    is_loopback: false,
                });
            }
        }
    }

    Ok(interfaces)
}

/// Get the path to the current executable
///
/// Returns the full path to the LocalRouter binary, useful for generating
/// STDIO MCP bridge configuration instructions.
#[tauri::command]
pub async fn get_executable_path() -> Result<String, String> {
    std::env::current_exe()
        .map(|p| p.to_string_lossy().to_string())
        .map_err(|e| format!("Failed to get executable path: {}", e))
}

// ============================================================================
// Server Control Commands
// ============================================================================

/// Get the current server status
#[tauri::command]
pub async fn get_server_status(
    server_manager: State<'_, Arc<lr_server::ServerManager>>,
) -> Result<String, String> {
    let status = server_manager.get_status();
    Ok(match status {
        lr_server::ServerStatus::Stopped => "stopped".to_string(),
        lr_server::ServerStatus::Running => "running".to_string(),
    })
}

/// Stop the web server
#[tauri::command]
pub async fn stop_server(
    server_manager: State<'_, Arc<lr_server::ServerManager>>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    tracing::info!("Stop server command received");

    server_manager.stop().await;

    // Emit event to update tray icon
    let _ = app.emit("server-status-changed", "stopped");

    Ok(())
}

// ============================================================================
// OAuth Commands
// ============================================================================

/// OAuth provider information for display
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthProviderInfo {
    pub provider_id: String,
    pub provider_name: String,
}

/// List available OAuth providers
///
/// Returns a list of all OAuth providers that can be authenticated with.
#[tauri::command]
pub async fn list_oauth_providers(
    oauth_manager: State<'_, Arc<lr_providers::oauth::OAuthManager>>,
) -> Result<Vec<OAuthProviderInfo>, String> {
    let providers = oauth_manager.list_providers();
    Ok(providers
        .into_iter()
        .map(|(id, name)| OAuthProviderInfo {
            provider_id: id,
            provider_name: name,
        })
        .collect())
}

/// Start OAuth flow for a provider
///
/// # Arguments
/// * `provider_id` - The OAuth provider ID (e.g., "github-copilot", "openai-codex")
///
/// # Returns
/// * `OAuthFlowResult` with instructions for the user
#[tauri::command]
pub async fn start_oauth_flow(
    provider_id: String,
    oauth_manager: State<'_, Arc<lr_providers::oauth::OAuthManager>>,
) -> Result<lr_providers::oauth::OAuthFlowResult, String> {
    oauth_manager
        .start_oauth(&provider_id)
        .await
        .map_err(|e| e.to_string())
}

/// Poll OAuth status for a provider
///
/// # Arguments
/// * `provider_id` - The OAuth provider ID
///
/// # Returns
/// * `OAuthFlowResult::Success` when authentication is complete
/// * `OAuthFlowResult::Pending` while waiting for user action
/// * `OAuthFlowResult::Error` if authentication failed or expired
#[tauri::command]
pub async fn poll_oauth_status(
    provider_id: String,
    oauth_manager: State<'_, Arc<lr_providers::oauth::OAuthManager>>,
) -> Result<lr_providers::oauth::OAuthFlowResult, String> {
    oauth_manager
        .poll_oauth(&provider_id)
        .await
        .map_err(|e| e.to_string())
}

/// Cancel OAuth flow for a provider
///
/// # Arguments
/// * `provider_id` - The OAuth provider ID
#[tauri::command]
pub async fn cancel_oauth_flow(
    provider_id: String,
    oauth_manager: State<'_, Arc<lr_providers::oauth::OAuthManager>>,
) -> Result<(), String> {
    oauth_manager
        .cancel_oauth(&provider_id)
        .await
        .map_err(|e| e.to_string())
}

/// List providers with stored OAuth credentials
///
/// Returns a list of provider IDs that have OAuth credentials stored.
#[tauri::command]
pub async fn list_oauth_credentials(
    oauth_manager: State<'_, Arc<lr_providers::oauth::OAuthManager>>,
) -> Result<Vec<String>, String> {
    oauth_manager
        .list_authenticated_providers()
        .await
        .map_err(|e| e.to_string())
}

/// Delete OAuth credentials for a provider
///
/// # Arguments
/// * `provider_id` - The OAuth provider ID whose credentials should be deleted
#[tauri::command]
pub async fn delete_oauth_credentials(
    provider_id: String,
    oauth_manager: State<'_, Arc<lr_providers::oauth::OAuthManager>>,
) -> Result<(), String> {
    oauth_manager
        .delete_credentials(&provider_id)
        .await
        .map_err(|e| e.to_string())
}

// ============================================================================
// OAuth Client Commands (for MCP)
// ============================================================================
///
///   OAuth client information for display
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthClientInfo {
    pub id: String,
    pub name: String,
    pub client_id: String,
    pub linked_server_ids: Vec<String>,
    pub enabled: bool,
    pub created_at: String,
}

/// List all OAuth clients
#[tauri::command]
pub async fn list_oauth_clients(
    oauth_client_manager: State<'_, Arc<OAuthClientManager>>,
) -> Result<Vec<OAuthClientInfo>, String> {
    let clients = oauth_client_manager.list_clients();
    Ok(clients
        .into_iter()
        .map(|c| OAuthClientInfo {
            id: c.id.clone(),
            name: c.name.clone(),
            client_id: c.client_id.clone(),
            linked_server_ids: c.linked_server_ids.clone(),
            enabled: c.enabled,
            created_at: c.created_at.to_rfc3339(),
        })
        .collect())
}

/// Create a new OAuth client
///
/// # Arguments
/// * `name` - Optional name for the client. If None, generates "mcp-client-{number}"
///
/// # Returns
/// * Tuple of (client_id, client_secret, OAuthClientInfo)
/// * client_secret is only returned once at creation time
#[tauri::command]
pub async fn create_oauth_client(
    name: Option<String>,
    oauth_client_manager: State<'_, Arc<OAuthClientManager>>,
    config_manager: State<'_, ConfigManager>,
    app: tauri::AppHandle,
) -> Result<(String, String, OAuthClientInfo), String> {
    tracing::info!("Creating new OAuth client with name: {:?}", name);

    let (client_id, client_secret, config) = oauth_client_manager
        .create_client(name)
        .await
        .map_err(|e| e.to_string())?;

    tracing::info!("OAuth client created: {} ({})", config.name, config.id);

    // Save to config file
    config_manager
        .update(|cfg| {
            cfg.oauth_clients.push(config.clone());
        })
        .map_err(|e| e.to_string())?;

    // Persist to disk
    config_manager.save().await.map_err(|e| e.to_string())?;

    // Rebuild tray menu
    if let Err(e) = crate::ui::tray::rebuild_tray_menu(&app) {
        tracing::error!("Failed to rebuild tray menu: {}", e);
    }

    // Notify frontend
    let _ = app.emit("oauth-clients-changed", ());

    Ok((
        client_id,
        client_secret,
        OAuthClientInfo {
            id: config.id,
            name: config.name,
            client_id: config.client_id,
            linked_server_ids: config.linked_server_ids,
            enabled: config.enabled,
            created_at: config.created_at.to_rfc3339(),
        },
    ))
}

/// Get the OAuth client secret from keychain
///
/// # Arguments
/// * `id` - The OAuth client ID (internal UUID, not client_id)
///
/// # Returns
/// * The client_secret string if it exists
/// * Error if secret doesn't exist or keychain access fails
#[tauri::command]
pub async fn get_oauth_client_secret(
    id: String,
    oauth_client_manager: State<'_, Arc<OAuthClientManager>>,
) -> Result<String, String> {
    oauth_client_manager
        .get_client_secret(&id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("OAuth client secret not found in keychain: {}", id))
}

/// Delete an OAuth client
///
/// # Arguments
/// * `id` - The OAuth client ID to delete
///
/// # Returns
/// * Ok(()) if the client was deleted successfully
/// * Error if the client doesn't exist or deletion fails
#[tauri::command]
pub async fn delete_oauth_client(
    id: String,
    oauth_client_manager: State<'_, Arc<OAuthClientManager>>,
    config_manager: State<'_, ConfigManager>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    // Delete from keychain
    oauth_client_manager
        .delete_client(&id)
        .map_err(|e| e.to_string())?;

    // Remove from config file
    config_manager
        .update(|cfg| {
            cfg.oauth_clients.retain(|c| c.id != id);
        })
        .map_err(|e| e.to_string())?;

    // Persist to disk
    config_manager.save().await.map_err(|e| e.to_string())?;

    // Rebuild tray menu
    if let Err(e) = crate::ui::tray::rebuild_tray_menu(&app) {
        tracing::error!("Failed to rebuild tray menu: {}", e);
    }

    // Notify frontend
    let _ = app.emit("oauth-clients-changed", ());

    Ok(())
}

/// Update an OAuth client's name
///
/// # Arguments
/// * `id` - The OAuth client ID to update
/// * `name` - The new name for the OAuth client
///
/// # Returns
/// * Ok(()) if the update succeeded
/// * Error if the client doesn't exist or update fails
#[tauri::command]
pub async fn update_oauth_client_name(
    id: String,
    name: String,
    oauth_client_manager: State<'_, Arc<OAuthClientManager>>,
    config_manager: State<'_, ConfigManager>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    // Validate name is not empty
    if name.trim().is_empty() {
        return Err("OAuth client name cannot be empty".to_string());
    }

    // Update in memory
    oauth_client_manager
        .update_client(&id, |cfg| {
            cfg.name = name.clone();
        })
        .map_err(|e| e.to_string())?;

    // Update in config file
    config_manager
        .update(|cfg| {
            if let Some(client) = cfg.oauth_clients.iter_mut().find(|c| c.id == id) {
                client.name = name.clone();
            }
        })
        .map_err(|e| e.to_string())?;

    // Persist to disk
    config_manager.save().await.map_err(|e| e.to_string())?;

    // Rebuild tray menu
    if let Err(e) = crate::ui::tray::rebuild_tray_menu(&app) {
        tracing::error!("Failed to rebuild tray menu: {}", e);
    }

    // Notify frontend
    let _ = app.emit("oauth-clients-changed", ());

    Ok(())
}

/// Toggle an OAuth client's enabled state
///
/// # Arguments
/// * `id` - The OAuth client ID to toggle
/// * `enabled` - Whether to enable (true) or disable (false) the client
///
/// # Returns
/// * Ok(()) if the toggle succeeded
/// * Error if the client doesn't exist or toggle fails
#[tauri::command]
pub async fn toggle_oauth_client_enabled(
    id: String,
    enabled: bool,
    oauth_client_manager: State<'_, Arc<OAuthClientManager>>,
    config_manager: State<'_, ConfigManager>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    // Update in memory
    oauth_client_manager
        .update_client(&id, |cfg| {
            cfg.enabled = enabled;
        })
        .map_err(|e| e.to_string())?;

    // Update in config file
    config_manager
        .update(|cfg| {
            if let Some(client) = cfg.oauth_clients.iter_mut().find(|c| c.id == id) {
                client.enabled = enabled;
            }
        })
        .map_err(|e| e.to_string())?;

    // Persist to disk
    config_manager.save().await.map_err(|e| e.to_string())?;

    // Rebuild tray menu
    if let Err(e) = crate::ui::tray::rebuild_tray_menu(&app) {
        tracing::error!("Failed to rebuild tray menu: {}", e);
    }

    // Notify frontend
    let _ = app.emit("oauth-clients-changed", ());

    Ok(())
}

/// Link an MCP server to an OAuth client
///
/// # Arguments
/// * `client_id` - The OAuth client ID
/// * `server_id` - The MCP server ID to link
///
/// # Returns
/// * Ok(()) if linking succeeded
/// * Error if the client doesn't exist or linking fails
#[tauri::command]
pub async fn link_mcp_server(
    client_id: String,
    server_id: String,
    oauth_client_manager: State<'_, Arc<OAuthClientManager>>,
    config_manager: State<'_, ConfigManager>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    // Link in memory
    oauth_client_manager
        .link_server(&client_id, server_id.clone())
        .map_err(|e| e.to_string())?;

    // Update in config file
    config_manager
        .update(|cfg| {
            if let Some(client) = cfg.oauth_clients.iter_mut().find(|c| c.id == client_id) {
                if !client.linked_server_ids.contains(&server_id) {
                    client.linked_server_ids.push(server_id);
                }
            }
        })
        .map_err(|e| e.to_string())?;

    // Persist to disk
    config_manager.save().await.map_err(|e| e.to_string())?;

    // Notify frontend
    let _ = app.emit("oauth-clients-changed", ());

    Ok(())
}

/// Unlink an MCP server from an OAuth client
///
/// # Arguments
/// * `client_id` - The OAuth client ID
/// * `server_id` - The MCP server ID to unlink
///
/// # Returns
/// * Ok(()) if unlinking succeeded
/// * Error if the client doesn't exist or unlinking fails
#[tauri::command]
pub async fn unlink_mcp_server(
    client_id: String,
    server_id: String,
    oauth_client_manager: State<'_, Arc<OAuthClientManager>>,
    config_manager: State<'_, ConfigManager>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    // Unlink in memory
    oauth_client_manager
        .unlink_server(&client_id, &server_id)
        .map_err(|e| e.to_string())?;

    // Update in config file
    config_manager
        .update(|cfg| {
            if let Some(client) = cfg.oauth_clients.iter_mut().find(|c| c.id == client_id) {
                client.linked_server_ids.retain(|id| id != &server_id);
            }
        })
        .map_err(|e| e.to_string())?;

    // Persist to disk
    config_manager.save().await.map_err(|e| e.to_string())?;

    // Notify frontend
    let _ = app.emit("oauth-clients-changed", ());

    Ok(())
}

/// Get all MCP servers linked to an OAuth client
///
/// # Arguments
/// * `client_id` - The OAuth client ID
///
/// # Returns
/// * List of MCP server IDs linked to this client
/// * Empty list if the client doesn't exist or has no linked servers
#[tauri::command]
pub async fn get_oauth_client_linked_servers(
    client_id: String,
    oauth_client_manager: State<'_, Arc<OAuthClientManager>>,
) -> Result<Vec<String>, String> {
    if let Some(client) = oauth_client_manager.get_client(&client_id) {
        Ok(client.linked_server_ids)
    } else {
        Ok(Vec::new())
    }
}

// ============================================================================
// OpenAPI Documentation
// ============================================================================

/// Get the OpenAPI specification
///
/// Returns the complete OpenAPI 3.1 specification in JSON format.
/// This can be used to display API documentation in the UI.
/// The server URLs are dynamically updated to match the actual running server port.
///
/// # Returns
/// * Ok(String) - The OpenAPI spec as JSON
/// * Err(String) - Error message if generation fails
#[tauri::command]
pub async fn get_openapi_spec(
    server_manager: State<'_, Arc<ServerManager>>,
) -> Result<String, String> {
    let mut spec_json = lr_server::openapi::get_openapi_json().map_err(|e| e.to_string())?;

    // Get the actual server port and dynamically update the spec
    if let Some(actual_port) = server_manager.get_actual_port() {
        // Replace hardcoded port 3625 with actual port in the spec
        spec_json = spec_json.replace(":3625", &format!(":{}", actual_port));
    }

    Ok(spec_json)
}

/// Get the internal test bearer token for UI model testing
/// This token is regenerated on each app start and allows the UI to bypass API key restrictions
/// when testing models directly. Only accessible via Tauri IPC, never exposed over HTTP.
/// Use this as a regular bearer token in the Authorization header.
#[tauri::command]
pub async fn get_internal_test_token(
    server_manager: State<'_, Arc<lr_server::ServerManager>>,
) -> Result<String, String> {
    let state = server_manager
        .get_state()
        .ok_or_else(|| "Server not started".to_string())?;

    Ok(state.get_internal_test_secret())
}

/// Create a temporary test client bound to a specific routing strategy.
/// This is used by the "Try It Out" feature to test requests with specific strategies.
/// The client is created and persisted so it can be used for testing.
/// Returns the bearer token that can be used to make requests with this strategy.
#[tauri::command]
pub async fn create_test_client_for_strategy(
    strategy_id: String,
    client_manager: State<'_, Arc<lr_clients::ClientManager>>,
    config_manager: State<'_, ConfigManager>,
) -> Result<String, String> {
    // Verify strategy exists and get its actual ID
    let config = config_manager.get();
    let strategy = config
        .strategies
        .iter()
        .find(|s| s.name == strategy_id)
        .ok_or_else(|| format!("Strategy not found: {}", strategy_id))?;
    let actual_strategy_id = strategy.id.clone();

    // Create a test client with a unique name
    let test_client_name = format!("_test_strategy_{}", strategy_id);

    // Check if we already have a test client for this strategy
    let existing_clients = client_manager.list_clients();
    if let Some(existing) = existing_clients.iter().find(|c| c.name == test_client_name) {
        // Return the existing client's secret
        return client_manager
            .get_secret(&existing.id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "Failed to retrieve test client secret".to_string());
    }

    // Create a new test client with the specified strategy
    let (_client_id, secret, client) = client_manager
        .create_client(test_client_name, actual_strategy_id)
        .map_err(|e| e.to_string())?;

    // Save client to config
    config_manager
        .update(|cfg| {
            cfg.clients.push(client);
        })
        .map_err(|e| e.to_string())?;

    Ok(secret)
}

// ============================================================================
// Setup Wizard Commands
// ============================================================================

/// Check if the setup wizard has been shown
///
/// Used for first-run detection. Returns true if the wizard has been shown,
/// false if this is the first time the app is being run.
#[tauri::command]
pub async fn get_setup_wizard_shown(
    config_manager: State<'_, ConfigManager>,
) -> Result<bool, String> {
    let config = config_manager.get();
    Ok(config.setup_wizard_shown)
}

/// Mark the setup wizard as shown
///
/// Called after the user completes or dismisses the setup wizard.
/// This prevents the wizard from showing again on subsequent app launches.
#[tauri::command]
pub async fn set_setup_wizard_shown(
    config_manager: State<'_, ConfigManager>,
) -> Result<(), String> {
    config_manager
        .update(|cfg| {
            cfg.setup_wizard_shown = true;
        })
        .map_err(|e| e.to_string())?;

    config_manager.save().await.map_err(|e| e.to_string())
}

// ============================================================================
// Access Logs Commands
// ============================================================================

use std::io::{BufRead, BufReader};

/// Get LLM access logs
///
/// Reads log entries from the LLM access log files.
/// Optimized to stop early once enough entries are collected.
///
/// # Arguments
/// * `limit` - Maximum number of entries to return (default: 100)
/// * `offset` - Number of entries to skip (default: 0)
/// * `client_name` - Optional filter by client name (API key name)
/// * `provider` - Optional filter by provider
/// * `model` - Optional filter by model
///
/// # Returns
/// * List of LLM access log entries (newest first)
#[tauri::command]
pub async fn get_llm_logs(
    limit: Option<usize>,
    offset: Option<usize>,
    client_name: Option<String>,
    provider: Option<String>,
    model: Option<String>,
) -> Result<Vec<lr_monitoring::logger::AccessLogEntry>, String> {
    use std::fs;

    let limit = limit.unwrap_or(100);
    let offset = offset.unwrap_or(0);
    let target_count = offset + limit;

    // Get log directory
    let log_dir = get_log_directory().map_err(|e: lr_types::AppError| e.to_string())?;

    // Read all log files (sorted by date, newest first)
    let mut log_files = Vec::new();
    if let Ok(entries) = fs::read_dir(&log_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                if let Some(filename) = path.file_name().and_then(|n| n.to_str()) {
                    // Match LLM log files (localrouter-YYYY-MM-DD.log, not localrouter-mcp-*.log)
                    if filename.starts_with("localrouter-")
                        && !filename.starts_with("localrouter-mcp-")
                        && filename.ends_with(".log")
                    {
                        log_files.push(path);
                    }
                }
            }
        }
    }

    // Sort by filename (date) in descending order (newest files first)
    log_files.sort_by(|a, b| b.cmp(a));

    // Read and parse log entries with filtering
    // Process files from newest to oldest, collect entries in reverse order per file
    let mut entries = Vec::new();
    let mut collected_enough = false;

    for log_file in log_files {
        if collected_enough {
            break;
        }

        if let Ok(file) = fs::File::open(&log_file) {
            let reader = BufReader::new(file);
            // Collect all matching entries from this file, then reverse to get newest first
            let mut file_entries: Vec<lr_monitoring::logger::AccessLogEntry> = Vec::new();

            for line in reader.lines().map_while(Result::ok) {
                if let Ok(entry) =
                    serde_json::from_str::<lr_monitoring::logger::AccessLogEntry>(&line)
                {
                    // Apply filters
                    let matches = client_name
                        .as_ref()
                        .is_none_or(|f| entry.api_key_name == *f)
                        && provider.as_ref().is_none_or(|f| entry.provider == *f)
                        && model.as_ref().is_none_or(|f| entry.model == *f);

                    if matches {
                        file_entries.push(entry);
                    }
                }
            }

            // Reverse to get newest entries first within this file
            file_entries.reverse();
            entries.extend(file_entries);

            // Check if we have collected enough entries (with buffer for sorting)
            // We need offset + limit entries to return the correct page
            if entries.len() >= target_count {
                collected_enough = true;
            }
        }
    }

    // Sort by timestamp (newest first) to handle entries spanning midnight
    entries.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));

    // Apply offset and limit
    let entries: Vec<_> = entries.into_iter().skip(offset).take(limit).collect();

    Ok(entries)
}

/// Get MCP access logs
///
/// Reads log entries from the MCP access log files.
/// Optimized to stop early once enough entries are collected.
///
/// # Arguments
/// * `limit` - Maximum number of entries to return (default: 100)
/// * `offset` - Number of entries to skip (default: 0)
/// * `client_id` - Optional filter by client ID
/// * `server_id` - Optional filter by server ID
///
/// # Returns
/// * List of MCP access log entries (newest first)
#[tauri::command]
pub async fn get_mcp_logs(
    limit: Option<usize>,
    offset: Option<usize>,
    client_id: Option<String>,
    server_id: Option<String>,
) -> Result<Vec<lr_monitoring::mcp_logger::McpAccessLogEntry>, String> {
    use std::fs;

    let limit = limit.unwrap_or(100);
    let offset = offset.unwrap_or(0);
    let target_count = offset + limit;

    // Get log directory
    let log_dir = get_log_directory().map_err(|e: lr_types::AppError| e.to_string())?;

    // Read all MCP log files (sorted by date, newest first)
    let mut log_files = Vec::new();
    if let Ok(entries) = fs::read_dir(&log_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                if let Some(filename) = path.file_name().and_then(|n| n.to_str()) {
                    // Match MCP log files (localrouter-mcp-YYYY-MM-DD.log)
                    if filename.starts_with("localrouter-mcp-") && filename.ends_with(".log") {
                        log_files.push(path);
                    }
                }
            }
        }
    }

    // Sort by filename (date) in descending order (newest files first)
    log_files.sort_by(|a, b| b.cmp(a));

    // Read and parse log entries with filtering
    // Process files from newest to oldest, collect entries in reverse order per file
    let mut entries = Vec::new();
    let mut collected_enough = false;

    for log_file in log_files {
        if collected_enough {
            break;
        }

        if let Ok(file) = fs::File::open(&log_file) {
            let reader = BufReader::new(file);
            // Collect all matching entries from this file, then reverse to get newest first
            let mut file_entries: Vec<lr_monitoring::mcp_logger::McpAccessLogEntry> = Vec::new();

            for line in reader.lines().map_while(Result::ok) {
                if let Ok(entry) =
                    serde_json::from_str::<lr_monitoring::mcp_logger::McpAccessLogEntry>(&line)
                {
                    // Apply filters
                    let matches = client_id.as_ref().is_none_or(|f| entry.client_id == *f)
                        && server_id.as_ref().is_none_or(|f| entry.server_id == *f);

                    if matches {
                        file_entries.push(entry);
                    }
                }
            }

            // Reverse to get newest entries first within this file
            file_entries.reverse();
            entries.extend(file_entries);

            // Check if we have collected enough entries (with buffer for sorting)
            // We need offset + limit entries to return the correct page
            if entries.len() >= target_count {
                collected_enough = true;
            }
        }
    }

    // Sort by timestamp (newest first) to handle entries spanning midnight
    entries.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));

    // Apply offset and limit
    let entries: Vec<_> = entries.into_iter().skip(offset).take(limit).collect();

    Ok(entries)
}

/// Get the OS-specific log directory
///
/// Delegates to AccessLogger::get_log_directory() to avoid code duplication.
fn get_log_directory() -> Result<PathBuf, lr_types::AppError> {
    AccessLogger::get_log_directory()
}

// ============================================================================
// Model Catalog Commands
// ============================================================================

/// Catalog metadata for the frontend
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogMetadata {
    pub fetch_date: String,
    pub source: String,
    pub total_models: usize,
}

/// Catalog statistics for display
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogStats {
    pub total_models: usize,
    pub fetch_date: String,
    pub providers: HashMap<String, usize>,
    pub modalities: HashMap<String, usize>,
}

/// Get catalog metadata
#[tauri::command]
pub fn get_catalog_metadata() -> CatalogMetadata {
    use lr_catalog as catalog;

    let meta = catalog::metadata();
    CatalogMetadata {
        fetch_date: meta.fetch_date().to_rfc3339(),
        source: meta.source.to_string(),
        total_models: meta.total_models,
    }
}

/// Get catalog statistics
#[tauri::command]
pub fn get_catalog_stats() -> CatalogStats {
    use lr_catalog as catalog;
    use std::collections::HashMap;

    let meta = catalog::metadata();
    let models = catalog::models();

    // Count providers
    let mut providers: HashMap<String, usize> = HashMap::new();
    for model in models {
        if let Some((provider, _)) = model.id.split_once('/') {
            *providers.entry(provider.to_string()).or_insert(0) += 1;
        }
    }

    // Count modalities
    let mut modalities: HashMap<String, usize> = HashMap::new();
    for model in models {
        let modality = match model.modality {
            lr_catalog::Modality::Text => "text",
            lr_catalog::Modality::Multimodal => "multimodal",
            lr_catalog::Modality::Image => "image",
        };
        *modalities.entry(modality.to_string()).or_insert(0) += 1;
    }

    CatalogStats {
        total_models: meta.total_models,
        fetch_date: meta.fetch_date().to_rfc3339(),
        providers,
        modalities,
    }
}

// ============================================================================
// Pricing Override Commands
// ============================================================================

/// Get pricing override for a specific model
#[tauri::command]
pub fn get_pricing_override(
    provider: String,
    model: String,
    config_manager: State<'_, ConfigManager>,
) -> Option<lr_config::ModelPricingOverride> {
    let config = config_manager.get();
    config
        .pricing_overrides
        .get(&provider)
        .and_then(|models| models.get(&model))
        .cloned()
}

/// Set or update pricing override for a specific model
#[tauri::command]
pub fn set_pricing_override(
    provider: String,
    model: String,
    input_per_million: f64,
    output_per_million: f64,
    config_manager: State<'_, ConfigManager>,
) -> Result<(), String> {
    config_manager
        .update(|config| {
            let provider_overrides = config
                .pricing_overrides
                .entry(provider.clone())
                .or_insert_with(std::collections::HashMap::new);

            provider_overrides.insert(
                model.clone(),
                lr_config::ModelPricingOverride {
                    input_per_million,
                    output_per_million,
                },
            );
        })
        .map_err(|e| e.to_string())
}

/// Delete pricing override for a specific model
#[tauri::command]
pub fn delete_pricing_override(
    provider: String,
    model: String,
    config_manager: State<'_, ConfigManager>,
) -> Result<(), String> {
    config_manager
        .update(|config| {
            if let Some(provider_overrides) = config.pricing_overrides.get_mut(&provider) {
                provider_overrides.remove(&model);

                // Clean up empty provider entry
                if provider_overrides.is_empty() {
                    config.pricing_overrides.remove(&provider);
                }
            }
        })
        .map_err(|e| e.to_string())
}

// ============================================================================
// Tray Graph Settings Commands
// ============================================================================

/// Tray graph settings response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrayGraphSettings {
    /// Whether dynamic tray graph is enabled
    pub enabled: bool,
    /// Refresh rate in seconds (1, 10, or 60)
    pub refresh_rate_secs: u64,
}

/// Get current tray graph settings
#[tauri::command]
pub fn get_tray_graph_settings(
    config_manager: State<'_, ConfigManager>,
) -> Result<TrayGraphSettings, String> {
    let config = config_manager.get();
    Ok(TrayGraphSettings {
        enabled: config.ui.tray_graph_enabled,
        refresh_rate_secs: config.ui.tray_graph_refresh_rate_secs,
    })
}

/// Update tray graph settings (refresh rate only - graph is always enabled)
#[tauri::command]
pub async fn update_tray_graph_settings(
    enabled: bool, // Kept for backwards compatibility, but ignored (always enabled)
    refresh_rate_secs: u64,
    config_manager: State<'_, ConfigManager>,
    tray_graph_manager: State<'_, Arc<crate::ui::tray::TrayGraphManager>>,
) -> Result<(), String> {
    // Validate parameters - only allow 1, 10, or 60
    if ![1, 10, 60].contains(&refresh_rate_secs) {
        return Err("refresh_rate_secs must be 1 (Fast), 10 (Medium), or 60 (Slow)".to_string());
    }

    let _ = enabled; // Suppress unused warning - kept for API compatibility

    // Update configuration (tray_graph_enabled is always true now)
    config_manager
        .update(|config| {
            config.ui.tray_graph_enabled = true; // Always enabled
            config.ui.tray_graph_refresh_rate_secs = refresh_rate_secs;
        })
        .map_err(|e| e.to_string())?;

    // Save to disk
    config_manager.save().await.map_err(|e| e.to_string())?;

    // Update tray graph manager (this triggers a refresh with new rate)
    let new_config = config_manager.get().ui.clone();
    tray_graph_manager.update_config(new_config);

    Ok(())
}

/// Get user's home directory
#[tauri::command]
pub fn get_home_dir() -> Result<String, String> {
    dirs::home_dir()
        .ok_or_else(|| "Failed to get home directory".to_string())?
        .to_str()
        .ok_or_else(|| "Invalid home directory path".to_string())
        .map(|s| s.to_string())
}

/// Get current app version
#[tauri::command]
pub fn get_app_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// Get update configuration
#[tauri::command]
pub async fn get_update_config(
    config_manager: State<'_, ConfigManager>,
) -> Result<lr_config::UpdateConfig, String> {
    Ok(config_manager.get().update.clone())
}

/// Update update configuration
#[tauri::command]
pub async fn update_update_config(
    mode: lr_config::UpdateMode,
    check_interval_days: u64,
    config_manager: State<'_, ConfigManager>,
) -> Result<(), String> {
    // Validate parameters
    if check_interval_days == 0 || check_interval_days > 365 {
        return Err("check_interval_days must be between 1 and 365".to_string());
    }

    // Update configuration
    config_manager
        .update(|config| {
            config.update.mode = mode;
            config.update.check_interval_days = check_interval_days;
        })
        .map_err(|e| e.to_string())?;

    // Save to disk
    config_manager.save().await.map_err(|e| e.to_string())?;

    Ok(())
}

/// Mark that an update check was performed (save timestamp)
/// This is called by the frontend after it performs an update check
#[tauri::command]
pub async fn mark_update_check_performed(
    config_manager: State<'_, ConfigManager>,
) -> Result<(), String> {
    crate::updater::save_last_check_timestamp(&config_manager).await
}

/// Skip a specific version (don't notify about it again), or clear skipped version if None
#[tauri::command]
pub async fn skip_update_version(
    version: Option<String>,
    config_manager: State<'_, ConfigManager>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    config_manager
        .update(|config| {
            config.update.skipped_version = version.clone();
        })
        .map_err(|e| e.to_string())?;

    config_manager.save().await.map_err(|e| e.to_string())?;

    // Only clear tray notification when skipping a version, not when clearing
    if version.is_some() {
        crate::ui::tray::set_update_available(&app, false).map_err(|e| e.to_string())?;
    }

    Ok(())
}

/// Set update notification in tray menu
#[tauri::command]
pub fn set_update_notification(app: tauri::AppHandle, available: bool) -> Result<(), String> {
    crate::ui::tray::set_update_available(&app, available).map_err(|e| e.to_string())
}

// ============================================================================
// Logging Configuration Commands
// ============================================================================

/// Logging configuration returned to the frontend
#[derive(serde::Serialize)]
pub struct LoggingConfigResponse {
    pub enabled: bool,
    pub log_dir: String,
}

/// Get logging configuration
#[tauri::command]
pub fn get_logging_config(
    config_manager: State<'_, ConfigManager>,
    access_logger: State<'_, Arc<lr_monitoring::logger::AccessLogger>>,
) -> Result<LoggingConfigResponse, String> {
    let config = config_manager.get();
    Ok(LoggingConfigResponse {
        enabled: config.logging.enable_access_log,
        log_dir: access_logger.log_dir().to_string_lossy().to_string(),
    })
}

/// Update logging configuration (enable/disable access logging)
#[tauri::command]
pub async fn update_logging_config(
    enabled: bool,
    config_manager: State<'_, ConfigManager>,
    access_logger: State<'_, Arc<lr_monitoring::logger::AccessLogger>>,
    mcp_access_logger: State<'_, Arc<lr_monitoring::mcp_logger::McpAccessLogger>>,
) -> Result<(), String> {
    // Update config
    config_manager
        .update(|config| {
            config.logging.enable_access_log = enabled;
        })
        .map_err(|e| e.to_string())?;

    config_manager.save().await.map_err(|e| e.to_string())?;

    // Update the loggers in real-time
    access_logger.set_enabled(enabled);
    mcp_access_logger.set_enabled(enabled);

    Ok(())
}

/// Open the logs folder in the system file manager
#[tauri::command]
pub async fn open_logs_folder(
    access_logger: State<'_, Arc<lr_monitoring::logger::AccessLogger>>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    use tauri_plugin_shell::ShellExt;

    let log_dir = access_logger.log_dir();

    // Ensure directory exists
    if !log_dir.exists() {
        std::fs::create_dir_all(log_dir)
            .map_err(|e| format!("Failed to create log directory: {}", e))?;
    }

    // Open in system file manager
    #[allow(deprecated)]
    app.shell()
        .open(log_dir.to_string_lossy().as_ref(), None)
        .map_err(|e| format!("Failed to open logs folder: {}", e))?;

    Ok(())
}

// ============================================================================
// File System Commands
// ============================================================================

/// Open a file or folder path in the system file manager / default application
#[tauri::command]
pub async fn open_path(path: String, app: tauri::AppHandle) -> Result<(), String> {
    use tauri_plugin_shell::ShellExt;

    let path = std::path::PathBuf::from(&path);
    if !path.exists() {
        return Err(format!("Path does not exist: {}", path.display()));
    }

    #[allow(deprecated)]
    app.shell()
        .open(path.to_string_lossy().as_ref(), None)
        .map_err(|e| format!("Failed to open path: {}", e))?;

    Ok(())
}

// ============================================================================
// Connection Graph Commands
// ============================================================================

/// Get list of active SSE connections (connected apps)
///
/// Returns a list of client IDs that currently have active SSE connections.
/// Used by the Dashboard connection graph to show which apps are connected.
#[tauri::command]
pub async fn get_active_connections(
    server_manager: State<'_, Arc<ServerManager>>,
) -> Result<Vec<String>, String> {
    let state = server_manager
        .get_state()
        .ok_or_else(|| "Server not started".to_string())?;

    Ok(state.sse_connection_manager.get_active_connections())
}

// ============================================================================
// Skills Commands
// ============================================================================

/// List all discovered skills
#[tauri::command]
pub async fn list_skills(
    skill_manager: State<'_, Arc<lr_skills::SkillManager>>,
) -> Result<Vec<lr_skills::SkillInfo>, String> {
    Ok(skill_manager.list())
}

/// Get a specific skill by name
#[tauri::command]
pub async fn get_skill(
    skill_name: String,
    skill_manager: State<'_, Arc<lr_skills::SkillManager>>,
) -> Result<lr_skills::SkillDefinition, String> {
    skill_manager
        .get(&skill_name)
        .ok_or_else(|| format!("Skill '{}' not found", skill_name))
}

/// Get skills configuration
#[tauri::command]
pub async fn get_skills_config(
    config_manager: State<'_, ConfigManager>,
) -> Result<SkillsConfig, String> {
    Ok(config_manager.get().skills)
}

/// Add a skill source path (directory, zip, or .skill file)
#[tauri::command]
pub async fn add_skill_source(
    path: String,
    config_manager: State<'_, ConfigManager>,
    skill_manager: State<'_, Arc<lr_skills::SkillManager>>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    config_manager
        .update(|cfg| {
            if !cfg.skills.paths.contains(&path) {
                cfg.skills.paths.push(path.clone());
            }
        })
        .map_err(|e| e.to_string())?;

    config_manager.save().await.map_err(|e| e.to_string())?;

    // Rescan skills
    let config = config_manager.get();
    skill_manager.rescan(&config.skills.paths, &config.skills.disabled_skills);

    // Notify watcher if available
    if let Some(watcher) = app.try_state::<Arc<lr_skills::SkillWatcher>>() {
        watcher.inner().add_path(path);
    }

    if let Err(e) = app.emit("skills-changed", ()) {
        tracing::error!("Failed to emit skills-changed event: {}", e);
    }

    Ok(())
}

/// Remove a skill source path
#[tauri::command]
pub async fn remove_skill_source(
    path: String,
    config_manager: State<'_, ConfigManager>,
    skill_manager: State<'_, Arc<lr_skills::SkillManager>>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    config_manager
        .update(|cfg| {
            cfg.skills.paths.retain(|p| p != &path);
        })
        .map_err(|e| e.to_string())?;

    config_manager.save().await.map_err(|e| e.to_string())?;

    let config = config_manager.get();
    skill_manager.rescan(&config.skills.paths, &config.skills.disabled_skills);

    // Notify watcher if available
    if let Some(watcher) = app.try_state::<Arc<lr_skills::SkillWatcher>>() {
        watcher.inner().remove_path(path);
    }

    if let Err(e) = app.emit("skills-changed", ()) {
        tracing::error!("Failed to emit skills-changed event: {}", e);
    }

    Ok(())
}

/// Toggle a skill's global enabled state
#[tauri::command]
pub async fn set_skill_enabled(
    skill_name: String,
    enabled: bool,
    config_manager: State<'_, ConfigManager>,
    skill_manager: State<'_, Arc<lr_skills::SkillManager>>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    // Update disabled_skills in config
    config_manager
        .update(|cfg| {
            if enabled {
                cfg.skills.disabled_skills.retain(|n| n != &skill_name);
            } else if !cfg.skills.disabled_skills.contains(&skill_name) {
                cfg.skills.disabled_skills.push(skill_name.clone());
            }
        })
        .map_err(|e| e.to_string())?;

    config_manager.save().await.map_err(|e| e.to_string())?;

    // Update in-memory manager
    skill_manager.set_skill_enabled(&skill_name, enabled);

    if let Err(e) = app.emit("skills-changed", ()) {
        tracing::error!("Failed to emit skills-changed event: {}", e);
    }

    Ok(())
}

/// Rescan all skill paths
#[tauri::command]
pub async fn rescan_skills(
    config_manager: State<'_, ConfigManager>,
    skill_manager: State<'_, Arc<lr_skills::SkillManager>>,
    app: tauri::AppHandle,
) -> Result<Vec<lr_skills::SkillInfo>, String> {
    let config = config_manager.get();
    let skills = skill_manager.rescan(&config.skills.paths, &config.skills.disabled_skills);

    if let Err(e) = app.emit("skills-changed", ()) {
        tracing::error!("Failed to emit skills-changed event: {}", e);
    }

    Ok(skills)
}

/// Tool information for a skill (for permission tree UI)
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SkillToolInfo {
    pub name: String,
    pub description: Option<String>,
}

/// Get tools exposed by a specific skill
///
/// Skills expose tools through their scripts. Each script becomes a callable tool.
/// Used by the permission tree UI to display available tools for each skill.
///
/// # Arguments
/// * `skill_name` - The skill name to get tools for
///
/// # Returns
/// * List of tool information (scripts exposed as tools)
#[tauri::command]
pub async fn get_skill_tools(
    skill_name: String,
    skill_manager: State<'_, Arc<lr_skills::SkillManager>>,
) -> Result<Vec<SkillToolInfo>, String> {
    // Get the skill definition
    let skill = skill_manager
        .get(&skill_name)
        .ok_or_else(|| format!("Skill '{}' not found", skill_name))?;

    // Skills expose scripts as tools
    // Tool names are typically: skill_{sanitized_skill_name}_{script_name}
    let sanitized_name = lr_skills::sanitize_name(&skill.metadata.name);

    let tools: Vec<SkillToolInfo> = skill
        .scripts
        .iter()
        .map(|script| {
            // Extract script name without extension
            let script_name = std::path::Path::new(script)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or(script);

            let tool_name = format!("skill_{}_{}", sanitized_name, script_name);

            SkillToolInfo {
                name: tool_name,
                description: Some(format!("Execute {} script from {} skill", script_name, skill.metadata.name)),
            }
        })
        .collect();

    tracing::debug!(
        "Skill '{}' has {} tools: {:?}",
        skill_name,
        tools.len(),
        tools.iter().map(|t| &t.name).collect::<Vec<_>>()
    );

    Ok(tools)
}

/// Set skills access for a client
#[tauri::command]
pub async fn set_client_skills_access(
    client_id: String,
    mode: SkillsAccessMode,
    skill_names: Vec<String>,
    config_manager: State<'_, ConfigManager>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    let access = match mode {
        SkillsAccessMode::None => SkillsAccess::None,
        SkillsAccessMode::All => SkillsAccess::All,
        SkillsAccessMode::Specific => SkillsAccess::Specific(skill_names),
    };

    tracing::info!(
        "Setting skills access for client {} to {:?}",
        client_id,
        access
    );

    let mut found = false;
    config_manager
        .update(|cfg| {
            if let Some(client) = cfg.clients.iter_mut().find(|c| c.id == client_id) {
                client.set_skills_access(access.clone());
                found = true;
            }
        })
        .map_err(|e| e.to_string())?;

    if !found {
        return Err(format!("Client not found: {}", client_id));
    }

    config_manager.save().await.map_err(|e| e.to_string())?;

    if let Err(e) = app.emit("clients-changed", ()) {
        tracing::error!("Failed to emit clients-changed event: {}", e);
    }

    Ok(())
}

/// Get files for a specific skill with content previews.
/// Walks the entire skill directory recursively to list all files.
/// Each file is categorized based on the skill's discovery logic:
/// - "skill_md": The SKILL.md definition file
/// - "script": Executable scripts in scripts/ directory
/// - "reference": Readable reference files in references/ directory
/// - "asset": Asset files in assets/ directory
/// - "": Other files not classified by the skill system
#[tauri::command]
pub async fn get_skill_files(
    skill_name: String,
    skill_manager: State<'_, Arc<lr_skills::SkillManager>>,
) -> Result<Vec<SkillFileInfo>, String> {
    let skill = skill_manager
        .get(&skill_name)
        .ok_or_else(|| format!("Skill '{}' not found", skill_name))?;

    // Build a set of known classified files for quick lookup
    let mut category_map = std::collections::HashMap::new();
    category_map.insert("SKILL.md".to_string(), "skill_md".to_string());
    for s in &skill.scripts {
        category_map.insert(s.clone(), "script".to_string());
    }
    for r in &skill.references {
        category_map.insert(r.clone(), "reference".to_string());
    }
    for a in &skill.assets {
        category_map.insert(a.clone(), "asset".to_string());
    }

    let mut files = Vec::new();
    collect_skill_files_recursive(
        &skill.skill_dir,
        &skill.skill_dir,
        &category_map,
        &mut files,
    );
    files.sort_by(|a, b| a.name.cmp(&b.name));

    Ok(files)
}

fn collect_skill_files_recursive(
    base_dir: &std::path::Path,
    current_dir: &std::path::Path,
    category_map: &std::collections::HashMap<String, String>,
    files: &mut Vec<SkillFileInfo>,
) {
    let Ok(entries) = std::fs::read_dir(current_dir) else {
        return;
    };

    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_key(|e| e.file_name());

    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            collect_skill_files_recursive(base_dir, &path, category_map, files);
        } else if path.is_file() {
            let relative = path
                .strip_prefix(base_dir)
                .unwrap_or(&path)
                .to_string_lossy()
                .to_string();

            let preview = if is_text_file(&path) {
                read_file_preview(&path)
            } else {
                None
            };

            let category = category_map.get(&relative).cloned().unwrap_or_default();

            files.push(SkillFileInfo {
                name: relative,
                category,
                content_preview: preview,
            });
        }
    }
}

#[derive(serde::Serialize)]
pub struct SkillFileInfo {
    pub name: String,
    pub category: String,
    pub content_preview: Option<String>,
}

fn read_file_preview(path: &std::path::Path) -> Option<String> {
    std::fs::read_to_string(path).ok().map(|content| {
        if content.len() > 500 {
            let truncated: String = content.chars().take(500).collect();
            format!("{}...", truncated)
        } else {
            content
        }
    })
}

fn is_text_file(path: &std::path::Path) -> bool {
    match path.extension().and_then(|e| e.to_str()) {
        Some(ext) => matches!(
            ext.to_lowercase().as_str(),
            "txt"
                | "md"
                | "json"
                | "yaml"
                | "yml"
                | "toml"
                | "xml"
                | "csv"
                | "html"
                | "css"
                | "js"
                | "ts"
                | "jsx"
                | "tsx"
                | "py"
                | "rs"
                | "sh"
                | "bash"
                | "zsh"
                | "pl"
                | "rb"
                | "lua"
                | "go"
                | "java"
                | "c"
                | "h"
                | "cpp"
                | "hpp"
                | "swift"
                | "kt"
                | "r"
                | "sql"
                | "graphql"
                | "proto"
                | "ini"
                | "cfg"
                | "conf"
                | "env"
                | "cmake"
        ),
        None => {
            // Handle extensionless text files by filename
            path.file_name()
                .and_then(|f| f.to_str())
                .map(|name| {
                    matches!(
                        name.to_lowercase().as_str(),
                        "dockerfile" | "makefile" | "gemfile" | "rakefile" | "license" | "readme"
                    )
                })
                .unwrap_or(false)
        }
    }
}

// ============================================================================
// Debug Commands (dev only)
// ============================================================================

/// Trigger a fake firewall approval popup after a 3 second delay.
///
/// This creates a real pending approval in the FirewallManager and opens
/// the approval popup window after the delay. The timer runs on the backend
/// so the caller can close the main window before the popup appears.
#[tauri::command]
pub async fn debug_trigger_firewall_popup(
    app: AppHandle,
    state: State<'_, Arc<lr_server::state::AppState>>,
) -> Result<(), String> {
    let firewall_manager = state.mcp_gateway.firewall_manager.clone();
    let app_clone = app.clone();

    // Spawn a background task so the frontend call returns immediately
    tauri::async_runtime::spawn(async move {
        // Wait 3 seconds
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;

        // Create a fake approval request (this inserts into FirewallManager)
        let request_id = uuid::Uuid::new_v4().to_string();
        let timeout_secs: u64 = 30;

        // For debug purposes, we don't need a response channel since there's no
        // real MCP request waiting for the approval. Setting response_sender to None
        // allows the popup to work without errors when submitting a response.
        let session = lr_mcp::gateway::firewall::FirewallApprovalSession {
            request_id: request_id.clone(),
            client_id: "debug-client".to_string(),
            client_name: "Debug Test Client".to_string(),
            tool_name: "filesystem__write_file".to_string(),
            server_name: "filesystem".to_string(),
            arguments_preview: r#"{"path": "/tmp/test.txt", "content": "hello world"}"#.to_string(),
            response_sender: None, // No response channel for debug mode
            created_at: std::time::Instant::now(),
            timeout_seconds: timeout_secs,
            is_model_request: false,
        };

        firewall_manager.insert_pending(request_id.clone(), session);

        // Rebuild tray menu to show the pending approval item
        if let Err(e) = crate::ui::tray::rebuild_tray_menu(&app_clone) {
            tracing::warn!("Failed to rebuild tray menu for firewall approval: {}", e);
        }

        // Trigger immediate tray icon update to show the question mark overlay
        if let Some(tray_graph_manager) =
            app_clone.try_state::<Arc<crate::ui::tray::TrayGraphManager>>()
        {
            tray_graph_manager.notify_activity();
        }

        // Create the firewall approval popup window
        use tauri::WebviewWindowBuilder;
        match WebviewWindowBuilder::new(
            &app_clone,
            format!("firewall-approval-{}", request_id),
            tauri::WebviewUrl::App("index.html".into()),
        )
        .title("Approve Tool")
        .inner_size(400.0, 340.0)
        .center()
        .visible(true)
        .resizable(false)
        .decorations(false)
        .build()
        {
            Ok(window) => {
                let _ = window.set_focus();
                tracing::info!("Debug firewall popup opened for request {}", request_id);
            }
            Err(e) => {
                tracing::error!("Failed to create debug firewall popup: {}", e);
            }
        }
    });

    Ok(())
}
