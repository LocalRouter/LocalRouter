use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

pub(crate) const CONFIG_VERSION: u32 = 13;

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
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AutoModelConfig {
    /// Whether auto-routing is enabled
    pub enabled: bool,
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
            name: client_strategy_name(&client_name),
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
    /// Interval between health checks (in seconds)
    /// Default: 600 (10 minutes)
    #[serde(default = "default_health_check_interval")]
    pub interval_secs: u64,
    /// Timeout for each health check (in seconds)
    /// Default: 5 seconds
    #[serde(default = "default_health_check_timeout")]
    pub timeout_secs: u64,
}

fn default_health_check_interval() -> u64 {
    600 // 10 minutes
}

fn default_health_check_timeout() -> u64 {
    5 // 5 seconds
}

impl Default for HealthCheckConfig {
    fn default() -> Self {
        Self {
            mode: HealthCheckMode::default(),
            interval_secs: default_health_check_interval(),
            timeout_secs: default_health_check_timeout(),
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
    5 // 5 seconds
}

impl Default for ModelCacheConfig {
    fn default() -> Self {
        Self {
            default_ttl_seconds: 5,
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

/// Skills configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
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

    /// Migration shim: old auto_scan_directories (deserialize only)
    #[serde(default, skip_serializing)]
    pub auto_scan_directories: Vec<String>,

    /// Migration shim: old skill_paths (deserialize only)
    #[serde(default, skip_serializing)]
    pub skill_paths: Vec<String>,
}

/// Marketplace configuration for MCP server and skill discovery
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MarketplaceConfig {
    /// Whether marketplace is enabled globally
    #[serde(default)]
    pub enabled: bool,

    /// MCP server registry URL
    #[serde(default = "default_marketplace_registry_url")]
    pub registry_url: String,

    /// Skill source repos to browse
    #[serde(default = "default_marketplace_skill_sources")]
    pub skill_sources: Vec<MarketplaceSkillSource>,
}

impl Default for MarketplaceConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            registry_url: default_marketplace_registry_url(),
            skill_sources: default_marketplace_skill_sources(),
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
    /// Use per-client guardrails.enabled instead
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

    /// Migration shim: old global category_actions (deserialize only, not serialized)
    /// Use per-client guardrails.category_actions instead
    #[serde(default, skip_serializing)]
    pub category_actions: Vec<CategoryActionEntry>,

    /// Global HuggingFace token for gated model downloads
    #[serde(default)]
    pub hf_token: Option<String>,

    /// Default confidence threshold for flagging (0.0-1.0)
    #[serde(default = "default_confidence_threshold")]
    pub default_confidence_threshold: f32,

    /// Idle timeout in seconds before auto-unloading GGUF models from memory.
    /// Default: 600 (10 minutes). Set to 0 to disable auto-unload.
    #[serde(default = "default_guardrails_idle_timeout")]
    pub idle_timeout_secs: u64,

    /// Context window size for local GGUF inference (in tokens).
    /// Larger values support longer input but use more GPU memory per inference.
    /// Default: 512. Range: 256-4096.
    #[serde(default = "default_guardrails_context_size")]
    pub context_size: u32,
}

fn default_confidence_threshold() -> f32 {
    0.5
}

fn default_guardrails_idle_timeout() -> u64 {
    600 // 10 minutes
}

fn default_guardrails_context_size() -> u32 {
    512
}

impl Default for GuardrailsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            scan_requests: true,
            scan_responses: false,
            safety_models: default_safety_models(),
            category_actions: vec![],
            hf_token: None,
            default_confidence_threshold: default_confidence_threshold(),
            idle_timeout_secs: default_guardrails_idle_timeout(),
            context_size: default_guardrails_context_size(),
        }
    }
}

/// Per-client guardrails configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ClientGuardrailsConfig {
    /// Whether guardrails are enabled for this client
    #[serde(default)]
    pub enabled: bool,
    /// Per-category actions: allow/notify/ask/block
    /// Categories are selected here; which models run is derived from which categories are selected
    #[serde(default)]
    pub category_actions: Vec<CategoryActionEntry>,
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
    /// Whether this model is enabled
    #[serde(default)]
    pub enabled: bool,
    /// Use existing provider (e.g. "ollama", "openrouter")
    #[serde(default)]
    pub provider_id: Option<String>,
    /// Model name on the provider (e.g. "granite3-guardian:2b")
    #[serde(default)]
    pub model_name: Option<String>,
    /// HuggingFace repo ID for direct download
    #[serde(default)]
    pub hf_repo_id: Option<String>,
    /// Specific GGUF filename to download
    #[serde(default)]
    pub gguf_filename: Option<String>,
    /// Whether this model requires HuggingFace authentication (gated)
    #[serde(default)]
    pub requires_auth: bool,
    /// Override the global confidence threshold for this model
    #[serde(default)]
    pub confidence_threshold: Option<f32>,
    /// Subset of categories to enable (None = all)
    #[serde(default)]
    pub enabled_categories: Option<Vec<String>>,
    /// Whether this is a predefined (built-in) model entry
    #[serde(default)]
    pub predefined: bool,
    /// Execution mode: "provider" or "local" (default: "provider")
    #[serde(default)]
    pub execution_mode: Option<String>,
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
    /// Estimated memory usage in MB when loaded
    #[serde(default)]
    pub memory_mb: Option<u32>,
    /// Estimated inference latency in milliseconds
    #[serde(default)]
    pub latency_ms: Option<u32>,
    /// On-disk size in MB
    #[serde(default)]
    pub disk_size_mb: Option<u32>,
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
    "ask".to_string()
}

