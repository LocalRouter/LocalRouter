//! System tray management
//!
//! Handles system tray icon and menu.

use tauri::{App, Result};
use tracing::info;

pub fn setup_tray(_app: &mut App) -> Result<()> {
    info!("Setting up system tray");

    // TODO: Implement system tray
    // - Create tray icon
    // - Build dynamic menu
    // - Handle menu events
    // - Update menu on config change

    Ok(())
}
