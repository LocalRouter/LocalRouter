//! System tray management
//!
//! Handles system tray icon and menu.

use crate::api_keys::ApiKeyManager;
use crate::config::{ActiveRoutingStrategy, ConfigManager, ModelSelection};
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
                    } else if let Some(rest) = id.strip_prefix("force_model_") {
                        // Format: force_model_{key_id}_{provider}_{model}
                        if let Some((key_id, rest)) = rest.split_once('_') {
                            if let Some((provider, model)) = rest.split_once('_') {
                                info!("Force model requested: key={}, provider={}, model={}", key_id, provider, model);
                                let app_clone = app.clone();
                                let key_id = key_id.to_string();
                                let provider = provider.to_string();
                                let model = model.to_string();
                                tauri::async_runtime::spawn(async move {
                                    if let Err(e) = handle_force_model(&app_clone, &key_id, &provider, &model).await {
                                        error!("Failed to force model: {}", e);
                                    }
                                });
                            }
                        }
                    } else if let Some(key_id) = id.strip_prefix("enable_available_models_") {
                        info!("Enable available models strategy: key={}", key_id);
                        let app_clone = app.clone();
                        let key_id = key_id.to_string();
                        tauri::async_runtime::spawn(async move {
                            if let Err(e) = handle_enable_available_models(&app_clone, &key_id).await {
                                error!("Failed to enable available models: {}", e);
                            }
                        });
                    } else if let Some(rest) = id.strip_prefix("toggle_provider_") {
                        // Format: toggle_provider_{key_id}_{provider}
                        if let Some((key_id, provider)) = rest.split_once('_') {
                            info!("Toggle provider requested: key={}, provider={}", key_id, provider);
                            let app_clone = app.clone();
                            let key_id = key_id.to_string();
                            let provider = provider.to_string();
                            tauri::async_runtime::spawn(async move {
                                if let Err(e) = handle_toggle_provider(&app_clone, &key_id, &provider).await {
                                    error!("Failed to toggle provider: {}", e);
                                }
                            });
                        }
                    } else if let Some(rest) = id.strip_prefix("toggle_model_") {
                        // Format: toggle_model_{key_id}_{provider}_{model}
                        if let Some((key_id, rest)) = rest.split_once('_') {
                            if let Some((provider, model)) = rest.split_once('_') {
                                info!("Toggle model requested: key={}, provider={}, model={}", key_id, provider, model);
                                let app_clone = app.clone();
                                let key_id = key_id.to_string();
                                let provider = provider.to_string();
                                let model = model.to_string();
                                tauri::async_runtime::spawn(async move {
                                    if let Err(e) = handle_toggle_available_model(&app_clone, &key_id, &provider, &model).await {
                                        error!("Failed to toggle model: {}", e);
                                    }
                                });
                            }
                        }
                    } else if let Some(key_id) = id.strip_prefix("prioritized_list_") {
                        info!("Prioritized list requested: key={}", key_id);
                        let app_clone = app.clone();
                        let key_id = key_id.to_string();
                        tauri::async_runtime::spawn(async move {
                            if let Err(e) = handle_prioritized_list(&app_clone, &key_id).await {
                                error!("Failed to open prioritized list: {}", e);
                            }
                        });
                    } else if let Some(rest) = id.strip_prefix("set_model_") {
                        // Legacy: Formats:
                        // - set_model_{key_id}_all
                        // - set_model_{key_id}_provider_{provider}
                        // - set_model_{key_id}_model_{provider}_{model}
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

    // Add API Keys section header
    menu_builder = menu_builder.text("api_keys_header", "API Keys");

    // Get API keys from manager and provider registry
    if let Some(key_manager) = app.try_state::<ApiKeyManager>() {
        let keys = key_manager.list_keys();

        if !keys.is_empty() {
            // Get provider registry to fetch models
            let provider_registry = app.try_state::<Arc<ProviderRegistry>>();

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

                // Add separator before routing strategy section
                submenu_builder = submenu_builder.separator();

                // Get routing config for this key
                let routing_config = key.get_routing_config();
                let active_strategy = routing_config.as_ref().map(|c| c.active_strategy);

                // Get cached models from registry
                let models = if let Some(ref registry) = provider_registry {
                    registry.get_cached_models()
                } else {
                    vec![]
                };

                if !models.is_empty() {
                    // 1. "Force Model" submenu
                    let force_model_text = if matches!(active_strategy, Some(ActiveRoutingStrategy::ForceModel)) {
                        "✓ Force Model"
                    } else {
                        "Force Model"
                    };
                    let mut force_model_submenu_builder = SubmenuBuilder::new(app, force_model_text);

                    // Get the currently forced model (if any)
                    let forced_model = routing_config.as_ref().and_then(|c| c.forced_model.as_ref());

                    for model in models.iter() {
                        let model_display = format!("{} ({})", model.id, model.provider);
                        let is_forced = if let Some((provider, model_name)) = forced_model {
                            provider == &model.provider && model_name == &model.id
                        } else {
                            false
                        };
                        let display_text = if is_forced {
                            format!("✓ {}", model_display)
                        } else {
                            model_display
                        };

                        force_model_submenu_builder = force_model_submenu_builder.text(
                            format!("force_model_{}_{}_{}", key.id, model.provider, model.id),
                            display_text
                        );
                    }

                    let force_model_submenu = force_model_submenu_builder.build()?;
                    submenu_builder = submenu_builder.item(&force_model_submenu);

                    // 2. "Available Models" submenu
                    let available_models_text = if matches!(active_strategy, Some(ActiveRoutingStrategy::AvailableModels)) {
                        "✓ Available Models"
                    } else {
                        "Available Models"
                    };
                    let mut available_models_submenu_builder = SubmenuBuilder::new(app, available_models_text);

                    // Add strategy toggle at the top
                    let toggle_text = if matches!(active_strategy, Some(ActiveRoutingStrategy::AvailableModels)) {
                        "✓ Use This Strategy (Active)"
                    } else {
                        "Use This Strategy"
                    };
                    available_models_submenu_builder = available_models_submenu_builder.text(
                        format!("enable_available_models_{}", key.id),
                        toggle_text
                    );

                    available_models_submenu_builder = available_models_submenu_builder.separator();

                    // Get available models selection
                    let available_models = routing_config.as_ref().map(|c| &c.available_models);

                    // Collect unique providers
                    let mut providers: Vec<String> = models.iter()
                        .map(|m| m.provider.clone())
                        .collect::<std::collections::HashSet<_>>()
                        .into_iter()
                        .collect();
                    providers.sort();

                    // Add provider options (all models from each provider)
                    if !providers.is_empty() {
                        for provider in providers.iter() {
                            let is_provider_selected = if let Some(avail) = available_models {
                                avail.all_provider_models.contains(provider)
                            } else {
                                false
                            };
                            let provider_text = if is_provider_selected {
                                format!("✓ All {} Models", provider)
                            } else {
                                format!("All {} Models", provider)
                            };
                            available_models_submenu_builder = available_models_submenu_builder.text(
                                format!("toggle_provider_{}_{}",key.id, provider),
                                provider_text
                            );
                        }

                        available_models_submenu_builder = available_models_submenu_builder.separator();
                    }

                    // Add individual models
                    for model in models.iter() {
                        let model_display = format!("{} ({})", model.id, model.provider);
                        let is_selected = if let Some(avail) = available_models {
                            avail.individual_models.iter().any(|(p, m)| p == &model.provider && m == &model.id)
                        } else {
                            false
                        };
                        let display_text = if is_selected {
                            format!("✓ {}", model_display)
                        } else {
                            model_display
                        };

                        available_models_submenu_builder = available_models_submenu_builder.text(
                            format!("toggle_model_{}_{}_{}", key.id, model.provider, model.id),
                            display_text
                        );
                    }

                    let available_models_submenu = available_models_submenu_builder.build()?;
                    submenu_builder = submenu_builder.item(&available_models_submenu);

                    // 3. "Prioritized List..." menu item
                    let prioritized_list_text = if matches!(active_strategy, Some(ActiveRoutingStrategy::PrioritizedList)) {
                        "✓ Prioritized List..."
                    } else {
                        "Prioritized List..."
                    };
                    submenu_builder = submenu_builder.text(
                        format!("prioritized_list_{}", key.id),
                        prioritized_list_text
                    );
                } else {
                    // No models available yet
                    submenu_builder = submenu_builder.text(
                        format!("no_models_{}", key.id),
                        "No models available"
                    );
                }

                let submenu = submenu_builder.build()?;
                menu_builder = menu_builder.item(&submenu);
            }
        }
    }

    // Add "Generate API Key" without separator before it
    menu_builder = menu_builder.text("generate_key", "➕ Generate API Key");

    // Add separator before Server section
    menu_builder = menu_builder.separator();

    // Get server status for section header and button text
    let (server_status_icon, server_text) = if let Some(server_manager) = app.try_state::<Arc<crate::server::ServerManager>>() {
        match server_manager.get_status() {
            crate::server::ServerStatus::Running => ("✓", "⏹️ Stop Server"),
            crate::server::ServerStatus::Stopped => ("✗", "▶️ Start Server"),
        }
    } else {
        ("✗", "▶️ Start Server")
    };

    // Add Server section header with status indicator
    menu_builder = menu_builder.text("server_header", format!("Server {}", server_status_icon));

    // Get port for URL display
    let port = if let Some(config_manager) = app.try_state::<ConfigManager>() {
        let config = config_manager.get();
        config.server.port
    } else {
        3000 // default
    };

    // Add server-related items
    menu_builder = menu_builder
        .text("copy_url", format!("📋 Copy URL :{}", port))
        .text("toggle_server", server_text);

    // Add separator before bottom items
    menu_builder = menu_builder.separator();

    // Add bottom menu items
    menu_builder = menu_builder
        .text("open_dashboard", "📊 Open Dashboard")
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

    // Add API Keys section header
    menu_builder = menu_builder.text("api_keys_header", "API Keys");

    // Get API keys from manager
    if let Some(key_manager) = app.try_state::<ApiKeyManager>() {
        let keys = key_manager.list_keys();

        if !keys.is_empty() {
            // Get provider registry to fetch models
            let provider_registry = app.try_state::<Arc<ProviderRegistry>>();

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

                // Add separator before routing strategy section
                submenu_builder = submenu_builder.separator();

                // Get routing config for this key
                let routing_config = key.get_routing_config();
                let active_strategy = routing_config.as_ref().map(|c| c.active_strategy);

                // Get cached models from registry
                let models = if let Some(ref registry) = provider_registry {
                    registry.get_cached_models()
                } else {
                    vec![]
                };

                if !models.is_empty() {
                    // 1. "Force Model" submenu
                    let force_model_text = if matches!(active_strategy, Some(ActiveRoutingStrategy::ForceModel)) {
                        "✓ Force Model"
                    } else {
                        "Force Model"
                    };
                    let mut force_model_submenu_builder = SubmenuBuilder::new(app, force_model_text);

                    // Get the currently forced model (if any)
                    let forced_model = routing_config.as_ref().and_then(|c| c.forced_model.as_ref());

                    for model in models.iter() {
                        let model_display = format!("{} ({})", model.id, model.provider);
                        let is_forced = if let Some((provider, model_name)) = forced_model {
                            provider == &model.provider && model_name == &model.id
                        } else {
                            false
                        };
                        let display_text = if is_forced {
                            format!("✓ {}", model_display)
                        } else {
                            model_display
                        };

                        force_model_submenu_builder = force_model_submenu_builder.text(
                            format!("force_model_{}_{}_{}", key.id, model.provider, model.id),
                            display_text
                        );
                    }

                    let force_model_submenu = force_model_submenu_builder.build()?;
                    submenu_builder = submenu_builder.item(&force_model_submenu);

                    // 2. "Available Models" submenu
                    let available_models_text = if matches!(active_strategy, Some(ActiveRoutingStrategy::AvailableModels)) {
                        "✓ Available Models"
                    } else {
                        "Available Models"
                    };
                    let mut available_models_submenu_builder = SubmenuBuilder::new(app, available_models_text);

                    // Add strategy toggle at the top
                    let toggle_text = if matches!(active_strategy, Some(ActiveRoutingStrategy::AvailableModels)) {
                        "✓ Use This Strategy (Active)"
                    } else {
                        "Use This Strategy"
                    };
                    available_models_submenu_builder = available_models_submenu_builder.text(
                        format!("enable_available_models_{}", key.id),
                        toggle_text
                    );

                    available_models_submenu_builder = available_models_submenu_builder.separator();

                    // Get available models selection
                    let available_models = routing_config.as_ref().map(|c| &c.available_models);

                    // Collect unique providers
                    let mut providers: Vec<String> = models.iter()
                        .map(|m| m.provider.clone())
                        .collect::<std::collections::HashSet<_>>()
                        .into_iter()
                        .collect();
                    providers.sort();

                    // Add provider options (all models from each provider)
                    if !providers.is_empty() {
                        for provider in providers.iter() {
                            let is_provider_selected = if let Some(avail) = available_models {
                                avail.all_provider_models.contains(provider)
                            } else {
                                false
                            };
                            let provider_text = if is_provider_selected {
                                format!("✓ All {} Models", provider)
                            } else {
                                format!("All {} Models", provider)
                            };
                            available_models_submenu_builder = available_models_submenu_builder.text(
                                format!("toggle_provider_{}_{}",key.id, provider),
                                provider_text
                            );
                        }

                        available_models_submenu_builder = available_models_submenu_builder.separator();
                    }

                    // Add individual models
                    for model in models.iter() {
                        let model_display = format!("{} ({})", model.id, model.provider);
                        let is_selected = if let Some(avail) = available_models {
                            avail.individual_models.iter().any(|(p, m)| p == &model.provider && m == &model.id)
                        } else {
                            false
                        };
                        let display_text = if is_selected {
                            format!("✓ {}", model_display)
                        } else {
                            model_display
                        };

                        available_models_submenu_builder = available_models_submenu_builder.text(
                            format!("toggle_model_{}_{}_{}", key.id, model.provider, model.id),
                            display_text
                        );
                    }

                    let available_models_submenu = available_models_submenu_builder.build()?;
                    submenu_builder = submenu_builder.item(&available_models_submenu);

                    // 3. "Prioritized List..." menu item
                    let prioritized_list_text = if matches!(active_strategy, Some(ActiveRoutingStrategy::PrioritizedList)) {
                        "✓ Prioritized List..."
                    } else {
                        "Prioritized List..."
                    };
                    submenu_builder = submenu_builder.text(
                        format!("prioritized_list_{}", key.id),
                        prioritized_list_text
                    );
                } else {
                    // No models available yet
                    submenu_builder = submenu_builder.text(
                        format!("no_models_{}", key.id),
                        "No models available"
                    );
                }

                let submenu = submenu_builder.build()?;
                menu_builder = menu_builder.item(&submenu);
            }
        }
    }

    // Add "Generate API Key" without separator before it
    menu_builder = menu_builder.text("generate_key", "➕ Generate API Key");

    // Add separator before Server section
    menu_builder = menu_builder.separator();

    // Get server status for section header and button text
    let (server_status_icon, server_text) = if let Some(server_manager) = app.try_state::<Arc<crate::server::ServerManager>>() {
        match server_manager.get_status() {
            crate::server::ServerStatus::Running => ("✓", "⏹️ Stop Server"),
            crate::server::ServerStatus::Stopped => ("✗", "▶️ Start Server"),
        }
    } else {
        ("✗", "▶️ Start Server")
    };

    // Add Server section header with status indicator
    menu_builder = menu_builder.text("server_header", format!("Server {}", server_status_icon));

    // Get port for URL display
    let port = if let Some(config_manager) = app.try_state::<ConfigManager>() {
        let config = config_manager.get();
        config.server.port
    } else {
        3000 // default
    };

    // Add server-related items
    menu_builder = menu_builder
        .text("copy_url", format!("📋 Copy URL :{}", port))
        .text("toggle_server", server_text);

    // Add separator before bottom items
    menu_builder = menu_builder.separator();

    // Add bottom menu items
    menu_builder = menu_builder
        .text("open_dashboard", "📊 Open Dashboard")
        .text("quit", "❌ Quit");

    menu_builder.build()
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
///
/// Supports toggling between different model selection types:
/// - "all" - Set to ModelSelection::All
/// - "provider_{name}" - Toggle all models from a provider
/// - "model_{provider}_{model}" - Toggle a specific model
async fn handle_set_model<R: Runtime>(app: &AppHandle<R>, key_id: &str, model_spec: &str) -> tauri::Result<()> {
    let key_manager = app.state::<ApiKeyManager>();
    let config_manager = app.state::<ConfigManager>();

    info!("Setting model {} for key {}", model_spec, key_id);

    // Get current key configuration
    let current_key = key_manager
        .get_key(key_id)
        .ok_or_else(|| tauri::Error::Anyhow(anyhow::anyhow!("API key not found")))?;

    let new_selection = if model_spec == "all" {
        // Set to "All Models"
        ModelSelection::All
    } else if let Some(provider) = model_spec.strip_prefix("provider_") {
        // Toggle provider in Custom selection
        match &current_key.model_selection {
            Some(ModelSelection::Custom { all_provider_models, individual_models }) => {
                let mut new_providers = all_provider_models.clone();
                let new_individual = individual_models.clone();

                // Toggle: if provider is already selected, remove it; otherwise add it
                if let Some(pos) = new_providers.iter().position(|p| p == provider) {
                    new_providers.remove(pos);
                } else {
                    new_providers.push(provider.to_string());
                }

                ModelSelection::Custom {
                    all_provider_models: new_providers,
                    individual_models: new_individual,
                }
            }
            _ => {
                // If not Custom, create new Custom with just this provider
                ModelSelection::Custom {
                    all_provider_models: vec![provider.to_string()],
                    individual_models: vec![],
                }
            }
        }
    } else if let Some(rest) = model_spec.strip_prefix("model_") {
        // Toggle individual model in Custom selection
        // Format: model_{provider}_{model}
        if let Some((provider, model)) = rest.split_once('_') {
            match &current_key.model_selection {
                Some(ModelSelection::Custom { all_provider_models, individual_models }) => {
                    let new_providers = all_provider_models.clone();
                    let mut new_individual = individual_models.clone();

                    // Toggle: if model is already selected, remove it; otherwise add it
                    let model_tuple = (provider.to_string(), model.to_string());
                    if let Some(pos) = new_individual.iter().position(|m| m == &model_tuple) {
                        new_individual.remove(pos);
                    } else {
                        new_individual.push(model_tuple);
                    }

                    ModelSelection::Custom {
                        all_provider_models: new_providers,
                        individual_models: new_individual,
                    }
                }
                _ => {
                    // If not Custom, create new Custom with just this model
                    ModelSelection::Custom {
                        all_provider_models: vec![],
                        individual_models: vec![(provider.to_string(), model.to_string())],
                    }
                }
            }
        } else {
            return Err(tauri::Error::Anyhow(anyhow::anyhow!("Invalid model spec format")));
        }
    } else {
        return Err(tauri::Error::Anyhow(anyhow::anyhow!("Unknown model spec format")));
    };

    // Update in key manager
    key_manager
        .update_key(key_id, |cfg| {
            cfg.model_selection = Some(new_selection.clone());
        })
        .map_err(|e| tauri::Error::Anyhow(e.into()))?;

    // Update in config
    config_manager
        .update(|cfg| {
            if let Some(k) = cfg.api_keys.iter_mut().find(|k| k.id == key_id) {
                k.model_selection = Some(new_selection);
            }
        })
        .map_err(|e| tauri::Error::Anyhow(e.into()))?;

    // Save to disk
    config_manager
        .save()
        .await
        .map_err(|e| tauri::Error::Anyhow(e.into()))?;

    // Rebuild tray menu to show updated checkmarks
    rebuild_tray_menu(app)?;

    info!("Model selection updated for key {}", key_id);

    Ok(())
}

