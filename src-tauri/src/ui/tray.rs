//! System tray management
//!
//! Handles system tray icon and menu.

#![allow(dead_code)]

use crate::clients::ClientManager;
use crate::config::{ConfigManager, McpServerAccess, UiConfig};
use crate::mcp::manager::McpServerManager;
use crate::monitoring::metrics::MetricsCollector;
use crate::providers::registry::ProviderRegistry;
use crate::ui::tray_graph::{platform_graph_config, DataPoint};
use crate::utils::test_mode::is_test_mode;
use chrono::{DateTime, Duration, Utc};
use parking_lot::RwLock;
use std::sync::Arc;
use tauri::{
    menu::{MenuBuilder, MenuItem, SubmenuBuilder},
    tray::TrayIconBuilder,
    App, AppHandle, Emitter, Manager, Runtime,
};
use tokio::sync::mpsc;
use tracing::{debug, error, info};

/// State to track if an update notification should be shown in the tray
pub struct UpdateNotificationState {
    pub update_available: Arc<RwLock<bool>>,
}

impl UpdateNotificationState {
    pub fn new() -> Self {
        Self {
            update_available: Arc::new(RwLock::new(false)),
        }
    }

    pub fn set_update_available(&self, available: bool) {
        *self.update_available.write() = available;
    }

    pub fn is_update_available(&self) -> bool {
        *self.update_available.read()
    }
}

/// Setup system tray icon and menu
pub fn setup_tray<R: Runtime>(app: &App<R>) -> tauri::Result<()> {
    info!("Setting up system tray");

    // Build the tray menu
    let menu = build_tray_menu(app)?;

    // Load the tray icon
    // On macOS, use the 32x32.png template icon specifically designed for the tray
    // This is a monochrome icon that will render properly with icon_as_template(true)
    // The icon is embedded at compile time from the icons directory
    const TRAY_ICON: &[u8] = include_bytes!("../../icons/32x32.png");
    let icon = tauri::image::Image::from_bytes(TRAY_ICON).map_err(|e| {
        error!("Failed to load embedded tray icon: {}", e);
        tauri::Error::Anyhow(anyhow::anyhow!("Failed to load tray icon: {}", e))
    })?;

    // Create the tray icon
    // In test mode, add a visual indicator to the tooltip
    let tooltip = if is_test_mode() {
        "🧪 LocalRouter AI [TEST MODE]"
    } else {
        "LocalRouter AI"
    };

    let _tray = TrayIconBuilder::with_id("main")
        .icon(icon)
        .menu(&menu)
        .tooltip(tooltip)
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
                "copy_url" => {
                    info!("Copy URL requested from tray");
                    let app_clone = app.clone();
                    tauri::async_runtime::spawn(async move {
                        if let Err(e) = handle_copy_url(&app_clone).await {
                            error!("Failed to copy URL: {}", e);
                        }
                    });
                }
                "open_dashboard" => {
                    if let Some(window) = app.get_webview_window("main") {
                        let _ = window.show();
                        let _ = window.set_focus();
                    }
                }
                "create_and_copy_api_key" => {
                    info!("Create and copy API key requested from tray");
                    let app_clone = app.clone();
                    tauri::async_runtime::spawn(async move {
                        if let Err(e) = handle_create_and_copy_api_key(&app_clone).await {
                            error!("Failed to create and copy API key: {}", e);
                        }
                    });
                }
                "toggle_tray_graph" => {
                    info!("Toggle tray graph requested from tray");
                    let app_clone = app.clone();
                    tauri::async_runtime::spawn(async move {
                        if let Err(e) = handle_toggle_tray_graph(&app_clone).await {
                            error!("Failed to toggle tray graph: {}", e);
                        }
                    });
                }
                "open_updates_tab" => {
                    info!("Open Updates tab requested from tray");
                    // Show the main window and emit event to navigate to Updates tab
                    if let Some(window) = app.get_webview_window("main") {
                        let _ = window.show();
                        let _ = window.set_focus();
                    }
                    // Emit event to frontend to navigate to Preferences → Updates
                    if let Err(e) = app.emit("open-updates-tab", ()) {
                        error!("Failed to emit open-updates-tab event: {}", e);
                    }
                }
                "quit" => {
                    info!("Quit requested from tray");
                    app.exit(0);
                }
                _ => {
                    // Handle copy MCP URL: copy_mcp_url_<client_id>_<server_id>
                    if let Some(rest) = id.strip_prefix("copy_mcp_url_") {
                        if let Some((client_id, server_id)) = rest.split_once('_') {
                            info!(
                                "Copy MCP URL requested: client={}, server={}",
                                client_id, server_id
                            );
                            let app_clone = app.clone();
                            let client_id = client_id.to_string();
                            let server_id = server_id.to_string();
                            tauri::async_runtime::spawn(async move {
                                if let Err(e) =
                                    handle_copy_mcp_url(&app_clone, &client_id, &server_id).await
                                {
                                    error!("Failed to copy MCP URL: {}", e);
                                }
                            });
                        }
                    }
                    // Handle copy MCP bearer token: copy_mcp_bearer_<client_id>_<server_id>
                    else if let Some(rest) = id.strip_prefix("copy_mcp_bearer_") {
                        if let Some((client_id, server_id)) = rest.split_once('_') {
                            info!(
                                "Copy MCP bearer token requested: client={}, server={}",
                                client_id, server_id
                            );
                            let app_clone = app.clone();
                            let client_id = client_id.to_string();
                            tauri::async_runtime::spawn(async move {
                                if let Err(e) = handle_copy_mcp_bearer(&app_clone, &client_id).await
                                {
                                    error!("Failed to copy MCP bearer token: {}", e);
                                }
                            });
                        }
                    }
                    // Handle add MCP: add_mcp_<client_id>_<server_id>
                    else if let Some(rest) = id.strip_prefix("add_mcp_") {
                        if let Some((client_id, server_id)) = rest.split_once('_') {
                            info!(
                                "Add MCP requested: client={}, server={}",
                                client_id, server_id
                            );
                            let app_clone = app.clone();
                            let client_id = client_id.to_string();
                            let server_id = server_id.to_string();
                            tauri::async_runtime::spawn(async move {
                                if let Err(e) =
                                    handle_add_mcp_to_client(&app_clone, &client_id, &server_id)
                                        .await
                                {
                                    error!("Failed to add MCP to client: {}", e);
                                }
                            });
                        }
                    }
                    // Handle prioritized list: prioritized_list_<client_id>
                    else if let Some(client_id) = id.strip_prefix("prioritized_list_") {
                        info!("Prioritized list requested for client: {}", client_id);
                        let app_clone = app.clone();
                        let client_id = client_id.to_string();
                        tauri::async_runtime::spawn(async move {
                            if let Err(e) = handle_prioritized_list(&app_clone, &client_id).await {
                                error!("Failed to open prioritized list: {}", e);
                            }
                        });
                    }
                    // Other events are for model routing configuration
                    // (force_model_*, toggle_provider_*, toggle_model_*, etc.)
                    // These will be handled by future implementation
                }
            }
        })
        .build(app)?;

    info!("System tray initialized successfully");
    Ok(())
}

