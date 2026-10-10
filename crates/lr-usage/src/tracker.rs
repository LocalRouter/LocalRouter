//! The usage tracker: per-account state fed by parsed readings and request
//! costs, persisted to disk, and turned into views for the UI.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::Arc;

use lr_config::UsageTrackingConfig;
use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use crate::types::{
    AccountKind, AccountRef, CreditsReading, DataSource, QuotaReading, UsageReport, WindowReading,
};
use crate::view::{self, UsageSnapshot};

/// Ledger bucket length.
pub const LEDGER_BUCKET_SECS: i64 = 15 * 60;
/// How long ledger buckets, samples and idle accounts are kept.
pub const RETENTION_SECS: i64 = 35 * 86_400;
/// Samples closer together than this replace each other.
const SAMPLE_SPACING_SECS: i64 = 60;
const MAX_SAMPLES: usize = 2_000;
const MAX_HISTORY: usize = 12;
/// Minimum gap between change notifications.
const NOTIFY_SPACING_SECS: i64 = 1;
/// Minimum gap between writes of the state file.
const PERSIST_SPACING_SECS: i64 = 30;
/// How often old data is pruned when nothing else triggers it.
const PRUNE_SPACING_SECS: i64 = 3_600;
/// A window is retired once this many of its periods have passed since it
/// ended without a new reading.
const RETIRE_AFTER_PERIODS: i64 = 2;
/// Period assumed for windows of unknown length when retiring them.
const DEFAULT_RETIRE_PERIOD_SECS: i64 = 86_400;
/// Window lengths beyond this (a year) are nonsense and dropped.
const MAX_WINDOW_SECS: i64 = 366 * 86_400;
/// Reset times further ahead than this (two years) are nonsense and dropped.
const MAX_RESET_AHEAD_SECS: i64 = 2 * 366 * 86_400;
const STATE_VERSION: u32 = 1;

pub(crate) fn now_secs() -> i64 {
    chrono::Utc::now().timestamp()
}

/// Tokens and API-equivalent cost of one request.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct LedgerEntry {
    pub requests: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    #[serde(default)]
    pub cache_read_tokens: u64,
    #[serde(default)]
    pub cache_write_tokens: u64,
    pub cost_usd: f64,
}

impl LedgerEntry {
    fn add(&mut self, o: &LedgerEntry) {
        self.requests += o.requests;
        self.input_tokens += o.input_tokens;
        self.output_tokens += o.output_tokens;
        self.cache_read_tokens += o.cache_read_tokens;
        self.cache_write_tokens += o.cache_write_tokens;
        self.cost_usd += o.cost_usd;
    }

    pub fn total_tokens(&self) -> u64 {
        self.input_tokens + self.output_tokens + self.cache_read_tokens + self.cache_write_tokens
    }
}

/// A window that has ended, and how far it got.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PastWindow {
    pub ended_at: i64,
    pub peak_percent: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct StoredWindow {
    pub reading: WindowReading,
    pub updated_at: i64,
    pub source: DataSource,
    /// `(unix secs, used %)` within the current window.
    #[serde(default)]
    pub samples: Vec<(i64, f64)>,
    #[serde(default)]
    pub history: Vec<PastWindow>,
}

