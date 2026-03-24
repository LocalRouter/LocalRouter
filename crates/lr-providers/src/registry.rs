//! Provider registry for managing provider types and instances
//!
//! The registry serves as the central hub for all provider management:
//! - Registers provider factory types at startup
//! - Creates and manages provider instances dynamically
//! - Integrates with health check system
//! - Provides model aggregation across all providers

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[cfg(test)]
use chrono::Duration;
use chrono::{DateTime, Utc};
use futures::future::join_all;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

use super::factory::{ProviderFactory, SetupParameter};
use super::health::HealthCheckManager;
use super::{ModelInfo, ModelProvider, ProviderHealth};
use lr_config::{FreeTierKind, ModelCacheConfig};
use lr_types::{AppError, AppResult};

/// Format a number with K/M/B suffix for human-readable display.
pub fn format_number(n: u64) -> String {
    if n == 0 {
        return "0".to_string();
    }
    if n >= 1_000_000_000 && n.is_multiple_of(1_000_000_000) {
        format!("{}B", n / 1_000_000_000)
    } else if n >= 1_000_000_000 {
        format!("{:.1}B", n as f64 / 1_000_000_000.0)
    } else if n >= 1_000_000 && n.is_multiple_of(1_000_000) {
        format!("{}M", n / 1_000_000)
    } else if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 && n.is_multiple_of(1_000) {
        format!("{}K", n / 1_000)
    } else if n >= 1_000 {
        format!("{:.1}K", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

/// Generate human-readable (short_text, long_text) descriptions from a FreeTierKind.
///
/// - `short_text`: brief label for provider cards (empty string for `None`)
/// - `long_text`: detailed description for free tier configuration tab
pub fn free_tier_description_texts(kind: &FreeTierKind) -> (String, String) {
    match kind {
        FreeTierKind::None => (
            String::new(),
            "No free tier available. All API usage is billed.".to_string(),
        ),
        FreeTierKind::AlwaysFreeLocal => (
            "Free — runs locally".to_string(),
            "Runs entirely on your machine. No API costs, no rate limits.".to_string(),
        ),
        FreeTierKind::Subscription => (
            "Included in subscription".to_string(),
            "Included in your existing subscription at no additional cost.".to_string(),
        ),
        FreeTierKind::RateLimitedFree {
            max_rpm,
            max_rpd,
            max_tpm,
            max_tpd,
            max_monthly_calls,
            max_monthly_tokens,
        } => {
            // Build limit parts for display
            let mut parts = Vec::new();
            if *max_rpm > 0 {
                parts.push(format!("{} req/min", max_rpm));
            }
            if *max_rpd > 0 {
                parts.push(format!("{} req/day", format_number(*max_rpd as u64)));
            }
            if *max_tpm > 0 {
                parts.push(format!("{} tokens/min", format_number(*max_tpm)));
            }
            if *max_tpd > 0 {
                parts.push(format!("{} tokens/day", format_number(*max_tpd)));
            }
            if *max_monthly_calls > 0 {
                parts.push(format!(
                    "{} calls/mo",
                    format_number(*max_monthly_calls as u64)
                ));
            }
            if *max_monthly_tokens > 0 {
                parts.push(format!("{} tokens/mo", format_number(*max_monthly_tokens)));
            }

            let short = if parts.len() >= 2 {
                format!("Free tier: {}", parts[..2].join(", "))
            } else if parts.len() == 1 {
                format!("Free tier: {}", parts[0])
            } else {
                "Free tier available".to_string()
            };

            let long = if parts.is_empty() {
                "Free access within rate limits. Router auto-skips when exhausted.".to_string()
            } else {
                format!(
                    "Free access within rate limits: {}. Router auto-skips when exhausted.",
                    parts.join(", ")
                )
            };

            (short, long)
        }
        FreeTierKind::CreditBased {
            budget_usd,
            reset_period,
            ..
        } => {
            if *budget_usd == 0.0 {
                (
                    "Free models available".to_string(),
                    "Some models available for free via provider API.".to_string(),
                )
            } else {
                let period_text = match reset_period {
                    lr_config::FreeTierResetPeriod::Monthly => "monthly",
                    lr_config::FreeTierResetPeriod::Daily => "daily",
                    lr_config::FreeTierResetPeriod::Never => "one-time",
                };
                let budget_str = if *budget_usd == budget_usd.floor() {
                    format!("${}", *budget_usd as u64)
                } else {
                    format!("${:.2}", budget_usd)
                };
                let short = if *reset_period == lr_config::FreeTierResetPeriod::Never {
                    format!("{} free credits", budget_str)
                } else {
                    format!("{}/mo free credits", budget_str)
                };
                let long = format!(
                    "{} in {} free credits. Router auto-skips when exhausted.",
                    budget_str, period_text
                );
                (short, long)
            }
        }
        FreeTierKind::FreeModelsOnly {
            max_rpm,
            free_model_patterns,
        } => {
            let short = if *max_rpm > 0 {
                format!("Free models: {} req/min", max_rpm)
            } else {
                "Free models available".to_string()
            };
            let model_count = free_model_patterns.len();
            let long = if *max_rpm > 0 {
                format!(
                    "{} free model{}. Rate-limited to {} req/min.",
                    model_count,
                    if model_count == 1 { "" } else { "s" },
                    max_rpm
                )
            } else {
                format!(
                    "{} free model{} available.",
                    model_count,
                    if model_count == 1 { "" } else { "s" },
                )
            };
            (short, long)
        }
    }
}

/// Cached model list for a single provider
#[derive(Clone)]
struct ModelCache {
    /// The cached models
    models: Vec<ModelInfo>,
    /// When this cache was populated
    fetched_at: DateTime<Utc>,
    /// Provider-specific TTL (from config or default)
    ttl_seconds: u64,
}

impl ModelCache {
    fn new(models: Vec<ModelInfo>, ttl_seconds: u64) -> Self {
        Self {
            models,
            fetched_at: Utc::now(),
            ttl_seconds,
        }
    }

    /// Check if cache is expired
    fn is_expired(&self) -> bool {
        let elapsed = Utc::now().signed_duration_since(self.fetched_at);
        elapsed.num_seconds() as u64 >= self.ttl_seconds
    }

    /// Get remaining seconds until expiration
    fn expires_in(&self) -> i64 {
        let elapsed = Utc::now().signed_duration_since(self.fetched_at);
        (self.ttl_seconds as i64) - elapsed.num_seconds()
    }
}

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

    /// Per-provider model cache
    model_cache: Arc<RwLock<HashMap<String, ModelCache>>>,

    /// Cache configuration
    cache_config: Arc<RwLock<ModelCacheConfig>>,

    /// Guard to prevent concurrent incremental refreshes
    refresh_in_progress: AtomicBool,
}

/// A registered provider instance
#[derive(Clone)]
pub struct ProviderInstance {
    /// User-defined instance name (e.g., "my-openai", "local-ollama")
    pub instance_name: String,

    /// Provider type (e.g., "ollama", "openai", "anthropic")
    pub provider_type: String,

    /// Catalog provider ID for models.dev lookup (may differ from provider_type)
    pub catalog_provider_id: Option<String>,

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
    pub display_name: String,
    pub category: super::factory::ProviderCategory,
    pub description: String,
    pub setup_parameters: Vec<SetupParameter>,
    pub default_free_tier: FreeTierKind,
    pub free_tier_short_text: String,
    pub free_tier_long_text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub free_tier_notes: Option<String>,
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
    pub fn new() -> Self {
        info!("Creating new provider registry");
        Self {
            factories: RwLock::new(HashMap::new()),
            instances: RwLock::new(HashMap::new()),
            health_manager: Arc::new(HealthCheckManager::default()),
            cached_models: RwLock::new(Vec::new()),
            model_cache: Arc::new(RwLock::new(HashMap::new())),
            cache_config: Arc::new(RwLock::new(ModelCacheConfig::default())),
            refresh_in_progress: AtomicBool::new(false),
        }
    }
}

impl Default for ProviderRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ProviderRegistry {
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

    /// Get the default free tier config for a provider type from its factory
    pub fn get_factory_default_free_tier(
        &self,
        provider_type: &lr_config::ProviderType,
    ) -> lr_config::FreeTierKind {
        let type_str = serde_json::to_string(provider_type)
            .unwrap_or_default()
            .trim_matches('"')
            .to_string();
        self.factories
            .read()
            .get(&type_str)
            .map(|f| f.default_free_tier())
            .unwrap_or(lr_config::FreeTierKind::None)
    }

    /// List all available provider types with setup parameters
    ///
    /// Used by: UI for showing available provider types
    pub fn list_provider_types(&self) -> Vec<ProviderTypeInfo> {
        self.factories
            .read()
            .values()
            .map(|factory| {
                let free_tier = factory.default_free_tier();
                let (short_text, long_text) = free_tier_description_texts(&free_tier);
                let notes = factory.free_tier_notes().map(|s| s.to_string());
                let long_text = if let Some(ref notes) = notes {
                    format!("{}\n\n{}", long_text, notes)
                } else {
                    long_text
                };
                ProviderTypeInfo {
                    provider_type: factory.provider_type().to_string(),
                    display_name: factory.display_name().to_string(),
                    category: factory.category(),
                    description: factory.description().to_string(),
                    setup_parameters: factory.setup_parameters(),
                    default_free_tier: free_tier,
                    free_tier_short_text: short_text,
                    free_tier_long_text: long_text,
                    free_tier_notes: notes,
                }
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
        let factory = self
            .get_factory(&provider_type)
            .ok_or_else(|| AppError::Config(format!("Unknown provider type: {}", provider_type)))?;

        // Create provider
        let catalog_provider_id = factory.catalog_provider_id().map(|s| s.to_string());
        let provider = factory.create(instance_name.clone(), config.clone())?;

        // Register with health check manager
        self.health_manager
            .register_provider(provider.clone())
            .await;

        // Store instance
        let instance = ProviderInstance {
            instance_name: instance_name.clone(),
            provider_type,
            catalog_provider_id,
            provider,
            config,
            created_at: Utc::now(),
            enabled: true,
        };

        self.instances
            .write()
            .insert(instance_name.clone(), instance);

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
        let factory = self
            .get_factory(&provider_type)
            .ok_or_else(|| AppError::Config(format!("Unknown provider type: {}", provider_type)))?;

        // Create new provider with updated config
        let catalog_provider_id = factory.catalog_provider_id().map(|s| s.to_string());
        let provider = factory.create(instance_name.clone(), config.clone())?;

        // Register with health check manager
        self.health_manager
            .register_provider(provider.clone())
            .await;

        // Store updated instance
        let instance = ProviderInstance {
            instance_name: instance_name.clone(),
            provider_type,
            catalog_provider_id,
            provider,
            config,
            created_at, // Preserve original creation time
            enabled,    // Preserve enabled state
        };

        self.instances
            .write()
            .insert(instance_name.clone(), instance);

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
    ///
    /// This operation is idempotent - removing a non-existent provider succeeds silently.
    pub fn remove_provider(&self, instance_name: &str) -> AppResult<()> {
        if self.instances.write().remove(instance_name).is_some() {
            info!("Removed provider instance: {}", instance_name);
        }
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
        info!("Set provider '{}' enabled: {}", instance_name, enabled);
        Ok(())
    }

    /// Check if a provider instance is enabled
    ///
    /// Returns None if the provider is not found
    pub fn is_provider_enabled(&self, instance_name: &str) -> Option<bool> {
        self.instances
            .read()
            .get(instance_name)
            .map(|inst| inst.enabled)
    }

    // ===== MODEL CACHE MANAGEMENT =====

    /// Get TTL for a specific provider instance
    fn get_provider_ttl(&self, instance_name: &str) -> u64 {
        let config = self.cache_config.read();

        // Check instance-specific override first
        if let Some(&ttl) = config.provider_ttl_overrides.get(instance_name) {
            return ttl;
        }

        // Check provider-type override
        if let Some(instance) = self.instances.read().get(instance_name) {
            if let Some(&ttl) = config.provider_ttl_overrides.get(&instance.provider_type) {
                return ttl;
            }
        }

        // Use default
        config.default_ttl_seconds
    }

    /// Get cached models for a provider
    fn get_cached_models_for_provider(&self, instance_name: &str) -> Option<ModelCache> {
        self.model_cache.read().get(instance_name).cloned()
    }

    /// Update cache for a provider
    async fn update_model_cache(&self, instance_name: &str, models: Vec<ModelInfo>) {
        let ttl = self.get_provider_ttl(instance_name);
        let cache = ModelCache::new(models, ttl);
        self.model_cache
            .write()
            .insert(instance_name.to_string(), cache);
        debug!(
            "Updated model cache for '{}' (TTL: {}s)",
            instance_name, ttl
        );
    }

    /// Get models from models.dev catalog for a provider type
    fn get_models_from_catalog(&self, provider_type: &str) -> Vec<ModelInfo> {
        lr_catalog::models()
            .iter()
            .filter(|m| m.id.starts_with(&format!("{}/", provider_type)))
            .map(|m| {
                let capabilities = match m.modality {
                    lr_catalog::Modality::Multimodal => {
                        vec![crate::Capability::Chat, crate::Capability::Vision]
                    }
                    _ => vec![crate::Capability::Chat],
                };

                ModelInfo {
                    id: m.id.split('/').next_back().unwrap_or(m.id).to_string(),
                    name: m.name.to_string(),
                    provider: provider_type.to_string(),
                    parameter_count: None,
                    context_window: m.context_length,
                    supports_streaming: true,
                    capabilities,
                    detailed_capabilities: None,
                }
            })
            .collect()
    }

    /// Fallback to catalog or stale cache
    async fn fallback_to_catalog_or_stale_cache(
        &self,
        instance_name: &str,
        provider_type: &str,
    ) -> AppResult<Vec<ModelInfo>> {
        let config = self.cache_config.read();

        if config.use_catalog_fallback {
            // Try models.dev catalog
            let catalog_models = self.get_models_from_catalog(provider_type);
            if !catalog_models.is_empty() {
                info!(
                    "Using models.dev catalog fallback for '{}' ({} models)",
                    instance_name,
                    catalog_models.len()
                );
                return Ok(catalog_models);
            }
        }

        // Use stale cache if available
        if let Some(cached) = self.get_cached_models_for_provider(instance_name) {
            warn!(
                "Using stale cache for '{}' (expired {} seconds ago)",
                instance_name,
                -cached.expires_in()
            );
            return Ok(cached.models.clone());
        }

        // Complete failure
        Err(AppError::Provider(format!(
            "No models available for '{}': API failed, no catalog data, no stale cache",
            instance_name
        )))
    }

    /// List models from a provider with caching
    pub async fn list_provider_models_cached(
        &self,
        instance_name: &str,
    ) -> AppResult<Vec<ModelInfo>> {
        // 1. Try cache first
        if let Some(cached) = self.get_cached_models_for_provider(instance_name) {
            if !cached.is_expired() {
                debug!(
                    "Using cached models for '{}' (expires in {}s)",
                    instance_name,
                    cached.expires_in()
                );
                return Ok(cached.models.clone());
            }
        }

        // 2. Try fetching from provider API
        let provider_instance = self
            .instances
            .read()
            .get(instance_name)
            .cloned()
            .ok_or_else(|| AppError::Config(format!("Provider '{}' not found", instance_name)))?;

        match provider_instance.provider.list_models().await {
            Ok(models) => {
                // Success: Cache and return
                self.update_model_cache(instance_name, models.clone()).await;
                Ok(models)
            }
            Err(e) => {
                warn!("Failed to fetch models from '{}': {}", instance_name, e);

                // 3. Fallback strategy - use catalog_provider_id if available, else provider_type
                let catalog_id = provider_instance
                    .catalog_provider_id
                    .as_deref()
                    .unwrap_or(&provider_instance.provider_type);
                self.fallback_to_catalog_or_stale_cache(instance_name, catalog_id)
                    .await
            }
        }
    }

    /// Invalidate cache for a specific provider
    pub fn invalidate_provider_cache(&self, instance_name: &str) {
        self.model_cache.write().remove(instance_name);
        info!("Invalidated model cache for '{}'", instance_name);
    }

    /// Invalidate all caches
    pub fn invalidate_all_caches(&self) {
        self.model_cache.write().clear();
        info!("Invalidated all model caches");
    }

    /// Update cache configuration at runtime
    pub fn update_cache_config(&self, config: ModelCacheConfig) {
        *self.cache_config.write() = config;
        info!("Model cache configuration updated");
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
    /// Uses caching to minimize API calls.
    ///
    /// Returns: Combined list of ModelInfo from all providers
    pub async fn list_all_models(&self) -> AppResult<Vec<ModelInfo>> {
        let enabled_instances: Vec<String> = self
            .instances
            .read()
            .values()
            .filter(|inst| inst.enabled)
            .map(|inst| inst.instance_name.clone())
            .collect();

        // Fetch all providers in parallel
        let results = join_all(
            enabled_instances
                .iter()
                .map(|name| self.list_provider_models_cached(name)),
        )
        .await;

        let mut all_models = Vec::new();
        for (instance_name, result) in enabled_instances.iter().zip(results) {
            match result {
                Ok(mut models) => {
                    for model in &mut models {
                        model.provider = instance_name.clone();
                    }
                    all_models.extend(models);
                }
                Err(e) => {
                    warn!("Failed to list models from '{}': {}", instance_name, e);
                }
            }
        }

        debug!(
            "Listed {} models from all enabled providers",
            all_models.len()
        );
        Ok(all_models)
    }

    /// Get all cached models instantly without network calls.
    ///
    /// Returns whatever is in the per-provider caches (even if expired).
    /// Used for instant UI display before a fresh fetch completes.
    pub fn get_all_cached_models_instant(&self) -> Vec<ModelInfo> {
        let cache = self.model_cache.read();
        let enabled: std::collections::HashSet<String> = self
            .instances
            .read()
            .values()
            .filter(|inst| inst.enabled)
            .map(|inst| inst.instance_name.clone())
            .collect();

        let mut all_models = Vec::new();
        for (instance_name, cached) in cache.iter() {
            if !enabled.contains(instance_name) {
                continue;
            }
            let mut models = cached.models.clone();
            for model in &mut models {
                model.provider = instance_name.clone();
            }
            all_models.extend(models);
        }
        all_models
    }

    /// Get the list of enabled provider instance names
    pub fn get_enabled_instance_names(&self) -> Vec<String> {
        self.instances
            .read()
            .values()
            .filter(|inst| inst.enabled)
            .map(|inst| inst.instance_name.clone())
            .collect()
    }

    /// Get the provider type for a given instance name (e.g., "openai", "anthropic")
    pub fn get_provider_type_for_instance(&self, instance_name: &str) -> Option<String> {
        self.instances
            .read()
            .get(instance_name)
            .map(|i| i.provider_type.clone())
    }

    /// Check if any enabled provider's cache is expired or missing
    fn has_any_expired_or_missing_cache(&self) -> bool {
        let cache = self.model_cache.read();
        let instances = self.instances.read();
        for inst in instances.values() {
            if !inst.enabled {
                continue;
            }
            match cache.get(&inst.instance_name) {
                None => return true,
                Some(cached) if cached.is_expired() => return true,
                _ => {}
            }
        }
        false
    }

    /// List all models instantly from cache, triggering background refresh if stale.
    ///
    /// Returns cached models immediately (even if expired) and spawns a background
    /// refresh task when the cache is stale. This is the preferred method for REST
    /// API endpoints — never blocks on network I/O.
    ///
    /// Returns empty list if cache has never been populated (e.g., during startup).
    pub fn list_all_models_instant(self: &Arc<Self>) -> Vec<ModelInfo> {
        let models = self.get_all_cached_models_instant();

        if self.has_any_expired_or_missing_cache() && self.try_start_refresh() {
            let registry = Arc::clone(self);
            tokio::spawn(async move {
                if let Err(e) = registry.refresh_model_cache().await {
                    tracing::warn!("Background model cache refresh failed: {}", e);
                }
                registry.finish_refresh();
            });
        }

        models
    }

    /// Try to acquire the refresh lock. Returns true if acquired.
    pub fn try_start_refresh(&self) -> bool {
        self.refresh_in_progress
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
    }

    /// Release the refresh lock.
    pub fn finish_refresh(&self) {
        self.refresh_in_progress.store(false, Ordering::SeqCst);
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

    /// Get health status for all provider instances (on-demand)
    ///
    /// Performs health checks for all registered providers in parallel.
    /// Used by: UI dashboard when user views provider health.
    pub async fn get_all_health(&self) -> HashMap<String, ProviderHealth> {
        self.health_manager.check_all_health().await
    }

    /// Get health status for a specific provider (on-demand)
    ///
    /// Performs a health check for the specified provider.
    #[allow(dead_code)]
    pub async fn get_provider_health(&self, instance_name: &str) -> Option<ProviderHealth> {
        if let Some(_provider) = self.get_provider_unchecked(instance_name) {
            self.health_manager.check_health(instance_name).await
        } else {
            None
        }
    }

    /// Get list of all provider names for initiating streaming health checks
    pub fn get_provider_names(&self) -> Vec<String> {
        self.instances.read().keys().cloned().collect()
    }

    /// Perform streaming health checks, calling callback as each completes
    pub async fn check_all_health_streaming<F>(&self, on_result: F) -> Vec<String>
    where
        F: FnMut(String, ProviderHealth) + Send,
    {
        self.health_manager
            .check_all_health_streaming(on_result)
            .await
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
        info!(
            "Loading {} providers from configuration",
            provider_configs.len()
        );

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
                    if let Err(e) =
                        self.set_provider_enabled(&provider_config.name, provider_config.enabled)
                    {
                        warn!(
                            "Failed to set provider '{}' enabled state: {}",
                            provider_config.name, e
                        );
                    }
                }
                Err(e) => {
                    warn!("Failed to load provider '{}': {}", provider_config.name, e);
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
    use crate::factory::OllamaProviderFactory;

    #[tokio::test]
    async fn test_registry_creation() {
        let registry = ProviderRegistry::new();

        assert_eq!(registry.list_providers().len(), 0);
        assert_eq!(registry.list_provider_types().len(), 0);
    }

    #[tokio::test]
    async fn test_register_factory() {
        let registry = ProviderRegistry::new();

        registry.register_factory(Arc::new(OllamaProviderFactory));

        let types = registry.list_provider_types();
        assert_eq!(types.len(), 1);
        assert_eq!(types[0].provider_type, "ollama");
    }

    #[tokio::test]
    async fn test_create_provider_instance() {
        let registry = ProviderRegistry::new();

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
        let registry = ProviderRegistry::new();

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
        let registry = ProviderRegistry::new();

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
        let registry = ProviderRegistry::new();

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
        let registry = ProviderRegistry::new();

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

    #[test]
    fn test_model_cache_expiration() {
        let cache = ModelCache::new(vec![], 3600);
        assert!(!cache.is_expired());

        let mut old_cache = ModelCache::new(vec![], 3600);
        old_cache.fetched_at = Utc::now() - Duration::seconds(3601);
        assert!(old_cache.is_expired());
    }

    #[test]
    fn test_cache_expires_in() {
        let cache = ModelCache::new(vec![], 3600);
        assert!(cache.expires_in() > 3590);
        assert!(cache.expires_in() <= 3600);
    }

    #[tokio::test]
    async fn test_cache_config_update() {
        let registry = ProviderRegistry::new();

        let new_config = ModelCacheConfig {
            default_ttl_seconds: 7200,
            provider_ttl_overrides: std::collections::HashMap::from([("ollama".to_string(), 300)]),
            ..Default::default()
        };

        registry.update_cache_config(new_config.clone());

        let config = registry.cache_config.read();
        assert_eq!(config.default_ttl_seconds, 7200);
        assert_eq!(config.provider_ttl_overrides.get("ollama"), Some(&300));
    }

    #[tokio::test]
    async fn test_invalidate_cache() {
        let registry = ProviderRegistry::new();

        // Manually add a cache entry
        registry
            .model_cache
            .write()
            .insert("test".to_string(), ModelCache::new(vec![], 3600));

        assert!(registry.model_cache.read().contains_key("test"));

        registry.invalidate_provider_cache("test");

        assert!(!registry.model_cache.read().contains_key("test"));
    }

    #[tokio::test]
    async fn test_invalidate_all_caches() {
        let registry = ProviderRegistry::new();

        // Manually add cache entries
        registry
            .model_cache
            .write()
            .insert("test1".to_string(), ModelCache::new(vec![], 3600));
        registry
            .model_cache
            .write()
            .insert("test2".to_string(), ModelCache::new(vec![], 3600));

        assert_eq!(registry.model_cache.read().len(), 2);

        registry.invalidate_all_caches();

        assert_eq!(registry.model_cache.read().len(), 0);
    }

    #[test]
    fn test_format_number() {
        assert_eq!(format_number(0), "0");
        assert_eq!(format_number(5), "5");
        assert_eq!(format_number(999), "999");
        assert_eq!(format_number(1_000), "1K");
        assert_eq!(format_number(6_000), "6K");
        assert_eq!(format_number(14_400), "14.4K");
        assert_eq!(format_number(60_000), "60K");
        assert_eq!(format_number(100_000), "100K");
        assert_eq!(format_number(250_000), "250K");
        assert_eq!(format_number(500_000), "500K");
        assert_eq!(format_number(1_000_000), "1M");
        assert_eq!(format_number(1_000_000_000), "1B");
    }

    #[test]
    fn test_free_tier_texts_none() {
        let (short, long) = free_tier_description_texts(&FreeTierKind::None);
        assert!(short.is_empty());
        assert!(long.contains("No free tier"));
    }

    #[test]
    fn test_free_tier_texts_always_free_local() {
        let (short, long) = free_tier_description_texts(&FreeTierKind::AlwaysFreeLocal);
        assert!(short.contains("locally"));
        assert!(long.contains("your machine"));
    }

    #[test]
    fn test_free_tier_texts_subscription() {
        let (short, long) = free_tier_description_texts(&FreeTierKind::Subscription);
        assert!(short.contains("subscription"));
        assert!(long.contains("subscription"));
    }

    #[test]
    fn test_free_tier_texts_rate_limited() {
        let kind = FreeTierKind::RateLimitedFree {
            max_rpm: 10,
            max_rpd: 250,
            max_tpm: 250_000,
            max_tpd: 0,
            max_monthly_calls: 0,
            max_monthly_tokens: 0,
        };
        let (short, long) = free_tier_description_texts(&kind);
        assert!(short.contains("10 req/min"));
        assert!(long.contains("250K tokens/min"));
    }

    #[test]
    fn test_free_tier_texts_credit_based_zero() {
        let kind = FreeTierKind::CreditBased {
            budget_usd: 0.0,
            reset_period: lr_config::FreeTierResetPeriod::Never,
            detection: lr_config::CreditDetection::ProviderApi,
        };
        let (short, _long) = free_tier_description_texts(&kind);
        assert_eq!(short, "Free models available");
    }

    #[test]
    fn test_free_tier_texts_credit_based_monthly() {
        let kind = FreeTierKind::CreditBased {
            budget_usd: 5.0,
            reset_period: lr_config::FreeTierResetPeriod::Monthly,
            detection: lr_config::CreditDetection::LocalOnly,
        };
        let (short, long) = free_tier_description_texts(&kind);
        assert_eq!(short, "$5/mo free credits");
        assert!(long.contains("monthly"));
    }

    #[test]
    fn test_free_tier_texts_credit_based_onetime() {
        let kind = FreeTierKind::CreditBased {
            budget_usd: 25.0,
            reset_period: lr_config::FreeTierResetPeriod::Never,
            detection: lr_config::CreditDetection::LocalOnly,
        };
        let (short, long) = free_tier_description_texts(&kind);
        assert_eq!(short, "$25 free credits");
        assert!(long.contains("one-time"));
    }

    #[test]
    fn test_free_tier_texts_free_models_only() {
        let kind = FreeTierKind::FreeModelsOnly {
            free_model_patterns: vec!["model-a".to_string()],
            max_rpm: 3,
        };
        let (short, long) = free_tier_description_texts(&kind);
        assert!(short.contains("3 req/min"));
        assert!(long.contains("1 free model"));
    }

    #[tokio::test]
    async fn test_provider_type_info_includes_free_tier() {
        let registry = ProviderRegistry::new();
        registry.register_factory(Arc::new(OllamaProviderFactory));
        let types = registry.list_provider_types();
        assert_eq!(types.len(), 1);
        assert_eq!(types[0].default_free_tier, FreeTierKind::AlwaysFreeLocal);
        assert!(!types[0].free_tier_short_text.is_empty());
        assert!(!types[0].free_tier_long_text.is_empty());
    }

    #[tokio::test]
    async fn test_provider_type_info_no_notes_for_local() {
        let registry = ProviderRegistry::new();
        registry.register_factory(Arc::new(OllamaProviderFactory));
        let types = registry.list_provider_types();
        assert_eq!(types[0].free_tier_notes, None);
    }

    #[tokio::test]
    async fn test_provider_notes_appended_to_long_text() {
        use crate::factory::GeminiProviderFactory;
        let registry = ProviderRegistry::new();
        registry.register_factory(Arc::new(GeminiProviderFactory));
        let types = registry.list_provider_types();
        assert_eq!(types.len(), 1);
        // Notes should be present
        assert!(types[0].free_tier_notes.is_some());
        let notes = types[0].free_tier_notes.as_ref().unwrap();
        // Notes should also be appended to long_text
        assert!(
            types[0].free_tier_long_text.contains(notes.as_str()),
            "long_text should contain the notes"
        );
        // Long text should contain both the auto-generated part and the notes
        assert!(types[0].free_tier_long_text.contains("Router auto-skips"));
        assert!(types[0].free_tier_long_text.contains("Flash models"));
    }

    #[tokio::test]
    async fn test_new_providers_register_in_registry() {
        use crate::factory::{
            CloudflareAIProviderFactory, GitHubModelsProviderFactory, HuggingFaceProviderFactory,
            KlusterAIProviderFactory, Llm7ProviderFactory, NvidiaNimProviderFactory,
            ZhipuProviderFactory,
        };
        let registry = ProviderRegistry::new();
        registry.register_factory(Arc::new(GitHubModelsProviderFactory));
        registry.register_factory(Arc::new(NvidiaNimProviderFactory));
        registry.register_factory(Arc::new(CloudflareAIProviderFactory));
        registry.register_factory(Arc::new(Llm7ProviderFactory));
        registry.register_factory(Arc::new(KlusterAIProviderFactory));
        registry.register_factory(Arc::new(HuggingFaceProviderFactory));
        registry.register_factory(Arc::new(ZhipuProviderFactory));

        let types = registry.list_provider_types();
        assert_eq!(types.len(), 7);

        let type_names: Vec<&str> = types.iter().map(|t| t.provider_type.as_str()).collect();
        assert!(type_names.contains(&"github_models"));
        assert!(type_names.contains(&"nvidia_nim"));
        assert!(type_names.contains(&"cloudflare_ai"));
        assert!(type_names.contains(&"llm7"));
        assert!(type_names.contains(&"kluster_ai"));
        assert!(type_names.contains(&"huggingface"));
        assert!(type_names.contains(&"zhipu"));

        // All should have notes
        for t in &types {
            assert!(
                t.free_tier_notes.is_some(),
                "{} should have notes in ProviderTypeInfo",
                t.provider_type
            );
        }
    }

    #[tokio::test]
    async fn test_provider_type_info_serialization_with_notes() {
        use crate::factory::GeminiProviderFactory;
        let registry = ProviderRegistry::new();
        registry.register_factory(Arc::new(GeminiProviderFactory));
        let types = registry.list_provider_types();

        // Should serialize without error
        let json = serde_json::to_string(&types[0]).unwrap();
        assert!(json.contains("free_tier_notes"));
        assert!(json.contains("Flash models"));

        // Should deserialize back
        let deserialized: ProviderTypeInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.free_tier_notes, types[0].free_tier_notes);
    }

    #[test]
    fn test_provider_type_info_deserialize_without_notes() {
        // Backward compat: old JSON without free_tier_notes should deserialize
        let json = r#"{
            "provider_type": "test",
            "display_name": "Test",
            "category": "local",
            "description": "Test provider",
            "setup_parameters": [],
            "default_free_tier": {"kind": "none"},
            "free_tier_short_text": "",
            "free_tier_long_text": "No free tier."
        }"#;
        let info: ProviderTypeInfo = serde_json::from_str(json).unwrap();
        assert_eq!(info.free_tier_notes, None);
    }
}