/// Build the system tray menu
fn build_tray_menu<R: Runtime, M: Manager<R>>(app: &M) -> tauri::Result<tauri::menu::Menu<R>> {
    let mut menu_builder = MenuBuilder::new(app);

    // 1. Open Dashboard at the top
    menu_builder = menu_builder.text("open_dashboard", "📊 Open Dashboard");

    // Add separator
    menu_builder = menu_builder.separator();

    // 2. Clients section
    let clients_header = MenuItem::with_id(app, "clients_header", "Clients", false, None::<&str>)?;
    menu_builder = menu_builder.item(&clients_header);

    // Get client manager and build client list
    if let Some(client_manager) = app.try_state::<Arc<ClientManager>>() {
        let clients = client_manager.list_clients();
        let mcp_server_manager = app.try_state::<Arc<McpServerManager>>();

        if !clients.is_empty() {
            for client in clients.iter() {
                let client_name = if client.name.is_empty() {
                    format!("Client {}", &client.id[..8])
                } else {
                    client.name.clone()
                };

                let mut client_submenu = SubmenuBuilder::new(app, &client_name);

                // Strategy configuration link
                client_submenu = client_submenu.text(
                    format!("configure_strategy_{}", client.id),
                    "Configure Strategy...",
                );

                // Add separator before Allowed MCPs
                client_submenu = client_submenu.separator();

                // Allowed MCPs section header
                let mcp_header = MenuItem::with_id(
                    app,
                    format!("mcp_header_{}", client.id),
                    "Allowed MCPs",
                    false,
                    None::<&str>,
                )?;
                client_submenu = client_submenu.item(&mcp_header);

                // Get MCP servers this client can access
                if let Some(ref mcp_manager) = mcp_server_manager {
                    let all_mcp_servers = mcp_manager.list_configs();

                    // Get allowed server IDs based on access mode
                    let allowed_server_ids: Vec<String> = match &client.mcp_server_access {
                        McpServerAccess::None => vec![],
                        McpServerAccess::All => {
                            all_mcp_servers.iter().map(|s| s.id.clone()).collect()
                        }
                        McpServerAccess::Specific(servers) => servers.clone(),
                    };

                    // Show allowed MCP servers
                    for server_id in allowed_server_ids.iter() {
                        if let Some(server) = all_mcp_servers.iter().find(|s| &s.id == server_id) {
                            let server_name = if server.name.is_empty() {
                                format!("MCP {}", &server.id[..8])
                            } else {
                                server.name.clone()
                            };

                            let mut mcp_submenu = SubmenuBuilder::new(app, &server_name);

                            // Get server URL
                            let config_manager = app.try_state::<ConfigManager>();
                            let url = if let Some(cfg_mgr) = config_manager {
                                let cfg = cfg_mgr.get();
                                format!(
                                    "http://{}:{}/mcp/{}",
                                    cfg.server.host, cfg.server.port, server.id
                                )
                            } else {
                                format!("http://127.0.0.1:3625/mcp/{}", server.id)
                            };

                            // Show URL as disabled item
                            let url_item = MenuItem::with_id(
                                app,
                                format!("mcp_url_display_{}_{}", client.id, server.id),
                                &url,
                                false,
                                None::<&str>,
                            )?;
                            mcp_submenu = mcp_submenu.item(&url_item);

                            mcp_submenu = mcp_submenu.text(
                                format!("copy_mcp_url_{}_{}", client.id, server.id),
                                "📋 Copy URL",
                            );

                            mcp_submenu = mcp_submenu.text(
                                format!("copy_mcp_bearer_{}_{}", client.id, server.id),
                                "📋 Copy Bearer token",
                            );

                            let mcp_menu = mcp_submenu.build()?;
                            client_submenu = client_submenu.item(&mcp_menu);
                        }
                    }
                }

                // Add "+ Add MCP" submenu with available MCP servers (only in Specific mode)
                if let Some(ref mcp_manager) = mcp_server_manager {
                    let all_mcp_servers = mcp_manager.list_configs();

                    // Find MCP servers not yet added to this client (only relevant for Specific mode)
                    let available_servers: Vec<_> = all_mcp_servers
                        .iter()
                        .filter(|s| !client.mcp_server_access.can_access(&s.id))
                        .collect();

                    if !available_servers.is_empty() {
                        let mut add_mcp_submenu = SubmenuBuilder::new(app, "➕ Add MCP");

                        for server in available_servers {
                            let server_name = if server.name.is_empty() {
                                format!("MCP {}", &server.id[..8])
                            } else {
                                server.name.clone()
                            };

                            add_mcp_submenu = add_mcp_submenu
                                .text(format!("add_mcp_{}_{}", client.id, server.id), server_name);
                        }

                        let add_mcp_menu = add_mcp_submenu.build()?;
                        client_submenu = client_submenu.item(&add_mcp_menu);
                    }
                    // If no available servers, just omit the menu item entirely
                }

                let client_menu = client_submenu.build()?;
                client_menu.set_enabled(true)?;
                menu_builder = menu_builder.item(&client_menu);
            }
        }
    }

    // Add "+ Create & copy API Key" button
    menu_builder = menu_builder.text("create_and_copy_api_key", "➕ Create && copy API Key");

    // Add separator before Server section
    menu_builder = menu_builder.separator();

    // Get port and server status
    let (host, port, server_text) = if let Some(config_manager) = app.try_state::<ConfigManager>() {
        let config = config_manager.get();
        let status =
            if let Some(server_manager) = app.try_state::<Arc<crate::server::ServerManager>>() {
                match server_manager.get_status() {
                    crate::server::ServerStatus::Running => "⏹️ Stop Server",
                    crate::server::ServerStatus::Stopped => "▶️ Start Server",
                }
            } else {
                "▶️ Start Server"
            };
        (config.server.host.clone(), config.server.port, status)
    } else {
        ("127.0.0.1".to_string(), 3625, "▶️ Start Server")
    };

    // Add Server section header with IP:Port
    let server_header = MenuItem::with_id(
        app,
        "server_header",
        format!("Listening on {}:{}", host, port),
        false,
        None::<&str>,
    )?;
    menu_builder = menu_builder.item(&server_header);

    // Add server-related items
    menu_builder = menu_builder
        .text("copy_url", "📋 Copy URL")
        .text("toggle_server", server_text);

    // Add separator before tray graph toggle
    menu_builder = menu_builder.separator();

    // Add tray graph toggle
    let tray_graph_text = if let Some(config_manager) = app.try_state::<ConfigManager>() {
        let config = config_manager.get();
        if config.ui.tray_graph_enabled {
            "✓ Dynamic Graph"
        } else {
            "Dynamic Graph"
        }
    } else {
        "Dynamic Graph"
    };
    menu_builder = menu_builder.text("toggle_tray_graph", tray_graph_text);

    // Add update notification if available
    if let Some(update_state) = app.try_state::<Arc<UpdateNotificationState>>() {
        if update_state.is_update_available() {
            menu_builder = menu_builder.separator();
            menu_builder = menu_builder.text("open_updates_tab", "🔔 Review new update");
        }
    }

    // Add separator before quit
    menu_builder = menu_builder.separator();

    // Add quit option
    menu_builder = menu_builder.text("quit", "❌ Shut down");

    menu_builder.build()
}

/// Rebuild the system tray menu with updated API keys
pub fn rebuild_tray_menu<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    info!("Rebuilding system tray menu");

    let menu = build_tray_menu(app)?;

    if let Some(tray) = app.tray_by_id("main") {
        tray.set_menu(Some(menu))?;
        info!("System tray menu updated");
    }

    Ok(())
}

/// Handle copying the server URL to clipboard
async fn handle_copy_url<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    let config_manager = app.state::<ConfigManager>();
    let config = config_manager.get();

    let url = format!("http://{}:{}", config.server.host, config.server.port);

    if let Err(e) = copy_to_clipboard(&url) {
        error!("Failed to copy URL to clipboard: {}", e);
        return Err(tauri::Error::Anyhow(e));
    }

    info!("Server URL copied to clipboard: {}", url);

    Ok(())
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
            let mcp_server_manager = app.state::<Arc<crate::mcp::McpServerManager>>();
            let rate_limiter = app.state::<Arc<crate::router::RateLimiterManager>>();
            let provider_registry = app.state::<Arc<ProviderRegistry>>();
            let client_manager = app.state::<Arc<crate::clients::ClientManager>>();
            let token_store = app.state::<Arc<crate::clients::TokenStore>>();
            let metrics_collector =
                app.state::<Arc<crate::monitoring::metrics::MetricsCollector>>();

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
                    crate::server::manager::ServerDependencies {
                        router: router.inner().clone(),
                        mcp_server_manager: mcp_server_manager.inner().clone(),
                        rate_limiter: rate_limiter.inner().clone(),
                        provider_registry: provider_registry.inner().clone(),
                        config_manager: Arc::new((*config_manager).clone()),
                        client_manager: client_manager.inner().clone(),
                        token_store: token_store.inner().clone(),
                        metrics_collector: metrics_collector.inner().clone(),
                    },
                )
                .await
                .map_err(tauri::Error::Anyhow)?;

            let _ = app.emit("server-status-changed", "running");
        }
    }

    // Rebuild tray menu to update button text
    rebuild_tray_menu(app)?;

    Ok(())
}
// /// Handle generating a new API key from the system tray
// async fn handle_generate_key_from_tray<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
//     info!("Generating new API key from tray");
//
// Get managers from state
//         let key_manager = app.state::<ApiKeyManager>();
//     let config_manager = app.state::<ConfigManager>();
//
// Create key with "All" model selection
//     let (key_value, config) = key_manager
//         .create_key(None)
//         .await
//         .map_err(|e| tauri::Error::Anyhow(e.into()))?;
//
// Set model selection to "All"
//     let _ = key_manager.update_key(&config.id, |cfg| {
//         cfg.model_selection = Some(ModelSelection::All);
//     });
//
// Save to config
//     config_manager
//         .update(|cfg| {
// Find and update the key in the config
//             if let Some(key) = cfg.api_keys.iter_mut().find(|k| k.id == config.id) {
//                 key.model_selection = Some(ModelSelection::All);
//             } else {
// Key not found, add it
//                 let mut new_config = config.clone();
//                 new_config.model_selection = Some(ModelSelection::All);
//                 cfg.api_keys.push(new_config);
//             }
//         })
//         .map_err(|e| tauri::Error::Anyhow(e.into()))?;
//
//     config_manager
//         .save()
//         .await
//         .map_err(|e| tauri::Error::Anyhow(e.into()))?;
//
// Copy to clipboard
//     if let Err(e) = copy_to_clipboard(&key_value) {
//         error!("Failed to copy to clipboard: {}", e);
//     }
//
// Rebuild tray menu
//     rebuild_tray_menu(app)?;
//
//     info!("API key generated and copied to clipboard: {}", config.name);
//
//     Ok(())
// }
// /// Handle copying an API key to clipboard
// async fn handle_copy_key<R: Runtime>(app: &AppHandle<R>, key_id: &str) -> tauri::Result<()> {
//         let key_manager = app.state::<ApiKeyManager>();
//
//     let key_value = key_manager
//         .get_key_value(key_id)
//         .map_err(|e| tauri::Error::Anyhow(e.into()))?
//         .ok_or_else(|| tauri::Error::Anyhow(anyhow::anyhow!("API key not found in keychain")))?;
//
//     if let Err(e) = copy_to_clipboard(&key_value) {
//         error!("Failed to copy to clipboard: {}", e);
//         return Err(tauri::Error::Anyhow(e));
//     }
//
//     info!("API key copied to clipboard: {}", key_id);
//
//     Ok(())
// }
// /// Handle toggling an API key's enabled state
// async fn handle_toggle_key<R: Runtime>(app: &AppHandle<R>, key_id: &str) -> tauri::Result<()> {
//         let key_manager = app.state::<ApiKeyManager>();
//     let config_manager = app.state::<ConfigManager>();
//
// Get current state
//     let key = key_manager
//         .get_key(key_id)
//         .ok_or_else(|| tauri::Error::Anyhow(anyhow::anyhow!("API key not found")))?;
//
//     let new_enabled = !key.enabled;
//
// Update in key manager
//     key_manager
//         .update_key(key_id, |cfg| {
//             cfg.enabled = new_enabled;
//         })
//         .map_err(|e| tauri::Error::Anyhow(e.into()))?;
//
// Update in config
//     config_manager
//         .update(|cfg| {
//             if let Some(k) = cfg.api_keys.iter_mut().find(|k| k.id == key_id) {
//                 k.enabled = new_enabled;
//             }
//         })
//         .map_err(|e| tauri::Error::Anyhow(e.into()))?;
//
//     config_manager
//         .save()
//         .await
//         .map_err(|e| tauri::Error::Anyhow(e.into()))?;
//
// Rebuild tray menu
//     rebuild_tray_menu(app)?;
//
//     info!("API key {} {}", key_id, if new_enabled { "enabled" } else { "disabled" });
//
//     Ok(())
// }

