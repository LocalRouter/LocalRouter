//! Codex (OpenAI) integration
//!
//! Two modes:
//! - **Try It Out**: Terminal command with env vars (LLM only, no MCP).
//! - **Permanent Config**: Write MCP servers to `~/.codex/config.toml`.

use crate::launcher::backup;
use crate::launcher::AppIntegration;
use crate::ui::commands_clients::{AppCapabilities, LaunchResult};

pub struct CodexIntegration;

fn mcp_config_path() -> std::path::PathBuf {
    dirs::home_dir()
        .unwrap_or_default()
        .join(".codex")
        .join("config.toml")
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
        // LLM only via env vars, no MCP in try-it-out mode
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
        let path = mcp_config_path();

        // Read existing file content
        let existing_content = if path.exists() {
            std::fs::read_to_string(&path)
                .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?
        } else {
            String::new()
        };

        // Build the new [mcp_servers.localrouter] section
        let new_section = format!(
            "[mcp_servers.localrouter]\nurl = \"{}\"\nheaders = {{ Authorization = \"Bearer {}\" }}",
            base_url, client_secret
        );

        // Replace existing section or append
        let updated_content = if existing_content.contains("[mcp_servers.localrouter]") {
            // Find and replace the existing section
            let mut result = String::new();
            let mut skip = false;
            for line in existing_content.lines() {
                if line.trim() == "[mcp_servers.localrouter]" {
                    skip = true;
                    result.push_str(&new_section);
                    result.push('\n');
                    continue;
                }
                // Stop skipping when we hit the next section header
                if skip && line.starts_with('[') {
                    skip = false;
                }
                if !skip {
                    result.push_str(line);
                    result.push('\n');
                }
            }
            result
        } else if existing_content.is_empty() {
            format!("{}\n", new_section)
        } else {
            format!("{}\n{}\n", existing_content.trim_end(), new_section)
        };

        let backup_path = backup::write_with_backup(&path, updated_content.as_bytes())?;
        let backup_files: Vec<String> = backup_path
            .iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect();

        Ok(LaunchResult {
            success: true,
            message: format!("MCP configured in {}", path.display()),
            modified_files: vec![path.to_string_lossy().to_string()],
            backup_files,
            terminal_command: None,
        })
    }
}
