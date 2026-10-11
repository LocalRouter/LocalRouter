//! Request traffic for the dashboard chart: finished requests, errors and
//! cost per time bucket. (Requests in progress right now come from the
//! monitor event store, not from here.)
//!
//! Every range is split into 60–96 buckets. Persisted metrics are per minute
//! at best, so the 10-minute range reads [`LiveTraffic`], which keeps the last
//! 15 minutes in 10-second buckets in memory. Longer ranges read the metrics
//! database: the `llm_global` and `mcp_global` rows.

use std::collections::{BTreeMap, HashSet};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use crate::graphs::TimeRange;
use crate::storage::{Granularity, MetricRow};

/// Metric type of all LLM requests in the metrics database.
pub const LLM_GLOBAL: &str = "llm_global";
/// Metric type of all MCP requests in the metrics database.
pub const MCP_GLOBAL: &str = "mcp_global";

/// Resolution of the in-memory tier.
pub const LIVE_BUCKET_SECS: i64 = 10;
/// How far back the in-memory tier reaches: the 10-minute range plus slack.
pub const LIVE_RETENTION_SECS: i64 = 15 * 60;

impl TimeRange {
    /// Bucket length of the dashboard traffic chart.
    pub fn traffic_bucket_secs(&self) -> i64 {
        match self {
            TimeRange::TenMinutes => LIVE_BUCKET_SECS, // 60 buckets, in memory
            TimeRange::Hour => 60,                     // 60, minute rows
            TimeRange::Day => 15 * 60,                 // 96, minute rows
            TimeRange::Week => 2 * 3_600,              // 84, hour rows
            TimeRange::Month => 8 * 3_600,             // 90, hour rows
        }
    }

    /// Bucket starts (Unix seconds, aligned to the bucket length) of the
    /// traffic chart, oldest first; the last one contains `now`.
    pub fn traffic_bucket_starts(&self, now: i64) -> Vec<i64> {
        let size = self.traffic_bucket_secs();
        let count = self.duration().num_seconds() / size;
        let current = now.div_euclid(size) * size;
        (0..count)
            .map(|i| current - (count - 1 - i) * size)
            .collect()
    }
}

/// Requests that finished in a bucket or range.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct TrafficCounts {
    /// Finished LLM requests, failed ones included.
    pub llm_requests: u64,
    /// LLM requests that failed.
    pub llm_errors: u64,
    /// Finished MCP requests, failed ones included.
    pub mcp_requests: u64,
    /// MCP requests that failed.
    pub mcp_errors: u64,
    /// Estimated LLM cost in USD.
    pub cost_usd: f64,
}

impl TrafficCounts {
    fn add(&mut self, other: &TrafficCounts) {
        self.llm_requests += other.llm_requests;
        self.llm_errors += other.llm_errors;
        self.mcp_requests += other.mcp_requests;
        self.mcp_errors += other.mcp_errors;
        self.cost_usd += other.cost_usd;
    }
}

/// One bucket of the traffic chart.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrafficBucket {
    /// Bucket start, Unix seconds.
    pub timestamp: i64,
    /// `None` when this period is not known at this resolution: before the
    /// app started (10-minute range), or history kept only as daily totals.
    pub counts: Option<TrafficCounts>,
}

/// The dashboard traffic chart for one time range.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrafficTimeline {
    pub bucket_secs: i64,
    /// Oldest first; the last bucket contains the current time.
    pub buckets: Vec<TrafficBucket>,
    /// Over the whole range, including periods only known as daily totals.
    pub totals: TrafficCounts,
}

impl TrafficTimeline {
    /// Build from in-memory buckets (the 10-minute range).
    fn from_buckets(bucket_secs: i64, buckets: Vec<TrafficBucket>) -> Self {
        let mut totals = TrafficCounts::default();
        for counts in buckets.iter().filter_map(|b| b.counts.as_ref()) {
            totals.add(counts);
        }
        Self {
            bucket_secs,
            buckets,
            totals,
        }
    }
}

/// Requests, errors and cost of one series.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct Sums {
    requests: u64,
    errors: u64,
    cost_usd: f64,
}

