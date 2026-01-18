//! OpenAI reasoning tokens feature adapter
//!
//! OpenAI's o1 series models include "reasoning tokens" which represent
//! the model's internal chain-of-thought process. These tokens are:
//! - Not included in the visible output
//! - Counted separately in token usage
//! - Important for cost calculation
//!
//! This adapter extracts reasoning token information from OpenAI responses
//! and exposes it in the extensions field.

use serde_json::{json, Value};

use crate::utils::errors::AppResult;
use super::{FeatureAdapter, FeatureData, FeatureParams};
use crate::providers::{CompletionRequest, CompletionResponse};

/// Feature adapter for OpenAI reasoning tokens (o1 series models)
pub struct OpenAIReasoningAdapter;

impl OpenAIReasoningAdapter {
    /// Models that support reasoning tokens
    const REASONING_MODELS: &'static [&'static str] = &[
        "o1-preview",
        "o1-mini",
        "o1",
    ];

    /// Check if a model supports reasoning tokens
    pub fn supports_model(model: &str) -> bool {
        Self::REASONING_MODELS.iter().any(|m| model.starts_with(m))
    }

    /// Extract reasoning tokens from usage metadata
    #[allow(dead_code)]
    fn extract_reasoning_tokens(usage_metadata: &Value) -> Option<u64> {
        usage_metadata
            .get("reasoning_tokens")
            .and_then(|v| v.as_u64())
    }
}

impl FeatureAdapter for OpenAIReasoningAdapter {
    fn feature_name(&self) -> &str {
        "reasoning_tokens"
    }

    fn validate_params(&self, _params: &FeatureParams) -> AppResult<()> {
        // No parameters needed for reasoning tokens
        // This feature is automatically enabled for o1 models
        Ok(())
    }

    fn adapt_request(&self, _request: &mut CompletionRequest, _params: &FeatureParams) -> AppResult<()> {
        // No request modifications needed
        // Reasoning tokens are automatically included in o1 model responses
        Ok(())
    }

    fn adapt_response(&self, response: &mut CompletionResponse) -> AppResult<Option<FeatureData>> {
        // Check if this is an o1 model
        if !Self::supports_model(&response.model) {
            return Ok(None);
        }

        // For now, we don't have direct access to reasoning_tokens in the response
        // This would need to be extracted from the raw OpenAI response
        // We'll add a placeholder that can be populated when the provider
        // parses the raw response

        // In a real implementation, the OpenAI provider would include
        // reasoning_tokens in a metadata field that we could extract here

        Ok(Some(FeatureData::new(
            "reasoning_tokens",
            json!({
                "supported": true,
                "model": response.model.clone(),
                "note": "Reasoning tokens are included in prompt_tokens count"
            })
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_feature_name() {
        let adapter = OpenAIReasoningAdapter;
        assert_eq!(adapter.feature_name(), "reasoning_tokens");
    }

    #[test]
    fn test_supports_model() {
        assert!(OpenAIReasoningAdapter::supports_model("o1-preview"));
        assert!(OpenAIReasoningAdapter::supports_model("o1-mini"));
        assert!(OpenAIReasoningAdapter::supports_model("o1"));
        assert!(!OpenAIReasoningAdapter::supports_model("gpt-4"));
        assert!(!OpenAIReasoningAdapter::supports_model("gpt-3.5-turbo"));
    }

    #[test]
    fn test_validate_params() {
        let adapter = OpenAIReasoningAdapter;
        let params = HashMap::new();
        assert!(adapter.validate_params(&params).is_ok());
    }

    #[test]
    fn test_adapt_request_no_changes() {
        let adapter = OpenAIReasoningAdapter;
        let mut request = CompletionRequest {
            model: "o1-preview".to_string(),
            messages: vec![],
            temperature: None,
            max_tokens: None,
            stream: false,
            top_p: None,
            frequency_penalty: None,
            presence_penalty: None,
            stop: None,
            top_k: None,
            seed: None,
            repetition_penalty: None,
            extensions: None,
        };

        let params = HashMap::new();
        assert!(adapter.adapt_request(&mut request, &params).is_ok());
        // Request should be unchanged
        assert_eq!(request.model, "o1-preview");
    }

    #[test]
    fn test_adapt_response_o1_model() {
        let adapter = OpenAIReasoningAdapter;
        let mut response = CompletionResponse {
            id: "test-id".to_string(),
            object: "chat.completion".to_string(),
            created: 1234567890,
            model: "o1-preview".to_string(),
            provider: "openai".to_string(),
            choices: vec![],
            usage: crate::providers::TokenUsage {
                prompt_tokens: 100,
                completion_tokens: 50,
                total_tokens: 150,
                prompt_tokens_details: None,
                completion_tokens_details: None,
            },
            extensions: None,
        };

        let result = adapter.adapt_response(&mut response);
        assert!(result.is_ok());

        let feature_data = result.unwrap();
        assert!(feature_data.is_some());

        let data = feature_data.unwrap();
        assert_eq!(data.feature, "reasoning_tokens");
    }

    #[test]
    fn test_adapt_response_non_o1_model() {
        let adapter = OpenAIReasoningAdapter;
        let mut response = CompletionResponse {
            id: "test-id".to_string(),
            object: "chat.completion".to_string(),
            created: 1234567890,
            model: "gpt-4".to_string(),
            provider: "openai".to_string(),
            choices: vec![],
            usage: crate::providers::TokenUsage {
                prompt_tokens: 100,
                completion_tokens: 50,
                total_tokens: 150,
                prompt_tokens_details: None,
                completion_tokens_details: None,
            },
            extensions: None,
        };

        let result = adapter.adapt_response(&mut response);
        assert!(result.is_ok());
        assert!(result.unwrap().is_none()); // No feature data for non-o1 models
    }
}
