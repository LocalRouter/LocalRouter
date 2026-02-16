# Skills System Refactoring Plan

## Summary

Refactor the skills system to add: file watching, hash-based extraction, atomic switchover with cleanup, unified path config, and proper three-tier access control.

---

## 1. Config Changes

### `SkillsConfig` (`src-tauri/src/config/mod.rs`)

```rust
// BEFORE
pub struct SkillsConfig {
    pub auto_scan_directories: Vec<String>,
    pub skill_paths: Vec<String>,
}

// AFTER
pub struct SkillsConfig {
    pub paths: Vec<String>,              // unified list of files or dirs
    pub disabled_skills: Vec<String>,    // globally disabled skill names

    // Migration shims (deserialize only, skip on serialize)
    #[serde(default, skip_serializing)]
    pub auto_scan_directories: Vec<String>,
    #[serde(default, skip_serializing)]
    pub skill_paths: Vec<String>,
}
```

### `SkillsAccess` (per-client)

Change `Specific(Vec<String>)` from skill **names** to source **paths**. A client with `Specific(["/path/to/dir"])` gets all skills discovered from that path (subject to global enable).

### Migration (`config/migration.rs`)

- Bump `CONFIG_VERSION` to 4
- `migrate_to_v4`: merge `auto_scan_directories` + `skill_paths` into `paths`, deduplicate
- Convert any `Specific(names)` to `All` since we can't reliably map names to paths without discovery

---

## 2. Type Changes (`skills/types.rs`)

Add to `SkillDefinition`:
- `enabled: bool` (default `true`) — set by manager based on `disabled_skills`
- `content_hash: Option<String>` — SHA-256 of source zip/skill file

Add to `SkillInfo`:
- `enabled: bool`

---

## 3. Hash-Based Extraction (`skills/discovery.rs`)

**Current**: extracts to `/tmp/localrouter-skills/{file_stem}/`, always deletes old first.

**New**: extracts to `/tmp/localrouter-skills/{file_stem}-{hash}/` where hash is SHA-256 of file content (first 16 bytes hex = 32 chars).

- If hash dir already exists, skip extraction (reuse)
- Never delete old directories during discovery — return them for manager cleanup
- New return type `DiscoveryResult { skills, old_extraction_dirs }`

Uses `sha2` crate (already in Cargo.toml). For hex encoding, use `format!("{:02x}", byte)` to avoid adding `hex` crate.

---

## 4. Manager Refactoring (`skills/manager.rs`)

### Storage model change

```rust
// BEFORE
skills: Arc<RwLock<Vec<SkillDefinition>>>

// AFTER
skills: Arc<RwLock<Arc<Vec<SkillDefinition>>>>
```

Readers clone the inner `Arc` (cheap pointer copy) to get a snapshot. Writer builds a new `Vec`, wraps in `Arc`, acquires write lock briefly for pointer swap. No new dependencies needed — uses existing `parking_lot::RwLock`.

### Atomic switchover flow

1. Watcher detects zip change
2. `discover_from_zip` computes new hash, extracts to new dir, returns skills + old dir paths
3. Manager builds new complete skills list from all paths
4. `*self.skills.write() = Arc::new(new_list)` — atomic pointer swap
5. Old extraction dirs queued for delayed cleanup

### Cleanup strategy

Old extraction dirs are tracked in `pending_cleanup: Arc<DashMap<PathBuf, Instant>>`. A background tokio task runs every 30s and deletes dirs older than 30s. This grace period exceeds any realistic in-flight request duration (script timeout max is 20s sync, but Arc snapshot keeps references alive regardless).

The inner `Arc<Vec<SkillDefinition>>` ensures that even if cleanup deletes a directory, any in-flight handler that already loaded the old snapshot still has valid `PathBuf` references — the cleanup task only removes dirs that have been superseded for 30+ seconds.

### New methods

- `rescan_paths(paths: &[String])` — selective re-discovery + atomic swap (used by watcher)
- `set_skill_enabled(name, enabled)` — toggles `disabled_skills` in config, re-applies to snapshot
- `start_watcher()` — spawns the file watcher background task

---

## 5. File Watcher (`skills/watcher.rs` — new file)

Uses `notify::recommended_watcher` (native FSEvents/inotify/ReadDirectoryChanges). No polling fallback.

### Architecture

- `notify` callback sends `FileEvent` over `tokio::sync::mpsc::unbounded_channel`
- Background tokio task receives events, debounces 500ms, then calls `manager.rescan_paths(affected)`
- Dynamic path management via `WatcherCommand::AddPath` / `RemovePath` (sent when user adds/removes skill sources)

### Watching rules

