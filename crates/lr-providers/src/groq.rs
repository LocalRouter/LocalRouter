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

use lr_types::{AppError, AppResult};

use super::{
    Capability, ChatMessage, ChunkChoice, ChunkDelta, CompletionChoice, CompletionChunk,
    CompletionRequest, CompletionResponse, HealthStatus, ModelInfo, ModelProvider, PricingInfo,
    ProviderHealth, TokenUsage,
};

const GROQ_API_BASE: &str = "https://api.groq.com/openai/v1";

/// Map audio file extension to MIME type
fn audio_mime_type(file_name: &str) -> String {
    let ext = file_name.rsplit('.').next().unwrap_or("").to_lowercase();
    match ext.as_str() {
        "mp3" => "audio/mpeg",
        "mp4" | "m4a" => "audio/mp4",
        "ogg" | "oga" => "audio/ogg",
        "wav" => "audio/wav",
        "webm" => "audio/webm",
        "flac" => "audio/flac",
        "opus" => "audio/opus",
        _ => "application/octet-stream",
    }
    .to_string()
}

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
        let client = crate::http_client::extended_client()?;

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

// Groq Models API response types (OpenAI-compatible)
#[derive(Debug, Deserialize)]
struct GroqModelsResponse {
    data: Vec<GroqModel>,
}

#[derive(Debug, Deserialize)]
struct GroqModel {
    id: String,
    #[serde(default)]
    context_window: Option<u32>,
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
            Err(e) => {
                let is_rate_limited = matches!(&e, lr_types::errors::AppError::RateLimitExceeded);
                ProviderHealth {
                    status: if is_rate_limited {
                        HealthStatus::Degraded
                    } else {
                        HealthStatus::Unhealthy
                    },
                    latency_ms: None,
                    last_checked: Utc::now(),
                    error_message: Some(if is_rate_limited {
                        "Rate limited (HTTP 429)".to_string()
                    } else {
                        e.to_string()
                    }),
                }
            }
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
            .map_err(|e| AppError::Provider(format!("Failed to fetch Groq models: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(AppError::Provider(format!(
                "Groq models API error {}: {}",
                status, error_text
            )));
        }

        let models_response: GroqModelsResponse = response.json().await.map_err(|e| {
            AppError::Provider(format!("Failed to parse Groq models response: {}", e))
        })?;

        let models = models_response
            .data
            .into_iter()
            .filter(|m| !m.id.contains("distil"))
            .map(|m| {
                let capabilities = if m.id.contains("whisper") {
                    vec![Capability::Audio]
                } else {
                    vec![Capability::Chat, Capability::FunctionCalling]
                };
                ModelInfo {
                    id: m.id.clone(),
                    name: m.id,
                    provider: "groq".to_string(),
                    parameter_count: None,
                    context_window: m.context_window.unwrap_or(8192),
                    supports_streaming: true,
                    capabilities,
                    detailed_capabilities: None,
                }
            })
            .collect();

        Ok(models)
    }

    async fn get_pricing(&self, model: &str) -> AppResult<PricingInfo> {
        // Groq pricing as of 2026-01
        let pricing = match model {
            "llama-3.3-70b-versatile" | "llama-3.1-70b-versatile" | "llama3-70b-8192" => {
                PricingInfo {
                    input_cost_per_1k: 0.00059,  // $0.59 per 1M tokens
                    output_cost_per_1k: 0.00079, // $0.79 per 1M tokens
                    reasoning_cost_per_1k: None,
                    currency: "USD".to_string(),
                }
            }
            "llama-3.1-8b-instant" | "llama3-8b-8192" | "gemma2-9b-it" => PricingInfo {
                input_cost_per_1k: 0.00005,  // $0.05 per 1M tokens
                output_cost_per_1k: 0.00008, // $0.08 per 1M tokens
                reasoning_cost_per_1k: None,
                currency: "USD".to_string(),
            },
            "mixtral-8x7b-32768" => PricingInfo {
                input_cost_per_1k: 0.00024,  // $0.24 per 1M tokens
                output_cost_per_1k: 0.00024, // $0.24 per 1M tokens
                reasoning_cost_per_1k: None,
                currency: "USD".to_string(),
            },
            _ => PricingInfo {
                // Default fallback pricing
                input_cost_per_1k: 0.0001,
                output_cost_per_1k: 0.0001,
                reasoning_cost_per_1k: None,
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
            .map_err(|e| AppError::Provider(format!("Groq streaming request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(AppError::Provider(format!(
                "Groq streaming API error {}: {}",
                status, error_text
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
                Err(e) => vec![Err(AppError::Provider(
                    crate::http_client::format_stream_error(&e),
                ))],
            };

            futures::stream::iter(chunks)
        });

        Ok(Box::pin(converted_stream))
    }

    async fn transcribe(
        &self,
        request: super::AudioTranscriptionRequest,
    ) -> AppResult<super::AudioTranscriptionResponse> {
        let mut form = reqwest::multipart::Form::new();

        // Add the audio file
        let file_part = reqwest::multipart::Part::bytes(request.file)
            .file_name(request.file_name.clone())
            .mime_str(&audio_mime_type(&request.file_name))
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
            .map_err(|e| AppError::Provider(format!("Groq transcription request failed: {}", e)))?;

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
                    "Groq transcription API error ({}): {}",
                    status, error_text
                )),
            });
        }

        let transcription: super::AudioTranscriptionResponse =
            response.json().await.map_err(|e| {
                AppError::Provider(format!(
                    "Failed to parse Groq transcription response: {}",
                    e
                ))
            })?;

        Ok(transcription)
    }

    async fn translate_audio(
        &self,
        request: super::AudioTranslationRequest,
    ) -> AppResult<super::AudioTranslationResponse> {
        let mut form = reqwest::multipart::Form::new();

        // Add the audio file
        let file_part = reqwest::multipart::Part::bytes(request.file)
            .file_name(request.file_name.clone())
            .mime_str(&audio_mime_type(&request.file_name))
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
            .header("Authorization", format!("Bearer {}", self.api_key))
            .multipart(form)
            .send()
            .await
            .map_err(|e| {
                AppError::Provider(format!("Groq audio translation request failed: {}", e))
            })?;

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
                    "Groq audio translation API error ({}): {}",
                    status, error_text
                )),
            });
        }

        let translation: super::AudioTranslationResponse = response.json().await.map_err(|e| {
            AppError::Provider(format!(
                "Failed to parse Groq audio translation response: {}",
                e
            ))
        })?;

        Ok(translation)
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
            .map_err(|e| AppError::Provider(format!("Groq speech request failed: {}", e)))?;

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
                    "Groq speech API error ({}): {}",
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
            .map_err(|e| AppError::Provider(format!("Failed to read Groq audio data: {}", e)))?
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

    fn get_feature_support(&self, instance_name: &str) -> super::ProviderFeatureSupport {
        let mut support = super::default_feature_support(self, instance_name);

        for f in &mut support.model_features {
            match f.name.as_str() {
                "N Completions" => {
                    f.support = super::SupportLevel::Partial;
                    f.notes = Some("Supported on some Groq models".into());
                }
                "Parallel Tool Calls" => {
                    f.support = super::SupportLevel::Supported;
                    f.notes = Some("Groq supports parallel tool calling".into());
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
