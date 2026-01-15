//! Ollama provider implementation using direct HTTP API
//!
//! Uses direct HTTP calls for all operations to enable comprehensive testing
//! and maintain full control over the OpenAI-compatible format.

use async_trait::async_trait;
use chrono::Utc;
use futures::{Stream, StreamExt};
use ollama_rs::Ollama as OllamaClient;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::pin::Pin;
use std::time::Instant;
use tracing::{debug, error};

use super::{
    Capability, ChatMessage, ChunkChoice, ChunkDelta, CompletionChoice, CompletionChunk,
    CompletionRequest, CompletionResponse, HealthStatus, ModelInfo, ModelProvider, PricingInfo,
    ProviderHealth, TokenUsage,
};
use crate::utils::errors::{AppError, AppResult};

/// Ollama provider using hybrid SDK + HTTP approach
pub struct OllamaProvider {
    sdk_client: OllamaClient,
    http_client: Client,
    base_url: String,
}

impl OllamaProvider {
    /// Creates a new Ollama provider with default settings
    pub fn new() -> Self {
        let base_url = "http://localhost:11434".to_string();
        let sdk_client = OllamaClient::new(base_url.clone(), 11434);

        Self {
            sdk_client,
            http_client: Client::new(),
            base_url,
        }
    }

    /// Creates a new Ollama provider with custom base URL
    pub fn with_base_url(base_url: String) -> Self {
        let port = base_url
            .split(':')
            .next_back()
            .and_then(|p| p.trim_end_matches('/').parse::<u16>().ok())
            .unwrap_or(11434);

        let sdk_client = OllamaClient::new(base_url.clone(), port);

        Self {
            sdk_client,
            http_client: Client::new(),
            base_url,
        }
    }

    /// Create from configuration
    pub fn from_config(config: Option<&serde_json::Value>) -> AppResult<Self> {
        let base_url = if let Some(cfg) = config {
            cfg.get("base_url")
                .and_then(|v| v.as_str())
                .unwrap_or("http://localhost:11434")
                .to_string()
        } else {
            "http://localhost:11434".to_string()
        };

        Ok(Self::with_base_url(base_url))
    }

    /// Create from stored key (no key needed for Ollama)
    pub fn from_stored_key(_provider_name: Option<&str>) -> AppResult<Self> {
        Ok(Self::new())
    }

    /// Parse model size from tags
    fn parse_parameter_count(name: &str) -> Option<u64> {
        let name_lower = name.to_lowercase();

        if name_lower.contains("70b") {
            Some(70_000_000_000)
        } else if name_lower.contains("65b") {
            Some(65_000_000_000)
        } else if name_lower.contains("34b") {
            Some(34_000_000_000)
        } else if name_lower.contains("13b") {
            Some(13_000_000_000)
        } else if name_lower.contains("8b") {
            Some(8_000_000_000)
        } else if name_lower.contains("7b") {
            Some(7_000_000_000)
        } else if name_lower.contains("3b") {
            Some(3_000_000_000)
        } else if name_lower.contains("1b") {
            Some(1_000_000_000)
        } else {
            None
        }
    }
}

impl Default for OllamaProvider {
    fn default() -> Self {
        Self::new()
    }
}

// Ollama API types for HTTP requests
#[derive(Debug, Serialize, Deserialize)]
struct OllamaChatRequest {
    model: String,
    messages: Vec<ChatMessage>,
    #[serde(default)]
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    options: Option<OllamaOptions>,
}

#[derive(Debug, Serialize, Deserialize)]
struct OllamaOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    num_predict: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stop: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Deserialize)]
struct OllamaChatResponse {
    message: ChatMessage,
    #[serde(default)]
    done: bool,
    #[serde(default)]
    final_data: Option<OllamaFinalData>,
}

#[derive(Debug, Serialize, Deserialize)]
struct OllamaFinalData {
    #[serde(default)]
    prompt_eval_count: Option<i64>,
    #[serde(default)]
    eval_count: Option<i64>,
}

#[derive(Debug, Serialize, Deserialize)]
struct OllamaStreamResponse {
    message: ChatMessage,
    #[serde(default)]
    done: bool,
}

// Types for /api/tags endpoint
#[derive(Debug, Serialize, Deserialize)]
struct OllamaTagsResponse {
    models: Vec<OllamaModel>,
}

