//! Google Gemini provider implementation
//!
//! Provides integration with Google Gemini models via the Gemini API.

use async_trait::async_trait;
use chrono::Utc;
use futures::{Stream, StreamExt};
use reqwest_middleware::ClientWithMiddleware;
use serde::{Deserialize, Serialize};
use std::pin::Pin;
use std::time::Instant;
use tracing::{debug, error, warn};

use super::{
    Capability, ChatMessage, ChunkChoice, ChunkDelta, CompletionChoice, CompletionChunk,
    CompletionRequest, CompletionResponse, FunctionCall, HealthStatus, ModelInfo, ModelProvider,
    PricingInfo, ProviderHealth, TokenUsage, ToolCall,
};
use crate::openai_compatible::stream_usage;
use lr_types::{AppError, AppResult};

/// Header carrying the Gemini API key.
///
/// The key is deliberately sent as a header rather than a `?key=` query
/// parameter: reqwest attaches the full request URL to transport errors
/// (timeouts, connection failures), and those messages are surfaced to API
/// clients as 502 bodies, logs and monitor events. Keeping the key out of the
/// URL means no error path can disclose it.
const API_KEY_HEADER: &str = "x-goog-api-key";

/// Format a transport error without its URL (defense in depth: URLs never
/// carry the key any more, but reqwest's `Display` would otherwise echo the
/// full request URL, query string included).
fn transport_error_message(e: reqwest_middleware::Error) -> String {
    match e {
        reqwest_middleware::Error::Reqwest(e) => e.without_url().to_string(),
        reqwest_middleware::Error::Middleware(e) => e.to_string(),
    }
}

/// Google Gemini provider
pub struct GeminiProvider {
    client: ClientWithMiddleware,
    api_key: String,
    base_url: String,
}

#[allow(dead_code)]
impl GeminiProvider {
    /// Creates a new Gemini provider with the given API key
    pub fn new(api_key: String) -> Self {
        Self {
            client: crate::http_client::default_client(),
            api_key,
            base_url: "https://generativelanguage.googleapis.com/v1beta".to_string(),
        }
    }

    /// Creates a new Gemini provider with custom base URL
    pub fn with_base_url(api_key: String, base_url: String) -> Self {
        Self {
            client: crate::http_client::default_client(),
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
        let api_key = super::key_storage::get_provider_key(name)?.ok_or_else(|| {
            AppError::Provider(format!("No API key found for provider '{}'", name))
        })?;
        Ok(Self::new(api_key))
    }

    /// Convert OpenAI messages to Gemini format, returning (system_instruction, contents)
    fn convert_messages_to_gemini(
        &self,
        messages: &[ChatMessage],
    ) -> (Option<GeminiContent>, Vec<GeminiContent>) {
        let mut gemini_contents = Vec::new();
        let mut system_instruction = None;

        for msg in messages {
            match msg.role.as_str() {
                "system" => {
                    // Use Gemini's systemInstruction field
                    system_instruction = Some(GeminiContent {
                        role: "user".to_string(),
                        parts: vec![GeminiPart::Text {
                            text: msg.content.as_text(),
                        }],
                    });
                }
                "tool" => {
                    // Tool response message - convert to user role with FunctionResponse
                    if let Some(_tool_call_id) = &msg.tool_call_id {
                        let tool_name = msg.name.clone().unwrap_or_else(|| "unknown".to_string());
                        let response_data: serde_json::Value = serde_json::from_str(
                            &msg.content.as_text(),
                        )
                        .unwrap_or_else(|_| serde_json::json!({"result": msg.content.as_text()}));

                        gemini_contents.push(GeminiContent {
                            role: "user".to_string(),
                            parts: vec![GeminiPart::FunctionResponse {
                                function_response: GeminiFunctionResponse {
                                    name: tool_name,
                                    response: response_data,
                                },
                            }],
                        });
                    }
                }
                "assistant" => {
                    // Check if this assistant message has tool calls
                    if let Some(tool_calls) = &msg.tool_calls {
                        let mut parts = Vec::new();

                        let text_content = msg.content.as_text();
                        if !text_content.is_empty() {
                            parts.push(GeminiPart::Text { text: text_content });
                        }

                        for tool_call in tool_calls {
                            let args: serde_json::Value =
                                serde_json::from_str(&tool_call.function.arguments)
                                    .unwrap_or(serde_json::json!({}));

                            parts.push(GeminiPart::FunctionCall {
                                function_call: GeminiFunctionCall {
                                    name: tool_call.function.name.clone(),
                                    args,
                                },
                            });
                        }

                        gemini_contents.push(GeminiContent {
                            role: "model".to_string(),
                            parts,
                        });
                    } else {
                        gemini_contents.push(GeminiContent {
                            role: "model".to_string(),
                            parts: vec![GeminiPart::Text {
                                text: msg.content.as_text(),
                            }],
                        });
                    }
                }
                "user" => {
                    gemini_contents.push(GeminiContent {
                        role: "user".to_string(),
                        parts: vec![GeminiPart::Text {
                            text: msg.content.as_text(),
                        }],
                    });
                }
                _ => {
                    gemini_contents.push(GeminiContent {
                        role: "user".to_string(),
                        parts: vec![GeminiPart::Text {
                            text: msg.content.as_text(),
                        }],
                    });
                }
            }
        }

        (system_instruction, gemini_contents)
    }

    /// Get model pricing information
    fn get_model_pricing(&self, model: &str) -> PricingInfo {
        // Pricing as of 2025 (per 1M tokens, converted to per 1K)
        match model {
            m if m.contains("gemini-1.5-pro") => PricingInfo {
                input_cost_per_1k: 0.00125, // $1.25 per 1M tokens
                output_cost_per_1k: 0.005,  // $5.00 per 1M tokens
                reasoning_cost_per_1k: None,
                cache_read_cost_per_1k: None,
                cache_write_cost_per_1k: None,
                currency: "USD".to_string(),
            },
            m if m.contains("gemini-1.5-flash") => PricingInfo {
                input_cost_per_1k: 0.000075, // $0.075 per 1M tokens
                output_cost_per_1k: 0.0003,  // $0.30 per 1M tokens
                reasoning_cost_per_1k: None,
                cache_read_cost_per_1k: None,
                cache_write_cost_per_1k: None,
                currency: "USD".to_string(),
            },
            m if m.contains("gemini-2.0-flash") => PricingInfo {
                input_cost_per_1k: 0.0, // Free during preview
                output_cost_per_1k: 0.0,
                reasoning_cost_per_1k: None,
                cache_read_cost_per_1k: None,
                cache_write_cost_per_1k: None,
                currency: "USD".to_string(),
            },
            _ => PricingInfo {
                // Default pricing for unknown models
                input_cost_per_1k: 0.001,
                output_cost_per_1k: 0.002,
                reasoning_cost_per_1k: None,
                cache_read_cost_per_1k: None,
                cache_write_cost_per_1k: None,
                currency: "USD".to_string(),
            },
        }
    }
}

#[async_trait]
#[allow(dead_code)]
impl ModelProvider for GeminiProvider {
    fn name(&self) -> &str {
        "gemini"
    }

