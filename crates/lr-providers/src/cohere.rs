//! Cohere provider implementation
//!
//! Implements the ModelProvider trait for Cohere's LLM API.
//! Cohere offers models like Command R+, Command R, and specialized embedding models.

use async_trait::async_trait;
use chrono::Utc;
use futures::stream::{Stream, StreamExt};
use reqwest_middleware::ClientWithMiddleware;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::pin::Pin;
use std::time::Instant;

use lr_types::{AppError, AppResult};

use super::{
    Capability, ChatMessage, ChunkChoice, ChunkDelta, CompletionChoice, CompletionChunk,
    CompletionRequest, CompletionResponse, FunctionCallDelta, HealthStatus, ModelInfo,
    ModelProvider, PricingInfo, PromptTokensDetails, ProviderHealth, TokenUsage, ToolCall,
    ToolCallDelta, ToolChoice,
};

const COHERE_API_BASE: &str = "https://api.cohere.com/v2";

/// Cohere AI provider
pub struct CohereProvider {
    client: ClientWithMiddleware,
    api_key: String,
    base_url: String,
}

#[allow(dead_code)]
impl CohereProvider {
    /// Create a new Cohere provider with an API key
    pub fn new(api_key: String) -> AppResult<Self> {
        Self::with_base_url(api_key, COHERE_API_BASE.to_string())
    }

    /// Create a new Cohere provider with a custom base URL (for testing)
    pub fn with_base_url(api_key: String, base_url: String) -> AppResult<Self> {
        let client = crate::http_client::extended_client()?;

        Ok(Self {
            client,
            api_key,
            base_url: base_url.trim_end_matches('/').to_string(),
        })
    }

    /// Create a new Cohere provider from stored API key
    pub fn from_stored_key(provider_name: Option<&str>) -> AppResult<Self> {
        let name = provider_name.unwrap_or("cohere");
        let api_key = super::key_storage::get_provider_key(name)?.ok_or_else(|| {
            AppError::Provider(format!("No API key found for provider '{}'", name))
        })?;
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
                detailed_capabilities: None,
            },
            ModelInfo {
                id: "command-r".to_string(),
                name: "Command R".to_string(),
                provider: "cohere".to_string(),
                parameter_count: Some(35_000_000_000),
                context_window: 128_000,
                supports_streaming: true,
                capabilities: vec![Capability::Chat, Capability::FunctionCalling],
                detailed_capabilities: None,
            },
            ModelInfo {
                id: "command".to_string(),
                name: "Command".to_string(),
                provider: "cohere".to_string(),
                parameter_count: None,
                context_window: 4096,
                supports_streaming: true,
                capabilities: vec![Capability::Chat],
                detailed_capabilities: None,
            },
            ModelInfo {
                id: "command-light".to_string(),
                name: "Command Light".to_string(),
                provider: "cohere".to_string(),
                parameter_count: None,
                context_window: 4096,
                supports_streaming: true,
                capabilities: vec![Capability::Chat],
                detailed_capabilities: None,
            },
        ]
    }

    /// POST a chat request to `{base}/chat`, failing on a non-2xx status.
    async fn send_chat(
        &self,
        request: &CompletionRequest,
        stream: bool,
    ) -> AppResult<reqwest::Response> {
        let url = format!("{}/chat", self.base_url);
        let cohere_request = self.convert_to_cohere_request(request, stream)?;

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

        Ok(response)
    }

    /// Build a Cohere v2 `/chat` request body from an OpenAI-shaped request.
    fn convert_to_cohere_request(
        &self,
        request: &CompletionRequest,
        stream: bool,
    ) -> AppResult<CohereRequest> {
        let messages = request.messages.iter().map(to_cohere_message).collect();

        let (tools, tool_choice) = match request.tools.as_deref() {
            Some(tools) if !tools.is_empty() => {
                let (choice, forced) = match request.tool_choice.as_ref() {
                    Some(ToolChoice::Auto(mode)) => match mode.as_str() {
                        "none" => (Some("NONE"), None),
                        "required" => (Some("REQUIRED"), None),
                        _ => (None, None),
                    },
                    // Cohere cannot force one named tool; offer only that
                    // tool and require a call.
                    Some(ToolChoice::Specific { function, .. }) => {
                        (Some("REQUIRED"), Some(function.name.as_str()))
                    }
                    None => (None, None),
                };
                let tools: Vec<CohereTool> = tools
                    .iter()
                    .filter(|t| forced.is_none_or(|name| t.function.name == name))
                    .map(|t| CohereTool {
                        tool_type: "function".to_string(),
                        function: CohereFunction {
                            name: t.function.name.clone(),
                            description: t.function.description.clone(),
                            parameters: t.function.parameters.clone(),
                        },
                    })
                    .collect();
                (Some(tools), choice.map(str::to_string))
            }
            _ => (None, None),
        };

        Ok(CohereRequest {
            model: request.model.clone(),
            messages,
            tools,
            tool_choice,
            temperature: request.temperature,
            max_tokens: request.max_tokens,
            p: request.top_p,
            k: request.top_k,
            frequency_penalty: request.frequency_penalty,
            presence_penalty: request.presence_penalty,
            stop_sequences: request.stop.clone(),
            seed: request.seed,
            stream,
        })
    }
}

