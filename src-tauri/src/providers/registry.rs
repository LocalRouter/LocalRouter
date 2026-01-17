//! Provider registry for managing provider types and instances
//!
//! The registry serves as the central hub for all provider management:
//! - Registers provider factory types at startup
//! - Creates and manages provider instances dynamically
//! - Integrates with health check system
//! - Provides model aggregation across all providers

use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

use super::{ModelInfo, ModelProvider, ProviderHealth};
use super::factory::{ProviderFactory, SetupParameter};
use super::health::HealthCheckManager;
use crate::utils::errors::{AppError, AppResult};

/// Central registry for managing provider types and instances
pub struct ProviderRegistry {
    /// Registered provider factories by type name
    factories: RwLock<HashMap<String, Arc<dyn ProviderFactory>>>,

    /// Active provider instances by instance name
    instances: RwLock<HashMap<String, ProviderInstance>>,

    /// Health check manager for all providers
    health_manager: Arc<HealthCheckManager>,

    /// Cached models from all providers (for synchronous access in UI)
    cached_models: RwLock<Vec<ModelInfo>>,
}

/// A registered provider instance
#[derive(Clone)]
pub struct ProviderInstance {
    /// User-defined instance name (e.g., "my-openai", "local-ollama")
    pub instance_name: String,

    /// Provider type (e.g., "ollama", "openai", "anthropic")
    pub provider_type: String,

    /// The provider implementation
    pub provider: Arc<dyn ModelProvider>,

    /// Configuration used to create this instance
    pub config: HashMap<String, String>,

    /// When this instance was created
    pub created_at: DateTime<Utc>,

    /// Whether this instance is enabled
    pub enabled: bool,
}

/// Information about a provider type (for setup UI)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderTypeInfo {
    pub provider_type: String,
    pub description: String,
    pub setup_parameters: Vec<SetupParameter>,
}

/// Information about a provider instance (for listing)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderInstanceInfo {
    pub instance_name: String,
    pub provider_type: String,
    pub provider_name: String,
    pub created_at: DateTime<Utc>,
    pub enabled: bool,
}

impl ProviderRegistry {
    // ===== INITIALIZATION =====

    /// Create a new provider registry
    pub fn new(health_manager: Arc<HealthCheckManager>) -> Self {
        info!("Creating new provider registry");
        Self {
            factories: RwLock::new(HashMap::new()),
            instances: RwLock::new(HashMap::new()),
            health_manager,
            cached_models: RwLock::new(Vec::new()),
        }
    }

    // ===== FACTORY MANAGEMENT (Setup Phase) =====

    /// Register a provider factory (called at startup)
    pub fn register_factory(&self, factory: Arc<dyn ProviderFactory>) {
        let provider_type = factory.provider_type().to_string();
        info!("Registering provider factory: {}", provider_type);
        self.factories.write().insert(provider_type, factory);
    }

    /// Get a factory by provider type
    pub fn get_factory(&self, provider_type: &str) -> Option<Arc<dyn ProviderFactory>> {
        self.factories.read().get(provider_type).cloned()
    }

    /// List all available provider types with setup parameters
    ///
    /// Used by: UI for showing available provider types
    pub fn list_provider_types(&self) -> Vec<ProviderTypeInfo> {
        self.factories
            .read()
            .values()
            .map(|factory| ProviderTypeInfo {
                provider_type: factory.provider_type().to_string(),
                description: factory.description().to_string(),
                setup_parameters: factory.setup_parameters(),
            })
            .collect()
    }

    // ===== INSTANCE MANAGEMENT (Runtime) =====

    /// Create and register a provider instance from configuration
    ///
    /// This method:
    /// 1. Validates the configuration
    /// 2. Creates the provider using the factory
    /// 3. Registers it with the health check manager
    /// 4. Stores the instance
    ///
    /// Used by: Configuration loading, UI provider creation
    pub async fn create_provider(
        &self,
        instance_name: String,
        provider_type: String,
        config: HashMap<String, String>,
    ) -> AppResult<()> {
        info!(
            "Creating provider instance '{}' of type '{}'",
            instance_name, provider_type
        );

        // Check for duplicate instance name
        if self.instances.read().contains_key(&instance_name) {
            return Err(AppError::Config(format!(
                "Provider instance '{}' already exists",
                instance_name
            )));
        }

        // Get factory
        let factory = self.get_factory(&provider_type).ok_or_else(|| {
            AppError::Config(format!("Unknown provider type: {}", provider_type))
        })?;

        // Create provider
        let provider = factory.create(instance_name.clone(), config.clone())?;

        // Register with health check manager
        self.health_manager.register_provider(provider.clone()).await;

        // Store instance
        let instance = ProviderInstance {
            instance_name: instance_name.clone(),
            provider_type,
            provider,
            config,
            created_at: Utc::now(),
            enabled: true,
        };

        self.instances.write().insert(instance_name.clone(), instance);

        info!("Successfully created provider instance: {}", instance_name);
        Ok(())
    }

