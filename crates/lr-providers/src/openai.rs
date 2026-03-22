//! OpenAI provider implementation

use super::{
    Capability, ChatMessage, ChunkChoice, ChunkDelta, CompletionChoice, CompletionChunk,
    CompletionRequest, CompletionResponse, HealthStatus, ModelInfo, ModelProvider, PricingInfo,
    ProviderHealth, TokenUsage,
};
use async_trait::async_trait;
use chrono::Utc;
use futures::stream::{Stream, StreamExt};
use lr_api_keys::{keychain_trait::KeychainStorage, CachedKeychain};
use lr_types::{AppError, AppResult};
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
    base_url: String,
}

#[allow(dead_code)]
impl OpenAIProvider {
    /// Create a new OpenAI provider with the given API key
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            client: crate::http_client::default_client(),
            base_url: OPENAI_API_BASE.to_string(),
        }
    }

    /// Create a new OpenAI provider with a custom base URL (for testing)
    pub fn with_base_url(api_key: String, base_url: String) -> AppResult<Self> {
        Ok(Self {
            api_key,
            client: crate::http_client::default_client(),
            base_url,
        })
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
            _ => {
                // Try stripping date suffix (-YYYY-MM-DD) and retrying
                let bytes = model.as_bytes();
                if bytes.len() > 11 {
                    let s = bytes.len() - 11;
                    if bytes[s] == b'-'
                        && bytes[s + 1..s + 5].iter().all(u8::is_ascii_digit)
                        && bytes[s + 5] == b'-'
                        && bytes[s + 6..s + 8].iter().all(u8::is_ascii_digit)
                        && bytes[s + 8] == b'-'
                        && bytes[s + 9..s + 11].iter().all(u8::is_ascii_digit)
                    {
                        return Self::get_model_pricing(&model[..s]);
                    }
                }
                None
            }
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
    #[serde(skip_serializing_if = "Option::is_none")]
    n: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    logit_bias: Option<std::collections::HashMap<String, f32>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    parallel_tool_calls: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    service_tier: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    store: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    metadata: Option<std::collections::HashMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    modalities: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    audio: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    prediction: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_effort: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenAIChatResponse {
    id: String,
    object: String,
    created: i64,
    model: String,
    choices: Vec<OpenAIChoice>,
    usage: OpenAIUsage,
    #[serde(default)]
    system_fingerprint: Option<String>,
    #[serde(default)]
    service_tier: Option<String>,
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

/// Determine MIME type for an audio file based on its extension.
fn audio_mime_type(file_name: &str) -> String {
    let ext = file_name.rsplit('.').next().unwrap_or("").to_lowercase();
    match ext.as_str() {
        "mp3" => "audio/mpeg",
        "mp4" | "m4a" => "audio/mp4",
        "mpeg" => "audio/mpeg",
        "mpga" => "audio/mpeg",
        "ogg" | "oga" => "audio/ogg",
        "wav" => "audio/wav",
        "webm" => "audio/webm",
        "flac" => "audio/flac",
        _ => "application/octet-stream",
    }
    .to_string()
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
            .get(format!("{}/models", self.base_url))
            .header("Authorization", self.auth_header())
            .send()
            .await;

        let latency_ms = start.elapsed().as_millis() as u64;

        match result {
            Ok(response) => {
                let status = response.status();
                if status.is_success() {
                    ProviderHealth {
                        status: HealthStatus::Healthy,
                        latency_ms: Some(latency_ms),
                        last_checked: Utc::now(),
                        error_message: None,
                    }
                } else if status.as_u16() == 429 {
                    ProviderHealth {
                        status: HealthStatus::Degraded,
                        latency_ms: Some(latency_ms),
                        last_checked: Utc::now(),
                        error_message: Some("Rate limited (HTTP 429)".to_string()),
                    }
                } else if status.is_server_error() {
                    ProviderHealth {
                        status: HealthStatus::Degraded,
                        latency_ms: Some(latency_ms),
                        last_checked: Utc::now(),
                        error_message: Some(format!("Server error (HTTP {})", status)),
                    }
                } else {
                    ProviderHealth {
                        status: HealthStatus::Unhealthy,
                        latency_ms: Some(latency_ms),
                        last_checked: Utc::now(),
                        error_message: Some(format!("API returned status: {}", status)),
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
            .get(format!("{}/models", self.base_url))
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
        if let Some(catalog_model) = lr_catalog::find_model("openai", model) {
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
            n: request.n,
            logit_bias: request.logit_bias,
            parallel_tool_calls: request.parallel_tool_calls,
            service_tier: request.service_tier,
            store: request.store,
            metadata: request.metadata,
            modalities: request.modalities,
            audio: request.audio,
            prediction: request.prediction,
            reasoning_effort: request.reasoning_effort,
        };

        let response = self
            .client
            .post(format!("{}/chat/completions", self.base_url))
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
            system_fingerprint: openai_response.system_fingerprint,
            service_tier: openai_response.service_tier,
            extensions: None,
            routellm_win_rate: None,
            request_usage_entries: None,
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
            n: request.n,
            logit_bias: request.logit_bias,
            parallel_tool_calls: request.parallel_tool_calls,
            service_tier: request.service_tier,
            store: request.store,
            metadata: request.metadata,
            modalities: request.modalities,
            audio: request.audio,
            prediction: request.prediction,
            reasoning_effort: request.reasoning_effort,
        };

        let response = self
            .client
            .post(format!("{}/chat/completions", self.base_url))
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
                Err(e) => vec![Err(AppError::Provider(
                    crate::http_client::format_stream_error(&e),
                ))],
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
            .post(format!("{}/embeddings", self.base_url))
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
            .post(format!("{}/images/generations", self.base_url))
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

    async fn transcribe(
        &self,
        request: super::AudioTranscriptionRequest,
    ) -> AppResult<super::AudioTranscriptionResponse> {
        let mut form = reqwest::multipart::Form::new();

        // Add the audio file
        let mime_type = audio_mime_type(&request.file_name);
        let file_part = reqwest::multipart::Part::bytes(request.file)
            .file_name(request.file_name)
            .mime_str(&mime_type)
            .map_err(|e| AppError::Provider(format!("Failed to set MIME type: {}", e)))?;
        form = form.part("file", file_part);

        // Add required model field
        form = form.text("model", request.model);

        // Add optional fields
        if let Some(language) = request.language {
            form = form.text("language", language);
        }
        if let Some(prompt) = request.prompt {
            form = form.text("prompt", prompt);
        }
        if let Some(response_format) = request.response_format {
            form = form.text("response_format", response_format);
        }
        if let Some(temperature) = request.temperature {
            form = form.text("temperature", temperature.to_string());
        }
        if let Some(granularities) = request.timestamp_granularities {
            for granularity in granularities {
                form = form.text("timestamp_granularities[]", granularity);
            }
        }

        let response = self
            .client
            .post(format!("{}/audio/transcriptions", self.base_url))
            .header("Authorization", self.auth_header())
            .multipart(form)
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

        let transcription: super::AudioTranscriptionResponse = response
            .json()
            .await
            .map_err(|e| AppError::Provider(format!("Failed to parse response: {}", e)))?;

        Ok(transcription)
    }

    async fn translate_audio(
        &self,
        request: super::AudioTranslationRequest,
    ) -> AppResult<super::AudioTranslationResponse> {
        let mut form = reqwest::multipart::Form::new();

        // Add the audio file
        let mime_type = audio_mime_type(&request.file_name);
        let file_part = reqwest::multipart::Part::bytes(request.file)
            .file_name(request.file_name)
            .mime_str(&mime_type)
            .map_err(|e| AppError::Provider(format!("Failed to set MIME type: {}", e)))?;
        form = form.part("file", file_part);

        // Add required model field
        form = form.text("model", request.model);

        // Add optional fields (no language field — translation always outputs English)
        if let Some(prompt) = request.prompt {
            form = form.text("prompt", prompt);
        }
        if let Some(response_format) = request.response_format {
            form = form.text("response_format", response_format);
        }
        if let Some(temperature) = request.temperature {
            form = form.text("temperature", temperature.to_string());
        }

        let response = self
            .client
            .post(format!("{}/audio/translations", self.base_url))
            .header("Authorization", self.auth_header())
            .multipart(form)
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

        let translation: super::AudioTranslationResponse = response
            .json()
            .await
            .map_err(|e| AppError::Provider(format!("Failed to parse response: {}", e)))?;

        Ok(translation)
    }

    async fn speech(&self, request: super::SpeechRequest) -> AppResult<super::SpeechResponse> {
        let response = self
            .client
            .post(format!("{}/audio/speech", self.base_url))
            .header("Authorization", self.auth_header())
            .header("Content-Type", "application/json")
            .json(&request)
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

        // Determine content type from response headers or requested format
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string())
            .unwrap_or_else(|| {
                // Fallback: derive from requested format
                match request.response_format.as_deref() {
                    Some("opus") => "audio/opus".to_string(),
                    Some("aac") => "audio/aac".to_string(),
                    Some("flac") => "audio/flac".to_string(),
                    Some("wav") => "audio/wav".to_string(),
                    Some("pcm") => "audio/pcm".to_string(),
                    _ => "audio/mpeg".to_string(), // mp3 is the default
                }
            });

        let audio_data = response
            .bytes()
            .await
            .map_err(|e| AppError::Provider(format!("Failed to read audio data: {}", e)))?
            .to_vec();

        Ok(super::SpeechResponse {
            audio_data,
            content_type,
        })
    }

    fn supports_transcription(&self) -> bool {
        true
    }

    fn supports_audio_translation(&self) -> bool {
        true
    }

    fn supports_speech(&self) -> bool {
        true
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
    ) -> Option<Box<dyn crate::features::FeatureAdapter>> {
        match feature {
            "reasoning_tokens" => Some(Box::new(
                crate::features::openai_reasoning::OpenAIReasoningAdapter,
            )),
            "structured_outputs" => Some(Box::new(
                crate::features::structured_outputs::StructuredOutputsAdapter,
            )),
            "logprobs" => Some(Box::new(crate::features::logprobs::LogprobsAdapter)),
            "json_mode" => Some(Box::new(crate::features::json_mode::JsonModeAdapter)),
            _ => None,
        }
    }

    fn supports_embeddings(&self) -> bool {
        true
    }

    fn supports_image_generation(&self) -> bool {
        true
    }

    fn get_feature_support(&self, instance_name: &str) -> super::ProviderFeatureSupport {
        let mut support = super::default_feature_support(self, instance_name);

        // Override model features with OpenAI-specific notes
        for f in &mut support.model_features {
            match f.name.as_str() {
                "Function Calling" => {
                    f.support = super::SupportLevel::Supported;
                    f.notes =
                        Some("GPT-4o, GPT-4 Turbo, and GPT-3.5 Turbo support tool calling".into());
                }
                "Vision" => {
                    f.support = super::SupportLevel::Supported;
                    f.notes = Some("GPT-4o and GPT-4 Turbo can process images".into());
                }
                "Reasoning Tokens" => {
                    f.support = super::SupportLevel::Partial;
                    f.notes = Some("Only o1-preview and o1-mini models use reasoning tokens; other models do not".into());
                }
                "Log Probabilities" => {
                    f.notes =
                        Some("Available on GPT-4o and GPT-3.5 Turbo via logprobs parameter".into());
                }
                "Structured Outputs" => {
                    f.notes = Some(
                        "GPT-4o supports strict JSON schema enforcement via response_format".into(),
                    );
                }
                "JSON Mode" => {
                    f.notes =
                        Some("All GPT-4 and GPT-3.5 Turbo models support JSON output mode".into());
                }
                "N Completions" => {
                    f.support = super::SupportLevel::Supported;
                    f.notes = Some("Generate up to 128 completion choices per request".into());
                }
                "Logit Bias" => {
                    f.support = super::SupportLevel::Supported;
                    f.notes = Some("Modify token likelihoods by token ID (-100 to 100)".into());
                }
                "Parallel Tool Calls" => {
                    f.support = super::SupportLevel::Supported;
                    f.notes =
                        Some("Models can generate multiple tool calls in a single response".into());
                }
                "Reasoning Effort" => {
                    f.support = super::SupportLevel::Partial;
                    f.notes = Some(
                        "Only o-series reasoning models support low/medium/high effort".into(),
                    );
                }
                "Predicted Output" => {
                    f.support = super::SupportLevel::Supported;
                    f.notes = Some(
                        "Supply predicted output for faster generation via speculative decoding"
                            .into(),
                    );
                }
                "Service Tier" => {
                    f.support = super::SupportLevel::Supported;
                    f.notes =
                        Some("Select 'auto' or 'default' latency tier for request routing".into());
                }
                "Audio Output" => {
                    f.support = super::SupportLevel::Partial;
                    f.notes = Some(
                        "Audio output via modalities parameter on gpt-4o-audio-preview models only"
                            .into(),
                    );
                }
                _ => {}
            }
        }

        // OpenAI endpoint-specific notes
        for e in &mut support.endpoints {
            match e.name.as_str() {
                "Moderations" => {
                    e.support = super::SupportLevel::NotImplemented;
                    e.notes = Some("OpenAI supports natively via text-moderation-latest; LocalRouter proxy not yet built".into());
                }
                "Responses API" => {
                    e.support = super::SupportLevel::NotImplemented;
                    e.notes =
                        Some("OpenAI supports natively; LocalRouter proxy not yet built".into());
                }
                "Batch Processing" => {
                    e.support = super::SupportLevel::NotImplemented;
                    e.notes = Some(
                        "OpenAI supports native async batches; LocalRouter proxy not yet built"
                            .into(),
                    );
                }
                "Audio Transcription" | "Audio Speech (TTS)" => {
                    e.support = super::SupportLevel::Supported;
                    e.notes = Some(
                        "Whisper for speech-to-text, TTS-1/TTS-1-HD for text-to-speech".into(),
                    );
                }
                "Realtime (WebSocket)" => {
                    e.support = super::SupportLevel::NotImplemented;
                    e.notes = Some("OpenAI supports natively — planned".into());
                }
                _ => {}
            }
        }

        support
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
