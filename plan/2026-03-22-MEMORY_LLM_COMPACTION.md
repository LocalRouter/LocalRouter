# LLM-Based Memory Compaction

## Context

The "Compact Now" button in Memory > Sessions currently just moves `.md` files from `sessions/` to `archive/` without performing any actual compaction. The `compaction.rs` file has TODO comments for LLM summarization. The user wants:
1. Compaction to actually summarize transcripts using an LLM
2. Summaries get indexed in FTS5, originals get unindexed (but kept on disk)
3. A "Re-compact" button to re-run summarization on already-archived sessions

## File Layout

```
archive/
  {uuid}.md              # Raw original transcript (preserved, NOT indexed)
  {uuid}-summary.md      # LLM summary (indexed in FTS5)
```

This matches the existing convention in `restore_from_archive()` which already deletes `{session_id}-summary.md`.

## Implementation

### Step 1: CompactionLlm trait + compaction logic

**File: `crates/lr-memory/Cargo.toml`**
- Add `async-trait = { workspace = true }`

**File: `crates/lr-memory/src/compaction.rs`**

Replace current file-move-only code with:

1. Define `CompactionLlm` trait:
   ```rust
   #[async_trait::async_trait]
   pub trait CompactionLlm: Send + Sync + 'static {
       async fn summarize(&self, model: &str, transcript: &str) -> Result<String, String>;
   }
   ```

2. Update `compact_session()` signature to accept optional LLM + model:
   ```rust
   pub async fn compact_session(
       session_path: &Path,
       archive_dir: &Path,
       llm: Option<&dyn CompactionLlm>,
       model: Option<&str>,
   ) -> Result<CompactionOutcome, String>
   ```
   - Move file to archive (always)
   - If llm + model provided: read raw content, call LLM, write `{uuid}-summary.md` in archive
   - Return `CompactionOutcome::ArchivedOnly` or `CompactionOutcome::ArchivedAndSummarized`

3. Add `recompact_session()`:
   ```rust
   pub async fn recompact_session(
       session_id: &str,
       archive_dir: &Path,
       llm: &dyn CompactionLlm,
       model: &str,
   ) -> Result<(), String>
   ```
   - Read raw `{uuid}.md` from archive
   - Call LLM to summarize
   - Write/overwrite `{uuid}-summary.md` in archive

4. Summarization prompt (system message):
   ```
   You are a memory compaction assistant. Summarize this conversation transcript
   preserving: decisions made, technical details, action items, code snippets,
   and key context. Include specific terms and names for searchability.
   Format as structured markdown with topic headers.
   ```
   Temperature: 0.0, max_tokens: 4096

### Step 2: MemoryService changes

**File: `crates/lr-memory/src/lib.rs`**

1. Add `compaction_llm: RwLock<Option<Arc<dyn compaction::CompactionLlm>>>` field to `MemoryService`
2. Add `set_compaction_llm()` setter method
3. Update `force_compact()`:
   - Pass LLM + model from config to `compact_session()`
   - After summarization: index summary with label `"session/{uuid}-summary"`, delete original label `"session/{uuid}"` from FTS5
   - Add `summarized_count` to `CompactResult`
4. Add `recompact_all()`:
   ```rust
   pub async fn recompact_all(&self, client_id: &str) -> Result<RecompactResult, String>
   ```
   - Scan `archive/` for raw `.md` files (exclude `*-summary.md`)
   - For each: call `recompact_session()`, then update FTS5 (delete old label, index summary)
   - Return `RecompactResult { recompacted_count, failed_count }`
5. Update `get_compaction_stats()`:
   - Add `summarized_sessions: usize` — count of `*-summary.md` files in archive
   - `archived_sessions` should only count raw files (NOT `*-summary.md`)
6. Update `count_md_files()` or add `count_md_files_excluding_summaries()` + `count_summary_files()`
7. Update `reindex()`:
   - When scanning `archive/`, if `{uuid}-summary.md` exists alongside `{uuid}.md`, index ONLY the summary with label `"session/{uuid}-summary"` — skip the raw file
   - If only raw file exists (no summary), index normally as `"session/{uuid}"`
8. Update `start_session_monitor()`:
   - Pass LLM + model to `compact_session()`
   - After successful summarization: update FTS5 index (delete raw label, index summary)
9. Fix `restore_from_archive()` in `transcript.rs`:
   - Change summary path from `sessions_dir.join(...)` to `archive_dir.join(...)` since summaries live in archive

### Step 3: Wire up Router as CompactionLlm

**File: `src-tauri/src/main.rs`**

After both Router and MemoryService are created:

1. Implement `RouterCompactionLlm` struct wrapping `Arc<Router>`
2. Implement `CompactionLlm` trait — calls `router.complete("memory-service", request)` with system+user messages
3. Call `service.set_compaction_llm(Arc::new(RouterCompactionLlm { router }))` on the memory service

Uses existing `"memory-service"` client_id bypass in Router (line 1421 of `lr-router/src/lib.rs`).

