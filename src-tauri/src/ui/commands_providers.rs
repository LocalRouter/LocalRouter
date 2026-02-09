//! Provider-related Tauri command handlers
//!
//! Provider API key management, registry management, health cache, and model listing.

use std::collections::HashMap;
use std::sync::Arc;

use lr_config::ConfigManager;
use lr_providers::registry::ProviderRegistry;
use serde::{Deserialize, Serialize};
use tauri::{Emitter, State};

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
    lr_providers::key_storage::store_provider_key(&provider, &api_key).map_err(|e| e.to_string())
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
    lr_providers::key_storage::has_provider_key(&provider).map_err(|e| e.to_string())
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
    lr_providers::key_storage::delete_provider_key(&provider).map_err(|e| e.to_string())
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
        let key_ref = provider_config
            .api_key_ref
            .as_deref()
            .unwrap_or(&provider_config.name)
            .to_string();

        let has_key = lr_providers::key_storage::has_provider_key(&key_ref).unwrap_or(false);

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
) -> Result<Vec<lr_providers::registry::ProviderTypeInfo>, String> {
    Ok(registry.list_provider_types())
}

/// List all provider instances
///
/// Returns information about all registered provider instances,
/// including their status (enabled/disabled).
#[tauri::command]
pub async fn list_provider_instances(
    registry: State<'_, Arc<ProviderRegistry>>,
) -> Result<Vec<lr_providers::registry::ProviderInstanceInfo>, String> {
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
    app: tauri::AppHandle,
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

            cfg.providers.push(lr_config::ProviderConfig {
                name: instance_name.clone(),
                provider_type: provider_type_enum,
                enabled: true,
                provider_config,
                api_key_ref: None,
            });
        })
        .map_err(|e| e.to_string())?;

    // Persist to disk
    config_manager.save().await.map_err(|e| e.to_string())?;

    // Notify frontend that providers and models changed
    let _ = app.emit("providers-changed", ());
    let _ = app.emit("models-changed", ());

    Ok(())
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
    app: tauri::AppHandle,
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
    config_manager.save().await.map_err(|e| e.to_string())?;

    // Notify frontend that providers and models changed
    let _ = app.emit("providers-changed", ());
    let _ = app.emit("models-changed", ());

    Ok(())
}

/// Rename a provider instance
///
/// # Arguments
/// * `instance_name` - Current name of the provider instance
/// * `new_name` - New name for the provider instance
///
/// # Returns
/// * `Ok(())` if the provider was renamed successfully
/// * `Err(String)` with error message if rename failed
#[tauri::command]
pub async fn rename_provider_instance(
    registry: State<'_, Arc<ProviderRegistry>>,
    config_manager: State<'_, ConfigManager>,
    app_state: State<'_, Arc<lr_server::state::AppState>>,
    app: tauri::AppHandle,
    instance_name: String,
    new_name: String,
) -> Result<(), String> {
    let new_name = new_name.trim().to_string();
    if instance_name == new_name {
        return Ok(());
    }
    if new_name.is_empty() {
        return Err("Provider name cannot be empty".to_string());
    }

    // Check new name doesn't conflict
    let instances = registry.list_providers();
    if instances.iter().any(|i| i.instance_name == new_name) {
        return Err(format!("Provider '{}' already exists", new_name));
    }

    // Get current state
    let old_info = instances
        .iter()
        .find(|i| i.instance_name == instance_name)
        .ok_or_else(|| format!("Provider '{}' not found", instance_name))?;
    let provider_type = old_info.provider_type.clone();
    let was_enabled = old_info.enabled;

    let config = registry
        .get_provider_config(&instance_name)
        .ok_or_else(|| format!("Provider '{}' config not found", instance_name))?;

    // Remove old instance from registry
    registry
        .remove_provider(&instance_name)
        .map_err(|e| e.to_string())?;

    // Create new instance with new name
    registry
        .create_provider(new_name.clone(), provider_type, config)
        .await
        .map_err(|e| e.to_string())?;

    // Restore enabled state
    if !was_enabled {
        let _ = registry.set_provider_enabled(&new_name, false);
    }

    // Update config file
    config_manager
        .update(|cfg| {
            if let Some(provider) = cfg.providers.iter_mut().find(|p| p.name == instance_name) {
                provider.name = new_name.clone();
            }
        })
        .map_err(|e| e.to_string())?;

    // Persist to disk
    config_manager.save().await.map_err(|e| e.to_string())?;

    // Update health cache
    app_state.health_cache.remove_provider(&instance_name);

    // Notify frontend that providers and models changed
    let _ = app.emit("providers-changed", ());
    let _ = app.emit("models-changed", ());

    Ok(())
}

