# Memory Compaction Visibility & Controls

## Context

The Indexed Conversation Memory system has a compaction mechanism that archives expired session files from `sessions/` to `archive/`. Currently there's no visibility into this process - users can't see how many sessions are pending compaction, trigger it manually, or rebuild the FTS5 index. This change adds stats display, force-compact, and reindex capabilities to the Sessions tab.

## Plan

### Step 1: Add `active_session_path` to SessionManager

**File:** `crates/lr-memory/src/session_manager.rs`

Add a method to expose the active session file path for a client:

```rust
pub fn active_session_path(&self, client_id: &str) -> Option<PathBuf> {
    self.active_sessions.get(client_id).map(|s| s.file_path.clone())
}
```

### Step 2: Add compaction methods to MemoryService

**File:** `crates/lr-memory/src/lib.rs`

Add structs and methods:

```rust
#[derive(Debug, Clone, serde::Serialize)]
pub struct CompactionStats {
    pub active_sessions: usize,
    pub pending_compaction: usize,
    pub archived_sessions: usize,
    pub indexed_sources: usize,
    pub total_lines: usize,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct CompactResult {
    pub archived_count: usize,
}
```

- `get_compaction_stats(&self, client_id: &str) -> Result<CompactionStats, String>`: Count `.md` files in `sessions/` and `archive/`, cross-reference with `active_session_path()`, get indexed source stats from `list_sources()`.

- `force_compact(&self, client_id: &str) -> Result<CompactResult, String>`: Iterate `.md` files in `sessions/`, skip the active session file, call `compact_session()` for each. Does NOT require `compaction_model` to be set.

- `reindex(&self, client_id: &str, progress_fn: impl Fn(usize, usize)) -> Result<usize, String>`: Remove store from cache, delete `memory.db`, collect all `.md` files from both dirs, recreate store, index each file with progress callback.

### Step 3: Add Tauri commands

**File:** `src-tauri/src/ui/commands.rs` (after line ~4917)

Three new commands:
- `get_memory_compaction_stats(client_id)` → `CompactionStatsResult`
- `force_compact_memory(client_id)` → `ForceCompactResult`
- `reindex_client_memory(client_id, app)` → spawns async task, emits `memory-reindex-progress` / `memory-reindex-complete` / `memory-reindex-failed` events via `app.emit()`

### Step 4: Register commands

**File:** `src-tauri/src/main.rs` (after line 2032)

Add: `get_memory_compaction_stats`, `force_compact_memory`, `reindex_client_memory`

### Step 5: TypeScript types

**File:** `src/types/tauri-commands.ts`

Add interfaces: `CompactionStatsResult`, `ForceCompactResult`, param types, event payload types.

### Step 6: Update Sessions tab UI

**File:** `src/views/memory/sessions-tab.tsx`

Add a **Compaction Status** card between Client Info and Search cards:

- **Stats grid** (4 items): Active Sessions (green), Pending Compaction (amber if >0), Archived (blue), Indexed Sources
- **"Compact Now" button**: `AlertDialog` confirmation, disabled when `pending_compaction === 0`, calls `force_compact_memory`
- **"Rebuild Index" button**: `AlertDialog` confirmation, calls `reindex_client_memory`, shows progress (X/Y files) via `listenSafe` for Tauri events
- Load stats when client selected, refresh after compact/reindex/clear

### Step 7: Demo mocks

**File:** `website/src/components/demo/TauriMockSetup.ts`

Add mock handlers for all three new commands.

### Step 8: Tests

**File:** `crates/lr-memory/src/tests.rs`

Test `active_session_path`, `get_compaction_stats`, `force_compact`, `reindex`.

### Step 9: Final review

1. Plan review — check all locations updated
2. Test coverage review — edge cases (empty dirs, no sessions, concurrent access)
3. Bug hunt — race conditions, file I/O errors during reindex, DashMap ref lifetimes

## Critical Files

- `crates/lr-memory/src/session_manager.rs` — add `active_session_path()`
- `crates/lr-memory/src/lib.rs` — add stats/compact/reindex methods + structs
- `crates/lr-memory/src/compaction.rs` — reuse existing `compact_session()`
- `src-tauri/src/ui/commands.rs` — 3 new Tauri commands
- `src-tauri/src/main.rs` — register commands (line ~2032)
- `src/types/tauri-commands.ts` — TypeScript interfaces
- `src/views/memory/sessions-tab.tsx` — compaction stats card + buttons
- `src/hooks/useTauriListener.ts` — reuse `listenSafe` for progress events
- `website/src/components/demo/TauriMockSetup.ts` — demo mocks

## Verification

1. `cargo test -p lr-memory` — unit tests pass
2. `cargo clippy && cargo fmt` — no warnings
3. `npx tsc --noEmit` — TypeScript types compile
4. Manual: Open Memory → Sessions tab → select a client → verify stats card shows correct counts
5. Manual: Click "Compact Now" → verify pending count drops, archived count rises
6. Manual: Click "Rebuild Index" → verify progress shows, index rebuilds correctly, search still works
