//! Client and strategy-related Tauri command handlers
//!
//! Unified client management and routing strategy commands.

use std::sync::Arc;

use lr_config::{
    client_strategy_name, ConfigManager, FirewallPolicy, FirewallRules, McpServerAccess,
    SkillsAccess,
};
use serde::{Deserialize, Serialize};
use tauri::{Emitter, State};

// ============================================================================
// Unified Client Management Commands
// ============================================================================

/// MCP server access mode for the UI
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum McpAccessMode {
    /// No MCP access
    None,
    /// Access to all MCP servers
    All,
    /// Access to specific servers only
    Specific,
}

/// Client information for display
///
/// NOTE: This struct does NOT contain the client secret. The secret is stored
/// securely in the keychain and must be fetched separately via `get_client_value`.
/// The `client_id` field here is just the public identifier (same as `id`),
/// NOT the secret key used for authentication.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientInfo {
    /// The unique client identifier (UUID)
    pub id: String,
    /// Human-readable name for the client
    pub name: String,
    /// Public client identifier for OAuth (same as `id`, NOT the secret).
    /// To get the actual secret/API key, use `get_client_value` command.
    pub client_id: String,
    pub enabled: bool,
    pub strategy_id: String,
    pub allowed_llm_providers: Vec<String>,
    /// The MCP access mode: "none", "all", or "specific"
    pub mcp_access_mode: McpAccessMode,
    /// List of specific MCP server IDs (only relevant when mcp_access_mode is "specific")
    pub mcp_servers: Vec<String>,
    pub mcp_deferred_loading: bool,
    /// Skills access mode: "none", "all", or "specific"
    pub skills_access_mode: SkillsAccessMode,
    /// List of specific skill names (only relevant when skills_access_mode is "specific")
    pub skills_names: Vec<String>,
    pub created_at: String,
    pub last_used: Option<String>,
    /// Firewall rules for this client
    pub firewall: FirewallRules,
}

/// Skills access mode for the UI
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SkillsAccessMode {
    /// No skills access
    None,
    /// Access to all discovered skills
    All,
    /// Access to specific skills only
    Specific,
}

/// Convert SkillsAccess to UI representation
fn skills_access_to_ui(access: &SkillsAccess) -> (SkillsAccessMode, Vec<String>) {
    match access {
        SkillsAccess::None => (SkillsAccessMode::None, vec![]),
        SkillsAccess::All => (SkillsAccessMode::All, vec![]),
        SkillsAccess::Specific(names) => (SkillsAccessMode::Specific, names.clone()),
    }
}

/// Convert McpServerAccess to UI representation
fn mcp_access_to_ui(access: &McpServerAccess) -> (McpAccessMode, Vec<String>) {
    match access {
        McpServerAccess::None => (McpAccessMode::None, vec![]),
        McpServerAccess::All => (McpAccessMode::All, vec![]),
        McpServerAccess::Specific(servers) => (McpAccessMode::Specific, servers.clone()),
    }
}

/// List all clients
#[tauri::command]
pub async fn list_clients(
    client_manager: State<'_, Arc<lr_clients::ClientManager>>,
) -> Result<Vec<ClientInfo>, String> {
    let clients = client_manager.list_clients();
    Ok(clients
        .into_iter()
        .filter(|c| !c.name.starts_with("_test_strategy_"))
        .map(|c| {
            let (mcp_access_mode, mcp_servers) = mcp_access_to_ui(&c.mcp_server_access);
            let (skills_access_mode, skills_names) = skills_access_to_ui(&c.skills_access);
            ClientInfo {
                id: c.id.clone(),
                name: c.name.clone(),
                client_id: c.id.clone(),
                enabled: c.enabled,
                strategy_id: c.strategy_id.clone(),
                allowed_llm_providers: c.allowed_llm_providers.clone(),
                mcp_access_mode,
                mcp_servers,
                mcp_deferred_loading: c.mcp_deferred_loading,
                skills_access_mode,
                skills_names,
                created_at: c.created_at.to_rfc3339(),
                last_used: c.last_used.map(|t| t.to_rfc3339()),
                firewall: c.firewall.clone(),
            }
        })
        .collect())
}

