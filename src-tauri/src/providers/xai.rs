//! xAI (Grok) provider implementation
//!
//! Implements the ModelProvider trait for xAI's Grok models.
//! xAI offers the Grok series of models with real-time knowledge access.

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

const XAI_API_BASE: &str = "https://api.x.ai/v1";

/// xAI (Grok) provider
pub struct XAIProvider {
    client: Client,
    api_key: String,
}

#[allow(dead_code)]
impl XAIProvider {
    /// Create a new xAI provider with an API key
    pub fn new(api_key: String) -> AppResult<Self> {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .map_err(|e| AppError::Provider(format!("Failed to create HTTP client: {}", e)))?;

        Ok(Self { client, api_key })
    }

    /// Create a new xAI provider from stored API key
    pub fn from_stored_key(provider_name: Option<&str>) -> AppResult<Self> {
        let name = provider_name.unwrap_or("xai");
        let api_key = super::key_storage::get_provider_key(name)?
            .ok_or_else(|| AppError::Provider(format!("No API key found for provider '{}'", name)))?;
        Self::new(api_key)
    }

    /// Get known model information
    fn get_known_models() -> Vec<ModelInfo> {
        vec![
            ModelInfo {
                id: "grok-2".to_string(),
                name: "Grok 2".to_string(),
                provider: "xai".to_string(),
                parameter_count: None,
                context_window: 131_072,
                supports_streaming: true,
                capabilities: vec![Capability::Chat, Capability::FunctionCalling],
                detailed_capabilities: None,
            },
            ModelInfo {
                id: "grok-2-mini".to_string(),
                name: "Grok 2 Mini".to_string(),
                provider: "xai".to_string(),
                parameter_count: None,
                context_window: 131_072,
                supports_streaming: true,
                capabilities: vec![Capability::Chat, Capability::FunctionCalling],
                detailed_capabilities: None,
            },
            ModelInfo {
                id: "grok-beta".to_string(),
                name: "Grok Beta".to_string(),
                provider: "xai".to_string(),
                parameter_count: None,
                context_window: 131_072,
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
impl ModelProvider for XAIProvider {
    fn name(&self) -> &str {
        "xai"
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
        // xAI pricing as of 2026-01
        let pricing = match model {
            "grok-2" => PricingInfo {
                input_cost_per_1k: 0.002,  // $2 per 1M tokens
                output_cost_per_1k: 0.010, // $10 per 1M tokens
                currency: "USD".to_string(),
            },
            "grok-2-mini" => PricingInfo {
                input_cost_per_1k: 0.0002, // $0.2 per 1M tokens
                output_cost_per_1k: 0.001, // $1 per 1M tokens
                currency: "USD".to_string(),
            },
            "grok-beta" => PricingInfo {
                input_cost_per_1k: 0.005,  // $5 per 1M tokens
                output_cost_per_1k: 0.015, // $15 per 1M tokens
                currency: "USD".to_string(),
            },
            _ => PricingInfo {
                input_cost_per_1k: 0.002,
                output_cost_per_1k: 0.010,
                currency: "USD".to_string(),
            },
        };

        Ok(pricing)
    }

    async fn complete(&self, request: CompletionRequest) -> AppResult<CompletionResponse> {
        let url = format!("{}/chat/completions", XAI_API_BASE);

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await
            .map_err(|e| AppError::Provider(format!("xAI request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(AppError::Provider(format!(
                "xAI API error {}: {}",
                status, error_text
            )));
        }

        let xai_response: OpenAIChatResponse = response
            .json()
            .await
            .map_err(|e| AppError::Provider(format!("Failed to parse xAI response: {}", e)))?;

        Ok(CompletionResponse {
            id: xai_response.id,
            object: xai_response.object,
            created: xai_response.created,
            model: xai_response.model,
            provider: self.name().to_string(),
            choices: xai_response
                .choices
                .into_iter()
                .map(|choice| CompletionChoice {
                    index: choice.index,
                    message: choice.message,
                    finish_reason: choice.finish_reason,
                })
                .collect(),
            usage: xai_response.usage,
            extensions: None,
        })
    }

    async fn stream_complete(
        &self,
        request: CompletionRequest,
    ) -> AppResult<Pin<Box<dyn Stream<Item = AppResult<CompletionChunk>> + Send>>> {
        let url = format!("{}/chat/completions", XAI_API_BASE);

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await
            .map_err(|e| AppError::Provider(format!("xAI streaming request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            return Err(AppError::Provider(format!(
                "xAI streaming API error: {}",
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
                            Ok(xai_chunk) => {
                                let chunk = CompletionChunk {
                                    id: xai_chunk.id,
                                    object: xai_chunk.object,
                                    created: xai_chunk.created,
                                    model: xai_chunk.model,
                                    choices: xai_chunk
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
        let models = XAIProvider::get_known_models();
        assert!(!models.is_empty());
        assert!(models.iter().any(|m| m.id == "grok-2"));
    }

    #[tokio::test]
    async fn test_pricing() {
        let provider = XAIProvider::new("test_key".to_string()).unwrap();
        let pricing = provider.get_pricing("grok-2").await.unwrap();
        assert!(pricing.input_cost_per_1k > 0.0);
    }
}
