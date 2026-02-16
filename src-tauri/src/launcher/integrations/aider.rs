//! Aider integration
//!
//! Two modes:
//! - **Try It Out**: Terminal command with env vars (LLM only, no MCP support).
//! - **Permanent Config**: Write LLM settings to `~/.aider.conf.yml`.

use crate::launcher::backup;
use crate::launcher::AppIntegration;
use crate::ui::commands_clients::{AppCapabilities, LaunchResult};

pub struct AiderIntegration;

fn config_path() -> std::path::PathBuf {
    dirs::home_dir().unwrap_or_default().join(".aider.conf.yml")
}

impl AppIntegration for AiderIntegration {
    fn name(&self) -> &str {
        "Aider"
    }

    fn check_installed(&self) -> AppCapabilities {
        let binary = which::which("aider").ok();

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
        Ok(LaunchResult {
            success: true,
            message: "Run the command below in your terminal:".to_string(),
            modified_files: vec![],
            backup_files: vec![],
            terminal_command: Some(format!(
                "OPENAI_API_BASE={} OPENAI_API_KEY={} aider",
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

        // Read existing YAML, merge our keys, write back
        let mut config: serde_yaml::Value = if path.exists() {
            let data = std::fs::read_to_string(&path)
                .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;
            serde_yaml::from_str(&data)
                .unwrap_or(serde_yaml::Value::Mapping(serde_yaml::Mapping::new()))
        } else {
            serde_yaml::Value::Mapping(serde_yaml::Mapping::new())
        };

        if let serde_yaml::Value::Mapping(ref mut map) = config {
            map.insert(
                serde_yaml::Value::String("openai-api-base".to_string()),
                serde_yaml::Value::String(base_url.to_string()),
            );
            map.insert(
                serde_yaml::Value::String("openai-api-key".to_string()),
                serde_yaml::Value::String(client_secret.to_string()),
            );
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
            message: format!("Configured Aider at {}", path.display()),
            modified_files: vec![path.to_string_lossy().to_string()],
            backup_files,
            terminal_command: None,
        })
    }
}
