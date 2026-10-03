# Dashboard traffic and Monitor snapshot

## Todo
- [x] Inspect the dashboard, data contracts, app shell, and repository conventions.
- [x] Create an isolated `feat/activity-home` worktree and preserve unrelated master changes.
- [x] Replace lifetime counters, the connection diagram, and metrics tabs with an LLM/MCP request timeline using area/line charts.
- [x] Show only “Dashboard” as the heading, without a subtitle or header action buttons.
- [x] Reuse Monitor's event table and detail component for a live snapshot of the latest eight events.
- [x] Integrate the Monitor redesign from master, including the shared question/answer columns.
- [x] Remove the right-hand column and its data loading/helpers; use full width for traffic and recent activity.
- [x] Verify light/dark themes, compact layouts, time ranges, event inspection, live updates, empty states, and failures; capture review screenshots with demo data.
- [x] Obtain the user's design approval and implement the requested simplifications.
- [x] Plan review: compare final behavior with the approved scope.
- [x] Test coverage review: cover graph alignment/totals, unavailable data, duplicate-hop classification, live snapshot races, and error handling.
- [x] Bug hunt: review request/detail races, timestamps, stale data, navigation, and overflow.
- [x] Run frontend build/type checks, unit/browser tests, and required stable Rust workspace validation.
- [ ] Commit only task changes, integrate into master, and push normally.

## Final design
A full-width request traffic chart shows the selected rolling period (1h, 24h, 7d, or 30d), LLM/MCP counts, and estimated LLM cost. Hover details and explicit bucket units explain the timeline. Metrics refresh every 15 seconds; unknown data remains distinct from zero traffic.

Recent activity directly reuses Monitor's EventList and EventDetail components. The snapshot shows all event types, including diagnostic events and marked duplicate hops. The in-progress request indicator excludes duplicate hops and internal processing steps. Row selection opens the shared inspector, and View all opens Monitor. No separate activity filters, health panels, client rankings, or connect/open-monitor header buttons remain.

## Validation
- Main app production build and app/website TypeScript checks pass.
- All 25 frontend unit tests and four dashboard browser integration tests pass. Browser coverage includes the shared question/answer columns, row details, a live completion arriving during a stale snapshot, empty/unavailable/retry states, expired details, and 1100/900/700px layouts.
- Stable Rust verified at rustc 1.99.0. Workspace Clippy with `-D warnings`, formatting, and full workspace tests pass. Logs: `/private/tmp/activity-home-clippy.log` and `/private/tmp/activity-home-workspace-tests.log`.
- Existing build notices remain for chunk size, Browserslist age, and a dependency's future Rust compatibility. No backend changes in this task.
- Integrated master through `a1a08e64`, including Monitor redesign `65f68a4b`, without conflicts. Local dependency symlinks and screenshot artifacts are excluded from staging.
- Review screenshots: `/private/tmp/localrouterai-activity-review/overview-light.png`, `overview-dark.png`, and `overview-compact.png`. All use demo data.

The plan was saved directly because `copy-plan.sh` accepts only the external Claude plans directory. The user approved the screenshot design and then requested the heading, shared Monitor snapshot, and removal of the right-hand column/header actions.
