//! Generic OpenAI-compatible provider implementation
//!
//! This provider works with any service that implements the OpenAI API specification,
//! including LocalAI, LM Studio, vLLM, and other compatible services.

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

/// Generic OpenAI-compatible provider with configurable endpoint
pub struct OpenAICompatibleProvider {
    name: String,
    api_key: Option<String>,
    base_url: String,
    client: Client,
}

impl OpenAICompatibleProvider {
    /// Create a new OpenAI-compatible provider
    ///
    /// # Arguments
    /// * `name` - Instance name for this provider
    /// * `base_url` - Base URL for the API (e.g., "http://localhost:8080/v1")
    /// * `api_key` - Optional API key (some services like LocalAI don't require one)
    pub fn new(name: String, base_url: String, api_key: Option<String>) -> Self {
        Self {
            name,
            api_key,
            base_url: base_url.trim_end_matches('/').to_string(),
            client: Client::new(),
        }
    }

    /// Build authorization header if API key is present
    fn auth_header(&self) -> Option<String> {
        self.api_key.as_ref().map(|key| format!("Bearer {}", key))
    }
}

// OpenAI API response types (reused from OpenAI provider)

#[derive(Debug, Deserialize)]
struct OpenAIModel {
    id: String,
    #[allow(dead_code)]
    object: String,
    #[allow(dead_code)]
    created: i64,
    #[allow(dead_code)]
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
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<super::Tool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<super::ToolChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_format: Option<super::ResponseFormat>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<super::ToolCallDelta>>,
}

// OpenAI Embeddings API types
#[derive(Debug, Serialize)]
struct OpenAIEmbeddingRequest {
    model: String,
    input: OpenAIEmbeddingInput,
    #[serde(skip_serializing_if = "Option::is_none")]
    encoding_format: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    dimensions: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    user: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
enum OpenAIEmbeddingInput {
    Single(String),
    Multiple(Vec<String>),
}

#[derive(Debug, Deserialize)]
struct OpenAIEmbeddingResponse {
    object: String,
    data: Vec<OpenAIEmbedding>,
    model: String,
    usage: OpenAIEmbeddingUsage,
}

#[derive(Debug, Deserialize)]
struct OpenAIEmbedding {
    object: String,
    embedding: Vec<f32>,
    index: usize,
}

#[derive(Debug, Deserialize)]
struct OpenAIEmbeddingUsage {
    prompt_tokens: u32,
    total_tokens: u32,
}

#[async_trait]
impl ModelProvider for OpenAICompatibleProvider {
    fn name(&self) -> &str {
        &self.name
    }

    async fn health_check(&self) -> ProviderHealth {
        let start = Instant::now();

        // Use /models endpoint for health check
        let mut request = self.client.get(format!("{}/models", self.base_url));

        if let Some(auth) = self.auth_header() {
            request = request.header("Authorization", auth);
        }

        let result = request.send().await;

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
        let mut request = self.client.get(format!("{}/models", self.base_url));

        if let Some(auth) = self.auth_header() {
            request = request.header("Authorization", auth);
        }

        let response = request
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

        let models = models_response
            .data
            .into_iter()
            .map(|model| {
                ModelInfo {
                    id: model.id.clone(),
                    name: model.id,
                    provider: self.name.clone(),
                    parameter_count: None, // Not available from API
                    context_window: 4096,  // Default, actual value depends on model
                    supports_streaming: true,
                    capabilities: vec![Capability::Chat, Capability::Completion],
                    detailed_capabilities: None,
                }
                .enrich_with_catalog_by_name()
            }) // Use model-only search for multi-provider system
            .collect();

        Ok(models)
    }

    async fn get_pricing(&self, _model: &str) -> AppResult<PricingInfo> {
        // Generic providers don't have standard pricing
        // Return free by default, can be overridden by configuration
        Ok(PricingInfo::free())
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
            tools: request.tools,
            tool_choice: request.tool_choice,
            response_format: request.response_format,
        };

        let mut req = self
            .client
            .post(format!("{}/chat/completions", self.base_url))
            .header("Content-Type", "application/json")
            .json(&openai_request);

        if let Some(auth) = self.auth_header() {
            req = req.header("Authorization", auth);
        }

        let response = req
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

        // Validate choices array is not empty
        let choices: Vec<CompletionChoice> = openai_response
            .choices
            .into_iter()
            .map(|choice| CompletionChoice {
                index: choice.index,
                message: choice.message,
                finish_reason: choice.finish_reason,
            })
            .collect();

        if choices.is_empty() {
            return Err(AppError::Provider(
                "API returned no choices in response".to_string(),
            ));
        }

