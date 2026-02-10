//! Droid integration
//!
//! Config-file only (no try-it-out). Writes TWO files:
//! - LLM: `~/.factory/settings.json` (customModels)
//! - MCP: `~/.factory/mcp.json` (separate file)

use crate::launcher::backup;
use crate::launcher::AppIntegration;
use crate::ui::commands_clients::{AppCapabilities, LaunchResult};
use std::path::PathBuf;

pub struct DroidIntegration;

fn settings_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_default()
        .join(".factory")
        .join("settings.json")
}

fn mcp_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_default()
        .join(".factory")
        .join("mcp.json")
}

impl AppIntegration for DroidIntegration {
    fn name(&self) -> &str {
        "Droid"
    }

    fn check_installed(&self) -> AppCapabilities {
        let binary = which::which("droid").ok();

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
        let mut modified_files = vec![];
        let mut all_backup_files = vec![];

        // 1. Write LLM config to settings.json
        let settings = settings_path();
        let mut config: serde_json::Value = if settings.exists() {
            let data = std::fs::read_to_string(&settings)
                .map_err(|e| format!("Failed to read config: {}", e))?;
            serde_json::from_str(&data).unwrap_or(serde_json::json!({}))
        } else {
            serde_json::json!({})
        };

        let model_entry = serde_json::json!({
            "model": "localrouter",
            "baseUrl": base_url,
            "apiKey": client_secret,
            "provider": "generic-chat-completion-api"
        });

        if let Some(obj) = config.as_object_mut() {
            let models = obj
                .entry("customModels")
                .or_insert_with(|| serde_json::json!([]));
            if let Some(arr) = models.as_array_mut() {
                arr.retain(|m| m.get("model").and_then(|v| v.as_str()) != Some("localrouter"));
                arr.push(model_entry);
            }
        }

        let data = serde_json::to_string_pretty(&config)
            .map_err(|e| format!("Failed to serialize settings: {}", e))?;

        let backup_path = backup::write_with_backup(&settings, data.as_bytes())?;
        modified_files.push(settings.to_string_lossy().to_string());
        if let Some(bp) = backup_path {
            all_backup_files.push(bp.to_string_lossy().to_string());
        }

        // 2. Write MCP config to mcp.json (separate file)
        let mcp = mcp_path();
        let mut mcp_config: serde_json::Value = if mcp.exists() {
            let data = std::fs::read_to_string(&mcp)
                .map_err(|e| format!("Failed to read MCP config: {}", e))?;
            serde_json::from_str(&data).unwrap_or(serde_json::json!({}))
        } else {
            serde_json::json!({})
        };

        let mcp_entry = serde_json::json!({
            "type": "http",
            "url": base_url,
            "headers": {
                "Authorization": format!("Bearer {}", client_secret)
            }
        });

        if let Some(obj) = mcp_config.as_object_mut() {
            let servers = obj
                .entry("mcpServers")
                .or_insert_with(|| serde_json::json!({}));
            if let Some(servers_obj) = servers.as_object_mut() {
                servers_obj.insert("localrouter".to_string(), mcp_entry);
            }
        }

        let mcp_data = serde_json::to_string_pretty(&mcp_config)
            .map_err(|e| format!("Failed to serialize MCP config: {}", e))?;

        let backup_path = backup::write_with_backup(&mcp, mcp_data.as_bytes())?;
        modified_files.push(mcp.to_string_lossy().to_string());
        if let Some(bp) = backup_path {
            all_backup_files.push(bp.to_string_lossy().to_string());
        }

        Ok(LaunchResult {
            success: true,
            message: format!(
                "Configured Droid: LLM at {}, MCP at {}",
                settings.display(),
                mcp.display()
            ),
            modified_files,
            backup_files: all_backup_files,
            terminal_command: None,
        })
    }
}
