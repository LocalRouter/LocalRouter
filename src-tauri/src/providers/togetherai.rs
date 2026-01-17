//! Together AI provider implementation
//!
//! Implements the ModelProvider trait for Together AI's platform.
//! Together AI offers a wide variety of open-source models with fast inference.

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

const TOGETHER_API_BASE: &str = "https://api.together.xyz/v1";

/// Together AI provider
pub struct TogetherAIProvider {
    client: Client,
    api_key: String,
}

#[allow(dead_code)]
impl TogetherAIProvider {
    /// Create a new Together AI provider with an API key
    pub fn new(api_key: String) -> AppResult<Self> {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .map_err(|e| AppError::Provider(format!("Failed to create HTTP client: {}", e)))?;

        Ok(Self { client, api_key })
    }

    /// Create a new Together AI provider from stored API key
    pub fn from_stored_key(provider_name: Option<&str>) -> AppResult<Self> {
        let name = provider_name.unwrap_or("togetherai");
        let api_key = super::key_storage::get_provider_key(name)?
            .ok_or_else(|| AppError::Provider(format!("No API key found for provider '{}'", name)))?;
        Self::new(api_key)
    }

    /// Get known model information
    fn get_known_models() -> Vec<ModelInfo> {
        vec![
            ModelInfo {
                id: "meta-llama/Meta-Llama-3.1-405B-Instruct-Turbo".to_string(),
                name: "Llama 3.1 405B Instruct Turbo".to_string(),
                provider: "togetherai".to_string(),
                parameter_count: Some(405_000_000_000),
                context_window: 130_000,
                supports_streaming: true,
                capabilities: vec![Capability::Chat, Capability::FunctionCalling],
                detailed_capabilities: None,
            },
            ModelInfo {
                id: "meta-llama/Meta-Llama-3.1-70B-Instruct-Turbo".to_string(),
                name: "Llama 3.1 70B Instruct Turbo".to_string(),
                provider: "togetherai".to_string(),
                parameter_count: Some(70_000_000_000),
                context_window: 130_000,
                supports_streaming: true,
                capabilities: vec![Capability::Chat, Capability::FunctionCalling],
                detailed_capabilities: None,
            },
            ModelInfo {
                id: "meta-llama/Meta-Llama-3.1-8B-Instruct-Turbo".to_string(),
                name: "Llama 3.1 8B Instruct Turbo".to_string(),
                provider: "togetherai".to_string(),
                parameter_count: Some(8_000_000_000),
                context_window: 130_000,
                supports_streaming: true,
                capabilities: vec![Capability::Chat, Capability::FunctionCalling],
                detailed_capabilities: None,
            },
            ModelInfo {
                id: "Qwen/Qwen2.5-72B-Instruct-Turbo".to_string(),
                name: "Qwen 2.5 72B Instruct".to_string(),
                provider: "togetherai".to_string(),
                parameter_count: Some(72_000_000_000),
                context_window: 32_000,
                supports_streaming: true,
                capabilities: vec![Capability::Chat, Capability::FunctionCalling],
                detailed_capabilities: None,
            },
            ModelInfo {
                id: "mistralai/Mixtral-8x7B-Instruct-v0.1".to_string(),
                name: "Mixtral 8x7B Instruct".to_string(),
                provider: "togetherai".to_string(),
                parameter_count: Some(47_000_000_000),
                context_window: 32_000,
                supports_streaming: true,
                capabilities: vec![Capability::Chat],
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
impl ModelProvider for TogetherAIProvider {
    fn name(&self) -> &str {
        "togetherai"
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
        Ok(Self::get_known_models())
    }

    async fn get_pricing(&self, model: &str) -> AppResult<PricingInfo> {
        // Together AI pricing as of 2026-01
        let pricing = if model.contains("405B") {
            PricingInfo {
                input_cost_per_1k: 0.005,  // $5 per 1M tokens
                output_cost_per_1k: 0.015, // $15 per 1M tokens
                currency: "USD".to_string(),
            }
        } else if model.contains("70B") || model.contains("72B") {
            PricingInfo {
                input_cost_per_1k: 0.0009, // $0.9 per 1M tokens
                output_cost_per_1k: 0.0009, // $0.9 per 1M tokens
                currency: "USD".to_string(),
            }
        } else {
            PricingInfo {
                input_cost_per_1k: 0.0002, // $0.2 per 1M tokens
                output_cost_per_1k: 0.0002, // $0.2 per 1M tokens
                currency: "USD".to_string(),
            }
        };

        Ok(pricing)
    }

    async fn complete(&self, request: CompletionRequest) -> AppResult<CompletionResponse> {
        let url = format!("{}/chat/completions", TOGETHER_API_BASE);

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await
            .map_err(|e| AppError::Provider(format!("Together AI request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(AppError::Provider(format!(
                "Together AI API error {}: {}",
                status, error_text
            )));
        }

        let together_response: OpenAIChatResponse = response
            .json()
            .await
            .map_err(|e| AppError::Provider(format!("Failed to parse Together AI response: {}", e)))?;

        Ok(CompletionResponse {
            id: together_response.id,
            object: together_response.object,
            created: together_response.created,
            model: together_response.model,
            provider: self.name().to_string(),
            choices: together_response
                .choices
                .into_iter()
                .map(|choice| CompletionChoice {
                    index: choice.index,
                    message: choice.message,
                    finish_reason: choice.finish_reason,
                })
                .collect(),
            usage: together_response.usage,
            extensions: None,
        })
    }

    async fn stream_complete(
        &self,
        request: CompletionRequest,
    ) -> AppResult<Pin<Box<dyn Stream<Item = AppResult<CompletionChunk>> + Send>>> {
        let url = format!("{}/chat/completions", TOGETHER_API_BASE);

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await
            .map_err(|e| AppError::Provider(format!("Together AI streaming request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            return Err(AppError::Provider(format!(
                "Together AI streaming API error: {}",
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
                            Ok(together_chunk) => {
                                let chunk = CompletionChunk {
                                    id: together_chunk.id,
                                    object: together_chunk.object,
                                    created: together_chunk.created,
                                    model: together_chunk.model,
                                    choices: together_chunk
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
        let models = TogetherAIProvider::get_known_models();
        assert!(!models.is_empty());
        assert!(models.iter().any(|m| m.id.contains("Llama-3.1-405B")));
    }

    #[tokio::test]
    async fn test_pricing() {
        let provider = TogetherAIProvider::new("test_key".to_string()).unwrap();
        let pricing = provider.get_pricing("meta-llama/Meta-Llama-3.1-405B-Instruct-Turbo").await.unwrap();
        assert!(pricing.input_cost_per_1k > 0.0);
    }
}
