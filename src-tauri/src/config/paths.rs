//! OS-specific path resolution for configuration files

#![allow(dead_code)]

use crate::utils::errors::{AppError, AppResult};
use std::path::PathBuf;

/// Get the configuration directory
///
/// Priority:
/// 1. Runtime override via `LOCALROUTER_ENV` environment variable: `~/.localrouter-{env}/`
/// 2. Development mode (debug builds): `~/.localrouter-dev/`
/// 3. Production mode (release builds): `~/.localrouter/`
pub fn config_dir() -> AppResult<PathBuf> {
    let home = dirs::home_dir()
        .ok_or_else(|| AppError::Config("Could not determine home directory".to_string()))?;

    // Runtime override via environment variable (for testing)
    if let Ok(env_suffix) = std::env::var("LOCALROUTER_ENV") {
        return Ok(home.join(format!(".localrouter-{}", env_suffix)));
    }

    // Use separate directory for development/debug builds
    #[cfg(debug_assertions)]
    let dir = home.join(".localrouter-dev");

    #[cfg(not(debug_assertions))]
    let dir = home.join(".localrouter");

    Ok(dir)
}

/// Get the configuration file path
pub fn config_file() -> AppResult<PathBuf> {
    Ok(config_dir()?.join("settings.yaml"))
}

/// Get the API keys storage file path
pub fn api_keys_file() -> AppResult<PathBuf> {
    Ok(config_dir()?.join("api_keys.json"))
}

/// Get the routers configuration file path
pub fn routers_file() -> AppResult<PathBuf> {
    Ok(config_dir()?.join("routers.yaml"))
}

/// Get the providers configuration file path
pub fn providers_file() -> AppResult<PathBuf> {
    Ok(config_dir()?.join("providers.yaml"))
}

/// Get the logs directory
///
/// All platforms: `~/.localrouter/logs/`
pub fn logs_dir() -> AppResult<PathBuf> {
    Ok(config_dir()?.join("logs"))
}

/// Get the cache directory
///
/// All platforms: `~/.localrouter/cache/`
pub fn cache_dir() -> AppResult<PathBuf> {
    Ok(config_dir()?.join("cache"))
}

/// Get the secrets file path (for file-based keychain storage in development)
///
/// All platforms: `~/.localrouter/secrets.json`
pub fn secrets_file() -> AppResult<PathBuf> {
    Ok(config_dir()?.join("secrets.json"))
}

/// Ensure a directory exists, creating it if necessary
pub fn ensure_dir_exists(path: &PathBuf) -> AppResult<()> {
    if !path.exists() {
        std::fs::create_dir_all(path).map_err(|e| {
            AppError::Config(format!(
                "Failed to create directory {}: {}",
                path.display(),
                e
            ))
        })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    #[test]
    fn test_config_dir() {
        // Clear env var to test default behavior
        env::remove_var("LOCALROUTER_ENV");

        let dir = config_dir().unwrap();
        assert!(!dir.as_os_str().is_empty());

        // In debug builds, uses .localrouter-dev; in release, uses .localrouter
        #[cfg(debug_assertions)]
        assert!(dir.to_string_lossy().ends_with(".localrouter-dev"));

        #[cfg(not(debug_assertions))]
        assert!(dir.to_string_lossy().ends_with(".localrouter"));
    }

    #[test]
    fn test_config_dir_with_env_override() {
        // Set the LOCALROUTER_ENV to test the override
        env::set_var("LOCALROUTER_ENV", "test");

        let dir = config_dir().unwrap();
        assert!(
            dir.to_string_lossy().ends_with(".localrouter-test"),
            "Expected path to end with .localrouter-test, got: {}",
            dir.display()
        );

        // Clean up
        env::remove_var("LOCALROUTER_ENV");
    }

    #[test]
    fn test_config_dir_env_override_custom_suffix() {
        // Test with a custom suffix
        env::set_var("LOCALROUTER_ENV", "e2e-testing");

        let dir = config_dir().unwrap();
        assert!(
            dir.to_string_lossy().ends_with(".localrouter-e2e-testing"),
            "Expected path to end with .localrouter-e2e-testing, got: {}",
            dir.display()
        );

        // Clean up
        env::remove_var("LOCALROUTER_ENV");
    }

    #[test]
    fn test_config_file() {
        let file = config_file().unwrap();
        assert!(file.to_string_lossy().ends_with("settings.yaml"));
    }

    #[test]
    fn test_api_keys_file() {
        let file = api_keys_file().unwrap();
        assert!(file.to_string_lossy().ends_with("api_keys.json"));
    }

    #[test]
    fn test_logs_dir() {
        let dir = logs_dir().unwrap();
        assert!(!dir.as_os_str().is_empty());

        #[cfg(debug_assertions)]
        assert!(dir.to_string_lossy().contains(".localrouter-dev"));

        #[cfg(not(debug_assertions))]
        assert!(dir.to_string_lossy().contains(".localrouter"));

        assert!(dir.to_string_lossy().ends_with("logs"));
    }

    #[test]
    fn test_cache_dir() {
        let dir = cache_dir().unwrap();
        assert!(!dir.as_os_str().is_empty());

        #[cfg(debug_assertions)]
        assert!(dir.to_string_lossy().contains(".localrouter-dev"));

        #[cfg(not(debug_assertions))]
        assert!(dir.to_string_lossy().contains(".localrouter"));

        assert!(dir.to_string_lossy().ends_with("cache"));
    }
}
