//! Tray menu building and event handlers
//!
//! ## WEBSITE DEMO SYNC REQUIRED
//!
//! The tray menu structure is replicated in the website demo at:
//!   `website/src/components/demo/MacOSTrayMenu.tsx`
//!
//! When modifying the menu structure, labels, or icons, please also
//! update the website demo component to match.
//!
//! Key sync points:
//! - TRAY_INDENT and ICON_PAD constants
//! - Menu item order and labels
//! - Submenu structure for clients
//! - Header text format ("LocalRouter on {host}:{port}")

#![allow(dead_code)]

use crate::ui::tray::{rebuild_tray_menu, UpdateNotificationState};
use lr_clients::ClientManager;
use lr_config::{ClientMode, ConfigManager};
use lr_mcp::manager::McpServerManager;
use lr_providers::health_cache::{AggregateHealthStatus, ItemHealthStatus};
use std::sync::Arc;
use tauri::{
    menu::{MenuBuilder, MenuItem, SubmenuBuilder},
    AppHandle, Emitter, Manager, Runtime,
};
use tracing::{debug, error, info};

/// Indent prefix for tray menu items without an icon.
/// Aligns text with items that have a leading emoji/icon character.
/// Uses an em-space (\u{2003}) plus two thin spaces (\u{2009}).
pub(crate) const TRAY_INDENT: &str = "\u{2003}\u{2009}\u{2009}";

/// Maximum number of items to show per section in client tray submenus.
/// When exceeded, a "More…" item is shown that opens the dashboard.
const MAX_TRAY_ITEMS: usize = 10;

/// Padding on each side of narrow (text-style) icons like ⌘ and ⧉
/// so they occupy the same visual width as full-width emoji icons (❕, ＋).
/// Applied before and after the icon character.
/// Uses two thin spaces (\u{2009}) per side.
pub(crate) const ICON_PAD: &str = "\u{2009}\u{2009}";

/// Format a rate limit for display in the tray menu.
/// Examples: "100 requests / hr", "$5.00 / day", "10,000 tokens / min"
fn format_rate_limit(limit: &lr_config::StrategyRateLimit) -> String {
    let value_str = match limit.limit_type {
        lr_config::RateLimitType::Cost => format!("${:.2}", limit.value),
        _ => {
            if limit.value >= 1000.0 || limit.value == limit.value.floor() {
                format!("{:.0}", limit.value)
            } else {
                format!("{:.1}", limit.value)
            }
        }
    };

    let type_str = match limit.limit_type {
        lr_config::RateLimitType::Requests => "requests",
        lr_config::RateLimitType::InputTokens => "input tokens",
        lr_config::RateLimitType::OutputTokens => "output tokens",
        lr_config::RateLimitType::TotalTokens => "tokens",
        lr_config::RateLimitType::Cost => "",
    };

    let window_str = match limit.time_window {
        lr_config::RateLimitTimeWindow::Minute => "min",
        lr_config::RateLimitTimeWindow::Hour => "hr",
        lr_config::RateLimitTimeWindow::Day => "day",
    };

    if type_str.is_empty() {
        format!("{} / {}", value_str, window_str)
    } else {
        format!("{} {} / {}", value_str, type_str, window_str)
    }
}

