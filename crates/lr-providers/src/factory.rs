//! Provider factory system for dynamic provider instantiation
//!
//! This module provides the factory pattern for creating provider instances
//! with validated configuration parameters.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use super::{
    anthropic::AnthropicProvider, cerebras::CerebrasProvider, cohere::CohereProvider,
    deepinfra::DeepInfraProvider, gemini::GeminiProvider, gpt4all::GPT4AllProvider,
    groq::GroqProvider, jan::JanProvider, llamacpp::LlamaCppProvider, lmstudio::LMStudioProvider,
    localai::LocalAIProvider, mistral::MistralProvider, ollama::OllamaProvider,
    openai::OpenAIProvider, openai_compatible::OpenAICompatibleProvider,
    openrouter::OpenRouterProvider, perplexity::PerplexityProvider, togetherai::TogetherAIProvider,
    xai::XAIProvider, ModelProvider,
};
use lr_config::FreeTierKind;
use lr_types::{AppError, AppResult};

/// Provider category for UI grouping
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderCategory {
    /// Generic/custom OpenAI-compatible providers
    Generic,
    /// Local providers running on user's machine
    Local,
    /// Subscription-based providers using OAuth
    Subscription,
    /// First-party cloud providers (model creators)
    FirstParty,
    /// Third-party hosting platforms
    ThirdParty,
}

/// Where a provider gets its model list from
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelListSource {
    /// Use provider's API, fall back to catalog if API fails/empty
    ApiWithCatalogFallback,
    /// Use catalog as primary source (no API available)
    CatalogOnly,
    /// Use provider's API only, no catalog fallback
    ApiOnly,
}

/// Factory for creating provider instances
#[async_trait]
pub trait ProviderFactory: Send + Sync {
    /// Provider type identifier (e.g., "ollama", "openai", "anthropic")
    fn provider_type(&self) -> &str;

    /// Human-readable display name (e.g., "OpenAI", "Google Gemini")
    fn display_name(&self) -> &str;

    /// Provider category for UI grouping
    fn category(&self) -> ProviderCategory;

    /// Human-readable description of the provider
    fn description(&self) -> &str;

    /// List of setup parameters required/optional for this provider
    fn setup_parameters(&self) -> Vec<SetupParameter>;

    /// Create a provider instance from configuration
    ///
    /// # Arguments
    /// * `instance_name` - User-defined name for this provider instance
    /// * `config` - Configuration parameters (validated before this is called)
    ///
    /// # Returns
    /// Arc-wrapped provider implementation ready to use
    fn create(
        &self,
        instance_name: String,
        config: HashMap<String, String>,
    ) -> AppResult<Arc<dyn ModelProvider>>;

    /// Validate configuration before creation
    ///
    /// Checks that all required parameters are present and valid
    fn validate_config(&self, config: &HashMap<String, String>) -> AppResult<()>;

    /// models.dev provider ID for catalog matching
    ///
    /// Returns the provider ID used in models.dev (e.g., "google" for Gemini).
    /// Returns None if this provider has no catalog mapping (local providers).
    ///
    /// Default implementation returns the same as provider_type().
    fn catalog_provider_id(&self) -> Option<&str> {
        Some(self.provider_type())
    }

    /// How this provider gets its model list
    ///
    /// Default: Use provider's API, fall back to catalog if API fails/empty
    fn model_list_source(&self) -> ModelListSource {
        ModelListSource::ApiWithCatalogFallback
    }

    /// Default free tier configuration for this provider type.
    ///
    /// Each provider declares its default free tier here. Users can override
    /// per provider instance via ProviderConfig.free_tier.
    fn default_free_tier(&self) -> FreeTierKind {
        FreeTierKind::None
    }

    /// Optional provider-specific notes about free tier caveats.
    /// Appended to the auto-generated long description text.
    fn free_tier_notes(&self) -> Option<&str> {
        None
    }
}

/// Trait for providers that can be automatically discovered on the local system
///
/// This is implemented by local providers (Ollama, LM Studio) that run on
/// the user's machine and can be detected by checking their default endpoints.
#[async_trait]
pub trait DiscoverableProvider: ProviderFactory {
    /// Check if this provider is available on the local system
    ///
    /// Returns true if the provider's service is running and responding
    async fn is_available(&self) -> bool;

    /// Get the default base URL for this provider
    fn default_base_url(&self) -> &str;

    /// Get the default display name for discovered instances
    fn default_instance_name(&self) -> &str {
        self.display_name()
    }
}

/// A parameter required for provider setup
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SetupParameter {
    /// Parameter key (e.g., "api_key", "base_url")
    pub key: String,

    /// Parameter type
    pub param_type: ParameterType,

    /// Whether this parameter is required
    pub required: bool,

    /// Human-readable description
    pub description: String,

    /// Default value if not provided
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value: Option<String>,

    /// Whether to mask in UI (for secrets like API keys)
    pub sensitive: bool,
}

impl SetupParameter {
    /// Create a new required parameter
    pub fn required(
        key: impl Into<String>,
        param_type: ParameterType,
        description: impl Into<String>,
        sensitive: bool,
    ) -> Self {
        Self {
            key: key.into(),
            param_type,
            required: true,
            description: description.into(),
            default_value: None,
            sensitive,
        }
    }

    /// Create a new optional parameter
    pub fn optional(
        key: impl Into<String>,
        param_type: ParameterType,
        description: impl Into<String>,
        default_value: Option<impl Into<String>>,
        sensitive: bool,
    ) -> Self {
        Self {
            key: key.into(),
            param_type,
            required: false,
            description: description.into(),
            default_value: default_value.map(|v| v.into()),
            sensitive,
        }
    }
}

/// Type of parameter
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ParameterType {
    /// API key or secret
    ApiKey,
    /// Base URL/endpoint
    BaseUrl,
    /// Organization ID
    Organization,
    /// Model name
    Model,
    /// Generic string parameter
    String,
    /// Numeric parameter
    Number,
    /// Boolean parameter
    Boolean,
    /// OAuth authentication (triggers OAuth flow in UI)
    #[serde(rename = "oauth")]
    OAuth,
}

// ==================== FACTORY IMPLEMENTATIONS ====================

/// Factory for Ollama providers
pub struct OllamaProviderFactory;

impl ProviderFactory for OllamaProviderFactory {
    fn provider_type(&self) -> &str {
        "ollama"
    }

    fn display_name(&self) -> &str {
        "Ollama"
    }

    fn category(&self) -> ProviderCategory {
        ProviderCategory::Local
    }

    fn description(&self) -> &str {
        "Local Ollama instance for running open-source models"
    }

    fn default_free_tier(&self) -> FreeTierKind {
        FreeTierKind::AlwaysFreeLocal
    }

    fn setup_parameters(&self) -> Vec<SetupParameter> {
        vec![SetupParameter::optional(
            "base_url",
            ParameterType::BaseUrl,
            "Ollama API base URL",
            Some("http://localhost:11434"),
            false,
        )]
    }

    fn create(
        &self,
        _instance_name: String,
        config: HashMap<String, String>,
    ) -> AppResult<Arc<dyn ModelProvider>> {
        self.validate_config(&config)?;

        let base_url = config
            .get("base_url")
            .cloned()
            .unwrap_or_else(|| "http://localhost:11434".to_string());

        Ok(Arc::new(OllamaProvider::with_base_url(base_url)))
    }

    fn validate_config(&self, config: &HashMap<String, String>) -> AppResult<()> {
        if let Some(url) = config.get("base_url") {
            if !url.starts_with("http://") && !url.starts_with("https://") {
                return Err(AppError::Config(
                    "base_url must start with http:// or https://".to_string(),
                ));
            }
        }
        Ok(())
    }

    fn catalog_provider_id(&self) -> Option<&str> {
        None // Local provider, no catalog mapping
    }

    fn model_list_source(&self) -> ModelListSource {
        ModelListSource::ApiOnly // Local models, catalog irrelevant
    }
}

#[async_trait]
impl DiscoverableProvider for OllamaProviderFactory {
    async fn is_available(&self) -> bool {
        let client = crate::http_client::discovery_client();

        // Primary check: /api/tags is Ollama-specific
        let tags_url = format!("{}/api/tags", self.default_base_url());
        if client.get(&tags_url).send().await.is_err() {
            return false;
        }

        // Bonus verification: GET / should return "Ollama is running"
        // If this fails, still return true since /api/tags on port 11434 is a strong signal
        let root_url = self.default_base_url().to_string();
        if let Ok(resp) = client.get(&root_url).send().await {
            if let Ok(body) = resp.text().await {
                if !body.contains("Ollama is running") {
                    tracing::debug!(
                        "Ollama root check: body does not contain expected string, but /api/tags responded OK"
                    );
                }
            }
        }

        true
    }

    fn default_base_url(&self) -> &str {
        "http://localhost:11434"
    }

    fn default_instance_name(&self) -> &str {
        "Ollama"
    }
}

/// Factory for OpenAI providers
pub struct OpenAIProviderFactory;

impl ProviderFactory for OpenAIProviderFactory {
    fn provider_type(&self) -> &str {
        "openai"
    }

    fn display_name(&self) -> &str {
        "OpenAI"
    }

    fn category(&self) -> ProviderCategory {
        ProviderCategory::FirstParty
    }

    fn description(&self) -> &str {
        "OpenAI API for GPT-4, GPT-3.5, and other models"
    }

    fn setup_parameters(&self) -> Vec<SetupParameter> {
        vec![
            SetupParameter::required(
                "api_key",
                ParameterType::ApiKey,
                "OpenAI API key (starts with sk-)",
                true,
            ),
            SetupParameter::optional(
                "organization",
                ParameterType::Organization,
                "OpenAI organization ID (optional)",
                None::<String>,
                false,
            ),
        ]
    }

    fn create(
        &self,
        _instance_name: String,
        config: HashMap<String, String>,
    ) -> AppResult<Arc<dyn ModelProvider>> {
        self.validate_config(&config)?;

        let api_key = config
            .get("api_key")
            .ok_or_else(|| AppError::Config("api_key is required".to_string()))?
            .clone();

        Ok(Arc::new(OpenAIProvider::new(api_key)))
    }

    fn validate_config(&self, config: &HashMap<String, String>) -> AppResult<()> {
        if !config.contains_key("api_key") {
            return Err(AppError::Config("api_key is required".to_string()));
        }
        Ok(())
    }
}

/// Factory for Anthropic Claude providers
pub struct AnthropicProviderFactory;

impl ProviderFactory for AnthropicProviderFactory {
    fn provider_type(&self) -> &str {
        "anthropic"
    }

    fn display_name(&self) -> &str {
        "Anthropic"
    }

    fn category(&self) -> ProviderCategory {
        ProviderCategory::FirstParty
    }

