//! Anthropic (Claude) provider implementation
//!
//! Implements the ModelProvider trait for Anthropic's Claude models.
//! Uses the Messages API format which differs from OpenAI's chat completions.

use async_trait::async_trait;
use chrono::Utc;
use futures::stream::{Stream, StreamExt};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::pin::Pin;
use std::time::Instant;
use tracing::{debug, info};

use lr_api_keys::{keychain_trait::KeychainStorage, CachedKeychain};
use lr_types::{AppError, AppResult};

use super::{
    Capability, ChatMessage, ChunkChoice, ChunkDelta, CompletionChoice, CompletionChunk,
    CompletionRequest, CompletionResponse, FunctionCall, HealthStatus, ModelInfo, ModelProvider,
    PricingInfo, ProviderHealth, TokenUsage, ToolCall,
};

const ANTHROPIC_API_BASE: &str = "https://api.anthropic.com/v1";
const ANTHROPIC_VERSION: &str = "2023-06-01";
const OAUTH_KEYCHAIN_SERVICE: &str = "LocalRouter-ProviderTokens";
const OAUTH_PROVIDER_ID: &str = "anthropic-claude";

/// Anthropic provider for Claude models
pub struct AnthropicProvider {
    client: Client,
    api_key: String,
    base_url: String,
}

#[allow(dead_code)]
impl AnthropicProvider {
    /// Create a new Anthropic provider with an API key
    pub fn new(api_key: String) -> AppResult<Self> {
        Self::with_base_url(api_key, ANTHROPIC_API_BASE.to_string())
    }

    /// Create a new Anthropic provider with a custom base URL (for testing)
    pub fn with_base_url(api_key: String, base_url: String) -> AppResult<Self> {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .map_err(|e| AppError::Provider(format!("Failed to create HTTP client: {}", e)))?;

        Ok(Self {
            client,
            api_key,
            base_url: base_url.trim_end_matches('/').to_string(),
        })
    }

    /// Create a new Anthropic provider from stored API key
    ///
    /// # Arguments
    /// * `provider_name` - The provider name used to store the key (defaults to "anthropic")
    ///
    /// # Returns
    /// * `Ok(Self)` if key exists and provider created successfully
    /// * `Err(AppError)` if key doesn't exist or keyring access fails
    pub fn from_stored_key(provider_name: Option<&str>) -> AppResult<Self> {
        let name = provider_name.unwrap_or("anthropic");
        let api_key = super::key_storage::get_provider_key(name)?.ok_or_else(|| {
            AppError::Provider(format!("No API key found for provider '{}'", name))
        })?;
        Self::new(api_key)
    }

    /// Create a new Anthropic provider from OAuth tokens or API key (OAuth-first)
    ///
    /// This method checks for OAuth tokens first, and falls back to API key if:
    /// - No OAuth tokens are stored
    /// - OAuth tokens are expired and cannot be refreshed
    ///
    /// # Arguments
    /// * `provider_name` - The provider name used to store the API key (defaults to "anthropic")
    ///
    /// # Returns
    /// * `Ok(Self)` if either OAuth tokens or API key are available
    /// * `Err(AppError)` if neither OAuth nor API key authentication is available
    pub fn from_oauth_or_key(provider_name: Option<&str>) -> AppResult<Self> {
        let keychain = CachedKeychain::system();

        // Try OAuth first
        if let Ok(Some(access_token)) = keychain.get(
            OAUTH_KEYCHAIN_SERVICE,
            &format!("{}_access_token", OAUTH_PROVIDER_ID),
        ) {
            info!("Using OAuth credentials for Anthropic provider");
            debug!("Loaded OAuth access token from keychain for anthropic-claude");
            return Self::new(access_token);
        }

        // Fall back to API key
        debug!("No OAuth credentials found, falling back to API key for Anthropic");
        Self::from_stored_key(provider_name)
    }

    /// Check if OAuth credentials are available for this provider
    ///
    /// # Returns
    /// * `true` if OAuth access token exists in keychain
    /// * `false` otherwise
    pub fn has_oauth_credentials() -> bool {
        let keychain = CachedKeychain::system();
        keychain
            .get(
                OAUTH_KEYCHAIN_SERVICE,
                &format!("{}_access_token", OAUTH_PROVIDER_ID),
            )
            .ok()
            .flatten()
            .is_some()
    }