/// Default safety models (all disabled, predefined, with direct_download GGUF info)
fn default_safety_models() -> Vec<SafetyModelConfig> {
    vec![
        SafetyModelConfig {
            id: "llama_guard".to_string(),
            label: "Llama Guard".to_string(),
            model_type: "llama_guard".to_string(),
            enabled: false,
            provider_id: Some("ollama".to_string()),
            model_name: Some("llama-guard3:1b".to_string()),
            hf_repo_id: Some("QuantFactory/Llama-Guard-3-1B-GGUF".to_string()),
            gguf_filename: Some("Llama-Guard-3-1B.Q4_K_M.gguf".to_string()),
            requires_auth: false,
            confidence_threshold: None,
            enabled_categories: None,
            predefined: true,
            execution_mode: Some("direct_download".to_string()),
            prompt_template: None,
            safe_indicator: None,
            output_regex: None,
            category_mapping: None,
            memory_mb: Some(700),
            latency_ms: Some(300),
            disk_size_mb: Some(955),
        },
        SafetyModelConfig {
            id: "granite_guardian".to_string(),
            label: "Granite Guardian".to_string(),
            model_type: "granite_guardian".to_string(),
            enabled: false,
            provider_id: Some("ollama".to_string()),
            model_name: Some("granite3-guardian:2b".to_string()),
            hf_repo_id: Some("mradermacher/granite-guardian-3.0-2b-GGUF".to_string()),
            gguf_filename: Some("granite-guardian-3.0-2b.Q4_K_M.gguf".to_string()),
            requires_auth: false,
            confidence_threshold: None,
            enabled_categories: None,
            predefined: true,
            execution_mode: Some("direct_download".to_string()),
            prompt_template: None,
            safe_indicator: None,
            output_regex: None,
            category_mapping: None,
            memory_mb: Some(1200),
            latency_ms: Some(500),
            disk_size_mb: Some(1500),
        },
        SafetyModelConfig {
            id: "shield_gemma".to_string(),
            label: "ShieldGemma".to_string(),
            model_type: "shield_gemma".to_string(),
            enabled: false,
            provider_id: Some("ollama".to_string()),
            model_name: Some("shieldgemma:2b".to_string()),
            hf_repo_id: Some("QuantFactory/shieldgemma-2b-GGUF".to_string()),
            gguf_filename: Some("shieldgemma-2b.Q4_K_M.gguf".to_string()),
            requires_auth: false,
            confidence_threshold: None,
            enabled_categories: None,
            predefined: true,
            execution_mode: Some("direct_download".to_string()),
            prompt_template: None,
            safe_indicator: None,
            output_regex: None,
            category_mapping: None,
            memory_mb: Some(1200),
            latency_ms: Some(400),
            disk_size_mb: Some(1700),
        },
        SafetyModelConfig {
            id: "nemotron_safety_guard".to_string(),
            label: "Nemotron Safety Guard".to_string(),
            model_type: "nemotron".to_string(),
            enabled: false,
            provider_id: Some("ollama".to_string()),
            model_name: Some("llama-3.1-nemotron-safety-guard:8b".to_string()),
            hf_repo_id: Some(
                "AXONVERTEX-AI-RESEARCH/Llama-3.1-Nemotron-Safety-Guard-8B-v3-Q8_0-GGUF"
                    .to_string(),
            ),
            gguf_filename: Some("llama-3.1-nemotron-safety-guard-8b-v3-q8_0.gguf".to_string()),
            requires_auth: false,
            confidence_threshold: None,
            enabled_categories: None,
            predefined: true,
            execution_mode: Some("direct_download".to_string()),
            prompt_template: None,
            safe_indicator: None,
            output_regex: None,
            category_mapping: None,
            memory_mb: Some(5000),
            latency_ms: Some(800),
            disk_size_mb: Some(8500),
        },
    ]
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

    /// Enable deferred loading for MCP tools (default: false)
    /// When enabled, only a search tool is initially visible. Tools are activated on-demand
    /// through search queries, dramatically reducing token consumption for large catalogs.
    #[serde(default)]
    pub mcp_deferred_loading: bool,

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
    /// Local LM Studio instance
    #[serde(rename = "lmstudio")]
    LMStudio,
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
            mcp_deferred_loading: false,
            skills_access: SkillsAccess::None,
            created_at: Utc::now(),
            last_used: None,
            strategy_id,
            roots: None,
            mcp_sampling_enabled: false,
            mcp_sampling_requires_approval: true,
            mcp_sampling_max_tokens: None,
            mcp_sampling_rate_limit: None,
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
        }
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
        let mut client =
            Client::new_with_strategy("Test Client".to_string(), "test-strategy".to_string());
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