    fn description(&self) -> &str {
        "Anthropic Claude API for advanced reasoning models"
    }

    fn setup_parameters(&self) -> Vec<SetupParameter> {
        vec![SetupParameter::required(
            "api_key",
            ParameterType::ApiKey,
            "Anthropic API key",
            true,
        )]
    }

    fn create(
        &self,
        _instance_name: String,
        config: HashMap<String, String>,
    ) -> AppResult<Arc<dyn ModelProvider>> {
        self.validate_config(&config)?;

        let api_key = config
            .get("api_key")
            .ok_or_else(|| AppError::Config("api_key is required".to_string()))?
            .clone();

        Ok(Arc::new(AnthropicProvider::new(api_key)?))
    }

    fn validate_config(&self, config: &HashMap<String, String>) -> AppResult<()> {
        if !config.contains_key("api_key") {
            return Err(AppError::Config("api_key is required".to_string()));
        }
        Ok(())
    }
}

/// Factory for Gemini providers
pub struct GeminiProviderFactory;

impl ProviderFactory for GeminiProviderFactory {
    fn provider_type(&self) -> &str {
        "gemini"
    }

    fn display_name(&self) -> &str {
        "Gemini"
    }

    fn category(&self) -> ProviderCategory {
        ProviderCategory::FirstParty
    }

    fn description(&self) -> &str {
        "Gemini API for multimodal AI models"
    }

    fn default_free_tier(&self) -> FreeTierKind {
        FreeTierKind::RateLimitedFree {
            max_rpm: 10,
            max_rpd: 20,
            max_tpm: 250_000,
            max_tpd: 0,
            max_monthly_calls: 0,
            max_monthly_tokens: 0,
        }
    }

    fn free_tier_notes(&self) -> Option<&str> {
        Some("Rate limits vary significantly by model: Flash models allow up to 250 RPD while Pro models are limited to 20 RPD. Limits may also vary by region.")
    }

    fn setup_parameters(&self) -> Vec<SetupParameter> {
        vec![SetupParameter::required(
            "api_key",
            ParameterType::ApiKey,
            "Google API key for Gemini",
            true,
        )]
    }

    fn create(
        &self,
        _instance_name: String,
        config: HashMap<String, String>,
    ) -> AppResult<Arc<dyn ModelProvider>> {
        self.validate_config(&config)?;

        let api_key = config
            .get("api_key")
            .ok_or_else(|| AppError::Config("api_key is required".to_string()))?
            .clone();

        Ok(Arc::new(GeminiProvider::new(api_key)))
    }

    fn validate_config(&self, config: &HashMap<String, String>) -> AppResult<()> {
        if !config.contains_key("api_key") {
            return Err(AppError::Config("api_key is required".to_string()));
        }
        Ok(())
    }

    fn catalog_provider_id(&self) -> Option<&str> {
        Some("google") // models.dev uses "google" not "gemini"
    }
}

/// Factory for OpenRouter providers
pub struct OpenRouterProviderFactory;

impl ProviderFactory for OpenRouterProviderFactory {
    fn provider_type(&self) -> &str {
        "openrouter"
    }

    fn display_name(&self) -> &str {
        "OpenRouter"
    }

    fn category(&self) -> ProviderCategory {
        ProviderCategory::ThirdParty
    }

    fn description(&self) -> &str {
        "OpenRouter multi-provider gateway for accessing multiple LLM providers"
    }

    fn default_free_tier(&self) -> FreeTierKind {
        FreeTierKind::FreeModelsOnly {
            free_model_patterns: vec![":free".to_string()],
            max_rpm: 20,
        }
    }

    fn free_tier_notes(&self) -> Option<&str> {
        Some("Free tier provides access to 25+ free models (model IDs ending in ':free') at 20 RPM / 50 RPD. Purchasing $10+ in credits unlocks 1,000 RPD on free models. BYOK gives 1M free requests/month.")
    }

    fn setup_parameters(&self) -> Vec<SetupParameter> {
        vec![
            SetupParameter::required("api_key", ParameterType::ApiKey, "OpenRouter API key", true),
            SetupParameter::optional(
                "app_name",
                ParameterType::String,
                "Application name for routing (optional)",
                None::<String>,
                false,
            ),
            SetupParameter::optional(
                "app_url",
                ParameterType::String,
                "Application URL for routing (optional)",
                None::<String>,
                false,
            ),
        ]
    }

    fn create(
        &self,
        _instance_name: String,
        config: HashMap<String, String>,
    ) -> AppResult<Arc<dyn ModelProvider>> {
        self.validate_config(&config)?;

        let api_key = config
            .get("api_key")
            .ok_or_else(|| AppError::Config("api_key is required".to_string()))?
            .clone();

        let mut provider = OpenRouterProvider::new(api_key);

        if let Some(app_name) = config.get("app_name") {
            provider = provider.with_app_name(app_name.clone());
        }

        if let Some(app_url) = config.get("app_url") {
            provider = provider.with_app_url(app_url.clone());
        }

        Ok(Arc::new(provider))
    }

    fn validate_config(&self, config: &HashMap<String, String>) -> AppResult<()> {
        if !config.contains_key("api_key") {
            return Err(AppError::Config("api_key is required".to_string()));
        }
        Ok(())
    }

    fn catalog_provider_id(&self) -> Option<&str> {
        None // Aggregator, uses name-based matching instead
    }
}

/// Factory for generic OpenAI-compatible providers
pub struct OpenAICompatibleProviderFactory;

impl ProviderFactory for OpenAICompatibleProviderFactory {
    fn provider_type(&self) -> &str {
        "openai_compatible"
    }

    fn display_name(&self) -> &str {
        "OpenAI Compatible"
    }

    fn category(&self) -> ProviderCategory {
        ProviderCategory::Generic
    }

    fn description(&self) -> &str {
        "Generic OpenAI-compatible API (LocalAI, LM Studio, vLLM, etc.)"
    }

    fn default_free_tier(&self) -> FreeTierKind {
        FreeTierKind::AlwaysFreeLocal
    }

    fn setup_parameters(&self) -> Vec<SetupParameter> {
        vec![
            SetupParameter::required(
                "base_url",
                ParameterType::BaseUrl,
                "API base URL (e.g., http://localhost:8080/v1)",
                false,
            ),
            SetupParameter::optional(
                "api_key",
                ParameterType::ApiKey,
                "API key (optional, not all services require one)",
                None::<String>,
                true,
            ),
        ]
    }

    fn create(
        &self,
        instance_name: String,
        config: HashMap<String, String>,
    ) -> AppResult<Arc<dyn ModelProvider>> {
        self.validate_config(&config)?;

        let base_url = config
            .get("base_url")
            .ok_or_else(|| AppError::Config("base_url is required".to_string()))?
            .clone();

        let api_key = config.get("api_key").cloned();

        Ok(Arc::new(OpenAICompatibleProvider::new(
            instance_name,
            base_url,
            api_key,
        )))
    }

    fn validate_config(&self, config: &HashMap<String, String>) -> AppResult<()> {
        if !config.contains_key("base_url") {
            return Err(AppError::Config("base_url is required".to_string()));
        }

        // Validate base_url format
        if let Some(url) = config.get("base_url") {
            if !url.starts_with("http://") && !url.starts_with("https://") {
                return Err(AppError::Config(
                    "base_url must start with http:// or https://".to_string(),
                ));
            }
        }

        Ok(())
    }

    fn catalog_provider_id(&self) -> Option<&str> {
        None // Generic provider, no catalog mapping
    }

    fn model_list_source(&self) -> ModelListSource {
        ModelListSource::ApiOnly // Generic provider, catalog irrelevant
    }
}

/// Factory for Groq providers
pub struct GroqProviderFactory;

impl ProviderFactory for GroqProviderFactory {
    fn provider_type(&self) -> &str {
        "groq"
    }

    fn display_name(&self) -> &str {
        "Groq"
    }

    fn category(&self) -> ProviderCategory {
        ProviderCategory::FirstParty
    }

    fn description(&self) -> &str {
        "Groq fast inference for open-source models"
    }

    fn default_free_tier(&self) -> FreeTierKind {
        FreeTierKind::RateLimitedFree {
            max_rpm: 30,
            max_rpd: 14_400,
            max_tpm: 6_000,
            max_tpd: 500_000,
            max_monthly_calls: 0,
            max_monthly_tokens: 0,
        }
    }

    fn free_tier_notes(&self) -> Option<&str> {
        Some("Rate limits vary by model. Some models (e.g. Llama 3.3 70B) have lower daily limits (1K RPD). Token limits also vary per model.")
    }

    fn setup_parameters(&self) -> Vec<SetupParameter> {
        vec![SetupParameter::required(
            "api_key",
            ParameterType::ApiKey,
            "Groq API key",
            true,
        )]
    }

    fn create(
        &self,
        _instance_name: String,
        config: HashMap<String, String>,
    ) -> AppResult<Arc<dyn ModelProvider>> {
        self.validate_config(&config)?;

        let api_key = config
            .get("api_key")
            .ok_or_else(|| AppError::Config("api_key is required".to_string()))?
            .clone();

        Ok(Arc::new(GroqProvider::new(api_key)?))
    }

    fn validate_config(&self, config: &HashMap<String, String>) -> AppResult<()> {
        if !config.contains_key("api_key") {
            return Err(AppError::Config("api_key is required".to_string()));
        }
        Ok(())
    }
}

/// Factory for Mistral providers
pub struct MistralProviderFactory;

impl ProviderFactory for MistralProviderFactory {
    fn provider_type(&self) -> &str {
        "mistral"
    }

    fn display_name(&self) -> &str {
        "Mistral AI"
    }

    fn category(&self) -> ProviderCategory {
        ProviderCategory::FirstParty
    }

    fn description(&self) -> &str {
        "Mistral AI models including Mistral Large and Codestral"
    }

    fn default_free_tier(&self) -> FreeTierKind {
        FreeTierKind::RateLimitedFree {
            max_rpm: 60,
            max_rpd: 0,
            max_tpm: 500_000,
            max_tpd: 0,
            max_monthly_calls: 0,
            max_monthly_tokens: 1_000_000_000,
        }
    }

    fn free_tier_notes(&self) -> Option<&str> {
        Some("Free tier (experiment plan) allows 1 request/second and 1 billion tokens/month. All models are accessible.")
    }

    fn setup_parameters(&self) -> Vec<SetupParameter> {
        vec![SetupParameter::required(
            "api_key",
            ParameterType::ApiKey,
            "Mistral API key",
            true,
        )]
    }

    fn create(
        &self,
        _instance_name: String,
        config: HashMap<String, String>,
    ) -> AppResult<Arc<dyn ModelProvider>> {
        self.validate_config(&config)?;

        let api_key = config
            .get("api_key")
            .ok_or_else(|| AppError::Config("api_key is required".to_string()))?
            .clone();

        Ok(Arc::new(MistralProvider::new(api_key)?))
    }

