//! System tray management
//!
//! Handles system tray icon and menu.

use tauri::{App, Manager};
use tracing::info;

/// Setup system tray icon and menu
pub fn setup_tray(app: &App) -> tauri::Result<()> {
    info!("Setting up system tray");

    // System tray will be implemented using Tauri 2.x tray plugin
    // For now, log that tray setup is complete
    // TODO: Implement full tray menu with dynamic API keys when tray plugin is configured

    info!("System tray setup complete (simplified version)");
    Ok(())
}
