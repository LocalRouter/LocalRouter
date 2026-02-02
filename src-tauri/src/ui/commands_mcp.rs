//! MCP-related Tauri command handlers
//!
//! MCP server management, health checks, OAuth browser flows, and inline OAuth flows.

use std::sync::Arc;

use lr_api_keys::keychain_trait::KeychainStorage;
use lr_config::{ConfigManager, McpAuthConfig, McpServerConfig, McpTransportConfig, McpTransportType};
use lr_mcp::McpServerManager;
use lr_server::ServerManager;
use serde::{Deserialize, Serialize};
use tauri::{Emitter, State};

// ============================================================================
// MCP Server Commands
// ============================================================================

/// Frontend auth config format (with raw secrets)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FrontendAuthConfig {
    None,
    BearerToken {
        token: String,
    },
    CustomHeaders {
        headers: std::collections::HashMap<String, String>,
    },
    OAuth {
        client_id: String,
        client_secret: String,
        auth_url: String,
        token_url: String,
        scopes: Vec<String>,
    },
    EnvVars {
        env: std::collections::HashMap<String, String>,
    },
    /// OAuth with browser-based authorization code flow (PKCE)
    /// Initially a placeholder - full OAuth details configured during first auth
    #[serde(rename = "oauth_browser")]
    OAuthBrowser {
        /// OAuth client ID (optional - can be configured later)
        #[serde(default)]
        client_id: Option<String>,
        /// Client secret (optional - can be configured later)
        #[serde(default)]
        client_secret: Option<String>,
        /// Authorization endpoint URL (optional - can be auto-discovered)
        #[serde(default)]
        auth_url: Option<String>,
        /// Token endpoint URL (optional - can be auto-discovered)
        #[serde(default)]
        token_url: Option<String>,
        /// OAuth scopes to request
        #[serde(default)]
        scopes: Vec<String>,
        /// Redirect URI (defaults to http://localhost:8080/callback)
        #[serde(default)]
        redirect_uri: Option<String>,
    },
}

/// Process frontend auth config and store secrets in keychain
/// Returns the backend McpAuthConfig with keychain references
fn process_auth_config(
    server_id: &str,
    auth_cfg: Option<serde_json::Value>,
) -> Result<Option<McpAuthConfig>, String> {
    let Some(auth_value) = auth_cfg else {
        tracing::debug!("No auth config provided to process_auth_config");
        return Ok(None);
    };

    tracing::debug!("Processing auth config: {}", auth_value);

    // Parse frontend format
    let frontend_auth: FrontendAuthConfig =
        serde_json::from_value(auth_value.clone()).map_err(|e| {
            tracing::error!("Failed to deserialize frontend auth config: {}", e);
            tracing::error!("Auth value was: {}", auth_value);
            format!("Invalid auth config format: {}", e)
        })?;

    tracing::debug!("Parsed frontend auth: {:?}", frontend_auth);

    // Convert to backend format, storing secrets in keychain
    let backend_auth = match frontend_auth {
        FrontendAuthConfig::None => return Ok(None),
        FrontendAuthConfig::BearerToken { token } => {
            // Store token in keychain
            let keychain = lr_api_keys::CachedKeychain::auto()
                .map_err(|e| format!("Failed to access keychain: {}", e))?;

            let key = format!("{}_bearer_token", server_id);
            keychain
                .store("LocalRouter-McpServers", &key, &token)
                .map_err(|e| format!("Failed to store token in keychain: {}", e))?;

            tracing::debug!("Stored bearer token in keychain with key: {}", key);

            McpAuthConfig::BearerToken { token_ref: key }
        }
        FrontendAuthConfig::CustomHeaders { headers } => McpAuthConfig::CustomHeaders { headers },
        FrontendAuthConfig::OAuth {
            client_id,
            client_secret,
            auth_url,
            token_url,
            scopes,
        } => {
            // Store client secret in keychain
            let keychain = lr_api_keys::CachedKeychain::auto()
                .map_err(|e| format!("Failed to access keychain: {}", e))?;

            let key = format!("{}_oauth_secret", server_id);
            keychain
                .store("LocalRouter-McpServers", &key, &client_secret)
                .map_err(|e| format!("Failed to store OAuth secret in keychain: {}", e))?;

            tracing::debug!("Stored OAuth secret in keychain with key: {}", key);

            McpAuthConfig::OAuth {
                client_id,
                client_secret_ref: key,
                auth_url,
                token_url,
                scopes,
            }
        }
        FrontendAuthConfig::EnvVars { env } => McpAuthConfig::EnvVars { env },
        FrontendAuthConfig::OAuthBrowser {
            client_id,
            client_secret,
            auth_url,
            token_url,
            scopes,
            redirect_uri,
        } => {
            // Store client secret in keychain if provided
            let secret_ref = if let Some(secret) = client_secret {
                let keychain = lr_api_keys::CachedKeychain::auto()
                    .map_err(|e| format!("Failed to access keychain: {}", e))?;

                let key = format!("{}_oauth_browser_secret", server_id);
                keychain
                    .store("LocalRouter-McpServers", &key, &secret)
                    .map_err(|e| format!("Failed to store OAuth secret in keychain: {}", e))?;

                tracing::debug!("Stored OAuth browser secret in keychain with key: {}", key);
                key
            } else {
                // No secret provided yet - use placeholder
                format!("{}_oauth_browser_secret", server_id)
            };

            McpAuthConfig::OAuthBrowser {
                client_id: client_id.unwrap_or_default(),
                client_secret_ref: secret_ref,
                auth_url: auth_url.unwrap_or_default(),
                token_url: token_url.unwrap_or_default(),
                scopes,
                redirect_uri: redirect_uri
                    .unwrap_or_else(|| "http://localhost:8080/callback".to_string()),
            }
        }
    };

    tracing::info!(
        "✅ Successfully processed auth config for server {}: {:?}",
        server_id,
        backend_auth
    );

    Ok(Some(backend_auth))
}

