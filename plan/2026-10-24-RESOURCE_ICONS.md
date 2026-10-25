# Resource icons

- [x] Inspect icon rendering and resource creation paths.
- [x] Add bounded favicon discovery from the user-configured endpoint, with local persistence and no render-time network requests.
- [x] Use discovered icons in resource lists, sidebar, and graph; retain bundled/category fallbacks.
- [x] Add a fixed client placeholder slot and remove dark-mode inversion of category icons.
- [x] Plan Review: check implementation against requested behavior and edge cases.
- [x] Test Coverage Review: cover discovery, failure, cache behavior, and icon rendering.
- [x] Bug Hunt: review limits, credential handling, stale requests, and theme behavior.
- [x] Run TypeScript and targeted tests. Full workspace validation was interrupted by disappearing generated build files; the user explicitly requested skipping further builds and pushing.
- [x] Commit and push task changes as explicitly requested, preserving unrelated changes.

Discovery is best effort on explicit resource creation/edit. It contacts only the configured origin, without API credentials, and caches image data locally. No third-party favicon service or background lookup. Stdio resources without a website keep bundled/category icons. The explicit favicon request authorizes this exception to the project's bundled-only asset rule; CSP stays local/data only.

Implementation review: HTML link declarations (including relative URLs after same-origin redirects and base64 inline raster images) are tried before /favicon.ico. Assets are limited to 256 KiB, decoded with dimension/allocation limits, and reduced to a 64px PNG. SVG-only or cross-origin images fall back to bundled/category icons. Cache keys distinguish provider instances and MCP server IDs, and follow rename/delete actions. Discovery runs after successful UI add/edit actions; existing resources can trigger it by editing their configuration.

Visual QA: Chrome preview verified identical label x positions for branded/custom clients and resource fallbacks in light/dark themes. MCP strokes inherit the theme; raster logos retain a light backing in dark mode.

Validation: app and website TypeScript checks passed; 7 frontend regression tests passed; 7 favicon discovery tests passed in both app test targets during the workspace run; formatting and diff checks passed. Initial workspace Clippy passed before the final redirect/inline additions. Final workspace Clippy/test checks were interrupted by removal of generated build artifacts, then explicitly stopped at the user's request to skip builds and push. No complete final workspace verdict is claimed.
