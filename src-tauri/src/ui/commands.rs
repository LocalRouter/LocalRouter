//! Tauri command handlers
//!
//! Functions exposed to the frontend via Tauri IPC.

use std::collections::HashMap;
use std::sync::Arc;

use crate::api_keys::ApiKeyManager;
use crate::config::{ActiveRoutingStrategy, ConfigManager, ModelSelection, ModelRoutingConfig, RouterConfig};
use crate::providers::registry::ProviderRegistry;
use serde::{Deserialize, Serialize};
use tauri::{Emitter, Manager, State};

/// API key information for display
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKeyInfo {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_selection: Option<ModelSelection>,
    pub enabled: bool,
    pub created_at: String,
}

/// List all API keys
#[tauri::command]
pub async fn list_api_keys(key_manager: State<'_, ApiKeyManager>) -> Result<Vec<ApiKeyInfo>, String> {
    let keys = key_manager.list_keys();
    // TODO: DELETE THIS DEBUG LOG LATER
    tracing::warn!("📋 LIST_KEYS: {} keys", keys.len());
    Ok(keys
        .into_iter()
        .map(|k| ApiKeyInfo {
            id: k.id.clone(),
            name: k.name.clone(),
            model_selection: k.model_selection.clone(),
            enabled: k.enabled,
            created_at: k.created_at.to_rfc3339(),
        })
        .collect())
}

/// Create a new API key with optional model selection
#[tauri::command]
pub async fn create_api_key(
    name: Option<String>,
    model_selection: Option<ModelSelection>,
    key_manager: State<'_, ApiKeyManager>,
    config_manager: State<'_, ConfigManager>,
    app: tauri::AppHandle,
) -> Result<(String, ApiKeyInfo), String> {
    tracing::info!("Creating new API key with name: {:?}, model_selection: {:?}", name, model_selection.is_some());

    let (key, mut config) = key_manager
        .create_key(name)
        .await
        .map_err(|e| e.to_string())?;

    tracing::info!("API key created: {} ({})", config.name, config.id);

    // Set model selection if provided
    if model_selection.is_some() {
        config.model_selection = model_selection.clone();
        // Update in-memory key manager
        key_manager
            .update_key(&config.id, |cfg| {
                cfg.model_selection = model_selection.clone();
            })
            .map_err(|e| e.to_string())?;
    }

    // Save to config file
    tracing::warn!("📝 BEFORE UPDATE: about to add key to config");
    config_manager
        .update(|cfg| {
            cfg.api_keys.push(config.clone());
        })
        .map_err(|e| {
            tracing::error!("UPDATE FAILED: {}", e);
            e.to_string()
        })?;
    tracing::warn!("📝 AFTER UPDATE: key added to config in memory");

    // Persist to disk
    tracing::warn!("📝 BEFORE SAVE: about to save config to disk");
    config_manager
        .save()
        .await
        .map_err(|e| {
            tracing::error!("SAVE FAILED: {}", e);
            e.to_string()
        })?;
    tracing::warn!("📝 AFTER SAVE: config saved to disk");

    // Rebuild tray menu with new API key
    if let Err(e) = crate::ui::tray::rebuild_tray_menu(&app) {
        tracing::error!("Failed to rebuild tray menu: {}", e);
    }

    Ok((
        key,
        ApiKeyInfo {
            id: config.id,
            name: config.name,
            model_selection: config.model_selection,
            enabled: config.enabled,
            created_at: config.created_at.to_rfc3339(),
        },
    ))
}

/// Get the actual API key value from keychain
///
/// # Arguments
/// * `id` - The API key ID
///
/// # Returns
/// * The actual API key string if it exists
/// * Error if key doesn't exist or keychain access fails
#[tauri::command]
pub async fn get_api_key_value(
    id: String,
    key_manager: State<'_, ApiKeyManager>,
) -> Result<String, String> {
    key_manager
        .get_key_value(&id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("API key not found in keychain: {}", id))
}