/// Convert one OpenAI chat message to a Cohere v2 message.
fn to_cohere_message(msg: &ChatMessage) -> CohereMessage {
    let text = msg.content.as_text();
    match msg.role.as_str() {
        "system" | "developer" => CohereMessage::text("system", text),
        "assistant" => {
            let tool_calls = msg.tool_calls.clone().filter(|calls| !calls.is_empty());
            let has_calls = tool_calls.is_some();
            CohereMessage {
                role: "assistant".to_string(),
                // `content` is optional on an assistant turn that calls tools.
                content: if has_calls && text.is_empty() {
                    None
                } else {
                    Some(text)
                },
                tool_calls,
                tool_call_id: None,
                // Cohere's tool plan comes back as `reasoning_content`; hand
                // it back with the calls it planned.
                tool_plan: if has_calls {
                    msg.reasoning_content.clone()
                } else {
                    None
                },
            }
        }
        "tool" => CohereMessage {
            tool_call_id: msg.tool_call_id.clone(),
            ..CohereMessage::text("tool", text)
        },
        _ => CohereMessage::text("user", text),
    }
}

/// Map a Cohere `finish_reason` to the OpenAI value.
fn map_finish_reason(reason: &str) -> String {
    match reason {
        "COMPLETE" | "STOP_SEQUENCE" => "stop".to_string(),
        "MAX_TOKENS" => "length".to_string(),
        "TOOL_CALL" => "tool_calls".to_string(),
        "ERROR" | "TIMEOUT" => "error".to_string(),
        other => other.to_ascii_lowercase(),
    }
}

/// Cohere v2 `/chat` request body.
#[derive(Debug, Serialize)]
struct CohereRequest {
    model: String,
    messages: Vec<CohereMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<CohereTool>>,
    /// `REQUIRED` or `NONE`; absent lets the model decide.
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    /// Top-p sampling (Cohere calls it "p")
    #[serde(skip_serializing_if = "Option::is_none")]
    p: Option<f32>,
    /// Top-k sampling (Cohere calls it "k")
    #[serde(skip_serializing_if = "Option::is_none")]
    k: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    frequency_penalty: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    presence_penalty: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stop_sequences: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    seed: Option<i64>,
    stream: bool,
}

#[derive(Debug, Serialize)]
struct CohereMessage {
    role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<String>,
    /// Same shape as OpenAI: `{id, type, function: {name, arguments}}`.
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<ToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_plan: Option<String>,
}

impl CohereMessage {
    fn text(role: &str, content: String) -> Self {
        Self {
            role: role.to_string(),
            content: Some(content),
            tool_calls: None,
            tool_call_id: None,
            tool_plan: None,
        }
    }
}

#[derive(Debug, Serialize)]
struct CohereTool {
    #[serde(rename = "type")]
    tool_type: String,
    function: CohereFunction,
}

#[derive(Debug, Serialize)]
struct CohereFunction {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    parameters: serde_json::Value,
}

/// Cohere v2 non-streaming `/chat` response.
#[derive(Debug, Deserialize)]
struct CohereResponse {
    id: String,
    message: CohereResponseMessage,
    #[serde(default)]
    finish_reason: Option<String>,
    #[serde(default)]
    usage: Option<CohereUsage>,
}

#[derive(Debug, Deserialize)]
struct CohereResponseMessage {
    #[serde(default)]
    content: Option<Vec<CohereContent>>,
    #[serde(default)]
    tool_calls: Option<Vec<ToolCall>>,
    #[serde(default)]
    tool_plan: Option<String>,
}

/// A response content block: `{type: "text", text}` or
/// `{type: "thinking", thinking}`.
#[derive(Debug, Deserialize)]
struct CohereContent {
    #[serde(rename = "type")]
    content_type: String,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    thinking: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct CohereUsage {
    #[serde(default)]
    billed_units: Option<CohereTokens>,
    #[serde(default)]
    tokens: Option<CohereTokens>,
    /// Prompt tokens that hit the inference cache (a subset of input).
    #[serde(default)]
    cached_tokens: Option<f64>,
}

/// Token counts; Cohere documents these as numbers, so accept floats.
#[derive(Debug, Default, Deserialize)]
struct CohereTokens {
    #[serde(default)]
    input_tokens: Option<f64>,
    #[serde(default)]
    output_tokens: Option<f64>,
}

impl CohereUsage {
    /// Actual token counts (`tokens`), falling back to `billed_units`.
    /// `None` when neither reports anything.
    fn to_token_usage(&self) -> Option<TokenUsage> {
        let pick = |f: fn(&CohereTokens) -> Option<f64>| {
            self.tokens
                .as_ref()
                .and_then(f)
                .or_else(|| self.billed_units.as_ref().and_then(f))
        };
        let input = pick(|t| t.input_tokens);
        let output = pick(|t| t.output_tokens);
        if input.is_none() && output.is_none() {
            return None;
        }
        let prompt_tokens = input.unwrap_or(0.0).round() as u32;
        let completion_tokens = output.unwrap_or(0.0).round() as u32;
        let cached = self
            .cached_tokens
            .map(|c| c.round() as u32)
            .filter(|&c| c > 0);
        Some(TokenUsage {
            prompt_tokens,
            completion_tokens,
            total_tokens: prompt_tokens + completion_tokens,
            prompt_tokens_details: cached.map(|c| PromptTokensDetails {
                cached_tokens: Some(c),
                cache_creation_tokens: None,
                cache_read_tokens: None,
            }),
            completion_tokens_details: None,
        })
    }
}

/// One Cohere v2 stream event (the `data:` payload of an SSE frame).
#[derive(Debug, Deserialize)]
struct CohereStreamEvent {
    #[serde(rename = "type")]
    event_type: String,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    index: Option<u32>,
    #[serde(default)]
    delta: Option<CohereStreamDelta>,
}

#[derive(Debug, Deserialize)]
struct CohereStreamDelta {
    #[serde(default)]
    message: Option<CohereStreamMessage>,
    #[serde(default)]
    finish_reason: Option<String>,
    #[serde(default)]
    usage: Option<CohereUsage>,
    #[serde(default)]
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CohereStreamMessage {
    #[serde(default)]
    content: Option<CohereStreamContent>,
    #[serde(default)]
    tool_plan: Option<String>,
    /// A single call object (not an array) in `tool-call-*` events.
    #[serde(default)]
    tool_calls: Option<CohereStreamToolCall>,
}

#[derive(Debug, Deserialize)]
struct CohereStreamContent {
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    thinking: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CohereStreamToolCall {
    #[serde(default)]
    id: Option<String>,
    #[serde(default, rename = "type")]
    tool_type: Option<String>,
    #[serde(default)]
    function: Option<CohereStreamFunction>,
}

#[derive(Debug, Deserialize)]
struct CohereStreamFunction {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}

/// Append `incoming` to `buffer` and return every complete line (without
/// its `\n` / `\r\n`), leaving a trailing partial line buffered. Works on
/// bytes so a multi-byte character split across reads survives.
fn drain_complete_lines(buffer: &mut Vec<u8>, incoming: &[u8]) -> Vec<String> {
    buffer.extend_from_slice(incoming);
    let mut lines = Vec::new();
    while let Some(pos) = buffer.iter().position(|&b| b == b'\n') {
        let line: Vec<u8> = buffer.drain(..=pos).collect();
        let line = String::from_utf8_lossy(&line[..pos]);
        lines.push(line.trim_end_matches('\r').to_string());
    }
    lines
}

/// Turns Cohere v2 stream bytes into OpenAI-shaped chunks.
struct CohereStreamParser {
    id: String,
    model: String,
    created: i64,
    buffer: Vec<u8>,
    /// Cohere's tool-call `index` -> our stable OpenAI tool-call index.
    tool_indices: HashMap<u32, u32>,
    next_tool_index: u32,
    current_tool_index: Option<u32>,
}

impl CohereStreamParser {
    fn new(model: String) -> Self {
        Self {
            id: format!("cohere-{}", uuid::Uuid::new_v4()),
            model,
            created: Utc::now().timestamp(),
            buffer: Vec::new(),
            tool_indices: HashMap::new(),
            next_tool_index: 0,
            current_tool_index: None,
        }
    }

