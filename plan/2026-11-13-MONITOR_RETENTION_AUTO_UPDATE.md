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
- [ ] Commit, merge to master (`--no-ff`), push, cut a release.

## Validation

- `cargo clippy --workspace --all-targets -D warnings`, `cargo fmt --check`: clean (rustup stable).
- `cargo test -p lr-monitor -p lr-config -p lr-server -p lr-proxy` and `cargo test -p localrouter --lib updater`: 662 passed, 0 failed.
- `npx tsc --noEmit`, `npm run test:unit` (151 passed), `npm run build`, `npm --prefix website run build:check`: pass.
- Not exercised: a real idle auto-install/restart against a published release (needs the release itself).
