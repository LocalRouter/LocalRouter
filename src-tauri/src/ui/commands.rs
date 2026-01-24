//! Tauri command handlers
//!
//! Functions exposed to the frontend via Tauri IPC.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use crate::api_keys::keychain_trait::KeychainStorage;
use crate::config::{
    client_strategy_name, ConfigManager, McpAuthConfig, McpServerAccess, McpServerConfig,
    McpTransportConfig, McpTransportType,
};
use crate::mcp::McpServerManager;
use crate::monitoring::logger::AccessLogger;
use crate::oauth_clients::OAuthClientManager;
use crate::providers::registry::ProviderRegistry;
use crate::server::ServerManager;
use serde::{Deserialize, Serialize};
use tauri::{Emitter, State};

/// Get current configuration
#[tauri::command]
pub async fn get_config(
    config_manager: State<'_, ConfigManager>,
) -> Result<serde_json::Value, String> {
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
    crate::providers::key_storage::has_provider_key(&provider).map_err(|e| e.to_string())
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
    crate::providers::key_storage::delete_provider_key(&provider).map_err(|e| e.to_string())
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

        let has_key = crate::providers::key_storage::has_provider_key(&key_ref).unwrap_or(false);

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
) -> Result<HashMap<String, crate::providers::ProviderHealth>, String> {
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
) -> Result<Vec<String>, String> {
    let provider_names = registry.get_provider_names();

    // Clone what we need for the spawned task
    let registry = registry.inner().clone();
    let app_handle = app.clone();

    // Spawn health checks for each provider instance in parallel
    // We check each instance directly (not via HealthCheckManager) to ensure
    // proper instance name mapping even with multiple instances of the same provider type
    tokio::spawn(async move {
        let instance_names = registry.get_provider_names();
        let mut handles = Vec::new();

        for instance_name in instance_names {
            let registry = registry.clone();
            let app_handle = app_handle.clone();
            let instance_name_clone = instance_name.clone();

            let handle = tokio::spawn(async move {
                if let Some(provider) = registry.get_provider_unchecked(&instance_name_clone) {
                    let health = provider.health_check().await;
                    let result = HealthCheckResult {
                        provider_name: instance_name_clone,
                        status: match health.status {
                            crate::providers::HealthStatus::Healthy => "healthy".to_string(),
                            crate::providers::HealthStatus::Degraded => "degraded".to_string(),
                            crate::providers::HealthStatus::Unhealthy => "unhealthy".to_string(),
                        },
                        latency_ms: health.latency_ms,
                        error_message: health.error_message,
                    };
                    let _ = app_handle.emit("provider-health-check", result);
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
    instance_name: String,
) -> Result<(), String> {
    let registry = registry.inner().clone();
    let app_handle = app.clone();

    // Spawn the health check in the background
    tokio::spawn(async move {
        // Get the provider directly and perform health check
        if let Some(provider) = registry.get_provider_unchecked(&instance_name) {
            let health = provider.health_check().await;
            let result = HealthCheckResult {
                provider_name: instance_name,
                status: match health.status {
                    crate::providers::HealthStatus::Healthy => "healthy".to_string(),
                    crate::providers::HealthStatus::Degraded => "degraded".to_string(),
                    crate::providers::HealthStatus::Unhealthy => "unhealthy".to_string(),
                },
                latency_ms: health.latency_ms,
                error_message: health.error_message,
            };
            let _ = app_handle.emit("provider-health-check", result);
        }
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
    registry.list_all_models().await.map_err(|e| e.to_string())
}

/// Source of pricing information
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PricingSource {
    /// Pricing from OpenRouter catalog (embedded at build time)
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
            use crate::catalog;

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
// Server Configuration Commands
// ============================================================================

/// Get server configuration (host and port)
#[tauri::command]
pub async fn get_server_config(
    config_manager: State<'_, ConfigManager>,
    server_manager: State<'_, Arc<ServerManager>>,
) -> Result<ServerConfigInfo, String> {
    let config = config_manager.get();
    let actual_port = server_manager.get_actual_port();
    Ok(ServerConfigInfo {
        host: config.server.host.clone(),
        port: config.server.port,
        actual_port,
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
    config_manager.save().await.map_err(|e| e.to_string())
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
    pub actual_port: Option<u16>,
    pub enable_cors: bool,
}

// ============================================================================
// Monitoring & Statistics Commands
// ============================================================================

/// Get aggregate statistics (requests, tokens, cost)
///
/// Returns statistics computed from persistent metrics database (last 90 days).
#[tauri::command]
pub async fn get_aggregate_stats(
    server_manager: State<'_, Arc<crate::server::ServerManager>>,
) -> Result<crate::server::state::AggregateStats, String> {
    // Get the app state from server manager
    let app_state = server_manager
        .get_state()
        .ok_or_else(|| "Server is not running".to_string())?;

    // Get persistent metrics from the last 90 days (all stored data)
    let now = chrono::Utc::now();
    let start = now - chrono::Duration::days(90);
    let data_points = app_state.metrics_collector.get_global_range(start, now);

    // Aggregate the metrics
    let mut total_requests = 0u64;
    let mut successful_requests = 0u64;
    let mut total_tokens = 0u64;
    let mut total_cost = 0.0f64;

    for point in data_points {
        total_requests += point.requests;
        successful_requests += point.successful_requests;
        total_tokens += point.total_tokens;
        total_cost += point.cost_usd;
    }

    Ok(crate::server::state::AggregateStats {
        total_requests,
        successful_requests,
        total_tokens,
        total_cost,
    })
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
// OAuth Client Commands (for MCP)
// ============================================================================
///
///   OAuth client information for display
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthClientInfo {
    pub id: String,
    pub name: String,
    pub client_id: String,
    pub linked_server_ids: Vec<String>,
    pub enabled: bool,
    pub created_at: String,
}

/// List all OAuth clients
#[tauri::command]
pub async fn list_oauth_clients(
    oauth_client_manager: State<'_, Arc<OAuthClientManager>>,
) -> Result<Vec<OAuthClientInfo>, String> {
    let clients = oauth_client_manager.list_clients();
    Ok(clients
        .into_iter()
        .map(|c| OAuthClientInfo {
            id: c.id.clone(),
            name: c.name.clone(),
            client_id: c.client_id.clone(),
            linked_server_ids: c.linked_server_ids.clone(),
            enabled: c.enabled,
            created_at: c.created_at.to_rfc3339(),
        })
        .collect())
}

/// Create a new OAuth client
///
/// # Arguments
/// * `name` - Optional name for the client. If None, generates "mcp-client-{number}"
///
/// # Returns
/// * Tuple of (client_id, client_secret, OAuthClientInfo)
/// * client_secret is only returned once at creation time
#[tauri::command]
pub async fn create_oauth_client(
    name: Option<String>,
    oauth_client_manager: State<'_, Arc<OAuthClientManager>>,
    config_manager: State<'_, ConfigManager>,
    app: tauri::AppHandle,
) -> Result<(String, String, OAuthClientInfo), String> {
    tracing::info!("Creating new OAuth client with name: {:?}", name);

    let (client_id, client_secret, config) = oauth_client_manager
        .create_client(name)
        .await
        .map_err(|e| e.to_string())?;

    tracing::info!("OAuth client created: {} ({})", config.name, config.id);

    // Save to config file
    config_manager
        .update(|cfg| {
            cfg.oauth_clients.push(config.clone());
        })
        .map_err(|e| e.to_string())?;

    // Persist to disk
    config_manager.save().await.map_err(|e| e.to_string())?;

    // Rebuild tray menu
    if let Err(e) = crate::ui::tray::rebuild_tray_menu(&app) {
        tracing::error!("Failed to rebuild tray menu: {}", e);
    }

    // Notify frontend
    let _ = app.emit("oauth-clients-changed", ());

    Ok((
        client_id,
        client_secret,
        OAuthClientInfo {
            id: config.id,
            name: config.name,
            client_id: config.client_id,
            linked_server_ids: config.linked_server_ids,
            enabled: config.enabled,
            created_at: config.created_at.to_rfc3339(),
        },
    ))
}

/// Get the OAuth client secret from keychain
///
/// # Arguments
/// * `id` - The OAuth client ID (internal UUID, not client_id)
///
/// # Returns
/// * The client_secret string if it exists
/// * Error if secret doesn't exist or keychain access fails
#[tauri::command]
pub async fn get_oauth_client_secret(
    id: String,
    oauth_client_manager: State<'_, Arc<OAuthClientManager>>,
) -> Result<String, String> {
    oauth_client_manager
        .get_client_secret(&id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("OAuth client secret not found in keychain: {}", id))
}

/// Delete an OAuth client
///
/// # Arguments
/// * `id` - The OAuth client ID to delete
///
/// # Returns
/// * Ok(()) if the client was deleted successfully
/// * Error if the client doesn't exist or deletion fails
#[tauri::command]
pub async fn delete_oauth_client(
    id: String,
    oauth_client_manager: State<'_, Arc<OAuthClientManager>>,
    config_manager: State<'_, ConfigManager>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    // Delete from keychain
    oauth_client_manager
        .delete_client(&id)
        .map_err(|e| e.to_string())?;

    // Remove from config file
    config_manager
        .update(|cfg| {
            cfg.oauth_clients.retain(|c| c.id != id);
        })
        .map_err(|e| e.to_string())?;

    // Persist to disk
    config_manager.save().await.map_err(|e| e.to_string())?;

    // Rebuild tray menu
    if let Err(e) = crate::ui::tray::rebuild_tray_menu(&app) {
        tracing::error!("Failed to rebuild tray menu: {}", e);
    }

    // Notify frontend
    let _ = app.emit("oauth-clients-changed", ());

    Ok(())
}

/// Update an OAuth client's name
///
/// # Arguments
/// * `id` - The OAuth client ID to update
/// * `name` - The new name for the OAuth client
///
/// # Returns
/// * Ok(()) if the update succeeded
/// * Error if the client doesn't exist or update fails
#[tauri::command]
pub async fn update_oauth_client_name(
    id: String,
    name: String,
    oauth_client_manager: State<'_, Arc<OAuthClientManager>>,
    config_manager: State<'_, ConfigManager>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    // Validate name is not empty
    if name.trim().is_empty() {
        return Err("OAuth client name cannot be empty".to_string());
    }

    // Update in memory
    oauth_client_manager
        .update_client(&id, |cfg| {
            cfg.name = name.clone();
        })
        .map_err(|e| e.to_string())?;

    // Update in config file
    config_manager
        .update(|cfg| {
            if let Some(client) = cfg.oauth_clients.iter_mut().find(|c| c.id == id) {
                client.name = name.clone();
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
    let _ = app.emit("oauth-clients-changed", ());

    Ok(())
}

/// Toggle an OAuth client's enabled state
///
/// # Arguments
/// * `id` - The OAuth client ID to toggle
/// * `enabled` - Whether to enable (true) or disable (false) the client
///
/// # Returns
/// * Ok(()) if the toggle succeeded
/// * Error if the client doesn't exist or toggle fails
#[tauri::command]
pub async fn toggle_oauth_client_enabled(
    id: String,
    enabled: bool,
    oauth_client_manager: State<'_, Arc<OAuthClientManager>>,
    config_manager: State<'_, ConfigManager>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    // Update in memory
    oauth_client_manager
        .update_client(&id, |cfg| {
            cfg.enabled = enabled;
        })
        .map_err(|e| e.to_string())?;

    // Update in config file
    config_manager
        .update(|cfg| {
            if let Some(client) = cfg.oauth_clients.iter_mut().find(|c| c.id == id) {
                client.enabled = enabled;
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
    let _ = app.emit("oauth-clients-changed", ());

    Ok(())
}

/// Link an MCP server to an OAuth client
///
/// # Arguments
/// * `client_id` - The OAuth client ID
/// * `server_id` - The MCP server ID to link
///
/// # Returns
/// * Ok(()) if linking succeeded
/// * Error if the client doesn't exist or linking fails
#[tauri::command]
pub async fn link_mcp_server(
    client_id: String,
    server_id: String,
    oauth_client_manager: State<'_, Arc<OAuthClientManager>>,
    config_manager: State<'_, ConfigManager>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    // Link in memory
    oauth_client_manager
        .link_server(&client_id, server_id.clone())
        .map_err(|e| e.to_string())?;

    // Update in config file
    config_manager
        .update(|cfg| {
            if let Some(client) = cfg.oauth_clients.iter_mut().find(|c| c.id == client_id) {
                if !client.linked_server_ids.contains(&server_id) {
                    client.linked_server_ids.push(server_id);
                }
            }
        })
        .map_err(|e| e.to_string())?;

    // Persist to disk
    config_manager.save().await.map_err(|e| e.to_string())?;

    // Notify frontend
    let _ = app.emit("oauth-clients-changed", ());

    Ok(())
}

/// Unlink an MCP server from an OAuth client
///
/// # Arguments
/// * `client_id` - The OAuth client ID
/// * `server_id` - The MCP server ID to unlink
///
/// # Returns
/// * Ok(()) if unlinking succeeded
/// * Error if the client doesn't exist or unlinking fails
#[tauri::command]
pub async fn unlink_mcp_server(
    client_id: String,
    server_id: String,
    oauth_client_manager: State<'_, Arc<OAuthClientManager>>,
    config_manager: State<'_, ConfigManager>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    // Unlink in memory
    oauth_client_manager
        .unlink_server(&client_id, &server_id)
        .map_err(|e| e.to_string())?;

    // Update in config file
    config_manager
        .update(|cfg| {
            if let Some(client) = cfg.oauth_clients.iter_mut().find(|c| c.id == client_id) {
                client.linked_server_ids.retain(|id| id != &server_id);
            }
        })
        .map_err(|e| e.to_string())?;

    // Persist to disk
    config_manager.save().await.map_err(|e| e.to_string())?;

    // Notify frontend
    let _ = app.emit("oauth-clients-changed", ());

    Ok(())
}

/// Get all MCP servers linked to an OAuth client
///
/// # Arguments
/// * `client_id` - The OAuth client ID
///
/// # Returns
/// * List of MCP server IDs linked to this client
/// * Empty list if the client doesn't exist or has no linked servers
#[tauri::command]
pub async fn get_oauth_client_linked_servers(
    client_id: String,
    oauth_client_manager: State<'_, Arc<OAuthClientManager>>,
) -> Result<Vec<String>, String> {
    if let Some(client) = oauth_client_manager.get_client(&client_id) {
        Ok(client.linked_server_ids)
    } else {
        Ok(Vec::new())
    }
}

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
            let keychain = crate::api_keys::CachedKeychain::auto()
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
            let keychain = crate::api_keys::CachedKeychain::auto()
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
                let keychain = crate::api_keys::CachedKeychain::auto()
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
) -> Result<crate::mcp::manager::McpServerHealth, String> {
    Ok(mcp_manager.get_server_health(&server_id).await)
}

/// Get health status for all MCP servers
///
/// # Returns
/// * List of health statuses for all servers
#[tauri::command]
pub async fn get_all_mcp_server_health(
    mcp_manager: State<'_, Arc<McpServerManager>>,
) -> Result<Vec<crate::mcp::manager::McpServerHealth>, String> {
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
    let server_ids: Vec<String> = mcp_manager.list_configs().iter().map(|c| c.id.clone()).collect();

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

            let handle = tokio::spawn(async move {
                let health = mcp_manager.get_server_health(&server_id).await;
                let result = McpHealthCheckResult {
                    server_id: health.server_id,
                    server_name: health.server_name,
                    status: match health.status {
                        crate::mcp::manager::HealthStatus::Healthy => "healthy".to_string(),
                        crate::mcp::manager::HealthStatus::Ready => "ready".to_string(),
                        crate::mcp::manager::HealthStatus::Unhealthy => "unhealthy".to_string(),
                        crate::mcp::manager::HealthStatus::Unknown => "unknown".to_string(),
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
    server_id: String,
) -> Result<(), String> {
    let mcp_manager = mcp_manager.inner().clone();
    let app_handle = app.clone();

    tokio::spawn(async move {
        let health = mcp_manager.get_server_health(&server_id).await;
        let result = McpHealthCheckResult {
            server_id: health.server_id,
            server_name: health.server_name,
            status: match health.status {
                crate::mcp::manager::HealthStatus::Healthy => "healthy".to_string(),
                crate::mcp::manager::HealthStatus::Ready => "ready".to_string(),
                crate::mcp::manager::HealthStatus::Unhealthy => "unhealthy".to_string(),
                crate::mcp::manager::HealthStatus::Unknown => "unknown".to_string(),
            },
            latency_ms: health.latency_ms,
            error: health.error,
        };
        let _ = app_handle.emit("mcp-health-check", result);
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
    config_manager: State<'_, ConfigManager>,
    app: tauri::AppHandle,
) -> Result<(), String> {
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
    server_manager: State<'_, Arc<crate::server::manager::ServerManager>>,
) -> Result<serde_json::Value, String> {
    use crate::mcp::protocol::JsonRpcRequest;
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
                    &crate::monitoring::mcp_metrics::McpRequestMetrics {
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
            state.metrics_collector.mcp().record(
                &crate::monitoring::mcp_metrics::McpRequestMetrics {
                    client_id,
                    server_id: &server_id,
                    method,
                    latency_ms,
                    success: false,
                    error_code: Some(error.code),
                },
            );
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
            .record(&crate::monitoring::mcp_metrics::McpRequestMetrics {
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
    server_manager: State<'_, Arc<crate::server::manager::ServerManager>>,
) -> Result<serde_json::Value, String> {
    use crate::mcp::protocol::JsonRpcRequest;
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
                    &crate::monitoring::mcp_metrics::McpRequestMetrics {
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
            state.metrics_collector.mcp().record(
                &crate::monitoring::mcp_metrics::McpRequestMetrics {
                    client_id,
                    server_id: &server_id,
                    method: &method,
                    latency_ms,
                    success: false,
                    error_code: Some(error.code),
                },
            );
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
            .record(&crate::monitoring::mcp_metrics::McpRequestMetrics {
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
        let tools_request = crate::mcp::protocol::JsonRpcRequest::new(
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
        let resources_request = crate::mcp::protocol::JsonRpcRequest::new(
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
        let prompts_request = crate::mcp::protocol::JsonRpcRequest::new(
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
    let savings_tokens = if total_tokens > deferred_tokens {
        total_tokens - deferred_tokens
    } else {
        0
    };
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
    pub created_at: String,
    pub last_used: Option<String>,
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
    client_manager: State<'_, Arc<crate::clients::ClientManager>>,
) -> Result<Vec<ClientInfo>, String> {
    let clients = client_manager.list_clients();
    Ok(clients
        .into_iter()
        .map(|c| {
            let (mcp_access_mode, mcp_servers) = mcp_access_to_ui(&c.mcp_server_access);
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
                created_at: c.created_at.to_rfc3339(),
                last_used: c.last_used.map(|t| t.to_rfc3339()),
            }
        })
        .collect())
}

/// Create a new client
#[tauri::command]
pub async fn create_client(
    name: String,
    client_manager: State<'_, Arc<crate::clients::ClientManager>>,
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
        created_at: client.created_at.to_rfc3339(),
        last_used: client.last_used.map(|t| t.to_rfc3339()),
    };

    Ok((secret, client_info))
}

/// Delete a client
#[tauri::command]
pub async fn delete_client(
    client_id: String,
    client_manager: State<'_, Arc<crate::clients::ClientManager>>,
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
    client_manager: State<'_, Arc<crate::clients::ClientManager>>,
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
    client_manager: State<'_, Arc<crate::clients::ClientManager>>,
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
    client_manager: State<'_, Arc<crate::clients::ClientManager>>,
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
    client_manager: State<'_, Arc<crate::clients::ClientManager>>,
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
    client_manager: State<'_, Arc<crate::clients::ClientManager>>,
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
    client_manager: State<'_, Arc<crate::clients::ClientManager>>,
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
    client_manager: State<'_, Arc<crate::clients::ClientManager>>,
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
    client_manager: State<'_, Arc<crate::clients::ClientManager>>,
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
    client_manager: State<'_, Arc<crate::clients::ClientManager>>,
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
    client_manager: State<'_, Arc<crate::clients::ClientManager>>,
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
) -> Result<Vec<crate::config::Strategy>, String> {
    let config = config_manager.get();
    Ok(config.strategies)
}

/// Get a specific strategy by ID
#[tauri::command]
pub async fn get_strategy(
    strategy_id: String,
    config_manager: State<'_, ConfigManager>,
) -> Result<crate::config::Strategy, String> {
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
) -> Result<crate::config::Strategy, String> {
    tracing::info!("Creating strategy: {}", name);

    let strategy = if let Some(parent_id) = parent {
        // Validate parent exists
        let config = config_manager.get();
        let client = config
            .clients
            .iter()
            .find(|c| c.id == parent_id)
            .ok_or_else(|| format!("Parent client not found: {}", parent_id))?;

        crate::config::Strategy::new_for_client(parent_id, client.name.clone())
    } else {
        crate::config::Strategy::new(name)
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
    allowed_models: Option<crate::config::AvailableModelsSelection>,
    auto_config: Option<crate::config::AutoModelConfig>,
    rate_limits: Option<Vec<crate::config::StrategyRateLimit>>,
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
) -> Result<Vec<crate::config::Client>, String> {
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
// OpenAPI Documentation
// ============================================================================

/// Get the OpenAPI specification
///
/// Returns the complete OpenAPI 3.1 specification in JSON format.
/// This can be used to display API documentation in the UI.
/// The server URLs are dynamically updated to match the actual running server port.
///
/// # Returns
/// * Ok(String) - The OpenAPI spec as JSON
/// * Err(String) - Error message if generation fails
#[tauri::command]
pub async fn get_openapi_spec(
    server_manager: State<'_, Arc<ServerManager>>,
) -> Result<String, String> {
    let mut spec_json = crate::server::openapi::get_openapi_json().map_err(|e| e.to_string())?;

    // Get the actual server port and dynamically update the spec
    if let Some(actual_port) = server_manager.get_actual_port() {
        // Replace hardcoded port 3625 with actual port in the spec
        spec_json = spec_json.replace(":3625", &format!(":{}", actual_port));
    }

    Ok(spec_json)
}

/// Get the internal test bearer token for UI model testing
/// This token is regenerated on each app start and allows the UI to bypass API key restrictions
/// when testing models directly. Only accessible via Tauri IPC, never exposed over HTTP.
/// Use this as a regular bearer token in the Authorization header.
#[tauri::command]
pub async fn get_internal_test_token(
    server_manager: State<'_, Arc<crate::server::ServerManager>>,
) -> Result<String, String> {
    let state = server_manager
        .get_state()
        .ok_or_else(|| "Server not started".to_string())?;

    Ok(state.get_internal_test_secret())
}

/// Create a temporary test client bound to a specific routing strategy.
/// This is used by the "Try It Out" feature to test requests with specific strategies.
/// The client is created and persisted so it can be used for testing.
/// Returns the bearer token that can be used to make requests with this strategy.
#[tauri::command]
pub async fn create_test_client_for_strategy(
    strategy_id: String,
    client_manager: State<'_, Arc<crate::clients::ClientManager>>,
    config_manager: State<'_, ConfigManager>,
) -> Result<String, String> {
    // Verify strategy exists
    let config = config_manager.get();
    let strategy_exists = config.strategies.iter().any(|s| s.name == strategy_id);
    if !strategy_exists {
        return Err(format!("Strategy not found: {}", strategy_id));
    }

    // Create a test client with a unique name
    let test_client_name = format!("_test_strategy_{}", strategy_id);

    // Check if we already have a test client for this strategy
    let existing_clients = client_manager.list_clients();
    if let Some(existing) = existing_clients.iter().find(|c| c.name == test_client_name) {
        // Return the existing client's secret
        return client_manager
            .get_secret(&existing.id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "Failed to retrieve test client secret".to_string());
    }

    // Create a new test client
    let (client_id, secret, _) = client_manager
        .create_client(test_client_name)
        .map_err(|e| e.to_string())?;

    // Assign the strategy to this client using ConfigManager
    config_manager
        .assign_client_strategy(&client_id, &strategy_id)
        .map_err(|e| e.to_string())?;

    // Sync the client manager with updated config
    let updated_config = config_manager.get();
    client_manager.sync_clients(updated_config.clients);

    Ok(secret)
}

// ============================================================================
// Setup Wizard Commands
// ============================================================================

/// Check if the setup wizard has been shown
///
/// Used for first-run detection. Returns true if the wizard has been shown,
/// false if this is the first time the app is being run.
#[tauri::command]
pub async fn get_setup_wizard_shown(
    config_manager: State<'_, ConfigManager>,
) -> Result<bool, String> {
    let config = config_manager.get();
    Ok(config.setup_wizard_shown)
}

/// Mark the setup wizard as shown
///
/// Called after the user completes or dismisses the setup wizard.
/// This prevents the wizard from showing again on subsequent app launches.
#[tauri::command]
pub async fn set_setup_wizard_shown(
    config_manager: State<'_, ConfigManager>,
) -> Result<(), String> {
    config_manager
        .update(|cfg| {
            cfg.setup_wizard_shown = true;
        })
        .map_err(|e| e.to_string())?;

    config_manager.save().await.map_err(|e| e.to_string())
}

// ============================================================================
// Access Logs Commands
// ============================================================================

use std::io::{BufRead, BufReader};

/// Get LLM access logs
///
/// Reads log entries from the LLM access log files.
/// Optimized to stop early once enough entries are collected.
///
/// # Arguments
/// * `limit` - Maximum number of entries to return (default: 100)
/// * `offset` - Number of entries to skip (default: 0)
/// * `client_name` - Optional filter by client name (API key name)
/// * `provider` - Optional filter by provider
/// * `model` - Optional filter by model
///
/// # Returns
/// * List of LLM access log entries (newest first)
#[tauri::command]
pub async fn get_llm_logs(
    limit: Option<usize>,
    offset: Option<usize>,
    client_name: Option<String>,
    provider: Option<String>,
    model: Option<String>,
) -> Result<Vec<crate::monitoring::logger::AccessLogEntry>, String> {
    use std::fs;

    let limit = limit.unwrap_or(100);
    let offset = offset.unwrap_or(0);
    let target_count = offset + limit;

    // Get log directory
    let log_dir = get_log_directory().map_err(|e: crate::utils::errors::AppError| e.to_string())?;

    // Read all log files (sorted by date, newest first)
    let mut log_files = Vec::new();
    if let Ok(entries) = fs::read_dir(&log_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                if let Some(filename) = path.file_name().and_then(|n| n.to_str()) {
                    // Match LLM log files (localrouter-YYYY-MM-DD.log, not localrouter-mcp-*.log)
                    if filename.starts_with("localrouter-")
                        && !filename.starts_with("localrouter-mcp-")
                        && filename.ends_with(".log")
                    {
                        log_files.push(path);
                    }
                }
            }
        }
    }

    // Sort by filename (date) in descending order (newest files first)
    log_files.sort_by(|a, b| b.cmp(a));

    // Read and parse log entries with filtering
    // Process files from newest to oldest, collect entries in reverse order per file
    let mut entries = Vec::new();
    let mut collected_enough = false;

    for log_file in log_files {
        if collected_enough {
            break;
        }

        if let Ok(file) = fs::File::open(&log_file) {
            let reader = BufReader::new(file);
            // Collect all matching entries from this file, then reverse to get newest first
            let mut file_entries: Vec<crate::monitoring::logger::AccessLogEntry> = Vec::new();

            for line in reader.lines().map_while(Result::ok) {
                if let Ok(entry) =
                    serde_json::from_str::<crate::monitoring::logger::AccessLogEntry>(&line)
                {
                    // Apply filters
                    let matches = client_name
                        .as_ref()
                        .map_or(true, |f| entry.api_key_name == *f)
                        && provider.as_ref().map_or(true, |f| entry.provider == *f)
                        && model.as_ref().map_or(true, |f| entry.model == *f);

                    if matches {
                        file_entries.push(entry);
                    }
                }
            }

            // Reverse to get newest entries first within this file
            file_entries.reverse();
            entries.extend(file_entries);

            // Check if we have collected enough entries (with buffer for sorting)
            // We need offset + limit entries to return the correct page
            if entries.len() >= target_count {
                collected_enough = true;
            }
        }
    }

    // Sort by timestamp (newest first) to handle entries spanning midnight
    entries.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));

    // Apply offset and limit
    let entries: Vec<_> = entries.into_iter().skip(offset).take(limit).collect();

    Ok(entries)
}

/// Get MCP access logs
///
/// Reads log entries from the MCP access log files.
/// Optimized to stop early once enough entries are collected.
///
/// # Arguments
/// * `limit` - Maximum number of entries to return (default: 100)
/// * `offset` - Number of entries to skip (default: 0)
/// * `client_id` - Optional filter by client ID
/// * `server_id` - Optional filter by server ID
///
/// # Returns
/// * List of MCP access log entries (newest first)
#[tauri::command]
pub async fn get_mcp_logs(
    limit: Option<usize>,
    offset: Option<usize>,
    client_id: Option<String>,
    server_id: Option<String>,
) -> Result<Vec<crate::monitoring::mcp_logger::McpAccessLogEntry>, String> {
    use std::fs;

    let limit = limit.unwrap_or(100);
    let offset = offset.unwrap_or(0);
    let target_count = offset + limit;

    // Get log directory
    let log_dir = get_log_directory().map_err(|e: crate::utils::errors::AppError| e.to_string())?;

    // Read all MCP log files (sorted by date, newest first)
    let mut log_files = Vec::new();
    if let Ok(entries) = fs::read_dir(&log_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                if let Some(filename) = path.file_name().and_then(|n| n.to_str()) {
                    // Match MCP log files (localrouter-mcp-YYYY-MM-DD.log)
                    if filename.starts_with("localrouter-mcp-") && filename.ends_with(".log") {
                        log_files.push(path);
                    }
                }
            }
        }
    }

    // Sort by filename (date) in descending order (newest files first)
    log_files.sort_by(|a, b| b.cmp(a));

    // Read and parse log entries with filtering
    // Process files from newest to oldest, collect entries in reverse order per file
    let mut entries = Vec::new();
    let mut collected_enough = false;

    for log_file in log_files {
        if collected_enough {
            break;
        }

        if let Ok(file) = fs::File::open(&log_file) {
            let reader = BufReader::new(file);
            // Collect all matching entries from this file, then reverse to get newest first
            let mut file_entries: Vec<crate::monitoring::mcp_logger::McpAccessLogEntry> =
                Vec::new();

            for line in reader.lines().map_while(Result::ok) {
                if let Ok(entry) =
                    serde_json::from_str::<crate::monitoring::mcp_logger::McpAccessLogEntry>(&line)
                {
                    // Apply filters
                    let matches = client_id.as_ref().map_or(true, |f| entry.client_id == *f)
                        && server_id.as_ref().map_or(true, |f| entry.server_id == *f);

                    if matches {
                        file_entries.push(entry);
                    }
                }
            }

            // Reverse to get newest entries first within this file
            file_entries.reverse();
            entries.extend(file_entries);

            // Check if we have collected enough entries (with buffer for sorting)
            // We need offset + limit entries to return the correct page
            if entries.len() >= target_count {
                collected_enough = true;
            }
        }
    }

    // Sort by timestamp (newest first) to handle entries spanning midnight
    entries.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));

    // Apply offset and limit
    let entries: Vec<_> = entries.into_iter().skip(offset).take(limit).collect();

    Ok(entries)
}

/// Get the OS-specific log directory
///
/// Delegates to AccessLogger::get_log_directory() to avoid code duplication.
fn get_log_directory() -> Result<PathBuf, crate::utils::errors::AppError> {
    AccessLogger::get_log_directory()
}

// ============================================================================
// Model Catalog Commands
// ============================================================================

/// Catalog metadata for the frontend
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogMetadata {
    pub fetch_date: String,
    pub api_version: String,
    pub total_models: usize,
}

/// Catalog statistics for display
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogStats {
    pub total_models: usize,
    pub fetch_date: String,
    pub providers: HashMap<String, usize>,
    pub modalities: HashMap<String, usize>,
}

/// Get catalog metadata
#[tauri::command]
pub fn get_catalog_metadata() -> CatalogMetadata {
    use crate::catalog;

    let meta = catalog::metadata();
    CatalogMetadata {
        fetch_date: meta.fetch_date().to_rfc3339(),
        api_version: meta.api_version.to_string(),
        total_models: meta.total_models,
    }
}

/// Get catalog statistics
#[tauri::command]
pub fn get_catalog_stats() -> CatalogStats {
    use crate::catalog;
    use std::collections::HashMap;

    let meta = catalog::metadata();
    let models = catalog::models();

    // Count providers
    let mut providers: HashMap<String, usize> = HashMap::new();
    for model in models {
        if let Some((provider, _)) = model.id.split_once('/') {
            *providers.entry(provider.to_string()).or_insert(0) += 1;
        }
    }

    // Count modalities
    let mut modalities: HashMap<String, usize> = HashMap::new();
    for model in models {
        let modality = match model.modality {
            crate::catalog::Modality::Text => "text",
            crate::catalog::Modality::Multimodal => "multimodal",
            crate::catalog::Modality::Image => "image",
        };
        *modalities.entry(modality.to_string()).or_insert(0) += 1;
    }

    CatalogStats {
        total_models: meta.total_models,
        fetch_date: meta.fetch_date().to_rfc3339(),
        providers,
        modalities,
    }
}

// ============================================================================
// Pricing Override Commands
// ============================================================================

/// Get pricing override for a specific model
#[tauri::command]
pub fn get_pricing_override(
    provider: String,
    model: String,
    config_manager: State<'_, ConfigManager>,
) -> Option<crate::config::ModelPricingOverride> {
    let config = config_manager.get();
    config
        .pricing_overrides
        .get(&provider)
        .and_then(|models| models.get(&model))
        .cloned()
}

/// Set or update pricing override for a specific model
#[tauri::command]
pub fn set_pricing_override(
    provider: String,
    model: String,
    input_per_million: f64,
    output_per_million: f64,
    config_manager: State<'_, ConfigManager>,
) -> Result<(), String> {
    config_manager
        .update(|config| {
            let provider_overrides = config
                .pricing_overrides
                .entry(provider.clone())
                .or_insert_with(std::collections::HashMap::new);

            provider_overrides.insert(
                model.clone(),
                crate::config::ModelPricingOverride {
                    input_per_million,
                    output_per_million,
                },
            );
        })
        .map_err(|e| e.to_string())
}

/// Delete pricing override for a specific model
#[tauri::command]
pub fn delete_pricing_override(
    provider: String,
    model: String,
    config_manager: State<'_, ConfigManager>,
) -> Result<(), String> {
    config_manager
        .update(|config| {
            if let Some(provider_overrides) = config.pricing_overrides.get_mut(&provider) {
                provider_overrides.remove(&model);

                // Clean up empty provider entry
                if provider_overrides.is_empty() {
                    config.pricing_overrides.remove(&provider);
                }
            }
        })
        .map_err(|e| e.to_string())
}

// ============================================================================
// Tray Graph Settings Commands
// ============================================================================

/// Tray graph settings response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrayGraphSettings {
    /// Whether dynamic tray graph is enabled
    pub enabled: bool,
    /// Refresh rate in seconds (1, 10, or 60)
    pub refresh_rate_secs: u64,
}

/// Get current tray graph settings
#[tauri::command]
pub fn get_tray_graph_settings(
    config_manager: State<'_, ConfigManager>,
) -> Result<TrayGraphSettings, String> {
    let config = config_manager.get();
    Ok(TrayGraphSettings {
        enabled: config.ui.tray_graph_enabled,
        refresh_rate_secs: config.ui.tray_graph_refresh_rate_secs,
    })
}

/// Update tray graph settings
#[tauri::command]
pub async fn update_tray_graph_settings(
    enabled: bool,
    refresh_rate_secs: u64,
    config_manager: State<'_, ConfigManager>,
    tray_graph_manager: State<'_, Arc<crate::ui::tray::TrayGraphManager>>,
) -> Result<(), String> {
    // Validate parameters - only allow 1, 10, or 60
    if ![1, 10, 60].contains(&refresh_rate_secs) {
        return Err("refresh_rate_secs must be 1 (Fast), 10 (Medium), or 60 (Slow)".to_string());
    }

    // Update configuration
    config_manager
        .update(|config| {
            config.ui.tray_graph_enabled = enabled;
            config.ui.tray_graph_refresh_rate_secs = refresh_rate_secs;
        })
        .map_err(|e| e.to_string())?;

    // Save to disk
    config_manager.save().await.map_err(|e| e.to_string())?;

    // Update tray graph manager (this triggers a refresh with new rate)
    let new_config = config_manager.get().ui.clone();
    tray_graph_manager.update_config(new_config);

    Ok(())
}

/// Get user's home directory
#[tauri::command]
pub fn get_home_dir() -> Result<String, String> {
    dirs::home_dir()
        .ok_or_else(|| "Failed to get home directory".to_string())?
        .to_str()
        .ok_or_else(|| "Invalid home directory path".to_string())
        .map(|s| s.to_string())
}

/// Get current app version
#[tauri::command]
pub fn get_app_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// Get update configuration
#[tauri::command]
pub async fn get_update_config(
    config_manager: State<'_, ConfigManager>,
) -> Result<crate::config::UpdateConfig, String> {
    Ok(config_manager.get().update.clone())
}

/// Update update configuration
#[tauri::command]
pub async fn update_update_config(
    mode: crate::config::UpdateMode,
    check_interval_days: u64,
    config_manager: State<'_, ConfigManager>,
) -> Result<(), String> {
    // Validate parameters
    if check_interval_days == 0 || check_interval_days > 365 {
        return Err("check_interval_days must be between 1 and 365".to_string());
    }

    // Update configuration
    config_manager
        .update(|config| {
            config.update.mode = mode;
            config.update.check_interval_days = check_interval_days;
        })
        .map_err(|e| e.to_string())?;

    // Save to disk
    config_manager.save().await.map_err(|e| e.to_string())?;

    Ok(())
}

/// Mark that an update check was performed (save timestamp)
/// This is called by the frontend after it performs an update check
#[tauri::command]
pub async fn mark_update_check_performed(
    config_manager: State<'_, ConfigManager>,
) -> Result<(), String> {
    crate::updater::save_last_check_timestamp(&config_manager).await
}

/// Skip a specific version (don't notify about it again)
#[tauri::command]
pub async fn skip_update_version(
    version: String,
    config_manager: State<'_, ConfigManager>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    config_manager
        .update(|config| {
            config.update.skipped_version = Some(version.clone());
        })
        .map_err(|e| e.to_string())?;

    config_manager.save().await.map_err(|e| e.to_string())?;

    // Clear update notification from tray
    crate::ui::tray::set_update_available(&app, false).map_err(|e| e.to_string())?;

    Ok(())
}

/// Set update notification in tray menu
#[tauri::command]
pub fn set_update_notification(app: tauri::AppHandle, available: bool) -> Result<(), String> {
    crate::ui::tray::set_update_available(&app, available).map_err(|e| e.to_string())
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
    oauth_browser_manager: State<'_, Arc<crate::mcp::oauth_browser::McpOAuthBrowserManager>>,
    config_manager: State<'_, ConfigManager>,
) -> Result<crate::mcp::oauth_browser::OAuthBrowserFlowResult, String> {
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
    oauth_browser_manager: State<'_, Arc<crate::mcp::oauth_browser::McpOAuthBrowserManager>>,
) -> Result<crate::mcp::oauth_browser::OAuthBrowserFlowStatus, String> {
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
    oauth_browser_manager: State<'_, Arc<crate::mcp::oauth_browser::McpOAuthBrowserManager>>,
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
    oauth_manager: State<'_, Arc<crate::mcp::oauth::McpOAuthManager>>,
) -> Result<Option<crate::config::McpOAuthDiscovery>, String> {
    match oauth_manager.discover_oauth(&base_url).await {
        Ok(Some(discovery)) => Ok(Some(crate::config::McpOAuthDiscovery {
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
    oauth_browser_manager: State<'_, Arc<crate::mcp::oauth_browser::McpOAuthBrowserManager>>,
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
    oauth_browser_manager: State<'_, Arc<crate::mcp::oauth_browser::McpOAuthBrowserManager>>,
) -> Result<(), String> {
    oauth_browser_manager
        .revoke_tokens(&server_id)
        .map_err(|e| e.to_string())
}

// ============================================================================
// Logging Configuration Commands
// ============================================================================

/// Logging configuration returned to the frontend
#[derive(serde::Serialize)]
pub struct LoggingConfigResponse {
    pub enabled: bool,
    pub log_dir: String,
}

/// Get logging configuration
#[tauri::command]
pub fn get_logging_config(
    config_manager: State<'_, ConfigManager>,
    access_logger: State<'_, Arc<crate::monitoring::logger::AccessLogger>>,
) -> Result<LoggingConfigResponse, String> {
    let config = config_manager.get();
    Ok(LoggingConfigResponse {
        enabled: config.logging.enable_access_log,
        log_dir: access_logger.log_dir().to_string_lossy().to_string(),
    })
}

/// Update logging configuration (enable/disable access logging)
#[tauri::command]
pub async fn update_logging_config(
    enabled: bool,
    config_manager: State<'_, ConfigManager>,
    access_logger: State<'_, Arc<crate::monitoring::logger::AccessLogger>>,
    mcp_access_logger: State<'_, Arc<crate::monitoring::mcp_logger::McpAccessLogger>>,
) -> Result<(), String> {
    // Update config
    config_manager
        .update(|config| {
            config.logging.enable_access_log = enabled;
        })
        .map_err(|e| e.to_string())?;

    config_manager.save().await.map_err(|e| e.to_string())?;

    // Update the loggers in real-time
    access_logger.set_enabled(enabled);
    mcp_access_logger.set_enabled(enabled);

    Ok(())
}

/// Open the logs folder in the system file manager
#[tauri::command]
pub async fn open_logs_folder(
    access_logger: State<'_, Arc<crate::monitoring::logger::AccessLogger>>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    use tauri_plugin_shell::ShellExt;

    let log_dir = access_logger.log_dir();

    // Ensure directory exists
    if !log_dir.exists() {
        std::fs::create_dir_all(log_dir)
            .map_err(|e| format!("Failed to create log directory: {}", e))?;
    }

    // Open in system file manager
    app.shell()
        .open(log_dir.to_string_lossy().as_ref(), None)
        .map_err(|e| format!("Failed to open logs folder: {}", e))?;

    Ok(())
}
