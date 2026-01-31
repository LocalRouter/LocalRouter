//! Logprobs (Log Probabilities) feature adapter
//!
//! This adapter enables extraction of log probabilities for generated tokens,
//! providing insights into model confidence and enabling advanced use cases:
//! - Token healing (selecting higher probability alternatives)
//! - Confidence scoring (detecting uncertain responses)
//! - Alternative generation paths (exploring top-k alternatives)
//!
//! Supported providers:
//! - OpenAI: Native support via logprobs parameter
//! - OpenRouter: Supports OpenAI's logprobs format
//!
//! Example usage:
//! ```json
//! {
//!   "model": "gpt-4",
//!   "messages": [...],
//!   "extensions": {
//!     "logprobs": {
//!       "enabled": true,
//!       "top_logprobs": 5
//!     }
//!   }
//! }
//! ```
//!
//! Response format:
//! ```json
//! {
//!   "extensions": {
//!     "logprobs": {
//!       "content": [
//!         {
//!           "token": "Hello",
//!           "logprob": -0.0001,
//!           "bytes": [72, 101, 108, 108, 111],
//!           "top_logprobs": [
//!             {"token": "Hello", "logprob": -0.0001},
//!             {"token": "Hi", "logprob": -2.5}
//!           ]
//!         }
//!       ]
//!     }
//!   }
//! }
//! ```

use serde_json::{json, Value};
use std::collections::HashMap;

use super::{FeatureAdapter, FeatureData, FeatureParams};
use crate::{CompletionRequest, CompletionResponse};
use lr_types::{AppError, AppResult};

/// Minimum top_logprobs value
const MIN_TOP_LOGPROBS: u32 = 0;

/// Maximum top_logprobs value (OpenAI limit)
const MAX_TOP_LOGPROBS: u32 = 20;

/// Feature adapter for logprobs extraction
pub struct LogprobsAdapter;

impl LogprobsAdapter {
    /// Get logprobs configuration from parameters
    fn get_logprobs_config(params: &FeatureParams) -> AppResult<LogprobsConfig> {
        let enabled = params
            .get("enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        let top_logprobs = params
            .get("top_logprobs")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;

        // Validate top_logprobs range
        if top_logprobs > MAX_TOP_LOGPROBS {
            return Err(AppError::Config(format!(
                "top_logprobs must be between {} and {} (got {})",
                MIN_TOP_LOGPROBS, MAX_TOP_LOGPROBS, top_logprobs
            )));
        }

        Ok(LogprobsConfig {
            enabled,
            top_logprobs,
        })
    }

    /// Extract logprobs from OpenAI response
    fn extract_openai_logprobs(extensions: &HashMap<String, Value>) -> Option<Value> {
        // OpenAI returns logprobs in choices[].logprobs.content[]
        extensions.get("logprobs").cloned()
    }

    /// Format logprobs to standardized structure
    fn format_logprobs(raw_logprobs: &Value) -> AppResult<Value> {
        // OpenAI format: { content: [ { token, logprob, bytes, top_logprobs: [...] } ] }
        // We'll keep this format as it's already well-structured

        if !raw_logprobs.is_object() {
            return Ok(json!({ "content": [] }));
        }

        // Validate structure
        if let Some(content) = raw_logprobs.get("content") {
            if !content.is_array() {
                return Err(AppError::Provider(
                    "Invalid logprobs format: content must be an array".to_string(),
                ));
            }
        }

        Ok(raw_logprobs.clone())
    }

    /// Calculate average confidence from logprobs
    fn calculate_average_confidence(logprobs: &Value) -> Option<f64> {
        let content = logprobs.get("content")?.as_array()?;

        if content.is_empty() {
            return None;
        }

        let sum: f64 = content
            .iter()
            .filter_map(|item| item.get("logprob")?.as_f64())
            .sum();

        Some(sum / content.len() as f64)
    }

    /// Detect if provider supports logprobs
    fn supports_logprobs(model: &str) -> bool {
        // OpenAI models support logprobs
        model.starts_with("gpt-") ||
        model.starts_with("o1-") ||
        // OpenRouter passes through to underlying provider
        model.contains("/")
    }
}

#[derive(Debug, Clone)]
struct LogprobsConfig {
    enabled: bool,
    top_logprobs: u32,
}

impl FeatureAdapter for LogprobsAdapter {
    fn feature_name(&self) -> &str {
        "logprobs"
    }

    fn validate_params(&self, params: &FeatureParams) -> AppResult<()> {
        Self::get_logprobs_config(params)?;
        Ok(())
    }

