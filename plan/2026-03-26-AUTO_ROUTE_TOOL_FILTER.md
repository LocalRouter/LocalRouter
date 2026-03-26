# Auto-Routing: Skip Models Without Tool Support

## Context

When a client sends a request with `tools` to `localrouter/auto`, the auto-router tries models in priority order. If a model doesn't support tools, the provider API rejects it (e.g., Cerebras 422), wasting time and causing unnecessary fallbacks. We need the auto-router to skip models that lack tool/function calling support when the request contains tools.

## Changes

### 1. Wire catalog `tool_call` into `FunctionCalling` capability enrichment

**File**: `crates/lr-providers/src/lib.rs`

In both `enrich_with_catalog()` (~line 771) and `enrich_with_catalog_by_name()` (~line 817), after the existing Vision enrichment block, add:

```rust
if catalog_model.capabilities.tool_call
    && !self.capabilities.contains(&Capability::FunctionCalling)
{
    self.capabilities.push(Capability::FunctionCalling);
}
```

This follows the exact pattern used for Vision. Safe because it only *adds* the capability — providers that already declare FunctionCalling won't be affected.

**Impact**: ~8 multi-provider systems (Ollama, LMStudio, OpenRouter, OpenAI-compatible, GPT4All, LlamaCpp, LocalAI, Jan) will get accurate FunctionCalling capabilities from the catalog.

### 2. Fix Gemini's missing FunctionCalling declaration

**File**: `crates/lr-providers/src/gemini.rs` (~line 293)

Gemini supports tool calling but doesn't declare it. Add `Capability::FunctionCalling` to the hardcoded capabilities:

```rust
let mut capabilities = vec![Capability::Chat, Capability::Completion, Capability::FunctionCalling];
```

This matches what all other tool-supporting providers (OpenAI, Anthropic, Groq, Mistral, etc.) already do.

### 3. Add `model_has_capability()` to ProviderRegistry

**File**: `crates/lr-providers/src/registry.rs`

Add a new method (after `get_all_cached_models_instant()`):

```rust
/// Check if a cached model has a specific capability (no I/O).
/// Returns None if provider/model not in cache (e.g., startup).
pub fn model_has_capability(
    &self,
    instance_name: &str,
    model_id: &str,
    capability: &Capability,
) -> Option<bool> {
    let cache = self.model_cache.read();
    let cached = cache.get(instance_name)?;
    let model = cached.models.iter().find(|m| m.id == model_id)?;
    Some(model.capabilities.contains(capability))
}
```

Add `Capability` to imports: `use super::{Capability, ModelInfo, ModelProvider, ProviderHealth};`

### 4. Add capability filtering in chat auto-routing

**File**: `crates/lr-router/src/lib.rs`

**Add `Capability` import** to the `lr_providers` use statement.

**In `complete_with_auto_routing()` and `stream_complete_with_auto_routing()`**:

Before the model loop, compute:
```rust
let request_has_tools = request.tools.as_ref().is_some_and(|t| !t.is_empty());
```

Inside the loop, after the backoff check and before the free-tier check, add:
```rust
if request_has_tools {
    match self.provider_registry.model_has_capability(
        provider, model, &Capability::FunctionCalling,
    ) {
        Some(false) => {
            debug!("Skipping {}/{}: no function calling support (request has tools)", provider, model);
            continue;
        }
        Some(true) => {}
        None => {
            // Not in cache yet (e.g., first request after startup) — be permissive
            debug!("Capability unknown for {}/{} (not in cache), allowing", provider, model);
        }
    }
}
```

**Only applies to chat completion auto-routing** (the two functions above). Embeddings/audio auto-routing don't involve tools, so no changes there. Direct model requests (non-auto) are also unchanged — if a user explicitly targets a model, the provider returns its own error.

## Verification

1. `cargo check -p lr-providers -p lr-router` — compilation
2. `cargo test -p lr-providers -p lr-router` — existing tests pass
3. `cargo clippy -p lr-providers -p lr-router` — no warnings
4. Manual test: send a request with tools to `localrouter/auto` where a non-tool-supporting model is first in priority — verify it's skipped and a tool-supporting model handles the request
