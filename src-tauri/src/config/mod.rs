//! Configuration management module
//!
//! Handles loading, saving, and managing application configuration.
//! Supports file watching and event emission for real-time config updates.

#![allow(dead_code)]

use crate::utils::errors::{AppError, AppResult};
use chrono::{DateTime, Utc};
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use tauri::{AppHandle, Emitter};
use tracing::{debug, error, info};
use uuid::Uuid;

mod migration;
pub mod paths;
mod storage;
mod validation;

pub use storage::{load_config, save_config};
// RateLimitType is now defined locally in this module (see line 610)

const CONFIG_VERSION: u32 = 2;

/// Time window for rate limits
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RateLimitTimeWindow {
    Minute,
    Hour,
    Day,
}

impl RateLimitTimeWindow {
    pub fn to_seconds(&self) -> i64 {
        match self {
            RateLimitTimeWindow::Minute => 60,
            RateLimitTimeWindow::Hour => 3600,
            RateLimitTimeWindow::Day => 86400,
        }
    }
}

/// Rate limit configuration for strategies
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StrategyRateLimit {
    pub limit_type: RateLimitType,
    pub value: f64,
    pub time_window: RateLimitTimeWindow,
}

/// RouteLLM download state
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RouteLLMDownloadState {
    NotDownloaded,
    Downloading,
    Downloaded,
    Failed,
}

/// RouteLLM download status
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RouteLLMDownloadStatus {
    pub state: RouteLLMDownloadState,
    pub progress: f32,
    pub current_file: Option<String>,
    pub total_bytes: u64,
    pub downloaded_bytes: u64,
    pub error: Option<String>,
}

/// MCP filesystem root configuration
///
/// Represents a directory boundary for MCP servers.
/// Note: Roots are advisory only, not enforced as a security boundary.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RootConfig {
    /// File URI (must use file:// scheme)
    pub uri: String,

    /// Optional display name for the root
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    /// Whether this root is enabled
    #[serde(default = "default_root_enabled")]
    pub enabled: bool,
}

fn default_root_enabled() -> bool {
    true
}

/// Global RouteLLM settings (stored in AppConfig)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RouteLLMGlobalSettings {
    /// Path to model directory (contains model.safetensors)
    /// Default: ~/.localrouter/routellm/model/
    /// Note: Field name kept as 'onnx_model_path' for backward compatibility
    #[serde(skip_serializing_if = "Option::is_none")]
    pub onnx_model_path: Option<PathBuf>,

    /// Path to tokenizer directory (contains tokenizer.json)
    /// Default: ~/.localrouter/routellm/tokenizer/
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tokenizer_path: Option<PathBuf>,

    /// Idle time before auto-unload (seconds)
    /// Default: 600 (10 minutes)
    #[serde(default = "default_idle_timeout")]
    pub idle_timeout_secs: u64,

    /// Download status (internal)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub download_status: Option<RouteLLMDownloadStatus>,
}

fn default_idle_timeout() -> u64 {
    600 // 10 minutes
}

impl Default for RouteLLMGlobalSettings {
    fn default() -> Self {
        Self {
            onnx_model_path: None,
            tokenizer_path: None,
            idle_timeout_secs: default_idle_timeout(),
            download_status: None,
        }
    }
}

/// RouteLLM intelligent routing configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RouteLLMConfig {
    /// Whether RouteLLM routing is enabled
    pub enabled: bool,

    /// Win rate threshold (0.0-1.0)
    /// If win_rate >= threshold, route to strong model
    /// Recommended: 0.3 (balanced), 0.7 (cost-optimized), 0.2 (quality-prioritized)
    pub threshold: f32,

    /// Strong model selection (used when win_rate >= threshold)
    pub strong_models: Vec<(String, String)>, // (provider, model)

    /// Weak model selection (used when win_rate < threshold)
    pub weak_models: Vec<(String, String)>,
}

impl Default for RouteLLMConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            threshold: 0.3, // Balanced profile
            strong_models: Vec::new(),
            weak_models: Vec::new(),
        }
    }
}

/// Auto model configuration for localrouter/auto virtual model
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AutoModelConfig {
    /// Whether auto-routing is enabled
    pub enabled: bool,
    /// Prioritized models list (in order) for fallback
    pub prioritized_models: Vec<(String, String)>,
    /// Available models (out of rotation)
    #[serde(default)]
    pub available_models: Vec<(String, String)>,
    /// RouteLLM intelligent routing configuration
    #[serde(skip_serializing_if = "Option::is_none")]
    pub routellm_config: Option<RouteLLMConfig>,
}

/// Routing strategy configuration (separate from clients)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Strategy {
    /// Unique identifier (UUID)
    pub id: String,
    /// User-defined name
    pub name: String,
    /// Client ID that owns this strategy (if any)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    /// Models that are allowed by this strategy
    #[serde(default)]
    pub allowed_models: AvailableModelsSelection,
    /// Auto-routing configuration for localrouter/auto
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_config: Option<AutoModelConfig>,
    /// Rate limits for this strategy
    #[serde(default)]
    pub rate_limits: Vec<StrategyRateLimit>,
}

impl Strategy {
    /// Create a new strategy with default settings
    pub fn new(name: String) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            name,
            parent: None,
            allowed_models: AvailableModelsSelection::default(),
            auto_config: None,
            rate_limits: vec![],
        }
    }

    /// Create a new strategy owned by a client
    pub fn new_for_client(client_id: String, client_name: String) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            name: format!("{}'s strategy", client_name),
            parent: Some(client_id),
            allowed_models: AvailableModelsSelection::all(),
            auto_config: None,
            rate_limits: vec![],
        }
    }

    /// Check if a model is allowed by this strategy
    pub fn is_model_allowed(&self, provider: &str, model: &str) -> bool {
        self.allowed_models.is_model_allowed(provider, model)
    }
}

impl AvailableModelsSelection {
    /// Create a selection that allows all models
    pub fn all() -> Self {
        Self {
            all_provider_models: vec![],
            individual_models: vec![],
        }
    }
}

