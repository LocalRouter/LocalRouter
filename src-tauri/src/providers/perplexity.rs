//! Perplexity provider implementation
//!
//! Implements the ModelProvider trait for Perplexity's search-augmented LLM API.
//! Perplexity models can search the web and provide sourced answers.

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

const PERPLEXITY_API_BASE: &str = "https://api.perplexity.ai";

/// Perplexity AI provider
pub struct PerplexityProvider {
    client: Client,
    api_key: String,
}

#[allow(dead_code)]
impl PerplexityProvider {
    /// Create a new Perplexity provider with an API key
    pub fn new(api_key: String) -> AppResult<Self> {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .map_err(|e| AppError::Provider(format!("Failed to create HTTP client: {}", e)))?;

        Ok(Self { client, api_key })
    }

    /// Create a new Perplexity provider from stored API key
    pub fn from_stored_key(provider_name: Option<&str>) -> AppResult<Self> {
        let name = provider_name.unwrap_or("perplexity");
        let api_key = super::key_storage::get_provider_key(name)?.ok_or_else(|| {
            AppError::Provider(format!("No API key found for provider '{}'", name))
        })?;
        Self::new(api_key)
    }

    /// Get known model information
    fn get_known_models() -> Vec<ModelInfo> {
        vec![
            ModelInfo {
                id: "sonar".to_string(),
                name: "Sonar".to_string(),
                provider: "perplexity".to_string(),
                parameter_count: Some(70_000_000_000), // Based on Llama 3.3 70B
                context_window: 128_000,
                supports_streaming: true,
                capabilities: vec![Capability::Chat],
                detailed_capabilities: None,
            },
            ModelInfo {
                id: "sonar-pro".to_string(),
                name: "Sonar Pro".to_string(),
                provider: "perplexity".to_string(),
                parameter_count: Some(70_000_000_000),
                context_window: 200_000,
                supports_streaming: true,
                capabilities: vec![Capability::Chat],
                detailed_capabilities: None,
            },
            ModelInfo {
                id: "sonar-reasoning-pro".to_string(),
                name: "Sonar Reasoning Pro".to_string(),
                provider: "perplexity".to_string(),
                parameter_count: None, // DeepSeek-R1 based
                context_window: 128_000,
                supports_streaming: true,
                capabilities: vec![Capability::Chat],
                detailed_capabilities: None,
            },
            ModelInfo {
                id: "sonar-deep-research".to_string(),
                name: "Sonar Deep Research".to_string(),
                provider: "perplexity".to_string(),
                parameter_count: None,
                context_window: 128_000,
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
impl ModelProvider for PerplexityProvider {
    fn name(&self) -> &str {
        "perplexity"
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
        // Try catalog first (embedded OpenRouter data)
        if let Some(catalog_model) = crate::catalog::find_model("perplexity", model) {
            tracing::debug!("Using catalog pricing for Perplexity model: {}", model);
            return Ok(PricingInfo {
                input_cost_per_1k: catalog_model.pricing.prompt_cost_per_1k(),
                output_cost_per_1k: catalog_model.pricing.completion_cost_per_1k(),
                currency: catalog_model.pricing.currency.to_string(),
            });
        }

        // Fallback to hardcoded pricing
        tracing::debug!("Using fallback pricing for Perplexity model: {}", model);

        // Perplexity pricing as of 2026-01
        let pricing = match model {
            "sonar" => PricingInfo {
                input_cost_per_1k: 0.001,  // $1 per 1M tokens
                output_cost_per_1k: 0.001, // $1 per 1M tokens
                currency: "USD".to_string(),
            },
            "sonar-pro" => PricingInfo {
                input_cost_per_1k: 0.003,  // $3 per 1M tokens
                output_cost_per_1k: 0.015, // $15 per 1M tokens
                currency: "USD".to_string(),
            },
            "sonar-reasoning-pro" => PricingInfo {
                input_cost_per_1k: 0.001,  // $1 per 1M tokens
                output_cost_per_1k: 0.005, // $5 per 1M tokens
                currency: "USD".to_string(),
            },
            "sonar-deep-research" => PricingInfo {
                input_cost_per_1k: 0.005,  // $5 per 1M tokens
                output_cost_per_1k: 0.005, // $5 per 1M tokens
                currency: "USD".to_string(),
            },
            _ => PricingInfo {
                input_cost_per_1k: 0.001, // Default pricing
                output_cost_per_1k: 0.001,
                currency: "USD".to_string(),
            },
        };

        Ok(pricing)
    }

    async fn complete(&self, request: CompletionRequest) -> AppResult<CompletionResponse> {
        let url = format!("{}/chat/completions", PERPLEXITY_API_BASE);

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await
            .map_err(|e| AppError::Provider(format!("Perplexity request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(AppError::Provider(format!(
                "Perplexity API error {}: {}",
                status, error_text
            )));
        }

        let perplexity_response: OpenAIChatResponse = response.json().await.map_err(|e| {
            AppError::Provider(format!("Failed to parse Perplexity response: {}", e))
        })?;

        Ok(CompletionResponse {
            id: perplexity_response.id,
            object: perplexity_response.object,
            created: perplexity_response.created,
            model: perplexity_response.model,
            provider: self.name().to_string(),
            choices: perplexity_response
                .choices
                .into_iter()
                .map(|choice| CompletionChoice {
                    index: choice.index,
                    message: choice.message,
                    finish_reason: choice.finish_reason,
                })
                .collect(),
            usage: perplexity_response.usage,
            extensions: None,
        })
    }

    async fn stream_complete(
        &self,
        request: CompletionRequest,
    ) -> AppResult<Pin<Box<dyn Stream<Item = AppResult<CompletionChunk>> + Send>>> {
        let url = format!("{}/chat/completions", PERPLEXITY_API_BASE);

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await
            .map_err(|e| {
                AppError::Provider(format!("Perplexity streaming request failed: {}", e))
            })?;

        if !response.status().is_success() {
            let status = response.status();
            return Err(AppError::Provider(format!(
                "Perplexity streaming API error: {}",
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
                            Ok(perplexity_chunk) => {
                                let chunk = CompletionChunk {
                                    id: perplexity_chunk.id,
                                    object: perplexity_chunk.object,
                                    created: perplexity_chunk.created,
                                    model: perplexity_chunk.model,
                                    choices: perplexity_chunk
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
        let models = PerplexityProvider::get_known_models();
        assert!(!models.is_empty());
        assert!(models.iter().any(|m| m.id.contains("sonar")));
    }

    #[tokio::test]
    async fn test_pricing() {
        let provider = PerplexityProvider::new("test_key".to_string()).unwrap();
        let pricing = provider.get_pricing("sonar").await.unwrap();
        assert!(pricing.input_cost_per_1k > 0.0);
    }
}
