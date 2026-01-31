//! Feature adapter schemas for OpenAPI documentation
//!
//! This module defines schemas for the 7 advanced feature adapters supported by LocalRouter.
//! These are provider-specific extensions that go beyond the standard OpenAI API.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Extended Thinking Parameters (Anthropic)
///
/// Enables Anthropic's extended thinking mode for deeper reasoning on complex problems.
/// When enabled, the model will spend more time "thinking" before responding.
///
/// **Provider**: Anthropic (Claude models)
///
/// **Use Cases**:
/// - Complex reasoning tasks
/// - Mathematical problems
/// - Strategic planning
/// - Code analysis
///
/// **Cost**: Uses additional tokens for reasoning (budget_tokens)
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(
    title = "Extended Thinking Parameters",
    description = "Anthropic's extended thinking mode for deeper reasoning",
    example = json!({
        "enabled": true,
        "budget_tokens": 5000
    })
)]
pub struct ExtendedThinkingParams {
    /// Enable extended thinking mode
    #[schema(default = true)]
    pub enabled: Option<bool>,

    /// Maximum tokens to use for thinking (token budget for reasoning process)
    #[schema(minimum = 1000, maximum = 100000)]
    pub budget_tokens: Option<u32>,
}

/// Reasoning Tokens Parameters (OpenAI)
///
/// For OpenAI's o1 series models, this enables the model to show its reasoning process.
/// Reasoning tokens are used internally by the model before generating the final response.
///
/// **Provider**: OpenAI (o1 models)
///
/// **Use Cases**:
/// - Mathematical proofs
/// - Complex problem solving
/// - Multi-step reasoning
/// - Code debugging
///
/// **Note**: Reasoning tokens are counted separately in usage metrics
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(
    title = "Reasoning Tokens Parameters",
    description = "OpenAI o1 model reasoning tokens configuration",
    example = json!({
        "enabled": true,
        "max_reasoning_tokens": 10000
    })
)]
pub struct ReasoningTokensParams {
    /// Enable reasoning tokens
    #[schema(default = true)]
    pub enabled: Option<bool>,

    /// Maximum reasoning tokens to use (for internal reasoning)
    #[schema(minimum = 1000, maximum = 100000)]
    pub max_reasoning_tokens: Option<u32>,
}

/// Thinking Level Parameters (Gemini)
///
/// Controls the depth of thinking for Google's Gemini models.
/// Higher levels result in more thorough reasoning but take longer.
///
/// **Provider**: Google (Gemini models)
///
/// **Levels**:
/// - 1: Quick, surface-level reasoning
/// - 2: Standard reasoning (default)
/// - 3: Deep reasoning for complex problems
///
/// **Use Cases**:
/// - Adjust based on problem complexity
/// - Balance speed vs thoroughness
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(
    title = "Thinking Level Parameters",
    description = "Gemini thinking depth control",
    example = json!({
        "level": 2
    })
)]
pub struct ThinkingLevelParams {
    /// Thinking depth level: 1=quick, 2=standard, 3=deep
    #[schema(minimum = 1, maximum = 3, default = 2)]
    pub level: Option<u8>,
}

/// Structured Outputs Parameters
///
/// Enforces JSON schema validation on model outputs.
/// Guarantees that the response matches your specified schema.
///
/// **Providers**: OpenAI, Anthropic, Google
///
/// **Use Cases**:
/// - API response generation
/// - Data extraction
/// - Form filling
/// - Structured data generation
///
/// **Benefits**:
/// - Type-safe outputs
/// - No parsing errors
/// - Consistent format
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(
    title = "Structured Outputs Parameters",
    description = "JSON schema validation for model outputs",
    example = json!({
        "enabled": true,
        "schema": {
            "type": "object",
            "properties": {
                "name": {"type": "string"},
                "age": {"type": "number"}
            },
            "required": ["name", "age"]
        },
        "strict": true
    })
)]
pub struct StructuredOutputsParams {
    /// Enable structured outputs (enforce schema validation)
    #[schema(default = true)]
    pub enabled: Option<bool>,

    /// JSON schema definition for output structure
    pub schema: Option<serde_json::Value>,

    /// Strict mode: strictly enforce schema (true) or allow best-effort (false)
    #[schema(default = true)]
    pub strict: Option<bool>,
}

