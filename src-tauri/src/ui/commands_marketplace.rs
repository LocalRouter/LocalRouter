//! Marketplace-related Tauri command handlers
//!
//! Commands for searching, browsing, and installing MCP servers and skills
//! from the marketplace.

use std::sync::Arc;

use lr_config::ConfigManager;
use lr_marketplace::{
    install_popup::{InstallAction, PendingInstallInfo},
    types::{InstalledServer, InstalledSkill, McpInstallConfig, McpServerListing, SkillListing, MCP_REGISTRY_SOURCE_ID},
    MarketplaceService,
};
use serde_json::Value;
use tauri::State;

/// Keyring service name for MCP server tokens
const MCP_KEYRING_SERVICE: &str = "LocalRouter-McpServers";

// ============================================================================
// Marketplace Config Commands
// ============================================================================

/// Get marketplace configuration
#[tauri::command]
pub async fn marketplace_get_config(
    config_manager: State<'_, ConfigManager>,
) -> Result<lr_config::MarketplaceConfig, String> {
    let config = config_manager.get();
    Ok(config.marketplace)
}

/// Set marketplace enabled state
#[tauri::command]
pub async fn marketplace_set_enabled(
    enabled: bool,
    config_manager: State<'_, ConfigManager>,
    marketplace_service: State<'_, Option<Arc<MarketplaceService>>>,
) -> Result<(), String> {
    config_manager
        .update(|cfg| {
            cfg.marketplace.enabled = enabled;
        })
        .map_err(|e| e.to_string())?;

    config_manager.save().await.map_err(|e| e.to_string())?;

    // Update the service config if available
    if let Some(ref service) = *marketplace_service.inner() {
        let config = config_manager.get();
        service.update_config(config.marketplace);
    }

    Ok(())
}

/// Set registry URL
#[tauri::command]
pub async fn marketplace_set_registry_url(
    url: String,
    config_manager: State<'_, ConfigManager>,
    marketplace_service: State<'_, Option<Arc<MarketplaceService>>>,
) -> Result<(), String> {
    config_manager
        .update(|cfg| {
            cfg.marketplace.registry_url = url;
        })
        .map_err(|e| e.to_string())?;

    config_manager.save().await.map_err(|e| e.to_string())?;

    // Update the service config if available
    if let Some(ref service) = *marketplace_service.inner() {
        let config = config_manager.get();
        service.update_config(config.marketplace);
    }

    Ok(())
}

/// Reset registry URL to default
#[tauri::command]
pub async fn marketplace_reset_registry_url(
    config_manager: State<'_, ConfigManager>,
    marketplace_service: State<'_, Option<Arc<MarketplaceService>>>,
) -> Result<String, String> {
    let default_url = lr_config::MarketplaceConfig::default().registry_url;

    config_manager
        .update(|cfg| {
            cfg.marketplace.registry_url = default_url.clone();
        })
        .map_err(|e| e.to_string())?;

    config_manager.save().await.map_err(|e| e.to_string())?;

    // Update the service config if available
    if let Some(ref service) = *marketplace_service.inner() {
        let config = config_manager.get();
        service.update_config(config.marketplace);
    }

    Ok(default_url)
}

/// List skill sources
#[tauri::command]
pub async fn marketplace_list_skill_sources(
    config_manager: State<'_, ConfigManager>,
) -> Result<Vec<lr_config::MarketplaceSkillSource>, String> {
    let config = config_manager.get();
    Ok(config.marketplace.skill_sources)
}

/// Add a skill source
#[tauri::command]
pub async fn marketplace_add_skill_source(
    source: lr_config::MarketplaceSkillSource,
    config_manager: State<'_, ConfigManager>,
    marketplace_service: State<'_, Option<Arc<MarketplaceService>>>,
) -> Result<(), String> {
    config_manager
        .update(|cfg| {
            cfg.marketplace.skill_sources.push(source);
        })
        .map_err(|e| e.to_string())?;

    config_manager.save().await.map_err(|e| e.to_string())?;

    // Update the service config if available
    if let Some(ref service) = *marketplace_service.inner() {
        let config = config_manager.get();
        service.update_config(config.marketplace);
    }

    Ok(())
}

