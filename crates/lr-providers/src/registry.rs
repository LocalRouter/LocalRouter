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
use super::model_settings::{self, ManagedModel};
use super::{Capability, ModelInfo, ModelListing, ModelProvider, PricingInfo, ProviderHealth};
use lr_config::{FreeTierKind, ModelCacheConfig, ModelPricingOverride, ProviderModelSettings};
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

/// How long a failed model listing is kept before it is retried, at most.
const FAILED_LISTING_TTL_SECS: u64 = 60;

/// Cached model list for a single provider
#[derive(Clone)]
struct ModelCache {
    /// What the provider listed (or the fallback used when listing failed),
    /// before the instance's model settings are applied
    listing: ModelListing,
    /// Why listing failed, when it did
    discovery_error: Option<String>,
    /// Every model with the instance's settings applied
    managed: Vec<ManagedModel>,
    /// The offered models (`managed` filtered by visibility)
    models: Vec<ModelInfo>,
    /// When this cache was populated
    fetched_at: DateTime<Utc>,
    /// Provider-specific TTL (from config or default)
    ttl_seconds: u64,
}

impl ModelCache {
    #[cfg(test)]
    fn new(models: Vec<ModelInfo>, ttl_seconds: u64) -> Self {
        Self::from_listing(
            ModelListing {
                models,
                new_models_hidden: false,
            },
            None,
            &ProviderModelSettings::default(),
            "",
            ttl_seconds,
        )
    }

    fn from_listing(
        listing: ModelListing,
        discovery_error: Option<String>,
        settings: &ProviderModelSettings,
        instance_name: &str,
        ttl_seconds: u64,
    ) -> Self {
        let mut cache = Self {
            listing,
            discovery_error,
            managed: Vec::new(),
            models: Vec::new(),
            fetched_at: Utc::now(),
            ttl_seconds,
        };
        cache.apply_settings(settings, instance_name);
        cache
    }

    /// Recompute the managed and offered lists from the cached listing.
    fn apply_settings(&mut self, settings: &ProviderModelSettings, instance_name: &str) {
        self.managed = model_settings::apply(
            &self.listing.models,
            settings,
            instance_name,
            self.listing.new_models_hidden,
        );
        self.models = model_settings::offered(&self.managed);
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

    /// Manual models, per-model visibility and overrides
    pub model_settings: ProviderModelSettings,
}

/// A provider instance's models with their settings applied, for the UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagedModelList {
    /// Every model: discovered, manual, and settings for models gone
    pub models: Vec<ManagedModel>,
    /// Why the last listing failed, when it did
    pub discovery_error: Option<String>,
    /// Whether discovered models without an entry are shown
    pub show_new_models: bool,
    /// The provider marked its listing as naming more than is installed
    pub new_models_hidden_by_provider: bool,
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
    /// Official documentation / homepage for this provider
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub docs_url: Option<String>,
    /// Page where the user creates or copies their API key
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key_url: Option<String>,
    /// Whether the type is offered when adding a provider. Unlisted types
    /// still load existing configured instances.
    #[serde(default = "default_listed")]
    pub listed: bool,
}

