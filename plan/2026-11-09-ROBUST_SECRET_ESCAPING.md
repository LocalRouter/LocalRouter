# Robust reversible secret handling

- [x] Implement bounded shared encoding-aware masking/restoration: JSON, quoted code strings, Unicode escapes, HTML entities, mixed/nested layers, both marker names.
- [x] Preserve unresolved/malformed references and emit metadata-only diagnostics without failing responses.
- [x] Limit schema validation to actual modifications; pass unrelated schemas and provider content unchanged.
- [x] Isolate single-entry purge from unrelated in-flight requests while preventing resurrection.
- [x] Expand regression tests for all review findings, escaping combinations, fragmentation, storage errors, and isolation.
- [x] Refresh Secret Scan global/client tabs and use a genuine in-memory scanner/vault round-trip playground, including escaped responses and diagnostics.
- [x] Build isolated file-storage debug app; run real Codex and Claude Code tool/file/reply tests, restart and unresolved-reference tests before pushing master.
- [x] Plan Review: reconcile implementation and requirements.
- [x] Test Coverage Review: cover boundaries and failure handling.
- [x] Bug Hunt: reread code for safety, resource limits, races, and corruption.
- [x] Run required Rust and applicable frontend/website checks.
- [ ] Commit signed changes, merge with --no-ff, and push master only after real-client success.
- [ ] Validate exact master SHA in hosted CI; dispatch release, monitor all jobs, verify artifacts, signatures and delivery.

## Review and test notes
- The release must include MCP discovery resilience (`99a0e3b5`, including `d8b94fab`), merged into this worktree before combined validation.
- Encoding transforms preserve source spans and reapply quoting/entity/percent layers, with nesting, decoded-byte, edit-count and output budgets. Outbound masking failures stop dispatch; inbound unresolved/unsafe transformations preserve original content and emit metadata-only diagnostics.
- Restoration notifications are committed only after structural transformation succeeds. A duplicate-key collision preserves the original response and emits no false Secret Restored event.
- Explicit store reveal uses an owned lookup so stale references produce an administration error rather than appearing to be revealed originals.
- The playground invokes the production scanner/vault with a fresh zeroizing memory store per call; it does not use native credentials or retain references.
- Native UI checks verified round-trip output, HTML-escaped dashes, unknown-reference pass-through, 38 pattern definitions, and client-scoped store tabs.
- Live initial runs passed exact file bytes and restored reply content for ordinary, complex/multiline, HTML-escaped and unknown references with both installed clients. Claude added explanatory prose to one ordinary-secret reply; its restored content and file bytes were correct.
- The debug restart was healthy and left both client activity timestamps unchanged. Combined-build client checks remain a pre-push gate.

## Final pre-push validation
- Combined source includes MCP discovery resilience (`99a0e3b5`, `d8b94fab`). Rust workspace including documentation tests: 5,066 passed, zero failed, 84 ignored. Stable all-target Clippy with warnings denied and formatting passed.
- Root: 149 frontend tests, TypeScript and production build passed. Website build and 34 static-route checks passed. Native global/client tabs and the real playground were inspected.
- Final combined debug binary: all six Codex/Claude Code complex-original, HTML-escaped alias and unknown-reference cases passed exact file bytes and exact replies. References issued before restart resolved afterward; activity timestamps were unchanged by restart.
- One initial Claude run wrote correct bytes to a different directory. The harness was made explicit about the absolute output path; the complete final matrix then passed.
- Direct firewall-policy regression verifies forwarded code bytes before Monitor masking contain no encoded known original. Monitor export is supplementary evidence, because it independently masks payloads.
- Native OS credential-store smoke tests were deliberately not run after the prior permission dialogs. Real client tests use an isolated file store and never access production mappings.
- Plan/coverage/bug review completed: bounded nested escaping, partial-stream boundaries, unknown/malformed references, lookup failure, transactional JSON-key collisions, advisory schemas, purge failure/retry and stale-vault resurrection are covered.
