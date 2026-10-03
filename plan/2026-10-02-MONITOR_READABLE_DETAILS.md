# Monitor: readable questions, answers, and request details

## Todo
- [x] Inspect Monitor data flow, project conventions, and current detail views.
- [x] Create an isolated worktree on `feat/monitor-readable-details`.
- [x] Add bounded question and answer previews to monitor summaries, including protocol and error handling.
- [x] Hide Type when only one event type is filtered; use equal-width, single-line Question and Answer columns with ellipsis.
- [x] Replace request/response/error tabs with a single scrollable detail page: request and response together, errors in response, expandable metadata, routing, reasoning, tools, and raw payloads.
- [x] Update demo data and verify the design in a browser at wide and narrow sizes.
- [x] Plan review: compare implemented behavior and edge cases with this plan.
- [x] Test coverage review: verify content extraction, live updates, filters, and pending/error details.
- [x] Bug hunt: review overflow, missing payloads, protocol variants, and selection state.
- [x] Run frontend checks and repository-required stable Rust formatting, clippy, and workspace tests; report any environmental blockers.
- [x] Capture screenshots and obtain the user's design approval before committing.
- [x] Commit the approved task changes, fast-forward master, and push origin/master normally.

## Design
Keep event scanning compact. Derive previews from actual input/output rather than model/token metadata; retain existing summaries for diagnostics/search. Use a fixed-layout table so long text cannot widen columns or wrap rows. Show Type based on enabled event types, not currently captured rows.

Use request and response cards side by side when the detail pane has room, stacking when narrow. Put the latest user question and complete response first, keep earlier conversation and request configuration expandable, and provide prominent copy actions. Status, client, model, timing and usage remain quickly visible. Retain System One answers, tools, reasoning, transformations, routing attempts, exact payloads, and full event metadata through accessible disclosure controls. Pending and failed events render in the same response card.

## Validation
Use focused Rust preview tests, existing frontend unit tests, TypeScript/build checks, and browser interaction checks with representative local mock data. Screenshots are previews of the implemented components, not generated mockups. The main checkout's unrelated edits must remain untouched. User approval is required before check-in by the explicit task instruction.

## Review and validation results
- Implemented content-only previews in the backend summary shared by list queries and live notifications. Search includes question, answer, and the existing diagnostic summary/trace ID. Empty type selection now returns no events in both backend queries and live filtering.
- Removed detail tabs across event types. Chat, MCP, and memory compaction show request and response together. Native OpenAI, Anthropic, Responses and System One payloads remain inspectable; full response bodies take precedence over truncated previews. Disclosures mount their content only when opened to keep large conversations responsive.
- Plan review, coverage review, and bug hunt completed. Preserved duplicate-hop warnings, transformed requests, routing attempts, reasoning, tools, archive links, and full event JSON. Verified keyboard row selection and narrow-pane stacking.
- Passed: `npm run build`; website `npm run typecheck`; 22 frontend unit tests; 4 Monitor browser regression tests; 38 lr-monitor tests; stable workspace Clippy; final lr-monitor Clippy; stable workspace formatting check; `git diff --check`.
- Stable was updated/verified as rustc 1.99.0. Disabled the unavailable sandbox sccache wrapper for Rust validation; used cache-only catalog builds to avoid unrelated generated changes.
- The initial workspace test run hit sandbox restrictions in two unrelated tests (backup-file creation and a loopback callback port). The complete rerun with required access passed on top of latest master: 3,639 tests passed, 93 ignored, no failures. Stable workspace Clippy, formatting, frontend build/unit tests, and website typecheck also passed after integration. Logs: `/private/tmp/monitor-workspace-tests-approved.log`, `/private/tmp/monitor-clippy-approved.log`.
- Browser tests: `PLAYWRIGHT_CHANNEL=chrome npx playwright test --config=tests/e2e/monitor.config.ts`. Preview: http://127.0.0.1:1432/demo (open Monitor). Screenshots: `test-results/monitor/monitor-llm.png`, `monitor-error.png`, `monitor-narrow.png`.
- The user approved the screenshot and explicitly requested commit and push to master. Latest master changes have been incorporated without conflicts. All implementation changes are isolated in `/private/tmp/localrouterai-monitor-redesign` on `feat/monitor-readable-details`. Local dependency symlinks and generated screenshots are not intended for staging.
