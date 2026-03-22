# Improve Memory Compaction

## Context

The current memory compaction system has a basic prompt, no monitor visibility, and a subtle bug where sessions are never archived when no compaction model is configured. By comparing with memsearch's approach and adding monitor integration + source type indicators, we can significantly improve the system.

**Key problems:**
1. Background monitor loop only archives sessions when `compaction_model` is set — sessions without it are removed from tracking but files stay in `sessions/` forever
2. The compaction prompt is basic and doesn't guide the LLM on output structure, compression expectations, or searchability
3. No monitor visibility — compaction happens silently in the background
4. LLMs reading memories can't distinguish raw transcripts from compacted summaries
5. No explicit enable/disable toggle — compaction is implicitly controlled by `compaction_model` being set or None

---

## Phase 1: Fix Background Loop + Add `compaction_enabled` Config

### 1.1 Add `compaction_enabled` to `MemoryConfig`

**File: `crates/lr-config/src/types.rs:2369`**

Add field to `MemoryConfig`:
```rust
/// Whether LLM-based compaction is enabled (default: false).
/// When false, expired sessions are archived without summarization.
#[serde(default)]
pub compaction_enabled: bool,
```

Default to `false` in the `Default` impl.

### 1.2 Fix background loop to always archive

**File: `crates/lr-memory/src/lib.rs:308-378` (`start_session_monitor`)**

Current bug: the entire `for (client_id, session) in expired` body is inside `if config.compaction_model.is_some()`. Fix:
- Always call `compact_session()` for expired sessions
- Only pass `llm`/`model` when BOTH `compaction_enabled == true` AND `compaction_model.is_some()`

```rust
for (client_id, session) in expired {
    let config = service.config.read().clone();
    let client_dir = service.memory_dir.join(&client_id);
    let archive_dir = client_dir.join("archive");

    // Only provide LLM when compaction is explicitly enabled + model configured
    let (llm_arc, model) = if config.compaction_enabled && config.compaction_model.is_some() {
        (service.compaction_llm.read().clone(), config.compaction_model.as_deref().map(|s| s.to_string()))
    } else {
        (None, None)
    };

    // Always archive, optionally summarize
    match compact_session(&session.file_path, &archive_dir, llm_arc.as_deref(), model.as_deref()).await {
        // ... same index update logic ...
    }
}
```

### 1.3 Update `force_compact` and `recompact_all`

**File: `crates/lr-memory/src/lib.rs:435-518`**

- `force_compact`: Gate LLM usage on `compaction_enabled && compaction_model.is_some()`
- `recompact_all`: Require `compaction_enabled` in addition to `compaction_model`

### 1.4 Update TypeScript types and frontend

**File: `src/types/tauri-commands.ts`** — Add `compaction_enabled: boolean` to `MemoryConfig`

**File: `src/views/memory/index.tsx`**:
- Add to `defaultConfig`: `compaction_enabled: false`
- Add a Switch toggle "Enable LLM Compaction" in the Compaction Model card, above the provider/model selectors
- When toggle is off, gray out the provider/model selectors
- When toggle is turned on with no model selected, auto-keep selectors enabled for user to pick

**File: `website/src/components/demo/TauriMockSetup.ts`** — Add `compaction_enabled: false` to mock

---

## Phase 2: Improve Compaction Prompt

**File: `crates/lr-memory/src/compaction.rs:31-35`**

Replace the current basic prompt with a more detailed one inspired by memsearch but improved:

```rust
const COMPACTION_SYSTEM_PROMPT: &str = "\
You are a memory compaction assistant. Your task is to compress a conversation \
transcript into a structured summary that preserves all important information \
while being significantly shorter than the original.

## Instructions

1. **Preserve completely**: decisions made, technical details, code snippets, \
action items, configuration changes, error messages, and their resolutions.

2. **Use structured markdown**: organize by topic with `##` headers and bullet points. \
Group related items together rather than preserving chronological order.

3. **Optimize for searchability**: include specific names, function/file names, \
model identifiers, error codes, and domain terms. A future search should be able \
to find any important detail mentioned in the original conversation.

4. **Compress aggressively**: remove greetings, filler, repeated context, \
and conversational back-and-forth. Keep only the information payload. \
Target 20-30% of the original length.

5. **Preserve code snippets**: include short code examples, commands, and \
configuration values verbatim — do not paraphrase technical content.

