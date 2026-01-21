//! Model provider implementations
//!
//! Abstractions and implementations for various AI model providers.

#![allow(dead_code)]

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use futures::Stream;
use serde::{Deserialize, Serialize};
use std::pin::Pin;
use utoipa::ToSchema;

use crate::utils::errors::{AppError, AppResult};

pub mod anthropic;
pub mod cerebras;
pub mod cohere;
pub mod deepinfra;
pub mod factory;
pub mod features;
pub mod gemini;
pub mod groq;
pub mod health;
pub mod key_storage;
pub mod lmstudio;
pub mod mistral;
pub mod oauth;
pub mod ollama;
pub mod openai;
pub mod openai_compatible;
pub mod openrouter;
pub mod perplexity;
pub mod registry;
pub mod togetherai;
pub mod xai;

/// Common provider trait for all AI model providers
#[async_trait]
pub trait ModelProvider: Send + Sync {
    /// Returns the name of the provider (e.g., "ollama", "openai")
    fn name(&self) -> &str;

    /// Performs a health check on the provider
    async fn health_check(&self) -> ProviderHealth;

    /// Lists all available models from this provider
    async fn list_models(&self) -> AppResult<Vec<ModelInfo>>;

    /// Gets pricing information for a specific model
    async fn get_pricing(&self, model: &str) -> AppResult<PricingInfo>;

    /// Sends a chat completion request
    async fn complete(&self, request: CompletionRequest) -> AppResult<CompletionResponse>;

    /// Sends a streaming chat completion request
    async fn stream_complete(
        &self,
        request: CompletionRequest,
    ) -> AppResult<Pin<Box<dyn Stream<Item = AppResult<CompletionChunk>> + Send>>>;

    /// Generate embeddings for text
    ///
    /// Used by: POST /v1/embeddings endpoint
    ///
    /// Default implementation returns an error indicating embeddings are not supported.
    /// Providers that support embeddings should override this method.
    #[allow(dead_code)]
    async fn embed(&self, _request: EmbeddingRequest) -> AppResult<EmbeddingResponse> {
        Err(AppError::Provider(format!(
            "Provider '{}' does not support embeddings",
            self.name()
        )))
    }

    /// Check if this provider supports a specific feature
    ///
    /// Features include things like:
    /// - "extended_thinking" (Anthropic Claude)
    /// - "reasoning_tokens" (OpenAI o1 series)
    /// - "structured_outputs" (OpenAI, Anthropic, Gemini)
    /// - "prompt_caching" (Anthropic, OpenRouter)
    /// - "thinking_level" (Google Gemini 3)
    ///
    /// Default implementation returns false for all features.
    fn supports_feature(&self, _feature: &str) -> bool {
        false
    }

    /// Get a feature adapter for a specific feature
    ///
    /// Returns None if the feature is not supported.
    /// Default implementation returns None for all features.
    fn get_feature_adapter(
        &self,
        _feature: &str,
    ) -> Option<Box<dyn crate::providers::features::FeatureAdapter>> {
        None
    }
}

/// Information about a model
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ModelInfo {
    /// Model ID (e.g., "llama3.3", "gpt-4")
    pub id: String,
    /// Human-readable name
    pub name: String,
    /// Provider name
    pub provider: String,
    /// Number of parameters (if known)
    pub parameter_count: Option<u64>,
    /// Context window size in tokens
    pub context_window: u32,
    /// Whether the model supports streaming
    pub supports_streaming: bool,
    /// Model capabilities (basic)
    pub capabilities: Vec<Capability>,
    /// Detailed capability information (Phase 1)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detailed_capabilities: Option<ModelCapabilities>,
}

