//! System tray management
//!
//! Handles system tray icon and menu.

use crate::api_keys::ApiKeyManager;
use crate::config::ModelSelection;
use tauri::{
    menu::MenuBuilder,
    tray::TrayIconBuilder,
    App, AppHandle, Manager, Runtime,
};
use tracing::info;

/// Setup system tray icon and menu
pub fn setup_tray<R: Runtime>(app: &App<R>) -> tauri::Result<()> {
    info!("Setting up system tray");

    // Build the tray menu
    let menu = build_tray_menu(app)?;

    // Create the tray icon
    let _tray = TrayIconBuilder::with_id("main")
        .icon(app.default_window_icon().unwrap().clone())
        .menu(&menu)
        .tooltip("LocalRouter AI")
        .icon_as_template(true)
        .show_menu_on_left_click(false)
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
                    if let Some(window) = app.get_webview_window("main") {
                        let _ = window.show();
                        let _ = window.set_focus();
                        // TODO: Emit event to open create key modal
                    }
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
                    // Handle API key selection
                    if id.starts_with("key_") {
                        info!("API key menu item clicked: {}", id);
                        // Format: key_{api_key_id}_{provider}_{model}
                        // TODO: Set active API key/model combination
                    }
                }
            }
        })
        .build(app)?;

    info!("System tray initialized successfully");
    Ok(())
}

/// Build the system tray menu
fn build_tray_menu<R: Runtime>(app: &App<R>) -> tauri::Result<tauri::menu::Menu<R>> {
    let mut menu_builder = MenuBuilder::new(app);

    // Get API keys from manager
    if let Some(key_manager) = app.try_state::<ApiKeyManager>() {
        let keys = key_manager.list_keys();

        if !keys.is_empty() {
            // Build a submenu for each API key
            for key in keys.iter().filter(|k| k.enabled) {
                let key_name = if key.name.is_empty() {
                    format!("Key {}", &key.id[..8])
                } else {
                    key.name.clone()
                };

                // Get the model selection for this key
                let model_info = match &key.model_selection {
                    // ModelSelection::Router { router_name } => {
                    //     format!("Router: {}", router_name)
                    // }
                    ModelSelection::DirectModel { provider, model } => {
                        format!("{}/{}", provider, model)
                    }
                    _ => "Unknown".to_string(),
                };

                // For now, just show a text item with the key and its model
                menu_builder = menu_builder.text(
                    format!("key_{}", key.id),
                    format!("{} → {}", key_name, model_info),
                );
            }

            menu_builder = menu_builder.separator();
        }
    }

    // Add standard menu items
    menu_builder = menu_builder
        .text("generate_key", "➕ Generate New Key...")
        .separator()
        .text("open_dashboard", "📊 Open Dashboard")
        .text("preferences", "⚙️ Preferences")
        .separator()
        .text("quit", "❌ Quit");

    menu_builder.build()
}

/// Rebuild the system tray menu with updated API keys
pub fn rebuild_tray_menu<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    info!("Rebuilding system tray menu");

    let menu = build_tray_menu_from_handle(app)?;

    if let Some(tray) = app.tray_by_id("main") {
        tray.set_menu(Some(menu))?;
        info!("System tray menu updated");
    }

    Ok(())
}

/// Build tray menu from AppHandle (used for rebuilding)
fn build_tray_menu_from_handle<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<tauri::menu::Menu<R>> {
    let mut menu_builder = MenuBuilder::new(app);

    // Get API keys from manager
    if let Some(key_manager) = app.try_state::<ApiKeyManager>() {
        let keys = key_manager.list_keys();

        if !keys.is_empty() {
            for key in keys.iter().filter(|k| k.enabled) {
                let key_name = if key.name.is_empty() {
                    format!("Key {}", &key.id[..8])
                } else {
                    key.name.clone()
                };

                let model_info = match &key.model_selection {
                    // ModelSelection::Router { router_name } => {
                    //     format!("Router: {}", router_name)
                    // }
                    ModelSelection::DirectModel { provider, model } => {
                        format!("{}/{}", provider, model)
                    }
                    _ => "Unknown".to_string(),
                };

                menu_builder = menu_builder.text(
                    format!("key_{}", key.id),
                    format!("{} → {}", key_name, model_info),
                );
            }

            menu_builder = menu_builder.separator();
        }
    }

    menu_builder = menu_builder
        .text("generate_key", "➕ Generate New Key...")
        .separator()
        .text("open_dashboard", "📊 Open Dashboard")
        .text("preferences", "⚙️ Preferences")
        .separator()
        .text("quit", "❌ Quit");

    menu_builder.build()
}
