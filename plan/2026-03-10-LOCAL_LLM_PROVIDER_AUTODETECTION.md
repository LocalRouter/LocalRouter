# Local LLM Provider Auto-Detection

## Context
LocalRouter already has a `DiscoverableProvider` trait and `discover_local_providers()` function that runs on first start. However, the detection includes unreliable providers (LocalAI and llama.cpp both use port 8080, which is too common), and there's no way to trigger discovery from the UI for testing. The goal is to make detection more reliable and add a debug page button.

## Provider Reliability Analysis

| Provider | Port | Unique Port? | Self-Identifies? | Auto-Detect? |
|----------|------|-------------|-------------------|--------------|
| Ollama | 11434 | Yes | Yes - GET `/` returns "Ollama is running", and `/api/tags` is Ollama-specific | **YES** |
| LM Studio | 1234 | Yes | No | **YES** (unique port) |
| Jan | 1337 | Yes | No | **YES** (unique port) |
| GPT4All | 4891 | Yes | No | **YES** (unique port) |
| LocalAI | 8080 | No | No | **NO** - port 8080 too common |
| llama.cpp | 8080 | No | No | **NO** - port 8080 too common |

## Changes

### 1. Remove unreliable providers from discovery list
**File:** `crates/lr-providers/src/factory.rs` (~line 1801)

Remove `LocalAIProviderFactory` and `LlamaCppProviderFactory` from the `discoverable` vec in `discover_local_providers()`. Keep their `DiscoverableProvider` trait impls in place (no harm, just unused).

### 2. Enhance Ollama detection with body verification
**File:** `crates/lr-providers/src/factory.rs` (~line 291)

In `OllamaProviderFactory::is_available()`, after confirming `/api/tags` responds OK, also check `GET /` and verify the body contains "Ollama is running". If the root check fails, still return true since `/api/tags` on port 11434 is already a strong signal. The root check is bonus verification.

### 3. Add `Serialize` to `DiscoveredProvider`
**File:** `crates/lr-providers/src/factory.rs` (~line 1784)

Change `#[derive(Debug, Clone)]` to `#[derive(Debug, Clone, Serialize)]` so it can be returned from a Tauri command.

### 4. Add `debug_discover_providers` Tauri command
**File:** `src-tauri/src/ui/commands.rs` (after existing debug commands ~line 2840)

Add a `DiscoverProviderResult` struct:
```rust
#[derive(Serialize)]
pub struct DiscoverProviderResult {
    pub discovered: Vec<lr_providers::factory::DiscoveredProvider>,
    pub added: Vec<String>,
    pub skipped: Vec<String>,
}
```

Add `debug_discover_providers` command that:
1. Calls `discover_local_providers().await`
2. Checks existing config providers to find which are already configured (compare `provider_type` strings)
3. Adds new ones via `config_manager.update()` using `ProviderConfig::default_*()` helpers
4. Also creates them in the registry via `registry.create_provider()`
5. Saves config, emits `"providers-changed"` and `"models-changed"` events
6. Returns `DiscoverProviderResult` with discovered/added/skipped lists

### 5. Register command
**File:** `src-tauri/src/main.rs` (~line 1689)

Add `ui::commands::debug_discover_providers` to the `invoke_handler` list after existing debug commands.

### 6. Remove unreliable providers from first-start logic
**File:** `src-tauri/src/main.rs` (~line 235-236)

Remove the `"localai"` and `"llamacpp"` match arms from the first-start discovery block (they'll never be returned by `discover_local_providers()` anymore, but clean up for consistency).

### 7. Add TypeScript types
**File:** `src/types/tauri-commands.ts`

```typescript
/** Rust: crates/lr-providers/src/factory.rs - DiscoveredProvider */
export interface DiscoveredProvider {
  provider_type: string
  instance_name: string
  base_url: string
}

/** Rust: src-tauri/src/ui/commands.rs - DiscoverProviderResult */
export interface DiscoverProviderResult {
  discovered: DiscoveredProvider[]
  added: string[]
  skipped: string[]
}
```

### 8. Add demo mock
**File:** `website/src/components/demo/TauriMockSetup.ts` (near other debug mocks)

```typescript
'debug_discover_providers': (): DiscoverProviderResult => ({
  discovered: [{ provider_type: 'ollama', instance_name: 'Ollama', base_url: 'http://localhost:11434' }],
  added: ['Ollama'],
  skipped: [],
}),
```

### 9. Add debug page UI
**File:** `src/views/debug/index.tsx`

Add a "Local Provider Discovery" card after the tray overlay section with:
- "Discover Providers" button that calls `invoke<DiscoverProviderResult>("debug_discover_providers")`
- State: `discovering` (bool), `discoveryResult` (DiscoverProviderResult | null)
- Results display: count found, green text for added providers, muted text for already-configured ones

## Verification
1. `cargo test && cargo clippy && cargo fmt`
2. `npx tsc --noEmit`
3. Start Ollama locally, open debug page, click "Discover Providers" — should detect and add Ollama
4. Click again — should show Ollama as "already configured"
5. Test with no local providers running — should show "No local providers detected"
