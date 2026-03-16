//! Tauri command handlers
//!
//! Functions exposed to the frontend via Tauri IPC.
//!
//! ## SYNC REQUIRED WHEN MODIFYING COMMANDS
//!
//! When adding or modifying Tauri commands, update these files:
//!
//! 1. **TypeScript types**: `src/types/tauri-commands.ts`
//!    - Add/update the return type interface
//!    - Include a comment linking back to this Rust source
//!
//! 2. **Demo mocks**: `website/src/components/demo/TauriMockSetup.ts`
//!    - Add a mock handler returning data matching the TypeScript type
//!    - Update `mockData.ts` if persistent mock state is needed
//!
//! 3. Run `npx tsc --noEmit` to verify TypeScript types compile
//!
//! See CLAUDE.md "Adding/Modifying Tauri Commands" for the full checklist.
//!
//! Note: Unimplemented commands show a toast warning in demo mode.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use lr_config::{ConfigManager, SkillsConfig};
use lr_monitoring::logger::AccessLogger;
use lr_oauth::clients::OAuthClientManager;
use lr_providers::registry::ProviderRegistry;
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

/// Get feature-level statistics (RouteLLM, JSON repair, compression, context mgmt)
///
/// Returns statistics from the persistent metrics database (last 90 days).
#[tauri::command]
pub async fn get_feature_stats(
    server_manager: State<'_, Arc<lr_server::ServerManager>>,
) -> Result<lr_server::state::FeatureStatsSnapshot, String> {
    let app_state = server_manager
        .get_state()
        .ok_or_else(|| "Server is not running".to_string())?;

    let totals = app_state.metrics_collector.get_feature_totals();

    Ok(lr_server::state::FeatureStatsSnapshot {
        routellm_strong: totals.routellm_strong,
        routellm_weak: totals.routellm_weak,
        json_repairs: totals.json_repairs,
        compression_tokens_saved: totals.compression_tokens_saved,
        compression_cost_saved_micros: totals.compression_cost_saved_micros,
        context_mgmt_tokens_saved: totals.context_mgmt_tokens_saved,
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
            lr_catalog::Modality::Video => "video",
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

/// Update tray graph settings (enabled state and refresh rate)
#[tauri::command]
pub async fn update_tray_graph_settings(
    enabled: bool,
    refresh_rate_secs: u64,
    config_manager: State<'_, ConfigManager>,
    tray_graph_manager: State<'_, Arc<crate::ui::tray::TrayGraphManager>>,
) -> Result<(), String> {
    // Validate parameters - only allow 1, 10, or 60
    if ![1, 10, 60].contains(&refresh_rate_secs) {
        return Err("refresh_rate_secs must be 1 (Fast), 10 (Medium), or 60 (Slow)".to_string());
    }

    // Update configuration
    config_manager
        .update(|config| {
            config.ui.tray_graph_enabled = enabled;
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

/// Get sidebar expanded state
#[tauri::command]
pub fn get_sidebar_expanded(config_manager: State<'_, ConfigManager>) -> Result<bool, String> {
    let config = config_manager.get();
    Ok(config.ui.sidebar_expanded)
}

/// Set sidebar expanded state
#[tauri::command]
pub async fn set_sidebar_expanded(
    expanded: bool,
    config_manager: State<'_, ConfigManager>,
) -> Result<(), String> {
    config_manager
        .update(|config| {
            config.ui.sidebar_expanded = expanded;
        })
        .map_err(|e| e.to_string())?;

    config_manager.save().await.map_err(|e| e.to_string())?;

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

/// Get user's config directory (platform-specific)
/// macOS: ~/Library/Application Support, Linux: ~/.config, Windows: %APPDATA%
#[tauri::command]
pub fn get_config_dir() -> Result<String, String> {
    dirs::config_dir()
        .ok_or_else(|| "Failed to get config directory".to_string())?
        .to_str()
        .ok_or_else(|| "Invalid config directory path".to_string())
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
    server_manager: State<'_, Arc<ServerManager>>,
) -> Result<LoggingConfigResponse, String> {
    let config = config_manager.get();
    let log_dir = server_manager
        .get_state()
        .map(|s| s.access_logger.log_dir().to_string_lossy().to_string())
        .unwrap_or_else(|| {
            AccessLogger::get_log_directory()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default()
        });
    Ok(LoggingConfigResponse {
        enabled: config.logging.enable_access_log,
        log_dir,
    })
}

/// Update logging configuration (enable/disable access logging)
#[tauri::command]
pub async fn update_logging_config(
    enabled: bool,
    config_manager: State<'_, ConfigManager>,
    server_manager: State<'_, Arc<ServerManager>>,
) -> Result<(), String> {
    // Update config
    config_manager
        .update(|config| {
            config.logging.enable_access_log = enabled;
        })
        .map_err(|e| e.to_string())?;

    config_manager.save().await.map_err(|e| e.to_string())?;

    // Update the loggers in real-time (uses current server state, works across restarts)
    if let Some(state) = server_manager.get_state() {
        state.access_logger.set_enabled(enabled);
        state.mcp_access_logger.set_enabled(enabled);
    }

    Ok(())
}

/// Open the logs folder in the system file manager
#[tauri::command]
pub async fn open_logs_folder(
    server_manager: State<'_, Arc<ServerManager>>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    use tauri_plugin_shell::ShellExt;

    let log_dir = server_manager
        .get_state()
        .map(|s| s.access_logger.log_dir().to_path_buf())
        .or_else(|| AccessLogger::get_log_directory().ok())
        .ok_or_else(|| "Could not determine log directory".to_string())?;

    // Ensure directory exists
    if !log_dir.exists() {
        std::fs::create_dir_all(&log_dir)
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

    // Resolve to canonical path to prevent symlink attacks
    let canonical = path
        .canonicalize()
        .map_err(|e| format!("Failed to resolve path: {}", e))?;

    // Validate path doesn't contain suspicious patterns
    let path_str = canonical.to_string_lossy();
    if path_str.contains("..") {
        return Err("Path traversal not allowed".to_string());
    }

    #[allow(deprecated)]
    app.shell()
        .open(canonical.to_string_lossy().as_ref(), None)
        .map_err(|e| format!("Failed to open path: {}", e))?;

    Ok(())
}

// ============================================================================
// Clipboard Commands
// ============================================================================

/// Copy a base64-encoded PNG image to the system clipboard
#[tauri::command]
pub async fn copy_image_to_clipboard(image_base64: String) -> Result<(), String> {
    use arboard::Clipboard;
    use image::ImageReader;
    use std::io::Cursor;

    let bytes = base64_decode(&image_base64)?;

    let img = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| format!("Failed to read image format: {}", e))?
        .decode()
        .map_err(|e| format!("Failed to decode image: {}", e))?;

    let rgba = img.to_rgba8();
    let (width, height) = rgba.dimensions();
    let img_data = arboard::ImageData {
        width: width as usize,
        height: height as usize,
        bytes: std::borrow::Cow::Owned(rgba.into_raw()),
    };

    let mut clipboard =
        Clipboard::new().map_err(|e| format!("Failed to access clipboard: {}", e))?;
    clipboard
        .set_image(img_data)
        .map_err(|e| format!("Failed to copy image to clipboard: {}", e))?;

    Ok(())
}

/// Copy text to the system clipboard
#[tauri::command]
pub async fn copy_text_to_clipboard(text: String) -> Result<(), String> {
    use arboard::Clipboard;

    let mut clipboard =
        Clipboard::new().map_err(|e| format!("Failed to access clipboard: {}", e))?;
    clipboard
        .set_text(text)
        .map_err(|e| format!("Failed to copy text to clipboard: {}", e))?;

    Ok(())
}

/// Decode a base64 string, stripping an optional `data:...;base64,` prefix.
fn base64_decode(input: &str) -> Result<Vec<u8>, String> {
    use base64::Engine;
    let raw = if let Some(pos) = input.find(";base64,") {
        &input[pos + 8..]
    } else {
        input
    };
    base64::engine::general_purpose::STANDARD
        .decode(raw)
        .map_err(|e| format!("Invalid base64: {}", e))
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

/// Get context management configuration
#[tauri::command]
pub async fn get_context_management_config(
    config_manager: State<'_, ConfigManager>,
) -> Result<lr_config::ContextManagementConfig, String> {
    Ok(config_manager.get().context_management.clone())
}

/// Update context management configuration
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn update_context_management_config(
    catalog_compression: Option<bool>,
    catalog_threshold_bytes: Option<usize>,
    response_threshold_bytes: Option<usize>,
    search_tool_name: Option<String>,
    read_tool_name: Option<String>,
    client_tools_indexing_default: Option<String>,
    config_manager: State<'_, ConfigManager>,
    context_mode_vs: State<'_, Arc<lr_mcp::gateway::context_mode::ContextModeVirtualServer>>,
    mcp_via_llm_manager: State<'_, Arc<lr_mcp_via_llm::McpViaLlmManager>>,
) -> Result<(), String> {
    config_manager
        .update(|cfg| {
            if let Some(v) = catalog_compression {
                cfg.context_management.catalog_compression = v;
            }
            if let Some(v) = catalog_threshold_bytes {
                cfg.context_management.catalog_threshold_bytes = v;
            }
            if let Some(v) = response_threshold_bytes {
                cfg.context_management.response_threshold_bytes = v;
            }
            if let Some(v) = search_tool_name {
                if !v.is_empty() {
                    cfg.context_management.search_tool_name = v;
                }
            }
            if let Some(v) = read_tool_name {
                if !v.is_empty() {
                    cfg.context_management.read_tool_name = v;
                }
            }
            if let Some(v) = &client_tools_indexing_default {
                cfg.context_management.client_tools_indexing_default = match v.as_str() {
                    "disable" => lr_config::IndexingState::Disable,
                    _ => lr_config::IndexingState::Enable,
                };
            }
        })
        .map_err(|e| e.to_string())?;

    // Propagate updated config to the in-memory virtual server + MCP via LLM manager
    let new_config = config_manager.get().context_management.clone();
    context_mode_vs.update_config(new_config.clone());
    mcp_via_llm_manager.update_context_management_config(new_config);

    config_manager.save().await.map_err(|e| e.to_string())?;
    Ok(())
}

/// Set a gateway indexing permission at global, server, or tool level.
#[tauri::command]
pub async fn set_gateway_indexing_permission(
    level: String,
    key: Option<String>,
    state: String,
    config_manager: State<'_, ConfigManager>,
    context_mode_vs: State<'_, Arc<lr_mcp::gateway::context_mode::ContextModeVirtualServer>>,
    mcp_via_llm_manager: State<'_, Arc<lr_mcp_via_llm::McpViaLlmManager>>,
) -> Result<(), String> {
    let indexing_state = match state.as_str() {
        "disable" => lr_config::IndexingState::Disable,
        _ => lr_config::IndexingState::Enable,
    };

    config_manager
        .update(|cfg| match level.as_str() {
            "global" => {
                cfg.context_management.gateway_indexing.global = indexing_state.clone();
            }
            "server" => {
                if let Some(ref k) = key {
                    cfg.context_management
                        .gateway_indexing
                        .servers
                        .insert(k.clone(), indexing_state.clone());
                }
            }
            "tool" => {
                if let Some(ref k) = key {
                    cfg.context_management
                        .gateway_indexing
                        .tools
                        .insert(k.clone(), indexing_state.clone());
                }
            }
            "server_clear" => {
                if let Some(ref k) = key {
                    cfg.context_management.gateway_indexing.servers.remove(k);
                }
            }
            "tool_clear" => {
                if let Some(ref k) = key {
                    cfg.context_management.gateway_indexing.tools.remove(k);
                }
            }
            _ => {}
        })
        .map_err(|e| e.to_string())?;

    let new_config = config_manager.get().context_management.clone();
    context_mode_vs.update_config(new_config.clone());
    mcp_via_llm_manager.update_context_management_config(new_config);

    config_manager.save().await.map_err(|e| e.to_string())?;
    Ok(())
}

/// Virtual MCP server indexing info for UI display.
#[derive(Serialize)]
pub struct VirtualMcpIndexingInfo {
    pub id: String,
    pub display_name: String,
    pub tools: Vec<VirtualMcpToolIndexingInfo>,
}

/// Single virtual MCP tool indexing info.
#[derive(Serialize)]
pub struct VirtualMcpToolIndexingInfo {
    pub name: String,
    pub indexable: bool,
}

/// List virtual MCP server indexing info for the UI.
#[tauri::command]
pub async fn list_virtual_mcp_indexing_info(
    state: State<'_, Arc<lr_server::state::AppState>>,
) -> Result<Vec<VirtualMcpIndexingInfo>, String> {
    let infos = state.mcp_gateway.list_virtual_server_indexing_info();
    Ok(infos
        .into_iter()
        .map(|info| VirtualMcpIndexingInfo {
            id: info.id,
            display_name: info.display_name,
            tools: info
                .tools
                .into_iter()
                .map(|t| VirtualMcpToolIndexingInfo {
                    name: t.name,
                    indexable: t.indexable,
                })
                .collect(),
        })
        .collect())
}

/// Set a virtual MCP indexing permission at global, server, or tool level.
#[tauri::command]
pub async fn set_virtual_indexing_permission(
    level: String,
    key: Option<String>,
    state: String,
    config_manager: State<'_, ConfigManager>,
    context_mode_vs: State<'_, Arc<lr_mcp::gateway::context_mode::ContextModeVirtualServer>>,
    mcp_via_llm_manager: State<'_, Arc<lr_mcp_via_llm::McpViaLlmManager>>,
) -> Result<(), String> {
    let indexing_state = match state.as_str() {
        "disable" => lr_config::IndexingState::Disable,
        _ => lr_config::IndexingState::Enable,
    };

    config_manager
        .update(|cfg| match level.as_str() {
            "global" => {
                cfg.context_management.virtual_indexing.global = indexing_state.clone();
            }
            "server" => {
                if let Some(ref k) = key {
                    cfg.context_management
                        .virtual_indexing
                        .servers
                        .insert(k.clone(), indexing_state.clone());
                }
            }
            "tool" => {
                if let Some(ref k) = key {
                    cfg.context_management
                        .virtual_indexing
                        .tools
                        .insert(k.clone(), indexing_state.clone());
                }
            }
            "server_clear" => {
                if let Some(ref k) = key {
                    cfg.context_management.virtual_indexing.servers.remove(k);
                }
            }
            "tool_clear" => {
                if let Some(ref k) = key {
                    cfg.context_management.virtual_indexing.tools.remove(k);
                }
            }
            _ => {}
        })
        .map_err(|e| e.to_string())?;

    let new_config = config_manager.get().context_management.clone();
    context_mode_vs.update_config(new_config.clone());
    mcp_via_llm_manager.update_context_management_config(new_config);

    config_manager.save().await.map_err(|e| e.to_string())?;
    Ok(())
}

/// Get known client tools for a given template ID.
#[tauri::command]
pub async fn get_known_client_tools(
    template_id: String,
) -> Result<Vec<lr_config::known_client_tools::KnownToolEntry>, String> {
    Ok(lr_config::known_client_tools::known_tools_for_template(
        &template_id,
    ))
}

/// Get seen client tools for a given client (auto-discovered at runtime).
#[tauri::command]
pub async fn get_seen_client_tools(
    client_id: String,
    mcp_via_llm_manager: State<'_, Arc<lr_mcp_via_llm::McpViaLlmManager>>,
) -> Result<Vec<String>, String> {
    Ok(mcp_via_llm_manager.get_seen_client_tools(&client_id))
}

/// Preview catalog compression at a given threshold.
///
/// Returns both compressed and uncompressed welcome messages plus per-server
/// detail for rendering a side-by-side catalog diff.
#[derive(Serialize)]
pub struct CatalogCompressionPreview {
    pub welcome_message: String,
    pub welcome_message_uncompressed: String,
    pub uncompressed_size: usize,
    pub compressed_size: usize,
    pub welcome_size: usize,
    pub tool_definitions_size: usize,
    pub compressed_tool_definitions_size: usize,
    pub indexed_welcomes_count: usize,
    pub deferred_servers_count: usize,
    pub welcome_toc_dropped_count: usize,
    pub batch_toc_dropped_count: usize,
    /// Per-server breakdown with full descriptions and compression state.
    pub servers: Vec<PreviewServerEntry>,
}

#[derive(Serialize)]
pub struct PreviewServerEntry {
    pub name: String,
    pub is_virtual: bool,
    pub tool_names: Vec<String>,
    pub resource_names: Vec<String>,
    pub prompt_names: Vec<String>,
    pub description: Option<String>,
    pub instructions: Option<String>,
    /// "visible" | "compressed" | "deferred" | "truncated"
    pub compression_state: String,
    pub tools: Vec<PreviewToolDetail>,
    pub resources: Vec<PreviewResourceDetail>,
    pub prompts: Vec<PreviewPromptDetail>,
}

#[derive(Serialize)]
pub struct PreviewToolDetail {
    pub name: String,
    pub description: Option<String>,
    pub input_schema: Option<serde_json::Value>,
}

#[derive(Serialize)]
pub struct PreviewResourceDetail {
    pub name: String,
    pub uri: Option<String>,
    pub description: Option<String>,
}

#[derive(Serialize)]
pub struct PreviewPromptDetail {
    pub name: String,
    pub description: Option<String>,
}

/// Generate a human-readable description from a namespaced tool name.
/// e.g. "github__issue_read" → "Issue read", "filesystem__read_file" → "Read file"
fn humanize_tool_name(name: &str) -> String {
    let raw = name.split("__").last().unwrap_or(name).replace('_', " ");
    let mut chars = raw.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().to_string() + chars.as_str(),
        None => raw,
    }
}

#[tauri::command]
pub async fn preview_catalog_compression(
    catalog_threshold_bytes: usize,
    source: Option<String>,
    state: State<'_, Arc<lr_server::state::AppState>>,
) -> Result<CatalogCompressionPreview, String> {
    use lr_mcp::gateway::{
        build_gateway_instructions, build_preview_mock_realistic, build_preview_mock_tool_catalog,
        compute_catalog_compression_plan,
    };

    let is_mock = matches!(source.as_deref(), None | Some("mock"));

    // Resolve the InstructionsContext based on source
    let mut ctx = match source.as_deref() {
        // Mock preset with realistic MCP server data
        None | Some("mock") => build_preview_mock_realistic(),
        // Real client by client_id
        Some(client_id) if client_id.starts_with("client:") => {
            let cid = &client_id["client:".len()..];
            // Resolve allowed server IDs from client's MCP permissions
            let config = state.config_manager.get();
            let client = config
                .clients
                .iter()
                .find(|c| c.id == cid)
                .ok_or_else(|| format!("Client not found: {cid}"))?;
            let all_server_ids: Vec<String> = config
                .mcp_servers
                .iter()
                .filter(|s| s.enabled)
                .map(|s| s.id.clone())
                .collect();
            let allowed_server_ids: Vec<String> = if client.mcp_permissions.global.is_enabled() {
                all_server_ids
            } else {
                all_server_ids
                    .into_iter()
                    .filter(|sid| client.mcp_permissions.has_any_enabled_for_server(sid))
                    .collect()
            };
            // Start servers on demand if needed
            state
                .mcp_gateway
                .get_or_build_preview_context(cid, allowed_server_ids)
                .await?
        }
        Some(other) => {
            return Err(format!("Unknown preview source: {other}"));
        }
    };

    // Fetch tool catalog for detailed tool info (descriptions, schemas).
    // Always try — if servers are running we get real data even for mock source.
    let (mut tool_catalog, resource_catalog, prompt_catalog) =
        state.mcp_gateway.fetch_preview_catalogs().await;

    // For mock source, supplement with the mock tool catalog so all tools
    // have verbose descriptions and inputSchemas even without running servers.
    if is_mock {
        let mock_catalog = build_preview_mock_tool_catalog();
        for mock_tool in mock_catalog {
            if !tool_catalog.iter().any(|t| t.name == mock_tool.name) {
                tool_catalog.push(mock_tool);
            }
        }
    }

    // Build uncompressed version (no plan)
    ctx.catalog_compression = None;
    let uncompressed = build_gateway_instructions(&ctx).unwrap_or_default();
    let uncompressed_size = uncompressed.len();

    // Compute definition sizes and add to context
    ctx.item_definition_sizes = lr_mcp::gateway::compute_item_definition_sizes(
        &tool_catalog,
        &resource_catalog,
        &prompt_catalog,
    );

    // When no real catalog is available (servers not running), generate estimated sizes
    // from tool/resource/prompt names so Phase 2 can still defer servers.
    if ctx.item_definition_sizes.is_empty() {
        for server in &ctx.servers {
            for name in server
                .tool_names
                .iter()
                .chain(server.resource_names.iter())
                .chain(server.prompt_names.iter())
            {
                // Estimate ~200 bytes per item (name + minimal schema)
                ctx.item_definition_sizes.entry(name.clone()).or_insert(200);
            }
        }
    }

    // Calculate size breakdown
    let welcome_size: usize = ctx
        .servers
        .iter()
        .map(|s| {
            s.description.as_ref().map(|d| d.len()).unwrap_or(0)
                + s.instructions.as_ref().map(|i| i.len()).unwrap_or(0)
        })
        .sum();
    let tool_definitions_size: usize = ctx.item_definition_sizes.values().sum();

    // Compute compression plan
    let plan = compute_catalog_compression_plan(&ctx, catalog_threshold_bytes, true, true, true);
    let indexed_welcomes_count = plan.indexed_welcomes.len();
    let deferred_servers_count = plan.deferred_servers.len();
    let welcome_toc_dropped_count = plan.welcome_toc_dropped.len();
    let batch_toc_dropped_count = plan.batch_toc_dropped.len();

    // Build compression state lookups
    let indexed_welcome_slugs: std::collections::HashSet<&str> = plan
        .indexed_welcomes
        .iter()
        .map(|w| w.server_slug.as_str())
        .collect();
    let deferred_slugs: std::collections::HashSet<&str> = plan
        .deferred_servers
        .iter()
        .map(|d| d.server_slug.as_str())
        .collect();

    // Build per-server entries
    let mut servers = Vec::new();

    // Virtual servers (always visible — never compressed)
    for vsi in &ctx.virtual_instructions {
        // Build tool details from catalog, falling back to humanized name
        let tools: Vec<PreviewToolDetail> = vsi
            .tool_names
            .iter()
            .map(|name| {
                let catalog_tool = tool_catalog.iter().find(|t| &t.name == name);
                PreviewToolDetail {
                    name: name.clone(),
                    description: catalog_tool
                        .and_then(|t| t.description.clone())
                        .or_else(|| Some(humanize_tool_name(name))),
                    input_schema: catalog_tool.map(|t| t.input_schema.clone()),
                }
            })
            .collect();

        servers.push(PreviewServerEntry {
            name: vsi.section_title.clone(),
            is_virtual: true,
            tool_names: vsi.tool_names.clone(),
            resource_names: Vec::new(),
            prompt_names: Vec::new(),
            description: Some(vsi.content.clone()),
            instructions: None,
            compression_state: "visible".to_string(),
            tools,
            resources: Vec::new(),
            prompts: Vec::new(),
        });
    }

    // MCP servers
    for server in &ctx.servers {
        let slug = server.name.to_lowercase().replace(' ', "-");
        let compression_state = if deferred_slugs.contains(slug.as_str()) {
            "deferred"
        } else if indexed_welcome_slugs.contains(slug.as_str()) {
            "compressed"
        } else {
            "visible"
        };

        // Build tool details from catalog, falling back to generated description
        let tools: Vec<PreviewToolDetail> = server
            .tool_names
            .iter()
            .map(|name| {
                let catalog_tool = tool_catalog.iter().find(|t| &t.name == name);
                PreviewToolDetail {
                    name: name.clone(),
                    description: catalog_tool
                        .and_then(|t| t.description.clone())
                        .or_else(|| Some(humanize_tool_name(name))),
                    input_schema: catalog_tool.map(|t| t.input_schema.clone()),
                }
            })
            .collect();

        let resources: Vec<PreviewResourceDetail> = server
            .resource_names
            .iter()
            .map(|name| {
                let catalog_res = resource_catalog.iter().find(|r| &r.name == name);
                PreviewResourceDetail {
                    name: name.clone(),
                    uri: catalog_res.map(|r| r.uri.clone()),
                    description: catalog_res
                        .and_then(|r| r.description.clone())
                        .or_else(|| Some(humanize_tool_name(name))),
                }
            })
            .collect();

        let prompts: Vec<PreviewPromptDetail> = server
            .prompt_names
            .iter()
            .map(|name| {
                let catalog_prompt = prompt_catalog.iter().find(|p| &p.name == name);
                PreviewPromptDetail {
                    name: name.clone(),
                    description: catalog_prompt
                        .and_then(|p| p.description.clone())
                        .or_else(|| Some(humanize_tool_name(name))),
                }
            })
            .collect();

        servers.push(PreviewServerEntry {
            name: server.name.clone(),
            is_virtual: false,
            tool_names: server.tool_names.clone(),
            resource_names: server.resource_names.clone(),
            prompt_names: server.prompt_names.clone(),
            description: server.description.clone(),
            instructions: server.instructions.clone(),
            compression_state: compression_state.to_string(),
            tools,
            resources,
            prompts,
        });
    }

    // Compute compressed tool definitions size (subtract savings from deferred servers)
    let deferred_savings: usize = plan
        .deferred_servers
        .iter()
        .map(|d| d.definition_savings)
        .sum();
    let compressed_tool_definitions_size = tool_definitions_size.saturating_sub(deferred_savings);

    // Set plan and build compressed version
    ctx.catalog_compression = Some(plan);
    let compressed = build_gateway_instructions(&ctx).unwrap_or_default();
    let compressed_size = compressed.len();

    Ok(CatalogCompressionPreview {
        welcome_message: compressed,
        welcome_message_uncompressed: uncompressed,
        uncompressed_size,
        compressed_size,
        welcome_size,
        tool_definitions_size,
        compressed_tool_definitions_size,
        indexed_welcomes_count,
        deferred_servers_count,
        welcome_toc_dropped_count,
        batch_toc_dropped_count,
        servers,
    })
}

/// List active MCP gateway sessions
#[tauri::command]
pub async fn list_active_sessions(
    state: State<'_, Arc<lr_server::state::AppState>>,
) -> Result<Vec<lr_mcp::gateway::ActiveSessionInfo>, String> {
    Ok(state.mcp_gateway.list_active_sessions().await)
}

/// Terminate an active MCP gateway session by session key
#[tauri::command]
pub async fn terminate_session(
    session_id: String,
    state: State<'_, Arc<lr_server::state::AppState>>,
) -> Result<(), String> {
    state.mcp_gateway.terminate_session(&session_id).await
}

/// Get catalog sources for a specific session by session key
#[tauri::command]
pub async fn get_session_context_sources(
    session_id: String,
    state: State<'_, Arc<lr_server::state::AppState>>,
) -> Result<Vec<lr_mcp::gateway::CatalogSourceEntry>, String> {
    state
        .mcp_gateway
        .get_session_context_sources(&session_id)
        .await
}

/// Get context stats for a specific session (calls ctx_stats on context-mode process)
#[tauri::command]
pub async fn get_session_context_stats(
    session_id: String,
    state: State<'_, Arc<lr_server::state::AppState>>,
) -> Result<serde_json::Value, String> {
    state
        .mcp_gateway
        .get_session_context_stats(&session_id)
        .await
}

/// Query the context index for a specific session (calls ctx_search on context-mode process)
#[tauri::command]
pub async fn query_session_context_index(
    session_id: String,
    query: String,
    source: Option<String>,
    state: State<'_, Arc<lr_server::state::AppState>>,
) -> Result<serde_json::Value, String> {
    state
        .mcp_gateway
        .query_session_context_index(&session_id, &query, source.as_deref())
        .await
}

// ─────────────────────────────────────────────────────────
// Response RAG Preview
// ─────────────────────────────────────────────────────────

fn rag_preview_store() -> &'static parking_lot::Mutex<Option<lr_context::ContentStore>> {
    static STORE: std::sync::OnceLock<parking_lot::Mutex<Option<lr_context::ContentStore>>> =
        std::sync::OnceLock::new();
    STORE.get_or_init(|| parking_lot::Mutex::new(None))
}

/// Truncate a string to at most `max_bytes` at a char boundary.
fn rag_truncate_to_char_boundary(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

#[derive(Serialize)]
pub struct RagPreviewIndexResult {
    pub compressed_preview: String,
    pub index_result: lr_context::IndexResult,
    pub sources: Vec<lr_context::SourceInfo>,
}

/// Index content into a temporary preview store and return the compressed preview.
#[tauri::command]
pub async fn preview_rag_index(
    content: String,
    label: String,
    response_threshold_bytes: usize,
) -> Result<RagPreviewIndexResult, String> {
    tokio::task::spawn_blocking(move || {
        let store = lr_context::ContentStore::new().map_err(|e| e.to_string())?;
        let index_result = store.index(&label, &content).map_err(|e| e.to_string())?;

        let byte_size = content.len();
        let preview_bytes = (response_threshold_bytes / 8).clamp(200, 500);
        let preview = rag_truncate_to_char_boundary(&content, preview_bytes);
        let compressed_preview = format!(
            "[Response compressed — {} bytes indexed as {}]\n\n{}\n\n\
             Full output indexed. Use IndexSearch(queries=[\"your search terms\"], source=\"{}\") to retrieve specific sections.",
            byte_size, label, preview, label
        );

        let sources = store.list_sources().map_err(|e| e.to_string())?;
        *rag_preview_store().lock() = Some(store);

        Ok(RagPreviewIndexResult {
            compressed_preview,
            index_result,
            sources,
        })
    })
    .await
    .unwrap_or_else(|e| Err(format!("Task panicked: {}", e)))
}

/// Search the preview RAG store.
#[tauri::command]
pub async fn preview_rag_search(
    query: Option<String>,
    queries: Option<Vec<String>>,
    limit: Option<usize>,
    source: Option<String>,
) -> Result<Vec<lr_context::SearchResult>, String> {
    tokio::task::spawn_blocking(move || {
        let guard = rag_preview_store().lock();
        let store = guard
            .as_ref()
            .ok_or("No content indexed yet. Index a document first.")?;
        let limit = limit.unwrap_or(5);
        store
            .search_combined(
                query.as_deref(),
                queries.as_deref(),
                limit,
                source.as_deref(),
            )
            .map_err(|e| e.to_string())
    })
    .await
    .unwrap_or_else(|e| Err(format!("Task panicked: {}", e)))
}

/// Read from the preview RAG store.
#[tauri::command]
pub async fn preview_rag_read(
    label: String,
    offset: Option<String>,
    limit: Option<usize>,
) -> Result<lr_context::ReadResult, String> {
    tokio::task::spawn_blocking(move || {
        let guard = rag_preview_store().lock();
        let store = guard
            .as_ref()
            .ok_or("No content indexed yet. Index a document first.")?;
        store
            .read(&label, offset.as_deref(), limit)
            .map_err(|e| e.to_string())
    })
    .await
    .unwrap_or_else(|e| Err(format!("Task panicked: {}", e)))
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

/// Create a new skill by writing a SKILL.md file to the user skills directory
///
/// Creates `{config_dir}/skills/{name}/SKILL.md` with the provided frontmatter and body,
/// then adds the skills directory as a source path if not already present and rescans.
#[tauri::command]
pub async fn create_skill(
    name: String,
    description: Option<String>,
    content: String,
    config_manager: State<'_, ConfigManager>,
    skill_manager: State<'_, Arc<lr_skills::SkillManager>>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    if name.trim().is_empty() {
        return Err("Skill name is required".to_string());
    }

    // Sanitize name for directory: lowercase, replace spaces/special chars with hyphens
    let dir_name = name
        .trim()
        .to_lowercase()
        .replace(|c: char| !c.is_alphanumeric() && c != '-' && c != '_', "-")
        .trim_matches('-')
        .to_string();

    if dir_name.is_empty() {
        return Err("Invalid skill name".to_string());
    }

    // Build SKILL.md content with YAML frontmatter
    let mut skill_md = String::from("---\n");
    skill_md.push_str(&format!("name: \"{}\"\n", name.trim()));
    if let Some(ref desc) = description {
        if !desc.trim().is_empty() {
            skill_md.push_str(&format!("description: \"{}\"\n", desc.trim()));
        }
    }
    skill_md.push_str("---\n\n");
    skill_md.push_str(&content);

    // Determine config dir and create skills subdirectory
    let config_dir =
        lr_utils::paths::config_dir().map_err(|e| format!("Failed to get config dir: {}", e))?;
    let skills_dir = config_dir.join("skills");
    let skill_dir = skills_dir.join(&dir_name);

    if skill_dir.exists() {
        return Err(format!("A skill directory '{}' already exists", dir_name));
    }

    std::fs::create_dir_all(&skill_dir)
        .map_err(|e| format!("Failed to create skill directory: {}", e))?;

    std::fs::write(skill_dir.join("SKILL.md"), &skill_md)
        .map_err(|e| format!("Failed to write SKILL.md: {}", e))?;

    // Ensure the user skills directory is in skill source paths
    let skills_dir_str = skills_dir
        .to_str()
        .ok_or_else(|| "Invalid skills directory path".to_string())?
        .to_string();

    config_manager
        .update(|cfg| {
            if !cfg.skills.paths.contains(&skills_dir_str) {
                cfg.skills.paths.push(skills_dir_str.clone());
            }
        })
        .map_err(|e| e.to_string())?;

    config_manager.save().await.map_err(|e| e.to_string())?;

    // Rescan to pick up the new skill
    let config = config_manager.get();
    skill_manager.rescan(&config.skills.paths, &config.skills.disabled_skills);

    // Notify watcher if available
    if let Some(watcher) = app.try_state::<Arc<lr_skills::SkillWatcher>>() {
        watcher.inner().add_path(skills_dir_str);
    }

    if let Err(e) = app.emit("skills-changed", ()) {
        tracing::error!("Failed to emit skills-changed event: {}", e);
    }

    Ok(())
}

/// Check if a skill was created by the user (lives in {config_dir}/skills/)
#[tauri::command]
pub async fn is_user_created_skill(skill_path: String) -> Result<bool, String> {
    let config_dir =
        lr_utils::paths::config_dir().map_err(|e| format!("Failed to get config dir: {}", e))?;
    let user_skills_dir = config_dir.join("skills");
    let skill_path_buf = std::path::PathBuf::from(&skill_path);
    Ok(skill_path_buf.starts_with(&user_skills_dir))
}

/// Delete a user-created skill from disk
///
/// Only allows deleting skills that live under {config_dir}/skills/.
#[tauri::command]
pub async fn delete_user_skill(
    skill_name: String,
    skill_path: String,
    config_manager: State<'_, ConfigManager>,
    skill_manager: State<'_, Arc<lr_skills::SkillManager>>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    let config_dir =
        lr_utils::paths::config_dir().map_err(|e| format!("Failed to get config dir: {}", e))?;
    let user_skills_dir = config_dir.join("skills");
    let skill_path_buf = std::path::PathBuf::from(&skill_path);

    if !skill_path_buf.starts_with(&user_skills_dir) {
        return Err(format!(
            "Skill '{}' is not a user-created skill and cannot be deleted this way",
            skill_name
        ));
    }

    // Delete the skill directory
    if skill_path_buf.exists() {
        std::fs::remove_dir_all(&skill_path_buf)
            .map_err(|e| format!("Failed to delete skill directory: {}", e))?;
    }

    // Trigger skill rescan
    let config = config_manager.get();
    skill_manager.rescan(&config.skills.paths, &config.skills.disabled_skills);

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
                description: Some(format!(
                    "Execute {} script from {} skill",
                    script_name, skill.metadata.name
                )),
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

/// Debug firewall popup type
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DebugFirewallType {
    /// MCP tool approval (default)
    #[default]
    McpTool,
    /// LLM model approval
    LlmModel,
    /// Skill approval
    Skill,
    /// Marketplace approval
    Marketplace,
    /// Free-tier fallback approval
    FreeTierFallback,
    /// Coding agent session approval
    CodingAgent,
    /// Guardrail approval (safety check flagged content)
    Guardrail,
    /// Secret scan approval (secrets detected in outbound request)
    SecretScan,
}

/// Trigger a fake firewall approval popup immediately.
///
/// This creates a real pending approval in the FirewallManager and opens
/// the approval popup window.
///
/// When `send_multiple` is true, creates 3 sessions:
/// - Session 1: original resource
/// - Session 2: same resource (duplicate)
/// - Session 3: different resource
#[tauri::command]
pub async fn debug_trigger_firewall_popup(
    popup_type: Option<DebugFirewallType>,
    send_multiple: Option<bool>,
    app: AppHandle,
    state: State<'_, Arc<lr_server::state::AppState>>,
) -> Result<(), String> {
    let popup_type = popup_type.unwrap_or_default();
    let send_multiple = send_multiple.unwrap_or(false);
    let firewall_manager = state.mcp_gateway.firewall_manager.clone();

    // Configure based on popup type
    let (tool_name, server_name, arguments_preview, is_model_request, is_free_tier_fallback) =
        match popup_type {
            DebugFirewallType::McpTool => (
                "filesystem__write_file".to_string(),
                "filesystem".to_string(),
                r#"{"path": "/tmp/test.txt", "content": "hello world"}"#.to_string(),
                false,
                false,
            ),
            DebugFirewallType::LlmModel => (
                "claude-3-5-sonnet".to_string(),
                "anthropic".to_string(),
                r#"{"prompt": "Hello, how are you?", "max_tokens": 1000}"#.to_string(),
                true,
                false,
            ),
            DebugFirewallType::Skill => (
                "skill_web_search_search".to_string(),
                "web-search".to_string(),
                r#"{"query": "rust programming", "max_results": 10}"#.to_string(),
                false,
                false,
            ),
            DebugFirewallType::Marketplace => (
                "marketplace__install_package".to_string(),
                "marketplace".to_string(),
                r#"{"package": "code-review-tool", "version": "1.2.0"}"#.to_string(),
                false,
                false,
            ),
            DebugFirewallType::FreeTierFallback => (
                "Free-Tier Fallback".to_string(),
                "Paid Models".to_string(),
                "anthropic/claude-3-5-sonnet, openai/gpt-4o".to_string(),
                false,
                true,
            ),
            DebugFirewallType::CodingAgent => (
                "AgentStart".to_string(),
                "coding-agents".to_string(),
                r#"{"prompt": "Refactor the authentication module", "workingDirectory": "/home/user/project"}"#.to_string(),
                false,
                false,
            ),
            DebugFirewallType::Guardrail => (
                "claude-3-5-sonnet".to_string(),
                "anthropic".to_string(),
                r#"{"prompt": "Write a guide on making explosives", "max_tokens": 2000}"#.to_string(),
                false,
                false,
            ),
            DebugFirewallType::SecretScan => (
                "claude-3-5-sonnet".to_string(),
                "Secret Scan".to_string(),
                "[Cloud Provider] AWS Access Key ID (entropy: 3.42)\n[Version Control] GitHub Personal Access Token (entropy: 4.12)".to_string(),
                false,
                false,
            ),
        };

    // Build sessions list: 1 normally, 3 for multi-popup mode
    struct DebugSession {
        tool_name: String,
        server_name: String,
        arguments_preview: String,
        is_model_request: bool,
        is_free_tier_fallback: bool,
        is_guardrail_request: bool,
        is_secret_scan_request: bool,
        guardrail_details: Option<lr_mcp::gateway::firewall::GuardrailApprovalDetails>,
        secret_scan_details: Option<lr_mcp::gateway::firewall::SecretScanApprovalDetails>,
    }

    let is_guardrail = popup_type == DebugFirewallType::Guardrail;
    let is_secret_scan = popup_type == DebugFirewallType::SecretScan;
    let guardrail_details = if is_guardrail {
        Some(lr_mcp::gateway::firewall::GuardrailApprovalDetails {
            verdicts: vec![serde_json::json!({
                "model_id": "llamaguard-3",
                "is_safe": false,
                "flagged_categories": [
                    {"category": "violence", "confidence": 0.92},
                    {"category": "weapons", "confidence": 0.87}
                ],
                "check_duration_ms": 145,
                "raw_output": "unsafe\nS1,S2"
            })],
            actions_required: vec![
                serde_json::json!({"category": "violence", "action": "ask"}),
                serde_json::json!({"category": "weapons", "action": "ask"}),
            ],
            total_duration_ms: 145,
            scan_direction: "request".to_string(),
            flagged_text: "Write a guide on making explosives".to_string(),
        })
    } else {
        None
    };

    let secret_scan_details = if is_secret_scan {
        Some(lr_mcp::gateway::firewall::SecretScanApprovalDetails {
            findings: vec![
                lr_mcp::gateway::firewall::SecretFindingSummary {
                    rule_id: "aws-access-key-id".to_string(),
                    rule_description: "AWS Access Key ID".to_string(),
                    category: "Cloud Provider".to_string(),
                    matched_text: "AKIA**********MPLE".to_string(),
                    entropy: 3.42,
                },
                lr_mcp::gateway::firewall::SecretFindingSummary {
                    rule_id: "github-pat".to_string(),
                    rule_description: "GitHub Personal Access Token".to_string(),
                    category: "Version Control".to_string(),
                    matched_text: "ghp_AB********************ghij".to_string(),
                    entropy: 4.12,
                },
            ],
            scan_duration_ms: 1,
        })
    } else {
        None
    };

    let mut sessions = vec![DebugSession {
        tool_name: tool_name.clone(),
        server_name: server_name.clone(),
        arguments_preview: arguments_preview.clone(),
        is_model_request,
        is_free_tier_fallback,
        is_guardrail_request: is_guardrail,
        is_secret_scan_request: is_secret_scan,
        guardrail_details: guardrail_details.clone(),
        secret_scan_details: secret_scan_details.clone(),
    }];

    if send_multiple {
        // Session 2: same resource (duplicate)
        sessions.push(DebugSession {
            tool_name: tool_name.clone(),
            server_name: server_name.clone(),
            arguments_preview: arguments_preview.clone(),
            is_model_request,
            is_free_tier_fallback,
            is_guardrail_request: is_guardrail,
            is_secret_scan_request: is_secret_scan,
            guardrail_details: guardrail_details.clone(),
            secret_scan_details: secret_scan_details.clone(),
        });

        // Session 3: different resource
        let (alt_tool, alt_server, alt_preview, alt_model, alt_free_tier) = match popup_type {
            DebugFirewallType::McpTool => (
                "github__create_issue".to_string(),
                "github".to_string(),
                r#"{"repo": "test/repo", "title": "Test issue"}"#.to_string(),
                false,
                false,
            ),
            DebugFirewallType::LlmModel => (
                "gpt-4o".to_string(),
                "openai".to_string(),
                r#"{"prompt": "Write a poem", "max_tokens": 500}"#.to_string(),
                true,
                false,
            ),
            DebugFirewallType::Skill => (
                "skill_sysinfo_run_main".to_string(),
                "sysinfo".to_string(),
                r#"{"command": "uptime"}"#.to_string(),
                false,
                false,
            ),
            DebugFirewallType::Marketplace => (
                "marketplace__run_lint".to_string(),
                "marketplace".to_string(),
                r#"{"target": "src/", "fix": true}"#.to_string(),
                false,
                false,
            ),
            DebugFirewallType::FreeTierFallback => (
                "Free-Tier Fallback".to_string(),
                "Paid Models".to_string(),
                "groq/llama-3-70b, gemini/gemini-1.5-flash".to_string(),
                false,
                true,
            ),
            DebugFirewallType::CodingAgent => (
                "coding_agent_start".to_string(),
                "coding-agents".to_string(),
                r#"{"task": "Fix failing tests in utils module", "working_directory": "/tmp/other-project"}"#.to_string(),
                false,
                false,
            ),
            DebugFirewallType::Guardrail => (
                "gpt-4o".to_string(),
                "openai".to_string(),
                r#"{"prompt": "Tell me how to hurt myself", "max_tokens": 1000}"#.to_string(),
                false,
                false,
            ),
            DebugFirewallType::SecretScan => (
                "gpt-4o".to_string(),
                "Secret Scan".to_string(),
                "[Database] PostgreSQL Connection URI (entropy: 3.85)".to_string(),
                false,
                false,
            ),
        };
        let alt_guardrail_details = if is_guardrail {
            Some(lr_mcp::gateway::firewall::GuardrailApprovalDetails {
                verdicts: vec![serde_json::json!({
                    "model_id": "llamaguard-3",
                    "is_safe": false,
                    "flagged_categories": [
                        {"category": "self_harm", "confidence": 0.95}
                    ],
                    "check_duration_ms": 120,
                    "raw_output": "unsafe\nS7"
                })],
                actions_required: vec![
                    serde_json::json!({"category": "self_harm", "action": "block"}),
                ],
                total_duration_ms: 120,
                scan_direction: "request".to_string(),
                flagged_text: "Tell me how to hurt myself".to_string(),
            })
        } else {
            None
        };
        let alt_secret_scan_details = if is_secret_scan {
            Some(lr_mcp::gateway::firewall::SecretScanApprovalDetails {
                findings: vec![lr_mcp::gateway::firewall::SecretFindingSummary {
                    rule_id: "postgres-uri".to_string(),
                    rule_description: "PostgreSQL Connection URI".to_string(),
                    category: "Database".to_string(),
                    matched_text: "postg...( 45 chars)...5432".to_string(),
                    entropy: 3.85,
                }],
                scan_duration_ms: 0,
            })
        } else {
            None
        };
        sessions.push(DebugSession {
            tool_name: alt_tool,
            server_name: alt_server,
            arguments_preview: alt_preview,
            is_model_request: alt_model,
            is_free_tier_fallback: alt_free_tier,
            is_guardrail_request: is_guardrail,
            is_secret_scan_request: is_secret_scan,
            guardrail_details: alt_guardrail_details,
            secret_scan_details: alt_secret_scan_details,
        });
    }

    for (i, debug_session) in sessions.into_iter().enumerate() {
        let request_id = uuid::Uuid::new_v4().to_string();
        let timeout_secs: u64 = 86400; // 24 hours — match real popup default

        let full_arguments: Option<serde_json::Value> =
            serde_json::from_str(&debug_session.arguments_preview).ok();

        let session = lr_mcp::gateway::firewall::FirewallApprovalSession {
            request_id: request_id.clone(),
            client_id: "debug-client".to_string(),
            client_name: "Debug Test Client".to_string(),
            tool_name: debug_session.tool_name,
            server_name: debug_session.server_name,
            arguments_preview: debug_session.arguments_preview,
            full_arguments,
            response_sender: None,
            created_at: std::time::Instant::now(),
            is_auto_router_request: false,
            is_mcp_via_llm_request: false,
            timeout_seconds: timeout_secs,
            is_model_request: debug_session.is_model_request,
            is_guardrail_request: debug_session.is_guardrail_request,
            is_free_tier_fallback: debug_session.is_free_tier_fallback,
            is_secret_scan_request: debug_session.is_secret_scan_request,
            guardrail_details: debug_session.guardrail_details,
            secret_scan_details: debug_session.secret_scan_details,
        };

        firewall_manager.insert_pending(request_id.clone(), session);

        // Small delay between windows to avoid overlap
        if i > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }

        // Create the firewall approval popup window
        use tauri::WebviewWindowBuilder;
        match WebviewWindowBuilder::new(
            &app,
            format!("firewall-approval-{}", request_id),
            tauri::WebviewUrl::App("index.html".into()),
        )
        .title("Approval Required")
        .inner_size(400.0, 320.0)
        .center()
        .visible(false)
        .resizable(false)
        .decorations(true)
        .always_on_top(true)
        .build()
        {
            Ok(window) => {
                let _ = window.set_focus();
                tracing::info!(
                    "Debug firewall popup ({:?}) opened for request {} (#{}/{})",
                    popup_type,
                    request_id,
                    i + 1,
                    if send_multiple { 3 } else { 1 }
                );
            }
            Err(e) => {
                tracing::error!("Failed to create debug firewall popup: {}", e);
            }
        }
    }

    // Rebuild tray menu to show the pending approval item(s)
    if let Err(e) = crate::ui::tray::rebuild_tray_menu(&app) {
        tracing::warn!("Failed to rebuild tray menu for firewall approval: {}", e);
    }

    // Trigger immediate tray icon update to show the question mark overlay
    if let Some(tray_graph_manager) = app.try_state::<Arc<crate::ui::tray::TrayGraphManager>>() {
        tray_graph_manager.notify_activity();
    }

    Ok(())
}

/// Debug tray overlay type for testing icon states
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DebugTrayOverlay {
    /// No overlay (normal icon)
    None,
    /// Yellow exclamation (degraded health)
    WarningYellow,
    /// Red exclamation (unhealthy)
    WarningRed,
    /// Down arrow (update available)
    UpdateAvailable,
    /// Green question mark (firewall pending)
    FirewallPending,
}

/// Set a debug override for the tray icon overlay.
///
/// Pass a specific overlay type to force that appearance, or `null`
/// to clear the override and return to normal behavior.
#[tauri::command]
pub async fn debug_set_tray_overlay(
    overlay: Option<DebugTrayOverlay>,
    app: AppHandle,
) -> Result<(), String> {
    use crate::ui::tray_graph::{StatusDotColors, TrayOverlay};

    let dark_mode = crate::ui::tray_graph_manager::detect_dark_mode(&app);

    let tray_overlay = overlay.map(|o| match o {
        DebugTrayOverlay::None => TrayOverlay::None,
        DebugTrayOverlay::WarningYellow => TrayOverlay::Warning(StatusDotColors::yellow(dark_mode)),
        DebugTrayOverlay::WarningRed => TrayOverlay::Warning(StatusDotColors::red(dark_mode)),
        DebugTrayOverlay::UpdateAvailable => TrayOverlay::UpdateAvailable,
        DebugTrayOverlay::FirewallPending => TrayOverlay::FirewallPending,
    });

    let tray_graph_manager = app
        .try_state::<Arc<crate::ui::tray::TrayGraphManager>>()
        .ok_or("TrayGraphManager not available")?;
    tray_graph_manager.set_debug_overlay(tray_overlay);

    Ok(())
}

// ============================================================================
// Local Provider Discovery Commands
// ============================================================================

/// Result of local provider discovery scan
#[derive(Serialize)]
pub struct DiscoverProviderResult {
    /// All providers detected as running
    pub discovered: Vec<lr_providers::factory::DiscoveredProvider>,
    /// Names of providers that were newly added
    pub added: Vec<String>,
    /// Names of providers already configured (skipped)
    pub skipped: Vec<String>,
}

/// Discover local LLM providers and add any new ones to the configuration.
///
/// Scans default ports for known local providers (Ollama, LM Studio, Jan, GPT4All).
/// Providers that are already configured are skipped.
#[tauri::command]
pub async fn debug_discover_providers(
    registry: State<'_, Arc<ProviderRegistry>>,
    config_manager: State<'_, ConfigManager>,
    app: AppHandle,
) -> Result<DiscoverProviderResult, String> {
    let discovered = lr_providers::factory::discover_local_providers().await;

    let mut added = Vec::new();
    let mut skipped = Vec::new();

    // Check which providers are already configured
    let existing_providers: Vec<String> = config_manager
        .get()
        .providers
        .iter()
        .map(|p| format!("{:?}", p.provider_type).to_lowercase())
        .collect();

    for provider in &discovered {
        if existing_providers.contains(&provider.provider_type) {
            skipped.push(provider.instance_name.clone());
            continue;
        }

        // Get the default config for this provider type
        let provider_config = match provider.provider_type.as_str() {
            "ollama" => lr_config::ProviderConfig::default_ollama(),
            "lmstudio" => lr_config::ProviderConfig::default_lmstudio(),
            "jan" => lr_config::ProviderConfig::default_jan(),
            "gpt4all" => lr_config::ProviderConfig::default_gpt4all(),
            _ => {
                tracing::warn!(
                    "Unknown discovered provider type: {}",
                    provider.provider_type
                );
                continue;
            }
        };

        // Create in registry
        let mut config_map = std::collections::HashMap::new();
        if let Some(ref cfg) = provider_config.provider_config {
            if let Some(obj) = cfg.as_object() {
                for (key, value) in obj {
                    if let Some(value_str) = value.as_str() {
                        config_map.insert(key.clone(), value_str.to_string());
                    } else {
                        config_map.insert(key.clone(), value.to_string());
                    }
                }
            }
        }

        if let Err(e) = registry
            .create_provider(
                provider_config.name.clone(),
                provider.provider_type.clone(),
                config_map,
            )
            .await
        {
            tracing::warn!(
                "Failed to create provider '{}': {}",
                provider.instance_name,
                e
            );
            continue;
        }

        // Add to config
        let name = provider_config.name.clone();
        let config_clone = provider_config.clone();
        if let Err(e) = config_manager.update(|cfg| {
            cfg.providers.push(config_clone.clone());
        }) {
            tracing::warn!("Failed to save provider config for '{}': {}", name, e);
            continue;
        }

        added.push(provider.instance_name.clone());
    }

    // Save config and emit events if any providers were added
    if !added.is_empty() {
        if let Err(e) = config_manager.save().await {
            tracing::warn!("Failed to persist config after discovery: {}", e);
        }
        let _ = app.emit("providers-changed", ());
        let _ = app.emit("models-changed", ());
    }

    Ok(DiscoverProviderResult {
        discovered,
        added,
        skipped,
    })
}

// ============================================================================
// GuardRails Configuration Commands
// ============================================================================

/// Get the current guardrails configuration
#[tauri::command]
pub async fn get_guardrails_config(
    config_manager: State<'_, ConfigManager>,
) -> Result<serde_json::Value, String> {
    let config = config_manager.get();
    serde_json::to_value(&config.guardrails).map_err(|e| e.to_string())
}

/// Update guardrails configuration
#[tauri::command]
pub async fn update_guardrails_config(
    config_json: String,
    config_manager: State<'_, ConfigManager>,
) -> Result<(), String> {
    let new_config: lr_config::GuardrailsConfig =
        serde_json::from_str(&config_json).map_err(|e| format!("Invalid config JSON: {}", e))?;

    config_manager
        .update(|config| {
            config.guardrails = new_config;
        })
        .map_err(|e| e.to_string())?;

    config_manager.save().await.map_err(|e| e.to_string())?;

    Ok(())
}

/// Rebuild the safety engine from current config.
/// Called after config changes (add/remove/enable/disable models, download complete, etc.)
#[tauri::command]
pub async fn rebuild_safety_engine(
    app_handle: tauri::AppHandle,
    config_manager: State<'_, ConfigManager>,
    state: State<'_, Arc<lr_server::state::AppState>>,
) -> Result<(), String> {
    let config = config_manager.get();
    let guardrails_config = &config.guardrails;

    if !guardrails_config.safety_models.is_empty() {
        // Build provider lookup
        let mut provider_lookup = HashMap::new();
        for p in &config.providers {
            if !p.enabled {
                continue;
            }
            let provider_type_str = match p.provider_type {
                lr_config::ProviderType::Ollama => "ollama",
                lr_config::ProviderType::LMStudio => "lmstudio",
                _ => "openai_compatible",
            };

            let endpoint = p
                .provider_config
                .as_ref()
                .and_then(|cfg| cfg.get("endpoint"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| match p.provider_type {
                    lr_config::ProviderType::Ollama => "http://localhost:11434".to_string(),
                    lr_config::ProviderType::LMStudio => "http://localhost:1234".to_string(),
                    _ => "http://localhost:8080".to_string(),
                });

            let api_key = p
                .provider_config
                .as_ref()
                .and_then(|cfg| cfg.get("api_key"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());

            provider_lookup.insert(
                p.name.clone(),
                lr_guardrails::ProviderInfo {
                    name: p.name.clone(),
                    base_url: endpoint,
                    api_key,
                    provider_type: provider_type_str.to_string(),
                },
            );
        }

        let model_inputs: Vec<lr_guardrails::SafetyModelConfigInput> = guardrails_config
            .safety_models
            .iter()
            .map(|m| lr_guardrails::SafetyModelConfigInput {
                id: m.id.clone(),
                model_type: m.model_type.clone(),
                provider_id: m.provider_id.clone(),
                model_name: m.model_name.clone(),
                enabled_categories: None,
            })
            .collect();

        let engine = Arc::new(lr_guardrails::SafetyEngine::from_config(
            &model_inputs,
            guardrails_config.default_confidence_threshold,
            &provider_lookup,
        ));

        // Emit load-failed events for models that couldn't be loaded
        for (model_id, error) in engine.load_errors() {
            let _ = app_handle.emit(
                "safety-model-load-failed",
                serde_json::json!({ "model_id": model_id, "error": error }),
            );
            tracing::warn!("Safety model '{}' failed to load: {}", model_id, error);
        }

        tracing::info!(
            "Safety engine rebuilt: {} models loaded, {} failed",
            engine.model_count(),
            engine.load_errors().len(),
        );
        state.replace_safety_engine(engine);
    } else {
        state.replace_safety_engine(Arc::new(lr_guardrails::SafetyEngine::empty()));
        tracing::info!("Safety engine rebuilt: no models configured");
    }

    Ok(())
}

/// Rebuild the secret scanner engine from current config.
/// Called on startup and when secret scanning settings change.
#[tauri::command]
pub async fn rebuild_secret_scanner(
    config_manager: State<'_, ConfigManager>,
    state: State<'_, Arc<lr_server::state::AppState>>,
) -> Result<(), String> {
    let config = config_manager.get();
    let ss_config = &config.secret_scanning;

    if ss_config.action == lr_config::SecretScanAction::Off {
        // Clear the scanner when scanning is disabled
        *state.secret_scanner.write() = None;
        tracing::info!("Secret scanner cleared (scanning is off)");
        return Ok(());
    }

    let engine_config = lr_secret_scanner::SecretScanEngineConfig {
        entropy_threshold: ss_config.entropy_threshold,
        allowlist: ss_config.allowlist.clone(),
        scan_system_messages: ss_config.scan_system_messages,
    };

    match lr_secret_scanner::SecretScanEngine::new(&engine_config) {
        Ok(engine) => {
            *state.secret_scanner.write() = Some(Arc::new(engine));
            tracing::info!("Secret scanner rebuilt successfully");
            Ok(())
        }
        Err(e) => {
            tracing::error!("Failed to rebuild secret scanner: {}", e);
            Err(format!("Failed to rebuild secret scanner: {}", e))
        }
    }
}

/// Get secret scanning global configuration
#[tauri::command]
pub async fn get_secret_scanning_config(
    config_manager: State<'_, ConfigManager>,
) -> Result<serde_json::Value, String> {
    let config = config_manager.get();
    serde_json::to_value(&config.secret_scanning).map_err(|e| e.to_string())
}

/// Update secret scanning global configuration
#[tauri::command]
pub async fn update_secret_scanning_config(
    config_json: String,
    config_manager: State<'_, ConfigManager>,
    state: State<'_, Arc<lr_server::state::AppState>>,
) -> Result<(), String> {
    let new_config: lr_config::SecretScanningConfig =
        serde_json::from_str(&config_json).map_err(|e| format!("Invalid config JSON: {}", e))?;

    config_manager
        .update(|config| {
            config.secret_scanning = new_config;
        })
        .map_err(|e| e.to_string())?;

    config_manager.save().await.map_err(|e| e.to_string())?;

    // Rebuild the scanner with new config
    rebuild_secret_scanner(config_manager, state).await?;

    Ok(())
}

/// Test secret scanning against input text.
/// Always works even when scanning is Off — builds a temporary scanner from current config.
/// Accepts an optional entropy_threshold override for testing different thresholds.
#[tauri::command]
pub async fn test_secret_scan(
    input: String,
    entropy_threshold: Option<f32>,
    config_manager: State<'_, ConfigManager>,
) -> Result<serde_json::Value, String> {
    let config = config_manager.get();
    let ss_config = &config.secret_scanning;

    let engine_config = lr_secret_scanner::SecretScanEngineConfig {
        entropy_threshold: entropy_threshold.unwrap_or(ss_config.entropy_threshold),
        allowlist: ss_config.allowlist.clone(),
        scan_system_messages: true, // Always scan everything in test mode
    };

    let scanner = lr_secret_scanner::SecretScanEngine::new(&engine_config)
        .map_err(|e| format!("Failed to build test scanner: {}", e))?;

    let texts = vec![lr_secret_scanner::ExtractedText {
        label: "test".to_string(),
        text: input,
        message_index: 0,
    }];

    let result = scanner.scan(&texts);
    serde_json::to_value(&result).map_err(|e| e.to_string())
}

/// List all compiled secret scanning patterns (builtin + custom)
#[tauri::command]
pub async fn get_secret_scanning_patterns(
    config_manager: State<'_, ConfigManager>,
) -> Result<serde_json::Value, String> {
    let config = config_manager.get();
    let ss_config = &config.secret_scanning;

    let engine = lr_secret_scanner::regex_engine::RegexEngine::new(
        ss_config.entropy_threshold,
        &ss_config.allowlist,
    )
    .map_err(|e| format!("Failed to build engine: {}", e))?;

    serde_json::to_value(engine.rule_metadata()).map_err(|e| e.to_string())
}

/// Test safety check against input text (runs all enabled models).
/// When `client_id` is provided, applies the client's category actions to the result.
#[tauri::command]
pub async fn test_safety_check(
    text: String,
    client_id: Option<String>,
    state: State<'_, Arc<lr_server::state::AppState>>,
    config_manager: State<'_, ConfigManager>,
) -> Result<serde_json::Value, String> {
    let engine = state.safety_engine.read().clone();
    let Some(engine) = engine else {
        return Err("Safety engine not initialized".to_string());
    };

    let result = engine
        .check_text(&text, lr_guardrails::ScanDirection::Input)
        .await;

    // Apply client-specific category actions if a client_id was provided
    let result = if let Some(ref cid) = client_id {
        let config = config_manager.get();
        if let Some(client) = config.clients.iter().find(|c| c.id == *cid) {
            let overrides: Vec<(String, lr_guardrails::CategoryAction)> = client
                .guardrails
                .category_actions
                .as_deref()
                .unwrap_or_default()
                .iter()
                .filter_map(|entry| {
                    let action: lr_guardrails::CategoryAction =
                        serde_json::from_value(serde_json::Value::String(entry.action.clone()))
                            .ok()?;
                    Some((entry.category.clone(), action))
                })
                .collect();
            result.apply_client_category_overrides(&overrides)
        } else {
            result
        }
    } else {
        result
    };

    serde_json::to_value(&result).map_err(|e| e.to_string())
}

/// Get status of a safety model (is provider configured? is model available?)
#[tauri::command]
pub async fn get_safety_model_status(
    model_id: String,
    config_manager: State<'_, ConfigManager>,
) -> Result<serde_json::Value, String> {
    let config = config_manager.get();
    let model = config
        .guardrails
        .safety_models
        .iter()
        .find(|m| m.id == model_id)
        .ok_or_else(|| format!("Safety model '{}' not found", model_id))?;

    // Check if provider is configured and enabled
    let provider_configured = if let Some(ref provider_id) = model.provider_id {
        config
            .providers
            .iter()
            .any(|p| p.name == *provider_id && p.enabled)
    } else {
        false
    };

    Ok(serde_json::json!({
        "id": model.id,
        "label": model.label,
        "model_type": model.model_type,
        "provider_configured": provider_configured,
        "provider_id": model.provider_id,
        "model_name": model.model_name,
        "available": provider_configured,
    }))
}

/// Test a single safety model against text
#[tauri::command]
pub async fn test_safety_model(
    model_id: String,
    text: String,
    state: State<'_, Arc<lr_server::state::AppState>>,
) -> Result<serde_json::Value, String> {
    let engine = state.safety_engine.read().clone();
    let Some(engine) = engine else {
        return Err("Safety engine not initialized".to_string());
    };

    let result = engine
        .check_text_single_model(&text, lr_guardrails::ScanDirection::Input, &model_id)
        .await;

    serde_json::to_value(&result.verdicts).map_err(|e| e.to_string())
}

/// Get all safety categories with which models support them
#[tauri::command]
pub async fn get_all_safety_categories(
    state: State<'_, Arc<lr_server::state::AppState>>,
) -> Result<serde_json::Value, String> {
    if state.safety_engine.read().is_none() {
        return Ok(serde_json::json!([]));
    };

    // Return a static list of all known categories
    // Fields match the TypeScript SafetyCategoryInfo interface:
    //   category, display_name, description, supported_by
    let categories = vec![
        serde_json::json!({"category": "violent_crimes", "display_name": "Violent Crimes", "description": "Content promoting or depicting violence or violent crimes", "supported_by": ["llama_guard", "nemotron", "granite_guardian"]}),
        serde_json::json!({"category": "non_violent_crimes", "display_name": "Non-Violent Crimes", "description": "Content related to non-violent criminal activities", "supported_by": ["llama_guard", "nemotron"]}),
        serde_json::json!({"category": "sex_crimes", "display_name": "Sex Crimes", "description": "Content related to sex-related crimes", "supported_by": ["llama_guard", "nemotron"]}),
        serde_json::json!({"category": "child_exploitation", "display_name": "Child Exploitation", "description": "Content involving child sexual exploitation", "supported_by": ["llama_guard", "nemotron"]}),
        serde_json::json!({"category": "defamation", "display_name": "Defamation", "description": "Content that defames individuals or groups", "supported_by": ["llama_guard", "nemotron"]}),
        serde_json::json!({"category": "specialized_advice", "display_name": "Specialized Advice", "description": "Unqualified professional advice (medical, legal, financial)", "supported_by": ["llama_guard", "nemotron"]}),
        serde_json::json!({"category": "privacy", "display_name": "Privacy", "description": "Content that violates personal privacy", "supported_by": ["llama_guard", "nemotron"]}),
        serde_json::json!({"category": "intellectual_property", "display_name": "Intellectual Property", "description": "Content that infringes intellectual property rights", "supported_by": ["llama_guard", "nemotron"]}),
        serde_json::json!({"category": "indiscriminate_weapons", "display_name": "Indiscriminate Weapons", "description": "Content related to weapons of mass destruction", "supported_by": ["llama_guard", "nemotron"]}),
        serde_json::json!({"category": "hate", "display_name": "Hate", "description": "Content promoting hatred or discrimination", "supported_by": ["llama_guard", "nemotron", "shield_gemma", "granite_guardian"]}),
        serde_json::json!({"category": "self_harm", "display_name": "Self-Harm", "description": "Content promoting self-harm or suicide", "supported_by": ["llama_guard", "nemotron"]}),
        serde_json::json!({"category": "sexual_content", "display_name": "Sexual Content", "description": "Sexually explicit or inappropriate content", "supported_by": ["llama_guard", "nemotron", "shield_gemma", "granite_guardian"]}),
        serde_json::json!({"category": "elections", "display_name": "Elections", "description": "Content related to election interference", "supported_by": ["llama_guard", "nemotron"]}),
        serde_json::json!({"category": "code_interpreter_abuse", "display_name": "Code Interpreter Abuse", "description": "Attempts to abuse code interpreter capabilities", "supported_by": ["llama_guard", "nemotron"]}),
        serde_json::json!({"category": "dangerous_content", "display_name": "Dangerous Content", "description": "Content facilitating harmful activities", "supported_by": ["shield_gemma"]}),
        serde_json::json!({"category": "harassment", "display_name": "Harassment", "description": "Content targeting individuals with harmful intent", "supported_by": ["shield_gemma"]}),
        serde_json::json!({"category": "jailbreak", "display_name": "Jailbreak", "description": "Attempts to bypass AI safety guidelines", "supported_by": ["granite_guardian"]}),
        serde_json::json!({"category": "social_bias", "display_name": "Social Bias", "description": "Content reinforcing social stereotypes or biases", "supported_by": ["granite_guardian"]}),
        serde_json::json!({"category": "profanity", "display_name": "Profanity", "description": "Vulgar or offensive language", "supported_by": ["nemotron", "granite_guardian"]}),
        serde_json::json!({"category": "unethical_behavior", "display_name": "Unethical Behavior", "description": "Actions that violate ethical norms", "supported_by": ["granite_guardian"]}),
        serde_json::json!({"category": "context_relevance", "display_name": "Context Relevance (RAG)", "description": "Retrieved context is not relevant to the query", "supported_by": ["granite_guardian"]}),
        serde_json::json!({"category": "groundedness", "display_name": "Groundedness (RAG)", "description": "Response not grounded in provided context", "supported_by": ["granite_guardian"]}),
        serde_json::json!({"category": "answer_relevance", "display_name": "Answer Relevance (RAG)", "description": "Response does not address the original question", "supported_by": ["granite_guardian"]}),
    ];

    Ok(serde_json::json!(categories))
}

// ============================================================================
// Safety Model Management Commands
// ============================================================================

/// Add a new safety model to the configuration
#[tauri::command]
pub async fn add_safety_model(
    config_json: String,
    config_manager: State<'_, ConfigManager>,
) -> Result<(), String> {
    let mut new_model: lr_config::SafetyModelConfig = serde_json::from_str(&config_json)
        .map_err(|e| format!("Invalid model config JSON: {}", e))?;

    // Generate unique ID if not provided
    if new_model.id.is_empty() {
        new_model.id = format!(
            "custom_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
    }

    // Check for duplicate ID
    {
        let config = config_manager.get();
        if config
            .guardrails
            .safety_models
            .iter()
            .any(|m| m.id == new_model.id)
        {
            return Err(format!(
                "A safety model with ID '{}' already exists",
                new_model.id
            ));
        }
    }

    config_manager
        .update(|config| {
            config.guardrails.safety_models.push(new_model);
        })
        .map_err(|e| e.to_string())?;

    config_manager.save().await.map_err(|e| e.to_string())?;
    Ok(())
}

/// Remove a safety model from the configuration
#[tauri::command]
pub async fn remove_safety_model(
    model_id: String,
    config_manager: State<'_, ConfigManager>,
) -> Result<(), String> {
    {
        let config = config_manager.get();
        config
            .guardrails
            .safety_models
            .iter()
            .find(|m| m.id == model_id)
            .ok_or_else(|| format!("Safety model '{}' not found", model_id))?;
    }

    config_manager
        .update(|config| {
            config.guardrails.safety_models.retain(|m| m.id != model_id);
        })
        .map_err(|e| e.to_string())?;

    config_manager.save().await.map_err(|e| e.to_string())?;

    Ok(())
}

// ============================================================================
// Provider Model Pull Commands
// ============================================================================

/// Pull (download) a model through a provider that supports it.
///
/// Supports any provider that implements `supports_pull()` (Ollama, LM Studio, LocalAI).
/// Streams progress via `provider-model-pull-progress` Tauri events.
/// Emits `provider-model-pull-complete` or `provider-model-pull-failed` on finish.
#[tauri::command]
pub async fn pull_provider_model(
    provider_id: String,
    model_name: String,
    app_handle: AppHandle,
    provider_registry: State<'_, Arc<ProviderRegistry>>,
) -> Result<(), String> {
    use futures::StreamExt;

    let provider = provider_registry
        .get_provider(&provider_id)
        .ok_or_else(|| format!("Provider '{}' not found or disabled", provider_id))?;

    if !provider.supports_pull() {
        return Err(format!(
            "Provider '{}' does not support model pulling",
            provider_id
        ));
    }

    let model_name_clone = model_name.clone();
    let provider_id_clone = provider_id.clone();

    // Spawn in background so the command returns immediately
    tokio::spawn(async move {
        match provider.pull_model(&model_name_clone).await {
            Ok(mut stream) => {
                while let Some(result) = stream.next().await {
                    match result {
                        Ok(progress) => {
                            let _ = app_handle.emit(
                                "provider-model-pull-progress",
                                serde_json::json!({
                                    "provider_id": provider_id_clone,
                                    "model_name": model_name_clone,
                                    "status": progress.status,
                                    "total": progress.total,
                                    "completed": progress.completed,
                                }),
                            );

                            // "success" status means the pull is complete
                            if progress.status == "success" {
                                let _ = app_handle.emit(
                                    "provider-model-pull-complete",
                                    serde_json::json!({
                                        "provider_id": provider_id_clone,
                                        "model_name": model_name_clone,
                                    }),
                                );
                                return;
                            }
                        }
                        Err(e) => {
                            tracing::warn!("Pull progress error for '{}': {}", model_name_clone, e);
                        }
                    }
                }

                // Stream ended without explicit "success" — treat as complete
                let _ = app_handle.emit(
                    "provider-model-pull-complete",
                    serde_json::json!({
                        "provider_id": provider_id_clone,
                        "model_name": model_name_clone,
                    }),
                );
            }
            Err(e) => {
                let _ = app_handle.emit(
                    "provider-model-pull-failed",
                    serde_json::json!({
                        "provider_id": provider_id_clone,
                        "model_name": model_name_clone,
                        "error": e.to_string(),
                    }),
                );
            }
        }
    });

    Ok(())
}

// ─── Prompt Compression Commands ─────────────────────────────────────────────

/// Get current prompt compression configuration
#[tauri::command]
pub async fn get_compression_config(
    config_manager: State<'_, ConfigManager>,
) -> Result<serde_json::Value, String> {
    let config = config_manager.get();
    serde_json::to_value(&config.prompt_compression).map_err(|e| e.to_string())
}

/// Update prompt compression configuration
#[tauri::command]
pub async fn update_compression_config(
    config_json: String,
    config_manager: State<'_, ConfigManager>,
) -> Result<(), String> {
    let new_config: lr_config::PromptCompressionConfig =
        serde_json::from_str(&config_json).map_err(|e| format!("Invalid config JSON: {}", e))?;

    config_manager
        .update(|config| {
            config.prompt_compression = new_config;
        })
        .map_err(|e| e.to_string())?;

    config_manager.save().await.map_err(|e| e.to_string())?;

    Ok(())
}

/// Get compression service status (model download state, loaded state)
#[tauri::command]
pub async fn get_compression_status(
    state: State<'_, Arc<lr_server::state::AppState>>,
    config_manager: State<'_, ConfigManager>,
) -> Result<serde_json::Value, String> {
    let service = state.compression_service.read().clone();
    let status = if let Some(svc) = service {
        svc.get_status().await
    } else {
        // Service not running — still check filesystem for downloaded model
        let config = config_manager.get();
        let model_size = &config.prompt_compression.model_size;
        let repo = lr_compression::repo_id_for_model(model_size).to_string();

        let config_dir = lr_utils::paths::config_dir().map_err(|e| e.to_string())?;
        let compression_dir = config_dir.join("compression").join(model_size);
        let model_path = compression_dir.join("model");
        let tokenizer_path = compression_dir.join("tokenizer");
        let downloaded = lr_compression::is_downloaded(&model_path, &tokenizer_path);

        let model_size_bytes = if downloaded {
            std::fs::metadata(model_path.join("model.safetensors"))
                .ok()
                .map(|m| m.len())
        } else {
            None
        };

        lr_compression::CompressionStatus {
            model_downloaded: downloaded,
            model_loaded: false,
            model_size_bytes,
            model_repo: repo,
        }
    };
    serde_json::to_value(&status).map_err(|e| e.to_string())
}

/// Download the LLMLingua-2 model from HuggingFace
#[tauri::command]
pub async fn install_compression(
    state: State<'_, Arc<lr_server::state::AppState>>,
    config_manager: State<'_, ConfigManager>,
    app: tauri::AppHandle,
) -> Result<String, String> {
    let config = config_manager.get();
    let svc = ensure_compression_service(&state, &config).await?;
    svc.download(Some(app)).await?;
    Ok("Model downloaded successfully".to_string())
}

/// Rebuild the compression service from current config
#[tauri::command]
pub async fn rebuild_compression_engine(
    state: State<'_, Arc<lr_server::state::AppState>>,
    config_manager: State<'_, ConfigManager>,
) -> Result<(), String> {
    let config = config_manager.get();

    if config.prompt_compression.enabled {
        let svc = Arc::new(lr_compression::CompressionService::new(
            config.prompt_compression.clone(),
        )?);
        *state.compression_service.write() = Some(svc);
        tracing::info!("Compression service rebuilt");
    } else {
        let existing = state.compression_service.read().clone();
        if let Some(svc) = existing.as_ref() {
            svc.unload().await;
        }
        *state.compression_service.write() = None;
        tracing::info!("Compression service disabled");
    }

    Ok(())
}

/// Test compression on sample text (for try-it-out tab)
#[tauri::command]
pub async fn test_compression(
    text: String,
    rate: f32,
    preserve_quoted: bool,
    compression_notice: bool,
    state: State<'_, Arc<lr_server::state::AppState>>,
    config_manager: State<'_, ConfigManager>,
) -> Result<serde_json::Value, String> {
    let config = config_manager.get();
    let svc = ensure_compression_service(&state, &config).await?;

    let (compressed_text, original_tokens, compressed_tokens, kept_indices, protected_indices) =
        svc.compress_text(&text, rate, preserve_quoted).await?;

    let (compressed_text, compressed_tokens) = if compression_notice {
        (
            format!("[abridged] {}", compressed_text),
            compressed_tokens + 1,
        )
    } else {
        (compressed_text, compressed_tokens)
    };

    let ratio = if compressed_tokens > 0 {
        original_tokens as f32 / compressed_tokens as f32
    } else {
        1.0
    };

    serde_json::to_value(serde_json::json!({
        "compressed_text": compressed_text,
        "original_tokens": original_tokens,
        "compressed_tokens": compressed_tokens,
        "ratio": ratio,
        "kept_indices": kept_indices,
        "protected_indices": protected_indices,
    }))
    .map_err(|e| e.to_string())
}

/// Helper: ensure a CompressionService exists in state, creating one if needed
async fn ensure_compression_service(
    state: &Arc<lr_server::state::AppState>,
    config: &lr_config::AppConfig,
) -> Result<Arc<lr_compression::CompressionService>, String> {
    let existing = state.compression_service.read().clone();
    if let Some(svc) = existing {
        return Ok(svc);
    }
    let svc = Arc::new(lr_compression::CompressionService::new(
        config.prompt_compression.clone(),
    )?);
    *state.compression_service.write() = Some(svc.clone());
    Ok(svc)
}

// ============================================================================
// JSON Repair commands
// ============================================================================

/// Get current JSON repair configuration
#[tauri::command]
pub async fn get_json_repair_config(
    config_manager: State<'_, ConfigManager>,
) -> Result<serde_json::Value, String> {
    let config = config_manager.get();
    serde_json::to_value(&config.json_repair).map_err(|e| e.to_string())
}

/// Update JSON repair configuration
#[tauri::command]
pub async fn update_json_repair_config(
    config_json: String,
    config_manager: State<'_, ConfigManager>,
) -> Result<(), String> {
    let new_config: lr_config::JsonRepairConfig =
        serde_json::from_str(&config_json).map_err(|e| format!("Invalid config JSON: {}", e))?;

    config_manager
        .update(|config| {
            config.json_repair = new_config;
        })
        .map_err(|e| e.to_string())?;

    config_manager.save().await.map_err(|e| e.to_string())?;

    Ok(())
}

/// Test JSON repair on sample input
#[tauri::command]
pub async fn test_json_repair(
    content: String,
    schema: Option<String>,
) -> Result<serde_json::Value, String> {
    let schema_value = if let Some(s) = schema {
        Some(
            serde_json::from_str::<serde_json::Value>(&s)
                .map_err(|e| format!("Invalid schema JSON: {}", e))?,
        )
    } else {
        None
    };

    let options = lr_json_repair::RepairOptions {
        syntax_repair: true,
        schema_coercion: schema_value.is_some(),
        strip_extra_fields: true,
        add_defaults: true,
        normalize_enums: true,
    };

    let result = lr_json_repair::repair_content(&content, schema_value.as_ref(), &options);

    serde_json::to_value(&result).map_err(|e| e.to_string())
}

// ============================================================================
// Sampling Approval Commands
// ============================================================================

/// Get details of a pending sampling approval request (for popup display)
#[tauri::command]
pub async fn get_sampling_approval_details(
    request_id: String,
    state: State<'_, Arc<lr_server::state::AppState>>,
) -> Result<serde_json::Value, String> {
    match state.sampling_approval_manager.get_details(&request_id) {
        Some(details) => serde_json::to_value(&details).map_err(|e| e.to_string()),
        None => Err(format!(
            "Sampling approval request {} not found",
            request_id
        )),
    }
}

/// Submit an approval decision for a sampling request
#[tauri::command]
pub async fn submit_sampling_approval(
    request_id: String,
    action: String,
    state: State<'_, Arc<lr_server::state::AppState>>,
) -> Result<(), String> {
    let approval_action = match action.as_str() {
        "allow" => lr_mcp::gateway::sampling_approval::SamplingApprovalAction::Allow,
        "deny" => lr_mcp::gateway::sampling_approval::SamplingApprovalAction::Deny,
        _ => return Err(format!("Invalid action: {}", action)),
    };

    state
        .sampling_approval_manager
        .submit_approval(&request_id, approval_action)
        .map_err(|e| e.to_string())
}

// ============================================================================
// Elicitation Commands
// ============================================================================

/// Get details of a pending elicitation request (for popup display)
#[tauri::command]
pub async fn get_elicitation_details(
    request_id: String,
    state: State<'_, Arc<lr_server::state::AppState>>,
) -> Result<serde_json::Value, String> {
    let manager = state.mcp_gateway.get_elicitation_manager();

    match manager.get_details(&request_id) {
        Some(details) => serde_json::to_value(&details).map_err(|e| e.to_string()),
        None => Err(format!("Elicitation request {} not found", request_id)),
    }
}

/// Submit a response to a pending elicitation request
#[tauri::command]
pub async fn submit_elicitation_response(
    request_id: String,
    data: serde_json::Value,
    state: State<'_, Arc<lr_server::state::AppState>>,
) -> Result<(), String> {
    let response = lr_mcp::protocol::ElicitationResponse { data };

    state
        .mcp_gateway
        .get_elicitation_manager()
        .submit_response(&request_id, response)
        .map_err(|e| e.to_string())
}

/// Cancel a pending elicitation request
#[tauri::command]
pub async fn cancel_elicitation(
    request_id: String,
    state: State<'_, Arc<lr_server::state::AppState>>,
) -> Result<(), String> {
    state
        .mcp_gateway
        .get_elicitation_manager()
        .cancel_request(&request_id)
        .map_err(|e| e.to_string())
}

// ============================================================================
// Debug: Sampling & Elicitation Popup Triggers
// ============================================================================

/// Debug: trigger a sampling approval popup with fake data
#[tauri::command]
pub async fn debug_trigger_sampling_approval_popup(
    state: State<'_, Arc<lr_server::state::AppState>>,
) -> Result<(), String> {
    use lr_mcp::protocol::{SamplingContent, SamplingMessage, SamplingRequest};

    let request_id = uuid::Uuid::new_v4().to_string();

    let sampling_request = SamplingRequest {
        messages: vec![SamplingMessage {
            role: "user".to_string(),
            content: SamplingContent::Text("What is the capital of France?".to_string()),
        }],
        model_preferences: None,
        system_prompt: Some("You are a helpful geography assistant.".to_string()),
        temperature: Some(0.7),
        max_tokens: Some(500),
        stop_sequences: None,
        metadata: None,
    };

    // Create the pending approval session
    let manager = state.sampling_approval_manager.clone();
    let rid = request_id.clone();
    tokio::spawn(async move {
        let _ = manager
            .request_approval(rid, "debug-server".to_string(), sampling_request, None)
            .await;
    });

    // The broadcast listener in main.rs will create the popup window
    tracing::info!("Debug sampling approval request created: {}", request_id);

    Ok(())
}

/// Debug: trigger an elicitation form popup with fake data
#[tauri::command]
pub async fn debug_trigger_elicitation_form_popup(
    state: State<'_, Arc<lr_server::state::AppState>>,
) -> Result<(), String> {
    use lr_mcp::protocol::ElicitationRequest;

    let request_id = uuid::Uuid::new_v4().to_string();

    let elicitation_request = ElicitationRequest {
        message: "Please provide your deployment configuration:".to_string(),
        schema: serde_json::json!({
            "type": "object",
            "properties": {
                "environment": {
                    "type": "string",
                    "title": "Environment",
                    "description": "Target environment",
                    "enum": ["development", "staging", "production"]
                },
                "confirm_deploy": {
                    "type": "boolean",
                    "title": "Confirm deployment",
                    "default": false
                },
                "max_instances": {
                    "type": "number",
                    "title": "Max instances",
                    "description": "Maximum number of instances",
                    "default": 3
                },
                "notes": {
                    "type": "string",
                    "title": "Notes",
                    "description": "Any additional notes"
                }
            }
        }),
    };

    // Create the pending elicitation session
    let manager = state.mcp_gateway.get_elicitation_manager().clone();
    tokio::spawn(async move {
        let _ = manager
            .request_input("debug-server".to_string(), elicitation_request, None)
            .await;
    });

    // The broadcast listener in main.rs will create the popup window
    tracing::info!("Debug elicitation form request created: {}", request_id);

    Ok(())
}

// ============================================================================
// Memory Configuration Commands
// ============================================================================

/// Get the current memory configuration
#[tauri::command]
pub async fn get_memory_config(
    config_manager: State<'_, ConfigManager>,
) -> Result<serde_json::Value, String> {
    let config = config_manager.get();
    serde_json::to_value(&config.memory).map_err(|e| e.to_string())
}

/// Update memory configuration
#[tauri::command]
pub async fn update_memory_config(
    config_json: String,
    config_manager: State<'_, ConfigManager>,
    state: State<'_, Arc<lr_server::state::AppState>>,
) -> Result<(), String> {
    let new_config: lr_config::MemoryConfig =
        serde_json::from_str(&config_json).map_err(|e| format!("Invalid config JSON: {}", e))?;

    config_manager
        .update(|config| {
            config.memory = new_config.clone();
        })
        .map_err(|e| e.to_string())?;

    config_manager.save().await.map_err(|e| e.to_string())?;

    // Update the running memory service config
    if let Some(ref svc) = *state.memory_service.read() {
        svc.update_config(new_config);
    }

    Ok(())
}

/// Get memory service status
#[tauri::command]
pub async fn get_memory_status(
    state: State<'_, Arc<lr_server::state::AppState>>,
) -> Result<serde_json::Value, String> {
    // Clone the Arc out of the lock to avoid holding RwLockReadGuard across .await
    let svc = state.memory_service.read().clone();

    let (memsearch_installed, memsearch_version) = if let Some(ref svc) = svc {
        match svc.cli.check_installed().await {
            Ok(v) => (true, Some(v)),
            Err(_) => (false, None),
        }
    } else {
        (false, None)
    };

    let (python_ok, python_version) = if let Some(ref svc) = svc {
        match svc.cli.check_python().await {
            Ok(v) => (true, Some(v)),
            Err(_) => (false, None),
        }
    } else {
        (false, None)
    };

    Ok(serde_json::json!({
        "python_ok": python_ok,
        "python_version": python_version,
        "memsearch_installed": memsearch_installed,
        "memsearch_version": memsearch_version,
        "model_ready": memsearch_installed, // Approximate — model downloads on first use
    }))
}

/// Run memsearch setup steps (check python, install memsearch, download model)
#[tauri::command]
pub async fn memory_setup(
    app_handle: tauri::AppHandle,
    state: State<'_, Arc<lr_server::state::AppState>>,
) -> Result<(), String> {
    let svc = {
        let guard = state.memory_service.read();
        guard.clone().ok_or("Memory service not initialized")?
    };

    // Step 1: Check Python
    let _ = app_handle.emit("memory-setup-progress", serde_json::json!({
        "step": "python", "status": "checking"
    }));
    match svc.cli.check_python().await {
        Ok(version) => {
            let _ = app_handle.emit("memory-setup-progress", serde_json::json!({
                "step": "python", "status": "ok", "version": version
            }));
        }
        Err(e) => {
            let _ = app_handle.emit("memory-setup-progress", serde_json::json!({
                "step": "python", "status": "error", "error": e
            }));
            return Err(format!("Python not found: {}", e));
        }
    }

    // Step 2: Install/upgrade memsearch with ONNX support
    // Always run pip install to ensure the [onnx] extra is present,
    // even if memsearch is already installed without it.
    let _ = app_handle.emit("memory-setup-progress", serde_json::json!({
        "step": "memsearch", "status": "installing"
    }));
    let install_result = tokio::process::Command::new("pip3")
        .args(["install", "--upgrade", "memsearch[onnx]"])
        .output()
        .await
        .map_err(|e| format!("pip3 not found: {}", e))?;

    if !install_result.status.success() {
        let stderr = String::from_utf8_lossy(&install_result.stderr);
        let _ = app_handle.emit("memory-setup-progress", serde_json::json!({
            "step": "memsearch", "status": "error", "error": stderr.to_string()
        }));
        return Err(format!("Failed to install memsearch[onnx]: {}", stderr));
    }

    match svc.cli.check_installed().await {
        Ok(version) => {
            let _ = app_handle.emit("memory-setup-progress", serde_json::json!({
                "step": "memsearch", "status": "ok", "version": version
            }));
        }
        Err(e) => {
            let _ = app_handle.emit("memory-setup-progress", serde_json::json!({
                "step": "memsearch", "status": "error", "error": e
            }));
            return Err(e);
        }
    }

    // Step 3: Model download (warmup)
    let _ = app_handle.emit("memory-setup-progress", serde_json::json!({
        "step": "model", "status": "checking"
    }));

    // Pre-warm the ONNX model by running a dummy search
    let warmup_result = tokio::time::timeout(
        std::time::Duration::from_secs(300),
        tokio::process::Command::new("memsearch")
            .args(["search", "--provider", "onnx", "warmup"])
            .output(),
    )
    .await;

    match warmup_result {
        Ok(Ok(_)) => {
            let _ = app_handle.emit("memory-setup-progress", serde_json::json!({
                "step": "model", "status": "ok"
            }));
        }
        Ok(Err(e)) => {
            let _ = app_handle.emit("memory-setup-progress", serde_json::json!({
                "step": "model", "status": "error", "error": e.to_string()
            }));
            return Err(format!("Model download failed: {}", e));
        }
        Err(_) => {
            let _ = app_handle.emit("memory-setup-progress", serde_json::json!({
                "step": "model", "status": "error", "error": "Timed out (5 min)"
            }));
            return Err("Model download timed out".to_string());
        }
    }

    Ok(())
}

/// Get per-client memory configuration
#[tauri::command]
pub async fn get_client_memory_config(
    client_id: String,
    config_manager: State<'_, ConfigManager>,
) -> Result<serde_json::Value, String> {
    let config = config_manager.get();
    let client = config
        .clients
        .iter()
        .find(|c| c.id == client_id)
        .ok_or_else(|| format!("Client not found: {}", client_id))?;

    Ok(serde_json::json!({
        "memory_enabled": client.memory_enabled,
    }))
}

/// Update per-client memory enabled state
#[tauri::command]
pub async fn update_client_memory_config(
    client_id: String,
    enabled: bool,
    config_manager: State<'_, ConfigManager>,
) -> Result<(), String> {
    config_manager
        .update(|config| {
            if let Some(client) = config.clients.iter_mut().find(|c| c.id == client_id) {
                client.memory_enabled = Some(enabled);
            }
        })
        .map_err(|e| e.to_string())?;

    config_manager.save().await.map_err(|e| e.to_string())?;
    Ok(())
}

const MEMORY_TEST_DIR_NAME: &str = "localrouter-memory-test";

/// Get or create a temporary directory for memory Try It Out tests.
/// Uses the system temp dir (cross-platform: /tmp on macOS/Linux, %TEMP% on Windows).
/// No config file needed — all commands pass `--provider onnx` via CLI args.
fn memory_test_dir() -> Result<std::path::PathBuf, String> {
    let dir = std::env::temp_dir().join(MEMORY_TEST_DIR_NAME);
    std::fs::create_dir_all(dir.join("sessions"))
        .map_err(|e| format!("Failed to create test dir: {}", e))?;
    Ok(dir)
}

/// Create an ONNX CLI instance for test commands.
fn memory_test_cli() -> lr_memory::MemsearchCli {
    lr_memory::MemsearchCli::with_provider("onnx".to_string())
}

/// Reset the memory test directory (wipe all indexed content).
/// Called when the Try It Out tab is loaded.
#[tauri::command]
pub async fn memory_test_reset() -> Result<(), String> {
    let dir = std::env::temp_dir().join(MEMORY_TEST_DIR_NAME);
    if dir.exists() {
        std::fs::remove_dir_all(&dir)
            .map_err(|e| format!("Failed to clean test dir: {}", e))?;
    }
    Ok(())
}

/// Test: index content into memsearch for the Try It Out tab.
/// Uses a temp directory that is cleaned up on tab load.
#[tauri::command]
pub async fn memory_test_index(
    content: String,
) -> Result<(), String> {
    let cli = memory_test_cli();

    // Verify memsearch is installed before attempting
    if cli.check_installed().await.is_err() {
        return Err("memsearch is not installed. Run Setup on the Info tab first.".to_string());
    }

    let dir = memory_test_dir()?;
    let sessions_dir = dir.join("sessions");
    let test_file = sessions_dir.join("test-memory.md");

    tokio::fs::write(
        &test_file,
        format!(
            "---\nsession_id: test\nstarted: {}\n---\n\n## Memory\n{}\n\n",
            chrono::Utc::now().to_rfc3339(),
            content
        ),
    )
    .await
    .map_err(|e| format!("Failed to write test file: {}", e))?;

    cli.index(&sessions_dir).await?;

    Ok(())
}

/// Test: search memsearch for the Try It Out tab
#[tauri::command]
pub async fn memory_test_search(
    query: String,
    top_k: Option<usize>,
) -> Result<String, String> {
    let dir = memory_test_dir()?;
    let sessions_dir = dir.join("sessions");

    let results = memory_test_cli()
        .search(&sessions_dir, &query, top_k.unwrap_or(5))
        .await?;

    if results.is_empty() {
        return Ok("No results found.".to_string());
    }

    let mut output = format!("Found {} results:\n\n", results.len());
    for (i, r) in results.iter().enumerate() {
        let score = r
            .score
            .map(|s| format!(" [score: {:.2}]", s))
            .unwrap_or_default();
        output.push_str(&format!(
            "{}. {}{}\n   Source: {}\n\n",
            i + 1,
            r.content.trim(),
            score,
            r.source,
        ));
    }
    Ok(output)
}

/// Test: compact the test memory file
#[tauri::command]
pub async fn memory_test_compact(
    config_manager: State<'_, ConfigManager>,
) -> Result<String, String> {
    let config = config_manager.get();
    let compaction = config
        .memory
        .compaction
        .as_ref()
        .ok_or("Compaction not configured — select a model in Settings")?;

    if !compaction.enabled {
        return Err("Compaction is disabled".to_string());
    }

    let dir = memory_test_dir()?;
    let test_file = dir.join("sessions").join("test-memory.md");

    if !test_file.exists() {
        return Err("No test content to compact — index something first".to_string());
    }

    memory_test_cli()
        .compact(&dir, &test_file, &compaction.llm_provider)
        .await?;

    Ok("Compaction complete. Search again to see compacted results.".to_string())
}

/// Open the memory storage directory in the system file manager
#[tauri::command]
pub async fn open_memory_folder(
    state: State<'_, Arc<lr_server::state::AppState>>,
) -> Result<(), String> {
    let svc = {
        let guard = state.memory_service.read();
        guard.clone().ok_or("Memory service not initialized")?
    };
    let path = svc.memory_dir();

    // Create the directory if it doesn't exist
    std::fs::create_dir_all(path).map_err(|e| format!("Failed to create directory: {}", e))?;

    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(path)
            .spawn()
            .map_err(|e| format!("Failed to open folder: {}", e))?;
    }

    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(path)
            .spawn()
            .map_err(|e| format!("Failed to open folder: {}", e))?;
    }

    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg(path)
            .spawn()
            .map_err(|e| format!("Failed to open folder: {}", e))?;
    }

    Ok(())
}
