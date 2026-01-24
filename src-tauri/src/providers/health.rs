//! On-demand health check system for model providers
//!
//! Health checks are performed when explicitly requested by the UI,
//! not periodically in the background.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::RwLock;
use tokio::time::Instant;
use tracing::debug;

use super::{HealthStatus, ModelProvider, ProviderHealth};

/// Configuration for health checks
#[derive(Debug, Clone)]
pub struct HealthCheckConfig {
    /// Timeout for each health check
    pub check_timeout: Duration,
    /// Latency threshold for degraded status (milliseconds)
    pub degraded_latency_threshold_ms: u64,
}

impl Default for HealthCheckConfig {
    fn default() -> Self {
        Self {
            check_timeout: Duration::from_secs(5),
            degraded_latency_threshold_ms: 2000, // 2 seconds
        }
    }
}

/// Health check manager for all providers
pub struct HealthCheckManager {
    /// Configuration
    config: HealthCheckConfig,
    /// Providers registered for health checking
    providers: Arc<RwLock<Vec<Arc<dyn ModelProvider>>>>,
}

impl HealthCheckManager {
    /// Create a new health check manager
    pub fn new(config: HealthCheckConfig) -> Self {
        Self {
            config,
            providers: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// Register a provider for health checking
    pub async fn register_provider(&self, provider: Arc<dyn ModelProvider>) {
        self.providers.write().await.push(provider);
    }

    /// Perform on-demand health checks for all providers
    ///
    /// This is called when the UI requests health status.
    /// Each provider is checked in parallel with a timeout.
    pub async fn check_all_health(&self) -> HashMap<String, ProviderHealth> {
        let providers = self.providers.read().await.clone();
        let mut results = HashMap::new();

        // Check all providers in parallel
        let futures: Vec<_> = providers
            .iter()
            .map(|provider| {
                let provider = provider.clone();
                let config = self.config.clone();
                async move {
                    let name = provider.name().to_string();
                    let health = check_provider_health(provider, &config).await;
                    (name, health)
                }
            })
            .collect();

        let health_results = futures::future::join_all(futures).await;

        for (name, health) in health_results {
            results.insert(name, health);
        }

        results
    }

    /// Check health for a single provider by name
    #[allow(dead_code)]
    pub async fn check_health(&self, provider_name: &str) -> Option<ProviderHealth> {
        let providers = self.providers.read().await;
        for provider in providers.iter() {
            if provider.name() == provider_name {
                return Some(check_provider_health(provider.clone(), &self.config).await);
            }
        }
        None
    }
}

impl Default for HealthCheckManager {
    fn default() -> Self {
        Self::new(HealthCheckConfig::default())
    }
}

/// Perform a health check on a specific provider
async fn check_provider_health(
    provider: Arc<dyn ModelProvider>,
    config: &HealthCheckConfig,
) -> ProviderHealth {
    let provider_name = provider.name();
    debug!("Performing health check for provider: {}", provider_name);

    let start = Instant::now();

    // Perform the health check with timeout
    let health_result =
        tokio::time::timeout(config.check_timeout, provider.health_check()).await;

    let latency_ms = start.elapsed().as_millis() as u64;

    match health_result {
        Ok(mut health) => {
            // Update latency if not set by provider
            if health.latency_ms.is_none() {
                health.latency_ms = Some(latency_ms);
            }

            // Check if latency indicates degraded performance
            if let Some(latency) = health.latency_ms {
                if latency > config.degraded_latency_threshold_ms
                    && health.status == HealthStatus::Healthy
                {
                    health.status = HealthStatus::Degraded;
                    health.error_message = Some(format!(
                        "High latency: {}ms (threshold: {}ms)",
                        latency, config.degraded_latency_threshold_ms
                    ));
                }
            }

            debug!(
                "Health check completed for {}: {:?} ({}ms)",
                provider_name, health.status, latency_ms
            );
            health
        }
        Err(_) => {
            debug!(
                "Health check timeout for provider: {} (timeout: {}s)",
                provider_name,
                config.check_timeout.as_secs()
            );
            ProviderHealth {
                status: HealthStatus::Unhealthy,
                latency_ms: None,
                last_checked: chrono::Utc::now(),
                error_message: Some(format!(
                    "Health check timeout after {}s",
                    config.check_timeout.as_secs()
                )),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use chrono::Utc;
    use futures::Stream;
    use std::pin::Pin;

    use crate::providers::{
        CompletionChunk, CompletionRequest, CompletionResponse, ModelInfo, PricingInfo,
    };
    use crate::utils::errors::AppResult;

    /// Mock provider for testing
    struct MockProvider {
        name: String,
        health_status: HealthStatus,
        latency_ms: u64,
        should_timeout: bool,
    }

    #[async_trait]
    impl ModelProvider for MockProvider {
        fn name(&self) -> &str {
            &self.name
        }

        async fn health_check(&self) -> ProviderHealth {
            if self.should_timeout {
                // Simulate timeout by sleeping longer than timeout
                tokio::time::sleep(Duration::from_secs(10)).await;
            } else {
                // Simulate latency
                tokio::time::sleep(Duration::from_millis(self.latency_ms)).await;
            }

            ProviderHealth {
                status: self.health_status.clone(),
                latency_ms: Some(self.latency_ms),
                last_checked: Utc::now(),
                error_message: None,
            }
        }

        async fn list_models(&self) -> AppResult<Vec<ModelInfo>> {
            Ok(vec![])
        }

        async fn get_pricing(&self, _model: &str) -> AppResult<PricingInfo> {
            Ok(PricingInfo {
                input_cost_per_1k: 0.0,
                output_cost_per_1k: 0.0,
                currency: "USD".to_string(),
            })
        }

        async fn complete(&self, _request: CompletionRequest) -> AppResult<CompletionResponse> {
            unimplemented!()
        }

        async fn stream_complete(
            &self,
            _request: CompletionRequest,
        ) -> AppResult<Pin<Box<dyn Stream<Item = AppResult<CompletionChunk>> + Send>>> {
            unimplemented!()
        }
    }

    #[tokio::test]
    async fn test_health_check_healthy_provider() {
        let config = HealthCheckConfig {
            check_timeout: Duration::from_secs(5),
            degraded_latency_threshold_ms: 2000,
        };

        let manager = HealthCheckManager::new(config);

        let provider = Arc::new(MockProvider {
            name: "test-provider".to_string(),
            health_status: HealthStatus::Healthy,
            latency_ms: 100,
            should_timeout: false,
        });

        manager.register_provider(provider).await;

        let all_health = manager.check_all_health().await;
        assert_eq!(all_health.len(), 1);
        let health = all_health.get("test-provider").unwrap();
        assert_eq!(health.status, HealthStatus::Healthy);
        assert!(health.latency_ms.is_some());
    }

    #[tokio::test]
    async fn test_health_check_high_latency_degraded() {
        let config = HealthCheckConfig {
            check_timeout: Duration::from_secs(5),
            degraded_latency_threshold_ms: 500, // Low threshold for testing
        };

        let manager = HealthCheckManager::new(config);

        let provider = Arc::new(MockProvider {
            name: "slow-provider".to_string(),
            health_status: HealthStatus::Healthy,
            latency_ms: 1000, // Exceeds threshold
            should_timeout: false,
        });

        manager.register_provider(provider).await;

        let all_health = manager.check_all_health().await;
        let health = all_health.get("slow-provider").unwrap();
        assert_eq!(health.status, HealthStatus::Degraded);
        assert!(health.error_message.is_some());
    }

    #[tokio::test]
    async fn test_health_check_timeout() {
        let config = HealthCheckConfig {
            check_timeout: Duration::from_millis(500), // Short timeout for testing
            degraded_latency_threshold_ms: 2000,
        };

        let manager = HealthCheckManager::new(config);

        let provider = Arc::new(MockProvider {
            name: "timeout-provider".to_string(),
            health_status: HealthStatus::Healthy,
            latency_ms: 100,
            should_timeout: true,
        });

        manager.register_provider(provider).await;

        let all_health = manager.check_all_health().await;
        let health = all_health.get("timeout-provider").unwrap();
        assert_eq!(health.status, HealthStatus::Unhealthy);
        assert!(health.error_message.is_some());
        assert!(health.error_message.as_ref().unwrap().contains("timeout"));
    }
}