/// Create a new client
#[tauri::command]
pub async fn create_client(
    name: String,
    client_manager: State<'_, Arc<lr_clients::ClientManager>>,
    config_manager: State<'_, ConfigManager>,
    app: tauri::AppHandle,
) -> Result<(String, ClientInfo), String> {
    tracing::info!("Creating new client with name: {}", name);

    // Create client with auto-created strategy
    let (client, _strategy) = config_manager
        .create_client_with_strategy(name.clone())
        .map_err(|e| e.to_string())?;

    tracing::info!("Client created: {} ({})", client.name, client.id);

    // Store client secret in keychain and add to client manager
    let secret = client_manager
        .add_client_with_secret(client.clone())
        .map_err(|e| e.to_string())?;

    // Persist to disk
    config_manager.save().await.map_err(|e| e.to_string())?;

    // Rebuild tray menu
    if let Err(e) = crate::ui::tray::rebuild_tray_menu(&app) {
        tracing::error!("Failed to rebuild tray menu: {}", e);
    }

    // Emit events for UI updates
    if let Err(e) = app.emit("clients-changed", ()) {
        tracing::error!("Failed to emit clients-changed event: {}", e);
    }
    if let Err(e) = app.emit("strategies-changed", ()) {
        tracing::error!("Failed to emit strategies-changed event: {}", e);
    }

    let (mcp_access_mode, mcp_servers) = mcp_access_to_ui(&client.mcp_server_access);
    let (skills_access_mode, skills_names) = skills_access_to_ui(&client.skills_access);
    let client_info = ClientInfo {
        id: client.id.clone(),
        name: client.name.clone(),
        client_id: client.id.clone(),
        enabled: client.enabled,
        strategy_id: client.strategy_id.clone(),
        allowed_llm_providers: client.allowed_llm_providers.clone(),
        mcp_access_mode,
        mcp_servers,
        mcp_deferred_loading: client.mcp_deferred_loading,
        skills_access_mode,
        skills_names,
        created_at: client.created_at.to_rfc3339(),
        last_used: client.last_used.map(|t| t.to_rfc3339()),
        firewall: client.firewall.clone(),
    };

    Ok((secret, client_info))
}

/// Delete a client
#[tauri::command]
pub async fn delete_client(
    client_id: String,
    client_manager: State<'_, Arc<lr_clients::ClientManager>>,
    config_manager: State<'_, ConfigManager>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    tracing::info!("Deleting client: {}", client_id);

    // Delete from client manager (removes from keychain and in-memory)
    client_manager
        .delete_client(&client_id)
        .map_err(|e| e.to_string())?;

    // Delete from config (cascade deletes owned strategies)
    config_manager
        .delete_client(&client_id)
        .map_err(|e| e.to_string())?;

    // Persist to disk
    config_manager.save().await.map_err(|e| e.to_string())?;

    // Rebuild tray menu
    if let Err(e) = crate::ui::tray::rebuild_tray_menu(&app) {
        tracing::error!("Failed to rebuild tray menu: {}", e);
    }

    // Emit events for UI updates
    if let Err(e) = app.emit("clients-changed", ()) {
        tracing::error!("Failed to emit clients-changed event: {}", e);
    }
    if let Err(e) = app.emit("strategies-changed", ()) {
        tracing::error!("Failed to emit strategies-changed event: {}", e);
    }

    Ok(())
}

/// Update client name
#[tauri::command]
pub async fn update_client_name(
    client_id: String,
    name: String,
    client_manager: State<'_, Arc<lr_clients::ClientManager>>,
    config_manager: State<'_, ConfigManager>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    tracing::info!("Updating client {} name to: {}", client_id, name);

    // Update in client manager (in-memory)
    client_manager
        .update_client(&client_id, Some(name.clone()), None)
        .map_err(|e| e.to_string())?;

    // Update in config
    let mut strategies_renamed = false;
    config_manager
        .update(|cfg| {
            if let Some(client) = cfg.clients.iter_mut().find(|c| c.id == client_id) {
                client.name = name.clone();
            }

            // Also rename strategies that have this client as parent
            for strategy in cfg.strategies.iter_mut() {
                if strategy.parent.as_ref() == Some(&client_id) {
                    strategy.name = client_strategy_name(&name);
                    strategies_renamed = true;
                }
            }
        })
        .map_err(|e| e.to_string())?;

    // Persist to disk
    config_manager.save().await.map_err(|e| e.to_string())?;

    // Rebuild tray menu
    if let Err(e) = crate::ui::tray::rebuild_tray_menu(&app) {
        tracing::error!("Failed to rebuild tray menu: {}", e);
    }

    // Emit events for UI updates
    if let Err(e) = app.emit("clients-changed", ()) {
        tracing::error!("Failed to emit clients-changed event: {}", e);
    }
    if strategies_renamed {
        if let Err(e) = app.emit("strategies-changed", ()) {
            tracing::error!("Failed to emit strategies-changed event: {}", e);
        }
    }

    Ok(())
}

