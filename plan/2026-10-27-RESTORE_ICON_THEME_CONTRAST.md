# Restore service icon theme contrast

- [x] Audit bundled logos and restore white-on-dark treatment only for monochrome assets; preserve full-color and discovered favicons.
- [x] Replace GitHub's opaque image with its official transparent mark and document source/license.
- [x] Render actual icons in light and dark mode and verify contrast and transparency.
- [x] Plan Review: verify image failure fallbacks and shared sizes remain correct.
- [x] Test Coverage Review: cover theme styling and color preservation.
- [x] Bug Hunt: inspect source selection, fallback and theme behavior.
- [x] Run frontend checks and required stable workspace Clippy, formatting and tests.
- [x] Commit and push the validated changes and automatic catalog updates, preserving unrelated changes.

Validation: TypeScript and all three service-icon regression tests passed. Stable workspace Clippy, formatting, and all workspace tests passed. Browser rendering confirmed transparent wrappers in light/dark mode, white monochrome marks in dark mode, unchanged colored logos, and successful transparent GitHub asset loading. Reviewed source-based filtering so discovered favicons are not recolored using a service name; fallback behavior and dimensions remain unchanged.