6. **Note unresolved items**: if the conversation ended with open questions \
or incomplete work, add a `## Open Items` section at the end.";
```

Key improvements over current prompt:
- Explicit compression target (20-30%)
- Topic-based grouping instead of chronological
- Searchability guidance (names, identifiers, error codes)
- Instruction to preserve code verbatim
- Open items section for incomplete work
- Aggressive noise removal guidance

Key improvements over memsearch:
- Structured markdown with topic headers (not just bullet points)
- Compression ratio target
- Open items tracking
- Searchability optimization
- Code preservation emphasis

---

## Phase 3: Add Monitor Events for Compaction

### 3.1 Add `MemoryCompaction` event type

**File: `crates/lr-monitor/src/types.rs`**

Add to `MonitorEventType` enum (after `PromptCompression`):
```rust
MemoryCompaction,
```

Add label: `"Memory Compaction"`
Add category: `"memory"` (new category)

Add to `MonitorEventData` enum:
```rust
MemoryCompaction {
    // Request fields (populated at creation)
    session_id: String,
    model: String,
    transcript_bytes: u64,

    // Response fields (filled on completion)
    #[serde(skip_serializing_if = "Option::is_none")]
    summary_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    compression_ratio: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    request_body: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_body: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
},
```

### 3.2 Add summary generation

**File: `crates/lr-monitor/src/summary.rs`**

Add match arm for `MemoryCompaction`:
- Pending: `"Compacting session {short_id} via {model}"`
- Complete: `"Compacted {short_id}: {transcript_bytes}B → {summary_bytes}B ({ratio}% reduction)"`
- Error: `"Compaction failed for {short_id}: {error}"`

### 3.3 Pass MonitorEventStore to MemoryService

**File: `crates/lr-memory/src/lib.rs`**

Add `monitor_store: Option<Arc<MonitorEventStore>>` field to `MemoryService`. Set via a `set_monitor_store()` method (same pattern as `set_compaction_llm()`).

**File: `src-tauri/src/main.rs`** — Call `service.set_monitor_store(monitor_store.clone())` during setup.

### 3.4 Emit events in compaction flow

**File: `crates/lr-memory/src/lib.rs` — `start_session_monitor` and `force_compact`**

When LLM compaction is about to happen:
1. Push a `Pending` `MemoryCompaction` event with `session_id`, `model`, `transcript_bytes`, and `request_body` (the transcript, truncated to ~10KB for display)
2. Call `compact_session()`
3. On success: update event to `Complete` with `summary_bytes`, `compression_ratio`, `response_body` (the summary)
4. On error: update event to `Error` with error message

For archive-only (no LLM), no monitor event needed — it's just a file move.

### 3.5 Add `lr-monitor` dependency to `lr-memory`

**File: `crates/lr-memory/Cargo.toml`** — Add `lr-monitor` as optional dependency or just direct dependency. Since `lr-monitor` has no dependencies on `lr-memory`, no circular dependency risk.

### 3.6 Frontend monitor integration

**File: `src/views/monitor/event-detail.tsx`**

Add `MemoryCompactionDetail` component:
- Show session ID, model used
- Show transcript size → summary size with compression ratio
- Expandable sections for request body (transcript) and response body (summary)

**File: `src/views/monitor/event-filters.tsx`**

Add "Memory" category to the filter groups with `MemoryCompaction` type.

**File: `src/views/monitor/event-list.tsx`** (if category icons/colors are defined here)

Add memory category color (e.g., amber/yellow to distinguish from existing categories).

---

## Phase 4: Indicate Transcript vs Summary in Search Results

### 4.1 Add source type annotation to search result display

**File: `crates/lr-context/src/types.rs:393-406` (SearchResult Display impl)**

In the hit display line, add a type indicator based on the source label:
```rust
// Current:
// **[1] session/abc123 — Topic** (lines 5-12)

// New:
// **[1] session/abc123 — Topic** (lines 5-12) [transcript]
// **[1] session/abc123-summary — Topic** (lines 5-12) [compacted summary]
```

Add logic in the Display impl to detect `session/` prefix and `-summary` suffix:
```rust
let source_annotation = if hit.source.starts_with("session/") {
    if hit.source.ends_with("-summary") {
        " `[compacted summary]`"
    } else {
        " `[transcript]`"
    }
} else {
    ""
};

writeln!(f, "**[{}] {} \u{2014} {}** (lines {}-{}){}",
    i + 1, hit.source, hit.title, hit.line_start, hit.line_end, source_annotation)?;
```

This is scoped to `session/`-prefixed sources only, so it won't affect IndexSearch results from MCP content stores.

---

## Critical Files Summary

| File | Change |
|------|--------|
| `crates/lr-config/src/types.rs` | Add `compaction_enabled: bool` to MemoryConfig |
| `crates/lr-memory/src/compaction.rs` | Improve compaction prompt |
| `crates/lr-memory/src/lib.rs` | Fix background loop, add monitor store, emit events |
| `crates/lr-memory/Cargo.toml` | Add `lr-monitor` dependency |
| `crates/lr-monitor/src/types.rs` | Add `MemoryCompaction` event type + data |
| `crates/lr-monitor/src/summary.rs` | Add summary generation for compaction events |
| `src-tauri/src/main.rs` | Wire monitor store to memory service |
| `src/types/tauri-commands.ts` | Add `compaction_enabled` to MemoryConfig type |
| `src/views/memory/index.tsx` | Add enable toggle, gray out model when disabled |
| `src/views/monitor/event-detail.tsx` | Add MemoryCompactionDetail component |
| `src/views/monitor/event-filters.tsx` | Add Memory filter category |
| `crates/lr-context/src/types.rs` | Add `[transcript]`/`[compacted summary]` annotation |
| `website/src/components/demo/TauriMockSetup.ts` | Update mock config |

---

## Verification

1. **Config**: Set `compaction_enabled: false` (default) → sessions expire and archive without summarization. Set `compaction_enabled: true` + select model → sessions get summarized
2. **Settings UI**: Toggle switch disables/enables model selectors. Toggling off saves `compaction_enabled: false`
3. **Monitor**: With compaction enabled, trigger a session expiry (or force-compact). Verify `MemoryCompaction` event appears in monitor with Pending → Complete transition, and request/response bodies are viewable
4. **Search results**: Search memory → verify `[transcript]` or `[compacted summary]` annotation appears in results
5. **Background loop fix**: With compaction disabled, let a session expire → verify file moves from `sessions/` to `archive/` (was previously stuck in `sessions/`)
6. **Tests**: `cargo test -p lr-memory && cargo test -p lr-monitor && cargo clippy`

## Final Steps (after implementation)

1. **Plan Review**: Check all items above are implemented
2. **Test Coverage Review**: Ensure new code paths have tests
3. **Bug Hunt**: Re-read implementation for edge cases
4. **Commit**: Stage only modified files and commit