    /// Convert OpenAI format messages to Anthropic format
    fn convert_messages(
        messages: &[ChatMessage],
    ) -> AppResult<(Option<String>, Vec<AnthropicMessage>)> {
        let mut system_prompt = None;
        let mut anthropic_messages = Vec::new();

        for msg in messages {
            match msg.role.as_str() {
                "system" => {
                    // Anthropic uses a separate system parameter
                    if system_prompt.is_some() {
                        return Err(AppError::Provider(
                            "Multiple system messages not supported".to_string(),
                        ));
                    }
                    system_prompt = Some(msg.content.as_text());
                }
                "user" => {
                    // User messages can include tool results
                    if let Some(tool_call_id) = &msg.tool_call_id {
                        // This is a tool result message
                        let content = AnthropicMessageContent::Blocks(vec![
                            AnthropicContentBlock::ToolResult {
                                tool_use_id: tool_call_id.clone(),
                                content: msg.content.as_text(),
                            },
                        ]);
                        anthropic_messages.push(AnthropicMessage {
                            role: "user".to_string(),
                            content,
                        });
                    } else {
                        // Regular user message
                        anthropic_messages.push(AnthropicMessage {
                            role: msg.role.clone(),
                            content: AnthropicMessageContent::Text(msg.content.as_text()),
                        });
                    }
                }
                "assistant" => {
                    // Assistant messages can include tool calls
                    if let Some(tool_calls) = &msg.tool_calls {
                        // Convert tool calls to Anthropic tool_use blocks
                        let mut blocks = Vec::new();

                        // Add text content if present and non-empty
                        let text_content = msg.content.as_text();
                        if !text_content.is_empty() {
                            blocks.push(AnthropicContentBlock::Text { text: text_content });
                        }

                        // Add tool use blocks
                        for tool_call in tool_calls {
                            let input: serde_json::Value =
                                serde_json::from_str(&tool_call.function.arguments)
                                    .unwrap_or(serde_json::json!({}));
                            blocks.push(AnthropicContentBlock::ToolUse {
                                id: tool_call.id.clone(),
                                name: tool_call.function.name.clone(),
                                input,
                            });
                        }

                        anthropic_messages.push(AnthropicMessage {
                            role: msg.role.clone(),
                            content: AnthropicMessageContent::Blocks(blocks),
                        });
                    } else {
                        // Regular assistant message
                        anthropic_messages.push(AnthropicMessage {
                            role: msg.role.clone(),
                            content: AnthropicMessageContent::Text(msg.content.as_text()),
                        });
                    }
                }
                "tool" => {
                    // Tool role messages are converted to user messages with tool_result blocks
                    if let Some(tool_call_id) = &msg.tool_call_id {
                        let content = AnthropicMessageContent::Blocks(vec![
                            AnthropicContentBlock::ToolResult {
                                tool_use_id: tool_call_id.clone(),
                                content: msg.content.as_text(),
                            },
                        ]);
                        anthropic_messages.push(AnthropicMessage {
                            role: "user".to_string(),
                            content,
                        });
                    } else {
                        return Err(AppError::Provider(
                            "Tool message missing tool_call_id".to_string(),
                        ));
                    }
                }
                _ => {
                    return Err(AppError::Provider(format!(
                        "Unsupported message role: {}",
                        msg.role
                    )));
                }
            }
        }

        Ok((system_prompt, anthropic_messages))
    }