#[derive(Debug, Serialize, Deserialize)]
struct OllamaModel {
    name: String,
    #[allow(dead_code)]
    modified_at: String,
    #[allow(dead_code)]
    size: i64,
    #[allow(dead_code)]
    digest: String,
    #[serde(default)]
    details: Option<OllamaModelDetails>,
}

#[derive(Debug, Serialize, Deserialize)]
struct OllamaModelDetails {
    #[allow(dead_code)]
    format: Option<String>,
    #[allow(dead_code)]
    family: Option<String>,
    #[allow(dead_code)]
    parameter_size: Option<String>,
}

#[async_trait]
impl ModelProvider for OllamaProvider {
    fn name(&self) -> &str {
        "ollama"
    }

    async fn health_check(&self) -> ProviderHealth {
        let start = Instant::now();

        // Use direct HTTP call instead of SDK to enable testing
        let url = format!("{}/api/tags", self.base_url);
        match self.http_client.get(&url).send().await {
            Ok(response) => {
                let latency = start.elapsed().as_millis() as u64;
                if response.status().is_success() {
                    ProviderHealth {
                        status: HealthStatus::Healthy,
                        latency_ms: Some(latency),
                        last_checked: Utc::now(),
                        error_message: None,
                    }
                } else {
                    ProviderHealth {
                        status: HealthStatus::Unhealthy,
                        latency_ms: Some(latency),
                        last_checked: Utc::now(),
                        error_message: Some(format!("API returned status: {}", response.status())),
                    }
                }
            }
            Err(e) => {
                error!("Ollama health check failed: {}", e);
                ProviderHealth {
                    status: HealthStatus::Unhealthy,
                    latency_ms: None,
                    last_checked: Utc::now(),
                    error_message: Some(e.to_string()),
                }
            }
        }
    }

    async fn list_models(&self) -> AppResult<Vec<ModelInfo>> {
        debug!("Fetching Ollama models using HTTP API");

        // Use direct HTTP call instead of SDK to enable testing
        let url = format!("{}/api/tags", self.base_url);
        let response = self
            .http_client
            .get(&url)
            .send()
            .await
            .map_err(|e| AppError::Provider(format!("Failed to fetch models: {}", e)))?;

        if !response.status().is_success() {
            return Err(AppError::Provider(format!(
                "API returned status: {}",
                response.status()
            )));
        }

        let tags_response: OllamaTagsResponse = response
            .json()
            .await
            .map_err(|e| AppError::Provider(format!("Failed to parse models response: {}", e)))?;

        let models: Vec<ModelInfo> = tags_response
            .models
            .into_iter()
            .map(|model| {
                let parameter_count = Self::parse_parameter_count(&model.name);

                ModelInfo {
                    id: model.name.clone(),
                    name: model.name,
                    provider: "ollama".to_string(),
                    parameter_count,
                    context_window: 4096,
                    supports_streaming: true,
                    capabilities: vec![Capability::Chat, Capability::Completion],
                }
            })
            .collect();

        debug!("Found {} Ollama models", models.len());
        Ok(models)
    }

    async fn get_pricing(&self, _model: &str) -> AppResult<PricingInfo> {
        Ok(PricingInfo::free())
    }

    async fn complete(&self, request: CompletionRequest) -> AppResult<CompletionResponse> {
        let url = format!("{}/api/chat", self.base_url);
        debug!("Sending completion request to Ollama: {}", url);

        let ollama_request = OllamaChatRequest {
            model: request.model.clone(),
            messages: request.messages.clone(),
            stream: false,
            options: Some(OllamaOptions {
                temperature: request.temperature,
                num_predict: request.max_tokens,
                top_p: request.top_p,
                stop: request.stop.clone(),
            }),
        };

        let response = self
            .http_client
            .post(&url)
            .json(&ollama_request)
            .send()
            .await
            .map_err(|e| AppError::Provider(format!("Ollama request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(AppError::Provider(format!(
                "Ollama API error {}: {}",
                status, error_text
            )));
        }

        let ollama_response: OllamaChatResponse = response
            .json()
            .await
            .map_err(|e| AppError::Provider(format!("Failed to parse Ollama response: {}", e)))?;

        let (prompt_tokens, completion_tokens) = if let Some(ref final_data) = ollama_response.final_data {
            (
                final_data.prompt_eval_count.unwrap_or(0) as u32,
                final_data.eval_count.unwrap_or(0) as u32,
            )
        } else {
            (0, 0)
        };

