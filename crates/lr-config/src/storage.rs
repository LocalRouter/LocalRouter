//! Configuration storage - loading and saving YAML files

use super::{migration, validation, AppConfig};
use chrono::Utc;
use lr_types::{AppError, AppResult};
use lr_utils::paths;
use std::path::{Path, PathBuf};
use tokio::fs;
use tracing::{debug, error, info, warn};

/// Maximum number of timestamped backups to keep
const MAX_BACKUPS: usize = 3;

/// Maximum allowed config file size (10 MB)
const MAX_CONFIG_FILE_SIZE: u64 = 10 * 1024 * 1024;

/// Load configuration from a file
///
/// If the file doesn't exist, returns a default configuration.
/// If the file exists but is invalid, tries to recover from backups.
/// Only returns an error if all recovery attempts fail.
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

    // Try to load from main file
    match load_config_from_file(path).await {
        Ok(config) => {
            info!("Configuration loaded successfully from {:?}", path);
            Ok(config)
        }
        Err(main_error) => {
            error!(
                "Failed to load configuration from {:?}: {}",
                path, main_error
            );

            // Try to recover from backups
            if let Some(parent) = path.parent() {
                let backups = list_backups(parent).await;
                if !backups.is_empty() {
                    info!("Attempting to recover from {} backup(s)...", backups.len());

                    for backup_path in &backups {
                        match load_config_from_file(backup_path).await {
                            Ok(config) => {
                                warn!("Recovered configuration from backup: {:?}", backup_path);
                                // Save the recovered config as the main file
                                // (this also creates a new backup of the corrupted file)
                                if let Err(e) = save_config(&config, path).await {
                                    warn!("Failed to save recovered config: {}", e);
                                }
                                return Ok(config);
                            }
                            Err(e) => {
                                debug!("Backup {:?} also failed: {}", backup_path, e);
                            }
                        }
                    }

                    error!("All {} backup(s) failed to load", backups.len());
                }
            }

            // All recovery attempts failed
            Err(main_error)
        }
    }
}

/// Load and parse configuration from a specific file
async fn load_config_from_file(path: &Path) -> AppResult<AppConfig> {
    debug!("Loading configuration from {:?}", path);

    // Check file size before reading to prevent excessive memory usage
    let metadata = fs::metadata(path)
        .await
        .map_err(|e| AppError::Config(format!("Failed to read config metadata: {}", e)))?;
    if metadata.len() > MAX_CONFIG_FILE_SIZE {
        return Err(AppError::Config(format!(
            "Config file too large: {} bytes (max {} bytes)",
            metadata.len(),
            MAX_CONFIG_FILE_SIZE
        )));
    }

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
    }

    // Validate configuration
    validation::validate_config(&config)?;

    // In production, migrate any plaintext secrets file to the system keychain.
    // This runs on every config load so the user can copy dev config to prod at any time.
    #[cfg(not(debug_assertions))]
    migrate_secrets_file_to_keychain(&config);

    Ok(config)
}

