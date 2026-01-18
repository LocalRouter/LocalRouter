//! Feature adapters for provider-specific advanced capabilities
//!
//! This module provides a pattern for extending models with provider-specific
//! features without polluting the base ModelProvider trait.

pub mod anthropic_thinking;
pub mod openai_reasoning;
pub mod gemini_thinking;
pub mod structured_outputs;
pub mod prompt_caching;
pub mod logprobs;
pub mod json_mode;

use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;

use crate::utils::errors::AppResult;
use super::{CompletionRequest, CompletionResponse};

// Re-export feature adapters

/// Feature parameters passed from the API request
pub type FeatureParams = HashMap<String, Value>;

/// Feature data extracted from provider responses
#[derive(Debug, Clone)]
pub struct FeatureData {
    /// Feature identifier
    pub feature: String,
    /// Feature-specific data
    pub data: Value,
}

impl FeatureData {
    pub fn new(feature: impl Into<String>, data: impl Into<Value>) -> Self {
        Self {
            feature: feature.into(),
            data: data.into(),
        }
    }
}

/// Trait for adapting provider-specific features
///
/// Feature adapters transform requests and responses to support
/// provider-specific capabilities like:
/// - Anthropic's extended thinking
/// - OpenAI's reasoning tokens
/// - Gemini's grounding
/// - Prompt caching
/// - Structured outputs
#[async_trait]
pub trait FeatureAdapter: Send + Sync {
    /// Returns the feature name (e.g., "extended_thinking", "reasoning", "caching")
    fn feature_name(&self) -> &str;

    /// Transform a request to add feature-specific fields
    ///
    /// This method modifies the CompletionRequest to include provider-specific
    /// parameters needed for the feature.
    ///
    /// # Arguments
    /// * `request` - The completion request to modify
    /// * `params` - Feature parameters from the API request
    ///
    /// # Returns
    /// Ok(()) if transformation succeeded, error otherwise
    fn adapt_request(
        &self,
        request: &mut CompletionRequest,
        params: &FeatureParams,
    ) -> AppResult<()>;

    /// Transform a response to extract feature-specific data
    ///
    /// This method extracts feature-specific information from the provider's
    /// response and returns it as structured data.
    ///
    /// # Arguments
    /// * `response` - The completion response to extract from
    ///
    /// # Returns
    /// Feature-specific data if present, None otherwise
    fn adapt_response(
        &self,
        response: &mut CompletionResponse,
    ) -> AppResult<Option<FeatureData>> {
        // Default implementation: no response transformation
        let _ = response;
        Ok(None)
    }

    /// Validate feature parameters before processing
    ///
    /// This method checks that the provided parameters are valid for this feature.
    ///
    /// # Arguments
    /// * `params` - Feature parameters to validate
    ///
    /// # Returns
    /// Ok(()) if parameters are valid, error with details otherwise
    fn validate_params(&self, params: &FeatureParams) -> AppResult<()> {
        // Default implementation: accept all parameters
        let _ = params;
        Ok(())
    }

    /// Get the cost multiplier for using this feature (1.0 = no extra cost)
    fn cost_multiplier(&self) -> f64 {
        1.0 // Default: no extra cost
    }

    /// Get help text for this feature
    #[allow(dead_code)]
    fn help_text(&self) -> &str {
        "No documentation available"
    }
}

/// Registry of feature adapters for a provider
pub struct FeatureRegistry {
    adapters: HashMap<String, Box<dyn FeatureAdapter>>,
}

impl FeatureRegistry {
    /// Create a new empty feature registry
    pub fn new() -> Self {
        Self {
            adapters: HashMap::new(),
        }
    }

    /// Register a feature adapter
    pub fn register(&mut self, adapter: Box<dyn FeatureAdapter>) {
        let name = adapter.feature_name().to_string();
        self.adapters.insert(name, adapter);
    }

    /// Get a feature adapter by name
    pub fn get(&self, feature: &str) -> Option<&dyn FeatureAdapter> {
        self.adapters.get(feature).map(|b| b.as_ref())
    }

    /// Check if a feature is supported
    pub fn supports(&self, feature: &str) -> bool {
        self.adapters.contains_key(feature)
    }

    /// List all supported features
    pub fn features(&self) -> Vec<String> {
        self.adapters.keys().cloned().collect()
    }
}

impl Default for FeatureRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestAdapter;

    #[async_trait]
    impl FeatureAdapter for TestAdapter {
        fn feature_name(&self) -> &str {
            "test_feature"
        }

        fn adapt_request(
            &self,
            _request: &mut CompletionRequest,
            _params: &FeatureParams,
        ) -> AppResult<()> {
            Ok(())
        }
    }

    #[test]
    fn test_feature_registry() {
        let mut registry = FeatureRegistry::new();
        assert!(!registry.supports("test_feature"));

        registry.register(Box::new(TestAdapter));
        assert!(registry.supports("test_feature"));
        assert!(registry.get("test_feature").is_some());
        assert_eq!(registry.features().len(), 1);
    }

    #[test]
    fn test_feature_data() {
        let data = FeatureData::new("test", serde_json::json!({"key": "value"}));
        assert_eq!(data.feature, "test");
    }
}
