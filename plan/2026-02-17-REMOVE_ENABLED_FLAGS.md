# Remove guardrails enabled flags; fix model_type_id filter bug

**Date**: 2026-02-17
**Status**: Implemented

## Problem

The guardrails system had unnecessary `enabled` flags at three levels:
1. **Global** `GuardrailsConfig.enabled` - already deprecated/migration shim
2. **Per-model** `SafetyModelConfig.enabled` - redundant; a model in the list IS enabled
3. **Per-client** `ClientGuardrailsConfig.enabled` - redundant; having non-allow category actions means guardrails are active

Additionally, `run_checks_filtered` compared `model_type_id()` (hardcoded type like `"llama_guard"`) against the instance ID (like `"llamaguard-4-local"`), so specific-model testing never matched.

## Changes Made

### 1. SafetyModelConfig.enabled → skip_serializing migration shim
- `crates/lr-config/src/types.rs`: Field kept for deserialization compat, skipped on serialize
- `crates/lr-guardrails/src/engine.rs`: Removed `enabled` from `SafetyModelConfigInput` and the `if !model_cfg.enabled` guard
- `src-tauri/src/main.rs`: Removed `guardrails_config.enabled &&` and `.any(|m| m.enabled)` checks
- `src-tauri/src/ui/commands.rs`: Updated `rebuild_safety_engine` and `get_safety_model_status`
- Frontend: Removed `enabled` from `SafetyModelConfig` type and all model creation sites

### 2. ClientGuardrailsConfig.enabled → skip_serializing migration shim
- `crates/lr-config/src/types.rs`: Field kept for deserialization compat, skipped on serialize
- `crates/lr-server/src/routes/chat.rs` and `completions.rs`: Check `category_actions.is_empty()` instead
- `src-tauri/src/ui/commands_clients.rs`: AllowPermanent clears category_actions; `set_client_guardrails_enabled` is now a no-op
- Frontend: Removed `enabled` from `ClientGuardrailsConfig`, removed Switch toggle from client guardrails tab

### 3. default_safety_models() → empty vec
- Predefined models are catalog entries shown in picker UI, not active models

### 4. Fixed model_type_id filter bug
- Added `fn id(&self) -> &str` to `SafetyModel` trait (returns instance ID)
- Implemented on all 5 model types: LlamaGuard, GraniteGuardian, ShieldGemma, Nemotron, Custom
- Changed `run_checks_filtered` to use `m.id()` instead of `m.model_type_id()`

## Verification

- `cargo test -p lr-guardrails`: 38 tests pass
- `cargo check`: compiles cleanly
- `cargo clippy`: no new warnings
- `npx tsc --noEmit`: TypeScript types compile
