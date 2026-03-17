# Unify lr-context: One Crate, Optional Features, Three Consumers

## Context

`lr-context::ContentStore` is already the shared crate for FTS5 search. We recently added optional vector search behind a feature flag. The three indexing consumers (Catalog, Response, Memory) each keep their own tools and virtual servers — but they should all use the same `ContentStore` capabilities and produce the same output format.

**What needs to change**: ContentStore needs to support optional compression (LLMLingua-2), and each consumer should opt into the features it needs. Vector search also needs to be wired into catalog's session store.

## ContentStore Feature Matrix

| Capability | Catalog Indexing | Response Indexing | Memory |
|-----------|:---:|:---:|:---:|
| FTS5 text search | yes | yes | yes |
| Vector search (embeddings) | yes | no | yes |
| LLMLingua-2 compression | no | no | optional |
| Persistence (SQLite on disk) | no (in-memory) | no (in-memory) | yes |
| Source labels | `mcp/{server}/...` | `{tool}:{run_id}` | `session/{id}` |
| Output format | line-numbered | line-numbered | line-numbered |

Each consumer creates its own `ContentStore` and opts in:
```rust
// Catalog: in-memory + vector
let store = ContentStore::new()?;
store.set_embedding_service(embedding_svc.clone());

// Response: in-memory, text-only
let store = ContentStore::new()?;
// (no embedding service, no compression)

// Memory: persistent + vector + optional compression
let store = ContentStore::open(&db_path)?;
store.set_embedding_service(embedding_svc.clone());
store.set_compression_service(compression_svc.clone()); // opt-in
```

## Implementation

### Phase 1: Add optional compression to ContentStore

Add LLMLingua-2 as an optional feature on ContentStore, mirroring how vector search was added.

**`crates/lr-context/Cargo.toml`**:
```toml
[features]
default = []
vector = ["lr-embeddings"]
compression = ["lr-compression"]
```

**`crates/lr-context/src/lib.rs`** — new methods:
```rust
#[cfg(feature = "compression")]
pub fn set_compression_service(&self, service: Arc<lr_compression::CompressionService>);

/// Index content with optional LLMLingua-2 compression applied first.
/// Only compresses if compression service is attached AND loaded.
/// Falls back to regular index() if compression unavailable.
#[cfg(feature = "compression")]
pub fn index_compressed(
    &self,
    label: &str,
    content: &str,
    rate: f32,
) -> Result<IndexResult, ContextError>;
```

The `index_compressed` method:
1. If compression service available and loaded → compress text via `compress_text(content, rate, false)`
2. Index the compressed result (extractive, keeps exact tokens)
3. FTS5 + optional vector indexing on the compressed content
4. If compression unavailable → falls back to regular `index()`

**Files**:

| File | Change |
|------|--------|
| `crates/lr-context/Cargo.toml` | Add optional `lr-compression` dep behind `compression` feature |
| `crates/lr-context/src/lib.rs` | Add `compression_service` field (feature-gated), `set_compression_service()`, `index_compressed()` |

### Phase 2: Wire vector search into catalog session stores

Currently only Memory's stores get the `EmbeddingService`. The context_mode session stores need it too for semantic catalog search.

**Note**: Catalog and Response currently share one session `ContentStore`. Since we want vector for catalog but not response, and splitting stores is a large refactor, we'll attach vector to the shared store. Response entries also get vector search — this doesn't hurt (only helps), and we can revisit splitting stores later if needed.

**Files**:

| File | Change |
|------|--------|
| `crates/lr-mcp/src/gateway/context_mode.rs` | Add `embedding_service: Option<Arc<EmbeddingService>>` to `ContextModeVirtualServer`; in `create_session_state()`, attach to new ContentStore; call `rebuild_vectors()` after batch catalog indexing |
| `crates/lr-mcp/Cargo.toml` | Add `lr-embeddings` dep |
| `src-tauri/src/main.rs` | Pass `EmbeddingService` to `ContextModeVirtualServer::new()` |

### Phase 3: Wire optional compression into Memory

Memory can optionally compress old session transcripts using LLMLingua-2 before indexing, reducing storage and search noise.

**Files**:

| File | Change |
|------|--------|
| `crates/lr-memory/Cargo.toml` | Add `lr-compression` dep; enable `compression` feature on lr-context |
| `crates/lr-memory/src/lib.rs` | Accept optional `Arc<CompressionService>` in constructor; in `index_transcript()`, use `index_compressed()` if compression enabled in config |
| `crates/lr-config/src/types.rs` | Add `compress_transcripts: bool` + `compression_rate: f32` to `MemoryConfig` (default: false, 0.5) |
| `src-tauri/src/main.rs` | Pass CompressionService to MemoryService if available |

### Phase 4: Ensure consistent output (already the case, verify)

Both MemorySearch/MemoryRead and IndexSearch/IndexRead already use the same:
- `lr_context::format_search_results()` for search output
- `ReadResult::to_string()` for read output (line-numbered `cat -n` format)
- Same `SearchHit` struct with `source`, `title`, `content`, `line_start`, `line_end`

**Verify**: The Memory "Try It Out" tab should show the same line-numbered format as the Response RAG "Try It Out" tab. If there are UI differences in how they display results, align them.

## Files to Modify (all phases)

| File | Change |
|------|--------|
| `crates/lr-context/Cargo.toml` | Add `compression = ["lr-compression"]` feature |
| `crates/lr-context/src/lib.rs` | Add compression_service field, `set_compression_service()`, `index_compressed()` |
| `crates/lr-mcp/Cargo.toml` | Add lr-embeddings dep |
| `crates/lr-mcp/src/gateway/context_mode.rs` | Accept + attach EmbeddingService to session stores |
| `crates/lr-memory/Cargo.toml` | Enable compression feature on lr-context, add lr-compression dep |
| `crates/lr-memory/src/lib.rs` | Accept optional CompressionService; use index_compressed() when enabled |
| `crates/lr-config/src/types.rs` | Add `compress_transcripts` + `compression_rate` to MemoryConfig |
| `src-tauri/src/main.rs` | Pass EmbeddingService to ContextModeVirtualServer; pass CompressionService to MemoryService |

## Verification

1. **Catalog vector search**: Connect an MCP server with many tools → IndexSearch with semantic query ("find the email sender") matches `compose_message` tool
2. **Response text-only**: Large tool response gets indexed → IndexSearch with keyword query finds it (vector also works since store is shared, but primary path is FTS5)
3. **Memory vector + text**: Index a transcript about PostgreSQL → MemorySearch for "SQL database for login" finds it via vector match
4. **Memory compression**: Enable `compress_transcripts`, index a long transcript → stored content is compressed, IndexRead returns compressed version, search still works on compressed tokens
5. **Graceful degradation**: Without embedding model → FTS5 only everywhere. Without compression model → regular indexing.
6. **Same output format**: All three features produce line-numbered search results and `cat -n` style read output
