# Safety Models: Download & Add New Model

## Context

The guardrails UI currently shows predefined safety models with enable/disable toggles and provider configuration fields, but there's no way to:
1. **Download models** - either via HuggingFace (built-in/local) or verify availability on a provider
2. **Add new models** - users can't add custom safety models beyond the 5 predefined ones

The user wants two execution paths:
- **Built-in (HuggingFace)**: Download GGUF models from HuggingFace and run locally
- **External provider**: Select from already-configured providers and pick an existing model

For predefined models, category mappings are already configured. For custom models (new HF or provider-based), users must also specify output-to-category mapping.

## Implementation

### 1. HuggingFace Model Downloader (Backend)

**Create `crates/lr-guardrails/src/downloader.rs`**

Follow the RouteLLM pattern from `crates/lr-routellm/src/downloader.rs`:
- Add `hf-hub` dependency to `crates/lr-guardrails/Cargo.toml`
- Add `tauri` as optional dependency for `AppHandle` (emit progress events)
- Download GGUF files from HuggingFace repos
- Store in `~/.localrouter-dev/safety_models/{model_id}/` (dev) or platform equivalent (prod)
- Emit events: `safety-model-download-progress`, `safety-model-download-complete`, `safety-model-download-failed`
- Progress payload: `{ model_id, progress: f32, total_bytes: u64, downloaded_bytes: u64 }`
- Global download lock (one model at a time)
- Disk space check, retry logic, atomic temp directory approach

The predefined models have `hf_repo_id` and `gguf_filename` already in `SafetyModelConfig`. Use these to know what to download.

### 2. New Tauri Commands (Backend)

**In `src-tauri/src/ui/commands.rs`:**

**`download_safety_model(model_id: String)`**
- Look up model config by ID
- Validate `hf_repo_id` and `gguf_filename` are set
- Call downloader with AppHandle for progress events
- Async (returns immediately, progress via events)

**`get_safety_model_download_status(model_id: String)`**
- Check if GGUF file exists at expected path
- Return: `{ downloaded: bool, file_path: Option<String>, file_size: Option<u64> }`

**`add_safety_model(config_json: String)`**
- Parse `SafetyModelConfig` from JSON
- Generate unique ID if not provided
- Set `predefined: false`
- Append to `config.guardrails.safety_models`
- Persist config

**`remove_safety_model(model_id: String)`**
- Only allow removing non-predefined models
- Remove from `config.guardrails.safety_models`
- Optionally delete downloaded GGUF files
- Persist config

### 3. Provider Model Availability (Backend Enhancement)

**Enhance `get_safety_model_status`** in `src-tauri/src/ui/commands.rs`:
- Currently returns `provider_configured` and `model_available`
- Add `downloaded: bool` (check if GGUF exists locally)
- Add `execution_mode: "provider" | "local" | "not_configured"`

**New: `list_provider_models_for_guardrails(provider_id: String)`**
- Reuse existing `list_provider_models` from `commands_providers.rs`
- Returns model list from the specified provider (Ollama, OpenRouter, etc.)
- Frontend uses this to populate model name dropdown

### 4. Frontend: Model Card Enhancements

**In `src/views/settings/guardrails-tab.tsx`:**

Enhance each model card to show execution mode and download/status:

**Execution mode selector** (radio or toggle):
- "Provider" - show provider dropdown + model selector
- "Local (Built-in)" - show download button + status

**Provider mode** (when selected):
- Provider dropdown: populated from `list_provider_instances()` (already exists as `ProviderInstanceInfo`)
- Model name: either free text input OR a dropdown populated from `list_provider_models(providerId)` when provider supports model listing
- Status: "Ready" (provider + model available), "Model Unavailable" (provider configured but model not found), "Not Configured"

**Local/Built-in mode** (when selected):
- Download button with progress bar (follow RouteLLM pattern)
- Listen to `safety-model-download-progress/complete/failed` events
- Show file size and path when downloaded
- Status: "Downloaded" / "Not Downloaded" / "Downloading..."

### 5. Frontend: Add Model Dialog