/// Save configuration to a file
///
/// Creates a timestamped backup of the existing file before writing.
/// Keeps up to MAX_BACKUPS most recent backups.
pub async fn save_config(config: &AppConfig, path: &Path) -> AppResult<()> {
    debug!("Saving configuration to {:?}", path);

    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        paths::ensure_dir_exists(&parent.to_path_buf())?;
    }

    // Validate before saving
    validation::validate_config(config)?;

    // Create timestamped backup of existing file
    if path.exists() {
        if let Some(parent) = path.parent() {
            let timestamp = Utc::now().format("%Y%m%d-%H%M%S");
            let backup_name = format!("settings.yaml.backup.{}", timestamp);
            let backup_path = parent.join(&backup_name);

            if let Err(e) = fs::copy(path, &backup_path).await {
                warn!("Failed to create backup: {}", e);
            } else {
                debug!("Created timestamped backup at {:?}", backup_path);

                // Set restrictive file permissions on backup file (Unix only)
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let permissions = std::fs::Permissions::from_mode(0o600);
                    if let Err(e) = std::fs::set_permissions(&backup_path, permissions) {
                        warn!("Failed to set backup file permissions: {}", e);
                    }
                }

                // Clean up old backups
                cleanup_old_backups(parent).await;
            }
        }
    }

    // Serialize to YAML
    let yaml = serde_yaml::to_string(config).map_err(|e| {
        AppError::Config(format!("Failed to serialize configuration to YAML: {}", e))
    })?;

    // Write to temporary file first (use unique name to avoid races between concurrent saves)
    let unique_id = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let temp_path = path.with_extension(format!("yaml.tmp.{}", unique_id));

    // Use explicit file operations with sync to ensure data is written before rename
    {
        use tokio::io::AsyncWriteExt;
        let mut file = fs::File::create(&temp_path).await.map_err(|e| {
            AppError::Config(format!(
                "Failed to create temp file '{}': {}",
                temp_path.display(),
                e
            ))
        })?;
        file.write_all(yaml.as_bytes()).await.map_err(|e| {
            AppError::Config(format!(
                "Failed to write to temp file '{}': {}",
                temp_path.display(),
                e
            ))
        })?;
        file.sync_all().await.map_err(|e| {
            AppError::Config(format!(
                "Failed to sync temp file '{}': {}",
                temp_path.display(),
                e
            ))
        })?;
    }

    // Verify temp file exists before attempting rename
    if !temp_path.exists() {
        return Err(AppError::Config(format!(
            "Temp file '{}' does not exist after write (possible race condition)",
            temp_path.display()
        )));
    }

    // Atomically rename temporary file to actual file
    if let Err(e) = fs::rename(&temp_path, path).await {
        // Clean up the temp file on failure
        let _ = fs::remove_file(&temp_path).await;
        return Err(AppError::Config(format!(
            "Failed to rename '{}' to '{}': {}",
            temp_path.display(),
            path.display(),
            e
        )));
    }

    // Set restrictive file permissions on Unix (owner read/write only)
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let permissions = std::fs::Permissions::from_mode(0o600);
        if let Err(e) = std::fs::set_permissions(path, permissions) {
            warn!("Failed to set config file permissions: {}", e);
        }
    }

    info!("Configuration saved successfully to {:?}", path);
    Ok(())
}

/// List available backup files, sorted by most recent first
async fn list_backups(dir: &Path) -> Vec<PathBuf> {
    let mut backups = Vec::new();

    // Read directory entries
    let mut entries = match fs::read_dir(dir).await {
        Ok(entries) => entries,
        Err(e) => {
            debug!("Failed to read backup directory: {}", e);
            return backups;
        }
    };

    // Collect backup files
    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            // Match both old-style .yaml.backup and new timestamped .yaml.backup.YYYYMMDD-HHMMSS
            if name.starts_with("settings.yaml.backup") {
                backups.push(path);
            }
        }
    }

    // Sort by filename descending (newer timestamps come first)
    // Timestamped backups sort naturally: settings.yaml.backup.20260125-120000 > settings.yaml.backup.20260124-120000
    // Old-style settings.yaml.backup sorts after timestamped ones (no timestamp = older)
    backups.sort_by(|a, b| {
        let a_name = a.file_name().and_then(|n| n.to_str()).unwrap_or("");
        let b_name = b.file_name().and_then(|n| n.to_str()).unwrap_or("");
        b_name.cmp(a_name)
    });

    backups
}

/// Clean up old backups, keeping only the most recent MAX_BACKUPS
async fn cleanup_old_backups(dir: &Path) {
    let backups = list_backups(dir).await;

    if backups.len() > MAX_BACKUPS {
        for backup_path in backups.into_iter().skip(MAX_BACKUPS) {
            if let Err(e) = fs::remove_file(&backup_path).await {
                debug!(
                    "Failed to remove old backup {}: {} (may have been cleaned up by another process)",
                    backup_path.display(),
                    e
                );
            } else {
                debug!("Removed old backup: {:?}", backup_path);
            }
        }
    }
}

