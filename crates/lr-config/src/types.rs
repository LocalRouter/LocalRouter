use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

pub(crate) const CONFIG_VERSION: u32 = 5;

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

/// Serializer for SkillsAccess that produces YAML-friendly format
pub(crate) fn serialize_skills_access<S>(
    access: &SkillsAccess,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    match access {
        SkillsAccess::None => serializer.serialize_str("none"),
        SkillsAccess::All => serializer.serialize_str("all"),
        SkillsAccess::Specific(skills) => {
            use serde::ser::SerializeMap;
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry("specific", skills)?;
            map.end()
        }
    }
}

/// Deserializer for SkillsAccess
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

    /// MCP server access control
    /// - None: No MCP access (default)
    /// - All: Access to all configured MCP servers
    /// - Specific: Access only to listed server IDs
    #[serde(
        default,
        deserialize_with = "deserialize_mcp_server_access",
        serialize_with = "serialize_mcp_server_access"
    )]
    pub mcp_server_access: McpServerAccess,

    /// Enable deferred loading for MCP tools (default: false)
    /// When enabled, only a search tool is initially visible. Tools are activated on-demand
    /// through search queries, dramatically reducing token consumption for large catalogs.
    #[serde(default)]
    pub mcp_deferred_loading: bool,

    /// Skills access control (by skill name)
    /// - None: No skills access (default)
    /// - All: Access to all discovered skills
    /// - Specific: Access only to specific skills by name
    #[serde(
        default,
        deserialize_with = "deserialize_skills_access",
        serialize_with = "serialize_skills_access"
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

    /// Whether this client has marketplace access (search + install tools)
    /// When this is set to true, the global marketplace.enabled is also auto-set to true
    #[serde(default)]
    pub marketplace_enabled: bool,
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

/// Serializer for McpServerAccess that produces YAML-friendly format
/// Matches the format expected by deserialize_mcp_server_access
pub(crate) fn serialize_mcp_server_access<S>(
    access: &McpServerAccess,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    match access {
        McpServerAccess::None => serializer.serialize_str("none"),
        McpServerAccess::All => serializer.serialize_str("all"),
        McpServerAccess::Specific(servers) => {
            use serde::ser::SerializeMap;
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_entry("specific", servers)?;
            map.end()
        }
    }
}

/// Deserializer for McpServerAccess that supports backward compatibility
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
            tray_graph_enabled: true, // Always enabled (dynamic tray icon graph)
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
        self.enabled && self.mcp_server_access.can_access(server_id)
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
    /// If mode is None, converts to Specific with this server
    /// If mode is All, no change needed
    /// If mode is Specific, adds to the list if not present
    pub fn add_mcp_server(&mut self, server_id: String) {
        match &mut self.mcp_server_access {
            McpServerAccess::None => {
                self.mcp_server_access = McpServerAccess::Specific(vec![server_id]);
            }
            McpServerAccess::All => {
                // Already has access to all, no change needed
            }
            McpServerAccess::Specific(servers) => {
                if !servers.contains(&server_id) {
                    servers.push(server_id);
                }
            }
        }
    }

    /// Remove MCP server access
    /// If mode is None, no change
    /// If mode is All, cannot remove individual servers (caller should set to Specific first)
    /// If mode is Specific, removes from the list and converts to None if empty
    pub fn remove_mcp_server(&mut self, server_id: &str) -> bool {
        match &mut self.mcp_server_access {
            McpServerAccess::None => false,
            McpServerAccess::All => false, // Can't remove from "All" mode
            McpServerAccess::Specific(servers) => {
                if let Some(pos) = servers.iter().position(|s| s == server_id) {
                    servers.remove(pos);
                    if servers.is_empty() {
                        self.mcp_server_access = McpServerAccess::None;
                    }
                    true
                } else {
                    false
                }
            }
        }
    }

    /// Set MCP server access mode
    pub fn set_mcp_server_access(&mut self, access: McpServerAccess) {
        self.mcp_server_access = access;
    }

    /// Get MCP server access mode
    pub fn mcp_server_access(&self) -> &McpServerAccess {
        &self.mcp_server_access
    }

    /// Check if this client can access a specific skill by name
    pub fn can_access_skill(&self, skill_name: &str) -> bool {
        self.enabled && self.skills_access.can_access_by_name(skill_name)
    }

    /// Set skills access mode
    pub fn set_skills_access(&mut self, access: SkillsAccess) {
        self.skills_access = access;
    }

    /// Get skills access mode
    pub fn skills_access(&self) -> &SkillsAccess {
        &self.skills_access
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
