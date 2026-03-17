//! DeepInfra provider implementation
//!
//! Implements the ModelProvider trait for DeepInfra's platform.
//! DeepInfra offers cost-effective hosting of open-source models.

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

const DEEPINFRA_API_BASE: &str = "https://api.deepinfra.com/v1/openai";

/// DeepInfra provider
pub struct DeepInfraProvider {
    client: Client,
    api_key: String,
}

#[allow(dead_code)]
impl DeepInfraProvider {
    /// Create a new DeepInfra provider with an API key
    pub fn new(api_key: String) -> AppResult<Self> {
        let client = crate::http_client::extended_client()?;

        Ok(Self { client, api_key })
    }

    /// Create a new DeepInfra provider from stored API key
    pub fn from_stored_key(provider_name: Option<&str>) -> AppResult<Self> {
        let name = provider_name.unwrap_or("deepinfra");
        let api_key = super::key_storage::get_provider_key(name)?.ok_or_else(|| {
            AppError::Provider(format!("No API key found for provider '{}'", name))
        })?;
        Self::new(api_key)
    }

    /// Get known model information
    fn get_known_models() -> Vec<ModelInfo> {
        vec![
            ModelInfo {
                id: "meta-llama/Meta-Llama-3.1-405B-Instruct".to_string(),
                name: "Llama 3.1 405B Instruct".to_string(),
                provider: "deepinfra".to_string(),
                parameter_count: Some(405_000_000_000),
                context_window: 32_000,
                supports_streaming: true,
                capabilities: vec![Capability::Chat],
                detailed_capabilities: None,
            },
            ModelInfo {
                id: "meta-llama/Meta-Llama-3.1-70B-Instruct".to_string(),
                name: "Llama 3.1 70B Instruct".to_string(),
                provider: "deepinfra".to_string(),
                parameter_count: Some(70_000_000_000),
                context_window: 128_000,
                supports_streaming: true,
                capabilities: vec![Capability::Chat],
                detailed_capabilities: None,
            },
            ModelInfo {
                id: "meta-llama/Meta-Llama-3.1-8B-Instruct".to_string(),
                name: "Llama 3.1 8B Instruct".to_string(),
                provider: "deepinfra".to_string(),
                parameter_count: Some(8_000_000_000),
                context_window: 128_000,
                supports_streaming: true,
                capabilities: vec![Capability::Chat],
                detailed_capabilities: None,
            },
            ModelInfo {
                id: "Qwen/Qwen2.5-72B-Instruct".to_string(),
                name: "Qwen 2.5 72B Instruct".to_string(),
                provider: "deepinfra".to_string(),
                parameter_count: Some(72_000_000_000),
                context_window: 32_000,
                supports_streaming: true,
                capabilities: vec![Capability::Chat],
                detailed_capabilities: None,
            },
            ModelInfo {
                id: "mistralai/Mixtral-8x7B-Instruct-v0.1".to_string(),
                name: "Mixtral 8x7B Instruct".to_string(),
                provider: "deepinfra".to_string(),
                parameter_count: Some(47_000_000_000),
                context_window: 32_000,
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

// DeepInfra Models API response types (OpenAI-compatible)
#[derive(Debug, Deserialize)]
struct DeepInfraModelsResponse {
    data: Vec<DeepInfraModel>,
}

#[derive(Debug, Deserialize)]
struct DeepInfraModel {
    id: String,
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
impl ModelProvider for DeepInfraProvider {
    fn name(&self) -> &str {
        "deepinfra"
    }

    async fn health_check(&self) -> ProviderHealth {
        let start = Instant::now();

        // Query a single model via /models/{id} instead of listing all models.
        // Accept both 200 (exists) and 404 (retired but API up, auth valid).
        // A bad API key returns 401, correctly treated as unhealthy.
        let result = self
            .client
            .get(format!(
                "{}/models/meta-llama/Meta-Llama-3.1-8B-Instruct",
                DEEPINFRA_API_BASE
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
        let url = format!("{}/models", DEEPINFRA_API_BASE);

        let response = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .send()
            .await
            .map_err(|e| AppError::Provider(format!("Failed to fetch DeepInfra models: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(AppError::Provider(format!(
                "DeepInfra models API error {}: {}",
                status, error_text
            )));
        }

        let models_response: DeepInfraModelsResponse = response.json().await.map_err(|e| {
            AppError::Provider(format!("Failed to parse DeepInfra models response: {}", e))
        })?;

        let models = models_response
            .data
            .into_iter()
            .filter(|m| !m.id.contains("embed") && !m.id.contains("whisper"))
            .map(|m| ModelInfo {
                id: m.id.clone(),
                name: m.id,
                provider: "deepinfra".to_string(),
                parameter_count: None,
                context_window: 32_000,
                supports_streaming: true,
                capabilities: vec![Capability::Chat],
                detailed_capabilities: None,
            })
            .collect();

        Ok(models)
    }

    async fn get_pricing(&self, model: &str) -> AppResult<PricingInfo> {
        // DeepInfra pricing as of 2026-01 (approximate)
        let pricing = if model.contains("405B") {
            PricingInfo {
                input_cost_per_1k: 0.0027,  // $2.7 per 1M tokens
                output_cost_per_1k: 0.0027, // $2.7 per 1M tokens
                currency: "USD".to_string(),
            }
        } else if model.contains("70B") || model.contains("72B") {
            PricingInfo {
                input_cost_per_1k: 0.00059,  // $0.59 per 1M tokens
                output_cost_per_1k: 0.00059, // $0.59 per 1M tokens
                currency: "USD".to_string(),
            }
        } else {
            PricingInfo {
                input_cost_per_1k: 0.00009,  // $0.09 per 1M tokens
                output_cost_per_1k: 0.00009, // $0.09 per 1M tokens
                currency: "USD".to_string(),
            }
        };

        Ok(pricing)
    }

    async fn complete(&self, request: CompletionRequest) -> AppResult<CompletionResponse> {
        let url = format!("{}/chat/completions", DEEPINFRA_API_BASE);

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await
            .map_err(|e| AppError::Provider(format!("DeepInfra request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(AppError::Provider(format!(
                "DeepInfra API error {}: {}",
                status, error_text
            )));
        }

        let deepinfra_response: OpenAIChatResponse = response.json().await.map_err(|e| {
            AppError::Provider(format!("Failed to parse DeepInfra response: {}", e))
        })?;

        Ok(CompletionResponse {
            id: deepinfra_response.id,
            object: deepinfra_response.object,
            created: deepinfra_response.created,
            model: deepinfra_response.model,
            provider: self.name().to_string(),
            choices: deepinfra_response
                .choices
                .into_iter()
                .map(|choice| CompletionChoice {
                    index: choice.index,
                    message: choice.message,
                    finish_reason: choice.finish_reason,
                    logprobs: None, // DeepInfra does not support logprobs
                })
                .collect(),
            usage: deepinfra_response.usage,
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
        let url = format!("{}/chat/completions", DEEPINFRA_API_BASE);

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await
            .map_err(|e| {
                AppError::Provider(format!("DeepInfra streaming request failed: {}", e))
            })?;

        if !response.status().is_success() {
            let status = response.status();
            return Err(AppError::Provider(format!(
                "DeepInfra streaming API error: {}",
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
                            Ok(deepinfra_chunk) => {
                                let chunk = CompletionChunk {
                                    id: deepinfra_chunk.id,
                                    object: deepinfra_chunk.object,
                                    created: deepinfra_chunk.created,
                                    model: deepinfra_chunk.model,
                                    choices: deepinfra_chunk
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
                Err(e) => vec![Err(AppError::Provider(crate::http_client::format_stream_error(&e)))],
            };

            futures::stream::iter(chunks)
        });

        Ok(Box::pin(converted_stream))
    }

    fn supports_transcription(&self) -> bool {
        true
    }

    fn supports_audio_translation(&self) -> bool {
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
            .post(format!("{}/audio/transcriptions", DEEPINFRA_API_BASE))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .multipart(form)
            .send()
            .await
            .map_err(|e| AppError::Provider(format!("DeepInfra request failed: {}", e)))?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            return Err(match status {
                reqwest::StatusCode::UNAUTHORIZED => AppError::Unauthorized,
                reqwest::StatusCode::TOO_MANY_REQUESTS => AppError::RateLimitExceeded,
                _ => {
                    AppError::Provider(format!("DeepInfra API error ({}): {}", status, error_text))
                }
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
            .post(format!("{}/audio/translations", DEEPINFRA_API_BASE))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .multipart(form)
            .send()
            .await
            .map_err(|e| AppError::Provider(format!("DeepInfra request failed: {}", e)))?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            return Err(match status {
                reqwest::StatusCode::UNAUTHORIZED => AppError::Unauthorized,
                reqwest::StatusCode::TOO_MANY_REQUESTS => AppError::RateLimitExceeded,
                _ => {
                    AppError::Provider(format!("DeepInfra API error ({}): {}", status, error_text))
                }
            });
        }

        let translation: super::AudioTranslationResponse = response
            .json()
            .await
            .map_err(|e| AppError::Provider(format!("Failed to parse response: {}", e)))?;

        Ok(translation)
    }

    fn get_feature_support(&self, instance_name: &str) -> super::ProviderFeatureSupport {
        let mut support = super::default_feature_support(self, instance_name);

        for f in &mut support.model_features {
            if f.name == "N Completions" {
                f.support = super::SupportLevel::Partial;
                f.notes = Some("Support depends on the model being used".into());
            }
        }

        support
    }

    fn supports_image_generation(&self) -> bool {
        true
    }

    async fn generate_image(
        &self,
        request: super::ImageGenerationRequest,
    ) -> AppResult<super::ImageGenerationResponse> {
        // DeepInfra uses OpenAI-compatible image generation API
        // Supported models include SDXL, SDXL Turbo
        let mut body = serde_json::json!({
            "model": request.model,
            "prompt": request.prompt,
            "n": request.n.unwrap_or(1),
        });

        if let Some(size) = &request.size {
            body["size"] = serde_json::json!(size);
        }

        if let Some(response_format) = &request.response_format {
            body["response_format"] = serde_json::json!(response_format);
        }

        let response = self
            .client
            .post(format!("{}/images/generations", DEEPINFRA_API_BASE))
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
    use super::*;

    #[test]
    fn test_known_models() {
        let models = DeepInfraProvider::get_known_models();
        assert!(!models.is_empty());
        assert!(models.iter().any(|m| m.id.contains("Llama-3.1-405B")));
    }

    #[tokio::test]
    async fn test_pricing() {
        let provider = DeepInfraProvider::new("test_key".to_string()).unwrap();
        let pricing = provider
            .get_pricing("meta-llama/Meta-Llama-3.1-405B-Instruct")
            .await
            .unwrap();
        assert!(pricing.input_cost_per_1k > 0.0);
    }
}