/// Handle setting a specific model for an API key
///
/// Supports toggling between different model selection types:
/// - "all" - Set to ModelSelection::All
/// - "provider_{name}" - Toggle all models from a provider
/// - "model_{provider}_{model}" - Toggle a specific model
// async fn handle_set_model<R: Runtime>(app: &AppHandle<R>, key_id: &str, model_spec: &str) -> tauri::Result<()> {
//         let key_manager = app.state::<ApiKeyManager>();
//     let config_manager = app.state::<ConfigManager>();
//
//     info!("Setting model {} for key {}", model_spec, key_id);
//
// Get current key configuration
//     let current_key = key_manager
//         .get_key(key_id)
//         .ok_or_else(|| tauri::Error::Anyhow(anyhow::anyhow!("API key not found")))?;
//
//     let new_selection = if model_spec == "all" {
// Set to "All Models"
//         ModelSelection::All
//     } else if let Some(provider) = model_spec.strip_prefix("provider_") {
// Toggle provider in Custom selection
//         match &current_key.model_selection {
//             Some(ModelSelection::Custom { all_provider_models, individual_models }) => {
//                 let mut new_providers = all_provider_models.clone();
//                 let new_individual = individual_models.clone();
//
// Toggle: if provider is already selected, remove it; otherwise add it
//                 if let Some(pos) = new_providers.iter().position(|p| p == provider) {
//                     new_providers.remove(pos);
//                 } else {
//                     new_providers.push(provider.to_string());
//                 }
//
//                 ModelSelection::Custom {
//                     all_provider_models: new_providers,
//                     individual_models: new_individual,
//                 }
//             }
//             _ => {
// If not Custom, create new Custom with just this provider
//                 ModelSelection::Custom {
//                     all_provider_models: vec![provider.to_string()],
//                     individual_models: vec![],
//                 }
//             }
//         }
//     } else if let Some(rest) = model_spec.strip_prefix("model_") {
// Toggle individual model in Custom selection
// Format: model_{provider}_{model}
//         if let Some((provider, model)) = rest.split_once('_') {
//             match &current_key.model_selection {
//                 Some(ModelSelection::Custom { all_provider_models, individual_models }) => {
//                     let new_providers = all_provider_models.clone();
//                     let mut new_individual = individual_models.clone();
//
// Toggle: if model is already selected, remove it; otherwise add it
//                     let model_tuple = (provider.to_string(), model.to_string());
//                     if let Some(pos) = new_individual.iter().position(|m| m == &model_tuple) {
//                         new_individual.remove(pos);
//                     } else {
//                         new_individual.push(model_tuple);
//                     }
//
//                     ModelSelection::Custom {
//                         all_provider_models: new_providers,
//                         individual_models: new_individual,
//                     }
//                 }
//                 _ => {
// If not Custom, create new Custom with just this model
//                     ModelSelection::Custom {
//                         all_provider_models: vec![],
//                         individual_models: vec![(provider.to_string(), model.to_string())],
//                     }
//                 }
//             }
//         } else {
//             return Err(tauri::Error::Anyhow(anyhow::anyhow!("Invalid model spec format")));
//         }
//     } else {
//         return Err(tauri::Error::Anyhow(anyhow::anyhow!("Unknown model spec format")));
//     };
//
// Update in key manager
//     key_manager
//         .update_key(key_id, |cfg| {
//             cfg.model_selection = Some(new_selection.clone());
//         })
//         .map_err(|e| tauri::Error::Anyhow(e.into()))?;
//
// Update in config
//     config_manager
//         .update(|cfg| {
//             if let Some(k) = cfg.api_keys.iter_mut().find(|k| k.id == key_id) {
//                 k.model_selection = Some(new_selection);
//             }
//         })
//         .map_err(|e| tauri::Error::Anyhow(e.into()))?;
//
// Save to disk
//     config_manager
//         .save()
//         .await
//         .map_err(|e| tauri::Error::Anyhow(e.into()))?;
//
// Rebuild tray menu to show updated checkmarks
//     rebuild_tray_menu(app)?;
//
//     info!("Model selection updated for key {}", key_id);
//
//     Ok(())
// }
#[allow(dead_code)]
async fn handle_prioritized_list<R: Runtime>(
    app: &AppHandle<R>,
    key_id: &str,
) -> tauri::Result<()> {
    info!("Opening prioritized list for key {}", key_id);

    // Open the dashboard window
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();

        // Emit event to open the prioritized list modal for this key
        let _ = app.emit("open-prioritized-list", key_id);
    }

    Ok(())
}

/// Handle creating a new client and copying the API key to clipboard
async fn handle_create_and_copy_api_key<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    info!("Creating new client and copying API key from tray");

    // Get managers from state
    let client_manager = app.state::<Arc<ClientManager>>();
    let config_manager = app.state::<ConfigManager>();

    // Create a new client with a default name
    let (client_id, secret, _config) = client_manager
        .create_client("App".to_string())
        .map_err(|e| tauri::Error::Anyhow(e.into()))?;

    // Save to config
    config_manager
        .update(|cfg| {
            cfg.clients = client_manager.get_configs();
        })
        .map_err(|e| tauri::Error::Anyhow(e.into()))?;

    config_manager
        .save()
        .await
        .map_err(|e| tauri::Error::Anyhow(e.into()))?;

    // Copy secret to clipboard
    if let Err(e) = copy_to_clipboard(&secret) {
        error!("Failed to copy API key to clipboard: {}", e);
    }

    // Rebuild tray menu
    rebuild_tray_menu(app)?;

    info!("API key created and copied to clipboard: {}", client_id);

    Ok(())
}

/// Handle copying MCP URL to clipboard
async fn handle_copy_mcp_url<R: Runtime>(
    app: &AppHandle<R>,
    _client_id: &str,
    server_id: &str,
) -> tauri::Result<()> {
    info!("Copying MCP URL for server: {}", server_id);

    // Get config manager for port
    let config_manager = app.state::<ConfigManager>();
    let config = config_manager.get();

    // Build MCP proxy URL
    let url = format!(
        "http://{}:{}/mcp/{}",
        config.server.host, config.server.port, server_id
    );

    // Copy to clipboard
    if let Err(e) = copy_to_clipboard(&url) {
        error!("Failed to copy MCP URL to clipboard: {}", e);
        return Err(tauri::Error::Anyhow(e));
    }

    info!("MCP URL copied to clipboard: {}", url);

    Ok(())
}

/// Handle copying MCP bearer token (client secret) to clipboard
async fn handle_copy_mcp_bearer<R: Runtime>(
    app: &AppHandle<R>,
    client_id: &str,
) -> tauri::Result<()> {
    info!("Copying bearer token for client: {}", client_id);

    // Get client manager
    let client_manager = app.state::<Arc<ClientManager>>();

    // Get client secret from keychain
    let secret = client_manager
        .get_secret(client_id)
        .map_err(|e| tauri::Error::Anyhow(e.into()))?
        .ok_or_else(|| tauri::Error::Anyhow(anyhow::anyhow!("Client secret not found")))?;

    // Copy to clipboard
    if let Err(e) = copy_to_clipboard(&secret) {
        error!("Failed to copy bearer token to clipboard: {}", e);
        return Err(tauri::Error::Anyhow(e));
    }

    info!("Bearer token copied to clipboard for client: {}", client_id);

    Ok(())
}

/// Handle adding an MCP server to a client's allowed list
async fn handle_add_mcp_to_client<R: Runtime>(
    app: &AppHandle<R>,
    client_id: &str,
    server_id: &str,
) -> tauri::Result<()> {
    info!("Adding MCP server {} to client {}", server_id, client_id);

    // Get managers from state
    let client_manager = app.state::<Arc<ClientManager>>();
    let config_manager = app.state::<ConfigManager>();

    // Add MCP server to client's allowed list
    client_manager
        .add_mcp_server(client_id, server_id)
        .map_err(|e| tauri::Error::Anyhow(e.into()))?;

    // Save to config
    config_manager
        .update(|cfg| {
            cfg.clients = client_manager.get_configs();
        })
        .map_err(|e| tauri::Error::Anyhow(e.into()))?;

    config_manager
        .save()
        .await
        .map_err(|e| tauri::Error::Anyhow(e.into()))?;

    // Rebuild tray menu
    rebuild_tray_menu(app)?;

    info!("MCP server {} added to client {}", server_id, client_id);

    Ok(())
}

