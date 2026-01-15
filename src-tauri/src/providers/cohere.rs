//! Cohere provider implementation
//!
//! Implements the ModelProvider trait for Cohere's LLM API.
//! Cohere offers models like Command R+, Command R, and specialized embedding models.

use async_trait::async_trait;
use chrono::Utc;
use futures::stream::{Stream, StreamExt};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::pin::Pin;
use std::time::Instant;

use crate::utils::errors::{AppError, AppResult};

use super::{
    Capability, ChatMessage, ChunkChoice, ChunkDelta, CompletionChoice, CompletionChunk,
    CompletionRequest, CompletionResponse, HealthStatus, ModelInfo, ModelProvider, PricingInfo,
    ProviderHealth, TokenUsage,
};

const COHERE_API_BASE: &str = "https://api.cohere.com/v2";

/// Cohere AI provider
pub struct CohereProvider {
    client: Client,
    api_key: String,
}

impl CohereProvider {
    /// Create a new Cohere provider with an API key
    pub fn new(api_key: String) -> AppResult<Self> {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .map_err(|e| AppError::Provider(format!("Failed to create HTTP client: {}", e)))?;

        Ok(Self { client, api_key })
    }

    /// Create a new Cohere provider from stored API key
    pub fn from_stored_key(provider_name: Option<&str>) -> AppResult<Self> {
        let name = provider_name.unwrap_or("cohere");
        let api_key = super::key_storage::get_provider_key(name)?
            .ok_or_else(|| AppError::Provider(format!("No API key found for provider '{}'", name)))?;
        Self::new(api_key)
    }

    /// Get known model information
    fn get_known_models() -> Vec<ModelInfo> {
        vec![
            ModelInfo {
                id: "command-r-plus".to_string(),
                name: "Command R+".to_string(),
                provider: "cohere".to_string(),
                parameter_count: Some(104_000_000_000),
                context_window: 128_000,
                supports_streaming: true,
                capabilities: vec![Capability::Chat, Capability::FunctionCalling],
            },
            ModelInfo {
                id: "command-r".to_string(),
                name: "Command R".to_string(),
                provider: "cohere".to_string(),
                parameter_count: Some(35_000_000_000),
                context_window: 128_000,
                supports_streaming: true,
                capabilities: vec![Capability::Chat, Capability::FunctionCalling],
            },
            ModelInfo {
                id: "command".to_string(),
                name: "Command".to_string(),
                provider: "cohere".to_string(),
                parameter_count: None,
                context_window: 4096,
                supports_streaming: true,
                capabilities: vec![Capability::Chat],
            },
            ModelInfo {
                id: "command-light".to_string(),
                name: "Command Light".to_string(),
                provider: "cohere".to_string(),
                parameter_count: None,
                context_window: 4096,
                supports_streaming: true,
                capabilities: vec![Capability::Chat],
            },
        ]
    }

