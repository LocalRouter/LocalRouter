//! Generic OpenAI-compatible provider implementation
//!
//! This provider works with any service that implements the OpenAI API specification,
//! including LocalAI, LM Studio, vLLM, and other compatible services.

use super::{
    Capability, ChatMessage, ChunkChoice, ChunkDelta, CompletionChoice, CompletionChunk,
    CompletionRequest, CompletionResponse, HealthStatus, ModelInfo, ModelListing, ModelProvider,
    PricingInfo, ProviderHealth, TokenUsage,
};
use crate::listing_metadata::ModelMetadata;
use async_trait::async_trait;
use chrono::Utc;
use futures::stream::{Stream, StreamExt};
use lr_types::{AppError, AppResult};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use reqwest_middleware::ClientWithMiddleware;
use serde::{Deserialize, Serialize};
use std::pin::Pin;
use std::time::Instant;

pub(crate) mod stream_usage;
use crate::gateway::{GatewayAttribution, GatewayKind, GatewayPreferences};

/// Generic OpenAI-compatible provider with configurable endpoint
pub struct OpenAICompatibleProvider {
    name: String,
    gateway_kind: Option<GatewayKind>,
    gateway_preferences: GatewayPreferences,
    api_key: Option<String>,
    base_url: String,
    extra_headers: HeaderMap,
    client: ClientWithMiddleware,
    /// System One support for gateways that also host decision models.
    systemone_gateway: Option<std::sync::Arc<crate::systemone::SystemOneGateway>>,
    /// Provider type id of the upstream, when known. Decides whether
    /// streaming requests ask for usage (`stream_options.include_usage`);
    /// unknown upstreams are never asked.
    provider_type: Option<String>,
    /// Whether `GET {base}/models` is asked for the model list. Off for
    /// servers without that endpoint: only manual models are offered.
    model_discovery: bool,
    /// FastFlowLM: list only the models installed on this machine.
    installed_models: Option<crate::fastflowlm::InstalledModels>,
    /// Each model's entry in the latest `GET {base}/models` (prices, tiers)
    listing: crate::listing_metadata::ListingMetadataCache,
}

/// Parse a `custom_headers` config value into a header map.
///
/// Format: one header per line, `Name: Value`. Empty lines are skipped.
/// Names and values are validated against HTTP header syntax so a typo
/// fails at provider creation instead of on every request. Repeating a
/// name emits the header multiple times, as HTTP allows.
pub fn parse_custom_headers(raw: &str) -> AppResult<HeaderMap> {
    let mut headers = HeaderMap::new();
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let (name, value) = line.split_once(':').ok_or_else(|| {
            AppError::Config(format!(
                "Invalid custom header '{}': expected 'Name: Value' format",
                line
            ))
        })?;
        let name = name.trim();
        let value = value.trim();
        let name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|e| AppError::Config(format!("Invalid header name '{}': {}", name, e)))?;
        let value = HeaderValue::from_str(value)
            .map_err(|e| AppError::Config(format!("Invalid value for header '{}': {}", name, e)))?;
        headers.append(name, value);
    }
    Ok(headers)
}

impl OpenAICompatibleProvider {
    /// Create a new OpenAI-compatible provider
    ///
    /// # Arguments
    /// * `name` - Instance name for this provider
    /// * `base_url` - Base URL for the API (e.g., "http://localhost:8080/v1")
    /// * `api_key` - Optional API key (some services like LocalAI don't require one)
    pub fn new(name: String, base_url: String, api_key: Option<String>) -> Self {
        Self {
            name,
            gateway_kind: None,
            gateway_preferences: Default::default(),
            api_key,
            base_url: base_url.trim().trim_end_matches('/').to_string(),
            extra_headers: HeaderMap::new(),
            client: crate::http_client::default_client(),
            systemone_gateway: None,
            provider_type: None,
            model_discovery: true,
            installed_models: None,
            listing: Default::default(),
        }
    }

    pub fn with_gateway_preferences(
        mut self,
        kind: GatewayKind,
        preferences: GatewayPreferences,
    ) -> AppResult<Self> {
        let preferences = preferences.merge(&GatewayPreferences::default())?;
        preferences.apply(kind, &mut serde_json::json!({"model":"model"}))?;
        if let Some(shared) = self.systemone_gateway.as_mut() {
            let gateway = std::sync::Arc::get_mut(shared).ok_or_else(|| {
                AppError::Config(
                    "Gateway preferences must be set before sharing the provider".into(),
                )
            })?;
            gateway.set_gateway_preferences(preferences.clone());
        }
        self.gateway_kind = Some(kind);
        self.gateway_preferences = preferences;
        Ok(self)
    }

    /// Turn model discovery (`GET {base}/models`) on or off.
    pub fn with_model_discovery(mut self, enabled: bool) -> Self {
        self.model_discovery = enabled;
        self
    }

    /// List the models FastFlowLM has installed instead of its whole catalog.
    pub fn with_installed_models(mut self, installed: crate::fastflowlm::InstalledModels) -> Self {
        self.installed_models = Some(installed);
        self
    }

    /// The configured base URL (trimmed, without a trailing slash).
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Identify the upstream by provider type id, so streaming requests ask
    /// for usage when that upstream is known to accept it.
    pub fn with_provider_type(mut self, provider_type: impl Into<String>) -> Self {
        self.provider_type = Some(provider_type.into());
        self
    }

    /// Serve the gateway's decision models natively through `/v1/systemone`.
    pub fn with_systemone_gateway(mut self, gateway: crate::systemone::SystemOneGateway) -> Self {
        self.gateway_kind = gateway.gateway_kind();
        self.systemone_gateway = Some(std::sync::Arc::new(gateway));
        self
    }

    /// Attach custom HTTP headers sent with every request to this provider
    pub fn with_extra_headers(mut self, headers: HeaderMap) -> Self {
        self.extra_headers = headers;
        self
    }

    /// Build authorization header if API key is present
    fn auth_header(&self) -> Option<String> {
        self.api_key.as_ref().map(|key| format!("Bearer {}", key))
    }