    fn validate_config(&self, config: &HashMap<String, String>) -> AppResult<()> {
        if !config.contains_key("api_key") {
            return Err(AppError::Config("api_key is required".to_string()));
        }
        Ok(())
    }
}

/// Factory for Cohere providers
pub struct CohereProviderFactory;

impl ProviderFactory for CohereProviderFactory {
    fn provider_type(&self) -> &str {
        "cohere"
    }

    fn display_name(&self) -> &str {
        "Cohere"
    }

    fn category(&self) -> ProviderCategory {
        ProviderCategory::FirstParty
    }

    fn description(&self) -> &str {
        "Cohere AI including Command R+ and specialized models"
    }

    fn default_free_tier(&self) -> FreeTierKind {
        FreeTierKind::RateLimitedFree {
            max_rpm: 20,
            max_rpd: 0,
            max_tpm: 100_000,
            max_tpd: 0,
            max_monthly_calls: 1_000,
            max_monthly_tokens: 0,
        }
    }

    fn free_tier_notes(&self) -> Option<&str> {
        Some("Trial API keys are limited to 1,000 API calls/month and 20 RPM. Contact support for production increases.")
    }

    fn setup_parameters(&self) -> Vec<SetupParameter> {
        vec![SetupParameter::required(
            "api_key",
            ParameterType::ApiKey,
            "Cohere API key",
            true,
        )]
    }

    fn create(
        &self,
        _instance_name: String,
        config: HashMap<String, String>,
    ) -> AppResult<Arc<dyn ModelProvider>> {
        self.validate_config(&config)?;

        let api_key = config
            .get("api_key")
            .ok_or_else(|| AppError::Config("api_key is required".to_string()))?
            .clone();

        Ok(Arc::new(CohereProvider::new(api_key)?))
    }

    fn validate_config(&self, config: &HashMap<String, String>) -> AppResult<()> {
        if !config.contains_key("api_key") {
            return Err(AppError::Config("api_key is required".to_string()));
        }
        Ok(())
    }
}

/// Factory for Together AI providers
pub struct TogetherAIProviderFactory;

impl ProviderFactory for TogetherAIProviderFactory {
    fn provider_type(&self) -> &str {
        "togetherai"
    }

    fn display_name(&self) -> &str {
        "Together AI"
    }

    fn category(&self) -> ProviderCategory {
        ProviderCategory::ThirdParty
    }

    fn description(&self) -> &str {
        "Together AI platform for open-source models"
    }

    fn default_free_tier(&self) -> FreeTierKind {
        FreeTierKind::FreeModelsOnly {
            free_model_patterns: vec!["meta-llama/Llama-3.3-70B-Instruct-Turbo-Free".to_string()],
            max_rpm: 3,
        }
    }

    fn free_tier_notes(&self) -> Option<&str> {
        Some("Only specific models are free (currently Llama 3.3 70B Instruct Turbo Free). Rate limited to 3 RPM on free models.")
    }

    fn setup_parameters(&self) -> Vec<SetupParameter> {
        vec![SetupParameter::required(
            "api_key",
            ParameterType::ApiKey,
            "Together AI API key",
            true,
        )]
    }

    fn create(
        &self,
        _instance_name: String,
        config: HashMap<String, String>,
    ) -> AppResult<Arc<dyn ModelProvider>> {
        self.validate_config(&config)?;

        let api_key = config
            .get("api_key")
            .ok_or_else(|| AppError::Config("api_key is required".to_string()))?
            .clone();

        Ok(Arc::new(TogetherAIProvider::new(api_key)?))
    }

    fn validate_config(&self, config: &HashMap<String, String>) -> AppResult<()> {
        if !config.contains_key("api_key") {
            return Err(AppError::Config("api_key is required".to_string()));
        }
        Ok(())
    }

    fn catalog_provider_id(&self) -> Option<&str> {
        Some("together") // models.dev uses "together" not "togetherai"
    }
}

/// Factory for Perplexity providers
pub struct PerplexityProviderFactory;

impl ProviderFactory for PerplexityProviderFactory {
    fn provider_type(&self) -> &str {
        "perplexity"
    }

    fn display_name(&self) -> &str {
        "Perplexity"
    }

    fn category(&self) -> ProviderCategory {
        ProviderCategory::FirstParty
    }

    fn description(&self) -> &str {
        "Perplexity AI search-augmented models"
    }

    fn setup_parameters(&self) -> Vec<SetupParameter> {
        vec![SetupParameter::required(
            "api_key",
            ParameterType::ApiKey,
            "Perplexity API key",
            true,
        )]
    }

    fn create(
        &self,
        _instance_name: String,
        config: HashMap<String, String>,
    ) -> AppResult<Arc<dyn ModelProvider>> {
        self.validate_config(&config)?;

        let api_key = config
            .get("api_key")
            .ok_or_else(|| AppError::Config("api_key is required".to_string()))?
            .clone();

        Ok(Arc::new(PerplexityProvider::new(api_key)?))
    }

    fn validate_config(&self, config: &HashMap<String, String>) -> AppResult<()> {
        if !config.contains_key("api_key") {
            return Err(AppError::Config("api_key is required".to_string()));
        }
        Ok(())
    }

    fn model_list_source(&self) -> ModelListSource {
        ModelListSource::CatalogOnly // Perplexity has no public /models endpoint
    }

    fn default_free_tier(&self) -> FreeTierKind {
        FreeTierKind::None
    }

    fn free_tier_notes(&self) -> Option<&str> {
        Some("No free API tier. All API usage requires payment.")
    }
}

/// Factory for DeepInfra providers
pub struct DeepInfraProviderFactory;

impl ProviderFactory for DeepInfraProviderFactory {
    fn provider_type(&self) -> &str {
        "deepinfra"
    }

    fn display_name(&self) -> &str {
        "DeepInfra"
    }

    fn category(&self) -> ProviderCategory {
        ProviderCategory::ThirdParty
    }

    fn description(&self) -> &str {
        "DeepInfra cost-effective hosting for open-source models"
    }

    fn default_free_tier(&self) -> FreeTierKind {
        FreeTierKind::CreditBased {
            budget_usd: 5.0,
            reset_period: lr_config::FreeTierResetPeriod::Monthly,
            detection: lr_config::CreditDetection::LocalOnly,
        }
    }

    fn free_tier_notes(&self) -> Option<&str> {
        Some("$5 monthly free credits for inference. Credits reset monthly.")
    }

    fn setup_parameters(&self) -> Vec<SetupParameter> {
        vec![SetupParameter::required(
            "api_key",
            ParameterType::ApiKey,
            "DeepInfra API key",
            true,
        )]
    }

    fn create(
        &self,
        _instance_name: String,
        config: HashMap<String, String>,
    ) -> AppResult<Arc<dyn ModelProvider>> {
        self.validate_config(&config)?;

        let api_key = config
            .get("api_key")
            .ok_or_else(|| AppError::Config("api_key is required".to_string()))?
            .clone();

        Ok(Arc::new(DeepInfraProvider::new(api_key)?))
    }

    fn validate_config(&self, config: &HashMap<String, String>) -> AppResult<()> {
        if !config.contains_key("api_key") {
            return Err(AppError::Config("api_key is required".to_string()));
        }
        Ok(())
    }
}

/// Factory for Cerebras providers
pub struct CerebrasProviderFactory;

impl ProviderFactory for CerebrasProviderFactory {
    fn provider_type(&self) -> &str {
        "cerebras"
    }

    fn display_name(&self) -> &str {
        "Cerebras"
    }

    fn category(&self) -> ProviderCategory {
        ProviderCategory::FirstParty
    }

    fn description(&self) -> &str {
        "Cerebras ultra-fast inference platform"
    }

    fn default_free_tier(&self) -> FreeTierKind {
        FreeTierKind::RateLimitedFree {
            max_rpm: 30,
            max_rpd: 14_400,
            max_tpm: 60_000,
            max_tpd: 1_000_000,
            max_monthly_calls: 0,
            max_monthly_tokens: 0,
        }
    }

    fn free_tier_notes(&self) -> Option<&str> {
        Some("Developer tier offers 10x higher limits. Exact free tier limits are not publicly documented and may change.")
    }

    fn setup_parameters(&self) -> Vec<SetupParameter> {
        vec![SetupParameter::required(
            "api_key",
            ParameterType::ApiKey,
            "Cerebras API key",
            true,
        )]
    }

    fn create(
        &self,
        _instance_name: String,
        config: HashMap<String, String>,
    ) -> AppResult<Arc<dyn ModelProvider>> {
        self.validate_config(&config)?;

        let api_key = config
            .get("api_key")
            .ok_or_else(|| AppError::Config("api_key is required".to_string()))?
            .clone();

        Ok(Arc::new(CerebrasProvider::new(api_key)?))
    }

    fn validate_config(&self, config: &HashMap<String, String>) -> AppResult<()> {
        if !config.contains_key("api_key") {
            return Err(AppError::Config("api_key is required".to_string()));
        }
        Ok(())
    }
}

/// Factory for xAI providers
pub struct XAIProviderFactory;

impl ProviderFactory for XAIProviderFactory {
    fn provider_type(&self) -> &str {
        "xai"
    }

    fn display_name(&self) -> &str {
        "xAI"
    }

    fn category(&self) -> ProviderCategory {
        ProviderCategory::FirstParty
    }

    fn description(&self) -> &str {
        "xAI Grok models with real-time knowledge access"
    }

    fn default_free_tier(&self) -> FreeTierKind {
        FreeTierKind::CreditBased {
            budget_usd: 25.0,
            reset_period: lr_config::FreeTierResetPeriod::Never,
            detection: lr_config::CreditDetection::LocalOnly,
        }
    }

    fn free_tier_notes(&self) -> Option<&str> {
        Some("$25 one-time signup credits. No recurring free tier.")
    }

    fn setup_parameters(&self) -> Vec<SetupParameter> {
        vec![SetupParameter::required(
            "api_key",
            ParameterType::ApiKey,
            "xAI API key",
            true,
        )]
    }

    fn create(
        &self,
        _instance_name: String,
        config: HashMap<String, String>,
    ) -> AppResult<Arc<dyn ModelProvider>> {
        self.validate_config(&config)?;

        let api_key = config
            .get("api_key")
            .ok_or_else(|| AppError::Config("api_key is required".to_string()))?
            .clone();

        Ok(Arc::new(XAIProvider::new(api_key)?))
    }

    fn validate_config(&self, config: &HashMap<String, String>) -> AppResult<()> {
        if !config.contains_key("api_key") {
            return Err(AppError::Config("api_key is required".to_string()));
        }
        Ok(())
    }
}

