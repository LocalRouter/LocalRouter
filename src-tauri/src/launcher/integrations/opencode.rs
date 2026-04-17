//! OpenCode integration
//!
//! Config-file only (no try-it-out). Modifies `<config_dir>/opencode/opencode.json`
//! to add LocalRouter as an LLM provider and MCP server.
//!
//! MCP format: key `"mcp"` (not `"mcpServers"`), type `"remote"` (not `"http"`).

use crate::launcher::backup;
use crate::launcher::{AppIntegration, ConfigSyncContext};
use crate::ui::commands_clients::{AppCapabilities, LaunchResult};
use std::path::PathBuf;

pub struct OpenCodeIntegration;

fn config_path() -> PathBuf {
    // OpenCode uses XDG config path (~/.config/opencode/) on all platforms,
    // not the macOS-native ~/Library/Application Support/
    let xdg_config = std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| dirs::home_dir().unwrap_or_default().join(".config"));
    xdg_config.join("opencode").join("opencode.json")
}

/// Read the existing opencode.json or create an empty object
fn read_config(path: &std::path::Path) -> serde_json::Value {
    if path.exists() {
        let data = std::fs::read_to_string(path).unwrap_or_default();
        serde_json::from_str(&data).unwrap_or(serde_json::json!({}))
    } else {
        serde_json::json!({})
    }
}

/// Build the MCP server entry (shared between configure_permanent and sync_config)
fn mcp_entry(base_url: &str, client_secret: &str) -> serde_json::Value {
    serde_json::json!({
        "type": "remote",
        "url": base_url,
        "headers": {
            "Authorization": format!("Bearer {}", client_secret)
        }
    })
}

/// Write config JSON to disk with backup
fn write_config(
    path: &std::path::Path,
    config: &serde_json::Value,
) -> Result<LaunchResult, String> {
    let data = serde_json::to_string_pretty(config)
        .map_err(|e| format!("Failed to serialize config: {}", e))?;

    let backup_path = backup::write_with_backup(path, data.as_bytes())?;
    let backup_files: Vec<String> = backup_path
        .iter()
        .map(|p| p.to_string_lossy().to_string())
        .collect();

    Ok(LaunchResult {
        success: true,
        message: format!(
            "Configured OpenCode at {}. Run /connect in OpenCode to apply changes.",
            path.display()
        ),
        modified_files: vec![path.to_string_lossy().to_string()],
        backup_files,
        terminal_command: None,
    })
}

impl AppIntegration for OpenCodeIntegration {
    fn name(&self) -> &str {
        "OpenCode"
    }

    fn check_installed(&self) -> AppCapabilities {
        let binary = super::find_binary("opencode");

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

    fn needs_model_list(&self) -> bool {
        true
    }

    fn configure_permanent(
        &self,
        base_url: &str,
        client_secret: &str,
        _client_id: &str,
    ) -> Result<LaunchResult, String> {
        self.write_config(base_url, client_secret, true, true, None)
    }

    fn sync_config(&self, ctx: &ConfigSyncContext) -> Result<LaunchResult, String> {
        self.write_config(
            &ctx.base_url,
            &ctx.client_secret,
            ctx.should_sync_llm(),
            ctx.should_sync_mcp(),
            Some(&ctx.models),
        )
    }
}

impl OpenCodeIntegration {
    fn write_config(
        &self,
        base_url: &str,
        client_secret: &str,
        sync_llm: bool,
        sync_mcp: bool,
        models: Option<&Vec<String>>,
    ) -> Result<LaunchResult, String> {
        let path = config_path();

        // Nothing to write or clean up
        if !sync_llm && !sync_mcp && !path.exists() {
            return Ok(LaunchResult {
                success: true,
                message: "No config to sync for current client mode".to_string(),
                modified_files: vec![],
                backup_files: vec![],
                terminal_command: None,
            });
        }

        let mut config = read_config(&path);

        if let Some(obj) = config.as_object_mut() {
            // LLM provider entry
            if sync_llm {
                let mut provider_entry = serde_json::json!({
                    "npm": "@ai-sdk/openai-compatible",
                    "name": "LocalRouter",
                    "options": {
                        "baseURL": format!("{}/v1", base_url),
                        "apiKey": client_secret
                    }
                });

                // Add models map if provided (from sync_config)
                if let Some(model_list) = models {
                    let mut models_map = serde_json::Map::new();
                    for model_id in model_list {
                        models_map
                            .insert(model_id.clone(), serde_json::json!({ "name": model_id }));
                    }
                    provider_entry["models"] = serde_json::Value::Object(models_map);
                }

                let provider = obj
                    .entry("provider")
                    .or_insert_with(|| serde_json::json!({}));
                if let Some(prov_obj) = provider.as_object_mut() {
                    prov_obj.insert("localrouter".to_string(), provider_entry);
                }
            } else {
                // Remove stale LLM entry
                if let Some(provider) = obj.get_mut("provider") {
                    if let Some(prov_obj) = provider.as_object_mut() {
                        prov_obj.remove("localrouter");
                    }
                }
            }

            // OpenCode uses "mcp" key (not "mcpServers")
            if sync_mcp {
                let mcp = obj.entry("mcp").or_insert_with(|| serde_json::json!({}));
                if let Some(mcp_obj) = mcp.as_object_mut() {
                    mcp_obj.insert(
                        "localrouter".to_string(),
                        mcp_entry(base_url, client_secret),
                    );
                }
            } else {
                // Remove stale MCP entry
                if let Some(mcp) = obj.get_mut("mcp") {
                    if let Some(mcp_obj) = mcp.as_object_mut() {
                        mcp_obj.remove("localrouter");
                    }
                }
            }
        }

        write_config(&path, &config)
    }
}
