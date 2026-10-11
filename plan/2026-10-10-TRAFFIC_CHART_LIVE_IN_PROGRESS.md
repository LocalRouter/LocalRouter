# Traffic chart: in progress only at "now", errors only where they happened

Follow-up to `2026-10-10-DASHBOARD_TRAFFIC_CHART.md` (shipped in 0.0.155).

## Problems
1. The chart drew a "Most in progress" step line: the peak number of requests
   running at once in every bucket. Read as a history line, it was confusing;
   the user wants the requests in progress shown once, at the latest point.
2. Errors had their own small panel with a line that sat at zero almost all
   the time: a constant zero line, and an empty panel when there were none.

## Decisions
- **One line**: finished requests per bucket (blue, categorical slot 1), as
  before, with the same 10 s / 1 min / 15 min / 2 h / 8 h buckets.
- **In progress now** is drawn once, at the bucket that contains the current
  time: an amber dot at the live in-flight count on the shared count axis,
  labelled "N in progress now". The label sits in a fixed gutter right of the
  plot (`NOW_GUTTER`), so no line can cross it and the plot does not shift as
  requests start and finish; with nothing running the gutter says "Idle now"
  (muted, no dot). The count is the dashboard's own live in-flight set (monitor
  events), so it updates the moment a request starts or finishes; the hover
  tooltip of the live bucket adds an "In progress now" row and "· now" to its
  time span. The y-axis top includes the count only when it is drawn.
- **Errors** are bars on the same time and count axis (errors are a subset of
  the finished requests of that bucket), drawn only for buckets with errors,
  over the line so it never hides them. Small counts get a minimum height of
  8 px and a count label above the bar while at most 8 buckets have errors
  (the tooltip and screen-reader table carry the rest). No errors in the range
  means no bars and no "Errors" legend key; the header keeps "0 errors". The
  bar series stays mounted (drawing nothing) so the time axis layout is the
  same with and without errors.
- **Backend clean-up**: per-bucket in-flight peaks are gone.
  `TrafficBucket.in_flight`, `LiveTraffic::set_in_flight` / `take_peaks`,
  `MetricsCollector::set_in_flight` / `persist_in_flight`, the 5 s persist
  ticker (`src-tauri/src/main.rs`), the pending-count listener wiring
  (`crates/lr-server/src/state.rs`) and the `in_flight_peaks` writer, reader
  and retention are removed. Opening a database drops the 0.0.155
  `in_flight_peaks` table (`DROP TABLE IF EXISTS`), so old databases load
  unchanged otherwise. `lr-monitor`'s pending count and `set_pending_listener`
  (`crates/lr-monitor/src/store.rs`) are left in place, now without a caller,
  because another investigation is looking at that code; they can be removed
  there (the `pending_requests` counter, `notify_pending` calls and listener).
- TS types (`TrafficBucket` without `in_flight`) and the demo mock are in sync.
  The mock has errors in a few buckets per range (LLM every 23rd bucket, MCP
  every 37th), stable per-bucket noise and a smooth daily cycle, and three
  pending monitor events, so the demo shows "3 in progress now".

## Steps
1. Backend removal in `lr-monitoring` (`traffic.rs`, `metrics.rs`,
   `storage.rs`), `lr-server` state wiring, `main.rs` ticker; tests updated,
   database compatibility test added.
2. Frontend: `activity-data.ts` (points carry `live`, no `inFlight`;
   `livePoint`; `trafficPeak(points, inFlightNow)`), `request-traffic.tsx`
   (single `ComposedChart`: line, error bars, now marker), types, mock.
3. Tests: unit specs for live bucket and peak; browser specs for one line,
   error bars, now marker text, no-error range, live update.
4. Screenshots of the demo, light and dark, 10m and 1h, hover on the live
   bucket and on an error bucket, and a range without errors while idle.

## Verification
- Rust (stable 1.99, shared target): `cargo fmt --all -- --check`,
  `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo test -p lr-monitoring -p lr-server -p lr-monitor`.
- TS: `npx tsc --noEmit` (app and website), unit specs
  (`tests/e2e/unit.config.ts`), browser specs
  (`tests/e2e/home-activity.config.ts`, with system Chrome via
  `PLAYWRIGHT_CHROME_EXECUTABLE`).
- Screenshots in `/tmp/cm-chart-shots/` from the website demo. The real dev
  app was not run: another agent was building and running `localrouter` from
  the shared target directory and dev profile at the same time.

## Final steps
1. Plan review against the implementation.
2. Test coverage review.
3. Bug hunt.
4. Commit (no push for this task).

## Review
- Plan review: both complaints addressed; tooltip keeps time span, finished,
  LLM/MCP split, cost and errors; light and dark checked in screenshots.
- Test coverage: `trafficPoints` live marking (in-window, rollover, stale),
  `livePoint`, `trafficPeak` with and without the live count; browser specs for
  the single line, error bars present / absent, legend key, now marker text
  and its live update, idle text; Rust: timeline without peaks, live tier,
  collector end to end with errors, old database with the peaks table.
- Bug hunt: the label used to sit left of the dot and collided with the line
  (moved to the gutter); tiny error bars were hidden under the line (drawn on
  top, 8 px minimum); the x-axis switched between point and band layout when
  errors appeared (bar series always mounted); the browser spec's in-progress
  baseline matched both the header pill and the chart label (anchored regex).