/// Build the system tray menu
pub(crate) fn build_tray_menu<R: Runtime, M: Manager<R>>(
    app: &M,
) -> tauri::Result<tauri::menu::Menu<R>> {
    let mut menu_builder = MenuBuilder::new(app);

    // Get server status and config early for header
    let (host, port, is_server_running) =
        if let Some(config_manager) = app.try_state::<ConfigManager>() {
            let config = config_manager.get();
            let running =
                if let Some(server_manager) = app.try_state::<Arc<lr_server::ServerManager>>() {
                    matches!(
                        server_manager.get_status(),
                        lr_server::ServerStatus::Running
                    )
                } else {
                    false
                };
            (config.server.host.clone(), config.server.port, running)
        } else {
            ("127.0.0.1".to_string(), 3625, false)
        };

    // 1. LocalRouter header (shows IP:port when server is running)
    let header_text = if is_server_running {
        format!("LocalRouter on {}:{}", host, port)
    } else {
        "LocalRouter".to_string()
    };
    let app_header = MenuItem::with_id(app, "app_header", &header_text, false, None::<&str>)?;
    menu_builder = menu_builder.item(&app_header);

    // 2. Open dashboard (immediately after header)
    menu_builder = menu_builder.text("open_dashboard", format!("{ICON_PAD}⌘{ICON_PAD} Open..."));

    // 3. Copy URL (LLM and MCP)
    menu_builder = menu_builder.text("copy_url", format!("{ICON_PAD}⧉{ICON_PAD} Copy URL"));

    // Add separator before clients
    menu_builder = menu_builder.separator();

    // 6. Clients section
    let clients_header = MenuItem::with_id(app, "clients_header", "Clients", false, None::<&str>)?;
    menu_builder = menu_builder.item(&clients_header);

    // Get client manager and build client list
    if let Some(client_manager) = app.try_state::<Arc<ClientManager>>() {
        let clients: Vec<_> = client_manager
            .list_clients()
            .into_iter()
            .filter(|c| !c.name.starts_with("_test_strategy_"))
            .collect();
        let mcp_server_manager = app.try_state::<Arc<McpServerManager>>();
        let config_manager = app.try_state::<ConfigManager>();

        // Get all strategies and context management config for quick toggles
        let (all_strategies, global_ctx_config) = config_manager
            .map(|cm| {
                let cfg = cm.get();
                (cfg.strategies.clone(), cfg.context_management.clone())
            })
            .unwrap_or_default();

        if !clients.is_empty() {
            for client in clients.iter() {
                let client_name = if client.name.is_empty() {
                    format!("Client {}", &client.id[..8])
                } else {
                    client.name.clone()
                };

                let mut client_submenu =
                    SubmenuBuilder::new(app, format!("{}{}", TRAY_INDENT, client_name));

                // === Client identity section ===
                let client_name_header = MenuItem::with_id(
                    app,
                    format!("client_name_header_{}", client.id),
                    &client_name,
                    false,
                    None::<&str>,
                )?;
                client_submenu = client_submenu.item(&client_name_header);

                // Enable/disable toggle
                let toggle_label = if client.enabled {
                    format!("{ICON_PAD}●{ICON_PAD} Enabled")
                } else {
                    format!("{ICON_PAD}○{ICON_PAD} Disabled")
                };
                client_submenu = client_submenu
                    .text(format!("toggle_client_enabled_{}", client.id), toggle_label);

                client_submenu = client_submenu.text(
                    format!("copy_client_id_{}", client.id),
                    format!("{ICON_PAD}⧉{ICON_PAD} Copy Client ID (OAuth)"),
                );
                client_submenu = client_submenu.text(
                    format!("copy_client_secret_{}", client.id),
                    format!("{ICON_PAD}⧉{ICON_PAD} Copy API Key / Client Secret (Bearer, OAuth)"),
                );

                client_submenu = client_submenu.text(
                    format!("open_client_settings_{}", client.id),
                    format!("{ICON_PAD}⚙{ICON_PAD} Settings"),
                );

                client_submenu = client_submenu.separator();

                let show_llm = !matches!(client.client_mode, ClientMode::McpOnly);
                let show_mcp = !matches!(client.client_mode, ClientMode::LlmOnly);

                // === Quick toggles section (hidden for McpOnly) ===
                if show_llm {
                    let models_header = MenuItem::with_id(
                        app,
                        format!("models_header_{}", client.id),
                        "Models",
                        false,
                        None::<&str>,
                    )?;
                    client_submenu = client_submenu.item(&models_header);

                    if let Some(strategy) =
                        all_strategies.iter().find(|s| s.id == client.strategy_id)
                    {
                        // Rate Limits
                        for (idx, limit) in strategy.rate_limits.iter().enumerate() {
                            let label = if limit.enabled {
                                format!("✓  Rate Limit: {}", format_rate_limit(limit))
                            } else {
                                format!("{}Rate Limit: {}", TRAY_INDENT, format_rate_limit(limit))
                            };
                            client_submenu = client_submenu
                                .text(format!("toggle_rate_limit_{}__{}", client.id, idx), label);
                        }

                        // Free Tier Mode toggle
                        {
                            let label = if strategy.free_tier_only {
                                "✓  Free Tier Mode".to_string()
                            } else {
                                format!("{}Free Tier Mode", TRAY_INDENT)
                            };
                            client_submenu = client_submenu
                                .text(format!("toggle_free_tier_{}", client.id), label);
                        }

                        // Weak Model Routing toggle (only if auto_config enabled + routellm has weak_models)
                        if let Some(ref auto_config) = strategy.auto_config {
                            if auto_config.permission.is_enabled() {
                                if let Some(ref routellm) = auto_config.routellm_config {
                                    if !routellm.weak_models.is_empty() {
                                        let label = if routellm.enabled {
                                            "✓  Weak Model Routing".to_string()
                                        } else {
                                            format!("{}Weak Model Routing", TRAY_INDENT)
                                        };
                                        client_submenu = client_submenu.text(
                                            format!("toggle_weak_model_{}", client.id),
                                            label,
                                        );
                                    }
                                }
                            }
                        }

                        client_submenu = client_submenu.separator();
                    }
                }

                // === MCP Allowlist section (hidden for LlmOnly) ===
                if show_mcp {
                    let mcp_header = MenuItem::with_id(
                        app,
                        format!("mcp_header_{}", client.id),
                        "MCP Allowlist",
                        false,
                        None::<&str>,
                    )?;
                    client_submenu = client_submenu.item(&mcp_header);

                    if let Some(ref mcp_manager) = mcp_server_manager {
                        let all_mcp_servers = mcp_manager.list_configs();

                        if all_mcp_servers.is_empty() {
                            let no_mcp_label = format!("{}No MCPs configured", TRAY_INDENT);
                            let no_mcp_item = MenuItem::with_id(
                                app,
                                format!("no_mcp_{}", client.id),
                                &no_mcp_label,
                                false,
                                None::<&str>,
                            )?;
                            client_submenu = client_submenu.item(&no_mcp_item);
                        } else {
                            for server in all_mcp_servers.iter().take(MAX_TRAY_ITEMS) {
                                let server_name = if server.name.is_empty() {
                                    format!("MCP {}", &server.id[..server.id.len().min(8)])
                                } else {
                                    server.name.clone()
                                };

                                let is_allowed = client.mcp_server_access.can_access(&server.id);
                                let label = if is_allowed {
                                    format!("✓  {}", server_name)
                                } else {
                                    format!("{}{}", TRAY_INDENT, server_name)
                                };

                                client_submenu = client_submenu
                                    .text(format!("toggle_mcp_{}_{}", client.id, server.id), label);
                            }

                            // Show "More…" if truncated
                            if all_mcp_servers.len() > MAX_TRAY_ITEMS {
                                client_submenu = client_submenu
                                    .text("open_mcp_servers_page", format!("{}More…", TRAY_INDENT));
                            }
                        }
                    } else {
                        let no_mcp_label = format!("{}No MCPs configured", TRAY_INDENT);
                        let no_mcp_item = MenuItem::with_id(
                            app,
                            format!("no_mcp_{}", client.id),
                            &no_mcp_label,
                            false,
                            None::<&str>,
                        )?;
                        client_submenu = client_submenu.item(&no_mcp_item);
                    }

                    // === Skills Allowlist section (also hidden for LlmOnly) ===
                    client_submenu = client_submenu.separator();

                    let skills_header = MenuItem::with_id(
                        app,
                        format!("skills_header_{}", client.id),
                        "Skills Allowlist",
                        false,
                        None::<&str>,
                    )?;
                    client_submenu = client_submenu.item(&skills_header);

                    if let Some(skill_manager) = app.try_state::<Arc<lr_skills::SkillManager>>() {
                        let all_skills = skill_manager.list();

                        if all_skills.is_empty() {
                            let no_skills_label = format!("{}No skills discovered", TRAY_INDENT);
                            let no_skills_item = MenuItem::with_id(
                                app,
                                format!("no_skills_{}", client.id),
                                &no_skills_label,
                                false,
                                None::<&str>,
                            )?;
                            client_submenu = client_submenu.item(&no_skills_item);
                        } else {
                            for skill_info in all_skills.iter().take(MAX_TRAY_ITEMS) {
                                let is_allowed = skill_info.enabled
                                    && client.skills_access.can_access_by_name(&skill_info.name);
                                let label = if !skill_info.enabled {
                                    format!("{}{} (disabled)", TRAY_INDENT, skill_info.name)
                                } else if is_allowed {
                                    format!("✓  {}", skill_info.name)
                                } else {
                                    format!("{}{}", TRAY_INDENT, skill_info.name)
                                };

                                client_submenu = client_submenu.text(
                                    format!("toggle_skill_{}_{}", client.id, skill_info.name),
                                    label,
                                );
                            }

                            // Show "More…" if truncated
                            if all_skills.len() > MAX_TRAY_ITEMS {
                                client_submenu = client_submenu
                                    .text("open_skills_page", format!("{}More…", TRAY_INDENT));
                            }
                        }
                    } else {
                        let no_skills_label = format!("{}No skills discovered", TRAY_INDENT);
                        let no_skills_item = MenuItem::with_id(
                            app,
                            format!("no_skills_{}", client.id),
                            &no_skills_label,
                            false,
                            None::<&str>,
                        )?;
                        client_submenu = client_submenu.item(&no_skills_item);
                    }

                    // === Coding Agent section (also hidden for LlmOnly) ===
                    client_submenu = client_submenu.separator();

                    let coding_agents_header = MenuItem::with_id(
                        app,
                        format!("coding_agents_header_{}", client.id),
                        "Coding Agent",
                        false,
                        None::<&str>,
                    )?;
                    client_submenu = client_submenu.item(&coding_agents_header);

                    {
                        let is_enabled = client.coding_agent_permission.is_enabled();
                        let agent_label = if let Some(at) = &client.coding_agent_type {
                            if is_enabled {
                                format!("✓  {}", at.display_name())
                            } else {
                                format!("{}{}  (off)", TRAY_INDENT, at.display_name())
                            }
                        } else if is_enabled {
                            format!("{}Enabled (no agent selected)", TRAY_INDENT)
                        } else {
                            format!("{}Off", TRAY_INDENT)
                        };

                        client_submenu = client_submenu
                            .text(format!("toggle_coding_agent_{}", client.id), agent_label);
                    }

                    // Catalog Compression toggle (per-client override)
                    {
                        let is_inherited = client.catalog_compression_enabled.is_none();
                        let effective = client.is_catalog_compression_enabled(&global_ctx_config);
                        let label = if effective {
                            if is_inherited {
                                "✓  Catalog Compression (default)".to_string()
                            } else {
                                "✓  Catalog Compression".to_string()
                            }
                        } else if is_inherited {
                            format!("{}Catalog Compression (default)", TRAY_INDENT)
                        } else {
                            format!("{}Catalog Compression", TRAY_INDENT)
                        };
                        client_submenu = client_submenu
                            .text(format!("toggle_catalog_compression_{}", client.id), label);
                    }

                    client_submenu = client_submenu.separator();
                }

                let client_menu = client_submenu.build()?;
                client_menu.set_enabled(true)?;
                menu_builder = menu_builder.item(&client_menu);
            }
        }
    }

    // Add "Quick Create & Copy API Key" button (creates with all models, no MCP)
    menu_builder = menu_builder.text("create_and_copy_api_key", "＋ Add && Copy Key");

    // Notifications section (dynamic: health issues, updates, firewall approvals)
    // Only shown with a separator when there are notifications to display.
    {
        let mut has_notifications = false;

        // Health issues (only when aggregate status is Yellow or Red)
        if let Some(app_state) = app.try_state::<Arc<lr_server::state::AppState>>() {
            let health_state = app_state.health_cache.get();
            debug!(
                "Tray menu: aggregate_status={:?}, providers={}, mcp_servers={}",
                health_state.aggregate_status,
                health_state.providers.len(),
                health_state.mcp_servers.len()
            );

            if matches!(
                health_state.aggregate_status,
                AggregateHealthStatus::Yellow | AggregateHealthStatus::Red
            ) {
                for (provider_name, health) in &health_state.providers {
                    if matches!(
                        health.status,
                        ItemHealthStatus::Unhealthy | ItemHealthStatus::Degraded
                    ) {
                        if !has_notifications {
                            menu_builder = menu_builder.separator();
                            has_notifications = true;
                        }
                        let label = format!(
                            "❕ Provider '{}' {}",
                            provider_name,
                            match health.status {
                                ItemHealthStatus::Unhealthy => "unhealthy",
                                ItemHealthStatus::Degraded => "degraded",
                                _ => "",
                            }
                        );
                        menu_builder = menu_builder
                            .text(format!("health_issue_provider_{}", provider_name), label);
                    }
                }

                for (server_id, health) in &health_state.mcp_servers {
                    if matches!(
                        health.status,
                        ItemHealthStatus::Unhealthy | ItemHealthStatus::Degraded
                    ) {
                        if !has_notifications {
                            menu_builder = menu_builder.separator();
                            has_notifications = true;
                        }
                        let display_name = if health.name.is_empty() {
                            format!("MCP {}", &server_id[..server_id.len().min(8)])
                        } else {
                            health.name.clone()
                        };
                        let label = format!(
                            "❕ MCP '{}' {}",
                            display_name,
                            match health.status {
                                ItemHealthStatus::Unhealthy => "unhealthy",
                                ItemHealthStatus::Degraded => "degraded",
                                _ => "",
                            }
                        );
                        menu_builder =
                            menu_builder.text(format!("health_issue_mcp_{}", server_id), label);
                    }
                }
            }
        } else {
            debug!("Tray menu: AppState not available");
        }

        // Update available
        if let Some(update_state) = app.try_state::<Arc<UpdateNotificationState>>() {
            if update_state.is_update_available() {
                if !has_notifications {
                    menu_builder = menu_builder.separator();
                    has_notifications = true;
                }
                menu_builder = menu_builder.text(
                    "update_and_restart",
                    format!("{ICON_PAD}↓{ICON_PAD} Update and restart"),
                );
            }
        }

        // Firewall pending approvals
        if let Some(app_state) = app.try_state::<Arc<lr_server::state::AppState>>() {
            let pending = app_state.mcp_gateway.firewall_manager.list_pending();
            for approval in &pending {
                if !has_notifications {
                    menu_builder = menu_builder.separator();
                    has_notifications = true;
                }
                let name_display = if approval.tool_name.len() > 25 {
                    format!("{}…", &approval.tool_name[..25])
                } else {
                    approval.tool_name.clone()
                };

                let label = if approval.is_guardrail_request {
                    format!(
                        "❔ Guardrail: \"{}\" — {}",
                        name_display, approval.client_name
                    )
                } else if approval.is_free_tier_fallback {
                    format!("❔ Free-Tier Fallback — {}", approval.client_name)
                } else if approval.is_model_request {
                    format!("❔ Model: \"{}\" — {}", name_display, approval.client_name)
                } else if approval.is_auto_router_request {
                    format!("❔ Auto Router — {}", approval.client_name)
                } else {
                    format!("❔ Tool: \"{}\" — {}", name_display, approval.client_name)
                };

                menu_builder =
                    menu_builder.text(format!("firewall_open_{}", approval.request_id), label);
            }
        }

        let _ = has_notifications;
    }

    // Add separator before quit
    menu_builder = menu_builder.separator();

    // Add quit option
    menu_builder = menu_builder.text("quit", format!("{ICON_PAD}⏻{ICON_PAD} Quit LocalRouter"));

    menu_builder.build()
}

