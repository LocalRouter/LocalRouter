//! JSON Mode feature adapter
//!
//! This adapter ensures model responses are valid JSON without strict schema validation.
//! It's lighter-weight than structured outputs and widely supported across providers.
//!
//! Supported providers:
//! - OpenAI: Native support via response_format: { type: "json_object" }
//! - Anthropic: Enforced via system prompts and validation
//! - Gemini: Enforced via prompts and validation
//! - OpenRouter: Supports OpenAI's response_format
//!
//! Example usage:
//! ```json
//! {
//!   "model": "gpt-4",
//!   "messages": [...],
//!   "extensions": {
//!     "json_mode": {
//!       "enabled": true
//!     }
//!   }
//! }
//! ```
//!
//! Difference from Structured Outputs:
//! - JSON Mode: Ensures valid JSON syntax (lightweight)
//! - Structured Outputs: Ensures JSON matches a specific schema (strict)

use serde_json::{json, Value};

use super::{FeatureAdapter, FeatureData, FeatureParams};
use crate::{CompletionRequest, CompletionResponse};
use lr_types::{AppError, AppResult};

/// Feature adapter for JSON mode
pub struct JsonModeAdapter;

impl JsonModeAdapter {
    /// Check if JSON mode is enabled
    fn is_enabled(params: &FeatureParams) -> bool {
        params
            .get("enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(true)
    }

    /// Validate that content is valid JSON
    fn validate_json(content: &str) -> AppResult<Value> {
        serde_json::from_str(content).map_err(|e| {
            AppError::Provider(format!(
                "Response is not valid JSON: {}. Content: {}",
                e,
                content.chars().take(200).collect::<String>()
            ))
        })
    }

    /// Create JSON mode instruction for system prompt
    fn create_json_instruction() -> String {
        "You must respond with valid JSON only. \
         Do not include any text before or after the JSON. \
         Your entire response should be parseable as JSON."
            .to_string()
    }

    /// Detect provider from model name
    fn detect_provider(model: &str) -> &str {
        if model.starts_with("gpt-") || model.starts_with("o1-") {
            "openai"
        } else if model.starts_with("claude-") {
            "anthropic"
        } else if model.starts_with("gemini-") {
            "gemini"
        } else if model.contains("/") {
            "openrouter"
        } else {
            "unknown"
        }
    }

    /// Check if provider supports JSON mode
    fn supports_json_mode(model: &str) -> bool {
        let provider = Self::detect_provider(model);
        matches!(provider, "openai" | "anthropic" | "gemini" | "openrouter")
    }
}

impl FeatureAdapter for JsonModeAdapter {
    fn feature_name(&self) -> &str {
        "json_mode"
    }

    fn validate_params(&self, _params: &FeatureParams) -> AppResult<()> {
        // JSON mode has no complex parameters to validate
        Ok(())
    }

    fn adapt_request(
        &self,
        request: &mut CompletionRequest,
        params: &FeatureParams,
    ) -> AppResult<()> {
        if !Self::is_enabled(params) {
            return Ok(());
        }

        if !Self::supports_json_mode(&request.model) {
            return Err(AppError::Config(format!(
                "JSON mode not supported for model: {}",
                request.model
            )));
        }

        let provider = Self::detect_provider(&request.model);
        let mut extensions = request.extensions.clone().unwrap_or_default();

        match provider {
            "openai" | "openrouter" => {
                // For OpenAI/OpenRouter, set response_format to json_object
                extensions.insert(
                    "response_format".to_string(),
                    json!({ "type": "json_object" }),
                );
            }
            "anthropic" | "gemini" => {
                // For Anthropic/Gemini, add JSON instruction to system message
                let instruction = Self::create_json_instruction();
                let system_message = crate::ChatMessage {
                    role: "system".to_string(),
                    content: crate::ChatMessageContent::Text(instruction),
                    tool_calls: None,
                    tool_call_id: None,
                    name: None,
                };
                request.messages.insert(0, system_message);
            }
            _ => {
                return Err(AppError::Config(format!(
                    "JSON mode not supported for provider: {}",
                    provider
                )));
            }
        }

        // Mark that JSON validation is needed
        extensions.insert("_json_mode_validation".to_string(), json!(true));

        request.extensions = Some(extensions);

        Ok(())
    }