/// Factory for LM Studio providers
pub struct LMStudioProviderFactory;

impl ProviderFactory for LMStudioProviderFactory {
    fn provider_type(&self) -> &str {
        "lmstudio"
    }

    fn display_name(&self) -> &str {
        "LM Studio"
    }

    fn category(&self) -> ProviderCategory {
        ProviderCategory::Local
    }

    fn description(&self) -> &str {
        "LM Studio local inference with OpenAI-compatible API"
    }

    fn default_free_tier(&self) -> FreeTierKind {
        FreeTierKind::AlwaysFreeLocal
    }

    fn setup_parameters(&self) -> Vec<SetupParameter> {
        vec![
            SetupParameter::optional(
                "base_url",
                ParameterType::BaseUrl,
                "LM Studio API base URL",
                Some("http://localhost:1234/v1"),
                false,
            ),
            SetupParameter::optional(
                "api_key",
                ParameterType::ApiKey,
                "API key (optional, not required for local LM Studio)",
                None::<String>,
                true,
            ),
        ]
    }

    fn create(
        &self,
        _instance_name: String,
        config: HashMap<String, String>,
    ) -> AppResult<Arc<dyn ModelProvider>> {
        self.validate_config(&config)?;

        let base_url = config
            .get("base_url")
            .cloned()
            .unwrap_or_else(|| "http://localhost:1234/v1".to_string());

        let api_key = config.get("api_key").cloned();

        Ok(Arc::new(
            LMStudioProvider::with_base_url(base_url).with_api_key(api_key),
        ))
    }

    fn validate_config(&self, config: &HashMap<String, String>) -> AppResult<()> {
        // Validate base_url format if provided
        if let Some(url) = config.get("base_url") {
            if !url.starts_with("http://") && !url.starts_with("https://") {
                return Err(AppError::Config(
                    "base_url must start with http:// or https://".to_string(),
                ));
            }
        }
        Ok(())
    }

    fn catalog_provider_id(&self) -> Option<&str> {
        None // Local provider, no catalog mapping
    }

    fn model_list_source(&self) -> ModelListSource {
        ModelListSource::ApiOnly // Local models, catalog irrelevant
    }
}

#[async_trait]
impl DiscoverableProvider for LMStudioProviderFactory {
    async fn is_available(&self) -> bool {
        let client = crate::http_client::discovery_client();

        let url = format!("{}/models", self.default_base_url());
        client.get(&url).send().await.is_ok()
    }

    fn default_base_url(&self) -> &str {
        "http://localhost:1234/v1"
    }

    fn default_instance_name(&self) -> &str {
        "LM Studio"
    }
}

/// Factory for Jan providers
pub struct JanProviderFactory;

impl ProviderFactory for JanProviderFactory {
    fn provider_type(&self) -> &str {
        "jan"
    }

    fn display_name(&self) -> &str {
        "Jan"
    }

    fn category(&self) -> ProviderCategory {
        ProviderCategory::Local
    }

    fn description(&self) -> &str {
        "Jan.ai local inference with OpenAI-compatible API"
    }

    fn default_free_tier(&self) -> FreeTierKind {
        FreeTierKind::AlwaysFreeLocal
    }

    fn setup_parameters(&self) -> Vec<SetupParameter> {
        vec![
            SetupParameter::optional(
                "base_url",
                ParameterType::BaseUrl,
                "Jan API base URL",
                Some("http://localhost:1337/v1"),
                false,
            ),
            SetupParameter::optional(
                "api_key",
                ParameterType::ApiKey,
                "API key (optional, not required for local Jan)",
                None::<String>,
                true,
            ),
        ]
    }

    fn create(
        &self,
        _instance_name: String,
        config: HashMap<String, String>,
    ) -> AppResult<Arc<dyn ModelProvider>> {
        self.validate_config(&config)?;

        let base_url = config
            .get("base_url")
            .cloned()
            .unwrap_or_else(|| "http://localhost:1337/v1".to_string());

        let api_key = config.get("api_key").cloned();

        Ok(Arc::new(
            JanProvider::with_base_url(base_url).with_api_key(api_key),
        ))
    }

    fn validate_config(&self, config: &HashMap<String, String>) -> AppResult<()> {
        if let Some(url) = config.get("base_url") {
            if !url.starts_with("http://") && !url.starts_with("https://") {
                return Err(AppError::Config(
                    "base_url must start with http:// or https://".to_string(),
                ));
            }
        }
        Ok(())
    }

    fn catalog_provider_id(&self) -> Option<&str> {
        None
    }

    fn model_list_source(&self) -> ModelListSource {
        ModelListSource::ApiOnly
    }
}

#[async_trait]
impl DiscoverableProvider for JanProviderFactory {
    async fn is_available(&self) -> bool {
        let client = crate::http_client::discovery_client();

        let url = format!("{}/models", self.default_base_url());
        client.get(&url).send().await.is_ok()
    }

    fn default_base_url(&self) -> &str {
        "http://localhost:1337/v1"
    }

    fn default_instance_name(&self) -> &str {
        "Jan"
    }
}

/// Factory for GPT4All providers
pub struct GPT4AllProviderFactory;

impl ProviderFactory for GPT4AllProviderFactory {
    fn provider_type(&self) -> &str {
        "gpt4all"
    }

    fn display_name(&self) -> &str {
        "GPT4All"
    }

    fn category(&self) -> ProviderCategory {
        ProviderCategory::Local
    }

    fn description(&self) -> &str {
        "GPT4All local inference with OpenAI-compatible API"
    }

    fn default_free_tier(&self) -> FreeTierKind {
        FreeTierKind::AlwaysFreeLocal
    }

    fn setup_parameters(&self) -> Vec<SetupParameter> {
        vec![
            SetupParameter::optional(
                "base_url",
                ParameterType::BaseUrl,
                "GPT4All API base URL",
                Some("http://localhost:4891/v1"),
                false,
            ),
            SetupParameter::optional(
                "api_key",
                ParameterType::ApiKey,
                "API key (optional, not required for local GPT4All)",
                None::<String>,
                true,
            ),
        ]
    }

    fn create(
        &self,
        _instance_name: String,
        config: HashMap<String, String>,
    ) -> AppResult<Arc<dyn ModelProvider>> {
        self.validate_config(&config)?;

        let base_url = config
            .get("base_url")
            .cloned()
            .unwrap_or_else(|| "http://localhost:4891/v1".to_string());

        let api_key = config.get("api_key").cloned();

        Ok(Arc::new(
            GPT4AllProvider::with_base_url(base_url).with_api_key(api_key),
        ))
    }

    fn validate_config(&self, config: &HashMap<String, String>) -> AppResult<()> {
        if let Some(url) = config.get("base_url") {
            if !url.starts_with("http://") && !url.starts_with("https://") {
                return Err(AppError::Config(
                    "base_url must start with http:// or https://".to_string(),
                ));
            }
        }
        Ok(())
    }

    fn catalog_provider_id(&self) -> Option<&str> {
        None
    }

    fn model_list_source(&self) -> ModelListSource {
        ModelListSource::ApiOnly
    }
}

#[async_trait]
impl DiscoverableProvider for GPT4AllProviderFactory {
    async fn is_available(&self) -> bool {
        let client = crate::http_client::discovery_client();

        let url = format!("{}/models", self.default_base_url());
        client.get(&url).send().await.is_ok()
    }

    fn default_base_url(&self) -> &str {
        "http://localhost:4891/v1"
    }

    fn default_instance_name(&self) -> &str {
        "GPT4All"
    }
}

/// Factory for LocalAI providers
pub struct LocalAIProviderFactory;

impl ProviderFactory for LocalAIProviderFactory {
    fn provider_type(&self) -> &str {
        "localai"
    }

    fn display_name(&self) -> &str {
        "LocalAI"
    }

    fn category(&self) -> ProviderCategory {
        ProviderCategory::Local
    }

    fn description(&self) -> &str {
        "LocalAI local inference with OpenAI-compatible API"
    }

    fn default_free_tier(&self) -> FreeTierKind {
        FreeTierKind::AlwaysFreeLocal
    }

    fn setup_parameters(&self) -> Vec<SetupParameter> {
        vec![
            SetupParameter::optional(
                "base_url",
                ParameterType::BaseUrl,
                "LocalAI API base URL",
                Some("http://localhost:8080/v1"),
                false,
            ),
            SetupParameter::optional(
                "api_key",
                ParameterType::ApiKey,
                "API key (optional, not required for local LocalAI)",
                None::<String>,
                true,
            ),
        ]
    }

    fn create(
        &self,
        _instance_name: String,
        config: HashMap<String, String>,
    ) -> AppResult<Arc<dyn ModelProvider>> {
        self.validate_config(&config)?;

        let base_url = config
            .get("base_url")
            .cloned()
            .unwrap_or_else(|| "http://localhost:8080/v1".to_string());

        let api_key = config.get("api_key").cloned();

        Ok(Arc::new(
            LocalAIProvider::with_base_url(base_url).with_api_key(api_key),
        ))
    }

    fn validate_config(&self, config: &HashMap<String, String>) -> AppResult<()> {
        if let Some(url) = config.get("base_url") {
            if !url.starts_with("http://") && !url.starts_with("https://") {
                return Err(AppError::Config(
                    "base_url must start with http:// or https://".to_string(),
                ));
            }
        }
        Ok(())
    }

    fn catalog_provider_id(&self) -> Option<&str> {
        None
    }

    fn model_list_source(&self) -> ModelListSource {
        ModelListSource::ApiOnly
    }
}

#[async_trait]
impl DiscoverableProvider for LocalAIProviderFactory {
    async fn is_available(&self) -> bool {
        let client = crate::http_client::discovery_client();

        let url = format!("{}/models", self.default_base_url());
        client.get(&url).send().await.is_ok()
    }

    fn default_base_url(&self) -> &str {
        "http://localhost:8080/v1"
    }

    fn default_instance_name(&self) -> &str {
        "LocalAI"
    }
}

/// Factory for llama.cpp providers
pub struct LlamaCppProviderFactory;

impl ProviderFactory for LlamaCppProviderFactory {
    fn provider_type(&self) -> &str {
        "llamacpp"
    }

    fn display_name(&self) -> &str {
        "llama.cpp"
    }

    fn category(&self) -> ProviderCategory {
        ProviderCategory::Local
    }

    fn description(&self) -> &str {
        "llama.cpp local inference server with OpenAI-compatible API"
    }

    fn default_free_tier(&self) -> FreeTierKind {
        FreeTierKind::AlwaysFreeLocal
    }

    fn setup_parameters(&self) -> Vec<SetupParameter> {
        vec![
            SetupParameter::optional(
                "base_url",
                ParameterType::BaseUrl,
                "llama.cpp API base URL",
                Some("http://localhost:8080/v1"),
                false,
            ),
            SetupParameter::optional(
                "api_key",
                ParameterType::ApiKey,
                "API key (optional, not required for local llama.cpp)",
                None::<String>,
                true,
            ),
        ]
    }