/// Handle copying the server URL to clipboard
pub(crate) async fn handle_copy_url<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
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
pub(crate) async fn handle_prioritized_list<R: Runtime>(
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
///
/// Creates a client with:
/// - All models allowed (via strategy)
/// - No MCP access
/// - No prioritized models
pub(crate) async fn handle_create_and_copy_api_key<R: Runtime>(
    app: &AppHandle<R>,
) -> tauri::Result<()> {
    info!("Quick creating new client and copying API key from tray");

    // Get managers from state
    let client_manager = app.state::<Arc<ClientManager>>();
    let config_manager = app.state::<ConfigManager>();

    // Create client with auto-created strategy (strategy defaults to all models allowed)
    let (client, _strategy) = config_manager
        .create_client_with_strategy("App".to_string())
        .map_err(|e| tauri::Error::Anyhow(e.into()))?;

    // Store client secret in keychain and add to client manager
    let secret = client_manager
        .add_client_with_secret(client.clone())
        .map_err(|e| tauri::Error::Anyhow(e.into()))?;

    // Persist to disk
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

    // Emit events for UI updates
    if let Err(e) = app.emit("clients-changed", ()) {
        error!("Failed to emit clients-changed event: {}", e);
    }
    if let Err(e) = app.emit("strategies-changed", ()) {
        error!("Failed to emit strategies-changed event: {}", e);
    }

    info!(
        "Quick client created and API key copied to clipboard: {}",
        client.id
    );

    Ok(())
}

/// Handle copying MCP URL to clipboard
pub(crate) async fn handle_copy_mcp_url<R: Runtime>(
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
pub(crate) async fn handle_copy_mcp_bearer<R: Runtime>(
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
pub(crate) async fn handle_add_mcp_to_client<R: Runtime>(
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

/// Handle opening client settings in the UI
pub(crate) fn handle_open_client_settings<R: Runtime>(
    app: &AppHandle<R>,
    client_id: &str,
) -> tauri::Result<()> {
    info!("Opening settings for client {}", client_id);

    // Show the main window
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
    }

    // Emit event to navigate to the client tab
    if let Err(e) = app.emit("open-client-tab", client_id) {
        error!("Failed to emit open-client-tab event: {}", e);
    }

    Ok(())
}

/// Handle toggling a rate limit's enabled state
pub(crate) async fn handle_toggle_rate_limit<R: Runtime>(
    app: &AppHandle<R>,
    client_id: &str,
    index: usize,
) -> tauri::Result<()> {
    info!("Toggling rate limit {} for client {}", index, client_id);

    let config_manager = app.state::<ConfigManager>();

    config_manager
        .update(|cfg| {
            if let Some(client) = cfg.clients.iter().find(|c| c.id == client_id) {
                if let Some(strategy) = cfg
                    .strategies
                    .iter_mut()
                    .find(|s| s.id == client.strategy_id)
                {
                    if let Some(limit) = strategy.rate_limits.get_mut(index) {
                        limit.enabled = !limit.enabled;
                    }
                }
            }
        })
        .map_err(|e| tauri::Error::Anyhow(e.into()))?;

    config_manager
        .save()
        .await
        .map_err(|e| tauri::Error::Anyhow(e.into()))?;

    rebuild_tray_menu(app)?;

    if let Err(e) = app.emit("strategies-changed", ()) {
        error!("Failed to emit strategies-changed event: {}", e);
    }

    Ok(())
}

/// Handle toggling free tier mode for a client's strategy
pub(crate) async fn handle_toggle_free_tier<R: Runtime>(
    app: &AppHandle<R>,
    client_id: &str,
) -> tauri::Result<()> {
    info!("Toggling free tier mode for client {}", client_id);

    let config_manager = app.state::<ConfigManager>();

    config_manager
        .update(|cfg| {
            if let Some(client) = cfg.clients.iter().find(|c| c.id == client_id) {
                if let Some(strategy) = cfg
                    .strategies
                    .iter_mut()
                    .find(|s| s.id == client.strategy_id)
                {
                    strategy.free_tier_only = !strategy.free_tier_only;
                }
            }
        })
        .map_err(|e| tauri::Error::Anyhow(e.into()))?;

    config_manager
        .save()
        .await
        .map_err(|e| tauri::Error::Anyhow(e.into()))?;

    rebuild_tray_menu(app)?;

    if let Err(e) = app.emit("strategies-changed", ()) {
        error!("Failed to emit strategies-changed event: {}", e);
    }

    Ok(())
}

/// Handle toggling weak model routing for a client's strategy
pub(crate) async fn handle_toggle_weak_model<R: Runtime>(
    app: &AppHandle<R>,
    client_id: &str,
) -> tauri::Result<()> {
    info!("Toggling weak model routing for client {}", client_id);

    let config_manager = app.state::<ConfigManager>();

    config_manager
        .update(|cfg| {
            if let Some(client) = cfg.clients.iter().find(|c| c.id == client_id) {
                if let Some(strategy) = cfg
                    .strategies
                    .iter_mut()
                    .find(|s| s.id == client.strategy_id)
                {
                    if let Some(ref mut auto_config) = strategy.auto_config {
                        if let Some(ref mut routellm) = auto_config.routellm_config {
                            routellm.enabled = !routellm.enabled;
                        }
                    }
                }
            }
        })
        .map_err(|e| tauri::Error::Anyhow(e.into()))?;

    config_manager
        .save()
        .await
        .map_err(|e| tauri::Error::Anyhow(e.into()))?;

    rebuild_tray_menu(app)?;

    if let Err(e) = app.emit("strategies-changed", ()) {
        error!("Failed to emit strategies-changed event: {}", e);
    }

    Ok(())
}

/// Handle toggling catalog compression for a client (per-client override).
/// If currently inherited (None), sets to opposite of default (true).
/// If currently overridden, clears back to inherited (None).
pub(crate) async fn handle_toggle_catalog_compression<R: Runtime>(
    app: &AppHandle<R>,
    client_id: &str,
) -> tauri::Result<()> {
    info!(
        "Toggling catalog compression override for client {}",
        client_id
    );

    let config_manager = app.state::<ConfigManager>();

    config_manager
        .update(|cfg| {
            let global_ctx = cfg.context_management.clone();
            if let Some(client) = cfg.clients.iter_mut().find(|c| c.id == client_id) {
                client.catalog_compression_enabled = match client.catalog_compression_enabled {
                    None => {
                        // Inherited → override to opposite of effective value
                        Some(!client.is_catalog_compression_enabled(&global_ctx))
                    }
                    Some(_) => {
                        // Overridden → clear back to inherited
                        None
                    }
                };
            }
        })
        .map_err(|e| tauri::Error::Anyhow(e.into()))?;

    config_manager
        .save()
        .await
        .map_err(|e| tauri::Error::Anyhow(e.into()))?;

    rebuild_tray_menu(app)?;

    if let Err(e) = app.emit("clients-changed", ()) {
        error!("Failed to emit clients-changed event: {}", e);
    }

    Ok(())
}

/// Handle toggling MCP server access for a client
pub(crate) async fn handle_toggle_mcp_access<R: Runtime>(
    app: &AppHandle<R>,
    client_id: &str,
    server_id: &str,
) -> tauri::Result<()> {
    info!("Toggling MCP {} access for client {}", server_id, client_id);

    // Get managers from state
    let client_manager = app.state::<Arc<ClientManager>>();
    let config_manager = app.state::<ConfigManager>();

    // Check if server is currently allowed and toggle using new permission system
    let is_allowed = client_manager.has_mcp_server_access(client_id, server_id);

    if is_allowed {
        // Remove MCP server permission (set to Off)
        client_manager
            .remove_mcp_server(client_id, server_id)
            .map_err(|e| tauri::Error::Anyhow(e.into()))?;
        info!("MCP {} removed from client {}", server_id, client_id);
    } else {
        // Add MCP server permission (set to Allow)
        client_manager
            .add_mcp_server(client_id, server_id)
            .map_err(|e| tauri::Error::Anyhow(e.into()))?;
        info!("MCP {} added to client {}", server_id, client_id);
    }

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

    // Emit event for UI updates
    if let Err(e) = app.emit("clients-changed", ()) {
        error!("Failed to emit clients-changed event: {}", e);
    }

    Ok(())
}

/// Handle toggling a client's enabled state
pub(crate) async fn handle_toggle_client_enabled<R: Runtime>(
    app: &AppHandle<R>,
    client_id: &str,
) -> tauri::Result<()> {
    info!("Toggling enabled state for client {}", client_id);

    let client_manager = app.state::<Arc<ClientManager>>();
    let config_manager = app.state::<ConfigManager>();

    // Check current state and toggle
    let is_enabled = client_manager
        .list_clients()
        .iter()
        .find(|c| c.id == client_id)
        .map(|c| c.enabled)
        .unwrap_or(true);

    if is_enabled {
        client_manager
            .disable_client(client_id)
            .map_err(|e| tauri::Error::Anyhow(e.into()))?;
        info!("Client {} disabled", client_id);
    } else {
        client_manager
            .enable_client(client_id)
            .map_err(|e| tauri::Error::Anyhow(e.into()))?;
        info!("Client {} enabled", client_id);
    }

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

    // Emit event for UI updates
    if let Err(e) = app.emit("clients-changed", ()) {
        error!("Failed to emit clients-changed event: {}", e);
    }

    Ok(())
}

/// Toggle skill access for a client from tray menu
/// Uses the new skills_permissions system
pub(crate) async fn handle_toggle_skill_access<R: Runtime>(
    app: &AppHandle<R>,
    client_id: &str,
    skill_name: &str,
) -> tauri::Result<()> {
    info!(
        "Toggling skill {} access for client {}",
        skill_name, client_id
    );

    let config_manager = app.state::<ConfigManager>();

    // Read current access and toggle by skill name using new permission system
    let mut found = false;
    config_manager
        .update(|cfg| {
            if let Some(client) = cfg.clients.iter_mut().find(|c| c.id == client_id) {
                // Check if skill is currently allowed using the new permission system
                let is_allowed = client
                    .skills_permissions
                    .resolve_skill(skill_name)
                    .is_enabled();
                if is_allowed {
                    // Set skill permission to Off
                    client
                        .skills_permissions
                        .skills
                        .insert(skill_name.to_string(), lr_config::PermissionState::Off);
                    info!("Skill {} removed from client {}", skill_name, client_id);
                } else {
                    // Set skill permission to Allow
                    client
                        .skills_permissions
                        .skills
                        .insert(skill_name.to_string(), lr_config::PermissionState::Allow);
                    info!("Skill {} added to client {}", skill_name, client_id);
                }
                found = true;
            }
        })
        .map_err(|e| tauri::Error::Anyhow(e.into()))?;

    if !found {
        return Err(tauri::Error::Anyhow(anyhow::anyhow!("Client not found")));
    }

    config_manager
        .save()
        .await
        .map_err(|e| tauri::Error::Anyhow(e.into()))?;

    // Rebuild tray menu
    rebuild_tray_menu(app)?;

    // Emit event for UI updates
    if let Err(e) = app.emit("clients-changed", ()) {
        error!("Failed to emit clients-changed event: {}", e);
    }

    Ok(())
}

/// Toggle coding agent permission for a client (Allow ↔ Off)
pub(crate) async fn handle_toggle_coding_agent_access<R: Runtime>(
    app: &AppHandle<R>,
    client_id: &str,
) -> tauri::Result<()> {
    info!("Toggling coding agent access for client {}", client_id);

    let config_manager = app.state::<ConfigManager>();

    let mut found = false;
    config_manager
        .update(|cfg| {
            if let Some(client) = cfg.clients.iter_mut().find(|c| c.id == client_id) {
                if client.coding_agent_permission.is_enabled() {
                    client.coding_agent_permission = lr_config::PermissionState::Off;
                    info!("Coding agent disabled for client {}", client_id);
                } else {
                    client.coding_agent_permission = lr_config::PermissionState::Allow;
                    info!("Coding agent enabled for client {}", client_id);
                }
                found = true;
            }
        })
        .map_err(|e| tauri::Error::Anyhow(e.into()))?;

    if !found {
        return Err(tauri::Error::Anyhow(anyhow::anyhow!("Client not found")));
    }

    config_manager
        .save()
        .await
        .map_err(|e| tauri::Error::Anyhow(e.into()))?;

    // Rebuild tray menu
    rebuild_tray_menu(app)?;

    // Emit event for UI updates
    if let Err(e) = app.emit("clients-changed", ()) {
        error!("Failed to emit clients-changed event: {}", e);
    }

    Ok(())
}

/// Copy text to clipboard
pub(crate) fn copy_to_clipboard(text: &str) -> Result<(), anyhow::Error> {
    let mut clipboard = arboard::Clipboard::new()
        .map_err(|e| anyhow::anyhow!("Failed to access clipboard: {}", e))?;

    clipboard
        .set_text(text)
        .map_err(|e| anyhow::anyhow!("Failed to set clipboard text: {}", e))?;

    Ok(())
}