/// Main application configuration structure
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AppConfig {
    /// Configuration schema version for migrations
    #[serde(default = "default_version")]
    pub version: u32,

    /// Server configuration
    #[serde(default)]
    pub server: ServerConfig,

    /// Router configurations
    #[serde(default)]
    pub routers: Vec<RouterConfig>,

    /// Provider configurations
    #[serde(default)]
    pub providers: Vec<ProviderConfig>,

    /// Logging configuration
    #[serde(default)]
    pub logging: LoggingConfig,

    /// OAuth clients for MCP (deprecated, use clients instead)
    #[serde(default)]
    pub oauth_clients: Vec<OAuthClientConfig>,

    /// MCP server configurations
    #[serde(default)]
    pub mcp_servers: Vec<McpServerConfig>,

    /// Unified clients (replaces api_keys and oauth_clients)
    #[serde(default)]
    pub clients: Vec<Client>,

    /// Routing strategies (separate from clients, reusable)
    #[serde(default)]
    pub strategies: Vec<Strategy>,

    /// Pricing overrides for specific provider/model combinations
    /// Format: {provider_name: {model_name: pricing_override}}
    #[serde(default)]
    pub pricing_overrides:
        std::collections::HashMap<String, std::collections::HashMap<String, ModelPricingOverride>>,

    /// UI configuration
    #[serde(default)]
    pub ui: UiConfig,

    /// Global RouteLLM settings
    #[serde(default)]
    pub routellm_settings: RouteLLMGlobalSettings,

    /// Update checking configuration
    #[serde(default)]
    pub update: UpdateConfig,

    /// Model cache configuration
    #[serde(default)]
    pub model_cache: ModelCacheConfig,

    /// Global MCP filesystem roots (advisory boundaries)
    #[serde(default)]
    pub roots: Vec<RootConfig>,
}

/// Pricing override for a specific model
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModelPricingOverride {
    /// Input/prompt price per million tokens
    pub input_per_million: f64,
    /// Output/completion price per million tokens
    pub output_per_million: f64,
}

/// UI configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UiConfig {
    /// Enable dynamic graph in system tray icon
    #[serde(default)]
    pub tray_graph_enabled: bool,

    /// Graph refresh rate
    /// - Fast (1): 1 second per bar, 30 second total (start fresh)
    /// - Medium (10): 10 seconds per bar, 5 minute total (interpolated from minute data)
    /// - Slow (60): 1 minute per bar, 30 minute total (direct mapping)
    #[serde(default = "default_tray_graph_refresh_rate")]
    pub tray_graph_refresh_rate_secs: u64,
}

/// Update checking mode
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum UpdateMode {
    /// User must manually click "Check Now" button
    Manual,
    /// Check for updates automatically on a schedule
    #[default]
    Automatic,
}

/// Update checking configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UpdateConfig {
    /// Update checking mode (manual or automatic)
    #[serde(default = "default_update_mode")]
    pub mode: UpdateMode,

    /// Interval between automatic update checks (in days)
    /// Default: 7 days
    #[serde(default = "default_check_interval")]
    pub check_interval_days: u64,

    /// Last time updates were checked
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_check: Option<DateTime<Utc>>,

    /// Version that user explicitly skipped (won't notify about this version)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skipped_version: Option<String>,
}

fn default_update_mode() -> UpdateMode {
    UpdateMode::Automatic
}

fn default_check_interval() -> u64 {
    7 // Check weekly
}

/// Model cache configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModelCacheConfig {
    /// Default TTL for model cache (seconds)
    #[serde(default = "default_model_cache_ttl")]
    pub default_ttl_seconds: u64,

    /// Per-provider TTL overrides
    #[serde(default)]
    pub provider_ttl_overrides: std::collections::HashMap<String, u64>,

    /// Whether to use OpenRouter catalog as fallback
    #[serde(default = "default_true")]
    pub use_catalog_fallback: bool,
}

fn default_model_cache_ttl() -> u64 {
    3600 // 1 hour
}

impl Default for ModelCacheConfig {
    fn default() -> Self {
        Self {
            default_ttl_seconds: 3600,
            provider_ttl_overrides: std::collections::HashMap::new(),
            use_catalog_fallback: true,
        }
    }
}

/// Server configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ServerConfig {
    /// Server host address
    pub host: String,

    /// Server port
    pub port: u16,

    /// Enable CORS for local development
    pub enable_cors: bool,
}

/// Model routing configuration for API keys
///
/// Supports three routing strategies:
/// 1. Available Models: Request model must be in the selected list
/// 2. Force Model: Always use a specific model, ignore request
/// 3. Prioritized List: Try models in order, retry on failure
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelRoutingConfig {
    /// The currently active routing strategy
    pub active_strategy: ActiveRoutingStrategy,

    /// Configuration for "Available Models" strategy
    /// Models are preserved even when switching to other strategies
    #[serde(default)]
    pub available_models: AvailableModelsSelection,

    /// Configuration for "Force Model" strategy
    /// The forced model is preserved even when switching to other strategies
    #[serde(skip_serializing_if = "Option::is_none")]
    pub forced_model: Option<(String, String)>,

    /// Configuration for "Prioritized List" strategy
    /// Models are in priority order; preserved even when switching to other strategies
    #[serde(default)]
    pub prioritized_models: Vec<(String, String)>,
}

/// Active routing strategy for an API key
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ActiveRoutingStrategy {
    /// Available Models: Request model must be in the selected list
    AvailableModels,
    /// Force Model: Always use a specific model, ignore request
    ForceModel,
    /// Prioritized List: Try models in order, retry on failure
    PrioritizedList,
}

/// Available models selection configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct AvailableModelsSelection {
    /// Providers where ALL models are selected (including future models)
    #[serde(default)]
    pub all_provider_models: Vec<String>,
    /// Individual models selected as (provider, model) pairs
    #[serde(default)]
    pub individual_models: Vec<(String, String)>,
}

/// OAuth client configuration for MCP
///
/// The actual client_secret is stored in the OS keychain.
/// This struct contains only metadata and the client_id (which is public).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OAuthClientConfig {
    /// Unique identifier (also used as keyring username)
    pub id: String,

    /// Human-readable name
    pub name: String,

    /// OAuth client_id (public identifier, lr-... format)
    pub client_id: String,

    /// MCP servers this client can access
    #[serde(default)]
    pub linked_server_ids: Vec<String>,

    /// Whether the client is enabled
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Creation timestamp
    pub created_at: DateTime<Utc>,

    /// Last used timestamp
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_used: Option<DateTime<Utc>>,
}

