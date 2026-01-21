//! Groq provider implementation
//!
//! Implements the ModelProvider trait for Groq's LLM API.
//! Groq offers fast inference for models like Llama, Mixtral, and Gemma.

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

const GROQ_API_BASE: &str = "https://api.groq.com/openai/v1";

/// Groq provider for fast LLM inference
pub struct GroqProvider {
    client: Client,
    api_key: String,
    base_url: String,
}

#[allow(dead_code)]
impl GroqProvider {
    /// Create a new Groq provider with an API key
    pub fn new(api_key: String) -> AppResult<Self> {
        Self::with_base_url(api_key, GROQ_API_BASE.to_string())
    }

    /// Create a new Groq provider with a custom base URL (for testing)
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

    /// Create a new Groq provider from stored API key
    pub fn from_stored_key(provider_name: Option<&str>) -> AppResult<Self> {
        let name = provider_name.unwrap_or("groq");
        let api_key = super::key_storage::get_provider_key(name)?.ok_or_else(|| {
            AppError::Provider(format!("No API key found for provider '{}'", name))
        })?;
        Self::new(api_key)
    }

    /// Get known model information
    fn get_known_models() -> Vec<ModelInfo> {
        vec![
            ModelInfo {
                id: "llama-3.3-70b-versatile".to_string(),
                name: "Llama 3.3 70B".to_string(),
                provider: "groq".to_string(),
                parameter_count: Some(70_000_000_000),
                context_window: 128_000,
                supports_streaming: true,
                capabilities: vec![Capability::Chat, Capability::FunctionCalling],
                detailed_capabilities: None,
            },
            ModelInfo {
                id: "llama-3.1-70b-versatile".to_string(),
                name: "Llama 3.1 70B".to_string(),
                provider: "groq".to_string(),
                parameter_count: Some(70_000_000_000),
                context_window: 128_000,
                supports_streaming: true,
                capabilities: vec![Capability::Chat, Capability::FunctionCalling],
                detailed_capabilities: None,
            },
            ModelInfo {
                id: "llama-3.1-8b-instant".to_string(),
                name: "Llama 3.1 8B Instant".to_string(),
                provider: "groq".to_string(),
                parameter_count: Some(8_000_000_000),
                context_window: 128_000,
                supports_streaming: true,
                capabilities: vec![Capability::Chat, Capability::FunctionCalling],
                detailed_capabilities: None,
            },
            ModelInfo {
                id: "llama3-70b-8192".to_string(),
                name: "Llama 3 70B".to_string(),
                provider: "groq".to_string(),
                parameter_count: Some(70_000_000_000),
                context_window: 8192,
                supports_streaming: true,
                capabilities: vec![Capability::Chat, Capability::FunctionCalling],
                detailed_capabilities: None,
            },
            ModelInfo {
                id: "llama3-8b-8192".to_string(),
                name: "Llama 3 8B".to_string(),
                provider: "groq".to_string(),
                parameter_count: Some(8_000_000_000),
                context_window: 8192,
                supports_streaming: true,
                capabilities: vec![Capability::Chat, Capability::FunctionCalling],
                detailed_capabilities: None,
            },
            ModelInfo {
                id: "mixtral-8x7b-32768".to_string(),
                name: "Mixtral 8x7B".to_string(),
                provider: "groq".to_string(),
                parameter_count: Some(47_000_000_000),
                context_window: 32_768,
                supports_streaming: true,
                capabilities: vec![Capability::Chat, Capability::FunctionCalling],
                detailed_capabilities: None,
            },
            ModelInfo {
                id: "gemma2-9b-it".to_string(),
                name: "Gemma 2 9B".to_string(),
                provider: "groq".to_string(),
                parameter_count: Some(9_000_000_000),
                context_window: 8192,
                supports_streaming: true,
                capabilities: vec![Capability::Chat],
                detailed_capabilities: None,
            },
        ]
    }
}

