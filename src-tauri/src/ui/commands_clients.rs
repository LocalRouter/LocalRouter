//! Client and strategy-related Tauri command handlers
//!
//! Unified client management and routing strategy commands.

use std::sync::Arc;

use lr_config::{
    client_strategy_name, ClientMode, ConfigManager, McpPermissions, ModelPermissions,
    PermissionState, SkillsPermissions,
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
    /// Unified MCP permissions (hierarchical Allow/Ask/Off)
    pub mcp_permissions: McpPermissions,
    /// Unified Skills permissions (hierarchical Allow/Ask/Off)
    pub skills_permissions: SkillsPermissions,
    /// Unified Model permissions (hierarchical Allow/Ask/Off)
    pub model_permissions: ModelPermissions,
    /// Marketplace permission state
    pub marketplace_permission: PermissionState,
    /// Client mode (both, llm_only, mcp_only)
    pub client_mode: ClientMode,
    /// Template ID used to create this client
    pub template_id: Option<String>,
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
            mcp_permissions: c.mcp_permissions.clone(),
            skills_permissions: c.skills_permissions.clone(),
            model_permissions: c.model_permissions.clone(),
            marketplace_permission: c.marketplace_permission.clone(),
            client_mode: c.client_mode.clone(),
            template_id: c.template_id.clone(),
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
        mcp_permissions: client.mcp_permissions.clone(),
        skills_permissions: client.skills_permissions.clone(),
        model_permissions: client.model_permissions.clone(),
        marketplace_permission: client.marketplace_permission.clone(),
        client_mode: client.client_mode.clone(),
        template_id: client.template_id.clone(),
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
// Firewall Approval Commands
// ============================================================================

/// Submit a response to a pending firewall approval request
#[tauri::command]
pub async fn submit_firewall_approval(
    app: tauri::AppHandle,
    request_id: String,
    action: lr_mcp::gateway::firewall::FirewallApprovalAction,
    edited_arguments: Option<String>,
    state: State<'_, Arc<lr_server::state::AppState>>,
    config_manager: State<'_, ConfigManager>,
    tray_graph_manager: State<'_, Arc<crate::ui::tray::TrayGraphManager>>,
) -> Result<(), String> {
    use lr_mcp::gateway::firewall::FirewallApprovalAction;

    tracing::info!(
        "Submitting firewall approval for request {}: {:?} (has_edits: {})",
        request_id,
        action,
        edited_arguments.is_some()
    );

    // Parse edited_arguments from JSON string to Value
    let edited_args_value: Option<serde_json::Value> = edited_arguments
        .as_ref()
        .and_then(|s| serde_json::from_str(s).ok());

    // If AllowPermanent, Allow1Hour, or DenyAlways, get the pending session info before submitting
    // so we can update client permissions or add time-based approval
    let pending_info = if matches!(
        action,
        FirewallApprovalAction::AllowPermanent
            | FirewallApprovalAction::Allow1Hour
            | FirewallApprovalAction::DenyAlways
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
        .submit_response(&request_id, action.clone(), edited_args_value)
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
        FirewallApprovalAction::DenyAlways => {
            if let Some(ref info) = pending_info {
                if info.is_model_request {
                    // Update model permissions to Off
                    update_model_permission_for_deny_permanent(&app, &config_manager, info).await?;
                } else {
                    // Update MCP/skill tool permissions to Off
                    update_permission_for_deny_permanent(&app, &config_manager, info).await?;
                }
            } else {
                tracing::warn!(
                    "DenyAlways requested but couldn't find pending info for request {}",
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

    // Save config to disk
    config_manager
        .save()
        .await
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
                } else {
                    // MCP tool — info.server_name is the server UUID,
                    // info.tool_name is the namespaced name (slug__original_name).
                    // Permission key must be UUID__original_name to match resolve_tool() and UI.
                    let original_name =
                        info.tool_name.split("__").nth(1).unwrap_or(&info.tool_name);
                    let key = format!("{}__{}", info.server_name, original_name);
                    client
                        .mcp_permissions
                        .tools
                        .insert(key.clone(), PermissionState::Allow);
                    tracing::info!("Set MCP tool permission to Allow: {}", key);
                }
            } else {
                tracing::warn!(
                    "Client {} not found for AllowPermanent update",
                    info.client_id
                );
            }
        })
        .map_err(|e: lr_types::AppError| e.to_string())?;

    // Save config to disk
    config_manager
        .save()
        .await
        .map_err(|e: lr_types::AppError| e.to_string())?;

    // Emit clients-changed event
    if let Err(e) = app.emit("clients-changed", ()) {
        tracing::error!("Failed to emit clients-changed event: {}", e);
    }

    Ok(())
}

/// Helper to update model permissions when DenyAlways is selected for a model request
async fn update_model_permission_for_deny_permanent(
    app: &tauri::AppHandle,
    config_manager: &ConfigManager,
    info: &lr_mcp::gateway::firewall::PendingApprovalInfo,
) -> Result<(), String> {
    use lr_config::PermissionState;

    tracing::info!(
        "Updating model permissions for DenyAlways: client={}, provider={}, model={}",
        info.client_id,
        info.server_name, // provider
        info.tool_name    // model_id
    );

    config_manager
        .update(|cfg| {
            if let Some(client) = cfg.clients.iter_mut().find(|c| c.id == info.client_id) {
                let model_key = format!("{}__{}", info.server_name, info.tool_name);
                client
                    .model_permissions
                    .models
                    .insert(model_key.clone(), PermissionState::Off);
                tracing::info!("Set model permission to Off: {}", model_key);
            } else {
                tracing::warn!(
                    "Client {} not found for DenyAlways model update",
                    info.client_id
                );
            }
        })
        .map_err(|e: lr_types::AppError| e.to_string())?;

    // Save config to disk
    config_manager
        .save()
        .await
        .map_err(|e: lr_types::AppError| e.to_string())?;

    // Emit clients-changed event
    if let Err(e) = app.emit("clients-changed", ()) {
        tracing::error!("Failed to emit clients-changed event: {}", e);
    }

    Ok(())
}

/// Helper to update client permissions when DenyAlways is selected for MCP/skill tools
async fn update_permission_for_deny_permanent(
    app: &tauri::AppHandle,
    config_manager: &ConfigManager,
    info: &lr_mcp::gateway::firewall::PendingApprovalInfo,
) -> Result<(), String> {
    use lr_config::PermissionState;

    tracing::info!(
        "Updating permissions for DenyAlways: client={}, tool={}, server={}",
        info.client_id,
        info.tool_name,
        info.server_name
    );

    config_manager
        .update(|cfg| {
            if let Some(client) = cfg.clients.iter_mut().find(|c| c.id == info.client_id) {
                if info.tool_name.starts_with("skill_") {
                    let skill_name = &info.server_name;
                    let key = format!("{}__{}", skill_name, info.tool_name);
                    client
                        .skills_permissions
                        .tools
                        .insert(key, PermissionState::Off);
                    tracing::info!(
                        "Set skill tool permission to Off: skill={}, tool={}",
                        skill_name,
                        info.tool_name
                    );
                } else {
                    // MCP tool — info.server_name is the server UUID,
                    // info.tool_name is the namespaced name (slug__original_name).
                    // Permission key must be UUID__original_name to match resolve_tool() and UI.
                    let original_name =
                        info.tool_name.split("__").nth(1).unwrap_or(&info.tool_name);
                    let key = format!("{}__{}", info.server_name, original_name);
                    client
                        .mcp_permissions
                        .tools
                        .insert(key.clone(), PermissionState::Off);
                    tracing::info!("Set MCP tool permission to Off: {}", key);
                }
            } else {
                tracing::warn!("Client {} not found for DenyAlways update", info.client_id);
            }
        })
        .map_err(|e: lr_types::AppError| e.to_string())?;

    // Save config to disk
    config_manager
        .save()
        .await
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

/// Get full arguments for a pending firewall approval request (for edit mode)
#[tauri::command]
pub async fn get_firewall_full_arguments(
    request_id: String,
    state: State<'_, Arc<lr_server::state::AppState>>,
) -> Result<Option<String>, String> {
    let pending = state.mcp_gateway.firewall_manager.list_pending();
    Ok(pending
        .into_iter()
        .find(|p| p.request_id == request_id)
        .and_then(|p| p.full_arguments))
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

/// Clear child MCP permissions for a client
/// If server_id is provided, only clears children of that server (tools, resources, prompts)
/// If server_id is None, clears all children (servers, tools, resources, prompts)
#[tauri::command]
pub async fn clear_client_mcp_child_permissions(
    client_id: String,
    server_id: Option<String>,
    config_manager: State<'_, ConfigManager>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    tracing::info!(
        "Clearing MCP child permissions for client {}, server_id: {:?}",
        client_id,
        server_id
    );

    let mut found = false;
    config_manager
        .update(|cfg| {
            if let Some(client) = cfg.clients.iter_mut().find(|c| c.id == client_id) {
                if let Some(ref sid) = server_id {
                    // Only clear children of the specific server
                    let prefix = format!("{sid}__");
                    client
                        .mcp_permissions
                        .tools
                        .retain(|k, _| !k.starts_with(&prefix));
                    client
                        .mcp_permissions
                        .resources
                        .retain(|k, _| !k.starts_with(&prefix));
                    client
                        .mcp_permissions
                        .prompts
                        .retain(|k, _| !k.starts_with(&prefix));
                } else {
                    // Clear all children
                    client.mcp_permissions.servers.clear();
                    client.mcp_permissions.tools.clear();
                    client.mcp_permissions.resources.clear();
                    client.mcp_permissions.prompts.clear();
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

/// Clear child Skills permissions for a client
/// If skill_name is provided, only clears children of that skill (tools)
/// If skill_name is None, clears all children (skills, tools)
#[tauri::command]
pub async fn clear_client_skills_child_permissions(
    client_id: String,
    skill_name: Option<String>,
    config_manager: State<'_, ConfigManager>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    tracing::info!(
        "Clearing Skills child permissions for client {}, skill_name: {:?}",
        client_id,
        skill_name
    );

    let mut found = false;
    config_manager
        .update(|cfg| {
            if let Some(client) = cfg.clients.iter_mut().find(|c| c.id == client_id) {
                if let Some(ref sname) = skill_name {
                    // Only clear children of the specific skill
                    let prefix = format!("{sname}__");
                    client
                        .skills_permissions
                        .tools
                        .retain(|k, _| !k.starts_with(&prefix));
                } else {
                    // Clear all children
                    client.skills_permissions.skills.clear();
                    client.skills_permissions.tools.clear();
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

/// Clear child Model permissions for a client
/// If provider is provided, only clears children of that provider (models)
/// If provider is None, clears all children (providers, models)
#[tauri::command]
pub async fn clear_client_model_child_permissions(
    client_id: String,
    provider: Option<String>,
    config_manager: State<'_, ConfigManager>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    tracing::info!(
        "Clearing Model child permissions for client {}, provider: {:?}",
        client_id,
        provider
    );

    let mut found = false;
    config_manager
        .update(|cfg| {
            if let Some(client) = cfg.clients.iter_mut().find(|c| c.id == client_id) {
                if let Some(ref prov) = provider {
                    // Only clear children of the specific provider
                    let prefix = format!("{prov}__");
                    client
                        .model_permissions
                        .models
                        .retain(|k, _| !k.starts_with(&prefix));
                } else {
                    // Clear all children
                    client.model_permissions.providers.clear();
                    client.model_permissions.models.clear();
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

// ============================================================================
// Client Template & Mode Commands
// ============================================================================

/// Set the client mode (both, llm_only, mcp_only)
#[tauri::command]
pub async fn set_client_mode(
    client_id: String,
    mode: ClientMode,
    config_manager: State<'_, ConfigManager>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    tracing::info!("Setting client {} mode to: {:?}", client_id, mode);

    let mut found = false;
    config_manager
        .update(|cfg| {
            if let Some(client) = cfg.clients.iter_mut().find(|c| c.id == client_id) {
                client.client_mode = mode.clone();
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

/// Set the template ID for a client
#[tauri::command]
pub async fn set_client_template(
    client_id: String,
    template_id: Option<String>,
    config_manager: State<'_, ConfigManager>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    tracing::info!(
        "Setting client {} template_id to: {:?}",
        client_id,
        template_id
    );

    let mut found = false;
    config_manager
        .update(|cfg| {
            if let Some(client) = cfg.clients.iter_mut().find(|c| c.id == client_id) {
                client.template_id = template_id.clone();
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

// ============================================================================
// App Launcher Commands
// ============================================================================

/// App capabilities: installation status and supported modes
#[derive(Debug, Serialize)]
pub struct AppCapabilities {
    pub installed: bool,
    pub binary_path: Option<String>,
    pub version: Option<String>,
    pub supports_try_it_out: bool,
    pub supports_permanent_config: bool,
}

/// Result of a configure or launch operation
#[derive(Debug, Serialize)]
pub struct LaunchResult {
    pub success: bool,
    pub message: String,
    pub modified_files: Vec<String>,
    pub backup_files: Vec<String>,
    /// For CLI apps: the command the user should run in their terminal.
    /// When set, the app was NOT spawned — the user needs to run it themselves.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub terminal_command: Option<String>,
}

/// Get app capabilities for a given template (installation status + supported modes)
#[tauri::command]
pub async fn get_app_capabilities(template_id: String) -> Result<AppCapabilities, String> {
    use crate::launcher;

    match launcher::get_integration(&template_id) {
        Some(integration) => Ok(integration.check_installed()),
        None => Ok(AppCapabilities {
            installed: false,
            binary_path: None,
            version: None,
            supports_try_it_out: false,
            supports_permanent_config: false,
        }),
    }
}

/// Try it out: one-time terminal command, no permanent file changes
#[tauri::command]
pub async fn try_it_out_app(
    client_id: String,
    config_manager: State<'_, ConfigManager>,
    client_manager: State<'_, Arc<lr_clients::ClientManager>>,
) -> Result<LaunchResult, String> {
    use crate::launcher;

    let config = config_manager.get();
    let client = config
        .clients
        .iter()
        .find(|c| c.id == client_id)
        .ok_or_else(|| format!("Client not found: {}", client_id))?;

    let template_id = client
        .template_id
        .as_deref()
        .ok_or("Client has no template_id set")?;

    let integration = launcher::get_integration(template_id)
        .ok_or_else(|| format!("No integration found for template: {}", template_id))?;

    let base_url = format!("http://{}:{}", config.server.host, config.server.port);

    let client_secret = client_manager
        .get_secret(&client_id)
        .map_err(|e| format!("Failed to get client secret: {}", e))?
        .ok_or("Client secret not found in keychain")?;

    integration.try_it_out(&base_url, &client_secret, &client_id)
}

/// Permanently configure the app to route through LocalRouter (modifies config files)
#[tauri::command]
pub async fn configure_app_permanent(
    client_id: String,
    config_manager: State<'_, ConfigManager>,
    client_manager: State<'_, Arc<lr_clients::ClientManager>>,
) -> Result<LaunchResult, String> {
    use crate::launcher;

    let config = config_manager.get();
    let client = config
        .clients
        .iter()
        .find(|c| c.id == client_id)
        .ok_or_else(|| format!("Client not found: {}", client_id))?;

    let template_id = client
        .template_id
        .as_deref()
        .ok_or("Client has no template_id set")?;

    let integration = launcher::get_integration(template_id)
        .ok_or_else(|| format!("No integration found for template: {}", template_id))?;

    let base_url = format!("http://{}:{}", config.server.host, config.server.port);

    let client_secret = client_manager
        .get_secret(&client_id)
        .map_err(|e| format!("Failed to get client secret: {}", e))?
        .ok_or("Client secret not found in keychain")?;

    integration.configure_permanent(&base_url, &client_secret, &client_id)
}
