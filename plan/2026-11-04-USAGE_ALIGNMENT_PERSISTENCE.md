# Usage alignment and section persistence

- [x] Inspect chart rows and durable section state wiring.
- [x] Align all window charts beside their meters, retaining all bars and spacing dense labels.
- [x] Allow durable writes after a startup state read fails; test both dashboard sections across fresh stores and browser contexts without localStorage.
- [x] Plan Review: verify alignment, accessibility and independent saved section states.
- [x] Test Coverage Review: cover dense charts and startup failure recovery.
- [x] Bug Hunt: inspect sizing, queued writes and restore behavior.
- [x] Run unit tests, browser regressions, desktop and website builds.
- [x] Commit, merge with --no-ff to master and push; monitor automatic website deployment.

## Validation

All 148 unit tests and 5 Chrome browser tests pass. Layout checks cover 300px, 360px and 720px cards and require all charts beside their meters with no bar or label overlap. Browser restart tests restore independent usage/traffic states from a shared IPC backend in fresh contexts with empty localStorage, including after an initial load failure. Desktop and website builds pass with existing chunk-size/mixed-import warnings. Screenshots inspected. No Rust changes or native app restart performed.

Plan review, coverage review and bug hunt complete. All 24 hourly bars and the full accessible summary are retained. The durable backend still writes each changed key to ui_state.json; a startup load failure no longer prevents later save attempts.
