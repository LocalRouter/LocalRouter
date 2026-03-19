use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use uuid::Uuid;

pub(crate) const CONFIG_VERSION: u32 = 23;

/// Keyring service name for provider API keys
pub const PROVIDER_KEYRING_SERVICE: &str = "LocalRouter-Providers";

/// Keyring service name for MCP server secrets
pub const MCP_KEYRING_SERVICE: &str = "LocalRouter-McpServers";

/// Suffix for auto-generated client strategy names
pub const CLIENT_STRATEGY_NAME_SUFFIX: &str = "'s strategy";

/// Generate a strategy name for a client
pub fn client_strategy_name(client_name: &str) -> String {
    format!("{}{}", client_name, CLIENT_STRATEGY_NAME_SUFFIX)
}

/// Time window for rate limits
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RateLimitTimeWindow {
    Minute,
    Hour,
    Day,
}

impl RateLimitTimeWindow {
    pub fn to_seconds(self) -> i64 {
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
    #[serde(default = "default_true")]
    pub enabled: bool,
}

/// Available models selection configuration
///
/// Determines which models are allowed for a strategy. The selection is evaluated in order:
/// 1. If `selected_all` is true, all models are allowed (including future ones)
/// 2. Otherwise, check if provider is in `selected_providers`
/// 3. Otherwise, check if specific model is in `selected_models`
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AvailableModelsSelection {
    /// If true, all models are allowed (including future models from new providers)
    #[serde(default = "default_selected_all")]
    pub selected_all: bool,
    /// Providers where ALL models are selected (including future models from that provider)
    #[serde(default)]
    pub selected_providers: Vec<String>,
    /// Individual models selected as (provider, model) pairs
    #[serde(default)]
    pub selected_models: Vec<(String, String)>,
}

fn default_selected_all() -> bool {
    true
}

impl Default for AvailableModelsSelection {
    fn default() -> Self {
        Self::all()
    }
}

impl AvailableModelsSelection {
    /// Create a selection that allows all models
    pub fn all() -> Self {
        Self {
            selected_all: true,
            selected_providers: vec![],
            selected_models: vec![],
        }
    }

    /// Create a selection that allows no models (empty selection)
    pub fn none() -> Self {
        Self {
            selected_all: false,
            selected_providers: vec![],
            selected_models: vec![],
        }
    }

    /// Check if a model is allowed by this selection
    ///
    /// Returns true if:
    /// 1. `selected_all` is true, OR
    /// 2. The provider is in `selected_providers`, OR
    /// 3. The specific (provider, model) pair is in `selected_models`
    pub fn is_model_allowed(&self, provider_name: &str, model_id: &str) -> bool {
        // If all models are selected, everything is allowed
        if self.selected_all {
            return true;
        }

        // Check if the provider is in the selected_providers list
        if self
            .selected_providers
            .iter()
            .any(|p| p.eq_ignore_ascii_case(provider_name))
        {
            return true;
        }

        // Check if the specific (provider, model) pair is in selected_models
        self.selected_models
            .iter()
            .any(|(p, m)| p.eq_ignore_ascii_case(provider_name) && m.eq_ignore_ascii_case(model_id))
    }
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
    /// If win_rate >= threshold, route to strong model (uses prioritized_models from AutoModelConfig)
    /// Recommended: 0.3 (balanced), 0.7 (cost-optimized), 0.2 (quality-prioritized)
    pub threshold: f32,

    /// Weak model selection (used when win_rate < threshold)
    pub weak_models: Vec<(String, String)>,
}

impl Default for RouteLLMConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            threshold: 0.3, // Balanced profile
            weak_models: Vec::new(),
        }
    }
}

/// Auto model configuration for localrouter/auto virtual model
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct AutoModelConfig {
    /// Permission state for auto-routing (Allow/Ask/Off)
    #[serde(default)]
    pub permission: PermissionState,
    /// Backward-compat: old `enabled` bool → migrated to `permission` in v18
    #[serde(default, skip_serializing)]
    pub enabled: Option<bool>,
    /// Custom model name for the auto router (default: "localrouter/auto")
    #[serde(default = "default_auto_model_name")]
    pub model_name: String,
    /// Prioritized models list (in order) for fallback
    pub prioritized_models: Vec<(String, String)>,
    /// Available models (out of rotation)
    #[serde(default)]
    pub available_models: Vec<(String, String)>,
    /// RouteLLM intelligent routing configuration
    #[serde(skip_serializing_if = "Option::is_none")]
    pub routellm_config: Option<RouteLLMConfig>,
}

impl AutoModelConfig {
    /// Migrate the old `enabled` bool field into `permission` if present.
    /// Called during config migration v18.
    pub fn migrate_enabled_field(&mut self) {
        if let Some(was_enabled) = self.enabled.take() {
            if self.permission == PermissionState::Off {
                self.permission = if was_enabled {
                    PermissionState::Allow
                } else {
                    PermissionState::Off
                };
            }
        }
    }
}

fn default_auto_model_name() -> String {
    "localrouter/auto".to_string()
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
    /// When true, the router only uses free-tier models/providers.
    /// When all free providers are exhausted, returns 429 with retry-after.
    #[serde(default)]
    pub free_tier_only: bool,
    /// What to do when free-tier models are exhausted (only used when free_tier_only is true)
    #[serde(default)]
    pub free_tier_fallback: FreeTierFallback,
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
            free_tier_only: false,
            free_tier_fallback: FreeTierFallback::default(),
        }
    }

    /// Create a new strategy owned by a client
    pub fn new_for_client(client_id: String, client_name: String) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            name: client_strategy_name(&client_name),
            parent: Some(client_id),
            allowed_models: AvailableModelsSelection::all(),
            auto_config: None,
            rate_limits: vec![],
            free_tier_only: false,
            free_tier_fallback: FreeTierFallback::default(),
        }
    }

    /// Check if a model is allowed by this strategy
    pub fn is_model_allowed(&self, provider: &str, model: &str) -> bool {
        self.allowed_models.is_model_allowed(provider, model)
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

    /// Streaming session configuration
    #[serde(default)]
    pub streaming: StreamingConfig,

    /// Whether the setup wizard has been shown (first-run detection)
    #[serde(default)]
    pub setup_wizard_shown: bool,

    /// Health check configuration for providers and MCP servers
    #[serde(default)]
    pub health_check: HealthCheckConfig,

    /// Skills configuration (AgentSkills.io)
    #[serde(default)]
    pub skills: SkillsConfig,

    /// Marketplace configuration for MCP server and skill discovery
    #[serde(default)]
    pub marketplace: MarketplaceConfig,

    /// GuardRails configuration for content inspection
    #[serde(default)]
    pub guardrails: GuardrailsConfig,

    /// AI coding agents configuration
    #[serde(default)]
    pub coding_agents: CodingAgentsConfig,

    /// Context management configuration (context-mode integration)
    #[serde(default)]
    pub context_management: ContextManagementConfig,

    /// Prompt compression configuration (LLMLingua-2 integration)
    #[serde(default)]
    pub prompt_compression: PromptCompressionConfig,

    /// JSON repair configuration (automatic JSON healing)
    #[serde(default)]
    pub json_repair: JsonRepairConfig,

    /// MCP via LLM configuration (experimental agentic orchestrator)
    #[serde(default)]
    pub mcp_via_llm: McpViaLlmConfig,

    /// Secret scanning configuration
    #[serde(default)]
    pub secret_scanning: SecretScanningConfig,

    /// Memory configuration (Zillis memsearch integration)
    /// Configured globally, enabled per-client
    #[serde(default)]
    pub memory: MemoryConfig,
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
    /// Enable dynamic activity graph in system tray icon.
    /// When false, shows a static icon with notification overlays only.
    /// When true, shows a live token usage sparkline graph.
    #[serde(default)]
    pub tray_graph_enabled: bool,

    /// Graph refresh rate
    /// - Fast (1): 1 second per bar, 30 second total (start fresh)
    /// - Medium (10): 10 seconds per bar, 5 minute total (interpolated from minute data)
    /// - Slow (60): 1 minute per bar, 30 minute total (direct mapping)
    #[serde(default = "default_tray_graph_refresh_rate")]
    pub tray_graph_refresh_rate_secs: u64,

    /// Whether the sidebar is expanded (showing labels) or collapsed (icons only).
    /// Defaults to true (expanded) on fresh installs.
    #[serde(default = "default_sidebar_expanded")]
    pub sidebar_expanded: bool,
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

/// Health check mode for providers and MCP servers
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum HealthCheckMode {
    /// Check health periodically on a schedule
    #[default]
    Periodic,
    /// Only check health when requests fail
    OnFailure,
}

/// Health check configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HealthCheckConfig {
    /// Health check mode (periodic or on-failure)
    #[serde(default)]
    pub mode: HealthCheckMode,
    /// Whether periodic health checks are enabled
    /// When false, only on-failure and user-triggered health checks run
    #[serde(default)]
    pub periodic_enabled: bool,
    /// Interval between health checks (in seconds)
    /// Default: 600 (10 minutes)
    #[serde(default = "default_health_check_interval")]
    pub interval_secs: u64,
    /// Timeout for each health check (in seconds)
    /// Default: 5 seconds
    #[serde(default = "default_health_check_timeout")]
    pub timeout_secs: u64,
    /// Interval for accelerated recovery checks when providers are unhealthy (seconds)
    /// Default: 30 seconds
    #[serde(default = "default_recovery_interval")]
    pub recovery_interval_secs: u64,
    /// Debounce cooldown for on-failure health marking (seconds)
    /// Prevents flooding health updates when many requests fail simultaneously
    /// Default: 10 seconds
    #[serde(default = "default_failure_cooldown")]
    pub failure_cooldown_secs: u64,
}

fn default_health_check_interval() -> u64 {
    600 // 10 minutes
}

fn default_health_check_timeout() -> u64 {
    5 // 5 seconds
}

fn default_recovery_interval() -> u64 {
    30 // 30 seconds
}

fn default_failure_cooldown() -> u64 {
    10 // 10 seconds
}

impl Default for HealthCheckConfig {
    fn default() -> Self {
        Self {
            mode: HealthCheckMode::default(),
            periodic_enabled: false,
            interval_secs: default_health_check_interval(),
            timeout_secs: default_health_check_timeout(),
            recovery_interval_secs: default_recovery_interval(),
            failure_cooldown_secs: default_failure_cooldown(),
        }
    }
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

    /// Whether to use models.dev catalog as fallback
    #[serde(default = "default_true")]
    pub use_catalog_fallback: bool,
}

fn default_model_cache_ttl() -> u64 {
    300 // 5 minutes
}

impl Default for ModelCacheConfig {
    fn default() -> Self {
        Self {
            default_ttl_seconds: 300,
            provider_ttl_overrides: std::collections::HashMap::new(),
            use_catalog_fallback: true,
        }
    }
}

/// Streaming session configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StreamingConfig {
    /// Maximum concurrent streaming sessions per client
    #[serde(default = "default_max_sessions_per_client")]
    pub max_sessions_per_client: usize,

    /// Session timeout in seconds (default: 1 hour)
    #[serde(default = "default_session_timeout")]
    pub session_timeout_secs: u64,

    /// Heartbeat interval in seconds (default: 30s)
    #[serde(default = "default_heartbeat_interval")]
    pub heartbeat_interval_secs: u64,

    /// Maximum pending events in merge channel
    #[serde(default = "default_max_pending_events")]
    pub max_pending_events: usize,

    /// Request timeout in seconds (default: 60s)
    #[serde(default = "default_request_timeout")]
    pub request_timeout_secs: u64,
}

fn default_max_sessions_per_client() -> usize {
    5
}

fn default_session_timeout() -> u64 {
    3600
}

fn default_heartbeat_interval() -> u64 {
    30
}

fn default_max_pending_events() -> usize {
    1000
}

fn default_request_timeout() -> u64 {
    60
}

