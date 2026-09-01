//! OpenAI provider implementation

use super::{
    Capability, ChatMessage, ChunkChoice, ChunkDelta, CompletionChoice, CompletionChunk,
    CompletionRequest, CompletionResponse, HealthStatus, ModelInfo, ModelProvider, PricingInfo,
    ProviderHealth, SupportLevel, TokenUsage,
};
use crate::oauth::OAuthTokenSource;
use async_trait::async_trait;
use chrono::Utc;
use futures::stream::{Stream, StreamExt};
use lr_api_keys::{keychain_trait::KeychainStorage, CachedKeychain};
use lr_types::{AppError, AppResult};
use reqwest_middleware::ClientWithMiddleware;
use serde::{Deserialize, Serialize};
use std::pin::Pin;
use std::sync::Arc;
use std::time::Instant;
use tracing::{debug, info};

const OPENAI_API_BASE: &str = "https://api.openai.com/v1";
/// ChatGPT Plus/Pro OAuth tokens (from the codex_cli flow) are scoped to
/// `chatgpt.com/backend-api/codex/*`, not `api.openai.com/v1/*`. Calling
/// `api.openai.com/v1/models` with those tokens returns 403. Verified
/// against openai/codex: `backend-client/src/client.rs` uses
/// `https://chatgpt.com/backend-api` as its base.
const CHATGPT_BACKEND_API_BASE: &str = "https://chatgpt.com/backend-api/codex";
const OAUTH_KEYCHAIN_SERVICE: &str = crate::oauth::openai_codex::KEYCHAIN_SERVICE;
const OAUTH_PROVIDER_ID: &str = crate::oauth::openai_codex::PROVIDER_ID;

/// How this provider instance authenticates.
///
/// OAuth credentials are resolved per request rather than snapshotted at
/// construction: the provider instance lives in the registry for the whole
/// process, while a ChatGPT Plus/Pro access token expires within the hour and
/// changes again whenever the user reconnects.
enum ProviderAuth {
    ApiKey(String),
    OAuth(Arc<OAuthTokenSource>),
}

impl ProviderAuth {
    async fn token(&self) -> AppResult<String> {
        match self {
            Self::ApiKey(key) => Ok(key.clone()),
            Self::OAuth(source) => source.access_token().await,
        }
    }

    /// A token to retry with after the upstream rejected `rejected` with a
    /// 401, or `None` when this instance has nothing better to offer.
    async fn token_after_unauthorized(&self, rejected: &str) -> Option<String> {
        match self {
            Self::ApiKey(_) => None,
            Self::OAuth(source) => source.refresh_after_unauthorized(rejected).await.ok(),
        }
    }
}

/// OpenAI provider implementation
pub struct OpenAIProvider {
    auth: ProviderAuth,
    client: ClientWithMiddleware,
    base_url: String,
}

#[allow(dead_code)]
impl OpenAIProvider {
    /// Create a new OpenAI provider with the given API key
    pub fn new(api_key: String) -> Self {
        Self {
            auth: ProviderAuth::ApiKey(api_key),
            client: crate::http_client::default_client(),
            base_url: OPENAI_API_BASE.to_string(),
        }
    }

    /// Create a new OpenAI provider with a custom base URL (for testing)
    pub fn with_base_url(api_key: String, base_url: String) -> AppResult<Self> {
        Ok(Self {
            auth: ProviderAuth::ApiKey(api_key),
            client: crate::http_client::default_client(),
            base_url,
        })
    }

    /// Create a new OpenAI provider from stored API key
    ///
    /// # Arguments
    /// * `provider_name` - The provider name used to store the key (defaults to "openai")
    ///
    /// # Returns
    /// * `Ok(Self)` if key exists and provider created successfully
    /// * `Err(AppError)` if key doesn't exist or keyring access fails
    pub fn from_stored_key(provider_name: Option<&str>) -> AppResult<Self> {
        let name = provider_name.unwrap_or("openai");
        let api_key = super::key_storage::get_provider_key(name)?.ok_or_else(|| {
            AppError::Provider(format!("No API key found for provider '{}'", name))
        })?;
        Ok(Self::new(api_key))
    }

    /// Create a new OpenAI provider from OAuth tokens or API key (OAuth-first)
    ///
    /// This method checks for OAuth tokens first, and falls back to API key if:
    /// - No OAuth tokens are stored
    /// - OAuth tokens are expired and cannot be refreshed
    ///
    /// # Arguments
    /// * `provider_name` - The provider name used to store the API key (defaults to "openai")
    ///
    /// # Returns
    /// * `Ok(Self)` if either OAuth tokens or API key are available
    /// * `Err(AppError)` if neither OAuth nor API key authentication is available
    pub fn from_oauth_or_key(provider_name: Option<&str>) -> AppResult<Self> {
        // Try OAuth first.
        //
        // ChatGPT Plus/Pro OAuth tokens can't call `api.openai.com/v1/*`
        // (returns 403) — they're only valid against the `chatgpt.com`
        // backend. Route OAuth-based instances there so health checks,
        // model listings, and eventually completions authenticate
        // successfully.
        //
        // Only the *presence* of credentials is decided here; the token
        // itself is read per request from the shared token source, which
        // refreshes it and picks up reconnects.
        if Self::has_oauth_credentials() {
            if let Some(source) = crate::oauth::token_source(OAUTH_PROVIDER_ID) {
                info!("Using OAuth credentials for OpenAI provider");
                debug!("Resolving OAuth access tokens for openai-codex per request");
                return Ok(Self {
                    auth: ProviderAuth::OAuth(source),
                    client: crate::http_client::default_client(),
                    base_url: CHATGPT_BACKEND_API_BASE.to_string(),
                });
            }
        }

        // Fall back to API key
        debug!("No OAuth credentials found, falling back to API key for OpenAI");
        Self::from_stored_key(provider_name)
    }

    /// Check if OAuth credentials are available for this provider
    ///
    /// # Returns
    /// * `true` if OAuth access token exists in keychain
    /// * `false` otherwise
    pub fn has_oauth_credentials() -> bool {
        let keychain = CachedKeychain::auto().unwrap_or_else(|_| CachedKeychain::system());
        keychain
            .get(
                OAUTH_KEYCHAIN_SERVICE,
                &format!("{}_access_token", OAUTH_PROVIDER_ID),
            )
            .ok()
            .flatten()
            .is_some()
    }

    /// True when this instance talks to the ChatGPT Plus/Pro backend
    /// (via the codex_cli OAuth) rather than the public platform API.
    ///
    /// Subscription credentials imply that backend — they are rejected by
    /// `api.openai.com` — so an OAuth-backed instance counts regardless of the
    /// base URL it was pointed at.
    fn is_chatgpt_backend(&self) -> bool {
        matches!(self.auth, ProviderAuth::OAuth(_))
            || self.base_url.starts_with(CHATGPT_BACKEND_API_BASE)
    }

