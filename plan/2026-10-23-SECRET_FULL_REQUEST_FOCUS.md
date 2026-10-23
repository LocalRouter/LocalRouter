- [x] Track each secret's masked position in the complete request JSON using offsets safe for JavaScript strings and JSON escaping.
- [x] Show only the full request in the existing inline viewer, highlight the selected secret and center its scroll position automatically; keep other secrets masked.
- [x] Synchronize native/demo payloads and regression coverage, including repeated values, Unicode, escaping and multiple secrets.
- [x] Plan Review: verify the full event is the default with the correct highlighted finding and no extra popup or open/jump button.
- [x] Test Coverage Review: cover automatic positioning, absence of tabs/category, reveal/hide and correct finding selection.
- [x] Bug Hunt: inspect offsets, serialization, masking and scroll boundaries.
- [x] Validate frontend/browser checks and exact stable Rust workspace checks with pinned compiler.
- [x] Commit and push validated changes and automatic catalog updates, preserving unrelated work.

User clarification: remove the Detected source option and both tab buttons. Show only the complete request, automatically focused on the highlighted secret. Move the reveal/hide icon to the right of the secret title and remove the cloud-provider/category label.

Additional user request: replace Model with LLM source. Preserve known native-proxy provider identity, resolve explicit/unique cached gateway providers to their display names, and label automatic or unresolved routing honestly without guessing from a model name.

Plan, coverage and bug reviews completed for the final requested UI: one full-request scroll area in the original popup, automatically centered on the selected masked secret; no tabs, separate source view, category badge, open button or jump button. The reveal/hide icon sits at the right of the rule title. Model is replaced by LLM source. Native proxy requests preserve known provider identity; gateway requests use explicit or uniquely cached provider identity, otherwise report automatic/unselected routing without guessing. No provider request or routing decision is changed to determine the label.

Finding offsets use JavaScript-compatible UTF-16 positions in the complete masked JSON, verified by reconstructing the exact masked payload. JSON escaping, non-BMP characters, repeated values, different findings and neighboring secrets are covered. Only the selected value is revealed; other values remain masked. Optional new response fields preserve old payload deserialization. The shared popup, native debug fixtures, website demos and frontend types are synchronized.

All 14 browser regressions and both frontend production builds pass; compact 440px visual QA confirms the icon position, provider label, absent tabs/category and highlighted secret. Stable workspace Clippy and formatting pass with the stable compiler pinned. Final stable workspace tests passed: 4,589 passed, zero failed, 69 ignored. An initial unrelated lr-engines start_timeout_kills_the_process timing assertion failed; the complete workspace rerun passed, including that test and the new request-position/provider-source regressions.

Validated changes are committed and pushed to the configured upstream. Unrelated release plans are preserved; no catalog snapshot update was generated.
