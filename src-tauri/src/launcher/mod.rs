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

use crate::ui::commands_clients::{AppCapabilities, LaunchResult};

/// Trait for all app integrations
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
}

/// Registry of all known integrations
pub fn get_integration(template_id: &str) -> Option<Box<dyn AppIntegration>> {
    integrations::get_integration(template_id)
}