/// Retrieve a provider API key from the system keyring
///
/// Resolves the correct keyring lookup name from the provider's config
/// (uses api_key_ref if set, otherwise the provider name).
///
/// # Arguments
/// * `instance_name` - Name of the provider instance
///
/// # Returns
/// * `Ok(Some(key))` if key exists
/// * `Ok(None)` if no key is stored
#[tauri::command]
pub async fn get_provider_api_key(
    config_manager: State<'_, ConfigManager>,
    instance_name: String,
) -> Result<Option<String>, String> {
    let config = config_manager.get();
    let key_ref = config
        .providers
        .iter()
        .find(|p| p.name == instance_name)
        .map(|p| p.api_key_ref.as_deref().unwrap_or(&p.name).to_string())
        .unwrap_or(instance_name);

    lr_providers::key_storage::get_provider_key(&key_ref).map_err(|e| e.to_string())
}

/// Helper function to convert provider type string to enum
fn provider_type_str_to_enum(provider_type: &str) -> lr_config::ProviderType {
    match provider_type {
        "ollama" => lr_config::ProviderType::Ollama,
        "lmstudio" => lr_config::ProviderType::LMStudio,
        "openai" => lr_config::ProviderType::OpenAI,
        "anthropic" => lr_config::ProviderType::Anthropic,
        "gemini" => lr_config::ProviderType::Gemini,
        "openrouter" => lr_config::ProviderType::OpenRouter,
        "groq" => lr_config::ProviderType::Groq,
        "mistral" => lr_config::ProviderType::Mistral,
        "cohere" => lr_config::ProviderType::Cohere,
        "togetherai" => lr_config::ProviderType::TogetherAI,
        "perplexity" => lr_config::ProviderType::Perplexity,
        "deepinfra" => lr_config::ProviderType::DeepInfra,
        "cerebras" => lr_config::ProviderType::Cerebras,
        "xai" => lr_config::ProviderType::XAI,
        "openai_compatible" => lr_config::ProviderType::Custom,
        _ => lr_config::ProviderType::Custom,
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
    app_state: State<'_, Arc<lr_server::state::AppState>>,
    app: tauri::AppHandle,
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
    config_manager.save().await.map_err(|e| e.to_string())?;

    // Remove from health cache (this emits health-status-changed event)
    app_state.health_cache.remove_provider(&instance_name);

    // Notify frontend that providers and models changed
    let _ = app.emit("providers-changed", ());
    let _ = app.emit("models-changed", ());

    Ok(())
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
    app_state: State<'_, Arc<lr_server::state::AppState>>,
    app: tauri::AppHandle,
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
    config_manager.save().await.map_err(|e| e.to_string())?;

    // Update health cache immediately - this recalculates aggregate status and emits event
    use lr_providers::health_cache::ItemHealth;
    if enabled {
        // Set to pending, then trigger a health check for this provider
        app_state
            .health_cache
            .update_provider(&instance_name, ItemHealth::pending(instance_name.clone()));

        // Spawn background health check for this provider
        let health_cache = app_state.health_cache.clone();
        let provider_registry = app_state.provider_registry.clone();
        let timeout_secs = config_manager.get().health_check.timeout_secs;
        let name = instance_name.clone();
        tokio::spawn(async move {
            if let Some(provider) = provider_registry.get_provider(&name) {
                let health = tokio::time::timeout(
                    std::time::Duration::from_secs(timeout_secs),
                    provider.health_check(),
                )
                .await;

                let item_health = match health {
                    Ok(h) => {
                        use lr_providers::HealthStatus;
                        match h.status {
                            HealthStatus::Healthy => {
                                ItemHealth::healthy(name.clone(), h.latency_ms)
                            }
                            HealthStatus::Degraded => ItemHealth::degraded(
                                name.clone(),
                                h.latency_ms,
                                h.error_message.unwrap_or_else(|| "Degraded".to_string()),
                            ),
                            HealthStatus::Unhealthy => ItemHealth::unhealthy(
                                name.clone(),
                                h.error_message.unwrap_or_else(|| "Unhealthy".to_string()),
                            ),
                        }
                    }
                    Err(_) => ItemHealth::unhealthy(
                        name.clone(),
                        format!("Health check timeout ({}s)", timeout_secs),
                    ),
                };
                health_cache.update_provider(&name, item_health);
            }
        });
    } else {
        // Set to disabled immediately
        app_state
            .health_cache
            .update_provider(&instance_name, ItemHealth::disabled(instance_name.clone()));
    }

    // Notify frontend that providers and models changed
    let _ = app.emit("providers-changed", ());
    let _ = app.emit("models-changed", ());

    Ok(())
}

/// Get health status for all provider instances
///
/// Returns a map of provider names to their health status.
/// Includes latency, status (healthy/degraded/unhealthy), and error messages.
#[tauri::command]
pub async fn get_providers_health(
    registry: State<'_, Arc<ProviderRegistry>>,
) -> Result<HashMap<String, lr_providers::ProviderHealth>, String> {
    Ok(registry.get_all_health().await)
}

/// Health check result for streaming to frontend
#[derive(Clone, Serialize)]
pub struct HealthCheckResult {
    pub provider_name: String,
    pub status: String,
    pub latency_ms: Option<u64>,
    pub error_message: Option<String>,
}

/// Start streaming health checks for all providers
///
/// Emits "provider-health-check" events as each provider's health check completes.
/// Returns immediately with the list of providers being checked.
#[tauri::command]
pub async fn start_provider_health_checks(
    app: tauri::AppHandle,
    registry: State<'_, Arc<ProviderRegistry>>,
    app_state: State<'_, Arc<lr_server::state::AppState>>,
) -> Result<Vec<String>, String> {
    let provider_names = registry.get_provider_names();

    // Clone what we need for the spawned task
    let registry = registry.inner().clone();
    let app_handle = app.clone();
    let health_cache = app_state.health_cache.clone();

    // Spawn health checks for each provider instance in parallel
    // We check each instance directly (not via HealthCheckManager) to ensure
    // proper instance name mapping even with multiple instances of the same provider type
    tokio::spawn(async move {
        let instance_names = registry.get_provider_names();
        let mut handles = Vec::new();

        for instance_name in instance_names {
            let registry = registry.clone();
            let app_handle = app_handle.clone();
            let health_cache = health_cache.clone();
            let instance_name_clone = instance_name.clone();

            let handle = tokio::spawn(async move {
                // Check if provider is disabled
                if let Some(enabled) = registry.is_provider_enabled(&instance_name_clone) {
                    if !enabled {
                        let result = HealthCheckResult {
                            provider_name: instance_name_clone.clone(),
                            status: "disabled".to_string(),
                            latency_ms: None,
                            error_message: None,
                        };
                        let _ = app_handle.emit("provider-health-check", result);
                        health_cache.update_provider(
                            &instance_name_clone,
                            lr_providers::health_cache::ItemHealth::disabled(
                                instance_name_clone.clone(),
                            ),
                        );
                        return;
                    }
                }

                if let Some(provider) = registry.get_provider_unchecked(&instance_name_clone) {
                    let health = provider.health_check().await;
                    let result = HealthCheckResult {
                        provider_name: instance_name_clone.clone(),
                        status: match &health.status {
                            lr_providers::HealthStatus::Healthy => "healthy".to_string(),
                            lr_providers::HealthStatus::Degraded => "degraded".to_string(),
                            lr_providers::HealthStatus::Unhealthy => "unhealthy".to_string(),
                        },
                        latency_ms: health.latency_ms,
                        error_message: health.error_message.clone(),
                    };
                    let _ = app_handle.emit("provider-health-check", result);

                    // Update centralized health cache so aggregate status (tray + sidebar) recalculates
                    use lr_providers::health_cache::ItemHealth;
                    use lr_providers::HealthStatus;
                    let item_health = match health.status {
                        HealthStatus::Healthy => {
                            ItemHealth::healthy(instance_name_clone.clone(), health.latency_ms)
                        }
                        HealthStatus::Degraded => ItemHealth::degraded(
                            instance_name_clone.clone(),
                            health.latency_ms,
                            health
                                .error_message
                                .unwrap_or_else(|| "Degraded".to_string()),
                        ),
                        HealthStatus::Unhealthy => ItemHealth::unhealthy(
                            instance_name_clone.clone(),
                            health
                                .error_message
                                .unwrap_or_else(|| "Unhealthy".to_string()),
                        ),
                    };
                    health_cache.update_provider(&instance_name_clone, item_health);
                }
            });
            handles.push(handle);
        }

        // Wait for all health checks to complete
        for handle in handles {
            let _ = handle.await;
        }
    });

    Ok(provider_names)
}

/// Check health for a single provider
///
/// Emits "provider-health-check" event when the health check completes.
#[tauri::command]
pub async fn check_single_provider_health(
    app: tauri::AppHandle,
    registry: State<'_, Arc<ProviderRegistry>>,
    app_state: State<'_, Arc<lr_server::state::AppState>>,
    instance_name: String,
) -> Result<(), String> {
    let registry = registry.inner().clone();
    let health_cache = app_state.health_cache.clone();
    let app_handle = app.clone();

    // Check if provider is disabled
    if let Some(enabled) = registry.is_provider_enabled(&instance_name) {
        if !enabled {
            let result = HealthCheckResult {
                provider_name: instance_name.clone(),
                status: "disabled".to_string(),
                latency_ms: None,
                error_message: None,
            };
            let _ = app_handle.emit("provider-health-check", result);
            health_cache.update_provider(
                &instance_name,
                lr_providers::health_cache::ItemHealth::disabled(instance_name.clone()),
            );
            return Ok(());
        }
    }

    // Spawn the health check in the background
    tokio::spawn(async move {
        // Get the provider directly and perform health check
        if let Some(provider) = registry.get_provider_unchecked(&instance_name) {
            let health = provider.health_check().await;
            let result = HealthCheckResult {
                provider_name: instance_name.clone(),
                status: match &health.status {
                    lr_providers::HealthStatus::Healthy => "healthy".to_string(),
                    lr_providers::HealthStatus::Degraded => "degraded".to_string(),
                    lr_providers::HealthStatus::Unhealthy => "unhealthy".to_string(),
                },
                latency_ms: health.latency_ms,
                error_message: health.error_message.clone(),
            };
            let _ = app_handle.emit("provider-health-check", result);

            // Update centralized health cache so aggregate status (tray + sidebar) recalculates
            use lr_providers::health_cache::ItemHealth;
            use lr_providers::HealthStatus;
            let item_health = match health.status {
                HealthStatus::Healthy => {
                    ItemHealth::healthy(instance_name.clone(), health.latency_ms)
                }
                HealthStatus::Degraded => ItemHealth::degraded(
                    instance_name.clone(),
                    health.latency_ms,
                    health
                        .error_message
                        .unwrap_or_else(|| "Degraded".to_string()),
                ),
                HealthStatus::Unhealthy => ItemHealth::unhealthy(
                    instance_name.clone(),
                    health
                        .error_message
                        .unwrap_or_else(|| "Unhealthy".to_string()),
                ),
            };
            health_cache.update_provider(&instance_name, item_health);
        }
    });

    Ok(())
}

// ============================================================================
// Centralized Health Cache Commands
// ============================================================================

/// Get the current cached health state
///
/// Returns the centralized health cache state including:
/// - Server running status and port
/// - Provider health statuses
/// - MCP server health statuses
/// - Last refresh timestamp
/// - Aggregate health status (red/yellow/green)
#[tauri::command]
pub async fn get_health_cache(
    app_state: State<'_, Arc<lr_server::state::AppState>>,
) -> Result<lr_providers::health_cache::HealthCacheState, String> {
    Ok(app_state.health_cache.get())
}

/// Refresh all health checks
///
/// Triggers a full refresh of all provider and MCP server health checks.
/// Results are emitted via "health-status-changed" event as they complete.
/// Returns immediately after starting the refresh.
#[tauri::command]
pub async fn refresh_all_health(
    _app: tauri::AppHandle,
    app_state: State<'_, Arc<lr_server::state::AppState>>,
    config_manager: State<'_, ConfigManager>,
) -> Result<(), String> {
    let health_cache = app_state.health_cache.clone();
    let provider_registry = app_state.provider_registry.clone();
    let mcp_server_manager = app_state.mcp_server_manager.clone();
    let timeout_secs = config_manager.get().health_check.timeout_secs;

    // Spawn the refresh in the background
    tokio::spawn(async move {
        tracing::info!("Manual health refresh triggered...");
        use lr_providers::health_cache::ItemHealth;

        // Check all providers
        let providers = provider_registry.list_providers();
        for provider_info in providers {
            // Skip disabled providers - emit disabled status
            if !provider_info.enabled {
                health_cache.update_provider(
                    &provider_info.instance_name,
                    ItemHealth::disabled(provider_info.instance_name.clone()),
                );
                continue;
            }

            if let Some(provider) = provider_registry.get_provider(&provider_info.instance_name) {
                let health = tokio::time::timeout(
                    std::time::Duration::from_secs(timeout_secs),
                    provider.health_check(),
                )
                .await;

                let item_health = match health {
                    Ok(h) => {
                        use lr_providers::HealthStatus;
                        match h.status {
                            HealthStatus::Healthy => ItemHealth::healthy(
                                provider_info.instance_name.clone(),
                                h.latency_ms,
                            ),
                            HealthStatus::Degraded => ItemHealth::degraded(
                                provider_info.instance_name.clone(),
                                h.latency_ms,
                                h.error_message.unwrap_or_else(|| "Degraded".to_string()),
                            ),
                            HealthStatus::Unhealthy => ItemHealth::unhealthy(
                                provider_info.instance_name.clone(),
                                h.error_message.unwrap_or_else(|| "Unhealthy".to_string()),
                            ),
                        }
                    }
                    Err(_) => ItemHealth::unhealthy(
                        provider_info.instance_name.clone(),
                        format!("Health check timeout ({}s)", timeout_secs),
                    ),
                };
                health_cache.update_provider(&provider_info.instance_name, item_health);
            }
        }

        // Check all MCP servers
        let mcp_configs = mcp_server_manager.list_configs();
        for config in mcp_configs {
            // Skip disabled MCP servers - emit disabled status
            if !config.enabled {
                health_cache.update_mcp_server(&config.id, ItemHealth::disabled(config.name));
                continue;
            }

            let mcp_server_health = mcp_server_manager.get_server_health(&config.id).await;
            use lr_mcp::manager::HealthStatus as McpHealthStatus;
            let server_id = mcp_server_health.server_id.clone();
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
            health_cache.update_mcp_server(&server_id, item_health);
        }

        health_cache.mark_refresh();
        tracing::info!("Manual health refresh completed");
    });

    Ok(())
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
) -> Result<Vec<lr_providers::ModelInfo>, String> {
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
) -> Result<Vec<lr_providers::ModelInfo>, String> {
    registry.list_all_models().await.map_err(|e| e.to_string())
}

/// Source of pricing information
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PricingSource {
    /// Pricing from models.dev catalog (embedded at build time)
    Catalog,
    /// User-provided pricing override
    Override,
}

/// Detailed model information for the frontend
#[derive(Debug, Clone, serde::Serialize)]
pub struct DetailedModelInfo {
    pub model_id: String,
    pub provider_instance: String,
    pub provider_type: String,
    pub capabilities: Vec<String>,
    pub context_window: u32,
    pub supports_streaming: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_price_per_million: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_price_per_million: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameter_count: Option<String>,
    /// Source of pricing data (catalog or user override)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pricing_source: Option<PricingSource>,
}

/// List all available models with detailed information
///
/// # Returns
/// * `Ok(Vec<DetailedModelInfo>)` with the detailed list of models
#[tauri::command]
pub async fn list_all_models_detailed(
    registry: State<'_, Arc<ProviderRegistry>>,
    config_manager: State<'_, ConfigManager>,
) -> Result<Vec<DetailedModelInfo>, String> {
    let models = registry
        .list_all_models()
        .await
        .map_err(|e| e.to_string())?;

    let detailed_models = models
        .into_iter()
        .map(|model| {
            use lr_catalog as catalog;

            // Extract provider type from provider instance name
            // Format is typically "provider_type/instance_name" or just "provider_type"
            let provider_type = model
                .provider
                .split('/')
                .next()
                .unwrap_or(&model.provider)
                .to_string();

            // Convert capabilities enum to strings
            let capabilities = model
                .capabilities
                .iter()
                .map(|cap| format!("{:?}", cap).to_lowercase())
                .collect();

            // Format parameter count as string
            let parameter_count = model.parameter_count.map(|count| {
                if count >= 1_000_000_000 {
                    format!("{:.1}B", count as f64 / 1_000_000_000.0)
                } else if count >= 1_000_000 {
                    format!("{:.1}M", count as f64 / 1_000_000.0)
                } else {
                    count.to_string()
                }
            });

            // Fetch pricing from override first, then catalog
            // Skip pricing for local/free providers unless there's an override
            let is_local_provider = matches!(
                provider_type.as_str(),
                "ollama" | "lmstudio" | "openai_compatible" | "localai"
            );

            let config = config_manager.get();

            // Check for pricing override first
            let override_pricing = config
                .pricing_overrides
                .get(&provider_type)
                .and_then(|models| models.get(&model.id));

            let (input_price_per_million, output_price_per_million, pricing_source) =
                if let Some(override_price) = override_pricing {
                    // Use override pricing
                    (
                        Some(override_price.input_per_million),
                        Some(override_price.output_per_million),
                        Some(PricingSource::Override),
                    )
                } else if is_local_provider {
                    // Local providers are free (unless overridden above)
                    (None, None, None)
                } else {
                    // Try catalog lookup
                    let catalog_model = catalog::find_model(&provider_type, &model.id)
                        .or_else(|| catalog::find_model_by_name(&model.id));

                    if let Some(cat_model) = catalog_model {
                        // Convert from per-token to per-million tokens
                        let input_price = cat_model.pricing.prompt_per_token * 1_000_000.0;
                        let output_price = cat_model.pricing.completion_per_token * 1_000_000.0;

                        // Only include pricing if it's non-zero
                        let input = if input_price > 0.0 {
                            Some(input_price)
                        } else {
                            None
                        };
                        let output = if output_price > 0.0 {
                            Some(output_price)
                        } else {
                            None
                        };

                        let source = if input.is_some() || output.is_some() {
                            Some(PricingSource::Catalog)
                        } else {
                            None
                        };

                        (input, output, source)
                    } else {
                        (None, None, None)
                    }
                };

            DetailedModelInfo {
                model_id: model.id,
                provider_instance: model.provider,
                provider_type,
                capabilities,
                context_window: model.context_window,
                supports_streaming: model.supports_streaming,
                input_price_per_million,
                output_price_per_million,
                parameter_count,
                pricing_source,
            }
        })
        .collect();

    Ok(detailed_models)
}