    async fn health_check(&self) -> ProviderHealth {
        let start = Instant::now();

        // Query a single model instead of listing all models.
        // Accept both 200 (exists) and 404 (retired but API up, auth valid).
        // A bad API key returns 401/403, correctly treated as unhealthy.
        let url = format!("{}/models/gemini-2.0-flash", self.base_url);

        match self
            .client
            .get(&url)
            .header(API_KEY_HEADER, &self.api_key)
            .send()
            .await
        {
            Ok(response) => {
                let latency_ms = start.elapsed().as_millis() as u64;
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
                    warn!("Gemini health check failed with status: {}", status);
                    ProviderHealth {
                        status: HealthStatus::Unhealthy,
                        latency_ms: Some(latency_ms),
                        last_checked: Utc::now(),
                        error_message: Some(format!("HTTP {}", status)),
                    }
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
        let url = format!("{}/models", self.base_url);
        debug!("Fetching Gemini models from: {}", url);

        let response = self
            .client
            .get(&url)
            .header(API_KEY_HEADER, &self.api_key)
            .send()
            .await
            .map_err(|e| {
                AppError::Provider(format!(
                    "Failed to connect to Gemini API: {}",
                    transport_error_message(e)
                ))
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
                // Include models that support generateContent or embedContent
                model
                    .supported_generation_methods
                    .contains(&"generateContent".to_string())
                    || model
                        .supported_generation_methods
                        .contains(&"embedContent".to_string())
            })
            .map(|model| {
                let is_embedding = model
                    .supported_generation_methods
                    .contains(&"embedContent".to_string());
                let is_generative = model
                    .supported_generation_methods
                    .contains(&"generateContent".to_string());

                // Extract capabilities based on supported generation methods
                let mut capabilities = if is_embedding && !is_generative {
                    vec![Capability::Embedding]
                } else {
                    vec![
                        Capability::Chat,
                        Capability::Completion,
                        Capability::FunctionCalling,
                    ]
                };

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
                    supports_streaming: is_generative,
                    capabilities,
                    detailed_capabilities: None,
                }
            })
            .collect();

        debug!("Found {} Gemini models", models.len());
        Ok(models)
    }

    async fn get_pricing(&self, model: &str) -> AppResult<PricingInfo> {
        // Try catalog first (embedded OpenRouter data)
        // Normalize model name: "models/gemini-2.0-flash" -> "gemini-2.0-flash"
        let model_id = model.strip_prefix("models/").unwrap_or(model);

        if let Some(catalog_model) = lr_catalog::find_model("google", model_id) {
            tracing::debug!("Using catalog pricing for Gemini model: {}", model);
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
        tracing::debug!("Using fallback pricing for Gemini model: {}", model);
        Ok(self.get_model_pricing(model))
    }

    async fn complete(&self, request: CompletionRequest) -> AppResult<CompletionResponse> {
        let model_name = if request.model.starts_with("models/") {
            request.model.clone()
        } else {
            format!("models/{}", request.model)
        };

        let url = format!("{}/{}:generateContent", self.base_url, model_name);
        debug!("Sending completion request to Gemini: {}", url);

        let (system_instruction, gemini_contents) =
            self.convert_messages_to_gemini(&request.messages);

        // Convert tools from OpenAI format to Gemini format
        let tools = request.tools.as_ref().map(|openai_tools| {
            vec![GeminiTool {
                function_declarations: openai_tools
                    .iter()
                    .map(|t| GeminiFunctionDeclaration {
                        name: t.function.name.clone(),
                        description: t.function.description.clone().unwrap_or_default(),
                        parameters: t.function.parameters.clone(),
                    })
                    .collect(),
            }]
        });

        // Convert tool_choice to Gemini toolConfig
        let tool_config = request.tool_choice.as_ref().map(|tc| {
            let (mode, names) = match tc {
                super::ToolChoice::Auto(s) => match s.as_str() {
                    "none" => ("NONE".to_string(), None),
                    "required" => ("ANY".to_string(), None),
                    _ => ("AUTO".to_string(), None),
                },
                super::ToolChoice::Specific { function, .. } => {
                    ("ANY".to_string(), Some(vec![function.name.clone()]))
                }
            };
            GeminiToolConfig {
                function_calling_config: GeminiFunctionCallingConfig {
                    mode,
                    allowed_function_names: names,
                },
            }
        });

        let gemini_request = GeminiRequest {
            contents: gemini_contents,
            system_instruction,
            generation_config: Some(GeminiGenerationConfig {
                temperature: request.temperature,
                max_output_tokens: request.max_tokens,
                top_p: request.top_p,
                top_k: request.top_k,
                stop_sequences: request.stop.clone(),
                seed: request.seed,
            }),
            tools,
            tool_config,
        };

        let response = self
            .client
            .post(&url)
            .header(API_KEY_HEADER, &self.api_key)
            .json(&gemini_request)
            .send()
            .await
            .map_err(|e| {
                AppError::Provider(format!(
                    "Gemini request failed: {}",
                    transport_error_message(e)
                ))
            })?;

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

        // Extract text content
        let content = candidate
            .content
            .parts
            .iter()
            .filter_map(|p| match p {
                GeminiPart::Text { text } => Some(text.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("");

        // Extract function calls (tool calls)
        let mut tool_calls = Vec::new();
        for (idx, part) in candidate.content.parts.iter().enumerate() {
            if let GeminiPart::FunctionCall { function_call } = part {
                tool_calls.push(ToolCall {
                    id: format!("call_gemini_{}", idx),
                    tool_type: "function".to_string(),
                    function: FunctionCall {
                        name: function_call.name.clone(),
                        arguments: serde_json::to_string(&function_call.args).unwrap_or_default(),
                    },
                });
            }
        }

        // Determine finish reason
        let finish_reason = if !tool_calls.is_empty() {
            "tool_calls"
        } else {
            match candidate.finish_reason.as_deref() {
                Some("STOP") => "stop",
                Some("MAX_TOKENS") => "length",
                Some("SAFETY") => "content_filter",
                _ => "stop",
            }
        };

        let usage = gemini_response.usage_metadata.as_ref();

        let completion_response = CompletionResponse {
            id: format!("chatcmpl-{}", uuid::Uuid::new_v4()),
            object: "chat.completion".to_string(),
            created: Utc::now().timestamp(),
            model: request.model,
            provider: self.name().to_string(),
            choices: vec![CompletionChoice {
                index: 0,
                message: ChatMessage {
                    role: "assistant".to_string(),
                    content: super::ChatMessageContent::Text(content),
                    tool_calls: if tool_calls.is_empty() {
                        None
                    } else {
                        Some(tool_calls)
                    },
                    tool_call_id: None,
                    name: None,
                    reasoning_content: None,
                },
                finish_reason: Some(finish_reason.to_string()),
                logprobs: None, // Gemini does not support logprobs
            }],
            usage: usage
                .map(GeminiUsageMetadata::to_token_usage)
                .unwrap_or(TokenUsage {
                    prompt_tokens: 0,
                    completion_tokens: 0,
                    total_tokens: 0,
                    prompt_tokens_details: None,
                    completion_tokens_details: None,
                }),
            system_fingerprint: None,
            service_tier: None,
            extensions: None,
            routellm_win_rate: None,
            request_usage_entries: None,
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
            "{}/{}:streamGenerateContent?alt=sse",
            self.base_url, model_name
        );
        debug!("Sending streaming completion request to Gemini: {}", url);

        let (system_instruction, gemini_contents) =
            self.convert_messages_to_gemini(&request.messages);

        // Convert tools from OpenAI format to Gemini format
        let tools = request.tools.as_ref().map(|openai_tools| {
            vec![GeminiTool {
                function_declarations: openai_tools
                    .iter()
                    .map(|t| GeminiFunctionDeclaration {
                        name: t.function.name.clone(),
                        description: t.function.description.clone().unwrap_or_default(),
                        parameters: t.function.parameters.clone(),
                    })
                    .collect(),
            }]
        });

        // Convert tool_choice to Gemini toolConfig
        let tool_config = request.tool_choice.as_ref().map(|tc| {
            let (mode, names) = match tc {
                super::ToolChoice::Auto(s) => match s.as_str() {
                    "none" => ("NONE".to_string(), None),
                    "required" => ("ANY".to_string(), None),
                    _ => ("AUTO".to_string(), None),
                },
                super::ToolChoice::Specific { function, .. } => {
                    ("ANY".to_string(), Some(vec![function.name.clone()]))
                }
            };
            GeminiToolConfig {
                function_calling_config: GeminiFunctionCallingConfig {
                    mode,
                    allowed_function_names: names,
                },
            }
        });

        let gemini_request = GeminiRequest {
            contents: gemini_contents,
            system_instruction,
            generation_config: Some(GeminiGenerationConfig {
                temperature: request.temperature,
                max_output_tokens: request.max_tokens,
                top_p: request.top_p,
                top_k: request.top_k,
                stop_sequences: request.stop.clone(),
                seed: request.seed,
            }),
            tools,
            tool_config,
        };

        let response = self
            .client
            .post(&url)
            .header(API_KEY_HEADER, &self.api_key)
            .json(&gemini_request)
            .send()
            .await
            .map_err(|e| {
                AppError::Provider(format!(
                    "Gemini streaming request failed: {}",
                    transport_error_message(e)
                ))
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(AppError::Provider(format!(
                "Gemini streaming API error {}: {}",
                status, error_text
            )));
        }

        let model = request.model.clone();
        let stream = crate::sse_lines::line_batches(response.bytes_stream());

        let converted_stream = stream.flat_map(move |result| {
            let model = model.clone();

            let chunks: Vec<AppResult<CompletionChunk>> = match result {
                Ok(lines) => {
                    let mut parsed_chunks = Vec::new();

                    for line in lines {
                        if line.trim().is_empty() {
                            continue;
                        }

                        if let Some(json_str) = line.strip_prefix("data: ") {
                            if json_str.trim() == "[DONE]" {
                                continue;
                            }

                            match serde_json::from_str::<GeminiResponse>(json_str) {
                                Ok(gemini_chunk) => {
                                    // Every chunk repeats `usageMetadata` with
                                    // running totals; `usage_once_at_end`
                                    // keeps the last one.
                                    let usage = gemini_chunk
                                        .usage_metadata
                                        .as_ref()
                                        .map(GeminiUsageMetadata::to_token_usage);

                                    if gemini_chunk.candidates.is_empty() {
                                        if usage.is_some() {
                                            parsed_chunks.push(Ok(CompletionChunk {
                                                id: format!("chatcmpl-{}", uuid::Uuid::new_v4()),
                                                object: "chat.completion.chunk".to_string(),
                                                created: Utc::now().timestamp(),
                                                model: model.clone(),
                                                choices: vec![],
                                                extensions: None,
                                                usage,
                                                provider: None,
                                            }));
                                        }
                                        continue;
                                    }

                                    let candidate = &gemini_chunk.candidates[0];

                                    let content = candidate
                                        .content
                                        .parts
                                        .iter()
                                        .filter_map(|p| match p {
                                            GeminiPart::Text { text } => Some(text.clone()),
                                            _ => None,
                                        })
                                        .collect::<Vec<_>>()
                                        .join("");

                                    let mut tool_call_deltas = Vec::new();
                                    for (idx, part) in candidate.content.parts.iter().enumerate() {
                                        if let GeminiPart::FunctionCall { function_call } = part {
                                            tool_call_deltas.push(super::ToolCallDelta {
                                                index: idx as u32,
                                                id: Some(format!("call_gemini_{}", idx)),
                                                tool_type: Some("function".to_string()),
                                                function: Some(super::FunctionCallDelta {
                                                    name: Some(function_call.name.clone()),
                                                    arguments: Some(
                                                        serde_json::to_string(&function_call.args)
                                                            .unwrap_or_default(),
                                                    ),
                                                }),
                                            });
                                        }
                                    }

                                    let has_tool_calls = !tool_call_deltas.is_empty();
                                    let finish_reason = if has_tool_calls
                                        && candidate.finish_reason.is_some()
                                    {
                                        Some("tool_calls".to_string())
                                    } else {
                                        match candidate.finish_reason.as_deref() {
                                            Some("STOP") => Some("stop".to_string()),
                                            Some("MAX_TOKENS") => Some("length".to_string()),
                                            Some("SAFETY") => Some("content_filter".to_string()),
                                            _ => None,
                                        }
                                    };

                                    parsed_chunks.push(Ok(CompletionChunk {
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
                                                tool_calls: if tool_call_deltas.is_empty() {
                                                    None
                                                } else {
                                                    Some(tool_call_deltas)
                                                },
                                                reasoning_content: None,
                                            },
                                            finish_reason,
                                        }],
                                        extensions: None,
                                        usage,
                                        provider: None,
                                    }));
                                }
                                Err(e) => {
                                    error!("Failed to parse Gemini stream chunk: {}", e);
                                }
                            }
                        }
                    }

                    parsed_chunks
                }
                Err(e) => vec![Err(AppError::Provider(
                    crate::http_client::format_stream_error(&e),
                ))],
            };

            futures::stream::iter(chunks)
        });

        Ok(stream_usage::usage_once_at_end(converted_stream))
    }

    fn supports_feature(&self, feature: &str) -> bool {
        matches!(
            feature,
            "thinking_level" | "web_grounding" | "code_execution"
        )
    }

    fn get_feature_adapter(
        &self,
        feature: &str,
    ) -> Option<Box<dyn crate::features::FeatureAdapter>> {
        match feature {
            "thinking_level" => Some(Box::new(
                crate::features::gemini_thinking::GeminiThinkingAdapter,
            )),
            "json_mode" => Some(Box::new(crate::features::json_mode::JsonModeAdapter)),
            _ => None,
        }
    }

    fn supports_embeddings(&self) -> bool {
        true
    }

    fn supports_image_generation(&self) -> bool {
        true
    }

    fn get_feature_support(&self, instance_name: &str) -> super::ProviderFeatureSupport {
        let mut support = super::default_feature_support(self, instance_name);

        for f in &mut support.model_features {
            match f.name.as_str() {
                "Thinking Level" => {
                    f.support = super::SupportLevel::Partial;
                    f.notes = Some("Only Gemini 2.0 Flash Thinking and Gemini 3 models support thinking level (low/medium/high); other Gemini models do not".into());
                }
                "Function Calling" => {
                    f.support = super::SupportLevel::Supported;
                    f.notes = Some("Gemini Pro and Flash models support function calling".into());
                }
                "Vision" => {
                    f.support = super::SupportLevel::Supported;
                    f.notes =
                        Some("Gemini Pro and Flash models can process images and video".into());
                }
                "JSON Mode" => {
                    f.notes = Some("Gemini supports JSON output via response MIME type".into());
                }
                "N Completions" => {
                    f.support = super::SupportLevel::Supported;
                    f.notes = Some(
                        "Gemini supports candidateCount parameter for multiple completions".into(),
                    );
                }
                "Reasoning Effort" => {
                    f.support = super::SupportLevel::Translated;
                    f.notes = Some("Mapped to Gemini's thinking_level config; requires Gemini 2.0+ thinking models".into());
                }
                _ => {}
            }
        }

        // Gemini-specific endpoint notes
        for e in &mut support.endpoints {
            if e.name == "Embeddings" {
                e.notes = Some("Gemini text-embedding models; single-input only (no batch)".into());
            }
        }

        support
    }

    async fn embed(&self, request: super::EmbeddingRequest) -> AppResult<super::EmbeddingResponse> {
        // Gemini only supports single text input for embeddings
        let text = match request.input {
            super::EmbeddingInput::Single(text) => text,
            super::EmbeddingInput::Multiple(_texts) => {
                // For multiple inputs, we need to make separate requests
                // For now, return error - we can implement batch later
                return Err(AppError::Provider(
                    "Gemini embeddings currently only support single text input. Use multiple requests for batch processing.".to_string(),
                ));
            }
            super::EmbeddingInput::Tokens(_) => {
                return Err(AppError::Provider(
                    "Gemini embeddings do not support pre-tokenized input".to_string(),
                ));
            }
        };

        // Gemini embeddings use the embedContent endpoint
        let url = format!("{}/models/{}:embedContent", self.base_url, request.model);

        let gemini_request = serde_json::json!({
            "content": {
                "parts": [
                    {
                        "text": text
                    }
                ]
            }
        });

        let response = self
            .client
            .post(&url)
            .header("Content-Type", "application/json")
            .header(API_KEY_HEADER, &self.api_key)
            .json(&gemini_request)
            .send()
            .await
            .map_err(|e| {
                AppError::Provider(format!(
                    "Gemini request failed: {}",
                    transport_error_message(e)
                ))
            })?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());

            return Err(AppError::Provider(format!(
                "Gemini API error ({}): {}",
                status, error_text
            )));
        }

        let gemini_response: serde_json::Value = response
            .json()
            .await
            .map_err(|e| AppError::Provider(format!("Failed to parse response: {}", e)))?;

        // Extract embedding values from Gemini response
        let embedding_values = gemini_response["embedding"]["values"]
            .as_array()
            .ok_or_else(|| AppError::Provider("No embedding values in response".to_string()))?
            .iter()
            .map(|v| v.as_f64().unwrap_or(0.0) as f32)
            .collect::<Vec<f32>>();

        // Gemini doesn't return token usage for embeddings, estimate it
        let estimated_tokens = (text.len() / 4).max(1) as u32;

        // Convert to our generic format
        Ok(super::EmbeddingResponse {
            object: "list".to_string(),
            data: vec![super::Embedding {
                object: "embedding".to_string(),
                embedding: Some(embedding_values),
                index: 0,
            }],
            model: request.model,
            usage: super::EmbeddingUsage {
                prompt_tokens: estimated_tokens,
                total_tokens: estimated_tokens,
            },
        })
    }

    async fn generate_image(
        &self,
        request: super::ImageGenerationRequest,
    ) -> AppResult<super::ImageGenerationResponse> {
        // Gemini uses Imagen models for image generation
        // API endpoint: models/{model}:predict
        let model = if request.model.starts_with("models/") {
            request.model.clone()
        } else {
            format!("models/{}", request.model)
        };

        let url = format!("{}/{}:predict", self.base_url, model);

        // Imagen request format
        let instances = vec![serde_json::json!({
            "prompt": request.prompt
        })];

        // Build parameters
        let mut parameters = serde_json::json!({
            "sampleCount": request.n.unwrap_or(1)
        });

        if let Some(size) = &request.size {
            // Parse size to get aspect ratio
            if let Some((w, h)) = size.split_once('x') {
                if let (Ok(width), Ok(height)) = (w.parse::<u32>(), h.parse::<u32>()) {
                    // Imagen uses aspectRatio instead of exact dimensions
                    let ratio = if width > height {
                        "16:9"
                    } else if height > width {
                        "9:16"
                    } else {
                        "1:1"
                    };
                    parameters["aspectRatio"] = serde_json::json!(ratio);
                }
            }
        }

        // Response format
        if request.response_format.as_deref() == Some("b64_json") {
            parameters["outputOptions"] = serde_json::json!({
                "mimeType": "image/png"
            });
        }

        let gemini_request = serde_json::json!({
            "instances": instances,
            "parameters": parameters
        });

        debug!(
            "Gemini image generation request to {}: {:?}",
            url, gemini_request
        );

        let response = self
            .client
            .post(&url)
            .header("Content-Type", "application/json")
            .header(API_KEY_HEADER, &self.api_key)
            .json(&gemini_request)
            .send()
            .await
            .map_err(|e| {
                AppError::Provider(format!("Request failed: {}", transport_error_message(e)))
            })?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            error!(
                "Gemini image generation failed: {} - {}",
                status, error_text
            );
            return Err(AppError::Provider(format!(
                "API error ({}): {}",
                status, error_text
            )));
        }

        let api_response: serde_json::Value = response
            .json()
            .await
            .map_err(|e| AppError::Provider(format!("Failed to parse response: {}", e)))?;

        debug!("Gemini image response: {:?}", api_response);

        // Parse Imagen response format
        let empty_vec = vec![];
        let predictions = api_response["predictions"].as_array().unwrap_or(&empty_vec);

        let data: Vec<super::GeneratedImage> = predictions
            .iter()
            .map(|pred| {
                // Imagen returns base64 encoded images in "bytesBase64Encoded" field
                let b64 = pred["bytesBase64Encoded"]
                    .as_str()
                    .or_else(|| pred["image"]["bytesBase64Encoded"].as_str())
                    .map(|s| s.to_string());

                super::GeneratedImage {
                    url: None,
                    b64_json: b64,
                    revised_prompt: None,
                }
            })
            .collect();

        Ok(super::ImageGenerationResponse {
            created: chrono::Utc::now().timestamp(),
            data,
        })
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
    #[serde(rename = "systemInstruction", skip_serializing_if = "Option::is_none")]
    system_instruction: Option<GeminiContent>,
    #[serde(rename = "generationConfig", skip_serializing_if = "Option::is_none")]
    generation_config: Option<GeminiGenerationConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<GeminiTool>>,
    #[serde(rename = "toolConfig", skip_serializing_if = "Option::is_none")]
    tool_config: Option<GeminiToolConfig>,
}

/// Gemini tool configuration for controlling function calling behavior
#[derive(Debug, Serialize, Deserialize)]
struct GeminiToolConfig {
    #[serde(rename = "functionCallingConfig")]
    function_calling_config: GeminiFunctionCallingConfig,
}

/// Gemini function calling mode configuration
#[derive(Debug, Serialize, Deserialize)]
struct GeminiFunctionCallingConfig {
    mode: String,
    #[serde(
        rename = "allowedFunctionNames",
        skip_serializing_if = "Option::is_none"
    )]
    allowed_function_names: Option<Vec<String>>,
}

