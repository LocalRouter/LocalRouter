//! Anthropic (Claude) provider implementation
//!
//! Implements the ModelProvider trait for Anthropic's Claude models.
//! Uses the Messages API format which differs from OpenAI's chat completions.

use async_trait::async_trait;
use chrono::Utc;
use futures::stream::{Stream, StreamExt};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::pin::Pin;
use std::time::Instant;

use crate::utils::errors::{AppError, AppResult};

use super::{
    Capability, ChatMessage, ChunkChoice, ChunkDelta, CompletionChoice, CompletionChunk,
    CompletionRequest, CompletionResponse, HealthStatus, ModelInfo, ModelProvider, PricingInfo,
    ProviderHealth, TokenUsage,
};

const ANTHROPIC_API_BASE: &str = "https://api.anthropic.com/v1";
const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Anthropic provider for Claude models
pub struct AnthropicProvider {
    client: Client,
    api_key: String,
    base_url: String,
}

#[allow(dead_code)]
impl AnthropicProvider {
    /// Create a new Anthropic provider with an API key
    pub fn new(api_key: String) -> AppResult<Self> {
        Self::with_base_url(api_key, ANTHROPIC_API_BASE.to_string())
    }

    /// Create a new Anthropic provider with a custom base URL (for testing)
    pub fn with_base_url(api_key: String, base_url: String) -> AppResult<Self> {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .map_err(|e| AppError::Provider(format!("Failed to create HTTP client: {}", e)))?;

        Ok(Self {
            client,
            api_key,
            base_url: base_url.trim_end_matches('/').to_string(),
        })
    }

    /// Create a new Anthropic provider from stored API key
    ///
    /// # Arguments
    /// * `provider_name` - The provider name used to store the key (defaults to "anthropic")
    ///
    /// # Returns
    /// * `Ok(Self)` if key exists and provider created successfully
    /// * `Err(AppError)` if key doesn't exist or keyring access fails
    pub fn from_stored_key(provider_name: Option<&str>) -> AppResult<Self> {
        let name = provider_name.unwrap_or("anthropic");
        let api_key = super::key_storage::get_provider_key(name)?.ok_or_else(|| {
            AppError::Provider(format!("No API key found for provider '{}'", name))
        })?;
        Self::new(api_key)
    }

    /// Convert OpenAI format messages to Anthropic format
    fn convert_messages(
        messages: &[ChatMessage],
    ) -> AppResult<(Option<String>, Vec<AnthropicMessage>)> {
        let mut system_prompt = None;
        let mut anthropic_messages = Vec::new();

        for msg in messages {
            match msg.role.as_str() {
                "system" => {
                    // Anthropic uses a separate system parameter
                    if system_prompt.is_some() {
                        return Err(AppError::Provider(
                            "Multiple system messages not supported".to_string(),
                        ));
                    }
                    system_prompt = Some(msg.content.clone());
                }
                "user" | "assistant" => {
                    anthropic_messages.push(AnthropicMessage {
                        role: msg.role.clone(),
                        content: msg.content.clone(),
                    });
                }
                _ => {
                    return Err(AppError::Provider(format!(
                        "Unsupported message role: {}",
                        msg.role
                    )));
                }
            }
        }

        Ok((system_prompt, anthropic_messages))
    }

