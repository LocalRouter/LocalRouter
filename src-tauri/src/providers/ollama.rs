//! Ollama provider implementation
//!
//! Provides integration with local Ollama models via HTTP API.

use async_trait::async_trait;
use chrono::Utc;
use futures::{Stream, StreamExt};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::pin::Pin;
use std::time::Instant;
use tracing::{debug, error, warn};

use super::{
    Capability, ChatMessage, ChunkChoice, ChunkDelta, CompletionChoice, CompletionChunk,
    CompletionRequest, CompletionResponse, HealthStatus, ModelInfo, ModelProvider, PricingInfo,
    ProviderHealth, TokenUsage,
};
use crate::utils::errors::{AppError, AppResult};

/// Ollama provider for local models
pub struct OllamaProvider {
    client: Client,
    base_url: String,
}

impl OllamaProvider {
    /// Creates a new Ollama provider with default settings
    pub fn new() -> Self {
        Self {
            client: Client::new(),
            base_url: "http://localhost:11434".to_string(),
        }
    }

    /// Creates a new Ollama provider with custom base URL
    pub fn with_base_url(base_url: String) -> Self {
        Self {
            client: Client::new(),
            base_url,
        }
    }

    /// Create a new Ollama provider from configuration
    ///
    /// Parses the provider_config JSON to extract Ollama-specific settings.
    ///
    /// # Configuration Format
    /// ```yaml
    /// provider_config:
    ///   base_url: "http://localhost:11434"  # Optional, defaults to localhost:11434
    ///   timeout_seconds: 120                 # Optional, request timeout
    /// ```
    ///
    /// # Arguments
    /// * `config` - The provider_config JSON value from ProviderConfig
    ///
    /// # Returns
    /// * `Ok(Self)` with parsed configuration, or defaults if config is None
    pub fn from_config(config: Option<&serde_json::Value>) -> AppResult<Self> {
        let base_url = if let Some(cfg) = config {
            // Try to extract base_url from config
            cfg.get("base_url")
                .and_then(|v| v.as_str())
                .unwrap_or("http://localhost:11434")
                .to_string()
        } else {
            "http://localhost:11434".to_string()
        };

        Ok(Self::with_base_url(base_url))
    }

    /// Create a new Ollama provider (no stored key needed for local provider)
    ///
    /// Ollama is a local provider and doesn't typically require API keys.
    /// This method exists for API consistency but simply returns a default instance.
    ///
    /// # Returns
    /// * `Ok(Self)` - Always succeeds with default configuration
    pub fn from_stored_key(_provider_name: Option<&str>) -> AppResult<Self> {
        Ok(Self::new())
    }

