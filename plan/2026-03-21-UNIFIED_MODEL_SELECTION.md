# Unified Model Selection: Merge Allowed Models + Auto Router

## Context

Currently, each client's strategy chooses between two mutually exclusive routing modes:
- **Allowed Models**: Client sees a curated model list, picks one directly
- **Auto Router**: Client sees only `localrouter/auto`, all requests get auto-routed

The user wants to **merge these into one unified mode**: all selected models are both visible for direct routing AND available for auto-routing through the priority order. This eliminates the mode selector and simplifies the mental model.

**Outcome**: A new "Models" tab replaces the routing mode choice with a single three-zone model selector (Enabled/Weak/Disabled). The old "LLM" tab is renamed to "LLM (Legacy)" and kept temporarily.

---

## Phase 1: New Frontend Tab + Component

### 1.1 Replace old tab with new tab

**File**: `src/views/clients/client-detail.tsx`

- Replace the existing `"models"` tab content to render the new `<UnifiedModelsTab>` instead of the old `<ModelsTab>`. Keep the tab value as `"models"` and label as "LLM".
- Remove the import of the old `ModelsTab`

**File**: `src/views/clients/tabs/models-tab.tsx`

- Rename to `src/views/clients/tabs/models-tab-legacy.tsx`
- Add a comment at the top: `// DEPRECATED: Legacy LLM tab — kept for reference. Replaced by unified-models-tab.tsx`
- Do NOT delete — keep as dead code for reference in case we need to pull old components back

### 1.2 Create `UnifiedModelsTab` component

**New file**: `src/views/clients/tabs/unified-models-tab.tsx`

Layout (four sections in order):

1. **Model Selection** — the three-zone selector (see 1.3)
2. **Rate Limits** — reuse existing `RateLimitEditor` from `src/components/strategies/RateLimitEditor.tsx`
3. **Free Tier Mode** — reuse the free-tier toggle + fallback UI from current `models-tab.tsx` (lines 349-414)
4. **Weak Models (RouteLLM)** — toggle switch for RouteLLM, threshold slider, download status. When toggled on, the model selector above shows the Weak zone

Data flow:
- Loads strategy via `invoke('get_strategy', { strategyId })`
- On change, calls `invoke('update_strategy', { strategyId, allowedModels, autoConfig, rateLimits, freeTierOnly, freeTierFallback })`
- Always ensures `auto_config` exists (creates default if null)
- Keeps `allowed_models` and `auto_config` in sync (see Phase 3)

### 1.3 Create three-zone model selector

**New file**: `src/components/strategy/ThreeZoneModelSelector.tsx`

Build on the patterns from `DragThresholdModelSelector.tsx` using `@dnd-kit`. Three zones:

**Zone 1 — Enabled (Strong) Models** (top):
- Priority-ordered list, numbered 1..N
- Each row: grip handle, priority #, model name (monospace), provider badge, pricing badge
- Drag to reorder within zone

**Zone 2 — Weak Models** (middle, only visible when RouteLLM is enabled via prop):
- Same row format with a subtle "weak" visual indicator (dimmer styling or badge)
- Drag to reorder within zone

**Zone 3 — Disabled Models** (bottom):
- Searchable, sortable (by name, provider, price, params)
- Grouped by provider with collapse/expand
- Not ordered — just a pool of available models

**Dividers** between zones:
- "── Strong / Weak ──" divider (only when RouteLLM enabled)
- "── Disabled ──" divider (always)
- Highlight on drag-over to indicate drop target

**Drag operations**:
- Between zones: moves model to target zone (at drop position or end)
- Within Enabled/Weak: reorders
- Click-to-toggle: click disabled model → add to Enabled at bottom; click enabled/weak → move to Disabled

**Props**:
```typescript
interface ThreeZoneModelSelectorProps {
  enabledModels: [string, string][]      // strong, priority ordered
  weakModels: [string, string][]         // weak, ordered
  showWeakZone: boolean                  // controlled by RouteLLM toggle
  allModels: { provider: string; models: string[] }[]
  modelPricing?: Record<string, ModelPricingInfo>
  modelParamCounts?: Record<string, string>
  freeTierKinds?: Record<string, FreeTierKind>
  onEnabledModelsChange: (models: [string, string][]) => void
  onWeakModelsChange: (models: [string, string][]) => void
}
```

### 1.4 Sync logic in `UnifiedModelsTab`

When enabled/weak models change, derive and save both fields atomically:

