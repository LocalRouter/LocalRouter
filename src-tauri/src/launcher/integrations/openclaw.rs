//! OpenClaw integration
//!
//! Config-file only (no try-it-out). LLM only, no MCP support.
//! Modifies `~/.openclaw/openclaw.json`.

use crate::launcher::backup;
use crate::launcher::AppIntegration;
use crate::ui::commands_clients::{AppCapabilities, LaunchResult};
use std::path::PathBuf;

pub struct OpenClawIntegration;

fn config_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_default()
        .join(".openclaw")
        .join("openclaw.json")
}

impl AppIntegration for OpenClawIntegration {
    fn name(&self) -> &str {
        "OpenClaw"
    }

    fn check_installed(&self) -> AppCapabilities {
        let binary = which::which("openclaw")
            .or_else(|_| which::which("clawdbot"))
            .ok();

        AppCapabilities {
            installed: binary.is_some(),
            binary_path: binary.map(|p| p.to_string_lossy().to_string()),
            version: None,
            supports_try_it_out: self.supports_try_it_out(),
            supports_permanent_config: self.supports_permanent_config(),
        }
    }

    fn supports_permanent_config(&self) -> bool {
        true
    }

    fn configure_permanent(
        &self,
        base_url: &str,
        client_secret: &str,
        _client_id: &str,
    ) -> Result<LaunchResult, String> {
        let path = config_path();

        let mut config: serde_json::Value = if path.exists() {
            let data = std::fs::read_to_string(&path)
                .map_err(|e| format!("Failed to read config: {}", e))?;
            serde_json::from_str(&data).unwrap_or(serde_json::json!({}))
        } else {
            serde_json::json!({})
        };

        let obj = config.as_object_mut().ok_or("Invalid config format")?;

        let models_section = obj
            .entry("models")
            .or_insert_with(|| serde_json::json!({}));
        let providers = models_section
            .as_object_mut()
            .ok_or("Invalid models section")?
            .entry("providers")
            .or_insert_with(|| serde_json::json!({}));

        let provider_entry = serde_json::json!({
            "baseUrl": base_url,
            "apiKey": client_secret,
            "api": "openai-completions"
        });

        if let Some(prov_obj) = providers.as_object_mut() {
            prov_obj.insert("localrouter".to_string(), provider_entry);
        }

        let data = serde_json::to_string_pretty(&config)
            .map_err(|e| format!("Failed to serialize config: {}", e))?;

        let backup_path = backup::write_with_backup(&path, data.as_bytes())?;
        let backup_files: Vec<String> = backup_path
            .iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect();

        Ok(LaunchResult {
            success: true,
            message: format!("Configured OpenClaw at {}", path.display()),
            modified_files: vec![path.to_string_lossy().to_string()],
            backup_files,
            terminal_command: None,
        })
    }
}
