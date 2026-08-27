//! Manager for dynamic tray icon graph updates

#![allow(dead_code)]

use crate::ui::tray::UpdateNotificationState;
use crate::ui::tray_format::{headline_value, metric_magnitude, usage_line};
use crate::ui::tray_graph::{
    platform_graph_config, GraphConfig, LabelMode, MultiPaneOptions, PaneContent, PaneSpec,
    StatusDotColors, TrayOverlay, GRAPH_WIDTH,
};
use chrono::{DateTime, Duration, Utc};
use lr_config::{
    normalize_tray_label, ConfigManager, TrayDisplay, TrayLabelMode, TrayLayout, TraySource,
    TrayStatsConfig, TrayStatsItem, TrayUsageMetric, UiConfig,
};
use lr_monitoring::metrics::{MetricDataPoint, MetricsCollector, UsageTotals};
use lr_providers::health_cache::AggregateHealthStatus;
use lr_types::RecordedRequest;
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;
use tauri::{AppHandle, Listener, Manager};
use tokio::sync::mpsc;
use tracing::{debug, error};

/// Determine the current tray overlay based on system state.
///
/// Priority order (highest first):
/// 1. Firewall approval pending (user action required)
/// 2. Health warning/error (provider issues)
/// 3. Update available
/// 4. None
pub fn determine_overlay(app_handle: &AppHandle, dark_mode: bool) -> TrayOverlay {
    // Highest priority: Firewall approvals pending
    let firewall_pending = app_handle
        .try_state::<Arc<lr_server::state::AppState>>()
        .is_some_and(|state| state.mcp_gateway.firewall_manager.has_pending());
    if firewall_pending {
        return TrayOverlay::FirewallPending;
    }

    // Second priority: Health warning/error
    let health_status = app_handle
        .try_state::<Arc<lr_server::state::AppState>>()
        .map(|state| state.health_cache.aggregate_status());
    if matches!(
        health_status,
        Some(AggregateHealthStatus::Yellow) | Some(AggregateHealthStatus::Red)
    ) {
        let status = health_status.unwrap();
        return TrayOverlay::Warning(StatusDotColors::for_status(status, dark_mode));
    }

    // Third priority: Update available
    let update_available = app_handle
        .try_state::<Arc<UpdateNotificationState>>()
        .is_some_and(|state| state.is_update_available());
    if update_available {
        return TrayOverlay::UpdateAvailable;
    }

    TrayOverlay::None
}

/// Number of bars per pane — matches `GRAPH_WIDTH` in tray_graph.rs.
const NUM_BUCKETS: usize = GRAPH_WIDTH as usize;

/// Maximum panes rendered into the icon (extra items still appear in the
/// tooltip and tray menu). Keeps the menu-bar item under ~110pt.
pub const MAX_PANES: usize = 6;

/// How long a cached usage snapshot stays fresh before the next update
/// tick re-queries the metrics store.
const USAGE_REFRESH_SECS: i64 = 10;

/// Minimum spacing between tray-menu rebuilds triggered by usage changes.
const MENU_REBUILD_THROTTLE_SECS: i64 = 30;

/// While idle (no activity, graph drained) the manager still wakes up at
/// this interval so rolling usage windows keep aging in the menu/title.
const IDLE_REFRESH_SECS: u64 = 60;

/// `(tray_graph_enabled, refresh_rate_secs, displayed sources)` — the
/// combination whose change resets bucket state.
type ModeKey = (bool, u64, Vec<TraySource>);

/// One graph bucket: traffic in one time slice.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Bucket {
    pub tokens: u64,
    pub requests: u64,
    /// Millionths of a dollar.
    pub cost_micro_usd: u64,
}

impl Bucket {
    fn add(&mut self, other: Bucket) {
        self.tokens += other.tokens;
        self.requests += other.requests;
        self.cost_micro_usd += other.cost_micro_usd;
    }

    fn is_zero(&self) -> bool {
        self.tokens == 0 && self.requests == 0 && self.cost_micro_usd == 0
    }

    fn value(&self, metric: TrayUsageMetric) -> u64 {
        match metric {
            TrayUsageMetric::Tokens => self.tokens,
            TrayUsageMetric::Requests => self.requests,
            TrayUsageMetric::Cost => self.cost_micro_usd,
        }
    }
}

/// Per-source sparkline state.
#[derive(Debug, Clone)]
struct SourceState {
    /// Bucket values, oldest first (26 buckets).
    buckets: Vec<Bucket>,
    /// Traffic recorded since the last update tick (Fast/Medium modes).
    accumulated: Bucket,
}

impl Default for SourceState {
    fn default() -> Self {
        Self {
            buckets: vec![Bucket::default(); NUM_BUCKETS],
            accumulated: Bucket::default(),
        }
    }
}

/// Cached usage for one displayed item, in display order.
#[derive(Debug, Clone)]
pub struct UsageEntry {
    pub source: TraySource,
    pub label: String,
    pub usage: UsageTotals,
}

/// Everything that gets pushed to the tray in one main-thread hop.
#[derive(Debug, Clone, Hash)]
struct TrayPresentation {
    icon: Vec<u8>,
    tooltip: String,
}

/// Manager for dynamic tray icon graph updates
pub struct TrayGraphManager {
    /// App handle for accessing tray and state
    app_handle: AppHandle,

    /// UI configuration
    config: Arc<RwLock<UiConfig>>,

    /// Last update timestamp for throttling visual redraws (1s)
    last_update: Arc<RwLock<Option<DateTime<Utc>>>>,

    /// Last bucket shift timestamp for controlling graph movement speed
    /// This is separate from last_update to allow immediate visual updates
    /// while only shifting buckets at the configured rate
    last_bucket_shift: Arc<RwLock<Option<DateTime<Utc>>>>,

    /// Channel for activity notifications
    activity_tx: mpsc::UnboundedSender<()>,

    /// Last activity timestamp for idle detection
    last_activity: Arc<RwLock<DateTime<Utc>>>,

    /// Sparkline state per displayed source. `All` is always present when
    /// the graph is enabled; other sources are added as they're configured.
    sources: Arc<RwLock<HashMap<TraySource, SourceState>>>,

    /// Rolling-window usage per displayed item (feeds title, tooltip, menu).
    usage: Arc<RwLock<Vec<UsageEntry>>>,
    usage_refreshed_at: Arc<RwLock<Option<DateTime<Utc>>>>,

    /// Hash of the last applied presentation to skip redundant updates
    last_presentation_hash: Arc<RwLock<u64>>,

    /// Last tooltip pushed to the menu (usage lines) and when the tray
    /// menu was last rebuilt because of it.
    last_menu_text: Arc<RwLock<String>>,
    last_menu_rebuild: Arc<RwLock<Option<DateTime<Utc>>>>,

    /// Debug override for the tray overlay (bypasses determine_overlay)
    debug_overlay_override: Arc<RwLock<Option<TrayOverlay>>>,

    /// Last rendered mode `(tray_graph_enabled, refresh_rate_secs, sources)`.
    /// Bucket contents are only meaningful at the time scale they were
    /// recorded at, so any mode change must reset bucket state — otherwise
    /// switching Fast/Medium/Slow reinterprets old bars at the new scale.
    last_mode: Arc<RwLock<Option<ModeKey>>>,
}

/// Compute how many bucket shifts are due since `last_shift`, and the new
/// `last_shift` timestamp to store if those shifts are applied.
///
/// Advances the timestamp by exact interval multiples (rather than snapping
/// to `now`) so the fractional remainder of each update isn't lost — with
/// snapping, a 1s interval polled every ~1.4s would drift ~40% slow and
/// shift erratically. When the graph is fully wiped (shifts >= num_buckets)
/// the remainder no longer matters, so the timestamp snaps to `now`.
fn compute_bucket_shifts(
    last_shift: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
    interval_ms: i64,
    num_buckets: usize,
) -> (usize, DateTime<Utc>) {
    match last_shift {
        None => (0, now),
        Some(ts) => {
            let elapsed_ms = now.signed_duration_since(ts).num_milliseconds();
            let shifts = (elapsed_ms / interval_ms).max(0) as usize;
            if shifts == 0 {
                (0, ts)
            } else if shifts >= num_buckets {
                (shifts, now)
            } else {
                (
                    shifts,
                    ts + Duration::milliseconds(shifts as i64 * interval_ms),
                )
            }
        }
    }
}

/// Shift buckets left by `shifts`, zero-filling the vacated slots.
fn shift_buckets(buckets: &mut [Bucket], shifts: usize) {
    let n = buckets.len();
    if shifts >= n {
        buckets.fill(Bucket::default());
    } else if shifts > 0 {
        buckets.rotate_left(shifts);
        for b in &mut buckets[n - shifts..] {
            *b = Bucket::default();
        }
    }
}

/// Seed Medium-mode buckets (10s each) by spreading each minute metric
/// forward across 6 buckets.
fn seed_medium_buckets(buckets: &mut [Bucket], metrics: &[MetricDataPoint], now: DateTime<Utc>) {
    let n = buckets.len() as i64;
    let window_secs = n * 10;
    buckets.fill(Bucket::default());

    for metric in metrics {
        let age_secs = now.signed_duration_since(metric.timestamp).num_seconds();
        if age_secs < 0 || age_secs >= window_secs {
            continue;
        }

        // Determine how many buckets we can actually place (some might fall outside window)
        let num_in_window = (0..6)
            .filter(|&offset| {
                let bucket_age = age_secs.saturating_sub(offset * 10);
                bucket_age >= 0 && bucket_age < window_secs
            })
            .count() as u64;
        if num_in_window == 0 {
            continue;
        }

        let per_bucket = Bucket {
            tokens: metric.total_tokens / num_in_window,
            requests: metric.requests / num_in_window,
            cost_micro_usd: RecordedRequest::micro_usd(metric.cost_usd) / num_in_window,
        };

        for offset in 0..6 {
            // Spread the minute forward in time (subtract offset, not add)
            let bucket_age_secs = age_secs.saturating_sub(offset * 10);
            if bucket_age_secs < 0 || bucket_age_secs >= window_secs {
                continue;
            }
            let idx = ((n - 1) - (bucket_age_secs / 10)).clamp(0, n - 1) as usize;
            buckets[idx].add(per_bucket);
        }
    }
}