/// Enable or disable a client
#[tauri::command]
pub async fn toggle_client_enabled(
    client_id: String,
    enabled: bool,
    client_manager: State<'_, Arc<lr_clients::ClientManager>>,
    config_manager: State<'_, ConfigManager>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    tracing::info!("Setting client {} enabled: {}", client_id, enabled);

    // Update in client manager
    if enabled {
        client_manager
            .enable_client(&client_id)
            .map_err(|e| e.to_string())?;
    } else {
        client_manager
            .disable_client(&client_id)
            .map_err(|e| e.to_string())?;
    }

    // Update in config
    let mut found = false;
    config_manager
        .update(|cfg| {
            if let Some(client) = cfg.clients.iter_mut().find(|c| c.id == client_id) {
                client.enabled = enabled;
                found = true;
            }
        })
        .map_err(|e| e.to_string())?;

    if !found {
        return Err(format!("Client not found: {}", client_id));
    }

    // Persist to disk
    config_manager.save().await.map_err(|e| e.to_string())?;

    // Rebuild tray menu
    if let Err(e) = crate::ui::tray::rebuild_tray_menu(&app) {
        tracing::error!("Failed to rebuild tray menu: {}", e);
    }

    // Emit clients-changed event for UI updates
    if let Err(e) = app.emit("clients-changed", ()) {
        tracing::error!("Failed to emit clients-changed event: {}", e);
    }

    Ok(())
}

/// Rotate a client's secret (API key)
///
/// Generates a new secret for the client and stores it in the keychain.
/// The old secret is immediately invalidated.
///
/// # Arguments
/// * `client_id` - The client ID whose secret should be rotated
///
/// # Returns
/// The new secret string (shown once to the user)
#[tauri::command]
pub async fn rotate_client_secret(
    client_id: String,
    client_manager: State<'_, Arc<lr_clients::ClientManager>>,
) -> Result<String, String> {
    tracing::info!("Rotating secret for client: {}", client_id);

    let new_secret = client_manager
        .rotate_secret(&client_id)
        .map_err(|e| e.to_string())?;

    Ok(new_secret)
}

/// Toggle MCP deferred loading for a client
///
/// When enabled, only a search tool is initially visible in the MCP gateway.
/// Tools are activated on-demand through search queries, dramatically reducing
/// token consumption for large catalogs.
///
/// # Arguments
/// * `client_id` - Client ID
/// * `enabled` - Whether to enable deferred loading
#[tauri::command]
pub async fn toggle_client_deferred_loading(
    client_id: String,
    enabled: bool,
    client_manager: State<'_, Arc<lr_clients::ClientManager>>,
    config_manager: State<'_, ConfigManager>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    tracing::info!(
        "Setting client {} MCP deferred loading: {}",
        client_id,
        enabled
    );

    // Update in client manager (in-memory)
    client_manager
        .set_mcp_deferred_loading(&client_id, enabled)
        .map_err(|e| e.to_string())?;

    // Update in config (for persistence)
    config_manager
        .update(|cfg| {
            if let Some(client) = cfg.clients.iter_mut().find(|c| c.id == client_id) {
                client.mcp_deferred_loading = enabled;
            }
        })
        .map_err(|e| e.to_string())?;

    // Persist to disk
    config_manager.save().await.map_err(|e| e.to_string())?;

    // Emit clients-changed event for UI updates
    if let Err(e) = app.emit("clients-changed", ()) {
        tracing::error!("Failed to emit clients-changed event: {}", e);
    }

    Ok(())
}

