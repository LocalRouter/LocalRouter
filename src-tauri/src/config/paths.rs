//! OS-specific path resolution for configuration files

use crate::utils::errors::{AppError, AppResult};
use std::path::PathBuf;

/// Get the configuration directory based on OS
///
/// - Linux: `~/.config/localrouter/` or `~/.localrouter/`
/// - macOS: `~/Library/Application Support/LocalRouter/`
/// - Windows: `%APPDATA%\LocalRouter\`
pub fn config_dir() -> AppResult<PathBuf> {
    let dir = if cfg!(target_os = "macos") {
        dirs::home_dir()
            .ok_or_else(|| AppError::Config("Could not determine home directory".to_string()))?
            .join("Library")
            .join("Application Support")
            .join("LocalRouter")
    } else if cfg!(target_os = "windows") {
        dirs::config_dir()
            .ok_or_else(|| AppError::Config("Could not determine config directory".to_string()))?
            .join("LocalRouter")
    } else {
        // Linux and other Unix-like systems
        dirs::home_dir()
            .ok_or_else(|| AppError::Config("Could not determine home directory".to_string()))?
            .join(".localrouter")
    };

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

/// Get the logs directory based on OS
///
/// - Linux: `/var/log/localrouter/` (if permissions) or `~/.localrouter/logs/`
/// - macOS: `~/Library/Logs/LocalRouter/`
/// - Windows: `%APPDATA%\LocalRouter\logs\`
pub fn logs_dir() -> AppResult<PathBuf> {
    let dir = if cfg!(target_os = "macos") {
        dirs::home_dir()
            .ok_or_else(|| AppError::Config("Could not determine home directory".to_string()))?
            .join("Library")
            .join("Logs")
            .join("LocalRouter")
    } else if cfg!(target_os = "windows") {
        config_dir()?.join("logs")
    } else {
        // Linux - try user directory since /var/log requires sudo
        config_dir()?.join("logs")
    };

    Ok(dir)
}

/// Get the cache directory based on OS
///
/// - Linux: `~/.cache/localrouter/`
/// - macOS: `~/Library/Caches/LocalRouter/`
/// - Windows: `%LOCALAPPDATA%\LocalRouter\`
pub fn cache_dir() -> AppResult<PathBuf> {
    let dir = if cfg!(target_os = "macos") {
        dirs::home_dir()
            .ok_or_else(|| AppError::Config("Could not determine home directory".to_string()))?
            .join("Library")
            .join("Caches")
            .join("LocalRouter")
    } else if cfg!(target_os = "windows") {
        dirs::cache_dir()
            .ok_or_else(|| AppError::Config("Could not determine cache directory".to_string()))?
            .join("LocalRouter")
    } else {
        // Linux
        dirs::cache_dir()
            .ok_or_else(|| AppError::Config("Could not determine cache directory".to_string()))?
            .join("localrouter")
    };

    Ok(dir)
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

        if cfg!(target_os = "macos") {
            assert!(dir
                .to_string_lossy()
                .contains("Library/Application Support/LocalRouter"));
        } else if cfg!(target_os = "windows") {
            assert!(dir.to_string_lossy().contains("LocalRouter"));
        } else {
            assert!(dir.to_string_lossy().contains(".localrouter"));
        }
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

        if cfg!(target_os = "macos") {
            assert!(dir.to_string_lossy().contains("Library/Logs/LocalRouter"));
        } else {
            assert!(dir.to_string_lossy().contains("logs"));
        }
    }

    #[test]
    fn test_cache_dir() {
        let dir = cache_dir().unwrap();
        assert!(!dir.as_os_str().is_empty());

        if cfg!(target_os = "macos") {
            assert!(dir.to_string_lossy().contains("Library/Caches/LocalRouter"));
        } else {
            assert!(dir.to_string_lossy().contains("localrouter"));
        }
    }
}