    /// Feed one network read; returns the chunks its complete lines yield.
    fn push(&mut self, bytes: &[u8]) -> Vec<AppResult<CompletionChunk>> {
        drain_complete_lines(&mut self.buffer, bytes)
            .into_iter()
            .filter_map(|line| self.handle_line(&line))
            .collect()
    }

    /// End of stream: parse a final line that had no trailing newline.
    fn finish(&mut self) -> Vec<AppResult<CompletionChunk>> {
        if self.buffer.is_empty() {
            return Vec::new();
        }
        let rest = std::mem::take(&mut self.buffer);
        let line = String::from_utf8_lossy(&rest).into_owned();
        self.handle_line(&line).into_iter().collect()
    }

    fn handle_line(&mut self, line: &str) -> Option<AppResult<CompletionChunk>> {
        let line = line.trim();
        // SSE `data:` frames; also accept bare JSON lines (NDJSON). `event:`,
        // `id:`, `retry:` and `:` comments are skipped: the payload carries
        // its own `type`.
        let data = match line.strip_prefix("data:") {
            Some(rest) => rest.trim_start(),
            None if line.starts_with('{') => line,
            None => return None,
        };
        if data.is_empty() || data == "[DONE]" {
            return None;
        }
        match serde_json::from_str::<CohereStreamEvent>(data) {
            Ok(event) => self.handle_event(event).map(Ok),
            Err(e) => Some(Err(AppError::Provider(format!(
                "Failed to parse Cohere stream event: {}",
                e
            )))),
        }
    }

    fn handle_event(&mut self, event: CohereStreamEvent) -> Option<CompletionChunk> {
        let message = event.delta.as_ref().and_then(|d| d.message.as_ref());
        match event.event_type.as_str() {
            "message-start" => {
                if let Some(id) = event.id {
                    self.id = id;
                }
                Some(self.chunk(
                    ChunkDelta {
                        role: Some("assistant".to_string()),
                        ..empty_delta()
                    },
                    None,
                    None,
                ))
            }
            "content-delta" => {
                let content = message?.content.as_ref()?;
                if content.text.is_none() && content.thinking.is_none() {
                    return None;
                }
                Some(self.chunk(
                    ChunkDelta {
                        content: content.text.clone(),
                        reasoning_content: content.thinking.clone(),
                        ..empty_delta()
                    },
                    None,
                    None,
                ))
            }
            "tool-plan-delta" => {
                let plan = message?.tool_plan.clone()?;
                Some(self.chunk(
                    ChunkDelta {
                        reasoning_content: Some(plan),
                        ..empty_delta()
                    },
                    None,
                    None,
                ))
            }
            "tool-call-start" => {
                let call = message?.tool_calls.as_ref()?;
                let index = self.next_tool_index;
                self.next_tool_index += 1;
                if let Some(upstream) = event.index {
                    self.tool_indices.insert(upstream, index);
                }
                self.current_tool_index = Some(index);
                let function = call.function.as_ref();
                Some(
                    self.tool_chunk(ToolCallDelta {
                        index,
                        id: call.id.clone(),
                        tool_type: Some(
                            call.tool_type
                                .clone()
                                .unwrap_or_else(|| "function".to_string()),
                        ),
                        function: Some(FunctionCallDelta {
                            name: function.and_then(|f| f.name.clone()),
                            arguments: Some(
                                function
                                    .and_then(|f| f.arguments.clone())
                                    .unwrap_or_default(),
                            ),
                        }),
                    }),
                )
            }
            "tool-call-delta" => {
                let arguments = message?
                    .tool_calls
                    .as_ref()?
                    .function
                    .as_ref()?
                    .arguments
                    .clone()?;
                let index = event
                    .index
                    .and_then(|i| self.tool_indices.get(&i).copied())
                    .or(self.current_tool_index)?;
                Some(self.tool_chunk(ToolCallDelta {
                    index,
                    id: None,
                    tool_type: None,
                    function: Some(FunctionCallDelta {
                        name: None,
                        arguments: Some(arguments),
                    }),
                }))
            }
            "message-end" => {
                let delta = event.delta.as_ref();
                let reason = delta.and_then(|d| d.finish_reason.as_deref());
                if let Some(error) = delta.and_then(|d| d.error.as_deref()) {
                    tracing::warn!("Cohere stream ended with error: {}", error);
                }
                let usage = delta
                    .and_then(|d| d.usage.as_ref())
                    .and_then(CohereUsage::to_token_usage);
                Some(self.chunk(
                    empty_delta(),
                    Some(reason.map_or_else(|| "stop".to_string(), map_finish_reason)),
                    usage,
                ))
            }
            // content-start/-end, tool-call-end, citation-start/-end, debug.
            _ => None,
        }
    }

