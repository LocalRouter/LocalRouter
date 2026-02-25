//! llama.cpp (llama-server) provider implementation
//!
//! llama.cpp provides a high-performance local inference server with an OpenAI-compatible API.
//! Default endpoint: http://localhost:8080/v1

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
use lr_types::{AppError, AppResult};

/// llama.cpp provider for local model inference
pub struct LlamaCppProvider {
    base_url: String,
    api_key: Option<String>,
    client: Client,
}

#[allow(dead_code)]
impl LlamaCppProvider {
    /// Creates a new llama.cpp provider with default settings
    pub fn new() -> Self {
        Self {
            base_url: "http://localhost:8080/v1".to_string(),
            api_key: None,
            client: Client::new(),
        }
    }

    /// Creates a new llama.cpp provider with custom base URL
    pub fn with_base_url(base_url: String) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key: None,
            client: Client::new(),
        }
    }

    /// Creates a new llama.cpp provider with optional API key
    pub fn with_api_key(mut self, api_key: Option<String>) -> Self {
        self.api_key = api_key;
        self
    }

    /// Create from configuration
    pub fn from_config(config: Option<&serde_json::Value>) -> AppResult<Self> {
        let base_url = if let Some(cfg) = config {
            cfg.get("base_url")
                .and_then(|v| v.as_str())
                .unwrap_or("http://localhost:8080/v1")
                .to_string()
        } else {
            "http://localhost:8080/v1".to_string()
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
impl Default for LlamaCppProvider {
    fn default() -> Self {
        Self::new()
    }
}

// llama.cpp API response types (OpenAI-compatible)

#[derive(Debug, Deserialize)]
struct LlamaCppModel {
    id: String,
    #[allow(dead_code)]
    object: String,
}

#[derive(Debug, Deserialize)]
struct LlamaCppModelsResponse {
    data: Vec<LlamaCppModel>,
}

#[derive(Debug, Serialize)]
struct LlamaCppChatRequest {
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
struct LlamaCppChatResponse {
    id: String,
    object: String,
    created: i64,
    model: String,
    choices: Vec<LlamaCppChoice>,
    usage: LlamaCppUsage,
}

#[derive(Debug, Deserialize)]
struct LlamaCppChoice {
    index: u32,
    message: ChatMessage,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct LlamaCppUsage {
    prompt_tokens: u32,
    completion_tokens: u32,
    total_tokens: u32,
}

#[derive(Debug, Deserialize)]
struct LlamaCppStreamChunk {
    id: String,
    object: String,
    created: i64,
    model: String,
    choices: Vec<LlamaCppStreamChoice>,
}

#[derive(Debug, Deserialize)]
struct LlamaCppStreamChoice {
    index: u32,
    delta: LlamaCppDelta,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct LlamaCppDelta {
    #[serde(skip_serializing_if = "Option::is_none")]
    role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<String>,
}

#[async_trait]
#[allow(dead_code)]
impl ModelProvider for LlamaCppProvider {
    fn name(&self) -> &str {
        "llamacpp"
    }

    async fn health_check(&self) -> ProviderHealth {
        let start = Instant::now();

        // llama.cpp has a dedicated /health endpoint at the server root (not under /v1)
        let health_url = self
            .base_url
            .trim_end_matches("/v1")
            .trim_end_matches('/')
            .to_string()
            + "/health";

        let mut request = self.client.get(&health_url);

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
                            "llama.cpp API returned status: {}",
                            response.status()
                        )),
                    }
                }
            }
            Err(e) => ProviderHealth {
                status: HealthStatus::Unhealthy,
                latency_ms: None,
                last_checked: Utc::now(),
                error_message: Some(format!("Failed to connect to llama.cpp: {}", e)),
            },
        }
    }

    async fn list_models(&self) -> AppResult<Vec<ModelInfo>> {
        debug!("Fetching llama.cpp models from {}", self.base_url);

        let mut request = self.client.get(format!("{}/models", self.base_url));

        if let Some(auth) = self.auth_header() {
            request = request.header("Authorization", auth);
        }

        let response = request
            .send()
            .await
            .map_err(|e| AppError::Provider(format!("Failed to fetch llama.cpp models: {}", e)))?;

        if !response.status().is_success() {
            return Err(AppError::Provider(format!(
                "llama.cpp API returned status: {}",
                response.status()
            )));
        }

        let models_response: LlamaCppModelsResponse = response.json().await.map_err(|e| {
            AppError::Provider(format!("Failed to parse llama.cpp models response: {}", e))
        })?;

        let models: Vec<ModelInfo> = models_response
            .data
            .into_iter()
            .map(|model| {
                ModelInfo {
                    id: model.id.clone(),
                    name: model.id,
                    provider: "llamacpp".to_string(),
                    parameter_count: None,
                    context_window: 4096,
                    supports_streaming: true,
                    capabilities: vec![Capability::Chat, Capability::Completion],
                    detailed_capabilities: None,
                }
                .enrich_with_catalog_by_name()
            })
            .collect();

        debug!("Found {} llama.cpp models", models.len());
        Ok(models)
    }

    async fn get_pricing(&self, _model: &str) -> AppResult<PricingInfo> {
        Ok(PricingInfo::free())
    }

    async fn complete(&self, request: CompletionRequest) -> AppResult<CompletionResponse> {
        debug!("Sending completion request to llama.cpp: {}", self.base_url);

        let llamacpp_request = LlamaCppChatRequest {
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
            .json(&llamacpp_request);

        if let Some(auth) = self.auth_header() {
            req = req.header("Authorization", auth);
        }

        let response = req
            .send()
            .await
            .map_err(|e| AppError::Provider(format!("llama.cpp request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(AppError::Provider(format!(
                "llama.cpp API error {}: {}",
                status, error_text
            )));
        }

        let llamacpp_response: LlamaCppChatResponse = response.json().await.map_err(|e| {
            AppError::Provider(format!("Failed to parse llama.cpp response: {}", e))
        })?;

        Ok(CompletionResponse {
            id: llamacpp_response.id,
            object: llamacpp_response.object,
            created: llamacpp_response.created,
            model: llamacpp_response.model,
            provider: self.name().to_string(),
            choices: llamacpp_response
                .choices
                .into_iter()
                .map(|choice| CompletionChoice {
                    index: choice.index,
                    message: choice.message,
                    finish_reason: choice.finish_reason,
                    logprobs: None,
                })
                .collect(),
            usage: TokenUsage {
                prompt_tokens: llamacpp_response.usage.prompt_tokens,
                completion_tokens: llamacpp_response.usage.completion_tokens,
                total_tokens: llamacpp_response.usage.total_tokens,
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
            "Sending streaming completion request to llama.cpp: {}",
            self.base_url
        );

        let llamacpp_request = LlamaCppChatRequest {
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
            .json(&llamacpp_request);

        if let Some(auth) = self.auth_header() {
            req = req.header("Authorization", auth);
        }

        let response = req.send().await.map_err(|e| {
            AppError::Provider(format!("llama.cpp streaming request failed: {}", e))
        })?;

        if !response.status().is_success() {
            let status = response.status();
            return Err(AppError::Provider(format!(
                "llama.cpp streaming API error: {}",
                status
            )));
        }

        use futures::StreamExt;

        let stream = response.bytes_stream().filter_map(|result| async move {
            match result {
                Ok(bytes) => {
                    let text = String::from_utf8_lossy(&bytes);

                    for line in text.lines() {
                        if let Some(json_str) = line.strip_prefix("data: ") {
                            if json_str.trim() == "[DONE]" {
                                continue;
                            }

                            match serde_json::from_str::<LlamaCppStreamChunk>(json_str) {
                                Ok(chunk) => {
                                    return Some(Ok(CompletionChunk {
                                        id: chunk.id,
                                        object: chunk.object,
                                        created: chunk.created,
                                        model: chunk.model,
                                        choices: chunk
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
                                        "Failed to parse llama.cpp chunk: {}",
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
        let provider = LlamaCppProvider::new();
        assert_eq!(provider.name(), "llamacpp");
    }

    #[test]
    fn test_default_base_url() {
        let provider = LlamaCppProvider::new();
        assert_eq!(provider.base_url, "http://localhost:8080/v1");
    }

    #[test]
    fn test_custom_base_url() {
        let provider = LlamaCppProvider::with_base_url("http://localhost:9090/v1".to_string());
        assert_eq!(provider.base_url, "http://localhost:9090/v1");
    }

    #[tokio::test]
    async fn test_pricing_is_free() {
        let provider = LlamaCppProvider::new();
        let pricing = provider.get_pricing("any-model").await.unwrap();
        assert_eq!(pricing.input_cost_per_1k, 0.0);
        assert_eq!(pricing.output_cost_per_1k, 0.0);
    }
}