/// Migrate secrets from a plaintext secrets.json file to the system keychain.
///
/// When the user copies their dev config directory to production, the file-based
/// keychain (`secrets.json`) comes along. This function cross-references the secrets
/// against the loaded config, stores referenced secrets in the OS keychain, and
/// deletes the file (and any backup copies).
///
/// Runs on every config load (not a one-time migration) so the user can re-copy
/// dev config to prod at any time.
///
/// No-op if:
/// - `LOCALROUTER_KEYCHAIN` is set to `"file"`
/// - The secrets file does not exist
#[cfg(not(debug_assertions))]
fn migrate_secrets_file_to_keychain(config: &super::AppConfig) {
    use lr_api_keys::keychain_trait::{KeychainStorage, SystemKeychain};
    use std::collections::HashSet;
    use super::{
        McpAuthConfig, CLIENT_KEYRING_SERVICE, MCP_KEYRING_SERVICE,
        OAUTH_CLIENT_KEYRING_SERVICE, PROVIDER_KEYRING_SERVICE,
    };

    // If the user explicitly wants file-based storage, don't migrate
    if matches!(
        std::env::var("LOCALROUTER_KEYCHAIN").as_deref(),
        Ok("file")
    ) {
        return;
    }

    let secrets_path = match lr_utils::paths::secrets_file() {
        Ok(p) => p,
        Err(e) => {
            warn!("Could not resolve secrets file path: {}", e);
            return;
        }
    };

    if !secrets_path.exists() {
        return;
    }

    info!(
        "Found plaintext secrets file at {}, migrating to system keychain...",
        secrets_path.display()
    );

    // Read and parse the secrets file
    let contents = match std::fs::read_to_string(&secrets_path) {
        Ok(c) => c,
        Err(e) => {
            warn!("Failed to read secrets file: {}", e);
            return;
        }
    };

    if contents.trim().is_empty() {
        cleanup_secrets_files(&secrets_path);
        info!("Removed empty secrets file");
        return;
    }

    let secrets: std::collections::HashMap<String, String> =
        match serde_json::from_str(&contents) {
            Ok(s) => s,
            Err(e) => {
                warn!("Failed to parse secrets file: {}", e);
                return;
            }
        };

    if secrets.is_empty() {
        cleanup_secrets_files(&secrets_path);
        info!("Removed empty secrets file");
        return;
    }

    // Build the set of secret keys that are actually referenced by the config.
    // Key format in secrets.json is "service:account".
    let mut referenced_keys: HashSet<String> = HashSet::new();

    // Providers: "LocalRouter-Providers:{provider_name}"
    for provider in &config.providers {
        referenced_keys.insert(format!("{}:{}", PROVIDER_KEYRING_SERVICE, provider.name));
    }

    // MCP servers: "LocalRouter-McpServers:{ref_uuid}"
    for server in &config.mcp_servers {
        if let Some(ref auth_config) = server.auth_config {
            match auth_config {
                McpAuthConfig::BearerToken { token_ref } => {
                    referenced_keys
                        .insert(format!("{}:{}", MCP_KEYRING_SERVICE, token_ref));
                }
                McpAuthConfig::CustomHeaders { header_refs } => {
                    for ref_key in header_refs.values() {
                        referenced_keys
                            .insert(format!("{}:{}", MCP_KEYRING_SERVICE, ref_key));
                    }
                }
                McpAuthConfig::EnvVars { env_refs } => {
                    for ref_key in env_refs.values() {
                        referenced_keys
                            .insert(format!("{}:{}", MCP_KEYRING_SERVICE, ref_key));
                    }
                }
                McpAuthConfig::OAuth {
                    client_secret_ref, ..
                }
                | McpAuthConfig::OAuthBrowser {
                    client_secret_ref, ..
                } => {
                    referenced_keys.insert(format!(
                        "{}:{}",
                        MCP_KEYRING_SERVICE, client_secret_ref
                    ));
                }
                McpAuthConfig::None => {}
            }
        }
    }

    // Clients: "LocalRouter-Clients:{client_id}"
    for client in &config.clients {
        referenced_keys.insert(format!("{}:{}", CLIENT_KEYRING_SERVICE, client.id));
    }

    // OAuth clients: "LocalRouter-OAuthClients:{oauth_client_id}"
    for oauth_client in &config.oauth_clients {
        referenced_keys
            .insert(format!("{}:{}", OAUTH_CLIENT_KEYRING_SERVICE, oauth_client.id));
    }

    // Migrate referenced secrets to system keychain
    let system_keychain = SystemKeychain;
    let mut migrated = 0u32;
    let mut skipped = 0u32;
    let mut errors = 0u32;

    for (key, secret) in &secrets {
        if !referenced_keys.contains(key) {
            skipped += 1;
            continue;
        }

        let Some((service, account)) = key.split_once(':') else {
            warn!("Skipping secret with invalid key format: {}", key);
            continue;
        };

        match system_keychain.store(service, account, secret) {
            Ok(()) => {
                info!("Migrated secret {}:{} to system keychain", service, account);
                migrated += 1;
            }
            Err(e) => {
                warn!(
                    "Failed to migrate secret {}:{} to system keychain: {}",
                    service, account, e
                );
                errors += 1;
            }
        }
    }

    info!(
        "Secrets migration: {} migrated, {} skipped (not in config), {} errors (total {} in file)",
        migrated,
        skipped,
        errors,
        secrets.len()
    );

    if errors > 0 {
        warn!(
            "Some secrets failed to migrate ({} errors). Keeping secrets file for retry on next launch.",
            errors
        );
        return;
    }

    cleanup_secrets_files(&secrets_path);
    info!(
        "Deleted secrets file and backups: {}",
        secrets_path.display()
    );
}

