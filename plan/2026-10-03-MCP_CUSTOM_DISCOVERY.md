# Custom MCP discovery

## Progress / todo
- [x] Verify latest MCP server discovery and OAuth metadata specifications and existing transports.
- [x] Add bounded, temporary URL/command probing: server/discover, legacy initialization fallback, authentication challenge and OAuth metadata detection. Never guess proprietary credential names or values.
- [x] Add a single URL-or-command entry and explicit Discover action to Custom; fill detected settings and identity, preserve manual overrides and ignore stale results. Keep existing manual transport, headers, environment, and auth fields.
- [x] Synchronize Tauri registration, response/request TypeScript types, and demo mock.
- [x] Add meaningful backend/browser tests for modern/legacy discovery, authentication, errors, cleanup, and overrides.
- [x] Plan review: reconcile implementation with intended behavior and edge cases.
- [x] Test coverage review: exercise uncovered new behavior and error paths.
- [x] Bug hunt: inspect cancellation, process/session cleanup, credential handling, and async state races.
- [x] Run stable toolchain workspace clippy, fmt, tests, frontend/website type checks and targeted browser tests.
- [x] Commit and push task changes and automatic catalog updates. The user's later ALL-changes release request also authorizes including validated wrapper startup recovery edits.
- [x] Restart and foreground updated debug app. The user subsequently lifted the release hold; tracked in RELEASE_0_0_149.

## Behavior
MCP defines server/discover for server identity/capabilities and OAuth Protected Resource/Authorization Server metadata discovery. URLs imply HTTP; commands imply STDIO. An explicit Discover action probes only the specified endpoint/command with provided headers/environment, does not execute tools, create persistent servers, register OAuth clients, or log in. Standard protocol headers are managed by the transport. Results recommend OAuth/browser or Bearer only when advertised; arbitrary required vendor headers/environment variables cannot be reliably inferred. Failed probing leaves manual configuration usable. User overrides remain editable and changing inputs invalidates in-flight results.

## Review notes
- Modern server identity supports both reserved result metadata and compatibility serverInfo.
- Legacy HTTP sessions, legacy SSE message endpoints, and STDIO initialization are covered.
- POST-only and legacy GET OAuth challenges are both supported; scope hints, path-aware OAuth/OpenID metadata and issuer validation are covered.
- Explicit authentication edits (including selecting None again) and edits during pending discovery are protected.
- Temporary subprocesses are killed; SSE connection setup now aborts reconnect tasks when cancelled, and dropping transports closes their streams.
- Redirects are reported instead of forwarding custom credential headers to another endpoint.
- Discovery does not attempt to guess arbitrary API-key names or environment variables; these remain manual, editable inputs.

## Validation
- Updated rustup stable (Rust 1.99.0).
- Stable cargo clippy --workspace --all-targets -- -D warnings: passed.
- Stable cargo fmt --all -- --check: passed.
- Stable cargo test --workspace, including doctests: passed.
- Application and website TypeScript checks and production frontend build: passed.
- Seven remote MCP browser tests and 31 frontend unit tests: passed.
