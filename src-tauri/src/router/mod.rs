//! Smart routing system
//!
//! Routes incoming requests to appropriate model providers based on API key configuration.

use std::sync::Arc;

use futures::{Stream, StreamExt};
use std::pin::Pin;
use tracing::{debug, error, info, warn};

use crate::config::ConfigManager;
use crate::providers::registry::ProviderRegistry;
use crate::providers::{
    CompletionChunk, CompletionRequest, CompletionResponse, EmbeddingRequest, EmbeddingResponse,
};
use crate::utils::errors::{AppError, AppResult};

pub mod rate_limit;

// Re-export commonly used types
pub use rate_limit::{RateLimiterManager, UsageInfo};

/// Router error classification for auto-routing fallback decisions
#[derive(Debug, Clone)]
pub enum RouterError {
    /// Provider rate limited - should retry with different model
    RateLimited {
        provider: String,
        model: String,
        retry_after_secs: u64,
    },
    /// Content policy violation - should retry with different model
    PolicyViolation {
        provider: String,
        model: String,
        reason: String,
    },
    /// Context length exceeded - should retry with different model
    ContextLengthExceeded {
        provider: String,
        model: String,
        max_tokens: u64,
    },
    /// Provider unreachable - should retry with different model
    Unreachable { provider: String, model: String },
    /// Other error - should not retry
    Other {
        provider: String,
        model: String,
        error: String,
    },
}

impl RouterError {
    /// Whether this error should trigger fallback to next model
    pub fn should_retry(&self) -> bool {
        matches!(
            self,
            RouterError::RateLimited { .. }
                | RouterError::Unreachable { .. }
                | RouterError::ContextLengthExceeded { .. }
                | RouterError::PolicyViolation { .. }
        )
    }

    /// Classify an AppError into a RouterError
    pub fn classify(error: &AppError, provider: &str, model: &str) -> Self {
        match error {
            AppError::RateLimitExceeded => RouterError::RateLimited {
                provider: provider.to_string(),
                model: model.to_string(),
                retry_after_secs: 60,
            },
            AppError::Provider(msg) if msg.contains("policy") || msg.contains("content_policy") => {
                RouterError::PolicyViolation {
                    provider: provider.to_string(),
                    model: model.to_string(),
                    reason: msg.clone(),
                }
            }
            AppError::Provider(msg)
                if msg.contains("context") || msg.contains("token") || msg.contains("length") =>
            {
                RouterError::ContextLengthExceeded {
                    provider: provider.to_string(),
                    model: model.to_string(),
                    max_tokens: 0,
                }
            }
            AppError::Provider(msg)
                if msg.contains("unreachable")
                    || msg.contains("timeout")
                    || msg.contains("connection") =>
            {
                RouterError::Unreachable {
                    provider: provider.to_string(),
                    model: model.to_string(),
                }
            }
            AppError::Io(_) => RouterError::Unreachable {
                provider: provider.to_string(),
                model: model.to_string(),
            },
            _ => RouterError::Other {
                provider: provider.to_string(),
                model: model.to_string(),
                error: error.to_string(),
            },
        }
    }

    /// Get a log-friendly string representation
    pub fn to_log_string(&self) -> String {
        match self {
            RouterError::RateLimited {
                provider,
                model,
                retry_after_secs,
            } => {
                format!(
                    "RATE_LIMITED: {}/{} (retry after {}s)",
                    provider, model, retry_after_secs
                )
            }
            RouterError::PolicyViolation {
                provider,
                model,
                reason,
            } => {
                format!("POLICY_VIOLATION: {}/{} - {}", provider, model, reason)
            }
            RouterError::ContextLengthExceeded {
                provider,
                model,
                max_tokens,
            } => {
                format!(
                    "CONTEXT_LENGTH_EXCEEDED: {}/{} (max: {})",
                    provider, model, max_tokens
                )
            }
            RouterError::Unreachable { provider, model } => {
                format!("UNREACHABLE: {}/{}", provider, model)
            }
            RouterError::Other {
                provider,
                model,
                error,
            } => {
                format!("ERROR: {}/{} - {}", provider, model, error)
            }
        }
    }
}

/// Wraps a completion stream to count tokens, record usage, and set the correct model name
///
/// This is an approximation: we estimate tokens based on content length
/// since streaming chunks don't include token counts.
///
/// The `resolved_model` parameter should be in `provider/model` format (e.g., "openai/gpt-4o")
/// and will be set on each chunk to ensure clients know which model actually processed the request.
async fn wrap_stream_with_usage_tracking(
    stream: Pin<Box<dyn Stream<Item = AppResult<CompletionChunk>> + Send>>,
    client_id: String,
    resolved_model: String,
    rate_limiter: Arc<RateLimiterManager>,
) -> Pin<Box<dyn Stream<Item = AppResult<CompletionChunk>> + Send>> {
    use std::sync::atomic::{AtomicU64, Ordering};

    // Track token counts as stream progresses
    let completion_chars = Arc::new(AtomicU64::new(0));

    // Clone for map closure (model injection)
    let resolved_model_for_map = resolved_model.clone();

    // Clone for inspect closure
    let completion_chars_for_inspect = completion_chars.clone();

    // Clone for then closure
    let completion_chars_for_then = completion_chars.clone();
    let client_id_for_then = client_id.clone();
    let rate_limiter_for_then = rate_limiter.clone();

    let wrapped = stream
        .map(move |chunk_result| {
            // Inject the resolved model name into each chunk
            chunk_result.map(|mut chunk| {
                chunk.model = resolved_model_for_map.clone();
                chunk
            })
        })
        .inspect(move |chunk_result| {
            if let Ok(chunk) = chunk_result {
                // Count content characters in this chunk
                for choice in &chunk.choices {
                    if let Some(content) = &choice.delta.content {
                        let char_count = content.len() as u64;
                        completion_chars_for_inspect.fetch_add(char_count, Ordering::Relaxed);
                    }
                }
            }
        })
        .then(move |chunk_result| {
        let client_id = client_id_for_then.clone();
        let rate_limiter = rate_limiter_for_then.clone();
        let completion_chars = completion_chars_for_then.clone();

        async move {
            // Check if this is an error or the last chunk
            let is_last = chunk_result.as_ref().map_or(true, |chunk| {
                chunk.choices.iter().any(|c| c.finish_reason.is_some())
            });

            // If stream is ending, record usage
            if is_last {
                // Estimate tokens: rough approximation is 1 token ≈ 4 characters
                // We'll use prompt_tokens=10 as baseline (can't know actual from stream)
                // and estimate completion tokens from character count
                let est_prompt = 10; // Baseline estimate since we don't know actual prompt tokens
                let est_completion = (completion_chars.load(Ordering::Relaxed) / 4).max(1);

                let usage = UsageInfo {
                    input_tokens: est_prompt,
                    output_tokens: est_completion,
                    cost_usd: 0.0,
                };

                // Record usage (best effort, don't fail the stream)
                if let Err(e) = rate_limiter.record_api_key_usage(&client_id, &usage).await {
                    warn!(
                        "Failed to record streaming usage for API key '{}': {}. \
                         Estimated {} tokens (approximate).",
                        client_id, e, est_prompt + est_completion
                    );
                } else {
                    debug!(
                        "Recorded estimated streaming usage for API key '{}': {} tokens (approximate)",
                        client_id, est_prompt + est_completion
                    );
                }
            }

            chunk_result
        }
    });

    Box::pin(wrapped)
}

