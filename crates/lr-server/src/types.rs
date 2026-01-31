//! API request and response types for OpenAI-compatible endpoints

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use utoipa::ToSchema;

// ==================== Chat Completions ====================

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(
    title = "Chat Completion Request",
    description = "Request for chat completion API compatible with OpenAI's format",
    example = json!({
        "model": "gpt-4",
        "messages": [
            {"role": "system", "content": "You are a helpful assistant."},
            {"role": "user", "content": "Hello!"}
        ],
        "temperature": 0.7,
        "max_tokens": 1000
    })
)]
pub struct ChatCompletionRequest {
    #[schema(example = "gpt-4")]
    pub model: String,

    #[schema(min_items = 1)]
    pub messages: Vec<ChatMessage>,

    // Sampling parameters
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(minimum = 0.0, maximum = 2.0)]
    pub temperature: Option<f32>,

    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(minimum = 0.0, maximum = 1.0)]
    pub top_p: Option<f32>,

    // Output control
    /// Maximum number of tokens to generate (deprecated, use max_completion_tokens for o-series models)
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(minimum = 1)]
    pub max_tokens: Option<u32>,

    /// Maximum number of tokens to generate (replaces max_tokens for o-series models)
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(minimum = 1)]
    pub max_completion_tokens: Option<u32>,

    /// Number of chat completion choices to generate (default: 1)
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(minimum = 1, maximum = 128, default = 1)]
    pub n: Option<u32>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop: Option<StopSequence>,

    // Streaming
    #[serde(default)]
    #[schema(default = false)]
    pub stream: bool,

    // Log probabilities
    /// Whether to return log probabilities of the output tokens
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logprobs: Option<bool>,

    /// Number of most likely tokens to return at each position (0-20)
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(minimum = 0, maximum = 20)]
    pub top_logprobs: Option<u32>,

    // Advanced parameters
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(minimum = -2.0, maximum = 2.0)]
    pub frequency_penalty: Option<f32>,

    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(minimum = -2.0, maximum = 2.0)]
    pub presence_penalty: Option<f32>,

    // Extended sampling parameters (Layer 2 - Extended OpenAI Compatibility)
    // Note: These are LocalRouter extensions not present in the standard OpenAI API
    /// Top-K sampling (LocalRouter extension, not in OpenAI API)
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(minimum = 1)]
    pub top_k: Option<u32>,

    /// Seed for deterministic generation (supported by some OpenAI models)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seed: Option<i64>,

    /// Repetition penalty (LocalRouter extension, not in OpenAI API)
    /// Range: 0.0-2.0, where 1.0 is no penalty, <1.0 encourages repetition, >1.0 discourages it
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(minimum = 0.0, maximum = 2.0)]
    pub repetition_penalty: Option<f32>,

    // Response format for structured outputs
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_format: Option<ResponseFormat>,

    // Tool calling
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<Tool>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<ToolChoice>,

    // Provider-specific extensions (Layer 3)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extensions: Option<HashMap<String, Value>>,

    // User tracking
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(untagged)]
pub enum ResponseFormat {
    JsonObject { r#type: String },
    JsonSchema { r#type: String, schema: Value },
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(untagged)]
pub enum StopSequence {
    Single(String),
    Multiple(Vec<String>),
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ChatMessage {
    #[schema(example = "user")]
    pub role: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<MessageContent>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    /// Tool calls made by the assistant (only for assistant role)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,

    /// Tool call ID (only for tool role messages)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

/// Tool call in assistant's response
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub tool_type: String,
    pub function: FunctionCall,
}

/// Function call details in tool call
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct FunctionCall {
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(untagged)]
pub enum MessageContent {
    Text(String),
    Parts(Vec<ContentPart>),
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ImageUrl {
    pub url: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentPart {
    Text { text: String },
    ImageUrl { image_url: ImageUrl },
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Tool {
    #[serde(rename = "type")]
    #[schema(example = "function")]
    pub tool_type: String,

    pub function: FunctionDefinition,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct FunctionDefinition {
    #[schema(example = "get_weather")]
    pub name: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    pub parameters: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct FunctionName {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(untagged)]
pub enum ToolChoice {
    Auto(String),
    Specific {
        #[serde(rename = "type")]
        tool_type: String,
        function: FunctionName,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(
    title = "Chat Completion Response",
    description = "Response from chat completion API",
    example = json!({
        "id": "chatcmpl-123",
        "object": "chat.completion",
        "created": 1677652288,
        "model": "gpt-4",
        "choices": [{
            "index": 0,
            "message": {"role": "assistant", "content": "Hello! How can I help you today?"},
            "finish_reason": "stop"
        }],
        "usage": {"prompt_tokens": 10, "completion_tokens": 20, "total_tokens": 30}
    })
)]
pub struct ChatCompletionResponse {
    #[schema(example = "chatcmpl-123")]
    pub id: String,

    #[schema(example = "chat.completion")]
    pub object: String,

    pub created: i64,

    #[schema(example = "gpt-4")]
    pub model: String,

    pub choices: Vec<ChatCompletionChoice>,

    pub usage: TokenUsage,

    /// Provider-specific extensions in the response
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extensions: Option<HashMap<String, Value>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ChatCompletionChoice {
    pub index: u32,

    pub message: ChatMessage,

    #[schema(example = "stop")]
    pub finish_reason: Option<String>,

    /// Log probability information for the choice
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logprobs: Option<ChatCompletionLogprobs>,
}

/// Log probability information for tokens
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ChatCompletionLogprobs {
    /// List of message content tokens with log probability information
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<Vec<ChatCompletionTokenLogprob>>,
}

/// Log probability information for a single token
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ChatCompletionTokenLogprob {
    /// The token
    pub token: String,

    /// The log probability of this token
    pub logprob: f64,

    /// A list of integers representing the UTF-8 bytes of the token
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
    pub bytes: Option<Vec<u8>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TokenUsage {
    pub prompt_tokens: u32,

    pub completion_tokens: u32,

    pub total_tokens: u32,

    /// Detailed prompt token breakdown (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_tokens_details: Option<crate::providers::PromptTokensDetails>,

    /// Detailed completion token breakdown (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completion_tokens_details: Option<crate::providers::CompletionTokensDetails>,
}

// Streaming chunk
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ChatCompletionChunk {
    #[schema(example = "chatcmpl-123")]
    pub id: String,

    #[schema(example = "chat.completion.chunk")]
    pub object: String,

    pub created: i64,

    #[schema(example = "gpt-4")]
    pub model: String,

    pub choices: Vec<ChatCompletionChunkChoice>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<TokenUsage>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ChatCompletionChunkChoice {
    pub index: u32,

    pub delta: ChunkDelta,

    pub finish_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ChunkDelta {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,

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

// ==================== Completions (Legacy) ====================

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CompletionRequest {
    #[schema(example = "gpt-3.5-turbo-instruct")]
    pub model: String,

    pub prompt: PromptInput,

    // Sampling
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(minimum = 0.0, maximum = 2.0)]
    pub temperature: Option<f32>,

    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(minimum = 0.0, maximum = 1.0)]
    pub top_p: Option<f32>,

    // Output
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(minimum = 1)]
    pub max_tokens: Option<u32>,

    /// Number of completion choices to generate (default: 1)
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(minimum = 1, maximum = 128, default = 1)]
    pub n: Option<u32>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop: Option<StopSequence>,

    // Advanced
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(minimum = -2.0, maximum = 2.0)]
    pub frequency_penalty: Option<f32>,

    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(minimum = -2.0, maximum = 2.0)]
    pub presence_penalty: Option<f32>,

    // Streaming
    #[serde(default)]
    #[schema(default = false)]
    pub stream: bool,

    // Log probabilities
    /// Whether to return log probabilities of the output tokens
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logprobs: Option<u32>,

    // Misc
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(untagged)]
pub enum PromptInput {
    Single(String),
    Multiple(Vec<String>),
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CompletionResponse {
    #[schema(example = "cmpl-123")]
    pub id: String,

    #[schema(example = "text_completion")]
    pub object: String,

    pub created: i64,

    pub model: String,

    pub choices: Vec<CompletionChoice>,

    pub usage: TokenUsage,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CompletionChoice {
    pub text: String,

    pub index: u32,

    pub finish_reason: Option<String>,

    pub logprobs: Option<Value>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CompletionChunk {
    pub id: String,

    pub object: String,

    pub created: i64,

    pub choices: Vec<CompletionChunkChoice>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CompletionChunkChoice {
    pub text: String,

    pub index: u32,

    pub finish_reason: Option<String>,
}

// ==================== Embeddings ====================

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct EmbeddingRequest {
    #[schema(example = "text-embedding-ada-002")]
    pub model: String,

    pub input: EmbeddingInput,

    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "float")]
    pub encoding_format: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub dimensions: Option<u32>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(untagged)]
pub enum EmbeddingInput {
    Single(String),
    Multiple(Vec<String>),
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct EmbeddingResponse {
    #[schema(example = "list")]
    pub object: String,

    pub data: Vec<EmbeddingData>,

    pub model: String,

    pub usage: EmbeddingUsage,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct EmbeddingData {
    #[schema(example = "embedding")]
    pub object: String,

    pub embedding: EmbeddingVector,

    pub index: u32,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(untagged)]
pub enum EmbeddingVector {
    Float(Vec<f32>),
    Base64(String),
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct EmbeddingUsage {
    pub prompt_tokens: u32,

    pub total_tokens: u32,
}

// ==================== Models ====================

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ModelsResponse {
    #[schema(example = "list")]
    pub object: String,

    pub data: Vec<ModelData>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ModelData {
    #[schema(example = "gpt-4")]
    pub id: String,

    #[schema(example = "model")]
    pub object: String,

    #[schema(example = "openai")]
    pub owned_by: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub created: Option<i64>,

    // LocalRouter-specific metadata
    #[schema(example = "openai")]
    pub provider: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameter_count: Option<u64>,

    #[schema(example = 8192)]
    pub context_window: u32,

    pub supports_streaming: bool,

    pub capabilities: Vec<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub pricing: Option<ModelPricing>,

    // Enhanced capability tracking (Phase 1)
    /// Detailed capability information with parameters and features
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detailed_capabilities: Option<crate::providers::ModelCapabilities>,

    /// List of supported advanced features (e.g., "structured_outputs", "thinking", "caching")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub features: Option<Vec<String>>,

    /// List of supported sampling parameters (e.g., "top_k", "seed", "repetition_penalty")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supported_parameters: Option<Vec<String>>,

    /// Performance metrics for this model
    #[serde(skip_serializing_if = "Option::is_none")]
    pub performance: Option<crate::providers::PerformanceMetrics>,

    /// Catalog information (Phase 4)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub catalog_info: Option<CatalogInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ModelPricing {
    #[schema(example = 0.03)]
    pub input_cost_per_1k: f64,

    #[schema(example = 0.06)]
    pub output_cost_per_1k: f64,

    #[schema(example = "USD")]
    pub currency: String,
}

/// Catalog information for model metadata
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CatalogInfo {
    /// Source of pricing information
    pub pricing_source: PricingSource,

    /// Date when catalog was last fetched (if from catalog)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub catalog_date: Option<String>,

    /// How the model was matched in catalog
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matched_via: Option<String>,
}

/// Source of pricing information
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum PricingSource {
    /// Pricing from embedded catalog (OpenRouter)
    Catalog,
    /// Pricing from provider API
    Provider,
    /// Pricing from hardcoded fallback
    Fallback,
}

// ==================== Generation Details ====================

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct GenerationDetailsResponse {
    #[schema(example = "chatcmpl-123")]
    pub id: String,

    #[schema(example = "gpt-4")]
    pub model: String,

    #[schema(example = "openai")]
    pub provider: String,

    pub created: i64,

    #[schema(example = "stop")]
    pub finish_reason: String,

    pub tokens: TokenUsage,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost: Option<CostDetails>,

    pub latency_ms: u64,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_health: Option<ProviderHealthSnapshot>,

    pub api_key_id: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,

    pub stream: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CostDetails {
    pub prompt_cost: f64,

    pub completion_cost: f64,

    pub total_cost: f64,

    #[schema(example = "USD")]
    pub currency: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ProviderHealthSnapshot {
    #[schema(example = "healthy")]
    pub status: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<u64>,
}

// ==================== Error Response ====================

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(
    description = "Error response following OpenAI format",
    example = json!({
        "error": {
            "message": "Invalid API key",
            "type": "invalid_request_error",
            "code": "invalid_api_key"
        }
    })
)]
pub struct ErrorResponse {
    pub error: ApiError,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ApiError {
    #[schema(example = "Invalid API key")]
    pub message: String,

    #[serde(rename = "type")]
    #[schema(example = "invalid_request_error")]
    pub error_type: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub param: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "invalid_api_key")]
    pub code: Option<String>,
}

impl ErrorResponse {
    pub fn new(error_type: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            error: ApiError {
                message: message.into(),
                error_type: error_type.into(),
                param: None,
                code: None,
            },
        }
    }

    pub fn with_param(mut self, param: impl Into<String>) -> Self {
        self.error.param = Some(param.into());
        self
    }

    pub fn with_code(mut self, code: impl Into<String>) -> Self {
        self.error.code = Some(code.into());
        self
    }
}

/// Simple success message response
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct MessageResponse {
    #[schema(example = "Operation completed successfully")]
    pub message: String,
}

// Helper functions for conversion
impl From<&crate::providers::ModelInfo> for ModelData {
    fn from(info: &crate::providers::ModelInfo) -> Self {
        use crate::providers::Capability;

        let capabilities = info
            .capabilities
            .iter()
            .map(|c| {
                match c {
                    Capability::Chat => "chat",
                    Capability::Completion => "completion",
                    Capability::Embedding => "embedding",
                    Capability::Vision => "vision",
                    Capability::FunctionCalling => "function_calling",
                }
                .to_string()
            })
            .collect();

        Self {
            id: info.id.clone(),
            object: "model".to_string(),
            owned_by: info.provider.clone(),
            created: None,
            provider: info.provider.clone(),
            parameter_count: info.parameter_count,
            context_window: info.context_window,
            supports_streaming: info.supports_streaming,
            capabilities,
            pricing: None,               // Will be filled separately
            detailed_capabilities: None, // Will be filled by /v1/models endpoint
            features: None,              // Will be filled by /v1/models endpoint
            supported_parameters: None,  // Will be filled by /v1/models endpoint
            performance: None,           // Will be filled by /v1/models endpoint
            catalog_info: None,          // TODO: Populate from catalog
        }
    }
}

// ==================== Image Generation ====================

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(
    title = "Image Generation Request",
    description = "Request for image generation API compatible with OpenAI's DALL-E format",
    example = json!({
        "model": "dall-e-3",
        "prompt": "A white siamese cat",
        "n": 1,
        "size": "1024x1024",
        "quality": "standard",
        "style": "vivid"
    })
)]
pub struct ImageGenerationRequest {
    /// The model to use for image generation
    #[schema(example = "dall-e-3")]
    pub model: String,

    /// A text description of the desired image(s)
    #[schema(example = "A white siamese cat")]
    pub prompt: String,

    /// The number of images to generate (1-10)
    #[serde(default = "default_image_count")]
    #[schema(minimum = 1, maximum = 10, default = 1)]
    pub n: Option<u32>,

    /// The size of the generated images
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "1024x1024")]
    pub size: Option<String>,

    /// The quality of the image (dall-e-3 only)
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "standard")]
    pub quality: Option<String>,

    /// The style of the generated images (dall-e-3 only)
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "vivid")]
    pub style: Option<String>,

    /// The format in which the generated images are returned
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "b64_json")]
    pub response_format: Option<String>,

    /// A unique identifier representing your end-user
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
}

fn default_image_count() -> Option<u32> {
    Some(1)
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(
    title = "Image Generation Response",
    description = "Response from image generation API"
)]
pub struct ImageGenerationResponse {
    /// Unix timestamp when the images were created
    pub created: i64,

    /// The generated images
    pub data: Vec<ImageData>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(title = "Image Data", description = "Generated image data")]
pub struct ImageData {
    /// The URL of the generated image (if response_format is "url")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,

    /// The base64-encoded JSON of the generated image (if response_format is "b64_json")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub b64_json: Option<String>,

    /// The prompt that was used to generate the image (dall-e-3 may revise the prompt)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revised_prompt: Option<String>,
}
