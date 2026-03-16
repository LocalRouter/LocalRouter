# Unify lr-context: Vector Search for All, Benchmark, Global Settings

**Date**: 2026-03-16

## Goals

1. Move `vector_search_enabled` from `MemoryConfig` to `ContextManagementConfig` (global setting)
2. Wire `EmbeddingService` into context_mode session stores (catalog + response indexing)
3. Add a benchmark to measure vector search overhead on indexing and searching

## Architecture

```
ContentStore instance 1: Session (in-memory, ephemeral)
├── Catalog entries: mcp/{server}/tool/{name}, mcp/{server}/resource/{uri}, ...
├── Tool responses: {tool_name}:{run_id}
├── Tools: IndexSearch + IndexRead
├── FTS5: always
└── Vector: if globally enabled + model loaded

ContentStore instance 2: Memory (persistent, per-client SQLite)
├── Conversation transcripts: session/{session_id}
├── Tools: MemorySearch + MemoryRead
├── FTS5: always
└── Vector: if globally enabled + model loaded
```

Both stores share the same `EmbeddingService`. The setting is global.

## Phase 1: Global vector search setting

### Files Modified

| File | Change |
|------|--------|
| `crates/lr-config/src/types.rs` | Add `vector_search_enabled` to `ContextManagementConfig`, mark as legacy in `MemoryConfig` |
| `src-tauri/src/main.rs` | Check `context_management.vector_search_enabled` before creating EmbeddingService |
| `src/types/tauri-commands.ts` | Move `vector_search_enabled` from `MemoryConfig` → `ContextManagementConfig` |
| `src/views/memory/index.tsx` | Remove `vector_search_enabled` from default config |
| `website/src/components/demo/TauriMockSetup.ts` | Update mock data |

## Phase 2: Wire EmbeddingService into context_mode session stores

### Files Modified

| File | Change |
|------|--------|
| `crates/lr-mcp/Cargo.toml` | Add `lr-embeddings` dep, enable `lr-context/vector` feature |
| `crates/lr-mcp/src/gateway/context_mode.rs` | Accept + store EmbeddingService; attach to session ContentStore |
| `src-tauri/src/main.rs` | Pass EmbeddingService to ContextModeVirtualServer |

## Phase 3: Benchmark

### Files Created/Modified

| File | Change |
|------|--------|
| `crates/lr-context/Cargo.toml` | Add criterion bench setup |
| `crates/lr-context/benches/vector_search.rs` | **CREATE** — benchmark suite |

## Key Decisions

- `index()` already creates vectors incrementally (line 252-256 of lib.rs), so no explicit `rebuild_vectors()` needed after catalog indexing
- Keep `vector_search_enabled` in `MemoryConfig` as legacy field (`skip_serializing`) for backward compat
- EmbeddingService only created when `vector_search_enabled` is true (global toggle)
- Session stores get embedding service passed through `set_embedding_service()` at creation time