/// Unified client configuration (replaces ApiKeyConfig and OAuthClientConfig)
///
/// A client can access both LLM routing and MCP servers using a single secret.
/// Supports two authentication methods:
/// 1. Direct Bearer Token: Authorization: Bearer {client_secret}
/// 2. OAuth Client Credentials: Get temporary token via POST /oauth/token
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Client {
    /// Unique identifier (internal, UUID)
    pub id: String,

    /// Human-readable name
    pub name: String,

    /// Whether this client is enabled
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// LLM providers this client can access
    /// Empty = no LLM access
    #[serde(default)]
    pub allowed_llm_providers: Vec<String>,

    /// MCP servers this client can access (by server ID)
    /// Empty = no MCP access
    #[serde(default)]
    pub allowed_mcp_servers: Vec<String>,

    /// Enable deferred loading for MCP tools (default: false)
    /// When enabled, only a search tool is initially visible. Tools are activated on-demand
    /// through search queries, dramatically reducing token consumption for large catalogs.
    #[serde(default)]
    pub mcp_deferred_loading: bool,

    /// When this client was created
    pub created_at: DateTime<Utc>,

    /// Last time this client was used
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_used: Option<DateTime<Utc>>,

    /// Reference to the routing strategy this client uses
    #[serde(default = "default_strategy_id")]
    pub strategy_id: String,

    /// Model routing configuration (deprecated, use strategy_id instead)
    /// Kept for backward compatibility during migration
    #[serde(skip_serializing_if = "Option::is_none")]
    #[deprecated(note = "Use strategy_id instead")]
    pub routing_config: Option<ModelRoutingConfig>,

    /// MCP filesystem roots override (per-client)
    /// If None, uses global roots from AppConfig
    /// If Some, replaces global roots entirely for this client
    #[serde(skip_serializing_if = "Option::is_none")]
    pub roots: Option<Vec<RootConfig>>,

    /// Enable MCP sampling (backend servers can request LLM completions)
    /// Default: false (sampling disabled for security)
    #[serde(default)]
    pub mcp_sampling_enabled: bool,

    /// Require user approval for each sampling request
    /// Default: true (when sampling is enabled)
    #[serde(default = "default_true")]
    pub mcp_sampling_requires_approval: bool,

    /// Maximum tokens per sampling request
    /// None = unlimited (uses provider defaults)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mcp_sampling_max_tokens: Option<u32>,

    /// Maximum sampling requests per hour
    /// None = unlimited
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mcp_sampling_rate_limit: Option<u32>,
}

/// MCP server configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpServerConfig {
    /// Unique identifier
    pub id: String,

    /// Human-readable name
    pub name: String,

    /// Transport type (STDIO or HTTP/SSE only)
    pub transport: McpTransportType,

    /// Transport-specific configuration
    pub transport_config: McpTransportConfig,

    /// Manual authentication configuration
    /// How LocalRouter authenticates TO this MCP server (outbound)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_config: Option<McpAuthConfig>,

    /// Auto-discovered OAuth configuration (legacy, for auto-detection)
    /// Populated automatically if server has .well-known/oauth-protected-resource
    #[serde(skip_serializing_if = "Option::is_none")]
    pub discovered_oauth: Option<McpOAuthDiscovery>,

    /// Legacy OAuth configuration (deprecated, use discovered_oauth)
    /// This field is kept for backward compatibility during migration
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(default)]
    pub oauth_config: Option<McpOAuthConfig>,

    /// Whether the server is enabled
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Creation timestamp
    pub created_at: DateTime<Utc>,
}

/// MCP transport type
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum McpTransportType {
    /// STDIO transport (spawn subprocess with piped stdin/stdout)
    Stdio,

    /// HTTP with Server-Sent Events (new naming convention)
    #[serde(alias = "sse")]
    HttpSse,

    /// WebSocket transport (bidirectional real-time communication)
    WebSocket,

    /// Server-Sent Events (HTTP + SSE) - DEPRECATED, use HttpSse
    #[deprecated(note = "Use HttpSse instead")]
    #[serde(skip_deserializing)]
    Sse,
}

/// Transport-specific configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum McpTransportConfig {
    /// STDIO process configuration
    Stdio {
        /// Command to execute
        command: String,
        /// Command arguments
        #[serde(default)]
        args: Vec<String>,
        /// Base environment variables (auth env vars go in McpAuthConfig::EnvVars)
        #[serde(default)]
        env: std::collections::HashMap<String, String>,
    },

    /// HTTP with Server-Sent Events configuration (new naming)
    #[serde(alias = "sse")]
    HttpSse {
        /// Server URL
        url: String,
        /// Base headers (auth headers go in McpAuthConfig::CustomHeaders or BearerToken)
        #[serde(default)]
        headers: std::collections::HashMap<String, String>,
    },

    /// WebSocket configuration
    WebSocket {
        /// WebSocket server URL (ws:// or wss://)
        url: String,
        /// HTTP headers to send during WebSocket handshake
        #[serde(default)]
        headers: std::collections::HashMap<String, String>,
    },

    /// SSE configuration - DEPRECATED, use HttpSse
    #[serde(skip_deserializing)]
    Sse {
        /// Server URL
        url: String,
        /// HTTP headers
        #[serde(default)]
        headers: std::collections::HashMap<String, String>,
    },
}

/// OAuth configuration for MCP server (auto-discovered)
///
/// Discovered via /.well-known/oauth-protected-resource endpoint
/// This is the legacy auto-discovery format, kept for compatibility
/// Client credentials stored in keychain service "LocalRouter-McpServerTokens"
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpOAuthConfig {
    /// Authorization endpoint URL
    pub auth_url: String,

    /// Token endpoint URL
    pub token_url: String,

    /// OAuth scopes
    #[serde(default)]
    pub scopes: Vec<String>,

    /// OAuth client_id for this MCP server
    pub client_id: String,

    /// Reference to client_secret in keychain (using server ID as key)
    /// Actual secret stored in keychain, not here
    #[serde(skip)]
    pub client_secret_ref: String,
}

/// Authentication configuration for MCP servers (outbound authentication)
///
/// Configures how LocalRouter authenticates TO external MCP servers.
/// This is separate from how clients authenticate TO LocalRouter (see Client struct).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum McpAuthConfig {
    /// No authentication required
    None,

    /// Bearer token authentication (Authorization: Bearer {token})
    BearerToken {
        /// Reference to token in keychain
        /// Stored in keyring: service="LocalRouter-McpServers", account=server.id
        token_ref: String,
    },

    /// Custom headers (can include auth headers)
    CustomHeaders {
        /// Headers to send with every request
        /// Can include: Authorization, X-API-Key, etc.
        /// Sensitive values should be stored in keychain and referenced here
        headers: std::collections::HashMap<String, String>,
    },

    /// Pre-registered OAuth credentials (client credentials flow)
    OAuth {
        /// OAuth client ID
        client_id: String,

        /// Reference to client secret in keychain
        client_secret_ref: String,

        /// Authorization endpoint URL
        auth_url: String,

        /// Token endpoint URL
        token_url: String,

        /// OAuth scopes to request
        scopes: Vec<String>,
    },

    /// OAuth with browser-based authorization code flow (PKCE)
    /// For user-interactive authentication (GitHub, GitLab, etc.)
    OAuthBrowser {
        /// OAuth client ID (public)
        client_id: String,

        /// Reference to client secret in keychain
        /// Stored in keyring: service="LocalRouter-McpServers", account="{server_id}_client_secret"
        client_secret_ref: String,

        /// Authorization endpoint URL
        auth_url: String,

        /// Token endpoint URL
        token_url: String,

        /// OAuth scopes to request
        scopes: Vec<String>,

        /// Redirect URI (usually http://localhost:8080/callback)
        /// Must match OAuth app registration
        redirect_uri: String,
    },

    /// Environment variables (for STDIO only)
    /// Can include API keys, tokens, etc.
    EnvVars {
        /// Environment variables to pass to subprocess
        /// Merged with transport_config.env at runtime
        /// Sensitive values should be stored in keychain and referenced here
        env: std::collections::HashMap<String, String>,
    },
}

