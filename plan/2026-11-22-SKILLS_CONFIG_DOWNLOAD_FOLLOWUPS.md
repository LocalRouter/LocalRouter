# Skills / config / download follow-ups

Follow-ups left by `plan/2026-11-21-SKILL_INSTALL_HARDENING.md` and
`plan/2026-11-21-PROXY_CLIENT_SYNC_CONFIG_WATCHER.md`.
Branch: `fix/skills-config-download-followups`.

## Findings (verified against the code at 63291aaf)

1. **Archive extractions in `<temp>/localrouter-skills`** — confirmed
   (`lr-skills/src/discovery.rs::extraction_root`). The same temp folder also
   holds async script output (`executor.rs`, `<temp>/localrouter-skills/<pid>/`),
   so a cleanup of the old location must only touch `<stem>-<32 hex>` and
   `.staging-*` entries.
2. **Cross-process race** — confirmed. Serialization is a process-local
   `Mutex` per target; dev and prod share `<temp>`; a superseded or "incomplete"
   extraction is removed with no regard for another process serving it.
3. **Re-extraction / tree walk** — partly confirmed. A marker survives
   upgrades, so re-extraction happens once (for extractions made before
   markers existed), not on every upgrade. Every scan does hash the whole
   archive and walks the whole extraction tree.
4. **Gap during a marketplace update** — confirmed (`lr-marketplace/src/staging.rs::promote`:
   `rename(target, previous)` then `rename(new, target)`).
5. **`lr-engines` extraction without byte budget** — confirmed
   (`download.rs::extract_zip` / `extract_tar_zst`: `std::io::copy` unbounded;
   traversal and link checks already exist).
6. **No model manifest** — confirmed: `is_downloaded` checks presence only;
   files are fetched from the moving `main` revision one at a time.
7. **Proxy sync** — confirmed: `Reconciler::seen` and `last_written` are in
   memory only; ownership is the substring `://<client_id>:`.
8. **Config reload races** — confirmed (documented as known limits).
9. **`ProxyConfig.enabled` unread** — confirmed: not read anywhere; the
   listener always starts; every existing settings.yaml has `enabled: false`
   written out explicitly.

## Decisions

1. Extraction root moves to `<data dir>/skill-archives` (`lr_utils::paths::config_dir`).
   After each scan, extractions whose marker names an archive that is no
   longer a configured source are queued for the delayed cleanup. At startup
   the old `<temp>/localrouter-skills` extractions and staging folders are
   removed once (a `.migrated-from-temp` marker in the new root records it);
   numeric per-PID script output folders are left alone.
2. Two file locks (std `File::lock`, flock on Unix, LockFileEx on Windows):
   - `<root>/.lock`, exclusive, around check+extract+swap and around every
     removal (cleanup, re-extraction of a damaged tree, stale staging).
   - A shared *lease* on each extraction's marker file, held by every process
     that serves skills from it. Removal takes an exclusive try-lock on the
     marker first and skips a leased tree (retried on the next cleanup tick).
