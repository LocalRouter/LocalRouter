# Add Disable/Enable Toggle for GuardRails Safety Models

## Context

Currently, safety models in GuardRails can only be added or removed. The user wants the ability to **disable** a model without removing it — the model stays in config but is not used for guardrails checks. This avoids re-configuring a model (provider, thresholds, categories) after temporarily disabling it.

`SafetyModelConfig` already has an `enabled: bool` field, but it's a migration shim (`skip_serializing`, defaults to `false`). We repurpose it as a real field.

## Changes

### 1. Config struct — `crates/lr-config/src/types.rs` (line 1943-1946)

Change `enabled` from migration shim to real serialized field:
- Remove `skip_serializing`
- Change default from `#[serde(default)]` (false) to `#[serde(default = "default_true")]` (true)
- Update the doc comment

Backward compat: old configs without `enabled` → defaults to `true` (active). The field was never serialized before, so no `false` values exist on disk.

### 2. Engine rebuild filter — `src-tauri/src/ui/commands.rs` (line 3563)

Add `.filter(|m| m.enabled)` before `.map()` when building `model_inputs` in `rebuild_safety_engine`. Add disabled count to the log message at line 3590.

### 3. New Tauri command — `src-tauri/src/ui/commands.rs` (after line 3952)

Add `toggle_safety_model(model_id: String, enabled: bool)`:
- Validates model exists
- Updates `model.enabled`
- Saves config

Register in `src-tauri/src/main.rs` at line 2055.

### 4. TypeScript types — `src/types/tauri-commands.ts`

- Add `enabled: boolean` to `SafetyModelConfig` (line 2540, after `model_type`)
- Add `ToggleSafetyModelParams { modelId: string; enabled: boolean }` (after line 2656)

### 5. UI panel — `src/views/guardrails/guardrails-panel.tsx`

- Add `onToggleModel: (modelId: string, enabled: boolean) => void` to props
- Model list: add `opacity-50` class when `!model.enabled`, show "Disabled" `Badge`
- Settings tab: add "Model Status" card with `Switch` above the Danger Zone card
- Import `Switch` and `Label`

### 6. Parent view — `src/views/guardrails/index.tsx`

- Add `handleToggleModel` handler (invoke `toggle_safety_model`, reload config, rebuild engine)
- Pass `onToggleModel={handleToggleModel}` to `GuardrailsPanel`
- Add `enabled: true` to `SafetyModelConfig` in `handlePickerSelect` (line 154)
- Import `ToggleSafetyModelParams`

### 7. Demo mock — `website/src/components/demo/TauriMockSetup.ts`

- Add `enabled: true` to each mock safety model
- Add `toggle_safety_model` mock handler

## Files to modify

| File | Change |
|------|--------|
| `crates/lr-config/src/types.rs` | `enabled` field: remove `skip_serializing`, default `true` |
| `src-tauri/src/ui/commands.rs` | Filter disabled models in rebuild, add `toggle_safety_model` cmd |
| `src-tauri/src/main.rs` | Register `toggle_safety_model` |
| `src/types/tauri-commands.ts` | Add `enabled` field + `ToggleSafetyModelParams` |
| `src/views/guardrails/guardrails-panel.tsx` | Toggle switch, disabled badge, dimmed list items |
| `src/views/guardrails/index.tsx` | `handleToggleModel` handler, prop wiring |
| `website/src/components/demo/TauriMockSetup.ts` | Mock data + handler |

## Verification

1. `cargo test && cargo clippy` — ensure Rust compiles
2. `npx tsc --noEmit` — ensure TypeScript types compile
3. `cargo tauri dev` — manually test:
   - Add a model → appears enabled by default
   - Disable via Settings toggle → badge shows "Disabled", model dimmed in list
   - Rebuild engine log shows disabled count
   - Re-enable → model active again
   - Try It Out on disabled model → fails gracefully (model not in engine)
