//! LocalAI provider implementation
//!
//! LocalAI is an open-source alternative for running LLMs locally with an OpenAI-compatible API.
//! Default endpoint: http://localhost:8080/v1
//!
//! Supports model pulling via POST /models/apply + GET /models/jobs/:uuid

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
    ProviderHealth, PullProgress, TokenUsage,
};
use lr_types::{AppError, AppResult};

/// LocalAI provider for local model inference
pub struct LocalAIProvider {
    base_url: String,
    api_key: Option<String>,
    client: Client,
}

#[allow(dead_code)]
impl LocalAIProvider {
    /// Creates a new LocalAI provider with default settings
    pub fn new() -> Self {
        Self {
            base_url: "http://localhost:8080/v1".to_string(),
            api_key: None,
            client: Client::new(),
        }
    }

    /// Creates a new LocalAI provider with custom base URL
    pub fn with_base_url(base_url: String) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key: None,
            client: Client::new(),
        }
    }

    /// Creates a new LocalAI provider with optional API key
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

    /// Derive the native (non-/v1) base URL for management APIs
    fn native_base_url(&self) -> String {
        self.base_url
            .trim_end_matches('/')
            .trim_end_matches("/v1")
            .to_string()
    }
}

#[allow(dead_code)]
impl Default for LocalAIProvider {
    fn default() -> Self {
        Self::new()
    }
}

// LocalAI API response types (OpenAI-compatible)

#[derive(Debug, Deserialize)]
struct LocalAIModel {
    id: String,
    #[allow(dead_code)]
    object: String,
}

#[derive(Debug, Deserialize)]
struct LocalAIModelsResponse {
    data: Vec<LocalAIModel>,
}

