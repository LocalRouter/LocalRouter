- [x] Trace discarded diagnostics and global health handshake; inspect configured Datadog evidence.
- [x] Use authenticated isolated protocol readiness for global health and preserve operation/error details in health and session diagnostics.
- [x] Add regressions for initialization, JSON-RPC/transport failures and monitor messages.
- [x] Plan Review: check implementation against the plan and address omissions.
- [x] Test Coverage Review: cover success, failure, recovery and diagnostic paths.
- [x] Bug Hunt: inspect cancellation, transport ownership and retained-session isolation.
- [x] Run stable workspace Clippy, formatting and tests; report actual results.
- [x] Commit and push task changes and automatic catalog updates, preserving unrelated changes.

## Findings and implementation

Live installed-app verification: Datadog Upwave Try It Out reports `OAuth browser flow error: Token refresh failed with status 400 Bad Request: invalid_grant; sign in again because the refresh token or grant is expired or revoked`. Settings reports Not authenticated and offers Authenticate. Restoring the grant requires the user's Datadog sign-in; no credentials or scopes were changed. The running app remains unchanged.

Global transports are opened without MCP initialization. Health now uses the existing isolated authenticated handshake/discovery workflow, including capability-aware tool discovery and temporary-transport cleanup, rather than sending tools/list on an uninitialized or stale global session. Current availability stays independent of retained client diagnostics. Health status remains Healthy when a global transport exists and the fresh probe succeeds, Ready otherwise.

Health and monitor messages preserve the failing stage, underlying transport/authentication errors, JSON-RPC error code/message/data and malformed discovery result details. Timeouts name their stage. Startup, reconnect, request, streaming-open and interrupted-stream diagnostics preserve their causes while keeping transport ownership checks and avoiding retries of uncertain tool writes.

Regressions require a negotiated session before tools/list, cover outage/recovery, exact monitor error propagation, JSON-RPC errors and malformed tools results, rejected initialize/discovery, and stored/callback transport and stream errors. An older missing-executable assertion now checks the raw OS error instead of a generic replacement.

Plan and coverage review and bug hunt confirmed temporary cleanup, independent session diagnostics, callback details, stateless fallback, capability-aware discovery and unchanged write replay behavior. No frontend, Tauri schema or endpoint contract changed.

Validation: rustup stable 1.99.1 at /Users/matus/.rustup/toolchains/stable-aarch64-apple-darwin/bin/rustc. Workspace all-target Clippy with -D warnings and cargo fmt --all -- --check pass. Existing dependency block 0.1.6 emits a future incompatibility notice. Full workspace tests pass: 4,895 passed, zero failed, 70 ignored across 113 suites. Separate lr-mcp tests pass: 463 passed, zero failed, 3 ignored. git diff --check passes. No frontend checks or hosted CI/release dispatch are required for this backend-only change. Logs: /tmp/lr-health-{clippy,fmt,workspace,mcp}.log. Automatic tracked model-catalog refresh is included as required.