/// Auto-discovered OAuth information (from .well-known endpoint)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpOAuthDiscovery {
    /// Authorization endpoint URL
    pub auth_url: String,

    /// Token endpoint URL
    pub token_url: String,

    /// Supported OAuth scopes
    pub scopes_supported: Vec<String>,

    /// When this was discovered
    pub discovered_at: DateTime<Utc>,
}

/// Router configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RouterConfig {
    /// Router name
    pub name: String,

    /// Model selection strategy
    pub model_selection: ModelSelectionStrategy,

    /// Routing strategies to apply
    #[serde(default)]
    pub strategies: Vec<RoutingStrategy>,

    /// Enable fallback to next model on failure
    #[serde(default = "default_true")]
    pub fallback_enabled: bool,

    /// Rate limiters
    #[serde(default)]
    pub rate_limiters: Vec<RateLimiter>,
}

/// Model selection strategy
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ModelSelectionStrategy {
    /// Automatic model selection with filters
    Automatic {
        /// Provider filters
        providers: Vec<ProviderFilter>,
        /// Minimum parameter count
        #[serde(skip_serializing_if = "Option::is_none")]
        min_parameters: Option<u64>,
        /// Maximum parameter count
        #[serde(skip_serializing_if = "Option::is_none")]
        max_parameters: Option<u64>,
    },
    /// Manual model list in priority order
    Manual {
        /// List of (provider, model) in priority order
        models: Vec<(String, String)>,
    },
}

/// Provider filter for model selection
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderFilter {
    /// Provider name
    pub provider_name: String,

    /// Include only these models (None = all models)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include_models: Option<Vec<String>>,

    /// Exclude these models
    #[serde(default)]
    pub exclude_models: Vec<String>,
}

/// Routing strategy
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RoutingStrategy {
    /// Route to lowest cost model
    LowestCost,
    /// Route to highest performance model
    HighestPerformance,
    /// Prefer local models first
    LocalFirst,
    /// Prefer remote models first
    RemoteFirst,
    /// Prefer subscription-based models
    SubscriptionFirst,
    /// Prefer API-based models
    ApiFirst,
}

/// Rate limiter configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RateLimiter {
    /// Type of rate limit
    pub limit_type: RateLimitType,

    /// Limit value
    pub value: f64,

    /// Time window in seconds
    pub time_window_seconds: u64,
}

/// Rate limit type
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RateLimitType {
    /// Requests per time window
    Requests,
    /// Input tokens per time window
    InputTokens,
    /// Output tokens per time window
    OutputTokens,
    /// Total tokens per time window
    TotalTokens,
    /// Cost in USD per time window
    Cost,
}

/// Provider configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderConfig {
    /// Provider name
    pub name: String,

    /// Provider type
    pub provider_type: ProviderType,

    /// Whether the provider is enabled
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Provider-specific configuration (flexible JSON/YAML object)
    ///
    /// Each provider can define its own configuration structure. Common examples:
    ///
    /// **OpenAI:**
    /// ```yaml
    /// provider_config:
    ///   endpoint: "https://api.openai.com/v1"  # Custom endpoint
    ///   organization: "org-xyz"                 # Organization ID
    ///   timeout_seconds: 30                     # Request timeout
    /// ```
    ///
    /// **Anthropic:**
    /// ```yaml
    /// provider_config:
    ///   endpoint: "https://api.anthropic.com/v1"
    ///   version: "2023-06-01"                   # API version
    /// ```
    ///
    /// **Gemini:**
    /// ```yaml
    /// provider_config:
    ///   base_url: "https://generativelanguage.googleapis.com/v1beta"
    /// ```
    ///
    /// **OpenRouter:**
    /// ```yaml
    /// provider_config:
    ///   app_name: "My Application"
    ///   app_url: "https://myapp.com"
    ///   extra_headers:
    ///     X-Custom: "value"
    /// ```
    ///
    /// **Ollama:**
    /// ```yaml
    /// provider_config:
    ///   base_url: "http://localhost:11434"
    ///   timeout_seconds: 120
    /// ```
    ///
    /// If `None`, providers use their default configuration.
    /// Providers should implement `from_config()` to parse this field.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_config: Option<serde_json::Value>,

    /// API key reference name for system keyring lookup
    ///
    /// This is the name used to store/retrieve the actual API key from the system keyring:
    /// - macOS: Keychain
    /// - Windows: Credential Manager
    /// - Linux: Secret Service / keyutils
    ///
    /// If `None`, the provider's `name` field is used as the keyring lookup name.
    /// The actual API key is NEVER stored in this config - only in the secure system keyring.
    ///
    /// Use `providers::key_storage` module to manage provider API keys.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key_ref: Option<String>,
}

/// Provider type
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderType {
    /// Local Ollama instance
    Ollama,
    /// OpenAI API
    OpenAI,
    /// OpenRouter proxy
    OpenRouter,
    /// Anthropic API
    Anthropic,
    /// Google Gemini API
    Gemini,
    /// Groq API
    Groq,
    /// Mistral API
    Mistral,
    /// Cohere API
    Cohere,
    /// Together AI API
    TogetherAI,
    /// Perplexity API
    Perplexity,
    /// DeepInfra API
    DeepInfra,
    /// Cerebras API
    Cerebras,
    /// xAI API
    #[allow(clippy::upper_case_acronyms)]
    XAI,
    /// Custom provider
    Custom,
}

