//! Server state management
//!
//! Shared state for the web server including router, API key manager,
//! rate limiter, and generation tracking.

#![allow(dead_code)]

use std::sync::Arc;
use std::time::Instant;

use chrono::{DateTime, Utc};
use dashmap::DashMap;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::clients::{ClientManager, TokenStore};
use crate::config::ConfigManager;
use crate::mcp::protocol::JsonRpcNotification;
use crate::mcp::{McpGateway, McpServerManager};
use crate::monitoring::logger::AccessLogger;
use crate::monitoring::mcp_logger::McpAccessLogger;
use crate::monitoring::metrics::MetricsCollector;
use crate::providers::registry::ProviderRegistry;
use crate::router::{RateLimiterManager, Router};
use crate::ui::tray::TrayGraphManager;

use super::types::{CostDetails, GenerationDetailsResponse, ProviderHealthSnapshot, TokenUsage};

/// Server state shared across all handlers
#[derive(Clone)]
pub struct AppState {
    /// Router for intelligent model selection and routing
    pub router: Arc<Router>,

    /// Unified client manager for authentication (replaces api_key_manager and oauth_client_manager)
    pub client_manager: Arc<ClientManager>,

    /// OAuth token store for short-lived access tokens
    pub token_store: Arc<TokenStore>,

    /// MCP server manager
    pub mcp_server_manager: Arc<McpServerManager>,

    /// MCP unified gateway
    pub mcp_gateway: Arc<McpGateway>,

    /// Rate limiter manager
    pub rate_limiter: Arc<RateLimiterManager>,

    /// Provider registry for listing models
    pub provider_registry: Arc<ProviderRegistry>,

    /// Configuration manager for accessing client routing configs
    pub config_manager: Arc<ConfigManager>,

    /// Generation tracking for /v1/generation endpoint
    pub generation_tracker: Arc<GenerationTracker>,

    /// Metrics collector for tracking usage
    pub metrics_collector: Arc<MetricsCollector>,

    /// Access logger for persistent request logging
    pub access_logger: Arc<AccessLogger>,

    /// MCP access logger for persistent MCP request logging
    pub mcp_access_logger: Arc<McpAccessLogger>,

    /// Tauri app handle for emitting events (set after initialization)
    pub app_handle: Arc<RwLock<Option<tauri::AppHandle>>>,

    /// Transient secret for internal UI testing (never persisted, regenerated on startup)
    /// Used to allow the Tauri frontend to bypass API key restrictions when testing models
    pub internal_test_secret: Arc<String>,

    /// RouteLLM intelligent routing service
    pub routellm_service: Option<Arc<crate::routellm::RouteLLMService>>,

    /// Tray graph manager for real-time token visualization (optional, only in UI mode)
    /// Behind RwLock to allow setting it after AppState creation during Tauri setup
    pub tray_graph_manager: Arc<RwLock<Option<Arc<TrayGraphManager>>>>,

    /// Broadcast channel for MCP server notifications
    /// Allows multiple clients to subscribe to real-time notifications from MCP servers
    /// Format: (server_id, notification)
    pub mcp_notification_broadcast:
        Arc<tokio::sync::broadcast::Sender<(String, JsonRpcNotification)>>,

    /// Streaming session manager for SSE multiplexing
    pub streaming_session_manager: Arc<crate::mcp::gateway::streaming::StreamingSessionManager>,
}

