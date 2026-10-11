# Traffic chart: dashed "now" extension, error line, errors filter

Follow-up to `2026-10-10-TRAFFIC_CHART_LIVE_IN_PROGRESS.md` (0.0.156).

## Requests from the user
1. Show the requests in progress not as a dot but as a light or dotted blue
   line from the previous bucket to the current one, an extension of the
   line that "solidifies" into the normal line. Drop the labels
   "Finished per 10 seconds" and "In progress now".
2. Errors as bars look like "a line up": draw them as a line like the blue
   one, rising from zero at the previous bucket and back down to zero.
3. A dashboard events filter for errors.

## Decisions
- **One requests line, split in two.** Every closed bucket plots its
  finished requests; the open bucket (the one containing now) plots a
  provisional value, finished so far + in progress now (`chartPoints` in
  `activity-data.ts`, field `requests`). The `Line` uses a custom shape
  (`RequestsCurve`) that draws the *same* monotone curve twice, clipped at the
  last closed bucket's x: solid to the left, the same blue dashed (`4 4`) and
  lighter (stroke opacity 0.55 light / 0.75 dark, set as
  `--traffic-provisional`) to the right. Because both halves are one curve,
  the smoothing runs through the join and the dashed part reads as the line
  continuing, not a separate series. When the bucket closes, the next render
  draws that stretch solid and a new dashed stretch starts. With nothing in
  progress and nothing finished yet, the dashed stretch goes to 0.
  A stale timeline (no bucket contains now) draws no dashed stretch.
- The y-axis top includes the provisional value (`trafficPeak`).
- **Removed**: the amber now-dot, the right gutter label ("N in progress now" /
  "Idle now"), and the whole legend row. The header keeps the "N in progress"
  pill and "⚠ N errors" (red icon), which together with the error count labels
  and the tooltip identify both lines; the right margin shrinks from 112 px to
  16 px. The plot keeps its 156 px height.
- **Hovering the open bucket**: the tooltip is headed "Now (in progress)",
  then the time span, "Finished so far", the LLM/MCP split and cost,
  "In progress" (dashed key) and errors. Its active dot is hollow. Closed
  buckets keep their tooltip.
- **Errors as a line** (`errorLine`): the error count where errors happened, 0
  at the neighbouring buckets, `null` elsewhere. The `Line` uses a custom
  shape (`ErrorsCurve`) that draws one monotone piece per run and never joins
  two zeros, so there is no red line along the baseline between runs (runs
  one bucket apart share their zero). Same stroke and smoothing as the
  requests line, drawn over it. A peak is drawn at least 8 px tall (a single
  error next to 80 requests would otherwise be a flat smear); the count
  label above the peak (while ≤ 8 error buckets) and the tooltip carry the
  exact number. Unknown buckets (no data at this resolution) stay unknown.
- **No bars** means the time axis is a point scale: the newest bucket sits on
  the plot's right edge. Round-time ticks on the last two buckets are dropped
  so labels do not run past the edge.
- **Errors filter**: an "Errors only" toggle (aria-pressed) next to the type
  filter in the dashboard's event list sets `MonitorEventFilter.status =
  'error'`. The backend already filters on the structured `EventStatus`
  (`crates/lr-monitor/src/store.rs` `match_filter`), and failed LLM / MCP calls
  and every error event type (auth, access denied, rate limit, validation,
  internal, connection, MCP server failures) carry `status: error`; live events
  go through the matching client predicate (`matchesFilter`). No Rust or type
  change was needed. The choice is persisted with the other filter fields.
  An empty filtered list says "No events match these filters" instead of
  "No events captured yet".
- **Demo mock**: the open bucket carries only the requests finished so far
  (scaled by elapsed time), and `get_monitor_events` applies the filter with
  the app's `matchesFilter`, like the backend.

## Steps
1. `activity-data.ts`: `trafficPeak` with the provisional value, `chartPoints`.
2. `request-traffic.tsx`: requests and errors lines with custom shapes,
   tooltip, legend and gutter removal, tick edge filter.
3. `event-filters.tsx`, `event-list.tsx`, `request-monitor.tsx`: errors
   toggle and filtered empty state.
4. Demo mock: partial open bucket, filtered `get_monitor_events`.
5. Tests: unit (`dashboard-live.spec.ts`: provisional value, peak, error line
   runs) and browser (`home-activity.spec.ts`: solid + dashed halves share one
   path and meet at the split, error pieces per run, no labels / legend / now
   marker, tooltip of the open and a closed bucket, live in-flight update moves
   the dashed end, no-error range, errors filter incl. live events and reload,
   filtered empty state).
6. Screenshots in `/tmp/cm-chart2-shots/`.

## Verification
- `npx tsc --noEmit` (app and website), unit specs (`tests/e2e/unit.config.ts`,
  51 passed), dashboard browser specs (`tests/e2e/home-activity.config.ts`
  with system Chrome via `PLAYWRIGHT_CHROME_EXECUTABLE`, 13 passed). No Rust
  change.
- Demo screenshots (desktop width, 2x): 10m and 1h, light and dark, with
  requests in progress, hovering the open bucket, idle, hovering an error
  peak, and the event list with "Errors only" in light and dark.
- Real dev app (`cargo tauri dev --no-watch`, private target dir, removed
  afterwards): the dashboard renders in WKWebView with the dashed open-bucket
  stretch (the CSS-variable stroke opacity applies) and the "Errors only"
  toggle. It had no authenticated traffic to show (sending LLM requests would
  have needed a dev client key), so the chart there is flat.

## Final steps
1. Plan review against the implementation.
2. Test coverage review.
3. Bug hunt.
4. Commit (no push for this task).

## Review
- Plan review: dashed extension at finished so far + in progress, solid
  elsewhere, same curve; dot, gutter label and both legend labels gone; error
  line from 0 to the count and back only around errors; tooltip for the open
  bucket; errors filter in the event list with light and dark screenshots.
- Test coverage: `chartPoints` (provisional only at the live bucket, idle,
  stale), `trafficPeak` with the provisional value, `errorLine` runs (separate
  runs, adjacent runs, none, unknown neighbours); browser specs as in step 5.
- Bug hunt: the dashed segment was nearly invisible in dark mode at 0.5
  opacity (raised to 0.75 there); single errors on the 1h range were flat on
  the baseline (8 px minimum peak); two zero buckets between error runs drew a
  red baseline stretch (split into runs in the shape); the last time tick ran
  past the plot edge once the axis became a point scale (dropped near the
  edge); the empty event list claimed "no events captured" under a filter.
- Note: early in a long bucket (1 min on 1h, 15 min on 24h) the provisional
  value can be below the previous bucket, so the dashed stretch slopes down
  and then rises as the bucket fills. This is the honest "so far" value.