#[derive(Debug, Serialize)]
struct LocalAIChatRequest {
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
struct LocalAIChatResponse {
    id: String,
    object: String,
    created: i64,
    model: String,
    choices: Vec<LocalAIChoice>,
    usage: LocalAIUsage,
}

#[derive(Debug, Deserialize)]
struct LocalAIChoice {
    index: u32,
    message: ChatMessage,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct LocalAIUsage {
    prompt_tokens: u32,
    completion_tokens: u32,
    total_tokens: u32,
}

#[derive(Debug, Deserialize)]
struct LocalAIStreamChunk {
    id: String,
    object: String,
    created: i64,
    model: String,
    choices: Vec<LocalAIStreamChoice>,
}

#[derive(Debug, Deserialize)]
struct LocalAIStreamChoice {
    index: u32,
    delta: LocalAIDelta,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct LocalAIDelta {
    #[serde(skip_serializing_if = "Option::is_none")]
    role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<String>,
}

// LocalAI model pull types

#[derive(Debug, Deserialize)]
struct LocalAIApplyResponse {
    uuid: String,
    #[allow(dead_code)]
    status: Option<String>,
}

#[derive(Debug, Deserialize)]
struct LocalAIJobStatus {
    #[allow(dead_code)]
    uuid: String,
    #[serde(default)]
    progress: f64,
    #[serde(default)]
    processed: bool,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    error: Option<String>,
}

#[async_trait]
#[allow(dead_code)]
impl ModelProvider for LocalAIProvider {
    fn name(&self) -> &str {
        "localai"
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
                            "LocalAI API returned status: {}",
                            response.status()
                        )),
                    }
                }
            }
            Err(e) => ProviderHealth {
                status: HealthStatus::Unhealthy,
                latency_ms: None,
                last_checked: Utc::now(),
                error_message: Some(format!("Failed to connect to LocalAI: {}", e)),
            },
        }
    }

    async fn list_models(&self) -> AppResult<Vec<ModelInfo>> {
        debug!("Fetching LocalAI models from {}", self.base_url);

        let mut request = self.client.get(format!("{}/models", self.base_url));

        if let Some(auth) = self.auth_header() {
            request = request.header("Authorization", auth);
        }

        let response = request
            .send()
            .await
            .map_err(|e| AppError::Provider(format!("Failed to fetch LocalAI models: {}", e)))?;

        if !response.status().is_success() {
            return Err(AppError::Provider(format!(
                "LocalAI API returned status: {}",
                response.status()
            )));
        }

        let models_response: LocalAIModelsResponse = response.json().await.map_err(|e| {
            AppError::Provider(format!("Failed to parse LocalAI models response: {}", e))
        })?;

        let models: Vec<ModelInfo> = models_response
            .data
            .into_iter()
            .map(|model| {
                ModelInfo {
                    id: model.id.clone(),
                    name: model.id,
                    provider: "localai".to_string(),
                    parameter_count: None,
                    context_window: 4096,
                    supports_streaming: true,
                    capabilities: vec![Capability::Chat, Capability::Completion],
                    detailed_capabilities: None,
                }
                .enrich_with_catalog_by_name()
            })
            .collect();

        debug!("Found {} LocalAI models", models.len());
        Ok(models)
    }

    async fn get_pricing(&self, _model: &str) -> AppResult<PricingInfo> {
        Ok(PricingInfo::free())
    }

    fn supports_pull(&self) -> bool {
        true
    }

    async fn pull_model(
        &self,
        model_name: &str,
    ) -> AppResult<Pin<Box<dyn Stream<Item = AppResult<PullProgress>> + Send>>> {
        let native_url = self.native_base_url();
        let apply_url = format!("{}/models/apply", native_url);

        let body = serde_json::json!({
            "id": model_name,
        });

        let mut req = self.client.post(&apply_url).json(&body);
        if let Some(auth) = self.auth_header() {
            req = req.header("Authorization", auth);
        }

        let response = req
            .send()
            .await
            .map_err(|e| AppError::Provider(format!("LocalAI model apply failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let body_text = response.text().await.unwrap_or_default();
            return Err(AppError::Provider(format!(
                "LocalAI model apply failed ({}): {}",
                status, body_text
            )));
        }

        let apply_response: LocalAIApplyResponse = response.json().await.map_err(|e| {
            AppError::Provider(format!("Failed to parse LocalAI apply response: {}", e))
        })?;

        let uuid = apply_response.uuid;
        let job_url = format!("{}/models/jobs/{}", native_url, uuid);
        let client = self.client.clone();
        let auth = self.auth_header();

        let stream = async_stream::stream! {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;

                let mut req = client.get(&job_url);
                if let Some(ref auth_header) = auth {
                    req = req.header("Authorization", auth_header.clone());
                }

                match req.send().await {
                    Ok(resp) => {
                        if !resp.status().is_success() {
                            yield Err(AppError::Provider(format!(
                                "LocalAI job status check failed: {}",
                                resp.status()
                            )));
                            break;
                        }

                        match resp.json::<LocalAIJobStatus>().await {
                            Ok(status) => {
                                if let Some(ref error) = status.error {
                                    yield Err(AppError::Provider(format!(
                                        "LocalAI pull failed: {}",
                                        error
                                    )));
                                    break;
                                }

                                let progress_pct = (status.progress * 100.0) as u64;
                                let message = status.message.unwrap_or_else(|| {
                                    format!("downloading {}%", progress_pct)
                                });

                                if status.processed {
                                    yield Ok(PullProgress {
                                        status: "success".to_string(),
                                        total: Some(100),
                                        completed: Some(100),
                                    });
                                    break;
                                }

                                yield Ok(PullProgress {
                                    status: message,
                                    total: Some(100),
                                    completed: Some(progress_pct),
                                });
                            }
                            Err(e) => {
                                yield Err(AppError::Provider(format!(
                                    "Failed to parse LocalAI job status: {}",
                                    e
                                )));
                                break;
                            }
                        }
                    }
                    Err(e) => {
                        yield Err(AppError::Provider(format!(
                            "LocalAI job status request failed: {}",
                            e
                        )));
                        break;
                    }
                }
            }
        };

        Ok(Box::pin(stream))
    }

    async fn complete(&self, request: CompletionRequest) -> AppResult<CompletionResponse> {
        debug!("Sending completion request to LocalAI: {}", self.base_url);

        let localai_request = LocalAIChatRequest {
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
            .json(&localai_request);

        if let Some(auth) = self.auth_header() {
            req = req.header("Authorization", auth);
        }

        let response = req
            .send()
            .await
            .map_err(|e| AppError::Provider(format!("LocalAI request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(AppError::Provider(format!(
                "LocalAI API error {}: {}",
                status, error_text
            )));
        }

        let localai_response: LocalAIChatResponse = response
            .json()
            .await
            .map_err(|e| AppError::Provider(format!("Failed to parse LocalAI response: {}", e)))?;

        Ok(CompletionResponse {
            id: localai_response.id,
            object: localai_response.object,
            created: localai_response.created,
            model: localai_response.model,
            provider: self.name().to_string(),
            choices: localai_response
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
                prompt_tokens: localai_response.usage.prompt_tokens,
                completion_tokens: localai_response.usage.completion_tokens,
                total_tokens: localai_response.usage.total_tokens,
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
            "Sending streaming completion request to LocalAI: {}",
            self.base_url
        );

        let localai_request = LocalAIChatRequest {
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
            .json(&localai_request);

        if let Some(auth) = self.auth_header() {
            req = req.header("Authorization", auth);
        }

        let response = req
            .send()
            .await
            .map_err(|e| AppError::Provider(format!("LocalAI streaming request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            return Err(AppError::Provider(format!(
                "LocalAI streaming API error: {}",
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

                            match serde_json::from_str::<LocalAIStreamChunk>(json_str) {
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
                                        "Failed to parse LocalAI chunk: {}",
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
        let provider = LocalAIProvider::new();
        assert_eq!(provider.name(), "localai");
    }

    #[test]
    fn test_default_base_url() {
        let provider = LocalAIProvider::new();
        assert_eq!(provider.base_url, "http://localhost:8080/v1");
    }

    #[test]
    fn test_custom_base_url() {
        let provider = LocalAIProvider::with_base_url("http://localhost:9090/v1".to_string());
        assert_eq!(provider.base_url, "http://localhost:9090/v1");
    }

    #[test]
    fn test_native_base_url() {
        let provider = LocalAIProvider::new();
        assert_eq!(provider.native_base_url(), "http://localhost:8080");

        let provider2 = LocalAIProvider::with_base_url("http://myhost:9000/v1/".to_string());
        assert_eq!(provider2.native_base_url(), "http://myhost:9000");
    }

    #[test]
    fn test_supports_pull() {
        let provider = LocalAIProvider::new();
        assert!(provider.supports_pull());
    }

    #[tokio::test]
    async fn test_pricing_is_free() {
        let provider = LocalAIProvider::new();
        let pricing = provider.get_pricing("any-model").await.unwrap();
        assert_eq!(pricing.input_cost_per_1k, 0.0);
        assert_eq!(pricing.output_cost_per_1k, 0.0);
    }
}