    fn tool_chunk(&self, call: ToolCallDelta) -> CompletionChunk {
        self.chunk(
            ChunkDelta {
                tool_calls: Some(vec![call]),
                ..empty_delta()
            },
            None,
            None,
        )
    }

    fn chunk(
        &self,
        delta: ChunkDelta,
        finish_reason: Option<String>,
        usage: Option<TokenUsage>,
    ) -> CompletionChunk {
        CompletionChunk {
            id: self.id.clone(),
            object: "chat.completion.chunk".to_string(),
            created: self.created,
            model: self.model.clone(),
            choices: vec![ChunkChoice {
                index: 0,
                delta,
                finish_reason,
            }],
            extensions: None,
            usage,
            provider: None,
        }
    }
}

fn empty_delta() -> ChunkDelta {
    ChunkDelta {
        role: None,
        content: None,
        tool_calls: None,
        reasoning_content: None,
    }
}

// Cohere Embeddings API types
#[derive(Debug, Serialize)]
struct CohereEmbedRequest {
    model: String,
    texts: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    input_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    embedding_types: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct CohereEmbedResponse {
    id: String,
    embeddings: CohereEmbeddings,
    texts: Vec<String>,
    #[allow(dead_code)]
    meta: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct CohereEmbeddings {
    #[serde(default)]
    float: Option<Vec<Vec<f32>>>,
}

// Cohere Models API response types (v1 endpoint)
#[derive(Debug, Deserialize)]
struct CohereModelsResponse {
    models: Vec<CohereModel>,
}

#[derive(Debug, Deserialize)]
struct CohereModel {
    name: String,
    #[serde(default)]
    endpoints: Vec<String>,
    #[serde(default)]
    context_length: Option<u32>,
}

#[async_trait]
#[allow(dead_code)]
impl ModelProvider for CohereProvider {
    fn name(&self) -> &str {
        "cohere"
    }

    fn health_check_interval_multiplier(&self) -> u32 {
        6 // Check every 60 min (6 × 10 min) — fits within 1000 calls/month trial limit
    }

    async fn health_check(&self) -> ProviderHealth {
        let start = Instant::now();

        // Query a single model via GET /v1/models/{id} instead of listing all models.
        // Accept both 200 (exists) and 404 (retired but API up, auth valid).
        // A bad API key returns 401, correctly treated as unhealthy.
        let result = self
            .client
            .get("https://api.cohere.com/v1/models/command-r")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .send()
            .await;

        let latency_ms = start.elapsed().as_millis() as u64;

        match result {
            Ok(response) => {
                let status = response.status();
                if status.is_success() || status.as_u16() == 404 {
                    ProviderHealth {
                        status: HealthStatus::Healthy,
                        latency_ms: Some(latency_ms),
                        last_checked: Utc::now(),
                        error_message: None,
                    }
                } else if status.as_u16() == 429 {
                    ProviderHealth {
                        status: HealthStatus::Degraded,
                        latency_ms: Some(latency_ms),
                        last_checked: Utc::now(),
                        error_message: Some("Rate limited (HTTP 429)".to_string()),
                    }
                } else if status.is_server_error() {
                    ProviderHealth {
                        status: HealthStatus::Degraded,
                        latency_ms: Some(latency_ms),
                        last_checked: Utc::now(),
                        error_message: Some(format!("Server error (HTTP {})", status)),
                    }
                } else {
                    ProviderHealth {
                        status: HealthStatus::Unhealthy,
                        latency_ms: Some(latency_ms),
                        last_checked: Utc::now(),
                        error_message: Some(format!("API returned status {}", status)),
                    }
                }
            }
            Err(e) => ProviderHealth {
                status: HealthStatus::Unhealthy,
                latency_ms: None,
                last_checked: Utc::now(),
                error_message: Some(format!("Connection failed: {}", e)),
            },
        }
    }

    async fn list_models(&self) -> AppResult<Vec<ModelInfo>> {
        // Use v1 models endpoint (not v2) to list available models
        // This also validates the API key is correct
        let models_url = "https://api.cohere.com/v1/models";

        let response = self
            .client
            .get(models_url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .query(&[("page_size", "100")])
            .send()
            .await
            .map_err(|e| AppError::Provider(format!("Failed to fetch Cohere models: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(AppError::Provider(format!(
                "Cohere models API error {}: {}",
                status, error_text
            )));
        }

        let models_response: CohereModelsResponse = response.json().await.map_err(|e| {
            AppError::Provider(format!("Failed to parse Cohere models response: {}", e))
        })?;

        // Convert Cohere models to our format
        let models = models_response
            .models
            .into_iter()
            .filter(|m| m.endpoints.contains(&"chat".to_string()))
            .map(|m| {
                let capabilities = if m.endpoints.contains(&"embed".to_string()) {
                    vec![Capability::Chat, Capability::Embedding]
                } else {
                    vec![Capability::Chat]
                };

                ModelInfo {
                    id: m.name.clone(),
                    name: m.name,
                    provider: "cohere".to_string(),
                    parameter_count: None,
                    context_window: m.context_length.unwrap_or(128_000_u32),
                    supports_streaming: true,
                    capabilities,
                    detailed_capabilities: None,
                }
            })
            .collect();

        Ok(models)
    }

    async fn get_pricing(&self, model: &str) -> AppResult<PricingInfo> {
        // Try catalog first (embedded OpenRouter data)
        if let Some(catalog_model) = lr_catalog::find_model("cohere", model) {
            tracing::debug!("Using catalog pricing for Cohere model: {}", model);
            return Ok(PricingInfo {
                input_cost_per_1k: catalog_model.pricing.prompt_cost_per_1k(),
                output_cost_per_1k: catalog_model.pricing.completion_cost_per_1k(),
                reasoning_cost_per_1k: catalog_model.pricing.reasoning_cost_per_1k(),
                cache_read_cost_per_1k: catalog_model.pricing.cache_read_cost_per_1k(),
                cache_write_cost_per_1k: catalog_model.pricing.cache_write_cost_per_1k(),
                currency: catalog_model.pricing.currency.to_string(),
            });
        }

        // Fallback to hardcoded pricing
        tracing::debug!("Using fallback pricing for Cohere model: {}", model);

        // Cohere pricing as of 2026-01
        let pricing = match model {
            "command-r-plus" => PricingInfo {
                input_cost_per_1k: 0.003,  // $3 per 1M tokens
                output_cost_per_1k: 0.015, // $15 per 1M tokens
                reasoning_cost_per_1k: None,
                cache_read_cost_per_1k: None,
                cache_write_cost_per_1k: None,
                currency: "USD".to_string(),
            },
            "command-r" => PricingInfo {
                input_cost_per_1k: 0.0005,  // $0.5 per 1M tokens
                output_cost_per_1k: 0.0015, // $1.5 per 1M tokens
                reasoning_cost_per_1k: None,
                cache_read_cost_per_1k: None,
                cache_write_cost_per_1k: None,
                currency: "USD".to_string(),
            },
            "command" => PricingInfo {
                input_cost_per_1k: 0.001,  // $1 per 1M tokens
                output_cost_per_1k: 0.002, // $2 per 1M tokens
                reasoning_cost_per_1k: None,
                cache_read_cost_per_1k: None,
                cache_write_cost_per_1k: None,
                currency: "USD".to_string(),
            },
            "command-light" => PricingInfo {
                input_cost_per_1k: 0.0003,  // $0.3 per 1M tokens
                output_cost_per_1k: 0.0006, // $0.6 per 1M tokens
                reasoning_cost_per_1k: None,
                cache_read_cost_per_1k: None,
                cache_write_cost_per_1k: None,
                currency: "USD".to_string(),
            },
            _ => PricingInfo {
                input_cost_per_1k: 0.001,
                output_cost_per_1k: 0.002,
                reasoning_cost_per_1k: None,
                cache_read_cost_per_1k: None,
                cache_write_cost_per_1k: None,
                currency: "USD".to_string(),
            },
        };

        Ok(pricing)
    }

    async fn complete(&self, request: CompletionRequest) -> AppResult<CompletionResponse> {
        let response = self.send_chat(&request, false).await?;

        let cohere_response: CohereResponse = response
            .json()
            .await
            .map_err(|e| AppError::Provider(format!("Failed to parse Cohere response: {}", e)))?;

        let mut text = Vec::new();
        let mut reasoning = Vec::new();
        for block in cohere_response.message.content.unwrap_or_default() {
            match block.content_type.as_str() {
                "text" => text.extend(block.text),
                "thinking" => reasoning.extend(block.thinking),
                _ => {}
            }
        }
        reasoning.extend(cohere_response.message.tool_plan);
        let tool_calls = cohere_response
            .message
            .tool_calls
            .filter(|calls| !calls.is_empty());

        let usage = cohere_response
            .usage
            .as_ref()
            .and_then(CohereUsage::to_token_usage)
            .unwrap_or(TokenUsage {
                prompt_tokens: 0,
                completion_tokens: 0,
                total_tokens: 0,
                prompt_tokens_details: None,
                completion_tokens_details: None,
            });

        Ok(CompletionResponse {
            id: cohere_response.id,
            object: "chat.completion".to_string(),
            created: Utc::now().timestamp(),
            model: request.model,
            provider: self.name().to_string(),
            choices: vec![CompletionChoice {
                index: 0,
                message: ChatMessage {
                    role: "assistant".to_string(),
                    content: super::ChatMessageContent::Text(text.join("\n")),
                    tool_calls,
                    tool_call_id: None,
                    name: None,
                    reasoning_content: if reasoning.is_empty() {
                        None
                    } else {
                        Some(reasoning.join("\n"))
                    },
                },
                finish_reason: cohere_response
                    .finish_reason
                    .as_deref()
                    .map(map_finish_reason),
                logprobs: None, // Cohere does not support logprobs
            }],
            usage,
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
        let response = self.send_chat(&request, true).await?;

        let parser = CohereStreamParser::new(request.model.clone());
        let upstream = response.bytes_stream().boxed();
        let stream = futures::stream::unfold(
            (upstream, parser, VecDeque::new(), false),
            |(mut upstream, mut parser, mut pending, mut done)| async move {
                loop {
                    if let Some(item) = pending.pop_front() {
                        return Some((item, (upstream, parser, pending, done)));
                    }
                    if done {
                        return None;
                    }
                    match upstream.next().await {
                        Some(Ok(bytes)) => pending.extend(parser.push(&bytes)),
                        Some(Err(e)) => {
                            done = true;
                            pending.push_back(Err(AppError::Provider(format!(
                                "Cohere stream error: {}",
                                e
                            ))));
                        }
                        None => {
                            done = true;
                            pending.extend(parser.finish());
                        }
                    }
                }
            },
        );

        Ok(Box::pin(stream))
    }

    fn supports_embeddings(&self) -> bool {
        true
    }

    async fn embed(&self, request: super::EmbeddingRequest) -> AppResult<super::EmbeddingResponse> {
        // Convert input to Cohere format (only supports multiple texts)
        let texts = match request.input {
            super::EmbeddingInput::Single(text) => vec![text],
            super::EmbeddingInput::Multiple(texts) => texts,
            super::EmbeddingInput::Tokens(_) => {
                return Err(AppError::Provider(
                    "Cohere embeddings do not support pre-tokenized input".to_string(),
                ));
            }
        };

        // Cohere requires input_type for v3 models
        // Default to "search_document" for general purpose embeddings
        let cohere_request = CohereEmbedRequest {
            model: request.model.clone(),
            texts,
            input_type: Some("search_document".to_string()),
            embedding_types: Some(vec!["float".to_string()]),
        };

        let url = format!("{}/embed", self.base_url);

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&cohere_request)
            .send()
            .await
            .map_err(|e| AppError::Provider(format!("Cohere embed request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(AppError::Provider(format!(
                "Cohere embed API error {}: {}",
                status, error_text
            )));
        }

        let cohere_response: CohereEmbedResponse = response.json().await.map_err(|e| {
            AppError::Provider(format!("Failed to parse Cohere embed response: {}", e))
        })?;

        // Convert Cohere response to our generic format
        let embeddings = cohere_response
            .embeddings
            .float
            .ok_or_else(|| AppError::Provider("No float embeddings in response".to_string()))?;

        // Estimate token usage (Cohere doesn't return this for embeddings)
        let total_chars: usize = cohere_response.texts.iter().map(|t| t.len()).sum();
        let estimated_tokens = (total_chars / 4).max(1) as u32; // Rough estimate: 4 chars per token

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
                prompt_tokens: estimated_tokens,
                total_tokens: estimated_tokens,
            },
        })
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