    fn create(
        &self,
        _instance_name: String,
        config: HashMap<String, String>,
    ) -> AppResult<Arc<dyn ModelProvider>> {
        self.validate_config(&config)?;

        let base_url = config
            .get("base_url")
            .cloned()
            .unwrap_or_else(|| "http://localhost:8080/v1".to_string());

        let api_key = config.get("api_key").cloned();

        Ok(Arc::new(
            LlamaCppProvider::with_base_url(base_url).with_api_key(api_key),
        ))
    }

    fn validate_config(&self, config: &HashMap<String, String>) -> AppResult<()> {
        if let Some(url) = config.get("base_url") {
            if !url.starts_with("http://") && !url.starts_with("https://") {
                return Err(AppError::Config(
                    "base_url must start with http:// or https://".to_string(),
                ));
            }
        }
        Ok(())
    }

    fn catalog_provider_id(&self) -> Option<&str> {
        None
    }

    fn model_list_source(&self) -> ModelListSource {
        ModelListSource::ApiOnly
    }
}

#[async_trait]
impl DiscoverableProvider for LlamaCppProviderFactory {
    async fn is_available(&self) -> bool {
        let client = crate::http_client::discovery_client();

        let url = format!("{}/models", self.default_base_url());
        client.get(&url).send().await.is_ok()
    }

    fn default_base_url(&self) -> &str {
        "http://localhost:8080/v1"
    }

    fn default_instance_name(&self) -> &str {
        "llama.cpp"
    }
}

// ==================== NEW FREE-TIER PROVIDER FACTORIES ====================

/// Factory for GitHub Models providers
pub struct GitHubModelsProviderFactory;

impl ProviderFactory for GitHubModelsProviderFactory {
    fn provider_type(&self) -> &str {
        "github_models"
    }

    fn display_name(&self) -> &str {
        "GitHub Models"
    }

    fn category(&self) -> ProviderCategory {
        ProviderCategory::ThirdParty
    }

    fn description(&self) -> &str {
        "GitHub Models free inference API"
    }

    fn default_free_tier(&self) -> FreeTierKind {
        FreeTierKind::RateLimitedFree {
            max_rpm: 10,
            max_rpd: 50,
            max_tpm: 0,
            max_tpd: 0,
            max_monthly_calls: 0,
            max_monthly_tokens: 0,
        }
    }

    fn free_tier_notes(&self) -> Option<&str> {
        Some("Limits vary by model tier: Low models get 15 RPM / 150 RPD, High models get 10 RPM / 50 RPD. Uses GitHub Personal Access Token for auth.")
    }

    fn setup_parameters(&self) -> Vec<SetupParameter> {
        vec![SetupParameter::required(
            "api_key",
            ParameterType::ApiKey,
            "GitHub Personal Access Token",
            true,
        )]
    }

    fn create(
        &self,
        _instance_name: String,
        config: HashMap<String, String>,
    ) -> AppResult<Arc<dyn ModelProvider>> {
        self.validate_config(&config)?;

        let api_key = config
            .get("api_key")
            .ok_or_else(|| AppError::Config("api_key is required".to_string()))?
            .clone();

        Ok(Arc::new(OpenAICompatibleProvider::new(
            "github_models".to_string(),
            "https://models.inference.ai.azure.com".to_string(),
            Some(api_key),
        )))
    }

    fn validate_config(&self, config: &HashMap<String, String>) -> AppResult<()> {
        if !config.contains_key("api_key") {
            return Err(AppError::Config("api_key is required".to_string()));
        }
        Ok(())
    }

    fn catalog_provider_id(&self) -> Option<&str> {
        Some("github-models") // models.dev uses "github-models" not "github_models"
    }
}

/// Factory for NVIDIA NIM providers
pub struct NvidiaNimProviderFactory;

impl ProviderFactory for NvidiaNimProviderFactory {
    fn provider_type(&self) -> &str {
        "nvidia_nim"
    }

    fn display_name(&self) -> &str {
        "NVIDIA NIM"
    }

    fn category(&self) -> ProviderCategory {
        ProviderCategory::ThirdParty
    }

    fn description(&self) -> &str {
        "NVIDIA NIM inference API for 100+ models"
    }

    fn default_free_tier(&self) -> FreeTierKind {
        FreeTierKind::RateLimitedFree {
            max_rpm: 40,
            max_rpd: 0,
            max_tpm: 0,
            max_tpd: 0,
            max_monthly_calls: 0,
            max_monthly_tokens: 0,
        }
    }

    fn free_tier_notes(&self) -> Option<&str> {
        Some("40 RPM on free tier. Access to 100+ models including Llama, Mistral, Qwen. Daily limits undocumented.")
    }

    fn setup_parameters(&self) -> Vec<SetupParameter> {
        vec![SetupParameter::required(
            "api_key",
            ParameterType::ApiKey,
            "NVIDIA API key",
            true,
        )]
    }

    fn create(
        &self,
        _instance_name: String,
        config: HashMap<String, String>,
    ) -> AppResult<Arc<dyn ModelProvider>> {
        self.validate_config(&config)?;

        let api_key = config
            .get("api_key")
            .ok_or_else(|| AppError::Config("api_key is required".to_string()))?
            .clone();

        Ok(Arc::new(OpenAICompatibleProvider::new(
            "nvidia_nim".to_string(),
            "https://integrate.api.nvidia.com/v1".to_string(),
            Some(api_key),
        )))
    }

    fn validate_config(&self, config: &HashMap<String, String>) -> AppResult<()> {
        if !config.contains_key("api_key") {
            return Err(AppError::Config("api_key is required".to_string()));
        }
        Ok(())
    }

    fn catalog_provider_id(&self) -> Option<&str> {
        Some("nvidia") // models.dev uses "nvidia" not "nvidia_nim"
    }
}

/// Factory for Cloudflare Workers AI providers
pub struct CloudflareAIProviderFactory;

impl ProviderFactory for CloudflareAIProviderFactory {
    fn provider_type(&self) -> &str {
        "cloudflare_ai"
    }

    fn display_name(&self) -> &str {
        "Cloudflare Workers AI"
    }

    fn category(&self) -> ProviderCategory {
        ProviderCategory::ThirdParty
    }

    fn description(&self) -> &str {
        "Cloudflare Workers AI inference platform"
    }

    fn default_free_tier(&self) -> FreeTierKind {
        FreeTierKind::RateLimitedFree {
            max_rpm: 0,
            max_rpd: 0,
            max_tpm: 0,
            max_tpd: 0,
            max_monthly_calls: 0,
            max_monthly_tokens: 0,
        }
    }

    fn free_tier_notes(&self) -> Option<&str> {
        Some("10,000 neurons/day free allowance. Neuron cost varies by model and input size. Requires Cloudflare account ID in base URL.")
    }

    fn setup_parameters(&self) -> Vec<SetupParameter> {
        vec![
            SetupParameter::required(
                "api_key",
                ParameterType::ApiKey,
                "Cloudflare API token",
                true,
            ),
            SetupParameter::required(
                "base_url",
                ParameterType::BaseUrl,
                "Cloudflare AI Gateway URL (find it on the AI Gateway page)",
                false,
            ),
        ]
    }

    fn create(
        &self,
        _instance_name: String,
        config: HashMap<String, String>,
    ) -> AppResult<Arc<dyn ModelProvider>> {
        self.validate_config(&config)?;

        let api_key = config
            .get("api_key")
            .ok_or_else(|| AppError::Config("api_key is required".to_string()))?
            .clone();

        let base_url = config
            .get("base_url")
            .ok_or_else(|| AppError::Config("base_url is required".to_string()))?
            .clone();

        Ok(Arc::new(OpenAICompatibleProvider::new(
            "cloudflare_ai".to_string(),
            base_url,
            Some(api_key),
        )))
    }

    fn validate_config(&self, config: &HashMap<String, String>) -> AppResult<()> {
        if !config.contains_key("api_key") {
            return Err(AppError::Config("api_key is required".to_string()));
        }
        if !config.contains_key("base_url") {
            return Err(AppError::Config("base_url is required".to_string()));
        }
        if let Some(url) = config.get("base_url") {
            if !url.starts_with("https://") {
                return Err(AppError::Config(
                    "base_url must start with https://".to_string(),
                ));
            }
        }
        Ok(())
    }

    fn catalog_provider_id(&self) -> Option<&str> {
        Some("cloudflare-workers-ai") // models.dev uses "cloudflare-workers-ai"
    }
}

/// Factory for LLM7.io providers
pub struct Llm7ProviderFactory;

impl ProviderFactory for Llm7ProviderFactory {
    fn provider_type(&self) -> &str {
        "llm7"
    }

    fn display_name(&self) -> &str {
        "LLM7.io"
    }

    fn category(&self) -> ProviderCategory {
        ProviderCategory::ThirdParty
    }

    fn description(&self) -> &str {
        "LLM7.io free inference API for open-source models"
    }

    fn default_free_tier(&self) -> FreeTierKind {
        FreeTierKind::RateLimitedFree {
            max_rpm: 30,
            max_rpd: 0,
            max_tpm: 0,
            max_tpd: 0,
            max_monthly_calls: 0,
            max_monthly_tokens: 0,
        }
    }

    fn free_tier_notes(&self) -> Option<&str> {
        Some("30 RPM without token, 120 RPM with token. Access to DeepSeek R1, Qwen2.5 Coder, and 27+ more models.")
    }

    fn setup_parameters(&self) -> Vec<SetupParameter> {
        vec![SetupParameter::optional(
            "api_key",
            ParameterType::ApiKey,
            "LLM7 API token (optional, increases rate limits)",
            None::<String>,
            true,
        )]
    }

    fn create(
        &self,
        _instance_name: String,
        config: HashMap<String, String>,
    ) -> AppResult<Arc<dyn ModelProvider>> {
        self.validate_config(&config)?;

        let api_key = config.get("api_key").cloned();

        Ok(Arc::new(OpenAICompatibleProvider::new(
            "llm7".to_string(),
            "https://api.llm7.io/v1".to_string(),
            api_key,
        )))
    }

    fn validate_config(&self, _config: &HashMap<String, String>) -> AppResult<()> {
        Ok(())
    }

    fn catalog_provider_id(&self) -> Option<&str> {
        None
    }
}

/// Factory for Kluster AI providers
pub struct KlusterAIProviderFactory;

impl ProviderFactory for KlusterAIProviderFactory {
    fn provider_type(&self) -> &str {
        "kluster_ai"
    }

    fn display_name(&self) -> &str {
        "Kluster AI"
    }

    fn category(&self) -> ProviderCategory {
        ProviderCategory::ThirdParty
    }