/// MCP server information for display
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerInfo {
    pub id: String,
    pub name: String,
    pub transport: String,
    pub transport_config: McpTransportConfig,
    pub auth_config: Option<McpAuthConfig>,
    pub enabled: bool,
    pub running: bool,
    pub created_at: String,
    /// The individual proxy endpoint URL for this server (e.g., http://localhost:3625/mcp/{server_id})
    pub proxy_url: String,
    /// The unified MCP gateway URL (always available at http://localhost:3625/)
    pub gateway_url: String,
    /// Legacy field for backward compatibility (deprecated, use proxy_url instead)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

/// List all MCP servers
#[tauri::command]
pub async fn list_mcp_servers(
    mcp_manager: State<'_, Arc<McpServerManager>>,
    server_manager: State<'_, Arc<ServerManager>>,
) -> Result<Vec<McpServerInfo>, String> {
    let configs = mcp_manager.list_configs();
    let mut servers = Vec::new();

    // Get the actual server port
    let port = server_manager.get_actual_port().unwrap_or(3625);
    let base_url = format!("http://localhost:{}", port);

    for config in configs {
        // All servers get a proxy URL at /mcp/{server_id}
        let proxy_url = format!("{}/mcp/{}", base_url, config.id);

        // Unified gateway URL is always at root
        let gateway_url = base_url.clone();

        // Legacy URL field for backward compatibility (deprecated)
        let url = Some(proxy_url.clone());

        servers.push(McpServerInfo {
            id: config.id.clone(),
            name: config.name.clone(),
            transport: format!("{:?}", config.transport),
            transport_config: config.transport_config.clone(),
            auth_config: config.auth_config.clone(),
            enabled: config.enabled,
            running: mcp_manager.is_running(&config.id),
            created_at: config.created_at.to_rfc3339(),
            proxy_url,
            gateway_url,
            url,
        });
    }

    Ok(servers)
}

/// Create a new MCP server
///
/// # Arguments
/// * `name` - Server name
/// * `transport` - Transport type ("stdio", "sse", "websocket")
/// * `transport_config` - Transport-specific configuration as JSON
///
/// # Returns
/// * The created server info
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn create_mcp_server(
    name: String,
    transport: String,
    transport_config: serde_json::Value,
    auth_config: Option<serde_json::Value>,
    mcp_manager: State<'_, Arc<McpServerManager>>,
    server_manager: State<'_, Arc<ServerManager>>,
    config_manager: State<'_, ConfigManager>,
    app: tauri::AppHandle,
) -> Result<McpServerInfo, String> {
    tracing::info!("Creating new MCP server: {} ({})", name, transport);

    // Parse transport type (case-insensitive)
    #[allow(deprecated)]
    let transport_type = match transport.to_lowercase().as_str() {
        "stdio" => McpTransportType::Stdio,
        "sse" | "httpsse" | "http_sse" => McpTransportType::HttpSse,
        _ => return Err(format!("Invalid transport type: {}", transport)),
    };

    // Parse transport config
    let parsed_config: McpTransportConfig = serde_json::from_value(transport_config)
        .map_err(|e| format!("Invalid transport config: {}", e))?;

    // Create server config (need ID for auth processing)
    let mut config = McpServerConfig::new(name, transport_type, parsed_config);

    // Parse auth config (if provided) and store secrets in keychain
    config.auth_config = process_auth_config(&config.id, auth_config)?;

    // Get the actual server port
    let port = server_manager.get_actual_port().unwrap_or(3625);
    let base_url = format!("http://localhost:{}", port);

    // All servers get a proxy URL at /mcp/{server_id}
    let proxy_url = format!("{}/mcp/{}", base_url, config.id);

    // Unified gateway URL is always at root
    let gateway_url = base_url.clone();

    // Legacy URL field for backward compatibility (deprecated)
    let url = Some(proxy_url.clone());

    let server_info = McpServerInfo {
        id: config.id.clone(),
        name: config.name.clone(),
        transport: format!("{:?}", config.transport),
        transport_config: config.transport_config.clone(),
        auth_config: config.auth_config.clone(),
        enabled: config.enabled,
        running: false,
        created_at: config.created_at.to_rfc3339(),
        proxy_url,
        gateway_url,
        url,
    };

    // Add to manager
    mcp_manager.add_config(config.clone());

    // Save to config file
    config_manager
        .update(|cfg| {
            cfg.mcp_servers.push(config);
        })
        .map_err(|e| e.to_string())?;

    // Persist to disk
    config_manager.save().await.map_err(|e| e.to_string())?;

    // Rebuild tray menu
    if let Err(e) = crate::ui::tray::rebuild_tray_menu(&app) {
        tracing::error!("Failed to rebuild tray menu: {}", e);
    }

    // Notify frontend
    let _ = app.emit("mcp-servers-changed", ());

    Ok(server_info)
}

/// Delete an MCP server
///
/// # Arguments
/// * `server_id` - The server ID to delete
///
/// # Returns
/// * Ok(()) if successful
#[tauri::command]
pub async fn delete_mcp_server(
    server_id: String,
    mcp_manager: State<'_, Arc<McpServerManager>>,
    config_manager: State<'_, ConfigManager>,
    app_state: State<'_, Arc<lr_server::state::AppState>>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    tracing::info!("Deleting MCP server: {}", server_id);

    // Stop if running
    if mcp_manager.is_running(&server_id) {
        mcp_manager
            .stop_server(&server_id)
            .await
            .map_err(|e| e.to_string())?;
    }

    // Remove from manager
    mcp_manager.remove_config(&server_id);

    // Remove from config file
    config_manager
        .update(|cfg| {
            cfg.mcp_servers.retain(|s| s.id != server_id);
        })
        .map_err(|e| e.to_string())?;

    // Persist to disk
    config_manager.save().await.map_err(|e| e.to_string())?;

    // Remove from health cache (this emits health-status-changed event)
    app_state.health_cache.remove_mcp_server(&server_id);

    // Rebuild tray menu
    if let Err(e) = crate::ui::tray::rebuild_tray_menu(&app) {
        tracing::error!("Failed to rebuild tray menu: {}", e);
    }

    // Notify frontend
    let _ = app.emit("mcp-servers-changed", ());

    Ok(())
}

/// Start an MCP server
///
/// # Arguments
/// * `server_id` - The server ID to start
///
/// # Returns
/// * Ok(()) if successful
#[tauri::command]
pub async fn start_mcp_server(
    server_id: String,
    mcp_manager: State<'_, Arc<McpServerManager>>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    tracing::info!("Starting MCP server: {}", server_id);

    mcp_manager
        .start_server(&server_id)
        .await
        .map_err(|e| e.to_string())?;

    // Notify frontend
    let _ = app.emit("mcp-servers-changed", ());

    Ok(())
}

/// Stop an MCP server
///
/// # Arguments
/// * `server_id` - The server ID to stop
///
/// # Returns
/// * Ok(()) if successful
#[tauri::command]
pub async fn stop_mcp_server(
    server_id: String,
    mcp_manager: State<'_, Arc<McpServerManager>>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    tracing::info!("Stopping MCP server: {}", server_id);

    mcp_manager
        .stop_server(&server_id)
        .await
        .map_err(|e| e.to_string())?;

    // Notify frontend
    let _ = app.emit("mcp-servers-changed", ());

    Ok(())
}

/// Get health status for an MCP server
///
/// # Arguments
/// * `server_id` - The server ID to check
///
/// # Returns
/// * The health status
#[tauri::command]
pub async fn get_mcp_server_health(
    server_id: String,
    mcp_manager: State<'_, Arc<McpServerManager>>,
) -> Result<lr_mcp::manager::McpServerHealth, String> {
    Ok(mcp_manager.get_server_health(&server_id).await)
}

/// Get health status for all MCP servers
///
/// # Returns
/// * List of health statuses for all servers
#[tauri::command]
pub async fn get_all_mcp_server_health(
    mcp_manager: State<'_, Arc<McpServerManager>>,
) -> Result<Vec<lr_mcp::manager::McpServerHealth>, String> {
    Ok(mcp_manager.get_all_health().await)
}

/// Health check result for streaming MCP health to frontend
#[derive(Clone, Serialize)]
pub struct McpHealthCheckResult {
    pub server_id: String,
    pub server_name: String,
    pub status: String,
    pub latency_ms: Option<u64>,
    pub error: Option<String>,
}

/// Start streaming health checks for all MCP servers
///
/// Emits "mcp-health-check" events as each server's health check completes.
/// Returns immediately with the list of server IDs being checked.
#[tauri::command]
pub async fn start_mcp_health_checks(
    app: tauri::AppHandle,
    mcp_manager: State<'_, Arc<McpServerManager>>,
) -> Result<Vec<String>, String> {
    let server_ids: Vec<String> = mcp_manager
        .list_configs()
        .iter()
        .map(|c| c.id.clone())
        .collect();

    let mcp_manager = mcp_manager.inner().clone();
    let app_handle = app.clone();

    // Spawn health checks for each server in parallel
    tokio::spawn(async move {
        let configs = mcp_manager.list_configs();
        let mut handles = Vec::new();

        for config in configs {
            let mcp_manager = mcp_manager.clone();
            let app_handle = app_handle.clone();
            let server_id = config.id.clone();
            let server_name = config.name.clone();
            let enabled = config.enabled;

            let handle = tokio::spawn(async move {
                // If server is disabled, emit disabled status without running health check
                if !enabled {
                    let result = McpHealthCheckResult {
                        server_id,
                        server_name,
                        status: "disabled".to_string(),
                        latency_ms: None,
                        error: None,
                    };
                    let _ = app_handle.emit("mcp-health-check", result);
                    return;
                }

                let health = mcp_manager.get_server_health(&server_id).await;
                let result = McpHealthCheckResult {
                    server_id: health.server_id,
                    server_name: health.server_name,
                    status: match health.status {
                        lr_mcp::manager::HealthStatus::Healthy => "healthy".to_string(),
                        lr_mcp::manager::HealthStatus::Ready => "ready".to_string(),
                        lr_mcp::manager::HealthStatus::Unhealthy => "unhealthy".to_string(),
                        lr_mcp::manager::HealthStatus::Unknown => "unknown".to_string(),
                    },
                    latency_ms: health.latency_ms,
                    error: health.error,
                };
                let _ = app_handle.emit("mcp-health-check", result);
            });
            handles.push(handle);
        }

        // Wait for all health checks to complete
        for handle in handles {
            let _ = handle.await;
        }
    });

    Ok(server_ids)
}

/// Check health for a single MCP server
///
/// Emits "mcp-health-check" event when the health check completes.
#[tauri::command]
pub async fn check_single_mcp_health(
    app: tauri::AppHandle,
    mcp_manager: State<'_, Arc<McpServerManager>>,
    app_state: State<'_, Arc<lr_server::state::AppState>>,
    server_id: String,
) -> Result<(), String> {
    let mcp_manager = mcp_manager.inner().clone();
    let health_cache = app_state.health_cache.clone();
    let app_handle = app.clone();

    // Check if server is disabled
    let config = mcp_manager
        .get_config(&server_id)
        .ok_or_else(|| format!("Server not found: {}", server_id))?;

    if !config.enabled {
        let result = McpHealthCheckResult {
            server_id: config.id.clone(),
            server_name: config.name.clone(),
            status: "disabled".to_string(),
            latency_ms: None,
            error: None,
        };
        let _ = app_handle.emit("mcp-health-check", result);
        health_cache.update_mcp_server(
            &config.id,
            lr_providers::health_cache::ItemHealth::disabled(config.name),
        );
        return Ok(());
    }

    tokio::spawn(async move {
        let health = mcp_manager.get_server_health(&server_id).await;
        let result = McpHealthCheckResult {
            server_id: health.server_id.clone(),
            server_name: health.server_name.clone(),
            status: match health.status {
                lr_mcp::manager::HealthStatus::Healthy => "healthy".to_string(),
                lr_mcp::manager::HealthStatus::Ready => "ready".to_string(),
                lr_mcp::manager::HealthStatus::Unhealthy => "unhealthy".to_string(),
                lr_mcp::manager::HealthStatus::Unknown => "unknown".to_string(),
            },
            latency_ms: health.latency_ms,
            error: health.error.clone(),
        };
        let _ = app_handle.emit("mcp-health-check", result);

        // Update centralized health cache so aggregate status (tray + sidebar) recalculates
        use lr_providers::health_cache::ItemHealth;
        let item_health = match health.status {
            lr_mcp::manager::HealthStatus::Healthy => {
                ItemHealth::healthy(health.server_name, health.latency_ms)
            }
            lr_mcp::manager::HealthStatus::Ready => ItemHealth::ready(health.server_name),
            lr_mcp::manager::HealthStatus::Unhealthy => ItemHealth::unhealthy(
                health.server_name,
                health.error.unwrap_or_else(|| "Unhealthy".to_string()),
            ),
            lr_mcp::manager::HealthStatus::Unknown => ItemHealth::unhealthy(
                health.server_name,
                health.error.unwrap_or_else(|| "Unknown status".to_string()),
            ),
        };
        health_cache.update_mcp_server(&health.server_id, item_health);
    });

    Ok(())
}

/// Update an MCP server's name
///
/// # Arguments
/// * `server_id` - The server ID to update
/// * `name` - The new name
///
/// # Returns
/// * Ok(()) if successful
#[tauri::command]
pub async fn update_mcp_server_name(
    server_id: String,
    name: String,
    config_manager: State<'_, ConfigManager>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    // Validate name is not empty
    if name.trim().is_empty() {
        return Err("MCP server name cannot be empty".to_string());
    }

    // Update in config file
    config_manager
        .update(|cfg| {
            if let Some(server) = cfg.mcp_servers.iter_mut().find(|s| s.id == server_id) {
                server.name = name.clone();
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
    let _ = app.emit("mcp-servers-changed", ());

    Ok(())
}

/// Update an MCP server's configuration
///
/// # Arguments
/// * `server_id` - The server ID to update
/// * `name` - Updated server name
/// * `transport_config` - Updated transport configuration
/// * `auth_config` - Updated authentication configuration (optional)
///
/// # Returns
/// * Ok(()) if successful
#[tauri::command]
pub async fn update_mcp_server_config(
    server_id: String,
    name: String,
    transport_config: serde_json::Value,
    auth_config: Option<serde_json::Value>,
    mcp_manager: State<'_, Arc<McpServerManager>>,
    config_manager: State<'_, ConfigManager>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    tracing::info!("Updating MCP server config: {}", server_id);
    tracing::debug!("Transport config JSON: {}", transport_config);
    tracing::debug!("Auth config JSON: {:?}", auth_config);

    // Validate name is not empty
    if name.trim().is_empty() {
        tracing::warn!("Attempted to update MCP server with empty name");
        return Err("MCP server name cannot be empty".to_string());
    }

    // Parse transport config
    let parsed_config: McpTransportConfig = serde_json::from_value(transport_config.clone())
        .map_err(|e| {
            tracing::error!("Failed to parse transport config: {}", e);
            format!("Invalid transport config: {}", e)
        })?;

    tracing::info!("Parsed transport config: {:?}", parsed_config);

    // Parse auth config (if provided) and store secrets in keychain
    // If auth_config is None, we preserve the existing auth config
    let should_update_auth = auth_config.is_some();
    let parsed_auth_config = if should_update_auth {
        process_auth_config(&server_id, auth_config)?
    } else {
        None
    };

    if should_update_auth {
        if let Some(ref auth) = parsed_auth_config {
            tracing::info!("Updating auth config: {:?}", auth);
        } else {
            tracing::info!("Clearing auth config (none provided)");
        }
    } else {
        tracing::info!("Preserving existing auth config (not provided in update)");
    }

    // Update in config file
    config_manager
        .update(|cfg| {
            if let Some(server) = cfg.mcp_servers.iter_mut().find(|s| s.id == server_id) {
                server.name = name.clone();
                server.transport_config = parsed_config.clone();
                // Only update auth if it was provided in the request
                if should_update_auth {
                    server.auth_config = parsed_auth_config.clone();
                }
                // Otherwise keep existing auth_config
            }
        })
        .map_err(|e| e.to_string())?;

    // Update in manager
    if let Some(config) = config_manager
        .get()
        .mcp_servers
        .iter()
        .find(|s| s.id == server_id)
        .cloned()
    {
        mcp_manager.add_config(config);
    }

    // Persist to disk
    config_manager.save().await.map_err(|e| e.to_string())?;

    // Rebuild tray menu
    if let Err(e) = crate::ui::tray::rebuild_tray_menu(&app) {
        tracing::error!("Failed to rebuild tray menu: {}", e);
    }

    // Notify frontend
    let _ = app.emit("mcp-servers-changed", ());

    Ok(())
}

/// Update an MCP server with partial updates
///
/// This command allows updating individual fields without requiring all fields.
/// Only the fields provided in the `updates` object will be modified.
///
/// # Arguments
/// * `server_id` - The server ID to update
/// * `updates` - JSON object with optional fields: name, transport_config, auth_config
///
/// # Returns
/// * Ok(()) if successful
#[tauri::command]
pub async fn update_mcp_server(
    server_id: String,
    updates: serde_json::Value,
    mcp_manager: State<'_, Arc<McpServerManager>>,
    config_manager: State<'_, ConfigManager>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    tracing::info!("Updating MCP server with partial updates: {}", server_id);
    tracing::debug!("Updates JSON: {}", updates);

    let updates_obj = updates.as_object().ok_or("Updates must be a JSON object")?;

    // Extract optional update fields
    let name_update = updates_obj.get("name").and_then(|v| v.as_str());
    let transport_config_update = updates_obj.get("transport_config");
    let auth_config_update = updates_obj.get("auth_config");

    // Validate name if provided
    if let Some(name) = name_update {
        if name.trim().is_empty() {
            return Err("MCP server name cannot be empty".to_string());
        }
    }

    // Parse transport config if provided
    let parsed_transport_config = if let Some(tc) = transport_config_update {
        Some(
            serde_json::from_value::<McpTransportConfig>(tc.clone()).map_err(|e| {
                tracing::error!("Failed to parse transport config: {}", e);
                format!("Invalid transport config: {}", e)
            })?,
        )
    } else {
        None
    };

    // Parse auth config if provided and store secrets in keychain
    let parsed_auth_config = if auth_config_update.is_some() {
        process_auth_config(&server_id, auth_config_update.cloned())?
    } else {
        None
    };

    // Update in config file
    config_manager
        .update(|cfg| {
            if let Some(server) = cfg.mcp_servers.iter_mut().find(|s| s.id == server_id) {
                // Update name if provided
                if let Some(name) = name_update {
                    server.name = name.to_string();
                }
                // Update transport config if provided
                if let Some(ref tc) = parsed_transport_config {
                    server.transport_config = tc.clone();
                }
                // Update auth config if provided (even if it's None to clear it)
                if auth_config_update.is_some() {
                    server.auth_config = parsed_auth_config.clone();
                }
            }
        })
        .map_err(|e| e.to_string())?;

    // Update in manager
    if let Some(config) = config_manager
        .get()
        .mcp_servers
        .iter()
        .find(|s| s.id == server_id)
        .cloned()
    {
        mcp_manager.add_config(config);
    }

    // Persist to disk
    config_manager.save().await.map_err(|e| e.to_string())?;

    // Rebuild tray menu
    if let Err(e) = crate::ui::tray::rebuild_tray_menu(&app) {
        tracing::error!("Failed to rebuild tray menu: {}", e);
    }

    // Notify frontend
    let _ = app.emit("mcp-servers-changed", ());

    Ok(())
}

/// Toggle an MCP server's enabled state
///
/// # Arguments
/// * `server_id` - The server ID to toggle
/// * `enabled` - Whether to enable (true) or disable (false)
///
/// # Returns
/// * Ok(()) if successful
#[tauri::command]
pub async fn toggle_mcp_server_enabled(
    server_id: String,
    enabled: bool,
    mcp_manager: State<'_, Arc<McpServerManager>>,
    config_manager: State<'_, ConfigManager>,
    app_state: State<'_, Arc<lr_server::state::AppState>>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    // Get server name before updating (needed for health cache)
    let server_name = mcp_manager
        .list_configs()
        .iter()
        .find(|c| c.id == server_id)
        .map(|c| c.name.clone())
        .unwrap_or_else(|| server_id.clone());

    // Update in MCP manager (in-memory)
    mcp_manager.set_config_enabled(&server_id, enabled);

    // Update in config file
    config_manager
        .update(|cfg| {
            if let Some(server) = cfg.mcp_servers.iter_mut().find(|s| s.id == server_id) {
                server.enabled = enabled;
            }
        })
        .map_err(|e| e.to_string())?;

    // Persist to disk
    config_manager.save().await.map_err(|e| e.to_string())?;

    // Update health cache immediately - this recalculates aggregate status and emits event
    use lr_providers::health_cache::ItemHealth;
    if enabled {
        // Set to pending, then trigger a health check for this MCP server
        app_state
            .health_cache
            .update_mcp_server(&server_id, ItemHealth::pending(server_name.clone()));

        // Spawn background health check for this MCP server
        let health_cache = app_state.health_cache.clone();
        let mcp_manager = app_state.mcp_server_manager.clone();
        let id = server_id.clone();
        tokio::spawn(async move {
            let mcp_server_health = mcp_manager.get_server_health(&id).await;
            use lr_mcp::manager::HealthStatus as McpHealthStatus;
            let server_name = mcp_server_health.server_name.clone();
            let item_health = match mcp_server_health.status {
                McpHealthStatus::Ready => ItemHealth::ready(server_name),
                McpHealthStatus::Healthy => {
                    ItemHealth::healthy(server_name, mcp_server_health.latency_ms)
                }
                McpHealthStatus::Unhealthy | McpHealthStatus::Unknown => ItemHealth::unhealthy(
                    server_name,
                    mcp_server_health
                        .error
                        .unwrap_or_else(|| "Unhealthy".to_string()),
                ),
            };
            health_cache.update_mcp_server(&id, item_health);
        });
    } else {
        // Set to disabled immediately
        app_state
            .health_cache
            .update_mcp_server(&server_id, ItemHealth::disabled(server_name));
    }

    // Rebuild tray menu
    if let Err(e) = crate::ui::tray::rebuild_tray_menu(&app) {
        tracing::error!("Failed to rebuild tray menu: {}", e);
    }

    // Notify frontend
    let _ = app.emit("mcp-servers-changed", ());

    Ok(())
}

/// List available tools from an MCP server
///
/// # Arguments
/// * `server_id` - The MCP server ID
///
/// # Returns
/// * List of available tools with their schemas
#[tauri::command]
pub async fn list_mcp_tools(
    server_id: String,
    mcp_manager: State<'_, Arc<McpServerManager>>,
    server_manager: State<'_, Arc<lr_server::manager::ServerManager>>,
) -> Result<serde_json::Value, String> {
    use lr_mcp::protocol::JsonRpcRequest;
    use std::time::Instant;

    tracing::info!("📋 Listing tools for MCP server: {}", server_id);
    let start_time = Instant::now();
    let method = "tools/list";
    let client_id = "ui"; // UI requests use a special client_id

    // Get app state for logging (if server is running)
    let app_state = server_manager.get_state();

    // Start server if not running (auto-start on demand)
    if !mcp_manager.is_running(&server_id) {
        tracing::info!("MCP server {} not running, starting it now...", server_id);
        mcp_manager
            .start_server(&server_id)
            .await
            .map_err(|e| format!("Failed to start MCP server: {}", e))?;
        tracing::info!("✅ MCP server {} started successfully", server_id);
    }

    // Create a tools/list request
    let request = JsonRpcRequest::with_id(1, method.to_string(), None);

    tracing::debug!("🔄 Sending tools/list request to server {}", server_id);

    // Send request to MCP server
    let response = mcp_manager
        .send_request(&server_id, request)
        .await
        .map_err(|e| {
            let latency_ms = start_time.elapsed().as_millis() as u64;
            tracing::error!(
                "❌ Failed to send tools/list request to server {}: {}",
                server_id,
                e
            );

            // Log failure (if server is running)
            if let Some(ref state) = app_state {
                let request_id = format!("mcp_ui_{}", uuid::Uuid::new_v4());
                let _ = state.mcp_access_logger.log_failure(
                    client_id,
                    &server_id,
                    method,
                    500,
                    None,
                    latency_ms,
                    "unknown",
                    &request_id,
                );

                // Record metrics
                state.metrics_collector.mcp().record(
                    &lr_monitoring::mcp_metrics::McpRequestMetrics {
                        client_id,
                        server_id: &server_id,
                        method,
                        latency_ms,
                        success: false,
                        error_code: None,
                    },
                );
            }

            format!("Failed to list tools: {}", e)
        })?;

    let latency_ms = start_time.elapsed().as_millis() as u64;

    // Check for error
    if let Some(error) = response.error {
        tracing::error!(
            "❌ MCP server {} returned error for tools/list: {} (code {})",
            server_id,
            error.message,
            error.code
        );

        // Log failure (if server is running)
        if let Some(ref state) = app_state {
            let request_id = format!("mcp_ui_{}", uuid::Uuid::new_v4());
            let _ = state.mcp_access_logger.log_failure(
                client_id,
                &server_id,
                method,
                500,
                Some(error.code),
                latency_ms,
                "unknown",
                &request_id,
            );

            // Record metrics
            state
                .metrics_collector
                .mcp()
                .record(&lr_monitoring::mcp_metrics::McpRequestMetrics {
                    client_id,
                    server_id: &server_id,
                    method,
                    latency_ms,
                    success: false,
                    error_code: Some(error.code),
                });
        }

        return Err(format!(
            "MCP error: {} (code {})",
            error.message, error.code
        ));
    }

    // Log success (if server is running)
    if let Some(ref state) = app_state {
        let request_id = format!("mcp_ui_{}", uuid::Uuid::new_v4());
        let _ = state.mcp_access_logger.log_success(
            client_id,
            &server_id,
            method,
            latency_ms,
            "unknown",
            &request_id,
        );

        // Record metrics
        state
            .metrics_collector
            .mcp()
            .record(&lr_monitoring::mcp_metrics::McpRequestMetrics {
                client_id,
                server_id: &server_id,
                method,
                latency_ms,
                success: true,
                error_code: None,
            });
    }

    // Return the tools list
    let result = response.result.unwrap_or(serde_json::Value::Null);

    // Log the number of tools if result is an object with "tools" array
    if let Some(obj) = result.as_object() {
        if let Some(tools) = obj.get("tools").and_then(|t| t.as_array()) {
            tracing::info!(
                "✅ Successfully listed {} tools from MCP server {}",
                tools.len(),
                server_id
            );
            for tool in tools {
                if let Some(tool_obj) = tool.as_object() {
                    if let Some(name) = tool_obj.get("name").and_then(|n| n.as_str()) {
                        tracing::debug!("  - Tool: {}", name);
                    }
                }
            }
        }
    }

    tracing::debug!("Tools list response: {}", result);

    Ok(result)
}

/// Call an MCP tool
///
/// # Arguments
/// * `server_id` - The MCP server ID
/// * `tool_name` - The tool name to call
/// * `arguments` - Tool arguments as JSON
///
/// # Returns
/// * The tool execution result
#[tauri::command]
pub async fn call_mcp_tool(
    server_id: String,
    tool_name: String,
    arguments: serde_json::Value,
    mcp_manager: State<'_, Arc<McpServerManager>>,
    server_manager: State<'_, Arc<lr_server::manager::ServerManager>>,
) -> Result<serde_json::Value, String> {
    use lr_mcp::protocol::JsonRpcRequest;
    use std::time::Instant;

    tracing::info!(
        "🔧 Calling MCP tool '{}' on server: {}",
        tool_name,
        server_id
    );
    tracing::debug!("Tool arguments: {}", arguments);

    let start_time = Instant::now();
    let method = format!("tools/call:{}", tool_name);
    let client_id = "ui"; // UI requests use a special client_id

    // Get app state for logging (if server is running)
    let app_state = server_manager.get_state();

    // Start server if not running (auto-start on demand)
    if !mcp_manager.is_running(&server_id) {
        tracing::info!("MCP server {} not running, starting it now...", server_id);
        mcp_manager
            .start_server(&server_id)
            .await
            .map_err(|e| format!("Failed to start MCP server: {}", e))?;
        tracing::info!("✅ MCP server {} started successfully", server_id);
    }

    // Create a tools/call request
    let params = serde_json::json!({
        "name": tool_name,
        "arguments": arguments
    });

    let request = JsonRpcRequest::with_id(1, "tools/call".to_string(), Some(params));

    tracing::debug!(
        "🔄 Sending tools/call request for '{}' to server {}",
        tool_name,
        server_id
    );

    // Send request to MCP server
    let response = mcp_manager
        .send_request(&server_id, request)
        .await
        .map_err(|e| {
            let latency_ms = start_time.elapsed().as_millis() as u64;
            tracing::error!(
                "❌ Failed to call tool '{}' on server {}: {}",
                tool_name,
                server_id,
                e
            );

            // Log failure (if server is running)
            if let Some(ref state) = app_state {
                let request_id = format!("mcp_ui_{}", uuid::Uuid::new_v4());
                let _ = state.mcp_access_logger.log_failure(
                    client_id,
                    &server_id,
                    &method,
                    500,
                    None,
                    latency_ms,
                    "unknown",
                    &request_id,
                );

                // Record metrics
                state.metrics_collector.mcp().record(
                    &lr_monitoring::mcp_metrics::McpRequestMetrics {
                        client_id,
                        server_id: &server_id,
                        method: &method,
                        latency_ms,
                        success: false,
                        error_code: None,
                    },
                );
            }

            format!("Failed to call tool: {}", e)
        })?;

    let latency_ms = start_time.elapsed().as_millis() as u64;

    // Check for error
    if let Some(error) = response.error {
        tracing::error!(
            "❌ MCP server {} returned error for tool '{}': {} (code {})",
            server_id,
            tool_name,
            error.message,
            error.code
        );

        // Log failure (if server is running)
        if let Some(ref state) = app_state {
            let request_id = format!("mcp_ui_{}", uuid::Uuid::new_v4());
            let _ = state.mcp_access_logger.log_failure(
                client_id,
                &server_id,
                &method,
                500,
                Some(error.code),
                latency_ms,
                "unknown",
                &request_id,
            );

            // Record metrics
            state
                .metrics_collector
                .mcp()
                .record(&lr_monitoring::mcp_metrics::McpRequestMetrics {
                    client_id,
                    server_id: &server_id,
                    method: &method,
                    latency_ms,
                    success: false,
                    error_code: Some(error.code),
                });
        }

        return Err(format!(
            "MCP error: {} (code {})",
            error.message, error.code
        ));
    }

    // Log success (if server is running)
    if let Some(ref state) = app_state {
        let request_id = format!("mcp_ui_{}", uuid::Uuid::new_v4());
        let _ = state.mcp_access_logger.log_success(
            client_id,
            &server_id,
            &method,
            latency_ms,
            "unknown",
            &request_id,
        );

        // Record metrics
        state
            .metrics_collector
            .mcp()
            .record(&lr_monitoring::mcp_metrics::McpRequestMetrics {
                client_id,
                server_id: &server_id,
                method: &method,
                latency_ms,
                success: true,
                error_code: None,
            });
    }

    // Return the result
    let result = response.result.unwrap_or(serde_json::Value::Null);

    tracing::info!(
        "✅ Successfully executed tool '{}' on server {} in {}ms",
        tool_name,
        server_id,
        latency_ms
    );
    tracing::debug!("Tool result: {}", result);

    Ok(result)
}

/// Server token statistics for deferred loading analysis
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerTokenStats {
    pub server_id: String,
    pub tool_count: usize,
    pub resource_count: usize,
    pub prompt_count: usize,
    pub estimated_tokens: usize,
}

/// MCP token statistics response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpTokenStats {
    pub server_stats: Vec<ServerTokenStats>,
    pub total_tokens: usize,
    pub deferred_tokens: usize,
    pub savings_tokens: usize,
    pub savings_percent: f64,
}

/// Get MCP token statistics for deferred loading analysis
///
/// Calculates token consumption for all MCP servers accessible by a client
/// to help users understand potential savings with deferred loading.
///
/// # Arguments
/// * `client_id` - Client ID to analyze
///
/// # Returns
/// Token statistics showing per-server breakdowns and potential savings
#[tauri::command]
pub async fn get_mcp_token_stats(
    client_id: String,
    config_manager: State<'_, ConfigManager>,
    mcp_manager: State<'_, Arc<McpServerManager>>,
) -> Result<McpTokenStats, String> {
    use lr_config::McpServerAccess;

    // Get client configuration
    let config = config_manager.get();
    let client = config
        .clients
        .iter()
        .find(|c| c.id == client_id)
        .ok_or_else(|| format!("Client not found: {}", client_id))?;

    // Determine which servers to analyze based on access mode
    let server_ids: Vec<String> = match &client.mcp_server_access {
        McpServerAccess::None => vec![],
        McpServerAccess::All => config.mcp_servers.iter().map(|s| s.id.clone()).collect(),
        McpServerAccess::Specific(servers) => servers.clone(),
    };

    let mut server_stats = Vec::new();
    let mut total_tokens = 0;

    // Analyze each allowed server
    for server_id in &server_ids {
        // Ensure server is started
        if !mcp_manager.is_running(server_id) {
            if let Err(e) = mcp_manager.start_server(server_id).await {
                tracing::warn!(
                    "Failed to start server {} for token analysis: {}",
                    server_id,
                    e
                );
                continue;
            }
        }

        // Fetch tools/list
        let tools_request = lr_mcp::protocol::JsonRpcRequest::new(
            Some(serde_json::json!(1)),
            "tools/list".to_string(),
            None,
        );

        let tools_count = match mcp_manager.send_request(server_id, tools_request).await {
            Ok(response) => {
                if let Some(result) = response.result {
                    if let Some(tools) = result.get("tools") {
                        if let Some(array) = tools.as_array() {
                            array.len()
                        } else {
                            0
                        }
                    } else {
                        0
                    }
                } else {
                    0
                }
            }
            Err(e) => {
                tracing::warn!("Failed to fetch tools from {}: {}", server_id, e);
                0
            }
        };

        // Fetch resources/list
        let resources_request = lr_mcp::protocol::JsonRpcRequest::new(
            Some(serde_json::json!(2)),
            "resources/list".to_string(),
            None,
        );

        let resources_count = match mcp_manager.send_request(server_id, resources_request).await {
            Ok(response) => {
                if let Some(result) = response.result {
                    if let Some(resources) = result.get("resources") {
                        if let Some(array) = resources.as_array() {
                            array.len()
                        } else {
                            0
                        }
                    } else {
                        0
                    }
                } else {
                    0
                }
            }
            Err(e) => {
                tracing::warn!("Failed to fetch resources from {}: {}", server_id, e);
                0
            }
        };

        // Fetch prompts/list
        let prompts_request = lr_mcp::protocol::JsonRpcRequest::new(
            Some(serde_json::json!(3)),
            "prompts/list".to_string(),
            None,
        );

        let prompts_count = match mcp_manager.send_request(server_id, prompts_request).await {
            Ok(response) => {
                if let Some(result) = response.result {
                    if let Some(prompts) = result.get("prompts") {
                        if let Some(array) = prompts.as_array() {
                            array.len()
                        } else {
                            0
                        }
                    } else {
                        0
                    }
                } else {
                    0
                }
            }
            Err(e) => {
                tracing::warn!("Failed to fetch prompts from {}: {}", server_id, e);
                0
            }
        };

        // Estimate tokens (rough heuristic: ~200 tokens per tool/resource/prompt)
        let estimated_tokens = (tools_count + resources_count + prompts_count) * 200;
        total_tokens += estimated_tokens;

        server_stats.push(ServerTokenStats {
            server_id: server_id.clone(),
            tool_count: tools_count,
            resource_count: resources_count,
            prompt_count: prompts_count,
            estimated_tokens,
        });
    }

    // Deferred loading: Only search tool visible (~300 tokens)
    let deferred_tokens = 300;
    let savings_tokens = total_tokens.saturating_sub(deferred_tokens);
    let savings_percent = if total_tokens > 0 {
        (savings_tokens as f64 / total_tokens as f64) * 100.0
    } else {
        0.0
    };

    Ok(McpTokenStats {
        server_stats,
        total_tokens,
        deferred_tokens,
        savings_tokens,
        savings_percent,
    })
}

// ============================================================================
// MCP OAuth Browser Flow Commands
// ============================================================================

/// Start a browser-based OAuth flow for an MCP server
///
/// # Arguments
/// * `server_id` - MCP server ID
///
/// # Returns
/// * OAuth flow result with authorization URL to open in browser
#[tauri::command]
pub async fn start_mcp_oauth_browser_flow(
    server_id: String,
    oauth_browser_manager: State<'_, Arc<lr_mcp::oauth_browser::McpOAuthBrowserManager>>,
    config_manager: State<'_, ConfigManager>,
) -> Result<lr_mcp::oauth_browser::OAuthBrowserFlowResult, String> {
    // Get server config
    let config = config_manager.get();
    let server = config
        .mcp_servers
        .iter()
        .find(|s| s.id == server_id)
        .ok_or_else(|| format!("MCP server not found: {}", server_id))?;

    // Get auth config
    let auth_config = server
        .auth_config
        .as_ref()
        .ok_or_else(|| format!("No auth config for server: {}", server_id))?;

    // Start browser flow
    oauth_browser_manager
        .start_browser_flow(&server_id, auth_config)
        .await
        .map_err(|e| e.to_string())
}

/// Poll the status of an OAuth browser flow
///
/// # Arguments
/// * `server_id` - MCP server ID
///
/// # Returns
/// * Current flow status (Pending, Success, Error, or Timeout)
#[tauri::command]
pub fn poll_mcp_oauth_browser_status(
    server_id: String,
    oauth_browser_manager: State<'_, Arc<lr_mcp::oauth_browser::McpOAuthBrowserManager>>,
) -> Result<lr_mcp::oauth_browser::OAuthBrowserFlowStatus, String> {
    oauth_browser_manager
        .poll_flow_status(&server_id)
        .map_err(|e| e.to_string())
}

/// Cancel an active OAuth browser flow
///
/// # Arguments
/// * `server_id` - MCP server ID
#[tauri::command]
pub fn cancel_mcp_oauth_browser_flow(
    server_id: String,
    oauth_browser_manager: State<'_, Arc<lr_mcp::oauth_browser::McpOAuthBrowserManager>>,
) -> Result<(), String> {
    oauth_browser_manager
        .cancel_flow(&server_id)
        .map_err(|e| e.to_string())
}

/// Discover OAuth endpoints for an MCP server
///
/// This uses the existing McpOAuthManager's discover_oauth function to find
/// OAuth configuration via .well-known/oauth-protected-resource endpoint.
///
/// # Arguments
/// * `base_url` - Base URL of the MCP server (e.g., "https://api.github.com")
///
/// # Returns
/// * OAuth discovery information (auth_url, token_url, scopes)
#[tauri::command]
pub async fn discover_mcp_oauth_endpoints(
    base_url: String,
    oauth_manager: State<'_, Arc<lr_mcp::oauth::McpOAuthManager>>,
) -> Result<Option<lr_config::McpOAuthDiscovery>, String> {
    match oauth_manager.discover_oauth(&base_url).await {
        Ok(Some(discovery)) => Ok(Some(lr_config::McpOAuthDiscovery {
            auth_url: discovery.auth_url,
            token_url: discovery.token_endpoint,
            scopes_supported: discovery.scopes_supported,
            discovered_at: chrono::Utc::now(),
        })),
        Ok(None) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

/// Test OAuth connection for an MCP server
///
/// Checks if the server has a valid OAuth token
///
/// # Arguments
/// * `server_id` - MCP server ID
///
/// # Returns
/// * `true` if server has valid authentication, `false` otherwise
#[tauri::command]
pub async fn test_mcp_oauth_connection(
    server_id: String,
    oauth_browser_manager: State<'_, Arc<lr_mcp::oauth_browser::McpOAuthBrowserManager>>,
) -> Result<bool, String> {
    Ok(oauth_browser_manager.has_valid_auth(&server_id).await)
}

/// Revoke OAuth tokens for an MCP server
///
/// Clears all stored tokens (access, refresh, and client secret) from keychain
///
/// # Arguments
/// * `server_id` - MCP server ID
#[tauri::command]
pub fn revoke_mcp_oauth_tokens(
    server_id: String,
    oauth_browser_manager: State<'_, Arc<lr_mcp::oauth_browser::McpOAuthBrowserManager>>,
) -> Result<(), String> {
    oauth_browser_manager
        .revoke_tokens(&server_id)
        .map_err(|e| e.to_string())
}

// ============================================================================
// Inline OAuth Flow Commands (for MCP server creation)
// ============================================================================

/// Result of starting an inline OAuth flow
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InlineOAuthFlowResult {
    /// Unique flow identifier for polling
    pub flow_id: String,
    /// Authorization URL to open in browser
    pub auth_url: String,
    /// Redirect URI used
    pub redirect_uri: String,
    /// CSRF state parameter
    pub state: String,
    /// Discovered OAuth endpoints
    pub discovery: InlineOAuthDiscovery,
}

/// OAuth discovery information for inline flows
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InlineOAuthDiscovery {
    pub auth_url: String,
    pub token_url: String,
    pub scopes: Vec<String>,
}

/// Result of polling an inline OAuth flow
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status")]
pub enum InlineOAuthFlowStatus {
    /// Still waiting for user to complete authorization
    Pending { time_remaining: Option<i64> },
    /// Exchanging authorization code for tokens
    ExchangingToken,
    /// Successfully completed
    Success {
        access_token: String,
        refresh_token: Option<String>,
        expires_in: Option<i64>,
    },
    /// Failed with error
    Error { message: String },
    /// Timed out
    Timeout,
    /// Cancelled
    Cancelled,
}

/// Start an inline OAuth flow for MCP server creation
///
/// This combines OAuth discovery and flow start in one call, allowing OAuth
/// to be completed during server creation before the server exists in config.
///
/// # Arguments
/// * `mcp_url` - The MCP server URL to discover OAuth endpoints from
/// * `client_id` - Optional OAuth client ID (for public clients, can be empty)
/// * `client_secret` - Optional OAuth client secret
///
/// # Returns
/// * OAuth flow result with flow_id, auth_url, and discovery info
#[tauri::command]
pub async fn start_inline_oauth_flow(
    mcp_url: String,
    client_id: Option<String>,
    client_secret: Option<String>,
    oauth_manager: State<'_, Arc<lr_mcp::oauth::McpOAuthManager>>,
    flow_manager: State<'_, Arc<lr_oauth::browser::OAuthFlowManager>>,
) -> Result<InlineOAuthFlowResult, String> {
    // Step 1: Discover OAuth endpoints from MCP URL
    let discovery = oauth_manager
        .discover_oauth(&mcp_url)
        .await
        .map_err(|e| format!("OAuth discovery failed: {}", e))?
        .ok_or_else(|| "This MCP server does not support OAuth".to_string())?;

    let auth_url = discovery.auth_url.clone();
    let token_url = discovery.token_endpoint.clone();
    let scopes = discovery.scopes_supported.clone();

    // Use provided client_id or generate a temporary one for discovery
    let client_id = client_id.unwrap_or_default();
    if client_id.is_empty() {
        return Err("Client ID is required for OAuth flow".to_string());
    }

    // Generate a temporary flow identifier
    let temp_account_id = format!("inline_oauth_{}", uuid::Uuid::new_v4());

    // Determine callback port (use 8080 as default)
    let callback_port = 8080u16;
    let redirect_uri = format!("http://localhost:{}/callback", callback_port);

    // Create flow config
    let config = lr_oauth::browser::OAuthFlowConfig {
        client_id: client_id.clone(),
        client_secret,
        auth_url: auth_url.clone(),
        token_url: token_url.clone(),
        scopes: scopes.clone(),
        redirect_uri: redirect_uri.clone(),
        callback_port,
        keychain_service: "LocalRouter-InlineOAuth".to_string(),
        account_id: temp_account_id,
        extra_auth_params: std::collections::HashMap::new(),
        extra_token_params: std::collections::HashMap::new(),
    };

    // Start the OAuth flow
    let start_result = flow_manager
        .start_flow(config)
        .await
        .map_err(|e| format!("Failed to start OAuth flow: {}", e))?;

    Ok(InlineOAuthFlowResult {
        flow_id: start_result.flow_id.to_string(),
        auth_url: start_result.auth_url,
        redirect_uri: start_result.redirect_uri,
        state: start_result.state,
        discovery: InlineOAuthDiscovery {
            auth_url,
            token_url,
            scopes,
        },
    })
}

/// Poll the status of an inline OAuth flow
///
/// # Arguments
/// * `flow_id` - Flow identifier from start_inline_oauth_flow
///
/// # Returns
/// * Current flow status
#[tauri::command]
pub fn poll_inline_oauth_status(
    flow_id: String,
    flow_manager: State<'_, Arc<lr_oauth::browser::OAuthFlowManager>>,
) -> Result<InlineOAuthFlowStatus, String> {
    // Parse flow_id back to FlowId
    let flow_id_obj = lr_oauth::browser::FlowId::parse(&flow_id)
        .map_err(|e| format!("Invalid flow ID: {}", e))?;

    let result = flow_manager
        .poll_status(flow_id_obj)
        .map_err(|e| e.to_string())?;

    // Convert to InlineOAuthFlowStatus
    match result {
        lr_oauth::browser::OAuthFlowResult::Pending { time_remaining } => {
            Ok(InlineOAuthFlowStatus::Pending { time_remaining })
        }
        lr_oauth::browser::OAuthFlowResult::ExchangingToken => {
            Ok(InlineOAuthFlowStatus::ExchangingToken)
        }
        lr_oauth::browser::OAuthFlowResult::Success { tokens } => {
            Ok(InlineOAuthFlowStatus::Success {
                access_token: tokens.access_token,
                refresh_token: tokens.refresh_token,
                expires_in: tokens.expires_in,
            })
        }
        lr_oauth::browser::OAuthFlowResult::Error { message } => {
            Ok(InlineOAuthFlowStatus::Error { message })
        }
        lr_oauth::browser::OAuthFlowResult::Timeout => Ok(InlineOAuthFlowStatus::Timeout),
        lr_oauth::browser::OAuthFlowResult::Cancelled => Ok(InlineOAuthFlowStatus::Cancelled),
    }
}

/// Cancel an inline OAuth flow
///
/// # Arguments
/// * `flow_id` - Flow identifier from start_inline_oauth_flow
#[tauri::command]
pub fn cancel_inline_oauth_flow(
    flow_id: String,
    flow_manager: State<'_, Arc<lr_oauth::browser::OAuthFlowManager>>,
) -> Result<(), String> {
    // Parse flow_id back to FlowId
    let flow_id_obj = lr_oauth::browser::FlowId::parse(&flow_id)
        .map_err(|e| format!("Invalid flow ID: {}", e))?;

    flow_manager
        .cancel_flow(flow_id_obj)
        .map_err(|e| e.to_string())
}