/// Remove a skill source
#[tauri::command]
pub async fn marketplace_remove_skill_source(
    repo_url: String,
    config_manager: State<'_, ConfigManager>,
    marketplace_service: State<'_, Option<Arc<MarketplaceService>>>,
) -> Result<(), String> {
    config_manager
        .update(|cfg| {
            cfg.marketplace
                .skill_sources
                .retain(|s| s.repo_url != repo_url);
        })
        .map_err(|e| e.to_string())?;

    config_manager.save().await.map_err(|e| e.to_string())?;

    // Update the service config if available
    if let Some(ref service) = *marketplace_service.inner() {
        let config = config_manager.get();
        service.update_config(config.marketplace);
    }

    Ok(())
}

/// Add default skill sources (if not already present)
#[tauri::command]
pub async fn marketplace_add_default_skill_sources(
    config_manager: State<'_, ConfigManager>,
    marketplace_service: State<'_, Option<Arc<MarketplaceService>>>,
) -> Result<u32, String> {
    let defaults = lr_config::MarketplaceConfig::default().skill_sources;
    let mut added_count = 0u32;

    config_manager
        .update(|cfg| {
            for default_source in &defaults {
                // Check if this source is already present (by repo_url)
                let already_exists = cfg
                    .marketplace
                    .skill_sources
                    .iter()
                    .any(|s| s.repo_url == default_source.repo_url);

                if !already_exists {
                    cfg.marketplace.skill_sources.push(default_source.clone());
                    added_count += 1;
                }
            }
        })
        .map_err(|e| e.to_string())?;

    if added_count > 0 {
        config_manager.save().await.map_err(|e| e.to_string())?;

        // Update the service config if available
        if let Some(ref service) = *marketplace_service.inner() {
            let config = config_manager.get();
            service.update_config(config.marketplace);
        }
    }

    Ok(added_count)
}

// ============================================================================
// Cache Commands
// ============================================================================

/// Get cache status
#[tauri::command]
pub async fn marketplace_get_cache_status(
    marketplace_service: State<'_, Option<Arc<MarketplaceService>>>,
) -> Result<lr_marketplace::CacheStatus, String> {
    let service = marketplace_service
        .inner()
        .as_ref()
        .ok_or_else(|| "Marketplace service not initialized".to_string())?;

    Ok(service.get_cache_status())
}

/// Refresh all caches
#[tauri::command]
pub async fn marketplace_refresh_cache(
    marketplace_service: State<'_, Option<Arc<MarketplaceService>>>,
) -> Result<(), String> {
    let service = marketplace_service
        .inner()
        .as_ref()
        .ok_or_else(|| "Marketplace service not initialized".to_string())?;

    service.refresh_all().await.map_err(|e| e.to_string())
}

/// Clear MCP cache only
#[tauri::command]
pub async fn marketplace_clear_mcp_cache(
    marketplace_service: State<'_, Option<Arc<MarketplaceService>>>,
) -> Result<(), String> {
    let service = marketplace_service
        .inner()
        .as_ref()
        .ok_or_else(|| "Marketplace service not initialized".to_string())?;

    service.clear_mcp_cache();
    Ok(())
}

/// Clear skills cache only
#[tauri::command]
pub async fn marketplace_clear_skills_cache(
    marketplace_service: State<'_, Option<Arc<MarketplaceService>>>,
) -> Result<(), String> {
    let service = marketplace_service
        .inner()
        .as_ref()
        .ok_or_else(|| "Marketplace service not initialized".to_string())?;

    service.clear_skills_cache();
    Ok(())
}

// ============================================================================
// Search Commands
// ============================================================================

