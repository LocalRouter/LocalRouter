//! Codex (OpenAI) integration
//!
//! Two modes:
//! - **Try It Out**: Terminal command with env vars (LLM routing).
//! - **Permanent Config**: Write MCP server entry to `~/.codex/config.toml`.
//!
//! See: <https://developers.openai.com/codex/config-reference/>

use crate::launcher::backup;
use crate::launcher::AppIntegration;
use crate::ui::commands_clients::{AppCapabilities, LaunchResult};

pub struct CodexIntegration;

/// Path to Codex global config file
fn config_path() -> std::path::PathBuf {
    dirs::home_dir()
        .unwrap_or_default()
        .join(".codex")
        .join("config.toml")
}

/// Read the existing config.toml or create an empty table
fn read_config(path: &std::path::Path) -> toml::Value {
    if path.exists() {
        let data = std::fs::read_to_string(path).unwrap_or_default();
        data.parse::<toml::Value>()
            .unwrap_or(toml::Value::Table(toml::map::Map::new()))
    } else {
        toml::Value::Table(toml::map::Map::new())
    }
}

/// Insert the LocalRouter MCP server entry into the config
fn insert_mcp_entry(config: &mut toml::Value, base_url: &str, client_secret: &str) {
    if let toml::Value::Table(ref mut table) = config {
        let mcp_servers = table
            .entry("mcp_servers")
            .or_insert(toml::Value::Table(toml::map::Map::new()));
        if let toml::Value::Table(ref mut servers) = mcp_servers {
            let mut entry = toml::map::Map::new();
            entry.insert("url".to_string(), toml::Value::String(base_url.to_string()));

            let mut headers = toml::map::Map::new();
            headers.insert(
                "Authorization".to_string(),
                toml::Value::String(format!("Bearer {}", client_secret)),
            );
            entry.insert("http_headers".to_string(), toml::Value::Table(headers));

            servers.insert("localrouter".to_string(), toml::Value::Table(entry));
        }
    }
}

/// Write config TOML to disk with backup
fn write_config(
    path: &std::path::Path,
    config: &toml::Value,
) -> Result<LaunchResult, String> {
    let data =
        toml::to_string_pretty(config).map_err(|e| format!("Failed to serialize config: {}", e))?;

    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create {}: {}", parent.display(), e))?;
    }

    let backup_path = backup::write_with_backup(path, data.as_bytes())?;
    let backup_files: Vec<String> = backup_path
        .iter()
        .map(|p| p.to_string_lossy().to_string())
        .collect();

    Ok(LaunchResult {
        success: true,
        message: format!(
            "MCP configured in {}. Restart Codex or run `codex mcp list` to verify.",
            path.display()
        ),
        modified_files: vec![path.to_string_lossy().to_string()],
        backup_files,
        terminal_command: None,
    })
}

impl AppIntegration for CodexIntegration {
    fn name(&self) -> &str {
        "Codex"
    }

    fn check_installed(&self) -> AppCapabilities {
        let binary = which::which("codex").ok();

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
                "OPENAI_BASE_URL={} OPENAI_API_KEY={} codex --oss",
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
        let mut config = read_config(&path);

        insert_mcp_entry(&mut config, base_url, client_secret);

        write_config(&path, &config)
    }
}
