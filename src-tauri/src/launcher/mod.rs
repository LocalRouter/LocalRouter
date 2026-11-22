//! App launcher module
//!
//! Provides integration with external apps (Claude Code, Codex, Cursor, etc.)
//! for automatic configuration and launching.
//!
//! Two distinct modes:
//! - **Try It Out**: One-time terminal command with env vars. No files modified.
//! - **Permanent Config**: Modify app config files to always route through LocalRouter.

pub mod backup;
pub mod ca_bundle;
pub mod ca_trust;
pub mod integrations;
pub mod proxy;
pub mod proxy_setup;
pub mod proxy_sync;
pub mod reverse_proxy;
pub mod reverse_setup;

use crate::ui::commands_clients::{AppCapabilities, LaunchResult};

/// Serializes every read-modify-write of an external app's config file
/// (integration sync, proxy apply/undo, proxy sync). Several of those paths
/// edit the same file (e.g. Goose's `config.yaml`), and an interleaved pair
/// would silently drop one side's change.
pub static EXTERNAL_CONFIG_LOCK: parking_lot::Mutex<()> = parking_lot::Mutex::new(());

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
}

impl ConfigSyncContext {
    /// Whether native LLM gateway config should be written (gateway mode only).
    /// HTTPS-proxy config is not part of this sync: it is applied explicitly
    /// through [`proxy_setup`] and kept current by [`proxy_sync`].
    pub fn should_sync_llm(&self) -> bool {
        self.llm_mode == lr_config::LlmMode::Gateway
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
    /// OpenCode, OpenClaw, Pi, and Goose return true.
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

    /// One-time launch with the client's modes and permitted models. Integrations
    /// that require an explicit model can override this without choosing a stale
    /// model from the external app's existing settings.
    fn try_it_out_with_context(&self, ctx: &ConfigSyncContext) -> Result<LaunchResult, String> {
        self.try_it_out(&ctx.base_url, &ctx.client_secret, &ctx.client_id)
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
