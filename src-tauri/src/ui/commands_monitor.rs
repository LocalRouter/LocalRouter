//! Monitor-related Tauri commands
//!
//! Commands for real-time traffic inspection via the in-memory MonitorEventStore.

use std::sync::Arc;
use tauri::State;

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
