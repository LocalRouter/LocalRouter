//! Provider factory system for dynamic provider instantiation
//!
//! This module provides the factory pattern for creating provider instances
//! with validated configuration parameters.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use super::{
    ModelProvider,
    ollama::OllamaProvider,
    openai::OpenAIProvider,
    openai_compatible::OpenAICompatibleProvider,
    anthropic::AnthropicProvider,
    gemini::GeminiProvider,
    openrouter::OpenRouterProvider,
};
use crate::utils::errors::{AppResult, AppError};

/// Factory for creating provider instances
#[async_trait]
pub trait ProviderFactory: Send + Sync {
    /// Provider type identifier (e.g., "ollama", "openai", "anthropic")
    fn provider_type(&self) -> &str;

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
}

// ==================== FACTORY IMPLEMENTATIONS ====================

/// Factory for Ollama providers
pub struct OllamaProviderFactory;

impl ProviderFactory for OllamaProviderFactory {
    fn provider_type(&self) -> &str {
        "ollama"
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
}

/// Factory for OpenAI providers
pub struct OpenAIProviderFactory;

impl ProviderFactory for OpenAIProviderFactory {
    fn provider_type(&self) -> &str {
        "openai"
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
}

/// Factory for OpenRouter providers
pub struct OpenRouterProviderFactory;

impl ProviderFactory for OpenRouterProviderFactory {
    fn provider_type(&self) -> &str {
        "openrouter"
    }

    fn description(&self) -> &str {
        "OpenRouter multi-provider gateway for accessing multiple LLM providers"
    }

    fn setup_parameters(&self) -> Vec<SetupParameter> {
        vec![
            SetupParameter::required(
                "api_key",
                ParameterType::ApiKey,
                "OpenRouter API key",
                true,
            ),
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
}

/// Factory for generic OpenAI-compatible providers
pub struct OpenAICompatibleProviderFactory;

impl ProviderFactory for OpenAICompatibleProviderFactory {
    fn provider_type(&self) -> &str {
        "openai_compatible"
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_setup_parameter_required() {
        let param = SetupParameter::required("api_key", ParameterType::ApiKey, "OpenAI API key", true);

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
        assert_eq!(param.default_value, Some("http://localhost:11434".to_string()));
    }

    #[test]
    fn test_parameter_type_serialization() {
        let json = serde_json::to_string(&ParameterType::ApiKey).unwrap();
        assert_eq!(json, "\"api_key\"");

        let json = serde_json::to_string(&ParameterType::BaseUrl).unwrap();
        assert_eq!(json, "\"base_url\"");
    }
}
