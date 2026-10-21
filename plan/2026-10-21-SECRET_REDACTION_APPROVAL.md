- [x] Trace secrets scanning, approval persistence, request mutation, and preview facilities.
- [x] Add request redaction with clear markers and per-client salted hashes for always-redacted secrets; keep existing allow/dismiss behavior.
- [x] Add Redact once / Always redact this secret to blocking approvals and management UI; synchronize command types and demo mocks.
- [x] Add regression coverage for redaction, repeated/overlapping findings, persistence and approval behavior.
- [x] Prepare and visually verify a local interactive preview for user review before shipping.
- [x] Plan Review: compare implementation with intended behavior and fix omissions.
- [x] Test Coverage Review: cover new paths, edge cases and error handling.
- [x] Bug Hunt: review mutation, persistence, concurrency and bypass behavior.
- [x] Validate frontend types and stable Rust workspace checks with pinned compiler.
- [x] Commit and push completed validated changes and automatic catalog updates only after user reviews the preview, preserving unrelated edits.

Review adjustments: one blocking popup per secret, shown sequentially; Block stops the sequence. Redact/Allow advance without forwarding until every secret has a decision. The secret is the largest text. Always choices live in the respective Redact and Allow dropdowns; no ignore checkbox. Dark mode Redact is dark yellow (#807000) with white text.

Plan, coverage, and bug reviews completed: legacy allowed-secret entries remain compatible; remembered redactions take priority over temporary bypasses; mixed requests keep new secrets gated; one-time choices never persist. Redaction rewrites chat, System One and native proxy payloads, including repeated values, overlaps, UTF-8 and decoded text attachments. Policy-save errors leave the popup open. The development-only preview is absent from production bundles.

User approved committing and pushing to master only; no release. Final validation uses stable rustc 1.99.1 with PATH/RUSTC pinned and the compiler wrapper disabled because sandboxed sccache cannot run.

Final validation passed: stable workspace Clippy with -D warnings, cargo fmt check, and all 4,557 Rust tests (65 ignored); 86 frontend regressions and 10 browser integration tests; app and website type checks and production builds. Backend regressions verify serial approval, stopping on denial, selective redaction, per-client policy isolation, hash reload, removal/cache invalidation, and bypass behavior. Browser tests verify the final dropdowns, large secret display, dark-yellow/white Redact and Modify buttons, persistence failure handling, and single/multiple-secret flows. Committing and pushing to master completes the authorized task; no version bump, tag, release or deployment.
