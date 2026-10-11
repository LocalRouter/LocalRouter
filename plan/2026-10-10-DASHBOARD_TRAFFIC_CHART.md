# Dashboard traffic chart and usage cards

> The per-bucket "in progress" peaks and the separate error panel described
> here were replaced in `2026-10-10-TRAFFIC_CHART_LIVE_IN_PROGRESS.md`.

## Problems
1. The usage-limit cards show a "Near limit" pill; the user wants it gone.
2. The money line on the usage cards ("API value 30d $4.95") does not say what
   the figure is or over which period, so nobody can act on it.
3. The Traffic card is not collapsible like the Usage limits card.
4. The traffic chart stacks LLM, MCP and in-progress bars; the user wants lines:
   finished requests, in-progress requests and errors as separate series.
5. Too few points: 10 minutes had 10 one-minute bars, 1 hour 12, 30 days 30.

## Decisions
- **Resolution per range** (`TimeRange::traffic_bucket_secs`):
  10m → 10 s (60 points), 1h → 1 min (60), 24h → 15 min (96),
  7d → 2 h (84), 30d → 8 h (90). Buckets are aligned to the bucket length and
  the last one contains "now".
- **In-memory 10-second tier** (`lr_monitoring::traffic::LiveTraffic`): the
  database is per minute at best, so the last 15 minutes are kept in 10-second
  buckets (≤ 91 buckets of five counters and a peak). Buckets before the app
  started are reported as unknown, not as zero.
- **Longer ranges read the database**, choosing per period the coarsest
  granularity whose rows fit in a bucket (hour rows for 2 h / 8 h buckets,
  minute rows otherwise; minute rows of an hour that is not rolled up yet are
  used). Hourly rows are now kept 31 days instead of 7 so the 30-day chart has
  sub-day detail. Days known only as a daily total (history older than the old
  7-day hourly retention, for the first weeks after upgrading) are unknown
  buckets on the 8-hour chart but still count in the range totals.
- **MCP traffic is persisted** as `mcp_global` rows (it was only in memory for
  24 hours), so MCP requests and errors survive restarts and show on 7d/30d.
- **In progress is real data**: the monitor event store keeps an O(1) count of
  pending requests (LLM calls and MCP requests, not internal steps or duplicate
  hops — the same rule as the dashboard's `isRequest`) and tells the metrics
  collector on every change, under the store lock so counts arrive in order.
  `LiveTraffic` keeps the peak per 10-second bucket and carries a running level
  across buckets. Every 5 s the per-minute peaks that rose are written to a new
  `in_flight_peaks` table (one row per busy minute, 31-day retention); longer
  ranges plot the peak per bucket from it. The live bucket also shows the
  frontend's current in-flight count, so it reacts before the next read.
- **New command** `get_traffic_timeline(timeRange)` returns buckets
  (`counts | null`, `in_flight | null`) and range totals; it replaces the three
  `get_global_*` reads the dashboard made. Existing graph commands are unchanged.
- **Chart** (dataviz skill): two panels sharing the time axis and a synced
  crosshair tooltip — finished requests (blue, categorical slot 1) and peak in
  progress (amber, step line) on one count axis; errors (status red, with an
  icon and label) on their own small panel so a single error is visible. No
  stacking, 2 px lines, hairline grid, legend with line keys, round local-time
  ticks, screen-reader table of busy buckets. Palette validated with the skill's
  validator in light and dark (orange was rejected next to the error red).
- **Collapsible sections**: Traffic and Usage limits share
  `CollapsibleSection` + `useCollapsed` (chevron, icon, remembered in
  localStorage). Traffic's totals and in-progress pill stay in the header when
  collapsed.
- **Usage cards**: the "Near limit" pill is removed (the "Limited" pill for a
  rejected account stays). The money line became "API-price value via
  LocalRouter" (subscriptions, with "4.6× plan" / "35% of plan") or "Spend via
  LocalRouter" (API keys, with request count), showing 24h / 7d / 30d, and a
  tooltip explaining list prices and what is not included. Uses existing
  `UsageSpendView` fields; `crates/lr-usage` is unchanged.

## Steps
1. Tasks tracked in the session todo list.
2. Backend: pending count in `lr-monitor`, `traffic.rs`, storage (peaks table,
   all-granularity query, hourly retention), collectors, command, ticker.
3. Frontend: types, demo mock, data helpers, chart, collapsible section, usage
   card changes.
4. Tests: Rust unit tests (bucketing, rollup de-duplication, unknown days,
   live tier, carry-over, peak hand-over, memory bound, pending count, collector
   end to end), TS unit specs, browser specs against the demo.
5. Run the dev app, generate traffic, screenshot light and dark.

## Verification
- `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --all --
  --check`, `cargo test -p lr-monitor -p lr-monitoring -p lr-server` (rustup
  stable 1.99).
- `npx tsc --noEmit` (app and website), unit specs
  (`tests/e2e/unit.config.ts`), browser specs (`tests/e2e/home-activity.config.ts`).
- Dev app (`cargo tauri dev --no-watch`, file keychain) with ~280 real LLM
  requests to local Ollama (successes, failures, slow requests): the database
  received `llm_global` minute rows with failures and per-minute in-progress
  peaks that rose and drained as Ollama worked through its queue. The window
  itself could not be captured (no Screen Recording permission for the agent),
  so the hour / 24 h / 30 d timelines were produced by the real backend code
  from a copy of that database and rendered in the demo UI for screenshots,
  light and dark. The 10-minute tier is in-process only and was checked by
  tests and the demo mock.

## Results
- Rust: clippy and fmt clean; `lr-monitor` 44, `lr-monitoring` 62,
  `lr-server` 173 tests pass.
- TS: `tsc` clean for app and website; 46 unit specs and 9 browser specs pass
  (the browser specs need a warm Vite server; the very first cold page load can
  exceed the 5 s heading wait).

## Review
- Plan review: all five requests are implemented; `lr-usage` and the usage
  settings tab were left alone (other work in progress there).
- Test coverage: bucket layout per range, rollup de-duplication, rows wider
  than a bucket (known and unknown), peaks per bucket, live tier (unknown
  before start, carry-over, hand-over, memory bound), pending count through
  push/update/evict/clear, storage peak MAX and retention, collector end to end
  including a simulated restart; TS points, ticks, live overlay, rollover,
  routed spend; browser specs for the line chart, 8-hour buckets, collapse
  memory, in-flight, empty and failed loads.
- Bug hunt fixes: stuck pending requests could grow the live map without a
  prune (prune now also runs on carry); `Array.prototype.at` is not in the
  website's TS lib; x-axis ticks used to repeat a minute label at 10 s buckets
  (ticks now land on round local times in any time zone); the error panel's
  axis overlapped the main panel's zero.
- Known limits: right after upgrading, MCP history before this change is not in
  the database (it was memory-only), and the 30-day chart shows days older than
  the old 7-day hourly retention as gaps until they age out; totals include
  them. Requests that a client abandons stay "in progress" until LocalRouter
  finishes them, as in the monitor list.

## Final steps
1. Plan review against the implementation.
2. Test coverage review.
3. Bug hunt.
4. Commit (the task says not to push).