/// Calculate cost in USD from token usage and pricing info
fn calculate_cost(
    input_tokens: u64,
    output_tokens: u64,
    pricing: &crate::providers::PricingInfo,
) -> f64 {
    let input_cost = (input_tokens as f64 / 1000.0) * pricing.input_cost_per_1k;
    let output_cost = (output_tokens as f64 / 1000.0) * pricing.output_cost_per_1k;
    input_cost + output_cost
}

/// Router for handling completion requests with API key-based model selection
pub struct Router {
    config_manager: Arc<ConfigManager>,
    provider_registry: Arc<ProviderRegistry>,
    rate_limiter: Arc<RateLimiterManager>,
    metrics_collector: Arc<crate::monitoring::metrics::MetricsCollector>,
    routellm_service: Option<Arc<crate::routellm::RouteLLMService>>,
}

impl Router {
    /// Create a new router
    pub fn new(
        config_manager: Arc<ConfigManager>,
        provider_registry: Arc<ProviderRegistry>,
        rate_limiter: Arc<RateLimiterManager>,
        metrics_collector: Arc<crate::monitoring::metrics::MetricsCollector>,
    ) -> Self {
        Self {
            config_manager,
            provider_registry,
            rate_limiter,
            metrics_collector,
            routellm_service: None,
        }
    }

    /// Set the RouteLLM service
    pub fn with_routellm(
        mut self,
        routellm_service: Option<Arc<crate::routellm::RouteLLMService>>,
    ) -> Self {
        self.routellm_service = routellm_service;
        self
    }

    /// Get the RouteLLM service
    pub fn get_routellm_service(&self) -> Option<&Arc<crate::routellm::RouteLLMService>> {
        self.routellm_service.as_ref()
    }

    /// Parse model string into (provider, model) tuple
    /// Supports formats: "provider/model" or just "model" (returns empty provider)
    fn parse_model_string(model: &str) -> (String, String) {
        if let Some((provider, model_name)) = model.split_once('/') {
            (provider.to_string(), model_name.to_string())
        } else {
            (String::new(), model.to_string())
        }
    }

    /// Validate client exists and is enabled, then get their strategy
    /// Returns 403 (via AppError::Unauthorized) if client is invalid or disabled
    fn validate_client_and_strategy(
        &self,
        client_id: &str,
    ) -> AppResult<(crate::config::Client, crate::config::Strategy)> {
        let config = self.config_manager.get();

        let client = config
            .clients
            .iter()
            .find(|c| c.id == client_id)
            .ok_or_else(|| {
                warn!("Client '{}' not found", client_id);
                AppError::Unauthorized
            })?
            .clone();

        if !client.enabled {
            warn!("Client '{}' is disabled", client_id);
            return Err(AppError::Unauthorized);
        }

        let strategy = config
            .strategies
            .iter()
            .find(|s| s.id == client.strategy_id)
            .ok_or_else(|| {
                warn!(
                    "Strategy '{}' not found for client '{}'",
                    client.strategy_id, client_id
                );
                AppError::Router(format!("Strategy '{}' not found", client.strategy_id))
            })?
            .clone();

        Ok((client, strategy))
    }

    /// Check client-level rate limits
    async fn check_client_rate_limits(&self, client_id: &str) -> AppResult<()> {
        let usage_estimate = UsageInfo {
            input_tokens: 0,
            output_tokens: 0,
            cost_usd: 0.0,
        };

        let rate_check = self
            .rate_limiter
            .check_api_key(client_id, &usage_estimate)
            .await?;

        if !rate_check.allowed {
            warn!(
                "Client '{}' rate limited. Retry after {} seconds",
                client_id,
                rate_check.retry_after_secs.unwrap_or(0)
            );
            return Err(AppError::RateLimitExceeded);
        }

        Ok(())
    }

    /// Find provider for a model when no provider is specified in model string
    /// Searches through strategy's allowed models to find which provider has this model
    async fn find_provider_for_model(
        &self,
        model: &str,
        strategy: &crate::config::Strategy,
    ) -> AppResult<(String, String)> {
        let normalized_requested = Self::normalize_model_id(model);
        let allowed = &strategy.allowed_models;

        // Check selected_models first (individual model selections)
        for (prov, mod_name) in &allowed.selected_models {
            let normalized_allowed = Self::normalize_model_id(mod_name);
            if normalized_allowed == normalized_requested {
                return Ok((prov.clone(), model.to_string()));
            }
        }

        // Check providers in selected_providers
        for prov in &allowed.selected_providers {
            if let Some(provider_instance) = self.provider_registry.get_provider(prov) {
                if let Ok(models) = provider_instance.list_models().await {
                    if models.iter().any(|m| {
                        let normalized_provider_model = Self::normalize_model_id(&m.id);
                        normalized_provider_model == normalized_requested
                    }) {
                        return Ok((prov.clone(), model.to_string()));
                    }
                }
            }
        }

        // If selected_all is true, search all providers
        if allowed.selected_all {
            for instance_info in self.provider_registry.list_providers() {
                if let Some(provider_instance) = self
                    .provider_registry
                    .get_provider(&instance_info.instance_name)
                {
                    if let Ok(models) = provider_instance.list_models().await {
                        if models.iter().any(|m| {
                            let normalized_provider_model = Self::normalize_model_id(&m.id);
                            normalized_provider_model == normalized_requested
                        }) {
                            return Ok((instance_info.instance_name.clone(), model.to_string()));
                        }
                    }
                }
            }
        }

        Err(AppError::Router(format!(
            "Model '{}' is not allowed by this strategy",
            model
        )))
    }