    use crate::openai_compatible::stream_usage::test_support::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// Cohere v2 SSE body: an `event:` line and a `data:` line per event.
    fn cohere_sse(events: &[serde_json::Value]) -> String {
        events
            .iter()
            .map(|e| format!("event: {}\ndata: {}\n\n", e["type"].as_str().unwrap(), e))
            .collect()
    }

    fn provider_for(server: &MockServer) -> CohereProvider {
        CohereProvider::with_base_url("k".to_string(), server.uri()).unwrap()
    }

    /// Tool-call deltas assembled per index: (id, name, arguments).
    fn assembled_tool_calls(chunks: &[CompletionChunk]) -> Vec<(String, String, String)> {
        let mut calls: Vec<(String, String, String)> = Vec::new();
        for delta in chunks
            .iter()
            .flat_map(|c| c.choices.iter())
            .flat_map(|c| c.delta.tool_calls.iter().flatten())
        {
            let i = delta.index as usize;
            if calls.len() <= i {
                calls.resize(i + 1, Default::default());
            }
            if let Some(id) = &delta.id {
                calls[i].0.push_str(id);
            }
            if let Some(f) = &delta.function {
                calls[i].1.push_str(f.name.as_deref().unwrap_or(""));
                calls[i].2.push_str(f.arguments.as_deref().unwrap_or(""));
            }
        }
        calls
    }

