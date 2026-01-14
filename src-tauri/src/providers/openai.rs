//! OpenAI provider implementation

use super::{
    Capability, ChatMessage, ChunkChoice, ChunkDelta, CompletionChoice, CompletionChunk,
    CompletionRequest, CompletionResponse, HealthStatus, ModelInfo, ModelProvider, PricingInfo,
    ProviderHealth, TokenUsage,
};
use crate::utils::errors::{AppError, AppResult};
use async_trait::async_trait;
use chrono::Utc;
use futures::stream::{Stream, StreamExt};
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use std::pin::Pin;
use std::time::Instant;

const OPENAI_API_BASE: &str = "https://api.openai.com/v1";

/// OpenAI provider implementation
pub struct OpenAIProvider {
    api_key: String,
    client: Client,
}

impl OpenAIProvider {
    /// Create a new OpenAI provider with the given API key
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            client: Client::new(),
        }
    }

    /// Get pricing information for known OpenAI models
    fn get_model_pricing(model: &str) -> Option<PricingInfo> {
        // Pricing information as of January 2025
        // Source: https://openai.com/api/pricing/
        match model {
            // GPT-4 Turbo models
            "gpt-4-turbo" | "gpt-4-turbo-2024-04-09" => Some(PricingInfo {
                input_cost_per_1k: 0.01,
                output_cost_per_1k: 0.03,
                currency: "USD".to_string(),
            }),
            "gpt-4-turbo-preview" | "gpt-4-0125-preview" | "gpt-4-1106-preview" => {
                Some(PricingInfo {
                    input_cost_per_1k: 0.01,
                    output_cost_per_1k: 0.03,
                    currency: "USD".to_string(),
                })
            }
            // GPT-4 models
            "gpt-4" | "gpt-4-0613" => Some(PricingInfo {
                input_cost_per_1k: 0.03,
                output_cost_per_1k: 0.06,
                currency: "USD".to_string(),
            }),
            "gpt-4-32k" | "gpt-4-32k-0613" => Some(PricingInfo {
                input_cost_per_1k: 0.06,
                output_cost_per_1k: 0.12,
                currency: "USD".to_string(),
            }),
            // GPT-3.5 Turbo models
            "gpt-3.5-turbo" | "gpt-3.5-turbo-0125" | "gpt-3.5-turbo-1106" => Some(PricingInfo {
                input_cost_per_1k: 0.0005,
                output_cost_per_1k: 0.0015,
                currency: "USD".to_string(),
            }),
            "gpt-3.5-turbo-instruct" => Some(PricingInfo {
                input_cost_per_1k: 0.0015,
                output_cost_per_1k: 0.002,
                currency: "USD".to_string(),
            }),
            // GPT-4o models (newest)
            "gpt-4o" | "gpt-4o-2024-11-20" | "gpt-4o-2024-08-06" | "gpt-4o-2024-05-13" => {
                Some(PricingInfo {
                    input_cost_per_1k: 0.0025,
                    output_cost_per_1k: 0.01,
                    currency: "USD".to_string(),
                })
            }
            "gpt-4o-mini" | "gpt-4o-mini-2024-07-18" => Some(PricingInfo {
                input_cost_per_1k: 0.00015,
                output_cost_per_1k: 0.0006,
                currency: "USD".to_string(),
            }),
            // o1 models (reasoning models)
            "o1-preview" | "o1-preview-2024-09-12" => Some(PricingInfo {
                input_cost_per_1k: 0.015,
                output_cost_per_1k: 0.06,
                currency: "USD".to_string(),
            }),
            "o1-mini" | "o1-mini-2024-09-12" => Some(PricingInfo {
                input_cost_per_1k: 0.003,
                output_cost_per_1k: 0.012,
                currency: "USD".to_string(),
            }),
            _ => None,
        }
    }

    /// Build authorization header
    fn auth_header(&self) -> String {
        format!("Bearer {}", self.api_key)
    }
}

// OpenAI API response types

#[derive(Debug, Deserialize)]
struct OpenAIModel {
    id: String,
    object: String,
    created: i64,
    owned_by: String,
}

#[derive(Debug, Deserialize)]
struct OpenAIModelsResponse {
    data: Vec<OpenAIModel>,
}

#[derive(Debug, Serialize)]
struct OpenAIChatRequest {
    model: String,
    messages: Vec<ChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    frequency_penalty: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    presence_penalty: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stop: Option<Vec<String>>,
    #[serde(default)]
    stream: bool,
}

#[derive(Debug, Deserialize)]
struct OpenAIChatResponse {
    id: String,
    object: String,
    created: i64,
    model: String,
    choices: Vec<OpenAIChoice>,
    usage: OpenAIUsage,
}