    /// Normalize a model ID for comparison
    ///
    /// Different providers return model IDs in different formats:
    /// - Ollama: "llama2" or "llama2:latest"
    /// - OpenAI-compatible: "provider/model" or just "model"
    /// - LMStudio: "model-name"
    ///
    /// This function normalizes by:
    /// 1. Stripping provider prefix (e.g., "openai/gpt-4" -> "gpt-4")
    /// 2. Stripping tag suffix (e.g., "llama2:latest" -> "llama2")
    ///
    /// Returns the normalized model name for case-insensitive comparison
    fn normalize_model_id(model_id: &str) -> String {
        // Strip provider prefix if present
        let without_provider = if let Some((_provider, model)) = model_id.split_once('/') {
            model
        } else {
            model_id
        };

        // Strip tag suffix if present (e.g., ":latest", ":7b")
        let without_tag = if let Some((base, _tag)) = without_provider.split_once(':') {
            base
        } else {
            without_provider
        };

        without_tag.to_lowercase()
    }

    /// Check strategy rate limits using metrics-based approach
    /// Returns error if projected usage would exceed any configured limits
    fn check_strategy_rate_limits(
        &self,
        strategy: &crate::config::Strategy,
        _provider: &str,
        _model: &str,
    ) -> AppResult<()> {
        if strategy.rate_limits.is_empty() {
            return Ok(());
        }

        // Get pre-estimates from recent usage (last 10 minutes)
        let (avg_tokens, avg_cost) = self
            .metrics_collector
            .get_pre_estimate_for_strategy(&strategy.id, 10);

        for limit in &strategy.rate_limits {
            let window_secs = limit.time_window.to_seconds();

            let (current_requests, current_tokens, current_cost) = self
                .metrics_collector
                .get_recent_usage_for_strategy(&strategy.id, window_secs);

            let (projected_usage, limit_value) = match limit.limit_type {
                crate::config::RateLimitType::Requests => {
                    (current_requests as f64 + 1.0, limit.value)
                }
                crate::config::RateLimitType::TotalTokens => {
                    (current_tokens as f64 + avg_tokens, limit.value)
                }
                crate::config::RateLimitType::Cost => {
                    // Special case: if avg_cost is 0 (free models), don't count against cost limit
                    if avg_cost == 0.0 {
                        continue;
                    }
                    (current_cost + avg_cost, limit.value)
                }
                _ => continue, // InputTokens/OutputTokens not supported for pre-check
            };

            if projected_usage > limit_value {
                warn!(
                    "Strategy rate limit exceeded: {} (current: {:.2}, projected: {:.2}, limit: {:.2})",
                    match limit.limit_type {
                        crate::config::RateLimitType::Requests => "requests",
                        crate::config::RateLimitType::TotalTokens => "tokens",
                        crate::config::RateLimitType::Cost => "cost",
                        _ => "unknown",
                    },
                    if matches!(limit.limit_type, crate::config::RateLimitType::Requests) {
                        current_requests as f64
                    } else if matches!(limit.limit_type, crate::config::RateLimitType::TotalTokens) {
                        current_tokens as f64
                    } else {
                        current_cost
                    },
                    projected_usage,
                    limit_value
                );
                return Err(AppError::RateLimitExceeded);
            }
        }

        Ok(())
    }

    /// Execute a completion request on a specific provider/model
    /// This is the core execution logic extracted from the old complete() method
    async fn execute_request(
        &self,
        client_id: &str,
        provider: &str,
        model: &str,
        request: CompletionRequest,
    ) -> AppResult<CompletionResponse> {
        // Get provider instance from registry
        let provider_instance = self
            .provider_registry
            .get_provider(provider)
            .ok_or_else(|| {
                AppError::Router(format!(
                    "Provider '{}' not found or disabled in registry",
                    provider
                ))
            })?;

        // Check provider health (log warning if unhealthy)
        let health = provider_instance.health_check().await;
        match health.status {
            crate::providers::HealthStatus::Healthy => {
                debug!(
                    "Provider '{}' is healthy (latency: {:?}ms)",
                    provider, health.latency_ms
                );
            }
            crate::providers::HealthStatus::Degraded => {
                warn!(
                    "Provider '{}' is degraded: {}",
                    provider,
                    health.error_message.as_deref().unwrap_or("unknown")
                );
            }
            crate::providers::HealthStatus::Unhealthy => {
                warn!(
                    "Provider '{}' is unhealthy: {}",
                    provider,
                    health.error_message.as_deref().unwrap_or("unknown")
                );
                // Continue anyway - let the request fail naturally
            }
        }

        // Modify the request to use just the model name (without provider prefix)
        let mut modified_request = request.clone();
        modified_request.model = model.to_string();

        // Apply feature adapters if extensions are present
        if let Some(ref extensions) = request.extensions {
            for (feature_name, feature_params) in extensions {
                // Check if provider supports this feature
                if provider_instance.supports_feature(feature_name) {
                    debug!(
                        "Provider '{}' supports feature '{}'",
                        provider, feature_name
                    );

                    // Get the feature adapter
                    if let Some(adapter) = provider_instance.get_feature_adapter(feature_name) {
                        // Validate parameters
                        let mut params: crate::providers::features::FeatureParams =
                            std::collections::HashMap::new();
                        if let serde_json::Value::Object(map) = feature_params {
                            for (k, v) in map {
                                params.insert(k.clone(), v.clone());
                            }
                        }
                        adapter.validate_params(&params).map_err(|e| {
                            warn!(
                                "Feature '{}' parameter validation failed: {}",
                                feature_name, e
                            );
                            e
                        })?;

                        // Adapt the request
                        adapter
                            .adapt_request(&mut modified_request, &params)
                            .map_err(|e| {
                                warn!(
                                    "Feature '{}' request adaptation failed: {}",
                                    feature_name, e
                                );
                                e
                            })?;

                        debug!(
                            "Successfully applied feature adapter for '{}'",
                            feature_name
                        );
                    }
                } else {
                    warn!(
                        "Provider '{}' does not support feature '{}', ignoring",
                        provider, feature_name
                    );
                }
            }
        }

        // Execute the completion
        let mut response = provider_instance.complete(modified_request).await?;

        // Apply feature adapters to response if needed
        if let Some(ref extensions) = request.extensions {
            for (feature_name, _feature_params) in extensions {
                if provider_instance.supports_feature(feature_name) {
                    if let Some(adapter) = provider_instance.get_feature_adapter(feature_name) {
                        adapter.adapt_response(&mut response)?;
                    }
                }
            }
        }

        // Add provider and model information to response
        response.provider = provider.to_string();
        response.model = model.to_string();

        // Calculate cost and record usage for rate limiting
        let pricing = provider_instance
            .get_pricing(model)
            .await
            .unwrap_or_else(|_| crate::providers::PricingInfo::free());

        let cost = calculate_cost(
            response.usage.prompt_tokens as u64,
            response.usage.completion_tokens as u64,
            &pricing,
        );

        let usage = UsageInfo {
            input_tokens: response.usage.prompt_tokens as u64,
            output_tokens: response.usage.completion_tokens as u64,
            cost_usd: cost,
        };

        // Record usage for rate limiting
        if let Err(e) = self
            .rate_limiter
            .record_api_key_usage(client_id, &usage)
            .await
        {
            warn!("Failed to record usage for API key '{}': {}", client_id, e);
        }

        Ok(response)
    }