    /// Apply auth + custom headers to an outgoing request.
    ///
    /// Custom headers are applied last and `RequestBuilder::headers` replaces
    /// same-named entries, so a user-supplied `Authorization` (or
    /// `Content-Type`) wins over the default instead of being sent twice.
    fn apply_headers(
        &self,
        mut request: reqwest_middleware::RequestBuilder,
    ) -> reqwest_middleware::RequestBuilder {
        if let Some(auth) = self.auth_header() {
            request = request.header("Authorization", auth);
        }
        if !self.extra_headers.is_empty() {
            request = request.headers(self.extra_headers.clone());
        }
        request
    }
}

impl OpenAICompatibleProvider {
    /// `GET {base}/models`, parsed, plus the gateway's decision models, and
    /// each listed model's raw entry (also kept for pricing).
    async fn fetch_listed_models(&self) -> AppResult<(Vec<ModelInfo>, ModelMetadata)> {
        let result = self.fetch_listed_models_uncached().await;
        match &result {
            Ok((_, metadata)) => self.listing.store(metadata.clone()),
            Err(_) => self.listing.mark_failed(),
        }
        result
    }

    async fn fetch_listed_models_uncached(&self) -> AppResult<(Vec<ModelInfo>, ModelMetadata)> {
        let request = self.apply_headers(self.client.get(format!("{}/models", self.base_url)));

        let response = request
            .send()
            .await
            .map_err(|e| AppError::Provider(format!("Failed to fetch models: {}", e)))?;

        if !response.status().is_success() {
            return Err(AppError::Provider(format!(
                "API returned status: {}",
                response.status()
            )));
        }

        let body = response
            .text()
            .await
            .map_err(|e| AppError::Provider(format!("Failed to read models response: {}", e)))?;

        let model_list: Vec<OpenAIModel> = parse_models_response(&body)
            .map_err(|e| AppError::Provider(format!("Failed to parse models response: {}", e)))?;
        let metadata = serde_json::from_str::<serde_json::Value>(&body)
            .map(|v| crate::listing_metadata::entries_by_id(&v))
            .unwrap_or_default();

        let mut models: Vec<ModelInfo> = model_list
            .into_iter()
            .map(|model| listed_model_info(model, &self.name))
            .collect();
        if let Some(gateway) = &self.systemone_gateway {
            gateway.merge_into(&self.name, &mut models).await;
        }
        Ok((models, std::sync::Arc::new(metadata)))
    }

    /// The price a model has in the server's model listing, if it lists
    /// one. Fetches the listing once when it was never fetched.
    async fn listed_pricing(&self, model: &str) -> Option<PricingInfo> {
        if self.listing.get().is_none() && self.model_discovery && self.listing.should_fetch() {
            let _ = self.fetch_listed_models().await;
        }
        self.listing
            .get()?
            .get(model)
            .and_then(crate::listing_metadata::listing_pricing)
    }
}

// OpenAI API response types (reused from OpenAI provider)

#[derive(Debug, Deserialize)]
struct OpenAIModel {
    id: String,
    /// Display name, when the listing has one (e.g. Free.ai)
    #[serde(default)]
    name: Option<String>,
    /// Model kind, when the listing declares one (e.g. Free.ai: `chat`,
    /// `embeddings`, `image`, `tts`, `vision`)
    #[serde(default, rename = "type")]
    kind: Option<String>,
    // The fields below are not used by LocalRouter, but the OpenAI spec includes them.
    // GitHub Models, Cloudflare Workers AI, and DigitalOcean Gradient omit some/all of
    // them, so they must be optional with serde defaults to avoid parse failures.
    #[allow(dead_code)]
    #[serde(default)]
    object: Option<String>,
    #[allow(dead_code)]
    #[serde(default)]
    created: Option<i64>,
    #[allow(dead_code)]
    #[serde(default)]
    owned_by: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenAIModelsResponse {
    data: Vec<OpenAIModel>,
}

/// Cloudflare Workers AI returns `{"result": [...], "success": true, ...}` instead
/// of the OpenAI `{"data": [...]}` envelope.
#[derive(Debug, Deserialize)]
struct CloudflareModelsResponse {
    result: Vec<OpenAIModel>,
}

/// Free.ai returns `{"models": [...]}`.
#[derive(Debug, Deserialize)]
struct ModelsEnvelopeResponse {
    models: Vec<OpenAIModel>,
}

/// Try the known model-list response shapes, in order.
fn parse_models_response(body: &str) -> Result<Vec<OpenAIModel>, serde_json::Error> {
    serde_json::from_str::<OpenAIModelsResponse>(body)
        .map(|r| r.data)
        .or_else(|_| serde_json::from_str::<Vec<OpenAIModel>>(body))
        .or_else(|_| serde_json::from_str::<CloudflareModelsResponse>(body).map(|r| r.result))
        .or_else(|_| serde_json::from_str::<ModelsEnvelopeResponse>(body).map(|r| r.models))
}

/// Capabilities of a listed model from the kind its listing declares.
/// Only kinds with an unambiguous meaning are mapped; anything else (or no
/// kind) is a chat model.
fn listed_capabilities(kind: Option<&str>) -> Vec<Capability> {
    match kind {
        Some("embeddings") => vec![Capability::Embedding],
        Some("image") => vec![Capability::ImageGeneration],
        Some("tts") => vec![Capability::TextToSpeech],
        Some("vision") => vec![Capability::Chat, Capability::Completion, Capability::Vision],
        _ => vec![Capability::Chat, Capability::Completion],
    }
}

/// A listed model as LocalRouter describes it.
fn listed_model_info(model: OpenAIModel, provider: &str) -> ModelInfo {
    let name = model
        .name
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| model.id.clone());
    ModelInfo {
        capabilities: listed_capabilities(model.kind.as_deref()),
        id: model.id,
        name,
        provider: provider.to_string(),
        parameter_count: None, // Not available from API
        context_window: 4096,  // Default, actual value depends on model
        supports_streaming: true,
        detailed_capabilities: None,
    }
    .enrich_with_catalog_by_name() // Model-only search for multi-provider system
}