/// Logging configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LoggingConfig {
    /// Log level
    pub level: LogLevel,

    /// Enable access logging
    #[serde(default = "default_true")]
    pub enable_access_log: bool,

    /// Access log directory (None = use default OS location)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_dir: Option<PathBuf>,

    /// Maximum number of days to keep logs
    #[serde(default = "default_log_retention")]
    pub retention_days: u32,
}

/// Log level
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

/// Thread-safe configuration manager with file watching and event emission
#[derive(Clone)]
pub struct ConfigManager {
    config: Arc<RwLock<AppConfig>>,
    config_path: PathBuf,
    app_handle: Option<AppHandle>,
}

// Manual Debug implementation since AppHandle doesn't implement Debug
impl std::fmt::Debug for ConfigManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConfigManager")
            .field("config", &self.config)
            .field("config_path", &self.config_path)
            .field("app_handle", &self.app_handle.is_some())
            .finish()
    }
}

impl ConfigManager {
    /// Create a new configuration manager
    pub fn new(config: AppConfig, config_path: PathBuf) -> Self {
        Self {
            config: Arc::new(RwLock::new(config)),
            config_path,
            app_handle: None,
        }
    }

    /// Load configuration from default location
    pub async fn load() -> AppResult<Self> {
        let config_path = paths::config_file()?;
        let config = load_config(&config_path).await?;
        Ok(Self::new(config, config_path))
    }

    /// Load configuration with custom path
    pub async fn load_from_path(path: PathBuf) -> AppResult<Self> {
        let config = load_config(&path).await?;
        Ok(Self::new(config, path))
    }

    /// Set the Tauri app handle for event emission
    ///
    /// This enables the config manager to emit events to the frontend when the config changes.
    /// Call this during app setup, after the ConfigManager is created.
    pub fn set_app_handle(&mut self, app_handle: AppHandle) {
        self.app_handle = Some(app_handle);
    }