/// Delete the secrets file and any backup/leftover copies in the same directory.
#[cfg(not(debug_assertions))]
fn cleanup_secrets_files(secrets_path: &Path) {
    // Delete the main file
    if let Err(e) = std::fs::remove_file(secrets_path) {
        warn!("Failed to delete secrets file: {}", e);
    }

    // Clean up any backup files (e.g. secrets.json.bak, secrets.json.backup.*)
    if let Some(dir) = secrets_path.parent() {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name_str = name.to_string_lossy();
                // Match secrets.json.* but not the main file itself (already deleted)
                if name_str.starts_with("secrets.json.") {
                    if let Err(e) = std::fs::remove_file(entry.path()) {
                        warn!("Failed to delete secrets backup {}: {}", name_str, e);
                    } else {
                        debug!("Deleted secrets backup: {}", name_str);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{LogLevel, LoggingConfig, ServerConfig};
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_save_and_load_config() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("settings.yaml");

        // Create a custom config
        let config = AppConfig {
            server: ServerConfig {
                host: "0.0.0.0".to_string(),
                port: 8080,
                enable_cors: false,
            },
            ..Default::default()
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
        #[cfg(debug_assertions)]
        assert_eq!(config.server.port, 33625);
        #[cfg(not(debug_assertions))]
        assert_eq!(config.server.port, 3625);

        // Verify file was created
        assert!(config_path.exists());
    }

    #[tokio::test]
    async fn test_save_creates_timestamped_backup() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("settings.yaml");

        // Save first config
        let config1 = AppConfig::default();
        save_config(&config1, &config_path).await.unwrap();

        // Save second config (should create timestamped backup)
        let mut config2 = AppConfig::default();
        config2.server.port = 9000;
        save_config(&config2, &config_path).await.unwrap();

        // Check that a timestamped backup exists
        let backups = list_backups(temp_dir.path()).await;
        assert!(!backups.is_empty(), "Should have at least one backup");

        // Verify backup contains original config
        let backup_contents = fs::read_to_string(&backups[0]).await.unwrap();
        let backup_config: AppConfig = serde_yaml::from_str(&backup_contents).unwrap();
        #[cfg(debug_assertions)]
        assert_eq!(backup_config.server.port, 33625);
        #[cfg(not(debug_assertions))]
        assert_eq!(backup_config.server.port, 3625);
    }

    #[tokio::test]
    async fn test_backup_cleanup() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("settings.yaml");

        // Create more backups than MAX_BACKUPS
        for i in 0..(MAX_BACKUPS + 5) {
            let mut config = AppConfig::default();
            config.server.port = 3000 + i as u16;
            save_config(&config, &config_path).await.unwrap();
            // Small delay to ensure different timestamps
            tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
        }

        // Check that only MAX_BACKUPS remain
        let backups = list_backups(temp_dir.path()).await;
        assert!(
            backups.len() <= MAX_BACKUPS,
            "Should have at most {} backups, found {}",
            MAX_BACKUPS,
            backups.len()
        );
    }

    #[tokio::test]
    async fn test_recovery_from_backup() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("settings.yaml");

        // Save a valid config (port 7777)
        let mut config = AppConfig::default();
        config.server.port = 7777;
        save_config(&config, &config_path).await.unwrap();

        // Save again to create a backup of the 7777 config
        config.server.port = 8888;
        save_config(&config, &config_path).await.unwrap();

        // Corrupt the main config file (which had port 8888)
        fs::write(&config_path, "invalid: yaml: content: [")
            .await
            .unwrap();

        // Load should recover from backup (which has port 7777)
        let loaded = load_config(&config_path).await.unwrap();
        // Backup contains the previous version before the 8888 save
        assert_eq!(loaded.server.port, 7777);
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
        let config = AppConfig {
            logging: LoggingConfig {
                level: LogLevel::Debug,
                enable_access_log: false,
                log_dir: Some(temp_dir.path().to_path_buf()),
                retention_days: 7,
            },
            ..Default::default()
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