    /// Get a provider instance by name
    ///
    /// Returns None if provider doesn't exist or is disabled
    ///
    /// Used by: Web server for routing requests
    pub fn get_provider(&self, instance_name: &str) -> Option<Arc<dyn ModelProvider>> {
        self.instances.read().get(instance_name).and_then(|inst| {
            if inst.enabled {
                Some(inst.provider.clone())
            } else {
                debug!("Provider '{}' exists but is disabled", instance_name);
                None
            }
        })
    }

    /// Get a provider instance by name (includes disabled)
    ///
    /// Used by: Admin commands, health checks
    pub fn get_provider_unchecked(&self, instance_name: &str) -> Option<Arc<dyn ModelProvider>> {
        self.instances
            .read()
            .get(instance_name)
            .map(|inst| inst.provider.clone())
    }

    /// List all provider instances
    ///
    /// Used by: GET /v1/models endpoint, UI provider list
    pub fn list_providers(&self) -> Vec<ProviderInstanceInfo> {
        self.instances
            .read()
            .values()
            .map(|inst| ProviderInstanceInfo {
                instance_name: inst.instance_name.clone(),
                provider_type: inst.provider_type.clone(),
                provider_name: inst.provider.name().to_string(),
                created_at: inst.created_at,
                enabled: inst.enabled,
            })
            .collect()
    }

    /// Get provider instance configuration
    ///
    /// Used by: UI for editing provider configuration
    pub fn get_provider_config(&self, instance_name: &str) -> Option<HashMap<String, String>> {
        self.instances
            .read()
            .get(instance_name)
            .map(|inst| inst.config.clone())
    }

    /// Update a provider instance configuration
    ///
    /// This method:
    /// 1. Removes the old instance
    /// 2. Creates a new instance with the updated config
    /// 3. Preserves the enabled state and created_at timestamp
    ///
    /// Used by: UI provider management
    pub async fn update_provider(
        &self,
        instance_name: String,
        provider_type: String,
        config: HashMap<String, String>,
    ) -> AppResult<()> {
        info!(
            "Updating provider instance '{}' of type '{}'",
            instance_name, provider_type
        );

        // Get the old instance to preserve state
        let (enabled, created_at) = {
            let instances = self.instances.read();
            let instance = instances.get(&instance_name).ok_or_else(|| {
                AppError::Config(format!("Provider instance '{}' not found", instance_name))
            })?;
            (instance.enabled, instance.created_at)
        };

        // Remove the old instance
        self.instances.write().remove(&instance_name);

        // Get factory
        let factory = self.get_factory(&provider_type).ok_or_else(|| {
            AppError::Config(format!("Unknown provider type: {}", provider_type))
        })?;

        // Create new provider with updated config
        let provider = factory.create(instance_name.clone(), config.clone())?;

        // Register with health check manager
        self.health_manager.register_provider(provider.clone()).await;

        // Store updated instance
        let instance = ProviderInstance {
            instance_name: instance_name.clone(),
            provider_type,
            provider,
            config,
            created_at, // Preserve original creation time
            enabled,    // Preserve enabled state
        };

        self.instances.write().insert(instance_name.clone(), instance);

        info!("Successfully updated provider instance: {}", instance_name);
        Ok(())
    }

    /// Get all enabled providers
    ///
    /// Used by: Smart router for finding available providers
    #[allow(dead_code)]
    pub fn get_enabled_providers(&self) -> Vec<Arc<dyn ModelProvider>> {
        self.instances
            .read()
            .values()
            .filter(|inst| inst.enabled)
            .map(|inst| inst.provider.clone())
            .collect()
    }

    /// Remove a provider instance
    ///
    /// Used by: UI provider management
    pub fn remove_provider(&self, instance_name: &str) -> AppResult<()> {
        self.instances
            .write()
            .remove(instance_name)
            .ok_or_else(|| {
                AppError::Config(format!("Provider instance '{}' not found", instance_name))
            })?;

        info!("Removed provider instance: {}", instance_name);
        Ok(())
    }

