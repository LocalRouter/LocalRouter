# Plan: Split LocalRouter into Multi-Crate Workspace

## Problem
Single crate (~67k LOC) means any change recompiles everything. Key pain points:
- 54k-line generated `catalog.rs` recompiles on unrelated changes
- Candle ML deps (with Metal) pulled in for all builds
- utoipa proc macros re-expand on any server-adjacent change
- 36 integration test binaries each link separately

## Approach
- **All at once** — extract all crates in a single pass
- **Clean break** — no re-exports; all imports update to `lr_*::` directly

## Proposed Workspace Layout

```
localrouterai/
├── Cargo.toml                    # Workspace root (expanded)
├── crates/
│   ├── lr-types/                 # Shared types, error types, traits
│   ├── lr-utils/                 # Crypto, test_mode helpers
│   ├── lr-catalog/               # Model catalog + build.rs (54k generated)
│   ├── lr-config/                # ConfigManager, AppConfig, validation, migration
│   ├── lr-api-keys/              # KeychainStorage, CachedKeychain
│   ├── lr-oauth/                 # oauth_browser + oauth_clients merged
│   ├── lr-clients/               # ClientManager, TokenStore
│   ├── lr-providers/             # 19 providers, ProviderRegistry, feature adapters
│   ├── lr-routellm/              # Candle-based ML routing (isolated heavy deps)
│   ├── lr-monitoring/            # MetricsCollector, SQLite storage, graphs
│   ├── lr-mcp/                   # MCP gateway, bridge, transports
│   ├── lr-skills/                # SkillManager, discovery, executor
│   ├── lr-router/                # Router, RateLimiter, routing strategies
│   └── lr-server/                # Axum server, routes, middleware, OpenAPI
└── src-tauri/                    # Tauri binary: main.rs + ui/ commands only
```

15 library crates + 1 binary crate.

## Dependency Graph

```
Layer 0 (foundation):
  lr-types          (no internal deps)
  lr-utils        → lr-types
  lr-api-keys     → lr-types

Layer 1 (core, parallel):
  lr-catalog       → lr-types                         [ISOLATED: 54k LOC generated]
  lr-config        → lr-types, lr-utils
  lr-routellm      → lr-types, lr-config              [ISOLATED: candle/Metal deps]

Layer 2 (services, parallel):
  lr-oauth         → lr-types, lr-config, lr-api-keys
  lr-clients       → lr-types, lr-config, lr-api-keys, lr-utils
  lr-providers     → lr-types, lr-catalog, lr-utils
  lr-monitoring    → lr-types, lr-config
  lr-mcp           → lr-types, lr-config, lr-api-keys
  lr-skills        → lr-types, lr-config

Layer 3 (orchestration):
  lr-router        → lr-types, lr-config, lr-providers, lr-monitoring, lr-routellm

Layer 4 (presentation):
  lr-server        → lr-router, lr-providers, lr-mcp, lr-clients, lr-monitoring, lr-config, lr-skills

Layer 5 (binary):
  src-tauri        → lr-server, lr-config, lr-providers, lr-mcp, lr-clients,
                     lr-monitoring, lr-routellm, lr-skills, lr-oauth (+ tauri deps)
```

## Key Design Decisions

### 1. `lr-types` — shared types crate
Extract from `utils::errors` and cross-module types (provider traits, common request/response structs). Foundation for all other crates.

### 2. `lr-catalog` — biggest compile-time win
Move `catalog/` dir + `buildtools/` + catalog `build.rs` logic into own crate. Main `src-tauri/build.rs` keeps only `tauri_build::build()`. 54k generated lines won't recompile on provider or route changes.

### 3. `lr-routellm` — second biggest win
Candle + Metal deps isolated. Normal server/provider work doesn't trigger ML recompilation.

### 4. `lr-oauth` — consolidation
Merge `oauth_browser` + `oauth_clients` into single crate. Closely related, small.

### 5. `lr-server` — OpenAPI contained
utoipa proc macros stay in server crate. Provider/routing changes don't re-expand OpenAPI macros.

### 6. Integration tests stay in `src-tauri/tests/`
High churn for marginal gain. Tests will import `lr_*` crates directly. Revisit later if needed.

