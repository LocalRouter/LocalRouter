//! Together AI provider implementation
//!
//! Implements the ModelProvider trait for Together AI's platform.
//! Together AI offers a wide variety of open-source models with fast inference.

use async_trait::async_trait;
use chrono::Utc;
use futures::stream::{Stream, StreamExt};
use reqwest_middleware::ClientWithMiddleware;
use serde::{Deserialize, Serialize};
use std::pin::Pin;
use std::time::Instant;

use crate::openai_compatible::stream_usage;
use lr_types::{AppError, AppResult};

use super::{
    Capability, ChatMessage, ChunkChoice, ChunkDelta, CompletionChoice, CompletionChunk,
    CompletionRequest, CompletionResponse, HealthStatus, ModelInfo, ModelProvider, PricingInfo,
    ProviderHealth, TokenUsage,
};

const TOGETHER_API_BASE: &str = "https://api.together.xyz/v1";

/// Together AI provider
pub struct TogetherAIProvider {
    client: ClientWithMiddleware,
    api_key: String,
    base_url: String,
}

#[allow(dead_code)]
impl TogetherAIProvider {
    /// Create a new Together AI provider with an API key
    pub fn new(api_key: String) -> AppResult<Self> {
        Self::with_base_url(api_key, TOGETHER_API_BASE.to_string())
    }

    /// Create a new Together AI provider with a custom base URL (for testing)
    pub fn with_base_url(api_key: String, base_url: String) -> AppResult<Self> {
        let client = crate::http_client::extended_client()?;

        Ok(Self {
            client,
            api_key,
            base_url: base_url.trim_end_matches('/').to_string(),
        })
    }

