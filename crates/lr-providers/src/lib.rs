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

use lr_types::{AppError, AppResult};

// ==================== FEATURE SUPPORT MATRIX TYPES ====================

/// Level of support for a feature or endpoint.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SupportLevel {
    /// Full native support
    Supported,
    /// Some models/configurations only
    Partial,
    /// Supported via LocalRouter translation layer
    Translated,
    /// Not available for this provider
    NotSupported,
    /// Planned but not yet implemented
    NotImplemented,
}

/// Support information for a single API endpoint.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct EndpointSupport {
    /// Human-readable endpoint name (e.g., "Chat Completions")
    pub name: String,
    /// API path (e.g., "/v1/chat/completions")
    pub endpoint: String,
    /// Level of support
    pub support: SupportLevel,
    /// Hover tooltip text
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

/// Support information for a single feature.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct FeatureSupport {
    /// Feature name (e.g., "Guardrails", "Extended Thinking")
    pub name: String,
    /// Level of support
    pub support: SupportLevel,
    /// Hover tooltip text
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

/// Complete feature support information for a provider.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ProviderFeatureSupport {
    /// Provider type identifier (e.g., "openai")
    pub provider_type: String,
    /// Provider instance name (e.g., "My OpenAI")
    pub provider_instance: String,
    /// API endpoint support
    pub endpoints: Vec<EndpointSupport>,
    /// Model feature support
    pub model_features: Vec<FeatureSupport>,
    /// Optimization feature support
    pub optimization_features: Vec<FeatureSupport>,
}

/// A cell in the feature-endpoint matrix.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatrixCell {
    pub support: SupportLevel,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

/// A row in the feature × endpoint matrix.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeatureEndpointRow {
    pub feature_name: String,
    pub cells: Vec<MatrixCell>,
}

/// A row in the feature/endpoint × client mode matrix.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeatureModeRow {
    pub name: String,
    pub cells: Vec<MatrixCell>,
}

/// Static matrix showing which optimization features apply to which endpoints and client modes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeatureEndpointMatrix {
    /// Column headers for the feature × endpoint table
    pub endpoints: Vec<String>,
    /// Column headers for the client mode table
    pub client_modes: Vec<String>,
    /// Feature × endpoint rows
    pub feature_rows: Vec<FeatureEndpointRow>,
    /// Feature/endpoint × client mode rows
    pub mode_rows: Vec<FeatureModeRow>,
}

// ==================== MODEL PULL TYPES ====================

/// Progress event for model pull/download operations.
///
/// Used by providers that support downloading models on-demand (Ollama, LM Studio, LocalAI).
/// Emitted as a stream of events during the pull process.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PullProgress {
    pub status: String,
    #[serde(default)]
    pub total: Option<u64>,
    #[serde(default)]
    pub completed: Option<u64>,
}

