//! Claude Code integration
//!
//! Two modes:
//! - **Try It Out**: Terminal command with env vars + inline `--mcp-config` JSON. Zero file changes.
//! - **Permanent Config**: Write MCP server entry to `~/.claude.json`.

use crate::launcher::backup;
use crate::launcher::AppIntegration;
use crate::ui::commands_clients::{AppCapabilities, LaunchResult};

pub struct ClaudeCodeIntegration;

/// Path to Claude's global settings file
fn mcp_config_path() -> std::path::PathBuf {
    dirs::home_dir().unwrap_or_default().join(".claude.json")
}

impl AppIntegration for ClaudeCodeIntegration {
    fn name(&self) -> &str {
        "Claude Code"
    }

    fn check_installed(&self) -> AppCapabilities {
        let binary = which::which("claude")
            .or_else(|_| {
                let home = dirs::home_dir().unwrap_or_default();
                let local_path = home.join(".claude/local/claude");
                if local_path.exists() {
                    Ok(local_path)
                } else {
                    Err(which::Error::CannotFindBinaryPath)
                }
            })
            .ok();

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
        // Build inline MCP config JSON for --mcp-config flag
        let mcp_config = serde_json::json!({
            "mcpServers": {
                "localrouter": {
                    "type": "http",
                    "url": base_url,
                    "headers": {
                        "Authorization": format!("Bearer {}", client_secret)
                    }
                }
            }
        });

        let mcp_json = serde_json::to_string(&mcp_config)
            .map_err(|e| format!("Failed to serialize MCP config: {}", e))?;

        Ok(LaunchResult {
            success: true,
            message: "Run the command below in your terminal:".to_string(),
            modified_files: vec![],
            backup_files: vec![],
            terminal_command: Some(format!(
                "ANTHROPIC_BASE_URL={} ANTHROPIC_API_KEY={} claude --mcp-config '{}'",
                base_url, client_secret, mcp_json
            )),
        })
    }

    fn configure_permanent(
        &self,
        base_url: &str,
        client_secret: &str,
        _client_id: &str,
    ) -> Result<LaunchResult, String> {
        // Configure MCP: add LocalRouter as an MCP server in ~/.claude.json
        let path = mcp_config_path();

        let mut config: serde_json::Value = if path.exists() {
            let data = std::fs::read_to_string(&path)
                .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;
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

        if let Some(obj) = config.as_object_mut() {
            let mcp_servers = obj
                .entry("mcpServers")
                .or_insert_with(|| serde_json::json!({}));
            if let Some(servers_obj) = mcp_servers.as_object_mut() {
                servers_obj.insert("localrouter".to_string(), mcp_entry);
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
            message: format!(
                "MCP configured in {}. For LLM routing, use env vars at launch time.",
                path.display()
            ),
            modified_files: vec![path.to_string_lossy().to_string()],
            backup_files,
            terminal_command: None,
        })
    }
}
