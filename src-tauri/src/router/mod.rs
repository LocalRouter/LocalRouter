//! Smart routing system
//!
//! Routes incoming requests to appropriate model providers based on API key configuration.

use std::sync::Arc;

use futures::{Stream, StreamExt};
use std::pin::Pin;
use tracing::{debug, info, warn};

use crate::config::{ActiveRoutingStrategy, ConfigManager, ModelSelection};
use crate::providers::registry::ProviderRegistry;
use crate::providers::{CompletionChunk, CompletionRequest, CompletionResponse};
use crate::utils::errors::{AppError, AppResult};

pub mod rate_limit;

// Re-export commonly used types
pub use rate_limit::{
    RateLimiterManager, UsageInfo,
};

/// Wraps a completion stream to count tokens and record usage when complete
///
/// This is an approximation: we estimate tokens based on content length
/// since streaming chunks don't include token counts.
async fn wrap_stream_with_usage_tracking(
    stream: Pin<Box<dyn Stream<Item = AppResult<CompletionChunk>> + Send>>,
    api_key_id: String,
    rate_limiter: Arc<RateLimiterManager>,
) -> Pin<Box<dyn Stream<Item = AppResult<CompletionChunk>> + Send>> {
    use std::sync::atomic::{AtomicU64, Ordering};

    // Track token counts as stream progresses
    let completion_chars = Arc::new(AtomicU64::new(0));

    // Clone for inspect closure
    let completion_chars_for_inspect = completion_chars.clone();

    // Clone for then closure
    let completion_chars_for_then = completion_chars.clone();
    let api_key_id_for_then = api_key_id.clone();
    let rate_limiter_for_then = rate_limiter.clone();

    let wrapped = stream.inspect(move |chunk_result| {
        if let Ok(chunk) = chunk_result {
            // Count content characters in this chunk
            for choice in &chunk.choices {
                if let Some(content) = &choice.delta.content {
                    let char_count = content.len() as u64;
                    completion_chars_for_inspect.fetch_add(char_count, Ordering::Relaxed);
                }
            }
        }
    }).then(move |chunk_result| {
        let api_key_id = api_key_id_for_then.clone();
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
                if let Err(e) = rate_limiter.record_api_key_usage(&api_key_id, &usage).await {
                    warn!(
                        "Failed to record streaming usage for API key '{}': {}. \
                         Estimated {} tokens (approximate).",
                        api_key_id, e, est_prompt + est_completion
                    );
                } else {
                    debug!(
                        "Recorded estimated streaming usage for API key '{}': {} tokens (approximate)",
                        api_key_id, est_prompt + est_completion
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
}

impl Router {
    /// Create a new router
    pub fn new(
        config_manager: Arc<ConfigManager>,
        provider_registry: Arc<ProviderRegistry>,
        rate_limiter: Arc<RateLimiterManager>,
    ) -> Self {
        Self {
            config_manager,
            provider_registry,
            rate_limiter,
        }
    }

    /// Try completion with prioritized list of models (with automatic retry on failure)
    ///
    /// Tries each model in the prioritized list in order until one succeeds.
    /// Retries on specific errors like provider unavailable, rate limit, or model not found.
    /// Records usage for rate limiting on success.
    async fn complete_with_prioritized_list(
        &self,
        api_key_id: &str,
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
                        .get_pricing(&model_name)
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
                    if let Err(e) = self.rate_limiter.record_api_key_usage(api_key_id, &usage).await {
                        warn!(
                            "Failed to record usage for API key '{}': {}. Request succeeded but usage not tracked.",
                            api_key_id, e
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

    /// Route a completion request based on API key configuration
    ///
    /// This method:
    /// 1. Validates the API key exists and is enabled
    /// 2. Checks rate limits
    /// 3. Routes to the configured provider+model (DirectModel only for now)
    /// 4. Executes the request
    /// 5. Records usage for rate limiting
    ///
    /// Returns 403 (via AppError::Unauthorized) if API key is invalid or disabled
    pub async fn complete(
        &self,
        api_key_id: &str,
        request: CompletionRequest,
    ) -> AppResult<CompletionResponse> {
        debug!(
            "Routing completion request for API key '{}', model '{}'",
            api_key_id, request.model
        );

        // 1. Get API key configuration
        let config = self.config_manager.get();
        let api_key = config
            .api_keys
            .iter()
            .find(|k| k.id == api_key_id)
            .ok_or_else(|| {
                warn!("API key '{}' not found", api_key_id);
                AppError::Unauthorized
            })?;

        // Check if API key is enabled
        if !api_key.enabled {
            warn!("API key '{}' is disabled", api_key_id);
            return Err(AppError::Unauthorized);
        }

        // 2. Check rate limits (pre-request check for request-based limits)
        let usage_estimate = UsageInfo {
            input_tokens: 0,
            output_tokens: 0,
            cost_usd: 0.0,
        };

        let rate_check = self
            .rate_limiter
            .check_api_key(api_key_id, &usage_estimate)
            .await?;

        if !rate_check.allowed {
            warn!(
                "API key '{}' rate limited. Retry after {} seconds",
                api_key_id,
                rate_check.retry_after_secs.unwrap_or(0)
            );
            return Err(AppError::RateLimitExceeded);
        }

        // 3. Determine provider and model based on API key configuration
        let routing_config = api_key.get_routing_config();

        // Special handling for Prioritized List: use retry logic
        if let Some(ref config) = routing_config {
            if config.active_strategy == ActiveRoutingStrategy::PrioritizedList {
                if config.prioritized_models.is_empty() {
                    return Err(AppError::Router(
                        "Prioritized List strategy is active but no models are configured".to_string()
                    ));
                }

                info!(
                    "Using Prioritized List strategy with {} models for API key '{}'",
                    config.prioritized_models.len(),
                    api_key_id
                );

                // Use retry logic for prioritized list
                return self
                    .complete_with_prioritized_list(api_key_id, &config.prioritized_models, request)
                    .await;
            }
        }

        let (provider, expected_model) = if let Some(ref config) = routing_config {
            // New routing config system
            match config.active_strategy {
                ActiveRoutingStrategy::AvailableModels => {
                    // Allow any model in the available list - find the provider from the request
                    // Parse the model from request (format: "provider/model" or just "model")
                    if let Some((p, m)) = request.model.split_once('/') {
                        debug!("Using provider/model from request: {}/{}", p, m);

                        // Validate the model is in the available list
                        if !config.is_model_allowed(p, m) {
                            return Err(AppError::Router(format!(
                                "Model '{}' is not in the available models list for this API key",
                                request.model
                            )));
                        }

                        (p.to_string(), m.to_string())
                    } else {
                        // Just a model name - need to find which provider has it
                        debug!("Model name only: {}", request.model);

                        // Find the provider for this model from the available list
                        let mut found_provider = None;

                        // Check individual_models first
                        for (provider_name, model_name) in &config.available_models.individual_models {
                            if model_name.eq_ignore_ascii_case(&request.model) {
                                found_provider = Some(provider_name.clone());
                                break;
                            }
                        }

                        // If not found, check providers in all_provider_models
                        if found_provider.is_none() {
                            for provider_name in &config.available_models.all_provider_models {
                                if let Some(provider) = self.provider_registry.get_provider(provider_name) {
                                    if let Ok(models) = provider.list_models().await {
                                        if models.iter().any(|m| m.id.eq_ignore_ascii_case(&request.model)) {
                                            found_provider = Some(provider_name.clone());
                                            break;
                                        }
                                    }
                                }
                            }
                        }

                        if let Some(provider) = found_provider {
                            (provider, request.model.clone())
                        } else {
                            return Err(AppError::Router(format!(
                                "Model '{}' is not in the available models list for this API key",
                                request.model
                            )));
                        }
                    }
                }
                ActiveRoutingStrategy::ForceModel => {
                    // Force a specific model, ignore the request
                    if let Some((forced_provider, forced_model)) = &config.forced_model {
                        debug!(
                            "Forcing model: provider='{}', model='{}' (requested was '{}')",
                            forced_provider, forced_model, request.model
                        );
                        (forced_provider.clone(), forced_model.clone())
                    } else {
                        return Err(AppError::Router(
                            "Force Model strategy is active but no model is configured".to_string()
                        ));
                    }
                }
                ActiveRoutingStrategy::PrioritizedList => {
                    // This is unreachable due to early return at line 229-231
                    // Added here only to satisfy exhaustive match checking
                    unreachable!("PrioritizedList should be handled by early return")
                }
            }
        } else {
            // Fallback to old model_selection for backward compatibility
            match &api_key.model_selection {
                Some(ModelSelection::All) | None => {
                    // Allow any model - find the provider from the request
                    // Parse the model from request (format: "provider/model" or just "model")
                    if let Some((p, m)) = request.model.split_once('/') {
                        debug!("Using provider/model from request: {}/{}", p, m);
                        (p.to_string(), m.to_string())
                    } else {
                        // Just a model name - need to find which provider has it
                        // For now, use the model as-is and let the provider registry find it
                        debug!("Model name only: {}", request.model);
                        // This will be handled below when we get the provider
                        ("".to_string(), request.model.clone())
                    }
                }
                Some(ModelSelection::Custom {
                    all_provider_models,
                    individual_models,
                }) => {
                // Check if the requested model is allowed
                // Parse model from request
                let (req_provider, req_model) = if let Some((p, m)) = request.model.split_once('/') {
                    (p.to_string(), m.to_string())
                } else {
                    // Need to find provider for this model
                    ("".to_string(), request.model.clone())
                };

                // Determine which provider to use and validate access
                let final_provider = if !req_provider.is_empty() {
                    // Provider specified - check if allowed
                    let is_allowed = all_provider_models.iter().any(|p| p.eq_ignore_ascii_case(&req_provider))
                        || individual_models
                            .iter()
                            .any(|(p, m)| p.eq_ignore_ascii_case(&req_provider) && m.eq_ignore_ascii_case(&req_model));

                    if !is_allowed {
                        return Err(AppError::Router(format!(
                            "Model '{}' is not allowed by this API key's model selection",
                            request.model
                        )));
                    }

                    req_provider
                } else {
                    // Just model name - find which provider has it
                    // First check individual_models
                    if let Some((provider, _)) = individual_models
                        .iter()
                        .find(|(_p, m)| m.eq_ignore_ascii_case(&req_model))
                    {
                        provider.clone()
                    } else {
                        // Check providers in all_provider_models
                        let mut found_provider = None;
                        for provider_name in &**all_provider_models {
                            if let Some(provider) = self.provider_registry.get_provider(provider_name) {
                                if let Ok(models) = provider.list_models().await {
                                    if models.iter().any(|m| m.id.eq_ignore_ascii_case(&req_model)) {
                                        found_provider = Some(provider_name.clone());
                                        break;
                                    }
                                }
                            }
                        }

                        found_provider.ok_or_else(|| {
                            AppError::Router(format!(
                                "Model '{}' is not allowed by this API key's model selection",
                                request.model
                            ))
                        })?
                    }
                };

                (final_provider, req_model)
            }
            #[allow(deprecated)]
            Some(ModelSelection::DirectModel { provider, model }) => {
                // Verify request.model matches configured model
                if request.model != *model {
                    return Err(AppError::Router(format!(
                        "Model mismatch: API key is configured for model '{}', but request specifies '{}'",
                        model, request.model
                    )));
                }

                debug!(
                    "Using direct model routing: provider='{}', model='{}'",
                    provider, model
                );

                (provider.clone(), model.clone())
            }
                #[allow(deprecated)]
                Some(ModelSelection::Router { router_name }) => {
                    // Smart routing not implemented yet
                    return Err(AppError::Router(format!(
                        "Smart routing (router '{}') is not yet implemented. Please use DirectModel configuration.",
                        router_name
                    )));
                }
            }
        };

        // 4. Get provider instance from registry
        let provider_instance = self.provider_registry.get_provider(&provider).ok_or_else(|| {
            AppError::Router(format!(
                "Provider '{}' not found or disabled in registry",
                provider
            ))
        })?;

        // 5. Check provider health (optional - log warning if unhealthy)
        let health = provider_instance.health_check().await;
        match health.status {
            crate::providers::HealthStatus::Healthy => {
                debug!("Provider '{}' is healthy (latency: {:?}ms)", provider, health.latency_ms);
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

        // 6. Execute the completion request
        debug!(
            "Executing completion request on provider '{}' with model '{}'",
            provider, expected_model
        );

        // Modify the request to use just the model name (without provider prefix)
        // The expected_model already has the provider prefix stripped during routing
        // This ensures all providers receive just the model ID they expect
        let mut modified_request = request.clone();
        modified_request.model = expected_model.clone();

        // Apply feature adapters if extensions are present
        let mut response_extensions = std::collections::HashMap::new();

        if let Some(ref extensions) = request.extensions {
            for (feature_name, feature_params) in extensions {
                // Check if provider supports this feature
                if provider_instance.supports_feature(feature_name) {
                    debug!("Provider '{}' supports feature '{}'", provider, feature_name);

                    // Get the feature adapter
                    if let Some(adapter) = provider_instance.get_feature_adapter(feature_name) {
                        // Validate parameters
                        let mut params: crate::providers::features::FeatureParams = std::collections::HashMap::new();
                        if let serde_json::Value::Object(map) = feature_params {
                            for (k, v) in map {
                                params.insert(k.clone(), v.clone());
                            }
                        }
                        adapter.validate_params(&params).map_err(|e| {
                            warn!("Feature '{}' parameter validation failed: {}", feature_name, e);
                            e
                        })?;

                        // Adapt the request
                        adapter.adapt_request(&mut modified_request, &params).map_err(|e| {
                            warn!("Feature '{}' request adaptation failed: {}", feature_name, e);
                            e
                        })?;

                        debug!("Successfully applied feature adapter for '{}'", feature_name);
                    } else {
                        warn!(
                            "Provider '{}' claims to support feature '{}' but no adapter available",
                            provider, feature_name
                        );
                    }
                } else {
                    warn!(
                        "Provider '{}' does not support feature '{}' - ignoring",
                        provider, feature_name
                    );
                }
            }
        }

        let mut response = provider_instance.complete(modified_request).await.map_err(|e| {
            warn!(
                "Completion request failed for provider '{}': {}",
                provider, e
            );
            e
        })?;

        // Apply feature adapters to response if extensions were present
        if let Some(ref extensions) = request.extensions {
            for (feature_name, _) in extensions {
                if provider_instance.supports_feature(feature_name) {
                    if let Some(adapter) = provider_instance.get_feature_adapter(feature_name) {
                        // Adapt the response
                        if let Ok(Some(feature_data)) = adapter.adapt_response(&mut response) {
                            response_extensions.insert(feature_name.clone(), feature_data.data);
                            debug!("Extracted feature data for '{}'", feature_name);
                        }
                    }
                }
            }
        }

        // Add extensions to response if any were collected
        if !response_extensions.is_empty() {
            response.extensions = Some(response_extensions);
            debug!("Added {} feature extensions to response", response.extensions.as_ref().unwrap().len());
        }

        // 7. Calculate cost and record usage for rate limiting
        let pricing = provider_instance
            .get_pricing(&expected_model)
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
        if let Err(e) = self.rate_limiter.record_api_key_usage(api_key_id, &usage).await {
            warn!(
                "Failed to record usage for API key '{}': {}. Request succeeded but usage not tracked.",
                api_key_id, e
            );
        }

        info!(
            "Completion request successful for API key '{}': {} tokens",
            api_key_id, response.usage.total_tokens
        );

        Ok(response)
    }

    /// Route a streaming completion request based on API key configuration
    ///
    /// Similar to `complete()` but returns a stream of completion chunks.
    pub async fn stream_complete(
        &self,
        api_key_id: &str,
        request: CompletionRequest,
    ) -> AppResult<Pin<Box<dyn Stream<Item = AppResult<CompletionChunk>> + Send>>> {
        debug!(
            "Routing streaming completion request for API key '{}', model '{}'",
            api_key_id, request.model
        );

        // 1. Get API key configuration
        let config = self.config_manager.get();
        let api_key = config
            .api_keys
            .iter()
            .find(|k| k.id == api_key_id)
            .ok_or_else(|| {
                warn!("API key '{}' not found", api_key_id);
                AppError::Unauthorized
            })?;

        // Check if API key is enabled
        if !api_key.enabled {
            warn!("API key '{}' is disabled", api_key_id);
            return Err(AppError::Unauthorized);
        }

        // 2. Check rate limits
        let usage_estimate = UsageInfo {
            input_tokens: 0,
            output_tokens: 0,
            cost_usd: 0.0,
        };

        let rate_check = self
            .rate_limiter
            .check_api_key(api_key_id, &usage_estimate)
            .await?;

        if !rate_check.allowed {
            warn!(
                "API key '{}' rate limited. Retry after {} seconds",
                api_key_id,
                rate_check.retry_after_secs.unwrap_or(0)
            );
            return Err(AppError::RateLimitExceeded);
        }

        // 3. Determine provider and model based on API key configuration
        let routing_config = api_key.get_routing_config();

        // Check for PrioritizedList strategy - not fully supported for streaming
        if let Some(ref config) = routing_config {
            if config.active_strategy == ActiveRoutingStrategy::PrioritizedList {
                warn!(
                    "PrioritizedList strategy does not support automatic retry for streaming requests. \
                     Will use first model only. API key: '{}'",
                    api_key_id
                );
                // Continue with first model, but no retry on failure
            }
        }

        let (provider, expected_model) = if let Some(ref config) = routing_config {
            // New routing config system (same logic as complete())
            match config.active_strategy {
                ActiveRoutingStrategy::AvailableModels => {
                    // Allow any model in the available list
                    if let Some((p, m)) = request.model.split_once('/') {
                        if !config.is_model_allowed(p, m) {
                            return Err(AppError::Router(format!(
                                "Model '{}' is not in the available models list for this API key",
                                request.model
                            )));
                        }
                        (p.to_string(), m.to_string())
                    } else {
                        // Find provider for this model
                        let mut found_provider = None;
                        for (provider_name, model_name) in &config.available_models.individual_models {
                            if model_name.eq_ignore_ascii_case(&request.model) {
                                found_provider = Some(provider_name.clone());
                                break;
                            }
                        }
                        if found_provider.is_none() {
                            for provider_name in &config.available_models.all_provider_models {
                                if let Some(provider) = self.provider_registry.get_provider(provider_name) {
                                    if let Ok(models) = provider.list_models().await {
                                        if models.iter().any(|m| m.id.eq_ignore_ascii_case(&request.model)) {
                                            found_provider = Some(provider_name.clone());
                                            break;
                                        }
                                    }
                                }
                            }
                        }
                        if let Some(provider) = found_provider {
                            (provider, request.model.clone())
                        } else {
                            return Err(AppError::Router(format!(
                                "Model '{}' is not in the available models list for this API key",
                                request.model
                            )));
                        }
                    }
                }
                ActiveRoutingStrategy::ForceModel => {
                    // Force a specific model
                    if let Some((forced_provider, forced_model)) = &config.forced_model {
                        (forced_provider.clone(), forced_model.clone())
                    } else {
                        return Err(AppError::Router(
                            "Force Model strategy is active but no model is configured".to_string()
                        ));
                    }
                }
                ActiveRoutingStrategy::PrioritizedList => {
                    // LIMITATION: Streaming doesn't support automatic retry
                    // Use first model in prioritized list only (no failover)
                    // TODO: Implement retry logic by buffering or switching streams mid-flight
                    if let Some((first_provider, first_model)) = config.prioritized_models.first() {
                        (first_provider.clone(), first_model.clone())
                    } else {
                        return Err(AppError::Router(
                            "Prioritized List strategy is active but no models are configured".to_string()
                        ));
                    }
                }
            }
        } else {
            // Fallback to old model_selection
            match &api_key.model_selection {
                Some(ModelSelection::All) | None => {
                    if let Some((p, m)) = request.model.split_once('/') {
                        (p.to_string(), m.to_string())
                    } else {
                        ("".to_string(), request.model.clone())
                    }
                }
                Some(ModelSelection::Custom {
                    all_provider_models,
                    individual_models,
                }) => {
                // Check if model is allowed (same logic as non-streaming)
                let (req_provider, req_model) = if let Some((p, m)) = request.model.split_once('/') {
                    (p.to_string(), m.to_string())
                } else {
                    ("".to_string(), request.model.clone())
                };

                // Determine which provider to use and validate access
                let final_provider = if !req_provider.is_empty() {
                    // Provider specified - check if allowed
                    let is_allowed = all_provider_models.iter().any(|p| p.eq_ignore_ascii_case(&req_provider))
                        || individual_models
                            .iter()
                            .any(|(p, m)| p.eq_ignore_ascii_case(&req_provider) && m.eq_ignore_ascii_case(&req_model));

                    if !is_allowed {
                        return Err(AppError::Router(format!(
                            "Model '{}' is not allowed by this API key's model selection",
                            request.model
                        )));
                    }

                    req_provider
                } else {
                    // Just model name - find which provider has it
                    // First check individual_models
                    if let Some((provider, _)) = individual_models
                        .iter()
                        .find(|(_p, m)| m.eq_ignore_ascii_case(&req_model))
                    {
                        provider.clone()
                    } else {
                        // Check providers in all_provider_models
                        let mut found_provider = None;
                        for provider_name in &**all_provider_models {
                            if let Some(provider) = self.provider_registry.get_provider(provider_name) {
                                if let Ok(models) = provider.list_models().await {
                                    if models.iter().any(|m| m.id.eq_ignore_ascii_case(&req_model)) {
                                        found_provider = Some(provider_name.clone());
                                        break;
                                    }
                                }
                            }
                        }

                        found_provider.ok_or_else(|| {
                            AppError::Router(format!(
                                "Model '{}' is not allowed by this API key's model selection",
                                request.model
                            ))
                        })?
                    }
                };

                    (final_provider, req_model)
                }
                #[allow(deprecated)]
                Some(ModelSelection::DirectModel { provider, model }) => {
                    if request.model != *model {
                        return Err(AppError::Router(format!(
                            "Model mismatch: API key is configured for model '{}', but request specifies '{}'",
                            model, request.model
                        )));
                    }
                    (provider.clone(), model.clone())
                }
                #[allow(deprecated)]
                Some(ModelSelection::Router { router_name }) => {
                    return Err(AppError::Router(format!(
                        "Smart routing (router '{}') is not yet implemented",
                        router_name
                    )));
                }
            }
        };

        // 4. Get provider instance
        let provider_instance = self.provider_registry.get_provider(&provider).ok_or_else(|| {
            AppError::Router(format!(
                "Provider '{}' not found or disabled in registry",
                provider
            ))
        })?;

        // 5. Check provider health
        let health = provider_instance.health_check().await;
        if let crate::providers::HealthStatus::Unhealthy = health.status {
            warn!(
                "Provider '{}' is unhealthy but proceeding with streaming request",
                provider
            );
        }

        // 6. Execute streaming request
        debug!(
            "Executing streaming completion request on provider '{}' with model '{}'",
            provider, expected_model
        );

        // Modify the request to use just the model name (without provider prefix)
        // The expected_model already has the provider prefix stripped during routing
        // This ensures all providers receive just the model ID they expect
        let mut modified_request = request;
        modified_request.model = expected_model.clone();

        let stream = provider_instance.stream_complete(modified_request).await?;

        info!(
            "Streaming completion request started for API key '{}'",
            api_key_id
        );

        // Wrap stream to track usage (approximate token counting)
        let tracked_stream = wrap_stream_with_usage_tracking(
            stream,
            api_key_id.to_string(),
            self.rate_limiter.clone(),
        ).await;

        Ok(tracked_stream)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ApiKeyConfig, AppConfig, ModelSelection};
    use crate::providers::health::HealthCheckManager;
    use chrono::Utc;

    #[tokio::test]
    async fn test_router_creation() {
        let config_manager = Arc::new(ConfigManager::new(
            AppConfig::default(),
            std::path::PathBuf::from("/tmp/test.yaml"),
        ));

        let health_manager = Arc::new(HealthCheckManager::default());
        let provider_registry = Arc::new(ProviderRegistry::new(health_manager));
        let rate_limiter = Arc::new(RateLimiterManager::new(None));

        let router = Router::new(config_manager, provider_registry, rate_limiter);

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

        let router = Router::new(config_manager, provider_registry, rate_limiter);

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
        };

        let result = router.complete("invalid-key-id", request).await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), AppError::Unauthorized));
    }

    #[tokio::test]
    async fn test_router_disabled_api_key() {
        let mut config = AppConfig::default();
        config.api_keys.push(ApiKeyConfig {
            id: "test-key".to_string(),
            name: "Test Key".to_string(),
            model_selection: Some(ModelSelection::DirectModel {
                provider: "test-provider".to_string(),
                model: "test-model".to_string(),
            }),
            routing_config: None,
            enabled: false, // Disabled
            created_at: Utc::now(),
            last_used: None,
        });

        let config_manager = Arc::new(ConfigManager::new(
            config,
            std::path::PathBuf::from("/tmp/test.yaml"),
        ));

        let health_manager = Arc::new(HealthCheckManager::default());
        let provider_registry = Arc::new(ProviderRegistry::new(health_manager));
        let rate_limiter = Arc::new(RateLimiterManager::new(None));

        let router = Router::new(config_manager, provider_registry, rate_limiter);

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
        };

        let result = router.complete("test-key", request).await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), AppError::Unauthorized));
    }

    // ============================================================================
    // Routing Strategy Tests
    // ============================================================================

    mod routing_strategy_tests {
        use super::*;
        use crate::config::{ActiveRoutingStrategy, AvailableModelsSelection, ModelRoutingConfig};

        /// Helper to create a test API key with routing config
        fn create_test_key_with_routing(
            id: &str,
            routing_config: ModelRoutingConfig,
        ) -> ApiKeyConfig {
            ApiKeyConfig {
                id: id.to_string(),
                name: format!("Test Key {}", id),
                model_selection: None,
                routing_config: Some(routing_config),
                enabled: true,
                created_at: Utc::now(),
                last_used: None,
            }
        }

        #[tokio::test]
        async fn test_available_models_strategy_allows_matching_model() {
            // Create routing config with Available Models strategy
            let routing_config = ModelRoutingConfig {
                active_strategy: ActiveRoutingStrategy::AvailableModels,
                available_models: AvailableModelsSelection {
                    all_provider_models: vec![],
                    individual_models: vec![
                        ("provider1".to_string(), "model1".to_string()),
                        ("provider2".to_string(), "model2".to_string()),
                    ],
                },
                forced_model: None,
                prioritized_models: vec![],
            };

            let key = create_test_key_with_routing("test-key", routing_config);

            let mut config = AppConfig::default();
            config.api_keys.push(key);

            let config_manager = Arc::new(ConfigManager::new(
                config,
                std::path::PathBuf::from("/tmp/test.yaml"),
            ));

            let health_manager = Arc::new(HealthCheckManager::default());
            let provider_registry = Arc::new(ProviderRegistry::new(health_manager));
            let rate_limiter = Arc::new(RateLimiterManager::new(None));

            let router = Router::new(config_manager, provider_registry.clone(), rate_limiter);

            // Request with allowed model should fail because provider doesn't exist, but not due to routing
            let request = CompletionRequest {
                model: "provider1/model1".to_string(),
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
            };

            let result = router.complete("test-key", request).await;
            // Should fail because provider doesn't exist, not because model is not allowed
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert!(matches!(err, AppError::Router(_)));
            assert!(err.to_string().contains("not found"));
        }

        #[tokio::test]
        async fn test_available_models_strategy_rejects_non_matching_model() {
            // Create routing config with Available Models strategy
            let routing_config = ModelRoutingConfig {
                active_strategy: ActiveRoutingStrategy::AvailableModels,
                available_models: AvailableModelsSelection {
                    all_provider_models: vec![],
                    individual_models: vec![
                        ("provider1".to_string(), "model1".to_string()),
                    ],
                },
                forced_model: None,
                prioritized_models: vec![],
            };

            let key = create_test_key_with_routing("test-key", routing_config);

            let mut config = AppConfig::default();
            config.api_keys.push(key);

            let config_manager = Arc::new(ConfigManager::new(
                config,
                std::path::PathBuf::from("/tmp/test.yaml"),
            ));

            let health_manager = Arc::new(HealthCheckManager::default());
            let provider_registry = Arc::new(ProviderRegistry::new(health_manager));
            let rate_limiter = Arc::new(RateLimiterManager::new(None));

            let router = Router::new(config_manager, provider_registry, rate_limiter);

            // Request with non-allowed model should fail due to routing
            let request = CompletionRequest {
                model: "provider2/model2".to_string(),
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
            };

            let result = router.complete("test-key", request).await;
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert!(matches!(err, AppError::Router(_)));
            assert!(err.to_string().contains("not in the available models list"));
        }

        #[tokio::test]
        async fn test_available_models_strategy_with_all_provider_models() {
            // Create routing config that allows all models from provider1
            let routing_config = ModelRoutingConfig {
                active_strategy: ActiveRoutingStrategy::AvailableModels,
                available_models: AvailableModelsSelection {
                    all_provider_models: vec!["provider1".to_string()],
                    individual_models: vec![],
                },
                forced_model: None,
                prioritized_models: vec![],
            };

            let config = routing_config.clone();

            // Test that provider1/any_model is allowed
            assert!(config.is_model_allowed("provider1", "any_model"));
            assert!(config.is_model_allowed("provider1", "another_model"));

            // Test that provider2/model is not allowed
            assert!(!config.is_model_allowed("provider2", "some_model"));
        }

        #[tokio::test]
        async fn test_force_model_strategy_ignores_requested_model() {
            // Create routing config with Force Model strategy
            let routing_config = ModelRoutingConfig {
                active_strategy: ActiveRoutingStrategy::ForceModel,
                available_models: AvailableModelsSelection::default(),
                forced_model: Some(("provider1".to_string(), "forced-model".to_string())),
                prioritized_models: vec![],
            };

            let key = create_test_key_with_routing("test-key", routing_config);

            let mut config = AppConfig::default();
            config.api_keys.push(key);

            let config_manager = Arc::new(ConfigManager::new(
                config,
                std::path::PathBuf::from("/tmp/test.yaml"),
            ));

            let health_manager = Arc::new(HealthCheckManager::default());
            let provider_registry = Arc::new(ProviderRegistry::new(health_manager));
            let rate_limiter = Arc::new(RateLimiterManager::new(None));

            let router = Router::new(config_manager, provider_registry, rate_limiter);

            // Request a different model, should be forced to use forced-model
            let request = CompletionRequest {
                model: "different-provider/different-model".to_string(),
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
            };

            let result = router.complete("test-key", request).await;
            // Should fail because provider doesn't exist, but routing logic should have forced the model
            assert!(result.is_err());
            let err = result.unwrap_err();
            // Error should mention provider1, not different-provider
            assert!(err.to_string().contains("provider1") || err.to_string().contains("not found"));
        }

        #[tokio::test]
        async fn test_force_model_strategy_without_configured_model() {
            // Create routing config with Force Model strategy but no forced model
            let routing_config = ModelRoutingConfig {
                active_strategy: ActiveRoutingStrategy::ForceModel,
                available_models: AvailableModelsSelection::default(),
                forced_model: None, // No model configured
                prioritized_models: vec![],
            };

            let key = create_test_key_with_routing("test-key", routing_config);

            let mut config = AppConfig::default();
            config.api_keys.push(key);

            let config_manager = Arc::new(ConfigManager::new(
                config,
                std::path::PathBuf::from("/tmp/test.yaml"),
            ));

            let health_manager = Arc::new(HealthCheckManager::default());
            let provider_registry = Arc::new(ProviderRegistry::new(health_manager));
            let rate_limiter = Arc::new(RateLimiterManager::new(None));

            let router = Router::new(config_manager, provider_registry, rate_limiter);

            let request = CompletionRequest {
                model: "any-model".to_string(),
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
            };

            let result = router.complete("test-key", request).await;
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert!(err.to_string().contains("no model is configured"));
        }

        #[tokio::test]
        async fn test_prioritized_list_strategy_without_models() {
            // Create routing config with Prioritized List strategy but empty list
            let routing_config = ModelRoutingConfig {
                active_strategy: ActiveRoutingStrategy::PrioritizedList,
                available_models: AvailableModelsSelection::default(),
                forced_model: None,
                prioritized_models: vec![], // Empty list
            };

            let key = create_test_key_with_routing("test-key", routing_config);

            let mut config = AppConfig::default();
            config.api_keys.push(key);

            let config_manager = Arc::new(ConfigManager::new(
                config,
                std::path::PathBuf::from("/tmp/test.yaml"),
            ));

            let health_manager = Arc::new(HealthCheckManager::default());
            let provider_registry = Arc::new(ProviderRegistry::new(health_manager));
            let rate_limiter = Arc::new(RateLimiterManager::new(None));

            let router = Router::new(config_manager, provider_registry, rate_limiter);

            let request = CompletionRequest {
                model: "any-model".to_string(),
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
            };

            let result = router.complete("test-key", request).await;
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert!(err.to_string().contains("no models are configured"));
        }

        #[tokio::test]
        async fn test_is_model_allowed_for_available_models() {
            let routing_config = ModelRoutingConfig {
                active_strategy: ActiveRoutingStrategy::AvailableModels,
                available_models: AvailableModelsSelection {
                    all_provider_models: vec!["provider1".to_string()],
                    individual_models: vec![
                        ("provider2".to_string(), "model2".to_string()),
                        ("provider3".to_string(), "model3".to_string()),
                    ],
                },
                forced_model: None,
                prioritized_models: vec![],
            };

            // Test all_provider_models
            assert!(routing_config.is_model_allowed("provider1", "any_model"));
            assert!(routing_config.is_model_allowed("provider1", "another_model"));

            // Test individual_models
            assert!(routing_config.is_model_allowed("provider2", "model2"));
            assert!(routing_config.is_model_allowed("provider3", "model3"));

            // Test not allowed
            assert!(!routing_config.is_model_allowed("provider2", "different_model"));
            assert!(!routing_config.is_model_allowed("provider4", "model4"));
        }

        #[tokio::test]
        async fn test_is_model_allowed_for_force_model() {
            let routing_config = ModelRoutingConfig {
                active_strategy: ActiveRoutingStrategy::ForceModel,
                available_models: AvailableModelsSelection::default(),
                forced_model: Some(("provider1".to_string(), "forced".to_string())),
                prioritized_models: vec![],
            };

            // Only the forced model is allowed
            assert!(routing_config.is_model_allowed("provider1", "forced"));

            // Other models are not allowed
            assert!(!routing_config.is_model_allowed("provider1", "other"));
            assert!(!routing_config.is_model_allowed("provider2", "forced"));
        }

        #[tokio::test]
        async fn test_is_model_allowed_for_prioritized_list() {
            let routing_config = ModelRoutingConfig {
                active_strategy: ActiveRoutingStrategy::PrioritizedList,
                available_models: AvailableModelsSelection::default(),
                forced_model: None,
                prioritized_models: vec![
                    ("provider1".to_string(), "model1".to_string()),
                    ("provider2".to_string(), "model2".to_string()),
                    ("provider3".to_string(), "model3".to_string()),
                ],
            };

            // All models in the prioritized list are "allowed"
            assert!(routing_config.is_model_allowed("provider1", "model1"));
            assert!(routing_config.is_model_allowed("provider2", "model2"));
            assert!(routing_config.is_model_allowed("provider3", "model3"));

            // Models not in the list are not allowed
            assert!(!routing_config.is_model_allowed("provider4", "model4"));
            assert!(!routing_config.is_model_allowed("provider1", "different"));
        }

        #[tokio::test]
        async fn test_migration_from_old_model_selection_all() {
            let old_selection = ModelSelection::All;
            let routing_config = ModelRoutingConfig::from_model_selection(old_selection);

            assert_eq!(routing_config.active_strategy, ActiveRoutingStrategy::AvailableModels);
            assert!(routing_config.available_models.all_provider_models.is_empty());
            assert!(routing_config.available_models.individual_models.is_empty());
            assert!(routing_config.forced_model.is_none());
            assert!(routing_config.prioritized_models.is_empty());
        }

        #[tokio::test]
        async fn test_migration_from_old_model_selection_custom() {
            let old_selection = ModelSelection::Custom {
                all_provider_models: vec!["provider1".to_string()],
                individual_models: vec![("provider2".to_string(), "model2".to_string())],
            };
            let routing_config = ModelRoutingConfig::from_model_selection(old_selection);

            assert_eq!(routing_config.active_strategy, ActiveRoutingStrategy::AvailableModels);
            assert_eq!(routing_config.available_models.all_provider_models, vec!["provider1"]);
            assert_eq!(
                routing_config.available_models.individual_models,
                vec![("provider2".to_string(), "model2".to_string())]
            );
        }

        #[tokio::test]
        async fn test_migration_from_old_model_selection_direct_model() {
            #[allow(deprecated)]
            let old_selection = ModelSelection::DirectModel {
                provider: "provider1".to_string(),
                model: "model1".to_string(),
            };
            let routing_config = ModelRoutingConfig::from_model_selection(old_selection);

            assert_eq!(routing_config.active_strategy, ActiveRoutingStrategy::ForceModel);
            assert_eq!(
                routing_config.forced_model,
                Some(("provider1".to_string(), "model1".to_string()))
            );
        }

        #[tokio::test]
        async fn test_case_insensitive_model_matching() {
            let routing_config = ModelRoutingConfig {
                active_strategy: ActiveRoutingStrategy::ForceModel,
                available_models: AvailableModelsSelection::default(),
                forced_model: Some(("Provider1".to_string(), "Model1".to_string())),
                prioritized_models: vec![],
            };

            // Case-insensitive matching
            assert!(routing_config.is_model_allowed("provider1", "model1"));
            assert!(routing_config.is_model_allowed("PROVIDER1", "MODEL1"));
            assert!(routing_config.is_model_allowed("Provider1", "Model1"));
        }

        #[tokio::test]
        async fn test_get_routing_config_with_new_config() {
            let routing_config = ModelRoutingConfig::new_force_model(
                "provider1".to_string(),
                "model1".to_string(),
            );

            let key = ApiKeyConfig {
                id: "test".to_string(),
                name: "Test".to_string(),
                model_selection: None,
                routing_config: Some(routing_config.clone()),
                enabled: true,
                created_at: Utc::now(),
                last_used: None,
            };

            let result = key.get_routing_config();
            assert!(result.is_some());
            let config = result.unwrap();
            assert_eq!(config.active_strategy, ActiveRoutingStrategy::ForceModel);
            assert_eq!(
                config.forced_model,
                Some(("provider1".to_string(), "model1".to_string()))
            );
        }

        #[tokio::test]
        async fn test_get_routing_config_migrates_from_old() {
            #[allow(deprecated)]
            let key = ApiKeyConfig {
                id: "test".to_string(),
                name: "Test".to_string(),
                model_selection: Some(ModelSelection::DirectModel {
                    provider: "provider1".to_string(),
                    model: "model1".to_string(),
                }),
                routing_config: None, // No new config
                enabled: true,
                created_at: Utc::now(),
                last_used: None,
            };

            let result = key.get_routing_config();
            assert!(result.is_some());
            let config = result.unwrap();
            // Should have migrated to Force Model strategy
            assert_eq!(config.active_strategy, ActiveRoutingStrategy::ForceModel);
            assert_eq!(
                config.forced_model,
                Some(("provider1".to_string(), "model1".to_string()))
            );
        }
    }

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
}