/// Delete an API key
///
/// # Arguments
/// * `id` - The API key ID to delete
///
/// # Returns
/// * Ok(()) if the key was deleted successfully
/// * Error if the key doesn't exist or deletion fails
#[tauri::command]
pub async fn delete_api_key(
    id: String,
    key_manager: State<'_, ApiKeyManager>,
    config_manager: State<'_, ConfigManager>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    // Delete from keychain
    key_manager
        .delete_key(&id)
        .map_err(|e| e.to_string())?;

    // Remove from config file
    config_manager
        .update(|cfg| {
            cfg.api_keys.retain(|k| k.id != id);
        })
        .map_err(|e| e.to_string())?;

    // Persist to disk
    config_manager
        .save()
        .await
        .map_err(|e| e.to_string())?;

    // Rebuild tray menu
    if let Err(e) = crate::ui::tray::rebuild_tray_menu(&app) {
        tracing::error!("Failed to rebuild tray menu: {}", e);
    }

    Ok(())
}

/// Update an API key's model selection
///
/// # Arguments
/// * `id` - The API key ID to update
/// * `model_selection` - The new model selection (or None to clear it)
///
/// # Returns
/// * The updated API key info if successful
/// * Error if the key doesn't exist or update fails
#[tauri::command]
pub async fn update_api_key_model(
    id: String,
    model_selection: Option<ModelSelection>,
    key_manager: State<'_, ApiKeyManager>,
    config_manager: State<'_, ConfigManager>,
) -> Result<ApiKeyInfo, String> {
    // Update in memory
    let updated_config = key_manager
        .update_key(&id, |cfg| {
            cfg.model_selection = model_selection.clone();
        })
        .map_err(|e| e.to_string())?;

    // Update in config file
    config_manager
        .update(|cfg| {
            if let Some(key) = cfg.api_keys.iter_mut().find(|k| k.id == id) {
                key.model_selection = model_selection.clone();
            }
        })
        .map_err(|e| e.to_string())?;

    // Persist to disk
    config_manager
        .save()
        .await
        .map_err(|e| e.to_string())?;

    Ok(ApiKeyInfo {
        id: updated_config.id,
        name: updated_config.name,
        model_selection: updated_config.model_selection,
        enabled: updated_config.enabled,
        created_at: updated_config.created_at.to_rfc3339(),
    })
}

/// Update an API key's name
///
/// # Arguments
/// * `id` - The API key ID to update
/// * `name` - The new name for the API key
///
/// # Returns
/// * Ok(()) if the update succeeded
/// * Error if the key doesn't exist or update fails
#[tauri::command]
pub async fn update_api_key_name(
    id: String,
    name: String,
    key_manager: State<'_, ApiKeyManager>,
    config_manager: State<'_, ConfigManager>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    // Validate name is not empty
    if name.trim().is_empty() {
        return Err("API key name cannot be empty".to_string());
    }

    // Update in memory
    key_manager
        .update_key(&id, |cfg| {
            cfg.name = name.clone();
        })
        .map_err(|e| e.to_string())?;

    // Update in config file
    config_manager
        .update(|cfg| {
            if let Some(key) = cfg.api_keys.iter_mut().find(|k| k.id == id) {
                key.name = name.clone();
            }
        })
        .map_err(|e| e.to_string())?;

    // Persist to disk
    config_manager
        .save()
        .await
        .map_err(|e| e.to_string())?;

    // Rebuild tray menu to show updated name
    if let Err(e) = crate::ui::tray::rebuild_tray_menu(&app) {
        tracing::error!("Failed to rebuild tray menu: {}", e);
    }

    Ok(())
}