#[derive(Debug, Deserialize)]
struct OpenAIChoice {
    index: u32,
    message: ChatMessage,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenAIUsage {
    prompt_tokens: u32,
    completion_tokens: u32,
    total_tokens: u32,
}

#[derive(Debug, Deserialize)]
struct OpenAIStreamChunk {
    id: String,
    object: String,
    created: i64,
    model: String,
    choices: Vec<OpenAIStreamChoice>,
}

#[derive(Debug, Deserialize)]
struct OpenAIStreamChoice {
    index: u32,
    delta: OpenAIDelta,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenAIDelta {
    #[serde(skip_serializing_if = "Option::is_none")]
    role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<String>,
}

#[async_trait]
impl ModelProvider for OpenAIProvider {
    fn name(&self) -> &str {
        "openai"
    }

    async fn health_check(&self) -> ProviderHealth {
        let start = Instant::now();

        // Use /v1/models endpoint for health check
        let result = self
            .client
            .get(format!("{}/models", OPENAI_API_BASE))
            .header("Authorization", self.auth_header())
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
                error_message: Some(format!("Connection failed: {}", e)),
            },
        }
    }

    async fn list_models(&self) -> AppResult<Vec<ModelInfo>> {
        let response = self
            .client
            .get(format!("{}/models", OPENAI_API_BASE))
            .header("Authorization", self.auth_header())
            .send()
            .await
            .map_err(|e| AppError::Provider(format!("Failed to fetch models: {}", e)))?;

        if !response.status().is_success() {
            return Err(AppError::Provider(format!(
                "API returned status: {}",
                response.status()
            )));
        }

        let models_response: OpenAIModelsResponse = response
            .json()
            .await
            .map_err(|e| AppError::Provider(format!("Failed to parse models response: {}", e)))?;

        let mut models = Vec::new();

        for model in models_response.data {
            // Only include chat completion models
            if !model.id.starts_with("gpt-")
                && !model.id.starts_with("o1-")
                && !model.id.starts_with("text-")
            {
                continue;
            }

            // Determine context window based on model name
            let context_window = if model.id.contains("32k") {
                32768
            } else if model.id.contains("turbo") {
                16384
            } else if model.id.starts_with("gpt-4o") {
                128000
            } else if model.id.starts_with("o1") {
                128000
            } else if model.id.starts_with("gpt-4") {
                8192
            } else if model.id.starts_with("gpt-3.5") {
                4096
            } else {
                4096
            };

            // Determine parameter count (estimates)
            let parameter_count = if model.id.starts_with("gpt-4") {
                Some(1_760_000_000_000) // 1.76T parameters (estimated)
            } else if model.id.starts_with("gpt-3.5") {
                Some(175_000_000_000) // 175B parameters
            } else {
                None
            };

            // Determine capabilities
            let mut capabilities = vec![Capability::Chat, Capability::Completion];
            if !model.id.starts_with("o1") {
                capabilities.push(Capability::FunctionCalling);
            }
            // GPT-4 Vision models
            if model.id.contains("vision") || model.id.starts_with("gpt-4o") {
                capabilities.push(Capability::Vision);
            }

            models.push(ModelInfo {
                id: model.id.clone(),
                name: model.id,
                provider: "openai".to_string(),
                parameter_count,
                context_window,
                supports_streaming: true,
                capabilities,
            });
        }

        Ok(models)
    }

    async fn get_pricing(&self, model: &str) -> AppResult<PricingInfo> {
        Self::get_model_pricing(model).ok_or_else(|| {
            AppError::Provider(format!(
                "Pricing information not available for model: {}",
                model
            ))
        })
    }

    async fn complete(&self, request: CompletionRequest) -> AppResult<CompletionResponse> {
        let openai_request = OpenAIChatRequest {
            model: request.model.clone(),
            messages: request.messages.clone(),
            temperature: request.temperature,
            max_tokens: request.max_tokens,
            top_p: request.top_p,
            frequency_penalty: request.frequency_penalty,
            presence_penalty: request.presence_penalty,
            stop: request.stop,
            stream: false,
        };

        let response = self
            .client
            .post(format!("{}/chat/completions", OPENAI_API_BASE))
            .header("Authorization", self.auth_header())
            .header("Content-Type", "application/json")
            .json(&openai_request)
            .send()
            .await
            .map_err(|e| AppError::Provider(format!("Request failed: {}", e)))?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());

