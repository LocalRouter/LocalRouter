# Project Cleanup and Gap Closure — 2026-07-02

Follow-up from a whole-project review. A2A integration was explicitly excluded by decision.

## Scope (in execution priority order)

1. **Delete dead `src-tauri/src` legacy tree** — `src-tauri/src/lib.rs` only declares
   `cli`, `launcher`, `ui`, `updater` and re-exports the `lr-*` crates; the sibling
   directories (`router/`, `providers/`, `mcp/`, `server/`, `config/`, `monitoring/`,
   `clients/`, …) are unreferenced leftovers from the crate migration. Verify each is
   truly unreferenced, delete, and update the architecture trees in `README.md` and
   `CLAUDE.md` to describe the `crates/` workspace layout.
2. **Fix Cargo.lock version-bump lag** — the bump process doesn't regenerate
   `Cargo.lock`, causing recurring "sync Cargo.lock" commits (lock at 0.0.122,
   Cargo.toml at 0.0.124). Make the bump script regenerate the lock; sync to 0.0.124.
3. **Mark `plan/2026-01-14-PROGRESS.md` historical** — tracker abandoned ~2026-01-21;
   its stats mislead readers.
4. **Wire real cost calculation** — `crates/lr-router/src/lib.rs` hardcodes
   `cost_usd: 0.0` (TODO). Compute from token usage × `lr-catalog` pricing.
   Also populate `catalog_info` in `crates/lr-server/src/types.rs` if in scope.
5. **Replace `unimplemented!()` panics** in `crates/lr-providers/src/health.rs:257,264`.
6. **Implement Anthropic thinking-block parsing**
   (`crates/lr-providers/src/features/anthropic_thinking.rs:114`), per
   `plan/2026-03-22-REASONING_TOKEN_SUPPORT.md`.
7. **Validate MCP elicitation responses against schema**
   (`crates/lr-mcp/src/gateway/elicitation.rs:191`).
8. **linux/arm64 builds** (GitHub issue #6) — add `linux/arm64` to
   `.github/workflows/docker.yml`; evaluate aarch64 Linux release artifacts.

## Conventions

- One conventional commit per item, only staging files modified by that item.
- Pre-commit CI parity: `rustup run stable cargo clippy --workspace --all-targets -- -D warnings`
  and `rustup run stable cargo fmt --all -- --check`; targeted `cargo test --package` per touched crate.
- `crates/lr-catalog/catalog/modelsdev_raw.json` working-tree change is pre-existing
  and out of scope — do not stage.

## Mandatory Final Steps

1. **Plan Review** — re-check each scope item against the implementation; close gaps.
2. **Test Coverage Review** — add tests for uncovered new/modified paths.
3. **Bug Hunt** — re-read all changes looking for off-by-ones, races, missing error
   handling, incorrect state transitions.
4. **Commit** — all changes committed (no push unless requested).