/// Gemini tool definition
#[derive(Debug, Serialize, Deserialize)]
struct GeminiTool {
    function_declarations: Vec<GeminiFunctionDeclaration>,
}

/// Gemini function declaration
#[derive(Debug, Serialize, Deserialize)]
struct GeminiFunctionDeclaration {
    name: String,
    description: String,
    parameters: serde_json::Value,
}

#[derive(Debug, Serialize, Deserialize)]
struct GeminiContent {
    role: String,
    parts: Vec<GeminiPart>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
enum GeminiPart {
    Text {
        text: String,
    },
    FunctionCall {
        #[serde(rename = "functionCall")]
        function_call: GeminiFunctionCall,
    },
    FunctionResponse {
        #[serde(rename = "functionResponse")]
        function_response: GeminiFunctionResponse,
    },
}

/// Gemini function call (from model)
#[derive(Debug, Serialize, Deserialize)]
struct GeminiFunctionCall {
    name: String,
    args: serde_json::Value,
}

/// Gemini function response (from user)
#[derive(Debug, Serialize, Deserialize)]
struct GeminiFunctionResponse {
    name: String,
    response: serde_json::Value,
}

#[derive(Debug, Serialize, Deserialize)]
struct GeminiGenerationConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(rename = "maxOutputTokens", skip_serializing_if = "Option::is_none")]
    max_output_tokens: Option<u32>,
    #[serde(rename = "topP", skip_serializing_if = "Option::is_none")]
    top_p: Option<f32>,
    #[serde(rename = "topK", skip_serializing_if = "Option::is_none")]
    top_k: Option<u32>,
    #[serde(rename = "stopSequences", skip_serializing_if = "Option::is_none")]
    stop_sequences: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    seed: Option<i64>,
}

