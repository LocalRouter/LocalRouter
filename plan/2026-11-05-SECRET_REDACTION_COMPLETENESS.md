# Secret redaction completeness fixes

- [x] Audit the shipped implementation against the approved behavior and all response transports.
- [x] Support reversible redaction through the native HTTP/SSE and WebSocket proxy paths, including Codex Responses.
- [x] Fix fragmented and structured response restoration, executable argument validation, edited arguments, and secret lifecycle gaps.
- [x] Add Notify popup option "Always dismiss this secret" for one value, retaining salted per-client digest storage and save-failure behavior.
- [x] Improve popup readability with separate Event and JWT sections; decode JWT without revealing its token, remove event pagination, and provide browser/screenshot proof.
- [x] Make settings consistent with existing pages: Redact, Ask, Notify, Off.
- [x] Add and run transport, parser, scope, leak, streaming, and executable restoration regression tests; verify OS-backed storage locally.
- [x] Plan review: reconcile every requirement and implement missed behavior.
- [x] Test coverage review: cover new paths and errors.
- [x] Bug hunt: inspect cancellation, state, framing, and fail-closed boundaries.
- [x] Run applicable local Rust/frontend/website checks and record exact results and limitations.
- [x] Commit and push validated changes with Matus identity. Release only after the complete fixes and validation, as authorized by the later request.

Preserve provider wire protocols and request-owned authorization. Never send plaintext as an unsupported-path fallback. Keep monitoring masked. Unknown or unauthorized placeholders must not restore. Retain the approved self-describing XML marker and native OS credential storage. No paid upstream model calls are authorized by this plan.

## Concrete audit results

Implemented native Codex HTTP/SSE/WebSocket and compaction, reverse-proxy JSON/SSE/Ollama NDJSON, response retrieval and scoped continuation, encrypted-state capabilities, visible reasoning summaries, legacy completion fragments, escaped/structured JSON, schema checks, approval edits and effective permissions, Ask decisions on new tool secrets, protection before response indexing, cancellation and bounded buffers, and durable cleanup for removed clients. Unsupported inference/file-upload transports block instead of silently bypassing protection.

UI: Redact / Ask / Notify / Off uses the GuardRails selector colors. Notify has the exact "Always dismiss this secret" wording and stores only the selected value's salted client-specific digest. The popup shows the secret first, Event/JWT in separate tabs, a continuous full-event viewer, independent JWT decoding, and fixed actions. The decoded-JWT command is wired through Rust, TypeScript, native registration and website mocks.

## Local validation

- Rust stable 1.99.1, confirmed unchanged by rustup update stable; compiler pinned through rustup and RUSTC.
- Workspace run: 81 executable test targets and workspace doctests. The one privacy-metadata assertion was corrected and its complete target rerun successfully; original failure and subsequent pass are retained in task logs.
- Final affected-package runs: clients 23, MCP 470 (3 intentionally ignored), server 235, desktop binary 418 (5 intentionally ignored), server-state integration 11, scanner 99 (2 native-only tests normally ignored), proxy 86 plus 19 transport integration tests. All passed.
- Explicit macOS Keychain proof: parent and fresh child process passed; isolated synthetic namespace deleted afterward.
- Workspace/all-target Clippy with -D warnings and cargo fmt checks passed.
- Frontend: 148 unit tests, desktop production build, website build/type checks passed. Popup 49, settings 2, landing/paper 6 browser tests passed. Light/dark screenshots inspected at the real 440x560 popup size.
- Integrated concurrent master website indexing changes with a signed no-ff merge; website validation repeated successfully after integration: 33 prerendered pages, 34 static outputs, and browser checks with and without JavaScript.
- Task logs and local screenshots: /tmp/localrouter-secret-fixes-logs/. Review server: http://127.0.0.1:1439/__preview/secret-redaction?jwt&notify&theme=light.

Native Windows/Linux storage and hosted model copying were not exercised locally. Binary/opaque inference and unsupported tool formats retain explicit protection errors. No paid model calls were made. The owner subsequently authorized release validation and a new stable release containing the latest master changes; publication is tracked in the release plan.