/// Search MCP servers in the registry
#[tauri::command]
pub async fn marketplace_search_mcp_servers(
    query: String,
    limit: Option<u32>,
    marketplace_service: State<'_, Option<Arc<MarketplaceService>>>,
) -> Result<Vec<McpServerListing>, String> {
    let service = marketplace_service
        .inner()
        .as_ref()
        .ok_or_else(|| "Marketplace service not initialized".to_string())?;

    if !service.is_enabled() {
        return Err("Marketplace is not enabled".to_string());
    }

    service
        .search_mcp_servers(&query, limit)
        .await
        .map_err(|e| e.to_string())
}

/// Search skills from configured sources
#[tauri::command]
pub async fn marketplace_search_skills(
    query: Option<String>,
    source: Option<String>,
    marketplace_service: State<'_, Option<Arc<MarketplaceService>>>,
) -> Result<Vec<SkillListing>, String> {
    let service = marketplace_service
        .inner()
        .as_ref()
        .ok_or_else(|| "Marketplace service not initialized".to_string())?;

    if !service.is_enabled() {
        return Err("Marketplace is not enabled".to_string());
    }

    service
        .search_skills(query.as_deref(), source.as_deref())
        .await
        .map_err(|e| e.to_string())
}

// ============================================================================
// Install Commands (Direct - for UI, not AI)
// ============================================================================

/// Install an MCP server directly (from UI)
#[tauri::command]
pub async fn marketplace_install_mcp_server_direct(
    config: McpInstallConfig,
    config_manager: State<'_, ConfigManager>,
    mcp_server_manager: State<'_, Arc<lr_mcp::McpServerManager>>,
    app_handle: tauri::AppHandle,
) -> Result<InstalledServer, String> {
    use lr_marketplace::install::create_mcp_server_config;

    // Create a dummy listing (we only need the config)
    let listing = McpServerListing {
        name: config.name.clone(),
        description: String::new(),
        source_id: MCP_REGISTRY_SOURCE_ID.to_string(),
        homepage: None,
        vendor: None,
        packages: vec![],
        remotes: vec![],
        available_transports: vec![],
        install_hint: None,
    };

    // Create the server config
    let server_config = create_mcp_server_config(&listing, &config).map_err(|e| e.to_string())?;
    let server_id = server_config.id.clone();
    let server_name = server_config.name.clone();

    // Handle bearer token - store in keychain if present
    if config.auth_type == "bearer" {
        if let Some(token) = &config.bearer_token {
            // Store the token in the keychain
            let keyring_entry = keyring::Entry::new(MCP_KEYRING_SERVICE, &server_id)
                .map_err(|e| format!("Failed to create keyring entry: {}", e))?;
            keyring_entry
                .set_password(token)
                .map_err(|e| format!("Failed to store token in keychain: {}", e))?;
        }
    }

    // Add to config
    config_manager
        .update(|cfg| {
            cfg.mcp_servers.push(server_config.clone());
        })
        .map_err(|e| e.to_string())?;

    config_manager.save().await.map_err(|e| e.to_string())?;

    // Add to MCP server manager and start it
    mcp_server_manager.add_config(server_config);
    if let Err(e) = mcp_server_manager.start_server(&server_id).await {
        tracing::warn!(
            "Failed to start newly installed server {}: {}",
            server_id,
            e
        );
        // Don't fail the install - server is added, just not started
    }

    // Emit event
    use tauri::Emitter;
    let _ = app_handle.emit("mcp-servers-changed", ());

    Ok(InstalledServer {
        server_id,
        name: server_name,
    })
}