    /// Start watching the configuration file for changes
    ///
    /// When the config file changes externally (e.g., user edits it), this will:
    /// 1. Reload the configuration from disk
    /// 2. Emit a "config-changed" event to the frontend
    ///
    /// Returns a file watcher that must be kept alive. Drop it to stop watching.
    pub fn start_watching(&self) -> AppResult<RecommendedWatcher> {
        let config_path = self.config_path.clone();
        let config_arc = self.config.clone();
        let app_handle = self.app_handle.clone();

        // Capture the Tokio runtime handle for spawning tasks from the file watcher thread
        let runtime_handle = tokio::runtime::Handle::current();

        let mut watcher =
            notify::recommended_watcher(move |result: Result<Event, notify::Error>| {
                match result {
                    Ok(event) => {
                        // Only respond to modify events
                        if matches!(event.kind, EventKind::Modify(_)) {
                            info!("Configuration file changed, reloading...");

                            // Reload config from disk (blocking operation in event handler)
                            let config_path_clone = config_path.clone();
                            let config_arc_clone = config_arc.clone();
                            let app_handle_clone = app_handle.clone();

                            // Use the captured runtime handle to spawn the task
                            runtime_handle.spawn(async move {
                                match load_config(&config_path_clone).await {
                                    Ok(new_config) => {
                                        // Update in-memory config
                                        *config_arc_clone.write() = new_config.clone();

                                        info!("Configuration reloaded successfully");

                                        // Emit event to frontend
                                        if let Some(handle) = app_handle_clone {
                                            if let Err(e) =
                                                handle.emit("config-changed", &new_config)
                                            {
                                                error!(
                                                    "Failed to emit config-changed event: {}",
                                                    e
                                                );
                                            } else {
                                                debug!("Emitted config-changed event to frontend");
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        error!("Failed to reload configuration: {}", e);
                                    }
                                }
                            });
                        }
                    }
                    Err(e) => {
                        error!("File watch error: {}", e);
                    }
                }
            })
            .map_err(|e| AppError::Config(format!("Failed to create file watcher: {}", e)))?;

        // Watch the config file
        watcher
            .watch(&self.config_path, RecursiveMode::NonRecursive)
            .map_err(|e| AppError::Config(format!("Failed to watch config file: {}", e)))?;

        info!(
            "Started watching configuration file: {:?}",
            self.config_path
        );
        Ok(watcher)
    }

    /// Get a read-only copy of the configuration
    pub fn get(&self) -> AppConfig {
        self.config.read().clone()
    }

    /// Update configuration with a function
    ///
    /// Updates the in-memory configuration and validates it.
    /// To persist changes, call `save()` afterwards.
    /// Emits "config-changed" event to frontend if app handle is set.
    pub fn update<F>(&self, f: F) -> AppResult<()>
    where
        F: FnOnce(&mut AppConfig),
    {
        let updated_config = {
            let mut config = self.config.write();
            f(&mut config);
            validation::validate_config(&config)?;
            config.clone()
        };

        // Emit event to frontend
        self.emit_config_changed(&updated_config);

        Ok(())
    }

    /// Save configuration to disk
    ///
    /// Writes the current in-memory configuration to the config file.
    /// Does NOT emit event (file watcher will handle that).
    pub async fn save(&self) -> AppResult<()> {
        let config = self.config.read().clone();
        // TODO: DELETE THIS DEBUG LOG LATER
        tracing::warn!("💾 SAVE_TO_DISK: {} clients", config.clients.len());
        save_config(&config, &self.config_path).await
    }

    /// Manually reload configuration from disk
    ///
    /// Useful for forcing a reload without waiting for file watcher.
    /// Emits "config-changed" event to frontend.
    pub async fn reload(&self) -> AppResult<()> {
        let new_config = load_config(&self.config_path).await?;
        *self.config.write() = new_config.clone();

        info!("Configuration reloaded manually");

        // Emit event to frontend
        self.emit_config_changed(&new_config);

        Ok(())
    }

    /// Emit config-changed event to frontend
    fn emit_config_changed(&self, config: &AppConfig) {
        if let Some(ref handle) = self.app_handle {
            if let Err(e) = handle.emit("config-changed", config) {
                error!("Failed to emit config-changed event: {}", e);
            } else {
                debug!("Emitted config-changed event to frontend");
            }
        }
    }

    /// Get the configuration file path
    pub fn config_path(&self) -> &PathBuf {
        &self.config_path
    }

    /// Get global filesystem roots
    ///
    /// Returns a clone of the configured global roots for MCP servers.
    /// Use with per-client roots to determine final roots for a session.
    pub fn get_roots(&self) -> Vec<RootConfig> {
        let config = self.config.read();
        config.roots.clone()
    }

    /// Ensure default strategy exists and assign clients without strategy
    pub async fn ensure_default_strategy(&self) -> AppResult<()> {
        let mut modified = false;

        self.update(|cfg| {
            // Ensure default strategy exists
            if !cfg.strategies.iter().any(|s| s.id == "default") {
                cfg.strategies.push(Strategy {
                    id: "default".to_string(),
                    name: "Default Strategy".to_string(),
                    parent: None,
                    allowed_models: AvailableModelsSelection::all(),
                    auto_config: None,
                    rate_limits: vec![],
                });
                info!("Created default strategy");
                modified = true;
            }

            // Assign clients without strategy to default
            for client in &mut cfg.clients {
                if client.strategy_id.is_empty() {
                    client.strategy_id = "default".to_string();
                    info!("Assigned client '{}' to default strategy", client.name);
                    modified = true;
                }
            }
        })?;

        // Save to disk if we made changes
        if modified {
            self.save().await?;
        }

        Ok(())
    }

    /// Create a client with an auto-created strategy
    pub fn create_client_with_strategy(&self, name: String) -> AppResult<(Client, Strategy)> {
        let client_id = Uuid::new_v4().to_string();
        let strategy = Strategy::new_for_client(client_id.clone(), name.clone());

        let client = Client {
            id: client_id,
            name,
            enabled: true,
            strategy_id: strategy.id.clone(),
            allowed_llm_providers: Vec::new(),
            allowed_mcp_servers: Vec::new(),
            mcp_deferred_loading: false,
            created_at: Utc::now(),
            last_used: None,
            #[allow(deprecated)]
            routing_config: None,
            roots: None,
            mcp_sampling_enabled: false,
            mcp_sampling_requires_approval: true,
            mcp_sampling_max_tokens: None,
            mcp_sampling_rate_limit: None,
        };

        self.update(|cfg| {
            cfg.clients.push(client.clone());
            cfg.strategies.push(strategy.clone());
        })?;

        Ok((client, strategy))
    }

    /// Delete a client and cascade delete its owned strategies
    pub fn delete_client(&self, client_id: &str) -> AppResult<()> {
        self.update(|cfg| {
            // Remove client
            cfg.clients.retain(|c| c.id != client_id);

            // Cascade delete owned strategies
            cfg.strategies
                .retain(|s| s.parent.as_ref() != Some(&client_id.to_string()));
        })?;

        Ok(())
    }

    /// Assign a client to a different strategy (clears parent if selecting non-owned strategy)
    pub fn assign_client_strategy(&self, client_id: &str, new_strategy_id: &str) -> AppResult<()> {
        // First check if client exists
        {
            let cfg = self.config.read();
            if !cfg.clients.iter().any(|c| c.id == client_id) {
                return Err(AppError::Config("Client not found".into()));
            }
        }

        self.update(|cfg| {
            if let Some(client) = cfg.clients.iter_mut().find(|c| c.id == client_id) {
                let old_strategy_id = client.strategy_id.clone();

                // If selecting a different strategy (not own), clear parent from that strategy
                if old_strategy_id != new_strategy_id {
                    if let Some(new_strategy) =
                        cfg.strategies.iter_mut().find(|s| s.id == new_strategy_id)
                    {
                        // Clear parent if it's not the current client
                        if new_strategy.parent.as_ref() != Some(&client_id.to_string()) {
                            new_strategy.parent = None;
                        }
                    }
                }

                client.strategy_id = new_strategy_id.to_string();
            }
        })
    }

    /// Rename a strategy (clears parent if changing from default name)
    pub fn rename_strategy(&self, strategy_id: &str, new_name: &str) -> AppResult<()> {
        // First check if strategy exists
        {
            let cfg = self.config.read();
            if !cfg.strategies.iter().any(|s| s.id == strategy_id) {
                return Err(AppError::Config("Strategy not found".into()));
            }
        }

        self.update(|cfg| {
            if let Some(strategy) = cfg.strategies.iter_mut().find(|s| s.id == strategy_id) {
                // Check if renaming from default pattern
                if let Some(parent_id) = &strategy.parent {
                    if let Some(client) = cfg.clients.iter().find(|c| c.id == *parent_id) {
                        let default_name = format!("{}'s strategy", client.name);
                        if strategy.name == default_name && new_name != default_name {
                            // Clear parent when customizing name
                            strategy.parent = None;
                        }
                    }
                }

                strategy.name = new_name.to_string();
            }
        })
    }
}

// Default value functions for serde
fn default_version() -> u32 {
    CONFIG_VERSION
}

fn default_true() -> bool {
    true
}

fn default_log_retention() -> u32 {
    30
}

fn default_strategy_id() -> String {
    "default".to_string()
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            version: CONFIG_VERSION,
            server: ServerConfig::default(),
            routers: vec![
                RouterConfig::default_minimum_cost(),
                RouterConfig::default_maximum_performance(),
            ],
            providers: vec![ProviderConfig::default_ollama()],
            logging: LoggingConfig::default(),
            oauth_clients: Vec::new(),
            mcp_servers: Vec::new(),
            clients: Vec::new(),
            strategies: vec![Strategy {
                id: "default".to_string(),
                name: "Default Strategy".to_string(),
                parent: None,
                allowed_models: AvailableModelsSelection::all(),
                auto_config: None,
                rate_limits: vec![],
            }],
            pricing_overrides: std::collections::HashMap::new(),
            ui: UiConfig::default(),
            routellm_settings: RouteLLMGlobalSettings::default(),
            update: UpdateConfig::default(),
            model_cache: ModelCacheConfig::default(),
            roots: Vec::new(),
        }
    }
}

impl Default for ServerConfig {
    fn default() -> Self {
        // Use different port for development to avoid conflicts
        #[cfg(debug_assertions)]
        let default_port = 33625;

        #[cfg(not(debug_assertions))]
        let default_port = 3625;

        Self {
            host: "127.0.0.1".to_string(),
            port: default_port,
            enable_cors: true,
        }
    }
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: LogLevel::Info,
            enable_access_log: true,
            log_dir: None,
            retention_days: 30,
        }
    }
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            tray_graph_enabled: false, // Disabled by default (opt-in)
            tray_graph_refresh_rate_secs: default_tray_graph_refresh_rate(),
        }
    }
}

impl Default for UpdateConfig {
    fn default() -> Self {
        Self {
            mode: default_update_mode(),
            check_interval_days: default_check_interval(),
            last_check: None,
            skipped_version: None,
        }
    }
}