### 7. `src-tauri` lib.rs removed
No more lib.rs re-exports. `src-tauri` becomes a pure binary crate with `main.rs` + `ui/` Tauri commands + `updater` + `cli`. All module code lives in `crates/`.

## Implementation Steps

All done in a single pass:

### Step 1: Create crate scaffolding
- Create `crates/lr-*/` directories with `Cargo.toml` and `src/lib.rs`
- Each `Cargo.toml` uses `workspace = true` for shared metadata and deps

### Step 2: Move source files
For each module → crate mapping:
- `src-tauri/src/utils/` → `crates/lr-utils/src/`
- `src-tauri/src/catalog/` + `src-tauri/catalog/` + `src-tauri/buildtools/` → `crates/lr-catalog/src/` with its own `build.rs`
- `src-tauri/src/config/` → `crates/lr-config/src/`
- `src-tauri/src/api_keys/` → `crates/lr-api-keys/src/`
- `src-tauri/src/oauth_browser/` + `src-tauri/src/oauth_clients/` → `crates/lr-oauth/src/`
- `src-tauri/src/clients/` → `crates/lr-clients/src/`
- `src-tauri/src/providers/` → `crates/lr-providers/src/`
- `src-tauri/src/routellm/` → `crates/lr-routellm/src/`
- `src-tauri/src/monitoring/` → `crates/lr-monitoring/src/`
- `src-tauri/src/mcp/` → `crates/lr-mcp/src/`
- `src-tauri/src/skills/` → `crates/lr-skills/src/`
- `src-tauri/src/router/` → `crates/lr-router/src/`
- `src-tauri/src/server/` → `crates/lr-server/src/`

Remaining in `src-tauri/src/`: `main.rs`, `ui/`, `updater/`, `cli.rs`

### Step 3: Extract types into `lr-types`
Identify types used across multiple modules and move them to `lr-types`. This requires analysis during implementation — common candidates:
- Error types from `utils::errors`
- Provider trait definitions
- Common config types that other crates need

### Step 4: Update all imports
- Replace all `use crate::module` → `use lr_module::` throughout
- Replace `use super::` where crossing crate boundaries
- Update `pub(crate)` visibility to `pub` where needed for cross-crate access

### Step 5: Update Cargo.toml files
- Root `Cargo.toml`: add all crate members + workspace deps for lr-* crates
- `src-tauri/Cargo.toml`: remove moved deps, add lr-* crate deps
- Each `crates/lr-*/Cargo.toml`: declare only the deps that crate needs

### Step 6: Update build.rs
- `src-tauri/build.rs`: remove catalog generation, keep only `tauri_build::build()`
- `crates/lr-catalog/build.rs`: catalog generation logic

### Step 7: Update `src-tauri`
- Remove `lib.rs` entirely (or make it minimal — just re-export for test convenience)
- `main.rs`: import from `lr_*` crates directly
- `ui/` commands: import from `lr_*` crates

### Step 8: Fix compilation
- Resolve visibility issues (`pub(crate)` → `pub` where needed)
- Fix any circular dependency issues by moving types to `lr-types`
- Handle `#[cfg(test)]` modules that reference cross-crate internals

## Verification

```bash
cargo build                    # Compiles
cargo test                     # All tests pass
cargo clippy                   # No new warnings
cargo tauri dev                # App launches
cargo build --timings          # Verify parallel compilation
```

Incremental build check: touch a file in `crates/lr-providers/src/`, rebuild, confirm only `lr-providers` → `lr-router` → `lr-server` → `src-tauri` recompile.

## Files Modified

- `Cargo.toml` — add 15 workspace members + lr-* workspace deps
- `src-tauri/Cargo.toml` — slim deps, add lr-* path deps, remove lib section
- `src-tauri/build.rs` — remove catalog generation
- `src-tauri/src/lib.rs` — remove or make minimal
- `src-tauri/src/main.rs` — update imports to lr_*
- `src-tauri/src/ui/**` — update imports to lr_*
- `src-tauri/src/updater/**` — update imports to lr_*
- `src-tauri/tests/**` — update imports to lr_*
- All moved source files — update `use crate::` → `use lr_*::`
- New: 15 `crates/lr-*/Cargo.toml` + `crates/lr-*/src/lib.rs`