/// Update the tray icon based on server status
pub fn update_tray_icon<R: Runtime>(app: &AppHandle<R>, status: &str) -> tauri::Result<()> {
    // Embed the tray icons at compile time
    const TRAY_ICON: &[u8] = include_bytes!("../../icons/32x32.png");
    const TRAY_ICON_ACTIVE: &[u8] = include_bytes!("../../icons/32x32-active.png");

    if let Some(tray) = app.tray_by_id("main") {
        match status {
            "stopped" => {
                // Stopped: Use template icon in template mode (monochrome/dimmed)
                let icon = tauri::image::Image::from_bytes(TRAY_ICON).map_err(|e| {
                    tauri::Error::Anyhow(anyhow::anyhow!("Failed to load tray icon: {}", e))
                })?;
                tray.set_icon(Some(icon))?;
                tray.set_icon_as_template(true)?;
                tray.set_tooltip(Some("LocalRouter AI - Server Stopped"))?;
                info!("Tray icon updated: stopped (template mode)");
            }
            "running" => {
                // Running: Use template icon in template mode (monochrome)
                let icon = tauri::image::Image::from_bytes(TRAY_ICON).map_err(|e| {
                    tauri::Error::Anyhow(anyhow::anyhow!("Failed to load tray icon: {}", e))
                })?;
                tray.set_icon(Some(icon))?;
                tray.set_icon_as_template(true)?;
                tray.set_tooltip(Some("LocalRouter AI - Server Running"))?;
                info!("Tray icon updated: running (template mode)");
            }
            "active" => {
                // Active: Use active icon in non-template mode to show activity
                let icon = tauri::image::Image::from_bytes(TRAY_ICON_ACTIVE).map_err(|e| {
                    tauri::Error::Anyhow(anyhow::anyhow!("Failed to load active tray icon: {}", e))
                })?;
                tray.set_icon(Some(icon))?;
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

/// Handle toggling the tray graph feature on/off
async fn handle_toggle_tray_graph<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    info!("Toggling tray graph feature from tray");

    // Get managers from state
    let config_manager = app.state::<ConfigManager>();
    let tray_graph_manager = app.state::<Arc<TrayGraphManager>>();

    // Get current state and toggle it
    let new_enabled = {
        let config = config_manager.get();
        !config.ui.tray_graph_enabled
    };

    // Update configuration
    config_manager
        .update(|config| {
            config.ui.tray_graph_enabled = new_enabled;
        })
        .map_err(|e| tauri::Error::Anyhow(e.into()))?;

    // Save to disk
    config_manager
        .save()
        .await
        .map_err(|e| tauri::Error::Anyhow(e.into()))?;

    // Update tray graph manager
    let new_config = config_manager.get().ui.clone();
    tray_graph_manager.update_config(new_config);

    // Rebuild tray menu to show updated checkmark
    rebuild_tray_menu(app)?;

    info!(
        "Tray graph feature {} from tray",
        if new_enabled { "enabled" } else { "disabled" }
    );

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

/// Manager for dynamic tray icon graph updates
pub struct TrayGraphManager {
    /// App handle for accessing tray and state
    app_handle: AppHandle,

    /// UI configuration
    config: Arc<RwLock<UiConfig>>,

    /// Last update timestamp for throttling
    #[allow(dead_code)]
    last_update: Arc<RwLock<Option<DateTime<Utc>>>>,

    /// Channel for activity notifications
    activity_tx: mpsc::UnboundedSender<()>,

    /// Last activity timestamp for idle detection
    last_activity: Arc<RwLock<DateTime<Utc>>>,

    /// Current bucket values for Fast/Medium modes (26 buckets)
    /// For Slow mode, this is not used (queries metrics directly)
    #[allow(dead_code)]
    buckets: Arc<RwLock<Vec<u64>>>,

    /// Accumulated tokens since last update (for Fast/Medium modes)
    /// This receives real-time token counts from completed requests
    accumulated_tokens: Arc<RwLock<u64>>,
}

impl TrayGraphManager {
    /// Create a new tray graph manager
    ///
    /// Starts a background task that listens for activity notifications
    /// and updates the tray icon graph at the configured interval.
    pub fn new(app_handle: AppHandle, config: UiConfig) -> Self {
        let (activity_tx, mut activity_rx) = mpsc::unbounded_channel();

        const NUM_BUCKETS: usize = 26; // Match GRAPH_WIDTH in tray_graph.rs

        let config = Arc::new(RwLock::new(config));
        let last_update = Arc::new(RwLock::new(None::<DateTime<Utc>>));
        let last_activity = Arc::new(RwLock::new(Utc::now()));
        let buckets = Arc::new(RwLock::new(vec![0u64; NUM_BUCKETS]));
        let accumulated_tokens = Arc::new(RwLock::new(0u64));

        // Clone for background task
        let app_handle_clone = app_handle.clone();
        let config_clone = config.clone();
        let last_update_clone = last_update.clone();
        let last_activity_clone = last_activity.clone();
        let buckets_clone = buckets.clone();
        let accumulated_tokens_clone = accumulated_tokens.clone();

        // Spawn background task with idle-aware timer for smooth graph shifting
        tauri::async_runtime::spawn(async move {
            debug!("TrayGraphManager background task started");

            const UPDATE_CHECK_INTERVAL_MS: u64 = 500;
            const UPDATE_THROTTLE_MS: i64 = 1000;
            const IDLE_TIMEOUT_SECS: i64 = 60;

            loop {
                // Wait for activity notification
                if activity_rx.recv().await.is_none() {
                    break;
                }

                // Activity detected, update timestamp
                *last_activity_clone.write() = Utc::now();

                // Start timer loop for active period
                let mut interval = tokio::time::interval(tokio::time::Duration::from_millis(
                    UPDATE_CHECK_INTERVAL_MS,
                ));
                interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

                // Keep updating while active (not idle)
                loop {
                    // Check for new activity notifications (non-blocking)
                    while let Ok(()) = activity_rx.try_recv() {
                        *last_activity_clone.write() = Utc::now();
                        debug!(
                            "TrayGraphManager: Activity notification received during update loop"
                        );
                    }

                    interval.tick().await;

                    // Check if feature is enabled
                    let enabled = config_clone.read().tray_graph_enabled;

                    if !enabled {
                        break;
                    }

                    // Check if idle (no activity for 60+ seconds)
                    let is_idle = {
                        let last_activity_read = last_activity_clone.read();
                        let elapsed = Utc::now().signed_duration_since(*last_activity_read);
                        elapsed.num_seconds() >= IDLE_TIMEOUT_SECS
                    };

                    if is_idle {
                        break;
                    }

                    // Check throttle: has enough time passed since last update?
                    let should_update = {
                        let last_update_read = last_update_clone.read();
                        match *last_update_read {
                            None => true, // First update
                            Some(last_ts) => {
                                let elapsed = Utc::now().signed_duration_since(last_ts);
                                elapsed.num_milliseconds() >= UPDATE_THROTTLE_MS
                            }
                        }
                    };

                    if !should_update {
                        // Too soon since last update
                        continue;
                    }

                    // Perform update
                    let is_first_update = last_update_clone.read().is_none();
                    if let Err(e) = Self::update_tray_graph_impl(
                        &app_handle_clone,
                        is_first_update,
                        &buckets_clone,
                        &accumulated_tokens_clone,
                    )
                    .await
                    {
                        error!("Failed to update tray graph: {}", e);
                    } else {
                        // Update last update timestamp
                        *last_update_clone.write() = Some(Utc::now());
                    }
                }
            }

            debug!("TrayGraphManager background task stopped");
        });

        let manager = Self {
            app_handle,
            config,
            last_update,
            activity_tx,
            last_activity,
            buckets,
            accumulated_tokens,
        };

        // Trigger initial update if graph is enabled
        if manager.is_enabled() {
            manager.notify_activity();
        }

        manager
    }

    /// Notify that new activity has occurred (metrics recorded)
    ///
    /// This triggers the throttled update cycle.
    pub fn notify_activity(&self) {
        // Update last activity time
        *self.last_activity.write() = Utc::now();

        debug!("TrayGraphManager: Activity notification received");

        // Send notification (non-blocking)
        if let Err(e) = self.activity_tx.send(()) {
            error!("Failed to send activity notification: {}", e);
        }
    }

    /// Record tokens from a completed request
    ///
    /// This accumulates tokens for Fast/Medium modes to display real-time activity
    /// without querying minute-level metrics.
    pub fn record_tokens(&self, tokens: u64) {
        // Accumulate tokens
        *self.accumulated_tokens.write() += tokens;

        // Trigger update cycle
        self.notify_activity();
    }

    /// Implementation of tray graph update
    ///
    /// Updates the tray graph based on the configured mode:
    /// - Fast (1s): Uses real-time token accumulation only (no metrics)
    /// - Medium (10s): Uses metrics for initial load, then real-time accumulation
    /// - Slow (60s): Always uses minute-level metrics (1:1 mapping)
    async fn update_tray_graph_impl(
        app_handle: &AppHandle,
        is_first_update: bool,
        buckets: &Arc<RwLock<Vec<u64>>>,
        accumulated_tokens: &Arc<RwLock<u64>>,
    ) -> Result<(), anyhow::Error> {
        // Get config and metrics collector from state
        let config_manager = app_handle
            .try_state::<ConfigManager>()
            .ok_or_else(|| anyhow::anyhow!("ConfigManager not in app state"))?;

        let metrics_collector = app_handle
            .try_state::<Arc<MetricsCollector>>()
            .ok_or_else(|| anyhow::anyhow!("MetricsCollector not in app state"))?;

        let refresh_rate_secs = config_manager.get().ui.tray_graph_refresh_rate_secs;

        // Graph has 26 pixels (32 - 2*border - 2*margin*2)
        const NUM_BUCKETS: i64 = 26;
        let now = Utc::now();

        let data_points = match refresh_rate_secs {
            // Fast mode: 1 second per bar, 26 second total
            // NO metrics querying - pure real-time tracking
            // Starts with empty buckets, accumulates only from live requests
            1 => {
                let mut bucket_state = buckets.write();

                if is_first_update {
                    // Start with empty buckets (no historical data)
                    bucket_state.fill(0);
                } else {
                    // Shift buckets left (remove first, append 0 at end)
                    bucket_state.rotate_left(1);
                    bucket_state[NUM_BUCKETS as usize - 1] = 0;
                }

                // Add accumulated tokens to rightmost bucket (real-time data)
                let tokens = *accumulated_tokens.read();
                bucket_state[NUM_BUCKETS as usize - 1] = tokens;

                // Reset accumulator for next cycle
                *accumulated_tokens.write() = 0;

                // Convert to DataPoints
                bucket_state
                    .iter()
                    .enumerate()
                    .map(|(i, &tokens)| DataPoint {
                        timestamp: now - Duration::seconds(NUM_BUCKETS - i as i64 - 1),
                        total_tokens: tokens,
                    })
                    .collect::<Vec<_>>()
            }

            // Medium mode: 10 seconds per bar, 260 seconds total (~4.3 minutes)
            // Initial load: Interpolate minute data across 6 buckets each
            // Continuous: Maintain buckets in memory, shift left every 10 seconds
            10 => {
                let mut bucket_state = buckets.write();

                if is_first_update {
                    // Initial load: Interpolate from minute-level metrics
                    let window_secs = NUM_BUCKETS * 10;
                    let start = now - Duration::seconds(window_secs + 120);
                    let metrics = metrics_collector.get_global_range(start, now);

                    bucket_state.fill(0);

                    // Interpolate each minute across 6 buckets (60s / 10s = 6)
                    for metric in metrics.iter() {
                        let age_secs = now.signed_duration_since(metric.timestamp).num_seconds();
                        if age_secs < 0 || age_secs >= window_secs {
                            continue;
                        }

                        // Determine how many buckets we can actually place (some might fall outside window)
                        // Check both that bucket_age_secs >= 0 (not too recent) and < window_secs (not too old)
                        let num_buckets_in_window = (0..6)
                            .filter(|&offset| {
                                let bucket_age = age_secs.saturating_sub(offset * 10);
                                bucket_age >= 0 && bucket_age < window_secs
                            })
                            .count() as u64;

                        if num_buckets_in_window == 0 {
                            continue;
                        }

                        let tokens_per_bucket = metric.total_tokens / num_buckets_in_window;

                        for offset in 0..6 {
                            // Spread the minute forward in time (subtract offset, not add)
                            // If metric is 100 seconds ago, spread to: 100, 90, 80, 70, 60, 50 seconds ago
                            let bucket_age_secs = age_secs.saturating_sub(offset * 10);
                            if bucket_age_secs < 0 || bucket_age_secs >= window_secs {
                                continue;
                            }

                            let bucket_index = (NUM_BUCKETS - 1) - (bucket_age_secs / 10);
                            let bucket_index = bucket_index.max(0).min(NUM_BUCKETS - 1) as usize;
                            bucket_state[bucket_index] += tokens_per_bucket;
                        }
                    }
                } else {
                    // Runtime: Use accumulated real-time tokens (NO metrics query)
                    // Shift buckets left (remove first, append 0 at end)
                    bucket_state.rotate_left(1);
                    bucket_state[NUM_BUCKETS as usize - 1] = 0;

                    // Add accumulated tokens to rightmost bucket (real-time data)
                    let tokens = *accumulated_tokens.read();
                    bucket_state[NUM_BUCKETS as usize - 1] = tokens;

                    // Reset accumulator for next cycle
                    *accumulated_tokens.write() = 0;
                }

                // Convert to DataPoints
                bucket_state
                    .iter()
                    .enumerate()
                    .map(|(i, &tokens)| DataPoint {
                        timestamp: now - Duration::seconds((NUM_BUCKETS - i as i64 - 1) * 10),
                        total_tokens: tokens,
                    })
                    .collect::<Vec<_>>()
            }

            // Slow mode: 1 minute per bar, 26 minute total (1560 seconds)
            // Direct mapping: one minute of metrics → one bar (no bucket management)
            60 | _ => {
                let window_secs = NUM_BUCKETS * 60; // 1560 seconds = 26 minutes
                let start = now - Duration::seconds(window_secs + 120);
                let metrics = metrics_collector.get_global_range(start, now);

                let mut bucket_tokens: Vec<u64> = vec![0; NUM_BUCKETS as usize];

                // Direct mapping: each minute metric goes to exactly one bucket
                for metric in metrics.iter() {
                    let age_secs = now.signed_duration_since(metric.timestamp).num_seconds();
                    if age_secs < 0 || age_secs >= window_secs {
                        continue;
                    }

                    let bucket_index = (NUM_BUCKETS - 1) - (age_secs / 60);
                    let bucket_index = bucket_index.max(0).min(NUM_BUCKETS - 1) as usize;
                    bucket_tokens[bucket_index] += metric.total_tokens;
                }

                bucket_tokens
                    .into_iter()
                    .enumerate()
                    .map(|(i, tokens)| DataPoint {
                        timestamp: now - Duration::seconds((NUM_BUCKETS - i as i64) * 60),
                        total_tokens: tokens,
                    })
                    .collect::<Vec<_>>()
            }
        };

        // Generate graph PNG
        let graph_config = platform_graph_config();
        let png_bytes = crate::ui::tray_graph::generate_graph(&data_points, &graph_config)
            .ok_or_else(|| anyhow::anyhow!("Failed to generate graph PNG"))?;

        // Update tray icon
        if let Some(tray) = app_handle.tray_by_id("main") {
            let icon = tauri::image::Image::from_bytes(&png_bytes)
                .map_err(|e| anyhow::anyhow!("Failed to create image from PNG: {}", e))?;

            tray.set_icon(Some(icon))
                .map_err(|e| anyhow::anyhow!("Failed to set tray icon: {}", e))?;

            // Set template mode based on platform
            #[cfg(target_os = "macos")]
            tray.set_icon_as_template(true)
                .map_err(|e| anyhow::anyhow!("Failed to set template mode: {}", e))?;

            #[cfg(not(target_os = "macos"))]
            tray.set_icon_as_template(false)
                .map_err(|e| anyhow::anyhow!("Failed to set template mode: {}", e))?;

            debug!(
                "Tray icon updated with graph ({} buckets)",
                data_points.len()
            );
        } else {
            return Err(anyhow::anyhow!("Tray icon 'main' not found"));
        }

        Ok(())
    }

    /// Update configuration and apply immediately
    pub fn update_config(&self, new_config: UiConfig) {
        *self.config.write() = new_config;

        // If enabled, trigger an immediate update
        if self.config.read().tray_graph_enabled {
            self.notify_activity();
        } else {
            // If disabled, restore static icon
            self.restore_static_icon();
        }
    }

    /// Restore the static tray icon (when feature is disabled)
    fn restore_static_icon(&self) {
        const TRAY_ICON: &[u8] = include_bytes!("../../icons/32x32.png");

        if let Some(tray) = self.app_handle.tray_by_id("main") {
            if let Ok(icon) = tauri::image::Image::from_bytes(TRAY_ICON) {
                let _ = tray.set_icon(Some(icon));
                let _ = tray.set_icon_as_template(true);
                debug!("Restored static tray icon");
            }
        }
    }

    /// Check if the manager has been idle (no activity for >60 seconds)
    pub fn is_idle(&self) -> bool {
        let last_activity = *self.last_activity.read();
        let elapsed = Utc::now().signed_duration_since(last_activity);
        elapsed.num_seconds() > 60
    }

    /// Check if tray graph feature is enabled
    pub fn is_enabled(&self) -> bool {
        self.config.read().tray_graph_enabled
    }
}

/// Set update notification state and rebuild tray menu
pub fn set_update_available<R: Runtime>(app: &AppHandle<R>, available: bool) -> tauri::Result<()> {
    info!("Setting update notification state: {}", available);

    if let Some(update_state) = app.try_state::<Arc<UpdateNotificationState>>() {
        update_state.set_update_available(available);

        // Rebuild tray menu to show/hide update notification
        rebuild_tray_menu(app)?;

        info!("Tray menu rebuilt with update notification");
    } else {
        error!("UpdateNotificationState not found in app state");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::monitoring::metrics::MetricDataPoint;
    use chrono::{DateTime, Duration, Timelike, Utc};

    /// Helper to create test metrics
    fn create_metric(timestamp: DateTime<Utc>, tokens: u64) -> MetricDataPoint {
        MetricDataPoint {
            timestamp,
            requests: 1,
            input_tokens: tokens / 2,
            output_tokens: tokens / 2,
            total_tokens: tokens,
            cost_usd: 0.0,
            total_latency_ms: 0,
            successful_requests: 1,
            failed_requests: 0,
            latency_samples: vec![],
            p50_latency_ms: None,
            p95_latency_ms: None,
            p99_latency_ms: None,
        }
    }

    /// Test bucketing logic in isolation
    fn bucket_metrics(
        metrics: Vec<MetricDataPoint>,
        now: DateTime<Utc>,
        interval_secs: i64,
    ) -> Vec<u64> {
        const NUM_BUCKETS: i64 = 30;
        let window_secs = NUM_BUCKETS * interval_secs;
        let mut bucket_tokens: Vec<u64> = vec![0; NUM_BUCKETS as usize];

        for metric in metrics.iter() {
            let age_duration = now.signed_duration_since(metric.timestamp);
            let age_secs = age_duration.num_seconds();

            if age_secs < 0 || age_secs >= window_secs {
                continue;
            }

            let bucket_index = (NUM_BUCKETS - 1) - (age_secs / interval_secs);
            let bucket_index = bucket_index.max(0).min(NUM_BUCKETS - 1) as usize;
            bucket_tokens[bucket_index] += metric.total_tokens;
        }

        bucket_tokens
    }

    #[test]
    fn test_single_metric_assigns_to_correct_bucket() {
        let now = Utc::now();
        let interval_secs = 2;

        // Metric 3 seconds old should go to bucket 28
        // age=3s → bucket_index = 29 - (3/2) = 29 - 1 = 28
        let metric = create_metric(now - Duration::seconds(3), 100);
        let buckets = bucket_metrics(vec![metric], now, interval_secs);

        assert_eq!(
            buckets[28], 100,
            "Metric with age 3s should be in bucket 28"
        );
        assert_eq!(
            buckets.iter().sum::<u64>(),
            100,
            "Total tokens should be 100"
        );
    }

    #[test]
    fn test_metric_shifts_left_as_time_advances() {
        let base_time = Utc::now();
        let interval_secs = 2;

        // Create a metric at a fixed timestamp
        let metric_time = base_time;
        let metric = create_metric(metric_time, 100);

        // At T+0: metric age = 0s → bucket 29
        let buckets_t0 = bucket_metrics(vec![metric.clone()], base_time, interval_secs);
        assert_eq!(buckets_t0[29], 100, "At T+0, metric should be in bucket 29");

        // At T+2: metric age = 2s → bucket 28
        let buckets_t2 = bucket_metrics(
            vec![metric.clone()],
            base_time + Duration::seconds(2),
            interval_secs,
        );
        assert_eq!(buckets_t2[28], 100, "At T+2, metric should be in bucket 28");

        // At T+4: metric age = 4s → bucket 27
        let buckets_t4 = bucket_metrics(
            vec![metric.clone()],
            base_time + Duration::seconds(4),
            interval_secs,
        );
        assert_eq!(buckets_t4[27], 100, "At T+4, metric should be in bucket 27");

        // At T+58: metric age = 58s → bucket 0
        let buckets_t58 = bucket_metrics(
            vec![metric.clone()],
            base_time + Duration::seconds(58),
            interval_secs,
        );
        assert_eq!(buckets_t58[0], 100, "At T+58, metric should be in bucket 0");
    }

    #[test]
    fn test_metric_disappears_when_too_old() {
        let base_time = Utc::now();
        let interval_secs = 2;
        let metric = create_metric(base_time, 100);

        // At T+60: metric age = 60s, window is 60s → out of range
        let buckets = bucket_metrics(
            vec![metric],
            base_time + Duration::seconds(60),
            interval_secs,
        );
        assert_eq!(
            buckets.iter().sum::<u64>(),
            0,
            "Metric should disappear after 60 seconds"
        );
    }

    #[test]
    fn test_multiple_metrics_aggregate_in_same_bucket() {
        let now = Utc::now();
        let interval_secs = 2;

        // Two metrics with age 3s (both should go to bucket 28)
        let metric1 = create_metric(now - Duration::seconds(3), 100);
        let metric2 = create_metric(now - Duration::seconds(3), 200);

        let buckets = bucket_metrics(vec![metric1, metric2], now, interval_secs);

        assert_eq!(
            buckets[28], 300,
            "Both metrics should aggregate in bucket 28"
        );
    }

    #[test]
    fn test_multiple_metrics_in_different_buckets() {
        let now = Utc::now();
        let interval_secs = 2;

        // Metric 1: age 3s → bucket 28
        // Metric 2: age 5s → bucket 27
        // Metric 3: age 7s → bucket 26
        let metrics = vec![
            create_metric(now - Duration::seconds(3), 100),
            create_metric(now - Duration::seconds(5), 200),
            create_metric(now - Duration::seconds(7), 300),
        ];

        let buckets = bucket_metrics(metrics, now, interval_secs);

        assert_eq!(buckets[28], 100, "Metric 1 should be in bucket 28");
        assert_eq!(buckets[27], 200, "Metric 2 should be in bucket 27");
        assert_eq!(buckets[26], 300, "Metric 3 should be in bucket 26");
        assert_eq!(buckets.iter().sum::<u64>(), 600, "Total should be 600");
    }

    #[test]
    fn test_empty_metrics_produces_empty_buckets() {
        let now = Utc::now();
        let interval_secs = 2;

        let buckets = bucket_metrics(vec![], now, interval_secs);

        assert_eq!(buckets.len(), 30, "Should have 30 buckets");
        assert_eq!(
            buckets.iter().sum::<u64>(),
            0,
            "All buckets should be empty"
        );
    }

    #[test]
    fn test_future_metrics_are_ignored() {
        let now = Utc::now();
        let interval_secs = 2;

        // Metric from the future
        let metric = create_metric(now + Duration::seconds(10), 100);
        let buckets = bucket_metrics(vec![metric], now, interval_secs);

        assert_eq!(
            buckets.iter().sum::<u64>(),
            0,
            "Future metrics should be ignored"
        );
    }

    #[test]
    fn test_bucket_boundaries_with_minute_level_metrics() {
        // This tests the real-world scenario where metrics are stored at minute boundaries
        // but buckets are 2-second intervals
        let now = Utc::now();
        let interval_secs = 2;

        // Simulate a metric stored at the minute boundary (like in production)
        let metric_time =
            now.with_second(0).unwrap().with_nanosecond(0).unwrap() - Duration::minutes(0); // Current minute

        let metric = create_metric(metric_time, 100);

        // Calculate expected bucket based on age
        let age = now.signed_duration_since(metric_time).num_seconds();
        let expected_bucket = (29 - (age / interval_secs)) as usize;

        let buckets = bucket_metrics(vec![metric], now, interval_secs);

        assert_eq!(
            buckets[expected_bucket], 100,
            "Minute-boundary metric should be in bucket {}",
            expected_bucket
        );
    }

    #[test]
    fn test_consistent_bucket_assignment_over_time() {
        // Verify that as time advances by 1 second increments,
        // the metric stays in the same bucket until age crosses a 2-second boundary
        let base_time = Utc::now();
        let interval_secs = 2;
        let metric = create_metric(base_time, 100);

        // At T+0 and T+1: age 0s and 1s → both bucket 29
        let buckets_t0 = bucket_metrics(vec![metric.clone()], base_time, interval_secs);
        let buckets_t1 = bucket_metrics(
            vec![metric.clone()],
            base_time + Duration::seconds(1),
            interval_secs,
        );
        assert_eq!(buckets_t0[29], 100, "T+0: bucket 29");
        assert_eq!(buckets_t1[29], 100, "T+1: bucket 29 (same as T+0)");

        // At T+2 and T+3: age 2s and 3s → both bucket 28
        let buckets_t2 = bucket_metrics(
            vec![metric.clone()],
            base_time + Duration::seconds(2),
            interval_secs,
        );
        let buckets_t3 = bucket_metrics(
            vec![metric.clone()],
            base_time + Duration::seconds(3),
            interval_secs,
        );
        assert_eq!(buckets_t2[28], 100, "T+2: bucket 28");
        assert_eq!(buckets_t3[28], 100, "T+3: bucket 28 (same as T+2)");

        // At T+4 and T+5: age 4s and 5s → both bucket 27
        let buckets_t4 = bucket_metrics(
            vec![metric.clone()],
            base_time + Duration::seconds(4),
            interval_secs,
        );
        let buckets_t5 = bucket_metrics(
            vec![metric.clone()],
            base_time + Duration::seconds(5),
            interval_secs,
        );
        assert_eq!(buckets_t4[27], 100, "T+4: bucket 27");
        assert_eq!(buckets_t5[27], 100, "T+5: bucket 27 (same as T+4)");
    }

    #[test]
    fn test_all_30_buckets_fill_correctly() {
        let now = Utc::now();
        let interval_secs = 2;

        // Create 30 metrics, one for each bucket
        let mut metrics = Vec::new();
        for i in 0..30 {
            // Metric with age i*2 seconds should go to bucket (29-i)
            let age = i * interval_secs;
            metrics.push(create_metric(now - Duration::seconds(age), 100));
        }

        let buckets = bucket_metrics(metrics, now, interval_secs);

        // Each bucket should have exactly 100 tokens
        for (i, &tokens) in buckets.iter().enumerate() {
            assert_eq!(tokens, 100, "Bucket {} should have 100 tokens", i);
        }

        assert_eq!(
            buckets.iter().sum::<u64>(),
            3000,
            "Total should be 3000 (30 buckets * 100 tokens)"
        );
    }

    #[test]
    fn test_different_interval_sizes() {
        let now = Utc::now();

        // Test with 1-second intervals
        let metric = create_metric(now - Duration::seconds(5), 100);
        let buckets_1s = bucket_metrics(vec![metric.clone()], now, 1);
        // age=5s, interval=1s → bucket = 29 - (5/1) = 24
        assert_eq!(buckets_1s[24], 100, "1s interval: bucket 24");

        // Test with 5-second intervals
        let buckets_5s = bucket_metrics(vec![metric.clone()], now, 5);
        // age=5s, interval=5s → bucket = 29 - (5/5) = 28
        assert_eq!(buckets_5s[28], 100, "5s interval: bucket 28");
    }

    // ============================================================================
    // COMPREHENSIVE MODE-SPECIFIC TESTS WITH VIRTUAL TIME
    // ============================================================================

    /// Simulates Fast mode bucketing (1 second per bar, 26 bars)
    /// This matches the actual implementation in update_tray_graph_impl
    /// Simulates Fast mode bucketing (1 second per bar, 26 bars)
    /// Fast mode does NOT use metrics - it only tracks real-time tokens
    fn simulate_fast_mode_buckets(
        buckets: &mut Vec<u64>,
        accumulated_tokens: u64, // Real-time tokens since last update
        is_first_update: bool,
    ) {
        const NUM_BUCKETS: usize = 26;

        if is_first_update {
            // Start with empty buckets (no historical data)
            buckets.fill(0);
        } else {
            // Shift left (remove first, append 0 at end)
            buckets.rotate_left(1);
            buckets[NUM_BUCKETS - 1] = 0;
        }

        // Add accumulated real-time tokens to rightmost bucket
        buckets[NUM_BUCKETS - 1] = accumulated_tokens;
    }

    #[test]
    fn test_fast_mode_bucket_shifting() {
        // Fast mode: 1s per bar, 26 bars total (26 second window)
        // Uses real-time token accumulation, NOT metrics
        let mut buckets = vec![0u64; 26];

        // T=0: First update with 100 tokens
        simulate_fast_mode_buckets(&mut buckets, 100, true);
        assert_eq!(buckets[25], 100, "T=0: rightmost bucket should have 100");
        assert_eq!(buckets.iter().sum::<u64>(), 100);

        // T=1: Shift left, new activity with 200 tokens
        simulate_fast_mode_buckets(&mut buckets, 200, false);
        assert_eq!(buckets[24], 100, "T=1: previous data shifted to bucket 24");
        assert_eq!(buckets[25], 200, "T=1: new data in bucket 25");
        assert_eq!(buckets.iter().sum::<u64>(), 300);

        // T=2: Shift left, new activity with 150 tokens
        simulate_fast_mode_buckets(&mut buckets, 150, false);
        assert_eq!(buckets[23], 100, "T=2: oldest data at bucket 23");
        assert_eq!(buckets[24], 200, "T=2: second data at bucket 24");
        assert_eq!(buckets[25], 150, "T=2: newest data at bucket 25");
        assert_eq!(buckets.iter().sum::<u64>(), 450);

        // T=3-26: Continue shifting with varying tokens
        for i in 3..=26 {
            let tokens = 50 * i; // Varying token amounts
            simulate_fast_mode_buckets(&mut buckets, tokens as u64, false);
        }

        // Original 100 tokens from T=0 should have fallen off (26+ shifts)
        // But we should still have recent data from the last 26 updates
        let sum: u64 = buckets.iter().sum();
        assert!(sum > 0, "T=26: Should still have recent data");
        assert_eq!(
            buckets.iter().filter(|&&x| x == 0).count(),
            0,
            "All 26 buckets should be filled after 26 updates"
        );
    }

    #[test]
    fn test_fast_mode_continuous_activity() {
        // Simulate continuous token generation every second for 30 seconds
        let mut buckets = vec![0u64; 26];

        // Generate activity every second
        for t in 0..30 {
            let tokens = 100 + (t * 10); // Increasing tokens: 100, 110, 120, ...
            let is_first = t == 0;
            simulate_fast_mode_buckets(&mut buckets, tokens as u64, is_first);

            if t < 26 {
                // Should have t+1 buckets filled
                let non_zero_count = buckets.iter().filter(|&&x| x > 0).count();
                assert_eq!(
                    non_zero_count,
                    (t + 1) as usize,
                    "At T={}, should have {} non-zero buckets",
                    t,
                    t + 1
                );
            } else {
                // Should have exactly 26 buckets filled (window is full)
                let non_zero_count = buckets.iter().filter(|&&x| x > 0).count();
                assert_eq!(
                    non_zero_count, 26,
                    "At T={}, should have 26 buckets (window full)",
                    t
                );
            }
        }

        // At T=29, rightmost bucket should have the latest data (390 tokens)
        assert_eq!(
            buckets[25], 390,
            "Latest data should be in rightmost bucket"
        );
    }

    /// Simulates Medium mode bucketing (10 seconds per bar, 26 bars)
    /// Simulates Medium mode bucketing (10 seconds per bar, 26 bars)
    /// Medium mode uses metrics ONLY for initial load, then real-time tokens
    fn simulate_medium_mode_buckets(
        buckets: &mut Vec<u64>,
        metrics: Vec<MetricDataPoint>,
        virtual_now: DateTime<Utc>,
        accumulated_tokens: u64, // Real-time tokens since last update (used in runtime)
        is_first_update: bool,
    ) {
        const NUM_BUCKETS: usize = 26;
        const INTERVAL_SECS: i64 = 10;

        if is_first_update {
            // Initial load: Interpolate from minute-level metrics
            let window_secs = (NUM_BUCKETS as i64) * INTERVAL_SECS; // 260 seconds
            let start = virtual_now - Duration::seconds(window_secs + 120);

            buckets.fill(0);

            // Interpolate each minute across 6 buckets (60s / 10s = 6)
            for metric in metrics.iter() {
                if metric.timestamp < start {
                    continue;
                }

                let age_secs = virtual_now
                    .signed_duration_since(metric.timestamp)
                    .num_seconds();
                if age_secs < 0 || age_secs >= window_secs {
                    continue;
                }

                // Determine how many buckets we can actually place (some might fall outside window)
                let num_buckets_in_window = (0..6)
                    .filter(|&offset| age_secs + (offset * INTERVAL_SECS) < window_secs)
                    .count() as u64;

                if num_buckets_in_window == 0 {
                    continue;
                }

                let tokens_per_bucket = metric.total_tokens / num_buckets_in_window;

                for offset in 0..6 {
                    let bucket_age_secs = age_secs + (offset * INTERVAL_SECS);
                    if bucket_age_secs >= window_secs {
                        break;
                    }

                    let bucket_index = (NUM_BUCKETS as i64 - 1) - (bucket_age_secs / INTERVAL_SECS);
                    let bucket_index = bucket_index.max(0).min((NUM_BUCKETS - 1) as i64) as usize;
                    buckets[bucket_index] += tokens_per_bucket;
                }
            }
        } else {
            // Runtime: Use accumulated real-time tokens (NO metrics query)
            buckets.rotate_left(1);
            buckets[NUM_BUCKETS - 1] = 0;

            // Add accumulated tokens to rightmost bucket
            buckets[NUM_BUCKETS - 1] = accumulated_tokens;
        }
    }

    #[test]
    fn test_medium_mode_interpolation() {
        // Medium mode: 10s per bar, 26 bars total (260 second window = 4.33 minutes)
        let base_time = Utc::now();
        let mut buckets = vec![0u64; 26];

        // Create minute-level metrics (as stored in production)
        // One metric at T=0 with 600 tokens (should be interpolated across 6 buckets)
        let metrics = vec![create_metric(base_time, 600)];

        simulate_medium_mode_buckets(&mut buckets, metrics.clone(), base_time, 0, true);

        // Each of the last 6 buckets (representing 0-59 seconds) should have 100 tokens
        for i in 20..26 {
            assert_eq!(
                buckets[i], 100,
                "Bucket {} should have 100 tokens from interpolation",
                i
            );
        }

        // Older buckets should be empty
        for i in 0..20 {
            assert_eq!(buckets[i], 0, "Bucket {} should be empty", i);
        }

        assert_eq!(buckets.iter().sum::<u64>(), 600, "Total should be 600");
    }

    #[test]
    fn test_medium_mode_shifting() {
        let base_time = Utc::now();
        let mut buckets = vec![0u64; 26];

        // Initial: Create metric at base_time and interpolate
        let initial_metrics = vec![create_metric(base_time, 600)];
        simulate_medium_mode_buckets(&mut buckets, initial_metrics, base_time, 0, true);

        let initial_sum: u64 = buckets.iter().sum();
        assert_eq!(initial_sum, 600, "Initial sum should be 600");

        // T=10: Shift and add new real-time data (200 tokens accumulated)
        simulate_medium_mode_buckets(&mut buckets, vec![], base_time, 200, false);

        // Buckets should have shifted left
        assert_eq!(buckets[25], 200, "T=10: new data in rightmost bucket");

        // Should have shifted data + new data
        let sum_after_shift: u64 = buckets.iter().sum();
        // 600 tokens interpolated across buckets 20-25 (100 each)
        // After shift: buckets 19-24 now have 100 each (5 buckets), bucket 25 has 200
        // Lost bucket 0 (which was 0), so total: 500 + 200 = 700
        // NOTE: If getting 800, we lost nothing (all 600 + 200 new)
        assert!(
            sum_after_shift >= 700,
            "T=10: should have at least 700 tokens, got {}",
            sum_after_shift
        );
    }

    #[test]
    fn test_medium_mode_multiple_minute_metrics() {
        // Test with multiple minute-level metrics
        let base_time = Utc::now();
        let mut buckets = vec![0u64; 26];

        // Create 3 minute-level metrics, each 60 seconds apart
        let metrics = vec![
            create_metric(base_time - Duration::seconds(120), 600), // 2 minutes ago
            create_metric(base_time - Duration::seconds(60), 1200), // 1 minute ago
            create_metric(base_time, 1800),                         // now
        ];

        simulate_medium_mode_buckets(&mut buckets, metrics, base_time, 0, true);

        // Total should be sum of all metrics
        let total: u64 = buckets.iter().sum();
        assert_eq!(total, 3600, "Total should be 600 + 1200 + 1800 = 3600");

        // Most recent minute (buckets 20-25) should have 1800/6 = 300 per bucket
        for i in 20..26 {
            assert_eq!(buckets[i], 300, "Bucket {} should have 300 tokens", i);
        }

        // Middle minute (buckets 14-19) should have 1200/6 = 200 per bucket
        for i in 14..20 {
            assert_eq!(buckets[i], 200, "Bucket {} should have 200 tokens", i);
        }

        // Oldest minute (buckets 8-13) should have 600/6 = 100 per bucket
        for i in 8..14 {
            assert_eq!(buckets[i], 100, "Bucket {} should have 100 tokens", i);
        }
    }

    /// Simulates Slow mode bucketing (60 seconds per bar, 26 bars)
    fn simulate_slow_mode_buckets(
        metrics: Vec<MetricDataPoint>,
        virtual_now: DateTime<Utc>,
    ) -> Vec<u64> {
        const NUM_BUCKETS: usize = 26;
        const INTERVAL_SECS: i64 = 60;
        let window_secs = (NUM_BUCKETS as i64) * INTERVAL_SECS; // 1560 seconds = 26 minutes

        let mut bucket_tokens = vec![0u64; NUM_BUCKETS];

        // Direct mapping: each minute metric goes to exactly one bucket
        for metric in metrics.iter() {
            let age_secs = virtual_now
                .signed_duration_since(metric.timestamp)
                .num_seconds();
            if age_secs < 0 || age_secs >= window_secs {
                continue;
            }

            let bucket_index = (NUM_BUCKETS as i64 - 1) - (age_secs / INTERVAL_SECS);
            let bucket_index = bucket_index.max(0).min((NUM_BUCKETS - 1) as i64) as usize;
            bucket_tokens[bucket_index] += metric.total_tokens;
        }

        bucket_tokens
    }

    #[test]
    fn test_slow_mode_direct_mapping() {
        // Slow mode: 60s per bar, 26 bars total (1560 seconds = 26 minutes)
        let base_time = Utc::now();

        // Create one metric per minute for 26 minutes
        let mut metrics = Vec::new();
        for i in 0..26 {
            let timestamp = base_time - Duration::seconds(i * 60);
            metrics.push(create_metric(timestamp, (100 * (i + 1)) as u64));
        }

        let buckets = simulate_slow_mode_buckets(metrics, base_time);

        // Each bucket should have exactly one metric's worth of tokens
        assert_eq!(buckets[25], 100, "Most recent bucket");
        assert_eq!(buckets[24], 200, "1 minute ago");
        assert_eq!(buckets[23], 300, "2 minutes ago");
        assert_eq!(buckets[0], 2600, "25 minutes ago");

        let total: u64 = buckets.iter().sum();
        // Sum of 100, 200, 300, ..., 2600 = 100 * (1+2+3+...+26) = 100 * 351 = 35100
        assert_eq!(total, 35100, "Total should be sum of arithmetic series");
    }

    #[test]
    fn test_slow_mode_virtual_time_progression() {
        let base_time = Utc::now();

        // Create initial metrics
        let mut metrics = vec![
            create_metric(base_time - Duration::seconds(120), 1000), // 2 min ago
            create_metric(base_time - Duration::seconds(60), 2000),  // 1 min ago
            create_metric(base_time, 3000),                          // now
        ];

        // At T=0
        let buckets_t0 = simulate_slow_mode_buckets(metrics.clone(), base_time);
        assert_eq!(buckets_t0[25], 3000, "T=0: most recent in bucket 25");
        assert_eq!(buckets_t0[24], 2000, "T=0: 1 min ago in bucket 24");
        assert_eq!(buckets_t0[23], 1000, "T=0: 2 min ago in bucket 23");

        // Advance time by 60 seconds
        let t60 = base_time + Duration::seconds(60);
        metrics.push(create_metric(t60, 4000));
        let buckets_t60 = simulate_slow_mode_buckets(metrics.clone(), t60);

        assert_eq!(buckets_t60[25], 4000, "T=60: new data in bucket 25");
        assert_eq!(
            buckets_t60[24], 3000,
            "T=60: previous bucket 25 shifted to 24"
        );
        assert_eq!(
            buckets_t60[23], 2000,
            "T=60: previous bucket 24 shifted to 23"
        );
        assert_eq!(
            buckets_t60[22], 1000,
            "T=60: previous bucket 23 shifted to 22"
        );

        // Advance time by another 60 seconds (T=120)
        let t120 = base_time + Duration::seconds(120);
        metrics.push(create_metric(t120, 5000));
        let buckets_t120 = simulate_slow_mode_buckets(metrics.clone(), t120);

        assert_eq!(buckets_t120[25], 5000, "T=120: newest data");
        assert_eq!(buckets_t120[24], 4000, "T=120: T=60 data shifted");
        assert_eq!(buckets_t120[23], 3000, "T=120: T=0 data shifted");
        assert_eq!(buckets_t120[22], 2000, "T=120: T=-60 data shifted");
        assert_eq!(buckets_t120[21], 1000, "T=120: T=-120 data shifted");
    }

    #[test]
    fn test_slow_mode_metric_expiration() {
        let base_time = Utc::now();

        // Create a metric just inside the window edge (25 minutes 30 seconds old)
        // Window is [0, 26 minutes), so 26 minutes exactly is outside
        let old_metric = create_metric(base_time - Duration::seconds(25 * 60 + 30), 1000);
        let buckets = simulate_slow_mode_buckets(vec![old_metric.clone()], base_time);

        // Should be in bucket 0 (oldest bucket, covering 25-26 minutes ago)
        assert_eq!(buckets[0], 1000, "25.5-minute-old metric in bucket 0");

        // Advance time by 60 seconds - metric is now 26.5 minutes old, outside window
        let t60 = base_time + Duration::seconds(60);
        let buckets_t60 = simulate_slow_mode_buckets(vec![old_metric], t60);

        assert_eq!(
            buckets_t60.iter().sum::<u64>(),
            0,
            "26.5-minute-old metric should be expired (outside 26-minute window)"
        );
    }

    #[test]
    fn test_all_modes_handle_empty_metrics() {
        let base_time = Utc::now();
        let empty_metrics = Vec::new();

        // Fast mode (starts empty, no metrics)
        let mut fast_buckets = vec![0u64; 26];
        simulate_fast_mode_buckets(&mut fast_buckets, 0, true);
        assert_eq!(
            fast_buckets.iter().sum::<u64>(),
            0,
            "Fast mode: starts with zero buckets"
        );

        // Medium mode
        let mut medium_buckets = vec![0u64; 26];
        simulate_medium_mode_buckets(
            &mut medium_buckets,
            empty_metrics.clone(),
            base_time,
            0,
            true,
        );
        assert_eq!(
            medium_buckets.iter().sum::<u64>(),
            0,
            "Medium mode: empty metrics should produce zero buckets"
        );

        // Slow mode
        let slow_buckets = simulate_slow_mode_buckets(empty_metrics, base_time);
        assert_eq!(
            slow_buckets.iter().sum::<u64>(),
            0,
            "Slow mode: empty metrics should produce zero buckets"
        );
    }

    #[test]
    fn test_all_modes_handle_sparse_data() {
        let base_time = Utc::now();

        // Create sparse metrics: only at T=0, T=-120, T=-240
        let sparse_metrics = vec![
            create_metric(base_time, 100),
            create_metric(base_time - Duration::seconds(120), 200),
            create_metric(base_time - Duration::seconds(240), 300),
        ];

        // Fast mode: Starts empty (no metrics, only real-time tokens)
        let mut fast_buckets = vec![0u64; 26];
        simulate_fast_mode_buckets(&mut fast_buckets, 100, true);
        assert_eq!(fast_buckets[25], 100, "Fast mode: real-time data");
        assert_eq!(
            fast_buckets.iter().sum::<u64>(),
            100,
            "Fast mode: only recent real-time tokens"
        );

        // Medium mode: Should interpolate metrics on initial load
        let mut medium_buckets = vec![0u64; 26];
        simulate_medium_mode_buckets(
            &mut medium_buckets,
            sparse_metrics.clone(),
            base_time,
            0,
            true,
        );
        assert!(
            medium_buckets.iter().sum::<u64>() >= 100,
            "Medium mode: should have at least recent data"
        );

        // Slow mode: Should show data in discrete buckets
        let slow_buckets = simulate_slow_mode_buckets(sparse_metrics, base_time);
        assert_eq!(slow_buckets[25], 100, "Slow mode: bucket 25 (now)");
        assert_eq!(slow_buckets[23], 200, "Slow mode: bucket 23 (2 min ago)");
        assert_eq!(slow_buckets[21], 300, "Slow mode: bucket 21 (4 min ago)");
    }

    #[test]
    fn test_mode_comparison_with_same_data() {
        // Compare all three modes with identical input data
        let base_time = Utc::now();

        // Create consistent metrics: one per minute for 5 minutes
        let metrics: Vec<_> = (0..5)
            .map(|i| create_metric(base_time - Duration::seconds(i * 60), 1000))
            .collect();

        // Fast mode: Starts empty (no historical metrics, only real-time)
        let mut fast_buckets = vec![0u64; 26];
        simulate_fast_mode_buckets(&mut fast_buckets, 0, true);
        let fast_sum: u64 = fast_buckets.iter().sum();

        // Medium mode: Loads historical metrics with interpolation
        let mut medium_buckets = vec![0u64; 26];
        simulate_medium_mode_buckets(&mut medium_buckets, metrics.clone(), base_time, 0, true);
        let medium_sum: u64 = medium_buckets.iter().sum();

        // Slow mode: Loads all historical metrics
        let slow_buckets = simulate_slow_mode_buckets(metrics, base_time);
        let slow_sum: u64 = slow_buckets.iter().sum();

        // Fast mode starts empty (no metrics)
        assert_eq!(fast_sum, 0, "Fast mode starts empty (no historical data)");

        // Medium and slow should both capture all 5 minutes of data
        // Note: Medium mode may lose a few tokens to integer division rounding during interpolation
        assert!(
            medium_sum >= 4980 && medium_sum <= 5000,
            "Medium mode should capture ~5000 tokens (got {}), small rounding loss OK",
            medium_sum
        );
        assert_eq!(slow_sum, 5000, "Slow mode should capture all 5000 tokens");

        // Medium and slow should be nearly identical on initial load
        // (Medium may have small rounding loss from interpolation)
        assert!(
            (medium_sum as i64 - slow_sum as i64).abs() <= 20,
            "Medium and slow modes should nearly match on initial load (medium: {}, slow: {})",
            medium_sum,
            slow_sum
        );
    }
}