    /// Fallback model list for ChatGPT Plus/Pro subscriptions when the
    /// remote `/models` catalog can't be reached. The ids match the
    /// visible set codex-rs bundles as its offline catalog
    /// (`models-manager/models.json`, `visibility: "list"`). The older
    /// entries that were here — `gpt-5-codex`, `gpt-4o`, `gpt-4o-mini`,
    /// `o1`, `o1-mini` — are *not* dispatchable via the ChatGPT-account
    /// Codex backend; calling them returns 400 "not supported when
    /// using Codex with a ChatGPT account".
    ///
    /// Model metadata (context window, capabilities) is resolved via
    /// `lr-catalog` so this fallback stays in sync with the embedded
    /// models.dev snapshot rather than carrying its own copy.
    fn chatgpt_plus_fallback_models() -> Vec<ModelInfo> {
        // Mirrors codex-rs `models-manager/models.json` — the entries it
        // marks `visibility: "list"` — newest-first so the picker defaults
        // to the latest frontier model. Keep in sync with that upstream
        // file; see CLAUDE.md "Updating the Model Catalog". Each tuple is
        // (id, display name, context window) copied verbatim from that
        // source; the 5.6 models ship a larger 372k window than the 272k
        // older ones.
        //
        // These are the ChatGPT-Codex-*backend* variants, so codex's context
        // windows are authoritative — deliberately NOT resolved via
        // lr-catalog/models.dev, whose identically-named public-API models
        // advertise a much larger window (e.g. 1.05M) that the Codex backend
        // does not honor. Every list-visible codex model accepts image input.
        let models = [
            ("gpt-5.6-sol", "GPT-5.6-Sol", 372_000),
            ("gpt-5.6-terra", "GPT-5.6-Terra", 372_000),
            ("gpt-5.6-luna", "GPT-5.6-Luna", 372_000),
            ("gpt-5.5", "GPT-5.5", 272_000),
            ("gpt-5.4", "GPT-5.4", 272_000),
            ("gpt-5.4-mini", "GPT-5.4-Mini", 272_000),
            ("gpt-5.2", "GPT-5.2", 272_000),
        ];
        models
            .iter()
            .map(|(id, name, ctx)| ModelInfo {
                id: (*id).to_string(),
                name: (*name).to_string(),
                provider: "openai".to_string(),
                parameter_count: None,
                context_window: *ctx,
                supports_streaming: true,
                capabilities: vec![
                    Capability::Chat,
                    Capability::FunctionCalling,
                    Capability::Vision,
                ],
                detailed_capabilities: None,
            })
            .collect()
    }

    /// Hit the Codex backend `/models` catalog (the same one the
    /// official `codex-rs` CLI fetches) and map visible entries to our
    /// `ModelInfo`. Returns `None` on any failure so the caller can
    /// fall back to the bundled list.
    async fn fetch_chatgpt_plus_models(&self) -> Option<Vec<ModelInfo>> {
        let token = self.auth.token().await.ok()?;
        let response = match self.get_chatgpt_models(&token).await? {
            // The catalog is often the first call made after a token goes
            // stale — refresh and try once more before falling back.
            response if response.status() == reqwest::StatusCode::UNAUTHORIZED => {
                let token = self.auth.token_after_unauthorized(&token).await?;
                self.get_chatgpt_models(&token).await?
            }
            response => response,
        };

        if !response.status().is_success() {
            debug!(
                "ChatGPT backend /models returned {}; using fallback list",
                response.status()
            );
            return None;
        }

        let body: ChatGptModelsResponse = response.json().await.ok()?;
        let mut models: Vec<ModelInfo> = body
            .models
            .into_iter()
            .filter(|m| m.visibility.as_deref().map(|v| v == "list").unwrap_or(true))
            .map(|m| {
                let has_image = m
                    .input_modalities
                    .as_ref()
                    .map(|mods| mods.iter().any(|s| s == "image"))
                    .unwrap_or(false);
                let mut caps = vec![Capability::Chat, Capability::FunctionCalling];
                if has_image {
                    caps.push(Capability::Vision);
                }
                ModelInfo {
                    id: m.slug.clone(),
                    name: m.display_name.unwrap_or(m.slug),
                    provider: "openai".to_string(),
                    parameter_count: None,
                    context_window: m.context_window.unwrap_or(272_000),
                    supports_streaming: true,
                    capabilities: caps,
                    detailed_capabilities: None,
                }
            })
            .collect();

        if models.is_empty() {
            return None;
        }
        models.sort_by(|a, b| a.id.cmp(&b.id));
        Some(models)
    }

    /// One GET of the Codex backend catalog with the given access token.
    async fn get_chatgpt_models(&self, access_token: &str) -> Option<reqwest::Response> {
        self.client
            .get(format!("{}/models", self.base_url))
            .bearer_auth(access_token)
            .header("OpenAI-Beta", "responses=v1")
            .send()
            .await
            .ok()
    }

    /// Get pricing information for known OpenAI models
    fn get_model_pricing(model: &str) -> Option<PricingInfo> {
        // Pricing information as of January 2025
        // Source: https://openai.com/api/pricing/
        match model {
            // GPT-4 Turbo models
            "gpt-4-turbo" | "gpt-4-turbo-2024-04-09" => Some(PricingInfo {
                input_cost_per_1k: 0.01,
                output_cost_per_1k: 0.03,
                reasoning_cost_per_1k: None,
                currency: "USD".to_string(),
            }),
            "gpt-4-turbo-preview" | "gpt-4-0125-preview" | "gpt-4-1106-preview" => {
                Some(PricingInfo {
                    input_cost_per_1k: 0.01,
                    output_cost_per_1k: 0.03,
                    reasoning_cost_per_1k: None,
                    currency: "USD".to_string(),
                })
            }
            // GPT-4 models
            "gpt-4" | "gpt-4-0613" => Some(PricingInfo {
                input_cost_per_1k: 0.03,
                output_cost_per_1k: 0.06,
                reasoning_cost_per_1k: None,
                currency: "USD".to_string(),
            }),
            "gpt-4-32k" | "gpt-4-32k-0613" => Some(PricingInfo {
                input_cost_per_1k: 0.06,
                output_cost_per_1k: 0.12,
                reasoning_cost_per_1k: None,
                currency: "USD".to_string(),
            }),
            // GPT-3.5 Turbo models
            "gpt-3.5-turbo" | "gpt-3.5-turbo-0125" | "gpt-3.5-turbo-1106" => Some(PricingInfo {
                input_cost_per_1k: 0.0005,
                output_cost_per_1k: 0.0015,
                reasoning_cost_per_1k: None,
                currency: "USD".to_string(),
            }),
            "gpt-3.5-turbo-instruct" => Some(PricingInfo {
                input_cost_per_1k: 0.0015,
                output_cost_per_1k: 0.002,
                reasoning_cost_per_1k: None,
                currency: "USD".to_string(),
            }),
            // GPT-4o models (newest)
            "gpt-4o" | "gpt-4o-2024-11-20" | "gpt-4o-2024-08-06" | "gpt-4o-2024-05-13" => {
                Some(PricingInfo {
                    input_cost_per_1k: 0.0025,
                    output_cost_per_1k: 0.01,
                    reasoning_cost_per_1k: None,
                    currency: "USD".to_string(),
                })
            }
            "gpt-4o-mini" | "gpt-4o-mini-2024-07-18" => Some(PricingInfo {
                input_cost_per_1k: 0.00015,
                output_cost_per_1k: 0.0006,
                reasoning_cost_per_1k: None,
                currency: "USD".to_string(),
            }),
            // o1 models (reasoning models)
            "o1-preview" | "o1-preview-2024-09-12" => Some(PricingInfo {
                input_cost_per_1k: 0.015,
                output_cost_per_1k: 0.06,
                reasoning_cost_per_1k: None,
                currency: "USD".to_string(),
            }),
            "o1-mini" | "o1-mini-2024-09-12" => Some(PricingInfo {
                input_cost_per_1k: 0.003,
                output_cost_per_1k: 0.012,
                reasoning_cost_per_1k: None,
                currency: "USD".to_string(),
            }),
            _ => {
                // Try stripping date suffix (-YYYY-MM-DD) and retrying
                let bytes = model.as_bytes();
                if bytes.len() > 11 {
                    let s = bytes.len() - 11;
                    if bytes[s] == b'-'
                        && bytes[s + 1..s + 5].iter().all(u8::is_ascii_digit)
                        && bytes[s + 5] == b'-'
                        && bytes[s + 6..s + 8].iter().all(u8::is_ascii_digit)
                        && bytes[s + 8] == b'-'
                        && bytes[s + 9..s + 11].iter().all(u8::is_ascii_digit)
                    {
                        return Self::get_model_pricing(&model[..s]);
                    }
                }
                None
            }
        }
    }

