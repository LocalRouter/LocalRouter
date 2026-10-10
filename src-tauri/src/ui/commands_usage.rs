//! Tauri commands for subscription and usage-limit tracking.

use std::sync::Arc;

use lr_config::{ConfigManager, UsageTrackingConfig};
use lr_usage::{UsageSnapshot, UsageTracker};
use tauri::{AppHandle, Manager, State};

use crate::ui::usage_poller::{UsagePollStatus, UsagePoller};

/// Every tracked account with its windows, limits, credits and spend.
#[tauri::command]
pub async fn get_usage_limits(
    tracker: State<'_, Arc<UsageTracker>>,
) -> Result<UsageSnapshot, String> {
    Ok(tracker.snapshot())
}

/// Ask the usage endpoints again now (as the settings allow).
#[tauri::command]
pub async fn refresh_usage_limits(poller: State<'_, Arc<UsagePoller>>) -> Result<(), String> {
    poller.refresh_now();
    Ok(())
}

/// Outcome of the last poll of each usage source.
#[tauri::command]
pub async fn get_usage_poll_status(
    poller: State<'_, Arc<UsagePoller>>,
) -> Result<Vec<UsagePollStatus>, String> {
    Ok(poller.statuses())
}

#[tauri::command]
pub async fn get_usage_tracking_config(
    config_manager: State<'_, ConfigManager>,
) -> Result<UsageTrackingConfig, String> {
    Ok(config_manager.get().usage_tracking)
}

/// Replace the `usage_tracking` settings; takes effect immediately.
#[tauri::command]
pub async fn update_usage_tracking_config(
    config: UsageTrackingConfig,
    app: AppHandle,
    config_manager: State<'_, ConfigManager>,
    tracker: State<'_, Arc<UsageTracker>>,
    poller: State<'_, Arc<UsagePoller>>,
) -> Result<(), String> {
    let mut config = config;
    config.poll_interval_secs = config.poll_interval_secs.clamp(60, 3_600);
    config.idle_poll_interval_secs = config.idle_poll_interval_secs.clamp(300, 86_400);
    let previous = config_manager.get().usage_tracking;
    config_manager
        .update(|cfg| cfg.usage_tracking = config.clone())
        .map_err(|e| e.to_string())?;
    config_manager.save().await.map_err(|e| e.to_string())?;
    let sources_changed = previous.enabled != config.enabled
        || previous.poll_provider_apis != config.poll_provider_apis
        || previous.poll_excluded_providers != config.poll_excluded_providers
        || previous.read_cli_logins != config.read_cli_logins;
    tracker.set_config(config);
    if sources_changed {
        poller.refresh_now();
    }
    if let Some(tray) = app.try_state::<Arc<crate::ui::tray_graph_manager::TrayGraphManager>>() {
        tray.usage_limits_changed();
    }
    Ok(())
}

/// Forget everything recorded for one account.
#[tauri::command]
pub async fn forget_usage_account(
    account_id: String,
    tracker: State<'_, Arc<UsageTracker>>,
) -> Result<bool, String> {
    Ok(tracker.forget(&account_id))
}
