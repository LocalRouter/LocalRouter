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
   tree by walking it once, then trusts it (keyed by marker metadata) for the
   rest of its life. The archive hash is cached in memory by
   (path, size, mtime, inode). Pre-marker extractions live in the old temp
   location and are not carried over: re-extracting a ≤256 MiB skill archive
   once is cheaper and safer than proving an unmarked tree matches.
4. Atomic directory swap: `renamex_np(RENAME_SWAP)` (macOS),
   `renameat2(RENAME_EXCHANGE)` via `syscall` (Linux); the two-rename path
   remains the fallback (other OSes, filesystems without support).
5. Engines: one budget across the main archive and its extras — 32 GiB total,
   16 GiB per file, 100,000 files; enforced on written bytes; links (and the
   existing traversal checks) unchanged; an extra entry may not replace a file.
6. Model manifest `.localrouter-model.json` (revision, per-file size +
   SHA-256) written last; revision pinned to the commit of the first file
   (read from the hf-hub cache path, no extra request). `is_downloaded`
   requires the manifest and matching sizes; load verifies hashes once per
   process. Directories downloaded before manifests existed are adopted
   (manifest written with `revision: null`) instead of forcing a 700 MiB–2 GiB
   re-download.
7. Proxy sync state file `<data dir>/proxy-client-sync.json`: the remembered
   (client, template) pairs and, per tool config file, the client/template it
   was written for and the SHA-256 of the URL written (never the URL — it
   holds the secret). Ownership comes from that record (with the marker as a
   cross-check that the user has not replaced it); the substring logic stays
   as the fallback for files configured before the state file. The
   "Configure"/"Unconfigure" commands record too. Startup always reconciles,
   so a delete followed by exit inside the debounce is handled on next start.
8. Three-way merge at field level (`lr-config/src/merge.rs`) over YAML values;
   lists whose items all carry a unique string `id` merge by id. Base is the
   configuration last read from or written to disk.
   - `save()` finding an unloaded outside edit: merge; no conflict → write the
     merge, adopt it in memory, emit `config-changed`/`config-reloaded`;
     conflict → refuse the save (error returned and `config-reload-failed`
     emitted, existing toast).
   - Reload finding unsaved in-app changes: merge; conflicts keep the file's
     value and are reported with `config-reload-failed`.
9. Deprecate `ProxyConfig.enabled` with a doc comment (kept for
   deserialization, never read). Honouring it would need a migration to
   `true` plus a UI toggle and changes to `ProxyService` (out of scope); the
   per-client `llm_mode` already decides whether traffic goes through it.

## Checklist

- [ ] Save this plan
- [ ] 1 Extraction root in data dir, unconfigured-archive cleanup, temp migration
- [ ] 2 Root lock + marker leases
- [ ] 3 Cheap marker check, hash cache
- [ ] 4 Atomic swap in marketplace promote
- [ ] 5 Engine extraction budgets
- [ ] 6 Model manifests (embeddings, compression)
- [ ] 7 Proxy sync state file
- [ ] 8 Config three-way merge
- [ ] 9 `ProxyConfig.enabled` deprecation
- [ ] 10 Bug hunt (list below)
- [ ] Plan Review
- [ ] Test Coverage Review
- [ ] Bug Hunt
- [ ] Validation, commit per item, push

## Bugs found in the hunt

(filled in as found)
