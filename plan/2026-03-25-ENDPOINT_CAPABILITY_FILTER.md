# Endpoint-Aware Model Capability Filtering for Auto-Routing

## Context

When auto-routing (`localrouter/auto`), the `prioritized_models` list may contain models of mixed types (chat, embedding, audio, TTS, image). When an embedding request arrives, the router tries chat models — they fail with `"Provider 'X' does not support embeddings"` (from the `ModelProvider` trait default), classified as `RouterError::Other` (non-retryable), causing the auto-routing loop to **fail immediately** instead of skipping to the next model.

**Goal**: Gracefully skip incompatible models during auto-routing. Each provider is responsible for tagging its models with capabilities. Models with unknown capabilities are tried for all endpoint types — incompatibility is only discovered at runtime and cached.

---

## Design Principles

1. **Providers own capability tagging** — each provider sets capabilities on its models based on its API data
2. **Unknown = try everything** — models without capability info are assumed to support all endpoints; failures are cached
3. **Catalog supplements** — uses `family` field (e.g., `"text-embedding"`) only; no guessing
4. **Negative cache** — when a model fails for an endpoint, cache that fact with TTL so it's not retried

---

## Part 1: Extend catalog to preserve endpoint capabilities from models.dev

### 1a. Add endpoint capability fields to `CatalogCapabilities`

**File**: `crates/lr-catalog/src/types.rs`

```rust
pub struct CatalogCapabilities {
    pub reasoning: bool,
    pub tool_call: bool,
    pub structured_output: bool,
    pub vision: bool,
    // NEW — derived from models.dev `family` and `modalities` at build time
    pub embedding: bool,       // family == "text-embedding"
    pub audio_input: bool,     // "audio" in modalities.input
    pub audio_output: bool,    // "audio" in modalities.output
    pub image_output: bool,    // "image" in modalities.output
}
```

Default: all `false`.

### 1b. Derive in codegen

**File**: `crates/lr-catalog/buildtools/codegen.rs`

In `generate_model_entry()`:
- `embedding`: `model.model.family.as_deref().map_or(false, |f| f.contains("embed"))` — catches `text-embedding` (22), `cohere-embed` (7), `mistral-embed` (2), `gemini-embedding` (1), `titan-embed` (1), `codestral-embed` (1) = 34 models. Family field only — no model name matching.
- `audio_input`: `model.model.modalities.input.contains("audio") && !model.model.modalities.input.contains("text")` — pure audio input only (whisper-like). Multimodal models that accept audio AND text are NOT audio-only.
- `audio_output`: `model.model.modalities.output.contains("audio") && !model.model.modalities.output.contains("text")` — pure TTS only. Omni models that output both text+audio are NOT TTS-only.
- `image_output`: `model.model.modalities.output.contains("image") && !model.model.modalities.output.contains("text")` — pure image gen only.

**Important**: OpenAI's tts-1, whisper-1, and Groq's whisper models are **NOT in models.dev at all**. TogetherAI is also absent. These must be tagged by their respective providers (Part 2d).

### 1c. Expose `family` field in `ModelsDevModel`

**File**: `crates/lr-catalog/buildtools/models.rs`

Already parsed as `pub family: Option<String>`. No change needed.

---

## Part 2: Rethink `ModelInfo.capabilities` — providers own tagging

The key insight: capabilities should be **per-model**, set by the provider. Models with no explicit capability info should NOT be assumed to be chat-only — they should be treated as "unknown" (try for everything).

### 2a. Make empty capabilities mean "unknown"

**Convention**: `ModelInfo.capabilities == []` means "unknown — try for all endpoint types." Non-empty means "only supports these."

This is already the semantic — we just need to ensure:
- Providers that know capabilities set them explicitly
- The router interprets empty as "compatible with everything"

### 2b. Update `enrich_with_catalog()` and `enrich_with_catalog_by_name()`

**File**: `crates/lr-providers/src/lib.rs` (~line 749)

After existing vision enrichment, add catalog-derived capabilities:
```rust
if catalog_model.capabilities.embedding && !self.capabilities.contains(&Capability::Embedding) {
    self.capabilities.push(Capability::Embedding);
}
if catalog_model.capabilities.audio_input && !self.capabilities.contains(&Capability::Audio) {
    self.capabilities.push(Capability::Audio);
}
if catalog_model.capabilities.audio_output && !self.capabilities.contains(&Capability::TextToSpeech) {
    self.capabilities.push(Capability::TextToSpeech);
}
```

