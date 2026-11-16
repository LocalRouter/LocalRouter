# Secret scanning: startup gap, Codex images, Not-a-secret toggle

## Tasks

- [x] Startup gap: the server accepted requests before Tauri `setup` built the
      secret scanner, so Codex traffic right after a restart (e.g. an update)
      was forwarded unscanned with "Secret scanner unavailable". Build the
      scanner in `AppState::new` (before the server serves the state) through
      one `AppState::rebuild_secret_scanner`, shared by startup and the
      settings commands. Record build failures in `secret_scanner_error`.
- [x] The unscanned-request warning names the cause: "failed to initialize
      (<error>)" or "not initialized".
- [x] Codex image tool: `POST .../images/generations` and `.../images/edits`
      (JSON with `prompt`, `model`, and for edits `images`) are a recognized
      wire format, so the prompt is scanned and redacted. Multipart edits
      (OpenAI public API) keep the unsupported-endpoint warning.
- [x] Stored secrets: "Not a secret" toggles in place (no confirmation, no
      list reload); clicking again undoes it via new
      `unmark_redacted_secret_not_secret`, which removes the client's
      not-a-secret decisions for that value (always-redact decisions kept).
      Store-change events refresh quietly. Sorted most recently used first.
- [x] Clients page no longer swaps in a page-wide spinner on every
      `clients-changed` event (which remounted the client view).
- [x] Monitor secret-scan findings with a stored reference show the same
      toggle, with its state from new `redacted_secret_is_not_secret`.
- [x] Tests: scanner present right after `AppState::new`; warning reason;
      Codex image prompt redaction (generations and edits); images path and
      parsing; forget restores masking and keeps redact decisions.
- [x] Plan review, test coverage review, bug hunt.
- [x] Commit, merge to master, push.

## Notes

- The Monitor toggle needs the stored original, so it appears only on
  redacted findings whose reference is still in the vault (14-day retention).
  Findings that were sent unchanged have no stored original to act on.

## Validation

- Rust 1.100 stable: fmt and workspace clippy clean; `cargo test -p lr-server -p lr-proxy -p lr-secret-scanner -p lr-guardrails -p localrouter`: 2437 passed, 0 failed.
- `npx tsc --noEmit`, `npm run test:unit` (151 passed), `npm run build`, `npm --prefix website run build:check`: pass.
- Not exercised against a live Codex session; the toggles were not clicked through in the running app.
