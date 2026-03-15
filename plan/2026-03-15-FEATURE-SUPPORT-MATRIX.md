# Feature Support Matrix UI

## Context

LocalRouter has many cross-cutting features (guardrails, compression, secret scanning, JSON repair, RouteLLM, etc.) and endpoint capabilities (chat, embeddings, images, audio, responses, etc.) that vary across providers and client modes. Currently there is no unified view showing what is supported where. Users need to understand:

1. **Per-provider**: What endpoints and features does this provider support? (Provider page → Info tab)
2. **Per-feature**: Which features apply to which client modes and endpoints? (Optimize page → new Support Matrix section)

This plan adds two interconnected matrices with hover tooltips explaining support levels.

## UI Design

### Matrix A: Provider Feature Matrix (Provider Page → Info tab)

**Location:** `src/views/resources/providers-panel.tsx` — existing Info tab, below the Health Status card.

**Layout:** A table/grid showing feature support for the selected provider.

```
┌─────────────────────────────────────────────────────────┐
│  Provider: OpenAI                                       │
│                                                         │
│  ┌─ Health Status ──────────────────────────────────┐   │
│  │ Healthy • 45ms                                   │   │
│  └──────────────────────────────────────────────────┘   │
│                                                         │
│  ┌─ Feature Support ────────────────────────────────┐   │
│  │                                                  │   │
│  │  API Endpoints                                   │   │
│  │  ┌──────────────────────┬──────────┐             │   │
│  │  │ Chat Completions     │ ✓        │             │   │
│  │  │ Completions (legacy) │ ✓        │             │   │
│  │  │ Embeddings           │ ✓        │             │   │
│  │  │ Image Generation     │ ✓        │             │   │
│  │  │ Audio Transcription  │ —        │             │   │
│  │  │ Audio Speech (TTS)   │ —        │             │   │
│  │  │ Moderations          │ —        │             │   │
│  │  │ Responses API        │ —        │             │   │
│  │  │ Batch Processing     │ —        │             │   │
│  │  │ Realtime (WebSocket) │ —        │             │   │
│  │  └──────────────────────┴──────────┘             │   │
│  │                                                  │   │
│  │  Model Features                                  │   │
│  │  ┌──────────────────────┬──────────┐             │   │
│  │  │ Streaming            │ ✓        │             │   │
│  │  │ Function Calling     │ ✓        │             │   │
│  │  │ Vision               │ ✓        │             │   │
│  │  │ Structured Outputs   │ ✓        │             │   │
│  │  │ JSON Mode            │ ✓        │             │   │
│  │  │ Log Probabilities    │ ✓        │             │   │
│  │  │ Reasoning Tokens     │ Partial  │ ← hover    │   │
│  │  │ Prompt Caching       │ —        │             │   │
│  │  │ Extended Thinking    │ —        │             │   │
│  │  └──────────────────────┴──────────┘             │   │
│  │                                                  │   │
│  │  Optimization Features                           │   │
│  │  ┌──────────────────────┬──────────┐             │   │
│  │  │ Guardrails           │ ✓        │             │   │
│  │  │ Prompt Compression   │ ✓        │             │   │
│  │  │ JSON Repair          │ ✓        │             │   │
│  │  │ RouteLLM Routing     │ ✓        │             │   │
│  │  │ Secret Scanning      │ ✓        │             │   │
│  │  └──────────────────────┴──────────┘             │   │
│  └──────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────┘
```

**Support Levels with colors:**
- `Supported` (green check) — Full native support
- `Partial` (yellow half-circle) — Some models/features, hover for details
- `Translated` (blue arrows) — Supported via LocalRouter translation layer
- `Not Supported` (gray dash) — Not available
- `Not Yet Implemented` (outline dash) — Planned but not built yet

**Hover tooltips** show a short explanation, e.g.:
- "Partial: Reasoning tokens supported for o1-preview and o1-mini models only"
- "Translated: Responses API requests are converted to chat completions by LocalRouter"
- "Not Yet Implemented: Audio endpoints are planned — see plan/2026-03-15-API-AUDIO.md"