    fn adapt_request(
        &self,
        request: &mut CompletionRequest,
        params: &FeatureParams,
    ) -> AppResult<()> {
        if !Self::supports_logprobs(&request.model) {
            return Err(AppError::Config(format!(
                "Logprobs not supported for model: {}",
                request.model
            )));
        }

        let config = Self::get_logprobs_config(params)?;

        if !config.enabled {
            return Ok(());
        }

        // Store logprobs configuration in extensions for the provider to use
        let mut extensions = request.extensions.clone().unwrap_or_default();

        extensions.insert("_logprobs_enabled".to_string(), json!(true));

        if config.top_logprobs > 0 {
            extensions.insert(
                "_logprobs_top_count".to_string(),
                json!(config.top_logprobs),
            );
        }

        request.extensions = Some(extensions);

        Ok(())
    }

    fn adapt_response(&self, response: &mut CompletionResponse) -> AppResult<Option<FeatureData>> {
        // Check if logprobs are present in response extensions
        let extensions = match &response.extensions {
            Some(ext) => ext,
            None => return Ok(None),
        };

        let raw_logprobs = match Self::extract_openai_logprobs(extensions) {
            Some(lp) => lp,
            None => return Ok(None),
        };

        // Format logprobs
        let formatted = Self::format_logprobs(&raw_logprobs)?;

        // Calculate average confidence
        let avg_confidence = Self::calculate_average_confidence(&formatted);

        // Count tokens
        let token_count = formatted
            .get("content")
            .and_then(|c| c.as_array())
            .map(|arr| arr.len())
            .unwrap_or(0);

        // Return feature data with logprobs
        Ok(Some(FeatureData::new(
            "logprobs",
            json!({
                "logprobs": formatted,
                "token_count": token_count,
                "average_confidence": avg_confidence,
            }),
        )))
    }

    fn cost_multiplier(&self) -> f64 {
        1.0 // No extra cost for logprobs
    }

    fn help_text(&self) -> &str {
        "Enable extraction of log probabilities for generated tokens. \
         Provides insights into model confidence and enables advanced use cases \
         like token healing and confidence scoring. \
         Supported by OpenAI and OpenRouter."
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_feature_name() {
        let adapter = LogprobsAdapter;
        assert_eq!(adapter.feature_name(), "logprobs");
    }

    #[test]
    fn test_validate_params_default() {
        let adapter = LogprobsAdapter;
        let params = HashMap::new();

        // Should use defaults: enabled=true, top_logprobs=0
        assert!(adapter.validate_params(&params).is_ok());
    }

    #[test]
    fn test_validate_params_with_top_logprobs() {
        let adapter = LogprobsAdapter;
        let mut params = HashMap::new();
        params.insert("enabled".to_string(), json!(true));
        params.insert("top_logprobs".to_string(), json!(5));

        assert!(adapter.validate_params(&params).is_ok());
    }

    #[test]
    fn test_validate_params_top_logprobs_too_high() {
        let adapter = LogprobsAdapter;
        let mut params = HashMap::new();
        params.insert("top_logprobs".to_string(), json!(25));

        let result = adapter.validate_params(&params);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("must be between"));
    }

    #[test]
    fn test_supports_logprobs() {
        assert!(LogprobsAdapter::supports_logprobs("gpt-4"));
        assert!(LogprobsAdapter::supports_logprobs("gpt-3.5-turbo"));
        assert!(LogprobsAdapter::supports_logprobs("o1-preview"));
        assert!(LogprobsAdapter::supports_logprobs("openai/gpt-4")); // OpenRouter
        assert!(!LogprobsAdapter::supports_logprobs("claude-3-opus"));
        assert!(!LogprobsAdapter::supports_logprobs("gemini-pro"));
    }

    #[test]
    fn test_adapt_request_openai() {
        let adapter = LogprobsAdapter;
        let mut request = CompletionRequest {
            model: "gpt-4".to_string(),
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
            tools: None,
            tool_choice: None,
            response_format: None,
            logprobs: None,
            top_logprobs: None,
        };

        let mut params = HashMap::new();
        params.insert("enabled".to_string(), json!(true));
        params.insert("top_logprobs".to_string(), json!(5));

        assert!(adapter.adapt_request(&mut request, &params).is_ok());

        // Check extensions were added
        assert!(request.extensions.is_some());
        let extensions = request.extensions.unwrap();
        assert_eq!(extensions.get("_logprobs_enabled").unwrap(), &json!(true));
        assert_eq!(extensions.get("_logprobs_top_count").unwrap(), &json!(5));
    }