    /// Select models for auto-routing using RouteLLM prediction if configured.
    ///
    /// Returns a tuple of (selected_models, optional_win_rate).
    /// - If RouteLLM is enabled and prediction succeeds, returns strong or weak models based on threshold
    /// - Falls back to prioritized_models if RouteLLM fails or is disabled
    async fn select_models_for_auto_routing(
        &self,
        auto_config: &crate::config::AutoModelConfig,
        request: &CompletionRequest,
        context: &str, // "streaming" or empty for logging
    ) -> (Vec<(String, String)>, Option<f32>) {
        let log_prefix = if context.is_empty() {
            "RouteLLM".to_string()
        } else {
            format!("RouteLLM ({})", context)
        };

        if let Some(routellm_config) = &auto_config.routellm_config {
            if routellm_config.enabled {
                // Extract prompt from request messages
                let prompt = request
                    .messages
                    .iter()
                    .map(|m| m.content.as_str())
                    .collect::<Vec<_>>()
                    .join("\n");

                // Get RouteLLM service
                if let Some(service) = &self.routellm_service {
                    // Predict with threshold
                    match service
                        .predict_with_threshold(&prompt, routellm_config.threshold)
                        .await
                    {
                        Ok((is_strong, win_rate)) => {
                            // Select models based on prediction
                            // Strong models use prioritized_models from auto_config
                            let models = if is_strong {
                                &auto_config.prioritized_models
                            } else {
                                &routellm_config.weak_models
                            };

                            info!(
                                "{}: win_rate={:.3}, threshold={:.3}, selected={}",
                                log_prefix,
                                win_rate,
                                routellm_config.threshold,
                                if is_strong { "strong" } else { "weak" }
                            );

                            return (models.clone(), Some(win_rate));
                        }
                        Err(e) => {
                            warn!(
                                "{} prediction failed: {}, fallback to prioritized models",
                                log_prefix, e
                            );
                        }
                    }
                } else {
                    warn!(
                        "{} enabled but service not available, using prioritized models",
                        log_prefix
                    );
                }
            }
        }

        // Default: use prioritized models
        (auto_config.prioritized_models.clone(), None)
    }

    /// Complete with auto-routing (localrouter/auto virtual model)
    /// Tries models in prioritized order with intelligent fallback
    async fn complete_with_auto_routing(
        &self,
        client_id: &str,
        strategy: &crate::config::Strategy,
        request: CompletionRequest,
    ) -> AppResult<CompletionResponse> {
        let auto_config = strategy.auto_config.as_ref().ok_or_else(|| {
            AppError::Router("localrouter/auto not configured for this strategy".into())
        })?;

        if !auto_config.enabled {
            return Err(AppError::Router(
                "localrouter/auto is disabled for this strategy".into(),
            ));
        }

        if auto_config.prioritized_models.is_empty() {
            return Err(AppError::Router(
                "No prioritized models configured for auto-routing".into(),
            ));
        }

        // Select models using RouteLLM prediction (if configured)
        let (selected_models, routellm_win_rate) = self
            .select_models_for_auto_routing(auto_config, &request, "")
            .await;

        if selected_models.is_empty() {
            return Err(AppError::Router(
                "No models available for auto-routing (RouteLLM returned empty list)".into(),
            ));
        }

        info!(
            "Auto-routing for client '{}' with {} selected models",
            client_id,
            selected_models.len()
        );

        let mut last_error = None;

        for (idx, (provider, model)) in selected_models.iter().enumerate() {
            debug!(
                "Auto-routing attempt {}/{}: {}/{}",
                idx + 1,
                auto_config.prioritized_models.len(),
                provider,
                model
            );

            // Check strategy rate limits before trying this model
            if let Err(e) = self.check_strategy_rate_limits(strategy, provider, model) {
                warn!(
                    "Strategy rate limit exceeded for {}/{}, trying next model: {}",
                    provider, model, e
                );
                last_error = Some(RouterError::RateLimited {
                    provider: provider.clone(),
                    model: model.clone(),
                    retry_after_secs: 60,
                });
                continue;
            }

            // PERFORMANCE NOTE: request.clone() is called here for each model attempt,
            // and execute_request clones it again internally. This double-cloning could
            // be optimized by restructuring to use Arc<CompletionRequest> or by passing
            // a mutable reference. However, this requires careful ownership management.
            // With large message histories and multiple fallback models, this could
            // become a bottleneck.
            match self
                .execute_request(client_id, provider, model, request.clone())
                .await
            {
                Ok(mut response) => {
                    info!("Auto-routing succeeded with {}/{}", provider, model);
                    // Attach RouteLLM win rate to response for logging
                    if let Some(win_rate) = routellm_win_rate {
                        response.routellm_win_rate = Some(win_rate);
                    }
                    return Ok(response);
                }
                Err(e) => {
                    let router_error = RouterError::classify(&e, provider, model);
                    warn!(
                        "Auto-routing attempt failed: {}",
                        router_error.to_log_string()
                    );

                    last_error = Some(router_error.clone());

                    // Continue to next model on retryable errors
                    if !router_error.should_retry() {
                        error!(
                            "Non-retryable error encountered: {}",
                            router_error.to_log_string()
                        );
                        return Err(e);
                    }
                }
            }
        }

        // All models failed
        Err(AppError::Router(format!(
            "All auto-routing models failed. Last error: {}",
            last_error
                .map(|e| e.to_log_string())
                .unwrap_or_else(|| "Unknown".to_string())
        )))
    }