impl Default for StreamingConfig {
    fn default() -> Self {
        Self {
            max_sessions_per_client: default_max_sessions_per_client(),
            session_timeout_secs: default_session_timeout(),
            heartbeat_interval_secs: default_heartbeat_interval(),
            max_pending_events: default_max_pending_events(),
            request_timeout_secs: default_request_timeout(),
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

/// MCP server access control configuration
///
/// Defines which MCP servers a client can access:
/// - `None`: No MCP access at all
/// - `All`: Access to all configured MCP servers
/// - `Specific`: Access only to listed server IDs
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "snake_case")]
pub enum McpServerAccess {
    /// No MCP access (default for new clients)
    #[default]
    None,
    /// Access to all configured MCP servers
    All,
    /// Access only to specific servers by ID
    Specific(Vec<String>),
}

impl McpServerAccess {
    /// Check if a specific server is accessible
    pub fn can_access(&self, server_id: &str) -> bool {
        match self {
            McpServerAccess::None => false,
            McpServerAccess::All => true,
            McpServerAccess::Specific(servers) => servers.contains(&server_id.to_string()),
        }
    }

    /// Check if any MCP access is granted
    pub fn has_any_access(&self) -> bool {
        !matches!(self, McpServerAccess::None)
    }

    /// Get the list of specific servers if in Specific mode
    pub fn specific_servers(&self) -> Option<&Vec<String>> {
        match self {
            McpServerAccess::Specific(servers) => Some(servers),
            _ => None,
        }
    }
}

/// Skills access control configuration
///
/// Defines which skills a client can access:
/// - `None`: No skills access (default)
/// - `All`: Access to all discovered skills
/// - `Specific`: Access only to listed skill names
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SkillsAccess {
    /// No skills access (default for new clients)
    #[default]
    None,
    /// Access to all discovered skills
    All,
    /// Access only to specific skills by name
    Specific(Vec<String>),
}

impl SkillsAccess {
    /// Check if a skill is accessible by its name
    pub fn can_access_by_name(&self, skill_name: &str) -> bool {
        match self {
            SkillsAccess::None => false,
            SkillsAccess::All => true,
            SkillsAccess::Specific(names) => names.iter().any(|n| n == skill_name),
        }
    }

    /// Check if any skills access is granted
    pub fn has_any_access(&self) -> bool {
        !matches!(self, SkillsAccess::None)
    }

    /// Get the list of skill names (for Specific mode)
    pub fn specific_skills(&self) -> Option<&Vec<String>> {
        match self {
            SkillsAccess::Specific(names) => Some(names),
            _ => None,
        }
    }
}

/// Firewall policy for MCP tool/skill access control
///
/// Determines how tool calls are handled:
/// - `Allow`: Tool call proceeds without restriction
/// - `Ask`: Tool call is held pending user approval (via popup)
/// - `Deny`: Tool call is rejected immediately
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum FirewallPolicy {
    /// Allow tool calls without restriction (default)
    #[default]
    Allow,
    /// Hold tool call and ask user for approval
    Ask,
    /// Deny tool call immediately
    Deny,
}

/// Unified permission state for access control
///
/// Merges access control and firewall into a single state:
/// - `Allow`: enabled, no approval needed
/// - `Ask`: enabled, requires approval popup
/// - `Off`: disabled (not accessible)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum PermissionState {
    /// Resource is enabled and allowed without restriction
    Allow,
    /// Resource is enabled but requires user approval
    Ask,
    /// Resource is disabled/not accessible
    #[default]
    Off,
}

impl PermissionState {
    /// Check if the resource is enabled (Allow or Ask)
    pub fn is_enabled(&self) -> bool {
        !matches!(self, PermissionState::Off)
    }

    /// Check if the resource requires approval
    pub fn requires_approval(&self) -> bool {
        matches!(self, PermissionState::Ask)
    }
}

/// MCP permission configuration for a client
///
/// Hierarchical permission system:
/// - global: applies to all MCP servers
/// - servers: per-server overrides
/// - tools: per-tool overrides (key: "server_id__tool_name")
/// - resources: per-resource overrides (key: "server_id__resource_uri")
/// - prompts: per-prompt overrides (key: "server_id__prompt_name")
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct McpPermissions {
    /// Global permission for all MCP servers
    #[serde(default)]
    pub global: PermissionState,
    /// Per-server permission overrides (server_id -> state)
    #[serde(default, skip_serializing_if = "std::collections::HashMap::is_empty")]
    pub servers: std::collections::HashMap<String, PermissionState>,
    /// Per-tool permission overrides (server_id__tool_name -> state)
    #[serde(default, skip_serializing_if = "std::collections::HashMap::is_empty")]
    pub tools: std::collections::HashMap<String, PermissionState>,
    /// Per-resource permission overrides (server_id__resource_uri -> state)
    #[serde(default, skip_serializing_if = "std::collections::HashMap::is_empty")]
    pub resources: std::collections::HashMap<String, PermissionState>,
    /// Per-prompt permission overrides (server_id__prompt_name -> state)
    #[serde(default, skip_serializing_if = "std::collections::HashMap::is_empty")]
    pub prompts: std::collections::HashMap<String, PermissionState>,
}

impl McpPermissions {
    /// Resolve the effective permission for an MCP server
    pub fn resolve_server(&self, server_id: &str) -> PermissionState {
        self.servers
            .get(server_id)
            .cloned()
            .unwrap_or(self.global.clone())
    }

    /// Resolve the effective permission for an MCP tool
    ///
    /// Resolution order: tool -> server -> global
    pub fn resolve_tool(&self, server_id: &str, tool_name: &str) -> PermissionState {
        let tool_key = format!("{}__{}", server_id, tool_name);
        if let Some(state) = self.tools.get(&tool_key) {
            return state.clone();
        }
        self.resolve_server(server_id)
    }

    /// Resolve the effective permission for an MCP resource
    pub fn resolve_resource(&self, server_id: &str, resource_uri: &str) -> PermissionState {
        let resource_key = format!("{}__{}", server_id, resource_uri);
        if let Some(state) = self.resources.get(&resource_key) {
            return state.clone();
        }
        self.resolve_server(server_id)
    }

    /// Resolve the effective permission for an MCP prompt
    pub fn resolve_prompt(&self, server_id: &str, prompt_name: &str) -> PermissionState {
        let prompt_key = format!("{}__{}", server_id, prompt_name);
        if let Some(state) = self.prompts.get(&prompt_key) {
            return state.clone();
        }
        self.resolve_server(server_id)
    }

    /// Check if a server has any enabled permissions (server-level or sub-item level)
    ///
    /// Returns true if:
    /// - The server itself is enabled (Allow/Ask), OR
    /// - Any tool under this server is explicitly enabled, OR
    /// - Any resource under this server is explicitly enabled, OR
    /// - Any prompt under this server is explicitly enabled
    pub fn has_any_enabled_for_server(&self, server_id: &str) -> bool {
        // Check server-level permission
        if self.resolve_server(server_id).is_enabled() {
            return true;
        }

        // Check if any tools for this server are enabled
        let prefix = format!("{server_id}__");
        for (key, state) in &self.tools {
            if key.starts_with(&prefix) && state.is_enabled() {
                return true;
            }
        }

        // Check if any resources for this server are enabled
        for (key, state) in &self.resources {
            if key.starts_with(&prefix) && state.is_enabled() {
                return true;
            }
        }

        // Check if any prompts for this server are enabled
        for (key, state) in &self.prompts {
            if key.starts_with(&prefix) && state.is_enabled() {
                return true;
            }
        }

        false
    }

    /// Check if the client has any MCP access configured at any level
    ///
    /// Returns true if:
    /// - Global MCP permission is enabled, OR
    /// - Any server has an enabled permission, OR
    /// - Any tool/resource/prompt is explicitly enabled
    pub fn has_any_access(&self) -> bool {
        // Check global permission
        if self.global.is_enabled() {
            return true;
        }

        // Check server-level permissions
        for state in self.servers.values() {
            if state.is_enabled() {
                return true;
            }
        }

        // Check tool-level permissions
        for state in self.tools.values() {
            if state.is_enabled() {
                return true;
            }
        }

        // Check resource-level permissions
        for state in self.resources.values() {
            if state.is_enabled() {
                return true;
            }
        }

        // Check prompt-level permissions
        for state in self.prompts.values() {
            if state.is_enabled() {
                return true;
            }
        }

        false
    }
}

/// Skills permission configuration for a client
///
/// Hierarchical permission system:
/// - global: applies to all skills
/// - skills: per-skill overrides
/// - tools: per-skill-tool overrides (key: "skill_name__tool_name")
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct SkillsPermissions {
    /// Global permission for all skills
    #[serde(default)]
    pub global: PermissionState,
    /// Per-skill permission overrides (skill_name -> state)
    #[serde(default, skip_serializing_if = "std::collections::HashMap::is_empty")]
    pub skills: std::collections::HashMap<String, PermissionState>,
    /// Per-skill-tool permission overrides (skill_name__tool_name -> state)
    #[serde(default, skip_serializing_if = "std::collections::HashMap::is_empty")]
    pub tools: std::collections::HashMap<String, PermissionState>,
}

impl SkillsPermissions {
    /// Resolve the effective permission for a skill
    pub fn resolve_skill(&self, skill_name: &str) -> PermissionState {
        self.skills
            .get(skill_name)
            .cloned()
            .unwrap_or(self.global.clone())
    }

    /// Resolve the effective permission for a skill tool
    ///
    /// Resolution order: tool -> skill -> global
    pub fn resolve_tool(&self, skill_name: &str, tool_name: &str) -> PermissionState {
        let tool_key = format!("{}__{}", skill_name, tool_name);
        if let Some(state) = self.tools.get(&tool_key) {
            return state.clone();
        }
        self.resolve_skill(skill_name)
    }

    /// Check if a skill has any enabled permissions (skill-level or tool-level)
    ///
    /// Returns true if:
    /// - The skill itself is enabled (Allow/Ask), OR
    /// - Any tool under this skill is explicitly enabled
    pub fn has_any_enabled_for_skill(&self, skill_name: &str) -> bool {
        // Check skill-level permission
        if self.resolve_skill(skill_name).is_enabled() {
            return true;
        }

        // Check if any tools for this skill are enabled
        let prefix = format!("{skill_name}__");
        for (key, state) in &self.tools {
            if key.starts_with(&prefix) && state.is_enabled() {
                return true;
            }
        }

        false
    }

    /// Check if the client has any skills access configured at any level
    ///
    /// Returns true if:
    /// - Global skills permission is enabled, OR
    /// - Any skill has an enabled permission, OR
    /// - Any skill tool is explicitly enabled
    pub fn has_any_access(&self) -> bool {
        // Check global permission
        if self.global.is_enabled() {
            return true;
        }

        // Check skill-level permissions
        for state in self.skills.values() {
            if state.is_enabled() {
                return true;
            }
        }

        // Check tool-level permissions
        for state in self.tools.values() {
            if state.is_enabled() {
                return true;
            }
        }

        false
    }
}

/// Model permission configuration for a client
///
/// Hierarchical permission system:
/// - global: applies to all models
/// - providers: per-provider overrides
/// - models: per-model overrides (key: "provider__model_id")
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ModelPermissions {
    /// Global permission for all models
    #[serde(default)]
    pub global: PermissionState,
    /// Per-provider permission overrides (provider_name -> state)
    #[serde(default, skip_serializing_if = "std::collections::HashMap::is_empty")]
    pub providers: std::collections::HashMap<String, PermissionState>,
    /// Per-model permission overrides (provider__model_id -> state)
    #[serde(default, skip_serializing_if = "std::collections::HashMap::is_empty")]
    pub models: std::collections::HashMap<String, PermissionState>,
}

impl ModelPermissions {
    /// Resolve the effective permission for a provider
    pub fn resolve_provider(&self, provider_name: &str) -> PermissionState {
        self.providers
            .get(provider_name)
            .cloned()
            .unwrap_or(self.global.clone())
    }

    /// Resolve the effective permission for a model
    ///
    /// Resolution order: model -> provider -> global
    pub fn resolve_model(&self, provider_name: &str, model_id: &str) -> PermissionState {
        let model_key = format!("{}__{}", provider_name, model_id);
        if let Some(state) = self.models.get(&model_key) {
            return state.clone();
        }
        self.resolve_provider(provider_name)
    }

    /// Check if a provider has any enabled permissions (provider-level or model-level)
    ///
    /// Returns true if:
    /// - The provider itself is enabled (Allow/Ask), OR
    /// - Any model under this provider is explicitly enabled
    pub fn has_any_enabled_for_provider(&self, provider_name: &str) -> bool {
        // Check provider-level permission
        if self.resolve_provider(provider_name).is_enabled() {
            return true;
        }

        // Check if any models for this provider are enabled
        let prefix = format!("{provider_name}__");
        for (key, state) in &self.models {
            if key.starts_with(&prefix) && state.is_enabled() {
                return true;
            }
        }

        false
    }