impl ModelInfo {
    /// Enrich this model with catalog metadata (pricing, capabilities, etc.)
    ///
    /// This is an optional enrichment step that uses the build-time embedded
    /// OpenRouter catalog to enhance model information with accurate pricing
    /// and capability data.
    ///
    /// # Arguments
    /// * `provider_type` - Provider identifier for matching (e.g., "openai", "anthropic")
    ///
    /// # Returns
    /// Self with potentially updated context_window and capabilities
    pub fn enrich_with_catalog(mut self, provider_type: &str) -> Self {
        use crate::catalog;

        if let Some(catalog_model) = catalog::find_model(provider_type, &self.id) {
            tracing::debug!(
                "Enriching model '{}' from catalog (provider: {})",
                self.id,
                provider_type
            );

            // Update context window if catalog has better info
            if catalog_model.context_length > self.context_window {
                tracing::debug!(
                    "Updating context window for '{}': {} -> {}",
                    self.id,
                    self.context_window,
                    catalog_model.context_length
                );
                self.context_window = catalog_model.context_length;
            }

            // Add vision capability if multimodal
            if catalog_model.modality == catalog::Modality::Multimodal
                && !self.capabilities.contains(&Capability::Vision)
            {
                self.capabilities.push(Capability::Vision);
            }
        } else {
            tracing::debug!(
                "Model '{}' not found in catalog (provider: {})",
                self.id,
                provider_type
            );
        }

        self
    }

    /// Enrich this model with catalog metadata using model name only
    ///
    /// This variant is designed for multi-provider systems (Ollama, LMStudio,
    /// DeepInfra, TogetherAI, OpenRouter, OpenAI-compatible) where the model
    /// may come from various providers and we want to search by model name only.
    ///
    /// # Returns
    /// Self with potentially updated context_window and capabilities
    pub fn enrich_with_catalog_by_name(mut self) -> Self {
        use crate::catalog;

        if let Some(catalog_model) = catalog::find_model_by_name(&self.id) {
            tracing::debug!(
                "Enriching model '{}' from catalog (provider-agnostic search)",
                self.id
            );

            // Update context window if catalog has better info
            if catalog_model.context_length > self.context_window {
                tracing::debug!(
                    "Updating context window for '{}': {} -> {}",
                    self.id,
                    self.context_window,
                    catalog_model.context_length
                );
                self.context_window = catalog_model.context_length;
            }

            // Add vision capability if multimodal
            if catalog_model.modality == catalog::Modality::Multimodal
                && !self.capabilities.contains(&Capability::Vision)
            {
                self.capabilities.push(Capability::Vision);
            }
        } else {
            tracing::debug!(
                "Model '{}' not found in catalog (provider-agnostic search)",
                self.id
            );
        }

        self
    }
}

/// Model capabilities (basic categorization)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
pub enum Capability {
    Chat,
    Completion,
    Embedding,
    Vision,
    FunctionCalling,
}

/// Core capability categories (for backward compatibility)
pub type CoreCapability = Capability;

/// Advanced feature capability with metadata
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct FeatureCapability {
    /// Feature name (e.g., "structured_outputs", "thinking", "caching")
    pub name: String,
    /// Whether this feature is supported
    pub supported: bool,
    /// Feature version (if applicable)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// Cost multiplier for using this feature (1.0 = no extra cost)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_multiplier: Option<f64>,
}

impl FeatureCapability {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            supported: true,
            version: None,
            cost_multiplier: None,
        }
    }

    pub fn with_version(mut self, version: impl Into<String>) -> Self {
        self.version = Some(version.into());
        self
    }

    pub fn with_cost_multiplier(mut self, multiplier: f64) -> Self {
        self.cost_multiplier = Some(multiplier);
        self
    }
}

/// Sampling parameter support information
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SamplingParameter {
    Temperature { min: f32, max: f32, default: f32 },
    TopP { min: f32, max: f32, default: f32 },
    TopK { min: u32, max: u32, default: u32 },
    RepetitionPenalty { min: f32, max: f32, default: f32 },
    FrequencyPenalty { min: f32, max: f32, default: f32 },
    PresencePenalty { min: f32, max: f32, default: f32 },
    Seed { supported: bool },
}

/// Parameter support information
#[derive(Debug, Clone, Serialize, Deserialize, Default, ToSchema)]
pub struct ParameterSupport {
    /// Sampling parameters supported
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub sampling: Vec<SamplingParameter>,
}

/// Performance metrics for a model
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PerformanceMetrics {
    /// Average latency in milliseconds
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avg_latency_ms: Option<u64>,
    /// Throughput in tokens per second
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tokens_per_second: Option<f64>,
    /// Cache hit rate (0.0 to 1.0)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_hit_rate: Option<f32>,
}