impl Sums {
    fn of(row: &MetricRow) -> Self {
        Self {
            requests: row.requests,
            errors: row.failed_requests,
            cost_usd: row.cost_usd.unwrap_or(0.0),
        }
    }

    fn add(&mut self, other: &Sums) {
        self.requests += other.requests;
        self.errors += other.errors;
        self.cost_usd += other.cost_usd;
    }
}

/// Sum one metric type's rows into buckets.
///
/// Rollups overlap the rows they were built from, so each period is counted
/// at one granularity: the coarsest one whose rows still fit in a bucket.
/// A row wider than a bucket (a daily total on an 8-hour chart) cannot be
/// placed; when the finer rows of its period are gone (they age out sooner)
/// the buckets of that period without finer rows are unknown (`None`), and
/// the missing part still counts towards the total.
fn bucket_rows(rows: &[MetricRow], starts: &[i64], bucket_secs: i64) -> (Vec<Option<Sums>>, Sums) {
    let mut buckets = vec![Some(Sums::default()); starts.len()];
    let mut total = Sums::default();
    let (Some(&first), Some(&last)) = (starts.first(), starts.last()) else {
        return (buckets, total);
    };
    let end = last + bucket_secs;
    let in_range = |ts: i64| ts >= first && ts < end;
    let fits = |g: Granularity| g.span_secs() <= bucket_secs;
    let period = |ts: i64, g: Granularity| ts.div_euclid(g.span_secs()) * g.span_secs();

    // Periods that a coarser, fitting rollup covers.
    let rolled_up = |g: Granularity| -> HashSet<i64> {
        rows.iter()
            .filter(|r| r.granularity == g && fits(g))
            .map(|r| r.timestamp.timestamp())
            .collect()
    };
    let hours = rolled_up(Granularity::Hour);
    let days = rolled_up(Granularity::Day);

    // Rows counted in buckets, by timestamp.
    let mut counted: BTreeMap<i64, Sums> = BTreeMap::new();
    for row in rows {
        let ts = row.timestamp.timestamp();
        if !in_range(ts) || !fits(row.granularity) {
            continue;
        }
        let superseded = match row.granularity {
            Granularity::Minute => {
                hours.contains(&period(ts, Granularity::Hour))
                    || days.contains(&period(ts, Granularity::Day))
            }
            Granularity::Hour => days.contains(&period(ts, Granularity::Day)),
            Granularity::Day => false,
        };
        if superseded {
            continue;
        }
        let sums = Sums::of(row);
        let index = ((ts - first) / bucket_secs) as usize;
        if let Some(Some(bucket)) = buckets.get_mut(index) {
            bucket.add(&sums);
        }
        total.add(&sums);
        counted.entry(ts).or_default().add(&sums);
    }

    // Rows too wide for a bucket, coarsest first, so a day already accounted
    // for is not counted again through its hours.
    let mut wide: Vec<&MetricRow> = rows
        .iter()
        .filter(|r| !fits(r.granularity) && in_range(r.timestamp.timestamp()))
        .collect();
    wide.sort_by_key(|r| std::cmp::Reverse(r.granularity.span_secs()));
    let mut accounted: Vec<(i64, i64)> = Vec::new();
    for row in wide {
        let from = row.timestamp.timestamp();
        let to = from + row.granularity.span_secs();
        if accounted.iter().any(|(s, e)| from >= *s && to <= *e) {
            continue;
        }
        let mut finer = Sums::default();
        for sums in counted.range(from..to).map(|(_, s)| s) {
            finer.add(sums);
        }
        let missing = row.requests.saturating_sub(finer.requests);
        if missing == 0 {
            continue;
        }
        total.add(&Sums {
            requests: missing,
            errors: row.failed_requests.saturating_sub(finer.errors),
            cost_usd: (row.cost_usd.unwrap_or(0.0) - finer.cost_usd).max(0.0),
        });
        accounted.push((from, to));
        for (index, start) in starts.iter().enumerate() {
            let bucket_end = start + bucket_secs;
            let overlaps = *start < to && bucket_end > from;
            if overlaps && counted.range(*start..bucket_end).next().is_none() {
                buckets[index] = None;
            }
        }
    }

    (buckets, total)
}