    /// Convert OpenAI messages to Cohere format
    fn convert_to_cohere_request(&self, request: &CompletionRequest) -> AppResult<CohereRequest> {
        let mut system_message = None;
        let mut chat_history = Vec::new();
        let mut user_message = String::new();

        for msg in &request.messages {
            match msg.role.as_str() {
                "system" => system_message = Some(msg.content.clone()),
                "user" => user_message = msg.content.clone(),
                "assistant" => chat_history.push(CohereMessage {
                    role: "CHATBOT".to_string(),
                    content: msg.content.clone(),
                }),
                _ => {}
            }
        }

        Ok(CohereRequest {
            model: request.model.clone(),
            message: user_message,
            preamble: system_message,
            chat_history: if chat_history.is_empty() {
                None
            } else {
                Some(chat_history)
            },
            temperature: request.temperature,
            max_tokens: request.max_tokens,
            stream: request.stream,
        })
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct CohereRequest {
    model: String,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    preamble: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    chat_history: Option<Vec<CohereMessage>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(default)]
    stream: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct CohereMessage {
    role: String,
    content: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct CohereResponse {
    id: String,
    message: CohereMessageContent,
    finish_reason: String,
    usage: CohereUsage,
}

#[derive(Debug, Serialize, Deserialize)]
struct CohereMessageContent {
    content: Vec<CohereContent>,
}

#[derive(Debug, Serialize, Deserialize)]
struct CohereContent {
    #[serde(rename = "type")]
    content_type: String,
    text: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct CohereUsage {
    tokens: CohereTokens,
}

#[derive(Debug, Serialize, Deserialize)]
struct CohereTokens {
    input_tokens: u32,
    output_tokens: u32,
}

#[async_trait]
impl ModelProvider for CohereProvider {
    fn name(&self) -> &str {
        "cohere"
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
            Err(e) => ProviderHealth {
                status: HealthStatus::Unhealthy,
                latency_ms: None,
                last_checked: Utc::now(),
                error_message: Some(e.to_string()),
            },
        }
    }

    async fn list_models(&self) -> AppResult<Vec<ModelInfo>> {
        Ok(Self::get_known_models())
    }

    async fn get_pricing(&self, model: &str) -> AppResult<PricingInfo> {
        // Cohere pricing as of 2026-01
        let pricing = match model {
            "command-r-plus" => PricingInfo {
                input_cost_per_1k: 0.003,  // $3 per 1M tokens
                output_cost_per_1k: 0.015, // $15 per 1M tokens
                currency: "USD".to_string(),
            },
            "command-r" => PricingInfo {
                input_cost_per_1k: 0.0005,  // $0.5 per 1M tokens
                output_cost_per_1k: 0.0015, // $1.5 per 1M tokens
                currency: "USD".to_string(),
            },
            "command" => PricingInfo {
                input_cost_per_1k: 0.001,  // $1 per 1M tokens
                output_cost_per_1k: 0.002, // $2 per 1M tokens
                currency: "USD".to_string(),
            },
            "command-light" => PricingInfo {
                input_cost_per_1k: 0.0003,  // $0.3 per 1M tokens
                output_cost_per_1k: 0.0006, // $0.6 per 1M tokens
                currency: "USD".to_string(),
            },
            _ => PricingInfo {
                input_cost_per_1k: 0.001,
                output_cost_per_1k: 0.002,
                currency: "USD".to_string(),
            },
        };

        Ok(pricing)
    }

    async fn complete(&self, request: CompletionRequest) -> AppResult<CompletionResponse> {
        let url = format!("{}/chat", COHERE_API_BASE);

        let cohere_request = self.convert_to_cohere_request(&request)?;

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&cohere_request)
            .send()
            .await
            .map_err(|e| AppError::Provider(format!("Cohere request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(AppError::Provider(format!(
                "Cohere API error {}: {}",
                status, error_text
            )));
        }

        let cohere_response: CohereResponse = response
            .json()
            .await
            .map_err(|e| AppError::Provider(format!("Failed to parse Cohere response: {}", e)))?;

        let content = cohere_response
            .message
            .content
            .into_iter()
            .filter(|c| c.content_type == "text")
            .map(|c| c.text)
            .collect::<Vec<_>>()
            .join("\n");

        Ok(CompletionResponse {
            id: cohere_response.id,
            object: "chat.completion".to_string(),
            created: Utc::now().timestamp(),
            model: request.model,
            choices: vec![CompletionChoice {
                index: 0,
                message: ChatMessage {
                    role: "assistant".to_string(),
                    content,
                },
                finish_reason: Some(cohere_response.finish_reason),
            }],
            usage: TokenUsage {
                prompt_tokens: cohere_response.usage.tokens.input_tokens,
                completion_tokens: cohere_response.usage.tokens.output_tokens,
                total_tokens: cohere_response.usage.tokens.input_tokens
                    + cohere_response.usage.tokens.output_tokens,
            },
        })
    }

    async fn stream_complete(
        &self,
        _request: CompletionRequest,
    ) -> AppResult<Pin<Box<dyn Stream<Item = AppResult<CompletionChunk>> + Send>>> {
        // Cohere streaming has a different API structure - simplified implementation
        Err(AppError::Provider(
            "Streaming not yet implemented for Cohere".to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_known_models() {
        let models = CohereProvider::get_known_models();
        assert!(!models.is_empty());
        assert!(models.iter().any(|m| m.id == "command-r-plus"));
    }

    #[tokio::test]
    async fn test_pricing() {
        let provider = CohereProvider::new("test_key".to_string()).unwrap();
        let pricing = provider.get_pricing("command-r-plus").await.unwrap();
        assert!(pricing.input_cost_per_1k > 0.0);
    }
}