    #[tokio::test]
    async fn stream_reports_upstream_usage() {
        let body = cohere_sse(&[
            serde_json::json!({"type": "message-start", "id": "msg-1", "delta": {"message": {"role": "assistant"}}}),
            serde_json::json!({"type": "content-start", "index": 0, "delta": {"message": {"content": {"type": "text", "text": ""}}}}),
            serde_json::json!({"type": "content-delta", "index": 0, "delta": {"message": {"content": {"text": "Hello"}}}}),
            serde_json::json!({"type": "content-delta", "index": 0, "delta": {"message": {"content": {"text": " world"}}}}),
            serde_json::json!({"type": "content-end", "index": 0}),
            serde_json::json!({"type": "message-end", "delta": {"finish_reason": "COMPLETE", "usage": {
                "billed_units": {"input_tokens": 3, "output_tokens": 5},
                "tokens": {"input_tokens": 12, "output_tokens": 5},
                "cached_tokens": 4
            }}}),
        ]);
        let server = sse_server("/chat", body).await;
        let stream = provider_for(&server)
            .stream_complete(stream_request("command-r"))
            .await
            .unwrap();
        let chunks = collect(stream).await;

        assert_eq!(content(&chunks), "Hello world");
        assert_eq!(finish_reasons(&chunks), vec!["stop"]);
        assert_eq!(
            chunks[0].choices[0].delta.role.as_deref(),
            Some("assistant")
        );
        assert!(chunks.iter().all(|c| c.id == "msg-1"
            && c.model == "command-r"
            && c.object == "chat.completion.chunk"
            && c.provider.is_none()));

        // `tokens` wins over `billed_units`.
        let usage = single_trailing_usage(&chunks);
        assert_eq!(usage.prompt_tokens, 12);
        assert_eq!(usage.completion_tokens, 5);
        assert_eq!(usage.total_tokens, 17);
        assert_eq!(usage.prompt_tokens_details.unwrap().cached_tokens, Some(4));

        let sent = received_body(&server).await;
        assert_eq!(sent["stream"], true);
        assert_eq!(sent["model"], "command-r");
        assert_eq!(
            sent["messages"],
            serde_json::json!([{"role": "user", "content": "hi"}])
        );
    }