    fn description(&self) -> &str {
        "Kluster AI inference for DeepSeek, Llama, and Qwen models"
    }

    fn default_free_tier(&self) -> FreeTierKind {
        FreeTierKind::RateLimitedFree {
            max_rpm: 30,
            max_rpd: 0,
            max_tpm: 0,
            max_tpd: 0,
            max_monthly_calls: 0,
            max_monthly_tokens: 0,
        }
    }

    fn free_tier_notes(&self) -> Option<&str> {
        Some("Free tier limits are undocumented. Supports DeepSeek-R1, Llama 4 Maverick, Qwen3-235B.")
    }

    fn setup_parameters(&self) -> Vec<SetupParameter> {
        vec![SetupParameter::required(
            "api_key",
            ParameterType::ApiKey,
            "Kluster AI API key",
            true,
        )]
    }

    fn create(
        &self,
        _instance_name: String,
        config: HashMap<String, String>,
    ) -> AppResult<Arc<dyn ModelProvider>> {
        self.validate_config(&config)?;

        let api_key = config
            .get("api_key")
            .ok_or_else(|| AppError::Config("api_key is required".to_string()))?
            .clone();

        Ok(Arc::new(OpenAICompatibleProvider::new(
            "kluster_ai".to_string(),
            "https://api.kluster.ai/v1".to_string(),
            Some(api_key),
        )))
    }

    fn validate_config(&self, config: &HashMap<String, String>) -> AppResult<()> {
        if !config.contains_key("api_key") {
            return Err(AppError::Config("api_key is required".to_string()));
        }
        Ok(())
    }

    fn catalog_provider_id(&self) -> Option<&str> {
        None
    }
}

/// Factory for Hugging Face Inference providers
pub struct HuggingFaceProviderFactory;

impl ProviderFactory for HuggingFaceProviderFactory {
    fn provider_type(&self) -> &str {
        "huggingface"
    }

    fn display_name(&self) -> &str {
        "Hugging Face"
    }

    fn category(&self) -> ProviderCategory {
        ProviderCategory::ThirdParty
    }

    fn description(&self) -> &str {
        "Hugging Face Inference API for thousands of models"
    }

    fn default_free_tier(&self) -> FreeTierKind {
        FreeTierKind::CreditBased {
            budget_usd: 0.10,
            reset_period: lr_config::FreeTierResetPeriod::Monthly,
            detection: lr_config::CreditDetection::LocalOnly,
        }
    }

    fn free_tier_notes(&self) -> Option<&str> {
        Some("$0.10/month free credits for all users. PRO users get $2/month. No markup — provider costs passed through directly. Uses HF User Access Token.")
    }

    fn setup_parameters(&self) -> Vec<SetupParameter> {
        vec![SetupParameter::required(
            "api_key",
            ParameterType::ApiKey,
            "Hugging Face User Access Token",
            true,
        )]
    }

    fn create(
        &self,
        _instance_name: String,
        config: HashMap<String, String>,
    ) -> AppResult<Arc<dyn ModelProvider>> {
        self.validate_config(&config)?;

        let api_key = config
            .get("api_key")
            .ok_or_else(|| AppError::Config("api_key is required".to_string()))?
            .clone();

        Ok(Arc::new(OpenAICompatibleProvider::new(
            "huggingface".to_string(),
            "https://router.huggingface.co/v1".to_string(),
            Some(api_key),
        )))
    }

    fn validate_config(&self, config: &HashMap<String, String>) -> AppResult<()> {
        if !config.contains_key("api_key") {
            return Err(AppError::Config("api_key is required".to_string()));
        }
        Ok(())
    }

    fn catalog_provider_id(&self) -> Option<&str> {
        Some("huggingface")
    }
}

/// Factory for Zhipu AI providers
pub struct ZhipuProviderFactory;

impl ProviderFactory for ZhipuProviderFactory {
    fn provider_type(&self) -> &str {
        "zhipu"
    }

    fn display_name(&self) -> &str {
        "Zhipu AI"
    }

    fn category(&self) -> ProviderCategory {
        ProviderCategory::FirstParty
    }

    fn description(&self) -> &str {
        "Zhipu AI GLM models for Chinese-language focused inference"
    }

    fn default_free_tier(&self) -> FreeTierKind {
        FreeTierKind::RateLimitedFree {
            max_rpm: 0,
            max_rpd: 0,
            max_tpm: 0,
            max_tpd: 0,
            max_monthly_calls: 0,
            max_monthly_tokens: 0,
        }
    }

    fn free_tier_notes(&self) -> Option<&str> {
        Some("Free tier limits are undocumented. Supports GLM-4.7-Flash, GLM-4.5-Flash, GLM-4.6V-Flash. Chinese-language focused provider.")
    }

    fn setup_parameters(&self) -> Vec<SetupParameter> {
        vec![SetupParameter::required(
            "api_key",
            ParameterType::ApiKey,
            "Zhipu API key",
            true,
        )]
    }

    fn create(
        &self,
        _instance_name: String,
        config: HashMap<String, String>,
    ) -> AppResult<Arc<dyn ModelProvider>> {
        self.validate_config(&config)?;

        let api_key = config
            .get("api_key")
            .ok_or_else(|| AppError::Config("api_key is required".to_string()))?
            .clone();

        Ok(Arc::new(OpenAICompatibleProvider::new(
            "zhipu".to_string(),
            "https://open.bigmodel.cn/api/paas/v4".to_string(),
            Some(api_key),
        )))
    }

    fn validate_config(&self, config: &HashMap<String, String>) -> AppResult<()> {
        if !config.contains_key("api_key") {
            return Err(AppError::Config("api_key is required".to_string()));
        }
        Ok(())
    }

    fn catalog_provider_id(&self) -> Option<&str> {
        None
    }
}

// ==================== SUBSCRIPTION PROVIDER FACTORIES ====================

/// Factory for GitHub Copilot (OAuth subscription)
pub struct GitHubCopilotProviderFactory;

impl ProviderFactory for GitHubCopilotProviderFactory {
    fn provider_type(&self) -> &str {
        "github-copilot"
    }

    fn display_name(&self) -> &str {
        "GitHub Copilot"
    }

    fn category(&self) -> ProviderCategory {
        ProviderCategory::Subscription
    }

    fn description(&self) -> &str {
        "Use your GitHub Copilot subscription for AI completions"
    }

    fn default_free_tier(&self) -> FreeTierKind {
        FreeTierKind::Subscription
    }

    fn setup_parameters(&self) -> Vec<SetupParameter> {
        vec![SetupParameter {
            key: "oauth".to_string(),
            param_type: ParameterType::OAuth,
            required: true,
            description: "Authenticate with your GitHub account".to_string(),
            default_value: None,
            sensitive: false,
        }]
    }

    fn create(
        &self,
        _instance_name: String,
        _config: HashMap<String, String>,
    ) -> AppResult<Arc<dyn ModelProvider>> {
        // GitHub Copilot uses a custom API, create OpenAI-compatible provider
        // with the OAuth token from keychain
        use lr_api_keys::{keychain_trait::KeychainStorage, CachedKeychain};

        let keychain = CachedKeychain::auto().unwrap_or_else(|_| CachedKeychain::system());
        let access_token = keychain
            .get("LocalRouter-ProviderTokens", "github-copilot_access_token")?
            .ok_or_else(|| {
                AppError::Config(
                    "No GitHub Copilot OAuth credentials found. Please authenticate first."
                        .to_string(),
                )
            })?;

        // GitHub Copilot uses a custom endpoint
        Ok(Arc::new(OpenAICompatibleProvider::new(
            "github-copilot".to_string(),
            "https://api.githubcopilot.com".to_string(),
            Some(access_token),
        )))
    }

    fn validate_config(&self, _config: &HashMap<String, String>) -> AppResult<()> {
        // OAuth validation is handled by the OAuth flow
        Ok(())
    }
}

/// Factory for OpenAI ChatGPT Plus (OAuth subscription)
pub struct OpenAICodexProviderFactory;

impl ProviderFactory for OpenAICodexProviderFactory {
    fn provider_type(&self) -> &str {
        "openai-chatgpt-plus"
    }

    fn display_name(&self) -> &str {
        "ChatGPT Plus"
    }

    fn category(&self) -> ProviderCategory {
        ProviderCategory::Subscription
    }

    fn description(&self) -> &str {
        "Use your ChatGPT Plus subscription for OpenAI models"
    }

    fn default_free_tier(&self) -> FreeTierKind {
        FreeTierKind::Subscription
    }

    fn setup_parameters(&self) -> Vec<SetupParameter> {
        vec![SetupParameter {
            key: "oauth".to_string(),
            param_type: ParameterType::OAuth,
            required: true,
            description: "Authenticate with your OpenAI account".to_string(),
            default_value: None,
            sensitive: false,
        }]
    }

    fn create(
        &self,
        _instance_name: String,
        _config: HashMap<String, String>,
    ) -> AppResult<Arc<dyn ModelProvider>> {
        // Use OAuth-first provider creation
        OpenAIProvider::from_oauth_or_key(None).map(|p| Arc::new(p) as Arc<dyn ModelProvider>)
    }

    fn validate_config(&self, _config: &HashMap<String, String>) -> AppResult<()> {
        // OAuth validation is handled by the OAuth flow
        Ok(())
    }
}

// ==================== LOCAL PROVIDER DISCOVERY ====================

/// Discovered local provider information
#[derive(Debug, Clone, Serialize)]
pub struct DiscoveredProvider {
    /// Provider type identifier
    pub provider_type: String,
    /// Display name for the provider instance
    pub instance_name: String,
    /// Base URL where the provider was found
    pub base_url: String,
}