    /// Build authorization header
    async fn auth_header(&self) -> AppResult<String> {
        Ok(format!("Bearer {}", self.auth.token().await?))
    }
}

// OpenAI API response types

#[derive(Debug, Deserialize)]
struct OpenAIModel {
    id: String,
    #[allow(dead_code)]
    object: String,
    #[allow(dead_code)]
    created: i64,
    #[allow(dead_code)]
    owned_by: String,
}

#[derive(Debug, Deserialize)]
struct OpenAIModelsResponse {
    data: Vec<OpenAIModel>,
}

/// Subset of the codex-rs `ModelsResponse` we care about (slug +
/// surfacing hints). Fields not listed are ignored; the upstream
/// catalog adds metadata frequently so we deliberately avoid a strict
/// schema here.
#[derive(Debug, Deserialize)]
struct ChatGptModelsResponse {
    models: Vec<ChatGptModelEntry>,
}

#[derive(Debug, Deserialize)]
struct ChatGptModelEntry {
    slug: String,
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    visibility: Option<String>,
    #[serde(default)]
    context_window: Option<u32>,
    #[serde(default)]
    input_modalities: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
struct OpenAIChatRequest {
    model: String,
    messages: Vec<ChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    frequency_penalty: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    presence_penalty: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stop: Option<Vec<String>>,
    #[serde(default)]
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<super::Tool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<super::ToolChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_format: Option<super::ResponseFormat>,
    #[serde(skip_serializing_if = "Option::is_none")]
    n: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    logit_bias: Option<std::collections::HashMap<String, f32>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    parallel_tool_calls: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    service_tier: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    store: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    metadata: Option<std::collections::HashMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    modalities: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    audio: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    prediction: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_effort: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenAIChatResponse {
    id: String,
    object: String,
    created: i64,
    model: String,
    choices: Vec<OpenAIChoice>,
    usage: OpenAIUsage,
    #[serde(default)]
    system_fingerprint: Option<String>,
    #[serde(default)]
    service_tier: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenAIChoice {
    index: u32,
    message: ChatMessage,
    finish_reason: Option<String>,
    logprobs: Option<super::Logprobs>,
}

#[derive(Debug, Deserialize)]
struct OpenAIUsage {
    prompt_tokens: u32,
    completion_tokens: u32,
    total_tokens: u32,
}

#[derive(Debug, Deserialize)]
struct OpenAIStreamChunk {
    id: String,
    object: String,
    created: i64,
    model: String,
    choices: Vec<OpenAIStreamChoice>,
}

#[derive(Debug, Deserialize)]
struct OpenAIStreamChoice {
    index: u32,
    delta: OpenAIDelta,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenAIDelta {
    #[serde(skip_serializing_if = "Option::is_none")]
    role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<super::ToolCallDelta>>,
    /// Reasoning/thinking content from reasoning models (o1, o3, DeepSeek-R1, etc.)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reasoning_content: Option<String>,
}

// OpenAI Embeddings API types
#[derive(Debug, Serialize)]
struct OpenAIEmbeddingRequest {
    model: String,
    input: OpenAIEmbeddingInput,
    #[serde(skip_serializing_if = "Option::is_none")]
    encoding_format: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    dimensions: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    user: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
enum OpenAIEmbeddingInput {
    Single(String),
    Multiple(Vec<String>),
}

#[derive(Debug, Deserialize)]
struct OpenAIEmbeddingResponse {
    object: String,
    data: Vec<OpenAIEmbedding>,
    model: String,
    usage: OpenAIEmbeddingUsage,
}

#[derive(Debug, Deserialize)]
struct OpenAIEmbedding {
    object: String,
    embedding: Vec<f32>,
    index: usize,
}

#[derive(Debug, Deserialize)]
struct OpenAIEmbeddingUsage {
    prompt_tokens: u32,
    total_tokens: u32,
}

/// Determine MIME type for an audio file based on its extension.
fn audio_mime_type(file_name: &str) -> String {
    let ext = file_name.rsplit('.').next().unwrap_or("").to_lowercase();
    match ext.as_str() {
        "mp3" => "audio/mpeg",
        "mp4" | "m4a" => "audio/mp4",
        "mpeg" => "audio/mpeg",
        "mpga" => "audio/mpeg",
        "ogg" | "oga" => "audio/ogg",
        "wav" => "audio/wav",
        "webm" => "audio/webm",
        "flac" => "audio/flac",
        _ => "application/octet-stream",
    }
    .to_string()
}

#[async_trait]
#[allow(dead_code)]
impl ModelProvider for OpenAIProvider {
    fn name(&self) -> &str {
        "openai"
    }

    async fn health_check(&self) -> ProviderHealth {
        let start = Instant::now();

        // The ChatGPT backend-api doesn't expose a cheap GET endpoint we
        // can probe for health without consuming quota. Resolving the OAuth
        // token is the meaningful check instead: it refreshes the token when
        // it is due and fails when the session needs a reconnect. The rest of
        // the signal comes from actual requests.
        if self.is_chatgpt_backend() {
            return match self.auth.token().await {
                Ok(_) => ProviderHealth {
                    status: HealthStatus::Healthy,
                    latency_ms: Some(start.elapsed().as_millis() as u64),
                    last_checked: Utc::now(),
                    error_message: None,
                },
                Err(e) => ProviderHealth {
                    status: HealthStatus::Unhealthy,
                    latency_ms: None,
                    last_checked: Utc::now(),
                    error_message: Some(e.to_string()),
                },
            };
        }

        let auth_header = match self.auth_header().await {
            Ok(header) => header,
            Err(e) => {
                return ProviderHealth {
                    status: HealthStatus::Unhealthy,
                    latency_ms: None,
                    last_checked: Utc::now(),
                    error_message: Some(e.to_string()),
                }
            }
        };

        // Use /v1/models endpoint for health check
        let result = self
            .client
            .get(format!("{}/models", self.base_url))
            .header("Authorization", auth_header)
            .send()
            .await;

        let latency_ms = start.elapsed().as_millis() as u64;

        match result {
            Ok(response) => {
                let status = response.status();
                if status.is_success() {
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
                        error_message: Some(format!("API returned status: {}", status)),
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
        // ChatGPT Plus/Pro OAuth tokens can't call `/v1/models`. The
        // `chatgpt.com/backend-api/codex/models` catalog is what
        // codex-rs itself fetches, so try that first and fall back to
        // the bundled list on any error.
        if self.is_chatgpt_backend() {
            if let Some(models) = self.fetch_chatgpt_plus_models().await {
                return Ok(models);
            }
            return Ok(Self::chatgpt_plus_fallback_models());
        }

        let response = self
            .client
            .get(format!("{}/models", self.base_url))
            .header("Authorization", self.auth_header().await?)
            .send()
            .await
            .map_err(|e| AppError::Provider(format!("Failed to fetch models: {}", e)))?;

        if !response.status().is_success() {
            return Err(AppError::Provider(format!(
                "API returned status: {}",
                response.status()
            )));
        }

        let models_response: OpenAIModelsResponse = response
            .json()
            .await
            .map_err(|e| AppError::Provider(format!("Failed to parse models response: {}", e)))?;

        let mut models = Vec::new();

        for model in models_response.data {
            // Classify models by prefix to assign capabilities and metadata
            let is_chat_model = model.id.starts_with("gpt-")
                || model.id.starts_with("o1-")
                || model.id.starts_with("text-");
            let is_embedding = model.id.starts_with("text-embedding-");
            let is_audio = model.id.starts_with("whisper-");
            let is_tts = model.id.starts_with("tts-");
            let is_image = model.id.starts_with("dall-e-");

            if !is_chat_model && !is_embedding && !is_audio && !is_tts && !is_image {
                continue;
            }

            // Non-chat models get specialized capabilities and defaults
            if is_embedding || is_audio || is_tts || is_image {
                let capabilities = if is_embedding {
                    vec![Capability::Embedding]
                } else if is_audio {
                    vec![Capability::Audio]
                } else if is_tts {
                    vec![Capability::TextToSpeech]
                } else {
                    // dall-e: empty = unknown/try everything
                    vec![]
                };

                models.push(ModelInfo {
                    id: model.id.clone(),
                    name: model.id,
                    provider: "openai".to_string(),
                    parameter_count: None,
                    context_window: 0,
                    supports_streaming: is_tts, // TTS supports streaming audio
                    capabilities,
                    detailed_capabilities: None,
                });
                continue;
            }

            // Determine context window based on model name
            let context_window = if model.id.contains("32k") {
                32768
            } else if model.id.contains("turbo") {
                16384
            } else if model.id.starts_with("gpt-4o") || model.id.starts_with("o1") {
                128000
            } else if model.id.starts_with("gpt-4") {
                8192
            } else {
                // Default for gpt-3.5 and others
                4096
            };

            // Determine parameter count (estimates)
            let parameter_count = if model.id.starts_with("gpt-4") {
                Some(1_760_000_000_000) // 1.76T parameters (estimated)
            } else if model.id.starts_with("gpt-3.5") {
                Some(175_000_000_000) // 175B parameters
            } else {
                None
            };

            // Determine capabilities
            let mut capabilities = vec![Capability::Chat, Capability::Completion];
            if !model.id.starts_with("o1") {
                capabilities.push(Capability::FunctionCalling);
            }
            // GPT-4 Vision models
            if model.id.contains("vision") || model.id.starts_with("gpt-4o") {
                capabilities.push(Capability::Vision);
            }

            models.push(ModelInfo {
                id: model.id.clone(),
                name: model.id,
                provider: "openai".to_string(),
                parameter_count,
                context_window,
                supports_streaming: true,
                capabilities,
                detailed_capabilities: None,
            });
        }

        Ok(models)
    }

    async fn get_pricing(&self, model: &str) -> AppResult<PricingInfo> {
        // Strip any "<provider>/" prefix — the model field on a
        // CompletionResponse can come back as "ChatGPT Plus/gpt-5.4"
        // when the request used a provider-qualified model id, but
        // the catalog and fallback tables only know bare model ids
        // ("gpt-5.4"). Without this, every ChatGPT Plus turn would
        // log a "not found in catalog" warning and zero out cost.
        let model_id = model.rsplit_once('/').map(|(_, m)| m).unwrap_or(model);

        // Try catalog first (embedded OpenRouter data)
        if let Some(catalog_model) = lr_catalog::find_model("openai", model_id) {
            tracing::debug!("Using catalog pricing for OpenAI model: {}", model_id);
            return Ok(PricingInfo {
                input_cost_per_1k: catalog_model.pricing.prompt_cost_per_1k(),
                output_cost_per_1k: catalog_model.pricing.completion_cost_per_1k(),
                reasoning_cost_per_1k: catalog_model.pricing.reasoning_cost_per_1k(),
                currency: catalog_model.pricing.currency.to_string(),
            });
        }

        // Fallback to hardcoded pricing (for models not in catalog)
        if let Some(pricing) = Self::get_model_pricing(model_id) {
            tracing::debug!("Using fallback pricing for OpenAI model: {}", model_id);
            return Ok(pricing);
        }

        // Log unmapped models
        tracing::warn!(
            "Model '{}' not found in catalog or fallback pricing (provider: openai)",
            model_id
        );

        Err(AppError::Provider(format!(
            "Pricing information not available for model: {}",
            model_id
        )))
    }

    async fn complete(&self, request: CompletionRequest) -> AppResult<CompletionResponse> {
        // ChatGPT Plus/Pro OAuth tokens authorize only the `/responses`
        // endpoint on `chatgpt.com/backend-api/codex` — not
        // `/chat/completions`. Translate the request through the
        // Responses API module so subscription users actually get a
        // reply (rather than the 404 they'd get against /chat/completions).
        if self.is_chatgpt_backend() {
            let req = crate::openai_responses::translate_completion_request(&request, false);
            let token = self.auth.token().await?;
            let result = crate::openai_responses::create_response(
                &self.client,
                &self.base_url,
                &token,
                self.name(),
                req.clone(),
            )
            .await;

            let Err(AppError::Unauthorized) = result else {
                return result;
            };

            // The token died between resolving it and using it, or was
            // replaced by a reconnect. Get a live one and try once more.
            let token = self
                .auth
                .token_after_unauthorized(&token)
                .await
                .ok_or(AppError::Unauthorized)?;
            return crate::openai_responses::create_response(
                &self.client,
                &self.base_url,
                &token,
                self.name(),
                req,
            )
            .await;
        }

        let openai_request = OpenAIChatRequest {
            model: request.model.clone(),
            messages: request.messages.clone(),
            temperature: request.temperature,
            max_tokens: request.max_tokens,
            top_p: request.top_p,
            frequency_penalty: request.frequency_penalty,
            presence_penalty: request.presence_penalty,
            stop: request.stop,
            stream: false,
            tools: request.tools,
            tool_choice: request.tool_choice,
            response_format: request.response_format,
            n: request.n,
            logit_bias: request.logit_bias,
            parallel_tool_calls: request.parallel_tool_calls,
            service_tier: request.service_tier,
            store: request.store,
            metadata: request.metadata,
            modalities: request.modalities,
            audio: request.audio,
            prediction: request.prediction,
            reasoning_effort: request.reasoning_effort,
        };

        let response = self
            .client
            .post(format!("{}/chat/completions", self.base_url))
            .header("Authorization", self.auth_header().await?)
            .header("Content-Type", "application/json")
            .json(&openai_request)
            .send()
            .await
            .map_err(|e| AppError::Provider(format!("Request failed: {}", e)))?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());

            return Err(crate::http_client::classify_openai_error(
                status,
                &error_text,
            ));
        }

        let openai_response: OpenAIChatResponse = response
            .json()
            .await
            .map_err(|e| AppError::Provider(format!("Failed to parse response: {}", e)))?;

        Ok(CompletionResponse {
            id: openai_response.id,
            object: openai_response.object,
            created: openai_response.created,
            model: openai_response.model,
            provider: self.name().to_string(),
            choices: openai_response
                .choices
                .into_iter()
                .map(|choice| CompletionChoice {
                    index: choice.index,
                    message: choice.message,
                    finish_reason: choice.finish_reason,
                    logprobs: choice.logprobs,
                })
                .collect(),
            usage: TokenUsage {
                prompt_tokens: openai_response.usage.prompt_tokens,
                completion_tokens: openai_response.usage.completion_tokens,
                total_tokens: openai_response.usage.total_tokens,
                prompt_tokens_details: None,
                completion_tokens_details: None,
            },
            system_fingerprint: openai_response.system_fingerprint,
            service_tier: openai_response.service_tier,
            extensions: None,
            routellm_win_rate: None,
            request_usage_entries: None,
        })
    }

    async fn stream_complete(
        &self,
        request: CompletionRequest,
    ) -> AppResult<Pin<Box<dyn Stream<Item = AppResult<CompletionChunk>> + Send>>> {
        // Same routing as `complete` above — ChatGPT Plus tokens go
        // through `/responses`; everything else uses /chat/completions.
        if self.is_chatgpt_backend() {
            let model = request.model.clone();
            let mut req = crate::openai_responses::translate_completion_request(&request, false);
            req.stream = true;
            let token = self.auth.token().await?;
            let result = crate::openai_responses::stream_response(
                &self.client,
                &self.base_url,
                &token,
                self.name(),
                model.clone(),
                req.clone(),
            )
            .await;

            let Err(AppError::Unauthorized) = result else {
                return result;
            };

            // Nothing has been streamed to the caller yet (the 401 comes from
            // the response head), so retrying with a live token is safe.
            let token = self
                .auth
                .token_after_unauthorized(&token)
                .await
                .ok_or(AppError::Unauthorized)?;
            return crate::openai_responses::stream_response(
                &self.client,
                &self.base_url,
                &token,
                self.name(),
                model,
                req,
            )
            .await;
        }

        let openai_request = OpenAIChatRequest {
            model: request.model.clone(),
            messages: request.messages.clone(),
            temperature: request.temperature,
            max_tokens: request.max_tokens,
            top_p: request.top_p,
            frequency_penalty: request.frequency_penalty,
            presence_penalty: request.presence_penalty,
            stop: request.stop,
            stream: true,
            tools: request.tools,
            tool_choice: request.tool_choice,
            response_format: request.response_format,
            n: request.n,
            logit_bias: request.logit_bias,
            parallel_tool_calls: request.parallel_tool_calls,
            service_tier: request.service_tier,
            store: request.store,
            metadata: request.metadata,
            modalities: request.modalities,
            audio: request.audio,
            prediction: request.prediction,
            reasoning_effort: request.reasoning_effort,
        };

        let response = self
            .client
            .post(format!("{}/chat/completions", self.base_url))
            .header("Authorization", self.auth_header().await?)
            .header("Content-Type", "application/json")
            .json(&openai_request)
            .send()
            .await
            .map_err(|e| AppError::Provider(format!("Request failed: {}", e)))?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());

            return Err(crate::http_client::classify_openai_error(
                status,
                &error_text,
            ));
        }

        // Parse SSE (Server-Sent Events) stream with proper line buffering
        let stream = response.bytes_stream();

        // Buffer for incomplete lines across byte chunks
        use std::sync::{Arc, Mutex};
        let line_buffer = Arc::new(Mutex::new(String::new()));

        let converted_stream = stream.flat_map(move |result| {
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
                        if let Some(json_str) = line.strip_prefix("data: ") {
                            // Check for [DONE] marker
                            if json_str.trim() == "[DONE]" {
                                continue;
                            }

                            // Parse JSON chunk
                            match serde_json::from_str::<OpenAIStreamChunk>(json_str) {
                                Ok(openai_chunk) => {
                                    // OpenAI sends delta chunks, not cumulative
                                    let chunk = CompletionChunk {
                                        id: openai_chunk.id,
                                        object: openai_chunk.object,
                                        created: openai_chunk.created,
                                        model: openai_chunk.model,
                                        choices: openai_chunk
                                            .choices
                                            .into_iter()
                                            .map(|choice| ChunkChoice {
                                                index: choice.index,
                                                delta: ChunkDelta {
                                                    role: choice.delta.role,
                                                    content: choice.delta.content,
                                                    tool_calls: choice.delta.tool_calls,
                                                    reasoning_content: choice
                                                        .delta
                                                        .reasoning_content,
                                                },
                                                finish_reason: choice.finish_reason,
                                            })
                                            .collect(),
                                        extensions: None,
                                    };
                                    chunks.push(Ok(chunk));
                                }
                                Err(e) => {
                                    tracing::error!(
                                        "Failed to parse OpenAI stream chunk: {} - Line: {}",
                                        e,
                                        json_str
                                    );
                                }
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

    async fn embed(&self, request: super::EmbeddingRequest) -> AppResult<super::EmbeddingResponse> {
        // Convert our generic EmbeddingRequest to OpenAI-specific format
        let input = match request.input {
            super::EmbeddingInput::Single(text) => OpenAIEmbeddingInput::Single(text),
            super::EmbeddingInput::Multiple(texts) => OpenAIEmbeddingInput::Multiple(texts),
            super::EmbeddingInput::Tokens(_) => {
                return Err(AppError::Provider(
                    "OpenAI embeddings do not support pre-tokenized input".to_string(),
                ));
            }
        };

        let encoding_format = request.encoding_format.map(|format| match format {
            super::EncodingFormat::Float => "float".to_string(),
            super::EncodingFormat::Base64 => "base64".to_string(),
        });

        let openai_request = OpenAIEmbeddingRequest {
            model: request.model.clone(),
            input,
            encoding_format,
            dimensions: request.dimensions,
            user: request.user,
        };

        let response = self
            .client
            .post(format!("{}/embeddings", self.base_url))
            .header("Authorization", self.auth_header().await?)
            .header("Content-Type", "application/json")
            .json(&openai_request)
            .send()
            .await
            .map_err(|e| AppError::Provider(format!("Request failed: {}", e)))?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());

            return Err(crate::http_client::classify_openai_error(
                status,
                &error_text,
            ));
        }

        let openai_response: OpenAIEmbeddingResponse = response
            .json()
            .await
            .map_err(|e| AppError::Provider(format!("Failed to parse response: {}", e)))?;

        // Convert OpenAI response to our generic format
        Ok(super::EmbeddingResponse {
            object: openai_response.object,
            data: openai_response
                .data
                .into_iter()
                .map(|emb| super::Embedding {
                    object: emb.object,
                    embedding: Some(emb.embedding),
                    index: emb.index,
                })
                .collect(),
            model: openai_response.model,
            usage: super::EmbeddingUsage {
                prompt_tokens: openai_response.usage.prompt_tokens,
                total_tokens: openai_response.usage.total_tokens,
            },
        })
    }

    async fn generate_image(
        &self,
        request: super::ImageGenerationRequest,
    ) -> AppResult<super::ImageGenerationResponse> {
        // Build the request body
        let mut body = serde_json::json!({
            "model": request.model,
            "prompt": request.prompt,
            "n": request.n.unwrap_or(1),
        });

        if let Some(size) = &request.size {
            body["size"] = serde_json::json!(size);
        }
        if let Some(quality) = &request.quality {
            body["quality"] = serde_json::json!(quality);
        }
        if let Some(style) = &request.style {
            body["style"] = serde_json::json!(style);
        }
        if let Some(response_format) = &request.response_format {
            body["response_format"] = serde_json::json!(response_format);
        }
        if let Some(user) = &request.user {
            body["user"] = serde_json::json!(user);
        }

        let response = self
            .client
            .post(format!("{}/images/generations", self.base_url))
            .header("Authorization", self.auth_header().await?)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| AppError::Provider(format!("Request failed: {}", e)))?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());

            return Err(crate::http_client::classify_openai_error(
                status,
                &error_text,
            ));
        }

        let openai_response: serde_json::Value = response
            .json()
            .await
            .map_err(|e| AppError::Provider(format!("Failed to parse response: {}", e)))?;

        // Convert OpenAI response to our generic format
        let created = openai_response["created"]
            .as_i64()
            .unwrap_or_else(|| chrono::Utc::now().timestamp());

        let data: Vec<super::GeneratedImage> = openai_response["data"]
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

    async fn transcribe(
        &self,
        request: super::AudioTranscriptionRequest,
    ) -> AppResult<super::AudioTranscriptionResponse> {
        let mut form = reqwest::multipart::Form::new();

        // Add the audio file
        let mime_type = audio_mime_type(&request.file_name);
        let file_part = reqwest::multipart::Part::bytes(request.file)
            .file_name(request.file_name)
            .mime_str(&mime_type)
            .map_err(|e| AppError::Provider(format!("Failed to set MIME type: {}", e)))?;
        form = form.part("file", file_part);

        // Add required model field
        form = form.text("model", request.model);

        // Add optional fields
        if let Some(language) = request.language {
            form = form.text("language", language);
        }
        if let Some(prompt) = request.prompt {
            form = form.text("prompt", prompt);
        }
        if let Some(response_format) = request.response_format {
            form = form.text("response_format", response_format);
        }
        if let Some(temperature) = request.temperature {
            form = form.text("temperature", temperature.to_string());
        }
        if let Some(granularities) = request.timestamp_granularities {
            for granularity in granularities {
                form = form.text("timestamp_granularities[]", granularity);
            }
        }

        let response = self
            .client
            .post(format!("{}/audio/transcriptions", self.base_url))
            .header("Authorization", self.auth_header().await?)
            .multipart(form)
            .send()
            .await
            .map_err(|e| AppError::Provider(format!("Request failed: {}", e)))?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());

            return Err(crate::http_client::classify_openai_error(
                status,
                &error_text,
            ));
        }

        let transcription: super::AudioTranscriptionResponse = response
            .json()
            .await
            .map_err(|e| AppError::Provider(format!("Failed to parse response: {}", e)))?;

        Ok(transcription)
    }

    async fn translate_audio(
        &self,
        request: super::AudioTranslationRequest,
    ) -> AppResult<super::AudioTranslationResponse> {
        let mut form = reqwest::multipart::Form::new();

        // Add the audio file
        let mime_type = audio_mime_type(&request.file_name);
        let file_part = reqwest::multipart::Part::bytes(request.file)
            .file_name(request.file_name)
            .mime_str(&mime_type)
            .map_err(|e| AppError::Provider(format!("Failed to set MIME type: {}", e)))?;
        form = form.part("file", file_part);

        // Add required model field
        form = form.text("model", request.model);

        // Add optional fields (no language field — translation always outputs English)
        if let Some(prompt) = request.prompt {
            form = form.text("prompt", prompt);
        }
        if let Some(response_format) = request.response_format {
            form = form.text("response_format", response_format);
        }
        if let Some(temperature) = request.temperature {
            form = form.text("temperature", temperature.to_string());
        }

        let response = self
            .client
            .post(format!("{}/audio/translations", self.base_url))
            .header("Authorization", self.auth_header().await?)
            .multipart(form)
            .send()
            .await
            .map_err(|e| AppError::Provider(format!("Request failed: {}", e)))?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());

            return Err(crate::http_client::classify_openai_error(
                status,
                &error_text,
            ));
        }

        let translation: super::AudioTranslationResponse = response
            .json()
            .await
            .map_err(|e| AppError::Provider(format!("Failed to parse response: {}", e)))?;

        Ok(translation)
    }

