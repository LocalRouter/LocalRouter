//! Configuration storage - loading and saving YAML files

use super::{migration, paths, validation, AppConfig};
use crate::utils::errors::{AppError, AppResult};
use std::path::Path;
use tokio::fs;
use tracing::{debug, info, warn};

/// Load configuration from a file
///
/// If the file doesn't exist, returns a default configuration.
/// If the file exists but is invalid, returns an error.
pub async fn load_config(path: &Path) -> AppResult<AppConfig> {
    // Ensure config directory exists
    if let Some(parent) = path.parent() {
        paths::ensure_dir_exists(&parent.to_path_buf())?;
    }

    // Check if file exists
    if !path.exists() {
        info!(
            "Configuration file not found at {:?}, creating default configuration",
            path
        );
        let default_config = AppConfig::default();
        save_config(&default_config, path).await?;
        return Ok(default_config);
    }

    debug!("Loading configuration from {:?}", path);

    // Read file contents
    let contents = fs::read_to_string(path)
        .await
        .map_err(|e| AppError::Config(format!("Failed to read configuration file: {}", e)))?;

    // Parse YAML
    let mut config: AppConfig = serde_yaml::from_str(&contents)
        .map_err(|e| AppError::Config(format!("Failed to parse configuration YAML: {}", e)))?;

    // Migrate if necessary
    if config.version < super::CONFIG_VERSION {
        warn!(
            "Configuration version {} is outdated (current: {}), migrating...",
            config.version,
            super::CONFIG_VERSION
        );
        config = migration::migrate_config(config)?;

        // Save migrated configuration
        save_config(&config, path).await?;
        info!("Configuration migrated successfully");
    }

    // Validate configuration
    validation::validate_config(&config)?;

    info!("Configuration loaded successfully from {:?}", path);
    Ok(config)
}

/// Save configuration to a file
///
/// Creates a backup of the existing file before writing.
pub async fn save_config(config: &AppConfig, path: &Path) -> AppResult<()> {
    debug!("Saving configuration to {:?}", path);

    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        paths::ensure_dir_exists(&parent.to_path_buf())?;
    }

    // Validate before saving
    validation::validate_config(config)?;

    // Create backup of existing file
    if path.exists() {
        let backup_path = path.with_extension("yaml.backup");
        if let Err(e) = fs::copy(path, &backup_path).await {
            warn!("Failed to create backup: {}", e);
        } else {
            debug!("Created backup at {:?}", backup_path);
        }
    }

    // Serialize to YAML
    let yaml = serde_yaml::to_string(config).map_err(|e| {
        AppError::Config(format!("Failed to serialize configuration to YAML: {}", e))
    })?;

    // Write to temporary file first
    let temp_path = path.with_extension("yaml.tmp");
    fs::write(&temp_path, yaml)
        .await
        .map_err(|e| AppError::Config(format!("Failed to write configuration file: {}", e)))?;

    // Atomically rename temporary file to actual file
    fs::rename(&temp_path, path)
        .await
        .map_err(|e| AppError::Config(format!("Failed to rename configuration file: {}", e)))?;

    info!("Configuration saved successfully to {:?}", path);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{LogLevel, LoggingConfig, ServerConfig};
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_save_and_load_config() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("settings.yaml");

        // Create a custom config
        let mut config = AppConfig::default();
        config.server = ServerConfig {
            host: "0.0.0.0".to_string(),
            port: 8080,
            enable_cors: false,
        };

        // Save config
        save_config(&config, &config_path).await.unwrap();

        // Load config
        let loaded_config = load_config(&config_path).await.unwrap();

        // Verify
        assert_eq!(loaded_config.server.host, "0.0.0.0");
        assert_eq!(loaded_config.server.port, 8080);
        assert!(!loaded_config.server.enable_cors);
    }

    #[tokio::test]
    async fn test_load_nonexistent_creates_default() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("settings.yaml");

        // Load config (should create default)
        let config = load_config(&config_path).await.unwrap();

        // Verify it's the default
        assert_eq!(config.server.host, "127.0.0.1");
        assert_eq!(config.server.port, 3000);

        // Verify file was created
        assert!(config_path.exists());
    }

    #[tokio::test]
    async fn test_save_creates_backup() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("settings.yaml");

        // Save first config
        let config1 = AppConfig::default();
        save_config(&config1, &config_path).await.unwrap();

        // Save second config (should create backup)
        let mut config2 = AppConfig::default();
        config2.server.port = 9000;
        save_config(&config2, &config_path).await.unwrap();

        // Check backup exists
        let backup_path = config_path.with_extension("yaml.backup");
        assert!(backup_path.exists());

        // Verify backup contains original config
        let backup_contents = fs::read_to_string(&backup_path).await.unwrap();
        let backup_config: AppConfig = serde_yaml::from_str(&backup_contents).unwrap();
        assert_eq!(backup_config.server.port, 3000);
    }

    #[tokio::test]
    async fn test_invalid_yaml_returns_error() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("settings.yaml");

        // Write invalid YAML
        fs::write(&config_path, "invalid: yaml: content: [")
            .await
            .unwrap();

        // Try to load (should fail)
        let result = load_config(&config_path).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Failed to parse"));
    }

    #[tokio::test]
    async fn test_config_roundtrip() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("settings.yaml");

        // Create config with various settings
        let mut config = AppConfig::default();
        config.logging = LoggingConfig {
            level: LogLevel::Debug,
            enable_access_log: false,
            log_dir: Some(temp_dir.path().to_path_buf()),
            retention_days: 7,
        };

        // Save and load
        save_config(&config, &config_path).await.unwrap();
        let loaded = load_config(&config_path).await.unwrap();

        // Verify all fields
        assert_eq!(loaded.logging.level, LogLevel::Debug);
        assert!(!loaded.logging.enable_access_log);
        assert_eq!(loaded.logging.retention_days, 7);
    }
}
