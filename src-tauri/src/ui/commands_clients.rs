//! Client and strategy-related Tauri command handlers
//!
//! Unified client management and routing strategy commands.

use std::sync::Arc;

use lr_config::{
    client_strategy_name, ClientMode, CodingAgentType, ConfigManager, McpPermissions,
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
    /// Per-client context management override (None = inherit global, Some(false) = disabled)
    pub context_management_enabled: Option<bool>,
    /// Per-client catalog compression override (None = inherit global)
    pub catalog_compression_enabled: Option<bool>,
    pub created_at: String,
    pub last_used: Option<String>,
    /// Unified MCP permissions (hierarchical Allow/Ask/Off)
    pub mcp_permissions: McpPermissions,
    /// Unified Skills permissions (hierarchical Allow/Ask/Off)
    pub skills_permissions: SkillsPermissions,
    /// Coding agent permission (Allow/Ask/Off)
    pub coding_agent_permission: PermissionState,
    /// Which coding agent type this client uses
    pub coding_agent_type: Option<CodingAgentType>,
    /// Unified Model permissions (hierarchical Allow/Ask/Off)
    pub model_permissions: ModelPermissions,
    /// Marketplace permission state
    pub marketplace_permission: PermissionState,
    /// Sampling permission (Allow/Ask/Off)
    pub mcp_sampling_permission: PermissionState,
    /// Elicitation permission (Allow/Ask/Off)
    pub mcp_elicitation_permission: PermissionState,
    /// Client mode (both, llm_only, mcp_only)
    pub client_mode: ClientMode,
    /// Template ID used to create this client
    pub template_id: Option<String>,
    /// Whether auto-sync of external app config is enabled
    pub sync_config: bool,
    /// Whether guardrails are active (has non-allow category_actions)
    pub guardrails_active: bool,
    /// Whether JSON repair is active (resolved from per-client override or global)
    pub json_repair_active: bool,
}

/// List all clients
#[tauri::command]
pub async fn list_clients(
    client_manager: State<'_, Arc<lr_clients::ClientManager>>,
    config_manager: State<'_, ConfigManager>,
) -> Result<Vec<ClientInfo>, String> {
    let config = config_manager.get();
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
            context_management_enabled: c.context_management_enabled,
            catalog_compression_enabled: c.catalog_compression_enabled,
            created_at: c.created_at.to_rfc3339(),
            last_used: c.last_used.map(|t| t.to_rfc3339()),
            mcp_permissions: c.mcp_permissions.clone(),
            skills_permissions: c.skills_permissions.clone(),
            coding_agent_permission: c.coding_agent_permission.clone(),
            coding_agent_type: c.coding_agent_type,
            model_permissions: c.model_permissions.clone(),
            marketplace_permission: c.marketplace_permission.clone(),
            mcp_sampling_permission: c.mcp_sampling_permission.clone(),
            mcp_elicitation_permission: c.mcp_elicitation_permission.clone(),
            client_mode: c.client_mode.clone(),
            template_id: c.template_id.clone(),
            sync_config: c.sync_config,
            guardrails_active: {
                let effective_actions = c
                    .guardrails
                    .category_actions
                    .as_deref()
                    .unwrap_or(&config.guardrails.category_actions);
                effective_actions.iter().any(|a| a.action != "allow")
            },
            json_repair_active: c.json_repair.enabled.unwrap_or(config.json_repair.enabled),
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
        context_management_enabled: client.context_management_enabled,
        catalog_compression_enabled: client.catalog_compression_enabled,
        created_at: client.created_at.to_rfc3339(),
        last_used: client.last_used.map(|t| t.to_rfc3339()),
        mcp_permissions: client.mcp_permissions.clone(),
        skills_permissions: client.skills_permissions.clone(),
        coding_agent_permission: client.coding_agent_permission.clone(),
        coding_agent_type: client.coding_agent_type,
        model_permissions: client.model_permissions.clone(),
        marketplace_permission: client.marketplace_permission.clone(),
        mcp_sampling_permission: client.mcp_sampling_permission.clone(),
        mcp_elicitation_permission: client.mcp_elicitation_permission.clone(),
        client_mode: client.client_mode.clone(),
        template_id: client.template_id.clone(),
        sync_config: client.sync_config,
        guardrails_active: {
            let cfg = config_manager.get();
            let effective_actions = client
                .guardrails
                .category_actions
                .as_deref()
                .unwrap_or(&cfg.guardrails.category_actions);
            effective_actions.iter().any(|a| a.action != "allow")
        },
        json_repair_active: {
            let cfg = config_manager.get();
            client
                .json_repair
                .enabled
                .unwrap_or(cfg.json_repair.enabled)
        },
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

/// Generate a clone name with dedup suffix
/// "Clone of Foo" -> if exists -> "Clone of Foo (2)" -> "Clone of Foo (3)" etc.
fn generate_clone_name(original_name: &str, existing_names: &[&str]) -> String {
    let base = format!("Clone of {}", original_name);
    if !existing_names.contains(&base.as_str()) {
        return base;
    }
    let mut n = 2;
    loop {
        let candidate = format!("{} ({})", base, n);
        if !existing_names.contains(&candidate.as_str()) {
            return candidate;
        }
        n += 1;
    }
}

/// Clone an existing client (deep copy with new identity)
///
/// Creates a new client by cloning all settings from an existing client.
/// The clone gets:
/// - A new UUID
/// - A new name ("Clone of {name}", with "(N)" suffix if duplicates exist)
/// - Its own secret in the keychain (independent from source)
/// - Its own strategy (deep-cloned from source)
/// - sync_config = false (to avoid conflicts with source)
/// - Fresh created_at timestamp, last_used = None
#[tauri::command]
pub async fn clone_client(
    client_id: String,
    client_manager: State<'_, Arc<lr_clients::ClientManager>>,
    config_manager: State<'_, ConfigManager>,
    app: tauri::AppHandle,
) -> Result<(String, ClientInfo), String> {
    tracing::info!("Cloning client: {}", client_id);

    let config = config_manager.get();

    // Find the source client
    let source_client = config
        .clients
        .iter()
        .find(|c| c.id == client_id)
        .ok_or_else(|| format!("Client not found: {}", client_id))?
        .clone();

    // Find the source strategy
    let source_strategy = config
        .strategies
        .iter()
        .find(|s| s.id == source_client.strategy_id)
        .cloned();

    // Generate clone name with dedup
    let existing_names: Vec<&str> = config.clients.iter().map(|c| c.name.as_str()).collect();
    let clone_name = generate_clone_name(&source_client.name, &existing_names);

    // Create new IDs
    let new_client_id = uuid::Uuid::new_v4().to_string();
    let new_strategy_id = uuid::Uuid::new_v4().to_string();

    // Clone the strategy
    let new_strategy = if let Some(src_strategy) = source_strategy {
        lr_config::Strategy {
            id: new_strategy_id.clone(),
            name: lr_config::client_strategy_name(&clone_name),
            parent: Some(new_client_id.clone()),
            allowed_models: src_strategy.allowed_models.clone(),
            auto_config: src_strategy.auto_config.clone(),
            rate_limits: src_strategy.rate_limits.clone(),
            free_tier_only: src_strategy.free_tier_only,
            free_tier_fallback: src_strategy.free_tier_fallback.clone(),
        }
    } else {
        lr_config::Strategy::new_for_client(new_client_id.clone(), clone_name.clone())
    };

    // Clone the client with modifications
    let new_client = lr_config::Client {
        id: new_client_id.clone(),
        name: clone_name,
        enabled: source_client.enabled,
        strategy_id: new_strategy_id,
        allowed_llm_providers: Vec::new(),
        mcp_server_access: lr_config::McpServerAccess::None,
        context_management_enabled: source_client.context_management_enabled,
        catalog_compression_enabled: source_client.catalog_compression_enabled,
        client_tools_indexing: source_client.client_tools_indexing.clone(),
        skills_access: lr_config::SkillsAccess::None,
        created_at: chrono::Utc::now(),
        last_used: None,
        roots: source_client.roots.clone(),
        mcp_sampling_permission: source_client.mcp_sampling_permission.clone(),
        mcp_elicitation_permission: source_client.mcp_elicitation_permission.clone(),
        mcp_sampling_max_tokens: source_client.mcp_sampling_max_tokens,
        mcp_sampling_rate_limit: source_client.mcp_sampling_rate_limit,
        mcp_sampling_enabled: false,
        mcp_sampling_requires_approval: true,
        firewall: source_client.firewall.clone(),
        marketplace_enabled: false,
        mcp_permissions: source_client.mcp_permissions.clone(),
        skills_permissions: source_client.skills_permissions.clone(),
        model_permissions: source_client.model_permissions.clone(),
        marketplace_permission: source_client.marketplace_permission.clone(),
        coding_agents_permissions: lr_config::CodingAgentsPermissions::default(),
        coding_agent_permission: source_client.coding_agent_permission.clone(),
        coding_agent_type: source_client.coding_agent_type,
        client_mode: source_client.client_mode.clone(),
        template_id: source_client.template_id.clone(),
        sync_config: false, // Disabled for clones to avoid conflicts
        guardrails_enabled: None,
        guardrails: source_client.guardrails.clone(),
        prompt_compression: source_client.prompt_compression.clone(),
        json_repair: source_client.json_repair.clone(),
        secret_scanning: source_client.secret_scanning.clone(),
        memory_enabled: source_client.memory_enabled,
    };

    // Add to config
    config_manager
        .update(|cfg| {
            cfg.clients.push(new_client.clone());
            cfg.strategies.push(new_strategy);
        })
        .map_err(|e| e.to_string())?;

    // Store new secret in keychain
    let secret = client_manager
        .add_client_with_secret(new_client.clone())
        .map_err(|e| e.to_string())?;

    // Persist to disk
    config_manager.save().await.map_err(|e| e.to_string())?;

    // Rebuild tray menu
    if let Err(e) = crate::ui::tray::rebuild_tray_menu(&app) {
        tracing::error!("Failed to rebuild tray menu: {}", e);
    }

    // Emit events
    if let Err(e) = app.emit("clients-changed", ()) {
        tracing::error!("Failed to emit clients-changed event: {}", e);
    }
    if let Err(e) = app.emit("strategies-changed", ()) {
        tracing::error!("Failed to emit strategies-changed event: {}", e);
    }

    let client_info = ClientInfo {
        id: new_client.id.clone(),
        name: new_client.name.clone(),
        client_id: new_client.id.clone(),
        enabled: new_client.enabled,
        strategy_id: new_client.strategy_id.clone(),
        context_management_enabled: new_client.context_management_enabled,
        catalog_compression_enabled: new_client.catalog_compression_enabled,
        created_at: new_client.created_at.to_rfc3339(),
        last_used: None,
        mcp_permissions: new_client.mcp_permissions.clone(),
        skills_permissions: new_client.skills_permissions.clone(),
        coding_agent_permission: new_client.coding_agent_permission.clone(),
        coding_agent_type: new_client.coding_agent_type,
        model_permissions: new_client.model_permissions.clone(),
        marketplace_permission: new_client.marketplace_permission.clone(),
        mcp_sampling_permission: new_client.mcp_sampling_permission.clone(),
        mcp_elicitation_permission: new_client.mcp_elicitation_permission.clone(),
        client_mode: new_client.client_mode.clone(),
        template_id: new_client.template_id.clone(),
        sync_config: false,
        guardrails_active: {
            let cfg = config_manager.get();
            let effective_actions = new_client
                .guardrails
                .category_actions
                .as_deref()
                .unwrap_or(&cfg.guardrails.category_actions);
            effective_actions.iter().any(|a| a.action != "allow")
        },
        json_repair_active: {
            let cfg = config_manager.get();
            new_client
                .json_repair
                .enabled
                .unwrap_or(cfg.json_repair.enabled)
        },
    };

    Ok((secret, client_info))
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
/// If the client has sync_config enabled, the external config is updated.
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
    config_manager: State<'_, ConfigManager>,
    provider_registry: State<'_, Arc<lr_providers::registry::ProviderRegistry>>,
) -> Result<String, String> {
    tracing::info!("Rotating secret for client: {}", client_id);

    let new_secret = client_manager
        .rotate_secret(&client_id)
        .map_err(|e| e.to_string())?;

    // If sync_config is enabled, update external config with new secret
    let config = config_manager.get();
    if let Some(client) = config.clients.iter().find(|c| c.id == client_id) {
        if client.sync_config && client.template_id.is_some() {
            let cm = config_manager.inner().clone();
            let cmgr = client_manager.inner().clone();
            let pr = provider_registry.inner().clone();
            let cid = client_id.clone();
            tokio::spawn(async move {
                if let Err(e) = sync_client_config_inner(&cid, &cm, &cmgr, &pr).await {
                    tracing::warn!(
                        "Failed to sync config after secret rotation for {}: {}",
                        cid,
                        e
                    );
                }
            });
        }
    }

    Ok(new_secret)
}

/// Toggle context management for a specific client.
///
/// # Arguments
/// * `client_id` - Client ID
/// * `enabled` - None = inherit global setting, Some(false) = disabled for this client
#[tauri::command]
pub async fn toggle_client_context_management(
    client_id: String,
    enabled: Option<bool>,
    client_manager: State<'_, Arc<lr_clients::ClientManager>>,
    config_manager: State<'_, ConfigManager>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    tracing::info!(
        "Setting client {} context management: {:?}",
        client_id,
        enabled
    );

    // Update in client manager (in-memory)
    client_manager
        .set_context_management_enabled(&client_id, enabled)
        .map_err(|e| e.to_string())?;

    // Update in config (for persistence)
    config_manager
        .update(|cfg| {
            if let Some(client) = cfg.clients.iter_mut().find(|c| c.id == client_id) {
                client.context_management_enabled = enabled;
            }
        })
        .map_err(|e| e.to_string())?;

    config_manager.save().await.map_err(|e| e.to_string())?;

    if let Err(e) = app.emit("clients-changed", ()) {
        tracing::error!("Failed to emit clients-changed event: {}", e);
    }

    Ok(())
}

