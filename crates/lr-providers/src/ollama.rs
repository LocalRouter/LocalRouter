//! Ollama provider implementation using direct HTTP API
//!
//! Uses direct HTTP calls for all operations to enable comprehensive testing
//! and maintain full control over the OpenAI-compatible format.

use async_trait::async_trait;
use chrono::Utc;
use futures::{Stream, StreamExt};
use ollama_rs::Ollama as OllamaClient;
use reqwest_middleware::ClientWithMiddleware;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::{Arc, OnceLock};
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
    http_client: ClientWithMiddleware,
    base_url: String,
    /// Whether each model is a decision model (Ollama 0.35+ lists
    /// `decision` among its capabilities), from the model list or a
    /// `/api/show` on first use.
    decision_models: Arc<parking_lot::RwLock<HashMap<String, bool>>>,
    /// Installed decision models from the last model list, sorted.
    installed_decision_models: Arc<parking_lot::RwLock<Vec<String>>>,
    /// Client for Ollama's native `/v1/systemone`, built on first use.
    systemone_client: OnceLock<Arc<crate::systemone::SystemOneProvider>>,
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
            decision_models: Arc::default(),
            installed_decision_models: Arc::default(),
            systemone_client: OnceLock::new(),
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
            decision_models: Arc::default(),
            installed_decision_models: Arc::default(),
            systemone_client: OnceLock::new(),
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

    /// Fetch per-model capabilities from `/api/show`.
    ///
    /// Ollama's `/api/show` returns a `capabilities` array with entries like
    /// `"completion"`, `"tools"`, `"vision"`, `"embedding"`, `"thinking"`. This
    /// complements `/api/tags` (which only lists model names) and lets the
    /// router filter correctly when a request carries tools.
    ///
    /// Returns `None` on any failure so the caller can fall back to defaults.
    /// Records whether the model is a decision model.
    async fn fetch_model_capabilities(&self, name: &str) -> Option<Vec<Capability>> {
        let url = format!("{}/api/show", self.base_url);
        let resp = self
            .http_client
            .post(&url)
            .json(&OllamaShowRequest { name })
            .send()
            .await
            .ok()?;
        if !resp.status().is_success() {
            return None;
        }
        let show: OllamaShowResponse = resp.json().await.ok()?;
        let caps = Self::map_ollama_capabilities(&show.capabilities);
        self.decision_models
            .write()
            .insert(name.to_string(), caps.contains(&Capability::Decision));
        Some(caps)
    }

    /// Whether `model` is one of Ollama's decision models, asking Ollama
    /// when it has not been seen yet.
    async fn is_decision_model(&self, model: &str) -> bool {
        if let Some(known) = self.decision_models.read().get(model) {
            return *known;
        }
        self.fetch_model_capabilities(model)
            .await
            .is_some_and(|caps| caps.contains(&Capability::Decision))
    }

    fn systemone_client(&self) -> AppResult<Arc<crate::systemone::SystemOneProvider>> {
        if let Some(client) = self.systemone_client.get() {
            return Ok(client.clone());
        }
        let client = Arc::new(crate::systemone::SystemOneProvider::new(
            crate::systemone::SystemOneFlavor::Generic,
            Some(self.base_url.clone()),
            None,
        )?);
        Ok(self.systemone_client.get_or_init(|| client).clone())
    }

    /// Map the Ollama `/api/show` `capabilities` strings onto our `Capability`
    /// enum. Extracted so the mapping is testable without HTTP.
    fn map_ollama_capabilities(raw: &[String]) -> Vec<Capability> {
        let has = |s: &str| raw.iter().any(|c| c.eq_ignore_ascii_case(s));
        let mut caps = Vec::with_capacity(4);
        // A generation model is always usable for both /chat and /completions.
        if has("completion") || has("chat") || has("generate") {
            caps.push(Capability::Chat);
            caps.push(Capability::Completion);
        }
        if has("tools") {
            caps.push(Capability::FunctionCalling);
        }
        if has("vision") {
            caps.push(Capability::Vision);
        }
        if has("embedding") {
            caps.push(Capability::Embedding);
        }
        // Ollama 0.35+: answers `/v1/systemone` (e.g. nimble, tev1).
        if has("decision") {
            caps.push(Capability::Decision);
        }
        caps
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
    /// Control thinking/reasoning for models that support it (Qwen3, DeepSeek-R1, etc.)
    /// None = model default (thinking enabled for supported models)
    /// Some(false) = disable thinking
    /// Some(true) = enable thinking
    #[serde(skip_serializing_if = "Option::is_none")]
    think: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    logprobs: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_logprobs: Option<u32>,
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
    /// Token counts — top-level in Ollama non-streaming responses
    #[serde(default)]
    prompt_eval_count: Option<i64>,
    #[serde(default)]
    eval_count: Option<i64>,
    /// Legacy nested field (kept for backward compat, unused in practice)
    #[serde(default)]
    final_data: Option<OllamaFinalData>,
    /// Per-token log probabilities when requested: a list in the shape of
    /// OpenAI's `logprobs.content`.
    #[serde(default)]
    logprobs: Option<serde_json::Value>,
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
    /// Prompt tokens evaluated; sent on the final (`done: true`) message.
    #[serde(default)]
    prompt_eval_count: Option<i64>,
    /// Tokens generated; sent on the final (`done: true`) message.
    #[serde(default)]
    eval_count: Option<i64>,
}

