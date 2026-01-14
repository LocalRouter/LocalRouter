//! Health check system for model providers
//!
//! This module implements a background health checking system with:
//! - Periodic health checks (every 30 seconds)
//! - Latency measurement
//! - Circuit breaker pattern to prevent repeated failures
//! - Health status caching

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use tokio::sync::RwLock;
use tokio::time::{interval, Instant};
use tracing::{debug, error, info, warn};

use super::{HealthStatus, ModelProvider, ProviderHealth};

/// Configuration for health checks
#[derive(Debug, Clone)]
pub struct HealthCheckConfig {
    /// How often to run health checks
    pub check_interval: Duration,
    /// Timeout for each health check
    pub check_timeout: Duration,
    /// Latency threshold for degraded status (milliseconds)
    pub degraded_latency_threshold_ms: u64,
    /// Circuit breaker failure threshold
    pub failure_threshold: u32,
    /// Circuit breaker recovery timeout
    pub recovery_timeout: Duration,
}

impl Default for HealthCheckConfig {
    fn default() -> Self {
        Self {
            check_interval: Duration::from_secs(30),
            check_timeout: Duration::from_secs(5),
            degraded_latency_threshold_ms: 2000, // 2 seconds
            failure_threshold: 3,                // 3 consecutive failures
            recovery_timeout: Duration::from_secs(60), // 1 minute
        }
    }
}

/// Circuit breaker states
#[derive(Debug, Clone, PartialEq, Eq)]
enum CircuitBreakerState {
    /// Circuit is closed, requests pass through normally
    Closed,
    /// Circuit is open, requests are blocked
    Open {
        /// When the circuit was opened
        opened_at: DateTime<Utc>,
    },
    /// Circuit is half-open, testing if service recovered
    HalfOpen,
}

/// Circuit breaker for a provider
#[derive(Debug, Clone)]
struct CircuitBreaker {
    /// Current state
    state: CircuitBreakerState,
    /// Number of consecutive failures
    consecutive_failures: u32,
    /// Configuration
    config: HealthCheckConfig,
}

impl CircuitBreaker {
    fn new(config: HealthCheckConfig) -> Self {
        Self {
            state: CircuitBreakerState::Closed,
            consecutive_failures: 0,
            config,
        }
    }

    /// Record a successful health check
    fn record_success(&mut self) {
        self.consecutive_failures = 0;
        self.state = CircuitBreakerState::Closed;
    }

    /// Record a failed health check
    fn record_failure(&mut self) {
        self.consecutive_failures += 1;

        if self.consecutive_failures >= self.config.failure_threshold {
            self.state = CircuitBreakerState::Open {
                opened_at: Utc::now(),
            };
        }
    }

    /// Check if the circuit should allow a health check
    fn should_allow_check(&mut self) -> bool {
        match &self.state {
            CircuitBreakerState::Closed => true,
            CircuitBreakerState::HalfOpen => true,
            CircuitBreakerState::Open { opened_at } => {
                let elapsed = Utc::now().signed_duration_since(*opened_at);
                if elapsed.to_std().unwrap_or(Duration::ZERO) >= self.config.recovery_timeout {
                    // Transition to half-open to test recovery
                    self.state = CircuitBreakerState::HalfOpen;
                    true
                } else {
                    false
                }
            }
        }
    }

    /// Get the current state
    fn state(&self) -> &CircuitBreakerState {
        &self.state
    }
}

/// Cached health status for a provider
#[derive(Debug, Clone)]
struct CachedHealth {
    /// The cached health status
    health: ProviderHealth,
    /// Circuit breaker for this provider
    circuit_breaker: CircuitBreaker,
}

/// Health check manager for all providers
pub struct HealthCheckManager {
    /// Configuration
    config: HealthCheckConfig,
    /// Cached health status for each provider
    cache: Arc<RwLock<HashMap<String, CachedHealth>>>,
    /// Providers being monitored
    providers: Arc<RwLock<Vec<Arc<dyn ModelProvider>>>>,
}

