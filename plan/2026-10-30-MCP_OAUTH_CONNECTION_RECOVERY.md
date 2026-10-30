- [x] Trace idle-token lifetime, health scheduling, OAuth cache ownership, Atlassian authorization, session recovery and Datadog stream failures.
- [x] Share MCP OAuth credential state and bound token requests; preserve refresh rotation and diagnose authorization failures without replaying tool writes.
- [x] Recover failed upstream attachments and expired upstream sessions within existing gateway permissions, initialization and cache notification workflows.
- [x] Report MCP tool-level errors accurately and cover response stream interruptions without unsafe tool-call retries.
- [x] Repair and verify the configured Atlassian connection using the supported endpoint and existing permissions; avoid direct Keychain CLI reads.
- [x] Add focused regression coverage and document idle access-token versus refresh-token lifetime and remaining provider limits.
- [x] Plan Review: compare implementation with every behavior and edge case above and complete missed work.
- [x] Test Coverage Review: cover modified success, failure, concurrency, expiry, recovery and permission paths.
- [x] Bug Hunt: review races, cancellation, refresh rotation, session identity, safe retry boundaries and secret redaction.
- [x] Validate with rustup stable workspace all-target Clippy, formatting and workspace tests; run frontend and website checks if shared inputs change.
- [ ] Commit task changes and automatic catalog updates, integrate into the original branch without disturbing unrelated changes, and push its configured upstream normally. Report local validation separately from hosted CI and installed-app state.

# MCP OAuth and connection recovery

Work in an isolated worktree. Source audit already established browser OAuth refresh at connection acquisition and on HTTP 401, separate OAuth/keychain caches in main startup, retained failed session attachments, and monitor success detection that ignores MCP result.isError. Fresh authenticated Upwave gateway probes reproduce Jira scope errors inside HTTP 200 while identity and granted-site discovery succeed. Claude Code logs separately record a Datadog response-body stream failure. Do not assume all failures share one cause, blindly refresh on arbitrary tool text, or replay mutating tool calls after an uncertain transport failure. Preserve local-only defaults, exact permission checks and provider issuer binding. No direct security CLI credential reads: previous diagnostic reads caused intrusive Keychain prompts.

Investigate and use existing native initialization/attachment paths before adding lifecycle infrastructure. Health probes may acquire/refresh credentials but do not promise indefinite provider grants; inspect periodic defaults and provider refresh-token policies. Follow the project normal push policy; do not dispatch a release or hosted CI without explicit authorization.

## Authorized release follow-up

- [ ] Verify the current upstream and latest stable release; choose the next unused patch version immediately before dispatch.
- [ ] Validate the exact integrated release candidate locally and through exact-SHA hosted CI as release validation.
- [ ] Dispatch the standard release.yml workflow, preserving its normal version, signing, asset and packaging behavior.
- [ ] Watch the release run by ID and verify tag ancestry, updater metadata, signatures and normal Docker/package follow-ups.
- [ ] Commit and push the final validation/release record; report installed-app state separately.

## Live findings

The installed configuration enables periodic health checks every 600 seconds. Authenticated MCP health probes acquire or refresh browser OAuth tokens. With checks disabled or the app stopped, refresh is deferred until a connection/request needs credentials. Provider refresh-token revocation, absolute expiry and consent-policy expiry remain authoritative; health checks cannot guarantee indefinite grants. Google External/Testing grants can expire after seven days even when used.

Changed only the configured Atlassian URL through the running app from /v1/mcp/authv2 to /v2/mcp. Existing granted-site discovery reported Upwave and Jira scopes, but v1 Jira calls failed with a scope mismatch inside HTTP 200. A fresh v2 gateway session successfully ran the same read-only Jira search. The supported tool catalog changes between endpoints. Temporary diagnostic sessions were deleted after verification. Raw OAuth credentials were never printed or read via the security CLI during implementation.

## Implementation and review

