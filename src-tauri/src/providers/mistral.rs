//! Mistral AI provider implementation
//!
//! Implements the ModelProvider trait for Mistral's LLM API.
//! Mistral offers models like Mistral Small, Medium, Large, and Mixtral.

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

const MISTRAL_API_BASE: &str = "https://api.mistral.ai/v1";

/// Mistral AI provider
pub struct MistralProvider {
    client: Client,
    api_key: String,
}

#[allow(dead_code)]
impl MistralProvider {
    /// Create a new Mistral provider with an API key
    pub fn new(api_key: String) -> AppResult<Self> {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .map_err(|e| AppError::Provider(format!("Failed to create HTTP client: {}", e)))?;

        Ok(Self { client, api_key })
    }

    /// Create a new Mistral provider from stored API key
    pub fn from_stored_key(provider_name: Option<&str>) -> AppResult<Self> {
        let name = provider_name.unwrap_or("mistral");
        let api_key = super::key_storage::get_provider_key(name)?
            .ok_or_else(|| AppError::Provider(format!("No API key found for provider '{}'", name)))?;
        Self::new(api_key)
    }

    /// Get known model information
    fn get_known_models() -> Vec<ModelInfo> {
        vec![
            ModelInfo {
                id: "mistral-large-latest".to_string(),
                name: "Mistral Large".to_string(),
                provider: "mistral".to_string(),
                parameter_count: None,
                context_window: 128_000,
                supports_streaming: true,
                capabilities: vec![Capability::Chat, Capability::FunctionCalling],
                detailed_capabilities: None,
            },
            ModelInfo {
                id: "mistral-small-latest".to_string(),
                name: "Mistral Small".to_string(),
                provider: "mistral".to_string(),
                parameter_count: None,
                context_window: 32_000,
                supports_streaming: true,
                capabilities: vec![Capability::Chat, Capability::FunctionCalling],
                detailed_capabilities: None,
            },
            ModelInfo {
                id: "codestral-latest".to_string(),
                name: "Codestral (Code Specialist)".to_string(),
                provider: "mistral".to_string(),
                parameter_count: None,
                context_window: 32_000,
                supports_streaming: true,
                capabilities: vec![Capability::Chat, Capability::Completion],
                detailed_capabilities: None,
            },
            ModelInfo {
                id: "mistral-medium-latest".to_string(),
                name: "Mistral Medium".to_string(),
                provider: "mistral".to_string(),
                parameter_count: None,
                context_window: 32_000,
                supports_streaming: true,
                capabilities: vec![Capability::Chat, Capability::FunctionCalling],
                detailed_capabilities: None,
            },
            ModelInfo {
                id: "open-mistral-7b".to_string(),
                name: "Open Mistral 7B".to_string(),
                provider: "mistral".to_string(),
                parameter_count: Some(7_000_000_000),
                context_window: 32_000,
                supports_streaming: true,
                capabilities: vec![Capability::Chat],
                detailed_capabilities: None,
            },
            ModelInfo {
                id: "open-mixtral-8x7b".to_string(),
                name: "Open Mixtral 8x7B".to_string(),
                provider: "mistral".to_string(),
                parameter_count: Some(47_000_000_000),
                context_window: 32_000,
                supports_streaming: true,
                capabilities: vec![Capability::Chat],
                detailed_capabilities: None,
            },
            ModelInfo {
                id: "open-mixtral-8x22b".to_string(),
                name: "Open Mixtral 8x22B".to_string(),
                provider: "mistral".to_string(),
                parameter_count: Some(141_000_000_000),
                context_window: 64_000,
                supports_streaming: true,
                capabilities: vec![Capability::Chat, Capability::FunctionCalling],
                detailed_capabilities: None,
            },
        ]
    }
}

// OpenAI-compatible API types
#[derive(Debug, Serialize, Deserialize)]
struct OpenAIChatResponse {
    id: String,
    object: String,
    created: i64,
    model: String,
    choices: Vec<OpenAIChoice>,
    usage: TokenUsage,
}

#[derive(Debug, Serialize, Deserialize)]
struct OpenAIChoice {
    index: u32,
    message: ChatMessage,
    finish_reason: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct OpenAIStreamChunk {
    id: String,
    object: String,
    created: i64,
    model: String,
    choices: Vec<OpenAIStreamChoice>,
}

#[derive(Debug, Serialize, Deserialize)]
struct OpenAIStreamChoice {
    index: u32,
    delta: ChunkDelta,
    finish_reason: Option<String>,
}

#[async_trait]
#[allow(dead_code)]
impl ModelProvider for MistralProvider {
    fn name(&self) -> &str {
        "mistral"
    }

    async fn health_check(&self) -> ProviderHealth {
        let start = Instant::now();

        match self.list_models().await {
            Ok(_) => {
                let latency = start.elapsed().as_millis() as u64;
                ProviderHealth {
                    status: HealthStatus::Healthy,
                    latency_ms: Some(latency),
                    last_checked: Utc::now(),
                    error_message: None,
                }
            }
            Err(e) => ProviderHealth {
                status: HealthStatus::Unhealthy,
                latency_ms: None,
                last_checked: Utc::now(),
                error_message: Some(e.to_string()),
            },
        }
    }

    async fn list_models(&self) -> AppResult<Vec<ModelInfo>> {
        // Return known models
        Ok(Self::get_known_models())
    }