    /// Enable or disable a provider instance
    ///
    /// Used by: UI provider management, circuit breaker
    pub fn set_provider_enabled(&self, instance_name: &str, enabled: bool) -> AppResult<()> {
        let mut instances = self.instances.write();
        let instance = instances.get_mut(instance_name).ok_or_else(|| {
            AppError::Config(format!("Provider instance '{}' not found", instance_name))
        })?;

        instance.enabled = enabled;
        info!(
            "Set provider '{}' enabled: {}",
            instance_name, enabled
        );
        Ok(())
    }

    // ===== MODEL AGGREGATION (for /v1/models endpoint) =====

    /// Get cached models (synchronous, for UI)
    ///
    /// Returns the last fetched models without making any network calls.
    /// If the cache is empty, returns an empty list.
    /// Call `refresh_model_cache()` to update the cache.
    pub fn get_cached_models(&self) -> Vec<ModelInfo> {
        self.cached_models.read().clone()
    }

    /// Refresh the model cache (asynchronous)
    ///
    /// Fetches models from all enabled providers and updates the cache.
    /// This should be called after providers are loaded or when models change.
    pub async fn refresh_model_cache(&self) -> AppResult<()> {
        let models = self.list_all_models().await?;
        *self.cached_models.write() = models;
        Ok(())
    }

    /// List all models from all enabled providers
    ///
    /// This is the main method for GET /v1/models endpoint.
    /// It queries all enabled providers and aggregates their models.
    ///
    /// Returns: Combined list of ModelInfo from all providers
    pub async fn list_all_models(&self) -> AppResult<Vec<ModelInfo>> {
        let enabled_instances: Vec<(String, Arc<dyn ModelProvider>)> = self
            .instances
            .read()
            .values()
            .filter(|inst| inst.enabled)
            .map(|inst| (inst.instance_name.clone(), inst.provider.clone()))
            .collect();

        let mut all_models = Vec::new();

        for (instance_name, provider) in enabled_instances {
            match provider.list_models().await {
                Ok(mut models) => {
                    // Override provider field with instance name
                    for model in &mut models {
                        model.provider = instance_name.clone();
                    }
                    all_models.extend(models);
                }
                Err(e) => {
                    warn!(
                        "Failed to list models from provider '{}': {}",
                        instance_name,
                        e
                    );
                    // Continue with other providers
                }
            }
        }

        debug!("Listed {} models from all enabled providers", all_models.len());
        Ok(all_models)
    }

    /// List models from a specific provider instance
    ///
    /// Used by: UI for showing models per provider
    pub async fn list_provider_models(&self, instance_name: &str) -> AppResult<Vec<ModelInfo>> {
        let provider = self.get_provider_unchecked(instance_name).ok_or_else(|| {
            AppError::Config(format!("Provider instance '{}' not found", instance_name))
        })?;

        let mut models = provider.list_models().await?;

        // Override provider field with instance name
        for model in &mut models {
            model.provider = instance_name.to_string();
        }

        Ok(models)
    }

    // ===== HEALTH CHECKS =====

    /// Get health status for all provider instances
    ///
    /// Used by: Health monitoring, UI dashboard
    pub async fn get_all_health(&self) -> HashMap<String, ProviderHealth> {
        self.health_manager.get_all_health().await
    }

    /// Get health status for a specific provider
    ///
    /// Used by: Provider selection in router
    #[allow(dead_code)]
    pub async fn get_provider_health(&self, instance_name: &str) -> Option<ProviderHealth> {
        if let Some(provider) = self.get_provider_unchecked(instance_name) {
            self.health_manager.get_health(provider.name()).await
        } else {
            None
        }
    }

    // ===== CONFIGURATION INTEGRATION =====

    /// Load providers from configuration on startup
    ///
    /// This method:
    /// 1. Reads provider configs from config manager
    /// 2. Loads API keys from encrypted storage (if available)
    /// 3. Creates provider instances
    /// 4. Registers them with health manager
    ///
    /// Called once during application initialization
    ///
    /// Note: This is a simplified version that doesn't integrate with encrypted storage yet.
    /// The full integration will be added when encrypted storage is ready.
    #[allow(dead_code)]
    pub async fn load_from_config_simple(
        &self,
        provider_configs: Vec<SimpleProviderConfig>,
    ) -> AppResult<()> {
        info!("Loading {} providers from configuration", provider_configs.len());

        for provider_config in provider_configs {
            let mut config = HashMap::new();

            // Add endpoint if provided
            if let Some(endpoint) = provider_config.endpoint {
                config.insert("base_url".to_string(), endpoint);
            }

            // Add parameters
            for (key, value) in provider_config.parameters {
                config.insert(key, value);
            }

            // Create instance
            let result = self
                .create_provider(
                    provider_config.name.clone(),
                    provider_config.provider_type,
                    config,
                )
                .await;

            match result {
                Ok(()) => {
                    // Set enabled state
                    if let Err(e) = self.set_provider_enabled(
                        &provider_config.name,
                        provider_config.enabled,
                    ) {
                        warn!(
                            "Failed to set provider '{}' enabled state: {}",
                            provider_config.name, e
                        );
                    }
                }
                Err(e) => {
                    warn!(
                        "Failed to load provider '{}': {}",
                        provider_config.name, e
                    );
                }
            }
        }

        info!("Provider loading complete");
        Ok(())
    }
}

