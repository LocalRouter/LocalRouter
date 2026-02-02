//! Update checking and management module
//!
//! Handles background update check scheduling and configuration.
//! The actual update checking is done by the Tauri updater plugin from the frontend.

#![allow(dead_code)]

use chrono::Utc;
use lr_config::{ConfigManager, UpdateMode};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tauri::{AppHandle, Emitter};
use tracing::{debug, error, info};

/// Update information returned to the frontend
/// This matches the structure from @tauri-apps/plugin-updater
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateInfo {
    /// Latest available version
    pub version: String,
    /// Release notes in markdown
    pub notes: String,
    /// Download URL for this platform
    pub download_url: String,
    /// Release date
    pub published_at: String,
}

/// Save the last check timestamp to config
pub async fn save_last_check_timestamp(config_manager: &ConfigManager) -> Result<(), String> {
    let now = Utc::now();

    config_manager
        .update(|config| {
            config.update.last_check = Some(now);
        })
        .map_err(|e| e.to_string())?;

    config_manager.save().await.map_err(|e| e.to_string())?;

    debug!("Updated last_check timestamp to {}", now);
    Ok(())
}

/// Start the background update checking timer
///
/// This function:
/// 1. Runs in a continuous loop
/// 2. Checks every 24 hours if automatic mode is enabled
/// 3. Only triggers update check if >= check_interval_days have passed since last check
/// 4. On first run (last_check = None), just sets timestamp without checking
/// 5. Emits "check-for-updates" event to frontend to trigger actual update check
pub async fn start_update_timer(app: AppHandle, config_manager: Arc<ConfigManager>) {
    info!("Starting background update timer");

    loop {
        // Sleep for 24 hours before next check
        tokio::time::sleep(tokio::time::Duration::from_secs(86400)).await;

        let config = config_manager.get();
        let update_config = &config.update;

        // Only check if automatic mode is enabled
        if update_config.mode != UpdateMode::Automatic {
            debug!("Automatic updates disabled, skipping check");
            continue;
        }

        let now = Utc::now();

        // Determine if we should check
        let should_check = match update_config.last_check {
            None => {
                // First run - set timestamp, don't check
                info!("First run detected - setting initial timestamp without checking");
                if let Err(e) = save_last_check_timestamp(&config_manager).await {
                    error!("Failed to save initial timestamp: {}", e);
                }
                false
            }
            Some(last) => {
                let days_since = (now - last).num_days();
                let should = days_since >= update_config.check_interval_days as i64;

                if should {
                    debug!(
                        "Time to check for updates ({} days since last check)",
                        days_since
                    );
                } else {
                    debug!(
                        "Not time to check yet ({} days since last check, interval: {} days)",
                        days_since, update_config.check_interval_days
                    );
                }

                should
            }
        };

        if !should_check {
            continue;
        }

        // Trigger update check by emitting event to frontend
        // Frontend will use @tauri-apps/plugin-updater to actually check
        // The frontend calls mark_update_check_performed after the check completes,
        // which saves the timestamp — no need to save here too.
        info!("Emitting check-for-updates event to frontend");
        if let Err(e) = app.emit("check-for-updates", ()) {
            error!("Failed to emit check-for-updates event: {}", e);
        }
    }
}
