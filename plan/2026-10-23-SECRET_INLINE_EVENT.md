- [x] Replace the separate event dialog with an inline viewer in the existing Secrets Detected popup.
- [x] Retain full source/request scrolling, highlighting, automatic positioning and masking; keep approval actions visible and remove window resizing.
- [x] Update browser regressions and preview for automatic inline display and existing approval flows.
- [x] Plan Review: verify there is no nested popup or window-size change.
- [x] Test Coverage Review: cover scrolling, masking, switching tabs, absence of open/jump buttons and approval actions.
- [x] Bug Hunt: inspect layout bounds, scroll positioning and pending-decision behavior.
- [x] Validate frontend builds/browser tests and required stable workspace Rust checks.
- [x] Commit and push the validated changes and automatic catalog updates, preserving unrelated work.

User clarification: render the event immediately, already scrolled to the highlighted secret. No open/view button and no jump button. The inline source replaces the separate secret-value box when context exists; the existing reveal control remains.

Plan, coverage and bug reviews complete: the event renders immediately in the existing finding card, with its own bounded scroll area centered on the highlighted secret. There is no event dialog, portal, window resize, open control or jump control. Full request JSON remains available in the adjacent tab; returning to source recenters the match without scrolling the enclosing popup. The existing reveal icon controls the highlighted value. Fallback findings without context keep their original value display. Compact previews now match native Ask/Notify dimensions. Approval actions remain visible while event text scrolls. Both removed metadata rows remain absent.

Visual QA passed at the unchanged 440px popup width and 480px Ask height. All 12 browser regressions pass, including automatic positioning, scrolling above/below the match, complete request content, masking, no extra popup/buttons and direct approval/dismissal behavior. Stable Clippy and formatting passed. App and website production builds passed. Full stable workspace tests passed: 4,587 passed, zero failed, 69 ignored.

Validated work is committed and pushed to the configured upstream. Unrelated release plans are preserved; no catalog snapshot update was generated.
