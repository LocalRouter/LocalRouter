//! Model provider implementations
//!
//! Abstractions and implementations for various AI model providers.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use futures::Stream;
use serde::{Deserialize, Serialize};
use std::pin::Pin;

use crate::utils::errors::{AppError, AppResult};

pub mod anthropic;
pub mod factory;
pub mod gemini;
pub mod health;
pub mod key_storage;
pub mod ollama;
pub mod openai;
pub mod openai_compatible;
pub mod openrouter;
pub mod registry;

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
    async fn embed(&self, _request: EmbeddingRequest) -> AppResult<EmbeddingResponse> {
        Err(AppError::Provider(format!(
            "Provider '{}' does not support embeddings",
            self.name()
        )))
    }
}

/// Information about a model
#[derive(Debug, Clone, Serialize, Deserialize)]
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
    /// Model capabilities
    pub capabilities: Vec<Capability>,
}

/// Model capabilities
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum Capability {
    Chat,
    Completion,
    Embedding,
    Vision,
    FunctionCalling,
}

/// Pricing information for a model
#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum HealthStatus {
    Healthy,
    Degraded,
    Unhealthy,
}

/// Chat completion request (OpenAI-compatible format)
#[derive(Debug, Clone, Serialize, Deserialize)]
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
}

/// Chat message
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    /// Role (system, user, assistant)
    pub role: String,
    /// Message content
    pub content: String,
}

/// Chat completion response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionResponse {
    /// Response ID
    pub id: String,
    /// Object type ("chat.completion")
    pub object: String,
    /// Creation timestamp
    pub created: i64,
    /// Model used
    pub model: String,
    /// Response choices
    pub choices: Vec<CompletionChoice>,
    /// Token usage information
    pub usage: TokenUsage,
}

/// Completion choice
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionChoice {
    /// Choice index
    pub index: u32,
    /// Message content
    pub message: ChatMessage,
    /// Finish reason ("stop", "length", "content_filter")
    pub finish_reason: Option<String>,
}

/// Token usage information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenUsage {
    /// Number of prompt tokens
    pub prompt_tokens: u32,
    /// Number of completion tokens
    pub completion_tokens: u32,
    /// Total tokens
    pub total_tokens: u32,
}

/// Streaming completion chunk
#[derive(Debug, Clone, Serialize, Deserialize)]
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
}

/// Streaming chunk choice
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkChoice {
    /// Choice index
    pub index: u32,
    /// Delta content
    pub delta: ChunkDelta,
    /// Finish reason (present in final chunk)
    pub finish_reason: Option<String>,
}

/// Delta content in streaming chunk
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkDelta {
    /// Role (only in first chunk)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// Content delta
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
}

// ==================== EMBEDDING TYPES ====================

/// Embedding request
#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EncodingFormat {
    /// Floating point array
    Float,
    /// Base64-encoded string
    Base64,
}

/// Embedding response
#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingUsage {
    /// Number of prompt tokens
    pub prompt_tokens: u32,
    /// Total tokens (same as prompt_tokens for embeddings)
    pub total_tokens: u32,
}
