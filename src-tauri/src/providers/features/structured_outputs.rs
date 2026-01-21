//! Structured Outputs feature adapter
//!
//! This adapter enables strict JSON schema validation for model responses.
//! Unlike basic JSON mode (which just ensures valid JSON), structured outputs
//! guarantee the response matches a specific JSON schema.
//!
//! Supported providers:
//! - OpenAI: Native support via response_format with json_schema type
//! - Anthropic: Schema enforcement via system prompts and validation
//!
//! Example usage:
//! ```json
//! {
//!   "model": "gpt-4",
//!   "messages": [...],
//!   "extensions": {
//!     "structured_outputs": {
//!       "schema": {
//!         "type": "object",
//!         "properties": {
//!           "name": {"type": "string"},
//!           "age": {"type": "number"}
//!         },
//!         "required": ["name", "age"]
//!       }
//!     }
//!   }
//! }
//! ```

use jsonschema::{Draft, JSONSchema};
use serde_json::{json, Value};

use super::{FeatureAdapter, FeatureData, FeatureParams};
use crate::providers::{CompletionRequest, CompletionResponse};
use crate::utils::errors::{AppError, AppResult};

/// Maximum schema size in bytes (1MB)
const MAX_SCHEMA_SIZE: usize = 1_048_576;

/// Feature adapter for structured outputs with JSON schema validation
pub struct StructuredOutputsAdapter;

impl StructuredOutputsAdapter {
    /// Validate that a schema is well-formed and not too large
    fn validate_schema(schema: &Value) -> AppResult<()> {
        // Check schema size
        let schema_json = serde_json::to_string(schema)
            .map_err(|e| AppError::Config(format!("Failed to serialize schema: {}", e)))?;

        if schema_json.len() > MAX_SCHEMA_SIZE {
            return Err(AppError::Config(format!(
                "Schema too large: {} bytes (max: {} bytes)",
                schema_json.len(),
                MAX_SCHEMA_SIZE
            )));
        }

        // Validate schema is a valid JSON Schema
        JSONSchema::options()
            .with_draft(Draft::Draft7)
            .compile(schema)
            .map_err(|e| AppError::Config(format!("Invalid JSON schema: {}", e)))?;

        Ok(())
    }

    /// Extract schema from feature parameters
    fn get_schema(params: &FeatureParams) -> AppResult<Value> {
        params
            .get("schema")
            .cloned()
            .ok_or_else(|| AppError::Config("schema parameter is required".to_string()))
    }

    /// Format schema for OpenAI API
    /// OpenAI expects: response_format: { type: "json_schema", json_schema: { name, schema, strict } }
    fn format_for_openai(schema: &Value) -> AppResult<Value> {
        Ok(json!({
            "type": "json_schema",
            "json_schema": {
                "name": "response_schema",
                "schema": schema,
                "strict": true
            }
        }))
    }

    /// Format schema for Anthropic API
    /// Anthropic doesn't have native structured outputs yet, so we:
    /// 1. Add schema to system prompt
    /// 2. Validate response against schema
    fn format_for_anthropic(schema: &Value) -> AppResult<String> {
        let schema_str = serde_json::to_string_pretty(schema)
            .map_err(|e| AppError::Config(format!("Failed to format schema: {}", e)))?;

        Ok(format!(
            "You must respond with valid JSON that matches this exact schema:\n\n{}\n\nIMPORTANT: Your entire response must be valid JSON matching this schema. Do not include any text before or after the JSON.",
            schema_str
        ))
    }

    /// Validate response content against schema
    fn validate_response(content: &str, schema: &Value) -> AppResult<()> {
        // Parse response as JSON
        let response_json: Value = serde_json::from_str(content).map_err(|e| {
            AppError::Provider(format!(
                "Response is not valid JSON: {}. Content: {}",
                e,
                content.chars().take(200).collect::<String>()
            ))
        })?;

        // Compile schema
        let compiled_schema = JSONSchema::options()
            .with_draft(Draft::Draft7)
            .compile(schema)
            .map_err(|e| AppError::Config(format!("Failed to compile schema: {}", e)))?;

        // Validate response against schema
        if let Err(errors) = compiled_schema.validate(&response_json) {
            let error_messages: Vec<String> = errors.map(|e| format!("- {}", e)).collect();

            return Err(AppError::Provider(format!(
                "Response does not match schema:\n{}",
                error_messages.join("\n")
            )));
        }

        Ok(())
    }

