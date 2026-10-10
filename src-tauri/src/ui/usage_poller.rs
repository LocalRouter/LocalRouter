//! Background usage polling and change fan-out for the usage tracker.
//!
//! Passive readings (proxy / gateway headers) need no polling; this asks
//! usage endpoints directly, only as the settings allow, and only as often as
//! traffic warrants: every `poll_interval_secs` (5 min) while requests for the
//! account keep coming in, every `idle_poll_interval_secs` (1 h) otherwise. A
//! fresh passive reading answers the poll. Failures back off exponentially
//! (5 → 10 → 20 min, capped at 1 h, with jitter), honouring `Retry-After`.
//!
//! Sources:
//! - "Ask connected providers" — ChatGPT Plus/Pro (LocalRouter's OAuth
//!   login), GitHub Copilot, and providers with a credits API (OpenRouter).
//! - "Use Claude Code and Codex logins" (off by default) — the CLIs' saved
//!   logins, for their subscription usage.
//!
//! It also forwards tracker changes to the UI (`usage-limits-changed`) and
//! the tray (usage items redraw; newly seen subscriptions are added to the
//! tray stats items).

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use lr_providers::registry::ProviderRegistry;
use lr_usage::fetch::{self, FetchError};
use lr_usage::{AccountKind, AccountRef, CreditsReading, DataSource, UsageReport, UsageTracker};
use parking_lot::{Mutex, RwLock};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::Notify;

/// Requests for an account within this long make it "active".
const ACTIVE_WINDOW_SECS: i64 = 10 * 60;
/// First back-off after a failure; doubles per consecutive failure.
const BACKOFF_BASE_SECS: i64 = 5 * 60;
const BACKOFF_CAP_SECS: i64 = 60 * 60;
/// How often due sources are checked.
const SCHEDULER_TICK_SECS: u64 = 30;

/// State of one polled usage source, for Settings.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct UsagePollStatus {
    /// e.g. `claude_code_login`, `chatgpt_oauth`, `credits:openrouter`.
    pub id: String,
    pub label: String,
    pub account_id: String,
    pub last_attempt: Option<i64>,
    pub last_success: Option<i64>,
    pub last_error: Option<String>,
    /// Not polled again before this time (unix secs) after an error.
    pub retry_after: Option<i64>,
}

pub struct UsagePoller {
    tracker: Arc<UsageTracker>,
    registry: Arc<ProviderRegistry>,
    client: reqwest::Client,
    wake: Notify,
    statuses: RwLock<BTreeMap<String, UsagePollStatus>>,
    schedules: Mutex<HashMap<String, Schedule>>,
    tray_sync_running: AtomicBool,
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

impl UsagePoller {
    pub fn new(tracker: Arc<UsageTracker>, registry: Arc<ProviderRegistry>) -> Arc<Self> {
        Arc::new(Self {
            tracker,
            registry,
            client: fetch::client(),
            wake: Notify::new(),
            statuses: RwLock::new(BTreeMap::new()),
            schedules: Mutex::new(HashMap::new()),
            tray_sync_running: AtomicBool::new(false),
        })
    }