### Matrix B: Feature × Client Mode × Endpoint Matrix (Optimize Page)

**Location:** `src/views/optimize-overview/index.tsx` — new card/section at the bottom of the page.

**Layout:** A matrix showing which optimization features apply to which endpoints, broken down by client mode.

```
┌─ Feature Support Matrix ──────────────────────────────────────────────┐
│                                                                       │
│  Shows which optimization features apply to each endpoint and mode.   │
│  Hover for details.                                                   │
│                                                                       │
│             │ Chat  │ Compl │ Embed │ Image │ Audio │ Mod  │ Resp │  │
│  ───────────┼───────┼───────┼───────┼───────┼───────┼──────┼──────┤  │
│  Guardrails │  ✓    │  ✓    │  —    │  —    │  —    │  —   │  ✓*  │  │
│  Compress   │  ✓    │  —    │  —    │  —    │  —    │  —   │  ✓*  │  │
│  JSON Repair│  ✓    │  ✓    │  —    │  —    │  —    │  —   │  ✓*  │  │
│  RouteLLM   │  ✓    │  —    │  —    │  —    │  —    │  —   │  —   │  │
│  SecretScan │  ✓    │  ✓    │  —    │  —    │  P    │  —   │  ✓*  │  │
│  Rate Limit │  ✓    │  ✓    │  ✓    │  ✓    │  ✓    │  ✓   │  ✓   │  │
│  Firewall   │  ✓    │  ✓    │  —    │  —    │  ✓    │  —   │  ✓   │  │
│  Tracking   │  ✓    │  ✓    │  ✓    │  ✓    │  ✓    │  ✓   │  ✓   │  │
│  Cost Calc  │  ✓    │  ✓    │  ✓    │  —    │  ✓    │  —   │  ✓   │  │
│  ───────────┼───────┼───────┼───────┼───────┼───────┼──────┼──────┤  │
│                                                                       │
│  ✓ = Supported  P = Partial  ✓* = Via translation  — = N/A           │
│                                                                       │
│  ┌─ Client Mode Compatibility ────────────────────────────────────┐   │
│  │                                                                │   │
│  │           │ LLM Only │ MCP Only │ MCP & LLM │ MCP via LLM │   │   │
│  │  ─────────┼──────────┼──────────┼───────────┼─────────────┤   │   │
│  │  Chat     │  ✓       │  —       │  ✓        │  ✓          │   │   │
│  │  Compl    │  ✓       │  —       │  ✓        │  —          │   │   │
│  │  Embed    │  ✓       │  —       │  ✓        │  —          │   │   │
│  │  Images   │  ✓       │  —       │  ✓        │  —          │   │   │
│  │  Audio    │  ✓       │  —       │  ✓        │  —          │   │   │
│  │  Mod      │  ✓       │  —       │  ✓        │  —          │   │   │
│  │  Resp     │  ✓       │  —       │  ✓        │  —          │   │   │
│  │  MCP GW   │  —       │  ✓       │  ✓        │  —          │   │   │
│  │  MCP WS   │  —       │  ✓       │  ✓        │  —          │   │   │
│  │  MCP→LLM  │  —       │  —       │  —        │  ✓          │   │   │
│  │  Guardrail│  ✓       │  —       │  ✓        │  ✓          │   │   │
│  │  Compress │  ✓       │  —       │  ✓        │  ✓          │   │   │
│  │  RouteLLM │  ✓       │  —       │  ✓        │  ✓          │   │   │
│  │  SecretSc │  ✓       │  —       │  ✓        │  ✓          │   │   │
│  │  JSON Rep │  ✓       │  —       │  ✓        │  ✓          │   │   │
│  │  Ctx Mgmt │  —       │  ✓       │  ✓        │  ✓          │   │   │
│  │  Catalog  │  —       │  ✓       │  ✓        │  ✓          │   │   │
│  │  Resp RAG │  —       │  ✓       │  ✓        │  ✓          │   │   │
│  │  ─────────┴──────────┴──────────┴───────────┴─────────────┤   │   │
│  └────────────────────────────────────────────────────────────┘   │
└───────────────────────────────────────────────────────────────────────┘
```