    /// Parses model size from tags (e.g., "7b", "13b", "70b")
    fn parse_parameter_count(name: &str) -> Option<u64> {
        // Look for patterns like "7b", "13b", "70b" in the model name
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

#[async_trait]
impl ModelProvider for OllamaProvider {
    fn name(&self) -> &str {
        "ollama"
    }

    async fn health_check(&self) -> ProviderHealth {
        let start = Instant::now();

        match self
            .client
            .get(format!("{}/api/tags", self.base_url))
            .send()
            .await
        {
            Ok(response) if response.status().is_success() => {
                let latency = start.elapsed().as_millis() as u64;
                ProviderHealth {
                    status: HealthStatus::Healthy,
                    latency_ms: Some(latency),
                    last_checked: Utc::now(),
                    error_message: None,
                }
            }
            Ok(response) => {
                let status_code = response.status();
                warn!("Ollama health check failed with status: {}", status_code);
                ProviderHealth {
                    status: HealthStatus::Unhealthy,
                    latency_ms: None,
                    last_checked: Utc::now(),
                    error_message: Some(format!("HTTP {}", status_code)),
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
        let url = format!("{}/api/tags", self.base_url);
        debug!("Fetching Ollama models from: {}", url);

        let response = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| AppError::Provider(format!("Failed to connect to Ollama: {}", e)))?;

        if !response.status().is_success() {
            return Err(AppError::Provider(format!(
                "Ollama API returned error: {}",
                response.status()
            )));
        }

        let tags_response: OllamaTagsResponse = response
            .json()
            .await
            .map_err(|e| AppError::Provider(format!("Failed to parse Ollama response: {}", e)))?;

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
                    context_window: 4096, // Default context window, could be read from model details
                    supports_streaming: true,
                    capabilities: vec![Capability::Chat, Capability::Completion],
                }
            })
            .collect();

        debug!("Found {} Ollama models", models.len());
        Ok(models)
    }

    async fn get_pricing(&self, _model: &str) -> AppResult<PricingInfo> {
        // Ollama is local and free
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
            .client
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

        // Convert Ollama response to OpenAI format
        let completion_response = CompletionResponse {
            id: format!("chatcmpl-{}", uuid::Uuid::new_v4()),
            object: "chat.completion".to_string(),
            created: Utc::now().timestamp(),
            model: request.model,
            choices: vec![CompletionChoice {
                index: 0,
                message: ollama_response.message,
                finish_reason: Some(
                    ollama_response
                        .done_reason
                        .unwrap_or_else(|| "stop".to_string()),
                ),
            }],
            usage: TokenUsage {
                prompt_tokens: ollama_response.prompt_eval_count.unwrap_or(0) as u32,
                completion_tokens: ollama_response.eval_count.unwrap_or(0) as u32,
                total_tokens: (ollama_response.prompt_eval_count.unwrap_or(0)
                    + ollama_response.eval_count.unwrap_or(0)) as u32,
            },
        };

        debug!(
            "Completion successful, tokens: {}",
            completion_response.usage.total_tokens
        );
        Ok(completion_response)
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
            .client
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

        // Convert Ollama streaming format to OpenAI format
        let converted_stream = stream
            .map(move |result| {
                let model = model.clone();
                match result {
                    Ok(bytes) => {
                        // Parse each line as JSON
                        let text = String::from_utf8_lossy(&bytes);

                        for line in text.lines() {
                            if line.is_empty() {
                                continue;
                            }

                            match serde_json::from_str::<OllamaStreamResponse>(line) {
                                Ok(ollama_chunk) => {
                                    let chunk = CompletionChunk {
                                        id: format!("chatcmpl-{}", uuid::Uuid::new_v4()),
                                        object: "chat.completion.chunk".to_string(),
                                        created: Utc::now().timestamp(),
                                        model: model.clone(),
                                        choices: vec![ChunkChoice {
                                            index: 0,
                                            delta: ChunkDelta {
                                                role: if ollama_chunk.message.role == "assistant" {
                                                    Some("assistant".to_string())
                                                } else {
                                                    None
                                                },
                                                content: if !ollama_chunk.message.content.is_empty()
                                                {
                                                    Some(ollama_chunk.message.content)
                                                } else {
                                                    None
                                                },
                                            },
                                            finish_reason: if ollama_chunk.done {
                                                Some(
                                                    ollama_chunk
                                                        .done_reason
                                                        .unwrap_or_else(|| "stop".to_string()),
                                                )
                                            } else {
                                                None
                                            },
                                        }],
                                    };
                                    return Ok(chunk);
                                }
                                Err(e) => {
                                    error!("Failed to parse Ollama stream chunk: {}", e);
                                    continue;
                                }
                            }
                        }

                        Err(AppError::Provider("No valid chunk in response".to_string()))
                    }
                    Err(e) => Err(AppError::Provider(format!("Stream error: {}", e))),
                }
            })
            .filter_map(|result| async move {
                match result {
                    Ok(chunk) => Some(Ok(chunk)),
                    Err(e) if e.to_string().contains("No valid chunk") => None,
                    Err(e) => Some(Err(e)),
                }
            });

        Ok(Box::pin(converted_stream))
    }
}

// Ollama API types

#[derive(Debug, Serialize, Deserialize)]
struct OllamaTagsResponse {
    models: Vec<OllamaModel>,
}

#[derive(Debug, Serialize, Deserialize)]
struct OllamaModel {
    name: String,
    #[serde(default)]
    modified_at: Option<String>,
    #[serde(default)]
    size: Option<i64>,
}

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
    done_reason: Option<String>,
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
    #[serde(default)]
    done_reason: Option<String>,
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
            OllamaProvider::parse_parameter_count("llama3.3:13b"),
            Some(13_000_000_000)
        );
        assert_eq!(
            OllamaProvider::parse_parameter_count("llama3.3:7b"),
            Some(7_000_000_000)
        );
        assert_eq!(
            OllamaProvider::parse_parameter_count("llama3.3:3b"),
            Some(3_000_000_000)
        );
        assert_eq!(OllamaProvider::parse_parameter_count("codellama"), None);
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

    // Integration tests (require Ollama to be running)
    #[tokio::test]
    #[ignore] // Only run with --ignored flag
    async fn test_health_check_integration() {
        let provider = OllamaProvider::new();
        let health = provider.health_check().await;
        assert_eq!(health.status, HealthStatus::Healthy);
        assert!(health.latency_ms.is_some());
    }

    #[tokio::test]
    #[ignore] // Only run with --ignored flag
    async fn test_list_models_integration() {
        let provider = OllamaProvider::new();
        let models = provider.list_models().await.unwrap();
        assert!(!models.is_empty(), "Expected at least one model");

        for model in models {
            assert_eq!(model.provider, "ollama");
            assert!(model.supports_streaming);
        }
    }

    #[tokio::test]
    #[ignore] // Only run with --ignored flag
    async fn test_completion_integration() {
        let provider = OllamaProvider::new();

        // First get available models
        let models = provider.list_models().await.unwrap();
        if models.is_empty() {
            panic!("No models available for testing");
        }

        let request = CompletionRequest {
            model: models[0].id.clone(),
            messages: vec![ChatMessage {
                role: "user".to_string(),
                content: "Say hello in one word".to_string(),
            }],
            temperature: Some(0.7),
            max_tokens: Some(10),
            stream: false,
            top_p: None,
            frequency_penalty: None,
            presence_penalty: None,
            stop: None,
        };

        let response = provider.complete(request).await.unwrap();
        assert_eq!(response.choices.len(), 1);
        assert!(!response.choices[0].message.content.is_empty());
    }
}