/// Install a skill directly (from UI)
#[tauri::command]
pub async fn marketplace_install_skill_direct(
    source_url: String,
    skill_name: String,
    marketplace_service: State<'_, Option<Arc<MarketplaceService>>>,
    config_manager: State<'_, ConfigManager>,
    skill_manager: State<'_, Arc<lr_skills::SkillManager>>,
    app_handle: tauri::AppHandle,
) -> Result<InstalledSkill, String> {
    let service = marketplace_service
        .inner()
        .as_ref()
        .ok_or_else(|| "Marketplace service not initialized".to_string())?;

    // Search for the skill to get full listing
    let results = service
        .search_skills(Some(&skill_name), None)
        .await
        .map_err(|e| e.to_string())?;

    let listing = results
        .into_iter()
        .find(|s| s.name == skill_name || s.skill_md_url == source_url)
        .ok_or_else(|| format!("Skill '{}' not found", skill_name))?;

    // Download skill to data directory
    let skills_dir = service.skills_data_dir();
    let skill_target_dir = skills_dir.join(&listing.source_label).join(&listing.name);

    // Use the skill_sources_client to download
    // Since skill_sources_client is internal, we'll do the download here
    let http_client = reqwest::Client::new();

    // Create target directory
    std::fs::create_dir_all(&skill_target_dir)
        .map_err(|e| format!("Failed to create directory: {}", e))?;

    // Download SKILL.md
    let skill_md = http_client
        .get(&listing.skill_md_url)
        .send()
        .await
        .map_err(|e| format!("Failed to download SKILL.md: {}", e))?
        .text()
        .await
        .map_err(|e| format!("Failed to read SKILL.md: {}", e))?;

    std::fs::write(skill_target_dir.join("SKILL.md"), skill_md)
        .map_err(|e| format!("Failed to write SKILL.md: {}", e))?;

    // Download additional files
    for file in &listing.files {
        let file_path = skill_target_dir.join(&file.path);
        if let Some(parent) = file_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create directory: {}", e))?;
        }

        let content = http_client
            .get(&file.url)
            .send()
            .await
            .map_err(|e| format!("Failed to download {}: {}", file.path, e))?
            .bytes()
            .await
            .map_err(|e| format!("Failed to read {}: {}", file.path, e))?;

        std::fs::write(&file_path, content)
            .map_err(|e| format!("Failed to write {}: {}", file.path, e))?;
    }

    // Add path to config if not already present
    let skill_path = skill_target_dir.to_string_lossy().to_string();
    let mut path_added = false;

    config_manager
        .update(|cfg| {
            if !cfg.skills.paths.contains(&skill_path) {
                cfg.skills.paths.push(skill_path.clone());
                path_added = true;
            }
        })
        .map_err(|e| e.to_string())?;

    if path_added {
        config_manager.save().await.map_err(|e| e.to_string())?;
    }

    // Trigger skill rescan
    let config = config_manager.get();
    skill_manager.rescan(&config.skills.paths, &config.skills.disabled_skills);

    // Emit event
    use tauri::Emitter;
    let _ = app_handle.emit("skills-changed", ());

    Ok(InstalledSkill {
        name: listing.name,
        path: skill_path,
    })
}

/// Delete a marketplace-installed skill
#[tauri::command]
pub async fn marketplace_delete_skill(
    skill_name: String,
    skill_path: String,
    marketplace_service: State<'_, Option<Arc<MarketplaceService>>>,
    config_manager: State<'_, ConfigManager>,
    skill_manager: State<'_, Arc<lr_skills::SkillManager>>,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    let service = marketplace_service
        .inner()
        .as_ref()
        .ok_or_else(|| "Marketplace service not initialized".to_string())?;

    // Verify the skill is from the marketplace directory
    let skills_dir = service.skills_data_dir();
    let skill_path_buf = std::path::PathBuf::from(&skill_path);

    if !skill_path_buf.starts_with(&skills_dir) {
        return Err(format!(
            "Skill '{}' is not a marketplace-installed skill and cannot be deleted this way",
            skill_name
        ));
    }

    // Delete the skill directory
    if skill_path_buf.exists() {
        std::fs::remove_dir_all(&skill_path_buf)
            .map_err(|e| format!("Failed to delete skill directory: {}", e))?;
    }

    // Remove path from config
    config_manager
        .update(|cfg| {
            cfg.skills.paths.retain(|p| p != &skill_path);
        })
        .map_err(|e| e.to_string())?;

    config_manager.save().await.map_err(|e| e.to_string())?;

    // Trigger skill rescan
    let config = config_manager.get();
    skill_manager.rescan(&config.skills.paths, &config.skills.disabled_skills);

    // Emit event
    use tauri::Emitter;
    let _ = app_handle.emit("skills-changed", ());

    Ok(())
}