fn default_listed() -> bool {
    true
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

    /// Local and Local Embedded providers talk to the one engine on this
    /// machine, so each type is added once. Returns the existing instance's
    /// name when `provider_type` is such a type and already added.
    pub fn existing_single_instance(&self, provider_type: &str) -> Option<String> {
        let category = self.factories.read().get(provider_type)?.category();
        if !matches!(
            category,
            crate::factory::ProviderCategory::Local | crate::factory::ProviderCategory::Embedded
        ) {
            return None;
        }
        self.list_providers()
            .into_iter()
            .find(|i| i.provider_type == provider_type)
            .map(|i| i.instance_name)
    }

    /// List all available provider types with setup parameters
    ///
    /// Used by: UI for showing available provider types
    pub fn list_provider_types(&self) -> Vec<ProviderTypeInfo> {
        let mut types: Vec<(u8, ProviderTypeInfo)> = self
            .factories
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
                let info = ProviderTypeInfo {
                    provider_type: factory.provider_type().to_string(),
                    display_name: factory.display_name().to_string(),
                    category: factory.category(),
                    description: factory.description().to_string(),
                    setup_parameters: factory.setup_parameters(),
                    default_free_tier: free_tier,
                    free_tier_short_text: short_text,
                    free_tier_long_text: long_text,
                    free_tier_notes: notes,
                    docs_url: factory.docs_url().map(|s| s.to_string()),
                    api_key_url: factory.api_key_url().map(|s| s.to_string()),
                    listed: factory.listed(),
                };
                (factory.list_priority(), info)
            })
            .collect();
        // Stable order for the UI: category order, then the factory's
        // priority, then display name.
        types.sort_by(|(pa, a), (pb, b)| {
            a.category
                .sort_rank()
                .cmp(&b.category.sort_rank())
                .then_with(|| pa.cmp(pb))
                .then_with(|| {
                    a.display_name
                        .to_lowercase()
                        .cmp(&b.display_name.to_lowercase())
                })
        });
        types.into_iter().map(|(_, info)| info).collect()
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
            .register_provider(&instance_name, provider.clone())
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
            model_settings: ProviderModelSettings::default(),
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
        let (enabled, created_at, model_settings) = {
            let instances = self.instances.read();
            let instance = instances.get(&instance_name).ok_or_else(|| {
                AppError::Config(format!("Provider instance '{}' not found", instance_name))
            })?;
            (
                instance.enabled,
                instance.created_at,
                instance.model_settings.clone(),
            )
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
            .register_provider(&instance_name, provider.clone())
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
            model_settings,
        };

        self.instances
            .write()
            .insert(instance_name.clone(), instance);
        // A new base URL, key or discovery setting lists different models.
        self.model_cache.write().remove(&instance_name);

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
        let Some(removed) = self.instances.write().remove(instance_name) else {
            return Ok(());
        };
        self.model_cache.write().remove(instance_name);
        info!("Removed provider instance: {}", instance_name);
        // Drop the health manager's reference too, so a provider that owns
        // resources (child processes, loaded models) is actually released.
        let health_manager = self.health_manager.clone();
        let name = instance_name.to_string();
        let provider = removed.provider;
        let unregister = async move {
            health_manager
                .unregister_provider_if_same(&name, &provider)
                .await
        };
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => {
                handle.spawn(unregister);
            }
            Err(_) => futures::executor::block_on(unregister),
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

    /// Get models from models.dev catalog for a provider type
    fn get_models_from_catalog(&self, provider_type: &str) -> Vec<ModelInfo> {
        lr_catalog::models()
            .iter()
            .filter(|m| m.id.starts_with(&format!("{}/", provider_type)))
            .map(|m| {
                let mut capabilities = Vec::new();
                if m.capabilities.embedding {
                    capabilities.push(crate::Capability::Embedding);
                } else if m.modality == lr_catalog::Modality::Image || m.capabilities.image_output {
                    // Image generation model — don't add Chat
                    capabilities.push(crate::Capability::ImageGeneration);
                } else {
                    // Chat model (or unknown)
                    capabilities.push(crate::Capability::Chat);
                    if m.modality == lr_catalog::Modality::Multimodal {
                        capabilities.push(crate::Capability::Vision);
                    }
                }
                if m.capabilities.audio_input {
                    capabilities.push(crate::Capability::Audio);
                }
                if m.capabilities.audio_output {
                    capabilities.push(crate::Capability::TextToSpeech);
                }

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

    /// What to use as the listing when the provider's listing failed:
    /// the models.dev catalog (when enabled and it knows the provider), else
    /// the previous listing, else nothing.
    fn fallback_listing(&self, instance_name: &str, catalog_id: &str) -> ModelListing {
        if self.cache_config.read().use_catalog_fallback {
            let catalog_models = self.get_models_from_catalog(catalog_id);
            if !catalog_models.is_empty() {
                info!(
                    "Using models.dev catalog fallback for '{}' ({} models)",
                    instance_name,
                    catalog_models.len()
                );
                return ModelListing {
                    models: catalog_models,
                    new_models_hidden: false,
                };
            }
        }
        if let Some(cached) = self.get_cached_models_for_provider(instance_name) {
            warn!(
                "Using stale model list for '{}' (expired {} seconds ago)",
                instance_name,
                -cached.expires_in()
            );
            return cached.listing;
        }
        ModelListing::default()
    }

    /// List models from a provider with caching
    ///
    /// Returns the offered models: the provider's listing (or its fallback
    /// when listing fails) plus manual models, with the instance's model
    /// settings applied. Errs only when listing failed and nothing is left
    /// to offer.
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
                return Self::offered_or_error(instance_name, &cached);
            }
        }

        // 2. Try fetching from provider API
        let provider_instance = self
            .instances
            .read()
            .get(instance_name)
            .cloned()
            .ok_or_else(|| AppError::Config(format!("Provider '{}' not found", instance_name)))?;

        let ttl = self.get_provider_ttl(instance_name);
        let (listing, discovery_error, ttl) =
            match provider_instance.provider.list_model_listing().await {
                Ok(listing) => (listing, None, ttl),
                Err(e) => {
                    warn!("Failed to fetch models from '{}': {}", instance_name, e);
                    // 3. Fallback - use catalog_provider_id if available, else provider_type
                    let catalog_id = provider_instance
                        .catalog_provider_id
                        .as_deref()
                        .unwrap_or(&provider_instance.provider_type);
                    (
                        self.fallback_listing(instance_name, catalog_id),
                        Some(e.to_string()),
                        ttl.min(FAILED_LISTING_TTL_SECS),
                    )
                }
            };

        // Apply the settings current now, not the ones read before the
        // fetch: they may have changed while it ran. Read them under the
        // cache lock so a concurrent `set_model_settings` either sees this
        // entry or is seen by it.
        let mut caches = self.model_cache.write();
        let Some(settings) = self.get_model_settings(instance_name) else {
            return Err(AppError::Config(format!(
                "Provider '{}' not found",
                instance_name
            )));
        };
        let cache =
            ModelCache::from_listing(listing, discovery_error, &settings, instance_name, ttl);
        let result = Self::offered_or_error(instance_name, &cache);
        caches.insert(instance_name.to_string(), cache);
        drop(caches);
        debug!(
            "Updated model cache for '{}' (TTL: {}s)",
            instance_name, ttl
        );
        result
    }

    fn offered_or_error(instance_name: &str, cache: &ModelCache) -> AppResult<Vec<ModelInfo>> {
        match &cache.discovery_error {
            Some(e) if cache.models.is_empty() => Err(AppError::Provider(format!(
                "No models available for '{}': {}",
                instance_name, e
            ))),
            _ => Ok(cache.models.clone()),
        }
    }

    // ===== PER-MODEL SETTINGS =====

    /// The model settings of a provider instance.
    pub fn get_model_settings(&self, instance_name: &str) -> Option<ProviderModelSettings> {
        self.instances
            .read()
            .get(instance_name)
            .map(|inst| inst.model_settings.clone())
    }

    /// Replace a provider instance's model settings. The cached listing is
    /// re-filtered right away; nothing is fetched.
    pub fn set_model_settings(
        &self,
        instance_name: &str,
        settings: ProviderModelSettings,
    ) -> AppResult<()> {
        {
            let mut instances = self.instances.write();
            let instance = instances.get_mut(instance_name).ok_or_else(|| {
                AppError::Config(format!("Provider instance '{}' not found", instance_name))
            })?;
            instance.model_settings = settings.clone();
        }
        let offered = {
            let mut cache = self.model_cache.write();
            match cache.get_mut(instance_name) {
                Some(cached) => {
                    cached.apply_settings(&settings, instance_name);
                    Some(cached.models.clone())
                }
                None => None,
            }
        };
        if let Some(offered) = offered {
            let mut all = self.cached_models.write();
            all.retain(|m| m.provider != instance_name);
            all.extend(offered);
        }
        Ok(())
    }

    /// Every model of a provider instance with its settings applied, from
    /// the cache (call `list_provider_models_cached` first to fill it).
    pub fn managed_models(&self, instance_name: &str) -> Option<ManagedModelList> {
        let settings = self.get_model_settings(instance_name)?;
        let cache = self.model_cache.read();
        let cached = cache.get(instance_name);
        let new_models_hidden = cached.is_some_and(|c| c.listing.new_models_hidden);
        let models = match cached {
            Some(c) => c.managed.clone(),
            None => model_settings::apply(&[], &settings, instance_name, false),
        };
        Some(ManagedModelList {
            models,
            discovery_error: cached.and_then(|c| c.discovery_error.clone()),
            show_new_models: model_settings::shows_new_models(&settings, new_models_hidden),
            new_models_hidden_by_provider: new_models_hidden,
        })
    }

    /// Whether `model_id` is a model this instance knows (listed, added by
    /// hand, or configured) and the user turned off. Unknown ids are not
    /// disabled: they pass through to the provider as before.
    pub fn is_model_disabled(&self, instance_name: &str, model_id: &str) -> bool {
        let cache = self.model_cache.read();
        if let Some(cached) = cache.get(instance_name) {
            let find = |exact: bool| {
                cached.managed.iter().find(|m| {
                    if exact {
                        m.info.id == model_id
                    } else {
                        m.info.id.eq_ignore_ascii_case(model_id)
                    }
                })
            };
            if let Some(m) = find(true).or_else(|| find(false)) {
                return !m.enabled;
            }
            return false;
        }
        drop(cache);
        // Not listed yet: only an explicit setting can say.
        self.instances
            .read()
            .get(instance_name)
            .and_then(|inst| inst.model_settings.entry(model_id).and_then(|e| e.enabled))
            .is_some_and(|enabled| !enabled)
    }

    /// The per-instance price override of a model, if set.
    pub fn pricing_override(
        &self,
        instance_name: &str,
        model_id: &str,
    ) -> Option<ModelPricingOverride> {
        self.instances
            .read()
            .get(instance_name)?
            .model_settings
            .entry(model_id)?
            .pricing
            .clone()
    }

    /// Pricing of a model on a provider instance: the instance's price
    /// override when set, else what the provider reports, else free.
    pub async fn get_pricing(&self, instance_name: &str, model_id: &str) -> PricingInfo {
        if let Some(p) = self.pricing_override(instance_name, model_id) {
            return PricingInfo {
                input_cost_per_1k: p.input_per_million / 1000.0,
                output_cost_per_1k: p.output_per_million / 1000.0,
                ..PricingInfo::free()
            };
        }
        let provider = self.get_provider_unchecked(instance_name);
        match provider {
            Some(p) => p
                .get_pricing(model_id)
                .await
                .unwrap_or_else(|_| PricingInfo::free()),
            None => PricingInfo::free(),
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

    /// Check if a cached model has a specific capability (no I/O).
    ///
    /// Returns `None` if the provider or model is not in the cache
    /// (e.g., cache not yet populated after startup).
    pub fn model_has_capability(
        &self,
        instance_name: &str,
        model_id: &str,
        capability: &Capability,
    ) -> Option<bool> {
        let cache = self.model_cache.read();
        let cached = cache.get(instance_name)?;
        let model = cached.models.iter().find(|m| m.id == model_id)?;
        Some(model.capabilities.contains(capability))
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

    /// Look up a model's capabilities from cached model list, falling back to catalog.
    /// Returns None if model not found anywhere (treat as unknown = try everything).
    pub fn get_model_capabilities(
        &self,
        instance_name: &str,
        model_id: &str,
    ) -> Option<Vec<crate::Capability>> {
        // Try cached model list first
        let cache = self.model_cache.read();
        if let Some(mc) = cache.get(instance_name) {
            if let Some(model) = mc.models.iter().find(|m| m.id == model_id) {
                return Some(model.capabilities.clone());
            }
        }
        drop(cache);

        // Fall back to catalog
        let provider_type = self.get_provider_type_for_instance(instance_name)?;
        let catalog_model = lr_catalog::find_model(&provider_type, model_id)
            .or_else(|| lr_catalog::find_model_by_name(model_id))?;

        let mut caps = Vec::new();
        if catalog_model.capabilities.embedding {
            caps.push(crate::Capability::Embedding);
        } else if catalog_model.modality == lr_catalog::Modality::Image
            || catalog_model.capabilities.image_output
        {
            // Image generation model — don't add Chat
            caps.push(crate::Capability::ImageGeneration);
        } else {
            caps.push(crate::Capability::Chat);
            if catalog_model.modality == lr_catalog::Modality::Multimodal {
                caps.push(crate::Capability::Vision);
            }
        }
        if catalog_model.capabilities.audio_input {
            caps.push(crate::Capability::Audio);
        }
        if catalog_model.capabilities.audio_output {
            caps.push(crate::Capability::TextToSpeech);
        }
        Some(caps)
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

    #[tokio::test]
    async fn local_types_are_added_once() {
        let registry = ProviderRegistry::new();
        registry.register_factory(Arc::new(crate::factory::OllamaProviderFactory));
        registry.register_factory(Arc::new(crate::factory::OpenAICompatibleProviderFactory));
        assert_eq!(registry.existing_single_instance("ollama"), None);
        registry
            .create_provider("Ollama".into(), "ollama".into(), HashMap::new())
            .await
            .unwrap();
        assert_eq!(
            registry.existing_single_instance("ollama").as_deref(),
            Some("Ollama")
        );
        // Generic types may be added many times.
        let mut cfg = HashMap::new();
        cfg.insert(
            "base_url".to_string(),
            "http://localhost:1234/v1".to_string(),
        );
        registry
            .create_provider("Custom".into(), "openai_compatible".into(), cfg)
            .await
            .unwrap();
        assert_eq!(registry.existing_single_instance("openai_compatible"), None);
        assert_eq!(registry.existing_single_instance("unknown"), None);
    }

    #[tokio::test]
    async fn llama_cpp_leads_the_local_embedded_providers() {
        let dir = tempfile::tempdir().unwrap();
        let supervisor = lr_engines::Supervisor::new(dir.path());
        let library = Arc::new(lr_local_models::Library::open(dir.path().join("models")));
        let registry = ProviderRegistry::new();
        registry.register_factory(Arc::new(
            crate::embedded::DeciderEmbeddedProviderFactory::new(supervisor.clone()),
        ));
        registry.register_factory(Arc::new(crate::embedded::KevEmbeddedProviderFactory::new(
            supervisor.clone(),
        )));
        registry.register_factory(Arc::new(
            crate::embedded::LlamaCppEmbeddedProviderFactory::new(library, supervisor.clone()),
        ));
        let types: Vec<String> = registry
            .list_provider_types()
            .into_iter()
            .map(|t| t.provider_type)
            .collect();
        assert_eq!(types, vec!["llamacpp_embedded", "decider", "kev"]);
    }

    #[tokio::test]
    async fn ollaya_and_the_dedicated_decision_engines_are_all_offered() {
        let dir = tempfile::tempdir().unwrap();
        let supervisor = lr_engines::Supervisor::new(dir.path());
        let registry = ProviderRegistry::new();
        registry.register_factory(Arc::new(
            crate::embedded::OllayaEmbeddedProviderFactory::new(supervisor.clone()),
        ));
        registry.register_factory(Arc::new(crate::embedded::LayaEmbeddedProviderFactory::new(
            supervisor.clone(),
        )));
        registry.register_factory(Arc::new(crate::embedded::KevEmbeddedProviderFactory::new(
            supervisor.clone(),
        )));
        let listed: Vec<(String, bool)> = registry
            .list_provider_types()
            .into_iter()
            .map(|t| (t.provider_type, t.listed))
            .collect();
        // The dedicated engines run their models on the GPU where Ollaya
        // cannot, so they stay offered next to it.
        assert!(listed.contains(&("ollaya".to_string(), true)));
        assert!(listed.contains(&("laya".to_string(), true)));
        assert!(listed.contains(&("kev".to_string(), true)));
    }

    #[tokio::test]
    async fn provider_types_are_ordered_with_embedded_first_and_legacy_hidden() {
        let registry = ProviderRegistry::new();
        registry.register_factory(Arc::new(crate::factory::OpenAICompatibleProviderFactory));
        registry.register_factory(Arc::new(crate::factory::LlamaCppProviderFactory));
        registry.register_factory(Arc::new(crate::factory::OllamaProviderFactory));
        for f in crate::factory::SystemOneProviderFactory::all() {
            registry.register_factory(Arc::new(f));
        }
        let types = registry.list_provider_types();
        let ranks: Vec<u8> = types.iter().map(|t| t.category.sort_rank()).collect();
        let mut sorted = ranks.clone();
        sorted.sort();
        assert_eq!(ranks, sorted, "types must be grouped by category order");
        let legacy = types
            .iter()
            .find(|t| t.provider_type == "llamacpp")
            .unwrap();
        assert!(!legacy.listed);
        assert!(types
            .iter()
            .filter(|t| t.provider_type != "llamacpp")
            .all(|t| t.listed));
        // Deterministic across calls.
        let again: Vec<String> = registry
            .list_provider_types()
            .into_iter()
            .map(|t| t.provider_type)
            .collect();
        assert_eq!(
            again,
            types
                .into_iter()
                .map(|t| t.provider_type)
                .collect::<Vec<_>>()
        );
    }

    #[tokio::test]
    async fn removing_a_provider_releases_it() {
        let registry = ProviderRegistry::new();
        registry.register_factory(Arc::new(crate::factory::OllamaProviderFactory));
        registry
            .create_provider("o".into(), "ollama".into(), HashMap::new())
            .await
            .unwrap();
        let weak = Arc::downgrade(&registry.get_provider_unchecked("o").unwrap());
        registry.remove_provider("o").unwrap();
        // The deferred unregister runs on the runtime.
        for _ in 0..50 {
            if weak.upgrade().is_none() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert!(
            weak.upgrade().is_none(),
            "removed provider is still referenced"
        );
    }

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

    // ----- Per-model settings -----

    mod model_settings_tests {
        use super::*;
        use crate::{ModelListing, PricingInfo, ProviderHealth};
        use async_trait::async_trait;
        use lr_config::{ModelEntry, ModelPricingOverride};
        use std::sync::atomic::{AtomicUsize, Ordering};

        /// What the mock provider's listing returns.
        #[derive(Clone)]
        enum Listing {
            Models(Vec<&'static str>, bool),
            Fails,
        }

        struct MockProvider {
            listing: Arc<parking_lot::Mutex<Listing>>,
            calls: Arc<AtomicUsize>,
        }

        #[async_trait]
        impl ModelProvider for MockProvider {
            fn name(&self) -> &str {
                "mock"
            }
            async fn health_check(&self) -> ProviderHealth {
                unimplemented!()
            }
            async fn list_models(&self) -> AppResult<Vec<ModelInfo>> {
                Ok(self.list_model_listing().await?.models)
            }
            async fn list_model_listing(&self) -> AppResult<ModelListing> {
                self.calls.fetch_add(1, Ordering::SeqCst);
                match self.listing.lock().clone() {
                    Listing::Models(ids, hidden) => Ok(ModelListing {
                        models: ids
                            .into_iter()
                            .map(|id| ModelInfo {
                                id: id.into(),
                                name: id.into(),
                                provider: "mock".into(),
                                parameter_count: None,
                                context_window: 1000,
                                supports_streaming: true,
                                capabilities: vec![Capability::Chat],
                                detailed_capabilities: None,
                            })
                            .collect(),
                        new_models_hidden: hidden,
                    }),
                    Listing::Fails => Err(AppError::Provider("connection refused".into())),
                }
            }
            async fn get_pricing(&self, _model: &str) -> AppResult<PricingInfo> {
                Ok(PricingInfo {
                    input_cost_per_1k: 0.5,
                    ..PricingInfo::free()
                })
            }
            async fn complete(
                &self,
                _request: crate::CompletionRequest,
            ) -> AppResult<crate::CompletionResponse> {
                unimplemented!()
            }
            async fn stream_complete(
                &self,
                _request: crate::CompletionRequest,
            ) -> AppResult<
                std::pin::Pin<
                    Box<dyn futures::Stream<Item = AppResult<crate::CompletionChunk>> + Send>,
                >,
            > {
                unimplemented!()
            }
        }

        struct MockFactory {
            listing: Arc<parking_lot::Mutex<Listing>>,
            calls: Arc<AtomicUsize>,
        }

        impl ProviderFactory for MockFactory {
            fn provider_type(&self) -> &str {
                "mock"
            }
            fn display_name(&self) -> &str {
                "Mock"
            }
            fn category(&self) -> crate::factory::ProviderCategory {
                crate::factory::ProviderCategory::Generic
            }
            fn description(&self) -> &str {
                "mock"
            }
            fn setup_parameters(&self) -> Vec<SetupParameter> {
                vec![]
            }
            fn create(
                &self,
                _instance_name: String,
                _config: HashMap<String, String>,
            ) -> AppResult<Arc<dyn ModelProvider>> {
                Ok(Arc::new(MockProvider {
                    listing: self.listing.clone(),
                    calls: self.calls.clone(),
                }))
            }
            fn validate_config(&self, _config: &HashMap<String, String>) -> AppResult<()> {
                Ok(())
            }
            fn catalog_provider_id(&self) -> Option<&str> {
                None
            }
        }

        struct Fixture {
            registry: ProviderRegistry,
            listing: Arc<parking_lot::Mutex<Listing>>,
            calls: Arc<AtomicUsize>,
        }

        async fn fixture(listing: Listing) -> Fixture {
            let listing = Arc::new(parking_lot::Mutex::new(listing));
            let calls = Arc::new(AtomicUsize::new(0));
            let registry = ProviderRegistry::new();
            registry.register_factory(Arc::new(MockFactory {
                listing: listing.clone(),
                calls: calls.clone(),
            }));
            registry
                .create_provider("p".into(), "mock".into(), HashMap::new())
                .await
                .unwrap();
            Fixture {
                registry,
                listing,
                calls,
            }
        }

        fn settings(entries: Vec<ModelEntry>) -> ProviderModelSettings {
            ProviderModelSettings {
                show_new_models: None,
                entries,
            }
        }

        fn entry(id: &str) -> ModelEntry {
            ModelEntry {
                id: id.into(),
                ..Default::default()
            }
        }

        fn ids(models: &[ModelInfo]) -> Vec<String> {
            models.iter().map(|m| m.id.clone()).collect()
        }

        #[tokio::test]
        async fn hiding_a_model_applies_without_refetching() {
            let f = fixture(Listing::Models(vec!["a", "b"], false)).await;
            let models = f.registry.list_provider_models_cached("p").await.unwrap();
            assert_eq!(ids(&models), vec!["a", "b"]);
            assert!(models.iter().all(|m| m.provider == "p"));

            f.registry
                .set_model_settings(
                    "p",
                    settings(vec![ModelEntry {
                        enabled: Some(false),
                        ..entry("b")
                    }]),
                )
                .unwrap();
            assert_eq!(
                ids(&f.registry.list_provider_models_cached("p").await.unwrap()),
                vec!["a"]
            );
            assert_eq!(ids(&f.registry.get_all_cached_models_instant()), vec!["a"]);
            assert_eq!(f.calls.load(Ordering::SeqCst), 1, "no second fetch");

            let managed = f.registry.managed_models("p").unwrap();
            assert_eq!(managed.models.len(), 2);
            assert!(!managed.models[1].enabled);
            assert!(managed.show_new_models);
            assert_eq!(managed.discovery_error, None);
        }

        #[tokio::test]
        async fn manual_models_survive_a_failed_listing() {
            let f = fixture(Listing::Fails).await;
            // Nothing to offer: the failure is an error, as before.
            let err = f
                .registry
                .list_provider_models_cached("p")
                .await
                .unwrap_err();
            assert!(err.to_string().contains("connection refused"), "{err}");
            assert!(f
                .registry
                .managed_models("p")
                .unwrap()
                .discovery_error
                .unwrap()
                .contains("connection refused"));

            f.registry
                .set_model_settings(
                    "p",
                    settings(vec![ModelEntry {
                        manual: true,
                        context_window: Some(32_000),
                        ..entry("my-model")
                    }]),
                )
                .unwrap();
            let models = f.registry.list_provider_models_cached("p").await.unwrap();
            assert_eq!(ids(&models), vec!["my-model"]);
            assert_eq!(models[0].context_window, 32_000);
            let managed = f.registry.managed_models("p").unwrap();
            assert!(managed.discovery_error.is_some());
        }

        #[tokio::test]
        async fn failed_listing_keeps_the_previous_models() {
            let f = fixture(Listing::Models(vec!["a"], false)).await;
            f.registry.list_provider_models_cached("p").await.unwrap();
            *f.listing.lock() = Listing::Fails;
            // Expire the cache.
            f.registry
                .model_cache
                .write()
                .get_mut("p")
                .unwrap()
                .fetched_at = Utc::now() - Duration::seconds(100_000);
            let models = f.registry.list_provider_models_cached("p").await.unwrap();
            assert_eq!(ids(&models), vec!["a"]);
            assert!(f
                .registry
                .managed_models("p")
                .unwrap()
                .discovery_error
                .is_some());
        }

        #[tokio::test]
        async fn listing_marked_untrusted_hides_new_models_by_default() {
            let f = fixture(Listing::Models(vec!["a", "b"], true)).await;
            assert!(f
                .registry
                .list_provider_models_cached("p")
                .await
                .unwrap()
                .is_empty());
            let managed = f.registry.managed_models("p").unwrap();
            assert!(!managed.show_new_models);
            assert!(managed.new_models_hidden_by_provider);

            f.registry
                .set_model_settings(
                    "p",
                    settings(vec![ModelEntry {
                        enabled: Some(true),
                        ..entry("b")
                    }]),
                )
                .unwrap();
            assert_eq!(
                ids(&f.registry.list_provider_models_cached("p").await.unwrap()),
                vec!["b"]
            );
            f.registry
                .set_model_settings(
                    "p",
                    ProviderModelSettings {
                        show_new_models: Some(true),
                        entries: vec![],
                    },
                )
                .unwrap();
            assert_eq!(
                ids(&f.registry.list_provider_models_cached("p").await.unwrap()),
                vec!["a", "b"]
            );
        }

        #[tokio::test]
        async fn only_known_turned_off_models_are_disabled() {
            let f = fixture(Listing::Models(vec!["Model-A", "b"], false)).await;
            let hidden = settings(vec![ModelEntry {
                enabled: Some(false),
                ..entry("Model-A")
            }]);
            f.registry.set_model_settings("p", hidden).unwrap();
            // Before the list is fetched, the setting alone decides.
            assert!(f.registry.is_model_disabled("p", "Model-A"));
            assert!(!f.registry.is_model_disabled("p", "b"));
            f.registry.list_provider_models_cached("p").await.unwrap();
            assert!(f.registry.is_model_disabled("p", "Model-A"));
            assert!(f.registry.is_model_disabled("p", "model-a"));
            assert!(!f.registry.is_model_disabled("p", "b"));
            assert!(!f.registry.is_model_disabled("p", "unknown"));
            assert!(!f.registry.is_model_disabled("other", "Model-A"));
        }

        #[tokio::test]
        async fn price_override_comes_first() {
            let f = fixture(Listing::Models(vec!["a"], false)).await;
            assert_eq!(
                f.registry.get_pricing("p", "a").await.input_cost_per_1k,
                0.5
            );
            f.registry
                .set_model_settings(
                    "p",
                    settings(vec![ModelEntry {
                        pricing: Some(ModelPricingOverride {
                            input_per_million: 3.0,
                            output_per_million: 15.0,
                        }),
                        ..entry("a")
                    }]),
                )
                .unwrap();
            let p = f.registry.get_pricing("p", "a").await;
            assert!((p.input_cost_per_1k - 0.003).abs() < 1e-12);
            assert!((p.output_cost_per_1k - 0.015).abs() < 1e-12);
            assert_eq!(
                f.registry
                    .pricing_override("p", "a")
                    .unwrap()
                    .input_per_million,
                3.0
            );
            // Other models and unknown providers: the provider's price, or free.
            assert_eq!(
                f.registry.get_pricing("p", "b").await.input_cost_per_1k,
                0.5
            );
            assert_eq!(
                f.registry.get_pricing("nope", "a").await.input_cost_per_1k,
                0.0
            );
        }

        #[tokio::test]
        async fn updating_a_provider_keeps_its_settings_and_refetches() {
            let f = fixture(Listing::Models(vec!["a"], false)).await;
            let s = settings(vec![ModelEntry {
                manual: true,
                ..entry("m")
            }]);
            f.registry.set_model_settings("p", s.clone()).unwrap();
            f.registry.list_provider_models_cached("p").await.unwrap();
            f.registry
                .update_provider("p".into(), "mock".into(), HashMap::new())
                .await
                .unwrap();
            assert_eq!(f.registry.get_model_settings("p"), Some(s));
            f.registry.list_provider_models_cached("p").await.unwrap();
            assert_eq!(f.calls.load(Ordering::SeqCst), 2);
            f.registry.remove_provider("p").unwrap();
            assert!(f.registry.managed_models("p").is_none());
            assert!(f.registry.model_cache.read().get("p").is_none());
        }
    }
}