/// Health from the status `GET {base}/models` answered with. Any reply
/// means the server is up; servers without a model listing answer 404,
/// so only auth failures, rate limits and server errors are reported.
fn health_from_status(status: reqwest::StatusCode, latency_ms: u64) -> ProviderHealth {
    let (status, error_message) = match status.as_u16() {
        401 | 403 => (
            HealthStatus::Unhealthy,
            Some(format!("Authentication failed (HTTP {})", status)),
        ),
        429 => (
            HealthStatus::Degraded,
            Some("Rate limited (HTTP 429)".to_string()),
        ),
        _ if status.is_server_error() => (
            HealthStatus::Degraded,
            Some(format!("Server error (HTTP {})", status)),
        ),
        _ => (HealthStatus::Healthy, None),
    };
    ProviderHealth {
        status,
        latency_ms: Some(latency_ms),
        last_checked: Utc::now(),
        error_message,
    }
}

#[derive(Debug, Serialize)]
struct OpenAIChatRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    service_tier: Option<String>,
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
    logprobs: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_logprobs: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    n: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    logit_bias: Option<std::collections::HashMap<String, f32>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    parallel_tool_calls: Option<bool>,
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
}

#[derive(Debug, Deserialize)]
struct OpenAIChoice {
    index: u32,
    message: ChatMessage,
    finish_reason: Option<String>,
    #[serde(default)]
    logprobs: Option<serde_json::Value>,
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
    /// Upstream usage, when the server reports it (usually on the last
    /// chunk or a usage-only chunk with empty `choices`).
    #[serde(default)]
    usage: Option<serde_json::Value>,
    /// Groq-hosted endpoints report usage under `x_groq.usage`.
    #[serde(default)]
    x_groq: Option<serde_json::Value>,
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
    /// Reasoning/thinking content from reasoning models
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

#[async_trait]
impl ModelProvider for OpenAICompatibleProvider {
    fn name(&self) -> &str {
        &self.name
    }

    /// The generic provider bills to whichever known API its base URL points
    /// at (an OpenAI-compatible provider on `api.groq.com` is Groq API
    /// usage). Typed providers bill to their type's account.
    fn usage_account(&self) -> Option<lr_usage::AccountRef> {
        match self.provider_type.as_deref() {
            None | Some("openai_compatible") => {
                lr_usage::providers::account_for_base_url(&self.base_url)
            }
            Some(_) => None,
        }
    }

    /// LLM Gateway `GET /v1/key` and Vercel AI Gateway `GET /v1/credits`
    /// (both documented, with the inference key) — only while the base URL
    /// is the provider's own API, so a key is never sent elsewhere.
    async fn fetch_usage(
        &self,
    ) -> Option<Result<lr_usage::UsageReport, lr_usage::fetch::FetchError>> {
        type Parse = fn(&serde_json::Value) -> Option<lr_usage::UsageReport>;
        let family = self.provider_type.as_deref()?;
        let (path, parse): (&str, Parse) = match family {
            "llmgateway" => ("key", lr_usage::bodies::parse_llmgateway_key),
            "vercel_ai_gateway" => ("credits", lr_usage::bodies::parse_vercel_credits),
            _ => return None,
        };
        let points_home = lr_usage::providers::account_for_base_url(&self.base_url)
            .is_some_and(|a| a.provider == family);
        let key = self.api_key.as_deref().filter(|_| points_home)?;
        let url = format!("{}/{path}", self.base_url.trim_end_matches('/'));
        let request = self.client.get(&url).bearer_auth(key);
        Some(crate::http_client::fetch_usage_json(request, parse).await)
    }

    async fn health_check(&self) -> ProviderHealth {
        let start = Instant::now();

        // Use /models endpoint for health check
        let request = self.apply_headers(self.client.get(format!("{}/models", self.base_url)));

        let result = request.send().await;

        let latency_ms = start.elapsed().as_millis() as u64;

        match result {
            Ok(response) => health_from_status(response.status(), latency_ms),
            Err(e) => ProviderHealth {
                status: HealthStatus::Unhealthy,
                latency_ms: None,
                last_checked: Utc::now(),
                error_message: Some(format!("Connection failed: {}", e)),
            },
        }
    }

    async fn list_models(&self) -> AppResult<Vec<ModelInfo>> {
        Ok(self.list_model_listing().await?.models)
    }

    async fn list_model_listing(&self) -> AppResult<ModelListing> {
        if !self.model_discovery {
            return Ok(ModelListing::default());
        }
        if let Some(installed) = &self.installed_models {
            if let Some(models) = installed.detect(&self.name, &self.base_url).await {
                return Ok(ModelListing {
                    models,
                    ..Default::default()
                });
            }
            // The server lists every model it supports, installed or not;
            // offer them only once the user picks.
            let (models, metadata) = self.fetch_listed_models().await?;
            return Ok(ModelListing {
                models,
                new_models_hidden: true,
                metadata,
            });
        }
        let (models, metadata) = self.fetch_listed_models().await?;
        Ok(ModelListing {
            models,
            new_models_hidden: false,
            metadata,
        })
    }

    async fn get_pricing(&self, model: &str) -> AppResult<PricingInfo> {
        // Gateways that publish prices in their model listing.
        if let Some(gateway) = &self.systemone_gateway {
            if let Some(pricing) = gateway.pricing(model).await {
                return Ok(pricing);
            }
        }
        // Prices the server lists for the model (per token, e.g. the
        // OpenRouter-style `pricing` object); otherwise free unless a price
        // is set for the model on its Models tab.
        Ok(self
            .listed_pricing(model)
            .await
            .unwrap_or_else(PricingInfo::free))
    }

    fn supports_systemone(&self) -> bool {
        self.systemone_gateway.is_some()
    }

    fn supports_decisions(&self) -> bool {
        self.systemone_gateway
            .as_ref()
            .is_some_and(|g| g.supports_decisions())
    }

    fn supports_gateway_options(&self) -> bool {
        self.gateway_kind.is_some()
    }
    fn validate_gateway_options(&self, request: &CompletionRequest) -> AppResult<()> {
        let Some(kind) = self.gateway_kind else {
            if request
                .extensions
                .as_ref()
                .is_some_and(|e| e.contains_key(crate::gateway::EXTENSION))
            {
                return Err(AppError::InvalidParams(
                    "Provider does not support gateway routing controls".into(),
                ));
            }
            return Ok(());
        };
        self.gateway_preferences
            .effective_for_chat(request)?
            .apply(kind, &mut serde_json::json!({"model":request.model}))?;
        Ok(())
    }