/// Check if a skill path is from the marketplace
#[tauri::command]
pub async fn marketplace_is_skill_from_marketplace(
    skill_path: String,
    marketplace_service: State<'_, Option<Arc<MarketplaceService>>>,
) -> Result<bool, String> {
    let service = match marketplace_service.inner().as_ref() {
        Some(s) => s,
        None => return Ok(false),
    };

    let skills_dir = service.skills_data_dir();
    let skill_path_buf = std::path::PathBuf::from(&skill_path);

    Ok(skill_path_buf.starts_with(&skills_dir))
}

// ============================================================================
// Install Popup Commands (for AI-triggered installs)
// ============================================================================

/// Get details of a pending install request
#[tauri::command]
pub async fn marketplace_get_pending_install(
    request_id: String,
    marketplace_service: State<'_, Option<Arc<MarketplaceService>>>,
) -> Result<Option<PendingInstallInfo>, String> {
    let service = marketplace_service
        .inner()
        .as_ref()
        .ok_or_else(|| "Marketplace service not initialized".to_string())?;

    Ok(service.install_manager().get_pending(&request_id))
}

/// List all pending install requests
#[tauri::command]
pub async fn marketplace_list_pending_installs(
    marketplace_service: State<'_, Option<Arc<MarketplaceService>>>,
) -> Result<Vec<PendingInstallInfo>, String> {
    let service = marketplace_service
        .inner()
        .as_ref()
        .ok_or_else(|| "Marketplace service not initialized".to_string())?;

    Ok(service.install_manager().list_pending())
}

/// Respond to a pending install request
#[tauri::command]
pub async fn marketplace_install_respond(
    request_id: String,
    action: String,
    config: Option<Value>,
    marketplace_service: State<'_, Option<Arc<MarketplaceService>>>,
) -> Result<(), String> {
    let service = marketplace_service
        .inner()
        .as_ref()
        .ok_or_else(|| "Marketplace service not initialized".to_string())?;

    let install_action = match action.as_str() {
        "install" => InstallAction::Install,
        "cancel" => InstallAction::Cancel,
        other => return Err(format!("Invalid action: {}", other)),
    };

    service
        .install_manager()
        .submit_response(&request_id, install_action, config)
        .map_err(|e| e.to_string())
}

// ============================================================================
// Client Marketplace Access Commands
// ============================================================================

/// Set client marketplace access
#[tauri::command]
pub async fn set_client_marketplace_enabled(
    client_id: String,
    enabled: bool,
    config_manager: State<'_, ConfigManager>,
) -> Result<(), String> {
    config_manager
        .update(|cfg| {
            if let Some(client) = cfg.clients.iter_mut().find(|c| c.id == client_id) {
                client.marketplace_enabled = enabled;
            }

            // If enabling for any client, also enable marketplace globally
            if enabled {
                cfg.marketplace.enabled = true;
            }
        })
        .map_err(|e| e.to_string())?;

    config_manager.save().await.map_err(|e| e.to_string())?;

    Ok(())
}

/// Get client marketplace access
#[tauri::command]
pub async fn get_client_marketplace_enabled(
    client_id: String,
    config_manager: State<'_, ConfigManager>,
) -> Result<bool, String> {
    let config = config_manager.get();
    let client = config
        .clients
        .iter()
        .find(|c| c.id == client_id)
        .ok_or_else(|| format!("Client '{}' not found", client_id))?;

    Ok(client.marketplace_enabled)
}
