# MCP connection error diagnosis

- [x] Diagnose Atlassian connectivity using local evidence and public endpoint probes.
- [x] Preserve HTTP and network failure details through connection setup; fail promptly on permanent HTTP errors without exposing credentials or response bodies.
- [x] Add regression coverage for rejected HTTP connections and existing inline-response fallback.
- [x] Plan Review: verify implementation against intended behaviors.
- [x] Test Coverage Review: check error, timeout and fallback coverage.
- [x] Bug Hunt: inspect cancellation, retry and redaction behavior.
- [x] Run stable toolchain workspace Clippy, formatting and tests with pinned compiler.
- [ ] Commit and push task changes and automatic catalog updates, preserving unrelated work.

## Findings and review

On October 21, the public Atlassian v2 endpoint returned HTTP 401 promptly without credentials. Using the configured Atlassian server's saved access token, a GET event-stream request timed out after 15 seconds with zero response bytes, and a POST initialize request timed out after 20 seconds with zero response bytes. Tokens were read only for these probes, never printed or modified. This reproduces a stall in authenticated requests, but cannot identify the internal Atlassian component responsible. The running application sends stdout/stderr to /dev/null; available file logs end October 16.

Connection setup currently waits 10 seconds for GET response headers. The change preserves the last HTTP/network failure and explicitly reports missing headers when no response arrives. Permanent HTTP rejections fail promptly. Response bodies and URL credentials/query parameters are excluded from the reported setup error. Existing 400/404/405 inline fallback remains intact. Retryable 408/429/5xx responses retain the existing retry policy. The connecting task guard still cancels unfinished background work on failure.

Plan review, coverage review and bug hunt completed: tests cover prompt 401/403/redirect rejection, secret omission, 503 timeout details, a server accepting TCP without sending headers, and all three inline fallback statuses. Existing transport tests exercise initialization, session preservation and token refresh after connection. This does not change OAuth configuration or claim to repair Atlassian's authenticated upstream stall.

Validation: rustup stable updated/confirmed at 1.99.1; compiler executable pinned via PATH and RUSTC. Workspace Clippy with -D warnings, cargo fmt --all -- --check, and cargo test --workspace all passed. All four new transport regression tests passed.