    #[tokio::test]
    async fn stream_usage_falls_back_to_billed_units() {
        let body = cohere_sse(&[
            serde_json::json!({"type": "content-delta", "index": 0, "delta": {"message": {"content": {"text": "x"}}}}),
            serde_json::json!({"type": "message-end", "delta": {"finish_reason": "MAX_TOKENS", "usage": {
                "billed_units": {"input_tokens": 7, "output_tokens": 2}
            }}}),
        ]);
        let server = sse_server("/chat", body).await;
        let chunks = collect(
            provider_for(&server)
                .stream_complete(stream_request("command-r"))
                .await
                .unwrap(),
        )
        .await;

        assert_eq!(finish_reasons(&chunks), vec!["length"]);
        let usage = single_trailing_usage(&chunks);
        assert_eq!(
            (
                usage.prompt_tokens,
                usage.completion_tokens,
                usage.total_tokens
            ),
            (7, 2, 9)
        );
        assert!(usage.prompt_tokens_details.is_none());
    }

    #[tokio::test]
    async fn stream_tool_calls() {
        let body = cohere_sse(&[
            serde_json::json!({"type": "message-start", "id": "msg-2", "delta": {"message": {"role": "assistant"}}}),
            serde_json::json!({"type": "tool-plan-delta", "delta": {"message": {"tool_plan": "I will check "}}}),
            serde_json::json!({"type": "tool-plan-delta", "delta": {"message": {"tool_plan": "the weather."}}}),
            serde_json::json!({"type": "tool-call-start", "index": 0, "delta": {"message": {"tool_calls": {
                "id": "call_a", "type": "function", "function": {"name": "get_weather", "arguments": ""}
            }}}}),
            serde_json::json!({"type": "tool-call-delta", "index": 0, "delta": {"message": {"tool_calls": {"function": {"arguments": "{\"city\":"}}}}}),
            serde_json::json!({"type": "tool-call-delta", "index": 0, "delta": {"message": {"tool_calls": {"function": {"arguments": "\"Paris\"}"}}}}}),
            serde_json::json!({"type": "tool-call-end", "index": 0}),
            serde_json::json!({"type": "tool-call-start", "index": 1, "delta": {"message": {"tool_calls": {
                "id": "call_b", "type": "function", "function": {"name": "get_time", "arguments": ""}
            }}}}),
            serde_json::json!({"type": "tool-call-delta", "index": 1, "delta": {"message": {"tool_calls": {"function": {"arguments": "{}"}}}}}),
            serde_json::json!({"type": "tool-call-end", "index": 1}),
            serde_json::json!({"type": "message-end", "delta": {"finish_reason": "TOOL_CALL", "usage": {
                "tokens": {"input_tokens": 30, "output_tokens": 20}
            }}}),
        ]);
        let server = sse_server("/chat", body).await;

        let mut request = stream_request("command-r");
        request.tools = Some(vec![serde_json::from_value(serde_json::json!({
            "type": "function",
            "function": {"name": "get_weather", "description": "Weather", "parameters": {"type": "object"}, "strict": true}
        }))
        .unwrap()]);
        request.tool_choice = Some(ToolChoice::Auto("required".to_string()));

        let chunks = collect(
            provider_for(&server)
                .stream_complete(request)
                .await
                .unwrap(),
        )
        .await;

        let reasoning: String = chunks
            .iter()
            .flat_map(|c| c.choices.iter())
            .filter_map(|c| c.delta.reasoning_content.as_deref())
            .collect();
        assert_eq!(reasoning, "I will check the weather.");
        assert_eq!(
            assembled_tool_calls(&chunks),
            vec![
                (
                    "call_a".to_string(),
                    "get_weather".to_string(),
                    r#"{"city":"Paris"}"#.to_string()
                ),
                (
                    "call_b".to_string(),
                    "get_time".to_string(),
                    "{}".to_string()
                ),
            ]
        );
        assert_eq!(finish_reasons(&chunks), vec!["tool_calls"]);
        assert_eq!(single_trailing_usage(&chunks).total_tokens, 50);

        let sent = received_body(&server).await;
        assert_eq!(sent["tool_choice"], "REQUIRED");
        assert_eq!(
            sent["tools"],
            serde_json::json!([{"type": "function", "function": {
                "name": "get_weather", "description": "Weather", "parameters": {"type": "object"}
            }}])
        );
    }

    #[tokio::test]
    async fn stream_accepts_bare_json_lines() {
        let body = [
            r#"{"type":"content-delta","delta":{"message":{"content":{"text":"1"}}}}"#,
            r#"{"type":"content-delta","delta":{"message":{"content":{"text":" 2"}}}}"#,
            // Last line without a trailing newline.
            r#"{"type":"message-end","delta":{"finish_reason":"STOP_SEQUENCE"}}"#,
        ]
        .join("\n");
        let server = sse_server("/chat", body).await;
        let chunks = collect(
            provider_for(&server)
                .stream_complete(stream_request("command-r"))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(content(&chunks), "1 2");
        assert_eq!(finish_reasons(&chunks), vec!["stop"]);
        assert!(chunks.iter().all(|c| c.usage.is_none()));
    }

    #[tokio::test]
    async fn stream_surfaces_http_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat"))
            .respond_with(ResponseTemplate::new(429).set_body_string("slow down"))
            .mount(&server)
            .await;
        let err = match provider_for(&server)
            .stream_complete(stream_request("command-r"))
            .await
        {
            Ok(_) => panic!("expected an error"),
            Err(e) => e.to_string(),
        };
        assert!(err.contains("429"), "{err}");
        assert!(err.contains("slow down"), "{err}");
    }

    #[test]
    fn stream_malformed_event_is_an_error() {
        let mut parser = CohereStreamParser::new("m".to_string());
        let out = parser.push(b"data: {not json}\n");
        assert_eq!(out.len(), 1);
        assert!(out[0].is_err());
    }