/// Toggle catalog compression for a specific client.
///
/// # Arguments
/// * `client_id` - Client ID
/// * `enabled` - None = inherit global setting, Some(false) = disabled for this client
#[tauri::command]
pub async fn toggle_client_catalog_compression(
    client_id: String,
    enabled: Option<bool>,
    client_manager: State<'_, Arc<lr_clients::ClientManager>>,
    config_manager: State<'_, ConfigManager>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    tracing::info!(
        "Setting client {} catalog compression: {:?}",
        client_id,
        enabled
    );

    // Update in client manager (in-memory)
    client_manager
        .set_catalog_compression_enabled(&client_id, enabled)
        .map_err(|e| e.to_string())?;

    // Update in config (for persistence)
    config_manager
        .update(|cfg| {
            if let Some(client) = cfg.clients.iter_mut().find(|c| c.id == client_id) {
                client.catalog_compression_enabled = enabled;
            }
        })
        .map_err(|e| e.to_string())?;

    config_manager.save().await.map_err(|e| e.to_string())?;

    if let Err(e) = app.emit("clients-changed", ()) {
        tracing::error!("Failed to emit clients-changed event: {}", e);
    }

    Ok(())
}

/// Get client tools indexing permissions for a client.
#[tauri::command]
pub async fn get_client_tools_indexing(
    client_id: String,
    config_manager: State<'_, ConfigManager>,
) -> Result<Option<lr_config::ClientToolsIndexingPermissions>, String> {
    let config = config_manager.get();
    let client = config
        .clients
        .iter()
        .find(|c| c.id == client_id)
        .ok_or_else(|| format!("Client not found: {}", client_id))?;
    Ok(client.client_tools_indexing.clone())
}