impl HealthCheckManager {
    /// Create a new health check manager
    pub fn new(config: HealthCheckConfig) -> Self {
        Self {
            config,
            cache: Arc::new(RwLock::new(HashMap::new())),
            providers: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// Create a new health check manager with default configuration
    pub fn default() -> Self {
        Self::new(HealthCheckConfig::default())
    }

    /// Register a provider for health checking
    pub async fn register_provider(&self, provider: Arc<dyn ModelProvider>) {
        let provider_name = provider.name().to_string();
        info!("Registering provider for health checks: {}", provider_name);

        // Add to providers list
        self.providers.write().await.push(provider);

        // Initialize cache entry
        let mut cache = self.cache.write().await;
        cache.insert(
            provider_name.clone(),
            CachedHealth {
                health: ProviderHealth {
                    status: HealthStatus::Healthy,
                    latency_ms: None,
                    last_checked: Utc::now(),
                    error_message: None,
                },
                circuit_breaker: CircuitBreaker::new(self.config.clone()),
            },
        );
    }

    /// Get the cached health status for a provider
    pub async fn get_health(&self, provider_name: &str) -> Option<ProviderHealth> {
        let cache = self.cache.read().await;
        cache.get(provider_name).map(|cached| cached.health.clone())
    }

    /// Get health status for all providers
    pub async fn get_all_health(&self) -> HashMap<String, ProviderHealth> {
        let cache = self.cache.read().await;
        cache
            .iter()
            .map(|(name, cached)| (name.clone(), cached.health.clone()))
            .collect()
    }

    /// Perform a health check on a specific provider
    async fn check_provider_health(&self, provider: Arc<dyn ModelProvider>) -> ProviderHealth {
        let provider_name = provider.name();
        debug!("Performing health check for provider: {}", provider_name);

        let start = Instant::now();

        // Perform the health check with timeout
        let health_result =
            tokio::time::timeout(self.config.check_timeout, provider.health_check()).await;

        let latency_ms = start.elapsed().as_millis() as u64;

        match health_result {
            Ok(mut health) => {
                // Update latency if not set by provider
                if health.latency_ms.is_none() {
                    health.latency_ms = Some(latency_ms);
                }

                // Check if latency indicates degraded performance
                if let Some(latency) = health.latency_ms {
                    if latency > self.config.degraded_latency_threshold_ms
                        && health.status == HealthStatus::Healthy
                    {
                        health.status = HealthStatus::Degraded;
                        health.error_message = Some(format!(
                            "High latency: {}ms (threshold: {}ms)",
                            latency, self.config.degraded_latency_threshold_ms
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
                warn!(
                    "Health check timeout for provider: {} (timeout: {}s)",
                    provider_name,
                    self.config.check_timeout.as_secs()
                );
                ProviderHealth {
                    status: HealthStatus::Unhealthy,
                    latency_ms: None,
                    last_checked: Utc::now(),
                    error_message: Some(format!(
                        "Health check timeout after {}s",
                        self.config.check_timeout.as_secs()
                    )),
                }
            }
        }
    }

    /// Update the cached health status for a provider
    async fn update_cache(&self, provider_name: String, health: ProviderHealth) {
        let mut cache = self.cache.write().await;

        if let Some(cached) = cache.get_mut(&provider_name) {
            // Update circuit breaker based on health status
            match health.status {
                HealthStatus::Healthy => {
                    cached.circuit_breaker.record_success();
                    debug!("Provider {} is healthy", provider_name);
                }
                HealthStatus::Degraded => {
                    // Degraded counts as success (provider is still operational)
                    cached.circuit_breaker.record_success();
                    warn!(
                        "Provider {} is degraded: {}",
                        provider_name,
                        health.error_message.as_deref().unwrap_or("unknown")
                    );
                }
                HealthStatus::Unhealthy => {
                    cached.circuit_breaker.record_failure();
                    error!(
                        "Provider {} is unhealthy: {}",
                        provider_name,
                        health.error_message.as_deref().unwrap_or("unknown")
                    );
                }
            }

            // Log circuit breaker state changes
            if let CircuitBreakerState::Open { opened_at } = cached.circuit_breaker.state() {
                error!(
                    "Circuit breaker OPEN for provider {} (opened at {})",
                    provider_name, opened_at
                );
            }

            cached.health = health;
        }
    }

    /// Run the background health check loop
    pub async fn run_background_checks(self: Arc<Self>) {
        info!(
            "Starting background health checks (interval: {}s)",
            self.config.check_interval.as_secs()
        );

        let mut interval = interval(self.config.check_interval);

        loop {
            interval.tick().await;

            let providers = self.providers.read().await.clone();
            debug!("Running health checks for {} providers", providers.len());

            // Check each provider
            for provider in providers {
                let provider_name = provider.name().to_string();

                // Check circuit breaker
                let should_check = {
                    let mut cache = self.cache.write().await;
                    if let Some(cached) = cache.get_mut(&provider_name) {
                        cached.circuit_breaker.should_allow_check()
                    } else {
                        true
                    }
                };

                if should_check {
                    // Perform health check
                    let health = self.check_provider_health(provider.clone()).await;
                    self.update_cache(provider_name.clone(), health).await;
                } else {
                    debug!(
                        "Skipping health check for {} (circuit breaker open)",
                        provider_name
                    );
                }
            }
        }
    }

    /// Start the background health check task
    pub fn start_background_task(self: Arc<Self>) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            self.run_background_checks().await;
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
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
            check_interval: Duration::from_secs(30),
            check_timeout: Duration::from_secs(5),
            degraded_latency_threshold_ms: 2000,
            failure_threshold: 3,
            recovery_timeout: Duration::from_secs(60),
        };

        let manager = Arc::new(HealthCheckManager::new(config));

        let provider = Arc::new(MockProvider {
            name: "test-provider".to_string(),
            health_status: HealthStatus::Healthy,
            latency_ms: 100,
            should_timeout: false,
        });

        manager.register_provider(provider.clone()).await;

        let health = manager.check_provider_health(provider).await;
        assert_eq!(health.status, HealthStatus::Healthy);
        assert!(health.latency_ms.is_some());
    }

    #[tokio::test]
    async fn test_health_check_high_latency_degraded() {
        let config = HealthCheckConfig {
            check_interval: Duration::from_secs(30),
            check_timeout: Duration::from_secs(5),
            degraded_latency_threshold_ms: 500, // Low threshold for testing
            failure_threshold: 3,
            recovery_timeout: Duration::from_secs(60),
        };

        let manager = Arc::new(HealthCheckManager::new(config));

        let provider = Arc::new(MockProvider {
            name: "slow-provider".to_string(),
            health_status: HealthStatus::Healthy,
            latency_ms: 1000, // Exceeds threshold
            should_timeout: false,
        });

        manager.register_provider(provider.clone()).await;

        let health = manager.check_provider_health(provider).await;
        assert_eq!(health.status, HealthStatus::Degraded);
        assert!(health.error_message.is_some());
    }

    #[tokio::test]
    async fn test_health_check_timeout() {
        let config = HealthCheckConfig {
            check_interval: Duration::from_secs(30),
            check_timeout: Duration::from_millis(500), // Short timeout for testing
            degraded_latency_threshold_ms: 2000,
            failure_threshold: 3,
            recovery_timeout: Duration::from_secs(60),
        };

        let manager = Arc::new(HealthCheckManager::new(config));

        let provider = Arc::new(MockProvider {
            name: "timeout-provider".to_string(),
            health_status: HealthStatus::Healthy,
            latency_ms: 100,
            should_timeout: true,
        });

        manager.register_provider(provider.clone()).await;

        let health = manager.check_provider_health(provider).await;
        assert_eq!(health.status, HealthStatus::Unhealthy);
        assert!(health.error_message.is_some());
        assert!(health.error_message.unwrap().contains("timeout"));
    }

    #[tokio::test]
    async fn test_circuit_breaker_opens_after_failures() {
        let mut breaker = CircuitBreaker::new(HealthCheckConfig {
            check_interval: Duration::from_secs(30),
            check_timeout: Duration::from_secs(5),
            degraded_latency_threshold_ms: 2000,
            failure_threshold: 3,
            recovery_timeout: Duration::from_secs(60),
        });

        assert_eq!(breaker.state(), &CircuitBreakerState::Closed);

        // Record failures
        breaker.record_failure();
        assert_eq!(breaker.state(), &CircuitBreakerState::Closed);

        breaker.record_failure();
        assert_eq!(breaker.state(), &CircuitBreakerState::Closed);

        breaker.record_failure();
        // Should open after 3 failures
        assert!(matches!(breaker.state(), CircuitBreakerState::Open { .. }));
    }

    #[tokio::test]
    async fn test_circuit_breaker_recovery() {
        let mut breaker = CircuitBreaker::new(HealthCheckConfig {
            check_interval: Duration::from_secs(30),
            check_timeout: Duration::from_secs(5),
            degraded_latency_threshold_ms: 2000,
            failure_threshold: 2,
            recovery_timeout: Duration::from_millis(100), // Short timeout for testing
        });

        // Open the circuit
        breaker.record_failure();
        breaker.record_failure();
        assert!(matches!(breaker.state(), CircuitBreakerState::Open { .. }));

        // Should not allow checks immediately
        assert!(!breaker.should_allow_check());

        // Wait for recovery timeout
        tokio::time::sleep(Duration::from_millis(150)).await;

        // Should transition to half-open
        assert!(breaker.should_allow_check());
        assert_eq!(breaker.state(), &CircuitBreakerState::HalfOpen);

        // Record success to close circuit
        breaker.record_success();
        assert_eq!(breaker.state(), &CircuitBreakerState::Closed);
    }

    #[tokio::test]
    async fn test_cache_operations() {
        let manager = Arc::new(HealthCheckManager::default());

        let provider = Arc::new(MockProvider {
            name: "cached-provider".to_string(),
            health_status: HealthStatus::Healthy,
            latency_ms: 100,
            should_timeout: false,
        });

        manager.register_provider(provider.clone()).await;

        // Check cached health
        let health = manager.get_health("cached-provider").await;
        assert!(health.is_some());

        // Get all health statuses
        let all_health = manager.get_all_health().await;
        assert_eq!(all_health.len(), 1);
        assert!(all_health.contains_key("cached-provider"));
    }
}
