# Plan: Add Moderation Endpoint Models to Guardrails

## Context

Currently, the guardrails system supports 5 safety model types, all either local (Llama Guard, ShieldGemma, Nemotron, Granite Guardian via Ollama) or OpenAI-specific (omni-moderation via `/v1/moderations`). Several cloud providers offer moderation capabilities that should be available as guardrails model options:

1. **Mistral** has a dedicated `/v1/moderations` endpoint with its own model (`mistral-moderation-latest`) and response format — requires a new model type
2. **DeepInfra, Groq, Together AI** host Llama Guard 4 12B as a cloud model — uses existing `llama_guard` type but needs cloud provider support
3. **OpenAI** moderation is already supported and free

There are two infrastructure gaps blocking cloud provider support:
- The guardrails executor only supports `/v1/completions` (legacy) and Ollama's `/api/generate` — cloud providers serve Llama Guard via `/v1/chat/completions`
- The provider lookup in `rebuild_safety_engine` doesn't resolve API keys from keychain or map cloud provider types correctly

## Changes

### 1. Add Chat Completions executor (`crates/lr-guardrails/src/executor.rs`)

Add `ChatCompletionExecutor` for cloud providers that serve models via `/v1/chat/completions`:

- New `ChatCompletionRequest` struct with `messages: Vec<ChatMessage>` instead of raw `prompt`
- New `ChatCompletionExecutor` struct (same pattern as `ProviderExecutor`: http_client, base_url, api_key, model_name)
- Calls `POST {base_url}/chat/completions` with `{"model": ..., "messages": [...], "max_tokens": 32, "temperature": 0}`
- Parses `choices[0].message.content` as output text
- Add `ChatProvider(ChatCompletionExecutor)` variant to `ModelExecutor` enum
- Add `chat_complete(&self, request: ChatCompletionRequest)` method on `ModelExecutor`

### 2. Update Llama Guard to support chat format (`crates/lr-guardrails/src/models/llama_guard.rs`)

- Add `build_chat_messages(&self, input: &SafetyCheckInput) -> Vec<(String, String)>` that returns `[(role, content)]` — system message with taxonomy + user message with conversation
- Update `check()` to detect executor type: if `ChatProvider`, use `chat_complete()` with messages; if `Provider`, use existing `complete()` with raw prompt
- The prompt content is the same — just formatted as chat messages vs raw text

### 3. Create Mistral Moderation model (`crates/lr-guardrails/src/models/mistral_moderation.rs` — new file)

Pattern follows `openai_moderation.rs`:

- **Category mappings** (Mistral → unified SafetyCategory):
  - `sexual` → `SexualContent`
  - `hate_and_discrimination` → `Hate`
  - `violence_and_threats` → `ViolentCrimes`
  - `dangerous_and_criminal_content` → `DangerousContent`
  - `selfharm` → `SelfHarm`
  - `health` → `SpecializedAdvice`
  - `financial` → `SpecializedAdvice`
  - `law` → `SpecializedAdvice`
  - `pii` → `Privacy`
- **MistralModerationExecutor**: calls `POST {base_url}/moderations` with `{"model": "mistral-moderation-latest", "input": [{"text": "..."}]}`
- **MistralModerationModel**: implements `SafetyModel`, `model_type_id() = "mistral_moderation"`, `inference_mode() = MultiCategory`
- **Response parsing**: similar to OpenAI — `results[].categories` (bool map) + `results[].category_scores` (f64 map)
- No new `SafetyCategory` variants needed (all map to existing ones)

### 4. Register in models module (`crates/lr-guardrails/src/models/mod.rs`)

- Add `pub mod mistral_moderation;`

### 5. Wire into engine (`crates/lr-guardrails/src/engine.rs`)

In `from_config()`:

- **Executor selection**: When building executor for `llama_guard`/`shield_gemma`/`nemotron`/`granite_guardian`, check `provider.provider_type`:
  - `"ollama"` → `ModelExecutor::Provider(ProviderExecutor::new(..., use_ollama=true))`
  - `"groq"` | `"deepinfra"` | `"togetherai"` | other cloud → `ModelExecutor::ChatProvider(ChatCompletionExecutor::new(...))`
  - Default (lmstudio, localai, etc.) → `ModelExecutor::Provider(ProviderExecutor::new(..., use_ollama=false))`
- **New match arm** for `"mistral_moderation"`:
  - Build `MistralModerationExecutor` from provider base_url + api_key
  - Construct `MistralModerationModel`

### 6. Fix provider lookup for cloud providers

**Both `src-tauri/src/main.rs` (~line 719) and `src-tauri/src/ui/commands.rs` (~line 3470)**:

a) **Provider type mapping** — expand to include all cloud types:
```
Groq => "groq", Mistral => "mistral", DeepInfra => "deepinfra",
TogetherAI => "togetherai", OpenAI => "openai", Anthropic => "anthropic",
Cohere => "cohere", OpenRouter => "openrouter", ...
```
Currently `commands.rs` only maps Ollama/LMStudio and falls back to `"openai_compatible"` — this loses the actual provider type needed for executor selection.