impl AppState {
    pub fn new(
        router: Arc<Router>,
        rate_limiter: Arc<RateLimiterManager>,
        provider_registry: Arc<ProviderRegistry>,
        config_manager: Arc<ConfigManager>,
        client_manager: Arc<ClientManager>,
        token_store: Arc<TokenStore>,
        metrics_collector: Arc<MetricsCollector>,
    ) -> Self {
        // Generate a random bearer token for internal UI testing
        // Format: lr-internal-<uuid> to match standard API key format
        // This is regenerated on every app start and never persisted
        let internal_test_secret = format!("lr-internal-{}", Uuid::new_v4().simple());
        tracing::info!("Generated transient internal test bearer token for UI model testing");

        // Initialize access logger with 30-day retention
        let access_logger = AccessLogger::new(30).unwrap_or_else(|e| {
            tracing::error!("Failed to initialize access logger: {}", e);
            panic!("Access logger initialization failed");
        });

        // Initialize MCP access logger with 30-day retention
        let mcp_access_logger = McpAccessLogger::new(30).unwrap_or_else(|e| {
            tracing::error!("Failed to initialize MCP access logger: {}", e);
            panic!("MCP access logger initialization failed");
        });

        // Create broadcast channel for MCP notifications
        // Capacity of 1000 messages - old messages dropped if no subscribers are reading fast enough
        let (notification_tx, _rx) = tokio::sync::broadcast::channel(1000);

        // Create placeholder MCP manager and gateway (will be replaced by with_mcp)
        let mcp_server_manager = Arc::new(McpServerManager::new());
        let mcp_gateway = Arc::new(McpGateway::new(
            mcp_server_manager.clone(),
            crate::mcp::gateway::GatewayConfig::default(),
            router.clone(),
        ));

        // Create placeholder streaming session manager (will be replaced by with_mcp)
        let streaming_config = config_manager.get().streaming.clone();
        let streaming_session_manager = Arc::new(
            crate::mcp::gateway::streaming::StreamingSessionManager::new(
                mcp_gateway.clone(),
                mcp_server_manager.clone(),
                streaming_config,
            ),
        );

        Self {
            router,
            client_manager,
            token_store,
            mcp_server_manager,
            mcp_gateway,
            rate_limiter,
            provider_registry,
            config_manager,
            generation_tracker: Arc::new(GenerationTracker::new()),
            metrics_collector,
            access_logger: Arc::new(access_logger),
            mcp_access_logger: Arc::new(mcp_access_logger),
            app_handle: Arc::new(RwLock::new(None)),
            internal_test_secret: Arc::new(internal_test_secret),
            routellm_service: None,
            tray_graph_manager: Arc::new(RwLock::new(None)),
            mcp_notification_broadcast: Arc::new(notification_tx),
            streaming_session_manager,
        }
    }

    /// Add MCP manager to the state
    pub fn with_mcp(self, mcp_server_manager: Arc<McpServerManager>) -> Self {
        // Create gateway with the actual MCP server manager
        let mcp_gateway = Arc::new(McpGateway::new(
            mcp_server_manager.clone(),
            crate::mcp::gateway::GatewayConfig::default(),
            self.router.clone(),
        ));

        // Create streaming session manager with the actual components
        let streaming_config = self.config_manager.get().streaming.clone();
        let streaming_session_manager = Arc::new(
            crate::mcp::gateway::streaming::StreamingSessionManager::new(
                mcp_gateway.clone(),
                mcp_server_manager.clone(),
                streaming_config,
            ),
        );

        Self {
            mcp_server_manager,
            mcp_gateway,
            streaming_session_manager,
            ..self
        }
    }

    /// Get the internal test secret for UI testing
    /// Only accessible via Tauri IPC, not exposed over HTTP
    pub fn get_internal_test_secret(&self) -> String {
        (*self.internal_test_secret).clone()
    }

    /// Set the Tauri app handle (called after Tauri initialization)
    pub fn set_app_handle(&self, handle: tauri::AppHandle) {
        *self.app_handle.write() = Some(handle.clone());

        // Also set app handle on loggers for event emission
        self.access_logger.set_app_handle(handle.clone());
        self.mcp_access_logger.set_app_handle(handle);
    }

    /// Set the tray graph manager (called after Tauri initialization when it's created)
    pub fn set_tray_graph_manager(&self, manager: Arc<TrayGraphManager>) {
        *self.tray_graph_manager.write() = Some(manager);
    }

    /// Initialize RouteLLM service with settings from config
    pub fn with_routellm(
        mut self,
        routellm_service: Option<Arc<crate::routellm::RouteLLMService>>,
    ) -> Self {
        self.routellm_service = routellm_service;
        self
    }