MCP startup now shares one OAuth manager and CachedKeychain with browser login and the unified token exchanger. Browser OAuth credentials are resolved before ordinary, notification and streaming requests, while HTTP 401 forces at most one refresh. Refresh rotation is preserved. Token requests have bounded connect/total timeouts, refuse redirects and expose only standard OAuth error codes in logs/UI. Short-lived access tokens receive a proportional expiry buffer; the MCP resolver does not subtract a second margin. Updating a registered-app secret uses a fresh keychain reference so a cached previous secret cannot survive the edit.

Gateway recovery reuses the existing attachment/handshake workflow and session initialization lock/abort handle. Discovery-triggered retries run in the background with a 30-second per-server cooldown, leaving healthy and local tools responsive. Affected direct calls can wait for that repair. Transport configuration snapshots detect URL/auth changes. Recovery preserves client identity, permissions, approvals, tool activations and response-index content, refreshes catalogs/index entries and sends list-changed notifications. It re-probes protocol support so a formerly legacy backend can recover as stateless. Teardown cancels recovery and orphaned transports are closed.

An explicit upstream HTTP 404 on a negotiated MCP session is a typed session-expiry rejection. Tool dispatch repairs that backend and retries the rejected call once. Broken/partial response streams and other uncertain transport failures are returned without replaying tool writes; the affected transport is rebuilt for a later request. Late responses from replaced transports cannot mark a new connection failed, including concurrent expired-session responses. Monitoring treats MCP result.isError as failure and retains the upstream tool response.

Coverage review includes expired/rotated/replaced/revoked credentials, short-lived tokens, normal and streaming HTTP 401 refresh, real HTTP session expiry, eight concurrent expired-session calls, a partially received SSE response after a write, retained healthy tools and session identity, retry cooldown, revoked server access, endpoint changes, modern protocol negotiation and refreshed indexes, nonblocking discovery and teardown cancellation, and tool errors inside HTTP 200. The complete gateway stability suite passes all 28 cases. An initial test fixture dropped its healthy MockServer handle; retaining the handle corrected the test, without a product-code workaround.

Plan review and bug hunt checked cancellation, session ownership, permission boundaries, protocol fallback, stale catalog entries, refresh-token preservation, concurrent old responses and safe replay boundaries. No Tauri command signature, frontend schema or endpoint contract changed. The typed upstream expiry maps to HTTP 502 if it escapes repair, preserving the downstream session.

Live verification: after the authorized Atlassian URL change, read-only Jira search succeeds. A fresh Datadog MCP session also successfully retrieved the DDSQL specification through its read-only tool. All temporary live sessions were deleted. The installed app remains 0.0.171 while release validation proceeds; the new runtime code is not claimed installed yet.

The final rotation audit found and fixed caller-cancellation during refresh: a bounded owned task retains the per-server refresh lock through token commit, so outer health/connection deadlines cannot discard a rotated grant. Network refresh and token commit are separated; commit verifies the original grant still exists under the same cache lock used by revocation. Explicit browser reauthentication clears the prior grant after starting the callback flow. A late refresh cannot restore revoked credentials or overwrite that reauthentication. A fixture aborts the original caller after the token request arrives and proves one exchange persists the rotated token; a second abort/revoke path proves no credentials reappear.

Final local stable workspace Clippy with all targets and warnings denied passes; formatting check passes. Full workspace tests pass on the same pinned stable 1.99.1 compiler: 4,713 passed, 69 ignored, zero failed across 111 suites. Existing dependency `block 0.1.6` emits a future-incompatibility notice; no Clippy warnings were suppressed. No frontend or command schema changed, so local frontend builds are not required for this patch; the authorized exact-candidate release CI includes frontend and website builds.

Final pre-commit validation: stable 1.99.1 compiler resolved to /Users/matus/.rustup/toolchains/stable-aarch64-apple-darwin/bin/rustc, with its sysroot/bin first on PATH and RUSTC pinned; sccache was disabled for these commands only. Workspace all-target Clippy with -D warnings, cargo fmt --all -- --check, git diff --check and cargo test --workspace pass. All 28 gateway stability regressions pass within that full run, including the partial-SSE-body failure assertion. Validation logs are /tmp/localrouter-mcp-recovery-{clippy,fmt,workspace}.log. The automatically refreshed models.dev snapshot is included under the project catalog policy. No package/script/frontend implementation changed.
