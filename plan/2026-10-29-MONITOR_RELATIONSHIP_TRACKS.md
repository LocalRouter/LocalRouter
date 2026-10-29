- [x] Inspect Monitor event relationship metadata and presentation.
- [x] Add colored request tracks on the left, including cross-type events, using recorded relationships.
- [x] Add focused grouping and browser regressions, including interleaved and filtered events.
- [x] Capture and inspect a real UI preview picture for user review before merging to master.
- [x] Plan Review: compare implementation with the plan and address missed behavior.
- [x] Test Coverage Review: cover grouping, missing IDs, and interleaved relationships.
- [x] Bug Hunt: reread layout, grouping, and interaction code for defects.
- [x] Validate frontend and exact stable workspace Clippy, formatting, and tests with the stable compiler pinned.
- [x] Commit and push the feature branch and automatic catalog updates, preserving unrelated changes. Merge to master only after the user reviews the preview and approves.

Inspection correction: session_id is generated for each API request; it is not a conversation ID. This preview links request events by session_id and cross-hop trace_id. Conversation-wide links across separate requests remain unimplemented and require a conversation identity signal; do not label request groups as conversations. Preserve chronological ordering and selection. Make the distinction understandable through labels/tooltips and selected-related row emphasis; never infer links from timing, client, or text alone. Keep lines continuous across unrelated interleaved events and only connect matching visible records. Use local bundled UI only.


Review notes:
- Preserve existing chronological order; draw continuous tracks through interleaved rows, with nodes only on matching events. Recompute tracks for the current filtered results.
- Union session and trace IDs so cross-hop LLM events also connect to request scans. Canonicalize group identity independently of row order.
- Limit tracks to four lanes to preserve table space; busy overflow groups have a marker and become visible on selection. Highlight selected-related rows, dim other tracks, and provide textual descriptions and ID tooltips.
- The demo includes a secret scan related to LLM and JSON repair events. It uses synthetic fixture IDs, not captured user data.
- Corrected existing browser tests that accidentally selected the traffic table and matched both Usage regions. Collapse usage cards in test setup to give the monitor a usable detail area.
- No Rust implementation, command signatures, event capture, or network behavior changed. The required build automatically refreshed the tracked catalog snapshot; include that generated update.


Final validation: TypeScript passed; desktop and website production builds passed. All 112 frontend unit tests passed. The Monitor browser suite passed 12/13 in its final run; the remaining test timed out waiting for the app to load before any docking interaction and passed independently on retry. All request-link checks passed. Exact stable workspace Clippy, formatting and tests passed with stable 1.99.1, stable sysroot bin first on PATH and explicit stable RUSTC: 4638 passed, 0 failed, 69 ignored including documentation tests.

Preview: http://127.0.0.1:1432/demo (shared Monitor component with synthetic demo data). Light/dark overview and selected-state screenshots: /private/tmp/localrouter-monitor-relationships-{light,dark}{,-selected}.png. Branch: feat/monitor-relationships. No merge to master or deployment; conversation-wide identity tracking remains outside this request-link preview.
