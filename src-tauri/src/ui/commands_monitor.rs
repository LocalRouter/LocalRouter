//! Monitor-related Tauri commands
//!
//! Commands for real-time traffic inspection via the in-memory MonitorEventStore.

use std::sync::Arc;
use tauri::State;

use lr_mcp::gateway::firewall::InterceptRule;
use lr_monitor::{MonitorEvent, MonitorEventFilter, MonitorEventListResponse, MonitorStats};
use lr_server::ServerManager;

/// Get paginated monitor events (summaries for list view).
#[tauri::command]
pub async fn get_monitor_events(
    offset: usize,
    limit: usize,
    filter: Option<MonitorEventFilter>,
    server_manager: State<'_, Arc<ServerManager>>,
) -> Result<MonitorEventListResponse, String> {
    let app_state = server_manager
        .get_state()
        .ok_or_else(|| "Server is not running".to_string())?;

    Ok(app_state.monitor_store.list(offset, limit, filter.as_ref()))
}

/// Get full detail for a single monitor event.
#[tauri::command]
pub async fn get_monitor_event_detail(
    event_id: String,
    server_manager: State<'_, Arc<ServerManager>>,
) -> Result<Option<MonitorEvent>, String> {
    let app_state = server_manager
        .get_state()
        .ok_or_else(|| "Server is not running".to_string())?;

    Ok(app_state.monitor_store.get(&event_id))
}

/// Clear all monitor events.
#[tauri::command]
pub async fn clear_monitor_events(
    server_manager: State<'_, Arc<ServerManager>>,
) -> Result<(), String> {
    let app_state = server_manager
        .get_state()
        .ok_or_else(|| "Server is not running".to_string())?;

    app_state.monitor_store.clear();
    Ok(())
}

/// Get monitor store statistics.
#[tauri::command]
pub async fn get_monitor_stats(
    server_manager: State<'_, Arc<ServerManager>>,
) -> Result<MonitorStats, String> {
    let app_state = server_manager
        .get_state()
        .ok_or_else(|| "Server is not running".to_string())?;

    Ok(app_state.monitor_store.stats())
}

/// Update the maximum event capacity.
#[tauri::command]
pub async fn set_monitor_max_capacity(
    capacity: usize,
    server_manager: State<'_, Arc<ServerManager>>,
) -> Result<(), String> {
    let app_state = server_manager
        .get_state()
        .ok_or_else(|| "Server is not running".to_string())?;

    app_state.monitor_store.set_max_capacity(capacity);
    Ok(())
}

/// Set or clear the monitor intercept rule.
///
/// When active, matching requests will force a firewall popup regardless of permissions.
/// Pass `None` to clear (disable interception).
#[tauri::command]
pub async fn set_monitor_intercept_rule(
    rule: Option<InterceptRule>,
    server_manager: State<'_, Arc<ServerManager>>,
) -> Result<(), String> {
    let app_state = server_manager
        .get_state()
        .ok_or_else(|| "Server is not running".to_string())?;

    app_state
        .mcp_gateway
        .firewall_manager
        .set_intercept_rule(rule);
    Ok(())
}

/// Get the current monitor intercept rule (or null if not active).
#[tauri::command]
pub async fn get_monitor_intercept_rule(
    server_manager: State<'_, Arc<ServerManager>>,
) -> Result<Option<InterceptRule>, String> {
    let app_state = server_manager
        .get_state()
        .ok_or_else(|| "Server is not running".to_string())?;

    Ok(app_state.mcp_gateway.firewall_manager.get_intercept_rule())
}
