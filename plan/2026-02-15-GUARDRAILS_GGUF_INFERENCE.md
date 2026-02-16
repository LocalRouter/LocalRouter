# Implement Local GGUF Inference for Safety Models

## Context
The `LocalGgufExecutor` in `crates/lr-guardrails/src/executor.rs` is a stub that returns "Local GGUF inference not yet implemented." Safety models downloaded as GGUF files (Llama Guard, ShieldGemma, Granite Guardian, Nemotron) cannot actually run inference. The previous session incorrectly tried to route through Ollama — that defeats the purpose of local GGUF downloads.

The project already uses Candle for RouteLLM (SafeTensors BERT models in `crates/lr-routellm/`), but GGUF quantized LLMs need `llama-cpp-2` (Rust bindings for llama.cpp) which is the native GGUF runtime.

## Plan

### 1. Add `llama-cpp-2` dependency to `crates/lr-guardrails/Cargo.toml`

```toml
llama-cpp-2 = "0.1"
encoding_rs = "0.8"
```

No special features needed — defaults give CPU + Metal (macOS Apple Silicon).

### 2. Rewrite `LocalGgufExecutor` in `crates/lr-guardrails/src/executor.rs`

Replace the stub with a real implementation:

- **`LlamaBackend`**: Initialize once per process via `OnceCell<LlamaBackend>` at module level
- **`LlamaModel`**: Loaded from the GGUF file path in `LocalGgufExecutor::new()` and stored in the struct
- **`complete()` method**:
  1. Create a `LlamaContext` from the stored model (cheap, per-request)
  2. Tokenize the prompt via `model.str_to_token()`
  3. Feed tokens into `LlamaBatch`, decode
  4. Sample with greedy (temperature 0.0 for safety classifiers)
  5. Collect output tokens until EOS or max_tokens
  6. Return `CompletionResponse { text, logprobs }`
- **Logprobs support**: After each decode, call `ctx.get_logits_ith()` to get raw logits. For Yes/No models (ShieldGemma, Granite Guardian), compute softmax over all tokens and extract Yes/No probabilities. Return as `LogprobsResult` with `TokenLogprob` entries.
- **Threading**: `LlamaContext` is `!Send`, so wrap `complete()` in `tokio::task::spawn_blocking()`

### 3. Revert Ollama routing in `engine.rs`

Change `direct_download`/`custom_download` handler back to using `LocalGgufExecutor`:
- Look up GGUF file path via `crate::downloader::model_file_path()`
- Check the file exists (skip model if not downloaded yet)
- Create `LocalGgufExecutor::new(gguf_path)`
- Wrap in `Arc::new(ModelExecutor::Local(...))`

### 4. Revert Ollama import code

- Remove `import_to_ollama()` and `ollama_model_name()` from `downloader.rs`
- Remove Ollama import logic from `download_safety_model` command in `commands.rs`
- Remove Ollama import event listeners from `guardrails-tab.tsx`
- Restore simple engine rebuild on download-complete

### 5. Model preloading

Store the `LlamaModel` in the executor (loaded once), not the context. Models are the expensive part (~1-2s to load). Contexts are cheap to create per-request.

Since `LlamaModel` may not be `Send`, the executor may need to hold it behind a `Mutex` or use `spawn_blocking` for all operations.

## Key Files
- `crates/lr-guardrails/Cargo.toml` — add `llama-cpp-2`, `encoding_rs`
- `crates/lr-guardrails/src/executor.rs` — rewrite `LocalGgufExecutor`
- `crates/lr-guardrails/src/engine.rs` — revert to `LocalGgufExecutor` for direct_download
- `crates/lr-guardrails/src/downloader.rs` — remove Ollama import functions
- `src-tauri/src/ui/commands.rs` — revert download command to simple version
- `src/views/settings/guardrails-tab.tsx` — revert Ollama event listeners

## Verification
1. `cargo check` — compiles without errors
2. `cargo test -p lr-guardrails` — all tests pass
3. `npx tsc --noEmit` — TypeScript check
4. Manual: Download a model → test panel → should get actual inference results instead of "not yet implemented" error
