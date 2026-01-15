//! Tauri command handlers
//!
//! Functions exposed to the frontend via Tauri IPC.

use std::collections::HashMap;
use std::sync::Arc;

use crate::api_keys::ApiKeyManager;
use crate::config::{ConfigManager, ModelSelection, RouterConfig};
use crate::providers::registry::ProviderRegistry;
use serde::{Deserialize, Serialize};
use tauri::{Emitter, State};

/// API key information for display
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKeyInfo {
    pub id: String,
    pub name: String,
    pub model_selection: ModelSelection,
    pub enabled: bool,
    pub created_at: String,
}

/// List all API keys
#[tauri::command]
pub async fn list_api_keys(key_manager: State<'_, ApiKeyManager>) -> Result<Vec<ApiKeyInfo>, String> {
    let keys = key_manager.list_keys();
    Ok(keys
        .into_iter()
        .map(|k| ApiKeyInfo {
            id: k.id,
            name: k.name,
            model_selection: k.model_selection,
            enabled: k.enabled,
            created_at: k.created_at.to_rfc3339(),
        })
        .collect())
}

/// Create a new API key
#[tauri::command]
pub async fn create_api_key(
    name: Option<String>,
    model_selection: ModelSelection,
    key_manager: State<'_, ApiKeyManager>,
    config_manager: State<'_, ConfigManager>,
    app: tauri::AppHandle,
) -> Result<(String, ApiKeyInfo), String> {
    let (key, config) = key_manager
        .create_key(name, model_selection)
        .await
        .map_err(|e| e.to_string())?;

    // Save to config file
    config_manager
        .update(|cfg| {
            cfg.api_keys.push(config.clone());
        })
        .map_err(|e| e.to_string())?;

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

    // Rebuild tray menu
    if let Err(e) = crate::ui::tray::rebuild_tray_menu(&app) {
        tracing::error!("Failed to rebuild tray menu: {}", e);
    }

    Ok(())
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
    instance_name: String,
    provider_type: String,
    config: HashMap<String, String>,
) -> Result<(), String> {
    registry
        .create_provider(instance_name, provider_type, config)
        .await
        .map_err(|e| e.to_string())
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
    instance_name: String,
) -> Result<(), String> {
    registry
        .remove_provider(&instance_name)
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
    instance_name: String,
    enabled: bool,
) -> Result<(), String> {
    registry
        .set_provider_enabled(&instance_name, enabled)
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
///
/// # Note
/// Changes are saved to configuration file but the server needs to be restarted for them to take effect.
/// Use `restart_server` command after calling this.
#[tauri::command]
pub async fn update_server_config(
    host: Option<String>,
    port: Option<u16>,
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
        })
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