    /// Check if the client has any model access configured at any level
    ///
    /// Returns true if:
    /// - Global model permission is enabled, OR
    /// - Any provider has an enabled permission, OR
    /// - Any model is explicitly enabled
    pub fn has_any_access(&self) -> bool {
        // Check global permission
        if self.global.is_enabled() {
            return true;
        }

        // Check provider-level permissions
        for state in self.providers.values() {
            if state.is_enabled() {
                return true;
            }
        }

        // Check model-level permissions
        for state in self.models.values() {
            if state.is_enabled() {
                return true;
            }
        }

        false
    }
}

/// Per-client firewall rules for MCP tools and skills
///
/// Resolution order (most specific wins):
/// 1. `tool_rules["server__tool_name"]` — exact tool match
/// 2. `skill_tool_rules["skill_tool_name"]` — exact skill tool match
/// 3. `server_rules[server_id]` — server-level policy
/// 4. `skill_rules[skill_name]` — skill-level policy
/// 5. `default_policy` — fallback
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct FirewallRules {
    /// Default policy when no specific rule matches (default: Allow)
    #[serde(default)]
    pub default_policy: FirewallPolicy,

    /// Per-server policies (server_id -> policy)
    #[serde(default, skip_serializing_if = "std::collections::HashMap::is_empty")]
    pub server_rules: std::collections::HashMap<String, FirewallPolicy>,

    /// Per-tool policies (namespaced tool name e.g. "filesystem__write_file" -> policy)
    #[serde(default, skip_serializing_if = "std::collections::HashMap::is_empty")]
    pub tool_rules: std::collections::HashMap<String, FirewallPolicy>,

    /// Per-skill policies (skill name -> policy)
    #[serde(default, skip_serializing_if = "std::collections::HashMap::is_empty")]
    pub skill_rules: std::collections::HashMap<String, FirewallPolicy>,

    /// Per-skill-tool policies (skill tool name -> policy)
    #[serde(default, skip_serializing_if = "std::collections::HashMap::is_empty")]
    pub skill_tool_rules: std::collections::HashMap<String, FirewallPolicy>,
}

impl FirewallRules {
    /// Resolve the effective policy for an MCP tool call
    ///
    /// Checks in order: tool_rules -> server_rules -> default_policy
    pub fn resolve_mcp_tool(&self, tool_name: &str, server_id: &str) -> &FirewallPolicy {
        // Most specific: exact tool name match
        if let Some(policy) = self.tool_rules.get(tool_name) {
            return policy;
        }
        // Server-level match
        if let Some(policy) = self.server_rules.get(server_id) {
            return policy;
        }
        // Fallback
        &self.default_policy
    }

    /// Resolve the effective policy for a skill tool call
    ///
    /// Checks in order: skill_tool_rules -> skill_rules -> default_policy
    pub fn resolve_skill_tool(&self, tool_name: &str, skill_name: &str) -> &FirewallPolicy {
        // Most specific: exact skill tool name match
        if let Some(policy) = self.skill_tool_rules.get(tool_name) {
            return policy;
        }
        // Skill-level match
        if let Some(policy) = self.skill_rules.get(skill_name) {
            return policy;
        }
        // Fallback
        &self.default_policy
    }

    /// Check if any rules are configured (non-default)
    pub fn has_any_rules(&self) -> bool {
        !matches!(self.default_policy, FirewallPolicy::Allow)
            || !self.server_rules.is_empty()
            || !self.tool_rules.is_empty()
            || !self.skill_rules.is_empty()
            || !self.skill_tool_rules.is_empty()
    }
}

fn default_skill_tool_name() -> String {
    "SkillRead".to_string()
}

fn default_skill_read_file_tool_name() -> String {
    "SkillReadFile".to_string()
}

/// Skills configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SkillsConfig {
    /// Unified list of skill source paths (directories, zip files, .skill files)
    #[serde(default)]
    pub paths: Vec<String>,

    /// Globally disabled skill names
    #[serde(default)]
    pub disabled_skills: Vec<String>,

    /// Enable async script execution tools (default: false)
    #[serde(default)]
    pub async_enabled: bool,

    /// Tool name for the skill read meta-tool (default: "SkillRead")
    #[serde(default = "default_skill_tool_name")]
    pub tool_name: String,

    /// Tool name for internal skill file reading (default: "SkillReadFile")
    #[serde(default = "default_skill_read_file_tool_name")]
    pub read_file_tool_name: String,

    /// Migration shim: old auto_scan_directories (deserialize only)
    #[serde(default, skip_serializing)]
    pub auto_scan_directories: Vec<String>,

    /// Migration shim: old skill_paths (deserialize only)
    #[serde(default, skip_serializing)]
    pub skill_paths: Vec<String>,
}

impl Default for SkillsConfig {
    fn default() -> Self {
        Self {
            paths: Vec::new(),
            disabled_skills: Vec::new(),
            async_enabled: false,
            tool_name: default_skill_tool_name(),
            read_file_tool_name: default_skill_read_file_tool_name(),
            auto_scan_directories: Vec::new(),
            skill_paths: Vec::new(),
        }
    }
}

/// Approval mode for coding agent tool/question approvals
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CodingAgentApprovalMode {
    /// Auto-approve all tool usage and questions (dangerous — autonomous mode)
    Allow,
    /// Show approval popup in LocalRouter UI
    Ask,
    /// Forward via MCP elicitation to the client (falls back to Ask if unsupported)
    #[default]
    Elicitation,
}

fn default_tool_prefix() -> String {
    "Agent".to_string()
}

/// Coding agents configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CodingAgentsConfig {
    /// Migration shim: old per-agent configurations (deserialize only)
    #[serde(default, skip_serializing)]
    pub agents: Vec<CodingAgentConfig>,

    /// Migration shim: old default working directory (deserialize only)
    #[serde(default, skip_serializing)]
    pub default_working_directory: Option<String>,

    /// Maximum concurrent sessions across all agents (default: 10)
    #[serde(default = "default_max_concurrent_sessions")]
    pub max_concurrent_sessions: usize,

    /// Output ring buffer size per session in lines (default: 1000)
    #[serde(default = "default_output_buffer_size")]
    pub output_buffer_size: usize,

    /// Tool name prefix. Default: "Agent"
    /// If ends with non-alphanumeric char, suffixes are lowercase (e.g., "agent_start")
    /// If ends with alphanumeric char, suffixes are PascalCase (e.g., "AgentStart")
    #[serde(default = "default_tool_prefix")]
    pub tool_prefix: String,

    /// Approval mode for agent tool/question requests
    #[serde(default)]
    pub approval_mode: CodingAgentApprovalMode,
}

impl Default for CodingAgentsConfig {
    fn default() -> Self {
        Self {
            agents: Vec::new(),
            default_working_directory: None,
            max_concurrent_sessions: default_max_concurrent_sessions(),
            output_buffer_size: default_output_buffer_size(),
            tool_prefix: default_tool_prefix(),
            approval_mode: CodingAgentApprovalMode::default(),
        }
    }
}

fn default_max_concurrent_sessions() -> usize {
    10
}

fn default_output_buffer_size() -> usize {
    1000
}

/// Whether indexing is enabled or disabled for a given scope.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum IndexingState {
    #[default]
    Enable,
    Disable,
}

impl IndexingState {
    pub fn is_enabled(&self) -> bool {
        matches!(self, IndexingState::Enable)
    }
}

/// Unified MCP Gateway indexing permissions (GLOBAL only).
///
/// Hierarchy: global → server → tool. Absent keys inherit from parent.
/// Controls which gateway tools get their catalog entries and responses indexed.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct GatewayIndexingPermissions {
    /// Global default for all gateway tools
    #[serde(default)]
    pub global: IndexingState,
    /// Per-server overrides: server_slug → state
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub servers: HashMap<String, IndexingState>,
    /// Per-tool overrides: "server_slug__tool_name" → state
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub tools: HashMap<String, IndexingState>,
}

impl GatewayIndexingPermissions {
    /// Resolve the indexing state for a specific tool.
    /// Checks: tool override → server override → global default.
    pub fn resolve_tool(&self, server_slug: &str, tool_name: &str) -> &IndexingState {
        // Check tool-level override (namespaced as "server_slug__tool_name")
        let tool_key = format!("{}__{}", server_slug, tool_name);
        if let Some(state) = self.tools.get(&tool_key) {
            return state;
        }
        // Check server-level override
        if let Some(state) = self.servers.get(server_slug) {
            return state;
        }
        // Fall back to global
        &self.global
    }

    /// Resolve the indexing state for a server (used for server welcome text).
    /// Checks: server override → global default.
    pub fn resolve_server(&self, server_slug: &str) -> &IndexingState {
        if let Some(state) = self.servers.get(server_slug) {
            return state;
        }
        &self.global
    }

    /// Whether a tool's output should be indexed.
    pub fn is_tool_eligible(&self, server_slug: &str, tool_name: &str) -> bool {
        self.resolve_tool(server_slug, tool_name).is_enabled()
    }

    /// Whether a server's welcome content should be indexed.
    pub fn is_server_eligible(&self, server_slug: &str) -> bool {
        self.resolve_server(server_slug).is_enabled()
    }

    /// Returns true if any indexing is enabled at any level (global, server, or tool).
    pub fn has_any_enabled(&self) -> bool {
        if self.global.is_enabled() {
            return true;
        }
        self.servers.values().any(|s| s.is_enabled()) || self.tools.values().any(|s| s.is_enabled())
    }
}

/// Client Tools indexing permissions — global default + per-tool overrides.
///
/// Used both in global config (just `global`) and per-client (global override + tools).
/// Absent keys inherit: tool → client global → config global.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ClientToolsIndexingPermissions {
    /// Client-level override. None = inherit from global config default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub global: Option<IndexingState>,
    /// Per-tool overrides: tool_name → state
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub tools: HashMap<String, IndexingState>,
}

impl ClientToolsIndexingPermissions {
    /// Resolve the indexing state for a specific client tool.
    /// Checks: tool override → self.global → global_default.
    pub fn resolve_tool<'a>(
        &'a self,
        tool_name: &str,
        global_default: &'a IndexingState,
    ) -> &'a IndexingState {
        if let Some(state) = self.tools.get(tool_name) {
            return state;
        }
        if let Some(ref state) = self.global {
            return state;
        }
        global_default
    }

    /// Whether a client tool's output should be indexed.
    pub fn is_tool_eligible(&self, tool_name: &str, global_default: &IndexingState) -> bool {
        self.resolve_tool(tool_name, global_default).is_enabled()
    }
}

fn default_search_tool_name() -> String {
    "IndexSearch".to_string()
}

fn default_read_tool_name() -> String {
    "IndexRead".to_string()
}

/// Context management configuration (native FTS5 search & catalog compression).
///
/// When enabled, uses a per-session native ContentStore for FTS5 search,
/// content indexing, and progressive catalog compression to reduce
/// context window consumption.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ContextManagementConfig {
    /// Enable catalog compression (defer tools/resources/prompts behind ctx_search)
    #[serde(default = "default_true")]
    pub catalog_compression: bool,

    /// Progressive catalog compression kicks in above this byte threshold
    #[serde(default = "default_catalog_threshold_bytes")]
    pub catalog_threshold_bytes: usize,

    /// Compress individual tool/resource/prompt responses above this byte threshold
    #[serde(default = "default_response_threshold_bytes")]
    pub response_threshold_bytes: usize,

    /// Unified MCP Gateway indexing permissions (GLOBAL only)
    #[serde(default)]
    pub gateway_indexing: GatewayIndexingPermissions,

    /// Built-in virtual MCP server indexing permissions
    #[serde(default)]
    pub virtual_indexing: GatewayIndexingPermissions,

    /// Default On/Off for client tool indexing (all clients)
    #[serde(default)]
    pub client_tools_indexing_default: IndexingState,

    /// Search tool name (default: "IndexSearch")
    #[serde(default = "default_search_tool_name")]
    pub search_tool_name: String,

    /// Read tool name (default: "IndexRead")
    #[serde(default = "default_read_tool_name")]
    pub read_tool_name: String,

    /// Enable semantic vector search (hybrid FTS5 + embeddings) globally.
    /// When true and the embedding model is downloaded, all ContentStore instances
    /// (session + memory) use both keyword (FTS5) and vector (cosine similarity) matching.
    #[serde(default = "default_vector_search_enabled")]
    pub vector_search_enabled: bool,
}

