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

/// Provider types whose usage endpoint LocalRouter can ask; Settings → Usage
/// shows a switch for each of their instances.
#[tauri::command]
pub async fn get_usage_polled_provider_types() -> Result<Vec<String>, String> {
    Ok(lr_usage::providers::polled_provider_types()
        .into_iter()
        .map(str::to_string)
        .collect())
}

#[tauri::command]
pub async fn get_usage_tracking_config(
    config_manager: State<'_, ConfigManager>,
) -> Result<UsageTrackingConfig, String> {
    Ok(config_manager.get().usage_tracking)
}

/// Replace the `usage_tracking` settings; takes effect immediately.
///
/// `tray_items_added` is LocalRouter's own bookkeeping, appended to in the
/// background: entries already recorded are kept even when `config` was
/// read before they were added, so a stale copy cannot bring back a tray
/// item the user removed.
#[tauri::command]
pub async fn update_usage_tracking_config(
    config: UsageTrackingConfig,
    app: AppHandle,
    config_manager: State<'_, ConfigManager>,
    poller: State<'_, Arc<UsagePoller>>,
) -> Result<(), String> {
    let mut config = config;
    config.poll_interval_secs = config.poll_interval_secs.clamp(60, 3_600);
    config.idle_poll_interval_secs = config.idle_poll_interval_secs.clamp(300, 86_400);
    config_manager
        .update(|cfg| cfg.usage_tracking = merge_usage_config(&cfg.usage_tracking, config))
        .map_err(|e| e.to_string())?;
    config_manager.save().await.map_err(|e| e.to_string())?;
    poller.apply_config(config_manager.get().usage_tracking);
    if let Some(tray) = app.try_state::<Arc<crate::ui::tray_graph_manager::TrayGraphManager>>() {
        tray.usage_limits_changed();
    }
    Ok(())
}

/// `incoming` with every `tray_items_added` entry of `current` kept.
fn merge_usage_config(
    current: &UsageTrackingConfig,
    mut incoming: UsageTrackingConfig,
) -> UsageTrackingConfig {
    for key in &current.tray_items_added {
        if !incoming.tray_items_added.contains(key) {
            incoming.tray_items_added.push(key.clone());
        }
    }
    incoming
}

/// Forget everything recorded for one account.
#[tauri::command]
pub async fn forget_usage_account(
    account_id: String,
    tracker: State<'_, Arc<UsageTracker>>,
) -> Result<bool, String> {
    Ok(tracker.forget(&account_id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_save_keeps_tray_items_added_meanwhile() {
        let loaded = UsageTrackingConfig::default();
        let mut current = loaded.clone();
        current
            .tray_items_added
            .push("anthropic:subscription|seven_day".into());
        let mut incoming = loaded;
        incoming.read_cli_logins = true;
        incoming
            .tray_items_added
            .push("openai:subscription|seven_day".into());
        let merged = merge_usage_config(&current, incoming);
        assert!(merged.read_cli_logins);
        assert_eq!(
            merged.tray_items_added,
            vec![
                "openai:subscription|seven_day".to_string(),
                "anthropic:subscription|seven_day".to_string(),
            ]
        );
    }
}
