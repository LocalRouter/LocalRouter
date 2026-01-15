//! OS-specific path resolution for configuration files

use crate::utils::errors::{AppError, AppResult};
use std::path::PathBuf;

/// Get the configuration directory
///
/// All platforms: `~/.localrouter/`
pub fn config_dir() -> AppResult<PathBuf> {
    let dir = dirs::home_dir()
        .ok_or_else(|| AppError::Config("Could not determine home directory".to_string()))?
        .join(".localrouter");

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

    #[test]
    fn test_config_dir() {
        let dir = config_dir().unwrap();
        assert!(!dir.as_os_str().is_empty());
        assert!(dir.to_string_lossy().ends_with(".localrouter"));
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
        assert!(dir.to_string_lossy().contains(".localrouter"));
        assert!(dir.to_string_lossy().ends_with("logs"));
    }

    #[test]
    fn test_cache_dir() {
        let dir = cache_dir().unwrap();
        assert!(!dir.as_os_str().is_empty());
        assert!(dir.to_string_lossy().contains(".localrouter"));
        assert!(dir.to_string_lossy().ends_with("cache"));
    }
}