    async fn validate_systemone_gateway_options(
        &self,
        request: &crate::systemone::SystemOneRequest,
    ) -> AppResult<()> {
        if self.gateway_preferences == Default::default()
            && !request.extra.contains_key("service_tier")
            && !request
                .extra
                .get("extensions")
                .is_some_and(|e| e.get(crate::gateway::EXTENSION).is_some())
        {
            return Ok(());
        }
        if let Some(gateway) = &self.systemone_gateway {
            if gateway
                .is_decision_model(request.model.as_deref().unwrap_or_default())
                .await
            {
                return gateway.validate_options(request);
            }
        }
        let mut completion =
            CompletionRequest::new(request.model.clone().unwrap_or_default(), Vec::new());
        completion.extensions = request
            .extra
            .get("extensions")
            .and_then(serde_json::Value::as_object)
            .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect());
        self.validate_gateway_options(&completion)
    }

    async fn supports_systemone_model(&self, model: &str) -> bool {
        match &self.systemone_gateway {
            Some(gateway) => gateway.is_decision_model(model).await,
            None => false,
        }
    }

    async fn systemone(
        &self,
        request: crate::SystemOneRequest,
    ) -> AppResult<crate::SystemOneResponse> {
        match &self.systemone_gateway {
            Some(gateway) => gateway.systemone(request).await,
            None => Err(AppError::Provider(format!(
                "Provider '{}' does not support system one decisions",
                self.name
            ))),
        }
    }

    async fn complete(&self, request: CompletionRequest) -> AppResult<CompletionResponse> {
        if request
            .extensions
            .as_ref()
            .is_some_and(|e| e.contains_key(crate::gateway::EXTENSION))
            && self.gateway_kind.is_none()
        {
            return Err(AppError::InvalidParams(
                "This provider does not support gateway routing controls".into(),
            ));
        }
        let preferences = if self.gateway_kind.is_some() {
            self.gateway_preferences.effective_for_chat(&request)?
        } else {
            self.gateway_preferences
                .effective(request.extensions.as_ref())?
        };
        let openai_request = OpenAIChatRequest {
            service_tier: request.service_tier.filter(|_| self.gateway_kind.is_none()),
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
            logprobs: request.logprobs,
            top_logprobs: request.top_logprobs,
            n: request.n,
            logit_bias: request.logit_bias,
            parallel_tool_calls: request.parallel_tool_calls,
            reasoning_effort: request.reasoning_effort,
        };

        let mut body = crate::gateway::wire_body(&openai_request)?;
        let gateway_headers = self
            .gateway_kind
            .map(|kind| preferences.apply(kind, &mut body))
            .transpose()?
            .unwrap_or_default();
        let req = self
            .apply_headers(
                self.client
                    .post(format!("{}/chat/completions", self.base_url))
                    .header("Content-Type", "application/json")
                    .json(&body),
            )
            .headers(gateway_headers);

        let response = req
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

        let raw: serde_json::Value = response
            .json()
            .await
            .map_err(|e| AppError::Provider(e.to_string()))?;
        let attribution = self
            .gateway_kind
            .map(|kind| GatewayAttribution::from_wire(kind, &raw, preferences));
        let openai_response: OpenAIChatResponse = serde_json::from_value(raw)
            .map_err(|e| AppError::Provider(format!("Invalid response: {e}")))?;

        // Validate choices array is not empty
        let choices: Vec<CompletionChoice> = openai_response
            .choices
            .into_iter()
            .map(|choice| CompletionChoice {
                index: choice.index,
                message: choice.message,
                finish_reason: choice.finish_reason,
                logprobs: choice
                    .logprobs
                    .as_ref()
                    .and_then(super::Logprobs::from_wire),
            })
            .collect();

        if choices.is_empty() {
            return Err(AppError::Provider(
                "API returned no choices in response".to_string(),
            ));
        }

        Ok(CompletionResponse {
            id: openai_response.id,
            object: openai_response.object,
            created: openai_response.created,
            model: openai_response.model,
            provider: self.name().to_string(),
            choices,
            usage: TokenUsage {
                prompt_tokens: openai_response.usage.prompt_tokens,
                completion_tokens: openai_response.usage.completion_tokens,
                total_tokens: openai_response.usage.total_tokens,
                prompt_tokens_details: None,
                completion_tokens_details: None,
            },
            system_fingerprint: None,
            service_tier: attribution.as_ref().and_then(|a| a.service_tier.clone()),
            extensions: attribution.map(GatewayAttribution::extensions),
            routellm_win_rate: None,
            request_usage_entries: None,
        })
    }