/// Add an LLM provider to a client's allowed list
#[tauri::command]
pub async fn add_client_llm_provider(
    client_id: String,
    provider: String,
    client_manager: State<'_, Arc<lr_clients::ClientManager>>,
    config_manager: State<'_, ConfigManager>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    tracing::info!("Adding LLM provider {} to client {}", provider, client_id);

    // Update in client manager
    client_manager
        .add_llm_provider(&client_id, &provider)
        .map_err(|e| e.to_string())?;

    // Update in config
    let mut found = false;
    config_manager
        .update(|cfg| {
            if let Some(client) = cfg.clients.iter_mut().find(|c| c.id == client_id) {
                if !client.allowed_llm_providers.contains(&provider) {
                    client.allowed_llm_providers.push(provider.clone());
                }
                found = true;
            }
        })
        .map_err(|e| e.to_string())?;

    if !found {
        return Err(format!("Client not found: {}", client_id));
    }

    // Persist to disk
    config_manager.save().await.map_err(|e| e.to_string())?;

    // Emit clients-changed event for UI updates
    if let Err(e) = app.emit("clients-changed", ()) {
        tracing::error!("Failed to emit clients-changed event: {}", e);
    }

    Ok(())
}

/// Remove an LLM provider from a client's allowed list
#[tauri::command]
pub async fn remove_client_llm_provider(
    client_id: String,
    provider: String,
    client_manager: State<'_, Arc<lr_clients::ClientManager>>,
    config_manager: State<'_, ConfigManager>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    tracing::info!(
        "Removing LLM provider {} from client {}",
        provider,
        client_id
    );

    // Update in client manager
    client_manager
        .remove_llm_provider(&client_id, &provider)
        .map_err(|e| e.to_string())?;

    // Update in config
    let mut found = false;
    config_manager
        .update(|cfg| {
            if let Some(client) = cfg.clients.iter_mut().find(|c| c.id == client_id) {
                client.allowed_llm_providers.retain(|p| p != &provider);
                found = true;
            }
        })
        .map_err(|e| e.to_string())?;

    if !found {
        return Err(format!("Client not found: {}", client_id));
    }

    // Persist to disk
    config_manager.save().await.map_err(|e| e.to_string())?;

    // Emit clients-changed event for UI updates
    if let Err(e) = app.emit("clients-changed", ()) {
        tracing::error!("Failed to emit clients-changed event: {}", e);
    }

    Ok(())
}

/// Add an MCP server to a client's allowed list
#[tauri::command]
pub async fn add_client_mcp_server(
    client_id: String,
    server_id: String,
    client_manager: State<'_, Arc<lr_clients::ClientManager>>,
    config_manager: State<'_, ConfigManager>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    tracing::info!("Adding MCP server {} to client {}", server_id, client_id);

    // Update in client manager
    client_manager
        .add_mcp_server(&client_id, &server_id)
        .map_err(|e| e.to_string())?;

    // Update in config
    let mut found = false;
    config_manager
        .update(|cfg| {
            if let Some(client) = cfg.clients.iter_mut().find(|c| c.id == client_id) {
                client.add_mcp_server(server_id.clone());
                found = true;
            }
        })
        .map_err(|e| e.to_string())?;

    if !found {
        return Err(format!("Client not found: {}", client_id));
    }

    // Persist to disk
    config_manager.save().await.map_err(|e| e.to_string())?;

    // Emit clients-changed event for UI updates
    if let Err(e) = app.emit("clients-changed", ()) {
        tracing::error!("Failed to emit clients-changed event: {}", e);
    }

    Ok(())
}

/// Remove an MCP server from a client's allowed list
#[tauri::command]
pub async fn remove_client_mcp_server(
    client_id: String,
    server_id: String,
    client_manager: State<'_, Arc<lr_clients::ClientManager>>,
    config_manager: State<'_, ConfigManager>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    tracing::info!(
        "Removing MCP server {} from client {}",
        server_id,
        client_id
    );

    // Update in client manager
    client_manager
        .remove_mcp_server(&client_id, &server_id)
        .map_err(|e| e.to_string())?;

    // Update in config
    let mut found = false;
    config_manager
        .update(|cfg| {
            if let Some(client) = cfg.clients.iter_mut().find(|c| c.id == client_id) {
                client.remove_mcp_server(&server_id);
                found = true;
            }
        })
        .map_err(|e| e.to_string())?;

    if !found {
        return Err(format!("Client not found: {}", client_id));
    }

    // Persist to disk
    config_manager.save().await.map_err(|e| e.to_string())?;

    // Emit clients-changed event for UI updates
    if let Err(e) = app.emit("clients-changed", ()) {
        tracing::error!("Failed to emit clients-changed event: {}", e);
    }

    Ok(())
}