/// Per-client context management overrides passed through the gateway API.
/// `None` fields fall back to the global `ContextManagementConfig` defaults.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ContextManagementOverrides {
    pub context_management_enabled: Option<bool>,
    pub catalog_compression_enabled: Option<bool>,
}

impl ContextManagementConfig {
    /// Context management is implicitly enabled when any indexing is configured.
    pub fn is_enabled(&self) -> bool {
        self.gateway_indexing.has_any_enabled()
            || self.virtual_indexing.has_any_enabled()
            || self.client_tools_indexing_default.is_enabled()
    }
}

impl Default for ContextManagementConfig {
    fn default() -> Self {
        Self {
            catalog_compression: true,
            catalog_threshold_bytes: default_catalog_threshold_bytes(),
            response_threshold_bytes: default_response_threshold_bytes(),
            gateway_indexing: GatewayIndexingPermissions::default(),
            virtual_indexing: GatewayIndexingPermissions::default(),
            client_tools_indexing_default: IndexingState::default(),
            search_tool_name: default_search_tool_name(),
            read_tool_name: default_read_tool_name(),
            vector_search_enabled: default_vector_search_enabled(),
        }
    }
}

fn default_catalog_threshold_bytes() -> usize {
    1000
}

fn default_response_threshold_bytes() -> usize {
    200
}

/// Configuration for a single coding agent.
/// An agent is implicitly enabled if its binary is installed on the system.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CodingAgentConfig {
    /// Which agent this configures
    pub agent_type: CodingAgentType,

    /// Default working directory for this agent's sessions
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_directory: Option<String>,

    /// Default model override
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,

    /// Default permission mode for new sessions
    #[serde(default)]
    pub permission_mode: CodingPermissionMode,

    /// Extra environment variables to set when spawning
    #[serde(default, skip_serializing_if = "std::collections::HashMap::is_empty")]
    pub env: std::collections::HashMap<String, String>,

    /// Custom binary path (auto-detected if not set)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binary_path: Option<String>,
}

/// Supported coding agent types
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum CodingAgentType {
    ClaudeCode,
    GeminiCli,
    Codex,
    Amp,
    Aider,
    Cursor,
    Opencode,
    QwenCode,
    Copilot,
    Droid,
}

impl CodingAgentType {
    /// Tool name prefix (used in MCP tool names like `{prefix}_start`)
    pub fn tool_prefix(&self) -> &str {
        match self {
            CodingAgentType::ClaudeCode => "claude_code",
            CodingAgentType::GeminiCli => "gemini_cli",
            CodingAgentType::Codex => "codex",
            CodingAgentType::Amp => "amp",
            CodingAgentType::Aider => "aider",
            CodingAgentType::Cursor => "cursor",
            CodingAgentType::Opencode => "opencode",
            CodingAgentType::QwenCode => "qwen_code",
            CodingAgentType::Copilot => "copilot",
            CodingAgentType::Droid => "droid",
        }
    }

    /// Human-readable display name
    pub fn display_name(&self) -> &str {
        match self {
            CodingAgentType::ClaudeCode => "Claude Code",
            CodingAgentType::GeminiCli => "Gemini CLI",
            CodingAgentType::Codex => "Codex",
            CodingAgentType::Amp => "Amp",
            CodingAgentType::Aider => "Aider",
            CodingAgentType::Cursor => "Cursor",
            CodingAgentType::Opencode => "Opencode",
            CodingAgentType::QwenCode => "Qwen Code",
            CodingAgentType::Copilot => "Copilot",
            CodingAgentType::Droid => "Droid",
        }
    }

    /// CLI binary name for auto-detection
    pub fn binary_name(&self) -> &str {
        match self {
            CodingAgentType::ClaudeCode => "claude",
            CodingAgentType::GeminiCli => "gemini",
            CodingAgentType::Codex => "codex",
            CodingAgentType::Amp => "amp",
            CodingAgentType::Aider => "aider",
            CodingAgentType::Cursor => "cursor",
            CodingAgentType::Opencode => "opencode",
            CodingAgentType::QwenCode => "qwen",
            CodingAgentType::Copilot => "copilot",
            CodingAgentType::Droid => "droid",
        }
    }

    /// Short description of the agent
    pub fn description(&self) -> &str {
        match self {
            CodingAgentType::ClaudeCode => "Anthropic's agentic coding tool. Operates directly in your terminal, understanding your codebase and executing commands.",
            CodingAgentType::GeminiCli => "Google's AI coding assistant for the command line, powered by Gemini models.",
            CodingAgentType::Codex => "OpenAI's autonomous coding agent that can write, run, and debug code in a sandboxed environment.",
            CodingAgentType::Amp => "Sourcegraph's AI coding agent for multi-step code tasks with full project context.",
            CodingAgentType::Aider => "AI pair programming in your terminal. Works with most LLMs, supports Git integration.",
            CodingAgentType::Cursor => "Cursor's CLI agent for AI-powered code editing and generation.",
            CodingAgentType::Opencode => "Open-source terminal AI coding assistant with multi-provider support.",
            CodingAgentType::QwenCode => "Alibaba's coding agent powered by Qwen models.",
            CodingAgentType::Copilot => "GitHub Copilot's CLI extension for terminal-based code assistance.",
            CodingAgentType::Droid => "Autonomous coding agent with a focus on full-stack development.",
        }
    }

    /// Whether the agent supports model selection via CLI
    pub fn supports_model_selection(&self) -> bool {
        matches!(
            self,
            CodingAgentType::ClaudeCode
                | CodingAgentType::GeminiCli
                | CodingAgentType::Codex
                | CodingAgentType::Aider
                | CodingAgentType::Opencode
        )
    }

    /// Which permission modes the agent supports
    pub fn supported_permission_modes(&self) -> Vec<CodingPermissionMode> {
        match self {
            CodingAgentType::ClaudeCode => vec![
                CodingPermissionMode::Auto,
                CodingPermissionMode::Supervised,
                CodingPermissionMode::Plan,
            ],
            CodingAgentType::Codex => {
                vec![CodingPermissionMode::Auto, CodingPermissionMode::Supervised]
            }
            _ => vec![CodingPermissionMode::Supervised],
        }
    }

    /// Version flag for the CLI binary (used to detect version)
    pub fn version_flag(&self) -> &str {
        match self {
            CodingAgentType::Aider => "--version",
            _ => "--version",
        }
    }

    /// All known agent types
    pub fn all() -> &'static [CodingAgentType] {
        &[
            CodingAgentType::ClaudeCode,
            CodingAgentType::GeminiCli,
            CodingAgentType::Codex,
            CodingAgentType::Amp,
            CodingAgentType::Aider,
            CodingAgentType::Cursor,
            CodingAgentType::Opencode,
            CodingAgentType::QwenCode,
            CodingAgentType::Copilot,
            CodingAgentType::Droid,
        ]
    }
}

impl std::fmt::Display for CodingAgentType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.display_name())
    }
}

/// Permission mode for coding agent sessions
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum CodingPermissionMode {
    /// Auto-approve all tool usage
    Auto,
    /// Require approval for tool usage
    #[default]
    Supervised,
    /// Plan-only mode
    Plan,
}

/// Per-client permissions for coding agents
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct CodingAgentsPermissions {
    /// Global permission for all coding agents (default: Off)
    #[serde(default)]
    pub global: PermissionState,
    /// Per-agent overrides (agent tool_prefix -> state)
    #[serde(default, skip_serializing_if = "std::collections::HashMap::is_empty")]
    pub agents: std::collections::HashMap<String, PermissionState>,
}

impl CodingAgentsPermissions {
    /// Resolve permission for a specific agent
    pub fn resolve_agent(&self, agent_prefix: &str) -> PermissionState {
        self.agents
            .get(agent_prefix)
            .cloned()
            .unwrap_or_else(|| self.global.clone())
    }

    /// Check if any agent access is possible
    pub fn has_any_access(&self) -> bool {
        self.global.is_enabled() || self.agents.values().any(|s| s.is_enabled())
    }
}

fn default_marketplace_search_tool_name() -> String {
    "MarketplaceSearch".to_string()
}

fn default_marketplace_install_tool_name() -> String {
    "MarketplaceInstall".to_string()
}

/// Marketplace configuration for MCP server and skill discovery
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MarketplaceConfig {
    /// Whether MCP marketplace is enabled
    #[serde(default)]
    pub mcp_enabled: bool,

    /// Whether Skills marketplace is enabled
    #[serde(default)]
    pub skills_enabled: bool,

    /// Legacy field for backward-compatible deserialization
    #[serde(default, skip_serializing)]
    pub enabled: bool,

    /// MCP server registry URL
    #[serde(default = "default_marketplace_registry_url")]
    pub registry_url: String,

    /// Skill source repos to browse
    #[serde(default = "default_marketplace_skill_sources")]
    pub skill_sources: Vec<MarketplaceSkillSource>,

    /// Search tool name (default: "MarketplaceSearch")
    #[serde(default = "default_marketplace_search_tool_name")]
    pub search_tool_name: String,

    /// Install tool name (default: "MarketplaceInstall")
    #[serde(default = "default_marketplace_install_tool_name")]
    pub install_tool_name: String,
}

impl Default for MarketplaceConfig {
    fn default() -> Self {
        Self {
            mcp_enabled: false,
            skills_enabled: false,
            enabled: false,
            registry_url: default_marketplace_registry_url(),
            skill_sources: default_marketplace_skill_sources(),
            search_tool_name: default_marketplace_search_tool_name(),
            install_tool_name: default_marketplace_install_tool_name(),
        }
    }
}

fn default_marketplace_registry_url() -> String {
    "https://registry.modelcontextprotocol.io/v0.1/servers".to_string()
}

fn default_marketplace_skill_sources() -> Vec<MarketplaceSkillSource> {
    vec![
        MarketplaceSkillSource {
            repo_url: "https://github.com/anthropics/skills".to_string(),
            branch: "main".to_string(),
            path: "skills".to_string(),
            label: "Anthropic".to_string(),
        },
        MarketplaceSkillSource {
            repo_url: "https://github.com/vercel-labs/agent-skills".to_string(),
            branch: "main".to_string(),
            path: "skills".to_string(),
            label: "Vercel".to_string(),
        },
        MarketplaceSkillSource {
            repo_url: "https://github.com/openai/skills".to_string(),
            branch: "main".to_string(),
            path: "skills/.curated".to_string(),
            label: "OpenAI".to_string(),
        },
        MarketplaceSkillSource {
            repo_url: "https://github.com/microsoft/agent-skills".to_string(),
            branch: "main".to_string(),
            path: ".github/skills".to_string(),
            label: "Microsoft".to_string(),
        },
        MarketplaceSkillSource {
            repo_url: "https://github.com/sickn33/antigravity-awesome-skills".to_string(),
            branch: "main".to_string(),
            path: "skills".to_string(),
            label: "Antigravity".to_string(),
        },
    ]
}

/// A skill source repository configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MarketplaceSkillSource {
    /// GitHub repository URL
    pub repo_url: String,

    /// Branch to use (default: main)
    #[serde(default = "default_main_branch")]
    pub branch: String,

    /// Path within the repo containing skills
    #[serde(default)]
    pub path: String,

    /// Human-readable label for this source
    pub label: String,
}

fn default_main_branch() -> String {
    "main".to_string()
}

/// GuardRails configuration for LLM-based content safety
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GuardrailsConfig {
    /// Migration shim: old master toggle (deserialize only, not serialized)
    #[serde(default, skip_serializing)]
    pub enabled: bool,

    /// Scan outgoing requests before sending to provider
    #[serde(default = "default_true")]
    pub scan_requests: bool,

    /// Migration shim: old scan_responses (deserialize only, not serialized)
    #[serde(default, skip_serializing)]
    pub scan_responses: bool,

    /// Configured safety models
    #[serde(default = "default_safety_models")]
    pub safety_models: Vec<SafetyModelConfig>,

    /// Default category actions for all clients. Per-client overrides take precedence.
    #[serde(default)]
    pub category_actions: Vec<CategoryActionEntry>,

    /// Default confidence threshold for flagging (0.0-1.0)
    #[serde(default = "default_confidence_threshold")]
    pub default_confidence_threshold: f32,

    /// Run guardrails in parallel with LLM request, buffering response until safe (default: true).
    /// Falls back to sequential when side effects are detected (e.g. web search tools, Perplexity Sonar).
    /// For MCP via LLM, guardrails run in parallel but gate before tool execution.
    #[serde(default = "default_true")]
    pub parallel_guardrails: bool,

    /// Enable the /v1/moderations API endpoint.
    /// When enabled, uses configured safety models to serve moderation requests
    /// in OpenAI-compatible format. Requires auth.
    #[serde(default = "default_true")]
    pub moderation_api_enabled: bool,
}