    async fn stream_complete(
        &self,
        request: CompletionRequest,
    ) -> AppResult<Pin<Box<dyn Stream<Item = AppResult<CompletionChunk>> + Send>>> {
        if request
            .extensions
            .as_ref()
            .is_some_and(|e| e.contains_key(crate::gateway::EXTENSION))
            && self.gateway_kind.is_none()
        {
            return Err(AppError::InvalidParams(
                "This provider does not support gateway routing controls".into(),
            ));
        }
        let preferences = if self.gateway_kind.is_some() {
            self.gateway_preferences.effective_for_chat(&request)?
        } else {
            self.gateway_preferences
                .effective(request.extensions.as_ref())?
        };
        let gateway_kind = self.gateway_kind;
        let openai_request = OpenAIChatRequest {
            service_tier: request.service_tier.filter(|_| self.gateway_kind.is_none()),
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
            logprobs: request.logprobs,
            top_logprobs: request.top_logprobs,
            n: request.n,
            logit_bias: request.logit_bias,
            parallel_tool_calls: request.parallel_tool_calls,
            reasoning_effort: request.reasoning_effort,
        };

        let mut body = stream_usage::streaming_body(
            &openai_request,
            self.provider_type.as_deref().unwrap_or_default(),
        )?;
        let gateway_headers = gateway_kind
            .map(|kind| preferences.apply(kind, &mut body))
            .transpose()?
            .unwrap_or_default();
        let req = self
            .apply_headers(
                self.client
                    .post(format!("{}/chat/completions", self.base_url))
                    .header("Content-Type", "application/json")
                    .json(&body),
            )
            .headers(gateway_headers);

        let response = req
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

        // Parse SSE (Server-Sent Events) stream
        // Use flat_map to handle multiple SSE events in a single byte chunk
        // Buffer incomplete lines across HTTP chunks

        let lines = crate::sse_lines::line_batches(response.bytes_stream());
        let mut latest_attribution: Option<GatewayAttribution> = None;
        let stream = lines.flat_map(move |result| {
            let chunks: Vec<AppResult<CompletionChunk>> = match result {
                Ok(lines) => {
                    let mut parsed_chunks = Vec::new();

                    for line in lines {
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
                                    let mut attribution = gateway_kind.and_then(|kind| {
                                        serde_json::from_str::<serde_json::Value>(json_str)
                                            .ok()
                                            .map(|raw| {
                                                GatewayAttribution::from_wire(
                                                    kind,
                                                    &raw,
                                                    preferences.clone(),
                                                )
                                            })
                                    });
                                    if let Some(current) = attribution.as_mut() {
                                        if let Some(previous) = &latest_attribution {
                                            current.retain_stream_metadata(previous);
                                        }
                                        latest_attribution = Some(current.clone());
                                    }
                                    parsed_chunks.push(Ok(CompletionChunk {
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
                                        extensions: attribution.map(GatewayAttribution::extensions),
                                        usage: stream_usage::chunk_usage(
                                            openai_chunk.usage.as_ref(),
                                            openai_chunk.x_groq.as_ref(),
                                        ),
                                        provider: None,
                                    }));
                                }
                                Err(e) => {
                                    parsed_chunks.push(Err(AppError::Provider(format!(
                                        "Failed to parse chunk: {}",
                                        e
                                    ))));
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

        Ok(stream_usage::usage_once_at_end(stream))
    }

    fn supports_embeddings(&self) -> bool {
        true
    }

    fn get_feature_support(&self, instance_name: &str) -> super::ProviderFeatureSupport {
        let mut support = super::default_feature_support(self, instance_name);

        // OpenAI-compatible providers — support depends on upstream server capabilities
        for f in &mut support.model_features {
            if f.support == super::SupportLevel::NotSupported {
                f.support = super::SupportLevel::Partial;
                f.notes = Some(format!(
                    "May be available depending on upstream server; {} is not guaranteed by generic OpenAI-compatible API",
                    f.name
                ));
            }
        }
        for e in &mut support.endpoints {
            if e.support == super::SupportLevel::NotImplemented {
                e.support = super::SupportLevel::Partial;
                e.notes = Some(format!(
                    "May be available depending on upstream server; {} is not guaranteed by generic OpenAI-compatible API",
                    e.name
                ));
            }
        }

        support
    }

    async fn embed(&self, request: super::EmbeddingRequest) -> AppResult<super::EmbeddingResponse> {
        // Convert our generic EmbeddingRequest to OpenAI-specific format
        let input = match request.input {
            super::EmbeddingInput::Single(text) => OpenAIEmbeddingInput::Single(text),
            super::EmbeddingInput::Multiple(texts) => OpenAIEmbeddingInput::Multiple(texts),
            super::EmbeddingInput::Tokens(_) => {
                return Err(AppError::Provider(
                    "OpenAI-compatible embeddings do not support pre-tokenized input".to_string(),
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

        let http_request = self.apply_headers(
            self.client
                .post(format!("{}/embeddings", self.base_url))
                .header("Content-Type", "application/json")
                .json(&openai_request),
        );

        let response = http_request
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn gateway_preferences_reach_normal_and_streaming_transport() {
        use futures::StreamExt;
        use wiremock::matchers::{body_partial_json, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        let provider = OpenAICompatibleProvider::new("test".into(), server.uri(), None)
            .with_gateway_preferences(
                GatewayKind::Vercel,
                serde_json::from_value(serde_json::json!({"only":["openai"],"zdr":true})).unwrap(),
            )
            .unwrap();
        let request: CompletionRequest = serde_json::from_value(serde_json::json!({"model":"model","messages":[{"role":"user","content":"hi"}],"stream":false,"extensions":{"localrouter.gateway":{"only":["openai"],"zdr":false}}})).unwrap();
        Mock::given(method("POST")).and(path("/chat/completions"))
            .and(body_partial_json(serde_json::json!({"stream":false,"providerOptions":{"gateway":{"only":["openai"],"zeroDataRetention":true}}})))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"id":"normal","object":"chat.completion","created":1,"model":"model","choices":[{"index":0,"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}],"usage":{"prompt_tokens":3,"completion_tokens":1,"total_tokens":4},"provider_metadata":{"gateway":{"cost":"0.03","routing":{"finalProvider":"openai"}}}}))).expect(1).mount(&server).await;
        let response = provider.complete(request.clone()).await.unwrap();
        assert_eq!(
            crate::gateway::reported_cost(response.extensions.as_ref()),
            Some(0.03)
        );
        Mock::given(method("POST")).and(path("/chat/completions"))
            .and(body_partial_json(serde_json::json!({"stream":true,"providerOptions":{"gateway":{"zeroDataRetention":true}}})))
            .respond_with(ResponseTemplate::new(200).set_body_string("data: {\"id\":\"stream\",\"object\":\"chat.completion.chunk\",\"created\":1,\"model\":\"model\",\"choices\":[],\"usage\":{\"prompt_tokens\":3,\"completion_tokens\":1,\"total_tokens\":4},\"provider_metadata\":{\"gateway\":{\"cost\":\"0.04\"}}}\n\ndata: [DONE]\n\n").insert_header("Content-Type","text/event-stream"))
            .expect(1).mount(&server).await;
        let chunks = provider
            .stream_complete(request)
            .await
            .unwrap()
            .collect::<Vec<_>>()
            .await;
        assert!(chunks.iter().any(|c| crate::gateway::reported_cost(
            c.as_ref().unwrap().extensions.as_ref()
        ) == Some(0.04)));
        server.verify().await;
    }
    #[tokio::test]
    async fn gateway_fallback_policy_overrides_conflicting_custom_header() {
        use wiremock::matchers::{body_partial_json, header};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(header("x-no-fallback", "true"))
            .and(body_partial_json(serde_json::json!({"model":"openai/m"})))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"id":"x","object":"chat.completion","created":1,"model":"m","choices":[{"index":0,"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}))).expect(1).mount(&server).await;
        let provider = OpenAICompatibleProvider::new("test".into(), server.uri(), None)
            .with_extra_headers(parse_custom_headers("X-No-Fallback: false").unwrap())
            .with_gateway_preferences(
                GatewayKind::Llmgateway,
                serde_json::from_value(
                    serde_json::json!({"only":["openai"],"allow_fallbacks":false}),
                )
                .unwrap(),
            )
            .unwrap();
        provider
            .complete(CompletionRequest::new("m", Vec::new()))
            .await
            .unwrap();
        server.verify().await;
    }
    #[test]
    fn generic_provider_bills_to_the_api_its_base_url_points_at() {
        let generic = |url: &str| {
            OpenAICompatibleProvider::new("x".into(), url.into(), None)
                .with_provider_type("openai_compatible")
                .usage_account()
                .map(|a| a.id())
        };
        assert_eq!(
            generic("https://api.groq.com/openai/v1").as_deref(),
            Some("groq:api")
        );
        assert_eq!(generic("http://localhost:8080/v1"), None);
        // Typed providers bill to their type's account (decided elsewhere).
        let typed = OpenAICompatibleProvider::new(
            "x".into(),
            "https://api.groq.com/openai/v1".into(),
            None,
        )
        .with_provider_type("huggingface");
        assert_eq!(typed.usage_account(), None);
    }

    #[tokio::test]
    async fn usage_endpoints_only_for_their_own_host() {
        let gateway = |kind: &str, url: &str, key: Option<&str>| {
            OpenAICompatibleProvider::new("x".into(), url.into(), key.map(str::to_string))
                .with_provider_type(kind)
        };
        // A gateway pointed elsewhere never sends its key there.
        assert!(gateway("llmgateway", "https://example.com/v1", Some("k"))
            .fetch_usage()
            .await
            .is_none());
        // No key, no request.
        assert!(
            gateway("vercel_ai_gateway", "https://ai-gateway.vercel.sh/v1", None)
                .fetch_usage()
                .await
                .is_none()
        );
        // Providers without a documented endpoint.
        assert!(
            gateway("huggingface", "https://router.huggingface.co/v1", Some("k"))
                .fetch_usage()
                .await
                .is_none()
        );
    }

    #[test]
    fn test_provider_name() {
        let provider = OpenAICompatibleProvider::new(
            "my-local-ai".to_string(),
            "http://localhost:8080/v1".to_string(),
            None,
        );
        assert_eq!(provider.name(), "my-local-ai");
    }

    #[test]
    fn test_auth_header_with_key() {
        let provider = OpenAICompatibleProvider::new(
            "test".to_string(),
            "http://localhost:8080/v1".to_string(),
            Some("test-key-123".to_string()),
        );
        assert_eq!(
            provider.auth_header(),
            Some("Bearer test-key-123".to_string())
        );
    }

    #[test]
    fn test_auth_header_without_key() {
        let provider = OpenAICompatibleProvider::new(
            "test".to_string(),
            "http://localhost:8080/v1".to_string(),
            None,
        );
        assert_eq!(provider.auth_header(), None);
    }

    #[test]
    fn test_base_url_trailing_slash() {
        let provider = OpenAICompatibleProvider::new(
            "test".to_string(),
            "http://localhost:8080/v1/".to_string(),
            None,
        );
        assert_eq!(provider.base_url, "http://localhost:8080/v1");
    }

    #[tokio::test]
    async fn test_pricing_is_free() {
        let provider = OpenAICompatibleProvider::new(
            "test".to_string(),
            "http://localhost:8080/v1".to_string(),
            None,
        )
        .with_model_discovery(false);
        let pricing = provider.get_pricing("any-model").await.unwrap();
        assert_eq!(pricing.input_cost_per_1k, 0.0);
        assert_eq!(pricing.output_cost_per_1k, 0.0);
    }

    #[tokio::test]
    async fn listing_entries_and_prices_come_from_the_model_list() {
        use wiremock::{matchers, Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(matchers::path("/models"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"models":[
                    {"id":"paid","tier":"pro","pricing":{"input":"0.000002","output":"0.000004"}},
                    {"id":"free","tier":"free","pricing":{"input":0}},
                    {"id":"plain"}
                ]}"#,
            ))
            .mount(&server)
            .await;
        let provider = OpenAICompatibleProvider::new("x".into(), server.uri(), None);
        // Pricing fetches the list once and keeps it.
        let paid = provider.get_pricing("paid").await.unwrap();
        assert!((paid.input_cost_per_1k - 0.002).abs() < 1e-12);
        assert!((paid.output_cost_per_1k - 0.004).abs() < 1e-12);
        assert_eq!(
            provider
                .get_pricing("free")
                .await
                .unwrap()
                .input_cost_per_1k,
            0.0
        );
        assert_eq!(
            provider
                .get_pricing("plain")
                .await
                .unwrap()
                .input_cost_per_1k,
            0.0
        );
        assert_eq!(
            provider
                .get_pricing("unknown")
                .await
                .unwrap()
                .input_cost_per_1k,
            0.0
        );
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn listing_keeps_each_models_entry() {
        use wiremock::{matchers, Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(matchers::path("/models"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(r#"{"data":[{"id":"a","tier":"free"},{"id":"b"}]}"#),
            )
            .mount(&server)
            .await;
        let provider = OpenAICompatibleProvider::new("x".into(), server.uri(), None);
        let listing = provider.list_model_listing().await.unwrap();
        assert_eq!(listing.metadata.len(), 2);
        assert_eq!(listing.metadata["a"]["tier"], "free");
    }

    #[tokio::test]
    async fn failed_listing_is_not_retried_for_every_price() {
        use wiremock::{matchers, Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(matchers::path("/models"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;
        let provider = OpenAICompatibleProvider::new("x".into(), server.uri(), None);
        for _ in 0..3 {
            assert_eq!(
                provider.get_pricing("m").await.unwrap().input_cost_per_1k,
                0.0
            );
        }
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }

    #[test]
    fn test_parse_models_openai_envelope() {
        let body = r#"{"object":"list","data":[
            {"id":"gpt-4o","object":"model","created":1234,"owned_by":"openai"},
            {"id":"gpt-4o-mini","object":"model","created":1235,"owned_by":"openai"}
        ]}"#;
        let models = parse_models_response(body).unwrap();
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].id, "gpt-4o");
        assert_eq!(models[1].id, "gpt-4o-mini");
    }

    #[test]
    fn test_parse_models_bare_array_with_optional_fields() {
        // GitHub Models returns a bare array and may omit object/created/owned_by.
        let body = r#"[
            {"id":"openai/gpt-4o"},
            {"id":"meta/llama-3.1-70b-instruct"}
        ]"#;
        let models = parse_models_response(body).unwrap();
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].id, "openai/gpt-4o");
        assert!(models[0].object.is_none());
        assert!(models[0].created.is_none());
        assert!(models[0].owned_by.is_none());
    }

    #[test]
    fn test_parse_models_cloudflare_envelope() {
        // Cloudflare Workers AI: {"result": [...], "success": true, ...}.
        let body = r#"{
            "result":[{"id":"@cf/meta/llama-3.1-8b-instruct"}],
            "success":true,
            "errors":[],
            "messages":[]
        }"#;
        let models = parse_models_response(body).unwrap();
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "@cf/meta/llama-3.1-8b-instruct");
    }

    #[test]
    fn test_parse_models_free_ai_envelope() {
        // Free.ai: {"models": [...]} with a display name, kind and prices.
        let body = r#"{"models":[
            {"id":"llama-3.3-70b","name":"Llama 3.3 70B","type":"chat","pricing":{"input":0}},
            {"id":"bge-m3","name":"BGE M3","type":"embeddings"},
            {"id":"flux","type":"image"}
        ]}"#;
        let models = parse_models_response(body).unwrap();
        assert_eq!(models.len(), 3);
        let infos: Vec<ModelInfo> = models
            .into_iter()
            .map(|m| listed_model_info(m, "free"))
            .collect();
        assert_eq!(infos[0].name, "Llama 3.3 70B");
        assert!(infos[0].capabilities.contains(&Capability::Chat));
        assert_eq!(infos[1].capabilities[0], Capability::Embedding);
        assert!(!infos[1].capabilities.contains(&Capability::Chat));
        assert_eq!(infos[2].name, "flux");
        assert_eq!(infos[2].capabilities[0], Capability::ImageGeneration);
        assert!(infos.iter().all(|m| m.provider == "free"));
    }

    #[test]
    fn listed_kinds_map_only_the_declared_set() {
        assert_eq!(
            listed_capabilities(Some("tts")),
            vec![Capability::TextToSpeech]
        );
        assert_eq!(
            listed_capabilities(Some("vision")),
            vec![Capability::Chat, Capability::Completion, Capability::Vision]
        );
        for other in [
            None,
            Some("chat"),
            Some("audio"),
            Some("Embeddings"),
            Some(""),
        ] {
            assert_eq!(
                listed_capabilities(other),
                vec![Capability::Chat, Capability::Completion],
                "{other:?}"
            );
        }
    }

    #[test]
    fn health_counts_any_reply_but_auth_limits_and_server_errors() {
        use reqwest::StatusCode as S;
        for ok in [S::OK, S::NOT_FOUND, S::METHOD_NOT_ALLOWED, S::BAD_REQUEST] {
            assert_eq!(
                health_from_status(ok, 1).status,
                HealthStatus::Healthy,
                "{ok}"
            );
        }
        for bad in [S::UNAUTHORIZED, S::FORBIDDEN] {
            assert_eq!(health_from_status(bad, 1).status, HealthStatus::Unhealthy);
        }
        for degraded in [
            S::TOO_MANY_REQUESTS,
            S::INTERNAL_SERVER_ERROR,
            S::BAD_GATEWAY,
        ] {
            assert_eq!(
                health_from_status(degraded, 1).status,
                HealthStatus::Degraded
            );
        }
    }

    #[tokio::test]
    async fn server_without_a_model_list_is_healthy() {
        use wiremock::{matchers, Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(matchers::path("/models"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        let provider = OpenAICompatibleProvider::new("x".into(), server.uri(), None);
        assert_eq!(provider.health_check().await.status, HealthStatus::Healthy);
    }

    #[tokio::test]
    async fn discovery_off_makes_no_request() {
        use wiremock::{matchers, Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(matchers::path("/models"))
            .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"data":[{"id":"a"}]}"#))
            .expect(0)
            .mount(&server)
            .await;
        let provider = OpenAICompatibleProvider::new("x".into(), server.uri(), None)
            .with_model_discovery(false);
        assert!(provider.list_models().await.unwrap().is_empty());
        let listing = provider.list_model_listing().await.unwrap();
        assert!(listing.models.is_empty());
        assert!(!listing.new_models_hidden);
    }

    #[tokio::test]
    async fn fastflowlm_without_installed_models_hides_its_catalog() {
        use wiremock::{matchers, Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(matchers::path("/models"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"object":"list","data":[{"id":"llama3.2:1b"},{"id":"qwen3:4b"}]}"#,
            ))
            .mount(&server)
            .await;
        let provider = OpenAICompatibleProvider::new("flm".into(), server.uri(), None)
            .with_provider_type("fastflowlm")
            .with_installed_models(crate::fastflowlm::InstalledModels::unavailable());
        let listing = provider.list_model_listing().await.unwrap();
        assert_eq!(listing.models.len(), 2);
        assert!(listing.new_models_hidden);
        // A generic provider's listing is trusted.
        let generic = OpenAICompatibleProvider::new("g".into(), server.uri(), None);
        assert!(
            !generic
                .list_model_listing()
                .await
                .unwrap()
                .new_models_hidden
        );
    }

    #[test]
    fn test_parse_models_invalid_json_fails() {
        let body = "not json";
        assert!(parse_models_response(body).is_err());
    }

    #[test]
    fn test_parse_custom_headers_valid() {
        let raw = "X-Api-Version: 2024-01-01\nX-Tenant-Id: acme";
        let headers = parse_custom_headers(raw).unwrap();
        assert_eq!(headers.len(), 2);
        assert_eq!(headers.get("x-api-version").unwrap(), "2024-01-01");
        assert_eq!(headers.get("X-Tenant-Id").unwrap(), "acme");
    }

    #[test]
    fn test_parse_custom_headers_skips_empty_lines_and_trims() {
        let raw = "\n  X-Foo :  bar baz  \n\n";
        let headers = parse_custom_headers(raw).unwrap();
        assert_eq!(headers.len(), 1);
        assert_eq!(headers.get("x-foo").unwrap(), "bar baz");
    }

    #[test]
    fn test_parse_custom_headers_value_may_contain_colons() {
        let raw = "X-Endpoint: https://example.com:8443/path";
        let headers = parse_custom_headers(raw).unwrap();
        assert_eq!(
            headers.get("x-endpoint").unwrap(),
            "https://example.com:8443/path"
        );
    }

    #[test]
    fn test_parse_custom_headers_repeated_name_kept() {
        let raw = "X-Multi: one\nX-Multi: two";
        let headers = parse_custom_headers(raw).unwrap();
        let values: Vec<_> = headers.get_all("x-multi").iter().collect();
        assert_eq!(values, vec!["one", "two"]);
    }

    #[test]
    fn test_parse_custom_headers_empty_input() {
        assert!(parse_custom_headers("").unwrap().is_empty());
        assert!(parse_custom_headers("  \n \n").unwrap().is_empty());
    }

    #[test]
    fn test_parse_custom_headers_missing_colon_fails() {
        let err = parse_custom_headers("NotAHeader").unwrap_err();
        assert!(err.to_string().contains("expected 'Name: Value'"));
    }

    #[test]
    fn test_parse_custom_headers_invalid_name_fails() {
        assert!(parse_custom_headers("Bad Name: value").is_err());
        // Empty name (line starting with ':')
        assert!(parse_custom_headers(": value").is_err());
    }

    fn build_headers(provider: &OpenAICompatibleProvider) -> reqwest::header::HeaderMap {
        provider
            .apply_headers(provider.client.get("http://localhost:8080/v1/models"))
            .build()
            .unwrap()
            .headers()
            .clone()
    }

    #[test]
    fn test_apply_headers_sends_custom_headers_and_auth() {
        let provider = OpenAICompatibleProvider::new(
            "test".to_string(),
            "http://localhost:8080/v1".to_string(),
            Some("test-key".to_string()),
        )
        .with_extra_headers(parse_custom_headers("X-Custom: abc").unwrap());

        let headers = build_headers(&provider);
        assert_eq!(headers.get("X-Custom").unwrap(), "abc");
        assert_eq!(headers.get("Authorization").unwrap(), "Bearer test-key");
    }

    #[test]
    fn test_apply_headers_custom_authorization_takes_precedence() {
        let provider = OpenAICompatibleProvider::new(
            "test".to_string(),
            "http://localhost:8080/v1".to_string(),
            Some("test-key".to_string()),
        )
        .with_extra_headers(parse_custom_headers("authorization: Custom scheme-token").unwrap());

        let headers = build_headers(&provider);
        let auth_values: Vec<_> = headers.get_all("authorization").iter().collect();
        assert_eq!(auth_values.len(), 1);
        assert_eq!(auth_values[0], "Custom scheme-token");
    }

    #[test]
    fn test_apply_headers_repeated_custom_header_sent_twice() {
        let provider = OpenAICompatibleProvider::new(
            "test".to_string(),
            "http://localhost:8080/v1".to_string(),
            None,
        )
        .with_extra_headers(parse_custom_headers("X-Multi: one\nX-Multi: two").unwrap());

        let headers = build_headers(&provider);
        let values: Vec<_> = headers.get_all("x-multi").iter().collect();
        assert_eq!(values, vec!["one", "two"]);
    }

    #[test]
    fn test_apply_headers_no_headers_no_key() {
        let provider = OpenAICompatibleProvider::new(
            "test".to_string(),
            "http://localhost:8080/v1".to_string(),
            None,
        );
        let headers = build_headers(&provider);
        assert!(headers.get("Authorization").is_none());
    }

    /// A generic endpoint is never asked (strict servers reject unknown fields), but usage it sends anyway is reported.
    #[tokio::test]
    async fn stream_reports_upstream_usage() {
        use crate::openai_compatible::stream_usage::test_support::*;

        let server = sse_server("/chat/completions", openai_stream(UsageAt::FinishChunk)).await;
        let provider = OpenAICompatibleProvider::new("custom".to_string(), server.uri(), None);
        let stream = provider
            .stream_complete(stream_request("test-model"))
            .await
            .unwrap();
        assert_openai_stream(&collect(stream).await);
        assert!(!asked_for_usage(&received_body(&server).await));
    }

    /// An endpoint identified as a type known to accept
    /// `stream_options.include_usage` is asked for usage.
    #[tokio::test]
    async fn stream_asks_for_usage_when_provider_type_accepts_it() {
        use crate::openai_compatible::stream_usage::test_support::*;

        let server = sse_server("/chat/completions", openai_stream(UsageAt::UsageOnlyChunk)).await;
        let provider = OpenAICompatibleProvider::new("embedded".to_string(), server.uri(), None)
            .with_provider_type("llamacpp_embedded");
        let stream = provider
            .stream_complete(stream_request("test-model"))
            .await
            .unwrap();
        assert_openai_stream(&collect(stream).await);
        assert!(asked_for_usage(&received_body(&server).await));
    }

    #[test]
    fn test_base_url_trims_whitespace() {
        let provider = OpenAICompatibleProvider::new(
            "test".to_string(),
            "  https://api.deepseek.com/v1  \n".to_string(),
            None,
        );
        assert_eq!(provider.base_url, "https://api.deepseek.com/v1");
    }

    #[test]
    fn test_base_url_trims_whitespace_and_trailing_slash() {
        let provider = OpenAICompatibleProvider::new(
            "test".to_string(),
            "  http://localhost:4891/v1/  ".to_string(),
            None,
        );
        assert_eq!(provider.base_url, "http://localhost:4891/v1");
    }
}
