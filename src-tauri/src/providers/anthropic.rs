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
}

impl AnthropicProvider {
    /// Create a new Anthropic provider with an API key
    pub fn new(api_key: String) -> AppResult<Self> {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .map_err(|e| AppError::Provider(format!("Failed to create HTTP client: {}", e)))?;

        Ok(Self { client, api_key })
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
            }),
            "claude-3-5-haiku-20241022" => Some(ModelInfo {
                id: model_id.to_string(),
                name: "Claude 3.5 Haiku".to_string(),
                provider: "anthropic".to_string(),
                parameter_count: None,
                context_window: 200_000,
                supports_streaming: true,
                capabilities: vec![Capability::Chat, Capability::Vision],
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
            }),
            "claude-3-sonnet-20240229" => Some(ModelInfo {
                id: model_id.to_string(),
                name: "Claude 3 Sonnet".to_string(),
                provider: "anthropic".to_string(),
                parameter_count: None,
                context_window: 200_000,
                supports_streaming: true,
                capabilities: vec![Capability::Chat, Capability::Vision],
            }),
            "claude-3-haiku-20240307" => Some(ModelInfo {
                id: model_id.to_string(),
                name: "Claude 3 Haiku".to_string(),
                provider: "anthropic".to_string(),
                parameter_count: None,
                context_window: 200_000,
                supports_streaming: true,
                capabilities: vec![Capability::Chat, Capability::Vision],
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
impl ModelProvider for AnthropicProvider {
    fn name(&self) -> &str {
        "anthropic"
    }

    async fn health_check(&self) -> ProviderHealth {
        let start = Instant::now();

        // Try to list models as a health check
        let result = self
            .client
            .get(format!("{}/models", ANTHROPIC_API_BASE))
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
            .post(format!("{}/messages", ANTHROPIC_API_BASE))
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
            },
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
            .post(format!("{}/messages", ANTHROPIC_API_BASE))
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
        let stream = response.bytes_stream().map(move |result| {
            let model = model.clone();
            match result {
                Ok(bytes) => {
                    // Parse SSE format
                    let text = String::from_utf8_lossy(&bytes);

                    // Parse each SSE event
                    for line in text.lines() {
                        if line.starts_with("data: ") {
                            let data = &line[6..];

                            // Skip [DONE] marker
                            if data == "[DONE]" {
                                continue;
                            }

                            // Parse JSON event
                            if let Ok(event) = serde_json::from_str::<AnthropicStreamEvent>(data) {
                                match event.event_type.as_str() {
                                    "content_block_delta" => {
                                        if let Some(delta) = event.delta {
                                            if let Some(text) = delta.text {
                                                return Ok(CompletionChunk {
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
                                                });
                                            }
                                        }
                                    }
                                    "message_stop" => {
                                        return Ok(CompletionChunk {
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
                                        });
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }

                    // Return empty chunk if we couldn't parse anything useful
                    Ok(CompletionChunk {
                        id: String::new(),
                        object: "chat.completion.chunk".to_string(),
                        created: Utc::now().timestamp(),
                        model: model.clone(),
                        choices: vec![],
                    })
                }
                Err(e) => Err(AppError::Provider(format!("Stream error: {}", e))),
            }
        });

        Ok(Box::pin(stream))
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