    /// Wire change notifications and start the poll loop.
    pub fn start(self: &Arc<Self>, app: AppHandle) {
        let weak = Arc::downgrade(self);
        let app_for_change = app.clone();
        self.tracker.set_on_change(move || {
            let _ = app_for_change.emit("usage-limits-changed", ());
            if let Some(tray) =
                app_for_change.try_state::<Arc<crate::ui::tray_graph_manager::TrayGraphManager>>()
            {
                tray.usage_limits_changed();
            }
            if let Some(poller) = weak.upgrade() {
                poller.schedule_tray_sync(&app_for_change);
            }
        });
        self.tracker.spawn_maintenance();
        self.schedule_tray_sync(&app);

        let poller = self.clone();
        tokio::spawn(async move {
            loop {
                poller.poll_once().await;
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(SCHEDULER_TICK_SECS)) => {}
                    _ = poller.wake.notified() => {}
                }
            }
        });
    }

    /// Add newly seen subscriptions to the tray stats items (one sync at a
    /// time; changes during a sync are picked up by the next change).
    fn schedule_tray_sync(self: &Arc<Self>, app: &AppHandle) {
        if self.tray_sync_running.swap(true, Ordering::AcqRel) {
            return;
        }
        let poller = self.clone();
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            crate::ui::tray_usage::sync_tray_items(&app).await;
            poller.tray_sync_running.store(false, Ordering::Release);
        });
    }

    /// Poll every source now, bypassing intervals and back-offs.
    pub fn refresh_now(&self) {
        self.schedules.lock().clear();
        self.wake.notify_one();
    }

    pub fn statuses(&self) -> Vec<UsagePollStatus> {
        self.statuses.read().values().cloned().collect()
    }

    /// Poll every allowed source that is due.
    pub async fn poll_once(&self) {
        let config = self.tracker.config();
        if !config.enabled {
            return;
        }
        // Drop statuses of sources that are no longer polled.
        let mut active: Vec<String> = Vec::new();

        if config.poll_provider_apis {
            let instances: Vec<_> = self
                .registry
                .list_providers()
                .into_iter()
                .filter(|i| i.enabled)
                .collect();
            if instances
                .iter()
                .any(|i| i.provider_type == "openai-chatgpt-plus")
            {
                active.push("chatgpt_oauth".into());
                self.poll_chatgpt_oauth().await;
            }
            if instances
                .iter()
                .any(|i| i.provider_type == "github-copilot")
            {
                active.push("github_copilot".into());
                self.poll_copilot().await;
            }
            for inst in &instances {
                let Some(account) = lr_usage::account_for_provider_type(&inst.provider_type)
                    .filter(|a| a.kind == AccountKind::Api)
                else {
                    continue;
                };
                let id = format!("credits:{}", inst.instance_name);
                if self
                    .poll_credits(&id, &inst.instance_name, &inst.provider_name, &account)
                    .await
                {
                    active.push(id);
                }
            }
        }
        if config.read_cli_logins {
            active.push("claude_code_login".into());
            self.poll_claude_code().await;
            active.push("codex_login".into());
            self.poll_codex_cli().await;
        }
        self.statuses.write().retain(|id, _| active.contains(id));
    }

    /// Whether source `id` (feeding `account`) should be polled now.
    fn due(&self, id: &str, account: &AccountRef) -> bool {
        let config = self.tracker.config();
        let timing = Timing {
            active_secs: config.poll_interval_secs.clamp(60, 3_600) as i64,
            idle_secs: config.idle_poll_interval_secs.clamp(300, 86_400) as i64,
        };
        let account_id = account.id();
        let schedule = self.schedules.lock().get(id).cloned().unwrap_or_default();
        is_due(
            &schedule,
            now(),
            self.tracker.last_traffic(&account_id),
            self.tracker.last_passive_reading(&account_id),
            &timing,
        )
    }

    /// Record an attempt's outcome for `id`.
    fn record(
        &self,
        id: &str,
        label: &str,
        account: &AccountRef,
        result: Result<(), String>,
        backoff_secs: Option<i64>,
    ) {
        let t = now();
        let retry_after = backoff_secs.map(|s| t + s);
        {
            let mut schedules = self.schedules.lock();
            let schedule = schedules.entry(id.to_string()).or_default();
            schedule.last_poll = Some(t);
            schedule.retry_at = retry_after;
        }
        let mut statuses = self.statuses.write();
        let s = statuses
            .entry(id.to_string())
            .or_insert_with(|| UsagePollStatus {
                id: id.to_string(),
                label: label.to_string(),
                account_id: account.id(),
                last_attempt: None,
                last_success: None,
                last_error: None,
                retry_after: None,
            });
        s.last_attempt = Some(t);
        s.retry_after = retry_after;
        match result {
            Ok(()) => {
                s.last_success = Some(t);
                s.last_error = None;
            }
            Err(e) => s.last_error = Some(e),
        }
    }

    /// Push `id`'s next attempt back by the failure back-off without
    /// recording a status row.
    fn defer_after_failure(&self, id: &str) {
        let mut schedules = self.schedules.lock();
        defer(
            schedules.entry(id.to_string()).or_default(),
            now(),
            jitter(),
        );
    }

    /// Apply a fetch result, recording status and back-off.
    fn finish(
        &self,
        id: &str,
        label: &str,
        account: &AccountRef,
        result: Result<UsageReport, FetchError>,
        source: DataSource,
    ) {
        match result {
            Ok(report) => {
                self.tracker.apply(account, &report, source);
                if let Some(schedule) = self.schedules.lock().get_mut(id) {
                    schedule.failures = 0;
                }
                self.record(id, label, account, Ok(()), None);
            }
            Err(e) => {
                let failures = {
                    let mut schedules = self.schedules.lock();
                    let schedule = schedules.entry(id.to_string()).or_default();
                    schedule.failures += 1;
                    schedule.failures
                };
                let retry_after = match &e {
                    FetchError::RateLimited(s) => *s,
                    _ => None,
                };
                let backoff = backoff_secs(failures, retry_after, jitter());
                tracing::debug!("Usage poll {} failed ({}); retrying in {}s", id, e, backoff);
                self.record(id, label, account, Err(e.to_string()), Some(backoff));
            }
        }
    }

    async fn poll_chatgpt_oauth(&self) {
        let id = "chatgpt_oauth";
        let label = "ChatGPT Plus/Pro (LocalRouter login)";
        let account = AccountRef::subscription("openai");
        if !self.due(id, &account) {
            return;
        }
        let Some(source) = lr_providers::oauth::token_source("openai-codex") else {
            return;
        };
        let token = match source.access_token().await {
            Ok(t) => t,
            Err(e) => {
                self.record(id, label, &account, Err(format!("no login: {e}")), None);
                return;
            }
        };
        let account_id = lr_usage::classify::chatgpt_account_from_jwt(&token);
        let mut result =
            fetch::fetch_codex_usage(&self.client, &token, account_id.as_deref()).await;
        if let Ok(report) = &mut result {
            if report.plan.is_none() {
                report.plan = lr_usage::classify::chatgpt_plan_from_jwt(&token);
            }
        }
        self.finish(id, label, &account, result, DataSource::ProviderApi);
    }

    async fn poll_copilot(&self) {
        let id = "github_copilot";
        let label = "GitHub Copilot";
        let account = AccountRef::subscription("github-copilot");
        if !self.due(id, &account) {
            return;
        }
        let token = tokio::task::spawn_blocking(|| {
            use lr_api_keys::{keychain_trait::KeychainStorage, CachedKeychain};
            let keychain = CachedKeychain::auto().unwrap_or_else(|_| CachedKeychain::system());
            keychain
                .get("LocalRouter-ProviderTokens", "github-copilot_access_token")
                .ok()
                .flatten()
        })
        .await
        .ok()
        .flatten();
        let Some(token) = token else {
            self.record(id, label, &account, Err("no login".into()), None);
            return;
        };
        let result = fetch::fetch_copilot_usage(&self.client, &token).await;
        self.finish(id, label, &account, result, DataSource::ProviderApi);
    }

    /// Credits / key limit via the provider's own API. Returns whether the
    /// provider reports credits at all (so it gets a status row).
    async fn poll_credits(
        &self,
        id: &str,
        instance: &str,
        provider_name: &str,
        account: &AccountRef,
    ) -> bool {
        if !self.due(id, account) {
            return true;
        }
        let Some(provider) = self.registry.get_provider(instance) else {
            return false;
        };
        let Some(info) = provider.check_credits().await else {
            // Either the provider reports no credits (no request is made) or
            // the request failed. Wait like a failed poll either way, so a
            // failing endpoint is not asked again on every tick.
            self.defer_after_failure(id);
            return false;
        };
        let has_limit = info.total_credits_usd.is_some_and(|t| t > 0.0);
        let report = UsageReport {
            credits: Some(CreditsReading {
                label: if has_limit { "Key limit" } else { "Spend" }.to_string(),
                balance_usd: info.remaining_credits_usd.filter(|_| has_limit),
                limit_usd: info.total_credits_usd.filter(|_| has_limit),
                used_usd: info.used_credits_usd,
                unlimited: !has_limit,
                currency: Some("USD".to_string()),
            }),
            ..Default::default()
        };
        self.finish(
            id,
            &format!("{provider_name} credits"),
            account,
            Ok(report),
            DataSource::ProviderApi,
        );
        true
    }

    async fn poll_claude_code(&self) {
        let id = "claude_code_login";
        let label = "Claude Code login";
        let account = AccountRef::subscription("anthropic");
        if !self.due(id, &account) {
            return;
        }
        let login = tokio::task::spawn_blocking(lr_usage::cli_logins::read_claude_code_login)
            .await
            .ok()
            .flatten();
        let Some(login) = login else {
            self.record(id, label, &account, Err("not logged in".into()), None);
            return;
        };
        if login.is_expired(chrono::Utc::now().timestamp_millis()) {
            // Claude Code refreshes its own login when it next runs.
            self.record(
                id,
                label,
                &account,
                Err("login expired — run Claude Code to refresh it".into()),
                None,
            );
            return;
        }
        let mut result = fetch::fetch_claude_usage(&self.client, &login.access_token).await;
        if let Ok(report) = &mut result {
            report.plan = report.plan.take().or(login.plan.clone());
        }
        self.finish(id, label, &account, result, DataSource::CliLogin);
    }

    async fn poll_codex_cli(&self) {
        let id = "codex_login";
        let label = "Codex login";
        let account = AccountRef::subscription("openai");
        if !self.due(id, &account) {
            return;
        }
        let login = tokio::task::spawn_blocking(lr_usage::cli_logins::read_codex_login)
            .await
            .ok()
            .flatten();
        let Some(login) = login else {
            self.record(
                id,
                label,
                &account,
                Err("not logged in with ChatGPT".into()),
                None,
            );
            return;
        };
        let account_id = login
            .account_id
            .clone()
            .or_else(|| lr_usage::classify::chatgpt_account_from_jwt(&login.access_token));
        let mut result =
            fetch::fetch_codex_usage(&self.client, &login.access_token, account_id.as_deref())
                .await;
        if let Ok(report) = &mut result {
            if report.plan.is_none() {
                report.plan = lr_usage::classify::chatgpt_plan_from_jwt(&login.access_token);
            }
        }
        self.finish(id, label, &account, result, DataSource::CliLogin);
    }
}