/// The traffic chart of a database-backed range.
pub fn timeline_from_rows(
    starts: &[i64],
    bucket_secs: i64,
    llm: &[MetricRow],
    mcp: &[MetricRow],
) -> TrafficTimeline {
    let (llm_buckets, llm_total) = bucket_rows(llm, starts, bucket_secs);
    let (mcp_buckets, mcp_total) = bucket_rows(mcp, starts, bucket_secs);
    let buckets = starts
        .iter()
        .enumerate()
        .map(|(i, &timestamp)| {
            let counts = match (llm_buckets[i], mcp_buckets[i]) {
                (Some(llm), Some(mcp)) => Some(TrafficCounts {
                    llm_requests: llm.requests,
                    llm_errors: llm.errors,
                    mcp_requests: mcp.requests,
                    mcp_errors: mcp.errors,
                    cost_usd: llm.cost_usd + mcp.cost_usd,
                }),
                _ => None,
            };
            TrafficBucket { timestamp, counts }
        })
        .collect();
    TrafficTimeline {
        bucket_secs,
        buckets,
        totals: TrafficCounts {
            llm_requests: llm_total.requests,
            llm_errors: llm_total.errors,
            mcp_requests: mcp_total.requests,
            mcp_errors: mcp_total.errors,
            cost_usd: llm_total.cost_usd + mcp_total.cost_usd,
        },
    }
}

#[derive(Debug)]
struct LiveState {
    /// When recording began (Unix seconds); earlier buckets are unknown.
    started_at: i64,
    buckets: BTreeMap<i64, TrafficCounts>,
}

fn live_bucket(ts: i64) -> i64 {
    ts.div_euclid(LIVE_BUCKET_SECS) * LIVE_BUCKET_SECS
}

impl LiveState {
    /// Drop buckets older than the retention, relative to `now`.
    fn prune(&mut self, now: i64) {
        let cutoff = live_bucket(now - LIVE_RETENTION_SECS);
        if self
            .buckets
            .first_key_value()
            .is_some_and(|(k, _)| *k < cutoff)
        {
            self.buckets = self.buckets.split_off(&cutoff);
        }
    }

    fn bucket(&mut self, at: i64) -> &mut TrafficCounts {
        self.prune(at);
        self.buckets.entry(live_bucket(at)).or_default()
    }
}

/// Recent traffic in 10-second buckets. Memory is bounded: buckets older
/// than [`LIVE_RETENTION_SECS`] are dropped (at most 91 buckets of a few
/// numbers each).
#[derive(Debug)]
pub struct LiveTraffic {
    state: Mutex<LiveState>,
}

impl LiveTraffic {
    /// Start recording at `now` (Unix seconds).
    pub fn new(now: i64) -> Self {
        Self {
            state: Mutex::new(LiveState {
                started_at: now,
                buckets: BTreeMap::new(),
            }),
        }
    }

    /// A finished LLM request.
    pub fn record_llm(&self, at: i64, success: bool, cost_usd: f64) {
        let mut state = self.state.lock();
        let counts = state.bucket(at);
        counts.llm_requests += 1;
        counts.llm_errors += u64::from(!success);
        counts.cost_usd += cost_usd;
    }

    /// A finished MCP request.
    pub fn record_mcp(&self, at: i64, success: bool) {
        let mut state = self.state.lock();
        let counts = state.bucket(at);
        counts.mcp_requests += 1;
        counts.mcp_errors += u64::from(!success);
    }