/// Toggle an API key's enabled state
///
/// # Arguments
/// * `id` - The API key ID to toggle
/// * `enabled` - Whether to enable (true) or disable (false) the key
///
/// # Returns
/// * Ok(()) if the toggle succeeded
/// * Error if the key doesn't exist or toggle fails
#[tauri::command]
pub async fn toggle_api_key_enabled(
    id: String,
    enabled: bool,
    key_manager: State<'_, ApiKeyManager>,
    config_manager: State<'_, ConfigManager>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    // Update in memory
    key_manager
        .update_key(&id, |cfg| {
            cfg.enabled = enabled;
        })
        .map_err(|e| e.to_string())?;

    // Update in config file
    config_manager
        .update(|cfg| {
            if let Some(key) = cfg.api_keys.iter_mut().find(|k| k.id == id) {
                key.enabled = enabled;
            }
        })
        .map_err(|e| e.to_string())?;

    // Persist to disk
    config_manager
        .save()
        .await
        .map_err(|e| e.to_string())?;

    // Rebuild tray menu to show updated enabled/disabled state
    if let Err(e) = crate::ui::tray::rebuild_tray_menu(&app) {
        tracing::error!("Failed to rebuild tray menu: {}", e);
    }

    Ok(())
}

/// Rotate an API key
///
/// Generates a new API key value while keeping the same ID, name, and settings.
/// The old key is immediately invalidated.
///
/// # Arguments
/// * `id` - The API key ID to rotate
///
/// # Returns
/// * The new API key string if rotation succeeded
/// * Error if the key doesn't exist or rotation fails
#[tauri::command]
pub async fn rotate_api_key(
    id: String,
    key_manager: State<'_, ApiKeyManager>,
) -> Result<String, String> {
    key_manager
        .rotate_key(&id)
        .await
        .map_err(|e| e.to_string())
}

/// List all routers
#[tauri::command]
pub async fn list_routers(config_manager: State<'_, ConfigManager>) -> Result<Vec<RouterConfig>, String> {
    let config = config_manager.get();
    Ok(config.routers)
}

/// Get current configuration
#[tauri::command]
pub async fn get_config(config_manager: State<'_, ConfigManager>) -> Result<serde_json::Value, String> {
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
// Provider API Key Management Commands
// ============================================================================

/// Store a provider API key in the system keyring
///
/// # Arguments
/// * `provider` - Provider name (e.g., "openai", "anthropic", "gemini")
/// * `api_key` - The API key to store securely
///
/// # Security
/// The API key is stored directly in the system keyring:
/// - macOS: Keychain (may prompt for Touch ID/password)
/// - Windows: Credential Manager
/// - Linux: Secret Service
#[tauri::command]
pub async fn set_provider_api_key(provider: String, api_key: String) -> Result<(), String> {
    crate::providers::key_storage::store_provider_key(&provider, &api_key)
        .map_err(|e| e.to_string())
}

/// Check if a provider has an API key stored
///
/// # Arguments
/// * `provider` - Provider name to check
///
/// # Returns
/// * `true` if the provider has an API key stored in the system keyring
/// * `false` if no key is stored
///
/// # Security
/// This command only returns whether a key exists, not the actual key value.
#[tauri::command]
pub async fn has_provider_api_key(provider: String) -> Result<bool, String> {
    crate::providers::key_storage::has_provider_key(&provider)
        .map_err(|e| e.to_string())
}

/// Delete a provider API key from the system keyring
///
/// # Arguments
/// * `provider` - Provider name whose key should be deleted
///
/// # Returns
/// * `Ok(())` if successful (even if the key didn't exist)
#[tauri::command]
pub async fn delete_provider_api_key(provider: String) -> Result<(), String> {
    crate::providers::key_storage::delete_provider_key(&provider)
        .map_err(|e| e.to_string())
}

/// List all providers (from config) with their key status
///
/// Returns a list of provider names from the configuration along with
/// whether each has an API key stored in the system keyring.
#[tauri::command]
pub async fn list_providers_with_key_status(
    config_manager: State<'_, ConfigManager>,
) -> Result<Vec<ProviderKeyStatus>, String> {
    let config = config_manager.get();

    let mut result = Vec::new();
    for provider_config in config.providers {
        // Check if key exists for this provider
        let key_ref = provider_config.api_key_ref.as_deref()
            .unwrap_or(&provider_config.name)
            .to_string();

        let has_key = crate::providers::key_storage::has_provider_key(&key_ref)
            .unwrap_or(false);

        result.push(ProviderKeyStatus {
            name: provider_config.name,
            provider_type: format!("{:?}", provider_config.provider_type),
            enabled: provider_config.enabled,
            has_api_key: has_key,
            key_ref,
        });
    }

    Ok(result)
}

/// Provider key status information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderKeyStatus {
    pub name: String,
    pub provider_type: String,
    pub enabled: bool,
    pub has_api_key: bool,
    pub key_ref: String,
}