/// Handle forcing a specific model for an API key
async fn handle_force_model<R: Runtime>(
    app: &AppHandle<R>,
    key_id: &str,
    provider: &str,
    model: &str,
) -> tauri::Result<()> {
    use crate::config::ModelRoutingConfig;

    let key_manager = app.state::<ApiKeyManager>();
    let config_manager = app.state::<ConfigManager>();

    info!(
        "Setting force model for key {}: provider={}, model={}",
        key_id, provider, model
    );

    // Get or create routing config
    let current_key = key_manager
        .get_key(key_id)
        .ok_or_else(|| tauri::Error::Anyhow(anyhow::anyhow!("API key not found")))?;

    let mut routing_config = current_key
        .get_routing_config()
        .unwrap_or_else(|| ModelRoutingConfig::new_force_model(provider.to_string(), model.to_string()));

    // Update to Force Model strategy
    routing_config.active_strategy = ActiveRoutingStrategy::ForceModel;
    routing_config.forced_model = Some((provider.to_string(), model.to_string()));

    // Update in key manager
    key_manager
        .update_key(key_id, |cfg| {
            cfg.routing_config = Some(routing_config.clone());
        })
        .map_err(|e| tauri::Error::Anyhow(e.into()))?;

    // Save to config
    config_manager
        .update(|cfg| {
            if let Some(k) = cfg.api_keys.iter_mut().find(|k| k.id == key_id) {
                k.routing_config = Some(routing_config);
            }
        })
        .map_err(|e| tauri::Error::Anyhow(e.into()))?;

    config_manager
        .save()
        .await
        .map_err(|e| tauri::Error::Anyhow(e.into()))?;

    // Rebuild tray menu
    rebuild_tray_menu(app)?;

    info!("Force model set successfully for key {}", key_id);

    Ok(())
}