#[derive(Debug, Serialize, Deserialize)]
struct GeminiResponse {
    /// Absent on a trailing usage-only stream chunk.
    #[serde(default)]
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
    /// Includes `cachedContentTokenCount`.
    #[serde(rename = "promptTokenCount", default)]
    prompt_token_count: u32,
    /// Visible output only; thinking is counted in `thoughtsTokenCount`.
    #[serde(rename = "candidatesTokenCount", default)]
    candidates_token_count: u32,
    #[serde(rename = "totalTokenCount", default)]
    total_token_count: u32,
    #[serde(
        rename = "thoughtsTokenCount",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    thoughts_token_count: Option<u32>,
    #[serde(
        rename = "cachedContentTokenCount",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    cached_content_token_count: Option<u32>,
}

impl GeminiUsageMetadata {
    /// Thinking tokens are billed as output, so `completion_tokens` is
    /// candidates + thoughts, with thoughts broken out as
    /// `reasoning_tokens`. Cached prompt tokens are part of
    /// `prompt_tokens`, broken out as `cached_tokens`.
    fn to_token_usage(&self) -> TokenUsage {
        let completion_tokens = self
            .candidates_token_count
            .saturating_add(self.thoughts_token_count.unwrap_or(0));
        TokenUsage {
            prompt_tokens: self.prompt_token_count,
            completion_tokens,
            total_tokens: if self.total_token_count > 0 {
                self.total_token_count
            } else {
                self.prompt_token_count.saturating_add(completion_tokens)
            },
            prompt_tokens_details: self.cached_content_token_count.map(|cached| {
                super::PromptTokensDetails {
                    cached_tokens: Some(cached),
                    cache_creation_tokens: None,
                    cache_read_tokens: None,
                }
            }),
            completion_tokens_details: self.thoughts_token_count.map(|thoughts| {
                super::CompletionTokensDetails {
                    reasoning_tokens: Some(thoughts),
                    thinking_tokens: None,
                    audio_tokens: None,
                }
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Security regression: the API key must travel in a header, never in the
    /// URL, and transport errors must not echo the URL. Previously the key was
    /// a `?key=` query parameter and reqwest's error text (which includes the
    /// full URL) was returned to API clients in 502 bodies.
    #[tokio::test]
    async fn api_key_never_appears_in_url_or_transport_errors() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        const KEY: &str = "AIzaSyTESTKEY-secret-0123456789";

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (seen_tx, seen_rx) = tokio::sync::oneshot::channel::<String>();
        tokio::spawn(async move {
            // First connection: read the request head, answer 500, report what we saw.
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = Vec::new();
            let mut tmp = [0u8; 1024];
            loop {
                let n = sock.read(&mut tmp).await.unwrap();
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&tmp[..n]);
                if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            let _ = sock
                .write_all(b"HTTP/1.1 500 Internal Server Error\r\ncontent-length: 4\r\nconnection: close\r\n\r\nboom")
                .await;
            let _ = seen_tx.send(String::from_utf8_lossy(&buf).to_string());
            // Second connection: drop immediately so reqwest reports a transport error.
            let (sock, _) = listener.accept().await.unwrap();
            drop(sock);
        });

        let provider =
            GeminiProvider::with_base_url(KEY.to_string(), format!("http://{}/v1beta", addr));

        // 1. The request carries the key in a header and not in the URL.
        let err = provider.list_models().await.unwrap_err().to_string();
        let seen = seen_rx.await.unwrap();
        let request_line = seen.lines().next().unwrap_or_default().to_string();
        assert!(
            request_line.starts_with("GET /v1beta/models "),
            "unexpected request line: {request_line}"
        );
        assert!(
            !request_line.contains(KEY),
            "key leaked into URL: {request_line}"
        );
        assert!(
            seen.to_ascii_lowercase()
                .contains(&format!("{}: {}", API_KEY_HEADER, KEY).to_ascii_lowercase()),
            "key header missing from request:\n{seen}"
        );
        assert!(!err.contains(KEY), "key leaked into HTTP error: {err}");

        // 2. A transport-level failure is reported without the URL (and so
        //    without anything that was ever in the URL).
        let err = provider.list_models().await.unwrap_err().to_string();
        assert!(!err.contains(KEY), "key leaked into transport error: {err}");
        assert!(
            !err.contains("for url"),
            "transport error still echoes the URL: {err}"
        );
    }
    use crate::{ChatMessageContent, FunctionCall, ToolCall};

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
                content: ChatMessageContent::Text("You are helpful".to_string()),
                tool_calls: None,
                tool_call_id: None,
                name: None,
                reasoning_content: None,
            },
            ChatMessage {
                role: "user".to_string(),
                content: ChatMessageContent::Text("Hello".to_string()),
                tool_calls: None,
                tool_call_id: None,
                name: None,
                reasoning_content: None,
            },
            ChatMessage {
                role: "assistant".to_string(),
                content: ChatMessageContent::Text("Hi there!".to_string()),
                tool_calls: None,
                tool_call_id: None,
                name: None,
                reasoning_content: None,
            },
        ];

        let (system_instruction, gemini_contents) = provider.convert_messages_to_gemini(&messages);

        // System message should be extracted as system_instruction
        assert!(system_instruction.is_some());
        match &system_instruction.unwrap().parts[0] {
            GeminiPart::Text { text } => {
                assert!(text.contains("You are helpful"));
            }
            _ => panic!("Expected Text part in system_instruction"),
        }
        assert_eq!(gemini_contents.len(), 2);
        assert_eq!(gemini_contents[0].role, "user");
        match &gemini_contents[0].parts[0] {
            GeminiPart::Text { text } => {
                assert_eq!(text, "Hello");
            }
            _ => panic!("Expected Text part"),
        }
        assert_eq!(gemini_contents[1].role, "model");
    }