/// Simplified provider config for loading (without encrypted storage integration yet)
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimpleProviderConfig {
    pub name: String,
    pub provider_type: String,
    pub enabled: bool,
    pub endpoint: Option<String>,
    pub parameters: HashMap<String, String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::factory::OllamaProviderFactory;

    #[tokio::test]
    async fn test_registry_creation() {
        let health_mgr = Arc::new(HealthCheckManager::default());
        let registry = ProviderRegistry::new(health_mgr);

        assert_eq!(registry.list_providers().len(), 0);
        assert_eq!(registry.list_provider_types().len(), 0);
    }

    #[tokio::test]
    async fn test_register_factory() {
        let health_mgr = Arc::new(HealthCheckManager::default());
        let registry = ProviderRegistry::new(health_mgr);

        registry.register_factory(Arc::new(OllamaProviderFactory));

        let types = registry.list_provider_types();
        assert_eq!(types.len(), 1);
        assert_eq!(types[0].provider_type, "ollama");
    }

    #[tokio::test]
    async fn test_create_provider_instance() {
        let health_mgr = Arc::new(HealthCheckManager::default());
        let registry = ProviderRegistry::new(health_mgr);

        registry.register_factory(Arc::new(OllamaProviderFactory));

        let mut config = HashMap::new();
        config.insert("base_url".to_string(), "http://localhost:11434".to_string());

        registry
            .create_provider("my-ollama".to_string(), "ollama".to_string(), config)
            .await
            .unwrap();

        let instances = registry.list_providers();
        assert_eq!(instances.len(), 1);
        assert_eq!(instances[0].instance_name, "my-ollama");
        assert_eq!(instances[0].provider_type, "ollama");
        assert!(instances[0].enabled);
    }

    #[tokio::test]
    async fn test_duplicate_instance_name() {
        let health_mgr = Arc::new(HealthCheckManager::default());
        let registry = ProviderRegistry::new(health_mgr);

        registry.register_factory(Arc::new(OllamaProviderFactory));

        let config = HashMap::new();

        registry
            .create_provider("test".to_string(), "ollama".to_string(), config.clone())
            .await
            .unwrap();

        let result = registry
            .create_provider("test".to_string(), "ollama".to_string(), config)
            .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_get_provider() {
        let health_mgr = Arc::new(HealthCheckManager::default());
        let registry = ProviderRegistry::new(health_mgr);

        registry.register_factory(Arc::new(OllamaProviderFactory));

        let config = HashMap::new();

        registry
            .create_provider("test".to_string(), "ollama".to_string(), config)
            .await
            .unwrap();

        let provider = registry.get_provider("test");
        assert!(provider.is_some());
        assert_eq!(provider.unwrap().name(), "ollama");
    }

    #[tokio::test]
    async fn test_enable_disable_provider() {
        let health_mgr = Arc::new(HealthCheckManager::default());
        let registry = ProviderRegistry::new(health_mgr);

        registry.register_factory(Arc::new(OllamaProviderFactory));

        let config = HashMap::new();

        registry
            .create_provider("test".to_string(), "ollama".to_string(), config)
            .await
            .unwrap();

        // Initially enabled
        assert!(registry.get_provider("test").is_some());

        // Disable
        registry.set_provider_enabled("test", false).unwrap();
        assert!(registry.get_provider("test").is_none());

        // Re-enable
        registry.set_provider_enabled("test", true).unwrap();
        assert!(registry.get_provider("test").is_some());
    }

    #[tokio::test]
    async fn test_remove_provider() {
        let health_mgr = Arc::new(HealthCheckManager::default());
        let registry = ProviderRegistry::new(health_mgr);

        registry.register_factory(Arc::new(OllamaProviderFactory));

        let config = HashMap::new();

        registry
            .create_provider("test".to_string(), "ollama".to_string(), config)
            .await
            .unwrap();

        assert_eq!(registry.list_providers().len(), 1);

        registry.remove_provider("test").unwrap();

        assert_eq!(registry.list_providers().len(), 0);
    }
}
