//! OpenRouter provider implementation
//!
//! Provides access to multiple AI providers through OpenRouter's unified API.

use async_trait::async_trait;
use chrono::Utc;
use futures::stream::{Stream, StreamExt};
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use std::pin::Pin;
use std::time::Instant;

use lr_types::{AppError, AppResult};

use super::{
    Capability, ChatMessage, ChunkChoice, ChunkDelta, CompletionChoice, CompletionChunk,
    CompletionRequest, CompletionResponse, HealthStatus, ModelInfo, ModelProvider, PricingInfo,
    ProviderHealth, TokenUsage,
};

const OPENROUTER_API_BASE: &str = "https://openrouter.ai/api/v1";

/// OpenRouter provider implementation
pub struct OpenRouterProvider {
    client: Client,
    api_key: String,
    app_name: Option<String>,
    app_url: Option<String>,
    base_url: String,
}

#[allow(dead_code)]
impl OpenRouterProvider {
    /// Creates a new OpenRouter provider with the given API key
    pub fn new(api_key: String) -> Self {
        Self::with_base_url(api_key, OPENROUTER_API_BASE.to_string())
    }

    /// Creates a new OpenRouter provider with a custom base URL (for testing)
    pub fn with_base_url(api_key: String, base_url: String) -> Self {
        Self {
            client: Client::new(),
            api_key,
            app_name: Some("LocalRouter".to_string()),
            app_url: Some("https://github.com/localrouter/localrouter".to_string()),
            base_url: base_url.trim_end_matches('/').to_string(),
        }
    }

    /// Sets the app name for routing headers
    pub fn with_app_name(mut self, name: String) -> Self {
        self.app_name = Some(name);
        self
    }

    /// Sets the app URL for routing headers
    pub fn with_app_url(mut self, url: String) -> Self {
        self.app_url = Some(url);
        self
    }

    /// Create a new OpenRouter provider from stored API key
    ///
    /// # Arguments
    /// * `provider_name` - The provider name used to store the key (defaults to "openrouter")
    ///
    /// # Returns
    /// * `Ok(Self)` if key exists and provider created successfully
    /// * `Err(AppError)` if key doesn't exist or keyring access fails
    pub fn from_stored_key(provider_name: Option<&str>) -> AppResult<Self> {
        let name = provider_name.unwrap_or("openrouter");
        let api_key = super::key_storage::get_provider_key(name)?.ok_or_else(|| {
            AppError::Provider(format!("No API key found for provider '{}'", name))
        })?;
        Ok(Self::new(api_key))
    }

    /// Builds request with authentication and routing headers
    fn build_request(&self, url: &str) -> reqwest::RequestBuilder {
        let mut req = self
            .client
            .get(url)
            .header("Authorization", format!("Bearer {}", self.api_key));

        if let Some(ref app_url) = self.app_url {
            req = req.header("HTTP-Referer", app_url);
        }

        if let Some(ref app_name) = self.app_name {
            req = req.header("X-Title", app_name);
        }

        req
    }

    /// Builds POST request with authentication and routing headers
    fn build_post_request(&self, url: &str) -> reqwest::RequestBuilder {
        let mut req = self
            .client
            .post(url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json");

        if let Some(ref app_url) = self.app_url {
            req = req.header("HTTP-Referer", app_url);
        }

        if let Some(ref app_name) = self.app_name {
            req = req.header("X-Title", app_name);
        }

        req
    }
}

#[async_trait]
#[allow(dead_code)]
impl ModelProvider for OpenRouterProvider {
    fn name(&self) -> &str {
        "openrouter"
    }

