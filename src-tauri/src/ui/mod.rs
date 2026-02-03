//! UI integration module
//!
//! Tauri commands and system tray management.

pub mod commands;
pub mod commands_clients;
pub mod commands_marketplace;
pub mod commands_mcp;
pub mod commands_mcp_metrics;
pub mod commands_metrics;
pub mod commands_providers;
pub mod commands_routellm;
pub mod tray;
pub mod tray_graph;
pub mod tray_graph_manager;
pub mod tray_menu;

// TODO: Implement UI integration
// - Tauri command handlers
// - System tray menu
// - IPC communication
