- [x] Inspect approval popups, scanner source labels, and existing preview/tests.
- [x] Add bounded, masked context with source location to secret findings before filtering remembered values; preserve in-memory reveal and approval behavior.
- [x] Add an inline View context expander with a highlighted match, using the GuardRail popup's content styling; update demos.
- [x] Plan Review: compare the implementation with the intended behavior and address omissions.
- [x] Test Coverage Review: cover masking, nearby secrets, Unicode, missing context, and popup interactions.
- [x] Bug Hunt: inspect source selection, filtering, serialization, and approval state.
- [x] Validate frontend types/browser regressions and exact stable Rust workspace checks with pinned compiler.
- [x] Commit and push completed validated changes and automatic catalog updates, preserving unrelated work.

Plan, coverage, and bug reviews: the shared card uses an inline native disclosure with keyboard support, source/message number and line, first-occurrence labeling, and an orange highlighted match. Both Ask and Notify use the same context. The source comes from the exact scanned text part rather than the last user message. Context is bounded to 180 characters on either side and masks all detected values before remembered policies are filtered. The existing explicit Reveal/Hide control governs the highlighted secret too. No new command or external asset is needed; optional context preserves older payload compatibility.

Visual QA passed at 440px width. All 12 popup browser regressions passed, including expanding/collapsing context, masking/reveal/hide, Notify dismissal and existing approval flows. App and website type checks and production builds passed. Stable rustc 1.99.1 was updated and verified with PATH/RUSTC pinned and RUSTC_WRAPPER cleared; workspace Clippy with warnings denied and formatting checks passed. Full stable workspace tests passed: 4,587 passed, zero failed, 69 ignored. The two new context regressions pass; the complete suite was rerun after correcting a test fixture to satisfy existing scanner token boundaries.

Validated work is committed and pushed to the configured upstream, with unrelated release plans preserved. No catalog snapshot update was generated.