fn default_confidence_threshold() -> f32 {
    0.5
}

impl Default for GuardrailsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            scan_requests: true,
            scan_responses: false,
            safety_models: default_safety_models(),
            category_actions: vec![],
            default_confidence_threshold: default_confidence_threshold(),
            parallel_guardrails: true,
            moderation_api_enabled: true,
        }
    }
}

/// Per-client guardrails configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ClientGuardrailsConfig {
    /// Migration shim: old enabled flag (deserialize only, not serialized).
    #[serde(default, skip_serializing)]
    pub enabled: bool,
    /// Per-category actions override: None = inherit global defaults, Some = client-specific.
    #[serde(default)]
    pub category_actions: Option<Vec<CategoryActionEntry>>,
}

/// Configuration for a single safety model
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SafetyModelConfig {
    /// Unique identifier (e.g. "granite_guardian_2b")
    pub id: String,
    /// Display name
    pub label: String,
    /// Model type: "llama_guard_4", "shield_gemma", "nemotron", "granite_guardian"
    pub model_type: String,
    /// Whether this safety model is active. Disabled models stay in config
    /// but are skipped during guardrails checks.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Use existing provider (e.g. "ollama", "openrouter")
    #[serde(default)]
    pub provider_id: Option<String>,
    /// Model name on the provider (e.g. "granite3-guardian:2b")
    #[serde(default)]
    pub model_name: Option<String>,
    /// Override the global confidence threshold for this model
    #[serde(default)]
    pub confidence_threshold: Option<f32>,
    /// Subset of categories to enable (None = all)
    #[serde(default)]
    pub enabled_categories: Option<Vec<String>>,
    /// Custom prompt template with `{content}` placeholder (for custom model_type)
    #[serde(default)]
    pub prompt_template: Option<String>,
    /// Safe indicator string in model output (e.g. "safe")
    #[serde(default)]
    pub safe_indicator: Option<String>,
    /// Regex to extract category from model output
    #[serde(default)]
    pub output_regex: Option<String>,
    /// Custom mapping from native model labels to safety categories
    #[serde(default)]
    pub category_mapping: Option<Vec<CategoryMappingEntry>>,
}

/// Mapping from a model's native output label to a normalized safety category
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CategoryMappingEntry {
    /// The label as output by the model (e.g. "S1", "violence")
    pub native_label: String,
    /// The normalized safety category (e.g. "violent_crimes", "hate")
    pub safety_category: String,
}

/// Per-category action configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CategoryActionEntry {
    /// SafetyCategory serialized name (e.g. "violent_crimes", "hate")
    pub category: String,
    /// Action: "allow", "notify", "ask", "block"
    #[serde(default = "default_category_action")]
    pub action: String,
}

fn default_category_action() -> String {
    "allow".to_string()
}

/// Default safety models: empty list.
/// Predefined models are catalog entries shown in the picker UI, not active models.
fn default_safety_models() -> Vec<SafetyModelConfig> {
    vec![]
}

/// Prompt compression configuration (LLMLingua-2 via Candle)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PromptCompressionConfig {
    /// Enable prompt compression globally (default: false)
    #[serde(default)]
    pub enabled: bool,

    /// Model for LLMLingua-2: "bert" (660MB, BERT Base Multilingual) or "xlm-roberta" (2.2GB, XLM-RoBERTa Large)
    #[serde(default = "default_compression_model_size")]
    pub model_size: String,

    /// Default compression rate (0.0-1.0, lower = more compression, default: 0.5)
    #[serde(default = "default_compression_rate")]
    pub default_rate: f32,

    /// Compress system prompts too (default: false)
    #[serde(default)]
    pub compress_system_prompt: bool,

    /// Minimum messages before compression activates (default: 6)
    #[serde(default = "default_min_messages")]
    pub min_messages: u32,

    /// Keep last N messages uncompressed (default: 4)
    #[serde(default = "default_preserve_recent")]
    pub preserve_recent: u32,

    /// Minimum word count for a message to be compressed (default: 5)
    #[serde(default = "default_min_message_words")]
    pub min_message_words: u32,

    /// Preserve quoted text and code blocks during compression (default: true)
    #[serde(default = "default_true")]
    pub preserve_quoted_text: bool,

    /// Prepend [abridged] to each compressed message (default: true)
    #[serde(default = "default_true")]
    pub compression_notice: bool,
}

fn default_compression_model_size() -> String {
    "bert".to_string()
}

fn default_compression_rate() -> f32 {
    0.8
}

fn default_min_messages() -> u32 {
    6
}

fn default_preserve_recent() -> u32 {
    4
}

fn default_min_message_words() -> u32 {
    5
}

impl Default for PromptCompressionConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            model_size: default_compression_model_size(),
            default_rate: default_compression_rate(),
            compress_system_prompt: false,
            min_messages: default_min_messages(),
            preserve_recent: default_preserve_recent(),
            min_message_words: default_min_message_words(),
            preserve_quoted_text: true,
            compression_notice: true,
        }
    }
}

/// Per-client prompt compression configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ClientPromptCompressionConfig {
    /// Enable compression for this client (None=inherit global, Some(bool)=override)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
}

/// JSON repair configuration (automatic JSON healing for LLM responses)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JsonRepairConfig {
    /// Enable JSON repair globally (default: true)
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Repair JSON syntax errors: trailing commas, unescaped chars,
    /// missing brackets, markdown wrappers (default: true)
    #[serde(default = "default_true")]
    pub syntax_repair: bool,

    /// Coerce JSON values to match expected schema (default: true)
    /// Requires response_format with json_schema to be effective
    #[serde(default = "default_true")]
    pub schema_coercion: bool,

    /// Remove fields not present in the schema (default: false)
    #[serde(default)]
    pub strip_extra_fields: bool,

    /// Add default values for missing required fields (default: true)
    #[serde(default = "default_true")]
    pub add_defaults: bool,

    /// Normalize enum values (case-insensitive matching) (default: true)
    #[serde(default = "default_true")]
    pub normalize_enums: bool,
}

impl Default for JsonRepairConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            syntax_repair: true,
            schema_coercion: true,
            strip_extra_fields: false,
            add_defaults: true,
            normalize_enums: true,
        }
    }
}

/// Per-client JSON repair configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ClientJsonRepairConfig {
    /// Enable JSON repair for this client (None=inherit global, Some(bool)=override)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,

    /// Override syntax repair setting
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub syntax_repair: Option<bool>,

    /// Override schema coercion setting
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema_coercion: Option<bool>,
}

/// Secret scanning action: what to do when a potential secret is detected
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SecretScanAction {
    /// Block the request and show a popup for user decision
    Ask,
    /// Allow the request but show a notification
    Notify,
    /// No scanning
    #[default]
    Off,
}

/// Secret scanning configuration (global)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SecretScanningConfig {
    /// What action to take when a secret is detected
    #[serde(default)]
    pub action: SecretScanAction,

    /// Minimum Shannon entropy for a match to be considered valid (global only)
    #[serde(default = "default_entropy_threshold")]
    pub entropy_threshold: f32,

    /// Whether to scan system messages (default: false)
    #[serde(default)]
    pub scan_system_messages: bool,

    /// Allowlist regex patterns that exclude matches (global only)
    #[serde(default)]
    pub allowlist: Vec<String>,
}

impl Default for SecretScanningConfig {
    fn default() -> Self {
        Self {
            action: SecretScanAction::Off,
            entropy_threshold: default_entropy_threshold(),
            scan_system_messages: false,
            allowlist: Vec::new(),
        }
    }
}

fn default_entropy_threshold() -> f32 {
    3.5
}

/// Per-client secret scanning configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ClientSecretScanningConfig {
    /// Override action for this client (None = inherit global)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<SecretScanAction>,
}

// ============================================================================
// Memory Configuration (Zillis memsearch integration)
// ============================================================================

/// Global memory configuration. Memory is enabled per-client, not globally.
///
/// Memory uses native FTS5 full-text search (no external dependencies).
/// Transcripts are indexed automatically and searchable immediately.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MemoryConfig {
    /// Compaction LLM model for session summarization, routed through LocalRouter.
    /// Format: "provider/model" (e.g., "anthropic/claude-haiku-4-5-20251001").
    /// None = compaction disabled (raw transcripts kept).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compaction_model: Option<String>,

    /// Number of search results to return (default: 5)
    #[serde(default = "default_memory_top_k")]
    pub search_top_k: usize,

    /// Session inactivity timeout in minutes (default: 180 = 3 hours)
    #[serde(default = "default_session_inactivity_minutes")]
    pub session_inactivity_minutes: u64,

    /// Max session duration in minutes (default: 480 = 8 hours)
    #[serde(default = "default_max_session_minutes")]
    pub max_session_minutes: u64,

    /// Tool name for recall (default: "MemoryRecall")
    #[serde(default = "default_memory_recall_tool_name")]
    pub recall_tool_name: String,

    // Legacy fields — kept for backwards-compatible deserialization, ignored at runtime.
    // vector_search_enabled moved to ContextManagementConfig (global setting).
    #[serde(default, skip_serializing)]
    pub vector_search_enabled: Option<bool>,
    #[serde(default, skip_serializing)]
    pub embedding_model: Option<String>,
    #[serde(default, skip_serializing)]
    pub embedding: Option<serde_json::Value>,
    #[serde(default, skip_serializing)]
    pub compaction: Option<serde_json::Value>,
    #[serde(default, skip_serializing)]
    pub auto_start_daemon: Option<bool>,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            compaction_model: None,
            search_top_k: default_memory_top_k(),
            session_inactivity_minutes: default_session_inactivity_minutes(),
            max_session_minutes: default_max_session_minutes(),
            recall_tool_name: default_memory_recall_tool_name(),
            vector_search_enabled: None,
            embedding_model: None,
            embedding: None,
            compaction: None,
            auto_start_daemon: None,
        }
    }
}

fn default_memory_top_k() -> usize {
    5
}

fn default_session_inactivity_minutes() -> u64 {
    180
}

fn default_max_session_minutes() -> u64 {
    480
}

fn default_memory_recall_tool_name() -> String {
    "MemorySearch".to_string()
}

fn default_vector_search_enabled() -> bool {
    true
}

/// MCP via LLM configuration (experimental agentic orchestrator)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct McpViaLlmConfig {
    /// Session TTL in seconds (default: 3600 = 60 minutes since last activity)
    #[serde(default = "default_mcp_via_llm_session_ttl")]
    pub session_ttl_seconds: u64,

    /// Maximum concurrent sessions (default: 100)
    #[serde(default = "default_mcp_via_llm_max_sessions")]
    pub max_concurrent_sessions: usize,

    /// Maximum agentic loop iterations per request (default: 4, minimum: 1)
    #[serde(default = "default_mcp_via_llm_max_iterations")]
    pub max_loop_iterations: u32,

    /// Maximum total timeout for the agentic loop in seconds (default: 300)
    #[serde(default = "default_mcp_via_llm_max_timeout")]
    pub max_loop_timeout_seconds: u64,

    /// Expose MCP resources as synthetic function tools (default: true)
    #[serde(default = "default_true")]
    pub expose_resources_as_tools: bool,

    /// Inject MCP prompts into conversations (default: true)
    #[serde(default = "default_true")]
    pub inject_prompts: bool,
}

fn default_mcp_via_llm_session_ttl() -> u64 {
    3600
}
fn default_mcp_via_llm_max_sessions() -> usize {
    100
}
fn default_mcp_via_llm_max_iterations() -> u32 {
    4
}
fn default_mcp_via_llm_max_timeout() -> u64 {
    300
}

impl Default for McpViaLlmConfig {
    fn default() -> Self {
        Self {
            session_ttl_seconds: default_mcp_via_llm_session_ttl(),
            max_concurrent_sessions: default_mcp_via_llm_max_sessions(),
            max_loop_iterations: default_mcp_via_llm_max_iterations(),
            max_loop_timeout_seconds: default_mcp_via_llm_max_timeout(),
            expose_resources_as_tools: true,
            inject_prompts: true,
        }
    }
}