// OpenAI-compatible API types (reused from OpenAI provider pattern)
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
impl ModelProvider for GroqProvider {
    fn name(&self) -> &str {
        "groq"
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
        // Return known models (Groq's model list endpoint requires paid tier)
        Ok(Self::get_known_models())
    }

    async fn get_pricing(&self, model: &str) -> AppResult<PricingInfo> {
        // Groq pricing as of 2026-01
        let pricing = match model {
            "llama-3.3-70b-versatile" | "llama-3.1-70b-versatile" | "llama3-70b-8192" => {
                PricingInfo {
                    input_cost_per_1k: 0.00059,  // $0.59 per 1M tokens
                    output_cost_per_1k: 0.00079, // $0.79 per 1M tokens
                    currency: "USD".to_string(),
                }
            }
            "llama-3.1-8b-instant" | "llama3-8b-8192" | "gemma2-9b-it" => PricingInfo {
                input_cost_per_1k: 0.00005,  // $0.05 per 1M tokens
                output_cost_per_1k: 0.00008, // $0.08 per 1M tokens
                currency: "USD".to_string(),
            },
            "mixtral-8x7b-32768" => PricingInfo {
                input_cost_per_1k: 0.00024,  // $0.24 per 1M tokens
                output_cost_per_1k: 0.00024, // $0.24 per 1M tokens
                currency: "USD".to_string(),
            },
            _ => PricingInfo {
                // Default fallback pricing
                input_cost_per_1k: 0.0001,
                output_cost_per_1k: 0.0001,
                currency: "USD".to_string(),
            },
        };

        Ok(pricing)
    }

    async fn complete(&self, request: CompletionRequest) -> AppResult<CompletionResponse> {
        let url = format!("{}/chat/completions", self.base_url);

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await
            .map_err(|e| AppError::Provider(format!("Groq request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(AppError::Provider(format!(
                "Groq API error {}: {}",
                status, error_text
            )));
        }

        let groq_response: OpenAIChatResponse = response
            .json()
            .await
            .map_err(|e| AppError::Provider(format!("Failed to parse Groq response: {}", e)))?;

        Ok(CompletionResponse {
            id: groq_response.id,
            object: groq_response.object,
            created: groq_response.created,
            model: groq_response.model,
            provider: self.name().to_string(),
            choices: groq_response
                .choices
                .into_iter()
                .map(|choice| CompletionChoice {
                    index: choice.index,
                    message: choice.message,
                    finish_reason: choice.finish_reason,
                    logprobs: None, // Groq does not support logprobs
                })
                .collect(),
            usage: groq_response.usage,
            extensions: None,
            routellm_win_rate: None,
        })
    }

    async fn stream_complete(
        &self,
        request: CompletionRequest,
    ) -> AppResult<Pin<Box<dyn Stream<Item = AppResult<CompletionChunk>> + Send>>> {
        let url = format!("{}/chat/completions", self.base_url);

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await
            .map_err(|e| AppError::Provider(format!("Groq streaming request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            return Err(AppError::Provider(format!(
                "Groq streaming API error: {}",
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

                        let data = &line[6..]; // Remove "data: " prefix

                        if data == "[DONE]" {
                            break;
                        }

                        match serde_json::from_str::<OpenAIStreamChunk>(data) {
                            Ok(groq_chunk) => {
                                let chunk = CompletionChunk {
                                    id: groq_chunk.id,
                                    object: groq_chunk.object,
                                    created: groq_chunk.created,
                                    model: groq_chunk.model,
                                    choices: groq_chunk
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
        let models = GroqProvider::get_known_models();
        assert!(!models.is_empty());
        assert!(models.iter().any(|m| m.id == "llama-3.3-70b-versatile"));
    }

    #[tokio::test]
    async fn test_pricing() {
        let provider = GroqProvider::new("test_key".to_string()).unwrap();
        let pricing = provider
            .get_pricing("llama-3.3-70b-versatile")
            .await
            .unwrap();
        assert!(pricing.input_cost_per_1k > 0.0);
    }
}
