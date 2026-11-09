# Nonblocking Secret Scan

- [x] Make masking and restoration transactional, best effort, and diagnostic-only on internal failures. Preserve client ownership and purge semantics.
- [x] Add client-scoped memory fallback for storage failures without touching native credentials during testing.
- [x] Remove scanner-state, unsupported-transport/document, session-metadata, encoding-budget and schema setup rejection paths.
- [x] Keep only explicit user Deny as a typed Secret Scan denial; timeout/dismissal/approval failure proceeds with best-effort masking.
- [x] Apply the policy to gateway, native/reverse proxy, model forwarding and MCP/tool iterations.
- [x] Make response wrappers preserve original buffered bytes and continue after restoration limits, compression, unsupported frames and incomplete streams; remove Secret Scan WebSocket sequencing rejections.
- [x] Expose accurate protection/restoration warnings; update settings/playground/docs/demo copy for best-effort coverage and memory-only persistence.
- [x] Add failure-injection regressions for every rejection category and full request/response forwarding, including ordinary mixed code and stored mappings.
- [x] Build isolated file-store app; retest installed Codex and Claude Code tools/files/replies, escaped content, restart and forced storage failure before pushing master; inject scanner failures in gateway/native integration tests.
- [x] Plan Review: reconcile every accepted decision against all call sites.
- [x] Test Coverage Review: verify failures are nonfatal except an explicit Deny.
- [x] Bug Hunt: review partial mutations, lost/duplicate bytes, cross-client isolation, purge resurrection, memory lifetime and stream state.
- [x] Run stable full workspace tests, all-target Clippy, formatting, frontend unit/build/type and website checks.
- [ ] Commit signed changes, merge --no-ff and push master after both real-client checks pass.
- [ ] Run exact-source release CI, publish next release, monitor and independently verify assets, updater signatures, native startup, Docker and website delivery.

## Implemented decisions and review

Internal scanner, storage, metadata, schema, encoding, document, protection and restoration failures are advisory. Transactional adapters preserve original requests on failed transformations. Only an explicit Secret Scan Deny returns HTTP 403, with code `secret_scan_denied_by_user`. Other authentication, model permissions, guardrails and actual transport/protocol failures retain their independent behavior.

The shared storage factory mirrors readable records and latches a client-owned memory fallback after its first backend failure. It never automatically writes a partial memory inventory over unread durable records. Malformed or foreign records are quarantined in memory without modifying the durable record. Admin purge reports when durable deletion cannot be confirmed and requires restoring storage access and restarting before retrying.

WebSocket cycles retain independent response/item identities. Identifiable overlapping responses restore independently; ambiguous events, unknown cancellations and unsupported response bytes continue unchanged with diagnostics. Codec failure releases un-emitted originals once rather than synthesizing errors or closing the connection. Work limits preserve safe edits and stop further rewriting.

Plan review traced gateway, model router, native/reverse proxy and MCP preparation paths. Coverage review added transactional fault injection, corrupt/foreign stores, storage-read/write outages, approval dismissal/timeout and explicit Deny, unsupported endpoints/documents/encodings, protocol/schema identities, concurrent WebSocket replies, malformed and binary events, incomplete streams and byte budgets. Bug review caught a callback-under-store-lock risk, source frame loss/duplication risks, ambiguous cycle identity matching and schema-property misclassification; regressions exercise those cases.

## Live evidence before master push

Sixteen installed-client runs passed across Codex and Claude Code: plain detected originals, HTML entities including numeric dashes, unresolved references preserved literally, complex quoted/multiline originals, forced storage failure and durable recovery. Every run checked replies, actual shell tool execution and exact file bytes. The isolated debug store was replaced temporarily with an empty directory to force initialization failure; its saved backup hash remained unchanged. Memory-only references worked across follow-up requests, the UI displayed the persistence warning, and Reveal/Hide worked. Restoring the backup recovered the original pre-outage references in both clients. Restart alone preserved activity timestamps. All 50 captured LLM calls from this validation window completed, with no LLM errors. Separate Secret Scan diagnostics correctly recorded unavailable persistence and unresolved references.

The actual debug UI also verified the overview, stored-secret inventory and full round-trip playground with HTML entities and escaped dashes. Native Keychain tests remain intentionally unrun; all storage failure work used mocks or the named isolated synthetic file store.

Local checks completed so far: stable all-target warnings-denied Clippy; 149 frontend tests; TypeScript; desktop frontend build; website check/build with 34 route outputs. Final full workspace and formatting results follow below.

The final runtime build passed an additional Codex and Claude Code complex-reference reply/tool/file run after the OCR advisory-path adjustment. The opaque OCR integration now asserts successful provider dispatch plus a Secret Scan coverage event; its authorization, quota, privacy and revocation checks remain green.

Final call-site review also found typed MCP error serialization after masking. That conversion now commits only a representable envelope, otherwise preserving the original error plus a warning. A field-name-overlap regression covers the actual failure. Approval expiry/cancellation regressions assert popup retirement and nonblocking fallback; independent model approval remains denying on timeout.

Final local validation passed on rustup stable: 5,089 Rust tests, 84 ignored, complete workspace/doc-tests, all-target Clippy with warnings denied, formatting, 149 frontend tests, TypeScript, desktop build and website checks. The last installed-client runs used the final runtime binary, including the MCP error rollback. All 50 LLM events in the validation window completed without errors.