    /// Detect provider from request model
    fn detect_provider(model: &str) -> &str {
        if model.starts_with("gpt-") || model.starts_with("o1-") {
            "openai"
        } else if model.starts_with("claude-") {
            "anthropic"
        } else {
            "unknown"
        }
    }
}

impl FeatureAdapter for StructuredOutputsAdapter {
    fn feature_name(&self) -> &str {
        "structured_outputs"
    }

    fn validate_params(&self, params: &FeatureParams) -> AppResult<()> {
        let schema = Self::get_schema(params)?;
        Self::validate_schema(&schema)?;
        Ok(())
    }

    fn adapt_request(
        &self,
        request: &mut CompletionRequest,
        params: &FeatureParams,
    ) -> AppResult<()> {
        let schema = Self::get_schema(params)?;
        let provider = Self::detect_provider(&request.model);

        let mut extensions = request.extensions.clone().unwrap_or_default();

        match provider {
            "openai" => {
                // For OpenAI, add response_format to extensions
                // The OpenAI provider will extract this and use it in the API request
                let response_format = Self::format_for_openai(&schema)?;
                extensions.insert("response_format".to_string(), response_format);

                // Store schema for response validation
                extensions.insert("_structured_outputs_schema".to_string(), schema);
            }
            "anthropic" => {
                // For Anthropic, add schema to system message
                let schema_prompt = Self::format_for_anthropic(&schema)?;

                // Add to beginning of messages as a system message
                let system_message = crate::providers::ChatMessage {
                    role: "system".to_string(),
                    content: crate::providers::ChatMessageContent::Text(schema_prompt),
                    tool_calls: None,
                    tool_call_id: None,
                    name: None,
                };
                request.messages.insert(0, system_message);

                // Store schema for response validation
                extensions.insert("_structured_outputs_schema".to_string(), schema);
            }
            _ => {
                return Err(AppError::Config(format!(
                    "Structured outputs not supported for provider: {}",
                    provider
                )));
            }
        }

        request.extensions = Some(extensions);
        Ok(())
    }

    fn adapt_response(&self, response: &mut CompletionResponse) -> AppResult<Option<FeatureData>> {
        // Check if schema validation is needed
        let schema = match &response.extensions {
            Some(ext) => ext.get("_structured_outputs_schema"),
            None => return Ok(None),
        };

        let schema = match schema {
            Some(s) => s,
            None => return Ok(None),
        };

        // Validate each choice against schema
        for choice in &response.choices {
            let content_text = choice.message.content.as_text();
            Self::validate_response(&content_text, schema)?;
        }

        // Return validation success metadata
        Ok(Some(FeatureData::new(
            "structured_outputs",
            json!({
                "validated": true,
                "schema_size": serde_json::to_string(schema).unwrap_or_default().len(),
                "choices_validated": response.choices.len()
            }),
        )))
    }

    fn cost_multiplier(&self) -> f64 {
        1.0 // No extra cost for structured outputs
    }

    fn help_text(&self) -> &str {
        "Enable strict JSON schema validation for model responses. \
         Ensures the response matches a specific JSON schema structure. \
         Supported by OpenAI (native) and Anthropic (via prompting)."
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_feature_name() {
        let adapter = StructuredOutputsAdapter;
        assert_eq!(adapter.feature_name(), "structured_outputs");
    }

    #[test]
    fn test_validate_params_valid_schema() {
        let adapter = StructuredOutputsAdapter;
        let mut params = HashMap::new();
        params.insert(
            "schema".to_string(),
            json!({
                "type": "object",
                "properties": {
                    "name": {"type": "string"},
                    "age": {"type": "number"}
                },
                "required": ["name", "age"]
            }),
        );

        assert!(adapter.validate_params(&params).is_ok());
    }

    #[test]
    fn test_validate_params_missing_schema() {
        let adapter = StructuredOutputsAdapter;
        let params = HashMap::new();

        let result = adapter.validate_params(&params);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("schema parameter is required"));
    }