    /// Emit an event if the app handle is available
    pub fn emit_event(&self, event: &str, payload: &str) {
        if let Some(handle) = self.app_handle.read().as_ref() {
            use tauri::Emitter;
            let _ = handle.emit(event, payload);
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

/// Aggregate statistics across all tracked generations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AggregateStats {
    pub total_requests: u64,
    pub total_tokens: u64,
    pub total_cost: f64,
    pub successful_requests: u64,
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

    /// Get aggregate statistics
    pub fn get_stats(&self) -> AggregateStats {
        let mut total_requests = 0u64;
        let mut total_tokens = 0u64;
        let mut total_cost = 0.0f64;

        for entry in self.generations.iter() {
            let details = entry.value();
            total_requests += 1;
            total_tokens += details.tokens.total_tokens as u64;
            if let Some(cost) = &details.cost {
                total_cost += cost.total_cost;
            }
        }

        AggregateStats {
            total_requests,
            successful_requests: total_requests, // GenerationTracker only tracks successful requests
            total_tokens,
            total_cost,
        }
    }

    /// Remove expired generations
    fn cleanup(&self) {
        let now = Utc::now();
        let cutoff = now.timestamp() - self.retention_period_secs;

        self.generations
            .retain(|_, details| details.created_at.timestamp() > cutoff);
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
        let latency_ms = self
            .completed_at
            .duration_since(self.started_at)
            .as_millis() as u64;

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
    pub model_selection: Option<ModelSelection>, // Legacy, kept for backward compatibility
    pub routing_config: Option<crate::config::ModelRoutingConfig>,
}

/// OAuth authenticated request context for MCP proxy
/// This is attached to requests after OAuth authentication middleware
#[derive(Clone)]
pub struct OAuthContext {
    /// OAuth client ID
    pub client_id: String,
    /// MCP servers this client can access
    pub linked_server_ids: Vec<String>,
}

/// Model selection mode for an API key
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ModelSelection {
    /// All models from all providers (including future models)
    All,

    /// Custom selection of providers and/or individual models
    Custom {
        /// If true, all models are allowed (including future models)
        #[serde(default)]
        selected_all: bool,
        /// Providers where ALL models are selected (including future models)
        selected_providers: Vec<String>,
        /// Individual models selected as (provider, model) pairs
        selected_models: Vec<(String, String)>,
    },

    /// Legacy: Direct model selection (deprecated, use Custom instead)
    #[deprecated(note = "Use ModelSelection::Custom instead")]
    DirectModel { provider: String, model: String },

    /// Legacy: Router-based selection (deprecated)
    #[deprecated(note = "Router-based selection is deprecated")]
    Router {
        #[allow(dead_code)]
        router_name: String,
    },
}

impl ModelSelection {
    /// Check if a model is allowed by this selection
    pub fn is_model_allowed(&self, provider_name: &str, model_id: &str) -> bool {
        match self {
            ModelSelection::All => true,
            ModelSelection::Custom {
                selected_all,
                selected_providers,
                selected_models,
            } => {
                // If all are selected, everything is allowed
                if *selected_all {
                    return true;
                }

                // Check if the provider is in the selected_providers list
                if selected_providers
                    .iter()
                    .any(|p| p.eq_ignore_ascii_case(provider_name))
                {
                    return true;
                }

                // Check if the specific (provider, model) pair is in selected_models
                selected_models.iter().any(|(p, m)| {
                    p.eq_ignore_ascii_case(provider_name) && m.eq_ignore_ascii_case(model_id)
                })
            }
            #[allow(deprecated)]
            ModelSelection::DirectModel { provider, model } => {
                provider.eq_ignore_ascii_case(provider_name) && model.eq_ignore_ascii_case(model_id)
            }
            #[allow(deprecated)]
            ModelSelection::Router { .. } => {
                // Router-based selection is deprecated
                // For now, allow all models (will be handled by router logic)
                true
            }
        }
    }
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
                prompt_tokens_details: None,
                completion_tokens_details: None,
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