impl OllamaStreamResponse {
    /// Usage from the final message; `None` on other messages or when the
    /// server reports neither count.
    fn usage(&self) -> Option<TokenUsage> {
        if !self.done || (self.prompt_eval_count.is_none() && self.eval_count.is_none()) {
            return None;
        }
        let count = |n: Option<i64>| n.map_or(0, |n| u32::try_from(n.max(0)).unwrap_or(u32::MAX));
        let prompt_tokens = count(self.prompt_eval_count);
        let completion_tokens = count(self.eval_count);
        Some(TokenUsage {
            prompt_tokens,
            completion_tokens,
            total_tokens: prompt_tokens.saturating_add(completion_tokens),
            prompt_tokens_details: None,
            completion_tokens_details: None,
        })
    }
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
    /// Thinking/reasoning content from reasoning models (Qwen3, DeepSeek-R1, etc.)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    thinking: Option<String>,
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
            thinking: msg.reasoning_content.clone(),
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
            reasoning_content: self.thinking,
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

// Types for /api/show endpoint (used to detect per-model capabilities)
#[derive(Debug, Serialize)]
struct OllamaShowRequest<'a> {
    name: &'a str,
}

#[derive(Debug, Deserialize)]
struct OllamaShowResponse {
    #[serde(default)]
    capabilities: Vec<String>,
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

        // Fetch per-model capabilities concurrently via /api/show so the
        // router's capability-based filters (FunctionCalling, Vision, ...)
        // have accurate data. Bounded concurrency keeps load on local Ollama
        // reasonable even with many installed models.
        use futures::stream::{self, StreamExt};
        let names: Vec<String> = tags_response
            .models
            .iter()
            .map(|m| m.name.clone())
            .collect();
        // Use `buffered` (ordered) so results align with `tags_response.models`
        // by index; `buffer_unordered` would break the zip below.
        let cap_results: Vec<Option<Vec<Capability>>> = stream::iter(names)
            .map(|name| async move { self.fetch_model_capabilities(&name).await })
            .buffered(8)
            .collect::<Vec<_>>()
            .await;
        // Forget decision models that are no longer installed.
        {
            let installed: std::collections::HashSet<&str> = tags_response
                .models
                .iter()
                .map(|m| m.name.as_str())
                .collect();
            self.decision_models
                .write()
                .retain(|name, _| installed.contains(name.as_str()));
            let mut decision: Vec<String> = tags_response
                .models
                .iter()
                .zip(&cap_results)
                .filter(|(_, caps)| {
                    caps.as_ref()
                        .is_some_and(|c| c.contains(&Capability::Decision))
                })
                .map(|(m, _)| m.name.clone())
                .collect();
            decision.sort();
            *self.installed_decision_models.write() = decision;
        }