    /// Get model information by ID
    fn get_model_info(model_id: &str) -> Option<ModelInfo> {
        match model_id {
            "claude-opus-4-20250514" => Some(ModelInfo {
                id: model_id.to_string(),
                name: "Claude Opus 4".to_string(),
                provider: "anthropic".to_string(),
                parameter_count: None,
                context_window: 200_000,
                supports_streaming: true,
                capabilities: vec![
                    Capability::Chat,
                    Capability::Vision,
                    Capability::FunctionCalling,
                ],
                detailed_capabilities: None,
            }),
            "claude-sonnet-4-20250514" => Some(ModelInfo {
                id: model_id.to_string(),
                name: "Claude Sonnet 4".to_string(),
                provider: "anthropic".to_string(),
                parameter_count: None,
                context_window: 200_000,
                supports_streaming: true,
                capabilities: vec![
                    Capability::Chat,
                    Capability::Vision,
                    Capability::FunctionCalling,
                ],
                detailed_capabilities: None,
            }),
            "claude-3-5-sonnet-20241022" => Some(ModelInfo {
                id: model_id.to_string(),
                name: "Claude 3.5 Sonnet".to_string(),
                provider: "anthropic".to_string(),
                parameter_count: None,
                context_window: 200_000,
                supports_streaming: true,
                capabilities: vec![
                    Capability::Chat,
                    Capability::Vision,
                    Capability::FunctionCalling,
                ],
                detailed_capabilities: None,
            }),
            "claude-3-5-haiku-20241022" => Some(ModelInfo {
                id: model_id.to_string(),
                name: "Claude 3.5 Haiku".to_string(),
                provider: "anthropic".to_string(),
                parameter_count: None,
                context_window: 200_000,
                supports_streaming: true,
                capabilities: vec![Capability::Chat, Capability::Vision],
                detailed_capabilities: None,
            }),
            "claude-3-opus-20240229" => Some(ModelInfo {
                id: model_id.to_string(),
                name: "Claude 3 Opus".to_string(),
                provider: "anthropic".to_string(),
                parameter_count: None,
                context_window: 200_000,
                supports_streaming: true,
                capabilities: vec![
                    Capability::Chat,
                    Capability::Vision,
                    Capability::FunctionCalling,
                ],
                detailed_capabilities: None,
            }),
            "claude-3-sonnet-20240229" => Some(ModelInfo {
                id: model_id.to_string(),
                name: "Claude 3 Sonnet".to_string(),
                provider: "anthropic".to_string(),
                parameter_count: None,
                context_window: 200_000,
                supports_streaming: true,
                capabilities: vec![Capability::Chat, Capability::Vision],
                detailed_capabilities: None,
            }),
            "claude-3-haiku-20240307" => Some(ModelInfo {
                id: model_id.to_string(),
                name: "Claude 3 Haiku".to_string(),
                provider: "anthropic".to_string(),
                parameter_count: None,
                context_window: 200_000,
                supports_streaming: true,
                capabilities: vec![Capability::Chat, Capability::Vision],
                detailed_capabilities: None,
            }),
            _ => None,
        }
    }

    /// Get pricing for a model
    fn get_model_pricing(model_id: &str) -> PricingInfo {
        match model_id {
            "claude-opus-4-20250514" => PricingInfo {
                input_cost_per_1k: 0.015,
                output_cost_per_1k: 0.075,
                currency: "USD".to_string(),
            },
            "claude-sonnet-4-20250514" => PricingInfo {
                input_cost_per_1k: 0.003,
                output_cost_per_1k: 0.015,
                currency: "USD".to_string(),
            },
            "claude-3-5-sonnet-20241022" => PricingInfo {
                input_cost_per_1k: 0.003,
                output_cost_per_1k: 0.015,
                currency: "USD".to_string(),
            },
            "claude-3-5-haiku-20241022" => PricingInfo {
                input_cost_per_1k: 0.001,
                output_cost_per_1k: 0.005,
                currency: "USD".to_string(),
            },
            "claude-3-opus-20240229" => PricingInfo {
                input_cost_per_1k: 0.015,
                output_cost_per_1k: 0.075,
                currency: "USD".to_string(),
            },
            "claude-3-sonnet-20240229" => PricingInfo {
                input_cost_per_1k: 0.003,
                output_cost_per_1k: 0.015,
                currency: "USD".to_string(),
            },
            "claude-3-haiku-20240307" => PricingInfo {
                input_cost_per_1k: 0.00025,
                output_cost_per_1k: 0.00125,
                currency: "USD".to_string(),
            },
            _ => PricingInfo {
                input_cost_per_1k: 0.0,
                output_cost_per_1k: 0.0,
                currency: "USD".to_string(),
            },
        }
    }
}

#[async_trait]
#[allow(dead_code)]
impl ModelProvider for AnthropicProvider {
    fn name(&self) -> &str {
        "anthropic"
    }