/// Discover available local LLM providers
///
/// Checks if local providers are running at their default endpoints.
/// Returns a list of discovered providers that can be auto-configured.
pub async fn discover_local_providers() -> Vec<DiscoveredProvider> {
    let mut discovered = Vec::new();

    let discoverable: Vec<Box<dyn DiscoverableProvider>> = vec![
        Box::new(OllamaProviderFactory),
        Box::new(LMStudioProviderFactory),
        Box::new(JanProviderFactory),
        Box::new(GPT4AllProviderFactory),
        // LocalAI and llama.cpp excluded: both use port 8080 which is too common
        // to reliably identify as a local LLM provider
    ];

    for factory in &discoverable {
        if factory.is_available().await {
            tracing::info!("Discovered local {} instance", factory.display_name());
            discovered.push(DiscoveredProvider {
                provider_type: factory.provider_type().to_string(),
                instance_name: factory.default_instance_name().to_string(),
                base_url: factory.default_base_url().to_string(),
            });
        }
    }

    discovered
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_setup_parameter_required() {
        let param =
            SetupParameter::required("api_key", ParameterType::ApiKey, "OpenAI API key", true);

        assert_eq!(param.key, "api_key");
        assert_eq!(param.param_type, ParameterType::ApiKey);
        assert!(param.required);
        assert!(param.sensitive);
        assert!(param.default_value.is_none());
    }

    #[test]
    fn test_setup_parameter_optional() {
        let param = SetupParameter::optional(
            "base_url",
            ParameterType::BaseUrl,
            "API endpoint",
            Some("http://localhost:11434"),
            false,
        );

        assert_eq!(param.key, "base_url");
        assert_eq!(param.param_type, ParameterType::BaseUrl);
        assert!(!param.required);
        assert!(!param.sensitive);
        assert_eq!(
            param.default_value,
            Some("http://localhost:11434".to_string())
        );
    }

    #[test]
    fn test_parameter_type_serialization() {
        let json = serde_json::to_string(&ParameterType::ApiKey).unwrap();
        assert_eq!(json, "\"api_key\"");

        let json = serde_json::to_string(&ParameterType::BaseUrl).unwrap();
        assert_eq!(json, "\"base_url\"");
    }

    // ==================== Updated defaults tests ====================

    #[test]
    fn test_gemini_free_tier_rpd_updated() {
        let factory = GeminiProviderFactory;
        match factory.default_free_tier() {
            FreeTierKind::RateLimitedFree {
                max_rpm,
                max_rpd,
                max_tpm,
                ..
            } => {
                assert_eq!(max_rpm, 10);
                assert_eq!(max_rpd, 20); // Was 250, now 20 for conservative Pro-tier limit
                assert_eq!(max_tpm, 250_000);
            }
            other => panic!("Expected RateLimitedFree, got {:?}", other),
        }
    }

    #[test]
    fn test_gemini_has_free_tier_notes() {
        let factory = GeminiProviderFactory;
        let notes = factory.free_tier_notes();
        assert!(notes.is_some());
        assert!(notes.unwrap().contains("Flash models"));
        assert!(notes.unwrap().contains("Pro models"));
    }

    #[test]
    fn test_openrouter_free_tier_is_free_models_only() {
        let factory = OpenRouterProviderFactory;
        match factory.default_free_tier() {
            FreeTierKind::FreeModelsOnly {
                free_model_patterns,
                max_rpm,
            } => {
                assert_eq!(max_rpm, 20);
                assert_eq!(free_model_patterns, vec![":free".to_string()]);
            }
            other => panic!("Expected FreeModelsOnly, got {:?}", other),
        }
    }

    #[test]
    fn test_openrouter_has_free_tier_notes() {
        let factory = OpenRouterProviderFactory;
        let notes = factory.free_tier_notes();
        assert!(notes.is_some());
        assert!(notes.unwrap().contains(":free"));
        assert!(notes.unwrap().contains("BYOK"));
    }

    // ==================== Existing provider notes tests ====================

    #[test]
    fn test_groq_has_free_tier_notes() {
        let factory = GroqProviderFactory;
        let notes = factory.free_tier_notes();
        assert!(notes.is_some());
        assert!(notes.unwrap().contains("vary by model"));
    }

    #[test]
    fn test_mistral_has_free_tier_notes() {
        let factory = MistralProviderFactory;
        let notes = factory.free_tier_notes();
        assert!(notes.is_some());
        assert!(notes.unwrap().contains("experiment plan"));
    }

    #[test]
    fn test_cohere_has_free_tier_notes() {
        let factory = CohereProviderFactory;
        let notes = factory.free_tier_notes();
        assert!(notes.is_some());
        assert!(notes.unwrap().contains("Trial API keys"));
    }

    #[test]
    fn test_togetherai_has_free_tier_notes() {
        let factory = TogetherAIProviderFactory;
        let notes = factory.free_tier_notes();
        assert!(notes.is_some());
        assert!(notes.unwrap().contains("3 RPM"));
    }

    #[test]
    fn test_perplexity_has_free_tier_notes() {
        let factory = PerplexityProviderFactory;
        assert_eq!(factory.default_free_tier(), FreeTierKind::None);
        let notes = factory.free_tier_notes();
        assert!(notes.is_some());
        assert!(notes.unwrap().contains("No free API tier"));
    }

    #[test]
    fn test_deepinfra_has_free_tier_notes() {
        let factory = DeepInfraProviderFactory;
        let notes = factory.free_tier_notes();
        assert!(notes.is_some());
        assert!(notes.unwrap().contains("$5 monthly"));
    }

    #[test]
    fn test_cerebras_has_free_tier_notes() {
        let factory = CerebrasProviderFactory;
        let notes = factory.free_tier_notes();
        assert!(notes.is_some());
        assert!(notes.unwrap().contains("Developer tier"));
    }

    #[test]
    fn test_xai_has_free_tier_notes() {
        let factory = XAIProviderFactory;
        let notes = factory.free_tier_notes();
        assert!(notes.is_some());
        assert!(notes.unwrap().contains("$25 one-time"));
    }

    #[test]
    fn test_local_providers_have_no_notes() {
        assert!(OllamaProviderFactory.free_tier_notes().is_none());
        assert!(LMStudioProviderFactory.free_tier_notes().is_none());
        assert!(JanProviderFactory.free_tier_notes().is_none());
        assert!(GPT4AllProviderFactory.free_tier_notes().is_none());
        assert!(LocalAIProviderFactory.free_tier_notes().is_none());
        assert!(LlamaCppProviderFactory.free_tier_notes().is_none());
    }

    // ==================== New provider factory tests ====================

    // --- GitHub Models ---

    #[test]
    fn test_github_models_factory_metadata() {
        let factory = GitHubModelsProviderFactory;
        assert_eq!(factory.provider_type(), "github_models");
        assert_eq!(factory.display_name(), "GitHub Models");
        assert_eq!(factory.category(), ProviderCategory::ThirdParty);
        assert_eq!(factory.catalog_provider_id(), Some("github-models"));
    }

    #[test]
    fn test_github_models_free_tier() {
        let factory = GitHubModelsProviderFactory;
        match factory.default_free_tier() {
            FreeTierKind::RateLimitedFree {
                max_rpm, max_rpd, ..
            } => {
                assert_eq!(max_rpm, 10);
                assert_eq!(max_rpd, 50);
            }
            other => panic!("Expected RateLimitedFree, got {:?}", other),
        }
        assert!(factory.free_tier_notes().is_some());
        assert!(factory
            .free_tier_notes()
            .unwrap()
            .contains("GitHub Personal Access Token"));
    }

    #[test]
    fn test_github_models_create_success() {
        let factory = GitHubModelsProviderFactory;
        let mut config = HashMap::new();
        config.insert("api_key".to_string(), "ghp_test123".to_string());
        let provider = factory.create("test".to_string(), config);
        assert!(provider.is_ok());
        assert_eq!(provider.unwrap().name(), "github_models");
    }

    #[test]
    fn test_github_models_validate_missing_key() {
        let factory = GitHubModelsProviderFactory;
        let config = HashMap::new();
        assert!(factory.validate_config(&config).is_err());
    }

    #[test]
    fn test_github_models_setup_params() {
        let factory = GitHubModelsProviderFactory;
        let params = factory.setup_parameters();
        assert_eq!(params.len(), 1);
        assert_eq!(params[0].key, "api_key");
        assert!(params[0].required);
        assert!(params[0].sensitive);
    }

    // --- NVIDIA NIM ---

    #[test]
    fn test_nvidia_nim_factory_metadata() {
        let factory = NvidiaNimProviderFactory;
        assert_eq!(factory.provider_type(), "nvidia_nim");
        assert_eq!(factory.display_name(), "NVIDIA NIM");
        assert_eq!(factory.category(), ProviderCategory::ThirdParty);
        assert_eq!(factory.catalog_provider_id(), Some("nvidia"));
    }

    #[test]
    fn test_nvidia_nim_free_tier() {
        let factory = NvidiaNimProviderFactory;
        match factory.default_free_tier() {
            FreeTierKind::RateLimitedFree { max_rpm, .. } => {
                assert_eq!(max_rpm, 40);
            }
            other => panic!("Expected RateLimitedFree, got {:?}", other),
        }
        assert!(factory.free_tier_notes().unwrap().contains("40 RPM"));
    }

    #[test]
    fn test_nvidia_nim_create_success() {
        let factory = NvidiaNimProviderFactory;
        let mut config = HashMap::new();
        config.insert("api_key".to_string(), "nvapi-test123".to_string());
        let provider = factory.create("test".to_string(), config);
        assert!(provider.is_ok());
        assert_eq!(provider.unwrap().name(), "nvidia_nim");
    }

    #[test]
    fn test_nvidia_nim_validate_missing_key() {
        let factory = NvidiaNimProviderFactory;
        let config = HashMap::new();
        assert!(factory.validate_config(&config).is_err());
    }

    // --- Cloudflare Workers AI ---

    #[test]
    fn test_cloudflare_ai_factory_metadata() {
        let factory = CloudflareAIProviderFactory;
        assert_eq!(factory.provider_type(), "cloudflare_ai");
        assert_eq!(factory.display_name(), "Cloudflare Workers AI");
        assert_eq!(factory.category(), ProviderCategory::ThirdParty);
        assert_eq!(factory.catalog_provider_id(), Some("cloudflare-workers-ai"));
    }

    #[test]
    fn test_cloudflare_ai_free_tier() {
        let factory = CloudflareAIProviderFactory;
        // Cloudflare uses neurons not RPM/RPD, so all limits are 0
        match factory.default_free_tier() {
            FreeTierKind::RateLimitedFree {
                max_rpm,
                max_rpd,
                max_tpm,
                max_tpd,
                ..
            } => {
                assert_eq!(max_rpm, 0);
                assert_eq!(max_rpd, 0);
                assert_eq!(max_tpm, 0);
                assert_eq!(max_tpd, 0);
            }
            other => panic!("Expected RateLimitedFree, got {:?}", other),
        }
        assert!(factory.free_tier_notes().unwrap().contains("neurons"));
    }

    #[test]
    fn test_cloudflare_ai_requires_both_params() {
        let factory = CloudflareAIProviderFactory;
        let params = factory.setup_parameters();
        assert_eq!(params.len(), 2);
        assert!(params.iter().all(|p| p.required));

        // Missing both
        let config = HashMap::new();
        assert!(factory.validate_config(&config).is_err());

        // Missing base_url
        let mut config = HashMap::new();
        config.insert("api_key".to_string(), "test".to_string());
        assert!(factory.validate_config(&config).is_err());

        // Missing api_key
        let mut config = HashMap::new();
        config.insert(
            "base_url".to_string(),
            "https://api.cloudflare.com/client/v4/accounts/123/ai/v1".to_string(),
        );
        assert!(factory.validate_config(&config).is_err());
    }

    #[test]
    fn test_cloudflare_ai_rejects_http_base_url() {
        let factory = CloudflareAIProviderFactory;
        let mut config = HashMap::new();
        config.insert("api_key".to_string(), "test".to_string());
        config.insert(
            "base_url".to_string(),
            "http://insecure.example.com".to_string(),
        );
        assert!(factory.validate_config(&config).is_err());
    }

    #[test]
    fn test_cloudflare_ai_create_success() {
        let factory = CloudflareAIProviderFactory;
        let mut config = HashMap::new();
        config.insert("api_key".to_string(), "cf-test123".to_string());
        config.insert(
            "base_url".to_string(),
            "https://api.cloudflare.com/client/v4/accounts/abc123/ai/v1".to_string(),
        );
        let provider = factory.create("test".to_string(), config);
        assert!(provider.is_ok());
        assert_eq!(provider.unwrap().name(), "cloudflare_ai");
    }

    // --- LLM7.io ---

    #[test]
    fn test_llm7_factory_metadata() {
        let factory = Llm7ProviderFactory;
        assert_eq!(factory.provider_type(), "llm7");
        assert_eq!(factory.display_name(), "LLM7.io");
        assert_eq!(factory.category(), ProviderCategory::ThirdParty);
        assert!(factory.catalog_provider_id().is_none());
    }

    #[test]
    fn test_llm7_free_tier() {
        let factory = Llm7ProviderFactory;
        match factory.default_free_tier() {
            FreeTierKind::RateLimitedFree { max_rpm, .. } => {
                assert_eq!(max_rpm, 30);
            }
            other => panic!("Expected RateLimitedFree, got {:?}", other),
        }
        assert!(factory
            .free_tier_notes()
            .unwrap()
            .contains("120 RPM with token"));
    }

    #[test]
    fn test_llm7_api_key_is_optional() {
        let factory = Llm7ProviderFactory;
        let params = factory.setup_parameters();
        assert_eq!(params.len(), 1);
        assert!(!params[0].required); // API key is optional
        assert!(params[0].sensitive);

        // Should create successfully without API key
        let config = HashMap::new();
        assert!(factory.validate_config(&config).is_ok());
        let provider = factory.create("test".to_string(), config);
        assert!(provider.is_ok());
        assert_eq!(provider.unwrap().name(), "llm7");
    }

    #[test]
    fn test_llm7_create_with_optional_key() {
        let factory = Llm7ProviderFactory;
        let mut config = HashMap::new();
        config.insert("api_key".to_string(), "llm7-token".to_string());
        let provider = factory.create("test".to_string(), config);
        assert!(provider.is_ok());
    }

    // --- Kluster AI ---

    #[test]
    fn test_kluster_ai_factory_metadata() {
        let factory = KlusterAIProviderFactory;
        assert_eq!(factory.provider_type(), "kluster_ai");
        assert_eq!(factory.display_name(), "Kluster AI");
        assert_eq!(factory.category(), ProviderCategory::ThirdParty);
        assert!(factory.catalog_provider_id().is_none());
    }

    #[test]
    fn test_kluster_ai_free_tier() {
        let factory = KlusterAIProviderFactory;
        match factory.default_free_tier() {
            FreeTierKind::RateLimitedFree { max_rpm, .. } => {
                assert_eq!(max_rpm, 30);
            }
            other => panic!("Expected RateLimitedFree, got {:?}", other),
        }
        assert!(factory.free_tier_notes().unwrap().contains("undocumented"));
    }

    #[test]
    fn test_kluster_ai_create_success() {
        let factory = KlusterAIProviderFactory;
        let mut config = HashMap::new();
        config.insert("api_key".to_string(), "kluster-test".to_string());
        let provider = factory.create("test".to_string(), config);
        assert!(provider.is_ok());
        assert_eq!(provider.unwrap().name(), "kluster_ai");
    }

    #[test]
    fn test_kluster_ai_validate_missing_key() {
        let factory = KlusterAIProviderFactory;
        let config = HashMap::new();
        assert!(factory.validate_config(&config).is_err());
    }

    // --- Hugging Face ---

    #[test]
    fn test_huggingface_factory_metadata() {
        let factory = HuggingFaceProviderFactory;
        assert_eq!(factory.provider_type(), "huggingface");
        assert_eq!(factory.display_name(), "Hugging Face");
        assert_eq!(factory.category(), ProviderCategory::ThirdParty);
        assert_eq!(factory.catalog_provider_id(), Some("huggingface"));
    }

    #[test]
    fn test_huggingface_free_tier() {
        let factory = HuggingFaceProviderFactory;
        match factory.default_free_tier() {
            FreeTierKind::CreditBased {
                budget_usd,
                reset_period,
                ..
            } => {
                assert!((budget_usd - 0.10).abs() < f64::EPSILON);
                assert_eq!(reset_period, lr_config::FreeTierResetPeriod::Monthly);
            }
            other => panic!("Expected CreditBased, got {:?}", other),
        }
        assert!(factory.free_tier_notes().unwrap().contains("$0.10/month"));
        assert!(factory.free_tier_notes().unwrap().contains("PRO users"));
    }

    #[test]
    fn test_huggingface_create_success() {
        let factory = HuggingFaceProviderFactory;
        let mut config = HashMap::new();
        config.insert("api_key".to_string(), "hf_test123".to_string());
        let provider = factory.create("test".to_string(), config);
        assert!(provider.is_ok());
        assert_eq!(provider.unwrap().name(), "huggingface");
    }

    #[test]
    fn test_huggingface_validate_missing_key() {
        let factory = HuggingFaceProviderFactory;
        let config = HashMap::new();
        assert!(factory.validate_config(&config).is_err());
    }

    // --- Zhipu AI ---

    #[test]
    fn test_zhipu_factory_metadata() {
        let factory = ZhipuProviderFactory;
        assert_eq!(factory.provider_type(), "zhipu");
        assert_eq!(factory.display_name(), "Zhipu AI");
        assert_eq!(factory.category(), ProviderCategory::FirstParty);
        assert!(factory.catalog_provider_id().is_none());
    }

    #[test]
    fn test_zhipu_free_tier() {
        let factory = ZhipuProviderFactory;
        // Undocumented limits, all zero
        match factory.default_free_tier() {
            FreeTierKind::RateLimitedFree {
                max_rpm,
                max_rpd,
                max_tpm,
                max_tpd,
                ..
            } => {
                assert_eq!(max_rpm, 0);
                assert_eq!(max_rpd, 0);
                assert_eq!(max_tpm, 0);
                assert_eq!(max_tpd, 0);
            }
            other => panic!("Expected RateLimitedFree, got {:?}", other),
        }
        assert!(factory.free_tier_notes().unwrap().contains("GLM"));
        assert!(factory.free_tier_notes().unwrap().contains("Chinese"));
    }

    #[test]
    fn test_zhipu_create_success() {
        let factory = ZhipuProviderFactory;
        let mut config = HashMap::new();
        config.insert("api_key".to_string(), "zhipu-test123".to_string());
        let provider = factory.create("test".to_string(), config);
        assert!(provider.is_ok());
        assert_eq!(provider.unwrap().name(), "zhipu");
    }

    #[test]
    fn test_zhipu_validate_missing_key() {
        let factory = ZhipuProviderFactory;
        let config = HashMap::new();
        assert!(factory.validate_config(&config).is_err());
    }

    // ==================== Cross-cutting tests ====================

    #[test]
    fn test_all_new_providers_have_notes() {
        let factories: Vec<Box<dyn ProviderFactory>> = vec![
            Box::new(GitHubModelsProviderFactory),
            Box::new(NvidiaNimProviderFactory),
            Box::new(CloudflareAIProviderFactory),
            Box::new(Llm7ProviderFactory),
            Box::new(KlusterAIProviderFactory),
            Box::new(HuggingFaceProviderFactory),
            Box::new(ZhipuProviderFactory),
        ];
        for factory in &factories {
            assert!(
                factory.free_tier_notes().is_some(),
                "{} should have free tier notes",
                factory.provider_type()
            );
            assert!(
                !factory.free_tier_notes().unwrap().is_empty(),
                "{} notes should not be empty",
                factory.provider_type()
            );
        }
    }

    #[test]
    fn test_new_providers_catalog_mappings() {
        // Providers with catalog data should have correct catalog_provider_id
        assert_eq!(
            GitHubModelsProviderFactory.catalog_provider_id(),
            Some("github-models")
        );
        assert_eq!(
            NvidiaNimProviderFactory.catalog_provider_id(),
            Some("nvidia")
        );
        assert_eq!(
            CloudflareAIProviderFactory.catalog_provider_id(),
            Some("cloudflare-workers-ai")
        );
        assert_eq!(
            HuggingFaceProviderFactory.catalog_provider_id(),
            Some("huggingface")
        );

        // Providers without catalog data should return None
        let no_catalog: Vec<Box<dyn ProviderFactory>> = vec![
            Box::new(Llm7ProviderFactory),
            Box::new(KlusterAIProviderFactory),
            Box::new(ZhipuProviderFactory),
        ];
        for factory in &no_catalog {
            assert!(
                factory.catalog_provider_id().is_none(),
                "{} should have no catalog mapping",
                factory.provider_type()
            );
        }
    }

    #[test]
    fn test_new_provider_type_strings_are_unique() {
        let factories: Vec<Box<dyn ProviderFactory>> = vec![
            Box::new(GitHubModelsProviderFactory),
            Box::new(NvidiaNimProviderFactory),
            Box::new(CloudflareAIProviderFactory),
            Box::new(Llm7ProviderFactory),
            Box::new(KlusterAIProviderFactory),
            Box::new(HuggingFaceProviderFactory),
            Box::new(ZhipuProviderFactory),
        ];
        let types: Vec<&str> = factories.iter().map(|f| f.provider_type()).collect();
        let mut deduped = types.clone();
        deduped.sort();
        deduped.dedup();
        assert_eq!(
            types.len(),
            deduped.len(),
            "Provider type strings must be unique"
        );
    }

    #[test]
    fn test_new_providers_use_default_model_list_source() {
        // All new providers should use the default (ApiWithCatalogFallback)
        let factories: Vec<Box<dyn ProviderFactory>> = vec![
            Box::new(GitHubModelsProviderFactory),
            Box::new(NvidiaNimProviderFactory),
            Box::new(CloudflareAIProviderFactory),
            Box::new(Llm7ProviderFactory),
            Box::new(KlusterAIProviderFactory),
            Box::new(HuggingFaceProviderFactory),
            Box::new(ZhipuProviderFactory),
        ];
        for factory in &factories {
            assert_eq!(
                factory.model_list_source(),
                ModelListSource::ApiWithCatalogFallback,
                "{} should use default model list source",
                factory.provider_type()
            );
        }
    }
}
