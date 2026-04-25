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
    // Create provider in registry (in-memory) — includes api_key for factory use
    registry
        .create_provider(instance_name.clone(), provider_type.clone(), config.clone())
        .await
        .map_err(|e| e.to_string())?;

    // Extract api_key and store in keychain (not in config file)
    if let Some(api_key) = config.get("api_key") {
        if !api_key.is_empty() {
            lr_providers::key_storage::store_provider_key(&instance_name, api_key)
                .map_err(|e| format!("Failed to store API key in keychain: {}", e))?;
        }
    }

    // Build config for disk persistence WITHOUT api_key
    let config_for_disk: HashMap<String, String> =
        config.into_iter().filter(|(k, _)| k != "api_key").collect();

    // Save to config file for persistence
    config_manager
        .update(|cfg| {
            // Convert provider_type string to ProviderType enum
            let provider_type_enum = provider_type_str_to_enum(&provider_type);

            // Convert config HashMap to provider_config JSON (without api_key)
            let provider_config = if !config_for_disk.is_empty() {
                Some(serde_json::to_value(&config_for_disk).unwrap_or(serde_json::Value::Null))
            } else {
                None
            };

            cfg.providers.push(lr_config::ProviderConfig {
                name: instance_name.clone(),
                provider_type: provider_type_enum,
                enabled: true,
                provider_config,
                api_key_ref: None,
                free_tier: None,
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
    // Update provider in registry (in-memory) — includes api_key for factory use
    registry
        .update_provider(instance_name.clone(), provider_type.clone(), config.clone())
        .await
        .map_err(|e| e.to_string())?;

    // Update api_key in keychain
    if let Some(api_key) = config.get("api_key") {
        if !api_key.is_empty() {
            lr_providers::key_storage::store_provider_key(&instance_name, api_key)
                .map_err(|e| format!("Failed to store API key in keychain: {}", e))?;
        }
    }

    // Build config for disk persistence WITHOUT api_key
    let config_for_disk: HashMap<String, String> =
        config.into_iter().filter(|(k, _)| k != "api_key").collect();

    // Update in config file (without api_key)
    config_manager
        .update(|cfg| {
            if let Some(provider) = cfg.providers.iter_mut().find(|p| p.name == instance_name) {
                provider.provider_type = provider_type_str_to_enum(&provider_type);
                provider.provider_config = if !config_for_disk.is_empty() {
                    Some(serde_json::to_value(&config_for_disk).unwrap_or(serde_json::Value::Null))
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

    // Migrate keychain entry from old name to new name
    match lr_providers::key_storage::get_provider_key(&instance_name) {
        Ok(Some(api_key)) => {
            if let Err(e) = lr_providers::key_storage::store_provider_key(&new_name, &api_key) {
                tracing::warn!(
                    "Failed to store API key under new name '{}': {}",
                    new_name,
                    e
                );
            } else if let Err(e) = lr_providers::key_storage::delete_provider_key(&instance_name) {
                tracing::warn!(
                    "Failed to delete old API key for '{}': {}",
                    instance_name,
                    e
                );
            }
        }
        Ok(None) => {} // No key to migrate (e.g., local provider)
        Err(e) => {
            tracing::warn!(
                "Failed to check API key for '{}' during rename: {}",
                instance_name,
                e
            );
        }
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

/// Generate a unique clone name for a provider
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
        "jan" => lr_config::ProviderType::Jan,
        "gpt4all" => lr_config::ProviderType::GPT4All,
        "localai" => lr_config::ProviderType::LocalAI,
        "llamacpp" => lr_config::ProviderType::LlamaCpp,
        "github_models" => lr_config::ProviderType::GitHubModels,
        "nvidia_nim" => lr_config::ProviderType::NvidiaNim,
        "cloudflare_ai" => lr_config::ProviderType::CloudflareAI,
        "llm7" => lr_config::ProviderType::Llm7,
        "kluster_ai" => lr_config::ProviderType::KlusterAI,
        "huggingface" => lr_config::ProviderType::HuggingFace,
        "zhipu" => lr_config::ProviderType::Zhipu,
        "openai-chatgpt-plus" => lr_config::ProviderType::ChatGPTPlus,
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

    // Clean up keychain entry (best-effort)
    if let Err(e) = lr_providers::key_storage::delete_provider_key(&instance_name) {
        tracing::warn!(
            "Failed to delete API key for provider '{}' from keychain: {}",
            instance_name,
            e
        );
    }

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

/// Clone an existing provider instance (deep copy with new identity)
///
/// Creates a new provider instance with the same configuration as the source,
/// including copying the API key from the keychain. The clone gets a unique
/// name like "Clone of {name}" (with dedup suffix if needed).
///
/// # Arguments
/// * `instance_name` - Name of the provider instance to clone
///
/// # Returns
/// * `Ok(())` if the provider was cloned successfully
/// * `Err(String)` if the source provider doesn't exist or cloning failed
#[tauri::command]
pub async fn clone_provider_instance(
    instance_name: String,
    registry: State<'_, Arc<ProviderRegistry>>,
    config_manager: State<'_, ConfigManager>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    tracing::info!("Cloning provider instance: {}", instance_name);

    let config = config_manager.get();

    // Find the source provider config
    let source = config
        .providers
        .iter()
        .find(|p| p.name == instance_name)
        .ok_or_else(|| format!("Provider not found: {}", instance_name))?
        .clone();

    // Get the provider_type string from the registry (matches factory key)
    let instances = registry.list_providers();
    let provider_type_str = instances
        .iter()
        .find(|i| i.instance_name == instance_name)
        .map(|i| i.provider_type.clone())
        .ok_or_else(|| format!("Provider '{}' not found in registry", instance_name))?;

    // Generate clone name
    let existing_names: Vec<&str> = config.providers.iter().map(|p| p.name.as_str()).collect();
    let clone_name = generate_clone_name(&source.name, &existing_names);

    // Get the API key from the old provider (if any)
    let key_ref = source.api_key_ref.as_deref().unwrap_or(&source.name);
    let api_key = lr_providers::key_storage::get_provider_key(key_ref)
        .ok()
        .flatten();

    // Build config map for creating the new provider
    // Start with existing provider_config
    let mut config_map: HashMap<String, String> = if let Some(ref pc) = source.provider_config {
        if let Some(obj) = pc.as_object() {
            obj.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                .collect()
        } else {
            HashMap::new()
        }
    } else {
        HashMap::new()
    };

    // Include the API key for the registry creation
    if let Some(ref key) = api_key {
        config_map.insert("api_key".to_string(), key.clone());
    }

    // Create provider in registry (in-memory)
    registry
        .create_provider(clone_name.clone(), provider_type_str, config_map)
        .await
        .map_err(|e| e.to_string())?;

    // Store API key in keychain under new name
    if let Some(ref key) = api_key {
        lr_providers::key_storage::store_provider_key(&clone_name, key)
            .map_err(|e| format!("Failed to store API key for clone: {}", e))?;
    }

    // Save to config file
    config_manager
        .update(|cfg| {
            cfg.providers.push(lr_config::ProviderConfig {
                name: clone_name.clone(),
                provider_type: source.provider_type,
                enabled: source.enabled,
                provider_config: source.provider_config.clone(),
                api_key_ref: None, // Uses clone_name for keyring lookup
                free_tier: source.free_tier.clone(),
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

/// Get whether periodic health checks are enabled
#[tauri::command]
pub async fn get_periodic_health_enabled(
    config_manager: State<'_, ConfigManager>,
) -> Result<bool, String> {
    Ok(config_manager.get().health_check.periodic_enabled)
}

/// Set whether periodic health checks are enabled
///
/// When disabled, only on-failure and user-triggered health checks run.
/// Takes effect immediately without requiring a server restart.
#[tauri::command]
pub async fn set_periodic_health_enabled(
    enabled: bool,
    config_manager: State<'_, ConfigManager>,
    app_state: State<'_, Arc<lr_server::state::AppState>>,
) -> Result<(), String> {
    // Update runtime flag immediately (no restart needed)
    app_state.health_cache.set_periodic_enabled(enabled);

    // Persist to config
    config_manager
        .update(|config| {
            config.health_check.periodic_enabled = enabled;
        })
        .map_err(|e| e.to_string())
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
        .list_provider_models_cached(&instance_name)
        .await
        .map_err(|e| e.to_string())
}

/// List all models from all enabled providers
///
/// Returns a combined list of all models available across all enabled providers.
/// Used by the UI to populate the model selection dropdown.
/// Fetches all providers in parallel for fast loading.
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

/// Get cached models instantly without any network calls
///
/// Returns whatever models are currently in the per-provider caches,
/// even if expired. Used for instant UI display before a fresh fetch completes.
#[tauri::command]
pub async fn get_cached_models(
    registry: State<'_, Arc<ProviderRegistry>>,
) -> Result<Vec<lr_providers::ModelInfo>, String> {
    Ok(registry.get_all_cached_models_instant())
}

/// Payload for the models-provider-loaded event
#[derive(Clone, serde::Serialize)]
struct ProviderModelsPayload {
    provider: String,
    models: Vec<lr_providers::ModelInfo>,
}

/// Payload for the models-refresh-started event
#[derive(Clone, serde::Serialize)]
struct ModelsRefreshStartedPayload {
    providers: Vec<String>,
}

/// Trigger an incremental model refresh in the background
///
/// Spawns parallel per-provider fetch tasks. As each provider completes,
/// emits a `models-provider-loaded` event with that provider's models.
/// When all providers are done, emits `models-changed`.
///
/// Returns immediately - the refresh happens in the background.
///
/// # Arguments
/// * `force` - If true, invalidates all caches before refreshing (bypasses TTL)
#[tauri::command]
pub async fn refresh_models_incremental(
    registry: State<'_, Arc<ProviderRegistry>>,
    app: tauri::AppHandle,
    force: Option<bool>,
) -> Result<(), String> {
    let registry = registry.inner().clone();

    // Invalidate caches when force-refreshing so we bypass TTL
    if force.unwrap_or(false) {
        registry.invalidate_all_caches();
    }

    if !registry.try_start_refresh() {
        // Another refresh is already in progress
        return Ok(());
    }

    tokio::spawn(async move {
        let enabled_instances = registry.get_enabled_instance_names();

        // Notify frontend which providers are being refreshed
        let _ = app.emit(
            "models-refresh-started",
            ModelsRefreshStartedPayload {
                providers: enabled_instances.clone(),
            },
        );

        // Spawn a task per provider for true parallelism
        let handles: Vec<_> = enabled_instances
            .into_iter()
            .map(|instance_name| {
                let registry = registry.clone();
                let app = app.clone();
                tokio::spawn(async move {
                    match registry.list_provider_models_cached(&instance_name).await {
                        Ok(mut models) => {
                            for model in &mut models {
                                model.provider = instance_name.clone();
                            }
                            let _ = app.emit(
                                "models-provider-loaded",
                                ProviderModelsPayload {
                                    provider: instance_name,
                                    models,
                                },
                            );
                        }
                        Err(e) => {
                            tracing::warn!(
                                "Incremental refresh failed for '{}': {}",
                                instance_name,
                                e
                            );
                        }
                    }
                })
            })
            .collect();

        // Wait for all providers to complete
        futures::future::join_all(handles).await;

        // Update aggregate cache
        let _ = registry.refresh_model_cache().await;
        registry.finish_refresh();

        // Notify frontend that all models are loaded
        let _ = app.emit("models-changed", ());
    });

    Ok(())
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

// ============================================================================
// Feature Support Matrix Commands
// ============================================================================

/// Get feature support information for a specific provider instance.
///
/// Returns endpoint support, model feature support, and optimization feature
/// support for the given provider, computed from trait methods and capabilities.
#[tauri::command]
pub async fn get_provider_feature_support(
    instance_name: String,
    registry: State<'_, Arc<ProviderRegistry>>,
) -> Result<lr_providers::ProviderFeatureSupport, String> {
    let provider = registry
        .get_provider(&instance_name)
        .ok_or_else(|| format!("Provider instance '{}' not found", instance_name))?;

    Ok(provider.get_feature_support(&instance_name))
}

/// Get feature support information for all provider instances.
///
/// Returns feature support data for every registered provider, useful for
/// rendering a cross-provider compatibility matrix.
#[tauri::command]
pub async fn get_all_provider_feature_support(
    registry: State<'_, Arc<ProviderRegistry>>,
) -> Result<Vec<lr_providers::ProviderFeatureSupport>, String> {
    let providers = registry.list_providers();
    let mut results = Vec::with_capacity(providers.len());

    for info in &providers {
        if let Some(provider) = registry.get_provider(&info.instance_name) {
            results.push(provider.get_feature_support(&info.instance_name));
        }
    }

    Ok(results)
}

/// Get the static feature × endpoint × client mode matrix.
///
/// Returns hardcoded data showing which optimization features apply to which
/// endpoints and client modes. This does not change per provider.
#[tauri::command]
pub async fn get_feature_endpoint_matrix() -> Result<lr_providers::FeatureEndpointMatrix, String> {
    Ok(lr_providers::build_feature_endpoint_matrix())
}

/// Support level for the three chat-ish LocalRouter endpoints, per
/// provider instance. Used by the Try It Out chat panel to annotate
/// endpoint dropdown entries with "(Translated)" when LocalRouter has
/// to emulate that path on top of a different upstream API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiPathSupport {
    pub chat_completions: lr_providers::SupportLevel,
    pub completions: lr_providers::SupportLevel,
    pub responses: lr_providers::SupportLevel,
}

/// Report per-endpoint support for a specific provider instance.
#[tauri::command]
pub async fn get_api_path_support(
    instance_name: String,
    registry: State<'_, Arc<ProviderRegistry>>,
) -> Result<ApiPathSupport, String> {
    let provider = registry
        .get_provider(&instance_name)
        .ok_or_else(|| format!("Provider instance '{}' not found", instance_name))?;

    Ok(ApiPathSupport {
        chat_completions: provider.api_path_support("chat_completions"),
        completions: provider.api_path_support("completions"),
        responses: provider.api_path_support("responses"),
    })
}
