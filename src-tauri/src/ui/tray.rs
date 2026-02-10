//! System tray management
//!
//! Handles system tray icon and menu.

#![allow(dead_code)]

use crate::ui::tray_menu::{
    build_tray_menu, copy_to_clipboard, handle_add_mcp_to_client, handle_copy_mcp_bearer,
    handle_copy_mcp_url, handle_copy_url, handle_create_and_copy_api_key, handle_prioritized_list,
    handle_set_client_strategy, handle_toggle_client_enabled, handle_toggle_mcp_access,
    handle_toggle_skill_access,
};
use lr_utils::test_mode::is_test_mode;
use parking_lot::RwLock;
use std::sync::Arc;
use tauri::{tray::TrayIconBuilder, App, AppHandle, Emitter, Listener, Manager, Runtime};
use tracing::{debug, error, info};

pub use crate::ui::tray_graph_manager::TrayGraphManager;

/// State to track if an update notification should be shown in the tray
pub struct UpdateNotificationState {
    pub update_available: Arc<RwLock<bool>>,
}

impl Default for UpdateNotificationState {
    fn default() -> Self {
        Self {
            update_available: Arc::new(RwLock::new(false)),
        }
    }
}

impl UpdateNotificationState {
    pub fn new() -> Self {
        Self::default()
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
        "🧪 LocalRouter [TEST MODE]"
    } else {
        "LocalRouter"
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
                    info!("Open dashboard requested from tray");
                    if let Some(window) = app.get_webview_window("main") {
                        info!("Found existing main window, showing it");
                        let _ = window.show();
                        let _ = window.set_focus();
                    } else {
                        info!("Main window not found, creating new window");
                        // Create the window if it doesn't exist
                        use tauri::WebviewWindowBuilder;
                        match WebviewWindowBuilder::new(
                            app,
                            "main",
                            tauri::WebviewUrl::App("index.html".into()),
                        )
                        .title("LocalRouter")
                        .inner_size(1200.0, 1000.0)
                        .center()
                        .visible(true)
                        .build()
                        {
                            Ok(window) => {
                                info!("Created new main window");
                                let _ = window.set_focus();
                            }
                            Err(e) => {
                                error!("Failed to create main window: {}", e);
                            }
                        }
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
                // "toggle_tray_graph" removed - dynamic graph is always enabled
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
                "update_and_restart" => {
                    info!("Update and restart requested from tray");
                    // Emit event to frontend to trigger immediate update
                    if let Err(e) = app.emit("update-and-restart", ()) {
                        error!("Failed to emit update-and-restart event: {}", e);
                    }
                }
                "quit" => {
                    info!("Quit requested from tray");
                    app.exit(0);
                }
                _ => {
                    // Handle firewall deny: firewall_deny_<request_id>
                    if let Some(request_id) = id.strip_prefix("firewall_deny_") {
                        info!("Firewall deny requested from tray for {}", request_id);
                        handle_firewall_action_from_tray(
                            app,
                            request_id,
                            lr_mcp::gateway::firewall::FirewallApprovalAction::Deny,
                        );
                    }
                    // Handle firewall allow once: firewall_allow_once_<request_id>
                    else if let Some(request_id) = id.strip_prefix("firewall_allow_once_") {
                        info!("Firewall allow once requested from tray for {}", request_id);
                        handle_firewall_action_from_tray(
                            app,
                            request_id,
                            lr_mcp::gateway::firewall::FirewallApprovalAction::AllowOnce,
                        );
                    }
                    // Handle firewall allow session: firewall_allow_session_<request_id>
                    else if let Some(request_id) = id.strip_prefix("firewall_allow_session_") {
                        info!(
                            "Firewall allow session requested from tray for {}",
                            request_id
                        );
                        handle_firewall_action_from_tray(
                            app,
                            request_id,
                            lr_mcp::gateway::firewall::FirewallApprovalAction::AllowSession,
                        );
                    }
                    // Handle firewall open popup: firewall_open_<request_id>
                    else if let Some(request_id) = id.strip_prefix("firewall_open_") {
                        info!("Firewall popup requested from tray for {}", request_id);
                        let app_clone = app.clone();
                        let request_id = request_id.to_string();
                        // Try to focus existing popup window, or create a new one
                        let window_label = format!("firewall-approval-{}", request_id);
                        if let Some(window) = app_clone.get_webview_window(&window_label) {
                            let _ = window.set_focus();
                        } else {
                            match tauri::WebviewWindowBuilder::new(
                                &app_clone,
                                &window_label,
                                tauri::WebviewUrl::App("index.html".into()),
                            )
                            .title("Approval Required")
                            .inner_size(400.0, 320.0)
                            .center()
                            .visible(true)
                            .resizable(false)
                            .decorations(true)
                            .always_on_top(true)
                            .build()
                            {
                                Ok(window) => {
                                    let _ = window.set_focus();
                                }
                                Err(e) => {
                                    error!("Failed to create firewall popup from tray: {}", e);
                                }
                            }
                        }
                    }
                    // Handle copy MCP URL: copy_mcp_url_<client_id>_<server_id>
                    else if let Some(rest) = id.strip_prefix("copy_mcp_url_") {
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
                    // Handle health issue provider click: health_issue_provider_{provider_name}
                    else if let Some(provider_name) = id.strip_prefix("health_issue_provider_") {
                        info!("Health issue clicked for provider: {}", provider_name);
                        // Show main window and navigate to LLM Resources tab
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                        // Emit event to navigate to Resources page
                        if let Err(e) = app.emit("open-resources-tab", ()) {
                            error!("Failed to emit open-resources-tab event: {}", e);
                        }
                    }
                    // Handle health issue MCP click: health_issue_mcp_{server_id}
                    else if let Some(server_id) = id.strip_prefix("health_issue_mcp_") {
                        info!("Health issue clicked for MCP server: {}", server_id);
                        // Show main window and navigate to MCP Servers tab
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                        // Emit event to navigate to MCP Servers page with the specific server
                        if let Err(e) = app.emit("open-mcp-server", server_id) {
                            error!("Failed to emit open-mcp-server event: {}", e);
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
                    // Handle set strategy: set_strategy_<client_id>_<strategy_id>
                    else if let Some(rest) = id.strip_prefix("set_strategy_") {
                        if let Some((client_id, strategy_id)) = rest.split_once('_') {
                            info!(
                                "Set strategy requested: client={}, strategy={}",
                                client_id, strategy_id
                            );
                            let app_clone = app.clone();
                            let client_id = client_id.to_string();
                            let strategy_id = strategy_id.to_string();
                            tauri::async_runtime::spawn(async move {
                                if let Err(e) =
                                    handle_set_client_strategy(&app_clone, &client_id, &strategy_id)
                                        .await
                                {
                                    error!("Failed to set client strategy: {}", e);
                                }
                            });
                        }
                    }
                    // Handle toggle MCP: toggle_mcp_<client_id>_<server_id>
                    else if let Some(rest) = id.strip_prefix("toggle_mcp_") {
                        if let Some((client_id, server_id)) = rest.split_once('_') {
                            info!(
                                "Toggle MCP requested: client={}, server={}",
                                client_id, server_id
                            );
                            let app_clone = app.clone();
                            let client_id = client_id.to_string();
                            let server_id = server_id.to_string();
                            tauri::async_runtime::spawn(async move {
                                if let Err(e) =
                                    handle_toggle_mcp_access(&app_clone, &client_id, &server_id)
                                        .await
                                {
                                    error!("Failed to toggle MCP access: {}", e);
                                }
                            });
                        }
                    }
                    // Handle toggle skill: toggle_skill_<client_id>_<skill_name>
                    else if let Some(rest) = id.strip_prefix("toggle_skill_") {
                        if let Some((client_id, skill_name)) = rest.split_once('_') {
                            info!(
                                "Toggle skill requested: client={}, skill={}",
                                client_id, skill_name
                            );
                            let app_clone = app.clone();
                            let client_id = client_id.to_string();
                            let skill_name = skill_name.to_string();
                            tauri::async_runtime::spawn(async move {
                                if let Err(e) =
                                    handle_toggle_skill_access(&app_clone, &client_id, &skill_name)
                                        .await
                                {
                                    error!("Failed to toggle skill access: {}", e);
                                }
                            });
                        }
                    }
                    // Handle toggle client enabled: toggle_client_enabled_<client_id>
                    else if let Some(client_id) = id.strip_prefix("toggle_client_enabled_") {
                        info!("Toggle client enabled requested: {}", client_id);
                        let app_clone = app.clone();
                        let client_id = client_id.to_string();
                        tauri::async_runtime::spawn(async move {
                            if let Err(e) =
                                handle_toggle_client_enabled(&app_clone, &client_id).await
                            {
                                error!("Failed to toggle client enabled: {}", e);
                            }
                        });
                    }
                    // Handle copy client ID: copy_client_id_<client_id>
                    else if let Some(client_id) = id.strip_prefix("copy_client_id_") {
                        info!("Copy client ID requested: {}", client_id);
                        if let Err(e) = copy_to_clipboard(client_id) {
                            error!("Failed to copy client ID to clipboard: {}", e);
                        }
                    }
                    // Handle copy client secret: copy_client_secret_<client_id>
                    else if let Some(client_id) = id.strip_prefix("copy_client_secret_") {
                        info!("Copy client secret requested: {}", client_id);
                        let app_clone = app.clone();
                        let client_id = client_id.to_string();
                        tauri::async_runtime::spawn(async move {
                            if let Err(e) = handle_copy_mcp_bearer(&app_clone, &client_id).await {
                                error!("Failed to copy client secret: {}", e);
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

    // Subscribe to health status changes to rebuild the tray menu
    // This ensures health issues appear in the menu when status changes
    let app_handle = app.handle().clone();
    app.listen("health-status-changed", move |_event| {
        debug!("Health status changed, rebuilding tray menu");
        if let Err(e) = rebuild_tray_menu(&app_handle) {
            error!("Failed to rebuild tray menu on health change: {}", e);
        }
    });

    info!("System tray initialized successfully");
    Ok(())
}

/// Rebuild the system tray menu with updated API keys
pub fn rebuild_tray_menu<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    debug!("Rebuilding system tray menu");

    let menu = build_tray_menu(app)?;

    if let Some(tray) = app.tray_by_id("main") {
        tray.set_menu(Some(menu))?;
        debug!("System tray menu updated");
    }

    Ok(())
}

/// Update the tray icon based on server status
///
/// Note: When the dynamic tray graph is enabled (always now), "running" status
/// only updates the tooltip - the graph icon is managed by TrayGraphManager.
pub fn update_tray_icon<R: Runtime>(app: &AppHandle<R>, status: &str) -> tauri::Result<()> {
    // Embed the tray icons at compile time
    const TRAY_ICON: &[u8] = include_bytes!("../../icons/32x32.png");

    if let Some(tray) = app.tray_by_id("main") {
        match status {
            "stopped" => {
                // Stopped: Use static template icon (dynamic graph shows red dot but server stopped)
                let icon = tauri::image::Image::from_bytes(TRAY_ICON).map_err(|e| {
                    tauri::Error::Anyhow(anyhow::anyhow!("Failed to load tray icon: {}", e))
                })?;
                tray.set_icon(Some(icon))?;
                tray.set_icon_as_template(true)?;
                tray.set_tooltip(Some("LocalRouter - Server Stopped"))?;
                info!("Tray icon updated: stopped (template mode)");
            }
            "running" => {
                // Running: Only update tooltip, don't change icon (graph manager handles it)
                // The dynamic graph with health dot is always displayed now
                tray.set_tooltip(Some("LocalRouter - Server Running"))?;
                info!("Tray tooltip updated: running (graph managed by TrayGraphManager)");
            }
            _ => {
                // Unknown status - just update tooltip
                info!(
                    "Unknown tray icon status: {}, updating tooltip only",
                    status
                );
                tray.set_tooltip(Some(&format!("LocalRouter - {}", status)))?;
            }
        }
    }

    Ok(())
}

// handle_toggle_tray_graph removed - dynamic tray graph is always enabled

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

/// Handle a firewall approval action from the tray menu
fn handle_firewall_action_from_tray<R: Runtime>(
    app: &AppHandle<R>,
    request_id: &str,
    action: lr_mcp::gateway::firewall::FirewallApprovalAction,
) {
    let app_clone = app.clone();
    let request_id = request_id.to_string();

    tauri::async_runtime::spawn(async move {
        // Submit the firewall response
        if let Some(app_state) = app_clone.try_state::<Arc<lr_server::state::AppState>>() {
            let action_debug = format!("{:?}", action);
            if let Err(e) = app_state
                .mcp_gateway
                .firewall_manager
                .submit_response(&request_id, action, None)
            {
                error!("Failed to submit firewall response from tray: {}", e);
                return;
            }
            info!(
                "Firewall response submitted from tray for {}: {}",
                request_id, action_debug
            );
        }

        // Close any open popup window for this request
        if let Some(window) =
            app_clone.get_webview_window(&format!("firewall-approval-{}", request_id))
        {
            let _ = window.close();
        }

        // Rebuild tray menu to remove the pending approval item
        if let Err(e) = rebuild_tray_menu(&app_clone) {
            error!(
                "Failed to rebuild tray menu after firewall action from tray: {}",
                e
            );
        }

        // Trigger immediate tray icon update to remove the question mark overlay
        if let Some(tray_graph_manager) =
            app_clone.try_state::<Arc<crate::ui::tray_graph_manager::TrayGraphManager>>()
        {
            tray_graph_manager.notify_activity();
        }
    });
}
