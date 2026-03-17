# Add llama.cpp as a Local Provider

## Context

LocalRouter now supports 5 local providers (Ollama, LM Studio, Jan, GPT4All, LocalAI). The user wants to add **llama.cpp** (llama-server) as a 6th local provider.

llama.cpp's server is OpenAI-compatible, runs on port **8080** by default, has a `/health` endpoint, and does **not** support model pulling at runtime (models are loaded at server startup). No API key is required by default.

**Note:** Port 8080 conflicts with LocalAI's default. We'll use 8080 as llama.cpp's default since that's what the project uses — users running both would configure different ports.

## Changes (8 files new/modified)

### 1. Provider module — `crates/lr-providers/src/llamacpp.rs` (NEW)

Copy the Jan provider pattern. Key differences:
- `LlamaCppProvider { base_url, api_key, client }`
- Default: `http://localhost:8080/v1`
- `fn name() -> "llamacpp"`
- Health check: `GET {base_url without /v1}/health` (llama.cpp has a dedicated `/health` endpoint returning `{"status": "ok"}`)
- `supports_pull()` → false (default)

### 2. Module declaration — `crates/lr-providers/src/lib.rs`

Add `pub mod llamacpp;`

### 3. Config type — `crates/lr-config/src/types.rs`

Add enum variant + default config helper:
```rust
/// Local llama.cpp server instance
#[serde(rename = "llamacpp")]
LlamaCpp,
```
Plus `pub fn default_llamacpp() -> Self` with base_url `http://localhost:8080/v1`.

### 4. Factory — `crates/lr-providers/src/factory.rs`

- Add import for `llamacpp::LlamaCppProvider`
- Add `LlamaCppProviderFactory` struct implementing `ProviderFactory` + `DiscoverableProvider`
  - `provider_type()` → `"llamacpp"`
  - `display_name()` → `"llama.cpp"`
  - `default_base_url()` → `"http://localhost:8080/v1"`
  - `category` → Local, `default_free_tier` → AlwaysFreeLocal, `catalog_provider_id` → None, `model_list_source` → ApiOnly
  - Discovery: check `GET {base_url}/models`
- Add to `discover_local_providers()` vector

### 5. Main registration — `src-tauri/src/main.rs`

- Add `LlamaCppProviderFactory` to import
- Register factory: `provider_registry.register_factory(Arc::new(LlamaCppProviderFactory));`
- Add match arm: `config::ProviderType::LlamaCpp => "llamacpp"`
- Add auto-discovery config case: `"llamacpp" => config::ProviderConfig::default_llamacpp()`
- Add safety engine provider type + default URL match arms

### 6. Icon — download + add to `ServiceIcon.tsx`

- Download `https://raw.githubusercontent.com/ggml-org/llama.cpp/master/media/llama1-icon-transparent.png` to `public/icons/llamacpp.png` and `website/public/icons/llamacpp.png`
- Add `llamacpp: 'llamacpp.png'` to ICON_MAP
- Add `llamacpp: '🦙'` to EMOJI_MAP

### 7. Mock data — `website/src/components/demo/mockData.ts`

Add llama.cpp entry to `providerTypes` array with category `"local"`.

## Verification

1. `cargo check` — compiles
2. `cargo test -p lr-providers` — all tests pass
3. `cargo clippy` — no warnings
4. `npx tsc --noEmit` — TypeScript compiles