    /// Stream complete with auto-routing (localrouter/auto virtual model for streaming)
    /// Tries models in prioritized order with intelligent fallback
    async fn stream_complete_with_auto_routing(
        &self,
        client_id: &str,
        strategy: &crate::config::Strategy,
        request: CompletionRequest,
    ) -> AppResult<Pin<Box<dyn Stream<Item = AppResult<CompletionChunk>> + Send>>> {
        let auto_config = strategy.auto_config.as_ref().ok_or_else(|| {
            AppError::Router("localrouter/auto not configured for this strategy".into())
        })?;

        if !auto_config.enabled {
            return Err(AppError::Router(
                "localrouter/auto is disabled for this strategy".into(),
            ));
        }

        if auto_config.prioritized_models.is_empty() {
            return Err(AppError::Router(
                "No prioritized models configured for auto-routing".into(),
            ));
        }

        // Select models using RouteLLM prediction (if configured)
        let (selected_models, _routellm_win_rate) = self
            .select_models_for_auto_routing(auto_config, &request, "streaming")
            .await;

        if selected_models.is_empty() {
            return Err(AppError::Router(
                "No models available for auto-routing (RouteLLM returned empty list)".into(),
            ));
        }

        info!(
            "Auto-routing streaming for client '{}' with {} selected models",
            client_id,
            selected_models.len()
        );

        let mut last_error = None;

        for (idx, (provider, model)) in selected_models.iter().enumerate() {
            debug!(
                "Auto-routing streaming attempt {}/{}: {}/{}",
                idx + 1,
                selected_models.len(),
                provider,
                model
            );

            // Check strategy rate limits before trying this model
            if let Err(e) = self.check_strategy_rate_limits(strategy, provider, model) {
                warn!(
                    "Strategy rate limit exceeded for {}/{}, trying next model: {}",
                    provider, model, e
                );
                last_error = Some(RouterError::RateLimited {
                    provider: provider.clone(),
                    model: model.clone(),
                    retry_after_secs: 60,
                });
                continue;
            }

            // Get provider instance
            let provider_instance = match self.provider_registry.get_provider(provider) {
                Some(p) => p,
                None => {
                    warn!("Provider '{}' not found, trying next model", provider);
                    last_error = Some(RouterError::Unreachable {
                        provider: provider.clone(),
                        model: model.clone(),
                    });
                    continue;
                }
            };

            // Execute streaming request
            let mut modified_request = request.clone();
            modified_request.model = model.clone();

            match provider_instance.stream_complete(modified_request).await {
                Ok(stream) => {
                    info!(
                        "Auto-routing streaming succeeded with {}/{}",
                        provider, model
                    );
                    let resolved_model = format!("{}/{}", provider, model);
                    return Ok(wrap_stream_with_usage_tracking(
                        stream,
                        client_id.to_string(),
                        resolved_model,
                        self.rate_limiter.clone(),
                    )
                    .await);
                }
                Err(e) => {
                    let router_error = RouterError::classify(&e, provider, model);
                    warn!(
                        "Auto-routing streaming attempt failed: {}",
                        router_error.to_log_string()
                    );

                    last_error = Some(router_error.clone());

                    // Continue to next model on retryable errors
                    if !router_error.should_retry() {
                        error!(
                            "Non-retryable error encountered: {}",
                            router_error.to_log_string()
                        );
                        return Err(e);
                    }
                }
            }
        }

        // All models failed
        Err(AppError::Router(format!(
            "All auto-routing streaming models failed. Last error: {}",
            last_error
                .map(|e| e.to_log_string())
                .unwrap_or_else(|| "Unknown".to_string())
        )))
    }

    /// Try completion with prioritized list of models (with automatic retry on failure)
    ///
    /// Tries each model in the prioritized list in order until one succeeds.
    /// Retries on specific errors like provider unavailable, rate limit, or model not found.
    /// Records usage for rate limiting on success.
    #[deprecated(note = "Use complete_with_auto_routing instead")]
    #[allow(dead_code)]
    async fn complete_with_prioritized_list(
        &self,
        client_id: &str,
        prioritized_models: &[(String, String)],
        mut request: CompletionRequest,
    ) -> AppResult<CompletionResponse> {
        let mut last_error = None;

        for (idx, (provider_name, model_name)) in prioritized_models.iter().enumerate() {
            debug!(
                "Trying prioritized model {}/{}: provider='{}', model='{}'",
                idx + 1,
                prioritized_models.len(),
                provider_name,
                model_name
            );

            // Get provider instance
            let provider_instance = match self.provider_registry.get_provider(provider_name) {
                Some(p) => p,
                None => {
                    warn!(
                        "Provider '{}' not found or disabled, trying next model",
                        provider_name
                    );
                    last_error = Some(AppError::Router(format!(
                        "Provider '{}' not found or disabled",
                        provider_name
                    )));
                    continue;
                }
            };

            // Update request with this model
            request.model = model_name.clone();

            // Try the completion
            match provider_instance.complete(request.clone()).await {
                Ok(response) => {
                    info!(
                        "Prioritized model succeeded: provider='{}', model='{}'",
                        provider_name, model_name
                    );

                    // Calculate cost and record usage for rate limiting
                    let pricing = provider_instance
                        .get_pricing(model_name)
                        .await
                        .unwrap_or_else(|_| crate::providers::PricingInfo::free());

                    let cost = calculate_cost(
                        response.usage.prompt_tokens as u64,
                        response.usage.completion_tokens as u64,
                        &pricing,
                    );

                    let usage = UsageInfo {
                        input_tokens: response.usage.prompt_tokens as u64,
                        output_tokens: response.usage.completion_tokens as u64,
                        cost_usd: cost,
                    };

                    // Log error but don't fail the request if usage recording fails
                    // The provider already succeeded and consumed tokens/cost
                    if let Err(e) = self
                        .rate_limiter
                        .record_api_key_usage(client_id, &usage)
                        .await
                    {
                        warn!(
                            "Failed to record usage for API key '{}': {}. Request succeeded but usage not tracked.",
                            client_id, e
                        );
                    }

                    return Ok(response);
                }
                Err(e) => {
                    warn!(
                        "Model failed: provider='{}', model='{}', error='{}'",
                        provider_name, model_name, e
                    );

                    // Determine if we should retry with next model
                    let should_retry = matches!(
                        e,
                        AppError::Provider(_)
                            | AppError::RateLimitExceeded
                            | AppError::Router(_)
                            | AppError::Internal(_)
                    );

                    if !should_retry {
                        // Non-retryable error (e.g., validation error) - fail immediately
                        warn!("Non-retryable error, stopping retry attempts");
                        return Err(e);
                    }

                    // Store error message for later use
                    last_error = Some(AppError::Router(format!(
                        "Model '{}' from provider '{}' failed: {}",
                        model_name, provider_name, e
                    )));

                    // Continue to next model
                    if idx < prioritized_models.len() - 1 {
                        debug!("Retrying with next prioritized model...");
                    }
                }
            }
        }

        // All models failed
        Err(last_error.unwrap_or_else(|| {
            AppError::Router("All prioritized models failed or no models configured".to_string())
        }))
    }

