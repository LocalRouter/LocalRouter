- [x] Test the existing popup with a large event before implementation.
- [x] Bound request rendering around the selected finding and expose earlier/later sections progressively, preserving privacy and decision actions.
- [x] Add large-event browser regressions and compare render/layout timings.
- [x] Plan Review: verify intended behavior and edge cases.
- [x] Test Coverage Review: cover navigation, reveal/hide, boundaries and switching findings.
- [x] Bug Hunt: re-read implementation for stale state, incorrect offsets and unbounded rendering.
- [x] Run unit tests, desktop build and website build:check.
- [x] Commit and push validated changes, preserving unrelated work.

Baseline: real SecretScanEvent component in headless installed Chrome, 440px viewport, synthetic wrapped conversation text. Render/layout: 102,432 chars 21ms; 1,023,836 chars 228ms; 5,119,070 chars 1181ms; 10,238,134 chars 2309ms. This isolates frontend layout, not native IPC latency.

Scope extended at user request: bound large payloads across firewall/tool/model, guardrail, Push Relay, sampling and elicitation popups, and shared JSON/Monitor viewers. Add browser coverage using real components for these paths, including editable field state across pages.

Implemented 16,000-character request/payload sections and 20-item collection pages across approval payloads, arguments, model candidates, marketplace descriptions, guardrail verdicts/findings, sampling messages, elicitation fields and shared JSON/Monitor views. Editors splice changed sections into the complete original value. Elicitation keeps defaults/edits in parent state and pages enum options in groups of 100. Large Monitor messages use bounded plain source to avoid parsing the entire Markdown/JSON document for display; full copy actions retain the original payload.

Plan, test coverage and bug reviews complete. Large-event browser regressions cover all approval variants, initial secret centering, sequential findings, reveal/hide, no-offset events, both section boundaries, bounded rendering after navigation, oversized secrets, collection paging, retained form defaults/values, and editing a middle section without losing surrounding data. The wider tests caught and fixed the auto-router's full-string candidate label. Small-event behavior remains covered by the existing suite.

After benchmark, same real component and Chrome fixture: 102,432 chars 9ms; 1,023,836 chars 12ms; 5,119,070 chars 17ms; 10,238,134 chars 16ms. Only about 16KB is rendered at each size. These are frontend render/layout measurements, not total native popup startup/IPC time. The desktop executable is not rebuilt/released by this task.

Validation: 46 approval/large-event browser tests passed in the final full run; 148 unit tests and desktop/website production builds passed. Monitor suite: 13 passed, 2 failures reproduced identically on unchanged master (dock localStorage string is JSON encoded; related-event count is 5 rather than expected 4). These unrelated assertions were left unchanged. No Rust changes/checks or hosted CI dispatch. Merged concurrent master update with a merge commit and repeated local checks afterward. Task changes are committed and merged/pushed with normal signed commits; unrelated release-plan files remain untouched.