    /// The 10-second buckets starting at `starts`. Buckets before recording
    /// began are unknown.
    pub fn timeline(&self, starts: &[i64], now: i64) -> TrafficTimeline {
        let mut state = self.state.lock();
        state.prune(now);
        let known_from = live_bucket(state.started_at);
        let buckets = starts
            .iter()
            .map(|&timestamp| {
                let counts = (timestamp >= known_from)
                    .then(|| state.buckets.get(&timestamp).copied().unwrap_or_default());
                TrafficBucket { timestamp, counts }
            })
            .collect();
        TrafficTimeline::from_buckets(LIVE_BUCKET_SECS, buckets)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::DateTime;

    const HOUR: i64 = 3_600;
    const DAY: i64 = 86_400;

    fn row(ts: i64, granularity: Granularity, requests: u64, failed: u64, cost: f64) -> MetricRow {
        MetricRow {
            timestamp: DateTime::from_timestamp(ts, 0).unwrap(),
            granularity,
            requests,
            successful_requests: requests - failed,
            failed_requests: failed,
            avg_latency_ms: 0.0,
            input_tokens: None,
            output_tokens: None,
            cost_usd: Some(cost),
            method_counts: None,
            p50_latency_ms: None,
            p95_latency_ms: None,
            p99_latency_ms: None,
        }
    }

    #[test]
    fn every_range_has_60_to_96_aligned_buckets_ending_now() {
        let now = 1_791_000_123;
        for (range, count) in [
            (TimeRange::TenMinutes, 60),
            (TimeRange::Hour, 60),
            (TimeRange::Day, 96),
            (TimeRange::Week, 84),
            (TimeRange::Month, 90),
        ] {
            let size = range.traffic_bucket_secs();
            let starts = range.traffic_bucket_starts(now);
            assert_eq!(starts.len(), count, "{range:?}");
            assert!(starts.iter().all(|s| s % size == 0), "{range:?} aligned");
            assert!(starts.windows(2).all(|w| w[1] - w[0] == size));
            let last = *starts.last().unwrap();
            assert!(last <= now && now < last + size, "{range:?} ends now");
        }
    }

    #[test]
    fn minute_rows_sum_into_their_bucket_with_errors_and_cost() {
        let starts = TimeRange::Day.traffic_bucket_starts(10 * DAY);
        let first = starts[0];
        let rows = [
            row(first, Granularity::Minute, 3, 1, 0.5),
            row(first + 14 * 60, Granularity::Minute, 2, 0, 0.25),
            row(first + 15 * 60, Granularity::Minute, 4, 4, 0.0),
            row(first - 60, Granularity::Minute, 9, 9, 9.0),
        ];
        let (buckets, total) = bucket_rows(&rows, &starts, 900);
        assert_eq!(
            buckets[0],
            Some(Sums {
                requests: 5,
                errors: 1,
                cost_usd: 0.75
            })
        );
        assert_eq!(buckets[1].unwrap().errors, 4);
        assert_eq!(total.requests, 9);
    }

    /// An hour row and the minute rows it was rolled up from are one period,
    /// counted once; minutes of an hour not rolled up yet still count.
    #[test]
    fn hourly_rollups_replace_their_minutes_without_double_counting() {
        let starts = TimeRange::Week.traffic_bucket_starts(30 * DAY);
        let first = starts[0];
        let rows = [
            row(first, Granularity::Hour, 10, 2, 1.0),
            row(first + 60, Granularity::Minute, 4, 1, 0.4),
            row(first + 6 * 60, Granularity::Minute, 6, 1, 0.6),
            row(first + HOUR + 60, Granularity::Minute, 5, 0, 0.5),
        ];
        let (buckets, total) = bucket_rows(&rows, &starts, 2 * HOUR);
        assert_eq!(buckets[0].unwrap().requests, 15);
        assert_eq!(total.requests, 15);
        assert_eq!(total.errors, 2);
    }

    /// Fine charts never place an hour total in one small bucket.
    #[test]
    fn rollups_wider_than_a_bucket_are_not_placed_when_their_minutes_exist() {
        let starts = TimeRange::Day.traffic_bucket_starts(10 * DAY);
        let i = starts.iter().position(|s| s % HOUR == 0).unwrap();
        let hour = starts[i];
        let rows = [
            row(hour, Granularity::Hour, 3, 0, 0.0),
            row(hour + 120, Granularity::Minute, 1, 0, 0.0),
            row(hour + 1_900, Granularity::Minute, 2, 0, 0.0),
        ];
        let (buckets, total) = bucket_rows(&rows, &starts, 900);
        assert_eq!(buckets[i].unwrap().requests, 1);
        assert_eq!(buckets[i + 2].unwrap().requests, 2);
        assert!(buckets.iter().all(Option::is_some));
        assert_eq!(total.requests, 3);
    }

    /// History older than the hourly retention is only kept per day: those
    /// days are unknown at 8-hour resolution but still count in the total.
    #[test]
    fn days_known_only_as_daily_totals_are_unknown_buckets_but_count_in_totals() {
        let starts = TimeRange::Month.traffic_bucket_starts(100 * DAY + 5 * HOUR);
        let i = starts.iter().position(|s| s % DAY == 0).unwrap();
        let day = starts[i];
        let hourly_day = day + DAY;
        let rows = [
            row(day, Granularity::Day, 30, 3, 3.0),
            row(hourly_day, Granularity::Day, 7, 0, 0.7),
            row(hourly_day + 9 * HOUR, Granularity::Hour, 7, 0, 0.7),
        ];
        let (buckets, total) = bucket_rows(&rows, &starts, 8 * HOUR);
        assert_eq!(&buckets[i..i + 3], &[None, None, None]);
        assert_eq!(buckets[i + 4].unwrap().requests, 7);
        assert_eq!(buckets[i + 3], Some(Sums::default()));
        assert!(buckets[i + 6].is_some());
        assert_eq!(total.requests, 37);
        assert_eq!(total.errors, 3);
        assert!((total.cost_usd - 3.7).abs() < 1e-9);
    }

    /// A day whose hours were only partly kept: the hours left are placed,
    /// the rest of the day is unknown, and the day total is not exceeded.
    #[test]
    fn partially_kept_days_mark_only_buckets_without_hours_unknown() {
        let starts = TimeRange::Month.traffic_bucket_starts(100 * DAY + 5 * HOUR);
        let i = starts.iter().position(|s| s % DAY == 0).unwrap();
        let day = starts[i];
        let rows = [
            row(day, Granularity::Day, 10, 0, 0.0),
            row(day + 20 * HOUR, Granularity::Hour, 4, 0, 0.0),
        ];
        let (buckets, total) = bucket_rows(&rows, &starts, 8 * HOUR);
        assert_eq!(&buckets[i..i + 2], &[None, None]);
        assert_eq!(buckets[i + 2].unwrap().requests, 4);
        assert_eq!(total.requests, 10);
    }

    #[test]
    fn timeline_merges_llm_and_mcp() {
        let starts = TimeRange::Hour.traffic_bucket_starts(50 * DAY + 30);
        let now_bucket = *starts.last().unwrap();
        let llm = [row(now_bucket, Granularity::Minute, 3, 1, 0.3)];
        let mcp = [row(now_bucket - 60, Granularity::Minute, 2, 2, 0.0)];
        let t = timeline_from_rows(&starts, 60, &llm, &mcp);
        assert_eq!(t.buckets.len(), 60);
        let last = &t.buckets[59];
        assert_eq!(last.counts.unwrap().llm_requests, 3);
        assert_eq!(t.buckets[58].counts.unwrap().mcp_errors, 2);
        assert_eq!(t.buckets[0].counts, Some(TrafficCounts::default()));
        assert_eq!(t.totals.llm_errors, 1);
        assert_eq!(t.totals.mcp_requests, 2);
    }

    #[test]
    fn live_buckets_before_recording_began_are_unknown() {
        let start = 1_000_005;
        let live = LiveTraffic::new(start);
        live.record_llm(start + 1, true, 0.5);
        live.record_llm(start + 12, false, 0.0);
        live.record_mcp(start + 12, true);
        let now = start + 10;
        let t = live.timeline(&TimeRange::TenMinutes.traffic_bucket_starts(now), now);
        assert_eq!(t.buckets.len(), 60);
        assert_eq!(t.buckets[57].counts, None);
        assert_eq!(t.buckets[58].counts.unwrap().llm_requests, 1);
        assert_eq!(t.buckets[59].counts.unwrap().llm_errors, 1);
        assert_eq!(t.buckets[59].counts.unwrap().mcp_requests, 1);
        assert_eq!(t.totals.llm_requests, 2);
        assert!((t.totals.cost_usd - 0.5).abs() < 1e-9);
    }

    #[test]
    fn live_memory_is_bounded() {
        let start = 4_000_000;
        let live = LiveTraffic::new(start);
        for i in 0..10_000 {
            live.record_llm(start + i * 7, true, 0.0);
        }
        let buckets = live.state.lock().buckets.len() as i64;
        assert!(buckets <= LIVE_RETENTION_SECS / LIVE_BUCKET_SECS + 1);
    }
}
