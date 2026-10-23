- [x] Inspect the existing context payload and shared event/dialog patterns.
- [x] Preserve full masked source text and full masked request JSON in pending approvals, before filtering remembered secrets.
- [x] Add a large scrollable event dialog with Source and Full request tabs, opening at the highlighted match and allowing scrolling in both directions.
- [x] Update native/website demos and browser/backend regressions for complete content, masking, scrolling and navigation.
- [x] Plan Review: check full-event behavior against the requested ability to inspect all surrounding content.
- [x] Test Coverage Review: cover full payloads, source selection, masking, Unicode and dialog interactions.
- [x] Bug Hunt: review serialization, scroll targeting, popup actions and state cleanup.
- [x] Validate frontend/browser checks and required stable Rust workspace lint, formatting and tests with pinned compiler.
- [x] Commit and push validated changes and automatic catalog updates, preserving unrelated work.

Also remove the scan-time and finding-count rows from the Secrets Detected popup, as requested. Full-event viewing preserves secret masking, enlarges the native popup while open and restores its compact size on close.

Review complete: complete source content replaces bounded excerpts; the full request retains other messages and parameters. Both tabs scroll independently. The source opens at the highlighted match and Jump to secret restores its position. All detected values are masked before filtering remembered policies. No-findings requests avoid cloning/serializing a popup payload. Reveal/Hide remains explicit and reveal errors are visible in the dialog. The native window grows to 860×720 for inspection and restores its compact size when closed, with queued resize operations. Both requested metadata rows are gone. Keyboard close returns to the pending decision without forwarding. Dialog colors inherit the current theme, including website previews rendered through a portal.

The 12 browser regressions cover full-source scrolling above/below the match, jump navigation, full-request fields, masking after reveal, tab switching, Notify dismissal, metadata removal and existing approval flows. Stable workspace Clippy and formatting passed. Final app/website builds and all 12 browser tests passed. The full stable Rust workspace suite passed: 4,587 passed, 0 failed, 69 ignored. Visual inspection passed at the native 860×720 event size.

Completed changes are committed and pushed to the configured upstream. Unrelated release plans remain untouched; no automatic catalog snapshot update was generated.
