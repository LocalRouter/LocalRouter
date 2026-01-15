//! Configuration management module
//!
//! Handles loading, saving, and managing application configuration.
//! Supports file watching and event emission for real-time config updates.

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

const CONFIG_VERSION: u32 = 1;

/// Main application configuration structure
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AppConfig {
    /// Configuration schema version for migrations
    #[serde(default = "default_version")]
    pub version: u32,

    /// Server configuration
    #[serde(default)]
    pub server: ServerConfig,

    /// API keys configuration
    #[serde(default)]
    pub api_keys: Vec<ApiKeyConfig>,

    /// Router configurations
    #[serde(default)]
    pub routers: Vec<RouterConfig>,

    /// Provider configurations
    #[serde(default)]
    pub providers: Vec<ProviderConfig>,

    /// Logging configuration
    #[serde(default)]
    pub logging: LoggingConfig,
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

/// API key configuration
///
/// The actual API key is stored in the OS keychain.
/// This struct contains only metadata.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ApiKeyConfig {
    /// Unique identifier (also used as keyring username)
    pub id: String,

    /// Human-readable name
    pub name: String,

    /// Model selection for this key
    pub model_selection: ModelSelection,

    /// Whether the key is enabled
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Creation timestamp
    pub created_at: DateTime<Utc>,

    /// Last used timestamp
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_used: Option<DateTime<Utc>>,
}

/// Model selection type
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ModelSelection {
    /// Direct model selection
    DirectModel {
        /// Provider name
        provider: String,
        /// Model identifier
        model: String,
    },
    /// Router-based selection
    Router {
        /// Router name
        router_name: String,
    },
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

        let mut watcher = notify::recommended_watcher(move |result: Result<Event, notify::Error>| {
            match result {
                Ok(event) => {
                    // Only respond to modify events
                    if matches!(event.kind, EventKind::Modify(_)) {
                        info!("Configuration file changed, reloading...");

                        // Reload config from disk (blocking operation in event handler)
                        let config_path_clone = config_path.clone();
                        let config_arc_clone = config_arc.clone();
                        let app_handle_clone = app_handle.clone();

                        tokio::spawn(async move {
                            match load_config(&config_path_clone).await {
                                Ok(new_config) => {
                                    // Update in-memory config
                                    *config_arc_clone.write() = new_config.clone();

                                    info!("Configuration reloaded successfully");

                                    // Emit event to frontend
                                    if let Some(handle) = app_handle_clone {
                                        if let Err(e) = handle.emit("config-changed", &new_config) {
                                            error!("Failed to emit config-changed event: {}", e);
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

        info!("Started watching configuration file: {:?}", self.config_path);
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

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            version: CONFIG_VERSION,
            server: ServerConfig::default(),
            api_keys: Vec::new(),
            routers: vec![
                RouterConfig::default_minimum_cost(),
                RouterConfig::default_maximum_performance(),
            ],
            providers: vec![ProviderConfig::default_ollama()],
            logging: LoggingConfig::default(),
        }
    }
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            port: 3625,
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

impl ApiKeyConfig {
    /// Create a new API key configuration
    pub fn new(name: String, model_selection: ModelSelection) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            name,
            model_selection,
            enabled: true,
            created_at: Utc::now(),
            last_used: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = AppConfig::default();
        assert_eq!(config.version, CONFIG_VERSION);
        assert_eq!(config.server.host, "127.0.0.1");
        assert_eq!(config.server.port, 3000);
        assert_eq!(config.routers.len(), 2);
        assert_eq!(config.providers.len(), 1);
    }

    #[test]
    fn test_server_config_default() {
        let server = ServerConfig::default();
        assert_eq!(server.host, "127.0.0.1");
        assert_eq!(server.port, 3000);
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
    fn test_api_key_config_new() {
        let key = ApiKeyConfig::new(
            "test-key".to_string(),
            "hash".to_string(),
            ModelSelection::Router {
                router_name: "Minimum Cost".to_string(),
            },
        );
        assert_eq!(key.name, "test-key");
        assert_eq!(key.key_hash, "hash");
        assert!(key.enabled);
        assert!(Uuid::parse_str(&key.id).is_ok());
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
}