```typescript
const handleModelsChange = (strong: [string, string][], weak: [string, string][]) => {
  const allEnabled = [...strong, ...weak]
  const allowedModels = {
    selected_all: false,
    selected_providers: [],
    selected_models: allEnabled,
  }
  const autoConfig = {
    permission: 'allow',
    model_name: 'localrouter/auto',
    prioritized_models: strong,
    available_models: [],
    routellm_config: routellmEnabled ? {
      enabled: true,
      threshold,
      weak_models: weak,
    } : null,
  }
  updateStrategy({ allowedModels, autoConfig })
}
```

This ensures `allowed_models` (controls `/v1/models` list) and `auto_config` (controls auto-routing) are always consistent.

---

## Phase 2: Backend — `/v1/models` Returns Both

### 2.1 Update `list_models` endpoint

**File**: `crates/lr-server/src/routes/models.rs` (lines 45-70)

**Current**: If auto_config enabled → return ONLY virtual model. Else → return allowed models.
**New**: Always return allowed models. If auto_config exists with prioritized models, ALSO prepend the virtual `localrouter/auto` model.

```rust
// Always collect allowed individual models
let all_models = state.provider_registry.list_all_models().await...;
let filtered_models: Vec<_> = all_models.into_iter()
    .filter(|m| strategy.is_model_allowed(&m.provider, &m.id))
    .collect();

let mut model_data_vec = Vec::new();

// Prepend virtual auto model if auto_config has prioritized models
if let Some(auto_config) = &strategy.auto_config {
    if auto_config.permission.is_enabled() && !auto_config.prioritized_models.is_empty() {
        model_data_vec.push(ModelData {
            id: auto_config.model_name.clone(),
            // ... virtual model fields
        });
    }
}

// Add individual models
for model_info in filtered_models { ... }
```

### 2.2 Update `get_model` endpoint

**File**: `crates/lr-server/src/routes/models.rs` (lines 134-200)

Remove the guard at lines 172-184 that returns 404 for `localrouter/auto` when auto_config is disabled. Instead, allow looking up the virtual model whenever auto_config exists with prioritized models.

### 2.3 Simplify chat.rs auto-routing trigger

**File**: `crates/lr-server/src/routes/chat.rs` (lines 95-310)

**Current behavior**: When auto_config enabled, EVERY request gets force-overridden to `"localrouter/auto"` regardless of what model the client sent.

**New behavior**: Only auto-route when client explicitly sends `"localrouter/auto"`. Remove the forced override.

Replace lines 95-310 with:

```rust
// Auto-routing firewall check — only for explicit localrouter/auto requests
if request.model == "localrouter/auto" {
    if let Ok((client, strategy)) = get_client_with_strategy(&state, &auth.api_key_id) {
        if let Some(auto_config) = &strategy.auto_config {
            // Monitor intercept: override Allow → Ask if intercept rule matches
            if auto_config.permission.is_enabled()
                && state.mcp_gateway.firewall_manager.should_intercept(
                    &client.id,
                    lr_mcp::gateway::firewall::InterceptCategory::Llm,
                )
            {
                // ... existing Ask popup logic (simplified) ...
            }
        } else {
            // No auto_config — reject localrouter/auto request
            return Err(ApiErrorResponse::not_found("Auto routing not configured"));
        }
    }
}
```

The key change: **remove lines 129-135** (the `request.model = "localrouter/auto"` override in the Allow branch). The rest of the firewall/monitor intercept logic stays for `localrouter/auto` requests only.

### 2.4 Config migration v25

**File**: `crates/lr-config/src/migration.rs`
**File**: `crates/lr-config/src/types.rs` (bump `CONFIG_VERSION` from 24 to 25)

Add `migrate_to_v25` — ensure all strategies have `auto_config`:

```rust
fn migrate_to_v25(config: &mut serde_json::Value) {
    if let Some(strategies) = config["strategies"].as_array_mut() {
        for strategy in strategies {
            if strategy["auto_config"].is_null() {
                strategy["auto_config"] = serde_json::json!({
                    "permission": "allow",
                    "model_name": "localrouter/auto",
                    "prioritized_models": [],
                    "available_models": []
                });
            } else if strategy["auto_config"]["permission"] == "off" {
                strategy["auto_config"]["permission"] = serde_json::json!("allow");
            }
        }
    }
}
```

Note: `prioritized_models` is left empty for migrated "allowed models" strategies. Their `allowed_models` field still works for direct routing. The auto router virtual model won't appear (no prioritized models) until the user configures priority in the new UI.

---

## Phase 3: Update Dependent Code

### 3.1 Connection graph

**File**: `src/components/connection-graph/utils/buildGraph.ts` (line 239-240)
- `auto_config` is now always present. Update the null check to instead check if `prioritized_models` is non-empty.

**File**: `src/components/connection-graph/types.ts` (line 138)
- Keep `auto_config` as optional in the type (backward compat), but code should handle it being always present.

### 3.2 Client info tab