    #[test]
    fn test_convert_messages_with_tool_calls() {
        let provider = GeminiProvider::new("test-key".to_string());
        let messages = vec![
            ChatMessage {
                role: "user".to_string(),
                content: ChatMessageContent::Text("What's the weather?".to_string()),
                tool_calls: None,
                tool_call_id: None,
                name: None,
                reasoning_content: None,
            },
            ChatMessage {
                role: "assistant".to_string(),
                content: ChatMessageContent::Text("".to_string()),
                tool_calls: Some(vec![ToolCall {
                    id: "call_123".to_string(),
                    tool_type: "function".to_string(),
                    function: FunctionCall {
                        name: "get_weather".to_string(),
                        arguments: r#"{"location":"San Francisco"}"#.to_string(),
                    },
                }]),
                tool_call_id: None,
                name: None,
                reasoning_content: None,
            },
        ];

        let (_system_instruction, gemini_contents) = provider.convert_messages_to_gemini(&messages);

        assert_eq!(gemini_contents.len(), 2);
        assert_eq!(gemini_contents[0].role, "user");
        assert_eq!(gemini_contents[1].role, "model");

        // Check that the assistant message has a FunctionCall part
        assert_eq!(gemini_contents[1].parts.len(), 1);
        match &gemini_contents[1].parts[0] {
            GeminiPart::FunctionCall { function_call } => {
                assert_eq!(function_call.name, "get_weather");
                assert_eq!(
                    function_call.args,
                    serde_json::json!({"location": "San Francisco"})
                );
            }
            _ => panic!("Expected FunctionCall part"),
        }
    }