### Step 4: Tauri commands

**File: `src-tauri/src/ui/commands.rs`**

1. Update `CompactionStatsResult`: add `summarized_sessions: usize`
2. Update `ForceCompactResult`: add `summarized_count: usize`
3. Make `force_compact_memory` async with progress events (like `reindex_client_memory`):
   - `memory-compact-progress`: `{ client_id, current, total }`
   - `memory-compact-complete`: `{ client_id, archived_count, summarized_count }`
   - `memory-compact-failed`: `{ client_id, error }`
4. Add `recompact_memory` command (async with events):
   - `memory-recompact-progress`: `{ client_id, current, total }`
   - `memory-recompact-complete`: `{ client_id, recompacted_count }`
   - `memory-recompact-failed`: `{ client_id, error }`
5. Register new command in `main.rs` handler list

### Step 5: Frontend

**File: `src/types/tauri-commands.ts`**

1. Update `CompactionStatsResult`: add `summarized_sessions: number`
2. Update `ForceCompactResult`: add `summarized_count: number`
3. Add event types: `MemoryCompactProgress`, `MemoryCompactComplete`, `MemoryRecompactProgress`, `MemoryRecompactComplete`
4. Add `RecompactMemoryParams`

**File: `src/views/memory/sessions-tab.tsx`**

1. Update stats grid — add "Summarized" card between Archived and Indexed
2. Update "Compact Now" to work with progress events instead of direct result:
   - Listen for `memory-compact-progress/complete/failed`
   - Show progress bar during compaction
   - Toast: "Compacted N sessions (M summarized)" or "Archived N sessions (no compaction model)"
3. Add "Re-compact" button (next to Compact Now and Rebuild Index):
   - Disabled when no `archived_sessions` exist or no compaction model configured
   - AlertDialog: "Re-compact archived sessions? This will re-run LLM summarization on N archived sessions. Existing summaries will be overwritten."
   - Listen for `memory-recompact-progress/complete/failed`
   - Show progress bar during re-compaction

**File: `website/src/components/demo/TauriMockSetup.ts`**

- Update mocks for `get_memory_compaction_stats` (add `summarized_sessions`)
- Update mocks for `force_compact_memory` (add `summarized_count`)
- Add `recompact_memory` mock

### Step 6: Tests

**File: `crates/lr-memory/src/tests.rs`**

1. Mock `CompactionLlm` implementation for tests
2. Test `compact_session` with LLM — verify summary file created, raw file moved
3. Test `compact_session` without LLM — verify archive-only behavior
4. Test `recompact_session` — verify summary overwritten
5. Test `reindex` with mixed raw/summary files — verify only summaries indexed when both exist
6. Test `get_compaction_stats` with `summarized_sessions` count
7. Test partial failures (LLM fails for one session, others succeed)

### Step 7: Final review

Per project conventions:
- Plan review: check all changes against this plan
- Test coverage review
- Bug hunt

## Key Design Decisions

- **Partial failure handling**: If LLM fails for a session, it still gets archived (raw file moved). Toast shows "N archived (M summarized)" so user knows some failed.
- **No compaction model**: If `compaction_model` is None, "Compact Now" archives without summarization (current behavior). "Re-compact" is disabled.
- **Label convention**: Summaries use `"session/{uuid}-summary"`, raw uses `"session/{uuid}"`. This lets FTS5 distinguish them.
- **restore_from_archive**: Already handles cleanup — will delete summary file when restoring a session.

## Critical Files

| File | Changes |
|------|---------|
| `crates/lr-memory/Cargo.toml` | Add async-trait dependency |
| `crates/lr-memory/src/compaction.rs` | CompactionLlm trait, rewrite compact_session, add recompact_session |
| `crates/lr-memory/src/lib.rs` | compaction_llm field, update force_compact/reindex/stats, add recompact_all |
| `crates/lr-memory/src/transcript.rs` | Fix restore_from_archive summary path |
| `crates/lr-memory/src/tests.rs` | Tests for new compaction logic |
| `src-tauri/src/main.rs` | RouterCompactionLlm impl, wire up to MemoryService |
| `src-tauri/src/ui/commands.rs` | Update stats/compact commands, add recompact_memory |
| `src/types/tauri-commands.ts` | New/updated types |
| `src/views/memory/sessions-tab.tsx` | Re-compact button, progress bars, summarized stats |
| `website/src/components/demo/TauriMockSetup.ts` | Updated mocks |

## Verification

1. `cargo test -p lr-memory` — all new + existing tests pass
2. `cargo clippy` — no warnings
3. `npx tsc --noEmit` — TypeScript types check
4. Manual: enable memory for a client, create some conversations, let sessions expire, hit Compact Now → verify summary files created in archive, raw files preserved, summary indexed
5. Manual: hit Re-compact → verify summaries regenerated
6. Manual: hit Rebuild Index → verify only summaries indexed (not raw files when summary exists)