pub mod anthropic;
pub mod cerebras;
pub mod cohere;
pub mod deepinfra;
pub mod factory;
pub mod features;
pub mod gemini;
pub mod gpt4all;
pub mod groq;
pub mod health;
pub mod health_cache;
pub mod http_client;
pub mod jan;
pub mod key_storage;
pub mod llamacpp;
pub mod lmstudio;
pub mod localai;
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

    /// Generate images from text prompts
    ///
    /// Used by: POST /v1/images/generations endpoint
    ///
    /// Default implementation returns an error indicating image generation is not supported.
    /// Providers that support image generation (e.g., OpenAI with DALL-E) should override this method.
    async fn generate_image(
        &self,
        _request: ImageGenerationRequest,
    ) -> AppResult<ImageGenerationResponse> {
        Err(AppError::Provider(format!(
            "Provider '{}' does not support image generation",
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
    ) -> Option<Box<dyn crate::features::FeatureAdapter>> {
        None
    }

    /// Check remaining credits/balance with this provider's API (if supported).
    ///
    /// Only providers with credit-check APIs implement this (e.g., OpenRouter).
    /// Default implementation returns None, meaning no API-based credit checking.
    async fn check_credits(&self) -> Option<ProviderCreditsInfo> {
        None
    }

    /// Whether this provider supports embeddings.
    /// Default: false. Override to true in providers that implement embed().
    fn supports_embeddings(&self) -> bool {
        false
    }

    /// Whether this provider supports image generation.
    /// Default: false. Override to true in providers that implement generate_image().
    fn supports_image_generation(&self) -> bool {
        false
    }

    /// Transcribe audio to text (Speech-to-Text)
    ///
    /// Used by: POST /v1/audio/transcriptions endpoint
    ///
    /// Default implementation returns an error indicating transcription is not supported.
    /// Providers that support STT should override this method.
    async fn transcribe(
        &self,
        _request: AudioTranscriptionRequest,
    ) -> AppResult<AudioTranscriptionResponse> {
        Err(AppError::Provider(format!(
            "Provider '{}' does not support audio transcription",
            self.name()
        )))
    }

    /// Translate audio to English text (Speech-to-Text translation)
    ///
    /// Used by: POST /v1/audio/translations endpoint
    ///
    /// Default implementation returns an error indicating audio translation is not supported.
    /// Providers that support STT translation should override this method.
    async fn translate_audio(
        &self,
        _request: AudioTranslationRequest,
    ) -> AppResult<AudioTranslationResponse> {
        Err(AppError::Provider(format!(
            "Provider '{}' does not support audio translation",
            self.name()
        )))
    }

    /// Generate speech from text (Text-to-Speech)
    ///
    /// Used by: POST /v1/audio/speech endpoint
    ///
    /// Default implementation returns an error indicating TTS is not supported.
    /// Providers that support TTS should override this method.
    async fn speech(&self, _request: SpeechRequest) -> AppResult<SpeechResponse> {
        Err(AppError::Provider(format!(
            "Provider '{}' does not support text-to-speech",
            self.name()
        )))
    }

    /// Whether this provider supports audio transcription (STT).
    /// Default: false. Override to true in providers that implement transcribe().
    fn supports_transcription(&self) -> bool {
        false
    }

    /// Whether this provider supports audio translation.
    /// Default: false. Override to true in providers that implement translate_audio().
    fn supports_audio_translation(&self) -> bool {
        false
    }

    /// Whether this provider supports text-to-speech.
    /// Default: false. Override to true in providers that implement speech().
    fn supports_speech(&self) -> bool {
        false
    }

    /// Returns feature support information for this provider.
    /// Default calls `default_feature_support()`. Override to customize.
    fn get_feature_support(&self, instance_name: &str) -> ProviderFeatureSupport {
        default_feature_support(self, instance_name)
    }

    /// Whether this provider supports pulling (downloading) models on demand.
    ///
    /// Providers that return true must also implement `pull_model()`.
    /// Currently supported by: Ollama, LM Studio, LocalAI.
    fn supports_pull(&self) -> bool {
        false
    }

    /// Pull (download) a model. Returns a stream of progress events.
    ///
    /// The stream should emit `PullProgress` items and end with status `"success"` on completion.
    /// Default implementation returns an error for providers that don't support pulling.
    async fn pull_model(
        &self,
        _model_name: &str,
    ) -> AppResult<Pin<Box<dyn Stream<Item = AppResult<PullProgress>> + Send>>> {
        Err(AppError::Provider(format!(
            "Provider '{}' does not support model pulling",
            self.name()
        )))
    }
}

/// Default feature support builder. Used by the trait default and called by
/// providers that want to start from the default and then customize.
pub fn default_feature_support(
    provider: &(impl ModelProvider + ?Sized),
    instance_name: &str,
) -> ProviderFeatureSupport {
    let has_chat = true;
    let has_embeddings = provider.supports_embeddings();
    let has_images = provider.supports_image_generation();

    let endpoints = vec![
        EndpointSupport {
            name: "Chat Completions".into(),
            endpoint: "/v1/chat/completions".into(),
            support: SupportLevel::Supported,
            notes: Some("Send messages and receive AI responses".into()),
        },
        EndpointSupport {
            name: "Completions (legacy)".into(),
            endpoint: "/v1/completions".into(),
            support: SupportLevel::Supported,
            notes: Some("Converted to chat completions internally by LocalRouter".into()),
        },
        EndpointSupport {
            name: "Streaming".into(),
            endpoint: "/v1/chat/completions".into(),
            support: SupportLevel::Supported,
            notes: Some("Server-sent events for real-time token streaming".into()),
        },
        EndpointSupport {
            name: "Embeddings".into(),
            endpoint: "/v1/embeddings".into(),
            support: if has_embeddings {
                SupportLevel::Supported
            } else {
                SupportLevel::NotSupported
            },
            notes: Some(if has_embeddings {
                "Generate vector embeddings for text".into()
            } else {
                "This provider does not offer an embeddings API".into()
            }),
        },
        EndpointSupport {
            name: "Image Generation".into(),
            endpoint: "/v1/images/generations".into(),
            support: if has_images {
                SupportLevel::Supported
            } else {
                SupportLevel::NotSupported
            },
            notes: Some(if has_images {
                "Generate images from text prompts".into()
            } else {
                "This provider does not offer image generation".into()
            }),
        },
        EndpointSupport {
            name: "Audio Transcription".into(),
            endpoint: "/v1/audio/transcriptions".into(),
            support: if provider.supports_transcription() {
                SupportLevel::Supported
            } else {
                SupportLevel::NotImplemented
            },
            notes: Some(if provider.supports_transcription() {
                "Speech-to-text transcription (e.g., Whisper)".into()
            } else {
                "Audio transcription endpoint not yet available in LocalRouter".into()
            }),
        },
        EndpointSupport {
            name: "Audio Speech (TTS)".into(),
            endpoint: "/v1/audio/speech".into(),
            support: if provider.supports_speech() {
                SupportLevel::Supported
            } else {
                SupportLevel::NotImplemented
            },
            notes: Some(if provider.supports_speech() {
                "Text-to-speech audio generation".into()
            } else {
                "Text-to-speech endpoint not yet available in LocalRouter".into()
            }),
        },
        EndpointSupport {
            name: "Moderations".into(),
            endpoint: "/v1/moderations".into(),
            support: SupportLevel::NotImplemented,
            notes: Some("Content moderation endpoint not yet available in LocalRouter".into()),
        },
        EndpointSupport {
            name: "Responses API".into(),
            endpoint: "/v1/responses".into(),
            support: SupportLevel::NotImplemented,
            notes: Some("OpenAI Responses API not yet available in LocalRouter".into()),
        },
        EndpointSupport {
            name: "Batch Processing".into(),
            endpoint: "/v1/batches".into(),
            support: SupportLevel::NotImplemented,
            notes: Some("Asynchronous batch processing not yet available in LocalRouter".into()),
        },
        EndpointSupport {
            name: "Realtime (WebSocket)".into(),
            endpoint: "/v1/realtime".into(),
            support: SupportLevel::NotImplemented,
            notes: Some(
                "WebSocket-based real-time audio/text streaming not yet available in LocalRouter"
                    .into(),
            ),
        },
    ];

    let model_features = vec![
        FeatureSupport {
            name: "Function Calling".into(),
            support: if provider.supports_feature("function_calling") {
                SupportLevel::Supported
            } else {
                SupportLevel::NotSupported
            },
            notes: Some(if provider.supports_feature("function_calling") {
                "Models can call functions/tools defined in the request".into()
            } else {
                "This provider's models do not support function/tool calling".into()
            }),
        },
        FeatureSupport {
            name: "Vision".into(),
            support: if provider.supports_feature("vision") {
                SupportLevel::Supported
            } else {
                SupportLevel::NotSupported
            },
            notes: Some(if provider.supports_feature("vision") {
                "Models can process images alongside text".into()
            } else {
                "This provider's models do not accept image inputs".into()
            }),
        },
        FeatureSupport {
            name: "Structured Outputs".into(),
            support: if provider.supports_feature("structured_outputs") {
                SupportLevel::Supported
            } else {
                SupportLevel::NotSupported
            },
            notes: Some(if provider.supports_feature("structured_outputs") {
                "Enforce JSON schema compliance in model responses".into()
            } else {
                "This provider does not support strict JSON schema enforcement".into()
            }),
        },
        FeatureSupport {
            name: "JSON Mode".into(),
            support: if provider.get_feature_adapter("json_mode").is_some() {
                SupportLevel::Supported
            } else {
                SupportLevel::NotSupported
            },
            notes: Some(if provider.get_feature_adapter("json_mode").is_some() {
                "Force model to output valid JSON".into()
            } else {
                "This provider does not support JSON-only output mode".into()
            }),
        },
        FeatureSupport {
            name: "Log Probabilities".into(),
            support: if provider.supports_feature("logprobs") {
                SupportLevel::Supported
            } else {
                SupportLevel::NotSupported
            },
            notes: Some(if provider.supports_feature("logprobs") {
                "Return per-token log probabilities with responses".into()
            } else {
                "This provider does not expose token log probabilities".into()
            }),
        },
        FeatureSupport {
            name: "Reasoning Tokens".into(),
            support: if provider.supports_feature("reasoning_tokens") {
                SupportLevel::Supported
            } else {
                SupportLevel::NotSupported
            },
            notes: Some(if provider.supports_feature("reasoning_tokens") {
                "Models use internal reasoning tokens before answering".into()
            } else {
                "This provider does not support reasoning token models".into()
            }),
        },
        FeatureSupport {
            name: "Extended Thinking".into(),
            support: if provider.supports_feature("extended_thinking") {
                SupportLevel::Supported
            } else {
                SupportLevel::NotSupported
            },
            notes: Some(if provider.supports_feature("extended_thinking") {
                "Configurable thinking budget for deep reasoning".into()
            } else {
                "This provider does not support extended thinking".into()
            }),
        },
        FeatureSupport {
            name: "Thinking Level".into(),
            support: if provider.supports_feature("thinking_level") {
                SupportLevel::Supported
            } else {
                SupportLevel::NotSupported
            },
            notes: Some(if provider.supports_feature("thinking_level") {
                "Adjustable thinking intensity (low/medium/high)".into()
            } else {
                "This provider does not support thinking level control".into()
            }),
        },
        FeatureSupport {
            name: "Prompt Caching".into(),
            support: if provider.supports_feature("prompt_caching") {
                SupportLevel::Supported
            } else {
                SupportLevel::NotSupported
            },
            notes: Some(if provider.supports_feature("prompt_caching") {
                "Cache repeated prompt prefixes for faster/cheaper requests".into()
            } else {
                "This provider does not support server-side prompt caching".into()
            }),
        },
        // Chat completion parameter support
        FeatureSupport {
            name: "N Completions".into(),
            support: SupportLevel::NotSupported,
            notes: Some("Generate multiple completion choices per request (n parameter)".into()),
        },
        FeatureSupport {
            name: "Logit Bias".into(),
            support: SupportLevel::NotSupported,
            notes: Some("Modify token likelihoods by token ID".into()),
        },
        FeatureSupport {
            name: "Parallel Tool Calls".into(),
            support: SupportLevel::NotSupported,
            notes: Some("Allow model to make multiple tool calls concurrently".into()),
        },
        FeatureSupport {
            name: "Reasoning Effort".into(),
            support: SupportLevel::NotSupported,
            notes: Some(
                "Control reasoning intensity (low/medium/high) for reasoning models".into(),
            ),
        },
        FeatureSupport {
            name: "Predicted Output".into(),
            support: SupportLevel::NotSupported,
            notes: Some(
                "Supply predicted output for faster generation (speculative decoding)".into(),
            ),
        },
        FeatureSupport {
            name: "Service Tier".into(),
            support: SupportLevel::NotSupported,
            notes: Some("Select latency tier for request processing".into()),
        },
        FeatureSupport {
            name: "Audio Output".into(),
            support: SupportLevel::NotSupported,
            notes: Some("Generate audio alongside text via modalities/audio parameters".into()),
        },
    ];

    let optimization_features = vec![
        FeatureSupport {
            name: "Guardrails".into(),
            support: if has_chat {
                SupportLevel::Supported
            } else {
                SupportLevel::NotSupported
            },
            notes: Some("Content safety scanning on chat/completion requests".into()),
        },
        FeatureSupport {
            name: "Prompt Compression".into(),
            support: if has_chat {
                SupportLevel::Supported
            } else {
                SupportLevel::NotSupported
            },
            notes: Some("LLMLingua-2 token-level compression for chat requests".into()),
        },
        FeatureSupport {
            name: "JSON Repair".into(),
            support: if has_chat {
                SupportLevel::Supported
            } else {
                SupportLevel::NotSupported
            },
            notes: Some("Automatic fix of malformed JSON responses".into()),
        },
        FeatureSupport {
            name: "RouteLLM Routing".into(),
            support: if has_chat {
                SupportLevel::Supported
            } else {
                SupportLevel::NotSupported
            },
            notes: Some("Strong/weak model routing based on request complexity".into()),
        },
        FeatureSupport {
            name: "Secret Scanning".into(),
            support: if has_chat {
                SupportLevel::Supported
            } else {
                SupportLevel::NotSupported
            },
            notes: Some("Detect potential secrets in outbound requests".into()),
        },
        FeatureSupport {
            name: "Rate Limiting".into(),
            support: SupportLevel::Supported,
            notes: Some("Available for all endpoints".into()),
        },
        FeatureSupport {
            name: "Model Firewall".into(),
            support: SupportLevel::Supported,
            notes: Some("Available for all LLM endpoints".into()),
        },
        FeatureSupport {
            name: "Generation Tracking".into(),
            support: SupportLevel::Supported,
            notes: Some("Available for all endpoints".into()),
        },
        FeatureSupport {
            name: "Cost Calculation".into(),
            support: SupportLevel::Supported,
            notes: Some("Based on catalog pricing data".into()),
        },
    ];

    ProviderFeatureSupport {
        provider_type: provider.name().to_string(),
        provider_instance: instance_name.to_string(),
        endpoints,
        model_features,
        optimization_features,
    }
}

/// Credit/balance information returned by a provider's API
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderCreditsInfo {
    /// Total credits in USD (if known)
    pub total_credits_usd: Option<f64>,
    /// Credits used in USD (if known)
    pub used_credits_usd: Option<f64>,
    /// Credits remaining in USD (if known)
    pub remaining_credits_usd: Option<f64>,
    /// Whether the key is on the provider's free tier
    pub is_free_tier: Option<bool>,
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
    /// models.dev catalog to enhance model information with accurate pricing
    /// and capability data.
    ///
    /// # Arguments
    /// * `provider_type` - Provider identifier for matching (e.g., "openai", "anthropic")
    ///
    /// # Returns
    /// Self with potentially updated context_window and capabilities
    pub fn enrich_with_catalog(mut self, provider_type: &str) -> Self {
        use lr_catalog as catalog;

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
        use lr_catalog as catalog;

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
    Audio,
    TextToSpeech,
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

/// Pre-computed RouteLLM classification result.
/// Set by the chat pipeline before routing, so the router can skip classification.
#[derive(Debug, Clone)]
pub struct PreComputedRouting {
    pub is_strong: bool,
    pub win_rate: f32,
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

    // Additional OpenAI-compatible parameters (pass-through)
    /// Number of completions to generate (default: 1)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub n: Option<u32>,
    /// Modify token likelihoods by token ID
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logit_bias: Option<std::collections::HashMap<String, f32>>,
    /// Allow concurrent function calling
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parallel_tool_calls: Option<bool>,
    /// Latency tier selection ("auto", "default")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_tier: Option<String>,
    /// Store for distillation/evaluation
    #[serde(skip_serializing_if = "Option::is_none")]
    pub store: Option<bool>,
    /// Developer-defined tags
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<std::collections::HashMap<String, String>>,
    /// Output modalities: ["text"], ["text", "audio"]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modalities: Option<Vec<String>>,
    /// Audio output configuration (voice, format)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audio: Option<serde_json::Value>,
    /// Predicted output for faster generation
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prediction: Option<serde_json::Value>,
    /// Reasoning effort level (low/medium/high) for reasoning models
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,

    /// Pre-computed RouteLLM routing (set by chat pipeline, never serialized)
    #[serde(skip)]
    pub pre_computed_routing: Option<PreComputedRouting>,
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
            ChatMessageContent::Parts(parts) => parts
                .iter()
                .filter_map(|part| match part {
                    ContentPart::Text { text } => Some(text.clone()),
                    ContentPart::ImageUrl { .. } => None,
                })
                .collect::<Vec<_>>()
                .join("\n"),
        }
    }

    /// Check if this message contains any images
    pub fn has_images(&self) -> bool {
        match self {
            ChatMessageContent::Text(_) => false,
            ChatMessageContent::Parts(parts) => parts
                .iter()
                .any(|part| matches!(part, ContentPart::ImageUrl { .. })),
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
    /// Model version identifier
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_fingerprint: Option<String>,
    /// Tier used for request
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_tier: Option<String>,
    /// Provider-specific extensions (Phase 3)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extensions: Option<std::collections::HashMap<String, serde_json::Value>>,
    /// RouteLLM win rate (0.0-1.0) if RouteLLM routing was used
    #[serde(skip_serializing_if = "Option::is_none")]
    pub routellm_win_rate: Option<f32>,
    /// Per-iteration token usage breakdown (only present when multiple LLM calls were made)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_usage_entries: Option<Vec<TokenUsage>>,
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

// ==================== Image Generation ====================

/// Image generation request
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ImageGenerationRequest {
    /// The model to use (e.g., "dall-e-3", "dall-e-2")
    pub model: String,
    /// The text prompt describing the image to generate
    pub prompt: String,
    /// Number of images to generate (1-10)
    #[serde(default = "default_image_n")]
    pub n: Option<u32>,
    /// Size of generated images (e.g., "1024x1024")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<String>,
    /// Quality level ("standard" or "hd", DALL-E 3 only)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quality: Option<String>,
    /// Style ("vivid" or "natural", DALL-E 3 only)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub style: Option<String>,
    /// Response format ("url" or "b64_json")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_format: Option<String>,
    /// User identifier for tracking
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
}

fn default_image_n() -> Option<u32> {
    Some(1)
}

/// Image generation response
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ImageGenerationResponse {
    /// Unix timestamp of creation
    pub created: i64,
    /// Array of generated images
    pub data: Vec<GeneratedImage>,
}

/// Single generated image
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct GeneratedImage {
    /// URL of the generated image (if response_format is "url")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Base64-encoded image data (if response_format is "b64_json")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub b64_json: Option<String>,
    /// The revised prompt (DALL-E 3 may modify the prompt)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revised_prompt: Option<String>,
}

// ==================== Audio Types ====================

/// Audio transcription request (Speech-to-Text)
#[derive(Debug, Clone)]
pub struct AudioTranscriptionRequest {
    /// The audio file data
    pub file: Vec<u8>,
    /// Original filename of the audio file
    pub file_name: String,
    /// The model to use (e.g., "whisper-1")
    pub model: String,
    /// The language of the audio in ISO-639-1 format
    pub language: Option<String>,
    /// Optional prompt to guide the model's style
    pub prompt: Option<String>,
    /// Response format: json, text, srt, verbose_json, vtt
    pub response_format: Option<String>,
    /// Sampling temperature (0 to 1)
    pub temperature: Option<f32>,
    /// Timestamp granularities: word, segment
    pub timestamp_granularities: Option<Vec<String>>,
}

/// Audio transcription response
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AudioTranscriptionResponse {
    /// The transcribed text
    pub text: String,
    /// The task type (always "transcribe")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
    /// The detected language
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// The duration of the audio in seconds
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration: Option<f64>,
    /// Word-level timestamps (when requested)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub words: Option<Vec<TranscriptionWord>>,
    /// Segment-level timestamps (when requested)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub segments: Option<Vec<TranscriptionSegment>>,
}

/// Word with timestamp from transcription
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TranscriptionWord {
    pub word: String,
    pub start: f64,
    pub end: f64,
}

/// Segment with timestamp from transcription
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TranscriptionSegment {
    pub id: u32,
    pub seek: u32,
    pub start: f64,
    pub end: f64,
    pub text: String,
    pub tokens: Vec<u32>,
    pub temperature: f64,
    pub avg_logprob: f64,
    pub compression_ratio: f64,
    pub no_speech_prob: f64,
}

/// Audio translation request (Speech-to-English)
#[derive(Debug, Clone)]
pub struct AudioTranslationRequest {
    /// The audio file data
    pub file: Vec<u8>,
    /// Original filename of the audio file
    pub file_name: String,
    /// The model to use (e.g., "whisper-1")
    pub model: String,
    /// Optional prompt to guide the model's style
    pub prompt: Option<String>,
    /// Response format: json, text, srt, verbose_json, vtt
    pub response_format: Option<String>,
    /// Sampling temperature (0 to 1)
    pub temperature: Option<f32>,
}

/// Audio translation response (same structure as transcription)
pub type AudioTranslationResponse = AudioTranscriptionResponse;

/// Text-to-Speech request
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SpeechRequest {
    /// The model to use (e.g., "tts-1", "tts-1-hd")
    pub model: String,
    /// The text to synthesize (max 4096 characters)
    pub input: String,
    /// The voice to use (alloy, echo, fable, onyx, nova, shimmer)
    pub voice: String,
    /// Audio output format: mp3, opus, aac, flac, wav, pcm
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_format: Option<String>,
    /// Speech speed (0.25 to 4.0)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speed: Option<f64>,
}

/// Text-to-Speech response (binary audio data)
pub struct SpeechResponse {
    /// The raw audio data
    pub audio_data: Vec<u8>,
    /// The content type (e.g., "audio/mpeg", "audio/opus")
    pub content_type: String,
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

    // ==================== Audio Type Tests ====================

    #[test]
    fn test_audio_transcription_response_serialization() {
        let response = AudioTranscriptionResponse {
            text: "Hello world".to_string(),
            task: Some("transcribe".to_string()),
            language: Some("en".to_string()),
            duration: Some(1.5),
            words: None,
            segments: None,
        };
        let json = serde_json::to_value(&response).unwrap();
        assert_eq!(json["text"], "Hello world");
        assert_eq!(json["task"], "transcribe");
        assert_eq!(json["language"], "en");
        assert_eq!(json["duration"], 1.5);
    }

    #[test]
    fn test_audio_transcription_response_deserialization() {
        let json = r#"{"text": "test", "task": "transcribe", "language": "en", "duration": 2.0}"#;
        let response: AudioTranscriptionResponse = serde_json::from_str(json).unwrap();
        assert_eq!(response.text, "test");
        assert_eq!(response.task.as_deref(), Some("transcribe"));
        assert_eq!(response.duration, Some(2.0));
    }

    #[test]
    fn test_audio_transcription_response_minimal_deserialization() {
        let json = r#"{"text": "minimal"}"#;
        let response: AudioTranscriptionResponse = serde_json::from_str(json).unwrap();
        assert_eq!(response.text, "minimal");
        assert!(response.task.is_none());
        assert!(response.language.is_none());
        assert!(response.duration.is_none());
        assert!(response.words.is_none());
        assert!(response.segments.is_none());
    }

    #[test]
    fn test_transcription_word_roundtrip() {
        let word = TranscriptionWord {
            word: "hello".to_string(),
            start: 0.5,
            end: 1.0,
        };
        let json = serde_json::to_string(&word).unwrap();
        let parsed: TranscriptionWord = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.word, "hello");
        assert_eq!(parsed.start, 0.5);
        assert_eq!(parsed.end, 1.0);
    }

    #[test]
    fn test_transcription_segment_roundtrip() {
        let segment = TranscriptionSegment {
            id: 0,
            seek: 0,
            start: 0.0,
            end: 5.0,
            text: "Test".to_string(),
            tokens: vec![1, 2, 3],
            temperature: 0.0,
            avg_logprob: -0.5,
            compression_ratio: 1.2,
            no_speech_prob: 0.01,
        };
        let json = serde_json::to_string(&segment).unwrap();
        let parsed: TranscriptionSegment = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.text, "Test");
        assert_eq!(parsed.tokens.len(), 3);
        assert_eq!(parsed.no_speech_prob, 0.01);
    }

    #[test]
    fn test_speech_request_serialization() {
        let request = SpeechRequest {
            model: "tts-1".to_string(),
            input: "Test".to_string(),
            voice: "alloy".to_string(),
            response_format: Some("opus".to_string()),
            speed: Some(1.5),
        };
        let json = serde_json::to_value(&request).unwrap();
        assert_eq!(json["model"], "tts-1");
        assert_eq!(json["input"], "Test");
        assert_eq!(json["voice"], "alloy");
        assert_eq!(json["response_format"], "opus");
        assert_eq!(json["speed"], 1.5);
    }

    #[test]
    fn test_speech_request_optional_omission() {
        let request = SpeechRequest {
            model: "tts-1".to_string(),
            input: "Test".to_string(),
            voice: "alloy".to_string(),
            response_format: None,
            speed: None,
        };
        let json = serde_json::to_value(&request).unwrap();
        assert!(json.get("response_format").is_none());
        assert!(json.get("speed").is_none());
    }

    #[test]
    fn test_capability_enum_has_audio_variants() {
        let audio = Capability::Audio;
        let tts = Capability::TextToSpeech;
        // Verify they serialize correctly
        let json_audio = serde_json::to_value(&audio).unwrap();
        let json_tts = serde_json::to_value(&tts).unwrap();
        assert_eq!(json_audio, "Audio");
        assert_eq!(json_tts, "TextToSpeech");
    }

    #[test]
    fn test_audio_translation_response_is_same_as_transcription() {
        // AudioTranslationResponse is a type alias for AudioTranscriptionResponse
        let response: AudioTranslationResponse =
            serde_json::from_str(r#"{"text": "Hello in English", "language": "en"}"#).unwrap();
        assert_eq!(response.text, "Hello in English");
    }
}

// ==================== FEATURE ENDPOINT MATRIX BUILDER ====================

/// Helper to create a MatrixCell.
fn cell(support: SupportLevel, notes: Option<&str>) -> MatrixCell {
    MatrixCell {
        support,
        notes: notes.map(|s| s.to_string()),
    }
}

/// Build the static feature × endpoint × client mode matrix.
/// This is hardcoded data that doesn't change per provider.
pub fn build_feature_endpoint_matrix() -> FeatureEndpointMatrix {
    use SupportLevel::*;

    let endpoints = vec![
        "Chat".into(),
        "Completions".into(),
        "Embeddings".into(),
        "Images".into(),
        "Audio".into(),
        "Moderations".into(),
        "Responses".into(),
        "Batches".into(),
        "Realtime".into(),
    ];

    let client_modes = vec![
        "LLM Only".into(),
        "MCP Only".into(),
        "MCP & LLM".into(),
        "MCP via LLM".into(),
    ];

    //                          Chat       Compl      Embed      Images     Audio      Mod        Resp       Batches    Realtime
    let feature_rows = vec![
        FeatureEndpointRow {
            feature_name: "Guardrails".into(),
            cells: vec![
                cell(Supported, None),
                cell(Supported, None),
                cell(NotSupported, None),
                cell(NotSupported, None),
                cell(NotSupported, None),
                cell(NotSupported, None),
                cell(Translated, Some("Via translation to chat completions")),
                cell(Translated, Some("Per-request in translated batch mode")),
                cell(NotSupported, None),
            ],
        },
        FeatureEndpointRow {
            feature_name: "Prompt Compression".into(),
            cells: vec![
                cell(Supported, None),
                cell(NotSupported, None),
                cell(NotSupported, None),
                cell(NotSupported, None),
                cell(NotSupported, None),
                cell(NotSupported, None),
                cell(Translated, Some("Via translation to chat completions")),
                cell(Translated, Some("Per-request in translated batch mode")),
                cell(NotSupported, None),
            ],
        },
        FeatureEndpointRow {
            feature_name: "JSON Repair".into(),
            cells: vec![
                cell(Supported, None),
                cell(Supported, None),
                cell(NotSupported, None),
                cell(NotSupported, None),
                cell(NotSupported, None),
                cell(NotSupported, None),
                cell(Translated, Some("Via translation to chat completions")),
                cell(NotSupported, None),
                cell(NotSupported, None),
            ],
        },
        FeatureEndpointRow {
            feature_name: "RouteLLM Routing".into(),
            cells: vec![
                cell(Supported, None),
                cell(NotSupported, None),
                cell(NotSupported, None),
                cell(NotSupported, None),
                cell(NotSupported, None),
                cell(NotSupported, None),
                cell(NotSupported, None),
                cell(NotSupported, None),
                cell(NotSupported, None),
            ],
        },
        FeatureEndpointRow {
            feature_name: "Secret Scanning".into(),
            cells: vec![
                cell(Supported, None),
                cell(Supported, None),
                cell(NotSupported, None),
                cell(NotSupported, None),
                cell(
                    Partial,
                    Some("TTS input text only; audio binary not scannable"),
                ),
                cell(NotSupported, None),
                cell(Translated, Some("Via translation to chat completions")),
                cell(Translated, Some("Per-request in translated batch mode")),
                cell(NotSupported, None),
            ],
        },
        FeatureEndpointRow {
            feature_name: "Rate Limiting".into(),
            cells: vec![
                cell(Supported, None),
                cell(Supported, None),
                cell(Supported, None),
                cell(Supported, None),
                cell(Supported, None),
                cell(Supported, None),
                cell(Supported, None),
                cell(Supported, None),
                cell(Partial, Some("Connection-time only, no per-message")),
            ],
        },
        FeatureEndpointRow {
            feature_name: "Model Firewall".into(),
            cells: vec![
                cell(Supported, None),
                cell(Supported, None),
                cell(NotSupported, None),
                cell(NotSupported, None),
                cell(Supported, None),
                cell(NotSupported, None),
                cell(Supported, None),
                cell(Partial, Some("Approve at batch creation time")),
                cell(Supported, None),
            ],
        },
        FeatureEndpointRow {
            feature_name: "Generation Tracking".into(),
            cells: vec![
                cell(Supported, None),
                cell(Supported, None),
                cell(Supported, None),
                cell(Supported, None),
                cell(Supported, None),
                cell(Supported, None),
                cell(Supported, None),
                cell(Supported, None),
                cell(Partial, Some("Per-session aggregation")),
            ],
        },
        FeatureEndpointRow {
            feature_name: "Cost Calculation".into(),
            cells: vec![
                cell(Supported, None),
                cell(Supported, None),
                cell(Supported, None),
                cell(NotSupported, None),
                cell(Supported, None),
                cell(NotSupported, None),
                cell(Supported, None),
                cell(Supported, None),
                cell(Supported, None),
            ],
        },
    ];

    //                                        LLM Only   MCP Only   MCP&LLM    MCP via LLM
    let mode_rows = vec![
        FeatureModeRow {
            name: "Chat Completions".into(),
            cells: vec![
                cell(Supported, None),
                cell(NotSupported, None),
                cell(Supported, None),
                cell(Supported, None),
            ],
        },
        FeatureModeRow {
            name: "Completions".into(),
            cells: vec![
                cell(Supported, None),
                cell(NotSupported, None),
                cell(Supported, None),
                cell(NotSupported, None),
            ],
        },
        FeatureModeRow {
            name: "Embeddings".into(),
            cells: vec![
                cell(Supported, None),
                cell(NotSupported, None),
                cell(Supported, None),
                cell(NotSupported, None),
            ],
        },
        FeatureModeRow {
            name: "Image Generation".into(),
            cells: vec![
                cell(Supported, None),
                cell(NotSupported, None),
                cell(Supported, None),
                cell(NotSupported, None),
            ],
        },
        FeatureModeRow {
            name: "Audio (STT/TTS)".into(),
            cells: vec![
                cell(Supported, None),
                cell(NotSupported, None),
                cell(Supported, None),
                cell(NotSupported, None),
            ],
        },
        FeatureModeRow {
            name: "Moderations".into(),
            cells: vec![
                cell(Supported, None),
                cell(NotSupported, None),
                cell(Supported, None),
                cell(NotSupported, None),
            ],
        },
        FeatureModeRow {
            name: "Responses API".into(),
            cells: vec![
                cell(Supported, None),
                cell(NotSupported, None),
                cell(Supported, None),
                cell(NotSupported, None),
            ],
        },
        FeatureModeRow {
            name: "Batch Processing".into(),
            cells: vec![
                cell(Supported, None),
                cell(NotSupported, None),
                cell(Supported, None),
                cell(NotSupported, None),
            ],
        },
        FeatureModeRow {
            name: "Realtime".into(),
            cells: vec![
                cell(Supported, None),
                cell(NotSupported, None),
                cell(Supported, None),
                cell(NotSupported, None),
            ],
        },
        FeatureModeRow {
            name: "MCP Gateway".into(),
            cells: vec![
                cell(NotSupported, None),
                cell(Supported, None),
                cell(Supported, None),
                cell(NotSupported, None),
            ],
        },
        FeatureModeRow {
            name: "MCP WebSocket".into(),
            cells: vec![
                cell(NotSupported, None),
                cell(Supported, None),
                cell(Supported, None),
                cell(NotSupported, None),
            ],
        },
        FeatureModeRow {
            name: "MCP → LLM Tools".into(),
            cells: vec![
                cell(NotSupported, None),
                cell(NotSupported, None),
                cell(NotSupported, None),
                cell(Supported, None),
            ],
        },
        FeatureModeRow {
            name: "Guardrails".into(),
            cells: vec![
                cell(Supported, None),
                cell(NotSupported, None),
                cell(Supported, None),
                cell(Supported, None),
            ],
        },
        FeatureModeRow {
            name: "Prompt Compression".into(),
            cells: vec![
                cell(Supported, None),
                cell(NotSupported, None),
                cell(Supported, None),
                cell(Supported, None),
            ],
        },
        FeatureModeRow {
            name: "JSON Repair".into(),
            cells: vec![
                cell(Supported, None),
                cell(NotSupported, None),
                cell(Supported, None),
                cell(Supported, None),
            ],
        },
        FeatureModeRow {
            name: "RouteLLM".into(),
            cells: vec![
                cell(Supported, None),
                cell(NotSupported, None),
                cell(Supported, None),
                cell(Supported, None),
            ],
        },
        FeatureModeRow {
            name: "Secret Scanning".into(),
            cells: vec![
                cell(Supported, None),
                cell(NotSupported, None),
                cell(Supported, None),
                cell(Supported, None),
            ],
        },
        FeatureModeRow {
            name: "Context Management".into(),
            cells: vec![
                cell(NotSupported, None),
                cell(Supported, None),
                cell(Supported, None),
                cell(Supported, None),
            ],
        },
        FeatureModeRow {
            name: "Catalog Compression".into(),
            cells: vec![
                cell(NotSupported, None),
                cell(Supported, None),
                cell(Supported, None),
                cell(Supported, None),
            ],
        },
        FeatureModeRow {
            name: "Response RAG".into(),
            cells: vec![
                cell(NotSupported, None),
                cell(Supported, None),
                cell(Supported, None),
                cell(Supported, None),
            ],
        },
    ];

    FeatureEndpointMatrix {
        endpoints,
        client_modes,
        feature_rows,
        mode_rows,
    }
}