/// Poll bookkeeping for one source.
#[derive(Debug, Clone, Default, PartialEq)]
struct Schedule {
    last_poll: Option<i64>,
    /// Consecutive failures (reset by a success).
    failures: u32,
    /// Not before this time after a failure.
    retry_at: Option<i64>,
}

/// Count a failed attempt at `now` and schedule the next one after the
/// failure back-off.
fn defer(schedule: &mut Schedule, now: i64, jitter: f64) {
    schedule.failures += 1;
    schedule.last_poll = Some(now);
    schedule.retry_at = Some(now + backoff_secs(schedule.failures, None, jitter));
}

struct Timing {
    active_secs: i64,
    idle_secs: i64,
}

/// Whether a source is due, given when it was last polled and when its
/// account last saw traffic and a passive reading.
fn is_due(
    schedule: &Schedule,
    now: i64,
    last_traffic: Option<i64>,
    last_passive: Option<i64>,
    timing: &Timing,
) -> bool {
    if schedule.retry_at.is_some_and(|r| r > now) {
        return false;
    }
    let Some(last_poll) = schedule.last_poll else {
        return true;
    };
    let since = now - last_poll;
    let active = last_traffic.is_some_and(|t| now - t <= ACTIVE_WINDOW_SECS);
    let interval = if active {
        timing.active_secs
    } else {
        timing.idle_secs
    };
    if since < interval {
        return false;
    }
    // Fresh readings from the traffic itself answer the poll; the endpoint
    // is still asked at the idle rate for what only it reports (per-model
    // weekly caps, extra usage).
    let passive_fresh = last_passive.is_some_and(|p| now - p < timing.active_secs);
    !(passive_fresh && since < timing.idle_secs)
}

