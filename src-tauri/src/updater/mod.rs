//! Update checking and management module
//!
//! Handles background update check scheduling and configuration.
//! The actual update checking is done by the Tauri updater plugin from the frontend.

use chrono::{DateTime, Utc};
use lr_config::{ConfigManager, UpdateConfig, UpdateMode};
use lr_utils::install_source::InstallSource;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_updater::UpdaterExt;
use tracing::{debug, error, info, warn};

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
            if last <= now
                && u64::try_from(days_since).is_ok_and(|days| days >= check_interval_days)
            {
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

/// How long LocalRouter must serve no client requests before an available
/// update is installed automatically.
pub const AUTO_INSTALL_IDLE: std::time::Duration = std::time::Duration::from_secs(5 * 60 * 60);

/// How often the idle auto-install loop re-evaluates.
const AUTO_INSTALL_POLL: std::time::Duration = std::time::Duration::from_secs(10 * 60);

/// After an installed update, how long the app must stay quiet before it
/// restarts, should a request have arrived during the download.
const RESTART_QUIET: std::time::Duration = std::time::Duration::from_secs(60);

/// Whether the idle auto-install should run now.
#[derive(Debug, PartialEq)]
pub enum AutoInstallDecision {
    Install,
    /// Auto-install or automatic checks are off.
    Disabled,
    ManagedExternally(InstallSource),
    /// Requests are in progress or the user is in the app window.
    Busy,
    /// A client request was served within [`AUTO_INSTALL_IDLE`].
    NotIdle,
    /// An attempt was made within [`AUTO_INSTALL_IDLE`]; idle stretches
    /// longer than that check again only once per period.
    RecentlyAttempted,
}

/// Pure decision logic for [`start_auto_install_timer`].
#[allow(clippy::too_many_arguments)]
pub fn auto_install_decision(
    update: &UpdateConfig,
    install_source: InstallSource,
    now: DateTime<Utc>,
    last_request_at: DateTime<Utc>,
    pending_requests: usize,
    window_focused: bool,
    last_attempt: Option<DateTime<Utc>>,
) -> AutoInstallDecision {
    if !install_source.is_self_updatable() {
        return AutoInstallDecision::ManagedExternally(install_source);
    }
    if !update.auto_install || update.mode != UpdateMode::Automatic {
        return AutoInstallDecision::Disabled;
    }
    if pending_requests > 0 || window_focused {
        return AutoInstallDecision::Busy;
    }
    let idle = chrono::Duration::from_std(AUTO_INSTALL_IDLE).expect("constant fits");
    if now - last_request_at < idle {
        return AutoInstallDecision::NotIdle;
    }
    if last_attempt.is_some_and(|attempt| now - attempt < idle) {
        return AutoInstallDecision::RecentlyAttempted;
    }
    AutoInstallDecision::Install
}

/// Install updates automatically once LocalRouter has been idle.
///
/// Every [`AUTO_INSTALL_POLL`], if auto-install is enabled and no client
/// request (LLM call or MCP request) has started or finished for
/// [`AUTO_INSTALL_IDLE`], none is in progress and the app window is not
/// focused, this checks for an update; an available, non-skipped update is
/// downloaded, installed and the app restarted.
pub async fn start_auto_install_timer(app: AppHandle, config_manager: Arc<ConfigManager>) {
    let install_source = lr_utils::install_source::current();
    if !install_source.is_self_updatable() {
        return;
    }
    info!("Starting idle auto-install timer");

    let mut last_attempt: Option<DateTime<Utc>> = None;
    loop {
        tokio::time::sleep(AUTO_INSTALL_POLL).await;

        let Some(store) = app
            .try_state::<Arc<lr_server::ServerManager>>()
            .and_then(|manager| manager.monitor_store())
        else {
            continue;
        };
        let window_focused = app
            .get_webview_window("main")
            .and_then(|window| window.is_focused().ok())
            .unwrap_or(false);
        let config = config_manager.get();
        let now = Utc::now();
        let decision = auto_install_decision(
            &config.update,
            install_source,
            now,
            store.last_request_at(),
            store.pending_requests(),
            window_focused,
            last_attempt,
        );
        if decision != AutoInstallDecision::Install {
            debug!("Idle auto-install: {:?}", decision);
            continue;
        }

        last_attempt = Some(now);
        if let Err(e) = install_available_update(
            &app,
            &config_manager,
            config.update.skipped_version.as_deref(),
            &store,
        )
        .await
        {
            warn!("Idle auto-install failed: {}", e);
        }
    }
}

/// Check for an update and, if one is available and not skipped, install it
/// and restart. Returns only when nothing was installed or installation failed.
async fn install_available_update(
    app: &AppHandle,
    config_manager: &ConfigManager,
    skipped_version: Option<&str>,
    store: &lr_monitor::MonitorEventStore,
) -> Result<(), String> {
    let update = app
        .updater()
        .map_err(|e| e.to_string())?
        .check()
        .await
        .map_err(|e| e.to_string())?;
    if let Err(e) = save_last_check_timestamp(config_manager).await {
        warn!("Failed to save update check timestamp: {}", e);
    }
    let Some(update) = update else {
        debug!("Idle auto-install: already up to date");
        return Ok(());
    };
    if skipped_version == Some(update.version.as_str()) {
        info!(
            "Idle auto-install: version {} was skipped by the user",
            update.version
        );
        return Ok(());
    }

    info!(
        "Idle auto-install: installing {} after {} hours without requests",
        update.version,
        AUTO_INSTALL_IDLE.as_secs() / 3600
    );
    update
        .download_and_install(|_, _| {}, || {})
        .await
        .map_err(|e| e.to_string())?;

    // A request may have arrived during the download; let it finish.
    loop {
        let quiet = Utc::now() - store.last_request_at();
        if store.pending_requests() == 0
            && quiet >= chrono::Duration::from_std(RESTART_QUIET).expect("constant fits")
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_secs(15)).await;
    }

    info!("Idle auto-install: restarting into {}", update.version);
    app.restart();
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn auto_config() -> UpdateConfig {
        UpdateConfig {
            auto_install: true,
            ..Default::default()
        }
    }

    #[test]
    fn auto_install_runs_after_five_idle_hours() {
        let now = Utc::now();
        let decide = |last_request: DateTime<Utc>, last_attempt| {
            auto_install_decision(
                &auto_config(),
                InstallSource::Direct,
                now,
                last_request,
                0,
                false,
                last_attempt,
            )
        };
        assert_eq!(
            decide(now - Duration::hours(4) - Duration::minutes(59), None),
            AutoInstallDecision::NotIdle
        );
        assert_eq!(
            decide(now - Duration::hours(5), None),
            AutoInstallDecision::Install
        );
        assert_eq!(
            decide(now - Duration::hours(9), Some(now - Duration::hours(1))),
            AutoInstallDecision::RecentlyAttempted
        );
        assert_eq!(
            decide(now - Duration::hours(11), Some(now - Duration::hours(5))),
            AutoInstallDecision::Install
        );
    }

    #[test]
    fn auto_install_waits_for_requests_and_the_focused_window() {
        let now = Utc::now();
        let idle = now - Duration::hours(6);
        for (pending, focused) in [(1, false), (0, true)] {
            assert_eq!(
                auto_install_decision(
                    &auto_config(),
                    InstallSource::Direct,
                    now,
                    idle,
                    pending,
                    focused,
                    None
                ),
                AutoInstallDecision::Busy
            );
        }
    }

    #[test]
    fn auto_install_respects_settings_and_install_source() {
        let now = Utc::now();
        let idle = now - Duration::hours(6);
        let off = UpdateConfig {
            auto_install: false,
            ..Default::default()
        };
        let manual = UpdateConfig {
            mode: UpdateMode::Manual,
            ..auto_config()
        };
        for config in [off, manual] {
            assert_eq!(
                auto_install_decision(&config, InstallSource::Direct, now, idle, 0, false, None),
                AutoInstallDecision::Disabled
            );
        }
        assert_eq!(
            auto_install_decision(
                &auto_config(),
                InstallSource::Homebrew,
                now,
                idle,
                0,
                false,
                None
            ),
            AutoInstallDecision::ManagedExternally(InstallSource::Homebrew)
        );
    }

    #[test]
    fn large_intervals_and_future_timestamps_do_not_trigger_updates() {
        let now = Utc::now();
        for (last, interval) in [
            (now, u64::MAX),
            (now + Duration::days(1), 0),
            (now + Duration::seconds(1), 0),
        ] {
            assert_eq!(
                should_check_for_updates(
                    &UpdateMode::Automatic,
                    Some(last),
                    interval,
                    now,
                    InstallSource::Direct
                ),
                UpdateCheckDecision::NotYet,
            );
        }
    }

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
            InstallSource::Nix,
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
