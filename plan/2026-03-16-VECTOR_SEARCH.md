# Vector Search: Hybrid FTS5 + Embeddings for lr-context

**Date**: 2026-03-16
**Status**: Implemented (Phases 1-4)

## Summary

Added optional vector search (semantic embeddings) to `lr-context`'s `ContentStore`, enabling hybrid FTS5 + cosine similarity search with RRF merge. All consumers (Memory, MCP IndexSearch, Response RAG) get hybrid search transparently.

## Architecture

```
ContentStore (lr-context)
├── FTS5 search (always available, keyword matching)
├── Optional: vector search (when EmbeddingService attached)
│   ├── In-memory Vec<VectorEntry> per store instance
│   ├── Cosine similarity (brute-force, L2-normalized dot product)
│   └── RRF merge with FTS5 results (k=60)
└── Transparent to callers — search() auto-upgrades

EmbeddingService (lr-embeddings crate)
├── BertModel (all-MiniLM-L6-v2, 384 dims, ~80MB)
├── Candle inference (Metal/CUDA/CPU)
├── HuggingFace Hub download
└── Shared across all ContentStore instances
```

## Files Created

| File | Description |
|------|-------------|
| `crates/lr-embeddings/Cargo.toml` | New crate for sentence embeddings |
| `crates/lr-embeddings/src/lib.rs` | EmbeddingService (lifecycle management) |
| `crates/lr-embeddings/src/model.rs` | SentenceEmbedder (BERT forward pass + mean pooling) |
| `crates/lr-embeddings/src/downloader.rs` | HF Hub download with retries |
| `crates/lr-context/src/hybrid.rs` | RRF merge logic |

## Files Modified

| File | Change |
|------|--------|
| `Cargo.toml` (root) | Added lr-embeddings to workspace |
| `crates/lr-context/Cargo.toml` | Added optional lr-embeddings dep via `vector` feature |
| `crates/lr-context/src/lib.rs` | Added vector fields, set_embedding_service(), hybrid search in index/search/delete |
| `crates/lr-context/src/chunk.rs` | Made chunk_content() pub |
| `crates/lr-context/src/types.rs` | Made Chunk pub |
| `crates/lr-memory/Cargo.toml` | Added lr-embeddings + vector feature for lr-context |
| `crates/lr-memory/src/lib.rs` | Accept EmbeddingService, pass to ContentStores |
| `crates/lr-config/src/types.rs` | Added vector_search_enabled to MemoryConfig |
| `crates/lr-server/Cargo.toml` | Added lr-embeddings dep |
| `crates/lr-server/src/state.rs` | Added embedding_service to AppState |
| `src-tauri/Cargo.toml` | Added lr-embeddings dep |
| `src-tauri/src/main.rs` | Create EmbeddingService, wire into AppState + MemoryService |
| `src-tauri/src/ui/commands.rs` | Added get_embedding_status, install_embedding_model commands |
| `src/types/tauri-commands.ts` | Added EmbeddingStatus type |
| `website/.../TauriMockSetup.ts` | Added mock handlers |

## Key Design Decisions

1. **Feature-gated**: Vector search behind `lr-context/vector` feature — zero cost if not enabled
2. **In-memory vectors**: Not persisted, rebuilt from FTS5 content on store open (via rebuild_vectors)
3. **Transparent upgrade**: Same search() API — callers don't need to change
4. **Graceful degradation**: Without model, everything works as FTS5-only (unchanged)
5. **Auto-load**: If model is downloaded, loads automatically on app start
