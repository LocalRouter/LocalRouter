//! Usage-limit windows as tray stats items: their panel value, their menu /
//! tooltip line, and automatically adding newly seen subscriptions to the
//! tray stats items.

use std::sync::Arc;

use chrono::TimeZone;
use lr_config::{ConfigManager, TraySource, TrayUsageShows};
use lr_usage::{AccountKind, UsageSnapshot, UsageTracker, UsageWindowView};
use tauri::{AppHandle, Manager};

/// A usage window's current reading, as shown by the tray.
#[derive(Debug, Clone, PartialEq)]
pub struct LimitReading {
    /// e.g. "Claude Max 20x".
    pub account_title: String,
    pub window: UsageWindowView,
}

impl LimitReading {
    /// Gauge fill, 0–1: the share used, or the share left. Empty once the
    /// window reset without a new reading.
    pub fn fill(&self, shows: TrayUsageShows) -> f32 {
        if self.window.stale {
            return 0.0;
        }
        let used = (self.window.used_percent / 100.0).clamp(0.0, 1.0);
        match shows {
            TrayUsageShows::Used => used as f32,
            TrayUsageShows::Remaining => (1.0 - used) as f32,
        }
    }

    /// Number-panel text: `45%` used or `55%` left, or `--` once the
    /// window reset.
    pub fn number(&self, shows: TrayUsageShows) -> String {
        if self.window.stale {
            "--".to_string()
        } else {
            match shows {
                TrayUsageShows::Used => format_percent(self.window.used_percent),
                TrayUsageShows::Remaining => format_remaining(self.window.used_percent),
            }
        }
    }
}

/// The reading for a `Usage` tray source, if the tracker has that window
/// (and the account is not hidden).
pub fn reading_for(snapshot: &UsageSnapshot, source: &TraySource) -> Option<LimitReading> {
    let TraySource::Usage { account, window } = source else {
        return None;
    };
    if !snapshot.enabled {
        return None;
    }
    let a = snapshot
        .accounts
        .iter()
        .find(|a| &a.id == account && !a.hidden)?;
    let w = a.windows.iter().find(|w| &w.id == window)?;
    Some(LimitReading {
        account_title: a.title.clone(),
        window: w.clone(),
    })
}

/// A used percentage for display, rounded *down* so a window with room left
/// never reads as 100%.
pub fn format_percent(used: f64) -> String {
    let p = used.max(0.0);
    format!("{}%", (p + 1e-6).floor() as i64)
}

/// The share left for a used percentage, as the complement of the
/// rounded-down used figure, so `45%` used and `55%` left always add up and
/// a window with room left never reads as `0%` left.
pub fn format_remaining(used: f64) -> String {
    let used = (used.max(0.0) + 1e-6).floor() as i64;
    format!("{}%", (100 - used).max(0))
}

/// A percentage of the window as the setting reports it: `45%` (used) or
/// `55% left`.
fn share(used: f64, shows: TrayUsageShows) -> String {
    match shows {
        TrayUsageShows::Used => format_percent(used),
        TrayUsageShows::Remaining => format!("{} left", format_remaining(used)),
    }
}

/// Tray menu / tooltip line for a usage item, e.g.
/// `A7D  Claude Max 20x · Weekly 45% · resets Tue 09:00 · on pace for 78%`.
pub fn usage_item_line(
    label: &str,
    reading: Option<&LimitReading>,
    shows: TrayUsageShows,
) -> String {
    match reading {
        Some(r) => format!(
            "{label}  {} · {}",
            r.account_title,
            window_line(&r.window, shows)
        ),
        None => format!("{label}  no data yet"),
    }
}

/// `Weekly 45% · resets Tue 09:00 · on pace for 78%`, or with `Remaining`
/// `Weekly 55% left · resets Tue 09:00 · on pace for 22% left`.
pub fn window_line(window: &UsageWindowView, shows: TrayUsageShows) -> String {
    if window.stale {
        return format!("{} — reset, waiting for new data", window.label);
    }
    let mut parts = vec![format!(
        "{} {}",
        window.label,
        share(window.used_percent, shows)
    )];
    if let Some(reset) = window
        .resets_at
        .and_then(|t| chrono::Local.timestamp_opt(t, 0).single())
    {
        parts.push(format!("resets {}", reset.format("%a %H:%M")));
    }
    if let Some(eta) = window
        .limit_eta
        .and_then(|t| chrono::Local.timestamp_opt(t, 0).single())
    {
        parts.push(format!("limit ~{}", eta.format("%a %H:%M")));
    } else if let Some(projected) = window.projected_percent {
        parts.push(format!("on pace for {}", share(projected, shows)));
    }
    parts.join(" · ")
}