    async fn get_pricing(&self, model: &str) -> AppResult<PricingInfo> {
        // Mistral pricing as of 2026-01
        let pricing = match model {
            "mistral-large-latest" | "mistral-large-2411" => PricingInfo {
                input_cost_per_1k: 0.002,  // $2 per 1M tokens
                output_cost_per_1k: 0.006, // $6 per 1M tokens
                currency: "USD".to_string(),
            },
            "mistral-medium-latest" => PricingInfo {
                input_cost_per_1k: 0.00275, // $2.75 per 1M tokens
                output_cost_per_1k: 0.0081, // $8.1 per 1M tokens
                currency: "USD".to_string(),
            },
            "mistral-small-latest" | "mistral-small-2409" => PricingInfo {
                input_cost_per_1k: 0.0002,  // $0.2 per 1M tokens
                output_cost_per_1k: 0.0006, // $0.6 per 1M tokens
                currency: "USD".to_string(),
            },
            "codestral-latest" => PricingInfo {
                input_cost_per_1k: 0.0002,  // $0.2 per 1M tokens
                output_cost_per_1k: 0.0006, // $0.6 per 1M tokens
                currency: "USD".to_string(),
            },
            "open-mistral-7b" => PricingInfo {
                input_cost_per_1k: 0.00025, // $0.25 per 1M tokens
                output_cost_per_1k: 0.00025, // $0.25 per 1M tokens
                currency: "USD".to_string(),
            },
            "open-mixtral-8x7b" => PricingInfo {
                input_cost_per_1k: 0.0007, // $0.7 per 1M tokens
                output_cost_per_1k: 0.0007, // $0.7 per 1M tokens
                currency: "USD".to_string(),
            },
            "open-mixtral-8x22b" => PricingInfo {
                input_cost_per_1k: 0.002, // $2 per 1M tokens
                output_cost_per_1k: 0.006, // $6 per 1M tokens
                currency: "USD".to_string(),
            },
            _ => PricingInfo {
                input_cost_per_1k: 0.001,
                output_cost_per_1k: 0.003,
                currency: "USD".to_string(),
            },
        };

        Ok(pricing)
    }

    async fn complete(&self, request: CompletionRequest) -> AppResult<CompletionResponse> {
        let url = format!("{}/chat/completions", MISTRAL_API_BASE);

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await
            .map_err(|e| AppError::Provider(format!("Mistral request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(AppError::Provider(format!(
                "Mistral API error {}: {}",
                status, error_text
            )));
        }

        let mistral_response: OpenAIChatResponse = response
            .json()
            .await
            .map_err(|e| AppError::Provider(format!("Failed to parse Mistral response: {}", e)))?;

        Ok(CompletionResponse {
            id: mistral_response.id,
            object: mistral_response.object,
            created: mistral_response.created,
            model: mistral_response.model,
            provider: self.name().to_string(),
            choices: mistral_response
                .choices
                .into_iter()
                .map(|choice| CompletionChoice {
                    index: choice.index,
                    message: choice.message,
                    finish_reason: choice.finish_reason,
                })
                .collect(),
            usage: mistral_response.usage,
            extensions: None,
        })
    }

    async fn stream_complete(
        &self,
        request: CompletionRequest,
    ) -> AppResult<Pin<Box<dyn Stream<Item = AppResult<CompletionChunk>> + Send>>> {
        let url = format!("{}/chat/completions", MISTRAL_API_BASE);

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await
            .map_err(|e| AppError::Provider(format!("Mistral streaming request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            return Err(AppError::Provider(format!(
                "Mistral streaming API error: {}",
                status
            )));
        }

        let stream = response.bytes_stream();

        let line_buffer = std::sync::Arc::new(std::sync::Mutex::new(String::new()));

        let converted_stream = stream.flat_map(move |result| {
            let line_buffer = line_buffer.clone();

            let chunks: Vec<AppResult<CompletionChunk>> = match result {
                Ok(bytes) => {
                    let text = String::from_utf8_lossy(&bytes);
                    let mut buffer = line_buffer.lock().unwrap();
                    buffer.push_str(&text);

                    let mut chunks = Vec::new();

                    while let Some(newline_pos) = buffer.find('\n') {
                        let line = buffer[..newline_pos].to_string();
                        *buffer = buffer[newline_pos + 1..].to_string();

                        let line = line.trim();
                        if line.is_empty() || !line.starts_with("data: ") {
                            continue;
                        }

                        let data = &line[6..];

                        if data == "[DONE]" {
                            break;
                        }

                        match serde_json::from_str::<OpenAIStreamChunk>(data) {
                            Ok(mistral_chunk) => {
                                let chunk = CompletionChunk {
                                    id: mistral_chunk.id,
                                    object: mistral_chunk.object,
                                    created: mistral_chunk.created,
                                    model: mistral_chunk.model,
                                    choices: mistral_chunk
                                        .choices
                                        .into_iter()
                                        .map(|choice| ChunkChoice {
                                            index: choice.index,
                                            delta: choice.delta,
                                            finish_reason: choice.finish_reason,
                                        })
                                        .collect(),
                                    extensions: None,
                                };
                                chunks.push(Ok(chunk));
                            }
                            Err(e) => {
                                chunks.push(Err(AppError::Provider(format!(
                                    "Failed to parse stream chunk: {}",
                                    e
                                ))));
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_known_models() {
        let models = MistralProvider::get_known_models();
        assert!(!models.is_empty());
        assert!(models.iter().any(|m| m.id == "mistral-large-latest"));
    }

    #[tokio::test]
    async fn test_pricing() {
        let provider = MistralProvider::new("test_key".to_string()).unwrap();
        let pricing = provider.get_pricing("mistral-large-latest").await.unwrap();
        assert!(pricing.input_cost_per_1k > 0.0);
    }
}
