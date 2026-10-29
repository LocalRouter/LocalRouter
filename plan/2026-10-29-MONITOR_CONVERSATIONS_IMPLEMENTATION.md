- [x] Inspect live Monitor capture, identity signals, privacy controls, and current UI; preserve unrelated work.
- [x] Implement bounded local conversation correlation using explicit caller session IDs and captured message-history continuity; keep request/session and transport identity distinct.
- [x] Carry conversation IDs and first-user-message header excerpts in event summaries/details; propagate to related scans and events, with privacy redaction and size accounting.
- [x] Implement the chosen UI directly in LocalRouter: Events/Conversations modes, one selected connector with dots in Events only, no links or conversation filter in grouped mode, and new activity moving its group to the top.
- [x] Add a dismissible conversation filter beside Search in Events mode. Use header excerpts or honest fallbacks, with no title-generation network calls.
- [x] Synchronize desktop types, demo mocks, documentation, and meaningful unit/browser/Rust regressions.
- [x] Plan Review: reconcile implementation and requirements; fix omissions and edge cases.
- [x] Test Coverage Review: cover identity boundaries, cross-type linkage, grouping updates, filters, redaction, and incomplete payloads.
- [x] Bug Hunt: inspect privacy, memory bounds, concurrent updates, identity collisions, selection, and layout.
- [x] Validate TypeScript, relevant builds/browser tests, and exact updated stable workspace Clippy, formatting and tests with stable PATH/RUSTC pinned.
- [x] Commit and push feature-branch changes and automatic catalog updates, preserving unrelated changes. Do not merge to master before user review.

Scope: ship the selected behavior in LocalRouter, not the throwaway standalone prototype. Conversation headers are locally derived excerpts from the earliest captured user message, not provider-supplied chat titles. Do not equate per-request UUIDs, client IDs, or MCP connection IDs with a whole conversation. Unknown identity must remain separate instead of guessing from timing or client alone. Retain chronological ordering within groups, latest-activity ordering across groups, existing event detail/replay/docking behavior, and privacy controls. Prefer existing source signals and local message-prefix matching without external fetches or new model calls. Extend the existing feature worktree/branch; master remains review-gated.


Validation: stable Rust 1.99.1, with the stable sysroot on PATH and RUSTC pinned to rustup's stable compiler. Workspace Clippy with all targets and warnings denied passed; workspace formatting check passed; workspace tests passed (4,644 passed, 0 failed, 69 ignored). Both desktop and website production builds passed. Frontend unit suite: 111 passed. Monitor browser suite in Chrome: 14 passed, including 15 interleaved conversations / 45 events, live conversation filtering, selected-only connectors, grouped ordering, and unchanged detail/replay/docking behavior. Existing large-bundle and dependency future-compatibility warnings remain.

Plan/coverage review and bug hunt completed. Fixed an unnecessary list reload when changing grouping modes and made history correlation prefer the longest unambiguous retained prefix. Explicit-ID resets, client isolation, ambiguous histories, privacy-disabled captures, bounded fingerprints, response chains, eviction/clear, and callback reentrancy have Rust regressions. No standalone-prototype changes were made. Master was not merged or modified by this task.