/// The window a newly seen subscription contributes to the tray: weekly
/// when it has one, else monthly, else its first window.
fn headline_window(windows: &[UsageWindowView]) -> Option<&UsageWindowView> {
    windows
        .iter()
        .find(|w| w.id == "seven_day")
        .or_else(|| windows.iter().find(|w| w.id == "monthly"))
        .or_else(|| windows.first())
}

/// `account|window` keys of subscriptions' headline windows that have not
/// been added to the tray yet.
pub fn pending_tray_items(
    snapshot: &UsageSnapshot,
    already_added: &[String],
) -> Vec<(String, String)> {
    if !snapshot.enabled {
        return Vec::new();
    }
    snapshot
        .accounts
        .iter()
        .filter(|a| a.kind == AccountKind::Subscription && !a.hidden)
        .filter_map(|a| {
            let w = headline_window(&a.windows)?;
            let key = format!("{}|{}", a.id, w.id);
            (!already_added.contains(&key)).then(|| (a.id.clone(), w.id.clone()))
        })
        .collect()
}

/// Add newly seen subscriptions' headline windows to the tray stats items,
/// once each (a removed item stays removed).
pub async fn sync_tray_items(app: &AppHandle) {
    let (Some(tracker), Some(config_manager)) = (
        app.try_state::<Arc<UsageTracker>>(),
        app.try_state::<ConfigManager>(),
    ) else {
        return;
    };
    let pending = pending_tray_items(
        &tracker.snapshot(),
        &config_manager.get().usage_tracking.tray_items_added,
    );
    if pending.is_empty() {
        return;
    }
    let updated = config_manager.update(|cfg| {
        for (account, window) in &pending {
            cfg.ui.tray_stats.on_usage_window_seen(account, window);
            let key = format!("{account}|{window}");
            if !cfg.usage_tracking.tray_items_added.contains(&key) {
                cfg.usage_tracking.tray_items_added.push(key);
            }
        }
    });
    if let Err(e) = updated {
        tracing::warn!("Failed to add usage windows to the tray: {}", e);
        return;
    }
    if let Err(e) = config_manager.save().await {
        tracing::warn!("Failed to save tray usage items: {}", e);
    }
    // Keep the tracker's copy of the usage settings current.
    tracker.set_config(config_manager.get().usage_tracking);
    if let Some(tray) = app.try_state::<Arc<crate::ui::tray_graph_manager::TrayGraphManager>>() {
        tray.update_config(config_manager.get().ui);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lr_usage::{DataSource, PaceStatus, UsageAccountView, UsageSpendView};

    fn window(id: &str, used: f64, stale: bool) -> UsageWindowView {
        UsageWindowView {
            id: id.into(),
            label: "Weekly".into(),
            used_percent: used,
            resets_at: Some(2_000_000_000),
            window_secs: Some(604_800),
            updated_at: 0,
            source: DataSource::ProxyHeaders,
            stale,
            elapsed_fraction: Some(0.5),
            projected_percent: Some(used * 2.0),
            limit_eta: None,
            pace: PaceStatus::Ok,
            slots: vec![],
            current_slot: 0,
            slot_secs: None,
            slots_basis: None,
            api_equivalent_usd: None,
            plan_share_usd: None,
            history: vec![],
        }
    }

    fn account(id: &str, kind: AccountKind, windows: Vec<UsageWindowView>) -> UsageAccountView {
        UsageAccountView {
            id: id.into(),
            provider: "anthropic".into(),
            provider_label: "Anthropic".into(),
            kind,
            title: "Claude Max 20x".into(),
            login_label: None,
            plan: None,
            plan_label: None,
            monthly_price_usd: None,
            plan_overridden: false,
            status: None,
            sources: vec![],
            first_seen: 0,
            last_seen: 0,
            hidden: false,
            windows,
            quotas: vec![],
            credits: None,
            reports_limits: true,
            spend: UsageSpendView::default(),
            value_multiplier: None,
        }
    }

    fn snapshot(accounts: Vec<UsageAccountView>) -> UsageSnapshot {
        UsageSnapshot {
            enabled: true,
            generated_at: 0,
            accounts,
        }
    }

    fn usage(account: &str, window: &str) -> TraySource {
        TraySource::Usage {
            account: account.into(),
            window: window.into(),
        }
    }

    #[test]
    fn readings_fill_and_number() {
        let snap = snapshot(vec![account(
            "anthropic:subscription",
            AccountKind::Subscription,
            vec![
                window("seven_day", 45.9, false),
                window("five_hour", 99.0, true),
            ],
        )]);
        let used = TrayUsageShows::Used;
        let left = TrayUsageShows::Remaining;
        let week = reading_for(&snap, &usage("anthropic:subscription", "seven_day")).unwrap();
        assert!((week.fill(used) - 0.459).abs() < 1e-6);
        assert!((week.fill(left) - 0.541).abs() < 1e-6);
        assert_eq!(week.number(used), "45%");
        assert_eq!(week.number(left), "55%");
        let five = reading_for(&snap, &usage("anthropic:subscription", "five_hour")).unwrap();
        assert_eq!(five.fill(used), 0.0);
        assert_eq!(five.fill(left), 0.0);
        assert_eq!(five.number(left), "--");
        assert!(reading_for(&snap, &usage("openai:subscription", "seven_day")).is_none());
        assert!(reading_for(&snap, &TraySource::All).is_none());
        assert!(usage_item_line("A7D", Some(&week), used)
            .starts_with("A7D  Claude Max 20x · Weekly 45%"));
        assert!(usage_item_line("A7D", Some(&week), left)
            .starts_with("A7D  Claude Max 20x · Weekly 55% left"));
        assert_eq!(usage_item_line("O7D", None, left), "O7D  no data yet");
    }

    #[test]
    fn hidden_and_disabled_have_no_reading() {
        let mut a = account(
            "anthropic:subscription",
            AccountKind::Subscription,
            vec![window("seven_day", 10.0, false)],
        );
        a.hidden = true;
        let snap = snapshot(vec![a]);
        assert!(reading_for(&snap, &usage("anthropic:subscription", "seven_day")).is_none());
        let mut off = snapshot(vec![account(
            "anthropic:subscription",
            AccountKind::Subscription,
            vec![window("seven_day", 10.0, false)],
        )]);
        off.enabled = false;
        assert!(reading_for(&off, &usage("anthropic:subscription", "seven_day")).is_none());
    }

    #[test]
    fn pending_items_are_headline_windows_of_new_subscriptions() {
        let snap = snapshot(vec![
            account(
                "anthropic:subscription",
                AccountKind::Subscription,
                vec![
                    window("five_hour", 1.0, false),
                    window("seven_day", 2.0, false),
                ],
            ),
            account(
                "github-copilot:subscription",
                AccountKind::Subscription,
                vec![window("monthly", 3.0, false)],
            ),
            account(
                "openai:api",
                AccountKind::Api,
                vec![window("seven_day", 1.0, false)],
            ),
            account("groq:subscription", AccountKind::Subscription, vec![]),
        ]);
        assert_eq!(
            pending_tray_items(&snap, &[]),
            vec![
                (
                    "anthropic:subscription".to_string(),
                    "seven_day".to_string()
                ),
                (
                    "github-copilot:subscription".to_string(),
                    "monthly".to_string()
                ),
            ]
        );
        let added = vec!["anthropic:subscription|seven_day".to_string()];
        assert_eq!(
            pending_tray_items(&snap, &added),
            vec![(
                "github-copilot:subscription".to_string(),
                "monthly".to_string()
            )]
        );
    }

    #[test]
    fn percent_rounds_down() {
        assert_eq!(format_percent(99.99), "99%");
        assert_eq!(format_percent(-1.0), "0%");
        assert_eq!(format_percent(100.0), "100%");
        // Remaining is the complement of the rounded-down used figure
        assert_eq!(format_remaining(99.99), "1%");
        assert_eq!(format_remaining(0.0), "100%");
        assert_eq!(format_remaining(45.9), "55%");
        assert_eq!(format_remaining(120.0), "0%");
    }
}
