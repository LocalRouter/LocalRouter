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
    ProviderHealth, PullProgress, TokenUsage, Tool,
};
use lr_types::{AppError, AppResult};

/// Ollama provider using hybrid SDK + HTTP approach
pub struct OllamaProvider {
    #[allow(dead_code)]
    sdk_client: OllamaClient,
    http_client: Client,
    base_url: String,
}

#[allow(dead_code)]
impl OllamaProvider {
    /// Creates a new Ollama provider with default settings
    pub fn new() -> Self {
        let base_url = "http://localhost:11434".to_string();
        let sdk_client = OllamaClient::new(base_url.clone(), 11434);

        Self {
            sdk_client,
            http_client: crate::http_client::default_client(),
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
            http_client: crate::http_client::default_client(),
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

#[allow(dead_code)]
impl Default for OllamaProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl OllamaProvider {
    /// Get the base URL for this Ollama instance
    pub fn base_url(&self) -> &str {
        &self.base_url
    }
}

// Ollama API types for HTTP requests
#[derive(Debug, Serialize, Deserialize)]
struct OllamaChatRequest {
    model: String,
    /// Messages in Ollama's native format (arguments as JSON objects, not strings)
    messages: Vec<OllamaMessage>,
    #[serde(default)]
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    options: Option<OllamaOptions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<Tool>>,
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
    top_k: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    seed: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    frequency_penalty: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    presence_penalty: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    repeat_penalty: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stop: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Deserialize)]
struct OllamaChatResponse {
    message: OllamaMessage,
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
    message: OllamaMessage,
    #[serde(default)]
    done: bool,
}

/// Ollama-specific message format.
/// Ollama sends tool call arguments as a JSON object, not a JSON string like OpenAI.
#[derive(Debug, Serialize, Deserialize)]
struct OllamaMessage {
    role: String,
    #[serde(default)]
    content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<OllamaToolCall>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct OllamaToolCall {
    #[serde(default)]
    id: Option<String>,
    function: OllamaFunctionCall,
}

#[derive(Debug, Serialize, Deserialize)]
struct OllamaFunctionCall {
    name: String,
    /// Ollama sends arguments as a JSON object, not a string
    arguments: serde_json::Value,
    /// Ollama sometimes includes an index field
    #[serde(default)]
    index: Option<u32>,
}

impl OllamaMessage {
    /// Convert from a standard ChatMessage to Ollama's format
    /// (arguments as JSON objects instead of strings)
    fn from_chat_message(msg: &ChatMessage) -> Self {
        let tool_calls = msg.tool_calls.as_ref().map(|tcs| {
            tcs.iter()
                .map(|tc| OllamaToolCall {
                    id: Some(tc.id.clone()),
                    function: OllamaFunctionCall {
                        name: tc.function.name.clone(),
                        // Convert JSON string back to JSON object for Ollama
                        arguments: serde_json::from_str(&tc.function.arguments).unwrap_or_else(
                            |_| serde_json::Value::String(tc.function.arguments.clone()),
                        ),
                        index: None,
                    },
                })
                .collect()
        });

        OllamaMessage {
            role: msg.role.clone(),
            content: msg.content.as_text(),
            tool_calls,
            tool_call_id: msg.tool_call_id.clone(),
        }
    }

    /// Convert to the standard ChatMessage format
    fn into_chat_message(self) -> ChatMessage {
        use super::{ChatMessageContent, FunctionCall, ToolCall};

        let tool_calls = self.tool_calls.map(|tcs| {
            tcs.into_iter()
                .map(|tc| ToolCall {
                    id: tc
                        .id
                        .unwrap_or_else(|| format!("call_{}", uuid::Uuid::new_v4().simple())),
                    tool_type: "function".to_string(),
                    function: FunctionCall {
                        name: tc.function.name,
                        // Convert JSON value to string (OpenAI format)
                        arguments: if tc.function.arguments.is_string() {
                            tc.function.arguments.as_str().unwrap().to_string()
                        } else {
                            tc.function.arguments.to_string()
                        },
                    },
                })
                .collect()
        });

        ChatMessage {
            role: self.role,
            content: ChatMessageContent::Text(self.content),
            name: None,
            tool_calls,
            tool_call_id: None,
        }
    }
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

// Ollama Embeddings API types
#[derive(Debug, Serialize)]
struct OllamaEmbedRequest {
    model: String,
    input: OllamaEmbedInput,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
enum OllamaEmbedInput {
    Single(String),
    Multiple(Vec<String>),
}

#[derive(Debug, Deserialize)]
struct OllamaEmbedResponse {
    #[serde(default)]
    embedding: Option<Vec<f32>>,
    #[serde(default)]
    embeddings: Option<Vec<Vec<f32>>>,
}

#[async_trait]
#[allow(dead_code)]
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
                    detailed_capabilities: None,
                }
                .enrich_with_catalog_by_name() // Use model-only search for multi-provider system
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
        debug!(
            "Sending completion request to Ollama: {} - Model: {}",
            url, request.model
        );

        let ollama_request = OllamaChatRequest {
            model: request.model.clone(),
            messages: request
                .messages
                .iter()
                .map(OllamaMessage::from_chat_message)
                .collect(),
            stream: false,
            options: Some(OllamaOptions {
                temperature: request.temperature,
                num_predict: request.max_tokens,
                top_p: request.top_p,
                top_k: request.top_k,
                seed: request.seed,
                frequency_penalty: request.frequency_penalty,
                presence_penalty: request.presence_penalty,
                repeat_penalty: request.repetition_penalty,
                stop: request.stop.clone(),
            }),
            tools: request.tools.clone(),
        };

        let response = self
            .http_client
            .post(&url)
            .json(&ollama_request)
            .send()
            .await
            .map_err(|e| {
                error!(
                    "Ollama request failed - URL: {} - Model: {} - Error: {}",
                    url, request.model, e
                );
                AppError::Provider(format!("Ollama request failed: {}", e))
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            error!(
                "Ollama completion failed: {} - Model: {} - Error: {}",
                status, request.model, error_text
            );
            return Err(AppError::Provider(format!(
                "Ollama API error: {} - {}",
                status, error_text
            )));
        }

        let ollama_response: OllamaChatResponse = response
            .json()
            .await
            .map_err(|e| AppError::Provider(format!("Failed to parse Ollama response: {}", e)))?;

        let (prompt_tokens, completion_tokens) =
            if let Some(ref final_data) = ollama_response.final_data {
                (
                    final_data.prompt_eval_count.unwrap_or(0) as u32,
                    final_data.eval_count.unwrap_or(0) as u32,
                )
            } else {
                (0, 0)
            };

        // Convert Ollama message to standard ChatMessage
        let message = ollama_response.message.into_chat_message();

        // Determine finish_reason based on whether tool calls are present
        let finish_reason = if message.tool_calls.as_ref().is_some_and(|tc| !tc.is_empty()) {
            Some("tool_calls".to_string())
        } else {
            Some("stop".to_string())
        };

        Ok(CompletionResponse {
            id: format!("chatcmpl-{}", uuid::Uuid::new_v4()),
            object: "chat.completion".to_string(),
            created: Utc::now().timestamp(),
            model: request.model,
            provider: self.name().to_string(),
            choices: vec![CompletionChoice {
                index: 0,
                message,
                finish_reason,
                logprobs: None, // Ollama does not support logprobs
            }],
            usage: TokenUsage {
                prompt_tokens,
                completion_tokens,
                total_tokens: prompt_tokens + completion_tokens,
                prompt_tokens_details: None,
                completion_tokens_details: None,
            },
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
        let url = format!("{}/api/chat", self.base_url);
        debug!(
            "Sending streaming completion request to Ollama: {} - Model: {}",
            url, request.model
        );

        let ollama_request = OllamaChatRequest {
            model: request.model.clone(),
            messages: request
                .messages
                .iter()
                .map(OllamaMessage::from_chat_message)
                .collect(),
            stream: true,
            options: Some(OllamaOptions {
                temperature: request.temperature,
                num_predict: request.max_tokens,
                top_p: request.top_p,
                top_k: request.top_k,
                seed: request.seed,
                frequency_penalty: request.frequency_penalty,
                presence_penalty: request.presence_penalty,
                repeat_penalty: request.repetition_penalty,
                stop: request.stop.clone(),
            }),
            tools: request.tools.clone(),
        };

        debug!("Ollama streaming request body: {:?}", ollama_request);

        let response = self
            .http_client
            .post(&url)
            .json(&ollama_request)
            .send()
            .await
            .map_err(|e| {
                error!(
                    "Ollama streaming request failed - URL: {} - Model: {} - Error: {}",
                    url, request.model, e
                );
                AppError::Provider(format!("Ollama streaming request failed: {}", e))
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let error_body = response
                .text()
                .await
                .unwrap_or_else(|_| "Unable to read error body".to_string());
            error!(
                "Ollama streaming request failed: {} - Model: {} - Error: {}",
                status, request.model, error_body
            );
            return Err(AppError::Provider(format!(
                "Ollama streaming API error: {} - {}",
                status, error_body
            )));
        }

        let model = request.model.clone();
        let stream = response.bytes_stream();

        // Track state across chunks
        use std::sync::{Arc, Mutex};
        let is_first_chunk = Arc::new(Mutex::new(true));
        // Track if any chunk in this stream contained tool calls
        let seen_tool_calls = Arc::new(Mutex::new(false));

        // Buffer for incomplete lines across byte chunks
        let line_buffer = Arc::new(Mutex::new(String::new()));

        let converted_stream = stream.flat_map(move |result| {
            let model = model.clone();
            let is_first_chunk = is_first_chunk.clone();
            let seen_tool_calls = seen_tool_calls.clone();
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
                                let message = ollama_chunk.message.into_chat_message();
                                let delta_content = message.content.as_text();
                                let mut first = is_first_chunk.lock().unwrap();
                                let is_first = *first;

                                let has_tool_calls =
                                    message.tool_calls.as_ref().is_some_and(|tc| !tc.is_empty());

                                if !delta_content.is_empty() || has_tool_calls {
                                    *first = false;
                                }

                                // Track tool calls across chunks
                                if has_tool_calls {
                                    *seen_tool_calls.lock().unwrap() = true;
                                }

                                // Convert tool calls to streaming delta format
                                let tool_call_deltas = message.tool_calls.map(|tcs| {
                                    tcs.into_iter()
                                        .enumerate()
                                        .map(|(i, tc)| super::ToolCallDelta {
                                            index: i as u32,
                                            id: Some(tc.id),
                                            tool_type: Some(tc.tool_type),
                                            function: Some(super::FunctionCallDelta {
                                                name: Some(tc.function.name),
                                                arguments: Some(tc.function.arguments),
                                            }),
                                        })
                                        .collect()
                                });

                                let finish_reason = if ollama_chunk.done {
                                    // Check both current chunk and any previous chunks
                                    if has_tool_calls || *seen_tool_calls.lock().unwrap() {
                                        Some("tool_calls".to_string())
                                    } else {
                                        Some("stop".to_string())
                                    }
                                } else {
                                    None
                                };

                                let chunk = CompletionChunk {
                                    id: format!("chatcmpl-{}", uuid::Uuid::new_v4()),
                                    object: "chat.completion.chunk".to_string(),
                                    created: Utc::now().timestamp(),
                                    model: model.clone(),
                                    choices: vec![ChunkChoice {
                                        index: 0,
                                        delta: ChunkDelta {
                                            role: if is_first {
                                                Some("assistant".to_string())
                                            } else {
                                                None
                                            },
                                            content: if !delta_content.is_empty() {
                                                Some(delta_content)
                                            } else {
                                                None
                                            },
                                            tool_calls: tool_call_deltas,
                                        },
                                        finish_reason,
                                    }],
                                    extensions: None,
                                };
                                chunks.push(Ok(chunk));
                            }
                            Err(e) => {
                                error!(
                                    "Failed to parse Ollama stream chunk: {} - Line: {}",
                                    e, line
                                );
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

    fn supports_pull(&self) -> bool {
        true
    }

    async fn pull_model(
        &self,
        model_name: &str,
    ) -> AppResult<Pin<Box<dyn Stream<Item = AppResult<PullProgress>> + Send>>> {
        let url = format!("{}/api/pull", self.base_url.trim_end_matches('/'));

        let body = serde_json::json!({
            "name": model_name,
            "stream": true,
        });

        let response = self
            .http_client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| AppError::Provider(format!("Ollama pull request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(AppError::Provider(format!(
                "Ollama pull failed ({}): {}",
                status, body
            )));
        }

        let stream = response.bytes_stream().map(|result| {
            result
                .map_err(|e| AppError::Provider(crate::http_client::format_stream_error(&e)))
                .and_then(|bytes| {
                    // Ollama streams NDJSON — each line is a JSON object
                    let text = String::from_utf8_lossy(&bytes);
                    // May contain multiple lines in one chunk
                    let mut last_progress = None;
                    for line in text.lines() {
                        let line = line.trim();
                        if line.is_empty() {
                            continue;
                        }
                        match serde_json::from_str::<PullProgress>(line) {
                            Ok(progress) => last_progress = Some(progress),
                            Err(e) => {
                                debug!("Failed to parse pull progress line: {} — {}", line, e);
                            }
                        }
                    }
                    last_progress
                        .ok_or_else(|| AppError::Provider("Empty pull progress chunk".to_string()))
                })
        });

        Ok(Box::pin(stream))
    }

    fn supports_embeddings(&self) -> bool {
        true
    }

    async fn embed(&self, request: super::EmbeddingRequest) -> AppResult<super::EmbeddingResponse> {
        // Convert input to Ollama format
        let input = match request.input {
            super::EmbeddingInput::Single(text) => OllamaEmbedInput::Single(text),
            super::EmbeddingInput::Multiple(texts) => OllamaEmbedInput::Multiple(texts),
            super::EmbeddingInput::Tokens(_) => {
                return Err(AppError::Provider(
                    "Ollama embeddings do not support pre-tokenized input".to_string(),
                ));
            }
        };

        let ollama_request = OllamaEmbedRequest {
            model: request.model.clone(),
            input,
        };

        let url = format!("{}/api/embed", self.base_url);

        let response = self
            .http_client
            .post(&url)
            .header("Content-Type", "application/json")
            .json(&ollama_request)
            .send()
            .await
            .map_err(|e| AppError::Provider(format!("Ollama embed request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(AppError::Provider(format!(
                "Ollama embed API error {}: {}",
                status, error_text
            )));
        }

        let ollama_response: OllamaEmbedResponse = response.json().await.map_err(|e| {
            AppError::Provider(format!("Failed to parse Ollama embed response: {}", e))
        })?;

        // Convert Ollama response to our generic format
        // Ollama's /api/embed endpoint always returns 'embeddings' (plural array)
        // even for single inputs, so we always look for 'embeddings' first
        let embeddings = ollama_response
            .embeddings
            .or_else(|| ollama_response.embedding.map(|e| vec![e]))
            .ok_or_else(|| AppError::Provider("No embeddings in response".to_string()))?;

        // Ollama's /api/embed endpoint doesn't return token usage
        Ok(super::EmbeddingResponse {
            object: "list".to_string(),
            data: embeddings
                .into_iter()
                .enumerate()
                .map(|(index, embedding)| super::Embedding {
                    object: "embedding".to_string(),
                    embedding: Some(embedding),
                    index,
                })
                .collect(),
            model: request.model,
            usage: super::EmbeddingUsage {
                prompt_tokens: 0,
                total_tokens: 0,
            },
        })
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
