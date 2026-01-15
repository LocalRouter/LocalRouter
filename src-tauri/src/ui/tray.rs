//! System tray management
//!
//! Handles system tray icon and menu.

use crate::api_keys::ApiKeyManager;
use crate::config::{ConfigManager, ModelSelection};
use crate::providers::registry::ProviderRegistry;
use tauri::{
    menu::{MenuBuilder, SubmenuBuilder},
    tray::TrayIconBuilder,
    App, AppHandle, Emitter, Manager, Runtime,
};
use tracing::{error, info};
use std::sync::Arc;

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
        .show_menu_on_left_click(true)
        .on_menu_event(move |app, event| {
            let id = event.id().as_ref();
            info!("Tray menu event: {}", id);

            match id {
                "toggle_server" => {
                    info!("Toggle server requested from tray");
                    let app_clone = app.clone();
                    tauri::async_runtime::spawn(async move {
                        if let Err(e) = handle_toggle_server(&app_clone).await {
                            error!("Failed to toggle server: {}", e);
                        }
                    });
                }
                "open_dashboard" => {
                    if let Some(window) = app.get_webview_window("main") {
                        let _ = window.show();
                        let _ = window.set_focus();
                    }
                }
                "generate_key" => {
                    info!("Generate new key requested from tray");
                    let app_clone = app.clone();
                    tauri::async_runtime::spawn(async move {
                        if let Err(e) = handle_generate_key_from_tray(&app_clone).await {
                            error!("Failed to generate key from tray: {}", e);
                        }
                    });
                }
                "quit" => {
                    info!("Quit requested from tray");
                    app.exit(0);
                }
                _ => {
                    // Handle API key actions
                    if let Some(key_id) = id.strip_prefix("copy_key_") {
                        info!("Copy key requested: {}", key_id);
                        let app_clone = app.clone();
                        let key_id = key_id.to_string();
                        tauri::async_runtime::spawn(async move {
                            if let Err(e) = handle_copy_key(&app_clone, &key_id).await {
                                error!("Failed to copy key: {}", e);
                            }
                        });
                    } else if let Some(key_id) = id.strip_prefix("toggle_key_") {
                        info!("Toggle key requested: {}", key_id);
                        let app_clone = app.clone();
                        let key_id = key_id.to_string();
                        tauri::async_runtime::spawn(async move {
                            if let Err(e) = handle_toggle_key(&app_clone, &key_id).await {
                                error!("Failed to toggle key: {}", e);
                            }
                        });
                    } else if let Some(rest) = id.strip_prefix("set_model_") {
                        // Format: set_model_{key_id}_{provider}_{model}
                        if let Some((key_id, model_spec)) = rest.split_once('_') {
                            info!("Set model requested: key={}, model={}", key_id, model_spec);
                            let app_clone = app.clone();
                            let key_id = key_id.to_string();
                            let model_spec = model_spec.to_string();
                            tauri::async_runtime::spawn(async move {
                                if let Err(e) = handle_set_model(&app_clone, &key_id, &model_spec).await {
                                    error!("Failed to set model: {}", e);
                                }
                            });
                        }
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

    // Get API keys from manager and provider registry
    if let Some(key_manager) = app.try_state::<ApiKeyManager>() {
        let keys = key_manager.list_keys();

        if !keys.is_empty() {
            // Build a submenu for each API key
            for key in keys.iter() {
                let key_name = if key.name.is_empty() {
                    format!("Key {}", &key.id[..8])
                } else {
                    key.name.clone()
                };

                // Build submenu for this API key
                let mut submenu_builder = SubmenuBuilder::new(app, &key_name);

                // Add "Copy API Key" option
                submenu_builder = submenu_builder
                    .text(format!("copy_key_{}", key.id), "📋 Copy API Key");

                // Add "Enable/Disable" option
                let toggle_text = if key.enabled {
                    "🚫 Disable"
                } else {
                    "✅ Enable"
                };
                submenu_builder = submenu_builder
                    .text(format!("toggle_key_{}", key.id), toggle_text);

                // Add separator before model selection
                submenu_builder = submenu_builder.separator();

                // TODO: Add model selection options
                // This requires fetching models from provider registry
                // For now, just add a placeholder
                submenu_builder = submenu_builder.text(
                    format!("models_{}", key.id),
                    "Select Model... (see UI)"
                );

                let submenu = submenu_builder.build()?;
                menu_builder = menu_builder.item(&submenu);
            }

            menu_builder = menu_builder.separator();
        }
    }

    // Get server status to show appropriate text
    let server_text = if let Some(server_manager) = app.try_state::<Arc<crate::server::ServerManager>>() {
        match server_manager.get_status() {
            crate::server::ServerStatus::Running => "⏹️ Stop Server",
            crate::server::ServerStatus::Stopped => "▶️ Start Server",
        }
    } else {
        "▶️ Start Server"
    };

    // Add standard menu items
    menu_builder = menu_builder
        .text("generate_key", "➕ Generate API Key")
        .separator()
        .text("toggle_server", server_text)
        .text("open_dashboard", "📊 Open Dashboard")
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
            for key in keys.iter() {
                let key_name = if key.name.is_empty() {
                    format!("Key {}", &key.id[..8])
                } else {
                    key.name.clone()
                };

                // Build submenu for this API key
                let mut submenu_builder = SubmenuBuilder::new(app, &key_name);

                // Add "Copy API Key" option
                submenu_builder = submenu_builder
                    .text(format!("copy_key_{}", key.id), "📋 Copy API Key");

                // Add "Enable/Disable" option
                let toggle_text = if key.enabled {
                    "🚫 Disable"
                } else {
                    "✅ Enable"
                };
                submenu_builder = submenu_builder
                    .text(format!("toggle_key_{}", key.id), toggle_text);

                // Add separator before model selection
                submenu_builder = submenu_builder.separator();

                // Add model selection placeholder
                submenu_builder = submenu_builder.text(
                    format!("models_{}", key.id),
                    "Select Model... (see UI)"
                );

                let submenu = submenu_builder.build()?;
                menu_builder = menu_builder.item(&submenu);
            }

            menu_builder = menu_builder.separator();
        }
    }

    // Get server status to show appropriate text
    let server_text = if let Some(server_manager) = app.try_state::<Arc<crate::server::ServerManager>>() {
        match server_manager.get_status() {
            crate::server::ServerStatus::Running => "⏹️ Stop Server",
            crate::server::ServerStatus::Stopped => "▶️ Start Server",
        }
    } else {
        "▶️ Start Server"
    };

    menu_builder = menu_builder
        .text("generate_key", "➕ Generate API Key")
        .separator()
        .text("toggle_server", server_text)
        .text("open_dashboard", "📊 Open Dashboard")
        .separator()
        .text("quit", "❌ Quit");

    menu_builder.build()
}

/// Handle toggling the server on/off
async fn handle_toggle_server<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    let server_manager = app.state::<Arc<crate::server::ServerManager>>();

    let status = server_manager.get_status();
    match status {
        crate::server::ServerStatus::Running => {
            info!("Stopping server from tray");
            server_manager.stop().await;
            let _ = app.emit("server-status-changed", "stopped");
        }
        crate::server::ServerStatus::Stopped => {
            info!("Starting server from tray");

            // Get dependencies
            let config_manager = app.state::<ConfigManager>();
            let router = app.state::<Arc<crate::router::Router>>();
            let api_key_manager = app.state::<ApiKeyManager>();
            let rate_limiter = app.state::<Arc<crate::router::RateLimiterManager>>();
            let provider_registry = app.state::<Arc<ProviderRegistry>>();

            // Get server config
            let server_config = {
                let config = config_manager.get();
                crate::server::ServerConfig {
                    host: config.server.host.clone(),
                    port: config.server.port,
                    enable_cors: config.server.enable_cors,
                }
            };

            // Start the server
            server_manager
                .start(
                    server_config,
                    router.inner().clone(),
                    (*api_key_manager.inner()).clone(),
                    rate_limiter.inner().clone(),
                    provider_registry.inner().clone(),
                )
                .await
                .map_err(|e| tauri::Error::Anyhow(e.into()))?;

            let _ = app.emit("server-status-changed", "running");
        }
    }

    // Rebuild tray menu to update button text
    rebuild_tray_menu(app)?;

    Ok(())
}

/// Handle generating a new API key from the system tray
async fn handle_generate_key_from_tray<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    info!("Generating new API key from tray");

    // Get managers from state
    let key_manager = app.state::<ApiKeyManager>();
    let config_manager = app.state::<ConfigManager>();

    // Create key with "All" model selection
    let (key_value, config) = key_manager
        .create_key(None)
        .await
        .map_err(|e| tauri::Error::Anyhow(e.into()))?;

    // Set model selection to "All"
    let _ = key_manager.update_key(&config.id, |cfg| {
        cfg.model_selection = Some(ModelSelection::All);
    });

    // Save to config
    config_manager
        .update(|cfg| {
            // Find and update the key in the config
            if let Some(key) = cfg.api_keys.iter_mut().find(|k| k.id == config.id) {
                key.model_selection = Some(ModelSelection::All);
            } else {
                // Key not found, add it
                let mut new_config = config.clone();
                new_config.model_selection = Some(ModelSelection::All);
                cfg.api_keys.push(new_config);
            }
        })
        .map_err(|e| tauri::Error::Anyhow(e.into()))?;

    config_manager
        .save()
        .await
        .map_err(|e| tauri::Error::Anyhow(e.into()))?;

    // Copy to clipboard
    if let Err(e) = copy_to_clipboard(&key_value) {
        error!("Failed to copy to clipboard: {}", e);
    }

    // Rebuild tray menu
    rebuild_tray_menu(app)?;

    info!("API key generated and copied to clipboard: {}", config.name);

    Ok(())
}

