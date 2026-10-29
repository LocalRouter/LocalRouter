- [x] Trace sequential secret approvals and approval-window lifecycle.
- [x] Publish completion/cancellation events for blocking approvals and retire their native windows; ignore obsolete popup-open events.
- [x] Keep successful submissions disabled while closing; handle closure independently from submission errors.
- [x] Add regressions for sequential redactions, cancellation, and expiry; review obsolete-open event handling.
- [x] Plan Review: compare implementation with planned behavior and implement missed cases.
- [x] Test Coverage Review: cover new paths, boundaries, and errors.
- [x] Bug Hunt: inspect ordering, cancellation, and window races.
- [x] Validate TypeScript and exact stable workspace Clippy, formatting, and tests after rustup update stable with compiler pinned.
- [x] Commit and push preparation: validated task changes staged; commit and normal upstream push are the final execution steps.

The pending approval map is authoritative. Request cancellation currently removes the blocking session via PendingRequestGuard without notifying the native popup. Old windows can still offer actions after the request is gone. Each sequential finding must retain its own blocking approval until explicitly answered, and obsolete windows must be retired on completion, timeout, or cancellation.

Review findings:
- Request-future cancellation already discarded pending sessions, but did not retire native windows. The RAII guard now publishes a request-id-only resolution event on every exit (approval, cancellation, timeout).
- The native listener distinguishes required/resolved events, checks pending state before opening queued notifications, and retires only the matching window.
- The app-wide close handler hides auxiliary windows. Retirement destroys them while preserving the existing macOS focus restoration on the main thread.
- Frontend closure failures cannot re-enable already-consumed approvals.
- Regression coverage checks three sequential blocking redactions, cancellation during the second approval, fail-closed expiry, and a browser flow redacting every finding. Browser preview exercises shared approval UI; backend tests exercise real approval sessions/events. Native window destruction is compiled and reviewed, not exercised by the browser preview.
- TypeScript passed; all 15 secret redaction browser checks passed. Stable workspace checks are in progress.

Final validation: stable Rust 1.99.1 with stable sysroot bin first on PATH and explicit stable RUSTC. Workspace Clippy, formatting, and tests passed (4642 passed, 0 failed, 69 ignored). TypeScript and all 15 browser checks passed. The native popup lifecycle is compile-checked and reviewed; native GUI interaction was not manually exercised. Test output: /tmp/localrouter-secret-approval-tests.log. Unrelated discovery work and its catalog update were committed separately during this task; historical release plans remain untouched.