/// Set client tools indexing permission at global or tool level.
/// Pass state=None to clear an override (revert to inherit).
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn set_client_tools_indexing(
    client_id: String,
    level: String,
    key: Option<String>,
    state: Option<String>,
    config_manager: State<'_, ConfigManager>,
    context_mode_vs: State<'_, Arc<lr_mcp::gateway::context_mode::ContextModeVirtualServer>>,
    mcp_via_llm_manager: State<'_, Arc<lr_mcp_via_llm::McpViaLlmManager>>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    config_manager
        .update(|cfg| {
            if let Some(client) = cfg.clients.iter_mut().find(|c| c.id == client_id) {
                let perms = client
                    .client_tools_indexing
                    .get_or_insert_with(Default::default);

                match level.as_str() {
                    "global" => {
                        perms.global = state.as_ref().map(|s| match s.as_str() {
                            "disable" => lr_config::IndexingState::Disable,
                            _ => lr_config::IndexingState::Enable,
                        });
                    }
                    "global_clear" => {
                        perms.global = None;
                    }
                    "tool" => {
                        if let Some(ref k) = key {
                            if let Some(ref s) = state {
                                let indexing_state = match s.as_str() {
                                    "disable" => lr_config::IndexingState::Disable,
                                    _ => lr_config::IndexingState::Enable,
                                };
                                perms.tools.insert(k.clone(), indexing_state);
                            }
                        }
                    }
                    "tool_clear" => {
                        if let Some(ref k) = key {
                            perms.tools.remove(k);
                        }
                    }
                    "clear_all" => {
                        client.client_tools_indexing = None;
                    }
                    _ => {}
                }

                // Clean up: if perms are now empty, set to None
                if let Some(ref p) = client.client_tools_indexing {
                    if p.global.is_none() && p.tools.is_empty() {
                        client.client_tools_indexing = None;
                    }
                }
            }
        })
        .map_err(|e| e.to_string())?;

    // Propagate updated config to runtime state managers
    let new_config = config_manager.get().context_management.clone();
    context_mode_vs.update_config(new_config.clone());
    mcp_via_llm_manager.update_context_management_config(new_config);

    config_manager.save().await.map_err(|e| e.to_string())?;

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

/// Effective configuration for a client (with inheritance resolved)
#[derive(Debug, Clone, Serialize)]
pub struct ClientEffectiveConfig {
    pub strategy_name: String,
    /// Effective context management (resolved from client override or global)
    pub context_management_effective: bool,
    /// "client" if overridden, "global" if inherited
    pub context_management_source: String,
    /// Effective catalog compression (resolved from client override or global)
    pub catalog_compression_effective: bool,
    /// "client" if overridden, "global" if inherited
    pub catalog_compression_source: String,
    /// Effective JSON repair (resolved from client override or global)
    pub json_repair_effective: bool,
    /// "client" if overridden, "global" if inherited
    pub json_repair_source: String,
}

/// Get the effective (inheritance-resolved) configuration for a client.
#[tauri::command]
pub async fn get_client_effective_config(
    client_id: String,
    config_manager: State<'_, ConfigManager>,
) -> Result<ClientEffectiveConfig, String> {
    let config = config_manager.get();

    let client = config
        .clients
        .iter()
        .find(|c| c.id == client_id)
        .ok_or_else(|| format!("Client not found: {}", client_id))?;

    let strategy_name = config
        .strategies
        .iter()
        .find(|s| s.id == client.strategy_id)
        .map(|s| s.name.clone())
        .unwrap_or_else(|| "Unknown".to_string());

    let ctx = &config.context_management;

    Ok(ClientEffectiveConfig {
        strategy_name,
        context_management_effective: client.is_context_management_enabled(ctx),
        context_management_source: if client.context_management_enabled.is_some() {
            "client".to_string()
        } else {
            "global".to_string()
        },
        catalog_compression_effective: client.is_catalog_compression_enabled(ctx),
        catalog_compression_source: if client.catalog_compression_enabled.is_some() {
            "client".to_string()
        } else {
            "global".to_string()
        },
        json_repair_effective: client
            .json_repair
            .enabled
            .unwrap_or(config.json_repair.enabled),
        json_repair_source: if client.json_repair.enabled.is_some() {
            "client".to_string()
        } else {
            "global".to_string()
        },
    })
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
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn update_strategy(
    strategy_id: String,
    name: Option<String>,
    allowed_models: Option<lr_config::AvailableModelsSelection>,
    auto_config: Option<lr_config::AutoModelConfig>,
    rate_limits: Option<Vec<lr_config::StrategyRateLimit>>,
    free_tier_only: Option<bool>,
    free_tier_fallback: Option<lr_config::FreeTierFallback>,
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
                if let Some(free_tier) = free_tier_only {
                    strategy.free_tier_only = free_tier;
                }
                if let Some(fallback) = free_tier_fallback {
                    strategy.free_tier_fallback = fallback;
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

// ============================================================================
// Feature Client Status Commands
// ============================================================================

/// Status of one client for a specific optimize feature
#[derive(Debug, Clone, Serialize)]
pub struct ClientFeatureStatus {
    pub client_id: String,
    pub client_name: String,
    /// Whether the feature is effectively active for this client
    pub active: bool,
    /// "override" if per-client setting exists, "global" if inherited
    pub source: String,
    /// Feature-specific effective value (e.g. "ask", "notify", "off" for secret_scanning)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effective_value: Option<String>,
}

/// Get effective feature status for all clients
#[tauri::command]
pub async fn get_feature_clients_status(
    feature: String,
    client_manager: State<'_, Arc<lr_clients::ClientManager>>,
    config_manager: State<'_, ConfigManager>,
) -> Result<Vec<ClientFeatureStatus>, String> {
    let config = config_manager.get();
    let clients = client_manager.list_clients();

    Ok(clients
        .into_iter()
        .filter(|c| !c.name.starts_with("_test_strategy_"))
        .map(|c| {
            let (active, source, effective_value) = match feature.as_str() {
                "json_repair" => (
                    c.json_repair.enabled.unwrap_or(config.json_repair.enabled),
                    if c.json_repair.enabled.is_some() {
                        "override"
                    } else {
                        "global"
                    },
                    None,
                ),
                "prompt_compression" => (
                    c.prompt_compression
                        .enabled
                        .unwrap_or(config.prompt_compression.enabled),
                    if c.prompt_compression.enabled.is_some() {
                        "override"
                    } else {
                        "global"
                    },
                    None,
                ),
                "guardrails" => {
                    let effective_actions = c
                        .guardrails
                        .category_actions
                        .as_deref()
                        .unwrap_or(&config.guardrails.category_actions);
                    let active_count = effective_actions
                        .iter()
                        .filter(|a| a.action != "allow")
                        .count();
                    (
                        active_count > 0,
                        if c.guardrails.category_actions.is_some() {
                            "override"
                        } else {
                            "global"
                        },
                        Some(format!("{} active", active_count)),
                    )
                }
                "secret_scanning" => {
                    let effective_action = c
                        .secret_scanning
                        .action
                        .as_ref()
                        .unwrap_or(&config.secret_scanning.action);
                    let value = match effective_action {
                        lr_config::SecretScanAction::Ask => "ask",
                        lr_config::SecretScanAction::Notify => "notify",
                        lr_config::SecretScanAction::Off => "off",
                    };
                    (
                        *effective_action != lr_config::SecretScanAction::Off,
                        if c.secret_scanning.action.is_some() {
                            "override"
                        } else {
                            "global"
                        },
                        Some(value.to_string()),
                    )
                }
                "catalog_compression" => (
                    c.is_catalog_compression_enabled(&config.context_management),
                    if c.catalog_compression_enabled.is_some() {
                        "override"
                    } else {
                        "global"
                    },
                    None,
                ),
                "context_management" => (
                    c.is_context_management_enabled(&config.context_management),
                    if c.context_management_enabled.is_some() {
                        "override"
                    } else {
                        "global"
                    },
                    None,
                ),
                "memory" => (
                    c.memory_enabled.unwrap_or(false),
                    if c.memory_enabled.is_some() {
                        "override"
                    } else {
                        "global"
                    },
                    None,
                ),
                "coding_agents" => {
                    let value = match c.coding_agent_permission {
                        lr_config::PermissionState::Allow => "allow",
                        lr_config::PermissionState::Ask => "ask",
                        lr_config::PermissionState::Off => "off",
                    };
                    (
                        c.coding_agent_permission != lr_config::PermissionState::Off,
                        "global", // coding agent permission is always per-client (no global default to inherit)
                        Some(value.to_string()),
                    )
                }
                "strong_weak" => {
                    let strategy = config.strategies.iter().find(|s| s.id == c.strategy_id);
                    let active = strategy
                        .and_then(|s| s.auto_config.as_ref())
                        .and_then(|ac| ac.routellm_config.as_ref())
                        .map(|rc| rc.enabled)
                        .unwrap_or(false);
                    (active, "global", None)
                }
                _ => (false, "global", None),
            };

            ClientFeatureStatus {
                client_id: c.id.clone(),
                client_name: c.name.clone(),
                active,
                source: source.to_string(),
                effective_value,
            }
        })
        .collect())
}

// ============================================================================
// Firewall Approval Commands
// ============================================================================

/// Submit a response to a pending firewall approval request
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn submit_firewall_approval(
    app: tauri::AppHandle,
    request_id: String,
    action: lr_mcp::gateway::firewall::FirewallApprovalAction,
    edited_arguments: Option<String>,
    state: State<'_, Arc<lr_server::state::AppState>>,
    config_manager: State<'_, ConfigManager>,
    client_manager: State<'_, Arc<lr_clients::ClientManager>>,
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

    // If a persistent action, get the pending session info before submitting
    // so we can update client permissions or add time-based approval/denial
    let pending_info = if matches!(
        action,
        FirewallApprovalAction::AllowPermanent
            | FirewallApprovalAction::Allow1Minute
            | FirewallApprovalAction::Allow1Hour
            | FirewallApprovalAction::DenyAlways
            | FirewallApprovalAction::BlockCategories
            | FirewallApprovalAction::AllowCategories
            | FirewallApprovalAction::Deny1Hour
            | FirewallApprovalAction::AllowSession
            | FirewallApprovalAction::DenySession
            | FirewallApprovalAction::DisableClient
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

    // Compute the submit action (some actions are remapped for the gateway)
    // BlockCategories and Deny1Hour are handled locally → submit as Deny
    // AllowCategories is handled locally → submit as AllowOnce
    let submit_action = match &action {
        FirewallApprovalAction::BlockCategories => FirewallApprovalAction::Deny,
        FirewallApprovalAction::Deny1Hour => FirewallApprovalAction::Deny,
        FirewallApprovalAction::DisableClient => FirewallApprovalAction::Deny,
        FirewallApprovalAction::AllowCategories => FirewallApprovalAction::AllowOnce,
        other => other.clone(),
    };

    // Phase 1: Update config/trackers BEFORE submitting response to the gateway.
    // This ensures that by the time the gateway unblocks and the client sends the
    // next request, the ClientManager already has the updated permissions
    // (config_manager.update() synchronously calls sync_clients()).
    // Wrapped in an async block so that errors don't prevent submit_response (Phase 2)
    // from being called — the current request must always get a response.
    let phase1_err = async {
    match action {
        FirewallApprovalAction::AllowPermanent => {
            if let Some(ref info) = pending_info {
                if info.is_guardrail_request {
                    // Clear all category actions for this client permanently (disables guardrails)
                    config_manager
                        .update(|cfg| {
                            if let Some(client) =
                                cfg.clients.iter_mut().find(|c| c.id == info.client_id)
                            {
                                client.guardrails.category_actions = None;
                            }
                        })
                        .map_err(|e| e.to_string())?;
                    config_manager.save().await.map_err(|e| e.to_string())?;
                    tracing::info!(
                        "Cleared guardrails category actions permanently for client {}",
                        info.client_id
                    );
                } else if info.is_auto_router_request {
                    // Set auto_config.permission to Allow permanently
                    update_auto_router_permission(
                        &app,
                        &config_manager,
                        &info.client_id,
                        lr_config::PermissionState::Allow,
                    )
                    .await?;
                } else if info.is_free_tier_fallback {
                    // Set free_tier_fallback to Allow permanently
                    update_free_tier_fallback_config(
                        &app,
                        &config_manager,
                        &info.client_id,
                        lr_config::FreeTierFallback::Allow,
                    )
                    .await?;
                } else if info.is_model_request {
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
        FirewallApprovalAction::Allow1Minute => {
            if let Some(ref info) = pending_info {
                if info.is_guardrail_request {
                    state
                        .guardrail_approval_tracker
                        .add_1_minute_bypass(&info.client_id);
                    tracing::info!(
                        "Added 1-minute guardrail bypass for client {}",
                        info.client_id
                    );
                } else if info.is_auto_router_request {
                    state
                        .auto_router_approval_tracker
                        .add_1_minute_approval(&info.client_id);
                    tracing::info!(
                        "Added 1-minute auto-router approval for client {}",
                        info.client_id
                    );
                } else if info.is_free_tier_fallback {
                    state
                        .free_tier_approval_tracker
                        .add_1_minute_approval(&info.client_id);
                    tracing::info!(
                        "Added 1-minute free-tier fallback approval for client {}",
                        info.client_id
                    );
                } else if info.is_model_request {
                    state.model_approval_tracker.add_1_minute_approval(
                        &info.client_id,
                        &info.server_name,
                        &info.tool_name,
                    );
                    tracing::info!(
                        "Added 1-minute model approval for client {} model {}__{}",
                        info.client_id,
                        info.server_name,
                        info.tool_name
                    );
                }
            } else {
                tracing::warn!(
                    "Allow1Minute requested but couldn't find pending info for request {}",
                    request_id
                );
            }
        }
        FirewallApprovalAction::Allow1Hour => {
            if let Some(ref info) = pending_info {
                if info.is_guardrail_request {
                    // Add time-based guardrail bypass (1 hour)
                    state
                        .guardrail_approval_tracker
                        .add_1_hour_bypass(&info.client_id);
                    tracing::info!(
                        "Added 1-hour guardrail bypass for client {}",
                        info.client_id
                    );
                } else if info.is_auto_router_request {
                    // Add time-based auto-router approval (1 hour)
                    state
                        .auto_router_approval_tracker
                        .add_1_hour_approval(&info.client_id);
                    tracing::info!(
                        "Added 1-hour auto-router approval for client {}",
                        info.client_id
                    );
                } else if info.is_free_tier_fallback {
                    // Add time-based free-tier fallback approval (1 hour)
                    state
                        .free_tier_approval_tracker
                        .add_1_hour_approval(&info.client_id);
                    tracing::info!(
                        "Added 1-hour free-tier fallback approval for client {}",
                        info.client_id
                    );
                } else if info.is_model_request {
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
                } else if info.is_secret_scan_request {
                    // Add time-based secret scan bypass (1 hour)
                    state
                        .secret_scan_approval_tracker
                        .add_1_hour_bypass(&info.client_id);
                    tracing::info!(
                        "Added 1-hour secret scan bypass for client {}",
                        info.client_id
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
                if info.is_guardrail_request {
                    // Disable guardrails for this client (clear category actions)
                    config_manager
                        .update(|cfg| {
                            if let Some(client) =
                                cfg.clients.iter_mut().find(|c| c.id == info.client_id)
                            {
                                client.guardrails.category_actions = None;
                            }
                        })
                        .map_err(|e| e.to_string())?;
                    config_manager.save().await.map_err(|e| e.to_string())?;
                    tracing::info!(
                        "Cleared guardrails category actions for client {} (DenyAlways → disable guardrails for client)",
                        info.client_id
                    );
                } else if info.is_auto_router_request {
                    // Set auto_config.permission to Off permanently
                    update_auto_router_permission(
                        &app,
                        &config_manager,
                        &info.client_id,
                        lr_config::PermissionState::Off,
                    )
                    .await?;
                } else if info.is_free_tier_fallback {
                    // Set free_tier_fallback to Off permanently
                    update_free_tier_fallback_config(
                        &app,
                        &config_manager,
                        &info.client_id,
                        lr_config::FreeTierFallback::Off,
                    )
                    .await?;
                } else if info.is_secret_scan_request {
                    // Disable secret scanning for this client
                    config_manager
                        .update(|cfg| {
                            if let Some(client) =
                                cfg.clients.iter_mut().find(|c| c.id == info.client_id)
                            {
                                client.secret_scanning.action =
                                    Some(lr_config::SecretScanAction::Off);
                            }
                        })
                        .map_err(|e| e.to_string())?;
                    config_manager.save().await.map_err(|e| e.to_string())?;
                    tracing::info!(
                        "Disabled secret scanning for client {} (DenyAlways → set action to Off)",
                        info.client_id
                    );
                } else if info.is_model_request {
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
        FirewallApprovalAction::BlockCategories => {
            if let Some(ref info) = pending_info {
                if info.is_guardrail_request {
                    // Extract flagged categories from the guardrail details
                    if let Some(ref details) = info.guardrail_details {
                        let flagged_categories: Vec<String> = details
                            .actions_required
                            .iter()
                            .filter_map(|a| {
                                a.get("category")
                                    .and_then(|c| c.as_str())
                                    .map(|s| s.to_string())
                            })
                            .collect();

                        config_manager
                            .update(|cfg| {
                                if let Some(client) =
                                    cfg.clients.iter_mut().find(|c| c.id == info.client_id)
                                {
                                    for category in &flagged_categories {
                                        // Update or add the category action to "block"
                                        let actions = client
                                            .guardrails
                                            .category_actions
                                            .get_or_insert_with(Vec::new);
                                        if let Some(existing) =
                                            actions.iter_mut().find(|a| a.category == *category)
                                        {
                                            existing.action = "block".to_string();
                                        } else {
                                            actions.push(lr_config::CategoryActionEntry {
                                                category: category.clone(),
                                                action: "block".to_string(),
                                            });
                                        }
                                    }
                                }
                            })
                            .map_err(|e| e.to_string())?;
                        config_manager.save().await.map_err(|e| e.to_string())?;
                        tracing::info!(
                            "Set flagged categories to 'block' for client {}",
                            info.client_id
                        );
                    }
                }
            }
        }
        FirewallApprovalAction::AllowCategories => {
            if let Some(ref info) = pending_info {
                if info.is_guardrail_request {
                    // Extract flagged categories and set them to "allow"
                    if let Some(ref details) = info.guardrail_details {
                        let flagged_categories: Vec<String> = details
                            .actions_required
                            .iter()
                            .filter_map(|a| {
                                a.get("category")
                                    .and_then(|c| c.as_str())
                                    .map(|s| s.to_string())
                            })
                            .collect();

                        config_manager
                            .update(|cfg| {
                                if let Some(client) =
                                    cfg.clients.iter_mut().find(|c| c.id == info.client_id)
                                {
                                    for category in &flagged_categories {
                                        // Update or add the category action to "allow"
                                        let actions = client
                                            .guardrails
                                            .category_actions
                                            .get_or_insert_with(Vec::new);
                                        if let Some(existing) =
                                            actions.iter_mut().find(|a| a.category == *category)
                                        {
                                            existing.action = "allow".to_string();
                                        } else {
                                            actions.push(lr_config::CategoryActionEntry {
                                                category: category.clone(),
                                                action: "allow".to_string(),
                                            });
                                        }
                                    }
                                }
                            })
                            .map_err(|e| e.to_string())?;
                        config_manager.save().await.map_err(|e| e.to_string())?;
                        tracing::info!(
                            "Set flagged categories to 'allow' for client {}",
                            info.client_id
                        );
                    }
                }
            }
        }
        FirewallApprovalAction::Deny1Hour => {
            if let Some(ref info) = pending_info {
                if info.is_guardrail_request {
                    // Add time-based guardrail denial (1 hour auto-deny)
                    state
                        .guardrail_denial_tracker
                        .add_1_hour_denial(&info.client_id);
                    tracing::info!(
                        "Added 1-hour guardrail denial for client {}",
                        info.client_id
                    );
                }
            }
        }
        FirewallApprovalAction::DisableClient => {
            if let Some(ref info) = pending_info {
                // Disable the client entirely
                client_manager
                    .disable_client(&info.client_id)
                    .map_err(|e| e.to_string())?;
                config_manager
                    .update(|cfg| {
                        if let Some(client) =
                            cfg.clients.iter_mut().find(|c| c.id == info.client_id)
                        {
                            client.enabled = false;
                        }
                    })
                    .map_err(|e| e.to_string())?;
                config_manager.save().await.map_err(|e| e.to_string())?;
                tracing::info!(
                    "Disabled client {} via guardrail DisableClient action",
                    info.client_id
                );
            }
        }
        _ => {}
    }
    Ok::<(), String>(())
    }.await.err();

    if let Some(ref e) = phase1_err {
        tracing::error!(
            "Pre-submit config/tracker update failed: {} — proceeding with submit anyway",
            e
        );
    }

    // Phase 2: Submit the response to the firewall manager (unblocks the gateway).
    // Config/trackers are already updated, so the next request will see the new permissions.
    state
        .mcp_gateway
        .firewall_manager
        .submit_response(&request_id, submit_action, edited_args_value)
        .map_err(|e| e.to_string())?;

    // Phase 3: Re-evaluate remaining pending approvals that might now be auto-resolvable
    reevaluate_pending_approvals(
        &app,
        &state.mcp_gateway.firewall_manager,
        &config_manager,
        &state.model_approval_tracker,
        &state.guardrail_approval_tracker,
        &state.guardrail_denial_tracker,
        &state.free_tier_approval_tracker,
        &state.auto_router_approval_tracker,
    );

    // Rebuild tray menu to remove the pending approval item
    if let Err(e) = crate::ui::tray::rebuild_tray_menu(&app) {
        tracing::warn!("Failed to rebuild tray menu after firewall approval: {}", e);
    }

    // Trigger immediate tray icon update to remove the question mark overlay
    tray_graph_manager.notify_activity();

    Ok(())
}

/// Re-evaluate all pending firewall approval sessions after a permission change.
///
/// Uses the unified `check_needs_approval` for clients found in config.
/// If a client is not found in config or the check returns `Ask`, the popup stays open.
#[allow(clippy::too_many_arguments)]
pub(crate) fn reevaluate_pending_approvals(
    app: &tauri::AppHandle,
    firewall_manager: &lr_mcp::gateway::firewall::FirewallManager,
    config_manager: &ConfigManager,
    model_approval_tracker: &lr_server::state::ModelApprovalTracker,
    guardrail_approval_tracker: &lr_server::state::GuardrailApprovalTracker,
    guardrail_denial_tracker: &lr_server::state::GuardrailDenialTracker,
    free_tier_approval_tracker: &lr_server::state::FreeTierApprovalTracker,
    auto_router_approval_tracker: &lr_server::state::AutoRouterApprovalTracker,
) {
    use lr_mcp::gateway::access_control::{
        check_needs_approval, FirewallCheckContext, FirewallCheckResult,
    };
    use lr_mcp::gateway::firewall::FirewallApprovalAction;
    use tauri::Manager;

    let pending = firewall_manager.list_pending();
    if pending.is_empty() {
        return;
    }
    let config = config_manager.get();

    for info in &pending {
        // Only resolve if client is found in config
        let Some(client) = config.clients.iter().find(|c| c.id == info.client_id) else {
            continue;
        };

        let ctx = if info.is_auto_router_request {
            let strategy = config
                .strategies
                .iter()
                .find(|s| s.id == client.strategy_id);
            if let Some(strategy) = strategy {
                if let Some(ref auto_config) = strategy.auto_config {
                    FirewallCheckContext::AutoRouter {
                        permission: &auto_config.permission,
                        has_time_based_approval: auto_router_approval_tracker
                            .has_valid_approval(&info.client_id),
                    }
                } else {
                    continue;
                }
            } else {
                continue;
            }
        } else if info.is_free_tier_fallback {
            let strategy = config
                .strategies
                .iter()
                .find(|s| s.id == client.strategy_id);
            if let Some(strategy) = strategy {
                FirewallCheckContext::FreeTierFallback {
                    fallback_mode: &strategy.free_tier_fallback,
                    has_time_based_approval: free_tier_approval_tracker
                        .has_valid_approval(&info.client_id),
                }
            } else {
                continue;
            }
        } else if info.is_guardrail_request {
            FirewallCheckContext::Guardrail {
                has_time_based_bypass: guardrail_approval_tracker.has_valid_bypass(&info.client_id),
                has_time_based_denial: guardrail_denial_tracker.has_valid_denial(&info.client_id),
                category_actions_empty: client
                    .guardrails
                    .category_actions
                    .as_ref()
                    .is_none_or(|a| a.is_empty()),
            }
        } else if info.is_model_request {
            FirewallCheckContext::Model {
                permissions: &client.model_permissions,
                provider: &info.server_name,
                model_id: &info.tool_name,
                has_time_based_approval: model_approval_tracker.has_valid_approval(
                    &info.client_id,
                    &info.server_name,
                    &info.tool_name,
                ),
            }
        } else if info.tool_name.starts_with("skill_") {
            FirewallCheckContext::SkillTool {
                permissions: &client.skills_permissions,
                skill_name: &info.server_name,
                tool_name: &info.tool_name,
                session_approved: false,
                session_denied: false,
            }
        } else {
            let original_name = info.tool_name.split("__").nth(1).unwrap_or(&info.tool_name);
            FirewallCheckContext::McpTool {
                permissions: &client.mcp_permissions,
                server_id: &info.server_name,
                original_tool_name: original_name,
                session_approved: false,
                session_denied: false,
            }
        };

        let auto_action = match check_needs_approval(&ctx) {
            FirewallCheckResult::Allow => Some(FirewallApprovalAction::AllowOnce),
            FirewallCheckResult::Deny => Some(FirewallApprovalAction::Deny),
            FirewallCheckResult::Ask => None,
        };

        // Guard: if monitor intercept is active and would match, don't auto-resolve Allow
        if auto_action == Some(FirewallApprovalAction::AllowOnce) {
            use lr_mcp::gateway::firewall::InterceptCategory;
            let category = if info.is_model_request || info.is_auto_router_request {
                InterceptCategory::Llm
            } else if info.is_guardrail_request {
                InterceptCategory::Guardrails
            } else if info.is_secret_scan_request {
                InterceptCategory::SecretScan
            } else {
                InterceptCategory::Mcp
            };
            if firewall_manager.should_intercept(&info.client_id, category) {
                continue; // Keep popup open — intercept is still active
            }
        }

        if let Some(action) = auto_action {
            tracing::info!(
                "Re-evaluation: auto-resolving pending firewall request {} ({}) → {:?}",
                info.request_id,
                info.tool_name,
                action
            );
            let _ = firewall_manager.submit_response(&info.request_id, action, None);
            if let Some(window) =
                app.get_webview_window(&format!("firewall-approval-{}", info.request_id))
            {
                let _ = window.close();
            }
        }
    }
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

/// Helper to update the free_tier_fallback setting on a client's strategy
async fn update_free_tier_fallback_config(
    app: &tauri::AppHandle,
    config_manager: &ConfigManager,
    client_id: &str,
    fallback: lr_config::FreeTierFallback,
) -> Result<(), String> {
    use tauri::Emitter;

    config_manager
        .update(|cfg| {
            // Find the client to get its strategy_id
            if let Some(client) = cfg.clients.iter().find(|c| c.id == client_id) {
                let strategy_id = client.strategy_id.clone();
                if let Some(strategy) = cfg.strategies.iter_mut().find(|s| s.id == strategy_id) {
                    strategy.free_tier_fallback = fallback.clone();
                    tracing::info!(
                        "Updated free_tier_fallback to {:?} for strategy {}",
                        fallback,
                        strategy_id
                    );
                }
            }
        })
        .map_err(|e| e.to_string())?;

    config_manager.save().await.map_err(|e| e.to_string())?;

    if let Err(e) = app.emit("strategies-changed", ()) {
        tracing::error!("Failed to emit strategies-changed event: {}", e);
    }
    // Also emit clients-changed so pending approvals are re-evaluated
    if let Err(e) = app.emit("clients-changed", ()) {
        tracing::error!("Failed to emit clients-changed event: {}", e);
    }

    Ok(())
}

/// Helper to update auto-router permission on a client's strategy
async fn update_auto_router_permission(
    app: &tauri::AppHandle,
    config_manager: &ConfigManager,
    client_id: &str,
    permission: lr_config::PermissionState,
) -> Result<(), String> {
    use tauri::Emitter;

    config_manager
        .update(|cfg| {
            if let Some(client) = cfg.clients.iter().find(|c| c.id == client_id) {
                let strategy_id = client.strategy_id.clone();
                if let Some(strategy) = cfg.strategies.iter_mut().find(|s| s.id == strategy_id) {
                    if let Some(ref mut auto_config) = strategy.auto_config {
                        auto_config.permission = permission.clone();
                        tracing::info!(
                            "Updated auto_config.permission to {:?} for strategy {}",
                            permission,
                            strategy_id
                        );
                    }
                }
            }
        })
        .map_err(|e| e.to_string())?;

    config_manager.save().await.map_err(|e| e.to_string())?;

    if let Err(e) = app.emit("strategies-changed", ()) {
        tracing::error!("Failed to emit strategies-changed event: {}", e);
    }
    if let Err(e) = app.emit("clients-changed", ()) {
        tracing::error!("Failed to emit clients-changed event: {}", e);
    }

    Ok(())
}

/// Apply a permanent permission update to a client for an MCP/skill tool.
///
/// Dispatches based on virtual server ID:
/// - `_skills`: extracts skill name from arguments, updates `skills_permissions.tools`
/// - `_marketplace`: updates `marketplace_permission` directly
/// - `_coding_agents`: updates `coding_agent_permission` directly
/// - other: treats as MCP tool, updates `mcp_permissions.tools`
fn apply_tool_permission_to_client(
    client: &mut lr_config::Client,
    info: &lr_mcp::gateway::firewall::PendingApprovalInfo,
    permission: PermissionState,
) {
    if info.server_name == "_skills" {
        // Extract skill name from the full_arguments JSON.
        // For skills, info.server_name is "_skills" and info.tool_name is
        // the meta-tool name (e.g. "SkillRead"). The actual skill name
        // must be extracted from full_arguments (the "name" parameter).
        let skill_name = info
            .full_arguments
            .as_ref()
            .and_then(|args_str| serde_json::from_str::<serde_json::Value>(args_str).ok())
            .and_then(|args| {
                args.get("name")
                    .or_else(|| args.get("skill"))
                    .and_then(|v| v.as_str().map(|s| s.to_string()))
            });

        if let Some(skill_name) = skill_name {
            let key = format!("{}__{}", skill_name, info.tool_name);
            client
                .skills_permissions
                .tools
                .insert(key, permission.clone());
            tracing::info!(
                "Set skill tool permission to {:?}: skill={}, tool={}",
                permission,
                skill_name,
                info.tool_name
            );
        } else {
            tracing::warn!(
                "Could not extract skill name from arguments: tool={}, args={:?}",
                info.tool_name,
                info.full_arguments
            );
        }
    } else if info.server_name == "_marketplace" {
        // Marketplace uses a single PermissionState field, not per-tool
        client.marketplace_permission = permission.clone();
        tracing::info!(
            "Set marketplace permission to {:?} for client {}",
            permission,
            info.client_id
        );
    } else if info.server_name == "_coding_agents" {
        // Coding agents uses a single PermissionState field, not per-tool
        client.coding_agent_permission = permission.clone();
        tracing::info!(
            "Set coding agent permission to {:?} for client {}",
            permission,
            info.client_id
        );
    } else {
        // MCP tool — info.server_name is the server UUID,
        // info.tool_name is the namespaced name (slug__original_name).
        // Permission key must be UUID__original_name to match resolve_tool() and UI.
        let original_name = info.tool_name.split("__").nth(1).unwrap_or(&info.tool_name);
        let key = format!("{}__{}", info.server_name, original_name);
        client
            .mcp_permissions
            .tools
            .insert(key.clone(), permission.clone());
        tracing::info!("Set MCP tool permission to {:?}: {}", permission, key);
    }
}

/// Helper to update client permissions when AllowPermanent is selected for MCP/skill tools
async fn update_permission_for_allow_permanent(
    app: &tauri::AppHandle,
    config_manager: &ConfigManager,
    info: &lr_mcp::gateway::firewall::PendingApprovalInfo,
) -> Result<(), String> {
    tracing::info!(
        "Updating permissions for AllowPermanent: client={}, tool={}, server={}",
        info.client_id,
        info.tool_name,
        info.server_name
    );

    config_manager
        .update(|cfg| {
            if let Some(client) = cfg.clients.iter_mut().find(|c| c.id == info.client_id) {
                apply_tool_permission_to_client(client, info, PermissionState::Allow);
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
    tracing::info!(
        "Updating permissions for DenyAlways: client={}, tool={}, server={}",
        info.client_id,
        info.tool_name,
        info.server_name
    );

    config_manager
        .update(|cfg| {
            if let Some(client) = cfg.clients.iter_mut().find(|c| c.id == info.client_id) {
                apply_tool_permission_to_client(client, info, PermissionState::Off);
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
                    cfg.marketplace.mcp_enabled = true;
                    cfg.marketplace.skills_enabled = true;
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

/// Set Sampling permission for a client
#[tauri::command]
pub async fn set_client_sampling_permission(
    client_id: String,
    state: PermissionState,
    config_manager: State<'_, ConfigManager>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    tracing::info!(
        "Setting Sampling permission for client {}: state={:?}",
        client_id,
        state
    );

    let mut found = false;
    config_manager
        .update(|cfg| {
            if let Some(client) = cfg.clients.iter_mut().find(|c| c.id == client_id) {
                client.mcp_sampling_permission = state.clone();
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

/// Set Elicitation permission for a client
#[tauri::command]
pub async fn set_client_elicitation_permission(
    client_id: String,
    state: PermissionState,
    config_manager: State<'_, ConfigManager>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    tracing::info!(
        "Setting Elicitation permission for client {}: state={:?}",
        client_id,
        state
    );

    let mut found = false;
    config_manager
        .update(|cfg| {
            if let Some(client) = cfg.clients.iter_mut().find(|c| c.id == client_id) {
                client.mcp_elicitation_permission = state.clone();
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

/// Set the client mode (both, llm_only, mcp_only, mcp_via_llm)
#[tauri::command]
pub async fn set_client_mode(
    client_id: String,
    mode: ClientMode,
    config_manager: State<'_, ConfigManager>,
    client_manager: State<'_, Arc<lr_clients::ClientManager>>,
    provider_registry: State<'_, Arc<lr_providers::registry::ProviderRegistry>>,
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

    // Resync external config to match the new mode
    if let Err(e) = sync_client_config_inner(
        &client_id,
        config_manager.inner(),
        client_manager.inner(),
        provider_registry.inner(),
    )
    .await
    {
        tracing::warn!(
            "Failed to resync config after mode change for {}: {}",
            client_id,
            e
        );
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

/// Get the guardrails configuration for a specific client
#[tauri::command]
pub async fn get_client_guardrails_config(
    client_id: String,
    config_manager: State<'_, ConfigManager>,
) -> Result<serde_json::Value, String> {
    let config = config_manager.get();
    let client = config
        .clients
        .iter()
        .find(|c| c.id == client_id)
        .ok_or_else(|| format!("Client not found: {}", client_id))?;

    serde_json::to_value(&client.guardrails).map_err(|e| e.to_string())
}

/// Update the guardrails configuration for a specific client
#[tauri::command]
pub async fn update_client_guardrails_config(
    client_id: String,
    config_json: String,
    config_manager: State<'_, ConfigManager>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    let new_config: lr_config::ClientGuardrailsConfig =
        serde_json::from_str(&config_json).map_err(|e| format!("Invalid config JSON: {}", e))?;

    let mut found = false;
    config_manager
        .update(|cfg| {
            if let Some(client) = cfg.clients.iter_mut().find(|c| c.id == client_id) {
                client.guardrails = new_config.clone();
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
// Per-Client Prompt Compression Commands
// ============================================================================

/// Get the prompt compression configuration for a specific client
#[tauri::command]
pub async fn get_client_compression_config(
    client_id: String,
    config_manager: State<'_, ConfigManager>,
) -> Result<serde_json::Value, String> {
    let config = config_manager.get();
    let client = config
        .clients
        .iter()
        .find(|c| c.id == client_id)
        .ok_or_else(|| format!("Client not found: {}", client_id))?;

    serde_json::to_value(&client.prompt_compression).map_err(|e| e.to_string())
}

/// Update the prompt compression configuration for a specific client
#[tauri::command]
pub async fn update_client_compression_config(
    client_id: String,
    config_json: String,
    config_manager: State<'_, ConfigManager>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    let new_config: lr_config::ClientPromptCompressionConfig =
        serde_json::from_str(&config_json).map_err(|e| format!("Invalid config JSON: {}", e))?;

    let mut found = false;
    config_manager
        .update(|cfg| {
            if let Some(client) = cfg.clients.iter_mut().find(|c| c.id == client_id) {
                client.prompt_compression = new_config.clone();
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

// ============================================================================
// Config Sync Commands
// ============================================================================

/// Inner helper to sync a single client's external config.
/// Returns Ok(Some(result)) if sync was performed, Ok(None) if skipped.
pub async fn sync_client_config_inner(
    client_id: &str,
    config_manager: &ConfigManager,
    client_manager: &Arc<lr_clients::ClientManager>,
    provider_registry: &Arc<lr_providers::registry::ProviderRegistry>,
) -> Result<Option<LaunchResult>, String> {
    use crate::launcher;

    let config = config_manager.get();
    let client = config
        .clients
        .iter()
        .find(|c| c.id == client_id)
        .ok_or_else(|| format!("Client not found: {}", client_id))?;

    // Skip if sync is not enabled or no template
    if !client.sync_config {
        return Ok(None);
    }
    let template_id = match client.template_id.as_deref() {
        Some(id) => id,
        None => return Ok(None),
    };

    let integration = launcher::get_integration(template_id)
        .ok_or_else(|| format!("No integration found for template: {}", template_id))?;

    let base_url = format!("http://{}:{}", config.server.host, config.server.port);

    let client_secret = client_manager
        .get_secret(client_id)
        .map_err(|e| format!("Failed to get client secret: {}", e))?
        .ok_or("Client secret not found in keychain")?;

    // Build model list if needed
    let models = if integration.needs_model_list() {
        // Get strategy for this client
        let strategy = config
            .strategies
            .iter()
            .find(|s| s.id == client.strategy_id);

        // If auto router is enabled, only include the auto model + prioritized/available models
        let auto_config_active = strategy
            .and_then(|s| s.auto_config.as_ref())
            .filter(|ac| ac.permission != lr_config::PermissionState::Off);

        if let Some(auto_config) = auto_config_active {
            let mut models = vec![auto_config.model_name.clone()];
            for (provider, model) in &auto_config.prioritized_models {
                models.push(format!("{}/{}", provider, model));
            }
            for (provider, model) in &auto_config.available_models {
                models.push(format!("{}/{}", provider, model));
            }
            models
        } else {
            // Get all available models
            let all_models = provider_registry
                .list_all_models()
                .await
                .map_err(|e| format!("Failed to list models: {}", e))?;

            // Filter by strategy and format as "provider/model_id"
            all_models
                .iter()
                .filter(|m| {
                    strategy
                        .map(|s| s.is_model_allowed(&m.provider, &m.id))
                        .unwrap_or(true)
                })
                .map(|m| format!("{}/{}", m.provider, m.id))
                .collect()
        }
    } else {
        vec![]
    };

    let ctx = launcher::ConfigSyncContext {
        base_url,
        client_secret,
        client_id: client_id.to_string(),
        models,
        client_mode: client.client_mode.clone(),
    };

    integration.sync_config(&ctx).map(Some)
}

/// Sync all clients that have sync_config enabled.
pub async fn sync_all_clients(
    config_manager: &ConfigManager,
    client_manager: &Arc<lr_clients::ClientManager>,
    provider_registry: &Arc<lr_providers::registry::ProviderRegistry>,
) {
    let config = config_manager.get();
    let sync_clients: Vec<String> = config
        .clients
        .iter()
        .filter(|c| c.sync_config && c.template_id.is_some())
        .map(|c| c.id.clone())
        .collect();

    for client_id in sync_clients {
        match sync_client_config_inner(
            &client_id,
            config_manager,
            client_manager,
            provider_registry,
        )
        .await
        {
            Ok(Some(result)) => {
                tracing::info!("Synced config for client {}: {}", client_id, result.message);
            }
            Ok(None) => {}
            Err(e) => {
                tracing::warn!("Failed to sync config for client {}: {}", client_id, e);
            }
        }
    }
}

/// Toggle auto-sync of external app config for a client
#[tauri::command]
pub async fn toggle_client_sync_config(
    client_id: String,
    enabled: bool,
    config_manager: State<'_, ConfigManager>,
    client_manager: State<'_, Arc<lr_clients::ClientManager>>,
    provider_registry: State<'_, Arc<lr_providers::registry::ProviderRegistry>>,
    app: tauri::AppHandle,
) -> Result<Option<LaunchResult>, String> {
    tracing::info!("Setting client {} sync_config: {}", client_id, enabled);

    let mut found = false;
    config_manager
        .update(|cfg| {
            if let Some(client) = cfg.clients.iter_mut().find(|c| c.id == client_id) {
                client.sync_config = enabled;
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

    // If enabling, immediately sync
    if enabled {
        sync_client_config_inner(
            &client_id,
            config_manager.inner(),
            client_manager.inner(),
            provider_registry.inner(),
        )
        .await
    } else {
        Ok(None)
    }
}

/// Manually trigger a config sync for a client
#[tauri::command]
pub async fn sync_client_config(
    client_id: String,
    config_manager: State<'_, ConfigManager>,
    client_manager: State<'_, Arc<lr_clients::ClientManager>>,
    provider_registry: State<'_, Arc<lr_providers::registry::ProviderRegistry>>,
) -> Result<Option<LaunchResult>, String> {
    sync_client_config_inner(
        &client_id,
        config_manager.inner(),
        client_manager.inner(),
        provider_registry.inner(),
    )
    .await
}

// ============================================================================
// Per-Client Secret Scanning Commands
// ============================================================================

/// Get the secret scanning configuration for a specific client
#[tauri::command]
pub async fn get_client_secret_scanning_config(
    client_id: String,
    config_manager: State<'_, ConfigManager>,
) -> Result<serde_json::Value, String> {
    let config = config_manager.get();
    let client = config
        .clients
        .iter()
        .find(|c| c.id == client_id)
        .ok_or_else(|| format!("Client not found: {}", client_id))?;

    serde_json::to_value(&client.secret_scanning).map_err(|e| e.to_string())
}

/// Update the secret scanning configuration for a specific client
#[tauri::command]
pub async fn update_client_secret_scanning_config(
    client_id: String,
    config_json: String,
    config_manager: State<'_, ConfigManager>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    let new_config: lr_config::ClientSecretScanningConfig =
        serde_json::from_str(&config_json).map_err(|e| format!("Invalid config JSON: {}", e))?;

    let mut found = false;
    config_manager
        .update(|cfg| {
            if let Some(client) = cfg.clients.iter_mut().find(|c| c.id == client_id) {
                client.secret_scanning = new_config.clone();
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
// Per-Client JSON Repair Commands
// ============================================================================

/// Get the JSON repair configuration for a specific client
#[tauri::command]
pub async fn get_client_json_repair_config(
    client_id: String,
    config_manager: State<'_, ConfigManager>,
) -> Result<serde_json::Value, String> {
    let config = config_manager.get();
    let client = config
        .clients
        .iter()
        .find(|c| c.id == client_id)
        .ok_or_else(|| format!("Client not found: {}", client_id))?;

    serde_json::to_value(&client.json_repair).map_err(|e| e.to_string())
}

/// Update the JSON repair configuration for a specific client
#[tauri::command]
pub async fn update_client_json_repair_config(
    client_id: String,
    config_json: String,
    config_manager: State<'_, ConfigManager>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    let new_config: lr_config::ClientJsonRepairConfig =
        serde_json::from_str(&config_json).map_err(|e| format!("Invalid config JSON: {}", e))?;

    let mut found = false;
    config_manager
        .update(|cfg| {
            if let Some(client) = cfg.clients.iter_mut().find(|c| c.id == client_id) {
                client.json_repair = new_config.clone();
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

#[cfg(test)]
mod tests {
    use super::*;
    use lr_mcp::gateway::firewall::PendingApprovalInfo;

    /// Create a test client with a known ID
    fn test_client(id: &str) -> lr_config::Client {
        let mut client =
            lr_config::Client::new_with_strategy("test".to_string(), "strat".to_string());
        client.id = id.to_string();
        client
    }

    /// Build a PendingApprovalInfo for testing
    fn make_info(
        client_id: &str,
        tool_name: &str,
        server_name: &str,
        full_arguments: Option<&str>,
    ) -> PendingApprovalInfo {
        PendingApprovalInfo {
            request_id: "req-1".to_string(),
            client_id: client_id.to_string(),
            client_name: "test-client".to_string(),
            tool_name: tool_name.to_string(),
            server_name: server_name.to_string(),
            arguments_preview: "{}".to_string(),
            full_arguments: full_arguments.map(|s| s.to_string()),
            created_at_secs_ago: 0,
            timeout_seconds: 30,
            is_model_request: false,
            is_guardrail_request: false,
            is_free_tier_fallback: false,
            is_auto_router_request: false,
            is_mcp_via_llm_request: false,
            is_secret_scan_request: false,
            guardrail_details: None,
            secret_scan_details: None,
        }
    }

    // =========================================================================
    // Skill tool permission tests
    // =========================================================================

    #[test]
    fn test_skill_allow_permanent_stores_correct_key() {
        let mut client = test_client("c1");
        let info = make_info(
            "c1",
            "SkillRead",
            "_skills",
            Some(r#"{"name": "weather"}"#),
        );

        apply_tool_permission_to_client(&mut client, &info, PermissionState::Allow);

        assert_eq!(
            client.skills_permissions.tools.get("weather__SkillRead"),
            Some(&PermissionState::Allow),
            "Should store permission under 'weather__SkillRead' in skills_permissions.tools"
        );
        assert!(
            client.mcp_permissions.tools.is_empty(),
            "Should NOT write to mcp_permissions"
        );
    }

    #[test]
    fn test_skill_deny_permanent_stores_correct_key() {
        let mut client = test_client("c1");
        let info = make_info(
            "c1",
            "SkillRead",
            "_skills",
            Some(r#"{"name": "weather"}"#),
        );

        apply_tool_permission_to_client(&mut client, &info, PermissionState::Off);

        assert_eq!(
            client.skills_permissions.tools.get("weather__SkillRead"),
            Some(&PermissionState::Off),
        );
    }

    #[test]
    fn test_skill_uses_skill_argument_key() {
        let mut client = test_client("c1");
        let info = make_info(
            "c1",
            "SkillRead",
            "_skills",
            Some(r#"{"skill": "sysinfo"}"#),
        );

        apply_tool_permission_to_client(&mut client, &info, PermissionState::Allow);

        assert_eq!(
            client.skills_permissions.tools.get("sysinfo__SkillRead"),
            Some(&PermissionState::Allow),
            "Should fall back to 'skill' argument when 'name' is absent"
        );
    }

    #[test]
    fn test_skill_no_arguments_does_not_crash() {
        let mut client = test_client("c1");
        let info = make_info("c1", "SkillRead", "_skills", None);

        apply_tool_permission_to_client(&mut client, &info, PermissionState::Allow);

        assert!(
            client.skills_permissions.tools.is_empty(),
            "Should not insert anything when skill name cannot be extracted"
        );
    }

    #[test]
    fn test_skill_empty_arguments_does_not_crash() {
        let mut client = test_client("c1");
        let info = make_info("c1", "SkillRead", "_skills", Some(r#"{}"#));

        apply_tool_permission_to_client(&mut client, &info, PermissionState::Allow);

        assert!(
            client.skills_permissions.tools.is_empty(),
            "Should not insert anything when skill name is missing from arguments"
        );
    }

    #[test]
    fn test_skill_permission_resolves_correctly() {
        let mut client = test_client("c1");
        client.skills_permissions.global = PermissionState::Ask;
        let info = make_info(
            "c1",
            "SkillRead",
            "_skills",
            Some(r#"{"name": "weather"}"#),
        );

        apply_tool_permission_to_client(&mut client, &info, PermissionState::Allow);

        // Verify the stored permission is found by the same resolution logic
        // used during access control checks
        assert_eq!(
            client
                .skills_permissions
                .resolve_tool("weather", "SkillRead"),
            PermissionState::Allow,
            "resolve_tool should find the permission set by apply_tool_permission_to_client"
        );
        // Other skills should still fall through to global
        assert_eq!(
            client
                .skills_permissions
                .resolve_tool("sysinfo", "SkillRead"),
            PermissionState::Ask,
        );
    }

    // =========================================================================
    // Marketplace permission tests
    // =========================================================================

    #[test]
    fn test_marketplace_allow_permanent() {
        let mut client = test_client("c1");
        assert_eq!(client.marketplace_permission, PermissionState::Off);

        let info = make_info("c1", "marketplace_search", "_marketplace", None);

        apply_tool_permission_to_client(&mut client, &info, PermissionState::Allow);

        assert_eq!(client.marketplace_permission, PermissionState::Allow);
        assert!(
            client.mcp_permissions.tools.is_empty(),
            "Should NOT write to mcp_permissions"
        );
    }

    #[test]
    fn test_marketplace_deny_permanent() {
        let mut client = test_client("c1");
        client.marketplace_permission = PermissionState::Allow;

        let info = make_info("c1", "marketplace_search", "_marketplace", None);

        apply_tool_permission_to_client(&mut client, &info, PermissionState::Off);

        assert_eq!(client.marketplace_permission, PermissionState::Off);
    }

    // =========================================================================
    // Coding agent permission tests
    // =========================================================================

    #[test]
    fn test_coding_agent_allow_permanent() {
        let mut client = test_client("c1");
        assert_eq!(client.coding_agent_permission, PermissionState::Off);

        let info = make_info("c1", "claude_code__start", "_coding_agents", None);

        apply_tool_permission_to_client(&mut client, &info, PermissionState::Allow);

        assert_eq!(client.coding_agent_permission, PermissionState::Allow);
        assert!(
            client.mcp_permissions.tools.is_empty(),
            "Should NOT write to mcp_permissions"
        );
    }

    #[test]
    fn test_coding_agent_deny_permanent() {
        let mut client = test_client("c1");
        client.coding_agent_permission = PermissionState::Allow;

        let info = make_info("c1", "claude_code__start", "_coding_agents", None);

        apply_tool_permission_to_client(&mut client, &info, PermissionState::Off);

        assert_eq!(client.coding_agent_permission, PermissionState::Off);
    }

    // =========================================================================
    // MCP tool permission tests
    // =========================================================================

    #[test]
    fn test_mcp_tool_allow_permanent() {
        let mut client = test_client("c1");
        let info = make_info("c1", "fs-slug__read_file", "srv-uuid-123", None);

        apply_tool_permission_to_client(&mut client, &info, PermissionState::Allow);

        assert_eq!(
            client
                .mcp_permissions
                .tools
                .get("srv-uuid-123__read_file"),
            Some(&PermissionState::Allow),
            "Should use server UUID and original tool name (after __) as key"
        );
    }

    #[test]
    fn test_mcp_tool_deny_permanent() {
        let mut client = test_client("c1");
        let info = make_info("c1", "fs-slug__write_file", "srv-uuid-123", None);

        apply_tool_permission_to_client(&mut client, &info, PermissionState::Off);

        assert_eq!(
            client
                .mcp_permissions
                .tools
                .get("srv-uuid-123__write_file"),
            Some(&PermissionState::Off),
        );
    }

    #[test]
    fn test_mcp_tool_no_namespace_uses_full_name() {
        let mut client = test_client("c1");
        let info = make_info("c1", "simple_tool", "srv-uuid", None);

        apply_tool_permission_to_client(&mut client, &info, PermissionState::Allow);

        assert_eq!(
            client.mcp_permissions.tools.get("srv-uuid__simple_tool"),
            Some(&PermissionState::Allow),
            "When tool_name has no __, should use the full name as the original"
        );
    }

    #[test]
    fn test_mcp_tool_permission_resolves_correctly() {
        let mut client = test_client("c1");
        client.mcp_permissions.global = PermissionState::Ask;
        let info = make_info("c1", "fs__read_file", "srv-uuid", None);

        apply_tool_permission_to_client(&mut client, &info, PermissionState::Allow);

        assert_eq!(
            client.mcp_permissions.resolve_tool("srv-uuid", "read_file"),
            PermissionState::Allow,
            "resolve_tool should find the permission set by apply_tool_permission_to_client"
        );
        // Other tools on same server should fall through to global
        assert_eq!(
            client
                .mcp_permissions
                .resolve_tool("srv-uuid", "write_file"),
            PermissionState::Ask,
        );
    }
}
