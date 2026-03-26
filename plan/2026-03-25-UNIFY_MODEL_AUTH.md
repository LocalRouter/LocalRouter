# Unify Model Authorization: Remove Duplicate System

## Context

There are **two independent model authorization checks** that both must pass for every request:

1. **Strategy `allowed_models`** (`AvailableModelsSelection`) — binary whitelist on `Strategy` struct
2. **Client `model_permissions`** (`ModelPermissions`) — hierarchical Allow/Ask/Off on `Client` struct

This causes the bug: user enables a model in the strategy UI, but the client's `model_permissions` defaults to `Off` → 403. The frontend `UnifiedModelsTab` already tries to present these as one concept, but the backend enforces them separately.

**Decision: Deprecate `allowed_models` on Strategy, move `model_permissions` from Client to Strategy.**

Rationale:
- `model_permissions` is the richer system (Allow/Ask/Off vs binary)
- `model_permissions` replaced the older `allowed_llm_providers` field (per plan `2026-02-04-PERMISSION_MIGRATION.md`)
- `allowed_models` has no ordering and its TypeScript type in `tauri-commands.ts` is already wrong
- Strategy is the correct location (each client gets its own strategy via `new_for_client()`)
- Eliminates the dual-check that caused the user's 403 bug

**End state:**
- One `model_permissions` field on Strategy with hierarchical Allow/Ask/Off
- `auto_config.prioritized_models` = ordered strong list (unchanged)
- `routellm_config.weak_models` = ordered weak list (unchanged)

---

## Step 1: Add `model_permissions` to Strategy, deprecate `allowed_models`

**File: `crates/lr-config/src/types.rs`**

On `Strategy` struct (~line 353):
- Add `pub model_permissions: ModelPermissions` with `#[serde(default)]`
- Change `allowed_models` to `#[serde(default, skip_serializing)]` (deserialize-only migration shim)
- Update `Strategy::new()` → set `model_permissions.global = Allow`
- Update `Strategy::new_for_client()` → set `model_permissions.global = Allow`
- Update `Strategy::is_model_allowed()` → delegate to `model_permissions.resolve_model().is_enabled()`

On `Client` struct (~line 2758):
- Change `model_permissions` to `#[serde(default, skip_serializing)]` (deserialize-only migration shim)

---

## Step 2: Config migration v25 → v26

**File: `crates/lr-config/src/migration.rs`** — Add `migrate_to_v26()`
**File: `crates/lr-config/src/types.rs`** — Bump `CONFIG_VERSION` to 26

The migration must **combine both systems** to produce the final `model_permissions` on each strategy. The current runtime behavior is: a model must pass BOTH checks. The migrated result must preserve that intersection.

### Algorithm per strategy:

**Phase A — Resolve strategy `allowed_models` into a permission map:**
- If `selected_all == true` → `strategy_global = Allow`, empty provider/model maps
- If `selected_all == false` → `strategy_global = Off`, then:
  - Each `selected_providers[p]` → `strategy_providers[p] = Allow`
  - Each `selected_models[(p, m)]` → `strategy_models["p__m"] = Allow`

**Phase B — Find owning client (via `strategy.parent`) and combine:**

For each model entry, the effective permission = **min(strategy_permission, client_permission)** where Off < Ask < Allow:

| Strategy says | Client says | Result |
|--------------|-------------|--------|
| Off          | (anything)  | Off    |
| Allow        | Off         | Off    |
| Allow        | Ask         | Ask    |
| Allow        | Allow       | Allow  |

Concretely:
1. Start with `final.global = min(strategy_global, client.model_permissions.global)`
2. For each provider `p` in **either** `strategy_providers` or `client.model_permissions.providers`:
   - Resolve the strategy-side permission for `p` (explicit entry or `strategy_global`)
   - Resolve the client-side permission for `p` (`client.model_permissions.resolve_provider(p)`)
   - `final.providers[p] = min(strategy_perm, client_perm)`
   - Only write if it differs from `final.global` (avoid redundant entries)
3. For each model key `"p__m"` in **either** `strategy_models` or `client.model_permissions.models`:
   - Resolve the strategy-side permission for this model (explicit entry → provider → global)
   - Resolve the client-side permission (`client.model_permissions.resolve_model(p, m)`)
   - `final.models["p__m"] = min(strategy_perm, client_perm)`
   - Only write if it differs from `final.providers[p]` or `final.global` (avoid redundant entries)

