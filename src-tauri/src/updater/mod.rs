//! Update checking and management module
//!
//! Handles background update check scheduling and configuration.
//! The actual update checking is done by the Tauri updater plugin from the frontend.

#![allow(dead_code)]

use chrono::Utc;
use lr_config::{ConfigManager, UpdateMode};
use lr_utils::install_source::InstallSource;
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

/// Result of evaluating whether an update check should be performed
#[derive(Debug, PartialEq)]
pub enum UpdateCheckDecision {
    /// Should perform an update check
    ShouldCheck,
    /// First run — set initial timestamp but don't check
    FirstRun,
    /// Not time to check yet (interval not elapsed)
    NotYet,
    /// Automatic mode is disabled
    Disabled,
    /// A package manager owns this install, so we must not self-update
    ManagedExternally(InstallSource),
}

/// Evaluate whether an update check should be performed based on config.
///
/// This is the pure decision logic extracted from `start_update_timer`
/// so it can be tested independently.
pub fn should_check_for_updates(
    mode: &UpdateMode,
    last_check: Option<chrono::DateTime<Utc>>,
    check_interval_days: u64,
    now: chrono::DateTime<Utc>,
    install_source: InstallSource,
) -> UpdateCheckDecision {
    // Checked before the mode so that a package-managed install reports *why*
    // it will never update, even if the user also left the mode on Automatic.
    // Self-updating on top of brew/apt/flatpak desynchronises the package
    // manager's recorded version from what is actually on disk.
    if !install_source.is_self_updatable() {
        return UpdateCheckDecision::ManagedExternally(install_source);
    }

    if *mode != UpdateMode::Automatic {
        return UpdateCheckDecision::Disabled;
    }

    match last_check {
        None => UpdateCheckDecision::FirstRun,
        Some(last) => {
            let days_since = (now - last).num_days();
            if days_since >= check_interval_days as i64 {
                UpdateCheckDecision::ShouldCheck
            } else {
                UpdateCheckDecision::NotYet
            }
        }
    }
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
    let install_source = lr_utils::install_source::current();

    // Detected once at startup — the install location cannot change while the
    // process is running, and probing the filesystem every 24h would be waste.
    if !install_source.is_self_updatable() {
        info!(
            "Install is managed by {}; background update timer will not run",
            install_source.label()
        );
        return;
    }

    info!("Starting background update timer");

    loop {
        // Sleep for 24 hours before next check
        tokio::time::sleep(tokio::time::Duration::from_secs(86400)).await;

        let config = config_manager.get();
        let update_config = &config.update;
        let now = Utc::now();

        let decision = should_check_for_updates(
            &update_config.mode,
            update_config.last_check,
            update_config.check_interval_days,
            now,
            install_source,
        );

        match decision {
            UpdateCheckDecision::ManagedExternally(source) => {
                // Unreachable given the early return above, but keeping the
                // match exhaustive means a future caller cannot silently fall
                // through to a self-update.
                debug!("Install managed by {}, skipping check", source.label());
                continue;
            }
            UpdateCheckDecision::Disabled => {
                debug!("Automatic updates disabled, skipping check");
                continue;
            }
            UpdateCheckDecision::FirstRun => {
                info!("First run detected - setting initial timestamp without checking");
                if let Err(e) = save_last_check_timestamp(&config_manager).await {
                    error!("Failed to save initial timestamp: {}", e);
                }
                continue;
            }
            UpdateCheckDecision::NotYet => {
                debug!("Not time to check yet");
                continue;
            }
            UpdateCheckDecision::ShouldCheck => {
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    #[test]
    fn package_managed_installs_never_self_update() {
        let now = Utc::now();
        // Automatic mode, interval long elapsed: the only thing stopping a
        // self-update here is the install source.
        let last_check = now - Duration::days(365);
        for source in [
            InstallSource::Homebrew,
            InstallSource::Scoop,
            InstallSource::WinGet,
            InstallSource::Aur,
            InstallSource::Debian,
            InstallSource::Rpm,
            InstallSource::SystemPackage,
            InstallSource::Flatpak,
            InstallSource::Snap,
            InstallSource::Docker,
        ] {
            assert_eq!(
                should_check_for_updates(&UpdateMode::Automatic, Some(last_check), 7, now, source,),
                UpdateCheckDecision::ManagedExternally(source),
                "{source:?} must not trigger a self-update"
            );
        }
    }

    #[test]
    fn managed_install_reports_the_manager_even_in_manual_mode() {
        // The UI needs to say "managed by Homebrew" rather than the generic
        // "updates are off", so the source check comes first.
        let now = Utc::now();
        assert_eq!(
            should_check_for_updates(
                &UpdateMode::Manual,
                Some(now),
                7,
                now,
                InstallSource::Homebrew,
            ),
            UpdateCheckDecision::ManagedExternally(InstallSource::Homebrew)
        );
    }

    #[test]
    fn appimage_still_self_updates() {
        // Tauri's updater can replace an AppImage in place, so it keeps the
        // normal scheduling behaviour.
        let now = Utc::now();
        let last_check = now - Duration::days(10);
        assert_eq!(
            should_check_for_updates(
                &UpdateMode::Automatic,
                Some(last_check),
                7,
                now,
                InstallSource::AppImage,
            ),
            UpdateCheckDecision::ShouldCheck
        );
    }

    #[test]
    fn test_disabled_mode_returns_disabled() {
        let now = Utc::now();
        assert_eq!(
            should_check_for_updates(
                &UpdateMode::Manual,
                Some(now),
                7,
                now,
                InstallSource::Direct,
            ),
            UpdateCheckDecision::Disabled
        );
    }

    #[test]
    fn test_first_run_no_last_check() {
        let now = Utc::now();
        assert_eq!(
            should_check_for_updates(&UpdateMode::Automatic, None, 7, now, InstallSource::Direct,),
            UpdateCheckDecision::FirstRun
        );
    }

    #[test]
    fn test_not_yet_within_interval() {
        let now = Utc::now();
        let last_check = now - Duration::days(3);
        assert_eq!(
            should_check_for_updates(
                &UpdateMode::Automatic,
                Some(last_check),
                7,
                now,
                InstallSource::Direct,
            ),
            UpdateCheckDecision::NotYet
        );
    }

    #[test]
    fn test_should_check_interval_elapsed() {
        let now = Utc::now();
        let last_check = now - Duration::days(7);
        assert_eq!(
            should_check_for_updates(
                &UpdateMode::Automatic,
                Some(last_check),
                7,
                now,
                InstallSource::Direct,
            ),
            UpdateCheckDecision::ShouldCheck
        );
    }

    #[test]
    fn test_should_check_interval_exceeded() {
        let now = Utc::now();
        let last_check = now - Duration::days(30);
        assert_eq!(
            should_check_for_updates(
                &UpdateMode::Automatic,
                Some(last_check),
                7,
                now,
                InstallSource::Direct,
            ),
            UpdateCheckDecision::ShouldCheck
        );
    }

    #[test]
    fn test_just_checked_returns_not_yet() {
        let now = Utc::now();
        assert_eq!(
            should_check_for_updates(
                &UpdateMode::Automatic,
                Some(now),
                1,
                now,
                InstallSource::Direct,
            ),
            UpdateCheckDecision::NotYet
        );
    }

    #[test]
    fn test_one_day_interval_checks_daily() {
        let now = Utc::now();
        let last_check = now - Duration::days(1);
        assert_eq!(
            should_check_for_updates(
                &UpdateMode::Automatic,
                Some(last_check),
                1,
                now,
                InstallSource::Direct,
            ),
            UpdateCheckDecision::ShouldCheck
        );
    }
}
