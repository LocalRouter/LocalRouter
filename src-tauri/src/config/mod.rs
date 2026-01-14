//! Configuration management module
//!
//! Handles loading, saving, and managing application configuration.

use crate::utils::errors::{AppError, AppResult};
use chrono::{DateTime, Utc};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
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
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ApiKeyConfig {
    /// Unique identifier
    pub id: String,

    /// Human-readable name
    pub name: String,

    /// Hashed API key (bcrypt)
    pub key_hash: String,

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

    /// API endpoint (for custom providers)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,

    /// API key reference (stored separately in encrypted storage)
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

/// Thread-safe configuration manager
#[derive(Debug, Clone)]
pub struct ConfigManager {
    config: Arc<RwLock<AppConfig>>,
    config_path: PathBuf,
}

impl ConfigManager {
    /// Create a new configuration manager
    pub fn new(config: AppConfig, config_path: PathBuf) -> Self {
        Self {
            config: Arc::new(RwLock::new(config)),
            config_path,
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

    /// Get a read-only copy of the configuration
    pub fn get(&self) -> AppConfig {
        self.config.read().clone()
    }

    /// Update configuration with a function
    pub fn update<F>(&self, f: F) -> AppResult<()>
    where
        F: FnOnce(&mut AppConfig),
    {
        let mut config = self.config.write();
        f(&mut config);
        validation::validate_config(&config)?;
        Ok(())
    }

    /// Save configuration to disk
    pub async fn save(&self) -> AppResult<()> {
        let config = self.config.read().clone();
        save_config(&config, &self.config_path).await
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
            port: 3000,
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
            endpoint: Some("http://localhost:11434".to_string()),
            api_key_ref: None,
        }
    }
}

impl ApiKeyConfig {
    /// Create a new API key configuration
    pub fn new(name: String, key_hash: String, model_selection: ModelSelection) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            name,
            key_hash,
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
