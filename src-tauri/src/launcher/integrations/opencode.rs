//! OpenCode integration
//!
//! Config-file only (no try-it-out). Modifies `<config_dir>/opencode/opencode.json`
//! to add LocalRouter as an LLM provider and MCP server.
//!
//! MCP format: key `"mcp"` (not `"mcpServers"`), type `"remote"` (not `"http"`).

use crate::launcher::backup;
use crate::launcher::AppIntegration;
use crate::ui::commands_clients::{AppCapabilities, LaunchResult};
use std::path::PathBuf;

pub struct OpenCodeIntegration;

fn config_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join(".config"))
        .join("opencode")
        .join("opencode.json")
}

impl AppIntegration for OpenCodeIntegration {
    fn name(&self) -> &str {
        "OpenCode"
    }

    fn check_installed(&self) -> AppCapabilities {
        let binary = which::which("opencode").ok();

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

        // LLM provider entry
        let provider_entry = serde_json::json!({
            "npm": "@ai-sdk/openai-compatible",
            "name": "LocalRouter",
            "options": {
                "baseURL": base_url
            }
        });

        // MCP server entry — OpenCode uses "mcp" key and "remote" type
        let mcp_entry = serde_json::json!({
            "type": "remote",
            "url": base_url,
            "headers": {
                "Authorization": format!("Bearer {}", client_secret)
            }
        });

        if let Some(obj) = config.as_object_mut() {
            let provider = obj
                .entry("provider")
                .or_insert_with(|| serde_json::json!({}));
            if let Some(prov_obj) = provider.as_object_mut() {
                prov_obj.insert("localrouter".to_string(), provider_entry);
            }

            // OpenCode uses "mcp" key (not "mcpServers")
            let mcp = obj
                .entry("mcp")
                .or_insert_with(|| serde_json::json!({}));
            if let Some(mcp_obj) = mcp.as_object_mut() {
                mcp_obj.insert("localrouter".to_string(), mcp_entry);
            }
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
            message: format!("Configured OpenCode at {}", path.display()),
            modified_files: vec![path.to_string_lossy().to_string()],
            backup_files,
            terminal_command: None,
        })
    }
}