/// Handle copying an API key to clipboard
async fn handle_copy_key<R: Runtime>(app: &AppHandle<R>, key_id: &str) -> tauri::Result<()> {
    let key_manager = app.state::<ApiKeyManager>();

    let key_value = key_manager
        .get_key_value(key_id)
        .map_err(|e| tauri::Error::Anyhow(e.into()))?
        .ok_or_else(|| tauri::Error::Anyhow(anyhow::anyhow!("API key not found in keychain")))?;

    if let Err(e) = copy_to_clipboard(&key_value) {
        error!("Failed to copy to clipboard: {}", e);
        return Err(tauri::Error::Anyhow(e));
    }

    info!("API key copied to clipboard: {}", key_id);

    Ok(())
}

/// Handle toggling an API key's enabled state
async fn handle_toggle_key<R: Runtime>(app: &AppHandle<R>, key_id: &str) -> tauri::Result<()> {
    let key_manager = app.state::<ApiKeyManager>();
    let config_manager = app.state::<ConfigManager>();

    // Get current state
    let key = key_manager
        .get_key(key_id)
        .ok_or_else(|| tauri::Error::Anyhow(anyhow::anyhow!("API key not found")))?;

    let new_enabled = !key.enabled;

    // Update in key manager
    key_manager
        .update_key(key_id, |cfg| {
            cfg.enabled = new_enabled;
        })
        .map_err(|e| tauri::Error::Anyhow(e.into()))?;

    // Update in config
    config_manager
        .update(|cfg| {
            if let Some(k) = cfg.api_keys.iter_mut().find(|k| k.id == key_id) {
                k.enabled = new_enabled;
            }
        })
        .map_err(|e| tauri::Error::Anyhow(e.into()))?;

    config_manager
        .save()
        .await
        .map_err(|e| tauri::Error::Anyhow(e.into()))?;

    // Rebuild tray menu
    rebuild_tray_menu(app)?;

    info!("API key {} {}", key_id, if new_enabled { "enabled" } else { "disabled" });

    Ok(())
}