## Data Model

### Backend: Provider Feature Support

Add a new Tauri command that returns feature support for a provider, computed from the provider trait and registry.

**New type in `crates/lr-providers/src/lib.rs`:**
```rust
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ProviderFeatureSupport {
    pub provider_type: String,
    pub provider_instance: String,

    // API Endpoints
    pub endpoints: Vec<EndpointSupport>,

    // Model Features (from feature adapters + capabilities)
    pub model_features: Vec<FeatureSupport>,

    // Optimization Features (which LocalRouter features work with this provider)
    pub optimization_features: Vec<FeatureSupport>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct EndpointSupport {
    pub name: String,           // "Chat Completions", "Audio Transcription", etc.
    pub endpoint: String,       // "/v1/chat/completions"
    pub support: SupportLevel,
    pub notes: Option<String>,  // Hover tooltip text
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct FeatureSupport {
    pub name: String,           // "Guardrails", "Extended Thinking", etc.
    pub support: SupportLevel,
    pub notes: Option<String>,  // Hover tooltip text
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SupportLevel {
    Supported,        // Full native support
    Partial,          // Some models/configurations
    Translated,       // Via LocalRouter translation layer
    NotSupported,     // Not available for this provider
    NotImplemented,   // Planned, not built yet
}
```

### Backend: Feature × Endpoint × Mode Matrix

This is static data — doesn't change per provider. Can be a hardcoded Tauri command or generated at build time.

**New type:**
```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeatureEndpointMatrix {
    pub endpoints: Vec<String>,                    // Column headers
    pub client_modes: Vec<String>,                 // Column headers for mode matrix
    pub feature_rows: Vec<FeatureEndpointRow>,     // Feature × Endpoint
    pub mode_rows: Vec<FeatureModeRow>,            // Feature/Endpoint × ClientMode
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeatureEndpointRow {
    pub feature_name: String,
    pub cells: Vec<MatrixCell>,  // One per endpoint
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeatureModeRow {
    pub name: String,
    pub cells: Vec<MatrixCell>,  // One per client mode
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatrixCell {
    pub support: SupportLevel,
    pub notes: Option<String>,
}
```

### Provider Feature Detection

**How to determine support per provider:**

#### API Endpoints
Determined by checking which trait methods the provider overrides (not default 501):
- **Chat/Completions/Streaming**: All providers → `Supported`
- **Embeddings**: Check `embed()` override → `Supported` or `NotSupported`
- **Image Generation**: Check `generate_image()` override → `Supported` or `NotSupported`
- **Audio STT/TTS**: Check `transcribe()`/`speech()` override (after implementation) → based on `plan/2026-03-15-PROVIDER-ENDPOINT-COVERAGE.md`
- **Moderations**: Only OpenAI → `Supported`; others → `NotSupported`
- **Responses API**: OpenAI/Groq/xAI → `Supported`; all chat-capable → `Translated` (after implementation); now → `NotImplemented`
- **Batches**: OpenAI/Groq/TogetherAI → `Supported`; all others → `Translated` (after implementation); now → `NotImplemented`
- **Realtime**: OpenAI/TogetherAI/LocalAI → `Supported`; others → `NotSupported`; now → `NotImplemented`

#### Model Features
Determined by `supports_feature()` and `get_feature_adapter()`:
- **Function Calling**: Check `Capability::FunctionCalling` in model capabilities
- **Vision**: Check `Capability::Vision`
- **Structured Outputs**: Check `supports_feature("structured_outputs")`
- **JSON Mode**: Check `supports_feature("json_mode")`
- **Logprobs**: Check `supports_feature("logprobs")`
- **Reasoning Tokens**: Check `supports_feature("reasoning_tokens")` — `Partial` with note "o1 series only"
- **Extended Thinking**: Check `supports_feature("extended_thinking")` — `Partial` with note "Claude 4.5 models only"
- **Thinking Level**: Check `supports_feature("thinking_level")`
- **Prompt Caching**: Check `supports_feature("prompt_caching")`