// ============================================================================
// Provider Registry Management Commands
// ============================================================================

/// List all available provider types with their setup parameters
///
/// Used by the UI to show available provider types when adding a new provider.
/// Returns factory information for all registered provider types.
#[tauri::command]
pub async fn list_provider_types(
    registry: State<'_, Arc<ProviderRegistry>>,
) -> Result<Vec<crate::providers::registry::ProviderTypeInfo>, String> {
    Ok(registry.list_provider_types())
}

/// List all provider instances
///
/// Returns information about all registered provider instances,
/// including their status (enabled/disabled).
#[tauri::command]
pub async fn list_provider_instances(
    registry: State<'_, Arc<ProviderRegistry>>,
) -> Result<Vec<crate::providers::registry::ProviderInstanceInfo>, String> {
    Ok(registry.list_providers())
}

/// Create a new provider instance
///
/// # Arguments
/// * `instance_name` - User-defined name for this provider instance
/// * `provider_type` - Type of provider (e.g., "ollama", "openai", "anthropic")
/// * `config` - Configuration parameters (e.g., {"api_key": "sk-...", "base_url": "..."})
///
/// # Returns
/// * `Ok(())` if the provider was created successfully
/// * `Err(String)` with error message if creation failed
#[tauri::command]
pub async fn create_provider_instance(
    registry: State<'_, Arc<ProviderRegistry>>,
    config_manager: State<'_, ConfigManager>,
    instance_name: String,
    provider_type: String,
    config: HashMap<String, String>,
) -> Result<(), String> {
    // Create provider in registry (in-memory)
    registry
        .create_provider(instance_name.clone(), provider_type.clone(), config.clone())
        .await
        .map_err(|e| e.to_string())?;

    // Save to config file for persistence
    config_manager
        .update(|cfg| {
            // Convert provider_type string to ProviderType enum
            let provider_type_enum = provider_type_str_to_enum(&provider_type);

            // Convert config HashMap to provider_config JSON
            let provider_config = if !config.is_empty() {
                Some(serde_json::to_value(&config).unwrap_or(serde_json::Value::Null))
            } else {
                None
            };

            cfg.providers.push(crate::config::ProviderConfig {
                name: instance_name.clone(),
                provider_type: provider_type_enum,
                enabled: true,
                provider_config,
                api_key_ref: None,
            });
        })
        .map_err(|e| e.to_string())?;

    // Persist to disk
    config_manager
        .save()
        .await
        .map_err(|e| e.to_string())
}

/// Get provider instance configuration
///
/// # Arguments
/// * `instance_name` - Name of the provider instance
///
/// # Returns
/// * `Ok(HashMap<String, String>)` with the provider's configuration
/// * `Err(String)` if the provider doesn't exist
#[tauri::command]
pub async fn get_provider_config(
    registry: State<'_, Arc<ProviderRegistry>>,
    instance_name: String,
) -> Result<HashMap<String, String>, String> {
    registry
        .get_provider_config(&instance_name)
        .ok_or_else(|| format!("Provider instance '{}' not found", instance_name))
}