    /// Route a completion request based on client configuration
    ///
    /// This method:
    /// 1. Validates the client exists and is enabled
    /// 2. Gets the client's routing strategy
    /// 3. Routes based on requested model (auto vs specific)
    /// 4. Executes the request with appropriate fallback behavior
    ///
    /// Returns 403 (via AppError::Unauthorized) if client is invalid or disabled
    pub async fn complete(
        &self,
        client_id: &str,
        request: CompletionRequest,
    ) -> AppResult<CompletionResponse> {
        debug!(
            "Routing completion request for client '{}', model '{}'",
            client_id, request.model
        );

        // Special handling for internal test token (bypasses all routing config)
        // SECURITY: Only enabled in debug builds to prevent production bypass
        #[cfg(debug_assertions)]
        if client_id == "internal-test" {
            debug!("Internal test token detected - bypassing routing config (test mode only)");
            let (provider, model) = Self::parse_model_string(&request.model);
            if provider.is_empty() {
                return Err(AppError::Router(
                    "Internal test requires provider/model format".into(),
                ));
            }
            return self
                .execute_request(client_id, &provider, &model, request)
                .await;
        }

        // In release builds, reject internal-test token to prevent security bypass
        #[cfg(not(debug_assertions))]
        if client_id == "internal-test" {
            warn!("Attempted to use internal-test bypass in production - rejected");
            return Err(AppError::Unauthorized);
        }

        // 1. Validate client and get strategy
        let (_client, strategy) = self.validate_client_and_strategy(client_id)?;

        // 2. Check client-level rate limits
        self.check_client_rate_limits(client_id).await?;

        // 3. Route based on requested model
        if request.model == "localrouter/auto" {
            debug!("Using auto-routing for client '{}'", client_id);
            return self
                .complete_with_auto_routing(client_id, &strategy, request)
                .await;
        }

        // 4. For specific model requests, check strategy rate limits
        self.check_strategy_rate_limits(&strategy, "", "")?;

        // 5. Parse and validate model
        let (provider, model) = Self::parse_model_string(&request.model);

        let (final_provider, final_model) = if provider.is_empty() {
            self.find_provider_for_model(&model, &strategy).await?
        } else {
            // Provider specified - validate it's allowed
            if !strategy.is_model_allowed(&provider, &model) {
                return Err(AppError::Router(format!(
                    "Model '{}/{}' is not allowed by this strategy",
                    provider, model
                )));
            }
            (provider, model)
        };

        // 6. Execute the request
        debug!(
            "Executing request for client '{}' on {}/{}",
            client_id, final_provider, final_model
        );

        self.execute_request(client_id, &final_provider, &final_model, request)
            .await
    }

    /// Route a streaming completion request based on API key configuration
    pub async fn stream_complete(
        &self,
        client_id: &str,
        request: CompletionRequest,
    ) -> AppResult<Pin<Box<dyn Stream<Item = AppResult<CompletionChunk>> + Send>>> {
        debug!(
            "Routing streaming completion request for client '{}', model '{}'",
            client_id, request.model
        );

        // Special handling for internal test token
        // SECURITY: Only enabled in debug builds to prevent production bypass
        #[cfg(debug_assertions)]
        if client_id == "internal-test" {
            debug!("Internal test token detected - bypassing routing config for streaming (test mode only)");
            let (provider, model) = Self::parse_model_string(&request.model);
            if provider.is_empty() {
                return Err(AppError::Router(
                    "Internal test requires provider/model format".into(),
                ));
            }
            let provider_instance = self
                .provider_registry
                .get_provider(&provider)
                .ok_or_else(|| AppError::Router(format!("Provider '{}' not found", provider)))?;
            let mut modified_request = request.clone();
            modified_request.model = model.clone();
            let stream = provider_instance.stream_complete(modified_request).await?;
            let resolved_model = format!("{}/{}", provider, model);
            return Ok(wrap_stream_with_usage_tracking(
                stream,
                client_id.to_string(),
                resolved_model,
                self.rate_limiter.clone(),
            )
            .await);
        }

        // In release builds, reject internal-test token to prevent security bypass
        #[cfg(not(debug_assertions))]
        if client_id == "internal-test" {
            warn!("Attempted to use internal-test bypass in production (streaming) - rejected");
            return Err(AppError::Unauthorized);
        }

        // 1. Validate client and get strategy
        let (_client, strategy) = self.validate_client_and_strategy(client_id)?;

        // 2. Check rate limits
        self.check_client_rate_limits(client_id).await?;

        // 3. Route - handle auto-routing for streaming
        if request.model == "localrouter/auto" {
            debug!("Using auto-routing for streaming client '{}'", client_id);
            return self
                .stream_complete_with_auto_routing(client_id, &strategy, request)
                .await;
        }

        // 4. Parse and validate model
        let (provider, model) = Self::parse_model_string(&request.model);

        let (final_provider, final_model) = if provider.is_empty() {
            self.find_provider_for_model(&model, &strategy).await?
        } else {
            if !strategy.is_model_allowed(&provider, &model) {
                return Err(AppError::Router(format!(
                    "Model '{}/{}' not allowed",
                    provider, model
                )));
            }
            (provider, model)
        };

        // 5. Check strategy rate limits
        self.check_strategy_rate_limits(&strategy, &final_provider, &final_model)?;

        // 6. Execute streaming request
        let provider_instance = self
            .provider_registry
            .get_provider(&final_provider)
            .ok_or_else(|| AppError::Router(format!("Provider '{}' not found", final_provider)))?;

        let mut modified_request = request.clone();
        modified_request.model = final_model.clone();
        let stream = provider_instance.stream_complete(modified_request).await?;
        let resolved_model = format!("{}/{}", final_provider, final_model);
        Ok(wrap_stream_with_usage_tracking(
            stream,
            client_id.to_string(),
            resolved_model,
            self.rate_limiter.clone(),
        )
        .await)
    }