/// Enhanced model capabilities with detailed feature tracking
#[derive(Debug, Clone, Serialize, Deserialize, Default, ToSchema)]
pub struct ModelCapabilities {
    /// Core capabilities (for backward compatibility)
    pub core: Vec<CoreCapability>,

    /// Advanced features with metadata
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub features: Vec<FeatureCapability>,

    /// Supported parameters
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameters: Option<ParameterSupport>,

    /// Performance characteristics
    #[serde(skip_serializing_if = "Option::is_none")]
    pub performance: Option<PerformanceMetrics>,
}

impl ModelCapabilities {
    /// Create capabilities from core capabilities only
    pub fn from_core(core: Vec<CoreCapability>) -> Self {
        Self {
            core,
            features: Vec::new(),
            parameters: None,
            performance: None,
        }
    }

    /// Add a feature to this model's capabilities
    pub fn with_feature(mut self, feature: FeatureCapability) -> Self {
        self.features.push(feature);
        self
    }

    /// Add parameter support
    pub fn with_parameters(mut self, parameters: ParameterSupport) -> Self {
        self.parameters = Some(parameters);
        self
    }

    /// Add performance metrics
    pub fn with_performance(mut self, performance: PerformanceMetrics) -> Self {
        self.performance = Some(performance);
        self
    }
}

/// Pricing information for a model
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PricingInfo {
    /// Cost per 1K input tokens
    pub input_cost_per_1k: f64,
    /// Cost per 1K output tokens
    pub output_cost_per_1k: f64,
    /// Currency (e.g., "USD")
    pub currency: String,
}

impl PricingInfo {
    /// Creates a free pricing info (for local models like Ollama)
    pub fn free() -> Self {
        Self {
            input_cost_per_1k: 0.0,
            output_cost_per_1k: 0.0,
            currency: "USD".to_string(),
        }
    }
}

/// Provider health status
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ProviderHealth {
    /// Current health status
    pub status: HealthStatus,
    /// Response latency in milliseconds (if available)
    pub latency_ms: Option<u64>,
    /// When the health check was performed
    pub last_checked: DateTime<Utc>,
    /// Error message if unhealthy
    pub error_message: Option<String>,
}

/// Health status enum
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
pub enum HealthStatus {
    Healthy,
    Degraded,
    Unhealthy,
}

/// Chat completion request (OpenAI-compatible format)
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CompletionRequest {
    /// Model to use
    pub model: String,
    /// Array of messages
    pub messages: Vec<ChatMessage>,
    /// Temperature (0.0 to 2.0)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    /// Maximum tokens to generate
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    /// Whether to stream the response
    #[serde(default)]
    pub stream: bool,
    /// Top-p sampling
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    /// Frequency penalty
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frequency_penalty: Option<f32>,
    /// Presence penalty
    #[serde(skip_serializing_if = "Option::is_none")]
    pub presence_penalty: Option<f32>,
    /// Stop sequences
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop: Option<Vec<String>>,

    // Extended parameters (Phase 1)
    /// Top-k sampling
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_k: Option<u32>,
    /// Seed for deterministic generation
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seed: Option<i64>,
    /// Repetition penalty
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repetition_penalty: Option<f32>,
    /// Provider-specific extensions (Phase 3)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extensions: Option<std::collections::HashMap<String, serde_json::Value>>,

    // Tool calling (Bug #4 fix)
    /// Tool definitions for function calling
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<Tool>>,
    /// Tool choice mode (auto, none, or specific function)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<ToolChoice>,

    // Response format (Bug #7 fix)
    /// Response format specification for structured outputs
    /// Note: Providers should enforce this using their native JSON mode or structured output features
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_format: Option<ResponseFormat>,

    // Log probabilities (Bug #6 fix)
    /// Whether to return log probabilities of the output tokens
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logprobs: Option<bool>,
    /// Number of most likely tokens to return at each position (0-20)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_logprobs: Option<u32>,
}

/// Tool definition for function calling
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Tool {
    #[serde(rename = "type")]
    pub tool_type: String,
    pub function: FunctionDefinition,
}

/// Function definition for tools
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct FunctionDefinition {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub parameters: serde_json::Value,
}

/// Function name for tool choice
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct FunctionName {
    pub name: String,
}

