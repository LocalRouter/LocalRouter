# Usage account grid and representative screenshots

- [x] Identify test-only card span override causing misleading singleton rows.
- [x] Size the shared grid for narrow-chart tests; remove layout overrides before screenshots.
- [x] Use auto-fit so accounts fill available columns, including fewer accounts than available tracks.
- [x] Add real viewport/account-count grid regressions for contiguous equal-width rows.
- [x] Plan review, test coverage review and bug hunt.
- [x] Run unit tests, browser checks, desktop build and website build.
- [ ] Commit, merge with --no-ff and push master; monitor website deployment.

## Results

Plan review, coverage review and bug hunt complete. The misleading screenshots came from setting a single card to grid-column: 1 / -1. Narrow chart checks now resize the entire grid and restore its natural width before screenshots. Real viewport checks at 720px, 1100px and 1440px validate two and six accounts: equal widths, contiguous rows, full first-row width and no horizontal overflow. Grid auto-fit also removes unused tracks when only two accounts exist.

148 unit tests and 11 Chrome browser tests pass. Desktop build and website build:check pass with existing bundle/import warnings. Natural six-account screenshot visually verified as a 3-by-2 grid. No native app restart or release performed.