/// Set MCP server access mode for a client
///
/// # Arguments
/// * `client_id` - The client ID
/// * `mode` - The access mode: "none", "all", or "specific"
/// * `servers` - List of server IDs (only used when mode is "specific")
#[tauri::command]
pub async fn set_client_mcp_access(
    client_id: String,
    mode: McpAccessMode,
    servers: Vec<String>,
    client_manager: State<'_, Arc<lr_clients::ClientManager>>,
    config_manager: State<'_, ConfigManager>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    let access = match mode {
        McpAccessMode::None => McpServerAccess::None,
        McpAccessMode::All => McpServerAccess::All,
        McpAccessMode::Specific => McpServerAccess::Specific(servers),
    };

    tracing::info!(
        "Setting MCP access for client {} to {:?}",
        client_id,
        access
    );

    // Update in client manager
    client_manager
        .set_mcp_server_access(&client_id, access.clone())
        .map_err(|e| e.to_string())?;

    // Update in config
    let mut found = false;
    config_manager
        .update(|cfg| {
            if let Some(client) = cfg.clients.iter_mut().find(|c| c.id == client_id) {
                client.set_mcp_server_access(access.clone());
                found = true;
            }
        })
        .map_err(|e| e.to_string())?;

    if !found {
        return Err(format!("Client not found: {}", client_id));
    }

    // Persist to disk
    config_manager.save().await.map_err(|e| e.to_string())?;

    // Emit clients-changed event for UI updates
    if let Err(e) = app.emit("clients-changed", ()) {
        tracing::error!("Failed to emit clients-changed event: {}", e);
    }

    Ok(())
}

/// Get the client bearer token value (secret)
///
/// For clients, the secret is stored in the keychain, just like API keys.
/// This provides a consistent interface with get_api_key_value.
///
/// # Arguments
/// * `id` - The client_id
///
/// # Returns
/// * The client secret (which is used as the bearer token)
#[tauri::command]
pub async fn get_client_value(
    id: String,
    client_manager: State<'_, Arc<lr_clients::ClientManager>>,
) -> Result<String, String> {
    // Get the client to verify it exists and get its internal ID
    let client = client_manager
        .get_client(&id)
        .ok_or_else(|| format!("Client not found: {}", id))?;

    // Retrieve the secret from the keychain using the internal ID
    client_manager
        .get_secret(&client.id)
        .map_err(|e| format!("Failed to retrieve client secret: {}", e))?
        .ok_or_else(|| format!("Client secret not found in keychain: {}", id))
}

// ============================================================================
// Strategy Management Commands
// ============================================================================

/// List all routing strategies
#[tauri::command]
pub async fn list_strategies(
    config_manager: State<'_, ConfigManager>,
) -> Result<Vec<lr_config::Strategy>, String> {
    let config = config_manager.get();
    Ok(config.strategies)
}

/// Get a specific strategy by ID
#[tauri::command]
pub async fn get_strategy(
    strategy_id: String,
    config_manager: State<'_, ConfigManager>,
) -> Result<lr_config::Strategy, String> {
    let config = config_manager.get();
    config
        .strategies
        .iter()
        .find(|s| s.id == strategy_id)
        .cloned()
        .ok_or_else(|| format!("Strategy not found: {}", strategy_id))
}

/// Create a new routing strategy
#[tauri::command]
pub async fn create_strategy(
    name: String,
    parent: Option<String>,
    config_manager: State<'_, ConfigManager>,
    app: tauri::AppHandle,
) -> Result<lr_config::Strategy, String> {
    tracing::info!("Creating strategy: {}", name);

    let strategy = if let Some(parent_id) = parent {
        // Validate parent exists
        let config = config_manager.get();
        let client = config
            .clients
            .iter()
            .find(|c| c.id == parent_id)
            .ok_or_else(|| format!("Parent client not found: {}", parent_id))?;

        lr_config::Strategy::new_for_client(parent_id, client.name.clone())
    } else {
        lr_config::Strategy::new(name)
    };

    let strategy_clone = strategy.clone();

    config_manager
        .update(|cfg| {
            cfg.strategies.push(strategy);
        })
        .map_err(|e| e.to_string())?;

    // Persist to disk
    config_manager.save().await.map_err(|e| e.to_string())?;

    // Rebuild tray menu
    if let Err(e) = crate::ui::tray::rebuild_tray_menu(&app) {
        tracing::error!("Failed to rebuild tray menu: {}", e);
    }

    // Emit event for UI updates
    if let Err(e) = app.emit("strategies-changed", ()) {
        tracing::error!("Failed to emit strategies-changed event: {}", e);
    }

    tracing::info!("Strategy created: {}", strategy_clone.id);

    Ok(strategy_clone)
}

