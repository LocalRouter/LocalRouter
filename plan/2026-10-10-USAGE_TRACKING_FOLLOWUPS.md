# Usage tracking: follow-ups from the pre-release review

Follow-ups to subscription usage tracking (2026-10-10-SUBSCRIPTION_USAGE_TRACKING,
2026-10-10-USAGE_POLLING_DEFAULTS). Each item was verified against the code
before fixing.

## Progress

- [x] 1. Stale config overwrite from Settings → Usage
- [x] 2. Usage state not saved on quit
- [x] 3. Config changes not reaching the tracker
- [x] 4. Retired windows never pruned
- [x] 5. Overflow-safe arithmetic on response numbers
- [x] 6. Poller / maintenance shutdown
- [x] 7. ChatGPT Plus/Pro on an API key attributed to the subscription
- [x] 8. Repeated macOS Keychain reads for the Claude Code login
- [x] 9. Unused `tracing::info` import in release builds of lr-api-keys
- [x] Plan review, test coverage review, bug hunt
- [x] Commit (no push, per the task)

## Items

### 1. Stale config overwrite — real

`usage-tab.tsx` saved `{...config, ...change}` from the copy loaded with the
tab. `sync_tray_items` appends to `tray_items_added` in the background, so a
later toggle wrote the old list back and the next sync re-added a tray item the
user had removed.

- Backend: `update_usage_tracking_config` merges inside `ConfigManager::update`
  (under the config lock): entries of `tray_items_added` already recorded are
  kept (`merge_usage_config`). The frontend never removes entries, so this is
  lossless and closes the race even for stale callers (the Dashboard's
  `setConfig` re-fetches but still has a get → update window).
- Settings → Usage: changes are functions of the current settings; each save
  re-fetches `get_usage_tracking_config`, applies the change and saves; saves
  are serialized through a promise chain. List edits (excluded providers,
  hidden accounts, plan overrides) are computed from the fresh copy. The tab
  also reloads its settings on `config-changed`.
- Test: `ui::commands_usage::tests::stale_save_keeps_tray_items_added_meanwhile`.

### 2. No save on quit — real

The state file is written at most every 30 s by the maintenance tick; nothing
wrote it on exit. `UsageTracker::shutdown()` cancels the maintenance task and,
when dirty, prunes and persists. `RunEvent::Exit` in `main.rs` calls it
(after stopping the poller). `persist_now` holds a mutex so the tick and the
exit write cannot interleave on the temp file.

- Test: `tracker::tests::shutdown_writes_unsaved_changes_and_stops_maintenance`.

### 3. Config changes not reaching the tracker — real

The tracker got settings only from `update_usage_tracking_config`, tray sync
and provider rename. The config file watcher is disabled in `main.rs`; the
mechanism other subsystems use is the `config-changed` event emitted by
`ConfigManager::update` / `reload` (e.g. `reload_config`). Not every
`ConfigManager` clone has the app handle (clones taken before
`set_app_handle` do not emit), but all clones share the same config `Arc`.

- `UsagePoller::start` listens for `config-changed` and also re-reads the
  config on every scheduler tick (30 s) as a backstop for writers that do not
  emit. Both call `UsagePoller::apply_config`, which is a no-op when the
  settings equal the tracker's, otherwise calls `tracker.set_config` and
  repolls immediately when the allowed sources changed (`sources_changed`).
  `update_usage_tracking_config` uses the same path, so whichever applies the
  change first repolls.
- Tests: `ui::usage_poller::tests::apply_config_reaches_the_tracker_and_repolls_on_source_changes`,
  `only_source_settings_trigger_a_refresh`.

### 4. Retired windows never pruned — real

Windows were only removed with their account (35 days idle). Decision: a
window is retired when `2 × period` has passed since it ended (its reset, or
its last reading when it has none; period = its length, the well-known length
for its id, or a day) **and** the source that last fed it has reported for the
account since, without it. That is direct evidence the provider dropped it
(e.g. a removed per-model weekly cap). An idle account keeps its ended
windows (no newer report means no evidence) so its history survives a quiet
fortnight. Any window unrefreshed for the retention period (35 days) after it
ended goes regardless (covers a source that was turned off).

Pruning runs at least hourly (not only when there is new data to persist),
including on the first tick after start; a removal marks the state dirty and
notifies the UI.

- Tests: `retired_window_is_pruned_once_its_source_reports_without_it`,
  `idle_account_keeps_its_ended_windows`,
  `window_without_reset_retires_from_its_last_reading`,
  `tick_prunes_even_without_new_data`.

### 5. Overflow-safe arithmetic — real

- Parsers: `headers::after_secs(now, secs)` replaces `now + secs as i64` in
  the Codex header, Codex body and generic reset parsers: negative or
  non-finite delays give `None`, huge ones saturate.
