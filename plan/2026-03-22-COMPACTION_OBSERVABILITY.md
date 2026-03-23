# Fix Memory Compaction: 0-byte Guard + Rich LLM Call Observability

## Context

Memory compaction produces 0-byte summaries (e.g., with `Ollama/qwen3.5:2b-q8_0`). The compaction event lacks visibility — no request/response bodies, no token counts, no file paths — making it impossible to debug. Root cause TBD pending better observability. Also, Ollama's non-streaming token counts are silently lost due to a response parsing bug.

**Goals:**
1. Guard against empty summaries (treat as error, not "complete")
2. Make the compaction event as rich as the existing `LlmCall` event
3. Add transcript/summary file paths with inline reading
4. Fix Ollama non-streaming token count parsing

---

## 1. Fix Ollama non-streaming token count parsing

**Bug**: `OllamaChatResponse.final_data` is always `None` because Ollama returns `prompt_eval_count` and `eval_count` at the **top level**, not nested under a `final_data` key.

**File**: `crates/lr-providers/src/ollama.rs`

Add `prompt_eval_count` and `eval_count` as top-level `#[serde(default)]` fields on `OllamaChatResponse`. Use those in the non-streaming path (line 485-493).

---

## 2. Guard against empty summaries

**File**: `crates/lr-memory/src/compaction.rs` (`compact_session`, line 113)
- After `llm.summarize()` returns `Ok(...)`, check if summary text is empty after trim
- If empty, log warning, return `ArchivedOnly` (treated as LLM failure)

---

## 3. Enrich CompactionLlm trait to return full response metadata

**File**: `crates/lr-memory/src/compaction.rs`

Add a result struct (avoids dependency on lr-providers):
```rust
pub struct CompactionResult {
    pub summary: String,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub reasoning_tokens: Option<u32>,
    pub finish_reason: Option<String>,
    pub request_body: Option<serde_json::Value>,
    pub response_body: Option<serde_json::Value>,
}
```

Change trait:
```rust
async fn summarize(&self, model: &str, transcript: &str) -> Result<CompactionResult, String>;
```

**File**: `src-tauri/src/main.rs` (`RouterCompactionLlm`)
- Serialize `CompletionRequest` and `CompletionResponse` into `serde_json::Value`
- Fill all `CompactionResult` fields from the response
- Keep returning `Err` on empty choices (existing behavior)

---

## 4. Restructure MemoryCompaction event to match LlmCall pattern

**File**: `crates/lr-monitor/src/types.rs` (line 494)

```rust
MemoryCompaction {
    // Request fields (at creation)
    session_id: String,
    model: String,
    transcript_bytes: u64,
    transcript_path: Option<String>,
    request_body: Option<serde_json::Value>,

    // Response fields (on completion)
    summary_bytes: Option<u64>,
    summary_path: Option<String>,
    compression_ratio: Option<f64>,
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    reasoning_tokens: Option<u64>,
    finish_reason: Option<String>,
    response_body: Option<serde_json::Value>,
    content_preview: Option<String>,
    error: Option<String>,
}
```

**File**: `crates/lr-memory/src/lib.rs`
- `emit_compaction_event`: Set `transcript_path` and `request_body`
- `complete_compaction_event`: Accept `CompactionResult`, fill all response fields + `summary_path`
- Change `CompactionOutcome::ArchivedAndSummarized` to carry `CompactionResult`

---

## 5. Add Tauri command to read archive files

**File**: `src-tauri/src/ui/commands_memory.rs`
```rust
#[tauri::command]
pub async fn read_memory_archive_file(client_id: String, filename: String) -> Result<String, String>
```
- Validates `filename` has no path traversal
- Reads from `{memory_dir}/{client_id}/archive/{filename}`

**File**: `src/types/tauri-commands.ts` — Add `ReadMemoryArchiveFileParams`
**File**: `website/src/components/demo/TauriMockSetup.ts` — Mock handler

---

## 6. Update monitor event detail UI

**File**: `src/views/monitor/event-detail.tsx` (`MemoryCompactionDetail`, line 1174)

Restructure to use Request/Response/Error tabs like `LlmCallDetail`:
- **Request tab**: Model, session, transcript size, transcript path + "Read" button, messages/params from request_body
- **Response tab**: Token counts, compression ratio, summary path + "Read" button, content preview, full response body JSON
- **Error tab**: Error message
- "Read" buttons use `read_memory_archive_file` to fetch content inline

---

## Files to Modify

| File | Change |
|------|--------|
| `crates/lr-providers/src/ollama.rs` | Fix non-streaming token count parsing |
| `crates/lr-memory/src/compaction.rs` | Add `CompactionResult`, enrich trait, guard empty summaries |
| `crates/lr-monitor/src/types.rs` | Restructure `MemoryCompaction` event fields |
| `crates/lr-memory/src/lib.rs` | Thread `CompactionResult`, populate new event fields |
| `src-tauri/src/main.rs` | Fill `CompactionResult` in `RouterCompactionLlm::summarize` |
| `src-tauri/src/ui/commands_memory.rs` | New `read_memory_archive_file` command |
| `src/types/tauri-commands.ts` | New TS types |
| `website/src/components/demo/TauriMockSetup.ts` | Mock handler |
| `src/views/monitor/event-detail.tsx` | Rich Request/Response/Error tabs |

---

## Verification

1. `cargo test && cargo clippy`
2. `npx tsc --noEmit`
3. Trigger compaction → verify monitor event shows full request/response with token counts
4. Verify transcript/summary paths show and "Read" buttons work
5. Verify empty summary → error status (not "complete" with 0 bytes)
