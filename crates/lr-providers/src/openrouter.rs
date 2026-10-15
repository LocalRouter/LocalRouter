//! OpenRouter provider implementation
//!
//! Provides access to multiple AI providers through OpenRouter's unified API.

use async_trait::async_trait;
use chrono::Utc;
use futures::stream::{Stream, StreamExt};
use reqwest::StatusCode;
use reqwest_middleware::ClientWithMiddleware;
use serde::{Deserialize, Serialize};
use std::pin::Pin;
use std::time::Instant;

use crate::openai_compatible::stream_usage;
use lr_types::{AppError, AppResult};

use super::{
    Capability, ChatMessage, ChunkChoice, ChunkDelta, CompletionChoice, CompletionChunk,
    CompletionRequest, CompletionResponse, HealthStatus, ModelInfo, ModelListing, ModelProvider,
    PricingInfo, ProviderCreditsInfo, ProviderHealth, TokenUsage, Tool, ToolChoice,
};
use crate::gateway::{GatewayAttribution, GatewayKind, GatewayPreferences};
use crate::listing_metadata::ModelMetadata;

const OPENROUTER_API_BASE: &str = "https://openrouter.ai/api/v1";

/// OpenRouter provider implementation
pub struct OpenRouterProvider {
    client: ClientWithMiddleware,
    api_key: String,
    app_name: Option<String>,
    app_url: Option<String>,
    base_url: String,
    /// Hosted decision models (TypeSafe Jev) on `POST /systemone`.
    systemone: Option<std::sync::Arc<crate::systemone::SystemOneGateway>>,
    /// Each model's entry in the latest `GET /models` (prices, modalities)
    listing: crate::listing_metadata::ListingMetadataCache,
    gateway_preferences: GatewayPreferences,
}

#[allow(dead_code)]
impl OpenRouterProvider {
    /// Creates a new OpenRouter provider with the given API key
    pub fn new(api_key: String) -> Self {
        Self::with_base_url(api_key, OPENROUTER_API_BASE.to_string())
    }

    /// Creates a new OpenRouter provider with a custom base URL (for testing)
    pub fn with_base_url(api_key: String, base_url: String) -> Self {
        let base_url = base_url.trim_end_matches('/').to_string();
        let systemone = crate::systemone::SystemOneGateway::openrouter(
            &base_url,
            &api_key,
            reqwest::header::HeaderMap::new(),
        )
        .map_err(|e| tracing::warn!("OpenRouter System One support unavailable: {e}"))
        .ok()
        .map(std::sync::Arc::new);
        Self {
            client: crate::http_client::default_client(),
            api_key,
            app_name: Some("LocalRouter".to_string()),
            app_url: Some("https://github.com/localrouter/localrouter".to_string()),
            base_url,
            systemone,
            listing: Default::default(),
            gateway_preferences: Default::default(),
        }
    }

    pub fn with_gateway_preferences(mut self, preferences: GatewayPreferences) -> AppResult<Self> {
        let preferences = preferences.merge(&GatewayPreferences::default())?;
        if let Some(shared) = self.systemone.as_mut() {
            let gateway = std::sync::Arc::get_mut(shared).ok_or_else(|| {
                AppError::Config(
                    "Gateway preferences must be set before sharing the provider".into(),
                )
            })?;
            gateway.set_gateway_preferences(preferences.clone());
        }
        self.gateway_preferences = preferences;
        Ok(self)
    }

    /// Sets the app name for routing headers
    pub fn with_app_name(mut self, name: String) -> Self {
        self.app_name = Some(name);
        self
    }

    /// Sets the app URL for routing headers
    pub fn with_app_url(mut self, url: String) -> Self {
        self.app_url = Some(url);
        self
    }

    /// Create a new OpenRouter provider from stored API key
    ///
    /// # Arguments
    /// * `provider_name` - The provider name used to store the key (defaults to "openrouter")
    ///
    /// # Returns
    /// * `Ok(Self)` if key exists and provider created successfully
    /// * `Err(AppError)` if key doesn't exist or keyring access fails
    pub fn from_stored_key(provider_name: Option<&str>) -> AppResult<Self> {
        let name = provider_name.unwrap_or("openrouter");
        let api_key = super::key_storage::get_provider_key(name)?.ok_or_else(|| {
            AppError::Provider(format!("No API key found for provider '{}'", name))
        })?;
        Ok(Self::new(api_key))
    }

