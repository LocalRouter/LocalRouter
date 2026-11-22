# HTTPS-Proxy Client Config Sync + Config File Watcher

**Date**: 2026-11-21
**Branch**: `feat/proxy-client-sync-config-watcher`

Two independent pieces of work in the Tauri shell, committed separately.

---

## 1. HTTPS-proxy client setup

### What "Part E" was

`plan/2026-07-21-HTTPS_PROXY_MODE.md`, Part E — "Client setup (manual + Claude
automated)": extend the `AppIntegration` system so `ConfigSyncContext` carries
`proxy_url` + `ca_cert_path`, and have `integrations/claude_code.rs::sync_config`
write `HTTPS_PROXY` / `NODE_EXTRA_CA_CERTS` into `~/.claude/settings.json` when
the client is in a proxy `llm_mode`, removing them when it leaves.

### What the code does today

Part E was never built that way. `plan/2026-07-30-HTTPS_PROXY_AUTOCONFIG_ALL_CLIENTS.md`
replaced it with `src-tauri/src/launcher/proxy_setup.rs`: a per-template
`ProxyPlan` (ten templates, not just Claude Code) with `apply` / `unapply`,
driven by the explicit `configure_client_proxy` / `unconfigure_client_proxy`
commands. Writing the proxy into a tool's config is a deliberate user action
with caveats shown first (VS Code's setting is editor-wide, JSONC comments are
dropped, …) — it is intentionally *not* part of `sync_config`.

Left behind from Part E, with no reader anywhere:

- `ConfigSyncContext::proxy_url`, `::ca_cert_path`, `::should_sync_proxy()`
  (`launcher/mod.rs`), all `#[allow(dead_code)]`;
- the `(None, None)` TODO in `sync_client_config_inner` and the `None, None`
  arguments at the two other `ConfigSyncContext` construction sites;
- `ProxyService::stop()` (`launcher/proxy.rs`), `#[allow(dead_code)]`, never
  called.

### Decision

Do not resurrect Part E. Remove the dead `ConfigSyncContext` stubs. Build the
missing lifecycle on top of the design that shipped (`proxy_setup`):

**Listener lifecycle (`ProxyService`)**

- `host` becomes replaceable; `RunningProxy` remembers the requested port, the
  host and the accept-loop task.
- `shutdown().await` stops the listener and waits for the accept loop to end,
  so the port is free before rebinding. `stop()` (sync) stays for app exit.
- `restart(host, port)` = shutdown + start. `ensure(host, port)` restarts only
  when the configured endpoint differs from what is bound (or nothing is
  bound, e.g. an earlier bind failure).
- Lifecycle operations are serialized by an async mutex.

**Client-side config sync (`launcher/proxy_sync.rs`, new)**

State-based and stateless on disk: a LocalRouter proxy URL is
`http://<client_id>:<secret>@host:port`, so a tool's config file says which
client it was configured for. No new config field, no flag to keep in step.

For each config file a known `(client, template)` pair maps to:

1. A client that is still in proxy mode and whose URL marker is in the file
   → if the proxy is running and the file does not hold the current URL
   (port, host or secret changed), re-apply the plan. Otherwise leave it.
2. Otherwise, a marker of a client that is no longer in proxy mode for that
   tool (mode changed, template changed, or client deleted) → `unapply`.
3. Otherwise, the file names LocalRouter's CA, holds no marker of any
   existing client, and no proxy-mode client uses that tool (Goose writes
   only the CA path; also covers leftovers) → `unapply`.