impl RouterConfig {
    /// Create default "Minimum Cost" router
    pub fn default_minimum_cost() -> Self {
        Self {
            name: "Minimum Cost".to_string(),
            model_selection: ModelSelectionStrategy::Automatic {
                providers: vec![],
                min_parameters: None,
                max_parameters: None,
            },
            strategies: vec![RoutingStrategy::LocalFirst, RoutingStrategy::LowestCost],
            fallback_enabled: true,
            rate_limiters: Vec::new(),
        }
    }

    /// Create default "Maximum Performance" router
    pub fn default_maximum_performance() -> Self {
        Self {
            name: "Maximum Performance".to_string(),
            model_selection: ModelSelectionStrategy::Automatic {
                providers: vec![],
                min_parameters: None,
                max_parameters: None,
            },
            strategies: vec![RoutingStrategy::HighestPerformance],
            fallback_enabled: true,
            rate_limiters: Vec::new(),
        }
    }
}

impl ProviderConfig {
    /// Create default Ollama provider configuration
    pub fn default_ollama() -> Self {
        Self {
            name: "Ollama".to_string(),
            provider_type: ProviderType::Ollama,
            enabled: true,
            provider_config: Some(serde_json::json!({
                "base_url": "http://localhost:11434"
            })),
            api_key_ref: None,
        }
    }
}

impl ModelRoutingConfig {
    /// Create a new routing config with "Available Models" as default strategy
    pub fn new_available_models() -> Self {
        Self {
            active_strategy: ActiveRoutingStrategy::AvailableModels,
            available_models: AvailableModelsSelection::default(),
            forced_model: None,
            prioritized_models: Vec::new(),
        }
    }

    /// Create a new routing config with "Force Model" strategy
    pub fn new_force_model(provider: String, model: String) -> Self {
        Self {
            active_strategy: ActiveRoutingStrategy::ForceModel,
            available_models: AvailableModelsSelection::default(),
            forced_model: Some((provider, model)),
            prioritized_models: Vec::new(),
        }
    }

    /// Create a new routing config with "Prioritized List" strategy
    pub fn new_prioritized_list(models: Vec<(String, String)>) -> Self {
        Self {
            active_strategy: ActiveRoutingStrategy::PrioritizedList,
            available_models: AvailableModelsSelection::default(),
            forced_model: None,
            prioritized_models: models,
        }
    }

    /// Check if a model is allowed by the current active strategy
    pub fn is_model_allowed(&self, provider_name: &str, model_id: &str) -> bool {
        match self.active_strategy {
            ActiveRoutingStrategy::AvailableModels => self
                .available_models
                .is_model_allowed(provider_name, model_id),
            ActiveRoutingStrategy::ForceModel => {
                // Only the forced model is allowed
                if let Some((forced_provider, forced_model)) = &self.forced_model {
                    forced_provider.eq_ignore_ascii_case(provider_name)
                        && forced_model.eq_ignore_ascii_case(model_id)
                } else {
                    false
                }
            }
            ActiveRoutingStrategy::PrioritizedList => {
                // Any model in the prioritized list is "allowed" for listing purposes
                self.prioritized_models.iter().any(|(p, m)| {
                    p.eq_ignore_ascii_case(provider_name) && m.eq_ignore_ascii_case(model_id)
                })
            }
        }
    }

    /// Get the model to use for a request (ignoring the requested model for Force and Prioritized strategies)
    pub fn get_model_for_request(&self, _requested_model: &str) -> Option<(String, String)> {
        match self.active_strategy {
            ActiveRoutingStrategy::AvailableModels => {
                // Use the requested model (caller should validate it's allowed)
                None // Signal to use requested model
            }
            ActiveRoutingStrategy::ForceModel => {
                // Always use the forced model
                self.forced_model.clone()
            }
            ActiveRoutingStrategy::PrioritizedList => {
                // Use the first model in the prioritized list
                self.prioritized_models.first().cloned()
            }
        }
    }

    /// Migration helper: Create ModelRoutingConfig from deprecated ModelSelection
    #[allow(deprecated)]
    pub fn from_model_selection(selection: crate::server::state::ModelSelection) -> Self {
        match selection {
            crate::server::state::ModelSelection::All => Self::new_available_models(),
            crate::server::state::ModelSelection::Custom {
                all_provider_models,
                individual_models,
            } => Self {
                active_strategy: ActiveRoutingStrategy::AvailableModels,
                available_models: AvailableModelsSelection {
                    all_provider_models,
                    individual_models,
                },
                forced_model: None,
                prioritized_models: Vec::new(),
            },
            crate::server::state::ModelSelection::DirectModel { provider, model } => {
                Self::new_force_model(provider, model)
            }
            crate::server::state::ModelSelection::Router { .. } => {
                // Router-based - default to Available Models
                Self::new_available_models()
            }
        }
    }
}

impl AvailableModelsSelection {
    /// Check if a model is allowed by this selection
    pub fn is_model_allowed(&self, provider_name: &str, model_id: &str) -> bool {
        // Check if the provider is in the all_provider_models list
        if self
            .all_provider_models
            .iter()
            .any(|p| p.eq_ignore_ascii_case(provider_name))
        {
            return true;
        }

        // Check if the specific (provider, model) pair is in individual_models
        self.individual_models
            .iter()
            .any(|(p, m)| p.eq_ignore_ascii_case(provider_name) && m.eq_ignore_ascii_case(model_id))
    }
}

impl OAuthClientConfig {
    /// Create a new OAuth client configuration
    pub fn new(name: String, client_id: String) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            name,
            client_id,
            linked_server_ids: Vec::new(),
            enabled: true,
            created_at: Utc::now(),
            last_used: None,
        }
    }
}

impl Client {
    /// Create a new client with auto-generated client_id
    /// The secret must be stored separately in the keychain
    pub fn new(name: String) -> Self {
        let id = Uuid::new_v4().to_string();
        Self {
            id: id.clone(),
            name,
            enabled: true,
            allowed_llm_providers: Vec::new(),
            allowed_mcp_servers: Vec::new(),
            mcp_deferred_loading: false,
            created_at: Utc::now(),
            last_used: None,
            strategy_id: "default".to_string(),
            #[allow(deprecated)]
            routing_config: None,
            roots: None,
            mcp_sampling_enabled: false,
            mcp_sampling_requires_approval: true,
            mcp_sampling_max_tokens: None,
            mcp_sampling_rate_limit: None,
        }
    }

    /// Update last used timestamp
    pub fn mark_used(&mut self) {
        self.last_used = Some(Utc::now());
    }

    /// Check if this client can access a specific LLM provider
    pub fn can_access_llm_provider(&self, provider_name: &str) -> bool {
        self.enabled
            && self
                .allowed_llm_providers
                .contains(&provider_name.to_string())
    }