/// Update a routing strategy
#[tauri::command]
pub async fn update_strategy(
    strategy_id: String,
    name: Option<String>,
    allowed_models: Option<lr_config::AvailableModelsSelection>,
    auto_config: Option<lr_config::AutoModelConfig>,
    rate_limits: Option<Vec<lr_config::StrategyRateLimit>>,
    config_manager: State<'_, ConfigManager>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    tracing::info!("Updating strategy: {}", strategy_id);

    let mut found = false;
    config_manager
        .update(|cfg| {
            if let Some(strategy) = cfg.strategies.iter_mut().find(|s| s.id == strategy_id) {
                if let Some(new_name) = name {
                    strategy.name = new_name;
                }
                if let Some(models) = allowed_models {
                    strategy.allowed_models = models;
                }
                if let Some(config) = auto_config {
                    strategy.auto_config = Some(config);
                }
                if let Some(limits) = rate_limits {
                    strategy.rate_limits = limits;
                }
                found = true;
            }
        })
        .map_err(|e| e.to_string())?;

    if !found {
        return Err(format!("Strategy not found: {}", strategy_id));
    }

    // Persist to disk
    config_manager.save().await.map_err(|e| e.to_string())?;

    // Rebuild tray menu
    if let Err(e) = crate::ui::tray::rebuild_tray_menu(&app) {
        tracing::error!("Failed to rebuild tray menu: {}", e);
    }

    // Emit event for UI updates
    if let Err(e) = app.emit("strategies-changed", ()) {
        tracing::error!("Failed to emit strategies-changed event: {}", e);
    }

    tracing::info!("Strategy updated: {}", strategy_id);

    Ok(())
}

/// Delete a routing strategy
#[tauri::command]
pub async fn delete_strategy(
    strategy_id: String,
    config_manager: State<'_, ConfigManager>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    tracing::info!("Deleting strategy: {}", strategy_id);

    // Check if any clients are using this strategy
    let config = config_manager.get();
    let clients_using = config
        .clients
        .iter()
        .filter(|c| c.strategy_id == strategy_id)
        .count();

    if clients_using > 0 {
        return Err(format!(
            "Cannot delete strategy: {} client(s) are using it",
            clients_using
        ));
    }

    // Delete the strategy
    config_manager
        .update(|cfg| {
            cfg.strategies.retain(|s| s.id != strategy_id);
        })
        .map_err(|e| e.to_string())?;

    // Persist to disk
    config_manager.save().await.map_err(|e| e.to_string())?;

    // Rebuild tray menu
    if let Err(e) = crate::ui::tray::rebuild_tray_menu(&app) {
        tracing::error!("Failed to rebuild tray menu: {}", e);
    }

    // Emit event for UI updates
    if let Err(e) = app.emit("strategies-changed", ()) {
        tracing::error!("Failed to emit strategies-changed event: {}", e);
    }

    tracing::info!("Strategy deleted: {}", strategy_id);

    Ok(())
}

/// Get all clients using a specific strategy
#[tauri::command]
pub async fn get_clients_using_strategy(
    strategy_id: String,
    config_manager: State<'_, ConfigManager>,
) -> Result<Vec<lr_config::Client>, String> {
    let config = config_manager.get();
    Ok(config
        .clients
        .iter()
        .filter(|c| c.strategy_id == strategy_id)
        .cloned()
        .collect())
}

/// Assign a client to a different strategy
#[tauri::command]
pub async fn assign_client_strategy(
    client_id: String,
    strategy_id: String,
    config_manager: State<'_, ConfigManager>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    tracing::info!("Assigning client {} to strategy {}", client_id, strategy_id);

    config_manager
        .assign_client_strategy(&client_id, &strategy_id)
        .map_err(|e| e.to_string())?;

    // Persist to disk
    config_manager.save().await.map_err(|e| e.to_string())?;

    // Rebuild tray menu
    if let Err(e) = crate::ui::tray::rebuild_tray_menu(&app) {
        tracing::error!("Failed to rebuild tray menu: {}", e);
    }

    // Emit events for UI updates
    if let Err(e) = app.emit("strategies-changed", ()) {
        tracing::error!("Failed to emit strategies-changed event: {}", e);
    }
    if let Err(e) = app.emit("clients-changed", ()) {
        tracing::error!("Failed to emit clients-changed event: {}", e);
    }

    tracing::info!("Client {} assigned to strategy {}", client_id, strategy_id);

    Ok(())
}

