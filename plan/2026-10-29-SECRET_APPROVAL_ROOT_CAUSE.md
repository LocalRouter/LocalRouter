- [x] Trace all approval removal paths and desktop submission/window ordering.
- [x] Reproduce both reported errors against the original handler, including a pending second secret.
- [x] Verify current behavior retains the inactive first alert while the next finding stays blocking.
- [x] Add regression coverage for session consumption, duplicate actions, and no forwarding before the final decision.
- [x] Plan Review: compare reproduction and implementation with intended behavior.
- [x] Test Coverage Review: cover original error sequence and subsequent blocking approval.
- [x] Bug Hunt: inspect window-close and backend-response races.
- [x] Validate exact stable workspace Clippy, formatting, tests and frontend/browser checks with stable compiler pinned after rustup update stable.
- [x] Commit and push completed validated changes and generated catalog updates, preserving unrelated work.

Prior commits addressed popup lifecycle but did not establish the reported failure. This investigation must reproduce a concrete code path rather than infer cancellation. Original handler placed backend submission and close() in one try/catch and re-enabled the consumed ID on either failure. Validate this path against original source and confirm the next secret is still pending.

Trace and reproduced defect:
1. `submit_firewall_approval` checks that RedactOnce targets a pending blocking scan, then `FirewallManager::submit_response` removes exactly that request ID and sends RedactOnce to its receiver.
2. `handle_secret_scan_approval` records that finding's redaction and awaits a new, different approval ID for the next finding. It cannot return a forwarding outcome until all findings have decisions. Permission re-evaluation explicitly skips secret scans. Neither path auto-approves later secrets.
3. Original desktop source (`a923e82b^:src/views/firewall-approval.tsx`) awaited backend submission AND `window.close()` in one try/catch. On any error, it reset submittingRef and re-enabled the old ID. A successful first submission followed by a close failure therefore resurrected a consumed popup.
4. A test-only Vite loader served that exact original source, without overwriting the working source. Injecting one window-close IPC failure reproduced: first "Window close failed", next Redact "Redaction is only available for a pending blocking secret scan", then Allow "Invalid parameters: Firewall approval request preview not found or expired". Test backend state was submissions=1, attempts=3, second pending=true, forwarded=false. No timeout, cancellation, or later secret approval was needed.
5. The current view separates acceptance from window teardown and leaves a dismissible informational popup. This follow-up records successful acceptance explicitly so teardown failures say "Your decision was accepted, but this window could not close" rather than implying an unknown submission outcome. Dismiss remains retryable if native closure fails again.

Evidence limits: this reproduces a concrete defect and both reported error messages against original source with mocked IPC/window operations. The original incident's native close error is not recoverable from available logs (the installed process writes stdout/stderr to /dev/null). This does not assert that its historical trigger was proven, or that cancellation can never produce similar stale-session errors.

Regression coverage: browser fixture now tracks request IDs, accepted submissions versus attempts, the pending next finding and forwarding eligibility. The close-failure regression asserts the second finding is still blocking and explicitly reviews it. A real Rust scanner regression retries Redact/Allow/Deny on the consumed first ID and verifies the second session remains pending and the scan future unfinished, then verifies both redactions. A second browser regression covers a repeated native close failure during Dismiss.

Final validation: updated stable Rust 1.99.1, stable sysroot bin prepended to PATH, explicit stable RUSTC. Exact workspace Clippy, formatting and tests passed (4643 passed, 0 failed, 69 ignored). Desktop and website TypeScript checks and all 24 browser checks passed. The original-handler reproduction also passed its assertions for both exact errors and a still-blocking second approval. Task changes are committed and pushed by the final execution step; no release/version bump or production configuration change.
