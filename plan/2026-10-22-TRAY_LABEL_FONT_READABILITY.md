- [x] Apply the selected original bold font, 2-image-pixel gaps, and a 28px-high single glyph capped at 13px total width.
- [x] Match the reviewed preview: 1/2/3/4-letter heights of 28/16/10/7px, with each entire stack centered in 36px.
- [x] Add rendering regression coverage for spacing, centering, clipping and existing tray layouts.
- [x] Compare additional official bitmap fonts and wider bold variants at actual size and 3x, keeping background padding.
- [x] Plan Review: compare implementation with the chosen preview and resolve omissions.
- [x] Test Coverage Review: cover changed sizes, boundaries, and narrow single-letter rendering.
- [x] Bug Hunt: inspect width calculation, row pitch, centering, and graph integration.
- [x] Run rustup update stable, pinned stable workspace clippy, fmt check, and workspace tests.
- [x] Commit and push the validated changes and automatic catalog updates, preserving unrelated user files.

Plan review, coverage review, and bug hunt complete: the original glyph data and horizontal font rendering remain unchanged. Vertical layouts match the approved preview: (width, height) = (13,28), (13,16), (9,10), (6,7) for 1–4 letters, with 2px gaps and centered stacks. Pixel-level regression tests cover the top/bottom margins, every occupied row, and both clear gap rows. Existing tray graph integration tests are included in workspace validation.

Stable rustc 1.99.1 was updated/verified and pinned through PATH and RUSTC with RUSTC_WRAPPER disabled. Workspace Clippy with -D warnings and formatting checks passed; workspace tests passed (4585 tests, 69 ignored across 111 suites). The additional comparison covers IBM/VGA 8x8, Adafruit LCD, native Tamzen bold, and wider variants. Browser validation covered 48 actual/enlarged samples, all 36 spacing/sizing/example combinations, background padding and narrow layout; font source/license notices are embedded locally. Comparison-only fonts are not bundled into the application.

The seven focused tray-font tests also passed in an isolated harness against the actual Rust module. Final browser checks confirmed font selection with a normal pointer click. Completed work is committed and pushed to the configured upstream; unrelated MCP work and historical release plans are excluded.