/// Update an existing provider instance
///
/// # Arguments
/// * `instance_name` - Name of the provider instance to update
/// * `provider_type` - Type of provider (must match the existing type)
/// * `config` - Updated configuration parameters
///
/// # Returns
/// * `Ok(())` if the provider was updated successfully
/// * `Err(String)` with error message if update failed
#[tauri::command]
pub async fn update_provider_instance(
    registry: State<'_, Arc<ProviderRegistry>>,
    config_manager: State<'_, ConfigManager>,
    instance_name: String,
    provider_type: String,
    config: HashMap<String, String>,
) -> Result<(), String> {
    // Update provider in registry (in-memory)
    registry
        .update_provider(instance_name.clone(), provider_type.clone(), config.clone())
        .await
        .map_err(|e| e.to_string())?;

    // Update in config file
    config_manager
        .update(|cfg| {
            if let Some(provider) = cfg.providers.iter_mut().find(|p| p.name == instance_name) {
                provider.provider_type = provider_type_str_to_enum(&provider_type);
                provider.provider_config = if !config.is_empty() {
                    Some(serde_json::to_value(&config).unwrap_or(serde_json::Value::Null))
                } else {
                    None
                };
            }
        })
        .map_err(|e| e.to_string())?;

    // Persist to disk
    config_manager
        .save()
        .await
        .map_err(|e| e.to_string())
}

/// Helper function to convert provider type string to enum
fn provider_type_str_to_enum(provider_type: &str) -> crate::config::ProviderType {
    match provider_type {
        "ollama" => crate::config::ProviderType::Ollama,
        "openai" => crate::config::ProviderType::OpenAI,
        "anthropic" => crate::config::ProviderType::Anthropic,
        "gemini" => crate::config::ProviderType::Gemini,
        "openrouter" => crate::config::ProviderType::OpenRouter,
        "groq" => crate::config::ProviderType::Groq,
        "mistral" => crate::config::ProviderType::Mistral,
        "cohere" => crate::config::ProviderType::Cohere,
        "togetherai" => crate::config::ProviderType::TogetherAI,
        "perplexity" => crate::config::ProviderType::Perplexity,
        "deepinfra" => crate::config::ProviderType::DeepInfra,
        "cerebras" => crate::config::ProviderType::Cerebras,
        "xai" => crate::config::ProviderType::XAI,
        "openai_compatible" => crate::config::ProviderType::Custom,
        _ => crate::config::ProviderType::Custom,
    }
}

/// Remove a provider instance
///
/// # Arguments
/// * `instance_name` - Name of the provider instance to remove
///
/// # Returns
/// * `Ok(())` if the provider was removed successfully
/// * `Err(String)` if the provider doesn't exist
#[tauri::command]
pub async fn remove_provider_instance(
    registry: State<'_, Arc<ProviderRegistry>>,
    config_manager: State<'_, ConfigManager>,
    instance_name: String,
) -> Result<(), String> {
    // Remove from registry (in-memory)
    registry
        .remove_provider(&instance_name)
        .map_err(|e| e.to_string())?;

    // Remove from config file
    config_manager
        .update(|cfg| {
            cfg.providers.retain(|p| p.name != instance_name);
        })
        .map_err(|e| e.to_string())?;

    // Persist to disk
    config_manager
        .save()
        .await
        .map_err(|e| e.to_string())
}