    #[test]
    fn test_convert_messages_with_tool_response() {
        let provider = GeminiProvider::new("test-key".to_string());
        let messages = vec![
            ChatMessage {
                role: "user".to_string(),
                content: ChatMessageContent::Text("What's the weather?".to_string()),
                tool_calls: None,
                tool_call_id: None,
                name: None,
                reasoning_content: None,
            },
            ChatMessage {
                role: "assistant".to_string(),
                content: ChatMessageContent::Text("".to_string()),
                tool_calls: Some(vec![ToolCall {
                    id: "call_123".to_string(),
                    tool_type: "function".to_string(),
                    function: FunctionCall {
                        name: "get_weather".to_string(),
                        arguments: r#"{"location":"San Francisco"}"#.to_string(),
                    },
                }]),
                tool_call_id: None,
                name: None,
                reasoning_content: None,
            },
            ChatMessage {
                role: "tool".to_string(),
                content: ChatMessageContent::Text(
                    r#"{"temperature":72,"conditions":"sunny"}"#.to_string(),
                ),
                tool_calls: None,
                tool_call_id: Some("call_123".to_string()),
                name: Some("get_weather".to_string()),
                reasoning_content: None,
            },
        ];

        let (_system_instruction, gemini_contents) = provider.convert_messages_to_gemini(&messages);

        assert_eq!(gemini_contents.len(), 3);

        // Check that the tool message was converted to user role with FunctionResponse
        assert_eq!(gemini_contents[2].role, "user");
        assert_eq!(gemini_contents[2].parts.len(), 1);
        match &gemini_contents[2].parts[0] {
            GeminiPart::FunctionResponse { function_response } => {
                assert_eq!(function_response.name, "get_weather");
                assert_eq!(
                    function_response.response,
                    serde_json::json!({"temperature": 72, "conditions": "sunny"})
                );
            }
            _ => panic!("Expected FunctionResponse part"),
        }
    }

