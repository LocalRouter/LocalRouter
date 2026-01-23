//! OpenAI provider implementation

use super::{
    Capability, ChatMessage, ChunkChoice, ChunkDelta, CompletionChoice, CompletionChunk,
    CompletionRequest, CompletionResponse, HealthStatus, ModelInfo, ModelProvider, PricingInfo,
    ProviderHealth, TokenUsage,
};
use crate::api_keys::{keychain_trait::KeychainStorage, CachedKeychain};
use crate::utils::errors::{AppError, AppResult};
use async_trait::async_trait;
use chrono::Utc;
use futures::stream::{Stream, StreamExt};
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use std::pin::Pin;
use std::time::Instant;
use tracing::{debug, info};

const OPENAI_API_BASE: &str = "https://api.openai.com/v1";
const OAUTH_KEYCHAIN_SERVICE: &str = "LocalRouter-ProviderTokens";
const OAUTH_PROVIDER_ID: &str = "openai-codex";

/// OpenAI provider implementation
pub struct OpenAIProvider {
    api_key: String,
    client: Client,
}

#[allow(dead_code)]
impl OpenAIProvider {
    /// Create a new OpenAI provider with the given API key
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            client: Client::new(),
        }
    }

    /// Create a new OpenAI provider from stored API key
    ///
    /// # Arguments
    /// * `provider_name` - The provider name used to store the key (defaults to "openai")
    ///
    /// # Returns
    /// * `Ok(Self)` if key exists and provider created successfully
    /// * `Err(AppError)` if key doesn't exist or keyring access fails
    pub fn from_stored_key(provider_name: Option<&str>) -> AppResult<Self> {
        let name = provider_name.unwrap_or("openai");
        let api_key = super::key_storage::get_provider_key(name)?.ok_or_else(|| {
            AppError::Provider(format!("No API key found for provider '{}'", name))
        })?;
        Ok(Self::new(api_key))
    }

    /// Create a new OpenAI provider from OAuth tokens or API key (OAuth-first)
    ///
    /// This method checks for OAuth tokens first, and falls back to API key if:
    /// - No OAuth tokens are stored
    /// - OAuth tokens are expired and cannot be refreshed
    ///
    /// # Arguments
    /// * `provider_name` - The provider name used to store the API key (defaults to "openai")
    ///
    /// # Returns
    /// * `Ok(Self)` if either OAuth tokens or API key are available
    /// * `Err(AppError)` if neither OAuth nor API key authentication is available
    pub fn from_oauth_or_key(provider_name: Option<&str>) -> AppResult<Self> {
        let keychain = CachedKeychain::system();

        // Try OAuth first
        if let Ok(Some(access_token)) = keychain.get(
            OAUTH_KEYCHAIN_SERVICE,
            &format!("{}_access_token", OAUTH_PROVIDER_ID),
        ) {
            info!("Using OAuth credentials for OpenAI provider");
            debug!("Loaded OAuth access token from keychain for openai-codex");
            return Ok(Self::new(access_token));
        }

        // Fall back to API key
        debug!("No OAuth credentials found, falling back to API key for OpenAI");
        Self::from_stored_key(provider_name)
    }

    /// Check if OAuth credentials are available for this provider
    ///
    /// # Returns
    /// * `true` if OAuth access token exists in keychain
    /// * `false` otherwise
    pub fn has_oauth_credentials() -> bool {
        let keychain = CachedKeychain::system();
        keychain
            .get(
                OAUTH_KEYCHAIN_SERVICE,
                &format!("{}_access_token", OAUTH_PROVIDER_ID),
            )
            .ok()
            .flatten()
            .is_some()
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
    logprobs: Option<super::Logprobs>,
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
#[allow(dead_code)]
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
            } else if model.id.starts_with("gpt-4o") || model.id.starts_with("o1") {
                128000
            } else if model.id.starts_with("gpt-4") {
                8192
            } else {
                // Default for gpt-3.5 and others
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
                detailed_capabilities: None,
            });
        }

        Ok(models)
    }

    async fn get_pricing(&self, model: &str) -> AppResult<PricingInfo> {
        // Try catalog first (embedded OpenRouter data)
        if let Some(catalog_model) = crate::catalog::find_model("openai", model) {
            tracing::debug!("Using catalog pricing for OpenAI model: {}", model);
            return Ok(PricingInfo {
                input_cost_per_1k: catalog_model.pricing.prompt_cost_per_1k(),
                output_cost_per_1k: catalog_model.pricing.completion_cost_per_1k(),
                currency: catalog_model.pricing.currency.to_string(),
            });
        }

        // Fallback to hardcoded pricing (for models not in catalog)
        if let Some(pricing) = Self::get_model_pricing(model) {
            tracing::debug!("Using fallback pricing for OpenAI model: {}", model);
            return Ok(pricing);
        }

        // Log unmapped models
        tracing::warn!(
            "Model '{}' not found in catalog or fallback pricing (provider: openai)",
            model
        );

        Err(AppError::Provider(format!(
            "Pricing information not available for model: {}",
            model
        )))
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
            provider: self.name().to_string(),
            choices: openai_response
                .choices
                .into_iter()
                .map(|choice| CompletionChoice {
                    index: choice.index,
                    message: choice.message,
                    finish_reason: choice.finish_reason,
                    logprobs: choice.logprobs,
                })
                .collect(),
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

        // Parse SSE (Server-Sent Events) stream with proper line buffering
        let stream = response.bytes_stream();

        // Buffer for incomplete lines across byte chunks
        use std::sync::{Arc, Mutex};
        let line_buffer = Arc::new(Mutex::new(String::new()));

        let converted_stream = stream.flat_map(move |result| {
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
                        if let Some(json_str) = line.strip_prefix("data: ") {
                            // Check for [DONE] marker
                            if json_str.trim() == "[DONE]" {
                                continue;
                            }

                            // Parse JSON chunk
                            match serde_json::from_str::<OpenAIStreamChunk>(json_str) {
                                Ok(openai_chunk) => {
                                    // OpenAI sends delta chunks, not cumulative
                                    let chunk = CompletionChunk {
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
                                    };
                                    chunks.push(Ok(chunk));
                                }
                                Err(e) => {
                                    tracing::error!(
                                        "Failed to parse OpenAI stream chunk: {} - Line: {}",
                                        e,
                                        json_str
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

    async fn embed(&self, request: super::EmbeddingRequest) -> AppResult<super::EmbeddingResponse> {
        // Convert our generic EmbeddingRequest to OpenAI-specific format
        let input = match request.input {
            super::EmbeddingInput::Single(text) => OpenAIEmbeddingInput::Single(text),
            super::EmbeddingInput::Multiple(texts) => OpenAIEmbeddingInput::Multiple(texts),
            super::EmbeddingInput::Tokens(_) => {
                return Err(AppError::Provider(
                    "OpenAI embeddings do not support pre-tokenized input".to_string(),
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

        let response = self
            .client
            .post(format!("{}/embeddings", OPENAI_API_BASE))
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

    async fn generate_image(
        &self,
        request: super::ImageGenerationRequest,
    ) -> AppResult<super::ImageGenerationResponse> {
        // Build the request body
        let mut body = serde_json::json!({
            "model": request.model,
            "prompt": request.prompt,
            "n": request.n.unwrap_or(1),
        });

        if let Some(size) = &request.size {
            body["size"] = serde_json::json!(size);
        }
        if let Some(quality) = &request.quality {
            body["quality"] = serde_json::json!(quality);
        }
        if let Some(style) = &request.style {
            body["style"] = serde_json::json!(style);
        }
        if let Some(response_format) = &request.response_format {
            body["response_format"] = serde_json::json!(response_format);
        }
        if let Some(user) = &request.user {
            body["user"] = serde_json::json!(user);
        }

        let response = self
            .client
            .post(format!("{}/images/generations", OPENAI_API_BASE))
            .header("Authorization", self.auth_header())
            .header("Content-Type", "application/json")
            .json(&body)
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

        let openai_response: serde_json::Value = response
            .json()
            .await
            .map_err(|e| AppError::Provider(format!("Failed to parse response: {}", e)))?;

        // Convert OpenAI response to our generic format
        let created = openai_response["created"]
            .as_i64()
            .unwrap_or_else(|| chrono::Utc::now().timestamp());

        let data: Vec<super::GeneratedImage> = openai_response["data"]
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .map(|item| super::GeneratedImage {
                url: item["url"].as_str().map(|s| s.to_string()),
                b64_json: item["b64_json"].as_str().map(|s| s.to_string()),
                revised_prompt: item["revised_prompt"].as_str().map(|s| s.to_string()),
            })
            .collect();

        Ok(super::ImageGenerationResponse { created, data })
    }

    fn supports_feature(&self, feature: &str) -> bool {
        matches!(
            feature,
            "reasoning_tokens" | "structured_outputs" | "logprobs"
        )
    }

    fn get_feature_adapter(
        &self,
        feature: &str,
    ) -> Option<Box<dyn crate::providers::features::FeatureAdapter>> {
        match feature {
            "reasoning_tokens" => Some(Box::new(
                crate::providers::features::openai_reasoning::OpenAIReasoningAdapter,
            )),
            "structured_outputs" => Some(Box::new(
                crate::providers::features::structured_outputs::StructuredOutputsAdapter,
            )),
            "logprobs" => Some(Box::new(
                crate::providers::features::logprobs::LogprobsAdapter,
            )),
            "json_mode" => Some(Box::new(
                crate::providers::features::json_mode::JsonModeAdapter,
            )),
            _ => None,
        }
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
