# MCP client working directories

- [x] Trace session creation, transport reconnects and wizard forms.
- [x] Infer local MCP peer process CWD and relay stdio bridge directory; scope detection to the MCP session.
- [x] Apply detected client directory when spawning session STDIO servers, retaining configured cwd as fallback and home as the final default. Preserve effective directory on reconnects. Remote peers use fallback.
- [x] Remove cwd from creation forms and describe the retained setting as a fallback.
- [x] Add meaningful detection, isolation, spawn and fallback tests.
- [x] Plan Review: compare implementation against requirements and close gaps.
- [x] Test Coverage Review: cover new branches and error handling.
- [x] Bug Hunt: inspect isolation, process lookup races, remote peers, reconnect behavior and invalid directories.
- [x] Run local Rust formatting, Clippy, workspace tests, desktop tests/build and website build.
- [x] Commit and push: signed commit, merge with --no-ff into master, normal push; preserve unrelated changes.

Client directories are launch-time context, not authorization. Socket inference is best effort; unsupported platforms, remote peers and lookup failure retain the saved fallback. No guessing among multiple MCP roots. Existing config field and frontend/backend command schemas remain compatible.

Bug hunt: transport snapshots must retain the saved configuration rather than the inferred cwd; otherwise connection-change detection would restart a healthy server on every request. Client cwd is held on the session and reapplied on recovery.

Behavior: local HTTP/SSE sessions use a socket owner lookup on Unix when lsof is available; Windows and unavailable lookups use the fallback. The stdio bridge relays its directory on all platforms. Remote peers cannot supply a path on the host. Shared stateless identities use fallback because they cannot identify one client process. MCP roots stay separate advisory workspace boundaries. Wizard creation omits cwd; the saved settings retain the existing field with home as its unset value.

The HTTP integration regression verifies detection against the actual bound listener port (which may differ from settings), separate bridge paths for concurrent sessions sharing a token, and directory stability on later requests.

Validation: rustup stable 1.99.1; cargo fmt --all -- --check; cargo clippy --workspace --all-targets -- -D warnings; cargo test --workspace (4,903 passed, 70 ignored, 0 failed across 113 suites, including 470 MCP tests). Desktop test:unit (148 passed), desktop build, website build:check, and TypeScript checks pass. Build output retains existing bundle-size/mixed-import and dependency future-compatibility warnings. An initial overlapping build invalidated an artifact during doctests; the final sequential workspace run, including doctests, passes. The automatic models.dev catalog refresh is included.

Plan review, test coverage review, and bug hunt are complete. The actual listener port is injected as request context; a regression uses a server whose bound port differs from settings. No permission, roots, or access-grant semantics change. Existing cwd values remain saved as fallbacks. Native Windows socket-to-CWD lookup is not implemented; the stdio bridge and fallback work there. No application release or hosted validation is dispatched.
