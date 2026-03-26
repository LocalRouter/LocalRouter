//! OpenClaw integration
//!
//! Config-file only (no try-it-out).
//! - **LLM**: `models.providers.localrouter` in `~/.openclaw/openclaw.json`.
//! - **MCP**: `mcp.servers.localrouter` in the same file.

use crate::launcher::backup;
use crate::launcher::{AppIntegration, ConfigSyncContext};
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
        // Audit: only 'openclaw' is documented in public docs.
        // See: https://docs.openclaw.ai/cli/models
        let binary = which::which("openclaw").ok();

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

impl OpenClawIntegration {
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

        let mut config: serde_json::Value = if path.exists() {
            let data = std::fs::read_to_string(&path)
                .map_err(|e| format!("Failed to read config: {}", e))?;
            serde_json::from_str(&data).unwrap_or(serde_json::json!({}))
        } else {
            serde_json::json!({})
        };

        let obj = config.as_object_mut().ok_or("Invalid config format")?;
        let mut parts = vec![];
        let mut changed = false;

        // 1. LLM provider entry under models.providers
        if sync_llm {
            let models_section = obj.entry("models").or_insert_with(|| serde_json::json!({}));
            let providers = models_section
                .as_object_mut()
                .ok_or("Invalid models section")?
                .entry("providers")
                .or_insert_with(|| serde_json::json!({}));

            let model_ids = models
                .cloned()
                .unwrap_or_else(|| vec!["localrouter/auto".to_string()]);
            let model_entries: Vec<serde_json::Value> = model_ids
                .iter()
                .map(|id| {
                    serde_json::json!({
                        "id": id,
                        "name": id
                    })
                })
                .collect();

            let provider_entry = serde_json::json!({
                "baseUrl": base_url,
                "apiKey": client_secret,
                "api": "openai-completions",
                "models": model_entries
            });

            if let Some(prov_obj) = providers.as_object_mut() {
                prov_obj.insert("localrouter".to_string(), provider_entry);
                changed = true;
            }

            // Set default model to autorouter under agents.defaults.model.primary
            let agents_section = obj.entry("agents").or_insert_with(|| serde_json::json!({}));
            let defaults_section = agents_section
                .as_object_mut()
                .ok_or("Invalid agents section")?
                .entry("defaults")
                .or_insert_with(|| serde_json::json!({}));
            let model_section = defaults_section
                .as_object_mut()
                .ok_or("Invalid agents.defaults section")?
                .entry("model")
                .or_insert_with(|| serde_json::json!({}));
            model_section
                .as_object_mut()
                .ok_or("Invalid agents.defaults.model section")?
                .insert(
                    "primary".to_string(),
                    serde_json::json!("localrouter:localrouter/auto"),
                );

            parts.push("LLM provider + default model");
        } else {
            // Remove stale LLM entry
            if let Some(models) = obj.get_mut("models") {
                if let Some(providers) = models.as_object_mut().and_then(|m| m.get_mut("providers"))
                {
                    if let Some(prov_obj) = providers.as_object_mut() {
                        if prov_obj.remove("localrouter").is_some() {
                            changed = true;
                            parts.push("removed LLM provider");
                        }
                    }
                }
            }
        }

        // 2. MCP server entry under mcp.servers
        if sync_mcp {
            let mcp_section = obj.entry("mcp").or_insert_with(|| serde_json::json!({}));
            let mcp_servers = mcp_section
                .as_object_mut()
                .ok_or("Invalid mcp section")?
                .entry("servers")
                .or_insert_with(|| serde_json::json!({}));

            let mcp_entry = serde_json::json!({
                "url": base_url,
                "headers": {
                    "Authorization": format!("Bearer {}", client_secret)
                }
            });

            if let Some(servers_obj) = mcp_servers.as_object_mut() {
                servers_obj.insert("localrouter".to_string(), mcp_entry);
                changed = true;
            }
            parts.push("MCP server");
        } else {
            // Remove stale MCP entry
            if let Some(mcp) = obj.get_mut("mcp") {
                if let Some(servers) = mcp.as_object_mut().and_then(|m| m.get_mut("servers")) {
                    if let Some(servers_obj) = servers.as_object_mut() {
                        if servers_obj.remove("localrouter").is_some() {
                            changed = true;
                            parts.push("removed MCP server");
                        }
                    }
                }
            }
        }

        if !changed {
            return Ok(LaunchResult {
                success: true,
                message: "No config changes needed".to_string(),
                modified_files: vec![],
                backup_files: vec![],
                terminal_command: None,
            });
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
            message: format!(
                "Configured OpenClaw: {} at {}",
                parts.join(" and "),
                path.display()
            ),
            modified_files: vec![path.to_string_lossy().to_string()],
            backup_files,
            terminal_command: None,
        })
    }
}