/// Handle enabling Available Models strategy for an API key
async fn handle_enable_available_models<R: Runtime>(
    app: &AppHandle<R>,
    key_id: &str,
) -> tauri::Result<()> {
    use crate::config::ModelRoutingConfig;

    let key_manager = app.state::<ApiKeyManager>();
    let config_manager = app.state::<ConfigManager>();

    info!("Enabling available models strategy for key {}", key_id);

    // Get or create routing config
    let current_key = key_manager
        .get_key(key_id)
        .ok_or_else(|| tauri::Error::Anyhow(anyhow::anyhow!("API key not found")))?;

    let mut routing_config = current_key
        .get_routing_config()
        .unwrap_or_else(|| ModelRoutingConfig::new_available_models());

    // Update to Available Models strategy
    routing_config.active_strategy = ActiveRoutingStrategy::AvailableModels;

    // Update in key manager
    key_manager
        .update_key(key_id, |cfg| {
            cfg.routing_config = Some(routing_config.clone());
        })
        .map_err(|e| tauri::Error::Anyhow(e.into()))?;

    // Save to config
    config_manager
        .update(|cfg| {
            if let Some(k) = cfg.api_keys.iter_mut().find(|k| k.id == key_id) {
                k.routing_config = Some(routing_config);
            }
        })
        .map_err(|e| tauri::Error::Anyhow(e.into()))?;

    config_manager
        .save()
        .await
        .map_err(|e| tauri::Error::Anyhow(e.into()))?;

    // Rebuild tray menu
    rebuild_tray_menu(app)?;

    info!("Available models strategy enabled for key {}", key_id);

    Ok(())
}