    /// Check if this client can access a specific MCP server
    pub fn can_access_mcp_server(&self, server_id: &str) -> bool {
        self.enabled && self.allowed_mcp_servers.contains(&server_id.to_string())
    }

    /// Add LLM provider access
    pub fn add_llm_provider(&mut self, provider_name: String) {
        if !self.allowed_llm_providers.contains(&provider_name) {
            self.allowed_llm_providers.push(provider_name);
        }
    }

    /// Remove LLM provider access
    pub fn remove_llm_provider(&mut self, provider_name: &str) -> bool {
        if let Some(pos) = self
            .allowed_llm_providers
            .iter()
            .position(|p| p == provider_name)
        {
            self.allowed_llm_providers.remove(pos);
            true
        } else {
            false
        }
    }

    /// Add MCP server access
    pub fn add_mcp_server(&mut self, server_id: String) {
        if !self.allowed_mcp_servers.contains(&server_id) {
            self.allowed_mcp_servers.push(server_id);
        }
    }

    /// Remove MCP server access
    pub fn remove_mcp_server(&mut self, server_id: &str) -> bool {
        if let Some(pos) = self.allowed_mcp_servers.iter().position(|s| s == server_id) {
            self.allowed_mcp_servers.remove(pos);
            true
        } else {
            false
        }
    }
}

impl McpServerConfig {
    /// Create a new MCP server configuration
    pub fn new(
        name: String,
        transport: McpTransportType,
        transport_config: McpTransportConfig,
    ) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            name,
            transport,
            transport_config,
            auth_config: None,
            discovered_oauth: None,
            oauth_config: None,
            enabled: true,
            created_at: Utc::now(),
        }
    }
}

fn default_tray_graph_refresh_rate() -> u64 {
    60 // Slow: 1 minute per bar (default)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = AppConfig::default();
        assert_eq!(config.version, CONFIG_VERSION);
        assert_eq!(config.server.host, "127.0.0.1");
        #[cfg(debug_assertions)]
        assert_eq!(config.server.port, 33625);
        #[cfg(not(debug_assertions))]
        assert_eq!(config.server.port, 3625);
        assert_eq!(config.routers.len(), 2);
        assert_eq!(config.providers.len(), 1);
    }

    #[test]
    fn test_server_config_default() {
        let server = ServerConfig::default();
        assert_eq!(server.host, "127.0.0.1");
        #[cfg(debug_assertions)]
        assert_eq!(server.port, 33625);
        #[cfg(not(debug_assertions))]
        assert_eq!(server.port, 3625);
        assert!(server.enable_cors);
    }

    #[test]
    fn test_logging_config_default() {
        let logging = LoggingConfig::default();
        assert_eq!(logging.level, LogLevel::Info);
        assert!(logging.enable_access_log);
        assert_eq!(logging.retention_days, 30);
    }

    #[test]
    fn test_router_defaults() {
        let min_cost = RouterConfig::default_minimum_cost();
        assert_eq!(min_cost.name, "Minimum Cost");
        assert!(min_cost.fallback_enabled);
        assert!(min_cost.strategies.contains(&RoutingStrategy::LowestCost));

        let max_perf = RouterConfig::default_maximum_performance();
        assert_eq!(max_perf.name, "Maximum Performance");
        assert!(max_perf
            .strategies
            .contains(&RoutingStrategy::HighestPerformance));
    }

    #[test]
    fn test_config_serialization() {
        let config = AppConfig::default();
        let yaml = serde_yaml::to_string(&config).unwrap();
        let deserialized: AppConfig = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(config, deserialized);
    }

    #[test]
    fn test_root_config_serialization() {
        let root = RootConfig {
            uri: "file:///Users/test/projects".to_string(),
            name: Some("Projects".to_string()),
            enabled: true,
        };

        let yaml = serde_yaml::to_string(&root).unwrap();
        let deserialized: RootConfig = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(root, deserialized);
    }

    #[test]
    fn test_app_config_with_roots() {
        let mut config = AppConfig::default();
        config.roots = vec![
            RootConfig {
                uri: "file:///Users/test/projects".to_string(),
                name: Some("Projects".to_string()),
                enabled: true,
            },
            RootConfig {
                uri: "file:///var/data".to_string(),
                name: None,
                enabled: true,
            },
        ];

        let yaml = serde_yaml::to_string(&config).unwrap();
        let deserialized: AppConfig = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(config.roots.len(), 2);
        assert_eq!(deserialized.roots.len(), 2);
        assert_eq!(deserialized.roots[0].uri, "file:///Users/test/projects");
        assert_eq!(deserialized.roots[1].name, None);
    }

    #[test]
    fn test_client_with_roots_override() {
        let mut client = Client::new("Test Client".to_string());
        client.roots = Some(vec![RootConfig {
            uri: "file:///custom/path".to_string(),
            name: Some("Custom".to_string()),
            enabled: true,
        }]);

        // Verify serialization
        let yaml = serde_yaml::to_string(&client).unwrap();
        let deserialized: Client = serde_yaml::from_str(&yaml).unwrap();
        assert!(deserialized.roots.is_some());
        assert_eq!(deserialized.roots.as_ref().unwrap().len(), 1);
        assert_eq!(
            deserialized.roots.as_ref().unwrap()[0].uri,
            "file:///custom/path"
        );
    }

    #[test]
    fn test_client_sampling_config_defaults() {
        let client = Client::new("Test Client".to_string());

        // Sampling disabled by default
        assert!(!client.mcp_sampling_enabled);

        // But requires approval when enabled
        assert!(client.mcp_sampling_requires_approval);

        // No limits by default
        assert!(client.mcp_sampling_max_tokens.is_none());
        assert!(client.mcp_sampling_rate_limit.is_none());
    }

    #[test]
    fn test_client_with_sampling_enabled() {
        let mut client = Client::new("Test Client".to_string());
        client.mcp_sampling_enabled = true;
        client.mcp_sampling_requires_approval = false;
        client.mcp_sampling_max_tokens = Some(2000);
        client.mcp_sampling_rate_limit = Some(100);

        // Verify serialization
        let yaml = serde_yaml::to_string(&client).unwrap();
        let deserialized: Client = serde_yaml::from_str(&yaml).unwrap();

        assert!(deserialized.mcp_sampling_enabled);
        assert!(!deserialized.mcp_sampling_requires_approval);
        assert_eq!(deserialized.mcp_sampling_max_tokens, Some(2000));
        assert_eq!(deserialized.mcp_sampling_rate_limit, Some(100));
    }
}
