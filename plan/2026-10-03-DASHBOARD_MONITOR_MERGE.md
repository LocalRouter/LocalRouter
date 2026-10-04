# Merge Monitor into Dashboard, 10m traffic window, 30d bucket fix, release 0.0.149

## Progress / todo
- [x] Move the Monitor view into the Dashboard as a full-height "Request monitor" card (filters, search, intercept, Try It Out, resizable detail pane). Remove the Clear button.
- [x] Keep the persisted `monitor.filter` selection (event types, status, search); default stays All Events.
- [x] Remove the Monitor sidebar entry and route; Dashboard's popup detail dialog replaced by the Monitor pane.
- [x] Add a `ten_minutes` time range (1-minute buckets) and make it the Dashboard default.
- [x] Fix the 30d zigzag: Month buckets were 12h over per-day rows; use daily buckets and skip the leading bucket that can never hold a row.
- [x] Update demo mock, TypeScript types and docs wording.
- [x] Plan review: every requested change is implemented (merge, retained features, Clear removed, 10m default, 30d fix, full height, pane detail, filter persistence).
- [x] Test coverage review: Rust tests for 10m serde/buckets and daily month aggregation; e2e for default range, per-day 30d, full-height monitor, Try It Out, no Clear, filter persistence, detail pane; Monitor e2e suite retargeted to the Dashboard.
- [x] Bug hunt: reload/refresh races in `useMonitorEvents`, partial leading buckets for all range/generator paths, intercept cleanup on unmount, removed `monitor` view references.
- [ ] Commit and push to master; wait for CI.
- [ ] Dispatch Release workflow for 0.0.149 (v0.0.149 was never published) and monitor it to completion.

## Design notes
- The Dashboard root is its own scroll container (`h-full overflow-y-auto`) and the monitor card is `h-full shrink-0`, so it is exactly one viewport tall once scrolled to.
- `TimeRange::bucket_timestamps` centralizes bucket boundary generation for both LLM and MCP generators. Daily (or coarser) buckets start at the first boundary inside the range because per-day rows are stamped at midnight and the query excludes rows before `start`.
- The Dashboard refresh button reloads metrics and the monitor snapshot; live events still stream in via `monitor-event-created/updated`.