    /// Builds request with authentication and routing headers
    fn build_request(&self, url: &str) -> reqwest_middleware::RequestBuilder {
        let mut req = self
            .client
            .get(url)
            .header("Authorization", format!("Bearer {}", self.api_key));

        if let Some(ref app_url) = self.app_url {
            req = req.header("HTTP-Referer", app_url);
        }

        if let Some(ref app_name) = self.app_name {
            req = req.header("X-Title", app_name);
        }

        req
    }

    /// Builds POST request with authentication and routing headers
    fn build_post_request(
        &self,
        url: &str,
        server_tools: bool,
    ) -> AppResult<reqwest_middleware::RequestBuilder> {
        let client = if server_tools {
            crate::http_client::native_client()?
        } else {
            self.client.clone()
        };
        let mut req = client
            .post(url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json");

        if let Some(ref app_url) = self.app_url {
            req = req.header("HTTP-Referer", app_url);
        }

        if let Some(ref app_name) = self.app_name {
            req = req.header("X-Title", app_name);
        }

        Ok(req)
    }

    /// `GET /models`: the chat models and each model's raw entry (also kept
    /// for pricing).
    async fn fetch_listing(&self) -> AppResult<(Vec<ModelInfo>, ModelMetadata)> {
        let result = self.fetch_listing_uncached().await;
        match &result {
            Ok((_, metadata)) => self.listing.store(metadata.clone()),
            Err(_) => self.listing.mark_failed(),
        }
        result
    }

    async fn fetch_listing_uncached(&self) -> AppResult<(Vec<ModelInfo>, ModelMetadata)> {
        let url = format!("{}/models", self.base_url);

        let response = self
            .build_request(&url)
            .send()
            .await
            .map_err(|e| AppError::Provider(format!("Failed to fetch models: {}", e)))?;

        if !response.status().is_success() {
            return Err(AppError::Provider(format!(
                "OpenRouter API returned status: {}",
                response.status()
            )));
        }

        let body: serde_json::Value = response
            .json()
            .await
            .map_err(|e| AppError::Provider(format!("Failed to parse models response: {}", e)))?;
        let models_response: OpenRouterModelsResponse = serde_json::from_value(body.clone())
            .map_err(|e| AppError::Provider(format!("Failed to parse models response: {}", e)))?;

        let models = models_response
            .data
            .into_iter()
            .map(|model| {
                let capabilities = if model.id.contains("vision") {
                    vec![Capability::Chat, Capability::Vision]
                } else {
                    vec![Capability::Chat]
                };

                ModelInfo {
                    id: model.id.clone(),
                    name: model.name.unwrap_or_else(|| model.id.clone()),
                    provider: "openrouter".to_string(),
                    parameter_count: None,
                    context_window: model.context_length,
                    supports_streaming: true,
                    capabilities,
                    detailed_capabilities: None,
                }
                .enrich_with_catalog_by_name() // Use model-only search for multi-provider system
            })
            .collect();
        let metadata = std::sync::Arc::new(crate::listing_metadata::entries_by_id(&body));
        Ok((models, metadata))
    }
}

#[async_trait]
#[allow(dead_code)]
impl ModelProvider for OpenRouterProvider {
    fn server_tools_protocol(
        &self,
        request: &CompletionRequest,
    ) -> Option<crate::server_tools::ServerToolsProtocol> {
        request
            .native_messages
            .is_none()
            .then_some(crate::server_tools::ServerToolsProtocol::OpenRouter)
    }

    fn supports_no_retention_filter(&self) -> bool {
        true
    }
    fn native_batch_kind(&self) -> Option<crate::batches::NativeBatchKind> {
        Some(crate::batches::NativeBatchKind::Openrouter)
    }
    fn native_batch_transport(&self) -> AppResult<Option<crate::batches::NativeBatchTransport>> {
        crate::batches::NativeBatchTransport::openrouter(
            &self.base_url,
            &self.api_key,
            self.gateway_preferences.clone(),
            self.app_name.as_deref(),
            self.app_url.as_deref(),
        )
        .map(Some)
    }