    #[test]
    fn line_buffer_joins_lines_split_across_reads() {
        let mut buffer = Vec::new();
        let first = drain_complete_lines(&mut buffer, b"event: content-delta\r\ndata: {\"a\":");
        assert_eq!(first, vec!["event: content-delta".to_string()]);
        // "é" (0xC3 0xA9) split between reads.
        assert!(drain_complete_lines(&mut buffer, b"\"\xC3").is_empty());
        let second = drain_complete_lines(&mut buffer, b"\xA9\"}\n\npartial");
        assert_eq!(
            second,
            vec!["data: {\"a\":\"é\"}".to_string(), String::new()]
        );
        assert_eq!(buffer, b"partial");
    }

    #[test]
    fn stream_parser_handles_event_split_across_reads() {
        let mut parser = CohereStreamParser::new("m".to_string());
        let frame = "event: content-delta\ndata: {\"type\":\"content-delta\",\"delta\":{\"message\":{\"content\":{\"text\":\"héllo\"}}}}\n\n";
        let (a, b) = frame.as_bytes().split_at(frame.find('é').unwrap() + 1);
        assert!(parser.push(a).is_empty());
        let out = parser.push(b);
        assert_eq!(out.len(), 1);
        let chunk = out.into_iter().next().unwrap().unwrap();
        assert_eq!(chunk.choices[0].delta.content.as_deref(), Some("héllo"));
        assert!(parser.finish().is_empty());
    }

    #[test]
    fn finish_reason_mapping() {
        assert_eq!(map_finish_reason("COMPLETE"), "stop");
        assert_eq!(map_finish_reason("STOP_SEQUENCE"), "stop");
        assert_eq!(map_finish_reason("MAX_TOKENS"), "length");
        assert_eq!(map_finish_reason("TOOL_CALL"), "tool_calls");
        assert_eq!(map_finish_reason("ERROR"), "error");
        assert_eq!(map_finish_reason("TIMEOUT"), "error");
    }

    #[test]
    fn request_uses_v2_messages() {
        let provider = CohereProvider::new("k".to_string()).unwrap();
        let mut request: CompletionRequest = serde_json::from_value(serde_json::json!({
            "model": "command-a",
            "messages": [
                {"role": "system", "content": "Be brief."},
                {"role": "user", "content": "Weather?"},
                {"role": "assistant", "content": null, "reasoning_content": "Look it up.",
                 "tool_calls": [{"id": "c1", "type": "function", "function": {"name": "w", "arguments": "{}"}}]},
                {"role": "tool", "tool_call_id": "c1", "content": "sunny"},
            ],
            "top_p": 0.5,
            "top_k": 10,
            "seed": 7,
            "tools": [
                {"type": "function", "function": {"name": "w", "parameters": {}}},
                {"type": "function", "function": {"name": "x", "parameters": {}}}
            ],
            "tool_choice": {"type": "function", "function": {"name": "w"}}
        }))
        .unwrap();
        let body =
            serde_json::to_value(provider.convert_to_cohere_request(&request, false).unwrap())
                .unwrap();
        assert_eq!(
            body["messages"],
            serde_json::json!([
                {"role": "system", "content": "Be brief."},
                {"role": "user", "content": "Weather?"},
                {"role": "assistant", "tool_plan": "Look it up.",
                 "tool_calls": [{"id": "c1", "type": "function", "function": {"name": "w", "arguments": "{}"}}]},
                {"role": "tool", "tool_call_id": "c1", "content": "sunny"},
            ])
        );
        assert_eq!(body["p"], 0.5);
        assert_eq!(body["k"], 10);
        assert_eq!(body["seed"], 7);
        assert_eq!(body["stream"], false);
        // A specific tool choice offers only that tool, required.
        assert_eq!(body["tool_choice"], "REQUIRED");
        assert_eq!(body["tools"].as_array().unwrap().len(), 1);
        assert_eq!(body["tools"][0]["function"]["name"], "w");

        request.tool_choice = Some(ToolChoice::Auto("none".to_string()));
        let body =
            serde_json::to_value(provider.convert_to_cohere_request(&request, true).unwrap())
                .unwrap();
        assert_eq!(body["tool_choice"], "NONE");
        assert_eq!(body["tools"].as_array().unwrap().len(), 2);

        request.tool_choice = Some(ToolChoice::Auto("auto".to_string()));
        let body =
            serde_json::to_value(provider.convert_to_cohere_request(&request, true).unwrap())
                .unwrap();
        assert!(body.get("tool_choice").is_none());
    }

    #[tokio::test]
    async fn complete_maps_tool_calls_and_usage() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "r1",
                "finish_reason": "TOOL_CALL",
                "message": {
                    "role": "assistant",
                    "content": [{"type": "thinking", "thinking": "Hmm."}],
                    "tool_plan": "Call w.",
                    "tool_calls": [{"id": "c1", "type": "function", "function": {"name": "w", "arguments": "{}"}}]
                },
                "usage": {"billed_units": {"input_tokens": 4, "output_tokens": 3}, "tokens": {"input_tokens": 9, "output_tokens": 3}}
            })))
            .mount(&server)
            .await;
        let response = provider_for(&server)
            .complete(CompletionRequest::new(
                "command-a",
                vec![
                    serde_json::from_value(serde_json::json!({"role": "user", "content": "hi"}))
                        .unwrap(),
                ],
            ))
            .await
            .unwrap();
        let choice = &response.choices[0];
        assert_eq!(choice.finish_reason.as_deref(), Some("tool_calls"));
        assert_eq!(choice.message.tool_calls.as_ref().unwrap()[0].id, "c1");
        assert_eq!(
            choice.message.reasoning_content.as_deref(),
            Some("Hmm.\nCall w.")
        );
        assert!(choice.message.content.is_empty());
        assert_eq!(response.usage.prompt_tokens, 9);
        assert_eq!(response.usage.total_tokens, 12);
        assert_eq!(received_body(&server).await["stream"], false);
    }
}