    async fn health_check(&self) -> ProviderHealth {
        let start = Instant::now();

        // Try to list models as a health check
        let result = self
            .client
            .get(format!("{}/models", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .send()
            .await;

        let latency_ms = start.elapsed().as_millis() as u64;

        match result {
            Ok(response) => {
                if response.status().is_success() {
                    ProviderHealth {
                        status: HealthStatus::Healthy,
                        latency_ms: Some(latency_ms),
                        last_checked: Utc::now(),
                        error_message: None,
                    }
                } else {
                    ProviderHealth {
                        status: HealthStatus::Unhealthy,
                        latency_ms: Some(latency_ms),
                        last_checked: Utc::now(),
                        error_message: Some(format!("API returned status: {}", response.status())),
                    }
                }
            }
            Err(e) => ProviderHealth {
                status: HealthStatus::Unhealthy,
                latency_ms: None,
                last_checked: Utc::now(),
                error_message: Some(format!("Request failed: {}", e)),
            },
        }
    }

    async fn list_models(&self) -> AppResult<Vec<ModelInfo>> {
        // Anthropic doesn't have a public models endpoint yet
        // Return a static list of known Claude models
        let models = vec![
            "claude-opus-4-20250514",
            "claude-sonnet-4-20250514",
            "claude-3-5-sonnet-20241022",
            "claude-3-5-haiku-20241022",
            "claude-3-opus-20240229",
            "claude-3-sonnet-20240229",
            "claude-3-haiku-20240307",
        ];

        Ok(models
            .into_iter()
            .filter_map(Self::get_model_info)
            .collect())
    }

    async fn get_pricing(&self, model: &str) -> AppResult<PricingInfo> {
        // Try catalog first (embedded OpenRouter data)
        if let Some(catalog_model) = crate::catalog::find_model("anthropic", model) {
            tracing::debug!("Using catalog pricing for Anthropic model: {}", model);
            return Ok(PricingInfo {
                input_cost_per_1k: catalog_model.pricing.prompt_cost_per_1k(),
                output_cost_per_1k: catalog_model.pricing.completion_cost_per_1k(),
                currency: catalog_model.pricing.currency.to_string(),
            });
        }

        // Fallback to hardcoded pricing
        tracing::debug!("Using fallback pricing for Anthropic model: {}", model);
        Ok(Self::get_model_pricing(model))
    }

    async fn complete(&self, request: CompletionRequest) -> AppResult<CompletionResponse> {
        let (system, messages) = Self::convert_messages(&request.messages)?;

        let anthropic_request = AnthropicRequest {
            model: request.model.clone(),
            messages,
            system,
            max_tokens: request.max_tokens.unwrap_or(4096),
            temperature: request.temperature,
            top_p: request.top_p,
            stop_sequences: request.stop,
            stream: Some(false),
        };

        let response = self
            .client
            .post(format!("{}/messages", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("content-type", "application/json")
            .json(&anthropic_request)
            .send()
            .await
            .map_err(|e| AppError::Provider(format!("Request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            return Err(AppError::Provider(format!(
                "API error ({}): {}",
                status, error_text
            )));
        }

        let anthropic_response: AnthropicResponse = response
            .json()
            .await
            .map_err(|e| AppError::Provider(format!("Failed to parse response: {}", e)))?;

        // Convert Anthropic response to OpenAI format
        let content = anthropic_response
            .content
            .first()
            .map(|c| c.text.clone())
            .unwrap_or_default();

        Ok(CompletionResponse {
            id: anthropic_response.id,
            object: "chat.completion".to_string(),
            created: Utc::now().timestamp(),
            model: anthropic_response.model,
            provider: self.name().to_string(),
            choices: vec![CompletionChoice {
                index: 0,
                message: ChatMessage {
                    role: "assistant".to_string(),
                    content,
                },
                finish_reason: Some(
                    anthropic_response
                        .stop_reason
                        .unwrap_or_else(|| "stop".to_string()),
                ),
            }],
            usage: TokenUsage {
                prompt_tokens: anthropic_response.usage.input_tokens,
                completion_tokens: anthropic_response.usage.output_tokens,
                total_tokens: anthropic_response.usage.input_tokens
                    + anthropic_response.usage.output_tokens,
                prompt_tokens_details: None,
                completion_tokens_details: None,
            },
            extensions: None,
        })
    }

    async fn stream_complete(
        &self,
        request: CompletionRequest,
    ) -> AppResult<Pin<Box<dyn Stream<Item = AppResult<CompletionChunk>> + Send>>> {
        let (system, messages) = Self::convert_messages(&request.messages)?;

        let anthropic_request = AnthropicRequest {
            model: request.model.clone(),
            messages,
            system,
            max_tokens: request.max_tokens.unwrap_or(4096),
            temperature: request.temperature,
            top_p: request.top_p,
            stop_sequences: request.stop,
            stream: Some(true),
        };

        let response = self
            .client
            .post(format!("{}/messages", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("content-type", "application/json")
            .json(&anthropic_request)
            .send()
            .await
            .map_err(|e| AppError::Provider(format!("Request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            return Err(AppError::Provider(format!(
                "API error ({}): {}",
                status, error_text
            )));
        }

        let model = request.model.clone();
        let stream = response.bytes_stream();

        // Buffer for incomplete lines across byte chunks
        use std::sync::{Arc, Mutex};
        let line_buffer = Arc::new(Mutex::new(String::new()));

        let converted_stream = stream.flat_map(move |result| {
            let model = model.clone();
            let line_buffer = line_buffer.clone();

            let chunks: Vec<AppResult<CompletionChunk>> = match result {
                Ok(bytes) => {
                    let text = String::from_utf8_lossy(&bytes);
                    let mut buffer = line_buffer.lock().unwrap();

                    // Append new data to buffer
                    buffer.push_str(&text);

                    let mut chunks = Vec::new();

                    // Process complete lines (those ending with \n)
                    while let Some(newline_pos) = buffer.find('\n') {
                        let line = buffer[..newline_pos].to_string();
                        *buffer = buffer[newline_pos + 1..].to_string();

                        if line.trim().is_empty() {
                            continue;
                        }

                        // Parse SSE format: "data: {...}"
                        if let Some(data) = line.strip_prefix("data: ") {
                            // Skip [DONE] marker
                            if data == "[DONE]" {
                                continue;
                            }

                            // Parse JSON event
                            match serde_json::from_str::<AnthropicStreamEvent>(data) {
                                Ok(event) => {
                                    match event.event_type.as_str() {
                                        "content_block_delta" => {
                                            if let Some(delta) = event.delta {
                                                if let Some(text) = delta.text {
                                                    // Anthropic sends delta chunks, not cumulative
                                                    let chunk = CompletionChunk {
                                                        id: event.message_id.unwrap_or_default(),
                                                        object: "chat.completion.chunk".to_string(),
                                                        created: Utc::now().timestamp(),
                                                        model: model.clone(),
                                                        choices: vec![ChunkChoice {
                                                            index: 0,
                                                            delta: ChunkDelta {
                                                                role: None,
                                                                content: Some(text),
                                                            },
                                                            finish_reason: None,
                                                        }],
                                                        extensions: None,
                                                    };
                                                    chunks.push(Ok(chunk));
                                                }
                                            }
                                        }
                                        "message_stop" => {
                                            let chunk = CompletionChunk {
                                                id: event.message_id.unwrap_or_default(),
                                                object: "chat.completion.chunk".to_string(),
                                                created: Utc::now().timestamp(),
                                                model: model.clone(),
                                                choices: vec![ChunkChoice {
                                                    index: 0,
                                                    delta: ChunkDelta {
                                                        role: None,
                                                        content: None,
                                                    },
                                                    finish_reason: Some("stop".to_string()),
                                                }],
                                                extensions: None,
                                            };
                                            chunks.push(Ok(chunk));
                                        }
                                        _ => {}
                                    }
                                }
                                Err(e) => {
                                    tracing::error!(
                                        "Failed to parse Anthropic stream event: {} - Line: {}",
                                        e,
                                        data
                                    );
                                }
                            }
                        }
                    }

                    chunks
                }
                Err(e) => vec![Err(AppError::Provider(format!("Stream error: {}", e)))],
            };

            futures::stream::iter(chunks)
        });

        Ok(Box::pin(converted_stream))
    }

    fn supports_feature(&self, feature: &str) -> bool {
        matches!(
            feature,
            "extended_thinking" | "prompt_caching" | "structured_outputs"
        )
    }

    fn get_feature_adapter(
        &self,
        feature: &str,
    ) -> Option<Box<dyn crate::providers::features::FeatureAdapter>> {
        match feature {
            "extended_thinking" => Some(Box::new(
                crate::providers::features::anthropic_thinking::AnthropicThinkingAdapter,
            )),
            "structured_outputs" => Some(Box::new(
                crate::providers::features::structured_outputs::StructuredOutputsAdapter,
            )),
            "prompt_caching" => Some(Box::new(
                crate::providers::features::prompt_caching::PromptCachingAdapter,
            )),
            "json_mode" => Some(Box::new(
                crate::providers::features::json_mode::JsonModeAdapter,
            )),
            _ => None,
        }
    }
}

// Anthropic API request/response structures

#[derive(Debug, Serialize)]
struct AnthropicRequest {
    model: String,
    messages: Vec<AnthropicMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
    max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stop_sequences: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize)]
struct AnthropicMessage {
    role: String,
    content: String,
}

#[derive(Debug, Deserialize)]
struct AnthropicResponse {
    id: String,
    model: String,
    content: Vec<AnthropicContent>,
    stop_reason: Option<String>,
    usage: AnthropicUsage,
}

#[derive(Debug, Deserialize)]
struct AnthropicContent {
    text: String,
}

#[derive(Debug, Deserialize)]
struct AnthropicUsage {
    input_tokens: u32,
    output_tokens: u32,
}

#[derive(Debug, Deserialize)]
struct AnthropicStreamEvent {
    #[serde(rename = "type")]
    event_type: String,
    #[serde(default)]
    message_id: Option<String>,
    #[serde(default)]
    delta: Option<AnthropicDelta>,
}

#[derive(Debug, Deserialize)]
struct AnthropicDelta {
    #[serde(default)]
    text: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_convert_messages_with_system() {
        let messages = vec![
            ChatMessage {
                role: "system".to_string(),
                content: "You are a helpful assistant.".to_string(),
            },
            ChatMessage {
                role: "user".to_string(),
                content: "Hello!".to_string(),
            },
        ];

        let (system, anthropic_messages) = AnthropicProvider::convert_messages(&messages).unwrap();

        assert_eq!(system, Some("You are a helpful assistant.".to_string()));
        assert_eq!(anthropic_messages.len(), 1);
        assert_eq!(anthropic_messages[0].role, "user");
        assert_eq!(anthropic_messages[0].content, "Hello!");
    }

    #[test]
    fn test_convert_messages_without_system() {
        let messages = vec![
            ChatMessage {
                role: "user".to_string(),
                content: "Hello!".to_string(),
            },
            ChatMessage {
                role: "assistant".to_string(),
                content: "Hi there!".to_string(),
            },
        ];

        let (system, anthropic_messages) = AnthropicProvider::convert_messages(&messages).unwrap();

        assert_eq!(system, None);
        assert_eq!(anthropic_messages.len(), 2);
    }

    #[test]
    fn test_model_info_lookup() {
        let info = AnthropicProvider::get_model_info("claude-3-5-sonnet-20241022").unwrap();
        assert_eq!(info.name, "Claude 3.5 Sonnet");
        assert_eq!(info.provider, "anthropic");
        assert_eq!(info.context_window, 200_000);
        assert!(info.supports_streaming);
    }

    #[test]
    fn test_pricing_lookup() {
        let pricing = AnthropicProvider::get_model_pricing("claude-3-5-sonnet-20241022");
        assert_eq!(pricing.input_cost_per_1k, 0.003);
        assert_eq!(pricing.output_cost_per_1k, 0.015);
        assert_eq!(pricing.currency, "USD");
    }

    #[test]
    fn test_model_info_unknown() {
        let info = AnthropicProvider::get_model_info("unknown-model");
        assert!(info.is_none());
    }

    #[tokio::test]
    async fn test_list_models() {
        let provider = AnthropicProvider::new("test-key".to_string()).unwrap();
        let models = provider.list_models().await.unwrap();

        assert!(!models.is_empty());
        assert!(models.iter().any(|m| m.id.contains("claude")));
    }
}
