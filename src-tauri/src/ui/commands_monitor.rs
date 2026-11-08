//! Monitor-related Tauri commands
//!
//! Commands for real-time traffic inspection via the in-memory MonitorEventStore.
//! The store outlives server restarts, so these also work while it is stopped.

use std::sync::Arc;
use tauri::State;

use lr_mcp::gateway::firewall::InterceptRule;
use lr_monitor::{MonitorEvent, MonitorEventFilter, MonitorEventListResponse, MonitorStats};
use lr_server::ServerManager;

fn monitor_store(
    server_manager: &ServerManager,
) -> Result<Arc<lr_monitor::MonitorEventStore>, String> {
    server_manager
        .monitor_store()
        .ok_or_else(|| "Server has not started".to_string())
}

/// Get paginated monitor events (summaries for list view).
#[tauri::command]
pub async fn get_monitor_events(
    offset: usize,
    limit: usize,
    filter: Option<MonitorEventFilter>,
    server_manager: State<'_, Arc<ServerManager>>,
) -> Result<MonitorEventListResponse, String> {
    Ok(monitor_store(&server_manager)?.list(offset, limit, filter.as_ref()))
}

/// Get full detail for a single monitor event.
#[tauri::command]
pub async fn get_monitor_event_detail(
    event_id: String,
    server_manager: State<'_, Arc<ServerManager>>,
) -> Result<Option<MonitorEvent>, String> {
    Ok(monitor_store(&server_manager)?.get(&event_id))
}

/// Reveal a retained redacted finding only through its captured client identity.
#[tauri::command]
pub async fn reveal_monitor_secret(
    event_id: String,
    finding_index: usize,
    server_manager: State<'_, Arc<ServerManager>>,
) -> Result<String, String> {
    use lr_types::secret_context::SecretContext;
    let event = monitor_store(&server_manager)?
        .get(&event_id)
        .ok_or("Event is no longer retained")?;
    let owner = event.client_id.ok_or("Event has no client identity")?;
    let lr_monitor::MonitorEventData::SecretScan {
        findings: Some(findings),
        ..
    } = event.data
    else {
        return Err("Event is not a retained secret scan".into());
    };
    let finding = findings
        .get(finding_index)
        .ok_or("Finding is not retained")?;
    if finding["redacted"] != true {
        return Err("Finding was not redacted".into());
    }
    let replacement = finding["replacement"]
        .as_str()
        .ok_or("Replacement was not retained")?;
    let vault = lr_secret_scanner::placeholders::Vault::find_client(
        lr_api_keys::keychain_trait::redacted_secret_storage(),
        &owner,
        lr_secret_scanner::placeholders::RETENTION_SECS,
    )
    .map_err(|e| e.to_string())?
    .ok_or("Secret mapping has expired or was removed")?;
    vault
        .restore_text(replacement, true)
        .map_err(|e| e.to_string())
}

/// Clear all monitor events.
#[tauri::command]
pub async fn clear_monitor_events(
    server_manager: State<'_, Arc<ServerManager>>,
) -> Result<(), String> {
    monitor_store(&server_manager)?.clear();
    Ok(())
}

/// Get monitor store statistics.
#[tauri::command]
pub async fn get_monitor_stats(
    server_manager: State<'_, Arc<ServerManager>>,
) -> Result<MonitorStats, String> {
    Ok(monitor_store(&server_manager)?.stats())
}

/// Update the maximum event capacity.
#[tauri::command]
pub async fn set_monitor_max_capacity(
    capacity: usize,
    server_manager: State<'_, Arc<ServerManager>>,
) -> Result<(), String> {
    monitor_store(&server_manager)?.set_max_capacity(capacity);
    Ok(())
}

/// Update the monitor's memory budget in bytes.
///
/// Events hold whole request/response bodies, so this — not the event count —
/// is what actually bounds the store's footprint. Oldest events are evicted
/// immediately if the new budget is smaller than what is already held.
#[tauri::command]
pub async fn set_monitor_max_bytes(
    max_bytes: usize,
    server_manager: State<'_, Arc<ServerManager>>,
) -> Result<(), String> {
    monitor_store(&server_manager)?.set_max_bytes(max_bytes);
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
