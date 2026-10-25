- [x] Trace HTTP protocol negotiation and confirm the MCP transport specification.
- [x] Retain the negotiated protocol version per HTTP transport and use it for subsequent traffic, preserving explicit stateless metadata.
- [x] Add regressions for older-version negotiation and error responses.
- [x] Plan Review: verify legacy, modern and per-session behavior.
- [x] Test Coverage Review: inspect changed paths and edge cases.
- [x] Bug Hunt: review initialization ordering and state isolation.
- [x] Run exact stable workspace Clippy, formatting and tests; commit and push task changes and automatic catalog updates.

The server-selected initialize result must control subsequent HTTP protocol headers. Do not lower the advertised global protocol or alter server health/client failure boundaries. Reference: https://modelcontextprotocol.io/specification/2025-06-18/basic/transports#protocol-version-header .

Implementation: store the exact initialize-result version on each HTTP transport before returning either an inline JSON/POST-SSE response or a response received on the persistent SSE stream. Use it for ordinary requests, streaming requests, notifications, GET reconnections and replies to server requests. Omit the initial version header so strict servers can negotiate using the initialize body. Explicit stateless metadata retains precedence.

Review: transport-local state preserves session isolation; no global protocol downgrade, health behavior change, credentials change or Tauri command changes. Failed initialize results do not replace a previously selected version. Regression fixtures cover all four Beeper-reported revisions with JSON and POST SSE, strict notification/tool headers, persistent legacy SSE and rejected reinitialization, plus stateless metadata precedence.

Validation in progress using rustup stable 1.99.1 with PATH/RUSTC pinned to its actual compiler and RUSTC_WRAPPER disabled. The initial sandboxed transport test attempt could not bind loopback listeners; the required workspace checks run with local-server access. Installed application and live Beeper connection have not been changed or claimed verified.

Final validation: exact stable workspace Clippy with warnings denied, formatting check and full workspace tests passed: 4,601 passed, 69 ignored across 111 suites. All new protocol regressions passed. No tracked catalog update was generated. Commit/push follows validation; unrelated release plans remain untouched.