        Ok(CompletionResponse {
            id: openai_response.id,
            object: openai_response.object,
            created: openai_response.created,
            model: openai_response.model,
            provider: self.name().to_string(),
            choices,
            usage: TokenUsage {
                prompt_tokens: openai_response.usage.prompt_tokens,
                completion_tokens: openai_response.usage.completion_tokens,
                total_tokens: openai_response.usage.total_tokens,
                prompt_tokens_details: None,
                completion_tokens_details: None,
            },
            extensions: None,
            routellm_win_rate: None,
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
            tools: request.tools,
            tool_choice: request.tool_choice,
            response_format: request.response_format,
        };

        let mut req = self
            .client
            .post(format!("{}/chat/completions", self.base_url))
            .header("Content-Type", "application/json")
            .json(&openai_request);

        if let Some(auth) = self.auth_header() {
            req = req.header("Authorization", auth);
        }

        let response = req
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
        // Use flat_map to handle multiple SSE events in a single byte chunk
        // Buffer incomplete lines across HTTP chunks
        use std::sync::{Arc, Mutex};
        let line_buffer = Arc::new(Mutex::new(String::new()));

        let stream = response.bytes_stream().flat_map(move |result| {
            let line_buffer = line_buffer.clone();

            let chunks: Vec<AppResult<CompletionChunk>> = match result {
                Ok(bytes) => {
                    let text = String::from_utf8_lossy(&bytes);
                    let mut buffer = line_buffer.lock().unwrap();
                    let mut parsed_chunks = Vec::new();

                    // Append new data to buffer
                    buffer.push_str(&text);

                    // Process complete lines (those ending with \n)
                    while let Some(newline_pos) = buffer.find('\n') {
                        let line = buffer[..newline_pos].to_string();
                        *buffer = buffer[newline_pos + 1..].to_string();

                        if line.trim().is_empty() {
                            continue;
                        }

                        // Parse SSE format: "data: {...}"
                        if let Some(json_str) = line.strip_prefix("data: ") {
                            // Check for [DONE] marker
                            if json_str.trim() == "[DONE]" {
                                continue;
                            }

                            // Parse JSON chunk
                            match serde_json::from_str::<OpenAIStreamChunk>(json_str) {
                                Ok(openai_chunk) => {
                                    parsed_chunks.push(Ok(CompletionChunk {
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
                                                    tool_calls: choice.delta.tool_calls,
                                                },
                                                finish_reason: choice.finish_reason,
                                            })
                                            .collect(),
                                        extensions: None,
                                    }));
                                }
                                Err(e) => {
                                    parsed_chunks.push(Err(AppError::Provider(format!(
                                        "Failed to parse chunk: {}",
                                        e
                                    ))));
                                }
                            }
                        }
                    }

                    parsed_chunks
                }
                Err(e) => vec![Err(AppError::Provider(format!("Stream error: {}", e)))],
            };

            futures::stream::iter(chunks)
        });

        Ok(Box::pin(stream))
    }

    async fn embed(&self, request: super::EmbeddingRequest) -> AppResult<super::EmbeddingResponse> {
        // Convert our generic EmbeddingRequest to OpenAI-specific format
        let input = match request.input {
            super::EmbeddingInput::Single(text) => OpenAIEmbeddingInput::Single(text),
            super::EmbeddingInput::Multiple(texts) => OpenAIEmbeddingInput::Multiple(texts),
            super::EmbeddingInput::Tokens(_) => {
                return Err(AppError::Provider(
                    "OpenAI-compatible embeddings do not support pre-tokenized input".to_string(),
                ));
            }
        };

        let encoding_format = request.encoding_format.map(|format| match format {
            super::EncodingFormat::Float => "float".to_string(),
            super::EncodingFormat::Base64 => "base64".to_string(),
        });

        let openai_request = OpenAIEmbeddingRequest {
            model: request.model.clone(),
            input,
            encoding_format,
            dimensions: request.dimensions,
            user: request.user,
        };

        let mut http_request = self
            .client
            .post(format!("{}/embeddings", self.base_url))
            .header("Content-Type", "application/json")
            .json(&openai_request);

        if let Some(auth) = self.auth_header() {
            http_request = http_request.header("Authorization", auth);
        }

        let response = http_request
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

        let openai_response: OpenAIEmbeddingResponse = response
            .json()
            .await
            .map_err(|e| AppError::Provider(format!("Failed to parse response: {}", e)))?;

        // Convert OpenAI response to our generic format
        Ok(super::EmbeddingResponse {
            object: openai_response.object,
            data: openai_response
                .data
                .into_iter()
                .map(|emb| super::Embedding {
                    object: emb.object,
                    embedding: Some(emb.embedding),
                    index: emb.index,
                })
                .collect(),
            model: openai_response.model,
            usage: super::EmbeddingUsage {
                prompt_tokens: openai_response.usage.prompt_tokens,
                total_tokens: openai_response.usage.total_tokens,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_provider_name() {
        let provider = OpenAICompatibleProvider::new(
            "my-local-ai".to_string(),
            "http://localhost:8080/v1".to_string(),
            None,
        );
        assert_eq!(provider.name(), "my-local-ai");
    }

    #[test]
    fn test_auth_header_with_key() {
        let provider = OpenAICompatibleProvider::new(
            "test".to_string(),
            "http://localhost:8080/v1".to_string(),
            Some("test-key-123".to_string()),
        );
        assert_eq!(
            provider.auth_header(),
            Some("Bearer test-key-123".to_string())
        );
    }

    #[test]
    fn test_auth_header_without_key() {
        let provider = OpenAICompatibleProvider::new(
            "test".to_string(),
            "http://localhost:8080/v1".to_string(),
            None,
        );
        assert_eq!(provider.auth_header(), None);
    }

    #[test]
    fn test_base_url_trailing_slash() {
        let provider = OpenAICompatibleProvider::new(
            "test".to_string(),
            "http://localhost:8080/v1/".to_string(),
            None,
        );
        assert_eq!(provider.base_url, "http://localhost:8080/v1");
    }

    #[tokio::test]
    async fn test_pricing_is_free() {
        let provider = OpenAICompatibleProvider::new(
            "test".to_string(),
            "http://localhost:8080/v1".to_string(),
            None,
        );
        let pricing = provider.get_pricing("any-model").await.unwrap();
        assert_eq!(pricing.input_cost_per_1k, 0.0);
        assert_eq!(pricing.output_cost_per_1k, 0.0);
    }
}
