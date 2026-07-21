//! App launcher module
//!
//! Provides integration with external apps (Claude Code, Codex, Cursor, etc.)
//! for automatic configuration and launching.
//!
//! Two distinct modes:
//! - **Try It Out**: One-time terminal command with env vars. No files modified.
//! - **Permanent Config**: Modify app config files to always route through LocalRouter.

pub mod backup;
pub mod integrations;
pub mod proxy;

use crate::ui::commands_clients::{AppCapabilities, LaunchResult};

/// Context for syncing external app config files
pub struct ConfigSyncContext {
    pub base_url: String,
    pub client_secret: String,
    pub client_id: String,
    /// Model IDs available to this client (e.g. "anthropic/claude-sonnet-4-20250514")
    pub models: Vec<String>,
    /// Current LLM access mode — controls whether native or proxy config is synced.
    pub llm_mode: lr_config::LlmMode,
    /// Current MCP access mode — controls whether MCP config is synced.
    pub mcp_mode: lr_config::McpMode,
    /// HTTPS inspection-proxy URL (with embedded basic-auth), when the proxy is
    /// running. `None` disables proxy sync even in a proxy `llm_mode`.
    // TODO(https-proxy): consumed by the Claude Code proxy setup writer (Part E).
    #[allow(dead_code)]
    pub proxy_url: Option<String>,
    /// Path to the proxy root CA the client must trust (`NODE_EXTRA_CA_CERTS`).
    // TODO(https-proxy): consumed by the Claude Code proxy setup writer (Part E).
    #[allow(dead_code)]
    pub ca_cert_path: Option<String>,
}

impl ConfigSyncContext {
    /// Whether native LLM gateway config should be written (gateway mode only).
    /// Proxy modes use [`Self::should_sync_proxy`] instead.
    pub fn should_sync_llm(&self) -> bool {
        self.llm_mode == lr_config::LlmMode::Gateway
    }

    /// Whether HTTPS-proxy env config should be written (a proxy `llm_mode`
    /// with the proxy actually running).
    // TODO(https-proxy): called by the Claude Code proxy setup writer (Part E).
    #[allow(dead_code)]
    pub fn should_sync_proxy(&self) -> bool {
        matches!(
            self.llm_mode,
            lr_config::LlmMode::ProxyInspect | lr_config::LlmMode::ProxyRewrite
        ) && self.proxy_url.is_some()
    }

    /// Whether MCP config should be written (direct MCP gateway only;
    /// mcp_via_llm handles MCP server-side so the external app shouldn't connect directly)
    pub fn should_sync_mcp(&self) -> bool {
        self.mcp_mode == lr_config::McpMode::Gateway
    }
}

/// Trait for all app integrations
#[allow(dead_code)]
pub trait AppIntegration: Send + Sync {
    /// Human-readable name
    fn name(&self) -> &str;

    /// Check if the app binary is installed and return its path/version
    fn check_installed(&self) -> AppCapabilities;

    /// Whether this integration supports "Try It Out" (one-time terminal command)
    fn supports_try_it_out(&self) -> bool {
        false
    }

    /// Whether this integration supports permanent config file modification
    fn supports_permanent_config(&self) -> bool {
        false
    }

    /// Whether this integration needs the model list for sync_config.
    /// Only OpenCode returns true.
    fn needs_model_list(&self) -> bool {
        false
    }

    /// One-time terminal command. No permanent file changes.
    fn try_it_out(
        &self,
        _base_url: &str,
        _client_secret: &str,
        _client_id: &str,
    ) -> Result<LaunchResult, String> {
        Err("Try It Out is not supported for this app".to_string())
    }

    /// Permanently modify config files to route through LocalRouter.
    fn configure_permanent(
        &self,
        _base_url: &str,
        _client_secret: &str,
        _client_id: &str,
    ) -> Result<LaunchResult, String> {
        Err("Permanent configuration is not supported for this app".to_string())
    }

    /// Sync config files with current state (models, secrets, URL).
    /// Default delegates to configure_permanent.
    fn sync_config(&self, ctx: &ConfigSyncContext) -> Result<LaunchResult, String> {
        self.configure_permanent(&ctx.base_url, &ctx.client_secret, &ctx.client_id)
    }
}

/// Registry of all known integrations
pub fn get_integration(template_id: &str) -> Option<Box<dyn AppIntegration>> {
    integrations::get_integration(template_id)
}