**Create `src/components/guardrails/AddSafetyModelDialog.tsx`:**

A dialog/modal with these fields:
- **Label** (display name)
- **Model Type** dropdown: predefined types (llama_guard, shield_gemma, nemotron, granite_guardian, custom)
- **Execution Mode**: "Provider" or "Local (HuggingFace)"
- If Provider:
  - Provider dropdown (from `list_provider_instances()`)
  - Model name input
- If Local:
  - HuggingFace Repo ID input (e.g. `QuantFactory/shieldgemma-2b-GGUF`)
  - GGUF Filename input (e.g. `shieldgemma-2b.Q4_K_M.gguf`)
- **Requires HF Auth** checkbox
- **Custom output mapping** (shown when model_type is "custom"):
  - Prompt template textarea (with `{content}` placeholder)
  - Safe indicator input (e.g. "safe")
  - Output regex input (e.g. `category:\s*(\w+)`)
  - Category mapping table: native label → safety category dropdown

Add an "Add Model" button below the model list in Section 2.
Add a "Remove" button (trash icon) on non-predefined model cards.

### 6. Config Schema Update

**In `crates/lr-config/src/types.rs`:**

Add to `SafetyModelConfig`:
```rust
pub execution_mode: Option<String>,  // "provider" or "local", default "provider"
// Custom model fields (for model_type == "custom"):
pub prompt_template: Option<String>,
pub safe_indicator: Option<String>,
pub output_regex: Option<String>,
pub category_mapping: Option<Vec<CategoryMappingEntry>>,
```

Add new struct:
```rust
pub struct CategoryMappingEntry {
    pub native_label: String,
    pub safety_category: String,
}
```

### 7. TypeScript Types

**In `src/types/tauri-commands.ts`:**

```typescript
// New types
export interface SafetyModelDownloadStatus {
  downloaded: boolean
  file_path: string | null
  file_size: number | null
}

export interface CategoryMappingEntry {
  native_label: string
  safety_category: string
}

// New param types
export interface DownloadSafetyModelParams { modelId: string }
export interface GetSafetyModelDownloadStatusParams { modelId: string }
export interface AddSafetyModelParams { configJson: string }
export interface RemoveSafetyModelParams { modelId: string }
export interface ListProviderModelsForGuardrailsParams { providerId: string }

// Update SafetyModelConfig to add:
//   execution_mode: string | null
//   prompt_template: string | null
//   safe_indicator: string | null
//   output_regex: string | null
//   category_mapping: CategoryMappingEntry[] | null
```

### 8. Demo Mocks

**In `website/src/components/demo/TauriMockSetup.ts`:**

Add mock handlers for new commands returning sensible defaults.

## Files to Modify/Create

### Create:
- `crates/lr-guardrails/src/downloader.rs` - HF GGUF downloader
- `src/components/guardrails/AddSafetyModelDialog.tsx` - Add model dialog

### Modify:
- `crates/lr-guardrails/Cargo.toml` - add `hf-hub`, `tauri` deps
- `crates/lr-guardrails/src/lib.rs` - add `pub mod downloader`
- `crates/lr-config/src/types.rs` - extend `SafetyModelConfig`, add `CategoryMappingEntry`
- `src-tauri/src/ui/commands.rs` - new commands: download, add, remove, download_status
- `src-tauri/src/main.rs` - register new commands
- `src/views/settings/guardrails-tab.tsx` - execution mode UI, download progress, add/remove buttons
- `src/types/tauri-commands.ts` - new types and param interfaces
- `website/src/components/demo/TauriMockSetup.ts` - mock handlers

## Verification

1. `cargo test && cargo clippy` - Rust compiles, no warnings
2. `npx tsc --noEmit` - TypeScript compiles
3. Manual: Configure Ollama provider, add a safety model via "Provider" mode, verify status shows "Ready"
4. Manual: Click "Download" on a built-in model with HF repo, verify progress bar and completion
5. Manual: Open "Add Model" dialog, add a custom model with output mapping
6. Manual: Remove a custom model, verify it's gone from the list