    #[test]
    fn test_parse_response_with_function_call() {
        use serde_json::json;

        // Create a mock Gemini response with a function call
        let gemini_response = GeminiResponse {
            candidates: vec![GeminiCandidate {
                content: GeminiContent {
                    role: "model".to_string(),
                    parts: vec![GeminiPart::FunctionCall {
                        function_call: GeminiFunctionCall {
                            name: "get_weather".to_string(),
                            args: json!({"location": "San Francisco", "unit": "celsius"}),
                        },
                    }],
                },
                finish_reason: Some("STOP".to_string()),
            }],
            usage_metadata: Some(GeminiUsageMetadata {
                prompt_token_count: 10,
                candidates_token_count: 5,
                total_token_count: 15,
                thoughts_token_count: None,
                cached_content_token_count: None,
            }),
        };

        // Simulate parsing logic from complete() method
        let candidate = &gemini_response.candidates[0];

        // Extract text content
        let content = candidate
            .content
            .parts
            .iter()
            .filter_map(|p| match p {
                GeminiPart::Text { text } => Some(text.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("");

        // Extract function calls (tool calls)
        let mut tool_calls = Vec::new();
        for (idx, part) in candidate.content.parts.iter().enumerate() {
            if let GeminiPart::FunctionCall { function_call } = part {
                tool_calls.push(ToolCall {
                    id: format!("call_gemini_{}", idx),
                    tool_type: "function".to_string(),
                    function: FunctionCall {
                        name: function_call.name.clone(),
                        arguments: serde_json::to_string(&function_call.args).unwrap_or_default(),
                    },
                });
            }
        }

        // Verify
        assert_eq!(content, ""); // No text content
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0].function.name, "get_weather");
        assert_eq!(
            tool_calls[0].function.arguments,
            r#"{"location":"San Francisco","unit":"celsius"}"#
        );
    }

    #[test]
    fn test_parse_response_with_text_and_function_call() {
        use serde_json::json;

        // Create a mock Gemini response with both text and function call
        let gemini_response = GeminiResponse {
            candidates: vec![GeminiCandidate {
                content: GeminiContent {
                    role: "model".to_string(),
                    parts: vec![
                        GeminiPart::Text {
                            text: "Let me check the weather for you.".to_string(),
                        },
                        GeminiPart::FunctionCall {
                            function_call: GeminiFunctionCall {
                                name: "get_weather".to_string(),
                                args: json!({"location": "San Francisco"}),
                            },
                        },
                    ],
                },
                finish_reason: Some("STOP".to_string()),
            }],
            usage_metadata: None,
        };

        let candidate = &gemini_response.candidates[0];

        // Extract text content
        let content = candidate
            .content
            .parts
            .iter()
            .filter_map(|p| match p {
                GeminiPart::Text { text } => Some(text.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("");

        // Extract function calls
        let mut tool_calls = Vec::new();
        for (idx, part) in candidate.content.parts.iter().enumerate() {
            if let GeminiPart::FunctionCall { function_call } = part {
                tool_calls.push(ToolCall {
                    id: format!("call_gemini_{}", idx),
                    tool_type: "function".to_string(),
                    function: FunctionCall {
                        name: function_call.name.clone(),
                        arguments: serde_json::to_string(&function_call.args).unwrap_or_default(),
                    },
                });
            }
        }

        // Verify both text and tool calls are extracted
        assert_eq!(content, "Let me check the weather for you.");
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0].function.name, "get_weather");
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
                content: ChatMessageContent::Text("Say hello in one word".to_string()),
                tool_calls: None,
                tool_call_id: None,
                name: None,
                reasoning_content: None,
            }],
            temperature: Some(0.7),
            max_tokens: Some(10),
            stream: false,
            top_p: None,
            frequency_penalty: None,
            presence_penalty: None,
            stop: None,
            top_k: None,
            seed: None,
            repetition_penalty: None,
            extensions: None,
            logprobs: None,
            top_logprobs: None,
            response_format: None,
            tool_choice: None,
            tools: None,
            n: None,
            logit_bias: None,
            parallel_tool_calls: None,
            service_tier: None,
            store: None,
            metadata: None,
            modalities: None,
            audio: None,
            prediction: None,
            reasoning_effort: None,
            pre_computed_routing: None,
        };

