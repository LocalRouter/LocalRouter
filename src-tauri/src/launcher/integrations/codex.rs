//! Codex (OpenAI) integration
//!
//! LLM-only client — Codex does not have an MCP client layer.
//! See: <https://developers.openai.com/codex/config-reference/>
//!
//! - **Try It Out**: Terminal command with env vars.

use crate::launcher::AppIntegration;
use crate::ui::commands_clients::{AppCapabilities, LaunchResult};

pub struct CodexIntegration;

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
        // Codex is LLM-only — no config file integration needed.
        // LLM routing is done via env vars at launch time.
        false
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
        _base_url: &str,
        _client_secret: &str,
        _client_id: &str,
    ) -> Result<LaunchResult, String> {
        Err("Codex does not support permanent configuration — use env vars instead".to_string())
    }
}
