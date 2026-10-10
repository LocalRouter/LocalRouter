//! UI integration module
//!
//! Tauri commands and system tray management.

pub mod commands;
pub mod commands_clients;
pub mod commands_coding_agents;
pub mod commands_engines;
pub mod commands_free_tier;
pub mod commands_local_models;
pub mod commands_marketplace;
pub mod commands_mcp;
pub mod commands_mcp_metrics;
pub mod commands_metrics;
pub mod commands_monitor;
pub mod commands_providers;
pub mod commands_reverse_proxy;
pub mod commands_usage;
mod input_validation;
mod skill_paths;
pub mod tray;
pub mod tray_font;
pub mod tray_format;
pub mod tray_graph;
pub mod tray_graph_manager;
pub mod tray_menu;
pub mod tray_menu_keeper;
pub mod tray_support;
pub mod tray_usage;
pub mod usage_poller;

// TODO: Implement UI integration
// - Tauri command handlers
// - System tray menu
// - IPC communication

pub mod commands_decision_routing;
