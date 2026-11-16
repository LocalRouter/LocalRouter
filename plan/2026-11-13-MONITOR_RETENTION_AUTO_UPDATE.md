# Monitor retention and idle auto-update

## Tasks

- [x] Monitor: when the byte budget is exceeded, trim payloads of the oldest
      finished events (keeping their list entry, summary and metadata) before
      evicting any event. Events are only dropped at the event-count limit, or
      when trimmed metadata alone exceeds the budget.
- [x] Monitor: expose `payload_trimmed` on events/summaries; event detail
      explains that the body was trimmed to save memory. UI list shows up to
      500 as before (rows are not virtualized).
- [x] Monitor: track the time of the last client request (LLM call / MCP
      request) for idle detection.
- [x] Config: `update.auto_install` (default on for new installs). Config
      migration v34 sets it off for existing configs.
- [x] Updater: background loop that, when auto-install and automatic checks
      are enabled and the install is self-updatable, waits for 5 hours with no
      client requests, no requests in progress and the window unfocused, then
      checks, downloads, installs and restarts. Respects skipped versions.
- [x] Tauri command `update_update_config` accepts `auto_install`; TS types,
      demo mock and Updates settings toggle.
- [x] Tests: store trimming/eviction order, idle tracking, migration,
      auto-install decision logic.
- [x] Plan review, test coverage review, bug hunt.
- [x] Commit, merge to master (`--no-ff`), push.
- [x] Cut release v0.0.186.

## Validation

- `cargo clippy --workspace --all-targets -D warnings`, `cargo fmt --check`: clean (rustup stable).
- `cargo test -p lr-monitor -p lr-config -p lr-server -p lr-proxy` and `cargo test -p localrouter --lib updater`: 662 passed, 0 failed.
- `npx tsc --noEmit`, `npm run test:unit` (151 passed), `npm run build`, `npm --prefix website run build:check`: pass.
- Not exercised: a real idle auto-install/restart against a published release (needs the release itself).

## Release v0.0.186 status

- Exact-source CI on master `200f1a32` passes (run 43729897995). An earlier CI run failed on a new
  Rust 1.100 `needless_bool` lint in pre-existing `lr-secret-scanner` code, fixed in `89ed415e`.
- Release run 43733036429: version bump `cfc8712d` and tag `v0.0.186` pushed; Windows and both Linux
  builds pass; both macOS builds fail at notarization with Apple HTTP 403 "A required agreement is
  missing or has expired". No GitHub release was created, so updaters still see v0.0.185.
- Resume: after the Apple Developer agreement is accepted, `gh run rerun 43733036429 --failed`
  (re-dispatching 0.0.186 would fail because the tag already exists).
- Agreement accepted; `gh run rerun 43733036429 --failed` (attempt 2) passes: all five builds, GitHub
  release, Nix pin, Snap and Flatpak. Publish to Package Managers is skipped as before. v0.0.186 is the
  latest release with 32 non-empty assets; Docker run 43780126229 succeeds; `latest.json` serves 0.0.186.
