# Add Local LLM Providers + Model Pull Support

**Date:** 2026-02-24

## Goal
Add Jan.ai, GPT4All, and LocalAI as providers. Add model pull support for LM Studio and LocalAI. Promote model pulling to a first-class trait.

## Approach
1. Add `PullProgress` type + `supports_pull()`/`pull_model()` methods to `ModelProvider` trait
2. Create 3 new provider modules (jan, gpt4all, localai) based on lmstudio pattern
3. Implement pull for LM Studio (REST API polling) and LocalAI (job polling)
4. Register in factory, config types, and main.rs
5. Rewrite `pull_provider_model` command to use trait generically
6. Update frontend (pullable providers, safety model variants, mock data)

## Files Modified
- `crates/lr-providers/src/lib.rs` - PullProgress + trait methods + module declarations
- `crates/lr-providers/src/ollama.rs` - Move pull_model into trait impl
- `crates/lr-providers/src/lmstudio.rs` - Add pull support
- `crates/lr-providers/src/factory.rs` - 3 factory impls + discovery
- `crates/lr-config/src/types.rs` - 3 ProviderType variants
- `src-tauri/src/main.rs` - Register factories + match arms
- `src-tauri/src/ui/commands.rs` - Generic pull_provider_model
- `src/components/guardrails/SafetyModelPicker.tsx` - PULLABLE_PROVIDER_TYPES
- `src/constants/safety-model-variants.ts` - LocalAI model mappings
- `website/src/components/demo/mockData.ts` - 3 provider types

## Files Created
- `crates/lr-providers/src/jan.rs`
- `crates/lr-providers/src/gpt4all.rs`
- `crates/lr-providers/src/localai.rs`