    #[test]
    fn test_adapt_request_unsupported_model() {
        let adapter = LogprobsAdapter;
        let mut request = CompletionRequest {
            model: "claude-3-opus".to_string(),
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
            tools: None,
            tool_choice: None,
            response_format: None,
            logprobs: None,
            top_logprobs: None,
        };

        let params = HashMap::new();
        let result = adapter.adapt_request(&mut request, &params);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not supported"));
    }

    #[test]
    fn test_format_logprobs() {
        let raw = json!({
            "content": [
                {
                    "token": "Hello",
                    "logprob": -0.0001,
                    "bytes": [72, 101, 108, 108, 111],
                    "top_logprobs": [
                        {"token": "Hello", "logprob": -0.0001},
                        {"token": "Hi", "logprob": -2.5}
                    ]
                },
                {
                    "token": " world",
                    "logprob": -0.002,
                    "bytes": [32, 119, 111, 114, 108, 100],
                    "top_logprobs": [
                        {"token": " world", "logprob": -0.002}
                    ]
                }
            ]
        });

        let formatted = LogprobsAdapter::format_logprobs(&raw).unwrap();
        assert_eq!(formatted, raw);
    }

    #[test]
    fn test_calculate_average_confidence() {
        let logprobs = json!({
            "content": [
                {"token": "A", "logprob": -1.0},
                {"token": "B", "logprob": -2.0},
                {"token": "C", "logprob": -3.0}
            ]
        });

        let avg = LogprobsAdapter::calculate_average_confidence(&logprobs).unwrap();
        assert!((avg - (-2.0)).abs() < 0.001);
    }

    #[test]
    fn test_calculate_average_confidence_empty() {
        let logprobs = json!({
            "content": []
        });

        let avg = LogprobsAdapter::calculate_average_confidence(&logprobs);
        assert!(avg.is_none());
    }

    #[test]
    fn test_adapt_response_with_logprobs() {
        let adapter = LogprobsAdapter;

        let mut extensions = HashMap::new();
        extensions.insert(
            "logprobs".to_string(),
            json!({
                "content": [
                    {
                        "token": "Hello",
                        "logprob": -0.5,
                        "bytes": [72, 101, 108, 108, 111],
                        "top_logprobs": [
                            {"token": "Hello", "logprob": -0.5},
                            {"token": "Hi", "logprob": -1.5}
                        ]
                    }
                ]
            }),
        );

        let mut response = CompletionResponse {
            id: "test-id".to_string(),
            object: "chat.completion".to_string(),
            created: 1234567890,
            model: "gpt-4".to_string(),
            provider: "openai".to_string(),
            choices: vec![],
            usage: crate::TokenUsage {
                prompt_tokens: 100,
                completion_tokens: 50,
                total_tokens: 150,
                prompt_tokens_details: None,
                completion_tokens_details: None,
            },
            extensions: Some(extensions),
            routellm_win_rate: None,
        };

        let result = adapter.adapt_response(&mut response);
        assert!(result.is_ok());

        let feature_data = result.unwrap();
        assert!(feature_data.is_some());

        let data = feature_data.unwrap();
        assert_eq!(data.feature, "logprobs");
        assert_eq!(data.data["token_count"], 1);
        assert_eq!(data.data["average_confidence"], -0.5);
        assert!(data.data["logprobs"]["content"].is_array());
    }

    #[test]
    fn test_adapt_response_no_logprobs() {
        let adapter = LogprobsAdapter;

        let mut response = CompletionResponse {
            id: "test-id".to_string(),
            object: "chat.completion".to_string(),
            created: 1234567890,
            model: "gpt-4".to_string(),
            provider: "openai".to_string(),
            choices: vec![],
            usage: crate::TokenUsage {
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
        assert!(result.unwrap().is_none());
    }

    #[test]
    fn test_get_logprobs_config() {
        let mut params = HashMap::new();
        params.insert("enabled".to_string(), json!(true));
        params.insert("top_logprobs".to_string(), json!(10));

        let config = LogprobsAdapter::get_logprobs_config(&params).unwrap();
        assert!(config.enabled);
        assert_eq!(config.top_logprobs, 10);
    }

    #[test]
    fn test_get_logprobs_config_defaults() {
        let params = HashMap::new();
        let config = LogprobsAdapter::get_logprobs_config(&params).unwrap();
        assert!(config.enabled);
        assert_eq!(config.top_logprobs, 0);
    }
}
