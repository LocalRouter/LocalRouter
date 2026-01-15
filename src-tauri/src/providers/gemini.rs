//! Google Gemini provider implementation
//!
//! Provides integration with Google Gemini models via the Gemini API.

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

/// Google Gemini provider
pub struct GeminiProvider {
    client: Client,
    api_key: String,
    base_url: String,
}

impl GeminiProvider {
    /// Creates a new Gemini provider with the given API key
    pub fn new(api_key: String) -> Self {
        Self {
            client: Client::new(),
            api_key,
            base_url: "https://generativelanguage.googleapis.com/v1beta".to_string(),
        }
    }

    /// Creates a new Gemini provider with custom base URL
    pub fn with_base_url(api_key: String, base_url: String) -> Self {
        Self {
            client: Client::new(),
            api_key,
            base_url,
        }
    }

    /// Create a new Gemini provider from stored API key
    ///
    /// # Arguments
    /// * `provider_name` - The provider name used to store the key (defaults to "gemini")
    ///
    /// # Returns
    /// * `Ok(Self)` if key exists and provider created successfully
    /// * `Err(AppError)` if key doesn't exist or keyring access fails
    pub fn from_stored_key(provider_name: Option<&str>) -> AppResult<Self> {
        let name = provider_name.unwrap_or("gemini");
        let api_key = super::key_storage::get_provider_key(name)?
            .ok_or_else(|| AppError::Provider(format!("No API key found for provider '{}'", name)))?;
        Ok(Self::new(api_key))
    }

    /// Convert OpenAI messages to Gemini format
    fn convert_messages_to_gemini(&self, messages: &[ChatMessage]) -> Vec<GeminiContent> {
        let mut gemini_contents = Vec::new();

        for msg in messages {
            // Map OpenAI roles to Gemini roles
            let role = match msg.role.as_str() {
                "system" => {
                    // Gemini doesn't have a system role, prepend to first user message
                    continue;
                }
                "user" => "user",
                "assistant" => "model",
                _ => "user", // Default to user for unknown roles
            };

            gemini_contents.push(GeminiContent {
                role: role.to_string(),
                parts: vec![GeminiPart {
                    text: msg.content.clone(),
                }],
            });
        }

        // Handle system message by prepending to first user message
        if let Some(system_msg) = messages.iter().find(|m| m.role == "system") {
            if let Some(first_user) = gemini_contents.iter_mut().find(|c| c.role == "user") {
                first_user.parts[0].text =
                    format!("{}\n\n{}", system_msg.content, first_user.parts[0].text);
            }
        }

        gemini_contents
    }

    /// Get model pricing information
    fn get_model_pricing(&self, model: &str) -> PricingInfo {
        // Pricing as of 2025 (per 1M tokens, converted to per 1K)
        match model {
            m if m.contains("gemini-1.5-pro") => PricingInfo {
                input_cost_per_1k: 0.00125, // $1.25 per 1M tokens
                output_cost_per_1k: 0.005,  // $5.00 per 1M tokens
                currency: "USD".to_string(),
            },
            m if m.contains("gemini-1.5-flash") => PricingInfo {
                input_cost_per_1k: 0.000075, // $0.075 per 1M tokens
                output_cost_per_1k: 0.0003,  // $0.30 per 1M tokens
                currency: "USD".to_string(),
            },
            m if m.contains("gemini-2.0-flash") => PricingInfo {
                input_cost_per_1k: 0.0, // Free during preview
                output_cost_per_1k: 0.0,
                currency: "USD".to_string(),
            },
            _ => PricingInfo {
                // Default pricing for unknown models
                input_cost_per_1k: 0.001,
                output_cost_per_1k: 0.002,
                currency: "USD".to_string(),
            },
        }
    }
}

#[async_trait]
impl ModelProvider for GeminiProvider {
    fn name(&self) -> &str {
        "gemini"
    }

