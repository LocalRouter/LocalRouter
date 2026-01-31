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
    deepinfra::DeepInfraProvider, gemini::GeminiProvider, groq::GroqProvider,
    lmstudio::LMStudioProvider, mistral::MistralProvider, ollama::OllamaProvider,
    openai::OpenAIProvider, openai_compatible::OpenAICompatibleProvider,
    openrouter::OpenRouterProvider, perplexity::PerplexityProvider, togetherai::TogetherAIProvider,
    xai::XAIProvider, ModelProvider,
};
use crate::utils::errors::{AppError, AppResult};

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
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(2))
            .build()
            .unwrap_or_default();

        let url = format!("{}/api/tags", self.default_base_url());
        client.get(&url).send().await.is_ok()
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

/// Factory for Google Gemini providers
pub struct GeminiProviderFactory;

impl ProviderFactory for GeminiProviderFactory {
    fn provider_type(&self) -> &str {
        "gemini"
    }

    fn display_name(&self) -> &str {
        "Google Gemini"
    }

    fn category(&self) -> ProviderCategory {
        ProviderCategory::FirstParty
    }

    fn description(&self) -> &str {
        "Google Gemini API for multimodal AI models"
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
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(2))
            .build()
            .unwrap_or_default();

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
        use crate::api_keys::{keychain_trait::KeychainStorage, CachedKeychain};

        let keychain = CachedKeychain::system();
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
#[derive(Debug, Clone)]
pub struct DiscoveredProvider {
    /// Provider type identifier
    pub provider_type: String,
    /// Display name for the provider instance
    pub instance_name: String,
    /// Base URL where the provider was found
    pub base_url: String,
}

/// Discover available local LLM providers (Ollama, LM Studio)
///
/// Checks if local providers are running at their default endpoints.
/// Returns a list of discovered providers that can be auto-configured.
pub async fn discover_local_providers() -> Vec<DiscoveredProvider> {
    let mut discovered = Vec::new();

    // Check Ollama
    let ollama_factory = OllamaProviderFactory;
    if ollama_factory.is_available().await {
        tracing::info!("Discovered local Ollama instance");
        discovered.push(DiscoveredProvider {
            provider_type: ollama_factory.provider_type().to_string(),
            instance_name: ollama_factory.default_instance_name().to_string(),
            base_url: ollama_factory.default_base_url().to_string(),
        });
    }

    // Check LM Studio
    let lmstudio_factory = LMStudioProviderFactory;
    if lmstudio_factory.is_available().await {
        tracing::info!("Discovered local LM Studio instance");
        discovered.push(DiscoveredProvider {
            provider_type: lmstudio_factory.provider_type().to_string(),
            instance_name: lmstudio_factory.default_instance_name().to_string(),
            base_url: lmstudio_factory.default_base_url().to_string(),
        });
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
}