**Phase C — No owning client (shared strategies without `parent`):**
- Use strategy-derived permissions only, with `Allow` as the client-side default (preserves current behavior where strategies without parent clients aren't gated by client permissions)

**Phase D — Write result:**
- Set `strategy.model_permissions = final`
- Clear `client.model_permissions` (field becomes skip_serializing)

---

## Step 3: Update validation

**File: `crates/lr-config/src/validation.rs`**

Update `validate_strategies()` (~line 364-383):
- Replace `allowed_models.selected_providers` / `selected_models` validation with `model_permissions.providers` / `model_permissions.models` key validation against configured provider names

---

## Step 4: Unify route handler authorization (eliminate dual check)

### 4a: Rewrite `validate_strategy_model_access()`

**File: `crates/lr-server/src/routes/helpers.rs` (~line 246)**

Replace `allowed_models` check with `model_permissions` check:
```rust
pub fn validate_strategy_model_access(state, strategy, model) -> HelperResult<()> {
    // Parse "provider/model" or just "model"
    // Use strategy.model_permissions.resolve_model(provider, model_id)
    // If !is_enabled() → 403
}
```

### 4b: Remove `validate_client_provider_access()` from all route handlers

These all have the same pattern: `validate_strategy_model_access()` call followed by `validate_client_provider_access()` call. Remove the second call and delete the function.

| File | Remove function | Remove call site(s) |
|------|----------------|---------------------|
| `crates/lr-server/src/routes/chat.rs` | ~line 846-917 | ~line 286 |
| `crates/lr-server/src/routes/completions.rs` | ~line 1120+ | ~line 118 |
| `crates/lr-server/src/routes/embeddings.rs` | ~line 413+ | ~line 99 |
| `crates/lr-server/src/routes/audio.rs` | ~line 1157+ | ~lines 318, 751, 996 |

### 4c: Update firewall check to read from strategy

**File: `crates/lr-server/src/routes/chat.rs` (~line 1006-1095)**

In `check_model_firewall_permission()`:
- Line 1048-1054: currently reads `client.model_permissions` for provider disambiguation → read from strategy instead
- Line 1068-1069: `FirewallCheckContext::Model { permissions: &client.model_permissions, ... }` → read from strategy's `model_permissions`

This requires passing the strategy (or its `model_permissions`) into `check_model_firewall_permission()`. The strategy is already available at the call site via `get_client_with_strategy()`.

---

## Step 5: Router + models endpoint — no code changes needed

After Step 1, `Strategy::is_model_allowed()` delegates to `model_permissions.resolve_model().is_enabled()`. All 6 call sites in `crates/lr-router/src/lib.rs` and 3 in `crates/lr-server/src/routes/models.rs` automatically pick up the new implementation.

---

## Step 6: Update Tauri commands

**File: `src-tauri/src/ui/commands_clients.rs`**

1. `update_strategy()` (~line 939): Add `model_permissions: Option<ModelPermissions>` param. Keep `allowed_models` param for backward compat but log a deprecation warning if used.

2. `set_client_model_permission()` (~line 2408): Change to `set_strategy_model_permission()` — operates on strategy instead of client. Keep old command as alias that looks up the client's strategy and delegates.

3. `clear_client_model_child_permissions()` (~line 2587): Same — change to operate on strategy.

4. `get_client_info()` / `ClientInfo`: Populate `model_permissions` from the client's strategy (for backward compat with frontend until updated).

---

## Step 7: Update frontend

**File: `src/types/tauri-commands.ts`**
- Fix/remove broken `AvailableModelsSelection` type (~line 264-267)
- Add `model_permissions: ModelPermissions` to Strategy type
- Update `UpdateStrategyParams` to include `modelPermissions`

**File: `src/views/clients/tabs/unified-models-tab.tsx`**
- `handleModelsChange()` (~line 363): Set `model_permissions` on strategy instead of `allowed_models`
- Build `model_permissions` from enabled models: `global: Off`, each enabled model → `models["provider__model"] = Allow`
- Remove `allowed_models` from `updateStrategy()` call

**File: `src/components/permissions/ModelsPermissionTree.tsx`**
- Change to operate on strategy ID instead of client ID
- Call `set_strategy_model_permission` instead of `set_client_model_permission`

**File: `website/src/components/demo/TauriMockSetup.ts`**
- Update mock to include `model_permissions` on strategy
- Remove `model_permissions` from client mock

---

## Step 8: Update tests

| File | Changes |
|------|---------|
| `src-tauri/tests/permission_inheritance_tests.rs` | Keep ModelPermissions resolution tests (struct unchanged). Add strategy-level permission tests. |
| `src-tauri/tests/router_strategy_tests.rs` | Update fixtures: remove client `model_permissions`, add strategy `model_permissions` |
| `crates/lr-config/src/types.rs` tests (~line 3957) | Update AvailableModelsSelection tests or remove; add strategy model_permissions tests |
| `crates/lr-config/src/migration.rs` tests | Add v25→v26 migration tests: selected_all→global Allow, selected_models→model entries, client merge |

---

## Step 9: Review, test, bug hunt

1. **Plan review**: Check all `allowed_models` and `client.model_permissions` references are addressed
2. **Test coverage**: `cargo test && cargo clippy && cargo fmt`
3. **Bug hunt**: Focus on case sensitivity (HashMap keys), migration edge cases (client without strategy parent), default permission state for new strategies
4. **Manual verification**: Create a client, enable/disable models in UI, verify `/v1/models` filtering and request authorization work with single check
5. **Commit**

---

## Verification

```bash
cargo test && cargo clippy && cargo fmt
```

Manual tests:
- New client → all models visible in `/v1/models`
- Disable a model in UI → model disappears from `/v1/models`, returns 403 on direct request
- Set model to Ask → firewall popup appears
- Auto-routing with strong/weak lists → respects permission states
- Old config files with `allowed_models` + client `model_permissions` → migrate correctly to v26