/// Deserializer for SkillsAccess (migration shim)
pub(crate) fn deserialize_skills_access<'de, D>(deserializer: D) -> Result<SkillsAccess, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::{self, Visitor};

    struct SkillsAccessVisitor;

    impl<'de> Visitor<'de> for SkillsAccessVisitor {
        type Value = SkillsAccess;

        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter
                .write_str("'none', 'all', or an object with 'specific' key containing skill names")
        }

        fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
            match v {
                "none" => Ok(SkillsAccess::None),
                "all" => Ok(SkillsAccess::All),
                _ => Err(E::custom(format!("unknown variant: {}", v))),
            }
        }

        fn visit_map<A: de::MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
            let mut specific: Option<Vec<String>> = None;
            while let Some(key) = map.next_key::<String>()? {
                match key.as_str() {
                    "specific" | "Specific" => {
                        specific = Some(map.next_value()?);
                    }
                    _ => {
                        let _: serde::de::IgnoredAny = map.next_value()?;
                    }
                }
            }
            match specific {
                Some(skills) => Ok(SkillsAccess::Specific(skills)),
                None => Err(de::Error::custom("expected 'specific' key in map")),
            }
        }

        fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
            Ok(SkillsAccess::None)
        }
    }

    deserializer.deserialize_any(SkillsAccessVisitor)
}

/// Client mode determines which features are exposed to the client
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ClientMode {
    /// Full access to both LLM routing and MCP features
    #[default]
    Both,
    /// Only LLM routing (no MCP servers or skills)
    LlmOnly,
    /// Only MCP proxy (no LLM model access)
    McpOnly,
    /// MCP tools injected into LLM requests, executed server-side (experimental)
    McpViaLlm,
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

    /// Migration shim: old LLM providers access (deserialize only)
    /// Use model_permissions instead
    #[serde(default, skip_serializing)]
    pub allowed_llm_providers: Vec<String>,

    /// Migration shim: old MCP server access (deserialize only)
    /// Use mcp_permissions instead
    #[serde(
        default,
        skip_serializing,
        deserialize_with = "deserialize_mcp_server_access"
    )]
    pub mcp_server_access: McpServerAccess,

    /// Enable context management for this client.
    /// None = inherit global setting, Some(false) = disabled regardless of global.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_management_enabled: Option<bool>,

    /// Enable catalog compression for this client.
    /// None = inherit global setting (enabled when context management is on),
    /// Some(false) = disabled regardless of global.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catalog_compression_enabled: Option<bool>,

    /// Per-client client tools indexing overrides (MCP via LLM only).
    /// None = no overrides, inherit global default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_tools_indexing: Option<ClientToolsIndexingPermissions>,

    /// Migration shim: old skills access (deserialize only)
    /// Use skills_permissions instead
    #[serde(
        default,
        skip_serializing,
        deserialize_with = "deserialize_skills_access"
    )]
    pub skills_access: SkillsAccess,

    /// When this client was created
    pub created_at: DateTime<Utc>,

    /// Last time this client was used
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_used: Option<DateTime<Utc>>,

    /// Reference to the routing strategy this client uses (required)
    pub strategy_id: String,

    /// MCP filesystem roots override (per-client)
    /// If None, uses global roots from AppConfig
    /// If Some, replaces global roots entirely for this client
    #[serde(skip_serializing_if = "Option::is_none")]
    pub roots: Option<Vec<RootConfig>>,

    /// Sampling permission (Allow/Ask/Off, default: Ask)
    #[serde(default = "default_ask")]
    pub mcp_sampling_permission: PermissionState,

    /// Elicitation permission (Allow/Ask/Off, default: Ask)
    #[serde(default = "default_ask")]
    pub mcp_elicitation_permission: PermissionState,

    /// Maximum tokens per sampling request (None = unlimited)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mcp_sampling_max_tokens: Option<u32>,

    /// Maximum sampling requests per hour (None = unlimited)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mcp_sampling_rate_limit: Option<u32>,

    /// Migration shim: old sampling enabled flag (deserialize only)
    #[serde(default, skip_serializing)]
    pub mcp_sampling_enabled: bool,

    /// Migration shim: old sampling requires approval flag (deserialize only)
    #[serde(default = "default_true", skip_serializing)]
    pub mcp_sampling_requires_approval: bool,

    /// Firewall rules for MCP tool/skill access control
    /// Controls per-tool Allow/Ask/Deny policies
    #[serde(default)]
    pub firewall: FirewallRules,

    /// Migration shim: old marketplace enabled flag (deserialize only)
    /// Use marketplace_permission instead
    #[serde(default, skip_serializing)]
    pub marketplace_enabled: bool,

    /// Unified MCP permissions (hierarchical Allow/Ask/Off)
    #[serde(default)]
    pub mcp_permissions: McpPermissions,

    /// Unified Skills permissions (hierarchical Allow/Ask/Off)
    #[serde(default)]
    pub skills_permissions: SkillsPermissions,

    /// Unified Model permissions (hierarchical Allow/Ask/Off)
    #[serde(default)]
    pub model_permissions: ModelPermissions,

    /// Marketplace permission state (Allow/Ask/Off)
    #[serde(default)]
    pub marketplace_permission: PermissionState,

    /// Migration shim: old hierarchical coding agents permissions (deserialize only)
    #[serde(default, skip_serializing)]
    pub coding_agents_permissions: CodingAgentsPermissions,

    /// Coding agent permission (Allow/Ask/Off)
    #[serde(default)]
    pub coding_agent_permission: PermissionState,

    /// Which coding agent type this client uses (None = not selected)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coding_agent_type: Option<CodingAgentType>,

    /// Client mode: controls which features (LLM, MCP, both) are exposed
    #[serde(default)]
    pub client_mode: ClientMode,

    /// Template ID used to create this client (e.g., "claude-code", "cursor")
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template_id: Option<String>,

    /// Auto-sync external app config files when models/secrets/config change.
    /// Only effective when template_id is set.
    #[serde(default)]
    pub sync_config: bool,

    /// Migration shim: old guardrails_enabled (deserialize only, not serialized)
    #[serde(default, skip_serializing)]
    pub guardrails_enabled: Option<bool>,

    /// Per-client guardrails configuration
    #[serde(default)]
    pub guardrails: ClientGuardrailsConfig,

    /// Per-client prompt compression configuration
    #[serde(default)]
    pub prompt_compression: ClientPromptCompressionConfig,

    /// Per-client JSON repair configuration
    #[serde(default)]
    pub json_repair: ClientJsonRepairConfig,

    /// Per-client secret scanning configuration
    #[serde(default)]
    pub secret_scanning: ClientSecretScanningConfig,

    /// Enable persistent memory for this client (default: disabled)
    /// When enabled, conversations are recorded and stored locally.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_enabled: Option<bool>,
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
        /// Full command to execute (parsed using shell-words at runtime)
        /// Example: "npx -y @modelcontextprotocol/server-filesystem /tmp"
        command: String,
        /// Legacy: Command arguments (deprecated, use command string instead)
        /// Kept for backward compatibility with existing configs
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
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

impl McpTransportConfig {
    /// Parse STDIO command into executable and arguments.
    ///
    /// Supports two formats for backward compatibility:
    /// 1. New format: Single command string parsed using shell-words
    ///    Example: "npx -y @modelcontextprotocol/server-filesystem /tmp"
    /// 2. Legacy format: Separate command + args fields
    ///
    /// Returns (executable, args, env) or error if parsing fails.
    #[allow(clippy::type_complexity)]
    pub fn parse_stdio_command(
        &self,
    ) -> Result<
        (
            String,
            Vec<String>,
            std::collections::HashMap<String, String>,
        ),
        String,
    > {
        match self {
            McpTransportConfig::Stdio { command, args, env } => {
                // If legacy args are provided, use them directly
                if !args.is_empty() {
                    return Ok((command.clone(), args.clone(), env.clone()));
                }

                // Parse the command string using shell-words
                let parts = shell_words::split(command)
                    .map_err(|e| format!("Failed to parse command '{}': {}", command, e))?;

                if parts.is_empty() {
                    return Err("Command is empty".to_string());
                }

                let executable = parts[0].clone();
                let parsed_args = parts[1..].to_vec();

                Ok((executable, parsed_args, env.clone()))
            }
            _ => Err("Not a STDIO transport".to_string()),
        }
    }
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
        /// Map of header name → keychain reference key
        /// Actual header values are stored in keychain (service: "LocalRouter-McpServers")
        header_refs: std::collections::HashMap<String, String>,
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
    #[serde(rename = "oauth_browser")]
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
        /// Map of env var name → keychain reference key
        /// Actual env var values are stored in keychain (service: "LocalRouter-McpServers")
        env_refs: std::collections::HashMap<String, String>,
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

/// What to do when free-tier models are exhausted
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FreeTierFallback {
    /// Return 429 error (no fallback)
    #[default]
    Off,
    /// Show approval popup before using paid models
    Ask,
    /// Automatically proceed with paid models
    Allow,
}

/// Free tier type for a provider
///
/// Providers have fundamentally different free tier models. This enum captures
/// all the variants with a common abstraction.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FreeTierKind {
    /// No known free tier. Always treated as paid.
    #[default]
    None,
    /// Local / self-hosted. Always free, no limits from provider.
    AlwaysFreeLocal,
    /// Subscription-based. Free within existing subscription.
    Subscription,
    /// Rate-limited free access (RPM/RPD/TPM) but no dollar credits.
    /// Used by: Gemini, Groq, Cerebras, Mistral, Cohere
    RateLimitedFree {
        /// Max requests per minute (0 = not tracked)
        #[serde(default)]
        max_rpm: u32,
        /// Max requests per day (0 = not tracked)
        #[serde(default)]
        max_rpd: u32,
        /// Max tokens per minute (0 = not tracked)
        #[serde(default)]
        max_tpm: u64,
        /// Max tokens per day (0 = not tracked)
        #[serde(default)]
        max_tpd: u64,
        /// Monthly call limit (0 = not tracked, Cohere: 1000)
        #[serde(default)]
        max_monthly_calls: u32,
        /// Monthly token limit (0 = not tracked, Mistral: 1B)
        #[serde(default)]
        max_monthly_tokens: u64,
    },
    /// Credit-based free tier (e.g. OpenRouter, xAI, DeepInfra)
    CreditBased {
        /// Budget in USD
        budget_usd: f64,
        /// Reset period
        reset_period: FreeTierResetPeriod,
        /// How credits are tracked
        detection: CreditDetection,
    },
    /// Specific free models only (e.g. Together AI free model, OpenRouter :free models)
    FreeModelsOnly {
        /// Model ID patterns that are free
        free_model_patterns: Vec<String>,
        /// Rate limits on free models (0 = not tracked)
        #[serde(default)]
        max_rpm: u32,
    },
}

/// Reset period for credit-based free tiers
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FreeTierResetPeriod {
    /// Resets daily
    Daily,
    /// Resets monthly
    Monthly,
    /// One-time credits, never resets
    Never,
}

/// How credit-based free tier usage is detected
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CreditDetection {
    /// All accounting is local (Together, DeepInfra, startup grants)
    LocalOnly,
    /// Use provider's built-in API (OpenRouter `/api/v1/key`)
    ProviderApi,
    /// Custom HTTP endpoint for checking credits
    CustomEndpoint {
        /// URL to check credits
        url: String,
        /// HTTP method (GET or POST)
        method: String,
        /// Headers with {{API_KEY}} template support
        #[serde(default)]
        headers: Vec<(String, String)>,
        /// JSONPath-like dotted path to extract remaining credits
        #[serde(skip_serializing_if = "Option::is_none")]
        remaining_credits_path: Option<String>,
        /// JSONPath-like dotted path to extract total credits
        #[serde(skip_serializing_if = "Option::is_none")]
        total_credits_path: Option<String>,
        /// JSONPath-like dotted path to extract is_free_tier flag
        #[serde(skip_serializing_if = "Option::is_none")]
        is_free_tier_path: Option<String>,
    },
}

/// Provider configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
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

    /// Free tier configuration for this provider instance.
    /// If None, uses the provider type's default free tier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub free_tier: Option<FreeTierKind>,
}