#### Optimization Features
These are LocalRouter features, not provider features. Support depends on the endpoint type, not the provider. Every provider that supports an endpoint gets the same optimization features for that endpoint.
- **Guardrails**: `Supported` if provider supports chat/completions
- **Prompt Compression**: `Supported` if provider supports chat
- **JSON Repair**: `Supported` if provider supports chat/completions with JSON response format
- **RouteLLM**: `Supported` if provider supports chat (and is in a strategy with auto-routing)
- **Secret Scanning**: `Supported` if provider supports chat/completions
- **Rate Limiting**: `Supported` for all endpoints
- **Model Firewall**: `Supported` for all LLM endpoints
- **Generation Tracking**: `Supported` for all LLM endpoints
- **Cost Calculation**: `Supported` if pricing data available

### New Trait Method

Add to `ModelProvider` trait in `crates/lr-providers/src/lib.rs`:
```rust
/// Returns feature support information for this provider.
/// Default implementation uses supports_feature() and capability checks.
fn get_feature_support(&self) -> ProviderFeatureSupport {
    // Default implementation that checks:
    // - embed() override via a dedicated method
    // - generate_image() override via a dedicated method
    // - supports_feature() for each known feature
    // - model capabilities from list_models()
    // Providers can override for custom support levels/notes
}
```

Also add simple capability methods:
```rust
/// Whether this provider supports embeddings
fn supports_embeddings(&self) -> bool { false }

/// Whether this provider supports image generation
fn supports_image_generation(&self) -> bool { false }
```

Each provider implementation sets these to `true` as appropriate, which is cleaner than trying to detect whether `embed()` has been overridden.

## Files to Create

### Backend
- No new crate needed — types go in `crates/lr-providers/src/lib.rs`

### Frontend
- `src/components/shared/feature-support-matrix.tsx` — Reusable matrix grid component with hover tooltips
- `src/components/shared/support-level-badge.tsx` — Badge component for support levels (colored icon + label)

## Files to Modify

### Backend
- `crates/lr-providers/src/lib.rs` — `SupportLevel` enum, `ProviderFeatureSupport` struct, `EndpointSupport`, `FeatureSupport` types. New trait methods: `get_feature_support()`, `supports_embeddings()`, `supports_image_generation()`. Hardcoded feature-endpoint-mode matrix data.
- `crates/lr-providers/src/openai.rs` — Override `supports_embeddings() → true`, `supports_image_generation() → true`. Override `get_feature_support()` with OpenAI-specific notes (reasoning tokens partial, etc.).
- `crates/lr-providers/src/anthropic.rs` — Override `get_feature_support()` with extended thinking partial notes.
- `crates/lr-providers/src/gemini.rs` — Override with thinking_level, web_grounding notes.
- `crates/lr-providers/src/groq.rs` — Override `supports_embeddings()`.
- `crates/lr-providers/src/cohere.rs` — Override `supports_embeddings()`.
- `crates/lr-providers/src/togetherai.rs` — Override `supports_embeddings() → true`, `supports_image_generation() → true`.
- `crates/lr-providers/src/deepinfra.rs` — Override `supports_image_generation() → true`.
- `crates/lr-providers/src/ollama.rs` — Override `supports_embeddings() → true`.
- `crates/lr-providers/src/openai_compatible.rs` — Return `Partial` for most features with note "depends on upstream server".
- `crates/lr-providers/src/openrouter.rs` — Override `supports_embeddings() → true`.
- All other providers: defaults return `NotSupported` for optional endpoints.
- `src-tauri/src/ui/commands_providers.rs` — New Tauri command: `get_provider_feature_support(instance_name: String) -> ProviderFeatureSupport`. New Tauri command: `get_feature_endpoint_matrix() -> FeatureEndpointMatrix`.
- `src-tauri/src/lib.rs` — Register new commands in `tauri::generate_handler![]`.
- `src/types/tauri-commands.ts` — TypeScript types for `ProviderFeatureSupport`, `SupportLevel`, `EndpointSupport`, `FeatureSupport`, `FeatureEndpointMatrix`.
- `website/src/components/demo/TauriMockSetup.ts` — Mock data for both commands.