/// Seconds to wait after the `failures`-th consecutive failure: 5, 10, 20,
/// … minutes capped at an hour, plus up to 20% jitter — or longer when the
/// endpoint said so (`Retry-After`).
fn backoff_secs(failures: u32, retry_after: Option<u64>, jitter: f64) -> i64 {
    let exp = failures.saturating_sub(1).min(16);
    let base = BACKOFF_BASE_SECS
        .saturating_mul(1_i64 << exp)
        .min(BACKOFF_CAP_SECS);
    let jittered = base + (base as f64 * 0.2 * jitter.clamp(0.0, 1.0)) as i64;
    jittered.max(retry_after.map(|s| s as i64).unwrap_or(0))
}

/// 0–1, from the clock's sub-second noise (spreads retries of sources that
/// failed together).
fn jitter() -> f64 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    (nanos % 1_000) as f64 / 1_000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    const T: Timing = Timing {
        active_secs: 300,
        idle_secs: 3_600,
    };

    #[test]
    fn missing_credits_back_off_instead_of_retrying_every_tick() {
        let mut schedule = Schedule::default();
        defer(&mut schedule, 1_000, 0.0);
        assert_eq!(schedule.retry_at, Some(1_000 + 300));
        // Not due on the next 30 s ticks, even for an account with traffic.
        assert!(!is_due(&schedule, 1_030, Some(1_000), None, &T));
        assert!(!is_due(&schedule, 1_299, Some(1_000), None, &T));

        defer(&mut schedule, 1_300, 0.0);
        assert_eq!(schedule.retry_at, Some(1_300 + 600));
        for _ in 0..10 {
            defer(&mut schedule, 2_000, 0.0);
        }
        assert_eq!(schedule.retry_at, Some(2_000 + 3_600));
    }

    fn polled_at(t: i64) -> Schedule {
        Schedule {
            last_poll: Some(t),
            ..Default::default()
        }
    }

    #[test]
    fn first_poll_is_due() {
        assert!(is_due(&Schedule::default(), 1_000, None, None, &T));
    }

    #[test]
    fn active_accounts_poll_every_five_minutes() {
        let now = 10_000;
        let traffic = Some(now - 60);
        assert!(!is_due(&polled_at(now - 200), now, traffic, None, &T));
        assert!(is_due(&polled_at(now - 300), now, traffic, None, &T));
    }

    #[test]
    fn idle_accounts_poll_hourly_and_resume_on_traffic() {
        let now = 100_000;
        let stale_traffic = Some(now - ACTIVE_WINDOW_SECS - 1);
        assert!(!is_due(
            &polled_at(now - 1_800),
            now,
            stale_traffic,
            None,
            &T
        ));
        assert!(is_due(
            &polled_at(now - 3_600),
            now,
            stale_traffic,
            None,
            &T
        ));
        // Traffic resumes: a poll older than five minutes is due right away.
        assert!(is_due(&polled_at(now - 1_800), now, Some(now), None, &T));
    }

    #[test]
    fn fresh_passive_reading_answers_the_poll_until_the_idle_interval() {
        let now = 100_000;
        let traffic = Some(now);
        let passive = Some(now - 30);
        assert!(!is_due(&polled_at(now - 600), now, traffic, passive, &T));
        assert!(is_due(&polled_at(now - 3_600), now, traffic, passive, &T));
        // A passive reading older than the active interval does not.
        assert!(is_due(
            &polled_at(now - 600),
            now,
            traffic,
            Some(now - 400),
            &T
        ));
    }

    #[test]
    fn back_off_blocks_until_retry_time() {
        let now = 50_000;
        let s = Schedule {
            last_poll: Some(now - 10_000),
            failures: 2,
            retry_at: Some(now + 5),
        };
        assert!(!is_due(&s, now, Some(now), None, &T));
        assert!(is_due(&s, now + 5, Some(now), None, &T));
    }

    #[test]
    fn back_off_doubles_caps_and_honours_retry_after() {
        assert_eq!(backoff_secs(1, None, 0.0), 300);
        assert_eq!(backoff_secs(2, None, 0.0), 600);
        assert_eq!(backoff_secs(3, None, 0.0), 1_200);
        assert_eq!(backoff_secs(5, None, 0.0), 3_600);
        assert_eq!(backoff_secs(40, None, 0.0), 3_600);
        assert_eq!(backoff_secs(1, None, 1.0), 360);
        assert_eq!(backoff_secs(1, Some(7_200), 0.5), 7_200);
        assert_eq!(backoff_secs(3, Some(10), 0.0), 1_200);
    }
}