/// Tool choice mode
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(untagged)]
pub enum ToolChoice {
    /// Auto mode - let the model decide
    Auto(String),
    /// Specific tool selection
    Specific {
        #[serde(rename = "type")]
        tool_type: String,
        function: FunctionName,
    },
}

/// Tool call in the assistant's response
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ToolCall {
    /// Unique ID for this tool call
    pub id: String,
    /// Type of tool (always "function" for now)
    #[serde(rename = "type")]
    pub tool_type: String,
    /// Function call details
    pub function: FunctionCall,
}

/// Function call details
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct FunctionCall {
    /// Function name
    pub name: String,
    /// Function arguments (JSON string)
    pub arguments: String,
}

/// Response format specification for structured outputs
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(untagged)]
pub enum ResponseFormat {
    /// JSON object mode - response will be valid JSON
    JsonObject {
        #[serde(rename = "type")]
        format_type: String,
    },
    /// JSON schema mode - response will conform to schema
    JsonSchema {
        #[serde(rename = "type")]
        format_type: String,
        /// JSON schema definition
        schema: serde_json::Value,
    },
}

/// Image URL for multimodal messages
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ImageUrl {
    pub url: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// Content part for multimodal messages
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentPart {
    Text { text: String },
    ImageUrl { image_url: ImageUrl },
}

/// Message content - either simple text or multimodal parts
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(untagged)]
pub enum ChatMessageContent {
    Text(String),
    Parts(Vec<ContentPart>),
}

impl ChatMessageContent {
    /// Extract text content from the message, ignoring images
    pub fn as_text(&self) -> String {
        match self {
            ChatMessageContent::Text(text) => text.clone(),
            ChatMessageContent::Parts(parts) => {
                parts
                    .iter()
                    .filter_map(|part| match part {
                        ContentPart::Text { text } => Some(text.clone()),
                        ContentPart::ImageUrl { .. } => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            }
        }
    }

    /// Check if this message contains any images
    pub fn has_images(&self) -> bool {
        match self {
            ChatMessageContent::Text(_) => false,
            ChatMessageContent::Parts(parts) => parts.iter().any(|part| matches!(part, ContentPart::ImageUrl { .. })),
        }
    }

    /// Get a reference to text content as a string slice
    /// For multimodal content, extracts and concatenates text parts
    pub fn as_str(&self) -> std::borrow::Cow<'_, str> {
        match self {
            ChatMessageContent::Text(text) => std::borrow::Cow::Borrowed(text.as_str()),
            ChatMessageContent::Parts(_) => std::borrow::Cow::Owned(self.as_text()),
        }
    }

    /// Check if the content is empty (no text or images)
    pub fn is_empty(&self) -> bool {
        match self {
            ChatMessageContent::Text(text) => text.is_empty(),
            ChatMessageContent::Parts(parts) => parts.is_empty(),
        }
    }
}

/// Chat message
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ChatMessage {
    /// Role (system, user, assistant, tool)
    pub role: String,
    /// Message content (text or multimodal)
    pub content: ChatMessageContent,
    /// Tool calls made by the assistant (only for assistant role)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    /// Tool call ID (only for tool role)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    /// Function name (deprecated, use tool_calls instead)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// Chat completion response
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CompletionResponse {
    /// Response ID
    pub id: String,
    /// Object type ("chat.completion")
    pub object: String,
    /// Creation timestamp
    pub created: i64,
    /// Model used
    pub model: String,
    /// Provider name
    pub provider: String,
    /// Response choices
    pub choices: Vec<CompletionChoice>,
    /// Token usage information
    pub usage: TokenUsage,
    /// Provider-specific extensions (Phase 3)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extensions: Option<std::collections::HashMap<String, serde_json::Value>>,
    /// RouteLLM win rate (0.0-1.0) if RouteLLM routing was used
    #[serde(skip_serializing_if = "Option::is_none")]
    pub routellm_win_rate: Option<f32>,
}

/// Completion choice
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CompletionChoice {
    /// Choice index
    pub index: u32,
    /// Message content
    pub message: ChatMessage,
    /// Finish reason ("stop", "length", "content_filter", "tool_calls")
    pub finish_reason: Option<String>,
    /// Log probability information for the choice
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logprobs: Option<Logprobs>,
}

/// Log probability information for tokens
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Logprobs {
    /// List of message content tokens with log probability information
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<Vec<TokenLogprob>>,
}

/// Log probability information for a single token
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TokenLogprob {
    /// The token
    pub token: String,
    /// The log probability of this token
    pub logprob: f64,
    /// A list of integers representing the UTF-8 bytes of the token
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes: Option<Vec<u8>>,
    /// List of the most likely tokens and their log probabilities
    pub top_logprobs: Vec<TopLogprob>,
}

/// Top alternative token with log probability
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TopLogprob {
    /// The token
    pub token: String,
    /// The log probability of this token
    pub logprob: f64,
    /// A list of integers representing the UTF-8 bytes of the token
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes: Option<Vec<u8>>,
}

/// Token usage information
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TokenUsage {
    /// Number of prompt tokens
    pub prompt_tokens: u32,
    /// Number of completion tokens
    pub completion_tokens: u32,
    /// Total tokens
    pub total_tokens: u32,
    /// Detailed prompt token breakdown (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_tokens_details: Option<PromptTokensDetails>,
    /// Detailed completion token breakdown (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completion_tokens_details: Option<CompletionTokensDetails>,
}

/// Detailed breakdown of prompt token usage
///
/// Used to track advanced token metrics like prompt caching.
/// All fields are optional to maintain compatibility across providers.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct PromptTokensDetails {
    /// Number of cached tokens (tokens already in cache)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cached_tokens: Option<u32>,
    /// Number of tokens written to cache (cache creation)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_creation_tokens: Option<u32>,
    /// Number of tokens read from cache (cache hits)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_read_tokens: Option<u32>,
}

/// Detailed breakdown of completion token usage
///
/// Used to track special token types like reasoning tokens, thinking tokens, and audio tokens.
/// All fields are optional to maintain compatibility across providers.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CompletionTokensDetails {
    /// Number of reasoning tokens (e.g., OpenAI o1 series)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_tokens: Option<u32>,
    /// Number of thinking tokens (e.g., Anthropic extended thinking)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking_tokens: Option<u32>,
    /// Number of audio tokens (for TTS/STT models)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audio_tokens: Option<u32>,
}

