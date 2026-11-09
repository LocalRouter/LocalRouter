# MCP discovery resilience

- [x] Trace startup, catalog refresh, notifications and session recovery.
- [x] Cache last successful catalogs per client, server configuration and working directory across sessions within the gateway lifetime. Include tools, resources, prompts and resource templates. Never cache results of tool executions or authentication state.
- [x] Serve cached discovery with explicit stale/loading metadata, current permission filtering and original routing; retain remote operation errors until ready. Replace successful families authoritatively (including empty lists), preserve failed families, notify on completion.
- [x] Bound foreground initialization to five seconds; keep upstream startup budgets independent.
- [x] Recover unavailable/disconnected backends periodically without client discovery requests; avoid overlapping retries and replaying operations.
- [x] Test slow startup, cross-client/config isolation, replacement/removal, retry and notification behavior.
- [x] Plan Review: compare implementation with this plan and resolve gaps.
- [x] Test Coverage Review: cover new failure paths and edge cases.
- [x] Bug Hunt: inspect locking, authorization, lifecycle and notification races.
- [x] Validate with stable Rust formatting, Clippy and workspace tests; document blockers.
- [x] Commit and push only task changes and automatic catalog updates, preserving concurrent work.

Disk persistence is intentionally excluded: discovery schemas can contain private data. The cache survives client session reloads but is cleared on gateway restart. A future opt-in disk cache needs retention and secure file handling.

## Implemented behavior and review

Snapshots contain tools, resources, prompts and resource templates, including original routing identifiers. Listings expose neither internal routing fields nor authentication state. Permission checks run against the current session on every cached response. Cached tool definitions carry a loading description and per-item metadata during initialization; remote operations return the existing retryable initialization error until ready.

A successful complete catalog replaces its snapshot, including empty catalogs. Invalid responses, failed pages, timeouts and page-limit exhaustion retain last-known data. Pagination shares the configured discovery deadline rather than receiving a new full timeout for every page. Snapshot identity includes client ID, working directory and connection configuration; learned capability snapshots are excluded from the key. Reconfigured transports cannot publish old data under a new configuration.

Snapshots expire after 24 hours. The memory cache admits at most 256 catalog entries, with a 32 MiB serialized-data budget and 4 MiB per family. It contains no tool execution results. The bounds are best-effort under concurrent snapshot insertions.

An independent 15-second maintenance task schedules recovery of disconnected/failed upstreams. Existing per-session initialization locking and 30-second retry backoff prevent overlapping retries; termination aborts recovery. Recovery never replays a tool operation. Catalog refresh and notification broadcasts log counts and receiver counts; broadcast receipt is not proof that a downstream client applied a refreshed catalog.

## Validation

- Initial stable Rust check for lr-mcp and lr-server passed.
- Initial gateway stability suite: 32 passed.
- Initial workspace Clippy (all targets, warnings denied): passed.
- Final targeted gateway suites: 99 mock integration tests and 33 stability tests passed; 2 existing tests ignored.
- Stable workspace Clippy (all targets, warnings denied) and formatting passed.
- Actual compiler: /Users/matus/.rustup/toolchains/stable-aarch64-apple-darwin/bin/rustc, version 1.99.1.
- Final full workspace run after the reconnect-race fix: 5,052 passed, 0 failed, 84 ignored across 113 suites (including doctests).

The full workspace run caught a first-list compatibility regression. Startup snapshots are now kept separately from session TTL caches: the first explicit discovery request still fetches the live catalog. The 99-test integration suite passes after this correction.

The incident's exact live cause remains unproven: no authenticated Datadog operations or production-session reloads were performed.


## Concurrent repair race

The full workspace run exposed a pre-existing race in concurrent expired-session recovery: one call could observe the gap between removing an old transport and inserting its replacement. Tool calls now use the explicit pre-dispatch/session-rejected error state when the transport is missing or marked for reconnection. The existing recovery path waits for the shared repair and retries only a call known not to have been dispatched. Broken response streams and timeouts retain their uncertain-outcome errors and are never replayed. A transport-level test covers the missing and connecting states plus successful use after initialization; the concurrent eight-write regression verifies shared repair and write counts.

The concurrent session-expiry/write-count regression passed 30 consecutive executions of the rebuilt test binary. The complete 33-test stability suite and workspace Clippy also pass after the repair-race correction.


Final validation logs:
- /tmp/localrouter-mcp-workspace-race.log
- /tmp/localrouter-mcp-clippy-race.log
- /tmp/localrouter-mcp-fmt-race.log
- /tmp/localrouter-mcp-recovery-stress-fixed.log

All final validation commands exited successfully. Implementation files were compared byte-for-byte with the isolated checkout before committing. No hosted CI or release/deployment was dispatched.