### Frontend
- `src/views/resources/providers-panel.tsx` — Add Feature Support card to Info tab (below Health Status card). Call `get_provider_feature_support` when provider selected. Render using `FeatureSupportMatrix` component.
- `src/views/optimize-overview/index.tsx` — Add Feature Support Matrix section at bottom. Call `get_feature_endpoint_matrix` on load. Render using `FeatureSupportMatrix` component.

## Complete Feature Inventory

### API Endpoints (per-provider, Matrix A)

| Feature | Currently Impl? | Notes |
|---------|----------------|-------|
| Chat Completions | Yes | All providers |
| Completions (legacy) | Yes | All providers (converted to chat) |
| Streaming | Yes | All providers |
| Embeddings | Yes | OpenAI, Gemini, Cohere, TogetherAI, OpenRouter, Ollama, OpenAI-compat |
| Image Generation | Yes | OpenAI, Gemini, DeepInfra, TogetherAI |
| Audio Transcription | Not yet | Planned: OpenAI, Groq, TogetherAI, DeepInfra |
| Audio Translation | Not yet | Planned: OpenAI, Groq, DeepInfra |
| Audio Speech (TTS) | Not yet | Planned: OpenAI, Groq, TogetherAI |
| Moderations | Not yet | Planned: OpenAI only |
| Responses API | Not yet | Planned: OpenAI, Groq, xAI (native); all (translated) |
| Batch Processing | Not yet | Planned: OpenAI, Groq, TogetherAI (native); all (translated) |
| Realtime (WebSocket) | Not yet | Planned: OpenAI, TogetherAI, LocalAI |

### Model Features (per-provider, Matrix A)

| Feature | Providers |
|---------|-----------|
| Function Calling | OpenAI, Anthropic, Gemini, Groq, Mistral, Cohere, xAI, OpenRouter, DeepInfra, TogetherAI |
| Vision | OpenAI, Anthropic, Gemini, Groq, xAI, OpenRouter, DeepInfra, TogetherAI, Ollama |
| Structured Outputs | OpenAI, Anthropic, Gemini |
| JSON Mode | OpenAI, Anthropic, Gemini, Cohere, Ollama |
| Log Probabilities | OpenAI |
| Reasoning Tokens | OpenAI (o1 series only → Partial) |
| Extended Thinking | Anthropic (Claude 4.5 only → Partial) |
| Thinking Level | Gemini (Gemini 3/2.0 → Partial) |
| Prompt Caching | Anthropic, OpenRouter |

### Optimization Features (per-endpoint, Matrix B)

| Feature | Chat | Completions | Embeddings | Images | Audio | Moderations | Responses | Batches | Realtime |
|---------|------|-------------|------------|--------|-------|-------------|-----------|---------|----------|
| Guardrails | ✓ | ✓ | — | — | — | — | ✓* | ✓** | — |
| Prompt Compression | ✓ | — | — | — | — | — | ✓* | ✓** | — |
| JSON Repair | ✓ | ✓ | — | — | — | — | ✓* | — | — |
| RouteLLM Routing | ✓ | — | — | — | — | — | — | — | — |
| Secret Scanning | ✓ | ✓ | — | — | P*** | — | ✓* | ✓** | — |
| Rate Limiting | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | P**** |
| Model Firewall | ✓ | ✓ | — | — | ✓ | — | ✓ | P***** | ✓ |
| Generation Tracking | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | P |
| Cost Calculation | ✓ | ✓ | ✓ | — | ✓ | — | ✓ | ✓ | ✓ |
| Context Mgmt | — | — | — | — | — | — | — | — | — |
| Catalog Compression | — | — | — | — | — | — | — | — | — |
| Response RAG | — | — | — | — | — | — | — | — | — |