/// Streaming completion chunk
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CompletionChunk {
    /// Chunk ID
    pub id: String,
    /// Object type ("chat.completion.chunk")
    pub object: String,
    /// Creation timestamp
    pub created: i64,
    /// Model used
    pub model: String,
    /// Chunk choices
    pub choices: Vec<ChunkChoice>,
    /// Provider-specific extensions (Phase 3)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extensions: Option<std::collections::HashMap<String, serde_json::Value>>,
}

/// Streaming chunk choice
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ChunkChoice {
    /// Choice index
    pub index: u32,
    /// Delta content
    pub delta: ChunkDelta,
    /// Finish reason (present in final chunk)
    pub finish_reason: Option<String>,
}

/// Delta content in streaming chunk
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ChunkDelta {
    /// Role (only in first chunk)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// Content delta
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    /// Tool calls delta (for streaming tool calls)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCallDelta>>,
}

/// Tool call delta for streaming
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ToolCallDelta {
    pub index: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub tool_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function: Option<FunctionCallDelta>,
}

/// Function call delta for streaming
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct FunctionCallDelta {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arguments: Option<String>,
}

// ==================== EMBEDDING TYPES ====================

/// Embedding request
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct EmbeddingRequest {
    /// Model to use for embeddings
    pub model: String,
    /// Input text(s) to embed
    pub input: EmbeddingInput,
    /// Encoding format for the embeddings
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encoding_format: Option<EncodingFormat>,
    /// Custom dimensions (if supported by model)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dimensions: Option<u32>,
    /// User identifier for tracking
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
}

/// Input for embedding request (can be single string, array of strings, or token arrays)
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(untagged)]
pub enum EmbeddingInput {
    /// Single text string
    Single(String),
    /// Multiple text strings
    Multiple(Vec<String>),
    /// Pre-tokenized input
    Tokens(Vec<Vec<u32>>),
}

/// Encoding format for embeddings
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum EncodingFormat {
    /// Floating point array
    Float,
    /// Base64-encoded string
    Base64,
}