    async fn speech(&self, request: super::SpeechRequest) -> AppResult<super::SpeechResponse> {
        let response = self
            .client
            .post(format!("{}/audio/speech", self.base_url))
            .header("Authorization", self.auth_header().await?)
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await
            .map_err(|e| AppError::Provider(format!("Request failed: {}", e)))?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());

            return Err(crate::http_client::classify_openai_error(
                status,
                &error_text,
            ));
        }

        // Determine content type from response headers or requested format
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string())
            .unwrap_or_else(|| {
                // Fallback: derive from requested format
                match request.response_format.as_deref() {
                    Some("opus") => "audio/opus".to_string(),
                    Some("aac") => "audio/aac".to_string(),
                    Some("flac") => "audio/flac".to_string(),
                    Some("wav") => "audio/wav".to_string(),
                    Some("pcm") => "audio/pcm".to_string(),
                    _ => "audio/mpeg".to_string(), // mp3 is the default
                }
            });

        let audio_data = response
            .bytes()
            .await
            .map_err(|e| AppError::Provider(format!("Failed to read audio data: {}", e)))?
            .to_vec();

        Ok(super::SpeechResponse {
            audio_data,
            content_type,
        })
    }

    fn supports_transcription(&self) -> bool {
        true
    }

    fn supports_audio_translation(&self) -> bool {
        true
    }

    fn supports_speech(&self) -> bool {
        true
    }

    fn supports_feature(&self, feature: &str) -> bool {
        matches!(
            feature,
            "reasoning_tokens" | "structured_outputs" | "logprobs"
        )
    }

    fn get_feature_adapter(
        &self,
        feature: &str,
    ) -> Option<Box<dyn crate::features::FeatureAdapter>> {
        match feature {
            "reasoning_tokens" => Some(Box::new(
                crate::features::openai_reasoning::OpenAIReasoningAdapter,
            )),
            "structured_outputs" => Some(Box::new(
                crate::features::structured_outputs::StructuredOutputsAdapter,
            )),
            "logprobs" => Some(Box::new(crate::features::logprobs::LogprobsAdapter)),
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

    /// OpenAI's public platform API speaks chat-completions, legacy
    /// completions, and — as of recently — the Responses API natively.
    /// ChatGPT Plus/Pro OAuth tokens *only* authorize the Responses
    /// endpoint on `chatgpt.com/backend-api/codex`, so chat-completions
    /// and legacy completions get translated to `/responses` by
    /// LocalRouter when that auth mode is active.
    fn api_path_support(&self, path: &str) -> SupportLevel {
        use super::SupportLevel as L;
        if self.is_chatgpt_backend() {
            match path {
                "responses" => L::Supported,
                "chat_completions" | "completions" => L::Translated,
                _ => L::NotSupported,
            }
        } else {
            match path {
                "chat_completions" | "completions" => L::Supported,
                // OpenAI does expose /responses natively on its platform
                // API, but LocalRouter currently proxies it through the
                // translation layer, so report it as translated.
                "responses" => L::Translated,
                _ => L::NotSupported,
            }
        }
    }

    fn get_feature_support(&self, instance_name: &str) -> super::ProviderFeatureSupport {
        let mut support = super::default_feature_support(self, instance_name);

        // Override model features with OpenAI-specific notes
        for f in &mut support.model_features {
            match f.name.as_str() {
                "Function Calling" => {
                    f.support = super::SupportLevel::Supported;
                    f.notes =
                        Some("GPT-4o, GPT-4 Turbo, and GPT-3.5 Turbo support tool calling".into());
                }
                "Vision" => {
                    f.support = super::SupportLevel::Supported;
                    f.notes = Some("GPT-4o and GPT-4 Turbo can process images".into());
                }
                "Reasoning Tokens" => {
                    f.support = super::SupportLevel::Partial;
                    f.notes = Some("Only o1-preview and o1-mini models use reasoning tokens; other models do not".into());
                }
                "Log Probabilities" => {
                    f.notes =
                        Some("Available on GPT-4o and GPT-3.5 Turbo via logprobs parameter".into());
                }
                "Structured Outputs" => {
                    f.notes = Some(
                        "GPT-4o supports strict JSON schema enforcement via response_format".into(),
                    );
                }
                "JSON Mode" => {
                    f.notes =
                        Some("All GPT-4 and GPT-3.5 Turbo models support JSON output mode".into());
                }
                "N Completions" => {
                    f.support = super::SupportLevel::Supported;
                    f.notes = Some("Generate up to 128 completion choices per request".into());
                }
                "Logit Bias" => {
                    f.support = super::SupportLevel::Supported;
                    f.notes = Some("Modify token likelihoods by token ID (-100 to 100)".into());
                }
                "Parallel Tool Calls" => {
                    f.support = super::SupportLevel::Supported;
                    f.notes =
                        Some("Models can generate multiple tool calls in a single response".into());
                }
                "Reasoning Effort" => {
                    f.support = super::SupportLevel::Partial;
                    f.notes = Some(
                        "Only o-series reasoning models support low/medium/high effort".into(),
                    );
                }
                "Predicted Output" => {
                    f.support = super::SupportLevel::Supported;
                    f.notes = Some(
                        "Supply predicted output for faster generation via speculative decoding"
                            .into(),
                    );
                }
                "Service Tier" => {
                    f.support = super::SupportLevel::Supported;
                    f.notes =
                        Some("Select 'auto' or 'default' latency tier for request routing".into());
                }
                "Audio Output" => {
                    f.support = super::SupportLevel::Partial;
                    f.notes = Some(
                        "Audio output via modalities parameter on gpt-4o-audio-preview models only"
                            .into(),
                    );
                }
                _ => {}
            }
        }

        // OpenAI endpoint-specific notes
        for e in &mut support.endpoints {
            match e.name.as_str() {
                "Moderations" => {
                    e.support = super::SupportLevel::NotImplemented;
                    e.notes = Some("OpenAI supports natively via text-moderation-latest; LocalRouter proxy not yet built".into());
                }
                "Responses API" => {
                    e.support = super::SupportLevel::NotImplemented;
                    e.notes =
                        Some("OpenAI supports natively; LocalRouter proxy not yet built".into());
                }
                "Batch Processing" => {
                    e.support = super::SupportLevel::NotImplemented;
                    e.notes = Some(
                        "OpenAI supports native async batches; LocalRouter proxy not yet built"
                            .into(),
                    );
                }
                "Audio Transcription" | "Audio Speech (TTS)" => {
                    e.support = super::SupportLevel::Supported;
                    e.notes = Some(
                        "Whisper for speech-to-text, TTS-1/TTS-1-HD for text-to-speech".into(),
                    );
                }
                "Realtime (WebSocket)" => {
                    e.support = super::SupportLevel::NotImplemented;
                    e.notes = Some("OpenAI supports natively — planned".into());
                }
                _ => {}
            }
        }

        support
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pricing_info_gpt4() {
        let pricing = OpenAIProvider::get_model_pricing("gpt-4").unwrap();
        assert_eq!(pricing.input_cost_per_1k, 0.03);
        assert_eq!(pricing.output_cost_per_1k, 0.06);
        assert_eq!(pricing.currency, "USD");
    }

    #[test]
    fn test_pricing_info_gpt35_turbo() {
        let pricing = OpenAIProvider::get_model_pricing("gpt-3.5-turbo").unwrap();
        assert_eq!(pricing.input_cost_per_1k, 0.0005);
        assert_eq!(pricing.output_cost_per_1k, 0.0015);
        assert_eq!(pricing.currency, "USD");
    }

    #[test]
    fn test_pricing_info_gpt4o() {
        let pricing = OpenAIProvider::get_model_pricing("gpt-4o").unwrap();
        assert_eq!(pricing.input_cost_per_1k, 0.0025);
        assert_eq!(pricing.output_cost_per_1k, 0.01);
        assert_eq!(pricing.currency, "USD");
    }

    #[test]
    fn test_pricing_info_unknown_model() {
        let pricing = OpenAIProvider::get_model_pricing("unknown-model");
        assert!(pricing.is_none());
    }

    #[test]
    fn test_provider_name() {
        let provider = OpenAIProvider::new("test-key".to_string());
        assert_eq!(provider.name(), "openai");
    }

    #[tokio::test]
    async fn test_auth_header() {
        let provider = OpenAIProvider::new("sk-test123".to_string());
        assert_eq!(provider.auth_header().await.unwrap(), "Bearer sk-test123");
    }

    #[tokio::test]
    async fn api_key_auth_does_not_retry_on_unauthorized() {
        // Only OAuth instances have somewhere to go after a 401; an API key
        // is all this instance will ever have.
        let provider = OpenAIProvider::new("sk-test123".to_string());
        assert!(provider
            .auth
            .token_after_unauthorized("sk-test123")
            .await
            .is_none());
    }

    /// A ChatGPT-backend provider whose token comes from `source`.
    fn oauth_provider(source: Arc<OAuthTokenSource>, base_url: String) -> OpenAIProvider {
        OpenAIProvider {
            auth: ProviderAuth::OAuth(source),
            client: crate::http_client::default_client(),
            base_url,
        }
    }

    /// Stands in for both `chatgpt.com/backend-api/codex` and OpenAI's token
    /// endpoint: `/responses` rejects everything but `new-token`, and
    /// `/token` hands out `new-token` for the refresh grant. Returns the
    /// bearer token seen on each `/responses` call.
    async fn fake_codex_backend() -> (std::net::SocketAddr, Arc<std::sync::Mutex<Vec<String>>>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let bearers = Arc::new(std::sync::Mutex::new(Vec::new()));
        let recorded = Arc::clone(&bearers);

        tokio::spawn(async move {
            while let Ok((mut sock, _)) = listener.accept().await {
                let recorded = Arc::clone(&recorded);
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 16384];
                    let n = sock.read(&mut buf).await.unwrap_or(0);
                    let head = String::from_utf8_lossy(&buf[..n]).to_string();
                    let bearer = head
                        .lines()
                        .find_map(|l| l.strip_prefix("authorization: Bearer "))
                        .map(|v| v.trim().to_string())
                        .unwrap_or_default();

                    let body = if head.starts_with("POST /token") {
                        r#"{"access_token":"new-token","token_type":"Bearer","expires_in":3600,"refresh_token":"refresh-2"}"#.to_string()
                    } else {
                        recorded.lock().unwrap().push(bearer.clone());
                        if bearer != "new-token" {
                            let _ = sock
                                .write_all(
                                    b"HTTP/1.1 401 Unauthorized\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
                                )
                                .await;
                            return;
                        }
                        r#"{"id":"resp_1","object":"response","status":"completed","model":"gpt-5.5","output":[{"type":"message","id":"m1","role":"assistant","content":[{"type":"output_text","text":"hi"}]}],"usage":{"input_tokens":1,"output_tokens":1,"total_tokens":2}}"#.to_string()
                    };

                    let _ = sock
                        .write_all(
                            format!(
                                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                                body.len(),
                                body
                            )
                            .as_bytes(),
                        )
                        .await;
                });
            }
        });

        (addr, bearers)
    }

    fn completion_request() -> CompletionRequest {
        CompletionRequest {
            model: "gpt-5.5".to_string(),
            messages: vec![ChatMessage {
                role: "user".to_string(),
                content: super::super::ChatMessageContent::Text("hi".to_string()),
                tool_calls: None,
                tool_call_id: None,
                name: None,
                reasoning_content: None,
            }],
            temperature: None,
            max_tokens: None,
            stream: false,
            top_p: None,
            frequency_penalty: None,
            presence_penalty: None,
            stop: None,
            top_k: None,
            seed: None,
            repetition_penalty: None,
            extensions: None,
            tools: None,
            tool_choice: None,
            response_format: None,
            logprobs: None,
            top_logprobs: None,
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
        }
    }

    /// The bug this guards: a live provider holding an expired token must
    /// recover on its own instead of 401-ing until it is re-created.
    #[tokio::test]
    async fn refreshes_and_retries_once_when_the_chatgpt_backend_returns_401() {
        let (addr, bearers) = fake_codex_backend().await;

        let keychain = lr_api_keys::CachedKeychain::new(Arc::new(lr_api_keys::MockKeychain::new()));
        keychain
            .store(OAUTH_KEYCHAIN_SERVICE, "test-codex_access_token", "expired")
            .unwrap();
        keychain
            .store(
                OAUTH_KEYCHAIN_SERVICE,
                "test-codex_refresh_token",
                "refresh-1",
            )
            .unwrap();

        let config = lr_oauth::browser::OAuthFlowConfig {
            token_url: format!("http://{addr}/token"),
            account_id: "test-codex".to_string(),
            ..crate::oauth::openai_codex::refresh_flow_config()
        };
        let source = Arc::new(OAuthTokenSource::with_keychain(config, keychain.clone()));
        let provider = oauth_provider(source, format!("http://{addr}"));

        let response = provider.complete(completion_request()).await.unwrap();
        assert_eq!(response.choices.len(), 1);

        // First attempt with the dead token, retry with the refreshed one.
        assert_eq!(
            *bearers.lock().unwrap(),
            vec!["expired".to_string(), "new-token".to_string()]
        );
        // The refreshed token is what the keychain holds afterwards, so a
        // restart doesn't fall back to the dead one.
        assert_eq!(
            keychain
                .get(OAUTH_KEYCHAIN_SERVICE, "test-codex_access_token")
                .unwrap(),
            Some("new-token".to_string())
        );
    }

    #[test]
    fn api_path_support_reflects_api_key_auth() {
        let provider = OpenAIProvider::new("sk-test".to_string());
        assert_eq!(
            provider.api_path_support("chat_completions"),
            SupportLevel::Supported
        );
        assert_eq!(
            provider.api_path_support("completions"),
            SupportLevel::Supported
        );
        assert_eq!(
            provider.api_path_support("responses"),
            SupportLevel::Translated
        );
        assert_eq!(
            provider.api_path_support("unknown"),
            SupportLevel::NotSupported
        );
    }

    #[test]
    fn api_path_support_reflects_chatgpt_oauth_backend() {
        let provider = OpenAIProvider::with_base_url(
            "oauth-token".to_string(),
            CHATGPT_BACKEND_API_BASE.to_string(),
        )
        .unwrap();
        assert_eq!(
            provider.api_path_support("responses"),
            SupportLevel::Supported
        );
        assert_eq!(
            provider.api_path_support("chat_completions"),
            SupportLevel::Translated
        );
        assert_eq!(
            provider.api_path_support("completions"),
            SupportLevel::Translated
        );
    }

    #[test]
    fn chatgpt_plus_fallback_list_covers_codex_visible_models() {
        let models = OpenAIProvider::chatgpt_plus_fallback_models();
        let ids: Vec<&str> = models.iter().map(|m| m.id.as_str()).collect();
        // Latest-first ordering — the picker uses the head of the list
        // as the default suggestion.
        assert_eq!(ids.first().copied(), Some("gpt-5.6-sol"));
        // Full codex `visibility: "list"` set — the 5.6 frontier trio plus
        // the older models codex still surfaces.
        assert!(ids.contains(&"gpt-5.6-sol"));
        assert!(ids.contains(&"gpt-5.6-terra"));
        assert!(ids.contains(&"gpt-5.6-luna"));
        assert!(ids.contains(&"gpt-5.5"));
        assert!(ids.contains(&"gpt-5.4"));
        assert!(ids.contains(&"gpt-5.4-mini"));
        assert!(ids.contains(&"gpt-5.2"));
        // The 5.6 frontier models carry the larger 372k window.
        let sol = models.iter().find(|m| m.id == "gpt-5.6-sol").unwrap();
        assert_eq!(sol.context_window, 372_000);
        // The old invalid ids must not sneak back in.
        assert!(!ids.contains(&"o1"));
        assert!(!ids.contains(&"o1-mini"));
        assert!(!ids.contains(&"gpt-4o"));
        assert!(!ids.contains(&"gpt-5-codex"));
    }

    #[tokio::test]
    async fn pricing_strips_provider_prefix_from_model_id() {
        // The CompletionResponse for ChatGPT Plus turns can come back
        // with `model = "ChatGPT Plus/gpt-5.4"` (the provider-qualified
        // form clients submit). `get_pricing` must strip the provider
        // prefix before consulting the catalog/fallback tables.
        let provider = OpenAIProvider::new("test-key".to_string());

        // Bare id resolves
        let bare = provider.get_pricing("gpt-5.4").await;
        // Prefixed id resolves to the same pricing
        let prefixed = provider.get_pricing("ChatGPT Plus/gpt-5.4").await;
        match (bare, prefixed) {
            (Ok(b), Ok(p)) => {
                assert!((b.input_cost_per_1k - p.input_cost_per_1k).abs() < f64::EPSILON);
                assert!((b.output_cost_per_1k - p.output_cost_per_1k).abs() < f64::EPSILON);
            }
            // If the catalog doesn't ship gpt-5.4 yet (test env quirk),
            // both forms must at least agree on Err — the prefix strip
            // shouldn't produce a different outcome than the bare id.
            (Err(_), Err(_)) => {}
            other => panic!(
                "prefix strip must produce the same outcome as the bare id, got {:?}",
                other
            ),
        }
    }
}