b) **Default endpoints for cloud providers** — add to the endpoint fallback match:
```
Groq => "https://api.groq.com/openai/v1"
DeepInfra => "https://api.deepinfra.com/v1/openai"
TogetherAI => "https://api.together.xyz/v1"
Mistral => "https://api.mistral.ai/v1"
OpenAI => "https://api.openai.com/v1"
```
Reuse the constants already defined in `crates/lr-providers/src/{groq,deepinfra,togetherai,mistral}.rs`.

c) **API key from keychain** — add fallback after the `provider_config.api_key` check:
```rust
.or_else(|| lr_providers::key_storage::get_provider_key(&p.name).ok().flatten())
```
The comment on line 758 of main.rs says "not keychain for safety models" — this was correct when only local providers were supported, but cloud providers store keys in keychain.

### 7. Frontend model mappings (`src/constants/safety-model-variants.ts`)

```typescript
// Add to MODEL_FAMILY_GROUPS:
{ family: "Mistral Moderation", modelType: "mistral_moderation" },

// Add cloud providers to PROVIDER_MODEL_NAMES.llama_guard:
deepinfra: "meta-llama/Llama-Guard-4-12B",
groq: "meta-llama/llama-guard-4-12b",
togetherai: "meta-llama/Llama-Guard-4-12B",

// Add new entry:
mistral_moderation: {
  mistral: "mistral-moderation-latest",
},

// Expand CLOUD_PROVIDER_TYPES:
new Set(["openai", "mistral", "deepinfra", "groq", "togetherai"])

// Add to CONFIDENCE_MODEL_TYPES:
"mistral_moderation"  // returns category_scores

// New pricing constant:
export const CLOUD_MODEL_PRICING: Record<string, Record<string, string>> = {
  openai_moderation: { openai: "Free" },
  mistral_moderation: { mistral: "~$0.10/1M tokens" },
  llama_guard: {
    deepinfra: "$0.18/1M tokens",
    groq: "$0.20/1M tokens",
    togetherai: "$0.20/1M tokens",
  },
}
```

### 8. Frontend pricing display (`src/components/guardrails/SafetyModelPicker.tsx`)

Import `CLOUD_MODEL_PRICING` and update the label display (currently hardcoded "Free" for all cloud):
```typescript
const pricing = CLOUD_MODEL_PRICING[entry.modelType]?.[entry.provider.provider_type]
const readyLabel = isCloudModel
  ? ` — ${entry.provider.instance_name} (${pricing || 'Cloud'})`
  : ` — Ready on ${entry.provider.instance_name}`
```

### 9. Demo mock (`website/src/components/demo/TauriMockSetup.ts`)

Add Mistral moderation and cloud Llama Guard examples to `get_guardrails_config` mock safety_models array.

## Files to Modify

| File | Change |
|------|--------|
| `crates/lr-guardrails/src/executor.rs` | Add `ChatCompletionExecutor`, `ChatProvider` variant |
| `crates/lr-guardrails/src/models/mistral_moderation.rs` | **New file** — Mistral moderation model + executor |
| `crates/lr-guardrails/src/models/mod.rs` | Add `pub mod mistral_moderation` |
| `crates/lr-guardrails/src/models/llama_guard.rs` | Add `build_chat_messages()`, update `check()` for chat executor |
| `crates/lr-guardrails/src/engine.rs` | Wire mistral_moderation type + chat executor selection |
| `src-tauri/src/main.rs` (~line 719) | Expand provider type mapping, default endpoints, keychain fallback |
| `src-tauri/src/ui/commands.rs` (~line 3470) | Same fixes as main.rs for `rebuild_safety_engine` |
| `src/constants/safety-model-variants.ts` | Add model families, provider mappings, pricing |
| `src/components/guardrails/SafetyModelPicker.tsx` | Pricing display |
| `website/src/components/demo/TauriMockSetup.ts` | Mock data |

## Implementation Order

1. Backend executor: `ChatCompletionExecutor` in `executor.rs`
2. Llama Guard chat support: `llama_guard.rs`
3. Mistral moderation: new file + `mod.rs`
4. Engine wiring: `engine.rs` (executor selection + mistral match arm)
5. Provider lookup fixes: `main.rs` + `commands.rs`
6. Frontend: `safety-model-variants.ts` + `SafetyModelPicker.tsx`
7. Demo mock: `TauriMockSetup.ts`

## Verification

1. `cargo test -p lr-guardrails` — unit tests for new model + executor
2. `cargo clippy` — lint check
3. `npx tsc --noEmit` — TypeScript type check
4. Manual: Add Mistral moderation model via UI, verify it appears in picker with pricing
5. Manual: Add Llama Guard via Groq/DeepInfra/Together AI, verify pricing display
6. Manual: Run "Try It Out" test against each new model type