**File**: `src/views/clients/tabs/info-tab.tsx` (lines 139-142)
- Show strong/weak model info when `auto_config?.prioritized_models?.length > 0` instead of checking `auto_config` existence.

### 3.3 Tray menu

**File**: `src-tauri/src/ui/tray_menu.rs` (lines 231-243)
- Keep the weak model toggle conditional, but simplify: show when `auto_config` exists AND `routellm_config` has weak models (which is always true now when RouteLLM is enabled).

**File**: `website/src/components/demo/MacOSTrayMenu.tsx` (line 99-100)
- Match the simplified condition.

### 3.4 TypeScript types

**File**: `src/types/tauri-commands.ts`
- Keep `auto_config?: AutoModelConfig | null` (don't change to required — backward compat with old configs pre-migration)
- Add comment: `// Always present after config v25 migration`

### 3.5 Demo mock data

**File**: `website/src/components/demo/mockData.ts`
- Update all strategies to always have `auto_config` (never `null`). Strategies that were "allowed models only" get `auto_config` with empty `prioritized_models`.

**File**: `website/src/components/demo/TauriMockSetup.ts`
- Update `create_strategy` mock to always include default `auto_config`
- Update `update_strategy` mock handler

### 3.6 Completions/embeddings endpoints

**Files**: `crates/lr-server/src/routes/completions.rs`, `crates/lr-server/src/routes/embeddings.rs`
- These already check `request.model == "localrouter/auto"` for special handling — no change needed since the model name isn't being force-overridden anymore.

### 3.7 Router module

**Files**: `src-tauri/src/router/mod.rs`, `crates/lr-router/src/lib.rs`
- The `if request.model == "localrouter/auto"` branching still works correctly — auto-routing happens when the client explicitly requests it.
- The permission checks (`auto_config.permission.is_enabled()`) remain as safety guards even though permission is always "allow".

### 3.8 Config validation

**File**: `crates/lr-config/src/validation.rs` (lines 254-283)
- Auto_config validation now always runs (since it's always present). No change needed — the `if let Some(auto_config)` pattern still works.

---

## Phase 4: Verification

### Manual testing
1. Open client settings → new "Models" tab appears, legacy "LLM (Legacy)" tab still accessible
2. Three-zone selector: drag models between Enabled/Weak/Disabled zones
3. Toggle RouteLLM → Weak zone appears/disappears in selector
4. Rate limits and free tier mode work in the new tab
5. API: `GET /v1/models` returns both `localrouter/auto` and individual enabled models
6. API: Send request with specific model → direct routing works
7. API: Send request with `localrouter/auto` → auto-routing works
8. API: Send request with disabled model → rejected
9. Monitor intercept still works for `localrouter/auto` requests
10. Connection graph displays correctly

### Automated
- `cargo test` — all existing tests pass
- `cargo clippy` — no warnings
- `npx tsc --noEmit` — TypeScript compiles
- Migration test: v24 config → v25 migration adds auto_config to strategies without it

### Mandatory final steps
1. **Plan review**: Check all planned changes were implemented
2. **Test coverage**: Add tests for migration, updated `list_models`, auto-routing trigger
3. **Bug hunt**: Re-read implementation for edge cases (empty prioritized_models, RouteLLM disabled but weak models in list, etc.)

---

## Files to Modify

| File | Change |
|------|--------|
| `src/views/clients/client-detail.tsx` | Swap `ModelsTab` → `UnifiedModelsTab` import |
| `src/views/clients/tabs/models-tab.tsx` | **RENAME** → `models-tab-legacy.tsx`, mark deprecated |
| `src/views/clients/tabs/unified-models-tab.tsx` | **NEW** — unified tab component |
| `src/components/strategy/ThreeZoneModelSelector.tsx` | **NEW** — three-zone DnD selector |
| `crates/lr-server/src/routes/models.rs` | Return both auto + individual models |
| `crates/lr-server/src/routes/chat.rs` | Remove forced auto-routing override |
| `crates/lr-config/src/types.rs` | Bump CONFIG_VERSION to 25 |
| `crates/lr-config/src/migration.rs` | Add v25 migration |
| `src/types/tauri-commands.ts` | Add comment about auto_config always present |
| `src/components/connection-graph/utils/buildGraph.ts` | Update auto_config null check |
| `src/views/clients/tabs/info-tab.tsx` | Simplify auto_config conditional |
| `src-tauri/src/ui/tray_menu.rs` | Simplify weak model toggle condition |
| `website/src/components/demo/mockData.ts` | All strategies get auto_config |
| `website/src/components/demo/TauriMockSetup.ts` | Update mock handlers |
| `website/src/components/demo/MacOSTrayMenu.tsx` | Simplify condition |
