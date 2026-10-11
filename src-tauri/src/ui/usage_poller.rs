//! Background usage polling and change fan-out for the usage tracker.
//!
//! Passive readings (proxy / gateway headers) need no polling; this asks
//! usage endpoints directly, only as the settings allow, and only as often as
//! traffic warrants: every `poll_interval_secs` (5 min) while requests for the
//! account keep coming in, every `idle_poll_interval_secs` (1 h) otherwise. A
//! fresh reading from elsewhere — the traffic itself, or another login of the
//! same account (LocalRouter's ChatGPT login and Codex's on one ChatGPT
//! account) — answers the poll. Failures back off exponentially
//! (5 → 10 → 20 min, capped at 1 h, with jitter), honouring `Retry-After`.
//!
//! Sources:
//! - "Ask connected providers" — ChatGPT Plus/Pro (LocalRouter's OAuth
//!   login), GitHub Copilot, and API providers with a documented usage or
//!   credits endpoint that takes their own key (OpenRouter, LLM Gateway,
//!   Vercel AI Gateway; `lr_usage::providers::polled_provider_types`).
//! - "Use Claude Code and Codex logins" (off by default) — the CLIs' saved
//!   logins, for their subscription usage.
//!
//! It also forwards tracker changes to the UI (`usage-limits-changed`) and
//! the tray (usage items redraw; newly seen subscriptions are added to the
//! tray stats items), and keeps the tracker's copy of the `usage_tracking`
//! settings in step with the app config.

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use lr_config::{ConfigManager, UsageTrackingConfig};
use lr_providers::registry::ProviderRegistry;
use lr_usage::fetch::{self, FetchError};
use lr_usage::{AccountRef, DataSource, UsageReport, UsageTracker};
use parking_lot::{Mutex, RwLock};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Listener, Manager};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

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
    /// e.g. `claude_code_login`, `chatgpt_oauth`, `credits:<instance>`.
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
    /// Stops the poll loop.
    shutdown: CancellationToken,
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
            shutdown: CancellationToken::new(),
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

        // Settings written anywhere (other commands, a reload) reach the
        // tracker: right away through `config-changed`, and on every
        // scheduler tick for writers that do not emit it.
        let weak = Arc::downgrade(self);
        let app_for_config = app.clone();
        app.listen("config-changed", move |_| {
            if let Some(poller) = weak.upgrade() {
                poller.sync_config(&app_for_config);
            }
        });

        let poller = self.clone();
        tokio::spawn(async move {
            loop {
                poller.sync_config(&app);
                tokio::select! {
                    _ = poller.shutdown.cancelled() => break,
                    _ = poller.poll_once() => {}
                }
                tokio::select! {
                    _ = poller.shutdown.cancelled() => break,
                    _ = tokio::time::sleep(Duration::from_secs(SCHEDULER_TICK_SECS)) => {}
                    _ = poller.wake.notified() => {}
                }
            }
        });
    }

    /// Stop the poll loop (app exit). An in-flight poll is abandoned.
    pub fn shutdown(&self) {
        self.shutdown.cancel();
    }

    /// Apply the app config's `usage_tracking` settings when they differ
    /// from the tracker's.
    fn sync_config(&self, app: &AppHandle) {
        if let Some(config_manager) = app.try_state::<ConfigManager>() {
            self.apply_config(config_manager.get().usage_tracking);
        }
    }

    /// Hand changed settings to the tracker, polling again right away when
    /// the allowed sources changed. Idempotent.
    pub fn apply_config(&self, config: UsageTrackingConfig) {
        let previous = self.tracker.config();
        if previous == config {
            return;
        }
        let refresh = sources_changed(&previous, &config);
        self.tracker.set_config(config);
        if refresh {
            self.refresh_now();
        }
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
                .filter(|i| !config.poll_excluded_providers.contains(&i.instance_name))
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
            let polled_api_types = lr_usage::providers::polled_api_provider_types();
            for inst in instances
                .iter()
                .filter(|i| polled_api_types.contains(&i.provider_type.as_str()))
            {
                let id = format!("credits:{}", inst.instance_name);
                if self
                    .poll_provider_usage(&id, &inst.instance_name, &inst.provider_name)
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

    /// Whether source `id` (feeding `account` as `source`) should be polled
    /// now. For sources whose login is only known once polled (ChatGPT), the
    /// account the last poll found stands in for `account`.
    fn due(&self, id: &str, account: &AccountRef, source: DataSource) -> bool {
        let config = self.tracker.config();
        let timing = Timing {
            active_secs: config.poll_interval_secs.clamp(60, 3_600) as i64,
            idle_secs: config.idle_poll_interval_secs.clamp(300, 86_400) as i64,
        };
        let schedule = self.schedules.lock().get(id).cloned().unwrap_or_default();
        let account_id = self
            .tracker
            .account_id(schedule.account.as_ref().unwrap_or(account));
        is_due(
            &schedule,
            now(),
            self.tracker.last_traffic(&account_id),
            self.tracker.last_reading_besides(&account_id, Some(source)),
            &timing,
        )
    }

    /// Remember which account source `id` turned out to feed.
    fn note_account(&self, id: &str, account: &AccountRef) {
        self.schedules
            .lock()
            .entry(id.to_string())
            .or_default()
            .account = Some(account.clone());
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
                account_id: String::new(),
                last_attempt: None,
                last_success: None,
                last_error: None,
                retry_after: None,
            });
        s.account_id = self.tracker.account_id(account);
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
        let source = DataSource::ProviderApi;
        let unknown = AccountRef::chatgpt(None);
        if !self.due(id, &unknown, source) {
            return;
        }
        let Some(tokens) = lr_providers::oauth::token_source("openai-codex") else {
            return;
        };
        let token = match tokens.access_token().await {
            Ok(t) => t,
            Err(e) => {
                self.record(id, label, &unknown, Err(format!("no login: {e}")), None);
                return;
            }
        };
        let account_id = lr_usage::classify::chatgpt_account_from_jwt(&token);
        let account = AccountRef::chatgpt(account_id.as_deref());
        self.note_account(id, &account);
        let result = fetch::fetch_codex_usage(&self.client, &token, account_id.as_deref())
            .await
            .map(|report| with_chatgpt_login(report, &token));
        self.finish(id, label, &account, result, source);
    }

    async fn poll_copilot(&self) {
        let id = "github_copilot";
        let label = "GitHub Copilot";
        let account = AccountRef::subscription("github-copilot");
        if !self.due(id, &account, DataSource::ProviderApi) {
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

    /// Usage / credits via the provider's own documented endpoint
    /// ([`ModelProvider::fetch_usage`]). Returns whether the instance has
    /// one (so it gets a status row).
    ///
    /// [`ModelProvider::fetch_usage`]: lr_providers::ModelProvider::fetch_usage
    async fn poll_provider_usage(&self, id: &str, instance: &str, provider_name: &str) -> bool {
        let Some(provider) = self.registry.get_provider(instance) else {
            return false;
        };
        let Some(account) = provider.usage_account().or_else(|| {
            self.registry
                .get_provider_type_for_instance(instance)
                .and_then(|t| lr_usage::account_for_provider_type(&t))
        }) else {
            return false;
        };
        if !self.due(id, &account, DataSource::ProviderApi) {
            return true;
        }
        let Some(result) = provider.fetch_usage().await else {
            // No endpoint for this instance (e.g. a gateway pointed at
            // another host): nothing was asked. Check again after the
            // failure back-off rather than on every tick.
            self.defer_after_failure(id);
            return false;
        };
        self.finish(
            id,
            &format!("{provider_name} usage"),
            &account,
            result,
            DataSource::ProviderApi,
        );
        true
    }

    async fn poll_claude_code(&self) {
        let id = "claude_code_login";
        let label = "Claude Code login";
        let account = AccountRef::subscription("anthropic");
        if !self.due(id, &account, DataSource::CliLogin) {
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
        match &mut result {
            Ok(report) => report.plan = report.plan.take().or(login.plan.clone()),
            // Re-read the login next time: Claude Code may have replaced it.
            Err(FetchError::Unauthorized) => {
                lr_usage::cli_logins::forget_cached_claude_code_login()
            }
            Err(_) => {}
        }
        self.finish(id, label, &account, result, DataSource::CliLogin);
    }

    async fn poll_codex_cli(&self) {
        let id = "codex_login";
        let label = "Codex login";
        let source = DataSource::CliLogin;
        let unknown = AccountRef::chatgpt(None);
        if !self.due(id, &unknown, source) {
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
                &unknown,
                Err("not logged in with ChatGPT".into()),
                None,
            );
            return;
        };
        let account_id = lr_usage::classify::chatgpt_account_from_jwt(&login.access_token)
            .or_else(|| login.account_id.clone());
        let account = AccountRef::chatgpt(account_id.as_deref());
        self.note_account(id, &account);
        let result =
            fetch::fetch_codex_usage(&self.client, &login.access_token, account_id.as_deref())
                .await
                .map(|report| with_chatgpt_login(report, &login.access_token));
        self.finish(id, label, &account, result, source);
    }
}

/// A ChatGPT usage report completed from the access token it was fetched
/// with: the plan when the endpoint did not say, and who the login is.
fn with_chatgpt_login(mut report: UsageReport, token: &str) -> UsageReport {
    if report.plan.is_none() {
        report.plan = lr_usage::classify::chatgpt_plan_from_jwt(token);
    }
    report.login_label = lr_usage::classify::chatgpt_email_from_jwt(token);
    report
}

/// Whether a settings change alters what is polled.
fn sources_changed(previous: &UsageTrackingConfig, next: &UsageTrackingConfig) -> bool {
    previous.enabled != next.enabled
        || previous.poll_provider_apis != next.poll_provider_apis
        || previous.poll_excluded_providers != next.poll_excluded_providers
        || previous.read_cli_logins != next.read_cli_logins
}

/// Poll bookkeeping for one source.
#[derive(Debug, Clone, Default, PartialEq)]
struct Schedule {
    /// The account the source fed when last polled, for sources whose login
    /// is only known once polled.
    account: Option<AccountRef>,
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
/// account last saw traffic and a reading from any other source.
fn is_due(
    schedule: &Schedule,
    now: i64,
    last_traffic: Option<i64>,
    last_elsewhere: Option<i64>,
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
    // Fresh readings from the traffic itself (or another login of the same
    // account) answer the poll; the endpoint is still asked at the idle rate
    // for what only it reports (per-model weekly caps, extra usage).
    let fresh_elsewhere = last_elsewhere.is_some_and(|p| now - p < timing.active_secs);
    !(fresh_elsewhere && since < timing.idle_secs)
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
    fn only_source_settings_trigger_a_refresh() {
        let base = UsageTrackingConfig::default();
        let mut c = base.clone();
        c.tray_items_added
            .push("anthropic:subscription|seven_day".into());
        c.hidden_accounts.push("openai:api".into());
        c.poll_interval_secs = 60;
        assert!(!sources_changed(&base, &c));
        for change in [
            |c: &mut UsageTrackingConfig| c.enabled = false,
            |c: &mut UsageTrackingConfig| c.poll_provider_apis = false,
            |c: &mut UsageTrackingConfig| c.read_cli_logins = true,
            |c: &mut UsageTrackingConfig| c.poll_excluded_providers.push("x".into()),
        ] {
            let mut c = base.clone();
            change(&mut c);
            assert!(sources_changed(&base, &c));
        }
    }

    #[test]
    fn apply_config_reaches_the_tracker_and_repolls_on_source_changes() {
        let tracker = Arc::new(UsageTracker::new(UsageTrackingConfig::default()));
        let poller = UsagePoller::new(tracker.clone(), Arc::new(ProviderRegistry::new()));
        let backed_off = || {
            poller
                .schedules
                .lock()
                .insert("chatgpt_oauth".into(), polled_at(1_000));
        };
        backed_off();

        let mut config = UsageTrackingConfig::default();
        config.hidden_accounts.push("openai:api".into());
        poller.apply_config(config.clone());
        assert_eq!(tracker.config(), config);
        assert!(
            !poller.schedules.lock().is_empty(),
            "display-only change keeps the schedule"
        );

        config.read_cli_logins = true;
        poller.apply_config(config.clone());
        assert!(tracker.config().read_cli_logins);
        assert!(
            poller.schedules.lock().is_empty(),
            "source change polls again now"
        );

        // Unchanged settings are a no-op.
        backed_off();
        poller.apply_config(config);
        assert!(!poller.schedules.lock().is_empty());
    }

    /// LocalRouter's ChatGPT login and Codex's, signed in to the same
    /// account: once one has polled it, the other is not due until the idle
    /// interval; a different account is polled on its own schedule.
    #[test]
    fn two_logins_of_one_chatgpt_account_poll_it_once() {
        let tracker = Arc::new(UsageTracker::new(UsageTrackingConfig::default()));
        let poller = UsagePoller::new(tracker.clone(), Arc::new(ProviderRegistry::new()));
        let unknown = AccountRef::chatgpt(None);
        let acc1 = AccountRef::chatgpt(Some("acc-1"));
        let acc2 = AccountRef::chatgpt(Some("acc-2"));
        let polled = |id: &str, account: &AccountRef, source: DataSource| {
            poller.note_account(id, account);
            poller.finish(
                id,
                id,
                account,
                Ok(UsageReport {
                    plan: Some("pro".into()),
                    ..Default::default()
                }),
                source,
            );
        };
        assert!(poller.due("chatgpt_oauth", &unknown, DataSource::ProviderApi));
        polled("chatgpt_oauth", &acc1, DataSource::ProviderApi);
        assert!(!poller.due("chatgpt_oauth", &unknown, DataSource::ProviderApi));

        // Codex has not been polled yet, so it is due. Ten minutes after its
        // last poll an account in use is due again — unless, as here, the
        // other login of the same account has just polled it.
        assert!(poller.due("codex_login", &unknown, DataSource::CliLogin));
        poller.note_account("codex_login", &acc1);
        poller
            .schedules
            .lock()
            .get_mut("codex_login")
            .unwrap()
            .last_poll = Some(now() - 600);
        tracker.note_traffic(&acc1);
        assert!(!poller.due("codex_login", &unknown, DataSource::CliLogin));
        // A different account in use has no fresh reading: due.
        poller.note_account("codex_login", &acc2);
        tracker.note_traffic(&acc2);
        assert!(poller.due("codex_login", &unknown, DataSource::CliLogin));
        polled("codex_login", &acc2, DataSource::CliLogin);

        let statuses = poller.statuses();
        let account_of = |id: &str| {
            statuses
                .iter()
                .find(|s| s.id == id)
                .map(|s| s.account_id.clone())
                .unwrap()
        };
        assert_eq!(account_of("chatgpt_oauth"), "openai:subscription");
        assert_eq!(account_of("codex_login"), acc2.id());
        assert_eq!(tracker.snapshot().accounts.len(), 2);
    }

    #[test]
    fn chatgpt_report_takes_plan_and_login_from_the_token() {
        use base64::Engine;
        let claims = serde_json::json!({
            "https://api.openai.com/auth": {"chatgpt_plan_type": "plus", "chatgpt_account_id": "a"},
            "https://api.openai.com/profile": {"email": "me@example.com"}
        });
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(serde_json::to_vec(&claims).unwrap());
        let token = format!("h.{payload}.s");
        let r = with_chatgpt_login(UsageReport::default(), &token);
        assert_eq!(r.plan.as_deref(), Some("plus"));
        assert_eq!(r.login_label.as_deref(), Some("me@example.com"));
        let r = with_chatgpt_login(
            UsageReport {
                plan: Some("pro".into()),
                ..Default::default()
            },
            &token,
        );
        assert_eq!(r.plan.as_deref(), Some("pro"), "the endpoint's plan wins");
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
            ..Default::default()
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
