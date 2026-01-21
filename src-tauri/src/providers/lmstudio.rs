//! LM Studio provider implementation
//!
//! LM Studio is a desktop application for running LLMs locally with an OpenAI-compatible API.
//! Default endpoint: http://localhost:1234/v1

use async_trait::async_trait;
use chrono::Utc;
use futures::Stream;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::pin::Pin;
use std::time::Instant;
use tracing::debug;

use super::{
    Capability, ChatMessage, ChunkChoice, ChunkDelta, CompletionChoice, CompletionChunk,
    CompletionRequest, CompletionResponse, HealthStatus, ModelInfo, ModelProvider, PricingInfo,
    ProviderHealth, TokenUsage,
};
use crate::utils::errors::{AppError, AppResult};

/// LM Studio provider for local model inference
pub struct LMStudioProvider {
    base_url: String,
    api_key: Option<String>,
    client: Client,
}

#[allow(dead_code)]
impl LMStudioProvider {
    /// Creates a new LM Studio provider with default settings
    pub fn new() -> Self {
        Self {
            base_url: "http://localhost:1234/v1".to_string(),
            api_key: None,
            client: Client::new(),
        }
    }

    /// Creates a new LM Studio provider with custom base URL
    pub fn with_base_url(base_url: String) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key: None,
            client: Client::new(),
        }
    }

    /// Creates a new LM Studio provider with optional API key
    pub fn with_api_key(mut self, api_key: Option<String>) -> Self {
        self.api_key = api_key;
        self
    }

    /// Create from configuration
    pub fn from_config(config: Option<&serde_json::Value>) -> AppResult<Self> {
        let base_url = if let Some(cfg) = config {
            cfg.get("base_url")
                .and_then(|v| v.as_str())
                .unwrap_or("http://localhost:1234/v1")
                .to_string()
        } else {
            "http://localhost:1234/v1".to_string()
        };

        let api_key = config
            .and_then(|cfg| cfg.get("api_key"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        Ok(Self::with_base_url(base_url).with_api_key(api_key))
    }

    /// Build authorization header if API key is present
    fn auth_header(&self) -> Option<String> {
        self.api_key.as_ref().map(|key| format!("Bearer {}", key))
    }
}

#[allow(dead_code)]
impl Default for LMStudioProvider {
    fn default() -> Self {
        Self::new()
    }
}

// LM Studio API response types (OpenAI-compatible)

#[derive(Debug, Deserialize)]
struct LMStudioModel {
    id: String,
    #[allow(dead_code)]
    object: String,
    #[allow(dead_code)]
    created: i64,
    #[allow(dead_code)]
    owned_by: String,
}

#[derive(Debug, Deserialize)]
struct LMStudioModelsResponse {
    data: Vec<LMStudioModel>,
}

#[derive(Debug, Serialize)]
struct LMStudioChatRequest {
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
struct LMStudioChatResponse {
    id: String,
    object: String,
    created: i64,
    model: String,
    choices: Vec<LMStudioChoice>,
    usage: LMStudioUsage,
}

#[derive(Debug, Deserialize)]
struct LMStudioChoice {
    index: u32,
    message: ChatMessage,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct LMStudioUsage {
    prompt_tokens: u32,
    completion_tokens: u32,
    total_tokens: u32,
}

#[derive(Debug, Deserialize)]
struct LMStudioStreamChunk {
    id: String,
    object: String,
    created: i64,
    model: String,
    choices: Vec<LMStudioStreamChoice>,
}

#[derive(Debug, Deserialize)]
struct LMStudioStreamChoice {
    index: u32,
    delta: LMStudioDelta,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct LMStudioDelta {
    #[serde(skip_serializing_if = "Option::is_none")]
    role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<String>,
}

#[async_trait]
#[allow(dead_code)]
impl ModelProvider for LMStudioProvider {
    fn name(&self) -> &str {
        "lmstudio"
    }

    async fn health_check(&self) -> ProviderHealth {
        let start = Instant::now();

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
                        error_message: Some(format!(
                            "LM Studio API returned status: {}",
                            response.status()
                        )),
                    }
                }
            }
            Err(e) => ProviderHealth {
                status: HealthStatus::Unhealthy,
                latency_ms: None,
                last_checked: Utc::now(),
                error_message: Some(format!("Failed to connect to LM Studio: {}", e)),
            },
        }
    }

    async fn list_models(&self) -> AppResult<Vec<ModelInfo>> {
        debug!("Fetching LM Studio models from {}", self.base_url);

        let mut request = self.client.get(format!("{}/models", self.base_url));

        if let Some(auth) = self.auth_header() {
            request = request.header("Authorization", auth);
        }

        let response = request
            .send()
            .await
            .map_err(|e| AppError::Provider(format!("Failed to fetch LM Studio models: {}", e)))?;

        if !response.status().is_success() {
            return Err(AppError::Provider(format!(
                "LM Studio API returned status: {}",
                response.status()
            )));
        }

        let models_response: LMStudioModelsResponse = response.json().await.map_err(|e| {
            AppError::Provider(format!("Failed to parse LM Studio models response: {}", e))
        })?;

        let models: Vec<ModelInfo> = models_response
            .data
            .into_iter()
            .map(|model| {
                ModelInfo {
                    id: model.id.clone(),
                    name: model.id,
                    provider: "lmstudio".to_string(),
                    parameter_count: None, // LM Studio doesn't expose parameter count
                    context_window: 4096,  // Default, actual value depends on loaded model
                    supports_streaming: true,
                    capabilities: vec![Capability::Chat, Capability::Completion],
                    detailed_capabilities: None,
                }
                .enrich_with_catalog_by_name()
            }) // Use model-only search for multi-provider system
            .collect();

        debug!("Found {} LM Studio models", models.len());
        Ok(models)
    }

    async fn get_pricing(&self, _model: &str) -> AppResult<PricingInfo> {
        // LM Studio is free (local execution)
        Ok(PricingInfo::free())
    }

    async fn complete(&self, request: CompletionRequest) -> AppResult<CompletionResponse> {
        debug!("Sending completion request to LM Studio: {}", self.base_url);

        let lmstudio_request = LMStudioChatRequest {
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

        let mut req = self
            .client
            .post(format!("{}/chat/completions", self.base_url))
            .header("Content-Type", "application/json")
            .json(&lmstudio_request);

        if let Some(auth) = self.auth_header() {
            req = req.header("Authorization", auth);
        }

        let response = req
            .send()
            .await
            .map_err(|e| AppError::Provider(format!("LM Studio request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(AppError::Provider(format!(
                "LM Studio API error {}: {}",
                status, error_text
            )));
        }

        let lmstudio_response: LMStudioChatResponse = response.json().await.map_err(|e| {
            AppError::Provider(format!("Failed to parse LM Studio response: {}", e))
        })?;

        Ok(CompletionResponse {
            id: lmstudio_response.id,
            object: lmstudio_response.object,
            created: lmstudio_response.created,
            model: lmstudio_response.model,
            provider: self.name().to_string(),
            choices: lmstudio_response
                .choices
                .into_iter()
                .map(|choice| CompletionChoice {
                    index: choice.index,
                    message: choice.message,
                    finish_reason: choice.finish_reason,
                    logprobs: None, // LMStudio does not support logprobs
                })
                .collect(),
            usage: TokenUsage {
                prompt_tokens: lmstudio_response.usage.prompt_tokens,
                completion_tokens: lmstudio_response.usage.completion_tokens,
                total_tokens: lmstudio_response.usage.total_tokens,
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
        debug!(
            "Sending streaming completion request to LM Studio: {}",
            self.base_url
        );

        let lmstudio_request = LMStudioChatRequest {
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

        let mut req = self
            .client
            .post(format!("{}/chat/completions", self.base_url))
            .header("Content-Type", "application/json")
            .json(&lmstudio_request);

        if let Some(auth) = self.auth_header() {
            req = req.header("Authorization", auth);
        }

        let response = req.send().await.map_err(|e| {
            AppError::Provider(format!("LM Studio streaming request failed: {}", e))
        })?;

        if !response.status().is_success() {
            let status = response.status();
            return Err(AppError::Provider(format!(
                "LM Studio streaming API error: {}",
                status
            )));
        }

        use futures::StreamExt;

        // Parse SSE (Server-Sent Events) stream
        let stream = response.bytes_stream().filter_map(|result| async move {
            match result {
                Ok(bytes) => {
                    let text = String::from_utf8_lossy(&bytes);

                    // Parse SSE format: "data: {...}\n\n"
                    for line in text.lines() {
                        if let Some(json_str) = line.strip_prefix("data: ") {
                            // Check for [DONE] marker
                            if json_str.trim() == "[DONE]" {
                                continue;
                            }

                            // Parse JSON chunk
                            match serde_json::from_str::<LMStudioStreamChunk>(json_str) {
                                Ok(lmstudio_chunk) => {
                                    return Some(Ok(CompletionChunk {
                                        id: lmstudio_chunk.id,
                                        object: lmstudio_chunk.object,
                                        created: lmstudio_chunk.created,
                                        model: lmstudio_chunk.model,
                                        choices: lmstudio_chunk
                                            .choices
                                            .into_iter()
                                            .map(|choice| ChunkChoice {
                                                index: choice.index,
                                                delta: ChunkDelta {
                                                    role: choice.delta.role,
                                                    content: choice.delta.content,
                                                    tool_calls: None,
                                                },
                                                finish_reason: choice.finish_reason,
                                            })
                                            .collect(),
                                        extensions: None,
                                    }));
                                }
                                Err(e) => {
                                    return Some(Err(AppError::Provider(format!(
                                        "Failed to parse LM Studio chunk: {}",
                                        e
                                    ))));
                                }
                            }
                        }
                    }
                    None
                }
                Err(e) => Some(Err(AppError::Provider(format!("Stream error: {}", e)))),
            }
        });

        Ok(Box::pin(stream))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_provider_name() {
        let provider = LMStudioProvider::new();
        assert_eq!(provider.name(), "lmstudio");
    }

    #[test]
    fn test_default_base_url() {
        let provider = LMStudioProvider::new();
        assert_eq!(provider.base_url, "http://localhost:1234/v1");
    }

    #[test]
    fn test_custom_base_url() {
        let provider = LMStudioProvider::with_base_url("http://localhost:5678/v1".to_string());
        assert_eq!(provider.base_url, "http://localhost:5678/v1");
    }

    #[test]
    fn test_base_url_trailing_slash() {
        let provider = LMStudioProvider::with_base_url("http://localhost:1234/v1/".to_string());
        assert_eq!(provider.base_url, "http://localhost:1234/v1");
    }

    #[test]
    fn test_auth_header_with_key() {
        let provider = LMStudioProvider::new().with_api_key(Some("test-key".to_string()));
        assert_eq!(provider.auth_header(), Some("Bearer test-key".to_string()));
    }

    #[test]
    fn test_auth_header_without_key() {
        let provider = LMStudioProvider::new();
        assert_eq!(provider.auth_header(), None);
    }

    #[tokio::test]
    async fn test_pricing_is_free() {
        let provider = LMStudioProvider::new();
        let pricing = provider.get_pricing("any-model").await.unwrap();
        assert_eq!(pricing.input_cost_per_1k, 0.0);
        assert_eq!(pricing.output_cost_per_1k, 0.0);
    }
}