    /// Create a new Together AI provider from stored API key
    pub fn from_stored_key(provider_name: Option<&str>) -> AppResult<Self> {
        let name = provider_name.unwrap_or("togetherai");
        let api_key = super::key_storage::get_provider_key(name)?.ok_or_else(|| {
            AppError::Provider(format!("No API key found for provider '{}'", name))
        })?;
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

/// Serialize a chat request for Together. Together takes `logprobs` as the
/// number of alternatives to return (0-20) rather than OpenAI's boolean plus
/// `top_logprobs`, so translate those two fields.
fn together_request_body(request: &CompletionRequest) -> AppResult<serde_json::Value> {
    let mut body = serde_json::to_value(request)
        .map_err(|e| AppError::Provider(format!("Failed to serialize request: {}", e)))?;
    if let Some(obj) = body.as_object_mut() {
        let top = obj.remove("top_logprobs").and_then(|v| v.as_u64());
        match obj.get("logprobs").and_then(|v| v.as_bool()) {
            Some(true) => {
                obj.insert(
                    "logprobs".to_string(),
                    serde_json::json!(top.unwrap_or(1).clamp(1, 20)),
                );
            }
            Some(false) => {
                obj.remove("logprobs");
            }
            None => {}
        }
    }
    Ok(body)
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    logprobs: Option<serde_json::Value>,
}

#[derive(Debug, Serialize, Deserialize)]
struct OpenAIStreamChunk {
    id: String,
    object: String,
    created: i64,
    model: String,
    choices: Vec<OpenAIStreamChoice>,
    /// Upstream usage, reported on the final chunk (or on every chunk,
    /// cumulatively, by some upstreams).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    usage: Option<serde_json::Value>,
}

#[derive(Debug, Serialize, Deserialize)]
struct OpenAIStreamChoice {
    index: u32,
    delta: ChunkDelta,
    finish_reason: Option<String>,
}

// Together AI Models API response types
#[derive(Debug, Deserialize)]
struct TogetherModel {
    id: String,
    #[serde(default)]
    context_length: Option<u32>,
    #[serde(rename = "type")]
    #[serde(default)]
    model_type: Option<String>,
}

/// Derive audio MIME type from file extension
fn audio_mime_type(file_name: &str) -> String {
    let ext = file_name.rsplit('.').next().unwrap_or("").to_lowercase();
    match ext.as_str() {
        "mp3" => "audio/mpeg",
        "mp4" | "m4a" => "audio/mp4",
        "mpeg" | "mpga" => "audio/mpeg",
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
impl ModelProvider for TogetherAIProvider {
    fn name(&self) -> &str {
        "togetherai"
    }

    async fn health_check(&self) -> ProviderHealth {
        let start = Instant::now();

        // Together AI's /v1/models endpoint returns their entire model catalog, which takes 25s+
        // to serialize and transfer. Instead, we query a single model via /v1/models/{id} which
        // responds in ~0.1s. We accept both:
        //   - 200: model exists, API is up, auth is valid
        //   - 404: model was retired, but API is still up and auth is valid
        // A bad or missing API key would return 401, which we correctly treat as unhealthy.
        let result = self
            .client
            .get(format!(
                "{}/models/meta-llama/Meta-Llama-3.1-8B-Instruct-Turbo",
                self.base_url
            ))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .send()
            .await;

        let latency_ms = start.elapsed().as_millis() as u64;

        match result {
            Ok(response) => {
                let status = response.status();
                if status.is_success() || status.as_u16() == 404 {
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
                        error_message: Some(format!("API returned status {}", status)),
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
        let url = format!("{}/models", self.base_url);

        let response = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .send()
            .await
            .map_err(|e| {
                AppError::Provider(format!("Failed to fetch Together AI models: {}", e))
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(AppError::Provider(format!(
                "Together AI models API error {}: {}",
                status, error_text
            )));
        }

        let models_list: Vec<TogetherModel> = response.json().await.map_err(|e| {
            AppError::Provider(format!(
                "Failed to parse Together AI models response: {}",
                e
            ))
        })?;

        let models = models_list
            .into_iter()
            .map(|m| {
                let capabilities = match m.model_type.as_deref() {
                    Some("chat") => vec![Capability::Chat, Capability::FunctionCalling],
                    Some("embedding") => vec![Capability::Embedding],
                    Some("image") => vec![],
                    Some("audio") => vec![Capability::Audio],
                    _ => vec![],
                };
                ModelInfo {
                    id: m.id.clone(),
                    name: m.id,
                    provider: "togetherai".to_string(),
                    parameter_count: None,
                    context_window: m.context_length.unwrap_or(32_000),
                    supports_streaming: true,
                    capabilities,
                    detailed_capabilities: None,
                }
            })
            .collect();

        Ok(models)
    }

    async fn get_pricing(&self, model: &str) -> AppResult<PricingInfo> {
        // Together AI pricing as of 2026-01
        let pricing = if model.contains("405B") {
            PricingInfo {
                input_cost_per_1k: 0.005,  // $5 per 1M tokens
                output_cost_per_1k: 0.015, // $15 per 1M tokens
                reasoning_cost_per_1k: None,
                cache_read_cost_per_1k: None,
                cache_write_cost_per_1k: None,
                currency: "USD".to_string(),
            }
        } else if model.contains("70B") || model.contains("72B") {
            PricingInfo {
                input_cost_per_1k: 0.0009,  // $0.9 per 1M tokens
                output_cost_per_1k: 0.0009, // $0.9 per 1M tokens
                reasoning_cost_per_1k: None,
                cache_read_cost_per_1k: None,
                cache_write_cost_per_1k: None,
                currency: "USD".to_string(),
            }
        } else {
            PricingInfo {
                input_cost_per_1k: 0.0002,  // $0.2 per 1M tokens
                output_cost_per_1k: 0.0002, // $0.2 per 1M tokens
                reasoning_cost_per_1k: None,
                cache_read_cost_per_1k: None,
                cache_write_cost_per_1k: None,
                currency: "USD".to_string(),
            }
        };

        Ok(pricing)
    }

    async fn complete(&self, request: CompletionRequest) -> AppResult<CompletionResponse> {
        let url = format!("{}/chat/completions", self.base_url);
        let body = together_request_body(&request)?;

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
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

        let together_response: OpenAIChatResponse = response.json().await.map_err(|e| {
            AppError::Provider(format!("Failed to parse Together AI response: {}", e))
        })?;

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
                    logprobs: choice
                        .logprobs
                        .as_ref()
                        .and_then(super::Logprobs::from_wire),
                })
                .collect(),
            usage: together_response.usage,
            system_fingerprint: None,
            service_tier: None,
            extensions: None,
            routellm_win_rate: None,
            request_usage_entries: None,
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
            .map_err(|e| {
                AppError::Provider(format!("Together AI streaming request failed: {}", e))
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(AppError::Provider(format!(
                "Together AI streaming API error {}: {}",
                status, error_text
            )));
        }

        let stream = crate::sse_lines::line_batches(response.bytes_stream());

        let converted_stream = stream.flat_map(move |result| {
            let chunks: Vec<AppResult<CompletionChunk>> = match result {
                Ok(lines) => {
                    let mut chunks = Vec::new();

                    for line in lines {
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
                                    usage: together_chunk
                                        .usage
                                        .as_ref()
                                        .and_then(stream_usage::parse_openai_usage),
                                    provider: None,
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
                Err(e) => vec![Err(AppError::Provider(
                    crate::http_client::format_stream_error(&e),
                ))],
            };

            futures::stream::iter(chunks)
        });

        Ok(stream_usage::usage_once_at_end(converted_stream))
    }

    fn supports_transcription(&self) -> bool {
        true
    }

    /// Together returns per-token logprobs when `logprobs` is set.
    fn supports_feature(&self, feature: &str) -> bool {
        feature == "logprobs"
    }

    fn supports_speech(&self) -> bool {
        true
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
            .header("Authorization", format!("Bearer {}", self.api_key))
            .multipart(form)
            .send()
            .await
            .map_err(|e| AppError::Provider(format!("Together AI request failed: {}", e)))?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            return Err(match status {
                reqwest::StatusCode::UNAUTHORIZED => AppError::Unauthorized,
                reqwest::StatusCode::TOO_MANY_REQUESTS => AppError::RateLimitExceeded,
                _ => AppError::Provider(format!(
                    "Together AI API error ({}): {}",
                    status, error_text
                )),
            });
        }

        let transcription: super::AudioTranscriptionResponse = response
            .json()
            .await
            .map_err(|e| AppError::Provider(format!("Failed to parse response: {}", e)))?;

        Ok(transcription)
    }

    async fn speech(&self, request: super::SpeechRequest) -> AppResult<super::SpeechResponse> {
        let response = self
            .client
            .post(format!("{}/audio/speech", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await
            .map_err(|e| AppError::Provider(format!("Together AI request failed: {}", e)))?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            return Err(match status {
                reqwest::StatusCode::UNAUTHORIZED => AppError::Unauthorized,
                reqwest::StatusCode::TOO_MANY_REQUESTS => AppError::RateLimitExceeded,
                _ => AppError::Provider(format!(
                    "Together AI API error ({}): {}",
                    status, error_text
                )),
            });
        }

        // Determine content type from response headers or requested format
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string())
            .unwrap_or_else(|| match request.response_format.as_deref() {
                Some("opus") => "audio/opus".to_string(),
                Some("aac") => "audio/aac".to_string(),
                Some("flac") => "audio/flac".to_string(),
                Some("wav") => "audio/wav".to_string(),
                Some("pcm") => "audio/pcm".to_string(),
                _ => "audio/mpeg".to_string(),
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

    fn get_feature_support(&self, instance_name: &str) -> super::ProviderFeatureSupport {
        let mut support = super::default_feature_support(self, instance_name);

        for f in &mut support.model_features {
            match f.name.as_str() {
                "N Completions" => {
                    f.support = super::SupportLevel::Partial;
                    f.notes = Some("Support depends on the model being used".into());
                }
                "Logit Bias" => {
                    f.support = super::SupportLevel::Partial;
                    f.notes = Some("Support depends on the model being used".into());
                }
                _ => {}
            }
        }

        support
    }

    fn supports_embeddings(&self) -> bool {
        true
    }

    fn supports_image_generation(&self) -> bool {
        true
    }

    async fn embed(&self, request: super::EmbeddingRequest) -> AppResult<super::EmbeddingResponse> {
        // TogetherAI uses OpenAI-compatible embeddings API
        let input = match request.input {
            super::EmbeddingInput::Single(text) => serde_json::json!(text),
            super::EmbeddingInput::Multiple(texts) => serde_json::json!(texts),
            super::EmbeddingInput::Tokens(_) => {
                return Err(AppError::Provider(
                    "TogetherAI embeddings do not support pre-tokenized input".to_string(),
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
            .post(format!("{}/embeddings", self.base_url))
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
                "TogetherAI API error ({}): {}",
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

    async fn generate_image(
        &self,
        request: super::ImageGenerationRequest,
    ) -> AppResult<super::ImageGenerationResponse> {
        // Together AI uses OpenAI-compatible image generation API
        // Supported models include FLUX.1 schnell, FLUX.1 pro, SDXL
        let mut body = serde_json::json!({
            "model": request.model,
            "prompt": request.prompt,
            "n": request.n.unwrap_or(1),
        });

        // Together AI specific parameters
        if let Some(size) = &request.size {
            // Parse size like "1024x1024" into width/height
            if let Some((w, h)) = size.split_once('x') {
                if let (Ok(width), Ok(height)) = (w.parse::<u32>(), h.parse::<u32>()) {
                    body["width"] = serde_json::json!(width);
                    body["height"] = serde_json::json!(height);
                }
            }
        }

        if let Some(response_format) = &request.response_format {
            body["response_format"] = serde_json::json!(response_format);
        }

        let response = self
            .client
            .post(format!("{}/images/generations", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
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
            return Err(AppError::Provider(format!(
                "API error ({}): {}",
                status, error_text
            )));
        }

        let api_response: serde_json::Value = response
            .json()
            .await
            .map_err(|e| AppError::Provider(format!("Failed to parse response: {}", e)))?;

        let created = api_response["created"]
            .as_i64()
            .unwrap_or_else(|| chrono::Utc::now().timestamp());

        let data: Vec<super::GeneratedImage> = api_response["data"]
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
}

#[cfg(test)]
mod tests {

    #[test]
    fn together_body_translates_logprobs_to_integer() {
        let mut req = CompletionRequest::new("m", vec![]);
        req.logprobs = Some(true);
        req.top_logprobs = Some(5);
        let body = together_request_body(&req).unwrap();
        assert_eq!(body["logprobs"], serde_json::json!(5));
        assert!(body.get("top_logprobs").is_none());

        req.top_logprobs = Some(50);
        assert_eq!(
            together_request_body(&req).unwrap()["logprobs"],
            serde_json::json!(20)
        );

        req.logprobs = Some(false);
        req.top_logprobs = None;
        assert!(together_request_body(&req)
            .unwrap()
            .get("logprobs")
            .is_none());

        let plain = CompletionRequest::new("m", vec![]);
        let body = together_request_body(&plain).unwrap();
        assert!(body.get("logprobs").is_none());
        assert_eq!(body["model"], "m");
    }

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
        let pricing = provider
            .get_pricing("meta-llama/Meta-Llama-3.1-405B-Instruct-Turbo")
            .await
            .unwrap();
        assert!(pricing.input_cost_per_1k > 0.0);
    }

    /// Together sends usage on the final chunk without being asked.
    #[tokio::test]
    async fn stream_reports_upstream_usage() {
        use crate::openai_compatible::stream_usage::test_support::*;

        let server = sse_server("/chat/completions", openai_stream(UsageAt::FinishChunk)).await;
        let provider = TogetherAIProvider::with_base_url("k".to_string(), server.uri()).unwrap();
        let stream = provider
            .stream_complete(stream_request("test-model"))
            .await
            .unwrap();
        assert_openai_stream(&collect(stream).await);
        assert!(!asked_for_usage(&received_body(&server).await));
    }
}