    fn adapt_response(&self, response: &mut CompletionResponse) -> AppResult<Option<FeatureData>> {
        // Check if JSON validation is needed
        let needs_validation = response
            .extensions
            .as_ref()
            .and_then(|ext| ext.get("_json_mode_validation"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        if !needs_validation {
            return Ok(None);
        }

        // Validate each choice contains valid JSON
        for (idx, choice) in response.choices.iter().enumerate() {
            let content_text = choice.message.content.as_text();
            let parsed = Self::validate_json(&content_text).map_err(|e| {
                AppError::Provider(format!("Choice {} failed JSON validation: {}", idx, e))
            })?;

            // Count JSON properties if it's an object
            let _property_count = if parsed.is_object() {
                parsed.as_object().map(|obj| obj.len()).unwrap_or(0)
            } else {
                0
            };
        }

        // Return validation success metadata
        Ok(Some(FeatureData::new(
            "json_mode",
            json!({
                "validated": true,
                "choices_validated": response.choices.len(),
            }),
        )))
    }

    fn cost_multiplier(&self) -> f64 {
        1.0 // No extra cost for JSON mode
    }

    fn help_text(&self) -> &str {
        "Ensure model responses are valid JSON without strict schema validation. \
         Lighter-weight than structured outputs. \
         Supported by OpenAI, Anthropic, Gemini, and OpenRouter."
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_feature_name() {
        let adapter = JsonModeAdapter;
        assert_eq!(adapter.feature_name(), "json_mode");
    }

    #[test]
    fn test_validate_params() {
        let adapter = JsonModeAdapter;
        let params = HashMap::new();
        assert!(adapter.validate_params(&params).is_ok());
    }

    #[test]
    fn test_detect_provider() {
        assert_eq!(JsonModeAdapter::detect_provider("gpt-4"), "openai");
        assert_eq!(
            JsonModeAdapter::detect_provider("claude-3-opus"),
            "anthropic"
        );
        assert_eq!(JsonModeAdapter::detect_provider("gemini-pro"), "gemini");
        assert_eq!(
            JsonModeAdapter::detect_provider("openai/gpt-4"),
            "openrouter"
        );
        assert_eq!(JsonModeAdapter::detect_provider("llama-3"), "unknown");
    }

    #[test]
    fn test_supports_json_mode() {
        assert!(JsonModeAdapter::supports_json_mode("gpt-4"));
        assert!(JsonModeAdapter::supports_json_mode("claude-3-opus"));
        assert!(JsonModeAdapter::supports_json_mode("gemini-pro"));
        assert!(JsonModeAdapter::supports_json_mode("openai/gpt-4"));
        assert!(!JsonModeAdapter::supports_json_mode("llama-3"));
    }

    #[test]
    fn test_validate_json_valid() {
        let json_str = r#"{"name": "Alice", "age": 30}"#;
        let result = JsonModeAdapter::validate_json(json_str);
        assert!(result.is_ok());

        let parsed = result.unwrap();
        assert_eq!(parsed["name"], "Alice");
        assert_eq!(parsed["age"], 30);
    }

    #[test]
    fn test_validate_json_invalid() {
        let invalid = "not valid json";
        let result = JsonModeAdapter::validate_json(invalid);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not valid JSON"));
    }

    #[test]
    fn test_adapt_request_openai() {
        let adapter = JsonModeAdapter;
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

        assert!(adapter.adapt_request(&mut request, &params).is_ok());

        // Check response_format was added
        assert!(request.extensions.is_some());
        let extensions = request.extensions.unwrap();
        assert_eq!(
            extensions.get("response_format").unwrap(),
            &json!({"type": "json_object"})
        );
        assert_eq!(
            extensions.get("_json_mode_validation").unwrap(),
            &json!(true)
        );
    }

    #[test]
    fn test_adapt_request_anthropic() {
        let adapter = JsonModeAdapter;
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
        assert!(adapter.adapt_request(&mut request, &params).is_ok());

        // Check system message was added
        assert_eq!(request.messages.len(), 1);
        assert_eq!(request.messages[0].role, "system");
        assert!(request.messages[0].content.as_str().contains("valid JSON"));

        // Check validation flag was set
        assert!(request.extensions.is_some());
        let extensions = request.extensions.unwrap();
        assert_eq!(
            extensions.get("_json_mode_validation").unwrap(),
            &json!(true)
        );
    }

    #[test]
    fn test_adapt_request_disabled() {
        let adapter = JsonModeAdapter;
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
        params.insert("enabled".to_string(), json!(false));

        assert!(adapter.adapt_request(&mut request, &params).is_ok());

        // Should not modify request
        assert!(request.extensions.is_none());
    }

    #[test]
    fn test_adapt_response_valid_json() {
        let adapter = JsonModeAdapter;

        let mut extensions = HashMap::new();
        extensions.insert("_json_mode_validation".to_string(), json!(true));

        let mut response = CompletionResponse {
            id: "test-id".to_string(),
            object: "chat.completion".to_string(),
            created: 1234567890,
            model: "gpt-4".to_string(),
            provider: "openai".to_string(),
            choices: vec![crate::CompletionChoice {
                index: 0,
                message: crate::ChatMessage {
                    role: "assistant".to_string(),
                    content: crate::ChatMessageContent::Text(
                        r#"{"result": "success", "count": 42}"#.to_string(),
                    ),
                    tool_calls: None,
                    tool_call_id: None,
                    name: None,
                },
                finish_reason: Some("stop".to_string()),
                logprobs: None,
            }],
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
        assert_eq!(data.feature, "json_mode");
        assert_eq!(data.data["validated"], true);
        assert_eq!(data.data["choices_validated"], 1);
    }

    #[test]
    fn test_adapt_response_invalid_json() {
        let adapter = JsonModeAdapter;

        let mut extensions = HashMap::new();
        extensions.insert("_json_mode_validation".to_string(), json!(true));

        let mut response = CompletionResponse {
            id: "test-id".to_string(),
            object: "chat.completion".to_string(),
            created: 1234567890,
            model: "gpt-4".to_string(),
            provider: "openai".to_string(),
            choices: vec![crate::CompletionChoice {
                index: 0,
                message: crate::ChatMessage {
                    role: "assistant".to_string(),
                    content: crate::ChatMessageContent::Text(
                        "This is not valid JSON".to_string(),
                    ),
                    tool_calls: None,
                    tool_call_id: None,
                    name: None,
                },
                finish_reason: Some("stop".to_string()),
                logprobs: None,
            }],
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
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("failed JSON validation"));
    }

    #[test]
    fn test_adapt_response_no_validation_needed() {
        let adapter = JsonModeAdapter;

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
}
