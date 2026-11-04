- [x] Inspect the secret-scan popup and existing reveal/privacy boundaries.
- [x] Decode JWT header and payload locally after Reveal; display formatted JSON and signature-verification status; handle malformed tokens without breaking the popup.
- [x] Add regressions for UTF-8/base64url decoding, malformed input, reveal/hide behavior and popup actions.
- [x] Plan Review: compare implementation with planned behavior.
- [x] Test Coverage Review: check new paths and error handling.
- [x] Bug Hunt: re-read parsing, privacy and layout behavior.
- [x] Run unit tests, desktop build, website build:check and focused browser tests.
- [x] Commit and push validated task changes, preserving unrelated work.

Implemented a local base64url/UTF-8 JSON decoder and a bounded, scrollable header/payload view in the shared approval card. JWT contents appear only after the existing Reveal action and disappear with Hide. No plaintext is added to IPC summaries or persisted. The view explicitly states that the signature is not verified; malformed claims show a decode failure while retaining the original token and decision controls.

Plan, coverage and bug reviews completed: object-only claims, invalid encoding/UTF-8/JSON, non-JWT input, unsigned display, Unicode, reveal/hide privacy, Ask/Notify layouts and malformed-token decisions are covered. Existing request rendering and approval behavior remain covered by the secret-redaction browser suite.

Validation: 142 unit tests passed; desktop production build and website build:check passed (existing bundle-size/import warnings). Final browser suite: 27 tests passed using installed Google Chrome. Initial browser attempts encountered missing bundled Chromium and an overlapping preview-server teardown; the final suite runs alone. No Rust changes; Rust checks were not run. Unrelated release-plan files are preserved.
