//! Google Gemini thinking level feature adapter
//!
//! Gemini 3 models support a `thinking_level` parameter that controls
//! how much the model should "think" before providing a response.
//!
//! Valid values:
//! - "automatic" (default): Let the model decide
//! - "baseline": Minimal thinking
//! - "enhanced": More deliberate thinking
//! - "maximum": Maximum thinking for complex tasks
//!
//! This adapter validates and applies the thinking_level parameter to Gemini requests.

use serde_json::json;

use super::{FeatureAdapter, FeatureData, FeatureParams};
use crate::providers::{CompletionRequest, CompletionResponse};
use crate::utils::errors::{AppError, AppResult};

/// Feature adapter for Gemini thinking_level parameter
pub struct GeminiThinkingAdapter;

impl GeminiThinkingAdapter {
    /// Valid thinking level values
    const VALID_THINKING_LEVELS: &'static [&'static str] =
        &["automatic", "baseline", "enhanced", "maximum"];

    /// Models that support thinking_level
    const SUPPORTED_MODELS: &'static [&'static str] = &["gemini-3", "gemini-2.0"];

    /// Check if a model supports thinking_level
    pub fn supports_model(model: &str) -> bool {
        Self::SUPPORTED_MODELS.iter().any(|m| model.starts_with(m))
    }

    /// Get thinking level from parameters
    fn get_thinking_level(params: &FeatureParams) -> AppResult<String> {
        params
            .get("thinking_level")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| AppError::Config("thinking_level parameter is required".to_string()))
    }

    /// Validate thinking level value
    fn validate_thinking_level(level: &str) -> AppResult<()> {
        if !Self::VALID_THINKING_LEVELS.contains(&level) {
            return Err(AppError::Config(format!(
                "Invalid thinking_level: '{}'. Must be one of: {}",
                level,
                Self::VALID_THINKING_LEVELS.join(", ")
            )));
        }
        Ok(())
    }
}

impl FeatureAdapter for GeminiThinkingAdapter {
    fn feature_name(&self) -> &str {
        "thinking_level"
    }

    fn validate_params(&self, params: &FeatureParams) -> AppResult<()> {
        let thinking_level = Self::get_thinking_level(params)?;
        Self::validate_thinking_level(&thinking_level)?;
        Ok(())
    }

    fn adapt_request(
        &self,
        request: &mut CompletionRequest,
        params: &FeatureParams,
    ) -> AppResult<()> {
        let thinking_level = Self::get_thinking_level(params)?;

        // Add thinking_level to request metadata
        // The Gemini provider will extract this and include it in the API request
        let mut metadata = request.extensions.clone().unwrap_or_default();

        metadata.insert("thinking_level".to_string(), json!(thinking_level));

        request.extensions = Some(metadata);

        Ok(())
    }

    fn adapt_response(&self, response: &mut CompletionResponse) -> AppResult<Option<FeatureData>> {
        // Gemini includes thinking process in the response metadata
        // This could be extracted and returned as feature data

        if !Self::supports_model(&response.model) {
            return Ok(None);
        }

        Ok(Some(FeatureData::new(
            "thinking_level",
            json!({
                "supported": true,
                "model": response.model.clone(),
                "note": "Thinking level applied to request"
            }),
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_feature_name() {
        let adapter = GeminiThinkingAdapter;
        assert_eq!(adapter.feature_name(), "thinking_level");
    }

    #[test]
    fn test_supports_model() {
        assert!(GeminiThinkingAdapter::supports_model("gemini-3-flash"));
        assert!(GeminiThinkingAdapter::supports_model("gemini-2.0-pro"));
        assert!(!GeminiThinkingAdapter::supports_model("gemini-1.5-pro"));
        assert!(!GeminiThinkingAdapter::supports_model("gpt-4"));
    }

    #[test]
    fn test_validate_params_valid() {
        let adapter = GeminiThinkingAdapter;
        let mut params = HashMap::new();
        params.insert("thinking_level".to_string(), json!("enhanced"));

        assert!(adapter.validate_params(&params).is_ok());
    }

    #[test]
    fn test_validate_params_missing() {
        let adapter = GeminiThinkingAdapter;
        let params = HashMap::new();

        let result = adapter.validate_params(&params);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("thinking_level parameter is required"));
    }

    #[test]
    fn test_validate_params_invalid_value() {
        let adapter = GeminiThinkingAdapter;
        let mut params = HashMap::new();
        params.insert("thinking_level".to_string(), json!("invalid"));

        let result = adapter.validate_params(&params);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Invalid thinking_level"));
    }

    #[test]
    fn test_adapt_request() {
        let adapter = GeminiThinkingAdapter;
        let mut request = CompletionRequest {
            model: "gemini-3-flash".to_string(),
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

        let mut params = HashMap::new();
        params.insert("thinking_level".to_string(), json!("enhanced"));

        assert!(adapter.adapt_request(&mut request, &params).is_ok());

        // Check that metadata was added
        assert!(request.extensions.is_some());
        let metadata = request.extensions.unwrap();
        assert_eq!(metadata.get("thinking_level").unwrap(), "enhanced");
    }

    #[test]
    fn test_adapt_response() {
        let adapter = GeminiThinkingAdapter;
        let mut response = CompletionResponse {
            id: "test-id".to_string(),
            object: "chat.completion".to_string(),
            created: 1234567890,
            model: "gemini-3-flash".to_string(),
            provider: "gemini".to_string(),
            choices: vec![],
            usage: crate::providers::TokenUsage {
                prompt_tokens: 100,
                completion_tokens: 50,
                total_tokens: 150,
                prompt_tokens_details: None,
                completion_tokens_details: None,
            },
            extensions: None,
            routellm_win_rate: None,
        };

        let result = adapter.adapt_response(&mut response);
        assert!(result.is_ok());

        let feature_data = result.unwrap();
        assert!(feature_data.is_some());

        let data = feature_data.unwrap();
        assert_eq!(data.feature, "thinking_level");
    }
}
