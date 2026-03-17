# Fix Permission Inheritance: Tests & Model Resolution Bug

## Context

The user reports that when `model_permissions.global: off` and `model_permissions.providers.X: off`, but `model_permissions.models.X__model: allow`, the model is still blocked with 403. The expected behavior is that the most specific permission (model-level) should override the parent (provider/global).

**Root cause**: The `resolve_model()` / `resolve_tool()` etc. functions in `lr-config/src/types.rs` are actually **correct** — child overrides parent. The real bug is in **model name resolution**: when a model name exists in multiple providers (e.g., `llama3.2:1b` in both "Ollama" and "Ollama [Subscription Free Tier]"), `.find()` picks the first match, which may be the wrong provider. The permission check then uses the wrong provider name, so the model-specific key doesn't match.

Example: User's config has `"Ollama [Subscription Free Tier]__llama3.2:1b": allow`, but `.find()` resolves to `"Ollama"` provider → key becomes `"Ollama__llama3.2:1b"` → not found → falls back to provider/global → `Off` → 403.

## Plan

### 1. Add comprehensive "child allows, parent blocks" inheritance tests

**File**: `src-tauri/tests/permission_inheritance_tests.rs`

Add new test modules for the missing edge cases across ALL permission types:

**For each type (MCP, Skills, Models):**
- `test_child_allow_overrides_parent_off` — child=Allow, parent=Off → Allow
- `test_child_ask_overrides_parent_off` — child=Ask, parent=Off → Ask
- `test_child_allow_overrides_all_parents_off` — child=Allow, parent=Off, grandparent=Off → Allow
- `test_child_ask_overrides_all_parents_off` — child=Ask, parent=Off, grandparent=Off → Ask

**MCP-specific:**
- Tool=Allow, Server=Off, Global=Off → Allow
- Resource=Allow, Server=Off, Global=Off → Allow
- Prompt=Allow, Server=Off, Global=Off → Allow
- Tool=Ask, Server=Off, Global=Off → Ask

**Skills-specific:**
- Tool=Allow, Skill=Off, Global=Off → Allow
- Tool=Ask, Skill=Off, Global=Off → Ask

**Models-specific:**
- Model=Allow, Provider=Off, Global=Off → Allow
- Model=Ask, Provider=Off, Global=Off → Ask

**Also for `has_any_enabled_for_*` methods:**
- Provider=Off but model=Allow → `has_any_enabled_for_provider` should return true
- Server=Off but tool=Allow → `has_any_enabled_for_server` should return true
- Skill=Off but tool=Allow → `has_any_enabled_for_skill` should return true

### 2. Fix model name resolution for duplicate models across providers

**Files**:
- `crates/lr-server/src/routes/chat.rs` (validate_client_provider_access + check_model_firewall_permission)
- `crates/lr-server/src/routes/completions.rs` (same pattern)
- `crates/lr-server/src/routes/embeddings.rs` (same pattern)

When resolving a model name without provider prefix, instead of `.find()` (first match), prefer a model from a provider that the client actually has permission for. Strategy:

1. Collect ALL matching models (not just first)
2. Try to find one from a provider where `model_permissions.resolve_model()` returns enabled
3. Fall back to first match if none are explicitly allowed (let the normal 403 flow handle it)

This is a targeted fix in the model name resolution block inside `validate_client_provider_access` and `check_model_firewall_permission`.

### 3. Add tests for model name resolution with duplicate provider models

**File**: `src-tauri/tests/permission_inheritance_tests.rs` (new module)

Test the scenario where the same model ID exists under multiple providers and client permissions differ per provider.

## Files to modify

1. `src-tauri/tests/permission_inheritance_tests.rs` — add ~30 new test cases
2. `crates/lr-server/src/routes/chat.rs` — fix model resolution in `validate_client_provider_access` and `check_model_firewall_permission`
3. `crates/lr-server/src/routes/completions.rs` — same fix
4. `crates/lr-server/src/routes/embeddings.rs` — same fix

## Verification

```bash
cargo test -p localrouter permission_inheritance
cargo test -p lr-server
cargo test -p lr-mcp -- access_control
cargo clippy
```