/// Provider type
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderType {
    /// Local Ollama instance
    Ollama,
    /// Local LM Studio instance
    #[serde(rename = "lmstudio")]
    LMStudio,
    /// OpenAI API
    OpenAI,
    /// OpenRouter proxy
    OpenRouter,
    /// Anthropic API
    Anthropic,
    /// Gemini API
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
    /// Local Jan.ai instance
    Jan,
    /// Local GPT4All instance
    #[serde(rename = "gpt4all")]
    GPT4All,
    /// Local LocalAI instance
    #[serde(rename = "localai")]
    LocalAI,
    /// Local llama.cpp server instance
    #[serde(rename = "llamacpp")]
    LlamaCpp,
    /// Custom provider
    Custom,
}

/// Logging configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LoggingConfig {
    /// Log level
    pub level: LogLevel,

    /// Enable access logging (disabled by default)
    #[serde(default)]
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

// Default value functions for serde
pub(crate) fn default_version() -> u32 {
    CONFIG_VERSION
}

pub(crate) fn default_true() -> bool {
    true
}

fn default_ask() -> PermissionState {
    PermissionState::Ask
}

fn default_log_retention() -> u32 {
    31
}
/// Deserializer for McpServerAccess (migration shim) that supports backward compatibility
/// with the old `allowed_mcp_servers: Vec<String>` format
pub(crate) fn deserialize_mcp_server_access<'de, D>(
    deserializer: D,
) -> Result<McpServerAccess, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::{self, Visitor};

    struct McpServerAccessVisitor;

    impl<'de> Visitor<'de> for McpServerAccessVisitor {
        type Value = McpServerAccess;

        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str("'none', 'all', or an object with 'specific' key containing server IDs, or legacy array of server IDs")
        }

        fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
            match v {
                "none" => Ok(McpServerAccess::None),
                "all" => Ok(McpServerAccess::All),
                _ => Err(E::custom(format!("unknown variant: {}", v))),
            }
        }

        fn visit_seq<A: de::SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
            // Legacy format: array of server IDs
            let mut servers = Vec::new();
            while let Some(server) = seq.next_element::<String>()? {
                servers.push(server);
            }
            if servers.is_empty() {
                // Empty array in old format meant "no access"
                Ok(McpServerAccess::None)
            } else {
                Ok(McpServerAccess::Specific(servers))
            }
        }

        fn visit_map<A: de::MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
            // New format: { "specific": [...] }
            let mut specific: Option<Vec<String>> = None;
            while let Some(key) = map.next_key::<String>()? {
                match key.as_str() {
                    "specific" | "Specific" => {
                        specific = Some(map.next_value()?);
                    }
                    _ => {
                        let _: serde::de::IgnoredAny = map.next_value()?;
                    }
                }
            }
            match specific {
                Some(servers) => Ok(McpServerAccess::Specific(servers)),
                None => Err(de::Error::custom("expected 'specific' key in map")),
            }
        }

        fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
            Ok(McpServerAccess::None)
        }
    }

    deserializer.deserialize_any(McpServerAccessVisitor)
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            version: CONFIG_VERSION,
            server: ServerConfig::default(),
            providers: Vec::new(), // Empty by default, discovered on first startup
            logging: LoggingConfig::default(),
            oauth_clients: Vec::new(),
            mcp_servers: Vec::new(),
            clients: Vec::new(),
            strategies: Vec::new(),
            pricing_overrides: std::collections::HashMap::new(),
            ui: UiConfig::default(),
            routellm_settings: RouteLLMGlobalSettings::default(),
            update: UpdateConfig::default(),
            model_cache: ModelCacheConfig::default(),
            roots: Vec::new(),
            streaming: StreamingConfig::default(),
            setup_wizard_shown: false,
            health_check: HealthCheckConfig::default(),
            skills: SkillsConfig::default(),
            marketplace: MarketplaceConfig::default(),
            guardrails: GuardrailsConfig::default(),
            coding_agents: CodingAgentsConfig::default(),
            context_management: ContextManagementConfig::default(),
            prompt_compression: PromptCompressionConfig::default(),
            json_repair: JsonRepairConfig::default(),
            mcp_via_llm: McpViaLlmConfig::default(),
            secret_scanning: SecretScanningConfig::default(),
            memory: MemoryConfig::default(),
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
            enable_access_log: false,
            log_dir: None,
            retention_days: 31,
        }
    }
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            tray_graph_enabled: false, // Static icon by default; user can enable activity graph
            tray_graph_refresh_rate_secs: default_tray_graph_refresh_rate(),
            sidebar_expanded: default_sidebar_expanded(),
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
            free_tier: None,
        }
    }

    /// Create default LM Studio provider configuration
    pub fn default_lmstudio() -> Self {
        Self {
            name: "LM Studio".to_string(),
            provider_type: ProviderType::LMStudio,
            enabled: true,
            provider_config: Some(serde_json::json!({
                "base_url": "http://localhost:1234/v1"
            })),
            api_key_ref: None,
            free_tier: None,
        }
    }

    /// Create default Jan provider configuration
    pub fn default_jan() -> Self {
        Self {
            name: "Jan".to_string(),
            provider_type: ProviderType::Jan,
            enabled: true,
            provider_config: Some(serde_json::json!({
                "base_url": "http://localhost:1337/v1"
            })),
            api_key_ref: None,
            free_tier: None,
        }
    }

    /// Create default GPT4All provider configuration
    pub fn default_gpt4all() -> Self {
        Self {
            name: "GPT4All".to_string(),
            provider_type: ProviderType::GPT4All,
            enabled: true,
            provider_config: Some(serde_json::json!({
                "base_url": "http://localhost:4891/v1"
            })),
            api_key_ref: None,
            free_tier: None,
        }
    }

    /// Create default LocalAI provider configuration
    pub fn default_localai() -> Self {
        Self {
            name: "LocalAI".to_string(),
            provider_type: ProviderType::LocalAI,
            enabled: true,
            provider_config: Some(serde_json::json!({
                "base_url": "http://localhost:8080/v1"
            })),
            api_key_ref: None,
            free_tier: None,
        }
    }

    /// Create default llama.cpp provider configuration
    pub fn default_llamacpp() -> Self {
        Self {
            name: "llama.cpp".to_string(),
            provider_type: ProviderType::LlamaCpp,
            enabled: true,
            provider_config: Some(serde_json::json!({
                "base_url": "http://localhost:8080/v1"
            })),
            api_key_ref: None,
            free_tier: None,
        }
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
    /// Create a new client with auto-generated client_id and explicit strategy
    /// The secret must be stored separately in the keychain
    pub fn new_with_strategy(name: String, strategy_id: String) -> Self {
        let id = Uuid::new_v4().to_string();
        Self {
            id,
            name,
            enabled: true,
            allowed_llm_providers: Vec::new(),
            mcp_server_access: McpServerAccess::None,
            context_management_enabled: None,
            catalog_compression_enabled: None,
            client_tools_indexing: None,
            skills_access: SkillsAccess::None,
            created_at: Utc::now(),
            last_used: None,
            strategy_id,
            roots: None,
            mcp_sampling_permission: PermissionState::Ask,
            mcp_elicitation_permission: PermissionState::Ask,
            mcp_sampling_max_tokens: None,
            mcp_sampling_rate_limit: None,
            mcp_sampling_enabled: false,
            mcp_sampling_requires_approval: true,
            firewall: FirewallRules::default(),
            marketplace_enabled: false,
            mcp_permissions: McpPermissions::default(),
            skills_permissions: SkillsPermissions::default(),
            model_permissions: ModelPermissions::default(),
            marketplace_permission: PermissionState::default(),
            client_mode: ClientMode::default(),
            template_id: None,
            sync_config: false,
            guardrails_enabled: None,
            guardrails: ClientGuardrailsConfig::default(),
            prompt_compression: ClientPromptCompressionConfig::default(),
            json_repair: ClientJsonRepairConfig::default(),
            secret_scanning: ClientSecretScanningConfig::default(),
            coding_agents_permissions: CodingAgentsPermissions::default(),
            coding_agent_permission: PermissionState::default(),
            coding_agent_type: None,
            memory_enabled: None,
        }
    }

    /// Resolve whether context management is enabled for this client.
    /// Checks per-client override first, then falls back to global config.
    pub fn is_context_management_enabled(&self, global: &ContextManagementConfig) -> bool {
        // Per-client override takes precedence
        if let Some(enabled) = self.context_management_enabled {
            return enabled;
        }
        // Fall back to global setting (derived from indexing config)
        global.is_enabled()
    }

    /// Resolve whether catalog compression is enabled for this client.
    /// Checks per-client override first, then falls back to global config.
    /// Only effective when context management is also enabled.
    pub fn is_catalog_compression_enabled(&self, global: &ContextManagementConfig) -> bool {
        if let Some(enabled) = self.catalog_compression_enabled {
            return enabled;
        }
        global.catalog_compression
    }

    /// Resolve the indexing state for a specific client tool.
    /// Checks per-client overrides first, then falls back to global default.
    pub fn resolve_client_tool_indexing<'a>(
        &'a self,
        tool_name: &str,
        global: &'a ContextManagementConfig,
    ) -> &'a IndexingState {
        if let Some(ref perms) = self.client_tools_indexing {
            return perms.resolve_tool(tool_name, &global.client_tools_indexing_default);
        }
        &global.client_tools_indexing_default
    }

    /// Whether a client tool's output should be indexed.
    pub fn is_client_tool_indexing_eligible(
        &self,
        tool_name: &str,
        global: &ContextManagementConfig,
    ) -> bool {
        self.resolve_client_tool_indexing(tool_name, global)
            .is_enabled()
    }

    /// Update last used timestamp
    pub fn mark_used(&mut self) {
        self.last_used = Some(Utc::now());
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

fn default_sidebar_expanded() -> bool {
    true // Expanded by default on fresh install
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
        assert_eq!(config.providers.len(), 0);
        // Strategies are empty by default (created on-demand for clients)
        assert!(config.strategies.is_empty());
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
        assert!(!logging.enable_access_log); // Disabled by default
        assert_eq!(logging.retention_days, 31);
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
        let config = AppConfig {
            roots: vec![
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
            ],
            ..Default::default()
        };

        let yaml = serde_yaml::to_string(&config).unwrap();
        let deserialized: AppConfig = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(config.roots.len(), 2);
        assert_eq!(deserialized.roots.len(), 2);
        assert_eq!(deserialized.roots[0].uri, "file:///Users/test/projects");
        assert_eq!(deserialized.roots[1].name, None);
    }

    #[test]
    fn test_client_with_roots_override() {
        let mut client =
            Client::new_with_strategy("Test Client".to_string(), "test-strategy".to_string());
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
        let client =
            Client::new_with_strategy("Test Client".to_string(), "test-strategy".to_string());

        // Sampling defaults to Ask
        assert_eq!(client.mcp_sampling_permission, PermissionState::Ask);

        // Elicitation defaults to Ask
        assert_eq!(client.mcp_elicitation_permission, PermissionState::Ask);

        // No limits by default
        assert!(client.mcp_sampling_max_tokens.is_none());
        assert!(client.mcp_sampling_rate_limit.is_none());
    }

    #[test]
    fn test_client_with_sampling_permission() {
        let mut client =
            Client::new_with_strategy("Test Client".to_string(), "test-strategy".to_string());
        client.mcp_sampling_permission = PermissionState::Allow;
        client.mcp_elicitation_permission = PermissionState::Off;
        client.mcp_sampling_max_tokens = Some(2000);
        client.mcp_sampling_rate_limit = Some(100);

        // Verify serialization
        let yaml = serde_yaml::to_string(&client).unwrap();
        let deserialized: Client = serde_yaml::from_str(&yaml).unwrap();

        assert_eq!(deserialized.mcp_sampling_permission, PermissionState::Allow);
        assert_eq!(
            deserialized.mcp_elicitation_permission,
            PermissionState::Off
        );
        assert_eq!(deserialized.mcp_sampling_max_tokens, Some(2000));
        assert_eq!(deserialized.mcp_sampling_rate_limit, Some(100));
    }

    // === AvailableModelsSelection tests ===

    #[test]
    fn test_available_models_all() {
        let selection = AvailableModelsSelection::all();
        assert!(selection.is_model_allowed("OpenAI", "gpt-4"));
        assert!(selection.is_model_allowed("Anthropic", "claude-3"));
        assert!(selection.is_model_allowed("Unknown", "any-model"));
    }

    #[test]
    fn test_available_models_none() {
        let selection = AvailableModelsSelection::none();
        assert!(!selection.is_model_allowed("OpenAI", "gpt-4"));
        assert!(!selection.is_model_allowed("Anthropic", "claude-3"));
    }

    #[test]
    fn test_available_models_by_provider() {
        let selection = AvailableModelsSelection {
            selected_all: false,
            selected_providers: vec!["OpenAI".to_string()],
            selected_models: vec![],
        };
        assert!(selection.is_model_allowed("OpenAI", "gpt-4"));
        assert!(selection.is_model_allowed("OpenAI", "gpt-3.5-turbo"));
        assert!(!selection.is_model_allowed("Anthropic", "claude-3"));
    }

    #[test]
    fn test_available_models_by_model() {
        let selection = AvailableModelsSelection {
            selected_all: false,
            selected_providers: vec![],
            selected_models: vec![("OpenAI".to_string(), "gpt-4".to_string())],
        };
        assert!(selection.is_model_allowed("OpenAI", "gpt-4"));
        assert!(!selection.is_model_allowed("OpenAI", "gpt-3.5-turbo"));
        assert!(!selection.is_model_allowed("Anthropic", "claude-3"));
    }

    #[test]
    fn test_available_models_case_insensitive() {
        let selection = AvailableModelsSelection {
            selected_all: false,
            selected_providers: vec!["OpenAI".to_string()],
            selected_models: vec![("Anthropic".to_string(), "Claude-3".to_string())],
        };
        // Provider match is case-insensitive
        assert!(selection.is_model_allowed("openai", "gpt-4"));
        assert!(selection.is_model_allowed("OPENAI", "gpt-4"));
        // Model match is case-insensitive
        assert!(selection.is_model_allowed("anthropic", "claude-3"));
        assert!(selection.is_model_allowed("ANTHROPIC", "CLAUDE-3"));
    }

    // ── Context Management config resolution tests ──────────────────

    fn make_client_with_overrides(cm: Option<bool>) -> Client {
        let mut client = Client::new_with_strategy("test".to_string(), "strat-1".to_string());
        client.context_management_enabled = cm;
        client
    }

    #[test]
    fn test_context_management_falls_back_to_global() {
        // Default config has indexing enabled → is_enabled() = true
        let enabled_config = ContextManagementConfig::default();
        let client = make_client_with_overrides(None);
        assert!(client.is_context_management_enabled(&enabled_config));

        // Config with all indexing disabled → is_enabled() = false
        let disabled_config = ContextManagementConfig {
            gateway_indexing: GatewayIndexingPermissions {
                global: IndexingState::Disable,
                ..Default::default()
            },
            virtual_indexing: GatewayIndexingPermissions {
                global: IndexingState::Disable,
                ..Default::default()
            },
            client_tools_indexing_default: IndexingState::Disable,
            ..Default::default()
        };
        assert!(!client.is_context_management_enabled(&disabled_config));
    }

    #[test]
    fn test_context_management_client_override_wins() {
        // Client explicitly disables even when global indexing is on
        let enabled_config = ContextManagementConfig::default();
        let client = make_client_with_overrides(Some(false));
        assert!(!client.is_context_management_enabled(&enabled_config));

        // Client explicitly enables even when global indexing is off
        let disabled_config = ContextManagementConfig {
            gateway_indexing: GatewayIndexingPermissions {
                global: IndexingState::Disable,
                ..Default::default()
            },
            virtual_indexing: GatewayIndexingPermissions {
                global: IndexingState::Disable,
                ..Default::default()
            },
            client_tools_indexing_default: IndexingState::Disable,
            ..Default::default()
        };
        let client = make_client_with_overrides(Some(true));
        assert!(client.is_context_management_enabled(&disabled_config));
    }

    #[test]
    fn test_context_management_config_serialization() {
        let config = ContextManagementConfig {
            catalog_threshold_bytes: 2000,
            response_threshold_bytes: 500,
            ..Default::default()
        };
        let yaml = serde_yaml::to_string(&config).unwrap();
        let deserialized: ContextManagementConfig = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(
            config.catalog_threshold_bytes,
            deserialized.catalog_threshold_bytes
        );
        assert_eq!(
            config.response_threshold_bytes,
            deserialized.response_threshold_bytes
        );
    }

    #[test]
    fn test_context_management_is_enabled_derived_from_indexing() {
        // Default: gateway_indexing.global=Enable, client_tools_indexing_default=Enable
        let config = ContextManagementConfig::default();
        assert!(config.is_enabled());

        // All indexing disabled
        let disabled = ContextManagementConfig {
            gateway_indexing: GatewayIndexingPermissions {
                global: IndexingState::Disable,
                ..Default::default()
            },
            virtual_indexing: GatewayIndexingPermissions {
                global: IndexingState::Disable,
                ..Default::default()
            },
            client_tools_indexing_default: IndexingState::Disable,
            ..Default::default()
        };
        assert!(!disabled.is_enabled());

        // Only client tools indexing enabled
        let client_only = ContextManagementConfig {
            gateway_indexing: GatewayIndexingPermissions {
                global: IndexingState::Disable,
                ..Default::default()
            },
            virtual_indexing: GatewayIndexingPermissions {
                global: IndexingState::Disable,
                ..Default::default()
            },
            client_tools_indexing_default: IndexingState::Enable,
            ..Default::default()
        };
        assert!(client_only.is_enabled());

        // Only a server-level override enabled
        let mut server_only = ContextManagementConfig {
            gateway_indexing: GatewayIndexingPermissions {
                global: IndexingState::Disable,
                ..Default::default()
            },
            client_tools_indexing_default: IndexingState::Disable,
            ..Default::default()
        };
        server_only
            .gateway_indexing
            .servers
            .insert("fs".to_string(), IndexingState::Enable);
        assert!(server_only.is_enabled());
    }

    #[test]
    fn test_catalog_compression_falls_back_to_global() {
        let global = ContextManagementConfig {
            catalog_compression: true,
            ..Default::default()
        };
        let client = make_client_with_overrides(None);
        assert!(client.is_catalog_compression_enabled(&global));

        let global_no_comp = ContextManagementConfig {
            catalog_compression: false,
            ..Default::default()
        };
        assert!(!client.is_catalog_compression_enabled(&global_no_comp));
    }

    #[test]
    fn test_catalog_compression_client_override_wins() {
        let global = ContextManagementConfig {
            catalog_compression: true,
            ..Default::default()
        };
        // Client explicitly disables
        let mut client = make_client_with_overrides(None);
        client.catalog_compression_enabled = Some(false);
        assert!(!client.is_catalog_compression_enabled(&global));

        // Client explicitly enables even when global is off
        let global_no_comp = ContextManagementConfig {
            catalog_compression: false,
            ..Default::default()
        };
        let mut client = make_client_with_overrides(None);
        client.catalog_compression_enabled = Some(true);
        assert!(client.is_catalog_compression_enabled(&global_no_comp));
    }

    #[test]
    fn test_context_management_config_defaults_catalog_compression_true() {
        // Deserialize empty YAML — catalog_compression should default to true
        let yaml = "catalog_threshold_bytes: 1000\n";
        let config: ContextManagementConfig = serde_yaml::from_str(yaml).unwrap();
        assert!(config.catalog_compression);
    }

    // --- Gateway Indexing Permission Resolution Tests ---

    #[test]
    fn test_gateway_indexing_global_enable_no_overrides() {
        let perms = GatewayIndexingPermissions::default(); // global=Enable
        assert!(perms.is_tool_eligible("filesystem", "read_file"));
        assert!(perms.is_server_eligible("filesystem"));
    }

    #[test]
    fn test_gateway_indexing_server_disable_overrides_global() {
        let mut perms = GatewayIndexingPermissions::default();
        perms
            .servers
            .insert("filesystem".to_string(), IndexingState::Disable);
        assert!(!perms.is_tool_eligible("filesystem", "read_file"));
        assert!(!perms.is_server_eligible("filesystem"));
        // Other servers still inherit global
        assert!(perms.is_tool_eligible("github", "search"));
    }

    #[test]
    fn test_gateway_indexing_tool_overrides_server() {
        let mut perms = GatewayIndexingPermissions::default();
        perms
            .servers
            .insert("filesystem".to_string(), IndexingState::Disable);
        perms
            .tools
            .insert("filesystem__read_file".to_string(), IndexingState::Enable);
        // Tool override wins over server
        assert!(perms.is_tool_eligible("filesystem", "read_file"));
        // Other tools in server still disabled
        assert!(!perms.is_tool_eligible("filesystem", "write_file"));
    }

    #[test]
    fn test_gateway_indexing_global_disable() {
        let perms = GatewayIndexingPermissions {
            global: IndexingState::Disable,
            ..Default::default()
        };
        assert!(!perms.is_tool_eligible("any_server", "any_tool"));
        assert!(!perms.is_server_eligible("any_server"));
    }

    #[test]
    fn test_gateway_indexing_tool_not_in_map_inherits() {
        let mut perms = GatewayIndexingPermissions::default();
        perms
            .servers
            .insert("filesystem".to_string(), IndexingState::Disable);
        // Tool not in map → inherits from server → Disable
        assert!(!perms.is_tool_eligible("filesystem", "unknown_tool"));
        // Tool in different server not in map → inherits from global → Enable
        assert!(perms.is_tool_eligible("other", "some_tool"));
    }

    // --- Client Tools Indexing Permission Resolution Tests ---

    #[test]
    fn test_client_tools_global_default_enable_no_override() {
        let config = ContextManagementConfig::default(); // client_tools_indexing_default=Enable
        let client = Client::new_with_strategy("test".to_string(), "s".to_string());
        assert!(client.is_client_tool_indexing_eligible("Read", &config));
    }

    #[test]
    fn test_client_tools_client_global_override_disable() {
        let config = ContextManagementConfig::default();
        let mut client = Client::new_with_strategy("test".to_string(), "s".to_string());
        client.client_tools_indexing = Some(ClientToolsIndexingPermissions {
            global: Some(IndexingState::Disable),
            ..Default::default()
        });
        assert!(!client.is_client_tool_indexing_eligible("Read", &config));
    }

    #[test]
    fn test_client_tools_per_tool_disable() {
        let config = ContextManagementConfig::default();
        let mut client = Client::new_with_strategy("test".to_string(), "s".to_string());
        let mut tools = HashMap::new();
        tools.insert("Write".to_string(), IndexingState::Disable);
        client.client_tools_indexing = Some(ClientToolsIndexingPermissions {
            global: None,
            tools,
        });
        assert!(!client.is_client_tool_indexing_eligible("Write", &config));
        // Other tools inherit global default
        assert!(client.is_client_tool_indexing_eligible("Read", &config));
    }

    #[test]
    fn test_client_tools_per_tool_enable_overrides_global_disable() {
        let config = ContextManagementConfig {
            client_tools_indexing_default: IndexingState::Disable,
            ..ContextManagementConfig::default()
        };
        let mut client = Client::new_with_strategy("test".to_string(), "s".to_string());
        let mut tools = HashMap::new();
        tools.insert("Read".to_string(), IndexingState::Enable);
        client.client_tools_indexing = Some(ClientToolsIndexingPermissions {
            global: None,
            tools,
        });
        assert!(client.is_client_tool_indexing_eligible("Read", &config));
        // Other tools inherit global default → Disable
        assert!(!client.is_client_tool_indexing_eligible("Write", &config));
    }

    #[test]
    fn test_client_tools_no_client_override_inherits_global() {
        let config = ContextManagementConfig {
            client_tools_indexing_default: IndexingState::Disable,
            ..ContextManagementConfig::default()
        };
        let client = Client::new_with_strategy("test".to_string(), "s".to_string());
        // No client_tools_indexing → inherits global default
        assert!(!client.is_client_tool_indexing_eligible("Read", &config));
    }

    #[test]
    fn test_context_management_config_tool_name_defaults() {
        let config = ContextManagementConfig::default();
        assert_eq!(config.search_tool_name, "IndexSearch");
        assert_eq!(config.read_tool_name, "IndexRead");
    }

    #[test]
    fn test_context_management_config_tool_names_deserialize() {
        let yaml = r#"
search_tool_name: "ctx_search"
read_tool_name: "ctx_read"
"#;
        let config: ContextManagementConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.search_tool_name, "ctx_search");
        assert_eq!(config.read_tool_name, "ctx_read");
    }
}