/// Handle toggling a provider in the available models list
async fn handle_toggle_provider<R: Runtime>(
    app: &AppHandle<R>,
    key_id: &str,
    provider: &str,
) -> tauri::Result<()> {
    use crate::config::ModelRoutingConfig;

    let key_manager = app.state::<ApiKeyManager>();
    let config_manager = app.state::<ConfigManager>();

    info!("Toggling provider {} for key {}", provider, key_id);

    // Get or create routing config
    let current_key = key_manager
        .get_key(key_id)
        .ok_or_else(|| tauri::Error::Anyhow(anyhow::anyhow!("API key not found")))?;

    let mut routing_config = current_key
        .get_routing_config()
        .unwrap_or_else(|| ModelRoutingConfig::new_available_models());

    // Toggle provider in the available models list
    if let Some(pos) = routing_config
        .available_models
        .all_provider_models
        .iter()
        .position(|p| p == provider)
    {
        routing_config.available_models.all_provider_models.remove(pos);
    } else {
        routing_config
            .available_models
            .all_provider_models
            .push(provider.to_string());
    }

    // Ensure we're using Available Models strategy
    routing_config.active_strategy = ActiveRoutingStrategy::AvailableModels;

    // Update in key manager
    key_manager
        .update_key(key_id, |cfg| {
            cfg.routing_config = Some(routing_config.clone());
        })
        .map_err(|e| tauri::Error::Anyhow(e.into()))?;

    // Save to config
    config_manager
        .update(|cfg| {
            if let Some(k) = cfg.api_keys.iter_mut().find(|k| k.id == key_id) {
                k.routing_config = Some(routing_config);
            }
        })
        .map_err(|e| tauri::Error::Anyhow(e.into()))?;

    config_manager
        .save()
        .await
        .map_err(|e| tauri::Error::Anyhow(e.into()))?;

    // Rebuild tray menu
    rebuild_tray_menu(app)?;

    info!("Provider {} toggled for key {}", provider, key_id);

    Ok(())
}