    /// Execute an embedding request on a specific provider/model
    async fn execute_embedding_request(
        &self,
        client_id: &str,
        provider: &str,
        model: &str,
        request: EmbeddingRequest,
    ) -> AppResult<EmbeddingResponse> {
        // Get provider instance from registry
        let provider_instance = self
            .provider_registry
            .get_provider(provider)
            .ok_or_else(|| {
                AppError::Router(format!(
                    "Provider '{}' not found or disabled in registry",
                    provider
                ))
            })?;

        // Check provider health (log warning if unhealthy)
        let health = provider_instance.health_check().await;
        match health.status {
            crate::providers::HealthStatus::Healthy => {
                debug!(
                    "Provider '{}' is healthy (latency: {:?}ms)",
                    provider, health.latency_ms
                );
            }
            crate::providers::HealthStatus::Degraded => {
                warn!(
                    "Provider '{}' is degraded: {}",
                    provider,
                    health.error_message.as_deref().unwrap_or("unknown")
                );
            }
            crate::providers::HealthStatus::Unhealthy => {
                warn!(
                    "Provider '{}' is unhealthy: {}",
                    provider,
                    health.error_message.as_deref().unwrap_or("unknown")
                );
                // Continue anyway - let the request fail naturally
            }
        }

        // Modify the request to use just the model name (without provider prefix)
        let modified_request = EmbeddingRequest {
            model: model.to_string(),
            input: request.input,
            encoding_format: request.encoding_format,
            dimensions: request.dimensions,
            user: request.user,
        };

        // Execute the embedding
        let response = provider_instance.embed(modified_request).await?;

        // Record usage for rate limiting (embeddings have simpler usage)
        let usage = UsageInfo {
            input_tokens: response.usage.prompt_tokens as u64,
            output_tokens: 0, // Embeddings don't have output tokens
            cost_usd: 0.0,    // TODO: Calculate cost from provider pricing
        };

        // Record usage for rate limiting
        if let Err(e) = self
            .rate_limiter
            .record_api_key_usage(client_id, &usage)
            .await
        {
            warn!("Failed to record usage for client '{}': {}", client_id, e);
        }

        Ok(response)
    }

    /// Embed with auto-routing (localrouter/auto virtual model)
    /// Tries models in prioritized order with intelligent fallback
    async fn embed_with_auto_routing(
        &self,
        client_id: &str,
        strategy: &crate::config::Strategy,
        request: EmbeddingRequest,
    ) -> AppResult<EmbeddingResponse> {
        let auto_config = strategy.auto_config.as_ref().ok_or_else(|| {
            AppError::Router("localrouter/auto not configured for this strategy".into())
        })?;

        if !auto_config.enabled {
            return Err(AppError::Router(
                "localrouter/auto is disabled for this strategy".into(),
            ));
        }

        if auto_config.prioritized_models.is_empty() {
            return Err(AppError::Router(
                "No prioritized models configured for auto-routing".into(),
            ));
        }

        // Note: RouteLLM is not applicable to embeddings (no strong/weak model selection)
        // We just use the prioritized_models list directly
        let selected_models = &auto_config.prioritized_models;

        info!(
            "Auto-routing embeddings for client '{}' with {} prioritized models",
            client_id,
            selected_models.len()
        );

        let mut last_error = None;

        for (idx, (provider, model)) in selected_models.iter().enumerate() {
            debug!(
                "Auto-routing embeddings attempt {}/{}: {}/{}",
                idx + 1,
                selected_models.len(),
                provider,
                model
            );

            // Check strategy rate limits before trying this model
            if let Err(e) = self.check_strategy_rate_limits(strategy, provider, model) {
                warn!(
                    "Strategy rate limit exceeded for {}/{}, trying next model: {}",
                    provider, model, e
                );
                last_error = Some(RouterError::RateLimited {
                    provider: provider.clone(),
                    model: model.clone(),
                    retry_after_secs: 60,
                });
                continue;
            }

            match self
                .execute_embedding_request(client_id, provider, model, request.clone())
                .await
            {
                Ok(response) => {
                    info!(
                        "Auto-routing embeddings succeeded with {}/{}",
                        provider, model
                    );
                    return Ok(response);
                }
                Err(e) => {
                    let router_error = RouterError::classify(&e, provider, model);
                    warn!(
                        "Auto-routing embeddings attempt failed: {}",
                        router_error.to_log_string()
                    );

                    last_error = Some(router_error.clone());

                    // Continue to next model on retryable errors
                    if !router_error.should_retry() {
                        error!(
                            "Non-retryable error encountered: {}",
                            router_error.to_log_string()
                        );
                        return Err(e);
                    }
                }
            }
        }

        // All models failed
        Err(AppError::Router(format!(
            "All auto-routing embedding models failed. Last error: {}",
            last_error
                .map(|e| e.to_log_string())
                .unwrap_or_else(|| "Unknown".to_string())
        )))
    }