/// Enable or disable a provider instance
///
/// # Arguments
/// * `instance_name` - Name of the provider instance
/// * `enabled` - Whether to enable (true) or disable (false) the provider
///
/// # Returns
/// * `Ok(())` if the provider state was updated successfully
/// * `Err(String)` if the provider doesn't exist
#[tauri::command]
pub async fn set_provider_enabled(
    registry: State<'_, Arc<ProviderRegistry>>,
    config_manager: State<'_, ConfigManager>,
    instance_name: String,
    enabled: bool,
) -> Result<(), String> {
    // Update in registry (in-memory)
    registry
        .set_provider_enabled(&instance_name, enabled)
        .map_err(|e| e.to_string())?;

    // Update in config file
    config_manager
        .update(|cfg| {
            if let Some(provider) = cfg.providers.iter_mut().find(|p| p.name == instance_name) {
                provider.enabled = enabled;
            }
        })
        .map_err(|e| e.to_string())?;

    // Persist to disk
    config_manager
        .save()
        .await
        .map_err(|e| e.to_string())
}

/// Get health status for all provider instances
///
/// Returns a map of provider names to their health status.
/// Includes latency, status (healthy/degraded/unhealthy), and error messages.
#[tauri::command]
pub async fn get_providers_health(
    registry: State<'_, Arc<ProviderRegistry>>,
) -> Result<HashMap<String, crate::providers::ProviderHealth>, String> {
    Ok(registry.get_all_health().await)
}

/// List models from a specific provider instance
///
/// # Arguments
/// * `instance_name` - Name of the provider instance
///
/// # Returns
/// * `Ok(Vec<ModelInfo>)` with the list of available models
/// * `Err(String)` if the provider doesn't exist or model listing failed
#[tauri::command]
pub async fn list_provider_models(
    registry: State<'_, Arc<ProviderRegistry>>,
    instance_name: String,
) -> Result<Vec<crate::providers::ModelInfo>, String> {
    registry
        .list_provider_models(&instance_name)
        .await
        .map_err(|e| e.to_string())
}

/// List all models from all enabled providers
///
/// Returns a combined list of all models available across all enabled providers.
/// Used by the UI to populate the model selection dropdown.
///
/// # Returns
/// * `Ok(Vec<ModelInfo>)` with the aggregated list of models
/// * Models are grouped by provider
#[tauri::command]
pub async fn list_all_models(
    registry: State<'_, Arc<ProviderRegistry>>,
) -> Result<Vec<crate::providers::ModelInfo>, String> {
    registry
        .list_all_models()
        .await
        .map_err(|e| e.to_string())
}

// ============================================================================
// Server Configuration Commands
// ============================================================================