Never applied automatically to a file the user did not already configure for
that client — "Configure" stays the only way a proxy lands in a tool's config.
A stopped listener never triggers removal (a transient bind failure must not
erase the user's setup). App exit does not undo client configs either: like
permanent gateway config, they are meant to survive a restart.

Deleted clients / changed templates are covered by remembering every
`(client_id, template_id)` pair seen during the session.

Triggers (debounced 500 ms, one background task): startup, `clients-changed`
(mode change, delete, template change, external config reload),
`config-changed` (listener endpoint check only), `server-restart-requested`
(forced listener restart), secret rotation.

External-app config writes (`sync_config`, proxy apply/unapply, proxy sync)
share one lock so two read-modify-write cycles on the same file cannot
interleave.

Only the public CA certificate path is ever handed to client configs. Nothing
here reads or writes the CA private key.

**Implementation notes**

- `proxy_setup::apply_in` / `unapply_in` take the backup directory, so the
  reconcile tests (which run the real apply/undo against temp files) never
  write into, or prune, the user's real `localrouter-backups` directory.
- `Reconciler` remembers the URL it last wrote per file: a config whose format
  hides the URL from a substring check is rewritten once, not on every pass.
- A disabled client keeps its tool config (disabling is usually temporary);
  only leaving proxy mode, a template change or deletion removes it.
- `ProxyConfig.enabled` stays unread, as before: the listener is always
  started (`main.rs`), and the field defaults to `false`, so honoring it now
  would stop the proxy for every existing install. "Proxy on/off" for a client
  is its `llm_mode`.
- `EXTERNAL_CONFIG_LOCK` covers `configure_app_permanent` too (both its
  `sync_config` and `configure_permanent` branches write the same files the
  proxy plans write, e.g. `~/.claude/settings.json`).
- No Tauri command was added or changed in shape (`rotate_client_secret` only
  gains an injected `AppHandle`), so `tauri-commands.ts` and the demo mock are
  untouched.

### Checklist

- [x] Read the plans and verify the survey's claims against the code
- [x] Remove dead `ConfigSyncContext` proxy stubs and their `dead_code` allows
- [x] `ProxyService`: `shutdown` / `restart` / `ensure`, drop the `dead_code` allow on `stop`
- [x] `launcher/proxy_sync.rs`: pure decision logic + file reconcile + background task
- [x] Wire: startup, `clients-changed`, `config-changed`, `server-restart-requested`, secret rotation, `RunEvent::Exit` → `stop()`
- [x] Shared lock around external-app config writes
- [x] Unit tests (decision logic) + temp-dir tests (apply → port change → refresh; leave proxy mode → removed; other client's config untouched) + listener lifecycle test

---

## 2. Config file watcher

### Problem

`main.rs` disabled `ConfigManager::start_watching` because every in-app save
produced a second `config-changed`. The old watcher also watched the file
itself (missing rename-based editor saves), reloaded once per raw event, and
used `load_config`, which on a parse error *restores a backup over the file*
— destroying the edit the user is in the middle of.

### Design (`crates/lr-config/src/watcher.rs`, new)

- **Watch the directory**, non-recursively; keep only events whose path is
  exactly the config file (by name, parent compared raw and canonicalized).
  Access events are dropped.
- **Debounce / coalesce**: events only wake a task. It waits for a quiet
  period (250 ms), bounded by a maximum wait (2 s), then looks at the file
  once. `next_deadline(first, last)` is pure.
- **Self-write suppression by content hash** (`ReloadGate`): the save path
  records the SHA-256 of the exact bytes it wrote; applying a reload records
  the hash of the bytes it read. The watcher hashes the file and ignores
  content the gate already knows. No timing guesses. A rejected (invalid)
  content hash is remembered too, so one bad edit is reported once.
- **Consistency**: the watcher inspects the file while holding the save
  mutex, so it can never see the file from before a save whose hash is
  already recorded (or the reverse).
- **Validate before applying**: strict load (`storage::load_config_strict`:
  size check, parse, migrate, self-heal, validate) with **no backup recovery
  and no writes**. On failure the running config is kept, the error is logged
  and `config-reload-failed` (message) is emitted; the UI shows a persistent
  error toast.
- **One reload path**: `ConfigManager::reload()` (the `reload_config` command)
  and the watcher both go through `reload_from_disk` → `apply_reloaded`, which
  swaps the config, syncs the client manager, emits `config-changed`, and —
  new — emits `config-reloaded` so the shell can fan out the granular events
  (`clients-changed`, `strategies-changed`, …) and rebuild the tray.
- A reload whose parsed config equals the running one (comment/whitespace
  edit) only records the hash; nothing is emitted.

**Save followed immediately by an external edit**: both events coalesce; the
file then holds the external content, whose hash is not the recorded own
write → it is applied. **No reload loop**: applying never writes the file; if
a subscriber saves in response, that write's hash is recorded and ignored.

Not hot-reloaded (same as the manual `reload_config` before this change, needs
an app restart): provider registry instances, MCP server manager entries,
OAuth clients, server host/port (the latter needs "Restart server").

**Implementation notes**

- `reload()` no longer goes through `load_config`: a manual reload of a broken
  file returns the error instead of restoring a backup over it, and a missing
  file is an error instead of being recreated with defaults.
- An empty (or whitespace-only) file is treated as a save in progress and
  never parsed — it must not turn into "every setting at its default".
- Known limit: an outside edit that lands between an in-app `update()` and
  its `save()` replaces that in-memory change (the edit wins; the pending
  save then writes the edit back). In-memory state that was never saved is
  likewise replaced by an applied reload.
- The reverse order — an outside edit lands, and an app save follows before
  the debounce has run — cannot be merged generically: the save wins. It is
  not silent, though: `save()` notices (under the save mutex) that the file
  holds content the gate has never seen, logs a warning, and the edited file
  survives as the `settings.yaml.backup.*` the save makes first. Tested.
- `start_watching` is synchronous (Tauri's `setup` is), and the gate is seeded
  when the manager loads the file.

### Checklist

- [x] `storage`: strict loader returning the content hash; `save_config` reports the hash it wrote
- [x] `ReloadGate` + debounce logic + directory watcher
- [x] `ConfigManager`: gate shared across clones, `save` records, unified reload path, events
- [x] Re-enable in `main.rs`; fan out granular events on `config-reloaded`
- [x] Frontend: toast for `config-reload-failed` / `config-reloaded`
- [x] Unit tests (gate, deadline, path filter, strict loader) + temp-dir integration tests with the real watcher

---

## Final steps

- [x] **Plan Review** — implementation checked against this plan
- [x] **Test Coverage Review** — new paths covered; gaps listed in the hand-off
- [x] **Bug Hunt** — re-read for races, loops, lost updates
- [x] Validation: `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test -p lr-config`, `cargo test -p localrouter --lib`, frontend `tsc` / unit tests / build, website `build:check`
  (lr-config 239 passed; localrouter lib + bin 460 passed each, 12 ignored;
  frontend 151 passed). The shared target dir served another worktree's
  `lr-config` artifact (path packages hash relative to the workspace root),
  so the Rust checks ran with
  `--config 'profile.dev.package.lr-config.codegen-units=251'` to give this
  worktree's `lr-config` and its dependents their own artifacts.
- [x] Bug hunt fixes: `configure_app_permanent` now takes
  `EXTERNAL_CONFIG_LOCK`; `save()` warns when it replaces an outside edit not
  yet loaded; `ProxyService::port` / `shutdown` (only tests call them) are
  `#[cfg(test)]` rather than dead code.
- [x] Commit (two commits) and push the branch