impl StoredWindow {
    fn peak(&self) -> f64 {
        self.samples
            .iter()
            .map(|(_, p)| *p)
            .fold(self.reading.used_percent, f64::max)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct StoredQuota {
    pub reading: QuotaReading,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct AccountState {
    pub provider: String,
    pub kind: AccountKind,
    #[serde(default)]
    pub plan: Option<String>,
    #[serde(default)]
    pub windows: BTreeMap<String, StoredWindow>,
    #[serde(default)]
    pub quotas: BTreeMap<String, StoredQuota>,
    #[serde(default)]
    pub credits: Option<(CreditsReading, i64)>,
    #[serde(default)]
    pub status: Option<(String, i64)>,
    /// Source → last time it reported.
    #[serde(default)]
    pub sources: BTreeMap<DataSource, i64>,
    pub first_seen: i64,
    pub last_seen: i64,
    /// Bucket start → totals.
    #[serde(default)]
    pub ledger: BTreeMap<i64, LedgerEntry>,
}

impl AccountState {
    fn new(account: &AccountRef, now: i64) -> Self {
        Self {
            provider: account.provider.clone(),
            kind: account.kind,
            plan: None,
            windows: BTreeMap::new(),
            quotas: BTreeMap::new(),
            credits: None,
            status: None,
            sources: BTreeMap::new(),
            first_seen: now,
            last_seen: now,
            ledger: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub(crate) struct State {
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub accounts: BTreeMap<String, AccountState>,
}

type ChangeCallback = Arc<dyn Fn() + Send + Sync>;

/// Tracks subscription windows, rate limits, credits and request costs per
/// usage account.
pub struct UsageTracker {
    state: RwLock<State>,
    config: RwLock<UsageTrackingConfig>,
    enabled: AtomicBool,
    path: Option<PathBuf>,
    dirty: AtomicBool,
    last_persist: AtomicI64,
    notify_pending: AtomicBool,
    last_notify: AtomicI64,
    on_change: RwLock<Option<ChangeCallback>>,
    /// Account id → last time real traffic for it was seen (not polls).
    traffic: RwLock<HashMap<String, i64>>,
    last_prune: AtomicI64,
    /// Serializes writes of the state file (maintenance tick vs. exit).
    persist_lock: Mutex<()>,
    /// Stops the maintenance task.
    shutdown: CancellationToken,
}

impl UsageTracker {
    /// In-memory tracker (tests).
    pub fn new(config: UsageTrackingConfig) -> Self {
        Self {
            enabled: AtomicBool::new(config.enabled),
            state: RwLock::new(State {
                version: STATE_VERSION,
                accounts: BTreeMap::new(),
            }),
            config: RwLock::new(config),
            path: None,
            dirty: AtomicBool::new(false),
            last_persist: AtomicI64::new(0),
            notify_pending: AtomicBool::new(false),
            last_notify: AtomicI64::new(0),
            on_change: RwLock::new(None),
            traffic: RwLock::new(HashMap::new()),
            last_prune: AtomicI64::new(0),
            persist_lock: Mutex::new(()),
            shutdown: CancellationToken::new(),
        }
    }

    /// Tracker persisted at `path`, restoring what is there. A missing or
    /// unreadable file starts empty.
    pub fn load(path: PathBuf, config: UsageTrackingConfig) -> Self {
        let mut tracker = Self::new(config);
        match std::fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice::<State>(&bytes) {
                Ok(state) => *tracker.state.get_mut() = state,
                Err(e) => tracing::warn!("Ignoring unreadable usage state {:?}: {}", path, e),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => tracing::warn!("Failed to read usage state {:?}: {}", path, e),
        }
        tracker.path = Some(path);
        tracker
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    pub fn config(&self) -> UsageTrackingConfig {
        self.config.read().clone()
    }

    /// Apply a changed config (enable switch, plan overrides, …).
    pub fn set_config(&self, config: UsageTrackingConfig) {
        self.enabled.store(config.enabled, Ordering::Relaxed);
        *self.config.write() = config;
        self.mark_changed();
    }

    /// Called (throttled) after state changes.
    pub fn set_on_change(&self, cb: impl Fn() + Send + Sync + 'static) {
        *self.on_change.write() = Some(Arc::new(cb));
    }

    /// Record a usage report for `account`.
    pub fn apply(&self, account: &AccountRef, report: &UsageReport, source: DataSource) {
        self.apply_at(account, report, source, now_secs());
    }

    pub(crate) fn apply_at(
        &self,
        account: &AccountRef,
        report: &UsageReport,
        source: DataSource,
        now: i64,
    ) {
        if !self.is_enabled() || report.is_empty() {
            return;
        }
        let mut changed = false;
        {
            let mut state = self.state.write();
            let acct = state
                .accounts
                .entry(account.id())
                .or_insert_with(|| AccountState::new(account, now));
            acct.last_seen = now;
            acct.sources.insert(source, now);
            if let Some(plan) = &report.plan {
                if acct.plan.as_ref() != Some(plan) {
                    acct.plan = Some(plan.clone());
                    changed = true;
                }
            }
            for reading in &report.windows {
                if let Some(reading) = sanitize_window(reading, now) {
                    changed |= apply_window(acct, &reading, source, now);
                }
            }
            for q in &report.quotas {
                let q = QuotaReading {
                    resets_at: sane_reset(q.resets_at, now),
                    ..q.clone()
                };
                let prev = acct.quotas.get(&q.id).map(|s| &s.reading);
                changed |= prev != Some(&q);
                acct.quotas.insert(
                    q.id.clone(),
                    StoredQuota {
                        reading: q,
                        updated_at: now,
                    },
                );
            }
            if let Some(c) = &report.credits {
                changed |= acct.credits.as_ref().map(|(c, _)| c) != Some(c);
                acct.credits = Some((c.clone(), now));
            }
            if let Some(s) = &report.status {
                changed |= acct.status.as_ref().map(|(s, _)| s) != Some(s);
                acct.status = Some((s.clone(), now));
            }
        }
        self.dirty.store(true, Ordering::Relaxed);
        if changed {
            self.mark_changed();
        }
    }

    /// Note that real traffic for `account` was just seen (drives how often
    /// its usage endpoint is polled).
    pub fn note_traffic(&self, account: &AccountRef) {
        self.traffic.write().insert(account.id(), now_secs());
    }

    /// When traffic for `account_id` was last seen.
    pub fn last_traffic(&self, account_id: &str) -> Option<i64> {
        self.traffic.read().get(account_id).copied()
    }

    /// When a passive source (headers or a usage response on traffic
    /// LocalRouter carried) last reported for `account_id`.
    pub fn last_passive_reading(&self, account_id: &str) -> Option<i64> {
        let state = self.state.read();
        let acct = state.accounts.get(account_id)?;
        [
            DataSource::ProxyHeaders,
            DataSource::ProxyUsageResponse,
            DataSource::GatewayHeaders,
        ]
        .iter()
        .filter_map(|s| acct.sources.get(s).copied())
        .max()
    }

    /// Add one request's tokens and API-equivalent cost to `account`.
    pub fn record_request(&self, account: &AccountRef, entry: LedgerEntry) {
        self.record_request_at(account, entry, now_secs());
    }

    pub(crate) fn record_request_at(&self, account: &AccountRef, entry: LedgerEntry, now: i64) {
        if !self.is_enabled() {
            return;
        }
        self.traffic.write().insert(account.id(), now);
        {
            let mut state = self.state.write();
            let acct = state
                .accounts
                .entry(account.id())
                .or_insert_with(|| AccountState::new(account, now));
            acct.last_seen = now;
            let bucket = now - now.rem_euclid(LEDGER_BUCKET_SECS);
            let is_new_bucket = !acct.ledger.contains_key(&bucket);
            acct.ledger.entry(bucket).or_default().add(&entry);
            if is_new_bucket {
                let cutoff = now - RETENTION_SECS;
                acct.ledger.retain(|b, _| *b >= cutoff);
            }
        }
        self.dirty.store(true, Ordering::Relaxed);
        self.mark_changed();
    }

    /// Forget everything recorded for an account.
    pub fn forget(&self, account_id: &str) -> bool {
        let removed = self.state.write().accounts.remove(account_id).is_some();
        if removed {
            self.dirty.store(true, Ordering::Relaxed);
            self.mark_changed();
        }
        removed
    }

    /// The current view of every account.
    pub fn snapshot(&self) -> UsageSnapshot {
        self.snapshot_at(now_secs())
    }

    pub(crate) fn snapshot_at(&self, now: i64) -> UsageSnapshot {
        let config = self.config.read();
        let state = self.state.read();
        view::build_snapshot(&state, &config, self.is_enabled(), now)
    }

    fn mark_changed(&self) {
        self.notify_pending.store(true, Ordering::Relaxed);
        self.flush_notify(now_secs());
    }

    fn flush_notify(&self, now: i64) {
        if !self.notify_pending.load(Ordering::Relaxed) {
            return;
        }
        let last = self.last_notify.load(Ordering::Relaxed);
        if now - last < NOTIFY_SPACING_SECS {
            return;
        }
        self.last_notify.store(now, Ordering::Relaxed);
        self.notify_pending.store(false, Ordering::Relaxed);
        let cb = self.on_change.read().clone();
        if let Some(cb) = cb {
            cb();
        }
    }

    /// Periodic housekeeping: deliver throttled notifications, prune old
    /// data, and write the state file when it changed.
    pub fn tick(&self) {
        self.tick_at(now_secs());
    }

    pub(crate) fn tick_at(&self, now: i64) {
        self.flush_notify(now);
        let persist_due = self.dirty.load(Ordering::Relaxed)
            && now - self.last_persist.load(Ordering::Relaxed) >= PERSIST_SPACING_SECS;
        if persist_due || now - self.last_prune.load(Ordering::Relaxed) >= PRUNE_SPACING_SECS {
            self.last_prune.store(now, Ordering::Relaxed);
            if self.prune(now) {
                self.dirty.store(true, Ordering::Relaxed);
                self.mark_changed();
            }
        }
        if self.dirty.load(Ordering::Relaxed)
            && now - self.last_persist.load(Ordering::Relaxed) >= PERSIST_SPACING_SECS
        {
            self.persist_now();
        }
    }

    /// Drop idle accounts, old ledger buckets and retired windows. Returns
    /// whether an account or window went away.
    fn prune(&self, now: i64) -> bool {
        let cutoff = now - RETENTION_SECS;
        let mut state = self.state.write();
        let accounts = state.accounts.len();
        state.accounts.retain(|_, a| a.last_seen >= cutoff);
        let mut changed = state.accounts.len() != accounts;
        for acct in state.accounts.values_mut() {
            acct.ledger.retain(|b, _| *b >= cutoff);
            let windows = acct.windows.len();
            let sources = &acct.sources;
            acct.windows
                .retain(|_, w| !window_retired(w, sources.get(&w.source).copied(), now));
            changed |= acct.windows.len() != windows;
        }
        changed
    }

    /// Stop the maintenance task and write unsaved changes. Called on app
    /// exit; the tracker keeps recording in memory afterwards.
    pub fn shutdown(&self) {
        self.shutdown.cancel();
        if self.dirty.load(Ordering::Relaxed) {
            self.prune(now_secs());
            self.persist_now();
        }
    }

    /// Write the state file now (atomic rename). No-op without a path.
    pub fn persist_now(&self) {
        let Some(path) = &self.path else {
            return;
        };
        let _write = self.persist_lock.lock();
        self.dirty.store(false, Ordering::Relaxed);
        self.last_persist.store(now_secs(), Ordering::Relaxed);
        let bytes = {
            let state = self.state.read();
            match serde_json::to_vec(&*state) {
                Ok(b) => b,
                Err(e) => {
                    tracing::warn!("Failed to serialize usage state: {}", e);
                    return;
                }
            }
        };
        let tmp = path.with_extension("json.tmp");
        let result = std::fs::write(&tmp, bytes).and_then(|_| std::fs::rename(&tmp, path));
        if let Err(e) = result {
            tracing::warn!("Failed to write usage state {:?}: {}", path, e);
            self.dirty.store(true, Ordering::Relaxed);
        }
    }

    /// Run [`tick`](Self::tick) every few seconds on the tokio runtime,
    /// until [`shutdown`](Self::shutdown).
    pub fn spawn_maintenance(self: &Arc<Self>) -> tokio::task::JoinHandle<()> {
        let tracker = Arc::downgrade(self);
        let stop = self.shutdown.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(2));
            loop {
                tokio::select! {
                    _ = stop.cancelled() => break,
                    _ = interval.tick() => {}
                }
                let Some(t) = tracker.upgrade() else { break };
                t.tick();
            }
        })
    }
}

/// `resets_at` when it is a plausible time: after the epoch and at most
/// [`MAX_RESET_AHEAD_SECS`] ahead.
fn sane_reset(resets_at: Option<i64>, now: i64) -> Option<i64> {
    resets_at.filter(|t| *t > 0 && *t <= now.saturating_add(MAX_RESET_AHEAD_SECS))
}

/// A window reading with nonsense from the response dropped: a non-finite
/// usage figure discards the reading; an implausible window length or reset
/// time is treated as unknown.
fn sanitize_window(reading: &WindowReading, now: i64) -> Option<WindowReading> {
    if !reading.used_percent.is_finite() {
        return None;
    }
    let mut r = reading.clone();
    r.window_secs = r.window_secs.filter(|w| (1..=MAX_WINDOW_SECS).contains(w));
    r.resets_at = sane_reset(r.resets_at, now);
    Some(r)
}

/// Whether a window has been retired by its provider (e.g. a per-model
/// weekly cap that went away): [`RETIRE_AFTER_PERIODS`] of its periods have
/// passed since it ended (its reset, or its last reading when it has no
/// reset time) and the source that fed it has reported since without it.
/// Windows nothing has refreshed for the account retention period go too.
/// An idle account's windows stay: no newer report means no evidence.
fn window_retired(w: &StoredWindow, source_last_seen: Option<i64>, now: i64) -> bool {
    let period = w
        .reading
        .window_secs
        .filter(|s| *s > 0)
        .or_else(|| crate::types::window_secs_for(&w.reading.id))
        .unwrap_or(DEFAULT_RETIRE_PERIOD_SECS);
    let ended = w.reading.resets_at.unwrap_or(w.updated_at);
    let grace = period.saturating_mul(RETIRE_AFTER_PERIODS);
    if now < ended.saturating_add(grace) {
        return false;
    }
    let reported_without_it = source_last_seen.is_some_and(|t| t > w.updated_at && t > ended);
    reported_without_it || now >= ended.saturating_add(grace.max(RETENTION_SECS))
}

/// Merge one window reading. Returns whether anything visible changed.
fn apply_window(
    acct: &mut AccountState,
    reading: &WindowReading,
    source: DataSource,
    now: i64,
) -> bool {
    let entry = acct.windows.entry(reading.id.clone());
    let stored = match entry {
        std::collections::btree_map::Entry::Vacant(v) => {
            v.insert(StoredWindow {
                reading: reading.clone(),
                updated_at: now,
                source,
                samples: vec![(now, reading.used_percent)],
                history: Vec::new(),
            });
            return true;
        }
        std::collections::btree_map::Entry::Occupied(o) => o.into_mut(),
    };

    // A later reset time means the window rolled over: close out the old
    // one so its samples never mix with the new window's.
    let rolled_over = match (stored.reading.resets_at, reading.resets_at) {
        (Some(old), Some(new)) => {
            let slack = reading
                .window_secs
                .or(stored.reading.window_secs)
                .map(|w| w / 2)
                .unwrap_or(3_600);
            new > old.saturating_add(slack) || (old <= now && new > old)
        }
        _ => false,
    };
    if rolled_over {
        let peak = stored.peak();
        if let Some(ended_at) = stored.reading.resets_at {
            stored.history.push(PastWindow {
                ended_at,
                peak_percent: peak,
            });
            if stored.history.len() > MAX_HISTORY {
                let excess = stored.history.len() - MAX_HISTORY;
                stored.history.drain(..excess);
            }
        }
        stored.samples.clear();
    }

    let changed = rolled_over
        || (stored.reading.used_percent - reading.used_percent).abs() > 1e-9
        || stored.reading.resets_at != reading.resets_at;

    let mut merged = reading.clone();
    // Endpoints know the window length; headers sometimes don't.
    if merged.window_secs.is_none() {
        merged.window_secs = stored.reading.window_secs;
    }
    if merged.resets_at.is_none() && !rolled_over {
        merged.resets_at = stored.reading.resets_at;
    }
    stored.reading = merged;
    stored.updated_at = now;
    stored.source = source;

    match stored.samples.last_mut() {
        Some(last) if now - last.0 < SAMPLE_SPACING_SECS => {
            *last = (now, reading.used_percent);
        }
        _ => stored.samples.push((now, reading.used_percent)),
    }
    if let (Some(reset), Some(len)) = (stored.reading.resets_at, stored.reading.window_secs) {
        let start = reset.saturating_sub(len);
        stored.samples.retain(|(t, _)| *t >= start);
    }
    if stored.samples.len() > MAX_SAMPLES {
        let excess = stored.samples.len() - MAX_SAMPLES;
        stored.samples.drain(..excess);
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    fn week(used: f64, resets_at: i64) -> UsageReport {
        UsageReport {
            windows: vec![WindowReading {
                id: "seven_day".into(),
                label: "Weekly".into(),
                used_percent: used,
                resets_at: Some(resets_at),
                window_secs: Some(604_800),
            }],
            ..Default::default()
        }
    }

    fn claude() -> AccountRef {
        AccountRef::subscription("anthropic")
    }

    #[test]
    fn applies_windows_and_samples() {
        let t = UsageTracker::new(UsageTrackingConfig::default());
        let reset = 1_000_000;
        t.apply_at(
            &claude(),
            &week(10.0, reset),
            DataSource::ProxyHeaders,
            reset - 500_000,
        );
        t.apply_at(
            &claude(),
            &week(10.5, reset),
            DataSource::ProxyHeaders,
            reset - 499_990,
        );
        t.apply_at(
            &claude(),
            &week(20.0, reset),
            DataSource::CliLogin,
            reset - 400_000,
        );
        let state = t.state.read();
        let w = &state.accounts["anthropic:subscription"].windows["seven_day"];
        assert_eq!(w.reading.used_percent, 20.0);
        assert_eq!(w.source, DataSource::CliLogin);
        // The two readings 10s apart collapse into one sample.
        assert_eq!(w.samples.len(), 2);
        assert!(w.history.is_empty());
    }

    #[test]
    fn rollover_records_history_and_resets_samples() {
        let t = UsageTracker::new(UsageTrackingConfig::default());
        let reset = 1_000_000;
        t.apply_at(
            &claude(),
            &week(80.0, reset),
            DataSource::ProxyHeaders,
            reset - 10,
        );
        t.apply_at(
            &claude(),
            &week(2.0, reset + 604_800),
            DataSource::ProxyHeaders,
            reset + 100,
        );
        let state = t.state.read();
        let w = &state.accounts["anthropic:subscription"].windows["seven_day"];
        assert_eq!(w.history.len(), 1);
        assert_eq!(w.history[0].peak_percent, 80.0);
        assert_eq!(w.history[0].ended_at, reset);
        assert_eq!(w.samples, vec![(reset + 100, 2.0)]);
    }

    #[test]
    fn small_reset_jitter_is_not_a_rollover() {
        let t = UsageTracker::new(UsageTrackingConfig::default());
        let reset = 1_000_000;
        t.apply_at(
            &claude(),
            &week(30.0, reset),
            DataSource::ProxyHeaders,
            reset - 1_000,
        );
        t.apply_at(
            &claude(),
            &week(31.0, reset + 1),
            DataSource::CliLogin,
            reset - 900,
        );
        let state = t.state.read();
        assert!(
            state.accounts["anthropic:subscription"].windows["seven_day"]
                .history
                .is_empty()
        );
    }

    #[test]
    fn disabled_tracker_records_nothing() {
        let t = UsageTracker::new(UsageTrackingConfig {
            enabled: false,
            ..Default::default()
        });
        t.apply_at(&claude(), &week(1.0, 10), DataSource::ProxyHeaders, 0);
        t.record_request_at(&claude(), LedgerEntry::default(), 0);
        assert!(t.state.read().accounts.is_empty());
    }

    #[test]
    fn ledger_buckets_and_retention() {
        let t = UsageTracker::new(UsageTrackingConfig::default());
        let e = LedgerEntry {
            requests: 1,
            input_tokens: 10,
            output_tokens: 5,
            cost_usd: 0.5,
            ..Default::default()
        };
        let now = 10 * LEDGER_BUCKET_SECS;
        t.record_request_at(&claude(), e, now);
        t.record_request_at(&claude(), e, now + 60);
        t.record_request_at(&claude(), e, now + RETENTION_SECS + LEDGER_BUCKET_SECS);
        let state = t.state.read();
        let ledger = &state.accounts["anthropic:subscription"].ledger;
        assert_eq!(ledger.len(), 1, "old bucket pruned");
        assert_eq!(ledger.values().next().unwrap().requests, 1);
    }

    #[test]
    fn notifications_are_throttled_and_flushed() {
        let t = UsageTracker::new(UsageTrackingConfig::default());
        let count = Arc::new(AtomicUsize::new(0));
        let c = count.clone();
        t.set_on_change(move || {
            c.fetch_add(1, Ordering::Relaxed);
        });
        t.apply_at(
            &claude(),
            &week(1.0, 10_000_000_000),
            DataSource::ProxyHeaders,
            0,
        );
        assert_eq!(count.load(Ordering::Relaxed), 1);
        // Pretend the last notification just happened.
        let later = now_secs() + 1_000;
        t.last_notify.store(later, Ordering::Relaxed);
        t.apply_at(
            &claude(),
            &week(2.0, 10_000_000_000),
            DataSource::ProxyHeaders,
            0,
        );
        assert_eq!(count.load(Ordering::Relaxed), 1);
        assert!(t.notify_pending.load(Ordering::Relaxed));
        t.flush_notify(later + 5);
        assert_eq!(count.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn persists_and_reloads() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("usage.json");
        let t = UsageTracker::load(path.clone(), UsageTrackingConfig::default());
        t.apply(
            &claude(),
            &week(42.0, now_secs() + 3_600),
            DataSource::CliLogin,
        );
        t.persist_now();
        let t2 = UsageTracker::load(path, UsageTrackingConfig::default());
        let state = t2.state.read();
        assert_eq!(
            state.accounts["anthropic:subscription"].windows["seven_day"]
                .reading
                .used_percent,
            42.0
        );
    }

    #[test]
    fn traffic_and_passive_readings() {
        let t = UsageTracker::new(UsageTrackingConfig::default());
        assert_eq!(t.last_traffic("anthropic:subscription"), None);
        t.record_request_at(&claude(), LedgerEntry::default(), 50);
        assert_eq!(t.last_traffic("anthropic:subscription"), Some(50));
        t.apply_at(&claude(), &week(1.0, 10), DataSource::CliLogin, 60);
        assert_eq!(t.last_passive_reading("anthropic:subscription"), None);
        t.apply_at(&claude(), &week(2.0, 10), DataSource::ProxyHeaders, 70);
        assert_eq!(t.last_passive_reading("anthropic:subscription"), Some(70));
    }

    fn window(id: &str, used: f64, resets_at: Option<i64>, len: Option<i64>) -> UsageReport {
        UsageReport {
            windows: vec![WindowReading {
                id: id.into(),
                label: id.into(),
                used_percent: used,
                resets_at,
                window_secs: len,
            }],
            ..Default::default()
        }
    }

    fn window_ids(t: &UsageTracker) -> Vec<String> {
        t.state.read().accounts["anthropic:subscription"]
            .windows
            .keys()
            .cloned()
            .collect()
    }

    #[test]
    fn retired_window_is_pruned_once_its_source_reports_without_it() {
        let t = UsageTracker::new(UsageTrackingConfig::default());
        let week_secs = 604_800;
        let reset = 10_000_000;
        // A per-model cap seen once, alongside the weekly window.
        let mut both = week(10.0, reset);
        both.windows
            .extend(window("seven_day_fable", 5.0, Some(reset), Some(week_secs)).windows);
        t.apply_at(&claude(), &both, DataSource::CliLogin, reset - 1_000);

        // The same source keeps reporting, without the per-model cap.
        let mut now = reset;
        while now < reset + 2 * week_secs {
            now += 86_400;
            let next_reset = reset + week_secs * ((now - reset) / week_secs + 1);
            t.apply_at(&claude(), &week(1.0, next_reset), DataSource::CliLogin, now);
        }
        // Before two periods have passed since its reset it is kept.
        assert!(!t.prune(reset + 2 * week_secs - 1));
        assert!(window_ids(&t).contains(&"seven_day_fable".to_string()));
        assert!(t.prune(reset + 2 * week_secs));
        assert_eq!(window_ids(&t), vec!["seven_day".to_string()]);
    }

    #[test]
    fn idle_account_keeps_its_ended_windows() {
        let t = UsageTracker::new(UsageTrackingConfig::default());
        let reset = 10_000_000;
        t.apply_at(
            &claude(),
            &week(40.0, reset),
            DataSource::ProxyHeaders,
            reset - 1_000,
        );
        // Nothing has reported since: the account is idle, not retired.
        assert!(!t.prune(reset + 3 * 604_800));
        assert_eq!(window_ids(&t), vec!["seven_day".to_string()]);
        // A different source reporting is no evidence either.
        t.apply_at(
            &claude(),
            &window("five_hour", 1.0, Some(reset + 4 * 604_800), Some(18_000)),
            DataSource::CliLogin,
            reset + 3 * 604_800,
        );
        assert!(!t.prune(reset + 3 * 604_800 + 1));
        assert!(window_ids(&t).contains(&"seven_day".to_string()));
        // Unrefreshed for the whole retention period: dropped.
        assert!(t.prune(reset + RETENTION_SECS));
        assert_eq!(window_ids(&t), vec!["five_hour".to_string()]);
    }

    #[test]
    fn window_without_reset_retires_from_its_last_reading() {
        let t = UsageTracker::new(UsageTrackingConfig::default());
        t.apply_at(
            &claude(),
            &window("credits_pool", 3.0, None, None),
            DataSource::ProviderApi,
            1_000,
        );
        t.apply_at(
            &claude(),
            &week(1.0, 10_000_000),
            DataSource::ProviderApi,
            2_000,
        );
        // Default period is a day.
        assert!(!t.prune(1_000 + 2 * 86_400 - 1));
        assert!(t.prune(1_000 + 2 * 86_400));
        assert_eq!(window_ids(&t), vec!["seven_day".to_string()]);
    }

    #[test]
    fn tick_prunes_even_without_new_data() {
        let t = UsageTracker::new(UsageTrackingConfig::default());
        t.apply_at(&claude(), &week(1.0, 10), DataSource::ProxyHeaders, 0);
        let later = RETENTION_SECS + 10;
        t.tick_at(later);
        assert!(t.state.read().accounts.is_empty());
    }

    #[test]
    fn nonsense_numbers_are_dropped_not_overflowed() {
        let t = UsageTracker::new(UsageTrackingConfig::default());
        let now = 1_000_000;
        for (resets_at, len) in [
            (Some(i64::MAX), Some(i64::MAX)),
            (Some(i64::MIN), Some(i64::MIN)),
            (Some(now + 3_600), Some(-5)),
            (Some(now + 3_600), Some(i64::MAX)),
            (Some(-1), Some(3_600)),
        ] {
            t.apply_at(
                &claude(),
                &window("five_hour", 10.0, resets_at, len),
                DataSource::ProxyHeaders,
                now,
            );
        }
        t.apply_at(
            &claude(),
            &window("seven_day", f64::NAN, Some(now + 10), Some(604_800)),
            DataSource::ProxyHeaders,
            now,
        );
        t.apply_at(
            &claude(),
            &UsageReport {
                quotas: vec![QuotaReading {
                    id: "requests".into(),
                    limit: Some(10.0),
                    remaining: Some(5.0),
                    resets_at: Some(i64::MAX),
                }],
                ..Default::default()
            },
            DataSource::ProxyHeaders,
            now,
        );
        {
            let state = t.state.read();
            let acct = &state.accounts["anthropic:subscription"];
            assert!(
                !acct.windows.contains_key("seven_day"),
                "NaN reading dropped"
            );
            // Only the plausible parts of the readings were kept.
            let w = &acct.windows["five_hour"].reading;
            assert_eq!(w.resets_at, Some(now + 3_600));
            assert_eq!(w.window_secs, Some(3_600));
            assert_eq!(acct.quotas["requests"].reading.resets_at, None);
        }
        // Still renders and prunes.
        t.snapshot_at(now);
        t.prune(now + RETENTION_SECS);
    }

    #[test]
    fn shutdown_writes_unsaved_changes_and_stops_maintenance() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("usage.json");
        let t = UsageTracker::load(path.clone(), UsageTrackingConfig::default());
        t.persist_now();
        // A change inside the persist spacing is not written by a tick…
        t.apply(
            &claude(),
            &week(42.0, now_secs() + 3_600),
            DataSource::CliLogin,
        );
        t.tick();
        let reloaded = UsageTracker::load(path.clone(), UsageTrackingConfig::default());
        assert!(reloaded.state.read().accounts.is_empty());
        // …but is on shutdown.
        t.shutdown();
        assert!(t.shutdown.is_cancelled());
        let reloaded = UsageTracker::load(path, UsageTrackingConfig::default());
        assert_eq!(
            reloaded.state.read().accounts["anthropic:subscription"].windows["seven_day"]
                .reading
                .used_percent,
            42.0
        );
    }

    #[tokio::test]
    async fn maintenance_task_ends_on_shutdown() {
        let t = Arc::new(UsageTracker::new(UsageTrackingConfig::default()));
        let task = t.spawn_maintenance();
        t.shutdown();
        tokio::time::timeout(std::time::Duration::from_secs(5), task)
            .await
            .expect("maintenance task stops on shutdown")
            .unwrap();
    }

    #[test]
    fn forget_removes_account() {
        let t = UsageTracker::new(UsageTrackingConfig::default());
        t.apply_at(&claude(), &week(1.0, 10), DataSource::ProxyHeaders, 0);
        assert!(t.forget("anthropic:subscription"));
        assert!(!t.forget("anthropic:subscription"));
    }
}
