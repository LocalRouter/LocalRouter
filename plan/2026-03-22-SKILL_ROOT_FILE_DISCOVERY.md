# Fix: Skill root-level files invisible to SkillRead + smart path resolution

## Context

When an LLM calls `SkillRead(name="sysinfo")`, it gets instructions saying "run `bash ./sysinfo.sh`". But `sysinfo.sh` lives in the skill's root directory (next to `SKILL.md`), not in a `scripts/` subdirectory. The discovery code only indexes files from `scripts/`, `references/`, and `assets/` subdirectories — root-level files are invisible.

This affects ALL user-created skills:
```
sysinfo/SKILL.md + sysinfo.sh      ← invisible
diskusage/SKILL.md + diskusage.pl  ← invisible
weather/SKILL.md + weather.sh      ← invisible
```

Additionally, when an LLM guesses the wrong path (e.g., `scripts/run.sh` instead of `sysinfo.sh`), the error message is unhelpful — it says "has no readable files" with no suggestions.

## Changes

### 1. `crates/lr-skills/src/discovery.rs` — Root file discovery

Add `list_root_files()` after `list_subdir_files()`:
- Scans skill root for regular, non-hidden, non-SKILL.md files
- Returns bare filenames (e.g., `"sysinfo.sh"`)

Modify `load_skill_from_dir()`:
```rust
let mut scripts = list_subdir_files(skill_dir, "scripts");
scripts.extend(list_root_files(skill_dir));
scripts.sort();
```

Everything downstream (validation, response, indexing, catalog) works automatically.

### 2. `crates/lr-skills/src/mcp_tools.rs` — Smart path resolution + fuzzy file matching

Replace the simple `all_files.contains(&subpath)` check in `read_skill_file` with a multi-layer resolution strategy:

**Path resolution order** (when exact match fails):
1. Try bare filename in root dir (e.g., `scripts/abd.sh` → try `abd.sh`)
2. Try prefixed variants: `scripts/{subpath}`, `references/{subpath}`, `assets/{subpath}`
3. Fuzzy match against all known files using `lr_types::fuzzy::find_best_match`

If resolved via step 1-2, return the content with a correction note (like skill name fuzzy matching already does).

If resolved via fuzzy match, return the content with a correction note showing what was matched.

If no match found, return an error listing available files as suggestions:
```
"File 'abd.sh' not found in skill 'sysinfo'. Available files: sysinfo.sh"
```

Also simplify the empty-files error message to just: `"This skill has no readable files."`

### 3. Tests in `discovery.rs`

1. Root-level files discovered as scripts
2. Root files coexist with scripts/ files
3. SKILL.md and hidden files excluded
4. Subdirectories in root skipped

### 4. Tests in `mcp_tools.rs` (or `src-tauri/tests/skills_e2e_test.rs`)

1. Path resolution: `scripts/sysinfo.sh` resolves to root `sysinfo.sh`
2. Path resolution: bare `run.sh` resolves to `scripts/run.sh`
3. Fuzzy match: `sysinf.sh` resolves to `sysinfo.sh` with correction note
4. No match: returns error with available file list

## Critical files

- `crates/lr-skills/src/discovery.rs` — root file discovery
- `crates/lr-skills/src/mcp_tools.rs:358-442` — `read_skill_file` path resolution
- `crates/lr-types/src/fuzzy.rs` — reuse `find_best_match` for file path fuzzy matching
- `crates/lr-skills/src/fuzzy.rs` — re-exports (already imports `find_best_match`)

## Verification

1. `cargo test -p lr-skills` — all tests pass
2. `cargo clippy` — no warnings
3. Commit and push
