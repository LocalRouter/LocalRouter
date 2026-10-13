# Traffic chart: LLM and MCP lines, stacked in-progress, static legend

Date: 2026-10-13

## Context

Feedback on the dashboard Traffic chart (screenshots from the installed
0.0.156, which still drew errors as bars and the old legend):

1. Errors in adjacent buckets should connect, not each drop to zero. Already
   the behaviour on master since 48a47f3e (errors are one line per run of
   error buckets); verified against the production metrics DB, where Oct 7–11
   has errors in every consecutive 8-hour bucket.
2. MCP requests should be a line of their own.
3. Labels should be plain and static ("Finished per 10 seconds" was unclear).
4. Requests in progress should stack on the finished line: previous bucket 3,
   live bucket 1 finished + 1 running → a segment from 3 to 2, above the
   line's 1, in the same color as its line but lighter.
5. The legend names only the lines (no "In progress" entry), and errors are
   keyed by a red line, not an icon.

## Changes

- `activity-data.ts`: `ChartPoint` carries `llmLine` / `mcpLine` (finished,
  never provisional) and `llmInProgress` / `mcpInProgress` (only at the bucket
  before the live one and at the live one, and only when requests of that
  kind are running). `InFlightCounts` + `inFlightCounts()` split running
  requests by kind; `trafficPeak` takes the tallest single line (LLM, MCP with
  in-progress, or errors) instead of their sum.
- `request-traffic.tsx`: two monotone lines (LLM blue, MCP aqua) and a linear
  in-progress segment per kind in its line's color at reduced opacity (0.4
  light, 0.55 dark); the dashed clipped provisional curve is gone. A static
  legend above the plot keys each line with a line swatch: LLM requests, MCP
  requests, Errors (red). Tooltip rows use the same labels, plus "LLM in
  progress" / "MCP in progress" at the live bucket.
- Colors validated with the dataviz checker (all pairs, light and dark): blue
  `#2a78d6`/`#3987e5`, aqua `#1baf7a`/`#199e70`, error red `#d03b3b`. Orange
  failed against the error red.

## Testing

- Unit (`npm run test:unit`): 55 pass, including new in-progress stacking,
  per-kind counts and peak tests.
- Browser (`tests/e2e/home-activity.config.ts`, system Chrome): 12/13 pass.
  "the page fills the window…" fails on master too (monitor ends at 1155px in
  a 1080px viewport); the legend adds ~20px to it.
