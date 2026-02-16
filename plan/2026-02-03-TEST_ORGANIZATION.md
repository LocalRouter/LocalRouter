# Plan: Move All Inline Tests to Integration Test Directories

## Summary
Move all 102 inline `#[cfg(test)] mod tests { ... }` blocks from source files across the entire project into proper `tests/` directories. Each workspace crate gets its own `tests/` directory; the main crate uses `src-tauri/tests/`.

## Step 1: Update CLAUDE.md
Add a rule to `CLAUDE.md` under Critical Rules:
```
### Test Organization
- **No inline tests** in source files (`src/` directories)
- All tests go in the crate's `tests/` directory as integration tests
- Test files named: `{module}_tests.rs` (e.g., `types_tests.rs`)
```

## Step 2: Migration Strategy (per file)

For each inline test module:
1. Create a new test file in the crate's `tests/` directory
2. Replace `use super::*` with proper `use crate_name::...` imports
3. Move mock types/helpers to the test file
4. Remove `#[cfg(test)] mod tests { ... }` from source
5. If items are `pub(crate)` or private, make them `pub` so integration tests can access them

### Naming convention
- `src/types.rs` → `tests/types_tests.rs`
- `src/lib.rs` → `tests/lib_tests.rs`
- `src/features/mod.rs` → `tests/features_tests.rs`
- `src/gateway/mod.rs` → `tests/gateway_tests.rs`

## Step 3: Files to Migrate

### Main crate (src-tauri) → `src-tauri/tests/`
| Source | Target |
|--------|--------|
| `src/cli.rs` | `tests/cli_tests.rs` |
| `src/ui/tray_graph.rs` | `tests/tray_graph_tests.rs` |
| `src/ui/tray_graph_manager.rs` | `tests/tray_graph_manager_tests.rs` |

### lr-config → `crates/lr-config/tests/`
| Source | Target |
|--------|--------|
| `src/types.rs` | `tests/types_tests.rs` |
| `src/validation.rs` | `tests/validation_tests.rs` |
| `src/migration.rs` | `tests/migration_tests.rs` |
| `src/storage.rs` | `tests/storage_tests.rs` |

### lr-clients → `crates/lr-clients/tests/`
| Source | Target |
|--------|--------|
| `src/manager.rs` | `tests/manager_tests.rs` |
| `src/token_store.rs` | `tests/token_store_tests.rs` |

### lr-providers → `crates/lr-providers/tests/`
35 files - every provider file + features + oauth + registry + health

### lr-server → `crates/lr-server/tests/`
| Source | Target |
|--------|--------|
| `src/lib.rs` | `tests/lib_tests.rs` |
| `src/openapi/mod.rs` | `tests/openapi_tests.rs` |
| `src/middleware/client_auth.rs` | `tests/client_auth_tests.rs` |
| `src/state.rs` | `tests/state_tests.rs` |
| `src/routes/helpers.rs` | `tests/route_helpers_tests.rs` |
| `src/routes/oauth.rs` | `tests/route_oauth_tests.rs` |
| `src/routes/mcp.rs` | `tests/route_mcp_tests.rs` |
| `src/routes/chat.rs` | `tests/route_chat_tests.rs` |

### lr-mcp → `crates/lr-mcp/tests/`
~18 files across transport, bridge, gateway, manager, oauth

### lr-oauth → `crates/lr-oauth/tests/`
6 files - browser types/token_exchange/callback_server/pkce/flow_manager + clients

### lr-monitoring → `crates/lr-monitoring/tests/`
8 files - logger, metrics, graphs, storage, aggregation, mcp variants

### lr-router → `crates/lr-router/tests/`
| Source | Target |
|--------|--------|
| `src/lib.rs` | `tests/lib_tests.rs` |
| `src/rate_limit.rs` | `tests/rate_limit_tests.rs` |

### lr-routellm → `crates/lr-routellm/tests/`
| Source | Target |
|--------|--------|
| `src/candle_router.rs` | `tests/candle_router_tests.rs` |
| `src/tests.rs` + `src/edge_case_tests.rs` | `tests/routellm_tests.rs` + `tests/edge_case_tests.rs` |

### lr-catalog → `crates/lr-catalog/tests/`
3 files - matcher, lib, buildtools/models

### lr-utils → `crates/lr-utils/tests/`
3 files - paths, test_mode, crypto

### lr-api-keys → `crates/lr-api-keys/tests/`
2 files - keychain, keychain_trait

### lr-skills → `crates/lr-skills/tests/`
3 files - types, discovery, manager

## Step 4: Visibility Changes
- Items accessed by tests that are currently private or `pub(crate)` will be made `pub`
- Mock types (e.g., `MockKeychain` in clients tests) move into the test file
- Test-only re-exports (`#[cfg(test)] pub use ...`) get removed from source

## Step 5: Handle External Test Module References
Files that already use `#[cfg(test)] mod tests;` (pointing to sibling tests.rs):
- `crates/lr-mcp/src/gateway/mod.rs` → move `gateway/tests.rs` to `tests/gateway_tests.rs`
- `crates/lr-routellm/src/lib.rs` → move `src/tests.rs` to `tests/routellm_tests.rs`
Remove the `#[cfg(test)]` declarations and sibling test files.

## Step 6: Verification
```bash
# Full test suite
cargo test --workspace

# Clippy
cargo clippy --workspace

# Verify no inline tests remain
grep -r '#\[cfg(test)\]' crates/*/src/ src-tauri/src/ --include='*.rs'
```

## Execution Order
Due to the massive scope (50+ files), execute in batches by crate:
1. Update CLAUDE.md
2. lr-utils, lr-api-keys (small, foundational)
3. lr-config, lr-types
4. lr-clients
5. lr-monitoring
6. lr-catalog
7. lr-providers (largest - 35 files)
8. lr-router, lr-routellm
9. lr-mcp (21 files)
10. lr-server
11. lr-oauth
12. lr-skills
13. src-tauri (main crate: cli, ui)
14. Final verification
