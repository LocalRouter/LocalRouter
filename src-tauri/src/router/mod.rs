//! Smart routing system
//!
//! Routes incoming requests to appropriate model providers based on API key configuration.

use std::sync::Arc;

use futures::Stream;
use std::pin::Pin;
use tracing::{debug, info, warn};

use crate::config::{ConfigManager, ModelSelection};
use crate::providers::registry::ProviderRegistry;
use crate::providers::{CompletionChunk, CompletionRequest, CompletionResponse};
use crate::utils::errors::{AppError, AppResult};

pub mod rate_limit;

// Re-export commonly used types
pub use rate_limit::{
    RateLimitCheckResult, RateLimitType, RateLimiter, RateLimiterKey, RateLimiterManager, UsageInfo,
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
        let (provider, expected_model) = match &api_key.model_selection {
            ModelSelection::DirectModel { provider, model } => {
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
            ModelSelection::Router { router_name } => {
                // Smart routing not implemented yet
                return Err(AppError::Router(format!(
                    "Smart routing (router '{}') is not yet implemented. Please use DirectModel configuration.",
                    router_name
                )));
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

        let response = provider_instance.complete(request).await.map_err(|e| {
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

        // 3. Determine provider and model
        let (provider, expected_model) = match &api_key.model_selection {
            ModelSelection::DirectModel { provider, model } => {
                if request.model != *model {
                    return Err(AppError::Router(format!(
                        "Model mismatch: API key is configured for model '{}', but request specifies '{}'",
                        model, request.model
                    )));
                }
                (provider.clone(), model.clone())
            }
            ModelSelection::Router { router_name } => {
                return Err(AppError::Router(format!(
                    "Smart routing (router '{}') is not yet implemented",
                    router_name
                )));
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

        let stream = provider_instance.stream_complete(request).await?;

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
            key_hash: "hash".to_string(),
            model_selection: ModelSelection::DirectModel {
                provider: "test-provider".to_string(),
                model: "test-model".to_string(),
            },
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