        Ok(CompletionResponse {
            id: format!("chatcmpl-{}", uuid::Uuid::new_v4()),
            object: "chat.completion".to_string(),
            created: Utc::now().timestamp(),
            model: request.model,
            choices: vec![CompletionChoice {
                index: 0,
                message: ollama_response.message,
                finish_reason: Some("stop".to_string()),
            }],
            usage: TokenUsage {
                prompt_tokens,
                completion_tokens,
                total_tokens: prompt_tokens + completion_tokens,
            },
        })
    }

    async fn stream_complete(
        &self,
        request: CompletionRequest,
    ) -> AppResult<Pin<Box<dyn Stream<Item = AppResult<CompletionChunk>> + Send>>> {
        let url = format!("{}/api/chat", self.base_url);
        debug!("Sending streaming completion request to Ollama: {}", url);

        let ollama_request = OllamaChatRequest {
            model: request.model.clone(),
            messages: request.messages.clone(),
            stream: true,
            options: Some(OllamaOptions {
                temperature: request.temperature,
                num_predict: request.max_tokens,
                top_p: request.top_p,
                stop: request.stop.clone(),
            }),
        };

        let response = self
            .http_client
            .post(&url)
            .json(&ollama_request)
            .send()
            .await
            .map_err(|e| AppError::Provider(format!("Ollama streaming request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            return Err(AppError::Provider(format!(
                "Ollama streaming API error: {}",
                status
            )));
        }

        let model = request.model.clone();
        let stream = response.bytes_stream();

        // Track previous content to compute deltas (Ollama sends cumulative content)
        use std::sync::{Arc, Mutex};
        let previous_content = Arc::new(Mutex::new(String::new()));

        // Buffer for incomplete lines across byte chunks
        let line_buffer = Arc::new(Mutex::new(String::new()));

        let converted_stream = stream
            .flat_map(move |result| {
                let model = model.clone();
                let previous_content = previous_content.clone();
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

                            match serde_json::from_str::<OllamaStreamResponse>(&line) {
                                Ok(ollama_chunk) => {
                                    // Ollama sends cumulative content, so we need to compute the delta
                                    let current_content = ollama_chunk.message.content;
                                    let mut prev = previous_content.lock().unwrap();

                                    let is_first_chunk = prev.is_empty();

                                    let delta_content = if current_content.starts_with(&*prev) {
                                        // Extract only the new part
                                        current_content[prev.len()..].to_string()
                                    } else {
                                        // If content doesn't start with previous (shouldn't happen), send full content
                                        error!(
                                            "Ollama content mismatch! Previous: {:?}, Current: {:?}",
                                            prev.as_str(),
                                            current_content.as_str()
                                        );
                                        current_content.clone()
                                    };

                                    // Update previous content
                                    *prev = current_content;

                                    let chunk = CompletionChunk {
                                        id: format!("chatcmpl-{}", uuid::Uuid::new_v4()),
                                        object: "chat.completion.chunk".to_string(),
                                        created: Utc::now().timestamp(),
                                        model: model.clone(),
                                        choices: vec![ChunkChoice {
                                            index: 0,
                                            delta: ChunkDelta {
                                                role: if is_first_chunk && !delta_content.is_empty() {
                                                    Some("assistant".to_string())
                                                } else {
                                                    None
                                                },
                                                content: if !delta_content.is_empty() {
                                                    Some(delta_content)
                                                } else {
                                                    None
                                                },
                                            },
                                            finish_reason: if ollama_chunk.done {
                                                Some("stop".to_string())
                                            } else {
                                                None
                                            },
                                        }],
                                    };
                                    chunks.push(Ok(chunk));
                                }
                                Err(e) => {
                                    error!("Failed to parse Ollama stream chunk: {} - Line: {}", e, line);
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_parameter_count() {
        assert_eq!(
            OllamaProvider::parse_parameter_count("llama3.3:70b"),
            Some(70_000_000_000)
        );
        assert_eq!(
            OllamaProvider::parse_parameter_count("llama3.3:7b"),
            Some(7_000_000_000)
        );
    }

    #[tokio::test]
    async fn test_provider_name() {
        let provider = OllamaProvider::new();
        assert_eq!(provider.name(), "ollama");
    }

    #[tokio::test]
    async fn test_pricing_is_free() {
        let provider = OllamaProvider::new();
        let pricing = provider.get_pricing("any-model").await.unwrap();
        assert_eq!(pricing.input_cost_per_1k, 0.0);
        assert_eq!(pricing.output_cost_per_1k, 0.0);
    }
}
