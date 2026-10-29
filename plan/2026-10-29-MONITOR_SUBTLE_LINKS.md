- [x] Soften the selected relationship connector, dots, and related-row emphasis without changing grouping or filtering.
- [x] Remove the standalone selection stripe so singleton events cannot look connected.
- [x] Anchor a visible selected conversation event during incoming updates, preserving latest-activity ordering while compensating scroll position before paint. Manual scrolling updates/releases the anchor; switching modes or selections resets it.
- [x] Replace Events/Conversations buttons with a Filter-style ordering popover: ordering icon, selected By timestamp / By conversation label, accessible radio choices.
- [x] Make relationship dots accessible buttons toggling the existing conversation filter without changing selection/details; keep the toolbar chip synchronized.
- [x] Plan Review: verify both requirements against the implementation.
- [x] Test Coverage Review: use existing Monitor regressions and inspect grouped/singleton selections; no new CSS-mirroring tests.
- [x] Bug Hunt: inspect connector endpoints, singleton rendering, and selection visibility.
- [x] Validate existing browser checks and capture actual local UI screenshots.
- [x] Commit and push only after user approval; the user's explicit review-first restriction overrides the standing push rule.

Verified both existing relationship browser regressions (2 passed), including 15 interleaved conversations. Direct browser inspection confirms singleton selection has no connector/dots or visible left border. Visual review confirms lighter 2px connectors, smaller translucent dots, and no related-row shading. Changes remain local for user review.

User subsequently explicitly authorized pushing to master and cutting the next stable release, including committed secret-scanning fixes. The review-before-push restriction is superseded by that authorization. All 15 Monitor browser regressions passed, including stable row anchoring, manual scroll-away behavior, ordering-popover selection, and dot-click/keyboard filter toggling.
