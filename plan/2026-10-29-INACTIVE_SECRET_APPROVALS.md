- [x] Replace automatic disappearance of invalidated secret approvals with visible informational state and Dismiss.
- [x] Preserve normal successful submission closure and keep active scans blocking.
- [x] Handle backend lifecycle events, lost-session load/submit races, and expiry without implying the request was forwarded.
- [x] Add regressions for inactive approval UI and preserved active approvals.
- [x] Plan Review: compare implementation with intended behavior and address missed cases.
- [x] Test Coverage Review: review new paths and error handling.
- [x] Bug Hunt: inspect event/loading/submission races and dismissal behavior.
- [x] Validate TypeScript, browser checks, exact stable workspace Clippy, formatting and tests with stable compiler pinned after rustup update stable.
- [x] Commit and push completed validated changes and catalog updates, preserving unrelated work.

User correction: an unexpectedly invalidated approval must remain visible as information, with Dismiss. Do not automatically hide it or claim that request cancellation is the established original root cause. The origin of the reported session loss remains unconfirmed; the client is unknown.

Review findings:
- Replaced native auto-destruction and expired-popup auto-close with a window event. The desktop view turns that alert informational and retains its detected-secret/request context until Dismiss.
- Kept obsolete queued alerts instead of skipping them. Loading a missing session displays a visible informational fallback with Dismiss; loading errors remain visible too.
- Subscribe before loading details, guard late asynchronous loads after cleanup, and recheck backend state when submission fails. These cover load/event/submit races without guessing the request's final outcome.
- Inactive alerts have no approval, policy, or fresh reveal actions. Existing revealed text can still be hidden. Dismiss remains available if a policy save is in flight.
- A successful submission normally closes its own window. If closing fails, it becomes informational and dismissible rather than re-enabling approval actions.
- Added real desktop-view browser regressions via the website IPC/window/event stubs, in addition to shared-card coverage. They simulate an ended active session, missing session at load, ending during load, missing session at submit, successful submission, and window-close failure. These exercise frontend lifecycle logic; native OS window rendering is not manually exercised.
- The original reported cause of the approval session disappearing is still unconfirmed. The user's client is unknown. This change implements the explicitly requested visible fallback and does not claim to prevent an unidentified cancellation or early resolution.

Final validation: stable 1.99.1 updated, stable sysroot bin first on PATH and explicit stable RUSTC. Exact workspace Clippy, formatting, and tests passed (4642 passed, 0 failed, 69 ignored). Desktop and website TypeScript checks passed. The 21-test browser suite passed, followed by all six final desktop lifecycle regressions including the added window-close failure case. Original early-resolution cause remains unconfirmed; no live user request was forwarded or configuration changed during investigation. Historical release-plan files remain untouched.