        let response = provider.complete(request).await.unwrap();
        assert_eq!(response.choices.len(), 1);
        assert!(!response.choices[0].message.content.as_text().is_empty());
    }

    /// `streamGenerateContent?alt=sse` repeats `usageMetadata` on every
    /// chunk with running totals; only the final numbers are reported, and
    /// thinking tokens count as output.
    #[tokio::test]
    async fn stream_reports_last_usage_metadata() {
        use crate::openai_compatible::stream_usage::test_support::*;

        let frames = [
            r#"{"candidates":[{"content":{"parts":[{"text":"Hello"}],"role":"model"},"index":0}],"usageMetadata":{"promptTokenCount":20,"totalTokenCount":20,"cachedContentTokenCount":8},"modelVersion":"gemini-2.5-flash"}"#,
            r#"{"candidates":[{"content":{"parts":[{"text":" world"}],"role":"model"},"index":0}],"usageMetadata":{"promptTokenCount":20,"candidatesTokenCount":2,"totalTokenCount":52,"thoughtsTokenCount":30,"cachedContentTokenCount":8},"modelVersion":"gemini-2.5-flash"}"#,
            r#"{"candidates":[{"content":{"parts":[{"text":"!"}],"role":"model"},"finishReason":"STOP","index":0}],"usageMetadata":{"promptTokenCount":20,"candidatesTokenCount":3,"totalTokenCount":53,"thoughtsTokenCount":30,"cachedContentTokenCount":8,"promptTokensDetails":[{"modality":"TEXT","tokenCount":20}]},"modelVersion":"gemini-2.5-flash"}"#,
        ];
        let body = frames
            .iter()
            .map(|f| format!("data: {f}\r\n\r\n"))
            .collect::<String>();
        let server = sse_server("/models/gemini-2.5-flash:streamGenerateContent", body).await;
        let provider = GeminiProvider::with_base_url("k".to_string(), server.uri());
        let stream = provider
            .stream_complete(stream_request("gemini-2.5-flash"))
            .await
            .unwrap();
        let chunks = collect(stream).await;

        assert_eq!(chunks.len(), 4, "3 content chunks + usage");
        assert_eq!(content(&chunks), "Hello world!");
        assert_eq!(finish_reasons(&chunks), vec!["stop"]);
        let usage = single_trailing_usage(&chunks);
        assert_eq!(
            (
                usage.prompt_tokens,
                usage.completion_tokens,
                usage.total_tokens
            ),
            (20, 33, 53)
        );
        assert_eq!(usage.prompt_tokens_details.unwrap().cached_tokens, Some(8));
        assert_eq!(
            usage.completion_tokens_details.unwrap().reasoning_tokens,
            Some(30)
        );
    }

    /// A trailing chunk with `usageMetadata` but no candidates still
    /// reports usage instead of being dropped.
    #[tokio::test]
    async fn stream_reports_usage_only_chunk() {
        use crate::openai_compatible::stream_usage::test_support::*;

        let body = [
            r#"{"candidates":[{"content":{"parts":[{"text":"Hi"}],"role":"model"},"finishReason":"STOP","index":0}]}"#,
            r#"{"usageMetadata":{"promptTokenCount":7,"candidatesTokenCount":1,"totalTokenCount":8}}"#,
        ]
        .iter()
        .map(|f| format!("data: {f}\n\n"))
        .collect::<String>();
        let server = sse_server("/models/gemini-2.0-flash:streamGenerateContent", body).await;
        let provider = GeminiProvider::with_base_url("k".to_string(), server.uri());
        let stream = provider
            .stream_complete(stream_request("gemini-2.0-flash"))
            .await
            .unwrap();
        let chunks = collect(stream).await;

        assert_eq!(chunks.len(), 2);
        assert_eq!(content(&chunks), "Hi");
        let usage = single_trailing_usage(&chunks);
        assert_eq!(
            (
                usage.prompt_tokens,
                usage.completion_tokens,
                usage.total_tokens
            ),
            (7, 1, 8)
        );
        assert!(usage.prompt_tokens_details.is_none());
        assert!(usage.completion_tokens_details.is_none());
    }
}