3. Marker gets a third line `archive <size> <mtime-ns>`. A process verifies a
   tree by walking it once, then trusts it (keyed by the marker's size and
   mtime) for the rest of its life. The archive hash is cached in memory by
   (path, size, mtime), and an extraction whose marker records the archive's
   current size and mtime is found without hashing it (its folder name carries
   the hash). Pre-marker extractions live in the old temp location and are not
   carried over: re-extracting a ≤256 MiB skill archive once is cheaper and
   safer than proving an unmarked tree matches. (Note: the claim "re-extraction
   on every upgrade" was wrong — a marker survives upgrades; it happened once.)
4. Atomic directory swap via `lr_utils::fs::exchange_paths`:
   `renamex_np(RENAME_SWAP)` (macOS), `renameat2(RENAME_EXCHANGE)` via
   `syscall` (Linux); the two-rename path with rollback and startup recovery
   remains the fallback (Windows, filesystems without support). Readers do not
   retry, so on Windows the short gap remains.
5. Engines: one budget across the main archive and its extras — 32 GiB total,
   16 GiB per file, 100,000 entries; enforced on written bytes (declared sizes
   reject early); copies check cancellation per chunk; zip link targets read
   with a 4 KiB cap. The existing traversal/link checks are unchanged.
6. Model manifest `.localrouter-model.json` (`lr_utils::model_manifest`:
   repo, revision, per-file size + SHA-256). All files are fetched into the
   hf-hub cache first; then `.localrouter-model.pending` is written, the files
   copied, the manifest written and the pending marker removed. The revision
   is pinned to the commit of the first file (read from the hf-hub cache
   path, no extra request). `is_downloaded` requires no pending marker and a
   manifest with matching sizes; loading hashes the files once per process.
   Compression additionally requires both directories to carry the same
   revision. Directories downloaded before manifests existed (all files, no
   manifest, no pending marker) are adopted on first load (manifest with
   `revision: null`) instead of forcing a 700 MiB–2 GiB re-download; a
   partial update of such a legacy directory made by an older build cannot be
   detected.
7. Proxy sync state file `<data dir>/proxy-client-sync.json`: the remembered
   (client, template) pairs and, per tool config file, the client/template it
   was written for and the SHA-256 of the URL written (never the URL — it
   holds the secret). Ownership comes from that record (with the client's URL
   marker as a cross-check that the user has not replaced it); a recorded URL
   is compared by hash. The substring logic stays as the fallback for files
   configured before the state file. Configure/Unconfigure record too. Pairs of
   deleted clients are dropped once their file has been dealt with. Startup
   always reconciles, so a delete followed by exit inside the debounce is
   handled on the next start (no flush on exit needed).
8. Three-way merge at field level (`lr-config/src/merge.rs`) over YAML values;
   lists whose items all carry a unique string `id` merge by id; the merged
   configuration must deserialize and validate. Base (`ConfigManager::synced`)
   is the configuration last read from or written to disk.
   - `save()` finding an outside edit not loaded yet: merge; no conflict →
     write the merge, adopt it in memory, sync clients, emit `config-changed`
     and `config-reloaded`; conflict → refuse the save (error returned,
     `config-merge-conflict` emitted) and leave the file alone; the watcher
     then applies the edit. An unparseable edit is still overwritten (kept in
     a backup) as before.
   - Reload finding unsaved in-app changes: merge, keep the file's value for
     conflicts, write the merge back at once (hash recorded) and emit
     `config-merge-conflict` listing the conflicts. A forced reload discards
     in-app changes as before.
   - New `config-merge-conflict` event, shown by `App.tsx` as a persistent
     warning toast.
9. Deprecate `ProxyConfig.enabled` with a doc comment (kept for
   deserialization, never read). Honouring it would need a migration to
   `true` plus a UI toggle and changes to `ProxyService` (out of scope); the
   per-client `llm_mode` already decides whether traffic goes through it.

## Checklist

- [x] Save this plan
- [x] 1 Extraction root in data dir, unconfigured-archive cleanup, temp migration
- [x] 2 Root lock + marker leases (cross-process test with a child process)
- [x] 3 Cheap marker check, hash cache
- [x] 4 Atomic swap in marketplace promote
- [x] 5 Engine extraction budgets
- [x] 6 Model manifests (embeddings, compression)
- [x] 7 Proxy sync state file
- [x] 8 Config three-way merge (both orders tested)
- [x] 9 `ProxyConfig.enabled` deprecation
- [x] 10 Bug hunt (list below)
- [x] Plan Review — dropped "an extra entry may not replace a file" for
      engines (extras deliberately land beside the binary; not changed)
- [x] Test Coverage Review — gaps: the network paths of the model
      downloaders (pinning, fetch-before-mark) and the Configure/Unconfigure
      commands' recording are not unit-tested (they need hf-hub / a Tauri
      app); `start_cleanup_task` is covered through `remove_due` and
      `migrate_legacy_root`
- [x] Bug Hunt
- [x] Validation, commit per item, push

## Bugs found in the hunt

1. Config backups were named per second; a second save within that second
   overwrote the first save's backup — possibly the only copy of an outside
   edit the first save replaced. Fixed with a numbered suffix (`storage.rs`).
2. Marketplace startup recovery deleted every staging directory, including a
   live install of a second instance on the same data directory, and could
   move its set-aside version back mid-swap. Each staging directory is now
   locked for the life of its install and recovery skips locked ones.
3. Skill archive entries and marketplace file paths named `CON`, `nul.txt`,
   `COM1`, … or ending in a dot or space open a device or alias another file
   on Windows. Refused on every platform (`lr_utils::fs::non_portable_component`).
4. (Own code, found in review) A failed model re-download marked a complete
   model as incomplete before any file was replaced; files are now fetched
   into the cache before the directory is marked.

Checked without finding bugs: the config watcher's path filter and debounce,
Windows branches in `lr-engines` (`install.rs`, `supervisor.rs`,
`process.rs`), marketplace path confinement.

Left as is: `lr-engines` `write_pid_records` writes non-atomically (a torn
file reads as "no records"); on Windows the marketplace update still has the
two-rename gap.

## Validation

- `cargo fmt --all -- --check` — clean
- `cargo clippy --workspace --all-targets -- -D warnings` — clean
- `cargo test -p lr-utils -p lr-skills -p lr-marketplace -p lr-config -p lr-engines -p lr-embeddings -p lr-compression` — all pass
- `cargo test -p localrouter --lib --bins` — 475 passed, 12 ignored (each)
- `npx tsc --noEmit`, `npm run test:unit` (161 passed), `npm run build`,
  `npm --prefix website run build:check`, `node scripts/check-demo-mock-coverage.mjs` — pass