//! System tray management
//!
//! Handles system tray icon and menu.

use tauri::{
    menu::{MenuBuilder, MenuItemBuilder, SubmenuBuilder},
    tray::TrayIconBuilder,
    App, Manager, Runtime,
};
use tracing::{error, info};

/// Setup system tray icon and menu
pub fn setup_tray<R: Runtime>(app: &App<R>) -> tauri::Result<()> {
    info!("Setting up system tray");

    // Build the tray menu
    let menu = build_tray_menu(app)?;

    // Create the tray icon
    let _tray = TrayIconBuilder::new()
        .icon(app.default_window_icon().unwrap().clone())
        .menu(&menu)
        .tooltip("LocalRouter AI")
        .on_menu_event(|app, event| {
            let id = event.id().as_ref();
            info!("Tray menu event: {}", id);

            match id {
                "open_dashboard" => {
                    if let Some(window) = app.get_webview_window("main") {
                        let _ = window.show();
                        let _ = window.set_focus();
                    }
                }
                "generate_key" => {
                    info!("Generate new key requested");
                    // TODO: Show dialog to generate new key
                }
                "preferences" => {
                    info!("Preferences requested");
                    // TODO: Show preferences window
                }
                "quit" => {
                    info!("Quit requested from tray");
                    app.exit(0);
                }
                _ => {
                    info!("Unhandled menu event: {}", id);
                }
            }
        })
        .build(app)?;

    info!("System tray initialized successfully");
    Ok(())
}

/// Build the system tray menu
fn build_tray_menu<R: Runtime>(app: &App<R>) -> tauri::Result<tauri::menu::Menu<R>> {
    // Build API key submenu
    let key_submenu = SubmenuBuilder::new(app, "my-app-1")
        .text("key1_router_mincost", "Router: Minimum Cost ✓")
        .text("key1_router_maxperf", "Router: Maximum Performance")
        .separator()
        .text("key1_ollama_llama", "Ollama/llama3.3")
        .text("key1_openai_gpt4", "OpenAI/gpt-4")
        .build()?;

    // Build main menu
    let menu = MenuBuilder::new(app)
        .item(&key_submenu)
        .text("generate_key", "➕ Generate New Key...")
        .separator()
        .text("open_dashboard", "📊 Open Dashboard")
        .text("preferences", "⚙️ Preferences")
        .separator()
        .text("quit", "❌ Quit")
        .build()?;

    Ok(menu)
}