/// Get server configuration (host and port)
#[tauri::command]
pub async fn get_server_config(
    config_manager: State<'_, ConfigManager>,
) -> Result<ServerConfigInfo, String> {
    let config = config_manager.get();
    Ok(ServerConfigInfo {
        host: config.server.host.clone(),
        port: config.server.port,
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
    config_manager
        .save()
        .await
        .map_err(|e| e.to_string())
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
    pub enable_cors: bool,
}

// ============================================================================
// Monitoring & Statistics Commands
// ============================================================================

/// Get aggregate statistics (requests, tokens, cost)
///
/// Returns statistics computed from all tracked generations in the retention period.
#[tauri::command]
pub async fn get_aggregate_stats(
    server_manager: State<'_, Arc<crate::server::ServerManager>>,
) -> Result<crate::server::state::AggregateStats, String> {
    // Get the app state from server manager
    let app_state = server_manager
        .get_state()
        .ok_or_else(|| "Server is not running".to_string())?;

    Ok(app_state.generation_tracker.get_stats())
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

// ============================================================================
// Server Control Commands
// ============================================================================

/// Get the current server status
#[tauri::command]
pub async fn get_server_status(
    server_manager: State<'_, Arc<crate::server::ServerManager>>,
) -> Result<String, String> {
    let status = server_manager.get_status();
    Ok(match status {
        crate::server::ServerStatus::Stopped => "stopped".to_string(),
        crate::server::ServerStatus::Running => "running".to_string(),
    })
}

/// Start the web server
#[tauri::command]
pub async fn start_server(
    server_manager: State<'_, Arc<crate::server::ServerManager>>,
    config_manager: State<'_, ConfigManager>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    tracing::info!("Start server command received");

    // Get dependencies from app state
    let router = app.state::<Arc<crate::router::Router>>();
    let api_key_manager = app.state::<ApiKeyManager>();
    let rate_limiter = app.state::<Arc<crate::router::RateLimiterManager>>();
    let provider_registry = app.state::<Arc<ProviderRegistry>>();

    // Get server config from configuration
    let server_config = {
        let config = config_manager.get();
        crate::server::ServerConfig {
            host: config.server.host.clone(),
            port: config.server.port,
            enable_cors: config.server.enable_cors,
        }
    };

    // Start the server
    server_manager
        .start(
            server_config,
            router.inner().clone(),
            (*api_key_manager.inner()).clone(),
            rate_limiter.inner().clone(),
            provider_registry.inner().clone(),
        )
        .await
        .map_err(|e| format!("Failed to start server: {}", e))?;

    // Emit event to update tray icon
    let _ = app.emit("server-status-changed", "running");

    Ok(())
}

/// Stop the web server
#[tauri::command]
pub async fn stop_server(
    server_manager: State<'_, Arc<crate::server::ServerManager>>,
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
    oauth_manager: State<'_, Arc<crate::providers::oauth::OAuthManager>>,
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
    oauth_manager: State<'_, Arc<crate::providers::oauth::OAuthManager>>,
) -> Result<crate::providers::oauth::OAuthFlowResult, String> {
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
    oauth_manager: State<'_, Arc<crate::providers::oauth::OAuthManager>>,
) -> Result<crate::providers::oauth::OAuthFlowResult, String> {
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
    oauth_manager: State<'_, Arc<crate::providers::oauth::OAuthManager>>,
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
    oauth_manager: State<'_, Arc<crate::providers::oauth::OAuthManager>>,
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
    oauth_manager: State<'_, Arc<crate::providers::oauth::OAuthManager>>,
) -> Result<(), String> {
    oauth_manager
        .delete_credentials(&provider_id)
        .await
        .map_err(|e| e.to_string())
}

// ============================================================================
// Visualization Commands
// ============================================================================

/// Get the visualization graph data
///
/// Returns a graph structure containing nodes (providers, models, API keys)
/// and edges (relationships between them) for visualization.
#[tauri::command]
pub async fn get_visualization_graph(
    provider_registry: State<'_, Arc<ProviderRegistry>>,
    key_manager: State<'_, ApiKeyManager>,
    config_manager: State<'_, ConfigManager>,
) -> Result<crate::ui::visualization::VisualizationGraph, String> {
    // Get all providers
    let providers = provider_registry.list_providers();

    // Get all models from all providers
    let models = provider_registry
        .list_all_models()
        .await
        .map_err(|e| e.to_string())?;

    // Get health statuses
    let health_statuses = provider_registry.get_all_health().await;

    // Get API keys
    let api_keys = list_api_keys(key_manager.clone()).await?;

    // Get full API key configs (with routing info)
    let config = config_manager.get();
    let api_key_configs = config.api_keys.clone();

    // Build the graph
    let graph = crate::ui::visualization::build_graph(
        providers,
        models,
        api_keys,
        api_key_configs,
        health_statuses,
    );

    Ok(graph)
}

// ============================================================================
// Routing Strategy Commands
// ============================================================================

/// Get the routing configuration for an API key
///
/// # Arguments
/// * `id` - The API key ID
///
/// # Returns
/// * The routing configuration if it exists, or None
#[tauri::command]
pub async fn get_routing_config(
    id: String,
    key_manager: State<'_, ApiKeyManager>,
) -> Result<Option<ModelRoutingConfig>, String> {
    let key = key_manager
        .get_key(&id)
        .ok_or_else(|| format!("API key not found: {}", id))?;

    Ok(key.get_routing_config())
}

/// Update the prioritized models list for an API key
///
/// # Arguments
/// * `id` - The API key ID
/// * `prioritized_models` - The new prioritized models list as (provider, model) pairs
///
/// # Returns
/// * Ok(()) if successful
#[tauri::command]
pub async fn update_prioritized_list(
    id: String,
    prioritized_models: Vec<(String, String)>,
    key_manager: State<'_, ApiKeyManager>,
    config_manager: State<'_, ConfigManager>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    tracing::info!(
        "Updating prioritized list for key {}: {} models",
        id,
        prioritized_models.len()
    );

    // Get or create routing config
    let current_key = key_manager
        .get_key(&id)
        .ok_or_else(|| format!("API key not found: {}", id))?;

    let mut routing_config = current_key
        .get_routing_config()
        .unwrap_or_else(|| ModelRoutingConfig::new_prioritized_list(prioritized_models.clone()));

    // Update prioritized models
    routing_config.prioritized_models = prioritized_models;

    // Update in memory
    key_manager
        .update_key(&id, |cfg| {
            cfg.routing_config = Some(routing_config.clone());
        })
        .map_err(|e| e.to_string())?;

    // Update in config file
    config_manager
        .update(|cfg| {
            if let Some(key) = cfg.api_keys.iter_mut().find(|k| k.id == id) {
                key.routing_config = Some(routing_config);
            }
        })
        .map_err(|e| e.to_string())?;

    // Persist to disk
    config_manager
        .save()
        .await
        .map_err(|e| e.to_string())?;

    // Rebuild tray menu
    if let Err(e) = crate::ui::tray::rebuild_tray_menu(&app) {
        tracing::error!("Failed to rebuild tray menu: {}", e);
    }

    tracing::info!("Prioritized list updated for key {}", id);

    Ok(())
}

/// Set the active routing strategy for an API key
///
/// # Arguments
/// * `id` - The API key ID
/// * `strategy` - The routing strategy to activate ("available_models", "force_model", "prioritized_list")
///
/// # Returns
/// * Ok(()) if successful
#[tauri::command]
pub async fn set_routing_strategy(
    id: String,
    strategy: String,
    key_manager: State<'_, ApiKeyManager>,
    config_manager: State<'_, ConfigManager>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    tracing::info!("Setting routing strategy for key {}: {}", id, strategy);

    // Parse strategy
    let active_strategy = match strategy.as_str() {
        "available_models" => ActiveRoutingStrategy::AvailableModels,
        "force_model" => ActiveRoutingStrategy::ForceModel,
        "prioritized_list" => ActiveRoutingStrategy::PrioritizedList,
        _ => return Err(format!("Invalid routing strategy: {}", strategy)),
    };

    // Get or create routing config
    let current_key = key_manager
        .get_key(&id)
        .ok_or_else(|| format!("API key not found: {}", id))?;

    let mut routing_config = current_key
        .get_routing_config()
        .unwrap_or_else(|| ModelRoutingConfig::new_available_models());

    // Update strategy
    routing_config.active_strategy = active_strategy;

    // Update in memory
    key_manager
        .update_key(&id, |cfg| {
            cfg.routing_config = Some(routing_config.clone());
        })
        .map_err(|e| e.to_string())?;

    // Update in config file
    config_manager
        .update(|cfg| {
            if let Some(key) = cfg.api_keys.iter_mut().find(|k| k.id == id) {
                key.routing_config = Some(routing_config);
            }
        })
        .map_err(|e| e.to_string())?;

    // Persist to disk
    config_manager
        .save()
        .await
        .map_err(|e| e.to_string())?;

    // Rebuild tray menu
    if let Err(e) = crate::ui::tray::rebuild_tray_menu(&app) {
        tracing::error!("Failed to rebuild tray menu: {}", e);
    }

    tracing::info!("Routing strategy set for key {}: {:?}", id, active_strategy);

    Ok(())
}