/// Embedding response
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct EmbeddingResponse {
    /// Object type ("list")
    pub object: String,
    /// Array of embeddings
    pub data: Vec<Embedding>,
    /// Model used
    pub model: String,
    /// Token usage information
    pub usage: EmbeddingUsage,
}

/// Single embedding
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Embedding {
    /// Object type ("embedding")
    pub object: String,
    /// Embedding vector (float array or base64 string)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embedding: Option<Vec<f32>>,
    /// Index in the input array
    pub index: usize,
}

/// Token usage for embeddings
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct EmbeddingUsage {
    /// Number of prompt tokens
    pub prompt_tokens: u32,
    /// Total tokens (same as prompt_tokens for embeddings)
    pub total_tokens: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_token_usage_basic_serialization() {
        let usage = TokenUsage {
            prompt_tokens: 100,
            completion_tokens: 50,
            total_tokens: 150,
            prompt_tokens_details: None,
            completion_tokens_details: None,
        };

        let json = serde_json::to_value(&usage).unwrap();

        // Verify basic fields
        assert_eq!(json["prompt_tokens"], 100);
        assert_eq!(json["completion_tokens"], 50);
        assert_eq!(json["total_tokens"], 150);

        // Verify optional fields are NOT present (OpenAI compatibility)
        assert!(json.get("prompt_tokens_details").is_none());
        assert!(json.get("completion_tokens_details").is_none());
    }

    #[test]
    fn test_token_usage_with_prompt_details() {
        let usage = TokenUsage {
            prompt_tokens: 1000,
            completion_tokens: 200,
            total_tokens: 1200,
            prompt_tokens_details: Some(PromptTokensDetails {
                cached_tokens: Some(500),
                cache_creation_tokens: Some(300),
                cache_read_tokens: Some(200),
            }),
            completion_tokens_details: None,
        };

        let json = serde_json::to_value(&usage).unwrap();

        // Verify basic fields
        assert_eq!(json["prompt_tokens"], 1000);
        assert_eq!(json["completion_tokens"], 200);
        assert_eq!(json["total_tokens"], 1200);

        // Verify prompt details are present
        let prompt_details = json["prompt_tokens_details"].as_object().unwrap();
        assert_eq!(prompt_details["cached_tokens"], 500);
        assert_eq!(prompt_details["cache_creation_tokens"], 300);
        assert_eq!(prompt_details["cache_read_tokens"], 200);

        // Verify completion details are NOT present
        assert!(json.get("completion_tokens_details").is_none());
    }

    #[test]
    fn test_token_usage_with_completion_details() {
        let usage = TokenUsage {
            prompt_tokens: 100,
            completion_tokens: 250,
            total_tokens: 350,
            prompt_tokens_details: None,
            completion_tokens_details: Some(CompletionTokensDetails {
                reasoning_tokens: Some(50),
                thinking_tokens: Some(30),
                audio_tokens: Some(20),
            }),
        };

        let json = serde_json::to_value(&usage).unwrap();

        // Verify completion details are present
        let completion_details = json["completion_tokens_details"].as_object().unwrap();
        assert_eq!(completion_details["reasoning_tokens"], 50);
        assert_eq!(completion_details["thinking_tokens"], 30);
        assert_eq!(completion_details["audio_tokens"], 20);

        // Verify prompt details are NOT present
        assert!(json.get("prompt_tokens_details").is_none());
    }

    #[test]
    fn test_token_usage_with_all_details() {
        let usage = TokenUsage {
            prompt_tokens: 1000,
            completion_tokens: 300,
            total_tokens: 1300,
            prompt_tokens_details: Some(PromptTokensDetails {
                cached_tokens: Some(500),
                cache_creation_tokens: Some(200),
                cache_read_tokens: Some(200),
            }),
            completion_tokens_details: Some(CompletionTokensDetails {
                reasoning_tokens: Some(100),
                thinking_tokens: Some(50),
                audio_tokens: None,
            }),
        };

        let json = serde_json::to_value(&usage).unwrap();

        // Verify all fields are present
        assert_eq!(json["prompt_tokens"], 1000);
        assert_eq!(json["completion_tokens"], 300);
        assert_eq!(json["total_tokens"], 1300);

        let prompt_details = json["prompt_tokens_details"].as_object().unwrap();
        assert_eq!(prompt_details["cached_tokens"], 500);

        let completion_details = json["completion_tokens_details"].as_object().unwrap();
        assert_eq!(completion_details["reasoning_tokens"], 100);
        assert_eq!(completion_details["thinking_tokens"], 50);
        // audio_tokens should not be present since it's None
        assert!(completion_details.get("audio_tokens").is_none());
    }

    #[test]
    fn test_token_usage_deserialization_basic() {
        // Test that old format (without details) deserializes correctly
        let json = r#"{
            "prompt_tokens": 100,
            "completion_tokens": 50,
            "total_tokens": 150
        }"#;

        let usage: TokenUsage = serde_json::from_str(json).unwrap();

        assert_eq!(usage.prompt_tokens, 100);
        assert_eq!(usage.completion_tokens, 50);
        assert_eq!(usage.total_tokens, 150);
        assert!(usage.prompt_tokens_details.is_none());
        assert!(usage.completion_tokens_details.is_none());
    }

    #[test]
    fn test_token_usage_deserialization_with_details() {
        let json = r#"{
            "prompt_tokens": 1000,
            "completion_tokens": 200,
            "total_tokens": 1200,
            "prompt_tokens_details": {
                "cached_tokens": 500,
                "cache_creation_tokens": 300,
                "cache_read_tokens": 200
            },
            "completion_tokens_details": {
                "reasoning_tokens": 50,
                "thinking_tokens": 30
            }
        }"#;

        let usage: TokenUsage = serde_json::from_str(json).unwrap();

        assert_eq!(usage.prompt_tokens, 1000);
        assert_eq!(usage.completion_tokens, 200);
        assert_eq!(usage.total_tokens, 1200);

        let prompt_details = usage.prompt_tokens_details.unwrap();
        assert_eq!(prompt_details.cached_tokens, Some(500));
        assert_eq!(prompt_details.cache_creation_tokens, Some(300));
        assert_eq!(prompt_details.cache_read_tokens, Some(200));

        let completion_details = usage.completion_tokens_details.unwrap();
        assert_eq!(completion_details.reasoning_tokens, Some(50));
        assert_eq!(completion_details.thinking_tokens, Some(30));
        assert_eq!(completion_details.audio_tokens, None);
    }

    #[test]
    fn test_prompt_tokens_details_partial_fields() {
        // Test that PromptTokensDetails with some None values works correctly
        let details = PromptTokensDetails {
            cached_tokens: Some(100),
            cache_creation_tokens: None,
            cache_read_tokens: Some(50),
        };

        let json = serde_json::to_value(&details).unwrap();

        assert_eq!(json["cached_tokens"], 100);
        assert_eq!(json["cache_read_tokens"], 50);
        // cache_creation_tokens should not be serialized
        assert!(json.get("cache_creation_tokens").is_none());
    }

    #[test]
    fn test_completion_tokens_details_partial_fields() {
        // Test that CompletionTokensDetails with some None values works correctly
        let details = CompletionTokensDetails {
            reasoning_tokens: Some(75),
            thinking_tokens: None,
            audio_tokens: None,
        };

        let json = serde_json::to_value(&details).unwrap();

        assert_eq!(json["reasoning_tokens"], 75);
        // Other fields should not be serialized
        assert!(json.get("thinking_tokens").is_none());
        assert!(json.get("audio_tokens").is_none());
    }

    #[test]
    fn test_openai_compatibility() {
        // Verify that TokenUsage without details matches OpenAI's exact format
        let usage = TokenUsage {
            prompt_tokens: 10,
            completion_tokens: 5,
            total_tokens: 15,
            prompt_tokens_details: None,
            completion_tokens_details: None,
        };

        let json_str = serde_json::to_string(&usage).unwrap();

        // Parse back to verify only expected fields are present
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        let obj = parsed.as_object().unwrap();

        // Should have exactly 3 fields
        assert_eq!(obj.len(), 3);
        assert!(obj.contains_key("prompt_tokens"));
        assert!(obj.contains_key("completion_tokens"));
        assert!(obj.contains_key("total_tokens"));
    }
}