    /// Route an embedding request to the appropriate provider
    ///
    /// This method:
    /// 1. Validates the client exists and is enabled
    /// 2. Gets the client's routing strategy
    /// 3. Routes based on requested model (auto vs specific)
    /// 4. Executes the request with appropriate fallback behavior
    ///
    /// Returns 403 (via AppError::Unauthorized) if client is invalid or disabled
    pub async fn embed(
        &self,
        client_id: &str,
        request: EmbeddingRequest,
    ) -> AppResult<EmbeddingResponse> {
        debug!(
            "Routing embedding request for client '{}', model '{}'",
            client_id, request.model
        );

        // Special handling for internal test token (bypasses all routing config)
        // SECURITY: Only enabled in debug builds to prevent production bypass
        #[cfg(debug_assertions)]
        if client_id == "internal-test" {
            debug!("Internal test token detected - bypassing routing config for embeddings (test mode only)");
            let (provider, model) = Self::parse_model_string(&request.model);
            if provider.is_empty() {
                return Err(AppError::Router(
                    "Internal test requires provider/model format".into(),
                ));
            }
            return self
                .execute_embedding_request(client_id, &provider, &model, request)
                .await;
        }

        // In release builds, reject internal-test token to prevent security bypass
        #[cfg(not(debug_assertions))]
        if client_id == "internal-test" {
            warn!("Attempted to use internal-test bypass in production (embeddings) - rejected");
            return Err(AppError::Unauthorized);
        }

        // 1. Validate client and get strategy
        let (_client, strategy) = self.validate_client_and_strategy(client_id)?;

        // 2. Check client-level rate limits
        self.check_client_rate_limits(client_id).await?;

        // 3. Route based on requested model
        if request.model == "localrouter/auto" {
            debug!("Using auto-routing for embeddings client '{}'", client_id);
            return self
                .embed_with_auto_routing(client_id, &strategy, request)
                .await;
        }

        // 4. For specific model requests, check strategy rate limits
        self.check_strategy_rate_limits(&strategy, "", "")?;

        // 5. Parse and validate model
        let (provider, model) = Self::parse_model_string(&request.model);

        let (final_provider, final_model) = if provider.is_empty() {
            self.find_provider_for_model(&model, &strategy).await?
        } else {
            // Provider specified - validate it's allowed
            if !strategy.is_model_allowed(&provider, &model) {
                return Err(AppError::Router(format!(
                    "Model '{}/{}' is not allowed by this strategy",
                    provider, model
                )));
            }
            (provider, model)
        };

        // 6. Execute the request
        debug!(
            "Executing embedding request for client '{}' on {}/{}",
            client_id, final_provider, final_model
        );

        self.execute_embedding_request(client_id, &final_provider, &final_model, request)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppConfig;
    use crate::providers::health::HealthCheckManager;

    #[tokio::test]
    async fn test_router_creation() {
        let config_manager = Arc::new(ConfigManager::new(
            AppConfig::default(),
            std::path::PathBuf::from("/tmp/test.yaml"),
        ));

        let health_manager = Arc::new(HealthCheckManager::default());
        let provider_registry = Arc::new(ProviderRegistry::new(health_manager));
        let rate_limiter = Arc::new(RateLimiterManager::new(None));

        // Create test metrics collector
        let metrics_db_path =
            std::env::temp_dir().join(format!("test_metrics_{}.db", uuid::Uuid::new_v4()));
        let metrics_db =
            Arc::new(crate::monitoring::storage::MetricsDatabase::new(metrics_db_path).unwrap());
        let metrics_collector = Arc::new(crate::monitoring::metrics::MetricsCollector::new(
            metrics_db,
        ));

        let router = Router::new(
            config_manager,
            provider_registry,
            rate_limiter,
            metrics_collector,
        );

        // Just verify it compiles and constructs
        assert!(Arc::strong_count(&router.config_manager) >= 1);
    }

    #[tokio::test]
    async fn test_router_unauthorized_api_key() {
        let config_manager = Arc::new(ConfigManager::new(
            AppConfig::default(),
            std::path::PathBuf::from("/tmp/test.yaml"),
        ));

        let health_manager = Arc::new(HealthCheckManager::default());
        let provider_registry = Arc::new(ProviderRegistry::new(health_manager));
        let rate_limiter = Arc::new(RateLimiterManager::new(None));

        // Create test metrics collector
        let metrics_db_path =
            std::env::temp_dir().join(format!("test_metrics_{}.db", uuid::Uuid::new_v4()));
        let metrics_db =
            Arc::new(crate::monitoring::storage::MetricsDatabase::new(metrics_db_path).unwrap());
        let metrics_collector = Arc::new(crate::monitoring::metrics::MetricsCollector::new(
            metrics_db,
        ));

        let router = Router::new(
            config_manager,
            provider_registry,
            rate_limiter,
            metrics_collector,
        );

        let request = CompletionRequest {
            model: "test-model".to_string(),
            messages: vec![],
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
            logprobs: None,
            top_logprobs: None,
            response_format: None,
            tool_choice: None,
            tools: None,
        };

        let result = router.complete("invalid-key-id", request).await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), AppError::Unauthorized));
    }

    // ============================================================================
    // Routing Strategy Tests - REMOVED (tests used obsolete ApiKeyConfig)
    // ============================================================================
    // Tests were removed during migration to unified Client architecture.
    // New tests should be written using the Client structure.

    // ============================================================================
    // TODO: Provider Retry Integration Tests
    // ============================================================================
    //
    // Future work: Add integration tests that test actual provider retry logic with mock
    // provider implementations. These tests should cover:
    //
    // 1. **Provider Failures with Retry**:
    //    - First provider fails with retryable error (Provider, RateLimitExceeded, etc.)
    //    - Router retries with next provider in prioritized list
    //    - Verify call counts and retry order
    //
    // 2. **Non-Retryable Errors**:
    //    - Provider fails with Validation error
    //    - Router stops immediately without retrying
    //
    // 3. **All Providers Fail**:
    //    - All providers in prioritized list fail
    //    - Router returns last error
    //
    // 4. **Health Check Failures**:
    //    - Provider health check fails
    //    - Router logs warning but continues with request
    //    - Completion still works if provider is functional
    //
    // 5. **Success on First Try**:
    //    - First provider succeeds
    //    - No retries attempted
    //    - Other providers never called
    //
    // **Implementation Requirements**:
    // - Create MockProviderFactory that implements ProviderFactory trait
    // - MockProvider with configurable failure modes (count, error type)
    // - Track call counts to verify retry logic
    // - Test with ProviderRegistry.create_provider() for proper architecture
    //
    // **Current Status**: Unit tests in routing_strategy_tests provide comprehensive
    // coverage of configuration logic and is_model_allowed() behavior. Integration
    // tests require ProviderRegistry architecture support.
    //
    // ============================================================================

    // Placeholder for future integration tests - see TODO comment above

    // ============================================================================
    // Test normalize_model_id
    // ============================================================================

    #[test]
    fn test_normalize_model_id_plain() {
        assert_eq!(Router::normalize_model_id("llama2"), "llama2");
        assert_eq!(Router::normalize_model_id("gpt-4"), "gpt-4");
        assert_eq!(Router::normalize_model_id("claude-3-opus"), "claude-3-opus");
    }

    #[test]
    fn test_normalize_model_id_with_tag() {
        assert_eq!(Router::normalize_model_id("llama2:latest"), "llama2");
        assert_eq!(Router::normalize_model_id("llama2:7b"), "llama2");
        assert_eq!(Router::normalize_model_id("mistral:instruct"), "mistral");
    }

    #[test]
    fn test_normalize_model_id_with_provider_prefix() {
        assert_eq!(Router::normalize_model_id("openai/gpt-4"), "gpt-4");
        assert_eq!(Router::normalize_model_id("anthropic/claude-3"), "claude-3");
        assert_eq!(Router::normalize_model_id("ollama/llama2"), "llama2");
    }

    #[test]
    fn test_normalize_model_id_with_both() {
        assert_eq!(Router::normalize_model_id("openai/gpt-4:turbo"), "gpt-4");
        assert_eq!(Router::normalize_model_id("ollama/llama2:latest"), "llama2");
        assert_eq!(Router::normalize_model_id("mistral/mistral:7b"), "mistral");
    }

    #[test]
    fn test_normalize_model_id_case_insensitive() {
        assert_eq!(Router::normalize_model_id("GPT-4"), "gpt-4");
        assert_eq!(Router::normalize_model_id("LLaMA2"), "llama2");
        assert_eq!(Router::normalize_model_id("Claude-3-Opus"), "claude-3-opus");
    }

    #[test]
    fn test_normalize_model_id_all_together() {
        // Case: "OpenAI/GPT-4:Turbo" -> "gpt-4"
        assert_eq!(Router::normalize_model_id("OpenAI/GPT-4:Turbo"), "gpt-4");
        // Case: "OLLAMA/LLaMA2:Latest" -> "llama2"
        assert_eq!(Router::normalize_model_id("OLLAMA/LLaMA2:Latest"), "llama2");
    }
}