        let models: Vec<ModelInfo> = tags_response
            .models
            .into_iter()
            .zip(cap_results)
            .map(|(model, caps)| {
                let parameter_count = Self::parse_parameter_count(&model.name);
                // Fall back to Chat+Completion when /api/show is unavailable
                // or doesn't return capabilities (e.g. older Ollama versions).
                let capabilities =
                    caps.unwrap_or_else(|| vec![Capability::Chat, Capability::Completion]);

                ModelInfo {
                    id: model.name.clone(),
                    name: model.name,
                    provider: "ollama".to_string(),
                    parameter_count,
                    context_window: 4096,
                    supports_streaming: true,
                    capabilities,
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
            // Map reasoning_effort to Ollama's think parameter:
            // "none" → disable thinking, any other value → enable, absent → model default
            think: request
                .reasoning_effort
                .as_ref()
                .map(|effort| !effort.eq_ignore_ascii_case("none")),
            logprobs: request.logprobs,
            top_logprobs: request.logprobs.filter(|l| *l).and(request.top_logprobs),
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

        // Token counts: prefer top-level fields (non-streaming), fall back to final_data (streaming)
        let prompt_tokens = ollama_response
            .prompt_eval_count
            .or_else(|| {
                ollama_response
                    .final_data
                    .as_ref()
                    .and_then(|fd| fd.prompt_eval_count)
            })
            .unwrap_or(0) as u32;
        let completion_tokens = ollama_response
            .eval_count
            .or_else(|| {
                ollama_response
                    .final_data
                    .as_ref()
                    .and_then(|fd| fd.eval_count)
            })
            .unwrap_or(0) as u32;

        let logprobs = ollama_response
            .logprobs
            .as_ref()
            .and_then(|l| super::Logprobs::from_wire(&serde_json::json!({ "content": l })));

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
                logprobs,
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
            think: request
                .reasoning_effort
                .as_ref()
                .map(|effort| !effort.eq_ignore_ascii_case("none")),
            // Streamed chunks carry no log probabilities.
            logprobs: None,
            top_logprobs: None,
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
        let stream = crate::sse_lines::line_batches(response.bytes_stream());

        // Track state across chunks
        use std::sync::{Arc, Mutex};
        let is_first_chunk = Arc::new(Mutex::new(true));
        // Track if any chunk in this stream contained tool calls
        let seen_tool_calls = Arc::new(Mutex::new(false));

        let converted_stream = stream.flat_map(move |result| {
            let model = model.clone();
            let is_first_chunk = is_first_chunk.clone();
            let seen_tool_calls = seen_tool_calls.clone();

            let chunks: Vec<AppResult<CompletionChunk>> = match result {
                Ok(lines) => {
                    let mut chunks = Vec::new();

                    for line in lines {
                        if line.trim().is_empty() {
                            continue;
                        }

                        // Check for Ollama error responses (e.g. {"error":"..."})
                        if let Ok(error_obj) = serde_json::from_str::<serde_json::Value>(&line) {
                            if let Some(error_msg) = error_obj.get("error").and_then(|v| v.as_str())
                            {
                                error!("Ollama streaming error: {} - Model: {}", error_msg, model);
                                chunks.push(Err(AppError::Provider(format!(
                                    "Ollama error: {}",
                                    error_msg
                                ))));
                                continue;
                            }
                        }

                        match serde_json::from_str::<OllamaStreamResponse>(&line) {
                            Ok(ollama_chunk) => {
                                let usage = ollama_chunk.usage();
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
                                            reasoning_content: message.reasoning_content,
                                        },
                                        finish_reason,
                                    }],
                                    extensions: None,
                                    usage,
                                    provider: None,
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

    fn supports_feature(&self, feature: &str) -> bool {
        // Non-streaming chat returns per-token log probabilities.
        feature == "logprobs"
    }

    /// Whether the last model list had a decision model.
    fn supports_systemone(&self) -> bool {
        !self.installed_decision_models.read().is_empty()
    }

    /// Decision models answer `/v1/systemone` natively; chat models go
    /// through the router's translation.
    async fn supports_systemone_model(&self, model: &str) -> bool {
        self.is_decision_model(model).await
    }

    async fn systemone(
        &self,
        mut request: crate::SystemOneRequest,
    ) -> AppResult<crate::SystemOneResponse> {
        if request.model.is_none() {
            let first = self.installed_decision_models.read().first().cloned();
            request.model = Some(first.ok_or_else(|| {
                AppError::InvalidParams(
                    "Ollama has no decision model (pull one, e.g. `ollama pull tev1`)".to_string(),
                )
            })?);
        }
        self.systemone_client()?.systemone(request).await
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

        Ok(pull_progress_stream(response.bytes_stream()))
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

    async fn generate_image(
        &self,
        request: super::ImageGenerationRequest,
    ) -> AppResult<super::ImageGenerationResponse> {
        // Ollama exposes an OpenAI-compatible image generation endpoint
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

        let url = format!("{}/v1/images/generations", self.base_url);
        let response = self
            .http_client
            .post(&url)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| AppError::Provider(format!("Ollama image request failed: {}", e)))?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(AppError::Provider(format!(
                "Ollama image API error {}: {}",
                status, error_text
            )));
        }

        let api_response: serde_json::Value = response.json().await.map_err(|e| {
            AppError::Provider(format!("Failed to parse Ollama image response: {}", e))
        })?;

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

/// Decode pull progress as NDJSON records, independent of network read sizes.
fn pull_progress_stream<S, B>(
    bytes: S,
) -> Pin<Box<dyn Stream<Item = AppResult<PullProgress>> + Send>>
where
    S: Stream<Item = Result<B, reqwest::Error>> + Send + 'static,
    B: AsRef<[u8]>,
{
    Box::pin(
        crate::sse_lines::lines(bytes).filter_map(|line| async move {
            match line {
                Ok(line) if line.trim().is_empty() => None,
                Ok(line) => Some(
                    serde_json::from_str::<PullProgress>(&line).map_err(|error| {
                        AppError::Provider(format!("Invalid Ollama pull progress: {error}"))
                    }),
                ),
                Err(error) => Some(Err(AppError::Provider(
                    crate::http_client::format_stream_error(&error),
                ))),
            }
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn pull_progress_preserves_all_records_across_byte_boundaries() {
        let body = "{\"status\":\"pulling é🌍\"}\r\n\n{\"status\":\"success\"}";
        for cuts in crate::sse_lines::test_support::split_variants(body) {
            let progress: Vec<_> =
                pull_progress_stream(crate::sse_lines::test_support::reads_split_at(body, &cuts))
                    .collect()
                    .await;
            let statuses: Vec<_> = progress
                .into_iter()
                .map(|item| item.unwrap().status)
                .collect();
            assert_eq!(statuses, ["pulling é🌍", "success"], "cuts: {cuts:?}");
        }
    }

    #[tokio::test]
    async fn pull_progress_surfaces_provider_error_records() {
        let body = "{\"error\":\"pull failed\"}\n";
        let progress: Vec<_> =
            pull_progress_stream(crate::sse_lines::test_support::reads_split_at(body, &[]))
                .collect()
                .await;
        assert_eq!(progress.len(), 1);
        assert!(progress[0].is_err());
    }

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

    #[test]
    fn test_map_ollama_capabilities_tools_and_vision() {
        // Matches the user's failing model: qwen3.5:27b-q8_0 reports
        // ["completion", "vision", "tools", "thinking"].
        let caps = OllamaProvider::map_ollama_capabilities(&[
            "completion".to_string(),
            "vision".to_string(),
            "tools".to_string(),
            "thinking".to_string(),
        ]);
        assert!(caps.contains(&Capability::Chat));
        assert!(caps.contains(&Capability::Completion));
        assert!(caps.contains(&Capability::FunctionCalling));
        assert!(caps.contains(&Capability::Vision));
        assert!(!caps.contains(&Capability::Embedding));
    }

    #[test]
    fn test_map_ollama_capabilities_embedding_only() {
        let caps = OllamaProvider::map_ollama_capabilities(&["embedding".to_string()]);
        assert!(caps.contains(&Capability::Embedding));
        // Embedding-only models should not advertise chat/completion.
        assert!(!caps.contains(&Capability::Chat));
        assert!(!caps.contains(&Capability::Completion));
        assert!(!caps.contains(&Capability::FunctionCalling));
    }

    #[test]
    fn test_map_ollama_capabilities_case_insensitive() {
        let caps = OllamaProvider::map_ollama_capabilities(&[
            "Completion".to_string(),
            "TOOLS".to_string(),
        ]);
        assert!(caps.contains(&Capability::Chat));
        assert!(caps.contains(&Capability::FunctionCalling));
    }

    #[test]
    fn test_map_ollama_capabilities_empty() {
        let caps = OllamaProvider::map_ollama_capabilities(&[]);
        assert!(caps.is_empty());
    }

    /// `/api/chat` streams NDJSON; the final `done: true` message carries
    /// `prompt_eval_count` / `eval_count`.
    #[tokio::test]
    async fn stream_reports_final_eval_counts() {
        use crate::openai_compatible::stream_usage::test_support::*;

        let body = [
            r#"{"model":"llama3.2","created_at":"2026-09-27T10:00:00Z","message":{"role":"assistant","content":"Hello"},"done":false}"#,
            r#"{"model":"llama3.2","created_at":"2026-09-27T10:00:00Z","message":{"role":"assistant","content":" world"},"done":false}"#,
            r#"{"model":"llama3.2","created_at":"2026-09-27T10:00:01Z","message":{"role":"assistant","content":""},"done":true,"done_reason":"stop","total_duration":4883583458,"load_duration":1334875,"prompt_eval_count":26,"prompt_eval_duration":342546000,"eval_count":282,"eval_duration":4535599000}"#,
        ]
        .iter()
        .map(|l| format!("{l}\n"))
        .collect::<String>();
        let server = stream_server("/api/chat", body, "application/x-ndjson").await;
        let provider = OllamaProvider::with_base_url(server.uri());
        let stream = provider
            .stream_complete(stream_request("llama3.2"))
            .await
            .unwrap();
        let chunks = collect(stream).await;

        assert_eq!(chunks.len(), 3);
        assert_eq!(content(&chunks), "Hello world");
        assert_eq!(finish_reasons(&chunks), vec!["stop"]);
        assert!(chunks[..2].iter().all(|c| c.usage.is_none()));
        let usage = single_trailing_usage(&chunks);
        assert_eq!(
            (
                usage.prompt_tokens,
                usage.completion_tokens,
                usage.total_tokens
            ),
            (26, 282, 308)
        );
    }

    #[test]
    fn decision_capability_maps_to_decision() {
        // tev1 on Ollama 0.35 reports ["decision","tools","thinking","completion"].
        let caps = OllamaProvider::map_ollama_capabilities(&[
            "decision".to_string(),
            "tools".to_string(),
            "thinking".to_string(),
            "completion".to_string(),
        ]);
        assert!(caps.contains(&Capability::Decision));
        assert!(caps.contains(&Capability::Chat));
    }

    /// An Ollama 0.35 server with one decision model and one chat model.
    async fn decision_server() -> wiremock::MockServer {
        use wiremock::matchers::{body_partial_json, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/tags"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "models": [
                    {"name": "tev1:0.8b", "modified_at": "", "size": 1, "digest": "a"},
                    {"name": "llama3.2:latest", "modified_at": "", "size": 1, "digest": "b"}
                ]
            })))
            .mount(&server)
            .await;
        for (name, caps) in [
            (
                "tev1:0.8b",
                serde_json::json!(["decision", "tools", "thinking", "completion"]),
            ),
            ("tev1", serde_json::json!(["decision", "completion"])),
            (
                "llama3.2:latest",
                serde_json::json!(["completion", "tools"]),
            ),
        ] {
            Mock::given(method("POST"))
                .and(path("/api/show"))
                .and(body_partial_json(serde_json::json!({"name": name})))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_json(serde_json::json!({"capabilities": caps})),
                )
                .mount(&server)
                .await;
        }
        Mock::given(method("POST"))
            .and(path("/v1/systemone"))
            .and(body_partial_json(serde_json::json!({"model": "tev1:0.8b"})))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "model": "tev1:0.8b",
                "answers": {"urgent": {"type": "noul", "noul": 0.42}},
                "usage": {"input_tokens": 101, "output_tokens": 1}
            })))
            .expect(2)
            .mount(&server)
            .await;
        server
    }

    fn decision_request(model: Option<&str>) -> crate::SystemOneRequest {
        let mut v = serde_json::json!({
            "state": "card charged twice",
            "questions": {"urgent": {"type": "noul", "instructions": "Is this urgent?"}}
        });
        if let Some(m) = model {
            v["model"] = m.into();
        }
        serde_json::from_value(v).unwrap()
    }

    #[tokio::test]
    async fn decision_models_answer_systemone_natively() {
        let server = decision_server().await;
        let provider = OllamaProvider::with_base_url(server.uri());
        // Nothing known before the model list is read.
        assert!(!provider.supports_systemone());

        let models = provider.list_models().await.unwrap();
        let tev1 = models.iter().find(|m| m.id == "tev1:0.8b").unwrap();
        assert!(tev1.capabilities.contains(&Capability::Decision));
        assert!(provider.supports_systemone());
        assert!(provider.supports_systemone_model("tev1:0.8b").await);
        assert!(!provider.supports_systemone_model("llama3.2:latest").await);
        // A name not in the list yet is looked up with /api/show.
        assert!(provider.supports_systemone_model("tev1").await);

        let resp = provider
            .systemone(decision_request(Some("tev1:0.8b")))
            .await
            .unwrap();
        assert_eq!(resp.model, "tev1:0.8b");
        assert_eq!(resp.usage.input_tokens, Some(101));
        // No model: the (only) decision model the list reported.
        let resp = provider.systemone(decision_request(None)).await.unwrap();
        assert!(resp.answers.contains_key("urgent"));
    }

    #[tokio::test]
    async fn chat_returns_logprobs_when_asked() {
        use wiremock::matchers::{body_partial_json, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        // Shape observed from Ollama 0.35's /api/chat.
        Mock::given(method("POST"))
            .and(path("/api/chat"))
            .and(body_partial_json(
                serde_json::json!({"logprobs": true, "top_logprobs": 3}),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "model": "tev1:0.8b",
                "message": {"role": "assistant", "content": "A"},
                "done": true,
                "logprobs": [{
                    "token": "A", "logprob": -0.164, "bytes": [65],
                    "top_logprobs": [
                        {"token": "A", "logprob": -0.164, "bytes": [65]},
                        {"token": "B", "logprob": -1.895, "bytes": [66]}
                    ]
                }],
                "prompt_eval_count": 80,
                "eval_count": 2
            })))
            .mount(&server)
            .await;
        let provider = OllamaProvider::with_base_url(server.uri());
        assert!(provider.supports_feature("logprobs"));
        let mut req = CompletionRequest::new(
            "tev1:0.8b",
            vec![ChatMessage {
                role: "user".to_string(),
                content: crate::ChatMessageContent::Text("pick".to_string()),
                name: None,
                tool_calls: None,
                tool_call_id: None,
                reasoning_content: None,
            }],
        );
        req.logprobs = Some(true);
        req.top_logprobs = Some(3);
        let resp = provider.complete(req).await.unwrap();
        let content = resp.choices[0]
            .logprobs
            .as_ref()
            .and_then(|l| l.content.as_ref())
            .unwrap();
        assert_eq!(content[0].token, "A");
        assert_eq!(content[0].top_logprobs.len(), 2);
        // The router's letter mode reads these as option probabilities.
        let dist = crate::systemone::emulation::parse_letter_response(&resp, 2).unwrap();
        assert!(dist[0] > dist[1]);
    }
}