/// Handle setting a specific model for an API key
async fn handle_set_model<R: Runtime>(app: &AppHandle<R>, key_id: &str, model_spec: &str) -> tauri::Result<()> {
    let _key_manager = app.state::<ApiKeyManager>();
    let _config_manager = app.state::<ConfigManager>();

    // Parse model_spec (format: provider_model)
    // For now, this is a placeholder - full implementation would parse the model spec
    info!("Setting model {} for key {}", model_spec, key_id);

    // TODO: Implement actual model selection update
    // This would involve:
    // 1. Parsing the model_spec to extract provider and model
    // 2. Creating a Custom ModelSelection with that specific model
    // 3. Updating the key configuration
    // 4. Saving to disk
    // 5. Rebuilding the tray menu

    Ok(())
}

/// Update the tray icon based on server status
pub fn update_tray_icon<R: Runtime>(app: &AppHandle<R>, status: &str) -> tauri::Result<()> {
    if let Some(tray) = app.tray_by_id("main") {
        match status {
            "stopped" => {
                // Stopped: Show as template (monochrome/dimmed)
                tray.set_icon_as_template(true)?;
                tray.set_tooltip(Some("LocalRouter AI - Server Stopped"))?;
                info!("Tray icon updated: stopped (template mode)");
            }
            "running" => {
                // Running: Show as template (monochrome)
                tray.set_icon_as_template(true)?;
                tray.set_tooltip(Some("LocalRouter AI - Server Running"))?;
                info!("Tray icon updated: running (template mode)");
            }
            "active" => {
                // Active: Show as non-template (full color) to indicate activity
                tray.set_icon_as_template(false)?;
                tray.set_tooltip(Some("LocalRouter AI - Processing Request"))?;
                info!("Tray icon updated: active (full color)");

                // Schedule a return to "running" state after 2 seconds
                let app_clone = app.clone();
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
                    if let Err(e) = update_tray_icon(&app_clone, "running") {
                        error!("Failed to reset tray icon to running: {}", e);
                    }
                });
            }
            _ => {
                error!("Unknown tray icon status: {}", status);
            }
        }
    }

    Ok(())
}

/// Copy text to clipboard
fn copy_to_clipboard(text: &str) -> Result<(), anyhow::Error> {
    let mut clipboard = arboard::Clipboard::new()
        .map_err(|e| anyhow::anyhow!("Failed to access clipboard: {}", e))?;

    clipboard
        .set_text(text)
        .map_err(|e| anyhow::anyhow!("Failed to set clipboard text: {}", e))?;

    Ok(())
}
