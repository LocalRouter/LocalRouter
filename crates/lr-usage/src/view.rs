//! What the Dashboard and menu bar show: stored readings plus derived pace,
//! projections, usage shape and money estimates.

use lr_config::UsageTrackingConfig;
use serde::{Deserialize, Serialize};

use crate::plans::{plan_info, provider_label, subscription_label};
use crate::tracker::{AccountState, LedgerEntry, PastWindow, State, StoredWindow};
use crate::types::{AccountKind, CreditsReading, DataSource};

/// Average month length, for prorating monthly prices.
const MONTH_SECS: f64 = 30.44 * 86_400.0;
/// Before this share of a window has elapsed, projections are too noisy.
const MIN_PROJECTION_ELAPSED: f64 = 0.02;
/// Projected end-of-window usage at or above this is a warning…
const PACE_WARNING_PERCENT: f64 = 75.0;
/// …and above this the limit will be hit before the reset.
const PACE_OVER_PERCENT: f64 = 105.0;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UsageSnapshot {
    pub enabled: bool,
    pub generated_at: i64,
    pub accounts: Vec<UsageAccountView>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaceStatus {
    /// On course to stay well under the limit.
    Ok,
    /// On course to end near the limit.
    Warning,
    /// On course to hit the limit before the reset (or already did).
    Over,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UsageWindowView {
    pub id: String,
    pub label: String,
    pub used_percent: f64,
    pub resets_at: Option<i64>,
    pub window_secs: Option<i64>,
    pub updated_at: i64,
    pub source: DataSource,
    /// The reset time has passed with no newer reading: the figure belongs
    /// to the window that ended.
    pub stale: bool,
    /// Share of the window elapsed (0–1): where even pace would be now.
    pub elapsed_fraction: Option<f64>,
    /// Usage at the reset if the pace so far continues.
    pub projected_percent: Option<f64>,
    /// When the limit is reached at the current pace, if before the reset.
    pub limit_eta: Option<i64>,
    pub pace: PaceStatus,
    /// Usage added per slot (per hour for 5h windows, per day for weekly),
    /// in percentage points. Slots after `current_slot` are in the future.
    pub slots: Vec<f64>,
    pub current_slot: usize,
    pub slot_secs: Option<i64>,
    /// API-list-price cost of the traffic LocalRouter saw in this window.
    pub api_equivalent_usd: Option<f64>,
    /// The part of the monthly price this window's usage accounts for.
    pub plan_share_usd: Option<f64>,
    /// Peak usage of recently ended windows, newest last.
    pub history: Vec<PastWindow>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UsageQuotaView {
    pub id: String,
    pub label: String,
    pub limit: Option<f64>,
    pub remaining: Option<f64>,
    pub used_percent: Option<f64>,
    pub resets_at: Option<i64>,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct UsageSpendView {
    pub last_24h_usd: f64,
    pub last_7d_usd: f64,
    pub last_30d_usd: f64,
    pub month_to_date_usd: f64,
    pub requests_30d: u64,
    pub tokens_30d: u64,
    /// API-equivalent cost per day, oldest first, ending today (UTC).
    pub daily_usd: Vec<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UsageAccountView {
    pub id: String,
    pub provider: String,
    pub provider_label: String,
    pub kind: AccountKind,
    /// e.g. "Claude Max 20x" or "OpenAI API".
    pub title: String,
    pub plan: Option<String>,
    pub plan_label: Option<String>,
    pub monthly_price_usd: Option<f64>,
    /// Plan or price come from the user's override.
    pub plan_overridden: bool,
    pub status: Option<String>,
    pub sources: Vec<DataSource>,
    pub first_seen: i64,
    pub last_seen: i64,
    pub hidden: bool,
    pub windows: Vec<UsageWindowView>,
    pub quotas: Vec<UsageQuotaView>,
    pub credits: Option<CreditsReading>,
    pub spend: UsageSpendView,
    /// API-equivalent cost of the last 30 days divided by the monthly price.
    pub value_multiplier: Option<f64>,
}

pub(crate) fn build_snapshot(
    state: &State,
    config: &UsageTrackingConfig,
    enabled: bool,
    now: i64,
) -> UsageSnapshot {
    let mut accounts: Vec<UsageAccountView> = state
        .accounts
        .iter()
        .map(|(id, acct)| account_view(id, acct, config, now))
        .collect();
    // Subscriptions first, then the busiest accounts.
    accounts.sort_by(|a, b| {
        (a.kind != AccountKind::Subscription)
            .cmp(&(b.kind != AccountKind::Subscription))
            .then(b.spend.last_30d_usd.total_cmp(&a.spend.last_30d_usd))
            .then(a.id.cmp(&b.id))
    });
    UsageSnapshot {
        enabled,
        generated_at: now,
        accounts,
    }
}

fn account_view(
    id: &str,
    acct: &AccountState,
    config: &UsageTrackingConfig,
    now: i64,
) -> UsageAccountView {
    let over = config.plans.get(id);
    let inferred = acct
        .plan
        .as_deref()
        .and_then(|p| plan_info(&acct.provider, p));
    let plan_label = over
        .and_then(|o| o.plan.clone())
        .or_else(|| inferred.as_ref().map(|p| p.label.clone()))
        .or_else(|| acct.plan.clone());
    let monthly_price_usd = over
        .and_then(|o| o.monthly_price_usd)
        .or_else(|| inferred.as_ref().and_then(|p| p.monthly_price_usd));
    let plan_overridden = over.is_some_and(|o| o.plan.is_some() || o.monthly_price_usd.is_some());

    let title = match acct.kind {
        AccountKind::Subscription => {
            let product = subscription_label(&acct.provider);
            match &plan_label {
                Some(p) => format!("{product} {p}"),
                None => format!("{product} subscription"),
            }
        }
        AccountKind::Api => format!("{} API", provider_label(&acct.provider)),
    };

    let spend = spend_view(&acct.ledger, now);
    let windows: Vec<UsageWindowView> = acct
        .windows
        .values()
        .map(|w| window_view(w, &acct.ledger, monthly_price_usd, now))
        .collect();
    let windows = order_windows(windows);
    let quotas = acct
        .quotas
        .values()
        .map(|q| {
            let r = &q.reading;
            UsageQuotaView {
                id: r.id.clone(),
                label: quota_label(&r.id),
                limit: r.limit,
                remaining: r.remaining,
                used_percent: match (r.limit, r.remaining) {
                    (Some(l), Some(rem)) if l > 0.0 => Some(((l - rem) / l * 100.0).max(0.0)),
                    _ => None,
                },
                resets_at: r.resets_at,
                updated_at: q.updated_at,
            }
        })
        .collect();
    let value_multiplier = monthly_price_usd
        .filter(|p| *p > 0.0 && acct.kind == AccountKind::Subscription)
        .map(|p| spend.last_30d_usd / p);

    UsageAccountView {
        id: id.to_string(),
        provider: acct.provider.clone(),
        provider_label: provider_label(&acct.provider),
        kind: acct.kind,
        title,
        plan: acct.plan.clone(),
        plan_label,
        monthly_price_usd,
        plan_overridden,
        status: acct.status.as_ref().map(|(s, _)| s.clone()),
        sources: acct.sources.keys().copied().collect(),
        first_seen: acct.first_seen,
        last_seen: acct.last_seen,
        hidden: config.hidden_accounts.iter().any(|h| h == id),
        windows,
        quotas,
        credits: acct.credits.as_ref().map(|(c, _)| c.clone()),
        spend,
        value_multiplier,
    }
}

/// Session first, then weekly, then scoped and other windows.
fn order_windows(mut windows: Vec<UsageWindowView>) -> Vec<UsageWindowView> {
    let rank = |id: &str| match id {
        "five_hour" => 0,
        "seven_day" => 1,
        "monthly" => 2,
        id if id.starts_with("seven_day_") => 3,
        _ => 4,
    };
    windows.sort_by(|a, b| rank(&a.id).cmp(&rank(&b.id)).then(a.id.cmp(&b.id)));
    windows
}

fn quota_label(id: &str) -> String {
    match id {
        "requests" => "Requests".to_string(),
        "tokens" => "Tokens".to_string(),
        "input_tokens" => "Input tokens".to_string(),
        "output_tokens" => "Output tokens".to_string(),
        "requests_day" => "Requests / day".to_string(),
        "tokens_minute" => "Tokens / minute".to_string(),
        "project_tokens" => "Project tokens".to_string(),
        "premium_interactions" => "Premium requests".to_string(),
        "rate" => "Rate limit".to_string(),
        other => crate::types::window_label(other),
    }
}

/// Slot count for a window: per hour up to a day, per day beyond.
fn slot_layout(window_secs: i64) -> (usize, i64) {
    if window_secs <= 86_400 {
        let n = (window_secs / 3_600).clamp(1, 24) as usize;
        (n, window_secs / n as i64)
    } else {
        let n = (window_secs / 86_400).clamp(1, 31) as usize;
        (n, window_secs / n as i64)
    }
}

fn window_view(
    w: &StoredWindow,
    ledger: &std::collections::BTreeMap<i64, LedgerEntry>,
    monthly_price_usd: Option<f64>,
    now: i64,
) -> UsageWindowView {
    let r = &w.reading;
    let used = r.used_percent;
    let stale = r.resets_at.is_some_and(|t| t <= now);
    let start = match (r.resets_at, r.window_secs) {
        (Some(reset), Some(len)) if len > 0 => Some(reset - len),
        _ => None,
    };

    let mut elapsed_fraction = None;
    let mut projected_percent = None;
    let mut limit_eta = None;
    let mut pace = if used >= 100.0 {
        PaceStatus::Over
    } else {
        PaceStatus::Ok
    };
    if let (Some(start), Some(reset), false) = (start, r.resets_at, stale) {
        let len = (reset - start) as f64;
        let elapsed = (now - start).clamp(0, reset - start) as f64;
        let frac = elapsed / len;
        elapsed_fraction = Some(frac);
        if frac >= MIN_PROJECTION_ELAPSED {
            let projected = used / frac;
            projected_percent = Some(projected);
            if used < 100.0 && used > 0.0 {
                let rate = used / elapsed; // points per second
                let eta = now + ((100.0 - used) / rate).round() as i64;
                if eta < reset {
                    limit_eta = Some(eta);
                }
            }
            if used < 100.0 {
                pace = if projected > PACE_OVER_PERCENT {
                    PaceStatus::Over
                } else if projected >= PACE_WARNING_PERCENT {
                    PaceStatus::Warning
                } else {
                    PaceStatus::Ok
                };
            }
        }
    }

    let (slots, current_slot, slot_secs) = match (start, r.window_secs) {
        (Some(start), Some(len)) if !stale => {
            let (n, slot_len) = slot_layout(len);
            let current = (((now - start).max(0)) / slot_len).min(n as i64 - 1) as usize;
            (
                usage_slots(w, ledger, start, slot_len, n, used, now),
                current,
                Some(slot_len),
            )
        }
        _ => (Vec::new(), 0, None),
    };

    let api_equivalent_usd = start.filter(|_| !stale).map(|s| {
        ledger
            .range(s - s.rem_euclid(crate::tracker::LEDGER_BUCKET_SECS)..)
            .filter(|(b, _)| **b + crate::tracker::LEDGER_BUCKET_SECS > s)
            .map(|(_, e)| e.cost_usd)
            .sum()
    });
    let plan_share_usd = match (monthly_price_usd, r.window_secs) {
        (Some(price), Some(len)) if !stale => {
            Some(price * (len as f64 / MONTH_SECS) * used / 100.0)
        }
        _ => None,
    };

    UsageWindowView {
        id: r.id.clone(),
        label: r.label.clone(),
        used_percent: used,
        resets_at: r.resets_at,
        window_secs: r.window_secs,
        updated_at: w.updated_at,
        source: w.source,
        stale,
        elapsed_fraction,
        projected_percent,
        limit_eta,
        pace,
        slots,
        current_slot,
        slot_secs,
        api_equivalent_usd,
        plan_share_usd,
        history: w.history.clone(),
    }
}

/// Per-slot usage in percentage points.
///
/// Shaped by the API-equivalent cost of the traffic LocalRouter saw (cost
/// weights cache reads and output tokens the way limits roughly do), scaled
/// so the slots add up to the reported usage. Without traffic in the window
/// the rises between successive readings are used instead.
fn usage_slots(
    w: &StoredWindow,
    ledger: &std::collections::BTreeMap<i64, LedgerEntry>,
    start: i64,
    slot_len: i64,
    n: usize,
    used: f64,
    now: i64,
) -> Vec<f64> {
    let slot_of = |t: i64| (((t - start).max(0)) / slot_len).min(n as i64 - 1) as usize;
    let mut cost = vec![0.0; n];
    for (bucket, e) in ledger.range(start..=now) {
        cost[slot_of(*bucket)] += e.cost_usd;
    }
    let total: f64 = cost.iter().sum();
    if total > 0.0 {
        return cost.into_iter().map(|c| c / total * used).collect();
    }
    let mut slots = vec![0.0; n];
    let mut prev: Option<f64> = None;
    for (t, p) in &w.samples {
        if *t < start {
            continue;
        }
        if let Some(prev) = prev {
            slots[slot_of(*t)] += (p - prev).max(0.0);
        }
        prev = Some(*p);
    }
    slots
}

fn spend_view(ledger: &std::collections::BTreeMap<i64, LedgerEntry>, now: i64) -> UsageSpendView {
    let mut v = UsageSpendView::default();
    let today = now - now.rem_euclid(86_400);
    let month_start = chrono::DateTime::from_timestamp(now, 0)
        .and_then(|d| {
            use chrono::Datelike;
            d.date_naive().with_day(1)
        })
        .and_then(|d| d.and_hms_opt(0, 0, 0))
        .map(|d| d.and_utc().timestamp())
        .unwrap_or(today);
    let mut daily = vec![0.0; 30];
    for (bucket, e) in ledger {
        let age = now - bucket;
        if age < 86_400 {
            v.last_24h_usd += e.cost_usd;
        }
        if age < 7 * 86_400 {
            v.last_7d_usd += e.cost_usd;
        }
        if age < 30 * 86_400 {
            v.last_30d_usd += e.cost_usd;
            v.requests_30d += e.requests;
            v.tokens_30d += e.total_tokens();
        }
        if *bucket >= month_start {
            v.month_to_date_usd += e.cost_usd;
        }
        let days_ago = (today - (bucket - bucket.rem_euclid(86_400))) / 86_400;
        if (0..30).contains(&days_ago) {
            daily[29 - days_ago as usize] += e.cost_usd;
        }
    }
    v.daily_usd = daily;
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tracker::UsageTracker;
    use crate::types::{AccountRef, UsageReport, WindowReading};
    use lr_config::UsagePlanOverride;

    const WEEK: i64 = 604_800;

    fn report(used: f64, reset: i64) -> UsageReport {
        UsageReport {
            plan: Some("default_claude_max_20x".into()),
            windows: vec![WindowReading {
                id: "seven_day".into(),
                label: "Weekly".into(),
                used_percent: used,
                resets_at: Some(reset),
                window_secs: Some(WEEK),
            }],
            ..Default::default()
        }
    }

    #[test]
    fn projection_pace_and_plan_value() {
        let t = UsageTracker::new(UsageTrackingConfig::default());
        let start = 1_000 * 86_400;
        let reset = start + WEEK;
        let now = start + WEEK / 2; // half-way
        let claude = AccountRef::subscription("anthropic");
        t.apply_at(&claude, &report(45.0, reset), DataSource::CliLogin, now);
        let snap = t.snapshot_at(now);
        let a = &snap.accounts[0];
        assert_eq!(a.title, "Claude Max 20x");
        assert_eq!(a.monthly_price_usd, Some(200.0));
        let w = &a.windows[0];
        assert!((w.elapsed_fraction.unwrap() - 0.5).abs() < 1e-9);
        assert!((w.projected_percent.unwrap() - 90.0).abs() < 1e-9);
        assert_eq!(w.pace, PaceStatus::Warning);
        assert_eq!(w.limit_eta, None, "90% at reset never hits the limit");
        assert_eq!(w.slots.len(), 7);
        assert_eq!(w.current_slot, 3);
        let share = w.plan_share_usd.unwrap();
        assert!((share - 200.0 * (WEEK as f64 / MONTH_SECS) * 0.45).abs() < 1e-9);
    }

    #[test]
    fn limit_eta_when_over_pace() {
        let t = UsageTracker::new(UsageTrackingConfig::default());
        let start = 1_000 * 86_400;
        let now = start + WEEK / 4;
        let claude = AccountRef::subscription("anthropic");
        t.apply_at(
            &claude,
            &report(50.0, start + WEEK),
            DataSource::CliLogin,
            now,
        );
        let w = &t.snapshot_at(now).accounts[0].windows[0];
        assert_eq!(w.pace, PaceStatus::Over);
        // 50 points in a quarter week → 100 at half a week.
        assert_eq!(w.limit_eta, Some(start + WEEK / 2));
    }

    #[test]
    fn stale_window_after_reset() {
        let t = UsageTracker::new(UsageTrackingConfig::default());
        let claude = AccountRef::subscription("anthropic");
        t.apply_at(
            &claude,
            &report(99.0, 5_000_000),
            DataSource::CliLogin,
            4_999_000,
        );
        let w = &t.snapshot_at(5_000_100).accounts[0].windows[0];
        assert!(w.stale);
        assert!(w.projected_percent.is_none());
        assert!(w.slots.is_empty());
    }

    #[test]
    fn slots_follow_ledger_cost_scaled_to_usage() {
        let t = UsageTracker::new(UsageTrackingConfig::default());
        let start = 1_000 * 86_400;
        let now = start + 2 * 86_400 + 100;
        let claude = AccountRef::subscription("anthropic");
        let e = |cost| LedgerEntry {
            requests: 1,
            cost_usd: cost,
            ..Default::default()
        };
        t.record_request_at(&claude, e(1.0), start + 10);
        t.record_request_at(&claude, e(3.0), start + 86_400 + 10);
        t.apply_at(
            &claude,
            &report(40.0, start + WEEK),
            DataSource::ProxyHeaders,
            now,
        );
        let snap = t.snapshot_at(now);
        let a = &snap.accounts[0];
        let w = &a.windows[0];
        assert!((w.slots[0] - 10.0).abs() < 1e-9);
        assert!((w.slots[1] - 30.0).abs() < 1e-9);
        assert!((w.api_equivalent_usd.unwrap() - 4.0).abs() < 1e-9);
        assert!((a.spend.last_7d_usd - 4.0).abs() < 1e-9);
        assert!((a.value_multiplier.unwrap() - 4.0 / 200.0).abs() < 1e-9);
    }

    #[test]
    fn slots_from_samples_without_traffic() {
        let t = UsageTracker::new(UsageTrackingConfig::default());
        let start = 1_000 * 86_400;
        let claude = AccountRef::subscription("anthropic");
        t.apply_at(
            &claude,
            &report(10.0, start + WEEK),
            DataSource::CliLogin,
            start + 100,
        );
        t.apply_at(
            &claude,
            &report(25.0, start + WEEK),
            DataSource::CliLogin,
            start + 86_500,
        );
        let now = start + 86_600;
        let w = &t.snapshot_at(now).accounts[0].windows[0];
        assert_eq!(w.slots[0], 0.0);
        assert!((w.slots[1] - 15.0).abs() < 1e-9);
    }

    #[test]
    fn plan_override_and_hidden() {
        let mut config = UsageTrackingConfig::default();
        config.plans.insert(
            "openai:subscription".into(),
            UsagePlanOverride {
                plan: Some("Pro".into()),
                monthly_price_usd: Some(180.0),
            },
        );
        config.hidden_accounts.push("openai:subscription".into());
        let t = UsageTracker::new(config);
        let openai = AccountRef::subscription("openai");
        t.apply_at(
            &openai,
            &UsageReport {
                plan: Some("plus".into()),
                status: Some("allowed".into()),
                ..Default::default()
            },
            DataSource::ProxyHeaders,
            0,
        );
        let a = &t.snapshot_at(0).accounts[0];
        assert_eq!(a.title, "ChatGPT Pro");
        assert_eq!(a.monthly_price_usd, Some(180.0));
        assert!(a.plan_overridden);
        assert!(a.hidden);
    }

    #[test]
    fn spend_daily_buckets() {
        let mut ledger = std::collections::BTreeMap::new();
        let now = 2_000 * 86_400 + 3_600;
        ledger.insert(
            now - 3_600,
            LedgerEntry {
                cost_usd: 2.0,
                requests: 1,
                ..Default::default()
            },
        );
        ledger.insert(
            now - 86_400,
            LedgerEntry {
                cost_usd: 1.0,
                requests: 1,
                ..Default::default()
            },
        );
        let v = spend_view(&ledger, now);
        assert_eq!(v.daily_usd.len(), 30);
        assert_eq!(v.daily_usd[29], 2.0);
        assert_eq!(v.daily_usd[28], 1.0);
        assert_eq!(v.last_24h_usd, 2.0);
        assert_eq!(v.requests_30d, 2);
    }

    #[test]
    fn quota_used_percent() {
        let t = UsageTracker::new(UsageTrackingConfig::default());
        let groq = AccountRef::api("groq");
        t.apply_at(
            &groq,
            &UsageReport {
                quotas: vec![crate::types::QuotaReading {
                    id: "requests_day".into(),
                    limit: Some(1000.0),
                    remaining: Some(750.0),
                    resets_at: None,
                }],
                ..Default::default()
            },
            DataSource::GatewayHeaders,
            0,
        );
        let a = &t.snapshot_at(0).accounts[0];
        assert_eq!(a.title, "Groq API");
        assert_eq!(a.quotas[0].used_percent, Some(25.0));
        assert_eq!(a.quotas[0].label, "Requests / day");
    }
}