/// Handle toggling an individual model in the available models list
async fn handle_toggle_available_model<R: Runtime>(
    app: &AppHandle<R>,
    key_id: &str,
    provider: &str,
    model: &str,
) -> tauri::Result<()> {
    use crate::config::ModelRoutingConfig;

    let key_manager = app.state::<ApiKeyManager>();
    let config_manager = app.state::<ConfigManager>();

    info!(
        "Toggling model {}/{} for key {}",
        provider, model, key_id
    );

    // Get or create routing config
    let current_key = key_manager
        .get_key(key_id)
        .ok_or_else(|| tauri::Error::Anyhow(anyhow::anyhow!("API key not found")))?;

    let mut routing_config = current_key
        .get_routing_config()
        .unwrap_or_else(|| ModelRoutingConfig::new_available_models());

    // Toggle model in the available models list
    let model_tuple = (provider.to_string(), model.to_string());
    if let Some(pos) = routing_config
        .available_models
        .individual_models
        .iter()
        .position(|m| m == &model_tuple)
    {
        routing_config.available_models.individual_models.remove(pos);
    } else {
        routing_config
            .available_models
            .individual_models
            .push(model_tuple);
    }

    // Ensure we're using Available Models strategy
    routing_config.active_strategy = ActiveRoutingStrategy::AvailableModels;

    // Update in key manager
    key_manager
        .update_key(key_id, |cfg| {
            cfg.routing_config = Some(routing_config.clone());
        })
        .map_err(|e| tauri::Error::Anyhow(e.into()))?;

    // Save to config
    config_manager
        .update(|cfg| {
            if let Some(k) = cfg.api_keys.iter_mut().find(|k| k.id == key_id) {
                k.routing_config = Some(routing_config);
            }
        })
        .map_err(|e| tauri::Error::Anyhow(e.into()))?;

    config_manager
        .save()
        .await
        .map_err(|e| tauri::Error::Anyhow(e.into()))?;

    // Rebuild tray menu
    rebuild_tray_menu(app)?;

    info!("Model {}/{} toggled for key {}", provider, model, key_id);

    Ok(())
}

/// Handle opening the prioritized list modal for an API key
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

/// Update the tray icon based on server status
pub fn update_tray_icon<R: Runtime>(app: &AppHandle<R>, status: &str) -> tauri::Result<()> {
    if let Some(tray) = app.tray_by_id("main") {
        match status {
            "stopped" => {
                // Stopped: Use default icon in template mode (monochrome/dimmed)
                if let Some(icon) = app.default_window_icon() {
                    tray.set_icon(Some(icon.clone()))?;
                }
                tray.set_icon_as_template(true)?;
                tray.set_tooltip(Some("LocalRouter AI - Server Stopped"))?;
                info!("Tray icon updated: stopped (template mode)");
            }
            "running" => {
                // Running: Use default icon in template mode (monochrome)
                if let Some(icon) = app.default_window_icon() {
                    tray.set_icon(Some(icon.clone()))?;
                }
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