- **Directories**: watch recursively (`RecursiveMode::Recursive`) — picks up new/removed skill subdirs
- **Files** (zip/skill): watch the parent directory non-recursively, filter events for the specific file — `notify` can't watch individual files on all platforms

### Event handling

```
notify callback → channel → tokio task (debounce 500ms) → manager.rescan_paths()
```

Debounce prevents thundering herd from rapid file writes (e.g., IDE saving, rsync).

---

## 6. Access Control Chain

A skill tool is served to a client only if **all three** conditions are met:

1. **Discovered** — skill exists in manager's list (its source path is in `SkillsConfig.paths`)
2. **Globally enabled** — `skill.enabled == true` (name not in `disabled_skills`)
3. **Client-allowed** — client's `SkillsAccess` matches:
   - `None` → no skills
   - `All` → all enabled skills
   - `Specific(paths)` → only skills whose `source_path` is in the list

### Changes in `mcp_tools.rs`

`build_skill_tools` takes `SkillsAccess` instead of `Vec<String>`:
```rust
fn build_skill_tools(manager: &SkillManager, access: &SkillsAccess) -> Vec<McpTool>
```
Filters: `skill.enabled && access.matches(skill.source_path)`.

### Changes in `gateway/session.rs`

`allowed_skills: Vec<String>` → `skills_access: SkillsAccess`

### Automatic access revocation

When a path is removed from global `SkillsConfig.paths`, skills from that path disappear from the manager's list. Clients with that path in `Specific(paths)` simply get no matches — no explicit client config cleanup needed. The path entry in the client config becomes a no-op.

---

## 7. Tauri Commands (`ui/commands.rs`)

### Remove
- `add_skill_scan_directory`
- `add_skill_path`
- `remove_skill_scan_directory`
- `remove_skill_path`

### Add
- `add_skill_source(path: String)` — adds to `SkillsConfig.paths`, tells watcher, rescans
- `remove_skill_source(path: String)` — removes from `SkillsConfig.paths`, tells watcher, rescans
- `set_skill_enabled(skill_name: String, enabled: bool)` — toggles in `disabled_skills`

### Modify
- `get_skills_config` — return new shape
- `set_client_skills_access` — `Specific` variant now contains paths

---

## 8. Frontend Changes

### `src/views/skills/index.tsx`
- Single "Add Skill Source" button (replaces separate scan dir / skill path buttons)
- Single list of configured paths (no scan/path distinction)
- Per-skill enable/disable toggle (calls `set_skill_enabled`)
- Disabled skills shown as dimmed

### `src/views/clients/tabs/skills-tab.tsx`
- `Specific` mode stores source paths, not skill names
- Globally disabled skills shown grayed out and non-selectable
- When user toggles a skill, add/remove its `source_path` from the client's list

---

## 9. Implementation Order

| Phase | Steps | Files |
|-------|-------|-------|
| **1. Types** | Add `enabled`, `content_hash` to `SkillDefinition`/`SkillInfo` | `types.rs` |
| **2. Hash extraction** | `content_hash_of_file`, `DiscoveryResult`, hash-based dirs | `discovery.rs` |
| **3. Manager core** | `Arc<RwLock<Arc<Vec<...>>>>`, `rescan_paths`, cleanup task | `manager.rs` |
| **4. Watcher** | New module, debounced event processing, dynamic path mgmt | `watcher.rs`, `mod.rs` |
| **5. Config** | Unified `paths`, `disabled_skills`, migration to v4 | `config/mod.rs`, `migration.rs` |
| **6. Access control** | Three-tier filtering, `SkillsAccess` path-based semantics | `mcp_tools.rs`, `gateway.rs`, `session.rs` |
| **7. Commands** | Unified add/remove, `set_skill_enabled` | `commands.rs`, `main.rs` |
| **8. Frontend** | Unified UI, enable/disable toggles, path-based client access | `index.tsx`, `skills-tab.tsx` |
| **9. Tests** | Update existing, add watcher/hash/switchover/access tests | `*_test.rs` |

---

## 10. Verification

1. `cargo test` — all existing + new tests pass
2. `cargo clippy` — no warnings
3. Manual test: add a `.skill` zip, verify extraction to hash-based dir
4. Manual test: modify the zip, verify watcher triggers re-extraction to new hash dir, old dir cleaned up after grace period
5. Manual test: add a directory with skills, add a new skill subdir, verify auto-discovery
6. Manual test: disable a skill globally, verify client can't access it
7. Manual test: remove a path from global config, verify client loses access to those skills
8. E2E test: `skills_e2e_test.rs` updated to cover hash-based extraction and enable/disable