    /// Get model information by ID
    fn get_model_info(model_id: &str) -> Option<ModelInfo> {
        match model_id {
            "claude-opus-4-20250514" => Some(ModelInfo {
                id: model_id.to_string(),
                name: "Claude Opus 4".to_string(),
                provider: "anthropic".to_string(),
                parameter_count: None,
                context_window: 200_000,
                supports_streaming: true,
                capabilities: vec![
                    Capability::Chat,
                    Capability::Vision,
                    Capability::FunctionCalling,
                ],
                detailed_capabilities: None,
            }),
            "claude-sonnet-4-20250514" => Some(ModelInfo {
                id: model_id.to_string(),
                name: "Claude Sonnet 4".to_string(),
                provider: "anthropic".to_string(),
                parameter_count: None,
                context_window: 200_000,
                supports_streaming: true,
                capabilities: vec![
                    Capability::Chat,
                    Capability::Vision,
                    Capability::FunctionCalling,
                ],
                detailed_capabilities: None,
            }),
            "claude-3-5-sonnet-20241022" => Some(ModelInfo {
                id: model_id.to_string(),
                name: "Claude 3.5 Sonnet".to_string(),
                provider: "anthropic".to_string(),
                parameter_count: None,
                context_window: 200_000,
                supports_streaming: true,
                capabilities: vec![
                    Capability::Chat,
                    Capability::Vision,
                    Capability::FunctionCalling,
                ],
                detailed_capabilities: None,
            }),
            "claude-3-5-haiku-20241022" => Some(ModelInfo {
                id: model_id.to_string(),
                name: "Claude 3.5 Haiku".to_string(),
                provider: "anthropic".to_string(),
                parameter_count: None,
                context_window: 200_000,
                supports_streaming: true,
                capabilities: vec![Capability::Chat, Capability::Vision],
                detailed_capabilities: None,
            }),
            "claude-3-opus-20240229" => Some(ModelInfo {
                id: model_id.to_string(),
                name: "Claude 3 Opus".to_string(),
                provider: "anthropic".to_string(),
                parameter_count: None,
                context_window: 200_000,
                supports_streaming: true,
                capabilities: vec![
                    Capability::Chat,
                    Capability::Vision,
                    Capability::FunctionCalling,
                ],
                detailed_capabilities: None,
            }),
            "claude-3-sonnet-20240229" => Some(ModelInfo {
                id: model_id.to_string(),
                name: "Claude 3 Sonnet".to_string(),
                provider: "anthropic".to_string(),
                parameter_count: None,
                context_window: 200_000,
                supports_streaming: true,
                capabilities: vec![Capability::Chat, Capability::Vision],
                detailed_capabilities: None,
            }),
            "claude-3-haiku-20240307" => Some(ModelInfo {
                id: model_id.to_string(),
                name: "Claude 3 Haiku".to_string(),
                provider: "anthropic".to_string(),
                parameter_count: None,
                context_window: 200_000,
                supports_streaming: true,
                capabilities: vec![Capability::Chat, Capability::Vision],
                detailed_capabilities: None,
            }),
            _ => None,
        }
    }

    /// Get pricing for a model
    fn get_model_pricing(model_id: &str) -> PricingInfo {
        match model_id {
            "claude-opus-4-20250514" => PricingInfo {
                input_cost_per_1k: 0.015,
                output_cost_per_1k: 0.075,
                currency: "USD".to_string(),
            },
            "claude-sonnet-4-20250514" => PricingInfo {
                input_cost_per_1k: 0.003,
                output_cost_per_1k: 0.015,
                currency: "USD".to_string(),
            },
            "claude-3-5-sonnet-20241022" => PricingInfo {
                input_cost_per_1k: 0.003,
                output_cost_per_1k: 0.015,
                currency: "USD".to_string(),
            },
            "claude-3-5-haiku-20241022" => PricingInfo {
                input_cost_per_1k: 0.001,
                output_cost_per_1k: 0.005,
                currency: "USD".to_string(),
            },
            "claude-3-opus-20240229" => PricingInfo {
                input_cost_per_1k: 0.015,
                output_cost_per_1k: 0.075,
                currency: "USD".to_string(),
            },
            "claude-3-sonnet-20240229" => PricingInfo {
                input_cost_per_1k: 0.003,
                output_cost_per_1k: 0.015,
                currency: "USD".to_string(),
            },
            "claude-3-haiku-20240307" => PricingInfo {
                input_cost_per_1k: 0.00025,
                output_cost_per_1k: 0.00125,
                currency: "USD".to_string(),
            },
            _ => PricingInfo {
                input_cost_per_1k: 0.0,
                output_cost_per_1k: 0.0,
                currency: "USD".to_string(),
            },
        }
    }
}

#[async_trait]
#[allow(dead_code)]
impl ModelProvider for AnthropicProvider {
    fn name(&self) -> &str {
        "anthropic"
    }

