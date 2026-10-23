- [x] Trace MCP health refresh and retained client initialization failures.
- [x] Make explicit/periodic server health probes report current server availability independently of retained client session failures; preserve session diagnostics and failure callbacks.
- [x] Add regression coverage for failed initialize followed by successful independent health probing, including a genuinely unavailable server.
- [x] Plan Review: verify refresh performs a real probe and does not alter client session state or permissions.
- [x] Test Coverage Review: cover retained initialization and discovery errors, recovery, and failed probes.
- [x] Bug Hunt: review health cache update ordering and session failure lifetime.
- [x] Run stable workspace Clippy, formatting, and tests with verified compiler; commit and push task changes and automatic catalog updates.

## Findings and behavior

The global health refresh called `get_server_health`, which returned the first retained client-session failure before running any protocol probe. A long-lived session with a failed initialize could therefore keep the resource Unhealthy indefinitely, despite successful Try It Out requests on independent transports.

Server refresh now always checks the UI-managed running transport or performs the existing isolated authenticated MCP handshake and catalog probe. Client failures still update the cache when they occur and remain in per-session state for diagnostics and per-operation recovery. A refresh reports current server availability; it does not repair or claim recovery of an existing failed client session. This deliberately revises the earlier availability-audit behavior that made any retained session failure override all server probes.

Plan/coverage review and bug hunt: verified both transport paths share the changed decision, actual unavailable servers still fail protocol discovery, repeated successful probes do not erase client failures, unrelated successful catalog operations do not erase failed operations, and no commands, API schemas, permissions, credentials or session lifecycle are modified. Regression tests retain a failed initialize across repeated probes and cover availability loss/recovery, plus retained tools/list failure and independent client recovery.

Validation: stable updated/confirmed at rustc 1.99.1, actual compiler path verified and pinned via PATH and RUSTC. Workspace Clippy with warnings denied and formatting passed. Full workspace tests passed: 4585 passed, 69 ignored across 111 suites, zero failures. Both health regressions passed.
