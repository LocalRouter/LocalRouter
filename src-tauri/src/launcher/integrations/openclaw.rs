//! OpenClaw integration
//!
//! Config-file only (no try-it-out). Writes TWO files:
//! - **LLM**: `models.providers.localrouter` in `~/.openclaw/openclaw.json`.
//! - **MCP**: `mcpServers.localrouter` in `~/.openclaw/config/mcporter.json`
//!   (requires the MCPorter skill: https://clawhub.ai/steipete/mcporter).

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

fn mcporter_config_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_default()
        .join(".openclaw")
        .join("config")
        .join("mcporter.json")
}

impl AppIntegration for OpenClawIntegration {
    fn name(&self) -> &str {
        "OpenClaw"
    }

    fn check_installed(&self) -> AppCapabilities {
        // Audit: only 'openclaw' is documented in public docs.
        // See: https://docs.openclaw.ai/cli/models
        let binary = super::find_binary("openclaw");

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
        let llm_path = config_path();
        let mcp_path = mcporter_config_path();

        // Nothing to write or clean up
        if !sync_llm && !sync_mcp && !llm_path.exists() && !mcp_path.exists() {
            return Ok(LaunchResult {
                success: true,
                message: "No config to sync for current client mode".to_string(),
                modified_files: vec![],
                backup_files: vec![],
                terminal_command: None,
            });
        }

        let mut modified_files = vec![];
        let mut all_backup_files = vec![];
        let mut parts = vec![];

        // 1. LLM config in ~/.openclaw/openclaw.json
        {
            let mut config: serde_json::Value = if llm_path.exists() {
                let data = std::fs::read_to_string(&llm_path)
                    .map_err(|e| format!("Failed to read config: {}", e))?;
                serde_json::from_str(&data).unwrap_or(serde_json::json!({}))
            } else {
                serde_json::json!({})
            };

            let obj = config.as_object_mut().ok_or("Invalid config format")?;
            let mut changed = false;

            if sync_llm {
                let models_section = obj.entry("models").or_insert_with(|| serde_json::json!({}));
                let providers = models_section
                    .as_object_mut()
                    .ok_or("Invalid models section")?
                    .entry("providers")
                    .or_insert_with(|| serde_json::json!({}));

                // Snapshot whether the user already had other providers
                // configured before we add our own. This drives whether
                // we're allowed to claim `agents.defaults.model.primary`
                // below — a first-time install wants LocalRouter wired
                // up as the default, but adding LocalRouter to an
                // existing multi-provider setup should respect the
                // user's current primary model.
                let had_other_providers = providers
                    .as_object()
                    .map(|m| m.keys().any(|k| k != "localrouter"))
                    .unwrap_or(false);

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

                // Only claim the default primary-model slot when LocalRouter
                // is the sole provider. If the user already wired up another
                // provider (openai, anthropic, …), leave their existing
                // `agents.defaults.model.primary` untouched.
                if !had_other_providers {
                    let agents_section =
                        obj.entry("agents").or_insert_with(|| serde_json::json!({}));
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
                        .insert("primary".to_string(), serde_json::json!("localrouter/auto"));
                }

                parts.push(format!("LLM provider at {}", llm_path.display()));
            } else {
                // Remove stale LLM entry
                if let Some(models_val) = obj.get_mut("models") {
                    if let Some(providers) = models_val
                        .as_object_mut()
                        .and_then(|m| m.get_mut("providers"))
                    {
                        if let Some(prov_obj) = providers.as_object_mut() {
                            if prov_obj.remove("localrouter").is_some() {
                                changed = true;
                                parts.push(format!(
                                    "removed LLM provider from {}",
                                    llm_path.display()
                                ));
                            }
                        }
                    }
                }
            }

            if changed {
                let data = serde_json::to_string_pretty(&config)
                    .map_err(|e| format!("Failed to serialize config: {}", e))?;
                let backup_path = backup::write_with_backup(&llm_path, data.as_bytes())?;
                modified_files.push(llm_path.to_string_lossy().to_string());
                if let Some(bp) = backup_path {
                    all_backup_files.push(bp.to_string_lossy().to_string());
                }
            }
        }

        // 2. MCP config in ~/.openclaw/config/mcporter.json (requires MCPorter skill)
        {
            if mcp_path.exists() || sync_mcp {
                let mut config: serde_json::Value = if mcp_path.exists() {
                    let data = std::fs::read_to_string(&mcp_path)
                        .map_err(|e| format!("Failed to read MCPorter config: {}", e))?;
                    serde_json::from_str(&data).unwrap_or(serde_json::json!({}))
                } else {
                    serde_json::json!({})
                };

                let mut changed = false;

                if sync_mcp {
                    let obj = config
                        .as_object_mut()
                        .ok_or("Invalid MCPorter config format")?;
                    let servers = obj
                        .entry("mcpServers")
                        .or_insert_with(|| serde_json::json!({}));

                    let mcp_entry = serde_json::json!({
                        "baseUrl": base_url,
                        "headers": {
                            "Authorization": format!("Bearer {}", client_secret)
                        }
                    });

                    if let Some(servers_obj) = servers.as_object_mut() {
                        servers_obj.insert("localrouter".to_string(), mcp_entry);
                        changed = true;
                    }

                    // Ensure imports array exists
                    obj.entry("imports")
                        .or_insert_with(|| serde_json::json!([]));

                    parts.push(format!("MCP server at {}", mcp_path.display()));
                } else {
                    // Remove stale MCP entry
                    if let Some(obj) = config.as_object_mut() {
                        if let Some(servers) = obj.get_mut("mcpServers") {
                            if let Some(servers_obj) = servers.as_object_mut() {
                                if servers_obj.remove("localrouter").is_some() {
                                    changed = true;
                                    parts.push(format!(
                                        "removed MCP server from {}",
                                        mcp_path.display()
                                    ));
                                }
                            }
                        }
                    }
                }

                if changed {
                    // Ensure parent directory exists
                    if let Some(parent) = mcp_path.parent() {
                        std::fs::create_dir_all(parent)
                            .map_err(|e| format!("Failed to create config dir: {}", e))?;
                    }
                    let data = serde_json::to_string_pretty(&config)
                        .map_err(|e| format!("Failed to serialize MCPorter config: {}", e))?;
                    let backup_path = backup::write_with_backup(&mcp_path, data.as_bytes())?;
                    modified_files.push(mcp_path.to_string_lossy().to_string());
                    if let Some(bp) = backup_path {
                        all_backup_files.push(bp.to_string_lossy().to_string());
                    }
                }
            }
        }

        if modified_files.is_empty() {
            return Ok(LaunchResult {
                success: true,
                message: "No config changes needed".to_string(),
                modified_files: vec![],
                backup_files: vec![],
                terminal_command: None,
            });
        }

        Ok(LaunchResult {
            success: true,
            message: format!("Configured OpenClaw: {}", parts.join(", ")),
            modified_files,
            backup_files: all_backup_files,
            terminal_command: None,
        })
    }
}
