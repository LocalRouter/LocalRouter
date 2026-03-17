# Fix Safety Model GGUF Loading Crash & Remove Broken Variant

## Context

Loading `llama_guard_4_12b` crashes the app via `GGML_ASSERT(hparams.n_expert_used <= hparams.n_expert)` in llama.cpp. This is a C `abort()` — uncatchable by Rust's panic handler. The root cause is a bad GGUF conversion from `DevQuasar` that has inconsistent MoE expert metadata. No reputable GGUF converter has published Llama Guard 4 12B yet (llama.cpp doesn't fully support the Llama 4 architecture).

More broadly, any corrupt GGUF file can trigger `abort()` in llama.cpp and kill the entire app. Load errors are silently swallowed at every layer — the UI shows "Ready" based on download status alone.

## Changes

### 1. Remove `llama_guard_4_12b` variant
**File:** `src/constants/safety-model-variants.ts`
- Remove the `llama_guard_4_12b` entry (lines 50-60). No reputable GGUF exists.

### 2. Add GGUF pre-validation before loading
**File:** `crates/lr-guardrails/src/executor.rs`
- Add `validate_gguf_file(path: &Path) -> Result<(), String>` that:
  - Checks GGUF magic bytes (`GGUF`)
  - Reads version (must be 2 or 3)
  - Parses metadata KV pairs to find `llama.expert_count` and `llama.expert_used_count`
  - Validates `expert_used <= expert_count`
  - Returns descriptive error for any failure (e.g. "Corrupt GGUF file: expert_used_count (16) > expert_count (0)")
- Call `validate_gguf_file()` at the start of `load_model()` before `LlamaModel::load_from_file()`

### 3. Make `LocalGgufExecutor::new()` return `Result`
**File:** `crates/lr-guardrails/src/executor.rs`
- Change signature to `pub fn new(...) -> Result<Self, String>`
- On pre-load failure, return `Err` instead of silently warn-logging

### 4. Propagate load errors from `SafetyEngine::from_config()`
**File:** `crates/lr-guardrails/src/engine.rs`
- Add a `load_errors: Vec<(String, String)>` field (model_id, error_message) to track which models failed
- Change return type: return `(Self, Vec<(String, String)>)` tuple
- When `LocalGgufExecutor::new()` returns `Err`, push to `load_errors` instead of silently continuing

**File:** `crates/lr-guardrails/src/lib.rs`
- No change needed (types re-exported already)

### 5. Emit `safety-model-load-failed` events from `rebuild_safety_engine`
**File:** `src-tauri/src/ui/commands.rs` (rebuild_safety_engine, ~line 2173)
- After `SafetyEngine::from_config()`, iterate load_errors and emit Tauri event `safety-model-load-failed` with `{ model_id, error }` for each
- Still return `Ok(())` (partial success is fine — other models may work)

### 6. Handle load errors in frontend
**File:** `src/views/settings/guardrails-tab.tsx`
- Add state: `loadErrors: Record<string, string>` mapping model_id → error message
- Listen for `safety-model-load-failed` event, update `loadErrors` state, show `toast.error`
- Pass `loadErrors` to `SafetyModelList`
- On retry: delete model files, clear error, re-download (call existing `delete_model_files` via new command or existing `download_safety_model`)

**File:** `src/components/guardrails/SafetyModelList.tsx`
- Accept new prop `loadErrors: Record<string, string>`
- When a model has a load error: show error badge ("Corrupt file" / "Load failed") in red instead of "Ready"
- Show the retry/re-download button (reuse existing `onRetryDownload`) — the handler in guardrails-tab will delete + re-download

### 7. Add `delete_safety_model_files` Tauri command
**File:** `src-tauri/src/ui/commands.rs`
- New command wrapping `lr_guardrails::downloader::delete_model_files(model_id)`
- This enables the UI to delete corrupt files before re-downloading

**File:** `src/types/tauri-commands.ts`
- Add `DeleteSafetyModelFilesParams { modelId: string }`

**File:** `website/src/components/demo/TauriMockSetup.ts`
- Add mock for `delete_safety_model_files`

### 8. Wire up retry flow in guardrails-tab
**File:** `src/views/settings/guardrails-tab.tsx`
- Modify `handleDownloadModel` (or create `handleRetryCorruptModel`):
  - Call `delete_safety_model_files` to remove corrupt file
  - Clear the load error for that model
  - Then call existing `download_safety_model`

## Verification

1. `cargo test -p lr-guardrails` — ensure existing tests pass
2. `cargo clippy` — no warnings
3. `npx tsc --noEmit` — TypeScript compiles
4. Manual: confirm `llama_guard_4_12b` no longer appears in the UI picker
5. Manual: to test error path, could temporarily corrupt a downloaded GGUF file (truncate it) and verify the UI shows "Load failed" with retry button
