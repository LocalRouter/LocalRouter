//! Claude Code integration
//!
//! Two modes:
//! - **Try It Out**: Terminal command with env vars + inline `--mcp-config` JSON. Zero file changes.
//! - **Permanent Config**: Write MCP server entry to `~/.claude.json`.

use crate::launcher::backup;
use crate::launcher::{AppIntegration, ConfigSyncContext};
use crate::ui::commands_clients::{AppCapabilities, LaunchResult};

pub struct ClaudeCodeIntegration;

/// Path to Claude's global settings file
fn mcp_config_path() -> std::path::PathBuf {
    dirs::home_dir().unwrap_or_default().join(".claude.json")
}

/// Path to Claude Code's `settings.json` (per the network-config docs), where
/// the `env` block for the inspection proxy lives.
// TODO(https-proxy): used by the automated proxy sync writer (follow-up); the
// manual one-off command path is wired today.
#[allow(dead_code)]
pub fn settings_json_path() -> std::path::PathBuf {
    dirs::home_dir()
        .unwrap_or_default()
        .join(".claude")
        .join("settings.json")
}

/// Build the one-off terminal command that launches Claude Code through the
/// inspection proxy, trusting the proxy's root CA. This is the manual
/// instruction shown for a client in a proxy LLM mode.
///
/// Per <https://code.claude.com/docs/en/network-config.md>: `HTTPS_PROXY`
/// carries the proxy URL (with Basic auth) and `NODE_EXTRA_CA_CERTS` points at
/// the CA to trust.
pub fn proxy_oneoff_command(proxy_url: &str, ca_cert_path: &str) -> String {
    format!("HTTPS_PROXY={proxy_url} NODE_EXTRA_CA_CERTS={ca_cert_path} claude")
}

/// The `settings.json` fragment that configures the proxy permanently — the
/// `env` block Claude Code reads on startup.
pub fn proxy_settings_json(proxy_url: &str, ca_cert_path: &str) -> serde_json::Value {
    serde_json::json!({
        "env": {
            "HTTPS_PROXY": proxy_url,
            "NODE_EXTRA_CA_CERTS": ca_cert_path,
        }
    })
}

/// Merge the proxy `env` keys into an existing (or empty) `settings.json` value,
/// preserving any other settings/env the user already has.
// TODO(https-proxy): used by the automated proxy sync writer (follow-up).
#[allow(dead_code)]
pub fn merge_proxy_settings(
    mut existing: serde_json::Value,
    proxy_url: &str,
    ca_cert_path: &str,
) -> serde_json::Value {
    if !existing.is_object() {
        existing = serde_json::json!({});
    }
    let obj = existing.as_object_mut().expect("object");
    let env = obj.entry("env").or_insert_with(|| serde_json::json!({}));
    if let Some(env_obj) = env.as_object_mut() {
        env_obj.insert("HTTPS_PROXY".to_string(), proxy_url.into());
        env_obj.insert("NODE_EXTRA_CA_CERTS".to_string(), ca_cert_path.into());
    }
    existing
}

impl AppIntegration for ClaudeCodeIntegration {
    fn name(&self) -> &str {
        "Claude Code"
    }

    fn check_installed(&self) -> AppCapabilities {
        let binary = super::find_binary("claude").or_else(|| {
            let home = dirs::home_dir().unwrap_or_default();
            let local_path = home.join(".claude/local/claude");
            local_path.exists().then_some(local_path)
        });

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
        // Include token in URL query param as fallback for clients that don't send custom headers
        let mcp_url = format!("{}?token={}", base_url, client_secret);
        let mcp_config = serde_json::json!({
            "mcpServers": {
                "localrouter": {
                    "type": "http",
                    "url": mcp_url,
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

        // Include token in URL query param as fallback for clients that don't send custom headers
        let mcp_url = format!("{}?token={}", base_url, client_secret);
        let mcp_entry = serde_json::json!({
            "type": "http",
            "url": mcp_url,
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

    fn sync_config(&self, ctx: &ConfigSyncContext) -> Result<LaunchResult, String> {
        // Claude Code permanent config only writes MCP entries.
        // In mcp_via_llm/llm_only modes, remove stale MCP entry (LLM uses env vars).
        if !ctx.should_sync_mcp() {
            let path = mcp_config_path();
            if path.exists() {
                let data = std::fs::read_to_string(&path)
                    .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;
                let mut config: serde_json::Value =
                    serde_json::from_str(&data).unwrap_or(serde_json::json!({}));

                let removed = config
                    .as_object_mut()
                    .and_then(|obj| obj.get_mut("mcpServers"))
                    .and_then(|s| s.as_object_mut())
                    .and_then(|servers| servers.remove("localrouter"))
                    .is_some();

                if removed {
                    let out = serde_json::to_string_pretty(&config)
                        .map_err(|e| format!("Failed to serialize config: {}", e))?;
                    let backup_path = backup::write_with_backup(&path, out.as_bytes())?;
                    return Ok(LaunchResult {
                        success: true,
                        message: format!("Removed MCP entry from {}", path.display()),
                        modified_files: vec![path.to_string_lossy().to_string()],
                        backup_files: backup_path
                            .iter()
                            .map(|p| p.to_string_lossy().to_string())
                            .collect(),
                        terminal_command: None,
                    });
                }
            }
            return Ok(LaunchResult {
                success: true,
                message: "No config to sync for current client mode (LLM uses env vars)"
                    .to_string(),
                modified_files: vec![],
                backup_files: vec![],
                terminal_command: None,
            });
        }
        self.configure_permanent(&ctx.base_url, &ctx.client_secret, &ctx.client_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_proxy_oneoff_command() {
        let cmd = proxy_oneoff_command(
            "http://cid:sec@127.0.0.1:3626",
            "/home/u/.localrouter/proxy/root-ca.pem",
        );
        assert_eq!(
            cmd,
            "HTTPS_PROXY=http://cid:sec@127.0.0.1:3626 NODE_EXTRA_CA_CERTS=/home/u/.localrouter/proxy/root-ca.pem claude"
        );
    }

    #[test]
    fn proxy_settings_json_has_env_block() {
        let v = proxy_settings_json("http://p", "/ca.pem");
        assert_eq!(v["env"]["HTTPS_PROXY"], "http://p");
        assert_eq!(v["env"]["NODE_EXTRA_CA_CERTS"], "/ca.pem");
    }

    #[test]
    fn merge_preserves_existing_settings_and_env() {
        let existing = serde_json::json!({
            "theme": "dark",
            "env": { "FOO": "bar" }
        });
        let merged = merge_proxy_settings(existing, "http://p", "/ca.pem");
        // Unrelated settings survive.
        assert_eq!(merged["theme"], "dark");
        assert_eq!(merged["env"]["FOO"], "bar");
        // Proxy keys added.
        assert_eq!(merged["env"]["HTTPS_PROXY"], "http://p");
        assert_eq!(merged["env"]["NODE_EXTRA_CA_CERTS"], "/ca.pem");
    }

    #[test]
    fn merge_into_empty_creates_env() {
        let merged = merge_proxy_settings(serde_json::json!(null), "http://p", "/ca.pem");
        assert_eq!(merged["env"]["HTTPS_PROXY"], "http://p");
    }
}