`✓*` = Via translation layer (Responses → Chat Completions)
`✓**` = Per-request in translated batch mode
`P***` = TTS input text only; audio binary not scannable
`P****` = Connection-time only, no per-message
`P*****` = Approve at batch creation time
`—` = Not applicable

### Client Mode Compatibility (Matrix B)

| Endpoint/Feature | LLM Only | MCP Only | MCP & LLM | MCP via LLM |
|-----------------|----------|----------|-----------|-------------|
| Chat Completions | ✓ | — | ✓ | ✓ |
| Completions | ✓ | — | ✓ | — |
| Embeddings | ✓ | — | ✓ | — |
| Image Generation | ✓ | — | ✓ | — |
| Audio (STT/TTS) | ✓ | — | ✓ | — |
| Moderations | ✓ | — | ✓ | — |
| Responses API | ✓ | — | ✓ | — |
| Batch Processing | ✓ | — | ✓ | — |
| Realtime | ✓ | — | ✓ | — |
| MCP Gateway | — | ✓ | ✓ | — |
| MCP WebSocket | — | ✓ | ✓ | — |
| MCP → LLM Tools | — | — | — | ✓ |
| Guardrails | ✓ | — | ✓ | ✓ |
| Prompt Compression | ✓ | — | ✓ | ✓ |
| JSON Repair | ✓ | — | ✓ | ✓ |
| RouteLLM | ✓ | — | ✓ | ✓ |
| Secret Scanning | ✓ | — | ✓ | ✓ |
| Context Management | — | ✓ | ✓ | ✓ |
| Catalog Compression | — | ✓ | ✓ | ✓ |
| Response RAG | — | ✓ | ✓ | ✓ |

## Implementation Steps

### Step 1: Backend Types & Data
1. Add `SupportLevel`, `ProviderFeatureSupport`, `EndpointSupport`, `FeatureSupport` types to `crates/lr-providers/src/lib.rs`
2. Add `supports_embeddings()`, `supports_image_generation()` to `ModelProvider` trait
3. Add `get_feature_support()` default implementation to `ModelProvider` trait
4. Override `supports_embeddings()` / `supports_image_generation()` in each provider
5. Override `get_feature_support()` in providers with special notes (OpenAI, Anthropic, Gemini, OpenAI-compatible)
6. Add `FeatureEndpointMatrix` type and builder function (hardcoded static data)

### Step 2: Tauri Commands
1. Add `get_provider_feature_support` command
2. Add `get_feature_endpoint_matrix` command
3. Register in handler macro
4. Add TypeScript types
5. Add mock data for demo

### Step 3: Frontend Components
1. Create `SupportLevelBadge` component — icon + label + color for each SupportLevel
2. Create `FeatureSupportMatrix` component — reusable table with hover tooltips using existing Tooltip component
3. Wire up to provider Info tab
4. Wire up to Optimize overview page

### Step 4: Update for New API Plans
When each API plan from `2026-03-15-API-*.md` is implemented:
1. Update `get_feature_support()` in affected providers (change `NotImplemented` → `Supported`/`Translated`)
2. Update `get_feature_endpoint_matrix()` to add new endpoint columns
3. UI updates automatically (data-driven)

## Verification
1. `cargo test` — type serialization, feature detection
2. `cargo clippy && cargo fmt`
3. `npx tsc --noEmit` — TypeScript types
4. Open provider Info tab → verify Feature Support card with correct data per provider
5. Open Optimize page → verify Feature Support Matrix with hover tooltips
6. Verify tooltip text appears on hover for Partial/Translated/NotImplemented items
7. Verify demo mock works (TauriMockSetup.ts)