- Tracker: readings are sanitized on ingestion — non-finite `used_percent`
  discards the reading; `window_secs` outside 1 s – 366 days and `resets_at`
  ≤ 0 or more than two years ahead become unknown (quotas' `resets_at` too).
  Rollover slack and sample trimming use saturating math.
- View (`view.rs`, overflow fixes only): window start via `checked_sub`,
  elapsed / slot math saturating, ETA `saturating_add`. Also a real panic:
  `BTreeMap::range(start..=now)` panics when `start > now`, which happens when a
  reset is further ahead than the window length (clock skew); the ledger is
  now skipped in that case. Old state files are not sanitized, so the view
  stays safe on raw stored values.
- Tests: `headers::tests::delays_saturate_instead_of_overflowing`,
  `tracker::tests::nonsense_numbers_are_dropped_not_overflowed`,
  `view::tests::extreme_stored_numbers_do_not_overflow`,
  `eta_saturates_for_a_tiny_rate`, `reset_beyond_the_window_length_does_not_panic`.

### 6. Poller / maintenance shutdown — real

Neither loop could be stopped. Both now use a `tokio_util` `CancellationToken`:
`UsagePoller::shutdown()` ends the poll loop (an in-flight poll is abandoned)
and `UsageTracker::shutdown()` ends the maintenance task. `main.rs` only stops
engines on `RunEvent::Exit`; the usage shutdown sits next to it.
`spawn_maintenance` returns its `JoinHandle` so the test can await it.

- Test: `tracker::tests::maintenance_task_ends_on_shutdown`.

### 7. Attribution of ChatGPT Plus/Pro on an API key — real

`openai-chatgpt-plus` uses `OpenAIProvider::from_oauth_or_key`, which falls
back to the stored API key; the gateway ledger still used
`account_for_provider_type` → `openai:subscription`, while the headers of
those requests (to `api.openai.com`) were classified as `openai:api`.

`ModelProvider::usage_account()` (default `None`) lets an instance state its
account; `OpenAIProvider` returns the subscription on the ChatGPT backend and
the API otherwise (same signal as `upstream_api`). `finalize.rs` prefers it
and falls back to the provider type.

- Test: `openai::tests::usage_account_follows_the_backend`.

### 8. Keychain prompts — real

`read_claude_code_login` ran `/usr/bin/security find-generic-password` on
every due poll. The Keychain result is now cached in memory only
(`KEYCHAIN_LOGIN`): a usable login until a minute before it expires (an hour
when it has no expiry); no login or an expired one is reused for 30 minutes
before asking again, so a denied prompt or a login Claude Code has not
refreshed is not re-asked on every poll. A 401 from the usage endpoint drops
the cache. The credentials file (no prompt) is still read each time. Nothing
is written to disk.

- Tests: `cli_logins::tests::keychain_read_is_reused_until_shortly_before_expiry`,
  `failed_or_expired_reads_are_retried_after_a_while`.

### 9. lr-api-keys release clippy — real

`use tracing::info;` under `cfg(not(debug_assertions))` had no user in the
file. Removed. `cargo clippy -p lr-api-keys --release -- -D warnings` passes.

## Final steps

- **Plan review**: every item above is implemented; no Tauri command
  signatures changed for the frontend (`update_usage_tracking_config` still
  takes `{ config }`; it no longer needs the tracker state), so
  `tauri-commands.ts` and the demo mock are unchanged. The Dashboard
  (`src/views/dashboard/**`) is owned by another change and was not touched;
  its `setConfig` is covered by the backend merge for `tray_items_added`.
- **Test coverage review**: added tests for each item except the `main.rs`
  exit wiring and the `config-changed` listener, which need a running Tauri
  app; the logic they call (`UsageTracker::shutdown`,
  `UsagePoller::apply_config`) is tested directly.
- **Bug hunt**: checked lock ordering (prune releases the state lock before
  persisting; persist takes its own mutex first, then a state read lock);
  Tauri's emit is re-entrant (`try_lock` + pending queue), so applying config
  from a `config-changed` handler that leads to further emits cannot
  deadlock; `clamp` in the view cannot panic (its upper bound is the window
  length, > 0); a reading discarded for a NaN still counts as its source
  reporting, which can only retire a window that is already two periods
  past its reset.

## Verification

- `cargo clippy --workspace --all-targets -- -D warnings` (rustup stable 1.99)
- `cargo clippy -p lr-api-keys --release -- -D warnings`
- `cargo fmt --all -- --check`
- `cargo test -p lr-usage -p lr-api-keys -p lr-config`,
  `cargo test -p localrouter --lib -- usage`,
  `cargo test -p lr-providers --lib -- usage_account upstream_api`
- `npx tsc --noEmit`
