# Debug popup delay

- [x] Add a default-off checkbox to Debug > Approval Popups that waits two seconds before invoking any popup trigger (firewall variants, sampling and elicitation).
- [x] Keep the running debug app available through Vite hot reload and verify TypeScript and the served change.
- [x] Plan Review: confirm all popup buttons honor the option and existing multiple-popup behavior remains intact.
- [x] Test Coverage Review: assess the small reversible UI change; verify immediate/default and delayed branches without adding implementation-mirroring tests.
- [x] Bug Hunt: check duplicate triggering, delay selection at click time and consistency across trigger handlers.
- [x] Commit and push the user-tested, approved debug changes to master after the required exact stable workspace checks.

The checkbox defaults to false on each mount and applies to all nine firewall variants, sampling and elicitation. Every handler sets its existing busy state before the wait, captures the current option at click time and waits before creating the backend request, so the approval timeout starts after the delay. Existing multiple-popup behavior is preserved. No Tauri commands changed.

TypeScript passed and the live Vite module serves the default-off checkbox and 2,000 ms delay. The user tested the debug changes and explicitly requested shipping to master. Existing frontend regressions and required stable workspace checks run before commit; this small reversible UI helper does not need implementation-mirroring tests.

Final validation passed: stable Rust 1.99.1 with PATH/RUSTC pinned to its compiler, workspace Clippy (-D warnings), formatting and 111 workspace test suites (4,547 passed, zero failures, 69 ignored), app/website TypeScript, 82 frontend regressions, and live Vite-module verification. Only the debug view and this plan belong to the change; unrelated release drafts are preserved.