/// Fill Slow-mode buckets (1 min each) directly from minute metrics.
fn fill_slow_buckets(buckets: &mut [Bucket], metrics: &[MetricDataPoint], now: DateTime<Utc>) {
    let n = buckets.len() as i64;
    let window_secs = n * 60;
    buckets.fill(Bucket::default());

    for metric in metrics {
        let age_secs = now.signed_duration_since(metric.timestamp).num_seconds();
        if age_secs < 0 || age_secs >= window_secs {
            continue;
        }
        let idx = ((n - 1) - (age_secs / 60)).clamp(0, n - 1) as usize;
        buckets[idx].add(Bucket {
            tokens: metric.total_tokens,
            requests: metric.requests,
            cost_micro_usd: RecordedRequest::micro_usd(metric.cost_usd),
        });
    }
}

/// Minute metrics for one source over `[start, end]`.
fn metrics_for_source(
    collector: &MetricsCollector,
    source: &TraySource,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Vec<MetricDataPoint> {
    match source {
        TraySource::All => collector.get_global_range(start, end),
        TraySource::Client { id } => collector.get_key_range(id, start, end),
        TraySource::Provider { instance } => collector.get_provider_range(instance, start, end),
        TraySource::Model { id } => collector.get_model_range(id, start, end),
    }
}

/// Whether a recorded request counts towards `source`.
fn source_matches(source: &TraySource, req: &RecordedRequest) -> bool {
    match source {
        TraySource::All => true,
        TraySource::Client { id } => id == &req.client_id,
        TraySource::Provider { instance } => instance == &req.provider,
        TraySource::Model { id } => id == &req.model,
    }
}

/// Layout the platform can actually render when the user leaves it on Auto.
///
/// Wide (multi-pane) icons and title text work on macOS and on GNOME's
/// AppIndicator extension; Windows squashes non-square icons and ignores
/// titles, and KDE shrinks wide icons into a square cell.
pub fn platform_default_layout() -> TrayLayout {
    if cfg!(target_os = "macos") {
        TrayLayout::Extended
    } else if cfg!(target_os = "linux") {
        let desktop = std::env::var("XDG_CURRENT_DESKTOP")
            .unwrap_or_default()
            .to_ascii_lowercase();
        if ["gnome", "unity", "cinnamon"]
            .iter()
            .any(|d| desktop.contains(d))
        {
            TrayLayout::Extended
        } else {
            TrayLayout::Compact
        }
    } else {
        TrayLayout::Compact
    }
}

/// Resolve the configured layout to a concrete one.
pub fn effective_layout(config: &TrayStatsConfig) -> TrayLayout {
    match config.layout {
        TrayLayout::Auto => platform_default_layout(),
        other => other,
    }
}

/// Panel label for an item: the user's label if set, else derived from the
/// item's name. Never empty — falls back to the source key.
pub fn resolve_label(item: &TrayStatsItem, client_name: Option<&str>) -> String {
    if let Some(custom) = item.label.as_deref() {
        let normalized = normalize_tray_label(custom);
        if !normalized.is_empty() {
            return normalized;
        }
    }
    let derived = normalize_tray_label(item.source.default_label_seed(client_name));
    if !derived.is_empty() {
        return derived;
    }
    normalize_tray_label(&item.source.key())
}

/// Enabled items that can currently be displayed (clients that no longer
/// exist are skipped), with their resolved labels.
fn displayable_items(
    stats: &TrayStatsConfig,
    clients: &[lr_config::Client],
) -> Vec<(TrayStatsItem, String)> {
    stats
        .enabled_items()
        .filter_map(|item| {
            let client_name = match &item.source {
                TraySource::Client { id } => {
                    Some(clients.iter().find(|c| &c.id == id)?.name.as_str())
                }
                _ => None,
            };
            Some((item.clone(), resolve_label(item, client_name)))
        })
        .collect()
}

impl TrayGraphManager {
    /// Create a new tray graph manager
    ///
    /// Starts a background task that listens for activity notifications
    /// and updates the tray icon graph at the configured interval.
    pub fn new(app_handle: AppHandle, config: UiConfig) -> Self {
        let (activity_tx, mut activity_rx) = mpsc::unbounded_channel();

        let config = Arc::new(RwLock::new(config));
        let last_update = Arc::new(RwLock::new(None::<DateTime<Utc>>));
        let last_bucket_shift = Arc::new(RwLock::new(None::<DateTime<Utc>>));
        let last_activity = Arc::new(RwLock::new(Utc::now()));
        let sources = Arc::new(RwLock::new(HashMap::<TraySource, SourceState>::new()));
        let usage = Arc::new(RwLock::new(Vec::<UsageEntry>::new()));
        let usage_refreshed_at = Arc::new(RwLock::new(None::<DateTime<Utc>>));
        let last_presentation_hash = Arc::new(RwLock::new(0u64));
        let last_menu_text = Arc::new(RwLock::new(String::new()));
        let last_menu_rebuild = Arc::new(RwLock::new(None::<DateTime<Utc>>));
        let debug_overlay_override = Arc::new(RwLock::new(None::<TrayOverlay>));
        let last_mode = Arc::new(RwLock::new(None::<ModeKey>));

        let manager = Self {
            app_handle: app_handle.clone(),
            config,
            last_update,
            last_bucket_shift,
            activity_tx: activity_tx.clone(),
            last_activity,
            sources,
            usage,
            usage_refreshed_at,
            last_presentation_hash,
            last_menu_text,
            last_menu_rebuild,
            debug_overlay_override,
            last_mode,
        };

        // Spawn background task with idle-aware timer for smooth graph shifting
        let task = manager.clone_handles();
        tauri::async_runtime::spawn(async move {
            debug!("TrayGraphManager background task started");

            const UPDATE_CHECK_INTERVAL_MS: u64 = 500;

            loop {
                // Wait for activity — or an idle tick so rolling usage
                // windows keep aging in the title/tooltip/menu.
                let idle_tick =
                    tokio::time::sleep(tokio::time::Duration::from_secs(IDLE_REFRESH_SECS));
                tokio::select! {
                    msg = activity_rx.recv() => {
                        if msg.is_none() {
                            debug!("TrayGraphManager: Channel closed, exiting");
                            break;
                        }
                        // Activity detected, update timestamp
                        *task.last_activity.write() = Utc::now();
                    }
                    _ = idle_tick => {
                        if let Err(e) = task.update_once().await {
                            error!("Failed to refresh tray on idle tick: {}", e);
                        }
                        continue;
                    }
                }

                // Start timer loop for active period
                let mut interval = tokio::time::interval(tokio::time::Duration::from_millis(
                    UPDATE_CHECK_INTERVAL_MS,
                ));
                interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

                // Keep updating while active (not idle)
                loop {
                    // Check for new activity notifications (non-blocking)
                    while let Ok(()) = activity_rx.try_recv() {
                        *task.last_activity.write() = Utc::now();
                        debug!(
                            "TrayGraphManager: Activity notification received during update loop"
                        );
                    }

                    interval.tick().await;

                    // Stop the update loop once no activity is happening AND
                    // the graph has fully drained (all bars are zero).
                    let no_recent_activity = {
                        let last = task.last_activity.read();
                        Utc::now().signed_duration_since(*last).num_seconds() >= 5
                    };
                    if no_recent_activity {
                        let graph_empty = task.sources.read().values().all(|s| {
                            s.buckets.iter().all(Bucket::is_zero) && s.accumulated.is_zero()
                        });
                        if graph_empty {
                            break;
                        }
                    }

                    // Visual updates happen every 1 second for responsiveness
                    // Bucket shifting is controlled separately in update_tray_graph_impl
                    const VISUAL_UPDATE_THROTTLE_MS: i64 = 1000;

                    let should_update = match *task.last_update.read() {
                        None => true,
                        Some(last_ts) => {
                            Utc::now().signed_duration_since(last_ts).num_milliseconds()
                                >= VISUAL_UPDATE_THROTTLE_MS
                        }
                    };
                    if !should_update {
                        continue;
                    }

                    if let Err(e) = task.update_once().await {
                        error!("Failed to update tray graph: {}", e);
                    }
                }
            }

            debug!("TrayGraphManager background task stopped");
        });

        // Subscribe to health status changes to refresh the tray icon
        // when health status changes (even when idle)
        let activity_tx_health = activity_tx;
        app_handle.listen("health-status-changed", move |_event| {
            debug!("TrayGraphManager: Health status changed, refreshing tray icon");
            if let Err(e) = activity_tx_health.send(()) {
                error!("Failed to send health activity notification: {}", e);
            }
        });

        // Trigger initial update to render tray icon (static or graph mode)
        manager.notify_activity();

        manager
    }

    /// A second handle onto the same shared state for the background task.
    fn clone_handles(&self) -> Self {
        Self {
            app_handle: self.app_handle.clone(),
            config: self.config.clone(),
            last_update: self.last_update.clone(),
            last_bucket_shift: self.last_bucket_shift.clone(),
            activity_tx: self.activity_tx.clone(),
            last_activity: self.last_activity.clone(),
            sources: self.sources.clone(),
            usage: self.usage.clone(),
            usage_refreshed_at: self.usage_refreshed_at.clone(),
            last_presentation_hash: self.last_presentation_hash.clone(),
            last_menu_text: self.last_menu_text.clone(),
            last_menu_rebuild: self.last_menu_rebuild.clone(),
            debug_overlay_override: self.debug_overlay_override.clone(),
            last_mode: self.last_mode.clone(),
        }
    }

    /// Run one update tick and stamp `last_update`.
    async fn update_once(&self) -> anyhow::Result<()> {
        self.update_tray_graph_impl().await?;
        *self.last_update.write() = Some(Utc::now());
        Ok(())
    }

    /// Notify that new activity has occurred (metrics recorded)
    ///
    /// This triggers the throttled update cycle.
    pub fn notify_activity(&self) {
        // Update last activity time
        *self.last_activity.write() = Utc::now();

        debug!("TrayGraphManager: Activity notification received");

        // Send notification (non-blocking)
        if let Err(e) = self.activity_tx.send(()) {
            error!("Failed to send activity notification: {}", e);
        }
    }

    /// Set a debug overlay override, bypassing `determine_overlay`.
    /// Pass `None` to clear the override and return to normal behavior.
    pub fn set_debug_overlay(&self, overlay: Option<TrayOverlay>) {
        *self.debug_overlay_override.write() = overlay;
        // Reset hash to force an immediate icon update
        *self.last_presentation_hash.write() = 0;
        self.notify_activity();
    }

    /// Record a completed request.
    ///
    /// Accumulates tokens/requests for every displayed source the request
    /// belongs to (global, its client, its provider, its model) so Fast /
    /// Medium modes show real-time activity without querying minute metrics.
    pub fn record_request(&self, req: &RecordedRequest) {
        {
            let mut sources = self.sources.write();
            for (source, state) in sources.iter_mut() {
                if source_matches(source, req) {
                    state.accumulated.tokens += req.tokens;
                    state.accumulated.requests += 1;
                    state.accumulated.cost_micro_usd += req.cost_micro_usd;
                }
            }
        }

        // Trigger update cycle
        self.notify_activity();
    }

    /// Cached usage snapshot for the displayed items (display order).
    /// Used by the tray menu; refreshed synchronously if stale so the
    /// menu never shows nothing on first open.
    pub fn usage_entries(&self) -> Vec<UsageEntry> {
        let stale = match *self.usage_refreshed_at.read() {
            None => true,
            Some(ts) => Utc::now().signed_duration_since(ts).num_seconds() >= USAGE_REFRESH_SECS,
        };
        if stale {
            self.refresh_usage(Utc::now());
        }
        self.usage.read().clone()
    }

    /// Current stats config.
    pub fn stats_config(&self) -> TrayStatsConfig {
        self.config.read().tray_stats.clone()
    }

    /// Re-query rolling-window usage for every enabled item.
    fn refresh_usage(&self, now: DateTime<Utc>) {
        let Some(config_manager) = self.app_handle.try_state::<ConfigManager>() else {
            return;
        };
        let Some(metrics_collector) = self.app_handle.try_state::<Arc<MetricsCollector>>() else {
            return;
        };
        let app_config = config_manager.get();
        let stats = &app_config.ui.tray_stats;
        let window = stats.usage_period.seconds();

        let entries: Vec<UsageEntry> = displayable_items(stats, &app_config.clients)
            .into_iter()
            .map(|(item, label)| UsageEntry {
                usage: metrics_collector.get_usage_for_type(&item.source.metric_type(), window),
                source: item.source,
                label,
            })
            .collect();

        *self.usage.write() = entries;
        *self.usage_refreshed_at.write() = Some(now);
    }

    /// Apply a freshly-rendered presentation to the tray on the main thread.
    ///
    /// Tray / menu-bar mutation is AppKit UI work that must run on the main
    /// thread. This manager renders and updates from a background task, and
    /// doing the `set_icon` off-thread made the icon flash "undrawn" while
    /// redrawing the next bucket. Dispatching to the main thread fixes that,
    /// and bundling `set_icon` + `set_icon_as_template` (+ title + tooltip)
    /// into a single closure makes the update atomic so the menu bar never
    /// repaints with a half-applied icon in between.
    fn apply_presentation(app_handle: &AppHandle, p: TrayPresentation) -> anyhow::Result<()> {
        let app = app_handle.clone();
        app_handle
            .run_on_main_thread(move || {
                let Some(tray) = app.tray_by_id("main") else {
                    return;
                };
                let icon = match tauri::image::Image::from_bytes(&p.icon) {
                    Ok(icon) => icon,
                    Err(e) => {
                        error!("Failed to create tray image: {}", e);
                        return;
                    }
                };
                if let Err(e) = tray.set_icon(Some(icon)) {
                    error!("Failed to set tray icon: {}", e);
                    return;
                }
                // Each new NSImage defaults to non-template, so the template
                // flag must be re-applied after every icon swap. On macOS this
                // lets the menu bar recolor for the current appearance.
                if let Err(e) = tray.set_icon_as_template(cfg!(target_os = "macos")) {
                    error!("Failed to set tray template mode: {}", e);
                }
                // Numbers live inside the icon now; make sure no title
                // text from an earlier build lingers beside it.
                if let Err(e) = tray.set_title(None::<&str>) {
                    error!("Failed to clear tray title: {}", e);
                }
                if let Err(e) = tray.set_tooltip(Some(&p.tooltip)) {
                    error!("Failed to set tray tooltip: {}", e);
                }
            })
            .map_err(|e| {
                anyhow::anyhow!("Failed to dispatch tray icon update to main thread: {}", e)
            })
    }

    /// Update sparkline buckets for every displayed source and return the
    /// bar values per source in `sources` order.
    ///
    /// Modes:
    /// - Fast (1s): real-time accumulation only (no metrics)
    /// - Medium (10s): metrics for initial load, then real-time accumulation
    /// - Slow (60s): always minute-level metrics (1:1 mapping)
    ///
    /// Visual updates happen every 1 second for responsiveness, but bucket
    /// shifting only occurs at the configured refresh rate (1s/10s/60s).
    fn update_buckets(
        &self,
        metrics_collector: &MetricsCollector,
        display_sources: &[TraySource],
        refresh_rate_secs: u64,
        mode_changed: bool,
        metric: TrayUsageMetric,
        now: DateTime<Utc>,
    ) -> Vec<Vec<u64>> {
        let mut sources = self.sources.write();

        // Drop state for sources that are no longer displayed.
        sources.retain(|s, _| display_sources.contains(s));

        // (Re)initialize bucket state on the first update after startup
        // and whenever the mode changes.
        let needs_init = mode_changed || self.last_bucket_shift.read().is_none();

        // Calculate how many bucket shifts are due since the last shift.
        // During normal operation this is 1; after an idle gap it can be
        // many, which lets the graph catch up instantly instead of
        // draining one bar at a time.
        let interval_ms = refresh_rate_secs.max(1) as i64 * 1000;
        let (shifts_needed, next_shift_ts) = if needs_init {
            (0, now)
        } else {
            compute_bucket_shifts(
                *self.last_bucket_shift.read(),
                now,
                interval_ms,
                NUM_BUCKETS,
            )
        };

        let mut bars = Vec::with_capacity(display_sources.len());

        for source in display_sources {
            let state = sources.entry(source.clone()).or_default();

            match refresh_rate_secs {
                // Fast mode: 1 second per bar, 26 second total.
                // NO metrics querying - pure real-time tracking.
                1 => {
                    if needs_init {
                        // Start with empty buckets (no historical data) and
                        // discard any backlog accumulated under the previous
                        // mode — it would otherwise render as one giant spike.
                        state.buckets.fill(Bucket::default());
                        state.accumulated = Bucket::default();
                    } else {
                        shift_buckets(&mut state.buckets, shifts_needed);
                    }
                    // Always add accumulated traffic to the rightmost bucket
                    let acc = std::mem::take(&mut state.accumulated);
                    state.buckets[NUM_BUCKETS - 1].add(acc);
                }

                // Medium mode: 10 seconds per bar, 260 seconds total.
                // Initial load interpolates minute data; then in-memory shifting.
                10 => {
                    if needs_init {
                        let window_secs = NUM_BUCKETS as i64 * 10;
                        let start = now - Duration::seconds(window_secs + 120);
                        let metrics = metrics_for_source(metrics_collector, source, start, now);
                        seed_medium_buckets(&mut state.buckets, &metrics, now);
                        // The interpolated metrics already include recently
                        // recorded traffic, so drop the accumulator to avoid
                        // double-counting it in the rightmost bucket.
                        state.accumulated = Bucket::default();
                    } else {
                        shift_buckets(&mut state.buckets, shifts_needed);
                    }
                    let acc = std::mem::take(&mut state.accumulated);
                    state.buckets[NUM_BUCKETS - 1].add(acc);
                }

                // Slow mode: 1 minute per bar, 26 minute total.
                // Direct mapping: one minute of metrics → one bar.
                _ => {
                    // Slow mode reads the metrics store directly, which
                    // already contains every recorded request — drain the
                    // real-time accumulator so it can't pile up and dump a
                    // giant spike into the graph on a later mode switch.
                    state.accumulated = Bucket::default();
                    let window_secs = NUM_BUCKETS as i64 * 60;
                    let start = now - Duration::seconds(window_secs + 120);
                    let metrics = metrics_for_source(metrics_collector, source, start, now);
                    fill_slow_buckets(&mut state.buckets, &metrics, now);
                }
            }

            bars.push(state.buckets.iter().map(|b| b.value(metric)).collect());
        }

        // Advance the shift clock once for all sources.
        if needs_init || refresh_rate_secs >= 60 {
            *self.last_bucket_shift.write() = Some(now);
        } else if shifts_needed > 0 {
            *self.last_bucket_shift.write() = Some(next_shift_ts);
        }

        bars
    }

    /// Build the pane list for `display_items` under `stats`.
    fn build_panes(
        stats: &TrayStatsConfig,
        display_items: &[(TrayStatsItem, String)],
        bars: &[Vec<u64>],
        usage_for: &dyn Fn(&TraySource) -> UsageTotals,
    ) -> Vec<PaneSpec> {
        let max_magnitude = display_items
            .iter()
            .map(|(i, _)| metric_magnitude(&usage_for(&i.source), stats.metric))
            .fold(0.0_f64, f64::max);

        display_items
            .iter()
            .enumerate()
            .map(|(idx, (item, label))| {
                let usage = usage_for(&item.source);
                let content = match stats.display {
                    TrayDisplay::Graph => {
                        PaneContent::Graph(bars.get(idx).cloned().unwrap_or_default())
                    }
                    TrayDisplay::UsageBar => PaneContent::UsageBar(if max_magnitude > 0.0 {
                        (metric_magnitude(&usage, stats.metric) / max_magnitude) as f32
                    } else {
                        0.0
                    }),
                    TrayDisplay::Number => {
                        PaneContent::Number(headline_value(&usage, stats.metric).to_uppercase())
                    }
                };
                PaneSpec {
                    label: Some(label.clone()),
                    content,
                }
            })
            .collect()
    }

    fn pane_options(stats: &TrayStatsConfig, extended: bool) -> MultiPaneOptions {
        MultiPaneOptions {
            labels: if !extended {
                LabelMode::Off
            } else {
                match stats.labels {
                    TrayLabelMode::Off => LabelMode::Off,
                    TrayLabelMode::Beside => LabelMode::Beside,
                    TrayLabelMode::Above => LabelMode::Above,
                }
            },
            units_per_pixel: match stats.metric {
                TrayUsageMetric::Tokens => Some(crate::ui::tray_graph::TOKENS_PER_PIXEL),
                TrayUsageMetric::Requests | TrayUsageMetric::Cost => None,
            },
        }
    }

    /// Render what the tray icon would look like under `stats` (which need
    /// not be saved) with dummy data, for the settings preview. `tick`
    /// advances the dummy series so the preview animates like the real
    /// graph shifts. Drawn white or black on transparent for the UI theme.
    /// Returns PNG bytes; `None` if nothing can be drawn.
    pub fn render_preview(
        &self,
        stats: &TrayStatsConfig,
        dark_ui: bool,
        tick: u64,
    ) -> Option<Vec<u8>> {
        let config_manager = self.app_handle.try_state::<ConfigManager>()?;
        let app_config = config_manager.get();
        let extended = effective_layout(stats) == TrayLayout::Extended;
        let items = displayable_items(stats, &app_config.clients);
        let extended = extended && !items.is_empty();
        let display_items: Vec<(TrayStatsItem, String)> = if extended {
            items.into_iter().take(MAX_PANES).collect()
        } else {
            vec![(
                TrayStatsItem::new(TraySource::All),
                resolve_label(&TrayStatsItem::new(TraySource::All), None),
            )]
        };

        // Dummy traffic: a bumpy, item-specific series that scrolls with
        // `tick`, in the configured metric's units.
        let unit: u64 = match stats.metric {
            TrayUsageMetric::Tokens => 25,
            TrayUsageMetric::Requests => 1,
            TrayUsageMetric::Cost => 20_000, // micro-dollars
        };
        let sample = |i: u64, idx: u64| -> u64 {
            let t = i + tick;
            let wave = ((t * 7 + idx * 5) % 11) + ((t * 3 + idx * 13) % 7);
            let burst = if (t + idx * 4).is_multiple_of(9) {
                12
            } else {
                0
            };
            (wave + burst + 1) * unit
        };
        let bars: Vec<Vec<u64>> = (0..display_items.len() as u64)
            .map(|idx| (0..NUM_BUCKETS as u64).map(|i| sample(i, idx)).collect())
            .collect();

        // Dummy usage totals: the first three items show ~$12, a `k` value
        // and cents (so every formatting shape is visible), the rest vary.
        // Each tick nudges them up a little so the preview visibly moves.
        const BASE_COST_USD: [f64; 6] = [12.0, 1_200.0, 0.07, 340.0, 0.5, 85_000.0];
        const USD_PER_TOKEN: f64 = 0.000_004;
        let growth = 1.0 + 0.01 * (tick % 30) as f64;
        let usage_map: HashMap<TraySource, UsageTotals> = display_items
            .iter()
            .enumerate()
            .map(|(idx, (item, _))| {
                let cost_usd = BASE_COST_USD[idx % BASE_COST_USD.len()] * growth;
                let tokens = (cost_usd / USD_PER_TOKEN).round() as u64;
                (
                    item.source.clone(),
                    UsageTotals {
                        requests: (tokens / 40).max(1),
                        tokens,
                        cost_usd,
                    },
                )
            })
            .collect();
        let usage_for = |s: &TraySource| usage_map.get(s).copied().unwrap_or_default();

        let panes = Self::build_panes(stats, &display_items, &bars, &usage_for);
        let fg = if dark_ui { 255 } else { 0 };
        let config = GraphConfig {
            foreground: image::Rgba([fg, fg, fg, 255]),
            background: image::Rgba([0, 0, 0, 0]),
            template_mode: true,
        };
        crate::ui::tray_graph::generate_multi_pane(
            &panes,
            Self::pane_options(stats, extended),
            &config,
            TrayOverlay::None,
            dark_ui,
        )
    }

    /// One full update: buckets → usage → render → push to tray (+ menu).
    /// Skips the tray push if nothing changed since the last one.
    async fn update_tray_graph_impl(&self) -> Result<(), anyhow::Error> {
        let app_handle = &self.app_handle;

        // Get config and metrics collector from state
        let config_manager = app_handle
            .try_state::<ConfigManager>()
            .ok_or_else(|| anyhow::anyhow!("ConfigManager not in app state"))?;

        let metrics_collector = app_handle
            .try_state::<Arc<MetricsCollector>>()
            .ok_or_else(|| anyhow::anyhow!("MetricsCollector not in app state"))?;

        let app_config = config_manager.get();
        let ui_config = app_config.ui.clone();
        let tray_graph_enabled = ui_config.tray_graph_enabled;
        let refresh_rate_secs = ui_config.tray_graph_refresh_rate_secs;
        let stats = &ui_config.tray_stats;
        let extended = effective_layout(stats) == TrayLayout::Extended;
        let now = Utc::now();

        // Items to present. Extended layout shows every enabled item (icon
        // capped at MAX_PANES); Compact keeps the single global pane.
        let items = displayable_items(stats, &app_config.clients);
        // An Extended layout with every item unchecked falls back to the
        // single global pane rather than rendering nothing.
        let extended = extended && !items.is_empty();
        let display_items: Vec<(TrayStatsItem, String)> = if extended {
            items.iter().take(MAX_PANES).cloned().collect()
        } else {
            vec![(
                TrayStatsItem::new(TraySource::All),
                resolve_label(&TrayStatsItem::new(TraySource::All), None),
            )]
        };
        let display_sources: Vec<TraySource> = display_items
            .iter()
            .map(|(i, _)| i.source.clone())
            .collect();

        // Detect mode changes (graph toggled, refresh rate switched, or the
        // displayed source set changed). Any change re-initializes bucket state.
        let mode_changed = {
            let mut last = self.last_mode.write();
            let current = (
                tray_graph_enabled,
                refresh_rate_secs,
                display_sources.clone(),
            );
            let changed = last.as_ref() != Some(&current);
            *last = Some(current);
            changed
        };

        // Sparkline data (only when the graph is on).
        let bars: Vec<Vec<u64>> = if tray_graph_enabled {
            self.update_buckets(
                &metrics_collector,
                &display_sources,
                refresh_rate_secs,
                mode_changed,
                stats.metric,
                now,
            )
        } else {
            // Static mode: drop any bucket state so the background loop's
            // "graph empty" idle check can stop the update timer.
            self.sources.write().clear();
            Vec::new()
        };

        // Rolling-window usage (title / tooltip / usage bar / menu).
        let usage_stale = mode_changed
            || match *self.usage_refreshed_at.read() {
                None => true,
                Some(ts) => now.signed_duration_since(ts).num_seconds() >= USAGE_REFRESH_SECS,
            };
        if usage_stale {
            self.refresh_usage(now);
        }
        let usage_entries = self.usage.read().clone();
        let usage_for = |source: &TraySource| -> UsageTotals {
            usage_entries
                .iter()
                .find(|e| &e.source == source)
                .map(|e| e.usage)
                .unwrap_or_default()
        };

        // Detect if system is in dark mode for color adjustments
        let dark_mode = detect_dark_mode(app_handle);

        // Clean up expired firewall approval requests and close their popups
        if let Some(app_state) = app_handle.try_state::<Arc<lr_server::state::AppState>>() {
            let expired_requests = app_state.mcp_gateway.firewall_manager.cleanup_expired();
            if !expired_requests.is_empty() {
                debug!(
                    "Cleaned up {} expired firewall approval requests",
                    expired_requests.len()
                );
                // Close any popup windows for expired requests
                for request_id in &expired_requests {
                    if let Some(window) =
                        app_handle.get_webview_window(&format!("firewall-approval-{}", request_id))
                    {
                        let _ = window.close();
                        debug!("Closed popup for expired firewall request {}", request_id);
                    }
                }
                // Rebuild tray menu to remove expired items
                if let Err(e) = crate::ui::tray::rebuild_tray_menu(app_handle) {
                    error!("Failed to rebuild tray menu after firewall cleanup: {}", e);
                }
            }

            // Clean up expired sampling approval requests and close their popups
            app_state.sampling_approval_manager.cleanup_expired();

            // Clean up expired elicitation requests
            app_state
                .mcp_gateway
                .get_elicitation_manager()
                .cleanup_expired();
        }

        // Determine overlay: debug override takes precedence, then normal priority
        let overlay = {
            let debug_override = self.debug_overlay_override.read();
            if let Some(ref ov) = *debug_override {
                ov.clone()
            } else {
                determine_overlay(app_handle, dark_mode)
            }
        };

        // ---- Icon ----
        let icon_bytes = if !tray_graph_enabled {
            // Static mode: theme-recolored graphic on transparent background
            // (never the graph frame). On macOS template mode handles
            // appearance-aware recoloring; on Windows/Linux the bytes are
            // rendered as-is.
            const STATIC_ICON: &[u8] = include_bytes!("../../icons/32x32.png");
            if overlay == TrayOverlay::None {
                crate::ui::tray_graph::generate_static_icon(STATIC_ICON, dark_mode)
                    .ok_or_else(|| anyhow::anyhow!("Failed to generate static icon"))?
            } else {
                crate::ui::tray_graph::generate_static_icon_with_overlay(
                    STATIC_ICON,
                    overlay,
                    dark_mode,
                )
                .ok_or_else(|| anyhow::anyhow!("Failed to generate static icon with overlay"))?
            }
        } else {
            let panes = Self::build_panes(stats, &display_items, &bars, &usage_for);
            let graph_config = platform_graph_config(dark_mode);
            crate::ui::tray_graph::generate_multi_pane(
                &panes,
                Self::pane_options(stats, extended),
                &graph_config,
                overlay,
                dark_mode,
            )
            .ok_or_else(|| anyhow::anyhow!("Failed to generate graph PNG"))?
        };

        // ---- Tooltip (legend: every enabled item, not just the panes) ----
        let mut tooltip_lines = vec![format!("LocalRouter · {}", stats.usage_period.label())];
        tooltip_lines.extend(
            usage_entries
                .iter()
                .map(|e| usage_line(&e.label, &e.usage, stats.metric, stats.usage_period)),
        );
        let tooltip = tooltip_lines.join("\n");

        // ---- Tray menu usage section (throttled) ----
        {
            let menu_text: String = tooltip_lines[1..].join("\n");
            let changed = *self.last_menu_text.read() != menu_text;
            if changed {
                let due = match *self.last_menu_rebuild.read() {
                    None => true,
                    Some(ts) => {
                        now.signed_duration_since(ts).num_seconds() >= MENU_REBUILD_THROTTLE_SECS
                    }
                };
                if due {
                    *self.last_menu_text.write() = menu_text;
                    *self.last_menu_rebuild.write() = Some(now);
                    if let Err(e) = crate::ui::tray::rebuild_tray_menu(app_handle) {
                        error!("Failed to rebuild tray menu with usage: {}", e);
                    }
                }
            }
        }

        let presentation = TrayPresentation {
            icon: icon_bytes,
            tooltip,
        };

        // Skip the (main-thread) tray push if nothing visible changed
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        presentation.hash(&mut hasher);
        let current_hash = hasher.finish();
        {
            let last_hash = *self.last_presentation_hash.read();
            if last_hash == current_hash && last_hash != 0 {
                return Ok(());
            }
        }

        Self::apply_presentation(app_handle, presentation)?;
        *self.last_presentation_hash.write() = current_hash;

        debug!(
            "Tray icon updated ({} panes, graph={}, extended={})",
            display_sources.len(),
            tray_graph_enabled,
            extended
        );

        Ok(())
    }

    /// Update configuration and apply immediately
    pub fn update_config(&self, new_config: UiConfig) {
        *self.config.write() = new_config;
        // Usage labels / period may have changed — refresh on next tick and
        // let the menu rebuild right away.
        *self.usage_refreshed_at.write() = None;
        *self.last_menu_rebuild.write() = None;
        *self.last_presentation_hash.write() = 0;

        // Trigger an immediate update to apply new settings
        self.notify_activity();
    }

    /// Check if the manager has been idle (no activity for >60 seconds)
    pub fn is_idle(&self) -> bool {
        let last_activity = *self.last_activity.read();
        let elapsed = Utc::now().signed_duration_since(last_activity);
        elapsed.num_seconds() > 60
    }

    /// Check if tray graph feature is enabled
    pub fn is_enabled(&self) -> bool {
        self.config.read().tray_graph_enabled
    }
}

impl lr_types::TokenRecorder for TrayGraphManager {
    fn record_request(&self, request: &RecordedRequest) {
        self.record_request(request);
    }
}

/// Detect if the system is in dark mode
///
/// On macOS the tray icon is rendered as a template image, so the menu bar
/// handles inversion automatically and dark-mode detection doesn't apply —
/// returning `false` produces a canonical black-on-transparent template.
///
/// On Windows/Linux this still uses the main window's theme (or defaults to
/// light) to pick foreground colors.
pub fn detect_dark_mode(app_handle: &AppHandle) -> bool {
    #[cfg(target_os = "macos")]
    {
        let _ = app_handle;
        false
    }

    #[cfg(not(target_os = "macos"))]
    {
        if let Some(window) = app_handle.get_webview_window("main") {
            if let Ok(theme) = window.theme() {
                return theme == tauri::Theme::Dark;
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::compute_bucket_shifts;
    use super::*;
    use chrono::{DateTime, Duration, Timelike, Utc};
    use lr_monitoring::metrics::MetricDataPoint;

    fn point(ts: DateTime<Utc>, tokens: u64, requests: u64) -> MetricDataPoint {
        MetricDataPoint {
            timestamp: ts,
            requests,
            input_tokens: tokens,
            output_tokens: 0,
            total_tokens: tokens,
            cost_usd: 0.0,
            total_latency_ms: 0,
            successful_requests: requests,
            failed_requests: 0,
            latency_samples: vec![],
            p50_latency_ms: None,
            p95_latency_ms: None,
            p99_latency_ms: None,
        }
    }

    #[test]
    fn shift_buckets_rotates_and_zero_fills() {
        let mut b: Vec<Bucket> = (1..=4)
            .map(|i| Bucket {
                tokens: i,
                requests: 1,
                cost_micro_usd: 0,
            })
            .collect();
        shift_buckets(&mut b, 1);
        assert_eq!(
            b.iter().map(|x| x.tokens).collect::<Vec<_>>(),
            vec![2, 3, 4, 0]
        );
        shift_buckets(&mut b, 0);
        assert_eq!(
            b.iter().map(|x| x.tokens).collect::<Vec<_>>(),
            vec![2, 3, 4, 0]
        );
        shift_buckets(&mut b, 10);
        assert!(b.iter().all(Bucket::is_zero));
    }

    #[test]
    fn slow_buckets_map_minutes_one_to_one() {
        let now = Utc::now();
        let mut b = vec![Bucket::default(); NUM_BUCKETS];
        let metrics = vec![
            point(now - Duration::seconds(30), 100, 2), // current minute → last bucket
            point(now - Duration::seconds(90), 50, 1),  // one minute ago
            point(now - Duration::seconds(NUM_BUCKETS as i64 * 60 + 5), 999, 9), // too old
        ];
        fill_slow_buckets(&mut b, &metrics, now);
        assert_eq!(
            b[NUM_BUCKETS - 1],
            Bucket {
                tokens: 100,
                requests: 2,
                cost_micro_usd: 0,
            }
        );
        assert_eq!(
            b[NUM_BUCKETS - 2],
            Bucket {
                tokens: 50,
                requests: 1,
                cost_micro_usd: 0,
            }
        );
        assert_eq!(b.iter().map(|x| x.tokens).sum::<u64>(), 150);
    }

    #[test]
    fn medium_buckets_spread_minute_forward_across_six() {
        let now = Utc::now();
        let mut b = vec![Bucket::default(); NUM_BUCKETS];
        // A minute recorded 100s ago spreads to 100,90,...,50s ago
        fill_medium_for_test(&mut b, &[point(now - Duration::seconds(100), 600, 6)], now);
        let filled: Vec<usize> = b
            .iter()
            .enumerate()
            .filter(|(_, x)| !x.is_zero())
            .map(|(i, _)| i)
            .collect();
        assert_eq!(filled.len(), 6);
        assert!(b.iter().all(|x| x.tokens == 0 || x.tokens == 100));
        assert_eq!(b.iter().map(|x| x.requests).sum::<u64>(), 6);
        // Newest of the six is 50s ago → bucket index NUM_BUCKETS-1-5
        assert_eq!(*filled.last().unwrap(), NUM_BUCKETS - 1 - 5);
    }

    fn fill_medium_for_test(b: &mut [Bucket], m: &[MetricDataPoint], now: DateTime<Utc>) {
        seed_medium_buckets(b, m, now)
    }

    #[test]
    fn source_matching() {
        let req = RecordedRequest {
            client_id: "c1".into(),
            provider: "anthropic".into(),
            model: "anthropic/claude".into(),
            tokens: 10,
            cost_micro_usd: 0,
        };
        assert!(source_matches(&TraySource::All, &req));
        assert!(source_matches(
            &TraySource::Client { id: "c1".into() },
            &req
        ));
        assert!(!source_matches(
            &TraySource::Client { id: "c2".into() },
            &req
        ));
        assert!(source_matches(
            &TraySource::Provider {
                instance: "anthropic".into()
            },
            &req
        ));
        assert!(source_matches(
            &TraySource::Model {
                id: "anthropic/claude".into()
            },
            &req
        ));
        assert!(!source_matches(
            &TraySource::Model {
                id: "claude".into()
            },
            &req
        ));
    }

    #[test]
    fn labels_resolve_custom_then_derived_then_key() {
        let mut item = TrayStatsItem::new(TraySource::Client {
            id: "abc-123".into(),
        });
        assert_eq!(resolve_label(&item, Some("Claude Code")), "CLAU");
        assert_eq!(resolve_label(&item, None), "ABC1");
        item.label = Some("cc".into());
        assert_eq!(resolve_label(&item, Some("Claude Code")), "CC");
        item.label = Some("--".into()); // normalizes to empty → derived
        assert_eq!(resolve_label(&item, Some("Claude Code")), "CLAU");
        let all = TrayStatsItem::new(TraySource::All);
        assert_eq!(resolve_label(&all, None), "ALL");
        // Nothing usable at all falls back to the source key
        let weird = TrayStatsItem::new(TraySource::Provider {
            instance: "---".into(),
        });
        assert_eq!(resolve_label(&weird, None), "PROV");
    }

    #[test]
    fn effective_layout_respects_override() {
        let mut cfg = TrayStatsConfig {
            layout: TrayLayout::Compact,
            ..Default::default()
        };
        assert_eq!(effective_layout(&cfg), TrayLayout::Compact);
        cfg.layout = TrayLayout::Extended;
        assert_eq!(effective_layout(&cfg), TrayLayout::Extended);
        cfg.layout = TrayLayout::Auto;
        assert_eq!(effective_layout(&cfg), platform_default_layout());
        if cfg!(target_os = "macos") {
            assert_eq!(platform_default_layout(), TrayLayout::Extended);
        }
        if cfg!(target_os = "windows") {
            assert_eq!(platform_default_layout(), TrayLayout::Compact);
        }
    }

    #[test]
    fn displayable_items_skip_missing_clients_and_disabled() {
        let mut stats = TrayStatsConfig::default();
        stats.add_source(TraySource::Client { id: "gone".into() });
        stats.add_source(TraySource::Client { id: "here".into() });
        stats.add_source(TraySource::Provider {
            instance: "openai".into(),
        });
        stats.items[3].enabled = false;
        let mut client = lr_config::Client::new_with_strategy("Cursor".into(), "s".into());
        client.id = "here".into();
        let items = displayable_items(&stats, &[client]);
        let labels: Vec<String> = items.iter().map(|(_, l)| l.clone()).collect();
        assert_eq!(labels, vec!["ALL", "CURS"]);
    }

    #[test]
    fn test_compute_shifts_none_last_shift() {
        let now = Utc::now();
        let (shifts, ts) = compute_bucket_shifts(None, now, 1000, 26);
        assert_eq!(shifts, 0);
        assert_eq!(ts, now);
    }

    #[test]
    fn test_compute_shifts_before_interval_elapsed() {
        let now = Utc::now();
        let last = now - Duration::milliseconds(900);
        let (shifts, ts) = compute_bucket_shifts(Some(last), now, 1000, 26);
        assert_eq!(shifts, 0, "0.9s elapsed on 1s interval: no shift");
        assert_eq!(ts, last, "timestamp must not advance without a shift");
    }

    #[test]
    fn test_compute_shifts_preserves_remainder() {
        // Updates arriving every 1.4s on a 1s interval must not lose the
        // 0.4s remainder — over 5 updates (7s) exactly 7 shifts are due.
        let start = Utc::now();
        let mut last_shift = Some(start);
        let mut total_shifts = 0usize;

        for i in 1..=5 {
            let now = start + Duration::milliseconds(1400 * i);
            let (shifts, ts) = compute_bucket_shifts(last_shift, now, 1000, 26);
            if shifts > 0 {
                last_shift = Some(ts);
            }
            total_shifts += shifts;
        }

        assert_eq!(
            total_shifts, 7,
            "7 seconds elapsed on a 1s interval must produce exactly 7 shifts"
        );
    }

    #[test]
    fn test_compute_shifts_catch_up_after_idle() {
        let now = Utc::now();
        let last = now - Duration::seconds(120);
        let (shifts, ts) = compute_bucket_shifts(Some(last), now, 1000, 26);
        assert!(shifts >= 26, "long idle gap should wipe all buckets");
        assert_eq!(ts, now, "full wipe snaps the timestamp to now");
    }

    #[test]
    fn test_compute_shifts_medium_interval() {
        let now = Utc::now();
        let last = now - Duration::seconds(25);
        let (shifts, ts) = compute_bucket_shifts(Some(last), now, 10_000, 26);
        assert_eq!(shifts, 2, "25s elapsed on 10s interval: 2 shifts");
        assert_eq!(
            ts,
            last + Duration::seconds(20),
            "timestamp advances by exact interval multiples"
        );
    }

    /// Helper to create test metrics
    fn create_metric(timestamp: DateTime<Utc>, tokens: u64) -> MetricDataPoint {
        MetricDataPoint {
            timestamp,
            requests: 1,
            input_tokens: tokens / 2,
            output_tokens: tokens / 2,
            total_tokens: tokens,
            cost_usd: 0.0,
            total_latency_ms: 0,
            successful_requests: 1,
            failed_requests: 0,
            latency_samples: vec![],
            p50_latency_ms: None,
            p95_latency_ms: None,
            p99_latency_ms: None,
        }
    }

    /// Test bucketing logic in isolation
    fn bucket_metrics(
        metrics: Vec<MetricDataPoint>,
        now: DateTime<Utc>,
        interval_secs: i64,
    ) -> Vec<u64> {
        const NUM_BUCKETS: i64 = 30;
        let window_secs = NUM_BUCKETS * interval_secs;
        let mut bucket_tokens: Vec<u64> = vec![0; NUM_BUCKETS as usize];

        for metric in metrics.iter() {
            let age_duration = now.signed_duration_since(metric.timestamp);
            let age_secs = age_duration.num_seconds();

            if age_secs < 0 || age_secs >= window_secs {
                continue;
            }

            let bucket_index = (NUM_BUCKETS - 1) - (age_secs / interval_secs);
            let bucket_index = bucket_index.clamp(0, NUM_BUCKETS - 1) as usize;
            bucket_tokens[bucket_index] += metric.total_tokens;
        }

        bucket_tokens
    }

    #[test]
    fn test_single_metric_assigns_to_correct_bucket() {
        let now = Utc::now();
        let interval_secs = 2;

        // Metric 3 seconds old should go to bucket 28
        // age=3s → bucket_index = 29 - (3/2) = 29 - 1 = 28
        let metric = create_metric(now - Duration::seconds(3), 100);
        let buckets = bucket_metrics(vec![metric], now, interval_secs);

        assert_eq!(
            buckets[28], 100,
            "Metric with age 3s should be in bucket 28"
        );
        assert_eq!(
            buckets.iter().sum::<u64>(),
            100,
            "Total tokens should be 100"
        );
    }

    #[test]
    fn test_metric_shifts_left_as_time_advances() {
        let base_time = Utc::now();
        let interval_secs = 2;

        // Create a metric at a fixed timestamp
        let metric_time = base_time;
        let metric = create_metric(metric_time, 100);

        // At T+0: metric age = 0s → bucket 29
        let buckets_t0 = bucket_metrics(vec![metric.clone()], base_time, interval_secs);
        assert_eq!(buckets_t0[29], 100, "At T+0, metric should be in bucket 29");

        // At T+2: metric age = 2s → bucket 28
        let buckets_t2 = bucket_metrics(
            vec![metric.clone()],
            base_time + Duration::seconds(2),
            interval_secs,
        );
        assert_eq!(buckets_t2[28], 100, "At T+2, metric should be in bucket 28");

        // At T+4: metric age = 4s → bucket 27
        let buckets_t4 = bucket_metrics(
            vec![metric.clone()],
            base_time + Duration::seconds(4),
            interval_secs,
        );
        assert_eq!(buckets_t4[27], 100, "At T+4, metric should be in bucket 27");

        // At T+58: metric age = 58s → bucket 0
        let buckets_t58 = bucket_metrics(
            vec![metric.clone()],
            base_time + Duration::seconds(58),
            interval_secs,
        );
        assert_eq!(buckets_t58[0], 100, "At T+58, metric should be in bucket 0");
    }

    #[test]
    fn test_metric_disappears_when_too_old() {
        let base_time = Utc::now();
        let interval_secs = 2;
        let metric = create_metric(base_time, 100);

        // At T+60: metric age = 60s, window is 60s → out of range
        let buckets = bucket_metrics(
            vec![metric],
            base_time + Duration::seconds(60),
            interval_secs,
        );
        assert_eq!(
            buckets.iter().sum::<u64>(),
            0,
            "Metric should disappear after 60 seconds"
        );
    }

    #[test]
    fn test_multiple_metrics_aggregate_in_same_bucket() {
        let now = Utc::now();
        let interval_secs = 2;

        // Two metrics with age 3s (both should go to bucket 28)
        let metric1 = create_metric(now - Duration::seconds(3), 100);
        let metric2 = create_metric(now - Duration::seconds(3), 200);

        let buckets = bucket_metrics(vec![metric1, metric2], now, interval_secs);

        assert_eq!(
            buckets[28], 300,
            "Both metrics should aggregate in bucket 28"
        );
    }

    #[test]
    fn test_multiple_metrics_in_different_buckets() {
        let now = Utc::now();
        let interval_secs = 2;

        // Metric 1: age 3s → bucket 28
        // Metric 2: age 5s → bucket 27
        // Metric 3: age 7s → bucket 26
        let metrics = vec![
            create_metric(now - Duration::seconds(3), 100),
            create_metric(now - Duration::seconds(5), 200),
            create_metric(now - Duration::seconds(7), 300),
        ];

        let buckets = bucket_metrics(metrics, now, interval_secs);

        assert_eq!(buckets[28], 100, "Metric 1 should be in bucket 28");
        assert_eq!(buckets[27], 200, "Metric 2 should be in bucket 27");
        assert_eq!(buckets[26], 300, "Metric 3 should be in bucket 26");
        assert_eq!(buckets.iter().sum::<u64>(), 600, "Total should be 600");
    }

    #[test]
    fn test_empty_metrics_produces_empty_buckets() {
        let now = Utc::now();
        let interval_secs = 2;

        let buckets = bucket_metrics(vec![], now, interval_secs);

        assert_eq!(buckets.len(), 30, "Should have 30 buckets");
        assert_eq!(
            buckets.iter().sum::<u64>(),
            0,
            "All buckets should be empty"
        );
    }

    #[test]
    fn test_future_metrics_are_ignored() {
        let now = Utc::now();
        let interval_secs = 2;

        // Metric from the future
        let metric = create_metric(now + Duration::seconds(10), 100);
        let buckets = bucket_metrics(vec![metric], now, interval_secs);

        assert_eq!(
            buckets.iter().sum::<u64>(),
            0,
            "Future metrics should be ignored"
        );
    }

    #[test]
    fn test_bucket_boundaries_with_minute_level_metrics() {
        // This tests the real-world scenario where metrics are stored at minute boundaries
        // but buckets are 2-second intervals
        let now = Utc::now();
        let interval_secs = 2;

        // Simulate a metric stored at the minute boundary (like in production)
        let metric_time =
            now.with_second(0).unwrap().with_nanosecond(0).unwrap() - Duration::minutes(0); // Current minute

        let metric = create_metric(metric_time, 100);

        // Calculate expected bucket based on age
        let age = now.signed_duration_since(metric_time).num_seconds();
        let expected_bucket = (29 - (age / interval_secs)) as usize;

        let buckets = bucket_metrics(vec![metric], now, interval_secs);

        assert_eq!(
            buckets[expected_bucket], 100,
            "Minute-boundary metric should be in bucket {}",
            expected_bucket
        );
    }

    #[test]
    fn test_consistent_bucket_assignment_over_time() {
        // Verify that as time advances by 1 second increments,
        // the metric stays in the same bucket until age crosses a 2-second boundary
        let base_time = Utc::now();
        let interval_secs = 2;
        let metric = create_metric(base_time, 100);

        // At T+0 and T+1: age 0s and 1s → both bucket 29
        let buckets_t0 = bucket_metrics(vec![metric.clone()], base_time, interval_secs);
        let buckets_t1 = bucket_metrics(
            vec![metric.clone()],
            base_time + Duration::seconds(1),
            interval_secs,
        );
        assert_eq!(buckets_t0[29], 100, "T+0: bucket 29");
        assert_eq!(buckets_t1[29], 100, "T+1: bucket 29 (same as T+0)");

        // At T+2 and T+3: age 2s and 3s → both bucket 28
        let buckets_t2 = bucket_metrics(
            vec![metric.clone()],
            base_time + Duration::seconds(2),
            interval_secs,
        );
        let buckets_t3 = bucket_metrics(
            vec![metric.clone()],
            base_time + Duration::seconds(3),
            interval_secs,
        );
        assert_eq!(buckets_t2[28], 100, "T+2: bucket 28");
        assert_eq!(buckets_t3[28], 100, "T+3: bucket 28 (same as T+2)");

        // At T+4 and T+5: age 4s and 5s → both bucket 27
        let buckets_t4 = bucket_metrics(
            vec![metric.clone()],
            base_time + Duration::seconds(4),
            interval_secs,
        );
        let buckets_t5 = bucket_metrics(
            vec![metric.clone()],
            base_time + Duration::seconds(5),
            interval_secs,
        );
        assert_eq!(buckets_t4[27], 100, "T+4: bucket 27");
        assert_eq!(buckets_t5[27], 100, "T+5: bucket 27 (same as T+4)");
    }

    #[test]
    fn test_all_30_buckets_fill_correctly() {
        let now = Utc::now();
        let interval_secs = 2;

        // Create 30 metrics, one for each bucket
        let mut metrics = Vec::new();
        for i in 0..30 {
            // Metric with age i*2 seconds should go to bucket (29-i)
            let age = i * interval_secs;
            metrics.push(create_metric(now - Duration::seconds(age), 100));
        }

        let buckets = bucket_metrics(metrics, now, interval_secs);

        // Each bucket should have exactly 100 tokens
        for (i, &tokens) in buckets.iter().enumerate() {
            assert_eq!(tokens, 100, "Bucket {} should have 100 tokens", i);
        }

        assert_eq!(
            buckets.iter().sum::<u64>(),
            3000,
            "Total should be 3000 (30 buckets * 100 tokens)"
        );
    }

    #[test]
    fn test_different_interval_sizes() {
        let now = Utc::now();

        // Test with 1-second intervals
        let metric = create_metric(now - Duration::seconds(5), 100);
        let buckets_1s = bucket_metrics(vec![metric.clone()], now, 1);
        // age=5s, interval=1s → bucket = 29 - (5/1) = 24
        assert_eq!(buckets_1s[24], 100, "1s interval: bucket 24");

        // Test with 5-second intervals
        let buckets_5s = bucket_metrics(vec![metric.clone()], now, 5);
        // age=5s, interval=5s → bucket = 29 - (5/5) = 28
        assert_eq!(buckets_5s[28], 100, "5s interval: bucket 28");
    }

    // ============================================================================
    // COMPREHENSIVE MODE-SPECIFIC TESTS WITH VIRTUAL TIME
    // ============================================================================

    /// Simulates Fast mode bucketing (1 second per bar, 26 bars)
    /// This matches the actual implementation in update_tray_graph_impl
    /// Simulates Fast mode bucketing (1 second per bar, 26 bars)
    /// Fast mode does NOT use metrics - it only tracks real-time tokens
    fn simulate_fast_mode_buckets(
        buckets: &mut [u64],
        accumulated_tokens: u64, // Real-time tokens since last update
        is_first_update: bool,
    ) {
        const NUM_BUCKETS: usize = 26;

        if is_first_update {
            // Start with empty buckets (no historical data)
            buckets.fill(0);
        } else {
            // Shift left (remove first, append 0 at end)
            buckets.rotate_left(1);
            buckets[NUM_BUCKETS - 1] = 0;
        }

        // Add accumulated real-time tokens to rightmost bucket
        buckets[NUM_BUCKETS - 1] = accumulated_tokens;
    }

    #[test]
    fn test_fast_mode_bucket_shifting() {
        // Fast mode: 1s per bar, 26 bars total (26 second window)
        // Uses real-time token accumulation, NOT metrics
        let mut buckets = vec![0u64; 26];

        // T=0: First update with 100 tokens
        simulate_fast_mode_buckets(&mut buckets, 100, true);
        assert_eq!(buckets[25], 100, "T=0: rightmost bucket should have 100");
        assert_eq!(buckets.iter().sum::<u64>(), 100);

        // T=1: Shift left, new activity with 200 tokens
        simulate_fast_mode_buckets(&mut buckets, 200, false);
        assert_eq!(buckets[24], 100, "T=1: previous data shifted to bucket 24");
        assert_eq!(buckets[25], 200, "T=1: new data in bucket 25");
        assert_eq!(buckets.iter().sum::<u64>(), 300);

        // T=2: Shift left, new activity with 150 tokens
        simulate_fast_mode_buckets(&mut buckets, 150, false);
        assert_eq!(buckets[23], 100, "T=2: oldest data at bucket 23");
        assert_eq!(buckets[24], 200, "T=2: second data at bucket 24");
        assert_eq!(buckets[25], 150, "T=2: newest data at bucket 25");
        assert_eq!(buckets.iter().sum::<u64>(), 450);

        // T=3-26: Continue shifting with varying tokens
        for i in 3..=26 {
            let tokens = 50 * i; // Varying token amounts
            simulate_fast_mode_buckets(&mut buckets, tokens as u64, false);
        }

        // Original 100 tokens from T=0 should have fallen off (26+ shifts)
        // But we should still have recent data from the last 26 updates
        let sum: u64 = buckets.iter().sum();
        assert!(sum > 0, "T=26: Should still have recent data");
        assert_eq!(
            buckets.iter().filter(|&&x| x == 0).count(),
            0,
            "All 26 buckets should be filled after 26 updates"
        );
    }

    #[test]
    fn test_fast_mode_continuous_activity() {
        // Simulate continuous token generation every second for 30 seconds
        let mut buckets = vec![0u64; 26];

        // Generate activity every second
        for t in 0..30 {
            let tokens = 100 + (t * 10); // Increasing tokens: 100, 110, 120, ...
            let is_first = t == 0;
            simulate_fast_mode_buckets(&mut buckets, tokens as u64, is_first);

            if t < 26 {
                // Should have t+1 buckets filled
                let non_zero_count = buckets.iter().filter(|&&x| x > 0).count();
                assert_eq!(
                    non_zero_count,
                    (t + 1) as usize,
                    "At T={}, should have {} non-zero buckets",
                    t,
                    t + 1
                );
            } else {
                // Should have exactly 26 buckets filled (window is full)
                let non_zero_count = buckets.iter().filter(|&&x| x > 0).count();
                assert_eq!(
                    non_zero_count, 26,
                    "At T={}, should have 26 buckets (window full)",
                    t
                );
            }
        }

        // At T=29, rightmost bucket should have the latest data (390 tokens)
        assert_eq!(
            buckets[25], 390,
            "Latest data should be in rightmost bucket"
        );
    }

    /// Simulates Medium mode bucketing (10 seconds per bar, 26 bars)
    /// Simulates Medium mode bucketing (10 seconds per bar, 26 bars)
    /// Medium mode uses metrics ONLY for initial load, then real-time tokens
    fn simulate_medium_mode_buckets(
        buckets: &mut [u64],
        metrics: Vec<MetricDataPoint>,
        virtual_now: DateTime<Utc>,
        accumulated_tokens: u64, // Real-time tokens since last update (used in runtime)
        is_first_update: bool,
    ) {
        const NUM_BUCKETS: usize = 26;
        const INTERVAL_SECS: i64 = 10;

        if is_first_update {
            // Initial load: Interpolate from minute-level metrics
            let window_secs = (NUM_BUCKETS as i64) * INTERVAL_SECS; // 260 seconds
            let start = virtual_now - Duration::seconds(window_secs + 120);

            buckets.fill(0);

            // Interpolate each minute across 6 buckets (60s / 10s = 6)
            for metric in metrics.iter() {
                if metric.timestamp < start {
                    continue;
                }

                let age_secs = virtual_now
                    .signed_duration_since(metric.timestamp)
                    .num_seconds();
                if age_secs < 0 || age_secs >= window_secs {
                    continue;
                }

                // Determine how many buckets we can actually place (some might fall outside window)
                let num_buckets_in_window = (0..6)
                    .filter(|&offset| age_secs + (offset * INTERVAL_SECS) < window_secs)
                    .count() as u64;

                if num_buckets_in_window == 0 {
                    continue;
                }

                let tokens_per_bucket = metric.total_tokens / num_buckets_in_window;

                for offset in 0..6 {
                    let bucket_age_secs = age_secs + (offset * INTERVAL_SECS);
                    if bucket_age_secs >= window_secs {
                        break;
                    }

                    let bucket_index = (NUM_BUCKETS as i64 - 1) - (bucket_age_secs / INTERVAL_SECS);
                    let bucket_index = bucket_index.max(0).min((NUM_BUCKETS - 1) as i64) as usize;
                    buckets[bucket_index] += tokens_per_bucket;
                }
            }
        } else {
            // Runtime: Use accumulated real-time tokens (NO metrics query)
            buckets.rotate_left(1);
            buckets[NUM_BUCKETS - 1] = 0;

            // Add accumulated tokens to rightmost bucket
            buckets[NUM_BUCKETS - 1] = accumulated_tokens;
        }
    }

    #[test]
    fn test_medium_mode_interpolation() {
        // Medium mode: 10s per bar, 26 bars total (260 second window = 4.33 minutes)
        let base_time = Utc::now();
        let mut buckets = vec![0u64; 26];

        // Create minute-level metrics (as stored in production)
        // One metric at T=0 with 600 tokens (should be interpolated across 6 buckets)
        let metrics = vec![create_metric(base_time, 600)];

        simulate_medium_mode_buckets(&mut buckets, metrics.clone(), base_time, 0, true);

        // Each of the last 6 buckets (representing 0-59 seconds) should have 100 tokens
        for (i, &bucket) in buckets.iter().enumerate().skip(20).take(6) {
            assert_eq!(
                bucket, 100,
                "Bucket {} should have 100 tokens from interpolation",
                i
            );
        }

        // Older buckets should be empty
        for (i, &bucket) in buckets.iter().enumerate().take(20) {
            assert_eq!(bucket, 0, "Bucket {} should be empty", i);
        }

        assert_eq!(buckets.iter().sum::<u64>(), 600, "Total should be 600");
    }

    #[test]
    fn test_medium_mode_shifting() {
        let base_time = Utc::now();
        let mut buckets = vec![0u64; 26];

        // Initial: Create metric at base_time and interpolate
        let initial_metrics = vec![create_metric(base_time, 600)];
        simulate_medium_mode_buckets(&mut buckets, initial_metrics, base_time, 0, true);

        let initial_sum: u64 = buckets.iter().sum();
        assert_eq!(initial_sum, 600, "Initial sum should be 600");

        // T=10: Shift and add new real-time data (200 tokens accumulated)
        simulate_medium_mode_buckets(&mut buckets, vec![], base_time, 200, false);

        // Buckets should have shifted left
        assert_eq!(buckets[25], 200, "T=10: new data in rightmost bucket");

        // Should have shifted data + new data
        let sum_after_shift: u64 = buckets.iter().sum();
        // 600 tokens interpolated across buckets 20-25 (100 each)
        // After shift: buckets 19-24 now have 100 each (5 buckets), bucket 25 has 200
        // Lost bucket 0 (which was 0), so total: 500 + 200 = 700
        // NOTE: If getting 800, we lost nothing (all 600 + 200 new)
        assert!(
            sum_after_shift >= 700,
            "T=10: should have at least 700 tokens, got {}",
            sum_after_shift
        );
    }

    #[test]
    fn test_medium_mode_multiple_minute_metrics() {
        // Test with multiple minute-level metrics
        let base_time = Utc::now();
        let mut buckets = vec![0u64; 26];

        // Create 3 minute-level metrics, each 60 seconds apart
        let metrics = vec![
            create_metric(base_time - Duration::seconds(120), 600), // 2 minutes ago
            create_metric(base_time - Duration::seconds(60), 1200), // 1 minute ago
            create_metric(base_time, 1800),                         // now
        ];

        simulate_medium_mode_buckets(&mut buckets, metrics, base_time, 0, true);

        // Total should be sum of all metrics
        let total: u64 = buckets.iter().sum();
        assert_eq!(total, 3600, "Total should be 600 + 1200 + 1800 = 3600");

        // Most recent minute (buckets 20-25) should have 1800/6 = 300 per bucket
        for (i, &bucket) in buckets.iter().enumerate().skip(20).take(6) {
            assert_eq!(bucket, 300, "Bucket {} should have 300 tokens", i);
        }

        // Middle minute (buckets 14-19) should have 1200/6 = 200 per bucket
        for (i, &bucket) in buckets.iter().enumerate().skip(14).take(6) {
            assert_eq!(bucket, 200, "Bucket {} should have 200 tokens", i);
        }

        // Oldest minute (buckets 8-13) should have 600/6 = 100 per bucket
        for (i, &bucket) in buckets.iter().enumerate().skip(8).take(6) {
            assert_eq!(bucket, 100, "Bucket {} should have 100 tokens", i);
        }
    }

    /// Simulates Slow mode bucketing (60 seconds per bar, 26 bars)
    fn simulate_slow_mode_buckets(
        metrics: Vec<MetricDataPoint>,
        virtual_now: DateTime<Utc>,
    ) -> Vec<u64> {
        const NUM_BUCKETS: usize = 26;
        const INTERVAL_SECS: i64 = 60;
        let window_secs = (NUM_BUCKETS as i64) * INTERVAL_SECS; // 1560 seconds = 26 minutes

        let mut bucket_tokens = vec![0u64; NUM_BUCKETS];

        // Direct mapping: each minute metric goes to exactly one bucket
        for metric in metrics.iter() {
            let age_secs = virtual_now
                .signed_duration_since(metric.timestamp)
                .num_seconds();
            if age_secs < 0 || age_secs >= window_secs {
                continue;
            }

            let bucket_index = (NUM_BUCKETS as i64 - 1) - (age_secs / INTERVAL_SECS);
            let bucket_index = bucket_index.max(0).min((NUM_BUCKETS - 1) as i64) as usize;
            bucket_tokens[bucket_index] += metric.total_tokens;
        }

        bucket_tokens
    }

    #[test]
    fn test_slow_mode_direct_mapping() {
        // Slow mode: 60s per bar, 26 bars total (1560 seconds = 26 minutes)
        let base_time = Utc::now();

        // Create one metric per minute for 26 minutes
        let mut metrics = Vec::new();
        for i in 0..26 {
            let timestamp = base_time - Duration::seconds(i * 60);
            metrics.push(create_metric(timestamp, (100 * (i + 1)) as u64));
        }

        let buckets = simulate_slow_mode_buckets(metrics, base_time);

        // Each bucket should have exactly one metric's worth of tokens
        assert_eq!(buckets[25], 100, "Most recent bucket");
        assert_eq!(buckets[24], 200, "1 minute ago");
        assert_eq!(buckets[23], 300, "2 minutes ago");
        assert_eq!(buckets[0], 2600, "25 minutes ago");

        let total: u64 = buckets.iter().sum();
        // Sum of 100, 200, 300, ..., 2600 = 100 * (1+2+3+...+26) = 100 * 351 = 35100
        assert_eq!(total, 35100, "Total should be sum of arithmetic series");
    }

    #[test]
    fn test_slow_mode_virtual_time_progression() {
        let base_time = Utc::now();

        // Create initial metrics
        let mut metrics = vec![
            create_metric(base_time - Duration::seconds(120), 1000), // 2 min ago
            create_metric(base_time - Duration::seconds(60), 2000),  // 1 min ago
            create_metric(base_time, 3000),                          // now
        ];

        // At T=0
        let buckets_t0 = simulate_slow_mode_buckets(metrics.clone(), base_time);
        assert_eq!(buckets_t0[25], 3000, "T=0: most recent in bucket 25");
        assert_eq!(buckets_t0[24], 2000, "T=0: 1 min ago in bucket 24");
        assert_eq!(buckets_t0[23], 1000, "T=0: 2 min ago in bucket 23");

        // Advance time by 60 seconds
        let t60 = base_time + Duration::seconds(60);
        metrics.push(create_metric(t60, 4000));
        let buckets_t60 = simulate_slow_mode_buckets(metrics.clone(), t60);

        assert_eq!(buckets_t60[25], 4000, "T=60: new data in bucket 25");
        assert_eq!(
            buckets_t60[24], 3000,
            "T=60: previous bucket 25 shifted to 24"
        );
        assert_eq!(
            buckets_t60[23], 2000,
            "T=60: previous bucket 24 shifted to 23"
        );
        assert_eq!(
            buckets_t60[22], 1000,
            "T=60: previous bucket 23 shifted to 22"
        );

        // Advance time by another 60 seconds (T=120)
        let t120 = base_time + Duration::seconds(120);
        metrics.push(create_metric(t120, 5000));
        let buckets_t120 = simulate_slow_mode_buckets(metrics.clone(), t120);

        assert_eq!(buckets_t120[25], 5000, "T=120: newest data");
        assert_eq!(buckets_t120[24], 4000, "T=120: T=60 data shifted");
        assert_eq!(buckets_t120[23], 3000, "T=120: T=0 data shifted");
        assert_eq!(buckets_t120[22], 2000, "T=120: T=-60 data shifted");
        assert_eq!(buckets_t120[21], 1000, "T=120: T=-120 data shifted");
    }

    #[test]
    fn test_slow_mode_metric_expiration() {
        let base_time = Utc::now();

        // Create a metric just inside the window edge (25 minutes 30 seconds old)
        // Window is [0, 26 minutes), so 26 minutes exactly is outside
        let old_metric = create_metric(base_time - Duration::seconds(25 * 60 + 30), 1000);
        let buckets = simulate_slow_mode_buckets(vec![old_metric.clone()], base_time);

        // Should be in bucket 0 (oldest bucket, covering 25-26 minutes ago)
        assert_eq!(buckets[0], 1000, "25.5-minute-old metric in bucket 0");

        // Advance time by 60 seconds - metric is now 26.5 minutes old, outside window
        let t60 = base_time + Duration::seconds(60);
        let buckets_t60 = simulate_slow_mode_buckets(vec![old_metric], t60);

        assert_eq!(
            buckets_t60.iter().sum::<u64>(),
            0,
            "26.5-minute-old metric should be expired (outside 26-minute window)"
        );
    }

    #[test]
    fn test_all_modes_handle_empty_metrics() {
        let base_time = Utc::now();
        let empty_metrics = Vec::new();

        // Fast mode (starts empty, no metrics)
        let mut fast_buckets = vec![0u64; 26];
        simulate_fast_mode_buckets(&mut fast_buckets, 0, true);
        assert_eq!(
            fast_buckets.iter().sum::<u64>(),
            0,
            "Fast mode: starts with zero buckets"
        );

        // Medium mode
        let mut medium_buckets = vec![0u64; 26];
        simulate_medium_mode_buckets(
            &mut medium_buckets,
            empty_metrics.clone(),
            base_time,
            0,
            true,
        );
        assert_eq!(
            medium_buckets.iter().sum::<u64>(),
            0,
            "Medium mode: empty metrics should produce zero buckets"
        );

        // Slow mode
        let slow_buckets = simulate_slow_mode_buckets(empty_metrics, base_time);
        assert_eq!(
            slow_buckets.iter().sum::<u64>(),
            0,
            "Slow mode: empty metrics should produce zero buckets"
        );
    }

    #[test]
    fn test_all_modes_handle_sparse_data() {
        let base_time = Utc::now();

        // Create sparse metrics: only at T=0, T=-120, T=-240
        let sparse_metrics = vec![
            create_metric(base_time, 100),
            create_metric(base_time - Duration::seconds(120), 200),
            create_metric(base_time - Duration::seconds(240), 300),
        ];

        // Fast mode: Starts empty (no metrics, only real-time tokens)
        let mut fast_buckets = vec![0u64; 26];
        simulate_fast_mode_buckets(&mut fast_buckets, 100, true);
        assert_eq!(fast_buckets[25], 100, "Fast mode: real-time data");
        assert_eq!(
            fast_buckets.iter().sum::<u64>(),
            100,
            "Fast mode: only recent real-time tokens"
        );

        // Medium mode: Should interpolate metrics on initial load
        let mut medium_buckets = vec![0u64; 26];
        simulate_medium_mode_buckets(
            &mut medium_buckets,
            sparse_metrics.clone(),
            base_time,
            0,
            true,
        );
        assert!(
            medium_buckets.iter().sum::<u64>() >= 100,
            "Medium mode: should have at least recent data"
        );

        // Slow mode: Should show data in discrete buckets
        let slow_buckets = simulate_slow_mode_buckets(sparse_metrics, base_time);
        assert_eq!(slow_buckets[25], 100, "Slow mode: bucket 25 (now)");
        assert_eq!(slow_buckets[23], 200, "Slow mode: bucket 23 (2 min ago)");
        assert_eq!(slow_buckets[21], 300, "Slow mode: bucket 21 (4 min ago)");
    }

    #[test]
    fn test_mode_comparison_with_same_data() {
        // Compare all three modes with identical input data
        let base_time = Utc::now();

        // Create consistent metrics: one per minute for 5 minutes
        let metrics: Vec<_> = (0..5)
            .map(|i| create_metric(base_time - Duration::seconds(i * 60), 1000))
            .collect();

        // Fast mode: Starts empty (no historical metrics, only real-time)
        let mut fast_buckets = vec![0u64; 26];
        simulate_fast_mode_buckets(&mut fast_buckets, 0, true);
        let fast_sum: u64 = fast_buckets.iter().sum();

        // Medium mode: Loads historical metrics with interpolation
        let mut medium_buckets = vec![0u64; 26];
        simulate_medium_mode_buckets(&mut medium_buckets, metrics.clone(), base_time, 0, true);
        let medium_sum: u64 = medium_buckets.iter().sum();

        // Slow mode: Loads all historical metrics
        let slow_buckets = simulate_slow_mode_buckets(metrics, base_time);
        let slow_sum: u64 = slow_buckets.iter().sum();

        // Fast mode starts empty (no metrics)
        assert_eq!(fast_sum, 0, "Fast mode starts empty (no historical data)");

        // Medium and slow should both capture all 5 minutes of data
        // Note: Medium mode may lose a few tokens to integer division rounding during interpolation
        assert!(
            (4980..=5000).contains(&medium_sum),
            "Medium mode should capture ~5000 tokens (got {}), small rounding loss OK",
            medium_sum
        );
        assert_eq!(slow_sum, 5000, "Slow mode should capture all 5000 tokens");

        // Medium and slow should be nearly identical on initial load
        // (Medium may have small rounding loss from interpolation)
        assert!(
            (medium_sum as i64 - slow_sum as i64).abs() <= 20,
            "Medium and slow modes should nearly match on initial load (medium: {}, slow: {})",
            medium_sum,
            slow_sum
        );
    }
}