    fn name(&self) -> &str {
        "openrouter"
    }

    async fn health_check(&self) -> ProviderHealth {
        let start = Instant::now();
        let url = format!("{}/models", self.base_url);

        match self.build_request(&url).send().await {
            Ok(response) => {
                let latency = start.elapsed().as_millis() as u64;
                match response.status() {
                    StatusCode::OK => ProviderHealth {
                        status: HealthStatus::Healthy,
                        latency_ms: Some(latency),
                        last_checked: Utc::now(),
                        error_message: None,
                    },
                    StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => ProviderHealth {
                        status: HealthStatus::Unhealthy,
                        latency_ms: Some(latency),
                        last_checked: Utc::now(),
                        error_message: Some("Authentication failed - invalid API key".to_string()),
                    },
                    status if status.is_server_error() => ProviderHealth {
                        status: HealthStatus::Degraded,
                        latency_ms: Some(latency),
                        last_checked: Utc::now(),
                        error_message: Some(format!("Server error: {}", status)),
                    },
                    status => ProviderHealth {
                        status: HealthStatus::Degraded,
                        latency_ms: Some(latency),
                        last_checked: Utc::now(),
                        error_message: Some(format!("Unexpected status: {}", status)),
                    },
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
        Ok(self.list_model_listing().await?.models)
    }

    async fn list_model_listing(&self) -> AppResult<ModelListing> {
        let (mut models, metadata) = self.fetch_listing().await?;
        // Decision models are listed only when asked for by modality.
        if let Some(gateway) = &self.systemone {
            gateway.merge_into("openrouter", &mut models).await;
        }
        Ok(ModelListing {
            models,
            new_models_hidden: false,
            metadata,
        })
    }

    async fn get_pricing(&self, model: &str) -> AppResult<PricingInfo> {
        if let Some(gateway) = &self.systemone {
            if let Some(pricing) = gateway.pricing(model).await {
                return Ok(pricing);
            }
        }
        // The model list the registry fetched; fetched here only when it
        // never was.
        if self.listing.get().is_none() && self.listing.should_fetch() {
            let _ = self.fetch_listing().await;
        }
        let entries = self
            .listing
            .get()
            .ok_or_else(|| AppError::Provider("OpenRouter model list unavailable".to_string()))?;
        let entry = entries
            .get(model)
            .ok_or_else(|| AppError::Provider(format!("Model not found: {}", model)))?;
        crate::listing_metadata::listing_pricing(entry)
            .ok_or_else(|| AppError::Provider(format!("No price listed for {}", model)))
    }

    async fn complete(&self, request: CompletionRequest) -> AppResult<CompletionResponse> {
        let preferences = self.gateway_preferences.effective_for_chat(&request)?;
        let url = format!("{}/chat/completions", self.base_url);

        let openrouter_request = OpenRouterRequest {
            model: request.model.clone(),
            messages: request.messages.clone(),
            temperature: request.temperature,
            max_tokens: request.max_tokens,
            top_p: request.top_p,
            frequency_penalty: request.frequency_penalty,
            presence_penalty: request.presence_penalty,
            stop: request.stop.clone(),
            tools: request.tools.clone(),
            tool_choice: request.tool_choice.clone(),
            response_format: request.response_format.clone(),
            stream: false,
            n: request.n,
            logit_bias: request.logit_bias.clone(),
            parallel_tool_calls: request.parallel_tool_calls,
            reasoning_effort: request.reasoning_effort.clone(),
            prediction: request.prediction.clone(),
        };

        let mut body = crate::gateway::wire_body(&openrouter_request)?;
        if let Some(context) = &request.server_tools {
            context.wire_body(
                crate::server_tools::ServerToolsProtocol::OpenRouter,
                &mut body,
                None,
            )?;
        }
        let gateway_headers = preferences.apply(GatewayKind::Openrouter, &mut body)?;
        let response = self
            .build_post_request(&url, request.server_tools.is_some())?
            .headers(gateway_headers)
            .json(&body)
            .send()
            .await
            .map_err(|e| AppError::Provider(format!("Failed to send completion request: {}", e)))?;

        if !response.status().is_success() {
            return Err(crate::http_client::response_error(response).await);
        }

        let raw: serde_json::Value = response
            .json()
            .await
            .map_err(|e| AppError::Provider(e.to_string()))?;
        let attribution = GatewayAttribution::from_wire(GatewayKind::Openrouter, &raw, preferences);
        let openrouter_response: OpenRouterResponse = serde_json::from_value(raw)
            .map_err(|e| AppError::Provider(format!("Invalid gateway response: {e}")))?;

        // Convert OpenRouter response to our standard format
        Ok(CompletionResponse {
            id: openrouter_response.id,
            object: "chat.completion".to_string(),
            created: openrouter_response.created,
            model: openrouter_response.model,
            provider: self.name().to_string(),
            choices: openrouter_response
                .choices
                .into_iter()
                .map(|choice| CompletionChoice {
                    index: choice.index,
                    message: choice.message,
                    finish_reason: choice.finish_reason,
                    logprobs: None, // OpenRouter does not support logprobs
                })
                .collect(),
            usage: openrouter_response.usage,
            system_fingerprint: None,
            service_tier: attribution.service_tier.clone(),
            extensions: Some(attribution.extensions()),
            routellm_win_rate: None,
            request_usage_entries: None,
        })
    }

    async fn stream_complete(
        &self,
        request: CompletionRequest,
    ) -> AppResult<Pin<Box<dyn Stream<Item = AppResult<CompletionChunk>> + Send>>> {
        let preferences = self.gateway_preferences.effective_for_chat(&request)?;
        let url = format!("{}/chat/completions", self.base_url);

        let openrouter_request = OpenRouterRequest {
            model: request.model.clone(),
            messages: request.messages.clone(),
            temperature: request.temperature,
            max_tokens: request.max_tokens,
            top_p: request.top_p,
            frequency_penalty: request.frequency_penalty,
            presence_penalty: request.presence_penalty,
            stop: request.stop,
            tools: request.tools,
            tool_choice: request.tool_choice,
            response_format: request.response_format,
            stream: true,
            n: request.n,
            logit_bias: request.logit_bias,
            parallel_tool_calls: request.parallel_tool_calls,
            reasoning_effort: request.reasoning_effort,
            prediction: request.prediction,
        };

        let mut body = stream_usage::streaming_body(&openrouter_request, "openrouter")?;
        if let Some(context) = &request.server_tools {
            context.wire_body(
                crate::server_tools::ServerToolsProtocol::OpenRouter,
                &mut body,
                None,
            )?;
        }
        let gateway_headers = preferences.apply(GatewayKind::Openrouter, &mut body)?;
        let response = self
            .build_post_request(&url, request.server_tools.is_some())?
            .headers(gateway_headers)
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                AppError::Provider(format!(
                    "Failed to send streaming completion request: {}",
                    e
                ))
            })?;

        if !response.status().is_success() {
            return Err(crate::http_client::response_error(response).await);
        }

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

                        if let Some(json_str) = line.strip_prefix("data: ") {
                            if json_str.trim() == "[DONE]" {
                                continue;
                            }

                            match serde_json::from_str::<OpenRouterChunk>(json_str) {
                                Ok(chunk) => {
                                    let mut attribution =
                                        serde_json::from_str::<serde_json::Value>(json_str)
                                            .ok()
                                            .map(|raw| {
                                                GatewayAttribution::from_wire(
                                                    GatewayKind::Openrouter,
                                                    &raw,
                                                    preferences.clone(),
                                                )
                                            });
                                    if let Some(current) = attribution.as_mut() {
                                        if let Some(previous) = &latest_attribution {
                                            current.retain_stream_metadata(previous);
                                        }
                                        latest_attribution = Some(current.clone());
                                    }
                                    parsed_chunks.push(Ok(CompletionChunk {
                                        id: chunk.id,
                                        object: "chat.completion.chunk".to_string(),
                                        created: chunk.created,
                                        model: chunk.model,
                                        choices: chunk
                                            .choices
                                            .into_iter()
                                            .map(|choice| ChunkChoice {
                                                index: choice.index,
                                                delta: choice.delta,
                                                finish_reason: choice.finish_reason,
                                            })
                                            .collect(),
                                        extensions: attribution.map(GatewayAttribution::extensions),
                                        usage: chunk
                                            .usage
                                            .as_ref()
                                            .and_then(stream_usage::parse_openai_usage),
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

    async fn embed(&self, request: super::EmbeddingRequest) -> AppResult<super::EmbeddingResponse> {
        // OpenRouter uses OpenAI-compatible embeddings API
        let input = match request.input {
            super::EmbeddingInput::Single(text) => serde_json::json!(text),
            super::EmbeddingInput::Multiple(texts) => serde_json::json!(texts),
            super::EmbeddingInput::Tokens(_) => {
                return Err(AppError::Provider(
                    "OpenRouter embeddings do not support pre-tokenized input".to_string(),
                ));
            }
        };

        let mut embed_request = serde_json::json!({
            "model": request.model,
            "input": input,
        });

        if let Some(fmt) = request.encoding_format {
            let format_str = match fmt {
                super::EncodingFormat::Float => "float",
                super::EncodingFormat::Base64 => "base64",
            };
            embed_request["encoding_format"] = serde_json::json!(format_str);
        }

        if let Some(dims) = request.dimensions {
            embed_request["dimensions"] = serde_json::json!(dims);
        }

        let response = self
            .client
            .post(format!("{}/embeddings", OPENROUTER_API_BASE))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&embed_request)
            .send()
            .await
            .map_err(|e| AppError::Provider(format!("Request failed: {}", e)))?;

        let status = response.status();
        if !status.is_success() {
            return Err(crate::http_client::response_error(response).await);
        }

        let api_response: serde_json::Value = response
            .json()
            .await
            .map_err(|e| AppError::Provider(format!("Failed to parse response: {}", e)))?;

        // Parse OpenAI-compatible response
        let data = api_response["data"]
            .as_array()
            .ok_or_else(|| AppError::Provider("No data array in response".to_string()))?;

        let embeddings: Vec<super::Embedding> = data
            .iter()
            .map(|item| {
                let embedding_array = item["embedding"].as_array().ok_or_else(|| {
                    AppError::Provider("No embedding array in data item".to_string())
                })?;
                let embedding: Vec<f32> = embedding_array
                    .iter()
                    .map(|v| v.as_f64().unwrap_or(0.0) as f32)
                    .collect();
                Ok(super::Embedding {
                    object: item["object"].as_str().unwrap_or("embedding").to_string(),
                    embedding: Some(embedding),
                    index: item["index"].as_u64().unwrap_or(0) as usize,
                })
            })
            .collect::<AppResult<Vec<_>>>()?;

        let usage = api_response["usage"]
            .as_object()
            .ok_or_else(|| AppError::Provider("No usage in response".to_string()))?;

        Ok(super::EmbeddingResponse {
            object: api_response["object"]
                .as_str()
                .unwrap_or("list")
                .to_string(),
            data: embeddings,
            model: api_response["model"]
                .as_str()
                .unwrap_or(&request.model)
                .to_string(),
            usage: super::EmbeddingUsage {
                prompt_tokens: usage["prompt_tokens"].as_u64().unwrap_or(0) as u32,
                total_tokens: usage["total_tokens"].as_u64().unwrap_or(0) as u32,
            },
        })
    }

    async fn check_credits(&self) -> Option<ProviderCreditsInfo> {
        let url = format!("{}/auth/key", self.base_url);
        let response = self
            .build_request(&url)
            .with_extension(crate::http_client::UsagePoll)
            .send()
            .await
            .ok()?;
        if !response.status().is_success() {
            return None;
        }
        let body: serde_json::Value = response.json().await.ok()?;
        let data = body.get("data")?;

        let usage = data.get("usage")?.as_f64();
        let limit = data.get("limit")?.as_f64();
        let is_free_tier = data.get("is_free_tier").and_then(|v| v.as_bool());
        let limit_remaining = data.get("limit_remaining").and_then(|v| v.as_f64());

        Some(ProviderCreditsInfo {
            total_credits_usd: limit,
            used_credits_usd: usage,
            remaining_credits_usd: limit_remaining.or(match (limit, usage) {
                (Some(l), Some(u)) => Some(l - u),
                _ => None,
            }),
            is_free_tier,
        })
    }

    /// `GET /key` (documented; the inference key): the key's spend limit or
    /// this month's spend, and the free-model daily allowance.
    async fn fetch_usage(
        &self,
    ) -> Option<Result<lr_usage::UsageReport, lr_usage::fetch::FetchError>> {
        let url = format!("{}/key", self.base_url);
        let now = chrono::Utc::now().timestamp();
        Some(
            crate::http_client::fetch_usage_json(self.build_request(&url), |body| {
                lr_usage::bodies::parse_openrouter_key(body, now)
            })
            .await,
        )
    }

    fn supports_systemone(&self) -> bool {
        self.systemone.is_some()
    }

    fn supports_decisions(&self) -> bool {
        self.systemone.is_some()
    }

    fn supports_gateway_options(&self) -> bool {
        true
    }
    fn validate_gateway_options(&self, request: &CompletionRequest) -> AppResult<()> {
        self.gateway_preferences
            .effective_for_chat(request)?
            .apply(
                GatewayKind::Openrouter,
                &mut serde_json::json!({"model":request.model}),
            )?;
        Ok(())
    }

    fn effective_gateway_preferences(
        &self,
        request: &CompletionRequest,
    ) -> AppResult<Option<GatewayPreferences>> {
        self.validate_gateway_options(request)?;
        Ok(Some(self.gateway_preferences.effective_for_chat(request)?))
    }

    fn effective_native_gateway_preferences(
        &self,
        request: &CompletionRequest,
    ) -> AppResult<Option<GatewayPreferences>> {
        if let Some(gateway) = &self.systemone {
            let mut extra = serde_json::Map::new();
            if let Some(extensions) = &request.extensions {
                extra.insert("extensions".into(), serde_json::json!(extensions));
            }
            if let Some(tier) = &request.service_tier {
                extra.insert("service_tier".into(), serde_json::json!(tier));
            }
            if let Some(effort) = &request.reasoning_effort {
                extra.insert("reasoning_effort".into(), serde_json::json!(effort));
            }
            gateway.validate_options(&crate::SystemOneRequest {
                decisions: None,
                model: Some(request.model.clone()),
                state: serde_json::Value::Null,
                questions: Default::default(),
                extra,
            })?;
        }
        Ok(Some(self.gateway_preferences.effective_for_chat(request)?))
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
        if let Some(gateway) = &self.systemone {
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
        match &self.systemone {
            Some(gateway) => gateway.is_decision_model(model).await,
            None => false,
        }
    }

    async fn systemone(
        &self,
        request: crate::SystemOneRequest,
    ) -> AppResult<crate::SystemOneResponse> {
        match &self.systemone {
            Some(gateway) => gateway.systemone(request).await,
            None => Err(AppError::Provider(
                "Provider 'openrouter' does not support system one decisions".to_string(),
            )),
        }
    }

    fn get_feature_support(&self, instance_name: &str) -> super::ProviderFeatureSupport {
        let mut support = super::default_feature_support(self, instance_name);

        for f in &mut support.model_features {
            match f.name.as_str() {
                "N Completions" => {
                    f.support = super::SupportLevel::Partial;
                    f.notes = Some(
                        "Passed through to upstream provider — support depends on model".into(),
                    );
                }
                "Logit Bias" => {
                    f.support = super::SupportLevel::Partial;
                    f.notes = Some(
                        "Passed through to upstream provider — support depends on model".into(),
                    );
                }
                "Parallel Tool Calls" => {
                    f.support = super::SupportLevel::Partial;
                    f.notes = Some(
                        "Passed through to upstream provider — support depends on model".into(),
                    );
                }
                "Reasoning Effort" => {
                    f.support = super::SupportLevel::Partial;
                    f.notes = Some(
                        "Passed through to upstream provider — support depends on model".into(),
                    );
                }
                "Predicted Output" => {
                    f.support = super::SupportLevel::Partial;
                    f.notes = Some(
                        "Passed through to upstream provider — support depends on model".into(),
                    );
                }
                _ => {}
            }
        }

        support
    }
}

// OpenRouter API types

#[derive(Debug, Serialize)]
struct OpenRouterRequest {
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
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<Tool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<ToolChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_format: Option<super::ResponseFormat>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    n: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    logit_bias: Option<std::collections::HashMap<String, f32>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    parallel_tool_calls: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_effort: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    prediction: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct OpenRouterResponse {
    id: String,
    created: i64,
    model: String,
    choices: Vec<OpenRouterChoice>,
    usage: TokenUsage,
}

#[derive(Debug, Deserialize)]
struct OpenRouterChoice {
    index: u32,
    message: ChatMessage,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenRouterChunk {
    id: String,
    created: i64,
    model: String,
    choices: Vec<OpenRouterChunkChoice>,
    /// Upstream usage, when the server reports it (on the final chunk or
    /// a usage-only chunk with empty `choices`).
    #[serde(default)]
    usage: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct OpenRouterChunkChoice {
    index: u32,
    delta: ChunkDelta,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenRouterModelsResponse {
    data: Vec<OpenRouterModel>,
}

#[derive(Debug, Deserialize)]
struct OpenRouterModel {
    id: String,
    name: Option<String>,
    context_length: u32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ChatMessageContent;

    #[test]
    fn test_provider_name() {
        let provider = OpenRouterProvider::new("test-key".to_string());
        assert_eq!(provider.name(), "openrouter");
    }

    #[test]
    fn test_with_app_name() {
        let provider =
            OpenRouterProvider::new("test-key".to_string()).with_app_name("Test App".to_string());
        assert_eq!(provider.app_name, Some("Test App".to_string()));
    }

    #[test]
    fn test_with_app_url() {
        let provider = OpenRouterProvider::new("test-key".to_string())
            .with_app_url("https://example.com".to_string());
        assert_eq!(provider.app_url, Some("https://example.com".to_string()));
    }

    #[tokio::test]
    async fn prices_come_from_the_kept_model_list() {
        use wiremock::{matchers, Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(matchers::path("/models"))
            .and(matchers::query_param("output_modalities", "decisions"))
            .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"data":[]}"#))
            .mount(&server)
            .await;
        Mock::given(matchers::path("/models"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"data":[
                    {"id":"a/paid","name":"Paid","context_length":8000,
                     "pricing":{"prompt":"0.000001","completion":"0.000002","request":"0"}},
                    {"id":"a/free:free","context_length":8000,
                     "pricing":{"prompt":"0","completion":"0"}}
                ]}"#,
            ))
            .mount(&server)
            .await;
        let provider = OpenRouterProvider::with_base_url("k".into(), server.uri());
        let listing = provider.list_model_listing().await.unwrap();
        assert_eq!(listing.models.len(), 2);
        assert_eq!(listing.metadata["a/free:free"]["pricing"]["prompt"], "0");
        let model_list_requests =
            |reqs: &[wiremock::Request]| reqs.iter().filter(|r| r.url.query().is_none()).count();
        let before = model_list_requests(&server.received_requests().await.unwrap());

        let paid = provider.get_pricing("a/paid").await.unwrap();
        assert!((paid.input_cost_per_1k - 0.001).abs() < 1e-12);
        assert!((paid.output_cost_per_1k - 0.002).abs() < 1e-12);
        let free = provider.get_pricing("a/free:free").await.unwrap();
        assert_eq!(free.input_cost_per_1k, 0.0);
        assert!(provider.get_pricing("a/unknown").await.is_err());
        // No second download of the model list.
        assert_eq!(
            model_list_requests(&server.received_requests().await.unwrap()),
            before
        );
    }

    // Integration tests require a valid API key
    #[tokio::test]
    #[ignore]
    async fn test_health_check() {
        let api_key = std::env::var("OPENROUTER_API_KEY").expect("OPENROUTER_API_KEY not set");
        let provider = OpenRouterProvider::new(api_key);
        let health = provider.health_check().await;
        assert_eq!(health.status, HealthStatus::Healthy);
    }

    #[tokio::test]
    #[ignore]
    async fn test_list_models() {
        let api_key = std::env::var("OPENROUTER_API_KEY").expect("OPENROUTER_API_KEY not set");
        let provider = OpenRouterProvider::new(api_key);
        let models = provider.list_models().await.unwrap();
        assert!(!models.is_empty());
        assert!(models.iter().any(|m| m.provider == "openrouter"));
    }

    #[tokio::test]
    #[ignore]
    async fn test_get_pricing() {
        let api_key = std::env::var("OPENROUTER_API_KEY").expect("OPENROUTER_API_KEY not set");
        let provider = OpenRouterProvider::new(api_key);
        let pricing = provider.get_pricing("openai/gpt-3.5-turbo").await.unwrap();
        assert!(pricing.input_cost_per_1k >= 0.0);
        assert!(pricing.output_cost_per_1k >= 0.0);
        assert_eq!(pricing.currency, "USD");
    }

    #[tokio::test]
    #[ignore]
    async fn test_completion() {
        let api_key = std::env::var("OPENROUTER_API_KEY").expect("OPENROUTER_API_KEY not set");
        let provider = OpenRouterProvider::new(api_key);

        let request = CompletionRequest {
            model: "openai/gpt-3.5-turbo".to_string(),
            messages: vec![ChatMessage {
                annotations: None,
                role: "user".to_string(),
                content: ChatMessageContent::Text(
                    "Say 'Hello, World!' and nothing else.".to_string(),
                ),
                tool_calls: None,
                tool_call_id: None,
                name: None,
                reasoning_content: None,
            }],
            temperature: Some(0.7),
            max_tokens: Some(50),
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
            native_messages: None,
            server_tools: None,
        };

        let response = provider.complete(request).await.unwrap();
        assert!(!response.choices.is_empty());
        assert!(!response.choices[0].message.content.is_empty());
    }

    /// OpenRouter sends usage in its last SSE message (a usage-only chunk).
    #[tokio::test]
    async fn stream_reports_upstream_usage() {
        use crate::openai_compatible::stream_usage::test_support::*;

        let server = sse_server("/chat/completions", openai_stream(UsageAt::UsageOnlyChunk)).await;
        let provider = OpenRouterProvider::with_base_url("k".to_string(), server.uri());
        let stream = provider
            .stream_complete(stream_request("test-model"))
            .await
            .unwrap();
        assert_openai_stream(&collect(stream).await);
        assert!(asked_for_usage(&received_body(&server).await));
    }
    #[tokio::test]
    async fn server_tools_preserve_citations_and_caps_without_redirect_reexecution() {
        use wiremock::matchers::{body_partial_json, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        let citation = serde_json::json!({"type":"url_citation","url_citation":{"url":"https://example.com/source","title":"Source","start_index":0,"end_index":2}});
        Mock::given(method("POST")).and(path("/chat/completions"))
            .and(body_partial_json(serde_json::json!({"tools":[{"type":"openrouter:web_search"}],"max_tool_calls":2})))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id":"search","created":1,"model":"m","choices":[{"index":0,"message":{"role":"assistant","content":"ok","annotations":[citation.clone()]},"finish_reason":"stop"}],
                "usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2,"server_tool_use":{"web_search_requests":1}}
            }))).expect(1).mount(&server).await;
        let provider = OpenRouterProvider::with_base_url("k".into(), server.uri());
        let mut request = CompletionRequest::new("m", vec![]);
        request.server_tools = Some(Box::new(crate::server_tools::ServerToolsContext {
            tools: vec![crate::server_tools::PositionedServerTool {
                position: 0,
                definition: crate::server_tools::ServerToolDefinition::OpenRouterWebSearch {
                    parameters: None,
                },
            }],
            original_tool_count: 1,
            max_tool_calls: Some(4),
            forced_type: None,
            policy_max_tool_calls: Some(2),
        }));
        let result = provider.complete(request.clone()).await.unwrap();
        assert_eq!(
            result.choices[0].message.annotations.as_ref().unwrap()[0],
            citation
        );
        assert_eq!(
            result.extensions.as_ref().unwrap()[crate::gateway::EXTENSION]["server_tool_use"]
                ["web_search_requests"],
            1
        );
        assert!(crate::gateway::reported_cost(result.extensions.as_ref()).is_none());
        server.verify().await;
        server.reset().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(
                ResponseTemplate::new(307)
                    .insert_header("location", format!("{}/redirected", server.uri())),
            )
            .expect(1)
            .mount(&server)
            .await;
        assert!(provider.complete(request).await.is_err());
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].url.path(), "/chat/completions");
    }
}
