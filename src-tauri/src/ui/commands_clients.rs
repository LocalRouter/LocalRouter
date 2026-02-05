//! Client and strategy-related Tauri command handlers
//!
//! Unified client management and routing strategy commands.

use std::sync::Arc;

use lr_config::{
    client_strategy_name, ConfigManager, FirewallPolicy, FirewallRules, McpPermissions,
    ModelPermissions, PermissionState, SkillsPermissions,
};
use serde::{Deserialize, Serialize};
use tauri::{Emitter, State};

// ============================================================================
// Unified Client Management Commands
// ============================================================================

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
    pub mcp_deferred_loading: bool,
    pub created_at: String,
    pub last_used: Option<String>,
    /// Firewall rules for this client
    pub firewall: FirewallRules,
    /// Unified MCP permissions (hierarchical Allow/Ask/Off)
    pub mcp_permissions: McpPermissions,
    /// Unified Skills permissions (hierarchical Allow/Ask/Off)
    pub skills_permissions: SkillsPermissions,
    /// Unified Model permissions (hierarchical Allow/Ask/Off)
    pub model_permissions: ModelPermissions,
    /// Marketplace permission state
    pub marketplace_permission: PermissionState,
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
        .map(|c| ClientInfo {
            id: c.id.clone(),
            name: c.name.clone(),
            client_id: c.id.clone(),
            enabled: c.enabled,
            strategy_id: c.strategy_id.clone(),
            mcp_deferred_loading: c.mcp_deferred_loading,
            created_at: c.created_at.to_rfc3339(),
            last_used: c.last_used.map(|t| t.to_rfc3339()),
            firewall: c.firewall.clone(),
            mcp_permissions: c.mcp_permissions.clone(),
            skills_permissions: c.skills_permissions.clone(),
            model_permissions: c.model_permissions.clone(),
            marketplace_permission: c.marketplace_permission.clone(),
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

    let client_info = ClientInfo {
        id: client.id.clone(),
        name: client.name.clone(),
        client_id: client.id.clone(),
        enabled: client.enabled,
        strategy_id: client.strategy_id.clone(),
        mcp_deferred_loading: client.mcp_deferred_loading,
        created_at: client.created_at.to_rfc3339(),
        last_used: client.last_used.map(|t| t.to_rfc3339()),
        firewall: client.firewall.clone(),
        mcp_permissions: client.mcp_permissions.clone(),
        skills_permissions: client.skills_permissions.clone(),
        model_permissions: client.model_permissions.clone(),
        marketplace_permission: client.marketplace_permission.clone(),
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
    app: tauri::AppHandle,
    request_id: String,
    action: lr_mcp::gateway::firewall::FirewallApprovalAction,
    state: State<'_, Arc<lr_server::state::AppState>>,
    config_manager: State<'_, ConfigManager>,
    tray_graph_manager: State<'_, Arc<crate::ui::tray::TrayGraphManager>>,
) -> Result<(), String> {
    use lr_mcp::gateway::firewall::FirewallApprovalAction;

    tracing::info!(
        "Submitting firewall approval for request {}: {:?}",
        request_id,
        action
    );

    // If AllowPermanent or Allow1Hour, get the pending session info before submitting
    // so we can update client permissions or add time-based approval
    let pending_info = if matches!(
        action,
        FirewallApprovalAction::AllowPermanent | FirewallApprovalAction::Allow1Hour
    ) {
        state
            .mcp_gateway
            .firewall_manager
            .list_pending()
            .into_iter()
            .find(|p| p.request_id == request_id)
    } else {
        None
    };

    // Submit the response to the firewall manager
    state
        .mcp_gateway
        .firewall_manager
        .submit_response(&request_id, action.clone())
        .map_err(|e| e.to_string())?;

    // Handle special actions that modify permissions
    match action {
        FirewallApprovalAction::AllowPermanent => {
            if let Some(ref info) = pending_info {
                if info.is_model_request {
                    // Update model permissions to Allow
                    update_model_permission_for_allow_permanent(&app, &config_manager, info)
                        .await?;
                } else {
                    // Update MCP/skill tool permissions to Allow
                    update_permission_for_allow_permanent(&app, &config_manager, info).await?;
                }
            } else {
                tracing::warn!(
                    "AllowPermanent requested but couldn't find pending info for request {}",
                    request_id
                );
            }
        }
        FirewallApprovalAction::Allow1Hour => {
            if let Some(ref info) = pending_info {
                if info.is_model_request {
                    // Add time-based model approval (1 hour)
                    // server_name contains provider, tool_name contains model_id
                    state.model_approval_tracker.add_1_hour_approval(
                        &info.client_id,
                        &info.server_name,
                        &info.tool_name,
                    );
                    tracing::info!(
                        "Added 1-hour model approval for client {} model {}__{}",
                        info.client_id,
                        info.server_name,
                        info.tool_name
                    );
                }
                // Note: Allow1Hour for MCP/skill tools is not applicable (they use AllowSession)
            } else {
                tracing::warn!(
                    "Allow1Hour requested but couldn't find pending info for request {}",
                    request_id
                );
            }
        }
        _ => {}
    }

    // Rebuild tray menu to remove the pending approval item
    if let Err(e) = crate::ui::tray::rebuild_tray_menu(&app) {
        tracing::warn!("Failed to rebuild tray menu after firewall approval: {}", e);
    }

    // Trigger immediate tray icon update to remove the question mark overlay
    tray_graph_manager.notify_activity();

    Ok(())
}

/// Helper to update model permissions when AllowPermanent is selected for a model request
async fn update_model_permission_for_allow_permanent(
    app: &tauri::AppHandle,
    config_manager: &ConfigManager,
    info: &lr_mcp::gateway::firewall::PendingApprovalInfo,
) -> Result<(), String> {
    use lr_config::PermissionState;

    tracing::info!(
        "Updating model permissions for AllowPermanent: client={}, provider={}, model={}",
        info.client_id,
        info.server_name, // provider
        info.tool_name    // model_id
    );

    config_manager
        .update(|cfg| {
            if let Some(client) = cfg.clients.iter_mut().find(|c| c.id == info.client_id) {
                // For model requests: server_name = provider, tool_name = model_id
                let model_key = format!("{}__{}", info.server_name, info.tool_name);
                client
                    .model_permissions
                    .models
                    .insert(model_key.clone(), PermissionState::Allow);
                tracing::info!("Set model permission to Allow: {}", model_key);
            } else {
                tracing::warn!(
                    "Client {} not found for AllowPermanent model update",
                    info.client_id
                );
            }
        })
        .map_err(|e: lr_types::AppError| e.to_string())?;

    // Emit clients-changed event
    if let Err(e) = app.emit("clients-changed", ()) {
        tracing::error!("Failed to emit clients-changed event: {}", e);
    }

    Ok(())
}

/// Helper to update client permissions when AllowPermanent is selected for MCP/skill tools
async fn update_permission_for_allow_permanent(
    app: &tauri::AppHandle,
    config_manager: &ConfigManager,
    info: &lr_mcp::gateway::firewall::PendingApprovalInfo,
) -> Result<(), String> {
    use lr_config::PermissionState;

    tracing::info!(
        "Updating permissions for AllowPermanent: client={}, tool={}, server={}",
        info.client_id,
        info.tool_name,
        info.server_name
    );

    config_manager
        .update(|cfg| {
            if let Some(client) = cfg.clients.iter_mut().find(|c| c.id == info.client_id) {
                // Determine if this is an MCP tool or skill tool based on the tool name format
                // MCP tools: "server__tool_name"
                // Skill tools: "skill_skillname_..."
                if info.tool_name.starts_with("skill_") {
                    // This is a skill tool - update skills_permissions.tools
                    // Tool name format: skill_skillname_tool_type_script_name
                    // We need to store it as "skillname__full_tool_name" in the permissions
                    let skill_name = &info.server_name;
                    let key = format!("{}__{}", skill_name, info.tool_name);
                    client
                        .skills_permissions
                        .tools
                        .insert(key, PermissionState::Allow);
                    tracing::info!(
                        "Set skill tool permission to Allow: skill={}, tool={}",
                        skill_name,
                        info.tool_name
                    );
                } else if info.tool_name.contains("__") {
                    // This is an MCP tool - update mcp_permissions.tools
                    // Tool name format: "server__tool_name"
                    client
                        .mcp_permissions
                        .tools
                        .insert(info.tool_name.clone(), PermissionState::Allow);
                    tracing::info!("Set MCP tool permission to Allow: {}", info.tool_name);
                } else {
                    // Unknown format, try to set as MCP tool with server prefix
                    let key = format!("{}__{}", info.server_name, info.tool_name);
                    client
                        .mcp_permissions
                        .tools
                        .insert(key.clone(), PermissionState::Allow);
                    tracing::info!(
                        "Set MCP tool permission to Allow (constructed key): {}",
                        key
                    );
                }
            } else {
                tracing::warn!(
                    "Client {} not found for AllowPermanent update",
                    info.client_id
                );
            }
        })
        .map_err(|e: lr_types::AppError| e.to_string())?;

    // Emit clients-changed event
    if let Err(e) = app.emit("clients-changed", ()) {
        tracing::error!("Failed to emit clients-changed event: {}", e);
    }

    Ok(())
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

// ============================================================================
// Unified Permission Commands
// ============================================================================

/// Permission level for MCP permissions
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum McpPermissionLevel {
    Global,
    Server,
    Tool,
    Resource,
    Prompt,
}

/// Permission level for Skills permissions
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SkillsPermissionLevel {
    Global,
    Skill,
    Tool,
}

/// Permission level for Model permissions
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ModelPermissionLevel {
    Global,
    Provider,
    Model,
}

/// Set MCP permission for a client
///
/// # Arguments
/// * `client_id` - The client ID
/// * `level` - Permission level: global, server, tool, resource, prompt
/// * `key` - The key for the permission (e.g., server_id, tool_name)
/// * `state` - The permission state to set
/// * `clear` - If true, removes the override (inherits from parent). If false/None, sets the state.
#[tauri::command]
pub async fn set_client_mcp_permission(
    client_id: String,
    level: McpPermissionLevel,
    key: Option<String>,
    state: PermissionState,
    clear: Option<bool>,
    config_manager: State<'_, ConfigManager>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    tracing::info!(
        "Setting MCP permission for client {}: level={:?}, key={:?}, state={:?}, clear={:?}",
        client_id,
        level,
        key,
        state,
        clear
    );

    let should_clear = clear.unwrap_or(false);

    let mut found = false;
    config_manager
        .update(|cfg| {
            if let Some(client) = cfg.clients.iter_mut().find(|c| c.id == client_id) {
                match level {
                    McpPermissionLevel::Global => {
                        client.mcp_permissions.global = state.clone();
                    }
                    McpPermissionLevel::Server => {
                        if let Some(k) = key.clone() {
                            if should_clear {
                                client.mcp_permissions.servers.remove(&k);
                            } else {
                                client.mcp_permissions.servers.insert(k, state.clone());
                            }
                        }
                    }
                    McpPermissionLevel::Tool => {
                        if let Some(k) = key.clone() {
                            if should_clear {
                                client.mcp_permissions.tools.remove(&k);
                            } else {
                                client.mcp_permissions.tools.insert(k, state.clone());
                            }
                        }
                    }
                    McpPermissionLevel::Resource => {
                        if let Some(k) = key.clone() {
                            if should_clear {
                                client.mcp_permissions.resources.remove(&k);
                            } else {
                                client.mcp_permissions.resources.insert(k, state.clone());
                            }
                        }
                    }
                    McpPermissionLevel::Prompt => {
                        if let Some(k) = key.clone() {
                            if should_clear {
                                client.mcp_permissions.prompts.remove(&k);
                            } else {
                                client.mcp_permissions.prompts.insert(k, state.clone());
                            }
                        }
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

/// Set Skills permission for a client
///
/// # Arguments
/// * `client_id` - The client ID
/// * `level` - Permission level: global, skill, tool
/// * `key` - The key for the permission (e.g., skill_name, tool_name)
/// * `state` - The permission state to set
/// * `clear` - If true, removes the override (inherits from parent). If false/None, sets the state.
#[tauri::command]
pub async fn set_client_skills_permission(
    client_id: String,
    level: SkillsPermissionLevel,
    key: Option<String>,
    state: PermissionState,
    clear: Option<bool>,
    config_manager: State<'_, ConfigManager>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    tracing::info!(
        "Setting Skills permission for client {}: level={:?}, key={:?}, state={:?}, clear={:?}",
        client_id,
        level,
        key,
        state,
        clear
    );

    let should_clear = clear.unwrap_or(false);

    let mut found = false;
    config_manager
        .update(|cfg| {
            if let Some(client) = cfg.clients.iter_mut().find(|c| c.id == client_id) {
                match level {
                    SkillsPermissionLevel::Global => {
                        client.skills_permissions.global = state.clone();
                    }
                    SkillsPermissionLevel::Skill => {
                        if let Some(k) = key.clone() {
                            if should_clear {
                                client.skills_permissions.skills.remove(&k);
                            } else {
                                client.skills_permissions.skills.insert(k, state.clone());
                            }
                        }
                    }
                    SkillsPermissionLevel::Tool => {
                        if let Some(k) = key.clone() {
                            if should_clear {
                                client.skills_permissions.tools.remove(&k);
                            } else {
                                client.skills_permissions.tools.insert(k, state.clone());
                            }
                        }
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

/// Set Model permission for a client
///
/// # Arguments
/// * `client_id` - The client ID
/// * `level` - Permission level: global, provider, model
/// * `key` - The key for the permission (e.g., provider_name, model_id)
/// * `state` - The permission state to set
/// * `clear` - If true, removes the override (inherits from parent). If false/None, sets the state.
#[tauri::command]
pub async fn set_client_model_permission(
    client_id: String,
    level: ModelPermissionLevel,
    key: Option<String>,
    state: PermissionState,
    clear: Option<bool>,
    config_manager: State<'_, ConfigManager>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    tracing::info!(
        "Setting Model permission for client {}: level={:?}, key={:?}, state={:?}, clear={:?}",
        client_id,
        level,
        key,
        state,
        clear
    );

    let should_clear = clear.unwrap_or(false);

    let mut found = false;
    config_manager
        .update(|cfg| {
            if let Some(client) = cfg.clients.iter_mut().find(|c| c.id == client_id) {
                match level {
                    ModelPermissionLevel::Global => {
                        client.model_permissions.global = state.clone();
                    }
                    ModelPermissionLevel::Provider => {
                        if let Some(k) = key.clone() {
                            if should_clear {
                                client.model_permissions.providers.remove(&k);
                            } else {
                                client.model_permissions.providers.insert(k, state.clone());
                            }
                        }
                    }
                    ModelPermissionLevel::Model => {
                        if let Some(k) = key.clone() {
                            if should_clear {
                                client.model_permissions.models.remove(&k);
                            } else {
                                client.model_permissions.models.insert(k, state.clone());
                            }
                        }
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

/// Clear all child MCP permissions for a client (servers, tools, resources, prompts)
/// Called when global permission changes to cascade the change
#[tauri::command]
pub async fn clear_client_mcp_child_permissions(
    client_id: String,
    config_manager: State<'_, ConfigManager>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    tracing::info!("Clearing MCP child permissions for client {}", client_id);

    let mut found = false;
    config_manager
        .update(|cfg| {
            if let Some(client) = cfg.clients.iter_mut().find(|c| c.id == client_id) {
                client.mcp_permissions.servers.clear();
                client.mcp_permissions.tools.clear();
                client.mcp_permissions.resources.clear();
                client.mcp_permissions.prompts.clear();
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

/// Clear all child Skills permissions for a client (skills, tools)
/// Called when global permission changes to cascade the change
#[tauri::command]
pub async fn clear_client_skills_child_permissions(
    client_id: String,
    config_manager: State<'_, ConfigManager>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    tracing::info!("Clearing Skills child permissions for client {}", client_id);

    let mut found = false;
    config_manager
        .update(|cfg| {
            if let Some(client) = cfg.clients.iter_mut().find(|c| c.id == client_id) {
                client.skills_permissions.skills.clear();
                client.skills_permissions.tools.clear();
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

/// Clear all child Model permissions for a client (providers, models)
/// Called when global permission changes to cascade the change
#[tauri::command]
pub async fn clear_client_model_child_permissions(
    client_id: String,
    config_manager: State<'_, ConfigManager>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    tracing::info!("Clearing Model child permissions for client {}", client_id);

    let mut found = false;
    config_manager
        .update(|cfg| {
            if let Some(client) = cfg.clients.iter_mut().find(|c| c.id == client_id) {
                client.model_permissions.providers.clear();
                client.model_permissions.models.clear();
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

/// Set Marketplace permission for a client
///
/// # Arguments
/// * `client_id` - The client ID
/// * `state` - The permission state to set
#[tauri::command]
pub async fn set_client_marketplace_permission(
    client_id: String,
    state: PermissionState,
    config_manager: State<'_, ConfigManager>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    tracing::info!(
        "Setting Marketplace permission for client {}: state={:?}",
        client_id,
        state
    );

    let mut found = false;
    config_manager
        .update(|cfg| {
            if let Some(client) = cfg.clients.iter_mut().find(|c| c.id == client_id) {
                client.marketplace_permission = state.clone();
                // Also update the old marketplace_enabled field for compatibility
                client.marketplace_enabled = state.is_enabled();
                // If enabling marketplace, also enable global marketplace
                if state.is_enabled() {
                    cfg.marketplace.enabled = true;
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