    #[test]
    fn test_validate_params_invalid_schema() {
        let adapter = StructuredOutputsAdapter;
        let mut params = HashMap::new();
        params.insert(
            "schema".to_string(),
            json!({
                "type": "invalid_type",
                "properties": {}
            }),
        );

        let result = adapter.validate_params(&params);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Invalid JSON schema"));
    }

    #[test]
    fn test_adapt_request_openai() {
        let adapter = StructuredOutputsAdapter;
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
        };

        let mut params = HashMap::new();
        let schema = json!({
            "type": "object",
            "properties": {
                "result": {"type": "string"}
            },
            "required": ["result"]
        });
        params.insert("schema".to_string(), schema.clone());

        assert!(adapter.adapt_request(&mut request, &params).is_ok());

        // Check that response_format was added to extensions
        assert!(request.extensions.is_some());
        let extensions = request.extensions.unwrap();
        assert!(extensions.contains_key("response_format"));
        assert!(extensions.contains_key("_structured_outputs_schema"));

        // Verify response_format structure
        let response_format = &extensions["response_format"];
        assert_eq!(response_format["type"], "json_schema");
        assert!(response_format["json_schema"]["schema"].is_object());
    }

    #[test]
    fn test_adapt_request_anthropic() {
        let adapter = StructuredOutputsAdapter;
        let mut request = CompletionRequest {
            model: "claude-3-5-sonnet-20241022".to_string(),
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
        let schema = json!({
            "type": "object",
            "properties": {
                "result": {"type": "string"}
            },
            "required": ["result"]
        });
        params.insert("schema".to_string(), schema);

        assert!(adapter.adapt_request(&mut request, &params).is_ok());

        // Check that system message was added
        assert_eq!(request.messages.len(), 1);
        assert_eq!(request.messages[0].role, "system");
        assert!(request.messages[0].content.as_str().contains("schema"));

        // Check that schema was stored in extensions
        assert!(request.extensions.is_some());
        let extensions = request.extensions.unwrap();
        assert!(extensions.contains_key("_structured_outputs_schema"));
    }

    #[test]
    fn test_validate_response_valid() {
        let content = r#"{"name": "Alice", "age": 30}"#;
        let schema = json!({
            "type": "object",
            "properties": {
                "name": {"type": "string"},
                "age": {"type": "number"}
            },
            "required": ["name", "age"]
        });

        assert!(StructuredOutputsAdapter::validate_response(content, &schema).is_ok());
    }

    #[test]
    fn test_validate_response_missing_required_field() {
        let content = r#"{"name": "Alice"}"#;
        let schema = json!({
            "type": "object",
            "properties": {
                "name": {"type": "string"},
                "age": {"type": "number"}
            },
            "required": ["name", "age"]
        });

        let result = StructuredOutputsAdapter::validate_response(content, &schema);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("does not match schema"));
    }

    #[test]
    fn test_validate_response_wrong_type() {
        let content = r#"{"name": "Alice", "age": "thirty"}"#;
        let schema = json!({
            "type": "object",
            "properties": {
                "name": {"type": "string"},
                "age": {"type": "number"}
            },
            "required": ["name", "age"]
        });

        let result = StructuredOutputsAdapter::validate_response(content, &schema);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("does not match schema"));
    }

    #[test]
    fn test_validate_response_invalid_json() {
        let content = "not valid json";
        let schema = json!({
            "type": "object",
            "properties": {
                "name": {"type": "string"}
            }
        });

        let result = StructuredOutputsAdapter::validate_response(content, &schema);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not valid JSON"));
    }

    #[test]
    fn test_adapt_response_with_validation() {
        let adapter = StructuredOutputsAdapter;
        let schema = json!({
            "type": "object",
            "properties": {
                "result": {"type": "string"}
            },
            "required": ["result"]
        });

        let mut extensions = HashMap::new();
        extensions.insert("_structured_outputs_schema".to_string(), schema);

        let mut response = CompletionResponse {
            id: "test-id".to_string(),
            object: "chat.completion".to_string(),
            created: 1234567890,
            model: "gpt-4".to_string(),
            provider: "openai".to_string(),
            choices: vec![crate::providers::CompletionChoice {
                index: 0,
                message: crate::providers::ChatMessage {
                    role: "assistant".to_string(),
                    content: crate::providers::ChatMessageContent::Text(r#"{"result": "success"}"#.to_string()),
                    tool_calls: None,
                    tool_call_id: None,
                    name: None,
                },
                finish_reason: Some("stop".to_string()),
            }],
            usage: crate::providers::TokenUsage {
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
        assert_eq!(data.feature, "structured_outputs");
        assert_eq!(data.data["validated"], true);
        assert_eq!(data.data["choices_validated"], 1);
    }

    #[test]
    fn test_adapt_response_validation_failure() {
        let adapter = StructuredOutputsAdapter;
        let schema = json!({
            "type": "object",
            "properties": {
                "result": {"type": "string"}
            },
            "required": ["result"]
        });

        let mut extensions = HashMap::new();
        extensions.insert("_structured_outputs_schema".to_string(), schema);

        let mut response = CompletionResponse {
            id: "test-id".to_string(),
            object: "chat.completion".to_string(),
            created: 1234567890,
            model: "gpt-4".to_string(),
            provider: "openai".to_string(),
            choices: vec![crate::providers::CompletionChoice {
                index: 0,
                message: crate::providers::ChatMessage {
                    role: "assistant".to_string(),
                    content: crate::providers::ChatMessageContent::Text(r#"{"wrong_field": "value"}"#.to_string()),
                    tool_calls: None,
                    tool_call_id: None,
                    name: None,
                },
                finish_reason: Some("stop".to_string()),
            }],
            usage: crate::providers::TokenUsage {
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
            .contains("does not match schema"));
    }

    #[test]
    fn test_detect_provider() {
        assert_eq!(StructuredOutputsAdapter::detect_provider("gpt-4"), "openai");
        assert_eq!(
            StructuredOutputsAdapter::detect_provider("gpt-3.5-turbo"),
            "openai"
        );
        assert_eq!(
            StructuredOutputsAdapter::detect_provider("o1-preview"),
            "openai"
        );
        assert_eq!(
            StructuredOutputsAdapter::detect_provider("claude-3-5-sonnet-20241022"),
            "anthropic"
        );
        assert_eq!(
            StructuredOutputsAdapter::detect_provider("claude-opus-4-5"),
            "anthropic"
        );
        assert_eq!(
            StructuredOutputsAdapter::detect_provider("llama-3"),
            "unknown"
        );
    }

    #[test]
    fn test_complex_schema_validation() {
        let content = r#"{
            "user": {
                "name": "Alice",
                "email": "alice@example.com",
                "age": 30
            },
            "items": [
                {"id": 1, "name": "Item 1"},
                {"id": 2, "name": "Item 2"}
            ]
        }"#;

        let schema = json!({
            "type": "object",
            "properties": {
                "user": {
                    "type": "object",
                    "properties": {
                        "name": {"type": "string"},
                        "email": {"type": "string", "format": "email"},
                        "age": {"type": "number", "minimum": 0}
                    },
                    "required": ["name", "email"]
                },
                "items": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "id": {"type": "number"},
                            "name": {"type": "string"}
                        },
                        "required": ["id", "name"]
                    }
                }
            },
            "required": ["user", "items"]
        });

        assert!(StructuredOutputsAdapter::validate_response(content, &schema).is_ok());
    }
}