    async fn health_check(&self) -> ProviderHealth {
        let start = Instant::now();
        let url = format!("{}/models", self.base_url);

        match self.build_request(&url).send().await {
            Ok(response) => {
                let latency = start.elapsed().as_millis() as u64;
                match response.status() {
                    StatusCode::OK => ProviderHealth {
                        status: HealthStatus::Healthy,
                        latency_ms: Some(latency),
                        last_checked: Utc::now(),
                        error_message: None,
                    },
                    StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => ProviderHealth {
                        status: HealthStatus::Unhealthy,
                        latency_ms: Some(latency),
                        last_checked: Utc::now(),
                        error_message: Some("Authentication failed - invalid API key".to_string()),
                    },
                    status if status.is_server_error() => ProviderHealth {
                        status: HealthStatus::Degraded,
                        latency_ms: Some(latency),
                        last_checked: Utc::now(),
                        error_message: Some(format!("Server error: {}", status)),
                    },
                    status => ProviderHealth {
                        status: HealthStatus::Degraded,
                        latency_ms: Some(latency),
                        last_checked: Utc::now(),
                        error_message: Some(format!("Unexpected status: {}", status)),
                    },
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
        let url = format!("{}/models", self.base_url);

        let response = self
            .build_request(&url)
            .send()
            .await
            .map_err(|e| AppError::Provider(format!("Failed to fetch models: {}", e)))?;

        if !response.status().is_success() {
            return Err(AppError::Provider(format!(
                "OpenRouter API returned status: {}",
                response.status()
            )));
        }

        let models_response: OpenRouterModelsResponse = response
            .json()
            .await
            .map_err(|e| AppError::Provider(format!("Failed to parse models response: {}", e)))?;

        Ok(models_response
            .data
            .into_iter()
            .map(|model| {
                let capabilities = if model.id.contains("vision") {
                    vec![Capability::Chat, Capability::Vision]
                } else {
                    vec![Capability::Chat]
                };

                ModelInfo {
                    id: model.id.clone(),
                    name: model.name.unwrap_or_else(|| model.id.clone()),
                    provider: "openrouter".to_string(),
                    parameter_count: None,
                    context_window: model.context_length,
                    supports_streaming: true,
                    capabilities,
                    detailed_capabilities: None,
                }
                .enrich_with_catalog_by_name() // Use model-only search for multi-provider system
            })
            .collect())
    }

    async fn get_pricing(&self, model: &str) -> AppResult<PricingInfo> {
        let url = format!("{}/models", self.base_url);
        let response = self
            .build_request(&url)
            .send()
            .await
            .map_err(|e| AppError::Provider(format!("Failed to fetch pricing: {}", e)))?;

        if !response.status().is_success() {
            return Err(AppError::Provider(format!(
                "OpenRouter API returned status: {}",
                response.status()
            )));
        }

        let models_response: OpenRouterModelsResponse = response
            .json()
            .await
            .map_err(|e| AppError::Provider(format!("Failed to parse pricing response: {}", e)))?;

        // Find the model in the response
        let model_data = models_response
            .data
            .iter()
            .find(|m| m.id == model)
            .ok_or_else(|| AppError::Provider(format!("Model not found: {}", model)))?;

        Ok(PricingInfo {
            input_cost_per_1k: model_data.pricing.prompt.parse::<f64>().unwrap_or(0.0) * 1000.0,
            output_cost_per_1k: model_data.pricing.completion.parse::<f64>().unwrap_or(0.0)
                * 1000.0,
            currency: "USD".to_string(),
        })
    }

    async fn complete(&self, request: CompletionRequest) -> AppResult<CompletionResponse> {
        let url = format!("{}/chat/completions", self.base_url);

        let openrouter_request = OpenRouterRequest {
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
            .build_post_request(&url)
            .json(&openrouter_request)
            .send()
            .await
            .map_err(|e| AppError::Provider(format!("Failed to send completion request: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            return Err(AppError::Provider(format!(
                "OpenRouter API error ({}): {}",
                status, error_text
            )));
        }

        let openrouter_response: OpenRouterResponse = response.json().await.map_err(|e| {
            AppError::Provider(format!("Failed to parse completion response: {}", e))
        })?;

        // Convert OpenRouter response to our standard format
        Ok(CompletionResponse {
            id: openrouter_response.id,
            object: "chat.completion".to_string(),
            created: openrouter_response.created,
            model: openrouter_response.model,
            provider: self.name().to_string(),
            choices: openrouter_response
                .choices
                .into_iter()
                .map(|choice| CompletionChoice {
                    index: choice.index,
                    message: choice.message,
                    finish_reason: choice.finish_reason,
                    logprobs: None, // OpenRouter does not support logprobs
                })
                .collect(),
            usage: openrouter_response.usage,
            extensions: None,
            routellm_win_rate: None,
        })
    }

    async fn stream_complete(
        &self,
        request: CompletionRequest,
    ) -> AppResult<Pin<Box<dyn Stream<Item = AppResult<CompletionChunk>> + Send>>> {
        let url = format!("{}/chat/completions", self.base_url);

        let openrouter_request = OpenRouterRequest {
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
            .build_post_request(&url)
            .json(&openrouter_request)
            .send()
            .await
            .map_err(|e| {
                AppError::Provider(format!(
                    "Failed to send streaming completion request: {}",
                    e
                ))
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            return Err(AppError::Provider(format!(
                "OpenRouter API error ({}): {}",
                status, error_text
            )));
        }

        let stream = response.bytes_stream().map(|result| {
            result
                .map_err(|e| AppError::Provider(format!("Stream error: {}", e)))
                .and_then(|bytes| {
                    let text = String::from_utf8_lossy(&bytes);

                    // Parse SSE format (data: {...}\n\n)
                    for line in text.lines() {
                        if let Some(json_str) = line.strip_prefix("data: ") {
                            if json_str == "[DONE]" {
                                continue;
                            }

                            let chunk: OpenRouterChunk =
                                serde_json::from_str(json_str).map_err(|e| {
                                    AppError::Provider(format!("Failed to parse chunk: {}", e))
                                })?;

                            return Ok(CompletionChunk {
                                id: chunk.id,
                                object: "chat.completion.chunk".to_string(),
                                created: chunk.created,
                                model: chunk.model,
                                choices: chunk
                                    .choices
                                    .into_iter()
                                    .map(|choice| ChunkChoice {
                                        index: choice.index,
                                        delta: choice.delta,
                                        finish_reason: choice.finish_reason,
                                    })
                                    .collect(),
                                extensions: None,
                            });
                        }
                    }

                    Err(AppError::Provider("No data in stream chunk".to_string()))
                })
        });

        Ok(Box::pin(stream))
    }

    async fn embed(&self, request: super::EmbeddingRequest) -> AppResult<super::EmbeddingResponse> {
        // OpenRouter uses OpenAI-compatible embeddings API
        let input = match request.input {
            super::EmbeddingInput::Single(text) => serde_json::json!(text),
            super::EmbeddingInput::Multiple(texts) => serde_json::json!(texts),
            super::EmbeddingInput::Tokens(_) => {
                return Err(AppError::Provider(
                    "OpenRouter embeddings do not support pre-tokenized input".to_string(),
                ));
            }
        };

        let mut embed_request = serde_json::json!({
            "model": request.model,
            "input": input,
        });

        if let Some(fmt) = request.encoding_format {
            let format_str = match fmt {
                super::EncodingFormat::Float => "float",
                super::EncodingFormat::Base64 => "base64",
            };
            embed_request["encoding_format"] = serde_json::json!(format_str);
        }

        if let Some(dims) = request.dimensions {
            embed_request["dimensions"] = serde_json::json!(dims);
        }

        let response = self
            .client
            .post(format!("{}/embeddings", OPENROUTER_API_BASE))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&embed_request)
            .send()
            .await
            .map_err(|e| AppError::Provider(format!("Request failed: {}", e)))?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());

            return Err(AppError::Provider(format!(
                "OpenRouter API error ({}): {}",
                status, error_text
            )));
        }

        let api_response: serde_json::Value = response
            .json()
            .await
            .map_err(|e| AppError::Provider(format!("Failed to parse response: {}", e)))?;

        // Parse OpenAI-compatible response
        let data = api_response["data"]
            .as_array()
            .ok_or_else(|| AppError::Provider("No data array in response".to_string()))?;

        let embeddings: Vec<super::Embedding> = data
            .iter()
            .map(|item| {
                let embedding_array = item["embedding"].as_array().ok_or_else(|| {
                    AppError::Provider("No embedding array in data item".to_string())
                })?;
                let embedding: Vec<f32> = embedding_array
                    .iter()
                    .map(|v| v.as_f64().unwrap_or(0.0) as f32)
                    .collect();
                Ok(super::Embedding {
                    object: item["object"].as_str().unwrap_or("embedding").to_string(),
                    embedding: Some(embedding),
                    index: item["index"].as_u64().unwrap_or(0) as usize,
                })
            })
            .collect::<AppResult<Vec<_>>>()?;

        let usage = api_response["usage"]
            .as_object()
            .ok_or_else(|| AppError::Provider("No usage in response".to_string()))?;

        Ok(super::EmbeddingResponse {
            object: api_response["object"]
                .as_str()
                .unwrap_or("list")
                .to_string(),
            data: embeddings,
            model: api_response["model"]
                .as_str()
                .unwrap_or(&request.model)
                .to_string(),
            usage: super::EmbeddingUsage {
                prompt_tokens: usage["prompt_tokens"].as_u64().unwrap_or(0) as u32,
                total_tokens: usage["total_tokens"].as_u64().unwrap_or(0) as u32,
            },
        })
    }
}

// OpenRouter API types

#[derive(Debug, Serialize)]
struct OpenRouterRequest {
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
    stream: bool,
}

#[derive(Debug, Deserialize)]
struct OpenRouterResponse {
    id: String,
    created: i64,
    model: String,
    choices: Vec<OpenRouterChoice>,
    usage: TokenUsage,
}

#[derive(Debug, Deserialize)]
struct OpenRouterChoice {
    index: u32,
    message: ChatMessage,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenRouterChunk {
    id: String,
    created: i64,
    model: String,
    choices: Vec<OpenRouterChunkChoice>,
}

#[derive(Debug, Deserialize)]
struct OpenRouterChunkChoice {
    index: u32,
    delta: ChunkDelta,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenRouterModelsResponse {
    data: Vec<OpenRouterModel>,
}

#[derive(Debug, Deserialize)]
struct OpenRouterModel {
    id: String,
    name: Option<String>,
    context_length: u32,
    pricing: OpenRouterPricing,
}

#[derive(Debug, Deserialize)]
struct OpenRouterPricing {
    prompt: String,
    completion: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use lr_providers::{ChatMessageContent, FunctionCall, ToolCall};

    #[test]
    fn test_provider_name() {
        let provider = OpenRouterProvider::new("test-key".to_string());
        assert_eq!(provider.name(), "openrouter");
    }

    #[test]
    fn test_with_app_name() {
        let provider =
            OpenRouterProvider::new("test-key".to_string()).with_app_name("Test App".to_string());
        assert_eq!(provider.app_name, Some("Test App".to_string()));
    }

    #[test]
    fn test_with_app_url() {
        let provider = OpenRouterProvider::new("test-key".to_string())
            .with_app_url("https://example.com".to_string());
        assert_eq!(provider.app_url, Some("https://example.com".to_string()));
    }

    // Integration tests require a valid API key
    #[tokio::test]
    #[ignore]
    async fn test_health_check() {
        let api_key = std::env::var("OPENROUTER_API_KEY").expect("OPENROUTER_API_KEY not set");
        let provider = OpenRouterProvider::new(api_key);
        let health = provider.health_check().await;
        assert_eq!(health.status, HealthStatus::Healthy);
    }

    #[tokio::test]
    #[ignore]
    async fn test_list_models() {
        let api_key = std::env::var("OPENROUTER_API_KEY").expect("OPENROUTER_API_KEY not set");
        let provider = OpenRouterProvider::new(api_key);
        let models = provider.list_models().await.unwrap();
        assert!(!models.is_empty());
        assert!(models.iter().any(|m| m.provider == "openrouter"));
    }

    #[tokio::test]
    #[ignore]
    async fn test_get_pricing() {
        let api_key = std::env::var("OPENROUTER_API_KEY").expect("OPENROUTER_API_KEY not set");
        let provider = OpenRouterProvider::new(api_key);
        let pricing = provider.get_pricing("openai/gpt-3.5-turbo").await.unwrap();
        assert!(pricing.input_cost_per_1k >= 0.0);
        assert!(pricing.output_cost_per_1k >= 0.0);
        assert_eq!(pricing.currency, "USD");
    }

    #[tokio::test]
    #[ignore]
    async fn test_completion() {
        let api_key = std::env::var("OPENROUTER_API_KEY").expect("OPENROUTER_API_KEY not set");
        let provider = OpenRouterProvider::new(api_key);

        let request = CompletionRequest {
            model: "openai/gpt-3.5-turbo".to_string(),
            messages: vec![ChatMessage {
                role: "user".to_string(),
                content: ChatMessageContent::Text(
                    "Say 'Hello, World!' and nothing else.".to_string(),
                ),
                tool_calls: None,
                tool_call_id: None,
                name: None,
            }],
            temperature: Some(0.7),
            max_tokens: Some(50),
            stream: false,
            top_p: None,
            frequency_penalty: None,
            presence_penalty: None,
            stop: None,
            top_k: None,
            seed: None,
            repetition_penalty: None,
            extensions: None,
            tools: None,
            tool_choice: None,
            response_format: None,
            logprobs: None,
            top_logprobs: None,
        };

        let response = provider.complete(request).await.unwrap();
        assert!(!response.choices.is_empty());
        assert!(!response.choices[0].message.content.is_empty());
    }
}
