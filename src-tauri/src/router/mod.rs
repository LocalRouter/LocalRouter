//! Smart routing system
//!
//! Routes incoming requests to appropriate model providers based on API key configuration.

use std::sync::Arc;

use futures::{Stream, StreamExt};
use std::pin::Pin;
use tracing::{debug, info, warn};

use crate::config::{ActiveRoutingStrategy, ConfigManager};
use crate::providers::registry::ProviderRegistry;
use crate::providers::{CompletionChunk, CompletionRequest, CompletionResponse};
use crate::utils::errors::{AppError, AppResult};

pub mod rate_limit;

// Re-export commonly used types
pub use rate_limit::{RateLimiterManager, UsageInfo};

/// Wraps a completion stream to count tokens and record usage when complete
///
/// This is an approximation: we estimate tokens based on content length
/// since streaming chunks don't include token counts.
async fn wrap_stream_with_usage_tracking(
    stream: Pin<Box<dyn Stream<Item = AppResult<CompletionChunk>> + Send>>,
    client_id: String,
    rate_limiter: Arc<RateLimiterManager>,
) -> Pin<Box<dyn Stream<Item = AppResult<CompletionChunk>> + Send>> {
    use std::sync::atomic::{AtomicU64, Ordering};

    // Track token counts as stream progresses
    let completion_chars = Arc::new(AtomicU64::new(0));

    // Clone for inspect closure
    let completion_chars_for_inspect = completion_chars.clone();

    // Clone for then closure
    let completion_chars_for_then = completion_chars.clone();
    let client_id_for_then = client_id.clone();
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
    /// 2. Checks rate limits
    /// 3. Routes to the configured provider+model based on routing config
    /// 4. Executes the request
    /// 5. Records usage for rate limiting
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

        // Special handling for internal test token (bypasses routing config)
        if client_id == "internal-test" {
            debug!("Internal test token detected - bypassing routing config");
            return self.complete_direct(request).await;
        }

        // 1. Get client configuration
        let config = self.config_manager.get();
        let client = config
            .clients
            .iter()
            .find(|c| c.id == client_id)
            .ok_or_else(|| {
                warn!("Client '{}' not found", client_id);
                AppError::Unauthorized
            })?;

        // Check if client is enabled
        if !client.enabled {
            warn!("Client '{}' is disabled", client_id);
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
            .check_api_key(client_id, &usage_estimate)
            .await?;

        if !rate_check.allowed {
            warn!(
                "API key '{}' rate limited. Retry after {} seconds",
                client_id,
                rate_check.retry_after_secs.unwrap_or(0)
            );
            return Err(AppError::RateLimitExceeded);
        }

        // 3. Determine provider and model based on client routing configuration
        let routing_config = client.routing_config.as_ref();

        // Special handling for Prioritized List: use retry logic
        if let Some(config) = routing_config {
            if config.active_strategy == ActiveRoutingStrategy::PrioritizedList {
                if config.prioritized_models.is_empty() {
                    return Err(AppError::Router(
                        "Prioritized List strategy is active but no models are configured"
                            .to_string(),
                    ));
                }

                info!(
                    "Using Prioritized List strategy with {} models for API key '{}'",
                    config.prioritized_models.len(),
                    client_id
                );

                // Use retry logic for prioritized list
                return self
                    .complete_with_prioritized_list(client_id, &config.prioritized_models, request)
                    .await;
            }
        }

        let (provider, expected_model) = if let Some(config) = routing_config {
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
                        for (provider_name, model_name) in
                            &config.available_models.individual_models
                        {
                            if model_name.eq_ignore_ascii_case(&request.model) {
                                found_provider = Some(provider_name.clone());
                                break;
                            }
                        }

                        // If not found, check providers in all_provider_models
                        if found_provider.is_none() {
                            for provider_name in &config.available_models.all_provider_models {
                                if let Some(provider) =
                                    self.provider_registry.get_provider(provider_name)
                                {
                                    if let Ok(models) = provider.list_models().await {
                                        if models
                                            .iter()
                                            .any(|m| m.id.eq_ignore_ascii_case(&request.model))
                                        {
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
                            "Force Model strategy is active but no model is configured".to_string(),
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
            // No routing config - allow any model from the request
            // Parse the model from request (format: "provider/model" or just "model")
            if let Some((p, m)) = request.model.split_once('/') {
                debug!("Using provider/model from request: {}/{}", p, m);
                (p.to_string(), m.to_string())
            } else {
                // Just a model name - need to find which provider has it
                debug!("Model name only: {}", request.model);
                // This will be handled below when we get the provider
                ("".to_string(), request.model.clone())
            }
        };

        // 4. Get provider instance from registry
        let provider_instance =
            self.provider_registry
                .get_provider(&provider)
                .ok_or_else(|| {
                    AppError::Router(format!(
                        "Provider '{}' not found or disabled in registry",
                        provider
                    ))
                })?;

        // 5. Check provider health (optional - log warning if unhealthy)
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

        let mut response = provider_instance
            .complete(modified_request)
            .await
            .map_err(|e| {
                warn!(
                    "Completion request failed for provider '{}': {}",
                    provider, e
                );
                e
            })?;

        // Apply feature adapters to response if extensions were present
        if let Some(ref extensions) = request.extensions {
            for feature_name in extensions.keys() {
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
            debug!(
                "Added {} feature extensions to response",
                response.extensions.as_ref().unwrap().len()
            );
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

        info!(
            "Completion request successful for API key '{}': {} tokens",
            client_id, response.usage.total_tokens
        );

        Ok(response)
    }

    /// Direct completion bypass (for internal test token)
    ///
    /// This bypasses all routing config, rate limiting, and client validation.
    /// Used only for the internal test token to allow direct provider testing from the UI.
    async fn complete_direct(&self, request: CompletionRequest) -> AppResult<CompletionResponse> {
        debug!("Direct completion request for model '{}'", request.model);

        // Parse the model from request (format: "provider/model")
        let (provider, model) = if let Some((p, m)) = request.model.split_once('/') {
            (p.to_string(), m.to_string())
        } else {
            return Err(AppError::Router(format!(
                "Model must be in format 'provider/model' for direct access. Got: '{}'",
                request.model
            )));
        };

        // Get provider instance from registry
        let provider_instance =
            self.provider_registry
                .get_provider(&provider)
                .ok_or_else(|| {
                    AppError::Router(format!(
                        "Provider '{}' not found or disabled in registry",
                        provider
                    ))
                })?;

        // Execute the completion request directly
        debug!(
            "Executing direct completion request on provider '{}' with model '{}'",
            provider, model
        );

        let mut modified_request = request.clone();
        modified_request.model = model.clone();

        let response = provider_instance
            .complete(modified_request)
            .await
            .map_err(|e| {
                warn!(
                    "Direct completion request failed for provider '{}': {}",
                    provider, e
                );
                e
            })?;

        info!(
            "Direct completion request successful: provider='{}', model='{}', {} tokens",
            provider, model, response.usage.total_tokens
        );

        Ok(response)
    }

    /// Route a streaming completion request based on API key configuration
    ///
    /// Similar to `complete()` but returns a stream of completion chunks.
    pub async fn stream_complete(
        &self,
        client_id: &str,
        request: CompletionRequest,
    ) -> AppResult<Pin<Box<dyn Stream<Item = AppResult<CompletionChunk>> + Send>>> {
        debug!(
            "Routing streaming completion request for client '{}', model '{}'",
            client_id, request.model
        );

        // Special handling for internal test token (bypasses routing config)
        if client_id == "internal-test" {
            debug!("Internal test token detected - bypassing routing config for streaming");
            return self.stream_complete_direct(request).await;
        }

        // 1. Get client configuration
        let config = self.config_manager.get();
        let client = config
            .clients
            .iter()
            .find(|c| c.id == client_id)
            .ok_or_else(|| {
                warn!("Client '{}' not found", client_id);
                AppError::Unauthorized
            })?;

        // Check if client is enabled
        if !client.enabled {
            warn!("Client '{}' is disabled", client_id);
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
            .check_api_key(client_id, &usage_estimate)
            .await?;

        if !rate_check.allowed {
            warn!(
                "API key '{}' rate limited. Retry after {} seconds",
                client_id,
                rate_check.retry_after_secs.unwrap_or(0)
            );
            return Err(AppError::RateLimitExceeded);
        }

        // 3. Determine provider and model based on client routing configuration
        let routing_config = client.routing_config.as_ref();

        // Check for PrioritizedList strategy - not fully supported for streaming
        if let Some(config) = routing_config {
            if config.active_strategy == ActiveRoutingStrategy::PrioritizedList {
                warn!(
                    "PrioritizedList strategy does not support automatic retry for streaming requests. \
                     Will use first model only. API key: '{}'",
                    client_id
                );
                // Continue with first model, but no retry on failure
            }
        }

        let (provider, expected_model) = if let Some(config) = routing_config {
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
                        for (provider_name, model_name) in
                            &config.available_models.individual_models
                        {
                            if model_name.eq_ignore_ascii_case(&request.model) {
                                found_provider = Some(provider_name.clone());
                                break;
                            }
                        }
                        if found_provider.is_none() {
                            for provider_name in &config.available_models.all_provider_models {
                                if let Some(provider) =
                                    self.provider_registry.get_provider(provider_name)
                                {
                                    if let Ok(models) = provider.list_models().await {
                                        if models
                                            .iter()
                                            .any(|m| m.id.eq_ignore_ascii_case(&request.model))
                                        {
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
                            "Force Model strategy is active but no model is configured".to_string(),
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
                            "Prioritized List strategy is active but no models are configured"
                                .to_string(),
                        ));
                    }
                }
            }
        } else {
            // No routing config - allow any model from the request
            // Parse the model from request (format: "provider/model" or just "model")
            if let Some((p, m)) = request.model.split_once('/') {
                debug!("Using provider/model from request: {}/{}", p, m);
                (p.to_string(), m.to_string())
            } else {
                // Just a model name - need to find which provider has it
                debug!("Model name only: {}", request.model);
                // This will be handled below when we get the provider
                ("".to_string(), request.model.clone())
            }
        };

        // 4. Get provider instance
        let provider_instance =
            self.provider_registry
                .get_provider(&provider)
                .ok_or_else(|| {
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
            client_id
        );

        // Wrap stream to track usage (approximate token counting)
        let tracked_stream = wrap_stream_with_usage_tracking(
            stream,
            client_id.to_string(),
            self.rate_limiter.clone(),
        )
        .await;

        Ok(tracked_stream)
    }

    /// Direct streaming completion bypass (for internal test token)
    ///
    /// This bypasses all routing config, rate limiting, and client validation.
    /// Used only for the internal test token to allow direct provider testing from the UI.
    async fn stream_complete_direct(
        &self,
        request: CompletionRequest,
    ) -> AppResult<Pin<Box<dyn Stream<Item = AppResult<CompletionChunk>> + Send>>> {
        debug!(
            "Direct streaming completion request for model '{}'",
            request.model
        );

        // Parse the model from request (format: "provider/model")
        let (provider, model) = if let Some((p, m)) = request.model.split_once('/') {
            (p.to_string(), m.to_string())
        } else {
            return Err(AppError::Router(format!(
                "Model must be in format 'provider/model' for direct access. Got: '{}'",
                request.model
            )));
        };

        // Get provider instance from registry
        let provider_instance =
            self.provider_registry
                .get_provider(&provider)
                .ok_or_else(|| {
                    AppError::Router(format!(
                        "Provider '{}' not found or disabled in registry",
                        provider
                    ))
                })?;

        // Execute streaming request directly
        debug!(
            "Executing direct streaming completion on provider '{}' with model '{}'",
            provider, model
        );

        let mut modified_request = request;
        modified_request.model = model.clone();

        let stream = provider_instance.stream_complete(modified_request).await?;

        info!(
            "Direct streaming completion request started: provider='{}', model='{}'",
            provider, model
        );

        Ok(stream)
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
            extensions: None,
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
}
