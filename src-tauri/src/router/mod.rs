//! Smart routing system
//!
//! Routes incoming requests to appropriate model providers based on API key configuration.

use std::sync::Arc;

use futures::Stream;
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

                    // Record usage for rate limiting
                    let usage = UsageInfo {
                        input_tokens: response.usage.prompt_tokens as u64,
                        output_tokens: response.usage.completion_tokens as u64,
                        cost_usd: 0.0, // TODO: Calculate actual cost from pricing
                    };

                    self.rate_limiter
                        .record_api_key_usage(api_key_id, &usage)
                        .await?;

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
                        if !config.is_model_allowed(&p, &m) {
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
                    // Use the first model in the prioritized list (retry logic will handle failures)
                    if let Some((first_provider, first_model)) = config.prioritized_models.first() {
                        debug!(
                            "Using first prioritized model: provider='{}', model='{}' (requested was '{}')",
                            first_provider, first_model, request.model
                        );
                        (first_provider.clone(), first_model.clone())
                    } else {
                        return Err(AppError::Router(
                            "Prioritized List strategy is active but no models are configured".to_string()
                        ));
                    }
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
        let mut modified_request = request;
        modified_request.model = expected_model.clone();

        let response = provider_instance.complete(modified_request).await.map_err(|e| {
            warn!(
                "Completion request failed for provider '{}': {}",
                provider, e
            );
            e
        })?;

        // 7. Record usage for rate limiting
        let usage = UsageInfo {
            input_tokens: response.usage.prompt_tokens as u64,
            output_tokens: response.usage.completion_tokens as u64,
            cost_usd: 0.0, // TODO: Calculate actual cost from pricing
        };

        self.rate_limiter
            .record_api_key_usage(api_key_id, &usage)
            .await?;

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

        let (provider, expected_model) = if let Some(ref config) = routing_config {
            // New routing config system (same logic as complete())
            match config.active_strategy {
                ActiveRoutingStrategy::AvailableModels => {
                    // Allow any model in the available list
                    if let Some((p, m)) = request.model.split_once('/') {
                        if !config.is_model_allowed(&p, &m) {
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
                    // Use first model in prioritized list
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

        // TODO: Record usage after stream completes
        // This is challenging because we need to count tokens as they stream
        // For now, usage recording is skipped for streaming requests

        Ok(stream)
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
        };

        let result = router.complete("test-key", request).await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), AppError::Unauthorized));
    }
}
