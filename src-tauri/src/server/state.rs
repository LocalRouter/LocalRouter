//! Server state management
//!
//! Shared state for the web server including router, API key manager,
//! rate limiter, and generation tracking.

use std::sync::Arc;
use std::time::Instant;

use chrono::{DateTime, Utc};
use dashmap::DashMap;
use parking_lot::RwLock;

use crate::api_keys::ApiKeyManager;
use crate::providers::registry::ProviderRegistry;
use crate::router::{RateLimiterManager, Router};

use super::types::{GenerationDetailsResponse, TokenUsage, CostDetails, ProviderHealthSnapshot};

/// Server state shared across all handlers
#[derive(Clone)]
pub struct AppState {
    /// Router for intelligent model selection and routing
    pub router: Arc<Router>,

    /// API key manager for authentication
    pub api_key_manager: Arc<RwLock<ApiKeyManager>>,

    /// Rate limiter manager
    pub rate_limiter: Arc<RateLimiterManager>,

    /// Provider registry for listing models
    pub provider_registry: Arc<ProviderRegistry>,

    /// Generation tracking for /v1/generation endpoint
    pub generation_tracker: Arc<GenerationTracker>,
}

impl AppState {
    pub fn new(
        router: Arc<Router>,
        api_key_manager: ApiKeyManager,
        rate_limiter: Arc<RateLimiterManager>,
        provider_registry: Arc<ProviderRegistry>,
    ) -> Self {
        Self {
            router,
            api_key_manager: Arc::new(RwLock::new(api_key_manager)),
            rate_limiter,
            provider_registry,
            generation_tracker: Arc::new(GenerationTracker::new()),
        }
    }
}

/// Tracks generation details for the /v1/generation endpoint
pub struct GenerationTracker {
    /// Map of generation ID to generation details
    generations: DashMap<String, GenerationDetails>,

    /// Retention period in seconds (default: 7 days)
    retention_period_secs: i64,
}

impl GenerationTracker {
    pub fn new() -> Self {
        Self {
            generations: DashMap::new(),
            retention_period_secs: 7 * 24 * 60 * 60, // 7 days
        }
    }

    /// Record a new generation
    pub fn record(&self, id: String, details: GenerationDetails) {
        self.generations.insert(id, details);

        // Clean up old generations (simple approach)
        self.cleanup();
    }

    /// Get generation details by ID
    pub fn get(&self, id: &str) -> Option<GenerationDetailsResponse> {
        self.generations.get(id).map(|entry| entry.to_response())
    }

    /// Remove expired generations
    fn cleanup(&self) {
        let now = Utc::now();
        let cutoff = now.timestamp() - self.retention_period_secs;

        self.generations.retain(|_, details| {
            details.created_at.timestamp() > cutoff
        });
    }
}

impl Default for GenerationTracker {
    fn default() -> Self {
        Self::new()
    }
}

/// Internal generation details
pub struct GenerationDetails {
    pub id: String,
    pub model: String,
    pub provider: String,
    pub created_at: DateTime<Utc>,
    pub finish_reason: String,
    pub tokens: TokenUsage,
    pub cost: Option<CostDetails>,
    pub started_at: Instant,
    pub completed_at: Instant,
    pub provider_health: Option<ProviderHealthSnapshot>,
    pub api_key_id: String,
    pub user: Option<String>,
    pub stream: bool,
}

impl GenerationDetails {
    pub fn to_response(&self) -> GenerationDetailsResponse {
        let latency_ms = self.completed_at.duration_since(self.started_at).as_millis() as u64;

        GenerationDetailsResponse {
            id: self.id.clone(),
            model: self.model.clone(),
            provider: self.provider.clone(),
            created: self.created_at.timestamp(),
            finish_reason: self.finish_reason.clone(),
            tokens: self.tokens.clone(),
            cost: self.cost.clone(),
            latency_ms,
            provider_health: self.provider_health.clone(),
            api_key_id: mask_api_key(&self.api_key_id),
            user: self.user.clone(),
            stream: self.stream,
        }
    }
}

/// Mask API key for display (show first 3 and last 3 chars)
fn mask_api_key(key: &str) -> String {
    if key.len() <= 6 {
        return "*".repeat(key.len());
    }

    let prefix = &key[..3];
    let suffix = &key[key.len() - 3..];
    format!("{}***{}", prefix, suffix)
}

/// Authenticated request context
/// This is attached to requests after authentication middleware
#[derive(Clone)]
pub struct AuthContext {
    pub api_key_id: String,
    pub model_selection: ModelSelection,
}

/// Model selection mode for an API key
#[derive(Debug, Clone)]
pub enum ModelSelection {
    /// Direct model selection (provider and model specified)
    DirectModel {
        provider: String,
        model: String,
    },
    /// Router-based selection (router name specified)
    Router {
        router_name: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mask_api_key() {
        assert_eq!(mask_api_key("sk-1234567890"), "sk-***890");
        assert_eq!(mask_api_key("lr-abc123def456"), "lr-***456");
        assert_eq!(mask_api_key("short"), "*****");
    }

    #[test]
    fn test_generation_tracker() {
        let tracker = GenerationTracker::new();

        let details = GenerationDetails {
            id: "gen-123".to_string(),
            model: "gpt-4".to_string(),
            provider: "openai".to_string(),
            created_at: Utc::now(),
            finish_reason: "stop".to_string(),
            tokens: TokenUsage {
                prompt_tokens: 10,
                completion_tokens: 20,
                total_tokens: 30,
            },
            cost: Some(CostDetails {
                prompt_cost: 0.0001,
                completion_cost: 0.0002,
                total_cost: 0.0003,
                currency: "USD".to_string(),
            }),
            started_at: Instant::now(),
            completed_at: Instant::now(),
            provider_health: None,
            api_key_id: "lr-test123".to_string(),
            user: None,
            stream: false,
        };

        tracker.record("gen-123".to_string(), details);

        let result = tracker.get("gen-123");
        assert!(result.is_some());

        let response = result.unwrap();
        assert_eq!(response.id, "gen-123");
        assert_eq!(response.model, "gpt-4");
        assert_eq!(response.api_key_id, "lr-***123");
    }
}