### 2c. Update `get_models_from_catalog()` in registry

**File**: `crates/lr-providers/src/registry.rs` (~line 643)

Replace the simple modality match with proper capability mapping:
```rust
let mut capabilities = Vec::new();
if m.capabilities.embedding {
    capabilities.push(Capability::Embedding);
}
if m.capabilities.audio_input {
    capabilities.push(Capability::Audio);
}
if m.capabilities.audio_output {
    capabilities.push(Capability::TextToSpeech);
}
if m.modality == lr_catalog::Modality::Image || m.capabilities.image_output {
    // image generation model — don't add Chat
} else if !m.capabilities.embedding {
    // chat model (or unknown — add Chat as default for non-embedding, non-image models)
    capabilities.push(Capability::Chat);
    if m.modality == lr_catalog::Modality::Multimodal {
        capabilities.push(Capability::Vision);
    }
}
```

### 2d. Update providers with explicit capability APIs

Each provider is responsible for setting capabilities on models it returns.

**Cohere** (`cohere.rs`): Already sets `Capability::Embedding` from `endpoints` array — no change needed. Best example of correct behavior.

**TogetherAI** (`togetherai.rs`): Has `type` field in API response. Currently filters to `type == "chat"` only. **Change**: Include all model types with capabilities based on `type`:
- `"chat"` → `[Chat, FunctionCalling]` (existing)
- `"embedding"` → `[Embedding]`
- `"image"` → `[]` (or new image capability)
- `"audio"` → `[Audio]`
- unknown/missing → `[]` (try everything)
- **Note**: TogetherAI is NOT in models.dev, so provider tagging is the ONLY source.

**Gemini** (`gemini.rs`): Has `supportedGenerationMethods` array. Currently filters to `generateContent` only. **Change**: Also include models with `embedContent` → `[Embedding]`. Google TTS models (output=audio) exist in catalog but need `supportedGenerationMethods` check too.

**Mistral** (`mistral.rs`): Has `capabilities` object. Currently only parses `function_calling`. **Change**: Check for additional fields if the API provides them.

**Groq** (`groq.rs`): No type field in API. Currently filters OUT whisper/distil by model ID. **Change**: Include whisper models with `[Audio]` capability (Groq whisper NOT in models.dev catalog). Keep distil models filtered if they're not useful.

**DeepInfra** (`deepinfra.rs`): No type field in API. Currently filters OUT embed/whisper by model ID. **Change**: Include them — models with "embed" in ID → `[Embedding]`, "whisper" in ID → `[Audio]`. Others → `[Chat]`. (DeepInfra has some of these in the catalog, but not all.)

**xAI** (`xai.rs`): No type field in API. Currently filters OUT embedding/image by model ID. **Change**: Include them with `[Embedding]` or appropriate capability.

**OpenAI** (`openai.rs`): No type field in API. Currently filters to `gpt-*`, `o1-*`, `text-*` patterns. **Change**: Stop filtering out non-chat models and instead include them with proper capabilities. Use only unambiguous prefixes for dedicated single-purpose models (these are NOT in models.dev, so provider tagging is essential):
- `text-embedding-` prefix → `[Embedding]` (text-embedding-3-small, text-embedding-3-large, text-embedding-ada-002)
- `whisper-` prefix → `[Audio]` (whisper-1)
- `tts-` prefix → `[TextToSpeech]` (tts-1, tts-1-hd)
- `dall-e-` prefix → `[]` (dall-e-2, dall-e-3 — image gen, empty caps = try everything for now)
- Everything else → existing behavior (`[Chat, Completion]` etc.)

**Do NOT use suffix matching** (e.g., `-tts`, `-embed`) — a model like `gpt-4o-audio-preview` could have misleading suffixes. Only match on prefixes that are unambiguously single-purpose model families.

**All other providers** (Anthropic, Ollama, OpenRouter, OpenAI-compatible, etc.): Keep existing behavior + catalog enrichment fills in capabilities.

### 2e. Removed — provider-specific handling covers all cases

Each provider handles its own models (2d above). OpenAI tags tts-*/whisper-*/dall-e-*/text-embedding-* directly. Groq/DeepInfra/xAI tag their whisper/embed models directly. Catalog enrichment (2b) covers the rest. No generic fallback needed.

