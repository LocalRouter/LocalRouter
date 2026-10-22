- [x] Return MCP initialize within a small foreground budget, retaining built-in Skills and explicitly reporting pending/unavailable upstreams. Continue bounded remote startup in the background with shared gateway state.
- [x] Keep lists and local ping usable while startup runs; publish session-scoped list changes when backends finish. Preserve permission checks, indexing, transport/session isolation and cancellation cleanup.
- [x] Add per-server initialization timing evidence and distinguish pending, failed and working connections; do not attribute the current incident to a server without measurements.
- [x] Plan Review: compare against Claude Code's approximately 25-second deadline and backwards compatibility.
- [x] Test Coverage Review: reproduce a stalled upstream, prove prompt initialize/local Skills/list/ping, late recovery and session teardown.
- [x] Bug Hunt: inspect cloned state sharing, cancellation, late notifications and no duplicate initialization.
- [ ] Run stable workspace Clippy/fmt/tests with pinned compiler; commit and push validated worktree changes and catalog updates.
- [ ] Merge and validate the combined master tree after the separate v0.0.164 release, then publish and verify the authorized follow-up release (v0.0.165).

## Incident evidence and compatibility

The supplied client diagnostics measured 46-122 second authenticated initialization, while Claude Code stopped after about 25 seconds. The VocalCord server e06d3095-8376-48aa-a457-0c1749cce07b returned HTTP 410 account_deleted. This proves that server is unavailable, but does not identify which upstream or startup phase consumed the delay. No production credentials or server configuration are changed by this fix.

Fast upstreams retain the existing handshake result. After a two-second foreground budget, initialization returns gateway capabilities and local virtual instructions, marking upstream discovery as pending. Existing backend timeouts remain bounded; session-scoped list_changed notifications announce completed warm-up. Legacy ping checks the local gateway; stateless peers retain their existing protocol behavior and response metadata. Pending/failed client connections remain unhealthy until their own handshake/catalog operations recover, independently of successful UI probes.

Regression coverage includes a six-second upstream initialization, local SkillRead/tools/list/ping during warm-up, late upstream tool discovery, session cancellation, and concurrent stateless discovery without duplicate initialization. Review also covers the deadline/completion notification race and replacement/expired session cleanup.

Validation: the complete stable-toolchain workspace run passed 4582 tests across 111 suites (69 ignored), including all five new gateway regressions. Clippy --workspace --all-targets -- -D warnings and formatting passed. The obsolete malformed-upstream assertion now verifies explicit partial failure rather than requiring gateway failure. Final exact-tree checks after the last capability declaration adjustment also passed: 4582 tests, 69 ignored, stable workspace Clippy and formatting. Compiler: rustc 1.99.1, pinned to the rustup stable toolchain binary.

Local skill catalog indexing now precedes remote startup when compression is enabled. Per-server transport_connect, protocol_probe and initialize durations are recorded using configured server names; no current production per-server timing is claimed. A failure during one client's warm-up cannot be erased by another client's independent health probe. The first release remains separate and builds from its immutable v0.0.164 tag.