    async fn health_check(&self) -> ProviderHealth {
        let start = Instant::now();
        let url = format!("{}/models?key={}", self.base_url, self.api_key);

        match self.client.get(&url).send().await {
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
                warn!("Gemini health check failed with status: {}", status_code);
                ProviderHealth {
                    status: HealthStatus::Unhealthy,
                    latency_ms: None,
                    last_checked: Utc::now(),
                    error_message: Some(format!("HTTP {}", status_code)),
                }
            }
            Err(e) => {
                error!("Gemini health check failed: {}", e);
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
        let url = format!("{}/models?key={}", self.base_url, self.api_key);
        debug!("Fetching Gemini models from: {}", url);

        let response =
            self.client.get(&url).send().await.map_err(|e| {
                AppError::Provider(format!("Failed to connect to Gemini API: {}", e))
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(AppError::Provider(format!(
                "Gemini API error {}: {}",
                status, error_text
            )));
        }

        let models_response: GeminiModelsResponse = response
            .json()
            .await
            .map_err(|e| AppError::Provider(format!("Failed to parse Gemini response: {}", e)))?;

        let models: Vec<ModelInfo> = models_response
            .models
            .into_iter()
            .filter(|model| {
                // Only include models that support generateContent
                model
                    .supported_generation_methods
                    .contains(&"generateContent".to_string())
            })
            .map(|model| {
                // Extract capabilities
                let mut capabilities = vec![Capability::Chat, Capability::Completion];

                if model.name.contains("vision") {
                    capabilities.push(Capability::Vision);
                }

                // Parse context window from input/output token limits
                let context_window = model.input_token_limit.unwrap_or(32768);

                ModelInfo {
                    id: model.name.clone(),
                    name: model.display_name.unwrap_or_else(|| model.name.clone()),
                    provider: "gemini".to_string(),
                    parameter_count: None, // Gemini doesn't expose parameter counts
                    context_window,
                    supports_streaming: true,
                    capabilities,
                }
            })
            .collect();

        debug!("Found {} Gemini models", models.len());
        Ok(models)
    }

    async fn get_pricing(&self, model: &str) -> AppResult<PricingInfo> {
        Ok(self.get_model_pricing(model))
    }

    async fn complete(&self, request: CompletionRequest) -> AppResult<CompletionResponse> {
        let model_name = if request.model.starts_with("models/") {
            request.model.clone()
        } else {
            format!("models/{}", request.model)
        };

        let url = format!(
            "{}/{}:generateContent?key={}",
            self.base_url, model_name, self.api_key
        );
        debug!("Sending completion request to Gemini: {}", url);

        let gemini_contents = self.convert_messages_to_gemini(&request.messages);

        let gemini_request = GeminiRequest {
            contents: gemini_contents,
            generation_config: Some(GeminiGenerationConfig {
                temperature: request.temperature,
                max_output_tokens: request.max_tokens,
                top_p: request.top_p,
                stop_sequences: request.stop.clone(),
            }),
        };

        let response = self
            .client
            .post(&url)
            .json(&gemini_request)
            .send()
            .await
            .map_err(|e| AppError::Provider(format!("Gemini request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(AppError::Provider(format!(
                "Gemini API error {}: {}",
                status, error_text
            )));
        }

        let gemini_response: GeminiResponse = response
            .json()
            .await
            .map_err(|e| AppError::Provider(format!("Failed to parse Gemini response: {}", e)))?;

        // Convert Gemini response to OpenAI format
        if gemini_response.candidates.is_empty() {
            return Err(AppError::Provider(
                "No candidates in Gemini response".to_string(),
            ));
        }

        let candidate = &gemini_response.candidates[0];
        let content = candidate
            .content
            .parts
            .iter()
            .map(|p| p.text.clone())
            .collect::<Vec<_>>()
            .join("");

        let finish_reason = match candidate.finish_reason.as_deref() {
            Some("STOP") => "stop",
            Some("MAX_TOKENS") => "length",
            Some("SAFETY") => "content_filter",
            _ => "stop",
        };

        let usage = gemini_response.usage_metadata.as_ref();

        let completion_response = CompletionResponse {
            id: format!("chatcmpl-{}", uuid::Uuid::new_v4()),
            object: "chat.completion".to_string(),
            created: Utc::now().timestamp(),
            model: request.model,
            choices: vec![CompletionChoice {
                index: 0,
                message: ChatMessage {
                    role: "assistant".to_string(),
                    content,
                },
                finish_reason: Some(finish_reason.to_string()),
            }],
            usage: TokenUsage {
                prompt_tokens: usage.map(|u| u.prompt_token_count).unwrap_or(0),
                completion_tokens: usage.map(|u| u.candidates_token_count).unwrap_or(0),
                total_tokens: usage.map(|u| u.total_token_count).unwrap_or(0),
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
        let model_name = if request.model.starts_with("models/") {
            request.model.clone()
        } else {
            format!("models/{}", request.model)
        };

        let url = format!(
            "{}/{}:streamGenerateContent?key={}&alt=sse",
            self.base_url, model_name, self.api_key
        );
        debug!("Sending streaming completion request to Gemini: {}", url);

        let gemini_contents = self.convert_messages_to_gemini(&request.messages);

        let gemini_request = GeminiRequest {
            contents: gemini_contents,
            generation_config: Some(GeminiGenerationConfig {
                temperature: request.temperature,
                max_output_tokens: request.max_tokens,
                top_p: request.top_p,
                stop_sequences: request.stop.clone(),
            }),
        };

        let response = self
            .client
            .post(&url)
            .json(&gemini_request)
            .send()
            .await
            .map_err(|e| AppError::Provider(format!("Gemini streaming request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            return Err(AppError::Provider(format!(
                "Gemini streaming API error: {}",
                status
            )));
        }

        let model = request.model.clone();
        let stream = response.bytes_stream();

        // Convert Gemini SSE format to OpenAI streaming format
        let converted_stream = stream
            .map(move |result| {
                let model = model.clone();
                match result {
                    Ok(bytes) => {
                        let text = String::from_utf8_lossy(&bytes);

                        // Parse SSE format: data: {...}
                        for line in text.lines() {
                            if let Some(json_str) = line.strip_prefix("data: ") {
                                // Remove "data: " prefix

                                match serde_json::from_str::<GeminiResponse>(json_str) {
                                    Ok(gemini_chunk) => {
                                        if gemini_chunk.candidates.is_empty() {
                                            continue;
                                        }

                                        let candidate = &gemini_chunk.candidates[0];
                                        let content = candidate
                                            .content
                                            .parts
                                            .iter()
                                            .map(|p| p.text.clone())
                                            .collect::<Vec<_>>()
                                            .join("");

                                        let finish_reason = match candidate.finish_reason.as_deref()
                                        {
                                            Some("STOP") => Some("stop".to_string()),
                                            Some("MAX_TOKENS") => Some("length".to_string()),
                                            Some("SAFETY") => Some("content_filter".to_string()),
                                            _ => None,
                                        };

                                        let chunk = CompletionChunk {
                                            id: format!("chatcmpl-{}", uuid::Uuid::new_v4()),
                                            object: "chat.completion.chunk".to_string(),
                                            created: Utc::now().timestamp(),
                                            model: model.clone(),
                                            choices: vec![ChunkChoice {
                                                index: 0,
                                                delta: ChunkDelta {
                                                    role: Some("assistant".to_string()),
                                                    content: if !content.is_empty() {
                                                        Some(content)
                                                    } else {
                                                        None
                                                    },
                                                },
                                                finish_reason,
                                            }],
                                        };
                                        return Ok(chunk);
                                    }
                                    Err(e) => {
                                        error!("Failed to parse Gemini stream chunk: {}", e);
                                        continue;
                                    }
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

// Gemini API types

#[derive(Debug, Serialize, Deserialize)]
struct GeminiModelsResponse {
    models: Vec<GeminiModel>,
}

#[derive(Debug, Serialize, Deserialize)]
struct GeminiModel {
    name: String,
    #[serde(rename = "displayName")]
    display_name: Option<String>,
    #[serde(rename = "supportedGenerationMethods")]
    supported_generation_methods: Vec<String>,
    #[serde(rename = "inputTokenLimit")]
    input_token_limit: Option<u32>,
    #[serde(rename = "outputTokenLimit")]
    output_token_limit: Option<u32>,
}

#[derive(Debug, Serialize, Deserialize)]
struct GeminiRequest {
    contents: Vec<GeminiContent>,
    #[serde(rename = "generationConfig", skip_serializing_if = "Option::is_none")]
    generation_config: Option<GeminiGenerationConfig>,
}

#[derive(Debug, Serialize, Deserialize)]
struct GeminiContent {
    role: String,
    parts: Vec<GeminiPart>,
}

#[derive(Debug, Serialize, Deserialize)]
struct GeminiPart {
    text: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct GeminiGenerationConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(rename = "maxOutputTokens", skip_serializing_if = "Option::is_none")]
    max_output_tokens: Option<u32>,
    #[serde(rename = "topP", skip_serializing_if = "Option::is_none")]
    top_p: Option<f32>,
    #[serde(rename = "stopSequences", skip_serializing_if = "Option::is_none")]
    stop_sequences: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Deserialize)]
struct GeminiResponse {
    candidates: Vec<GeminiCandidate>,
    #[serde(rename = "usageMetadata")]
    usage_metadata: Option<GeminiUsageMetadata>,
}

#[derive(Debug, Serialize, Deserialize)]
struct GeminiCandidate {
    content: GeminiContent,
    #[serde(rename = "finishReason")]
    finish_reason: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct GeminiUsageMetadata {
    #[serde(rename = "promptTokenCount")]
    prompt_token_count: u32,
    #[serde(rename = "candidatesTokenCount")]
    candidates_token_count: u32,
    #[serde(rename = "totalTokenCount")]
    total_token_count: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_provider_name() {
        let provider = GeminiProvider::new("test-key".to_string());
        assert_eq!(provider.name(), "gemini");
    }

    #[test]
    fn test_pricing_gemini_pro() {
        let provider = GeminiProvider::new("test-key".to_string());
        let pricing = provider.get_model_pricing("gemini-1.5-pro");
        assert_eq!(pricing.input_cost_per_1k, 0.00125);
        assert_eq!(pricing.output_cost_per_1k, 0.005);
        assert_eq!(pricing.currency, "USD");
    }

    #[test]
    fn test_pricing_gemini_flash() {
        let provider = GeminiProvider::new("test-key".to_string());
        let pricing = provider.get_model_pricing("gemini-1.5-flash");
        assert_eq!(pricing.input_cost_per_1k, 0.000075);
        assert_eq!(pricing.output_cost_per_1k, 0.0003);
    }

    #[test]
    fn test_pricing_gemini_2_flash() {
        let provider = GeminiProvider::new("test-key".to_string());
        let pricing = provider.get_model_pricing("gemini-2.0-flash");
        assert_eq!(pricing.input_cost_per_1k, 0.0);
        assert_eq!(pricing.output_cost_per_1k, 0.0);
    }

    #[test]
    fn test_convert_messages() {
        let provider = GeminiProvider::new("test-key".to_string());
        let messages = vec![
            ChatMessage {
                role: "system".to_string(),
                content: "You are helpful".to_string(),
            },
            ChatMessage {
                role: "user".to_string(),
                content: "Hello".to_string(),
            },
            ChatMessage {
                role: "assistant".to_string(),
                content: "Hi there!".to_string(),
            },
        ];

        let gemini_contents = provider.convert_messages_to_gemini(&messages);

        // System message should be prepended to first user message
        assert_eq!(gemini_contents.len(), 2);
        assert_eq!(gemini_contents[0].role, "user");
        assert!(gemini_contents[0].parts[0].text.contains("You are helpful"));
        assert!(gemini_contents[0].parts[0].text.contains("Hello"));
        assert_eq!(gemini_contents[1].role, "model");
    }

    // Integration tests (require valid API key)
    #[tokio::test]
    #[ignore] // Only run with --ignored flag and valid API key
    async fn test_health_check_integration() {
        let api_key = std::env::var("GEMINI_API_KEY").expect("GEMINI_API_KEY not set");
        let provider = GeminiProvider::new(api_key);
        let health = provider.health_check().await;
        assert_eq!(health.status, HealthStatus::Healthy);
        assert!(health.latency_ms.is_some());
    }

    #[tokio::test]
    #[ignore] // Only run with --ignored flag and valid API key
    async fn test_list_models_integration() {
        let api_key = std::env::var("GEMINI_API_KEY").expect("GEMINI_API_KEY not set");
        let provider = GeminiProvider::new(api_key);
        let models = provider.list_models().await.unwrap();
        assert!(!models.is_empty(), "Expected at least one model");

        for model in models {
            assert_eq!(model.provider, "gemini");
            assert!(model.supports_streaming);
        }
    }

    #[tokio::test]
    #[ignore] // Only run with --ignored flag and valid API key
    async fn test_completion_integration() {
        let api_key = std::env::var("GEMINI_API_KEY").expect("GEMINI_API_KEY not set");
        let provider = GeminiProvider::new(api_key);

        let request = CompletionRequest {
            model: "gemini-1.5-flash".to_string(),
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