---

## Part 3: `EndpointType` enum and compatibility checking

### 3a. Define `EndpointType`

**File**: `crates/lr-providers/src/lib.rs`

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EndpointType {
    Chat,
    Embedding,
    Transcription,
    Translation,
    Speech,
    ImageGeneration,
}

impl EndpointType {
    /// Check if a model's capabilities indicate support for this endpoint.
    /// Returns true if compatible OR if capabilities are unknown (empty).
    pub fn is_compatible_with(&self, capabilities: &[Capability]) -> bool {
        if capabilities.is_empty() {
            return true; // Unknown capabilities — try everything
        }
        match self {
            EndpointType::Chat => capabilities.contains(&Capability::Chat)
                || capabilities.contains(&Capability::Completion),
            EndpointType::Embedding => capabilities.contains(&Capability::Embedding),
            EndpointType::Transcription | EndpointType::Translation => {
                capabilities.contains(&Capability::Audio)
            }
            EndpointType::Speech => capabilities.contains(&Capability::TextToSpeech),
            EndpointType::ImageGeneration => false, // TODO: add ImageGeneration capability
        }
    }

    /// Check provider-level support
    pub fn is_supported_by_provider(&self, provider: &dyn ModelProvider) -> bool {
        match self {
            EndpointType::Chat => true,
            EndpointType::Embedding => provider.supports_embeddings(),
            EndpointType::Transcription => provider.supports_transcription(),
            EndpointType::Translation => provider.supports_audio_translation(),
            EndpointType::Speech => provider.supports_speech(),
            EndpointType::ImageGeneration => provider.supports_image_generation(),
        }
    }
}
```

### 3b. Add model capability lookup to `ProviderRegistry`

**File**: `crates/lr-providers/src/registry.rs`

```rust
/// Look up a model's capabilities from cached model list, falling back to catalog.
/// Returns None if model not found anywhere (treat as unknown = try everything).
pub fn get_model_capabilities(&self, instance_name: &str, model_id: &str) -> Option<Vec<Capability>> {
    // Try cached model list first
    let cache = self.model_cache.read();
    if let Some(mc) = cache.get(instance_name) {
        if let Some(model) = mc.models.iter().find(|m| m.id == model_id) {
            return Some(model.capabilities.clone());
        }
    }
    drop(cache);

    // Fall back to catalog
    let provider_type = self.get_provider_type_for_instance(instance_name)?;
    let catalog_model = lr_catalog::find_model(&provider_type, model_id)
        .or_else(|| lr_catalog::find_model_by_name(model_id))?;

    let mut caps = Vec::new();
    if catalog_model.capabilities.embedding {
        caps.push(Capability::Embedding);
    } else {
        caps.push(Capability::Chat);
        if catalog_model.modality == lr_catalog::Modality::Multimodal {
            caps.push(Capability::Vision);
        }
    }
    if catalog_model.capabilities.audio_input {
        caps.push(Capability::Audio);
    }
    if catalog_model.capabilities.audio_output {
        caps.push(Capability::TextToSpeech);
    }
    Some(caps)
}
```

---

## Part 4: Negative capability cache for runtime discovery

### 4a. Create `EndpointCapabilityCache`

**New file**: `crates/lr-router/src/endpoint_cache.rs`

DashMap-based cache: `(provider, model, EndpointType) → Instant`. TTL: 1 hour. Methods: `record_unsupported()`, `is_known_unsupported()`, `cleanup_expired()`.

### 4b. Add `EndpointNotSupported` to `RouterError`

**File**: `crates/lr-router/src/lib.rs`

New retryable variant:
```rust
EndpointNotSupported { provider: String, model: String, endpoint: String }
```

Update `should_retry()` to include it.

Update `classify()`: match against our own trait default error messages. These 5 exact messages are defined in `crates/lr-providers/src/lib.rs` (lines 192-299) and are the ONLY source of these strings:
- `"does not support embeddings"` (line 193)
- `"does not support image generation"` (line 209)
- `"does not support audio transcription"` (line 270)
- `"does not support audio translation"` (line 286)
- `"does not support text-to-speech"` (line 299)

This is safe because we control both the source (trait defaults) and the consumer (classify). If we ever change the messages, both sides update together.

Update `to_log_string()`.

---

## Part 5: Integrate into auto-routing loops

### 5a. Add `should_skip_for_endpoint()` helper on Router

**File**: `crates/lr-router/src/lib.rs`

```rust
fn should_skip_for_endpoint(&self, provider: &str, model: &str, endpoint: EndpointType) -> bool {
    // Layer 1: Negative cache (previous runtime failures)
    if self.endpoint_cache.is_known_unsupported(provider, model, endpoint) {
        return true;
    }
    // Layer 2: Provider-level support
    if let Some(provider_instance) = self.provider_registry.get_provider(provider) {
        if !endpoint.is_supported_by_provider(provider_instance.as_ref()) {
            self.endpoint_cache.record_unsupported(provider, model, endpoint);
            return true;
        }
    }
    // Layer 3: Model capability lookup (registry cache → catalog → None)
    if let Some(capabilities) = self.provider_registry.get_model_capabilities(provider, model) {
        if !endpoint.is_compatible_with(&capabilities) {
            return true; // Known incompatible; don't cache since capabilities may refresh
        }
    }
    // Model not found or capabilities unknown → try it
    false
}
```

### 5b. Insert into auto-routing loops

**`embed_with_auto_routing`** (~line 1859): after backoff check, add:
```rust
if self.should_skip_for_endpoint(provider, model, EndpointType::Embedding) { continue; }
```

**`complete_with_auto_routing`** (~line 994): after backoff check:
```rust
if self.should_skip_for_endpoint(provider, model, EndpointType::Chat) { continue; }
```

**`stream_complete_with_auto_routing`** (~line 1181): after backoff check:
```rust
if self.should_skip_for_endpoint(provider, model, EndpointType::Chat) { continue; }
```

In all three error handlers: when `EndpointNotSupported` is received, call `endpoint_cache.record_unsupported(...)`.

### 5c. Add `endpoint_cache` field to Router struct

Initialize in both `new()` and `new_without_free_tier()` constructors with 1-hour TTL.

---

## Files to modify

| File | Changes |
|------|---------|
| `crates/lr-catalog/src/types.rs` | Add `embedding`, `audio_input`, `audio_output`, `image_output` to `CatalogCapabilities` |
| `crates/lr-catalog/buildtools/codegen.rs` | Derive new bools from `family` field and `modalities` |
| `crates/lr-providers/src/lib.rs` | `EndpointType` enum; update `enrich_with_catalog()`; add single-purpose model fallback detection |
| `crates/lr-providers/src/registry.rs` | `get_model_capabilities()`; update `get_models_from_catalog()` |
| `crates/lr-providers/src/togetherai.rs` | Include non-chat models with proper capabilities |
| `crates/lr-providers/src/gemini.rs` | Parse `supportedGenerationMethods` for `embedContent` |
| `crates/lr-providers/src/groq.rs` | Include whisper models with `[Audio]` capability |
| `crates/lr-providers/src/deepinfra.rs` | Include embed/whisper models with proper capabilities |
| `crates/lr-providers/src/xai.rs` | Include embedding/image models with proper capabilities |
| `crates/lr-providers/src/mistral.rs` | Parse additional capability fields if available |
| `crates/lr-router/src/lib.rs` | `RouterError::EndpointNotSupported`; `should_skip_for_endpoint()`; Router struct + constructors; 3 auto-routing loops |
| `crates/lr-router/src/endpoint_cache.rs` | **New file** — `EndpointCapabilityCache` |

## Verification

1. `LOCALROUTER_REBUILD_CATALOG=1 cargo check --package lr-catalog --package lr-providers --package lr-router`
2. `cargo test --package lr-catalog` — verify embedding models get `embedding: true` from `family` field
3. `cargo test --package lr-providers` — provider tests pass, capability tagging correct
4. `cargo test --package lr-router` — router tests pass, skip logic works
5. `cargo clippy --package lr-catalog --package lr-providers --package lr-router`
6. Manual: auto-routing with mixed models → embedding request skips chat models, succeeds with embedding model

## Mandatory final steps

1. **Plan Review**: Review plan vs implementation for missed items
2. **Test Coverage Review**: Ensure all new code paths have tests
3. **Bug Hunt**: Re-read for off-by-one, race conditions, missing error handling
4. **Commit**: Stage only modified files
