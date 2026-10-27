# Round service icon images

- [x] Round the image element for every bundled and discovered service icon, proportional to its size.
- [x] Verify opaque discovered icons in light and dark mode and retain the existing color treatment.
- [x] Plan Review: confirm the clipping applies to the image rather than its padded wrapper.
- [x] Test Coverage Review: update existing shared style expectations and run frontend checks.
- [x] Bug Hunt: review sizing, transparency and fallback behavior.
- [x] Run required stable workspace Clippy, formatting and tests.
- [x] Commit and push the validated changes and any automatic catalog updates, preserving unrelated changes.

Validation: TypeScript checking, all three existing icon regression tests, stable workspace Clippy, formatting, and the full workspace tests passed. Browser preview verified 20% image border radius for a cached opaque discovered PNG in both themes, unchanged image colors, and transparent wrappers. Rounded corners apply directly to the image without changing sizing, spacing, theme filters, or fallback icons.