// ============================================================================
// Firewall Commands
// ============================================================================

/// Get firewall rules for a client
#[tauri::command]
pub async fn get_client_firewall_rules(
    client_id: String,
    config_manager: State<'_, ConfigManager>,
) -> Result<FirewallRules, String> {
    let config = config_manager.get();
    let client = config
        .clients
        .iter()
        .find(|c| c.id == client_id)
        .ok_or_else(|| format!("Client not found: {}", client_id))?;
    Ok(client.firewall.clone())
}

/// Set default firewall policy for a client
#[tauri::command]
pub async fn set_client_default_firewall_policy(
    client_id: String,
    policy: FirewallPolicy,
    config_manager: State<'_, ConfigManager>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    tracing::info!(
        "Setting default firewall policy for client {} to {:?}",
        client_id,
        policy
    );

    let mut found = false;
    config_manager
        .update(|cfg| {
            if let Some(client) = cfg.clients.iter_mut().find(|c| c.id == client_id) {
                client.firewall.default_policy = policy.clone();
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

/// Set a firewall rule for a client
///
/// # Arguments
/// * `client_id` - Client ID
/// * `rule_type` - One of: "server", "tool", "skill", "skill_tool"
/// * `key` - The rule key (server_id, tool_name, skill_name, or skill_tool_name)
/// * `policy` - The policy to set, or null to remove the rule
#[tauri::command]
pub async fn set_client_firewall_rule(
    client_id: String,
    rule_type: String,
    key: String,
    policy: Option<FirewallPolicy>,
    config_manager: State<'_, ConfigManager>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    tracing::info!(
        "Setting firewall rule for client {}: type={}, key={}, policy={:?}",
        client_id,
        rule_type,
        key,
        policy
    );

    // Validate rule_type before updating config
    if !["server", "tool", "skill", "skill_tool"].contains(&rule_type.as_str()) {
        return Err(format!(
            "Invalid rule_type '{}': must be one of server, tool, skill, skill_tool",
            rule_type
        ));
    }

    let mut found = false;
    config_manager
        .update(|cfg| {
            if let Some(client) = cfg.clients.iter_mut().find(|c| c.id == client_id) {
                let rules_map = match rule_type.as_str() {
                    "server" => &mut client.firewall.server_rules,
                    "tool" => &mut client.firewall.tool_rules,
                    "skill" => &mut client.firewall.skill_rules,
                    "skill_tool" => &mut client.firewall.skill_tool_rules,
                    _ => unreachable!(), // validated above
                };
                match policy {
                    Some(p) => {
                        rules_map.insert(key.clone(), p);
                    }
                    None => {
                        rules_map.remove(&key);
                    }
                }
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

/// Submit a response to a pending firewall approval request
#[tauri::command]
pub async fn submit_firewall_approval(
    request_id: String,
    action: lr_mcp::gateway::firewall::FirewallApprovalAction,
    state: State<'_, Arc<lr_server::state::AppState>>,
) -> Result<(), String> {
    tracing::info!(
        "Submitting firewall approval for request {}: {:?}",
        request_id,
        action
    );

    state
        .mcp_gateway
        .firewall_manager
        .submit_response(&request_id, action)
        .map_err(|e| e.to_string())
}

/// List all pending firewall approval requests
#[tauri::command]
pub async fn list_pending_firewall_approvals(
    state: State<'_, Arc<lr_server::state::AppState>>,
) -> Result<Vec<lr_mcp::gateway::firewall::PendingApprovalInfo>, String> {
    Ok(state.mcp_gateway.firewall_manager.list_pending())
}

/// Get details for a specific pending firewall approval request
#[tauri::command]
pub async fn get_firewall_approval_details(
    request_id: String,
    state: State<'_, Arc<lr_server::state::AppState>>,
) -> Result<Option<lr_mcp::gateway::firewall::PendingApprovalInfo>, String> {
    let pending = state.mcp_gateway.firewall_manager.list_pending();
    Ok(pending.into_iter().find(|p| p.request_id == request_id))
}