    async fn health_check(&self) -> ProviderHealth {
        let start = Instant::now();

        // Try to list models as a health check
        let result = self
            .client
            .get(format!("{}/models", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .send()
            .await;

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
                        error_message: Some(format!("API returned status: {}", response.status())),
                    }
                }
            }
            Err(e) => ProviderHealth {
                status: HealthStatus::Unhealthy,
                latency_ms: None,
                last_checked: Utc::now(),
                error_message: Some(format!("Request failed: {}", e)),
            },
        }
    }

    async fn list_models(&self) -> AppResult<Vec<ModelInfo>> {
        let url = format!("{}/models", self.base_url);

        let response = self
            .client
            .get(&url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .send()
            .await
            .map_err(|e| AppError::Provider(format!("Failed to fetch Anthropic models: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(AppError::Provider(format!(
                "Anthropic models API error {}: {}",
                status, error_text
            )));
        }

        let models_response: AnthropicModelsResponse = response
            .json()
            .await
            .map_err(|e| AppError::Provider(format!("Failed to parse Anthropic models response: {}", e)))?;

        let models = models_response
            .data
            .into_iter()
            .filter_map(|m| Self::get_model_info(&m.id))
            .collect();

        Ok(models)
    }

    async fn get_pricing(&self, model: &str) -> AppResult<PricingInfo> {
        // Try catalog first (embedded OpenRouter data)
        if let Some(catalog_model) = lr_catalog::find_model("anthropic", model) {
            tracing::debug!("Using catalog pricing for Anthropic model: {}", model);
            return Ok(PricingInfo {
                input_cost_per_1k: catalog_model.pricing.prompt_cost_per_1k(),
                output_cost_per_1k: catalog_model.pricing.completion_cost_per_1k(),
                currency: catalog_model.pricing.currency.to_string(),
            });
        }

        // Fallback to hardcoded pricing
        tracing::debug!("Using fallback pricing for Anthropic model: {}", model);
        Ok(Self::get_model_pricing(model))
    }

    async fn complete(&self, request: CompletionRequest) -> AppResult<CompletionResponse> {
        let (system, messages) = Self::convert_messages(&request.messages)?;

        // Convert tools from OpenAI format to Anthropic format
        let tools = request.tools.as_ref().map(|openai_tools| {
            openai_tools
                .iter()
                .map(|t| AnthropicTool {
                    name: t.function.name.clone(),
                    description: t.function.description.clone(),
                    input_schema: t.function.parameters.clone(),
                })
                .collect()
        });

        let anthropic_request = AnthropicRequest {
            model: request.model.clone(),
            messages,
            system,
            max_tokens: request.max_tokens.unwrap_or(4096),
            temperature: request.temperature,
            top_p: request.top_p,
            stop_sequences: request.stop,
            stream: Some(false),
            tools,
        };

        let response = self
            .client
            .post(format!("{}/messages", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("content-type", "application/json")
            .json(&anthropic_request)
            .send()
            .await
            .map_err(|e| AppError::Provider(format!("Request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            return Err(AppError::Provider(format!(
                "API error ({}): {}",
                status, error_text
            )));
        }

        let anthropic_response: AnthropicResponse = response
            .json()
            .await
            .map_err(|e| AppError::Provider(format!("Failed to parse response: {}", e)))?;

        // Convert Anthropic response to OpenAI format
        // Parse content blocks - extract text and tool_use blocks
        let mut text_content = String::new();
        let mut tool_calls = Vec::new();

        for content_block in &anthropic_response.content {
            match content_block {
                AnthropicResponseContent::Text { text } => {
                    if !text_content.is_empty() {
                        text_content.push('\n');
                    }
                    text_content.push_str(text);
                }
                AnthropicResponseContent::ToolUse { id, name, input } => {
                    tool_calls.push(ToolCall {
                        id: id.clone(),
                        tool_type: "function".to_string(),
                        function: FunctionCall {
                            name: name.clone(),
                            arguments: serde_json::to_string(input).unwrap_or_default(),
                        },
                    });
                }
            }
        }

        let tool_calls_opt = if tool_calls.is_empty() {
            None
        } else {
            Some(tool_calls)
        };

        // Determine finish reason - if tool_calls present and stop_reason is "end_turn", change to "tool_calls"
        let finish_reason = if tool_calls_opt.is_some() {
            Some("tool_calls".to_string())
        } else {
            Some(
                anthropic_response
                    .stop_reason
                    .unwrap_or_else(|| "stop".to_string()),
            )
        };

        Ok(CompletionResponse {
            id: anthropic_response.id,
            object: "chat.completion".to_string(),
            created: Utc::now().timestamp(),
            model: anthropic_response.model,
            provider: self.name().to_string(),
            choices: vec![CompletionChoice {
                index: 0,
                message: ChatMessage {
                    role: "assistant".to_string(),
                    content: super::ChatMessageContent::Text(text_content),
                    tool_calls: tool_calls_opt,
                    tool_call_id: None,
                    name: None,
                },
                finish_reason,
                logprobs: None, // Anthropic does not support logprobs
            }],
            usage: TokenUsage {
                prompt_tokens: anthropic_response.usage.input_tokens,
                completion_tokens: anthropic_response.usage.output_tokens,
                total_tokens: anthropic_response.usage.input_tokens
                    + anthropic_response.usage.output_tokens,
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
        let (system, messages) = Self::convert_messages(&request.messages)?;

        // Convert tools from OpenAI format to Anthropic format
        let tools = request.tools.as_ref().map(|openai_tools| {
            openai_tools
                .iter()
                .map(|t| AnthropicTool {
                    name: t.function.name.clone(),
                    description: t.function.description.clone(),
                    input_schema: t.function.parameters.clone(),
                })
                .collect()
        });

        let anthropic_request = AnthropicRequest {
            model: request.model.clone(),
            messages,
            system,
            max_tokens: request.max_tokens.unwrap_or(4096),
            temperature: request.temperature,
            top_p: request.top_p,
            stop_sequences: request.stop,
            stream: Some(true),
            tools,
        };

        let response = self
            .client
            .post(format!("{}/messages", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("content-type", "application/json")
            .json(&anthropic_request)
            .send()
            .await
            .map_err(|e| AppError::Provider(format!("Request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            return Err(AppError::Provider(format!(
                "API error ({}): {}",
                status, error_text
            )));
        }

        let model = request.model.clone();
        let stream = response.bytes_stream();

        // Buffer for incomplete lines across byte chunks
        use std::sync::{Arc, Mutex};
        let line_buffer = Arc::new(Mutex::new(String::new()));

        let converted_stream = stream.flat_map(move |result| {
            let model = model.clone();
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

                        // Parse SSE format: "data: {...}"
                        if let Some(data) = line.strip_prefix("data: ") {
                            // Skip [DONE] marker
                            if data == "[DONE]" {
                                continue;
                            }

                            // Parse JSON event
                            match serde_json::from_str::<AnthropicStreamEvent>(data) {
                                Ok(event) => {
                                    match event.event_type.as_str() {
                                        "content_block_delta" => {
                                            if let Some(delta) = event.delta {
                                                if let Some(text) = delta.text {
                                                    // Anthropic sends delta chunks, not cumulative
                                                    let chunk = CompletionChunk {
                                                        id: event.message_id.unwrap_or_default(),
                                                        object: "chat.completion.chunk".to_string(),
                                                        created: Utc::now().timestamp(),
                                                        model: model.clone(),
                                                        choices: vec![ChunkChoice {
                                                            index: 0,
                                                            delta: ChunkDelta {
                                                                role: None,
                                                                content: Some(text),
                                                                tool_calls: None,
                                                            },
                                                            finish_reason: None,
                                                        }],
                                                        extensions: None,
                                                    };
                                                    chunks.push(Ok(chunk));
                                                }
                                            }
                                        }
                                        "message_stop" => {
                                            let chunk = CompletionChunk {
                                                id: event.message_id.unwrap_or_default(),
                                                object: "chat.completion.chunk".to_string(),
                                                created: Utc::now().timestamp(),
                                                model: model.clone(),
                                                choices: vec![ChunkChoice {
                                                    index: 0,
                                                    delta: ChunkDelta {
                                                        role: None,
                                                        content: None,
                                                        tool_calls: None,
                                                    },
                                                    finish_reason: Some("stop".to_string()),
                                                }],
                                                extensions: None,
                                            };
                                            chunks.push(Ok(chunk));
                                        }
                                        _ => {}
                                    }
                                }
                                Err(e) => {
                                    tracing::error!(
                                        "Failed to parse Anthropic stream event: {} - Line: {}",
                                        e,
                                        data
                                    );
                                }
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

    fn supports_feature(&self, feature: &str) -> bool {
        matches!(
            feature,
            "extended_thinking" | "prompt_caching" | "structured_outputs"
        )
    }

    fn get_feature_adapter(
        &self,
        feature: &str,
    ) -> Option<Box<dyn crate::features::FeatureAdapter>> {
        match feature {
            "extended_thinking" => Some(Box::new(
                crate::features::anthropic_thinking::AnthropicThinkingAdapter,
            )),
            "structured_outputs" => Some(Box::new(
                crate::features::structured_outputs::StructuredOutputsAdapter,
            )),
            "prompt_caching" => Some(Box::new(
                crate::features::prompt_caching::PromptCachingAdapter,
            )),
            "json_mode" => Some(Box::new(
                crate::features::json_mode::JsonModeAdapter,
            )),
            _ => None,
        }
    }
}

// Anthropic API request/response structures

/// Anthropic tool definition
#[derive(Debug, Serialize, Deserialize)]
struct AnthropicTool {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    input_schema: serde_json::Value,
}

#[derive(Debug, Serialize)]
struct AnthropicRequest {
    model: String,
    messages: Vec<AnthropicMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
    max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stop_sequences: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream: Option<bool>,
    /// Tool definitions for function calling
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<AnthropicTool>>,
}

/// Anthropic message with content blocks
#[derive(Debug, Serialize, Deserialize)]
struct AnthropicMessage {
    role: String,
    content: AnthropicMessageContent,
}

/// Anthropic message content (can be string or array of content blocks)
#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
enum AnthropicMessageContent {
    Text(String),
    Blocks(Vec<AnthropicContentBlock>),
}

/// Anthropic content block
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
enum AnthropicContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    #[serde(rename = "tool_result")]
    ToolResult {
        tool_use_id: String,
        content: String,
    },
}

#[derive(Debug, Deserialize)]
struct AnthropicResponse {
    id: String,
    model: String,
    content: Vec<AnthropicResponseContent>,
    stop_reason: Option<String>,
    usage: AnthropicUsage,
}

/// Anthropic response content block
#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum AnthropicResponseContent {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
}

#[derive(Debug, Deserialize)]
struct AnthropicUsage {
    input_tokens: u32,
    output_tokens: u32,
}

// Anthropic Models API response types
#[derive(Debug, Deserialize)]
struct AnthropicModelsResponse {
    data: Vec<AnthropicModel>,
}

#[derive(Debug, Deserialize)]
struct AnthropicModel {
    id: String,
    display_name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AnthropicStreamEvent {
    #[serde(rename = "type")]
    event_type: String,
    #[serde(default)]
    message_id: Option<String>,
    #[serde(default)]
    delta: Option<AnthropicDelta>,
}

#[derive(Debug, Deserialize)]
struct AnthropicDelta {
    #[serde(default)]
    text: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ChatMessageContent, FunctionCall, ToolCall};

    #[test]
    fn test_convert_messages_with_system() {
        let messages = vec![
            ChatMessage {
                role: "system".to_string(),
                content: ChatMessageContent::Text("You are a helpful assistant.".to_string()),
                tool_calls: None,
                tool_call_id: None,
                name: None,
            },
            ChatMessage {
                role: "user".to_string(),
                content: ChatMessageContent::Text("Hello!".to_string()),
                tool_calls: None,
                tool_call_id: None,
                name: None,
            },
        ];

        let (system, anthropic_messages) = AnthropicProvider::convert_messages(&messages).unwrap();

        assert_eq!(system, Some("You are a helpful assistant.".to_string()));
        assert_eq!(anthropic_messages.len(), 1);
        assert_eq!(anthropic_messages[0].role, "user");
        match &anthropic_messages[0].content {
            AnthropicMessageContent::Text(text) => assert_eq!(text, "Hello!"),
            _ => panic!("Expected Text content"),
        }
    }

    #[test]
    fn test_convert_messages_without_system() {
        let messages = vec![
            ChatMessage {
                role: "user".to_string(),
                content: ChatMessageContent::Text("Hello!".to_string()),
                tool_calls: None,
                tool_call_id: None,
                name: None,
            },
            ChatMessage {
                role: "assistant".to_string(),
                content: ChatMessageContent::Text("Hi there!".to_string()),
                tool_calls: None,
                tool_call_id: None,
                name: None,
            },
        ];

        let (system, anthropic_messages) = AnthropicProvider::convert_messages(&messages).unwrap();

        assert_eq!(system, None);
        assert_eq!(anthropic_messages.len(), 2);
    }

    #[test]
    fn test_model_info_lookup() {
        let info = AnthropicProvider::get_model_info("claude-3-5-sonnet-20241022").unwrap();
        assert_eq!(info.name, "Claude 3.5 Sonnet");
        assert_eq!(info.provider, "anthropic");
        assert_eq!(info.context_window, 200_000);
        assert!(info.supports_streaming);
    }

    #[test]
    fn test_pricing_lookup() {
        let pricing = AnthropicProvider::get_model_pricing("claude-3-5-sonnet-20241022");
        assert_eq!(pricing.input_cost_per_1k, 0.003);
        assert_eq!(pricing.output_cost_per_1k, 0.015);
        assert_eq!(pricing.currency, "USD");
    }

    #[test]
    fn test_model_info_unknown() {
        let info = AnthropicProvider::get_model_info("unknown-model");
        assert!(info.is_none());
    }

    #[test]
    fn test_convert_messages_with_tool_calls() {
        let messages = vec![
            ChatMessage {
                role: "user".to_string(),
                content: ChatMessageContent::Text("What's the weather?".to_string()),
                tool_calls: None,
                tool_call_id: None,
                name: None,
            },
            ChatMessage {
                role: "assistant".to_string(),
                content: ChatMessageContent::Text("".to_string()),
                tool_calls: Some(vec![ToolCall {
                    id: "toolu_123".to_string(),
                    tool_type: "function".to_string(),
                    function: FunctionCall {
                        name: "get_weather".to_string(),
                        arguments: r#"{"location":"San Francisco","unit":"fahrenheit"}"#
                            .to_string(),
                    },
                }]),
                tool_call_id: None,
                name: None,
            },
        ];

        let (system, anthropic_messages) = AnthropicProvider::convert_messages(&messages).unwrap();

        assert_eq!(system, None);
        assert_eq!(anthropic_messages.len(), 2);

        // Check that the assistant message has ToolUse content block
        match &anthropic_messages[1].content {
            AnthropicMessageContent::Blocks(blocks) => {
                assert_eq!(blocks.len(), 1);
                match &blocks[0] {
                    AnthropicContentBlock::ToolUse { id, name, input } => {
                        assert_eq!(id, "toolu_123");
                        assert_eq!(name, "get_weather");
                        assert_eq!(
                            input,
                            &serde_json::json!({"location": "San Francisco", "unit": "fahrenheit"})
                        );
                    }
                    _ => panic!("Expected ToolUse block"),
                }
            }
            _ => panic!("Expected Blocks content"),
        }
    }

    #[test]
    fn test_convert_messages_with_tool_response() {
        let messages = vec![
            ChatMessage {
                role: "user".to_string(),
                content: ChatMessageContent::Text("What's the weather?".to_string()),
                tool_calls: None,
                tool_call_id: None,
                name: None,
            },
            ChatMessage {
                role: "assistant".to_string(),
                content: ChatMessageContent::Text("".to_string()),
                tool_calls: Some(vec![ToolCall {
                    id: "toolu_123".to_string(),
                    tool_type: "function".to_string(),
                    function: FunctionCall {
                        name: "get_weather".to_string(),
                        arguments: r#"{"location":"San Francisco"}"#.to_string(),
                    },
                }]),
                tool_call_id: None,
                name: None,
            },
            ChatMessage {
                role: "tool".to_string(),
                content: ChatMessageContent::Text(
                    r#"{"temperature":72,"conditions":"sunny"}"#.to_string(),
                ),
                tool_calls: None,
                tool_call_id: Some("toolu_123".to_string()),
                name: Some("get_weather".to_string()),
            },
        ];

        let (system, anthropic_messages) = AnthropicProvider::convert_messages(&messages).unwrap();

        assert_eq!(system, None);
        assert_eq!(anthropic_messages.len(), 3);

        // Check that the tool message was converted to user role with ToolResult block
        assert_eq!(anthropic_messages[2].role, "user");
        match &anthropic_messages[2].content {
            AnthropicMessageContent::Blocks(blocks) => {
                assert_eq!(blocks.len(), 1);
                match &blocks[0] {
                    AnthropicContentBlock::ToolResult {
                        tool_use_id,
                        content,
                    } => {
                        assert_eq!(tool_use_id, "toolu_123");
                        assert_eq!(content, r#"{"temperature":72,"conditions":"sunny"}"#);
                    }
                    _ => panic!("Expected ToolResult block"),
                }
            }
            _ => panic!("Expected Blocks content"),
        }
    }

    #[test]
    fn test_parse_response_with_tool_use() {
        use serde_json::json;

        // Create a mock Anthropic response with tool use
        let anthropic_response = AnthropicResponse {
            id: "msg_123".to_string(),
            model: "claude-3-5-sonnet-20241022".to_string(),
            content: vec![AnthropicResponseContent::ToolUse {
                id: "toolu_456".to_string(),
                name: "get_weather".to_string(),
                input: json!({"location": "San Francisco", "unit": "celsius"}),
            }],
            stop_reason: Some("tool_use".to_string()),
            usage: AnthropicUsage {
                input_tokens: 100,
                output_tokens: 50,
            },
        };

        // Simulate parsing logic from complete() method
        let mut text_content = String::new();
        let mut tool_calls = Vec::new();

        for content_block in &anthropic_response.content {
            match content_block {
                AnthropicResponseContent::Text { text } => {
                    text_content.push_str(text);
                }
                AnthropicResponseContent::ToolUse { id, name, input } => {
                    tool_calls.push(ToolCall {
                        id: id.clone(),
                        tool_type: "function".to_string(),
                        function: FunctionCall {
                            name: name.clone(),
                            arguments: serde_json::to_string(input).unwrap_or_default(),
                        },
                    });
                }
            }
        }

        // Verify
        assert_eq!(text_content, "");
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0].id, "toolu_456");
        assert_eq!(tool_calls[0].function.name, "get_weather");
        assert_eq!(
            tool_calls[0].function.arguments,
            r#"{"location":"San Francisco","unit":"celsius"}"#
        );
    }

    #[test]
    fn test_parse_response_with_text_and_tool_use() {
        use serde_json::json;

        // Create a mock Anthropic response with both text and tool use
        let anthropic_response = AnthropicResponse {
            id: "msg_123".to_string(),
            model: "claude-3-5-sonnet-20241022".to_string(),
            content: vec![
                AnthropicResponseContent::Text {
                    text: "Let me check the weather for you.".to_string(),
                },
                AnthropicResponseContent::ToolUse {
                    id: "toolu_789".to_string(),
                    name: "get_weather".to_string(),
                    input: json!({"location": "San Francisco"}),
                },
            ],
            stop_reason: Some("tool_use".to_string()),
            usage: AnthropicUsage {
                input_tokens: 100,
                output_tokens: 50,
            },
        };

        let mut text_content = String::new();
        let mut tool_calls = Vec::new();

        for content_block in &anthropic_response.content {
            match content_block {
                AnthropicResponseContent::Text { text } => {
                    text_content.push_str(text);
                }
                AnthropicResponseContent::ToolUse { id, name, input } => {
                    tool_calls.push(ToolCall {
                        id: id.clone(),
                        tool_type: "function".to_string(),
                        function: FunctionCall {
                            name: name.clone(),
                            arguments: serde_json::to_string(input).unwrap_or_default(),
                        },
                    });
                }
            }
        }

        // Verify both text and tool calls are extracted
        assert_eq!(text_content, "Let me check the weather for you.");
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0].function.name, "get_weather");
    }

    #[tokio::test]
    async fn test_list_models() {
        let provider = AnthropicProvider::new("test-key".to_string()).unwrap();
        let models = provider.list_models().await.unwrap();

        assert!(!models.is_empty());
        assert!(models.iter().any(|m| m.id.contains("claude")));
    }
}
