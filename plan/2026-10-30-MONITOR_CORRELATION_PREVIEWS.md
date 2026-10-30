- [x] Inspect Monitor grouping, capture sites and question/answer extraction; preserve unrelated changes.
- [x] Fix request/conversation identity propagation for secret scans and firewall decisions, including checks before LLM capture.
- [x] Fix missing previews for supported request/response payloads, tool exchanges and large bodies while honoring capture privacy.
- [x] Add meaningful regression tests for actual failure paths and identity/privacy boundaries.
- [x] Plan Review: reconcile implementation against all planned behavior and fix omissions.
- [x] Test Coverage Review: inspect modified paths, edge cases and error handling; add missing coverage.
- [x] Bug Hunt: reread identity, extraction, memory and concurrency behavior for defects.
- [x] Run applicable local validation with latest rustup stable and record exact results.
- [x] Commit and push completed validated changes and any automatic catalog updates, preserving unrelated work.

Use explicit recorded relationships and caller identity; never join unrelated traffic merely by client or timing. Retain bounded previews, payload redaction and existing grouping UI semantics. Investigate all supported API paths before choosing a narrow implementation.

Implemented request IDs across gateway secret scans, guardrail scans/approvals,
model approvals and compression, plus HTTPS proxy checks and recorded calls.
Standalone secret scans generate an ID shared by sequential decisions. WebSocket
turns receive separate IDs; handshake traces remain available for hop accounting
but are excluded from relationship keys because they identify the connection.
Capture redaction continues to remove identities, previews and conversation data.

Previews now handle incremental tool results, custom tool calls, refusals and
document placeholders. Thinking/reasoning blocks remain excluded. Oversized
Unicode excerpts shrink within the existing capture budget instead of being
dropped. Events without an actual question/answer remain empty; disabled capture retains
its existing payload-not-retained placeholders. No content is fabricated.

Coverage review includes real sequential secret approval/denial lifecycles,
parent conversation inheritance, cross-request separation, disabled capture,
proxy approvals preceding parent capture, distinct WebSocket cycles, real model
denial via HTTP, and preview/truncation regressions. Bug hunt caught and fixed
connection-wide trace IDs linking unrelated WebSocket turns.

Validation on macOS with updated rustup stable 1.99.1, stable sysroot first on
PATH and RUSTC explicitly pinned: workspace tests passed (4,699 passed, zero
failed, 69 ignored across 111 suites). Workspace Clippy with all targets and
warnings denied, formatting and git diff checks passed. The 15 Monitor frontend
presentation/relationship regressions also passed. The first workspace test run
caught an incorrect new test expectation about disabled-capture placeholders;
that assertion was corrected and the entire suite passed on rerun. No frontend
source or Tauri command contract changed, so frontend builds were not needed.
The build automatically refreshed modelsdev_raw.json (8,925 models); include it.
Existing block 0.1.6 future-compatibility notice remains. No hosted CI dispatch,
application release, or running-app restart is part of this change.