/// Prompt Caching Parameters
///
/// Caches parts of your prompt to reduce costs and latency on repeated requests.
/// Can save 50-90% on costs for prompts with repeated content.
///
/// **Providers**: Anthropic, OpenAI
///
/// **Use Cases**:
/// - Large system prompts
/// - Repeated context (e.g., documentation)
/// - Multi-turn conversations
/// - Few-shot examples
///
/// **Savings**: 50-90% cost reduction on cached tokens
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(
    title = "Prompt Caching Parameters",
    description = "Cache prompt content for cost optimization (50-90% savings)",
    example = json!({
        "enabled": true,
        "cache_control": {
            "type": "ephemeral"
        }
    })
)]
pub struct PromptCachingParams {
    /// Enable prompt caching
    #[schema(default = true)]
    pub enabled: Option<bool>,

    /// Cache control settings (provider-specific)
    pub cache_control: Option<CacheControl>,
}

/// Cache Control Configuration
///
/// Specifies how prompt caching should behave.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(
    title = "Cache Control",
    description = "Cache control configuration",
    example = json!({
        "type": "ephemeral"
    })
)]
pub struct CacheControl {
    /// Cache type (ephemeral for temporary cache)
    pub r#type: String,
}

/// Logprobs Parameters
///
/// Returns token-level probability information for the generated response.
/// Useful for understanding model confidence and alternative tokens.
///
/// **Providers**: OpenAI, Anthropic
///
/// **Use Cases**:
/// - Confidence scoring
/// - Alternative generation paths
/// - Model debugging
/// - Uncertainty quantification
///
/// **Output**: Top N alternative tokens with probabilities for each position
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(
    title = "Logprobs Parameters",
    description = "Token probability information extraction",
    example = json!({
        "enabled": true,
        "top_logprobs": 5
    })
)]
pub struct LogprobsParams {
    /// Enable logprobs (include logprobs in response)
    #[schema(default = true)]
    pub enabled: Option<bool>,

    /// Number of top alternative tokens with probabilities
    #[schema(minimum = 0, maximum = 20, default = 5)]
    pub top_logprobs: Option<u8>,
}

/// JSON Mode Parameters
///
/// Lightweight JSON validation that encourages JSON output without strict schema enforcement.
/// Less strict than structured_outputs but more reliable than plain text.
///
/// **Providers**: OpenAI, Anthropic, Google, Anthropic
///
/// **Use Cases**:
/// - Flexible JSON generation
/// - When exact schema isn't critical
/// - Quick prototyping
///
/// **Difference from structured_outputs**:
/// - json_mode: Best-effort JSON (may not match exact schema)
/// - structured_outputs: Guaranteed schema compliance
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(
    title = "JSON Mode Parameters",
    description = "Lightweight JSON output validation",
    example = json!({
        "enabled": true
    })
)]
pub struct JsonModeParams {
    /// Enable JSON mode
    #[schema(default = true)]
    pub enabled: Option<bool>,
}

/// Feature Adapter Extensions
///
/// Container for all feature adapter parameters.
/// Include this in your ChatCompletionRequest's `extensions` field.
///
/// **Example Usage**:
/// ```json
/// {
///   "model": "claude-3-5-sonnet-20241022",
///   "messages": [...],
///   "extensions": {
///     "extended_thinking": {
///       "enabled": true,
///       "budget_tokens": 5000
///     },
///     "prompt_caching": {
///       "enabled": true,
///       "cache_control": {"type": "ephemeral"}
///     }
///   }
/// }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(
    title = "Feature Adapter Extensions",
    description = "Advanced provider-specific features",
    example = json!({
        "extended_thinking": {
            "enabled": true,
            "budget_tokens": 5000
        },
        "prompt_caching": {
            "enabled": true,
            "cache_control": {"type": "ephemeral"}
        },
        "logprobs": {
            "enabled": true,
            "top_logprobs": 5
        }
    })
)]
pub struct FeatureAdapterExtensions {
    /// Anthropic extended thinking mode
    pub extended_thinking: Option<ExtendedThinkingParams>,

    /// OpenAI o1 reasoning tokens
    pub reasoning_tokens: Option<ReasoningTokensParams>,

    /// Gemini thinking depth control
    pub thinking_level: Option<ThinkingLevelParams>,

    /// JSON schema validation
    pub structured_outputs: Option<StructuredOutputsParams>,

    /// Prompt caching for cost optimization
    pub prompt_caching: Option<PromptCachingParams>,

    /// Token probability extraction
    pub logprobs: Option<LogprobsParams>,

    /// Lightweight JSON validation
    pub json_mode: Option<JsonModeParams>,
}
