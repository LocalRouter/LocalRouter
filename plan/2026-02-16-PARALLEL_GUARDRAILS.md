# Parallel Guardrails Implementation

**Date**: 2026-02-16
**Status**: Implemented

## Summary

Run guardrails safety checks in parallel with LLM requests by default, buffering the response until the safety check passes. This reduces perceived latency for guarded requests. Sequential mode is forced when the request may cause side effects.

## Changes Made

### 1. Config: `parallel_guardrails` field
- **`crates/lr-config/src/types.rs`**: Added `parallel_guardrails: bool` to `GuardrailsConfig` (default: `true`)
- **`src/types/tauri-commands.ts`**: Added `parallel_guardrails: boolean` to TS type
- **`website/src/components/demo/TauriMockSetup.ts`**: Updated mock

### 2. Side-Effect Detection
- **`crates/lr-server/src/routes/chat.rs`**: Added `has_side_effects()` — checks for non-function tools and Perplexity Sonar models
- **`crates/lr-server/src/routes/completions.rs`**: Same (model-only check, no tools in legacy completions)

### 3. Per-Client Category Filtering (Bug Fix)
- **`crates/lr-guardrails/src/safety_model.rs`**: Added `apply_client_category_overrides()` to `SafetyCheckResult`
- **`crates/lr-server/src/routes/chat.rs`**: Applied overrides in `run_guardrails_scan`
- **`crates/lr-server/src/routes/completions.rs`**: Same

### 4. Response Guardrails Removal
- Deleted `check_response_guardrails_body()` from both `chat.rs` and `completions.rs`
- Removed call sites in `handle_non_streaming`
- Removed `guardrails_aborted` dead code from streaming handlers

### 5. Non-Streaming Parallel Mode
- **`chat.rs`**: Added `handle_non_streaming_parallel()` — uses `tokio::join!` to run guardrails and LLM concurrently
- **`completions.rs`**: Same pattern
- Extracted `build_non_streaming_response()` helper shared by both modes

### 6. Streaming Parallel Mode
- **`chat.rs`**: Added `handle_streaming_parallel()` — uses `watch` channel for guardrail gate + `mpsc` channel for buffered SSE events
- **`completions.rs`**: Same pattern
- Added `tokio-stream` dependency for `ReceiverStream`

### 7. Frontend Toggle
- **`src/views/settings/guardrails-tab.tsx`**: Added "Parallel Scanning" switch with description

## Architecture

```
Request → guardrail_handle = spawn(scan)
        → rate_limits
        → convert_request

if parallel && !side_effects:
    Non-streaming: tokio::join!(guardrail, llm) → check guardrails → return response
    Streaming:     spawn(buffer_worker) → SSE from mpsc channel
                   buffer_worker: select! { stream chunks, gate change }
                   gate = Passed → flush buffer, forward chunks
                   gate = Denied → send error event
else:
    await guardrails → call LLM (original flow)
```
