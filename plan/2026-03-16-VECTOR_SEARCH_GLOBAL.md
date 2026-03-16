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

## Phase 3: Benchmark Results

Benchmark source: `crates/lr-context/benches/vector_search.rs`
Run with: `cargo bench -p lr-context --bench vector_search --features vector`

Measured on Apple Silicon (M-series), all-MiniLM-L6-v2 model (~80MB), Metal GPU acceleration.

### Index Latency

| Content Size | FTS5 Only | FTS5 + Vector | Overhead |
|-------------|-----------|---------------|----------|
| Small (1KB) | 611 µs | 29.0 ms | ~47x |
| Medium (10KB) | 1.50 ms | 237 ms | ~158x |
| Large (100KB) | 10.9 ms | 2.33 s | ~214x |

### Search Latency

| Content Size | FTS5 Only | FTS5 + Vector | Overhead |
|-------------|-----------|---------------|----------|
| Small (1KB) | 44.5 µs | 7.15 ms | ~161x |
| Medium (10KB) | 96.8 µs | 7.30 ms | ~75x |
| Large (100KB) | 465 µs | 7.76 ms | ~17x |

### Rebuild Vectors (retroactive embedding of existing FTS5 content)

| Entry Count | Time |
|-------------|------|
| 50 entries | 358 ms |
| 200 entries | 1.45 s |

### Cold Start (model loading)

| Operation | Time |
|-----------|------|
| `ensure_loaded()` | 32.0 ms |

### Analysis

- **Search overhead is dominated by embedding the query** (~7ms constant), not by the number of indexed chunks. This means vector search adds a fixed ~7ms regardless of content size.
- **Index overhead scales linearly with chunk count** since each chunk must be embedded. For typical MCP tool descriptions (~1KB each), the 29ms overhead is acceptable.
- **Cold start is fast** (32ms) since the model is memory-mapped from SafeTensors, not loaded into RAM.
- **Rebuild is expensive** — 200 entries takes 1.45s. This confirms the design choice to use incremental `index()` rather than bulk `rebuild_vectors()` for session stores.

## Key Decisions

- `index()` already creates vectors incrementally (line 252-256 of lib.rs), so no explicit `rebuild_vectors()` needed after catalog indexing
- Keep `vector_search_enabled` in `MemoryConfig` as legacy field (`skip_serializing`) for backward compat
- EmbeddingService only created when `vector_search_enabled` is true (global toggle)
- Session stores get embedding service passed through `set_embedding_service()` at creation time
