//! Goose integration
//!
//! Two modes:
//! - **Try It Out**: Terminal command with env vars (LLM only).
//! - **Permanent Config**: Write MCP extension to `~/.config/goose/config.yaml`.

use crate::launcher::backup;
use crate::launcher::AppIntegration;
use crate::ui::commands_clients::{AppCapabilities, LaunchResult};

pub struct GooseIntegration;

fn config_path() -> std::path::PathBuf {
    dirs::home_dir()
        .unwrap_or_default()
        .join(".config")
        .join("goose")
        .join("config.yaml")
}

impl AppIntegration for GooseIntegration {
    fn name(&self) -> &str {
        "Goose"
    }

    fn check_installed(&self) -> AppCapabilities {
        let binary = which::which("goose").ok();

        AppCapabilities {
            installed: binary.is_some(),
            binary_path: binary.map(|p| p.to_string_lossy().to_string()),
            version: None,
            supports_try_it_out: self.supports_try_it_out(),
            supports_permanent_config: self.supports_permanent_config(),
        }
    }

    fn supports_try_it_out(&self) -> bool {
        true
    }

    fn supports_permanent_config(&self) -> bool {
        true
    }

    fn try_it_out(
        &self,
        base_url: &str,
        client_secret: &str,
        _client_id: &str,
    ) -> Result<LaunchResult, String> {
        // LLM only via env vars
        Ok(LaunchResult {
            success: true,
            message: "Run the command below in your terminal:".to_string(),
            modified_files: vec![],
            backup_files: vec![],
            terminal_command: Some(format!(
                "OPENAI_BASE_URL={} OPENAI_API_KEY={} goose",
                base_url, client_secret
            )),
        })
    }

    fn configure_permanent(
        &self,
        base_url: &str,
        client_secret: &str,
        _client_id: &str,
    ) -> Result<LaunchResult, String> {
        let path = config_path();

        // Read existing YAML, merge our extension under `extensions`
        let mut config: serde_yaml::Value = if path.exists() {
            let data = std::fs::read_to_string(&path)
                .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;
            serde_yaml::from_str(&data).unwrap_or(serde_yaml::Value::Mapping(
                serde_yaml::Mapping::new(),
            ))
        } else {
            serde_yaml::Value::Mapping(serde_yaml::Mapping::new())
        };

        // Build our extension entry
        let mut extension = serde_yaml::Mapping::new();
        extension.insert(
            serde_yaml::Value::String("type".to_string()),
            serde_yaml::Value::String("streamable_http".to_string()),
        );
        extension.insert(
            serde_yaml::Value::String("name".to_string()),
            serde_yaml::Value::String("LocalRouter".to_string()),
        );
        extension.insert(
            serde_yaml::Value::String("uri".to_string()),
            serde_yaml::Value::String(base_url.to_string()),
        );
        extension.insert(
            serde_yaml::Value::String("enabled".to_string()),
            serde_yaml::Value::Bool(true),
        );
        let mut headers = serde_yaml::Mapping::new();
        headers.insert(
            serde_yaml::Value::String("Authorization".to_string()),
            serde_yaml::Value::String(format!("Bearer {}", client_secret)),
        );
        extension.insert(
            serde_yaml::Value::String("headers".to_string()),
            serde_yaml::Value::Mapping(headers),
        );

        if let serde_yaml::Value::Mapping(ref mut map) = config {
            let extensions = map
                .entry(serde_yaml::Value::String("extensions".to_string()))
                .or_insert(serde_yaml::Value::Mapping(serde_yaml::Mapping::new()));
            if let serde_yaml::Value::Mapping(ref mut ext_map) = extensions {
                ext_map.insert(
                    serde_yaml::Value::String("localrouter".to_string()),
                    serde_yaml::Value::Mapping(extension),
                );
            }
        }

        let data = serde_yaml::to_string(&config)
            .map_err(|e| format!("Failed to serialize config: {}", e))?;

        let backup_path = backup::write_with_backup(&path, data.as_bytes())?;
        let backup_files: Vec<String> = backup_path
            .iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect();

        Ok(LaunchResult {
            success: true,
            message: format!("MCP extension configured in {}", path.display()),
            modified_files: vec![path.to_string_lossy().to_string()],
            backup_files,
            terminal_command: None,
        })
    }
}