            return Err(match status {
                StatusCode::UNAUTHORIZED => AppError::Unauthorized,
                StatusCode::TOO_MANY_REQUESTS => AppError::RateLimitExceeded,
                _ => AppError::Provider(format!("API error ({}): {}", status, error_text)),
            });
        }

        let openai_response: OpenAIChatResponse = response
            .json()
            .await
            .map_err(|e| AppError::Provider(format!("Failed to parse response: {}", e)))?;

        Ok(CompletionResponse {
            id: openai_response.id,
            object: openai_response.object,
            created: openai_response.created,
            model: openai_response.model,
            choices: openai_response
                .choices
                .into_iter()
                .map(|choice| CompletionChoice {
                    index: choice.index,
                    message: choice.message,
                    finish_reason: choice.finish_reason,
                })
                .collect(),
            usage: TokenUsage {
                prompt_tokens: openai_response.usage.prompt_tokens,
                completion_tokens: openai_response.usage.completion_tokens,
                total_tokens: openai_response.usage.total_tokens,
            },
        })
    }

    async fn stream_complete(
        &self,
        request: CompletionRequest,
    ) -> AppResult<Pin<Box<dyn Stream<Item = AppResult<CompletionChunk>> + Send>>> {
        let openai_request = OpenAIChatRequest {
            model: request.model.clone(),
            messages: request.messages.clone(),
            temperature: request.temperature,
            max_tokens: request.max_tokens,
            top_p: request.top_p,
            frequency_penalty: request.frequency_penalty,
            presence_penalty: request.presence_penalty,
            stop: request.stop,
            stream: true,
        };

        let response = self
            .client
            .post(format!("{}/chat/completions", OPENAI_API_BASE))
            .header("Authorization", self.auth_header())
            .header("Content-Type", "application/json")
            .json(&openai_request)
            .send()
            .await
            .map_err(|e| AppError::Provider(format!("Request failed: {}", e)))?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());

            return Err(match status {
                StatusCode::UNAUTHORIZED => AppError::Unauthorized,
                StatusCode::TOO_MANY_REQUESTS => AppError::RateLimitExceeded,
                _ => AppError::Provider(format!("API error ({}): {}", status, error_text)),
            });
        }

        // Parse SSE (Server-Sent Events) stream
        let stream = response.bytes_stream().map(|result| {
            result
                .map_err(|e| AppError::Provider(format!("Stream error: {}", e)))
                .and_then(|bytes| {
                    let text = String::from_utf8_lossy(&bytes);

                    // Parse SSE format: "data: {...}\n\n"
                    for line in text.lines() {
                        if let Some(json_str) = line.strip_prefix("data: ") {
                            // Check for [DONE] marker
                            if json_str.trim() == "[DONE]" {
                                continue;
                            }

                            // Parse JSON chunk
                            let openai_chunk: OpenAIStreamChunk = serde_json::from_str(json_str)
                                .map_err(|e| {
                                    AppError::Provider(format!("Failed to parse chunk: {}", e))
                                })?;

                            return Ok(CompletionChunk {
                                id: openai_chunk.id,
                                object: openai_chunk.object,
                                created: openai_chunk.created,
                                model: openai_chunk.model,
                                choices: openai_chunk
                                    .choices
                                    .into_iter()
                                    .map(|choice| ChunkChoice {
                                        index: choice.index,
                                        delta: ChunkDelta {
                                            role: choice.delta.role,
                                            content: choice.delta.content,
                                        },
                                        finish_reason: choice.finish_reason,
                                    })
                                    .collect(),
                            });
                        }
                    }

                    // No valid chunk found in this batch
                    Err(AppError::Provider(
                        "No valid chunk found in stream".to_string(),
                    ))
                })
        });

        Ok(Box::pin(stream))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pricing_info_gpt4() {
        let pricing = OpenAIProvider::get_model_pricing("gpt-4").unwrap();
        assert_eq!(pricing.input_cost_per_1k, 0.03);
        assert_eq!(pricing.output_cost_per_1k, 0.06);
        assert_eq!(pricing.currency, "USD");
    }

    #[test]
    fn test_pricing_info_gpt35_turbo() {
        let pricing = OpenAIProvider::get_model_pricing("gpt-3.5-turbo").unwrap();
        assert_eq!(pricing.input_cost_per_1k, 0.0005);
        assert_eq!(pricing.output_cost_per_1k, 0.0015);
        assert_eq!(pricing.currency, "USD");
    }

    #[test]
    fn test_pricing_info_gpt4o() {
        let pricing = OpenAIProvider::get_model_pricing("gpt-4o").unwrap();
        assert_eq!(pricing.input_cost_per_1k, 0.0025);
        assert_eq!(pricing.output_cost_per_1k, 0.01);
        assert_eq!(pricing.currency, "USD");
    }

    #[test]
    fn test_pricing_info_unknown_model() {
        let pricing = OpenAIProvider::get_model_pricing("unknown-model");
        assert!(pricing.is_none());
    }

    #[test]
    fn test_provider_name() {
        let provider = OpenAIProvider::new("test-key".to_string());
        assert_eq!(provider.name(), "openai");
    }

    #[test]
    fn test_auth_header() {
        let provider = OpenAIProvider::new("sk-test123".to_string());
        assert_eq!(provider.auth_header(), "Bearer sk-test123");
    }
}
