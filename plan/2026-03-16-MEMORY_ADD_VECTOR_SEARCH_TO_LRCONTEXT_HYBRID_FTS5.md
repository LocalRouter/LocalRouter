# Memory: Add vector search to lr-context (hybrid FTS5 + embeddings)

## Context

FTS5 keyword search works well for exact matches, but misses semantic relationships — searching "SQL database for user login" won't find "We chose PostgreSQL for the authentication service." We add vector search as an **optional addon** inside lr-context's `ContentStore`, so ALL consumers get hybrid search transparently:

- **MemoryRecall** (conversation memory) — biggest win, natural language recall
- **IndexSearch** (MCP context management) — helps find tools/resources by intent ("find the email sender tool" matches `compose_message`)
- **Response RAG** — semantic matching on indexed content

No new crate for search logic — just add optional vector support to ContentStore. A small `lr-embeddings` module handles the ML model (BertModel via candle).

**Key requirement**: Don't persist the vector index. Only persist markdown. Rebuild on demand. Optionally cache raw embeddings (content_hash → Vec<f32>) to avoid re-computing.

## Architecture

```
ContentStore (lr-context)
├── FTS5 search (always available, keyword matching)
├── Optional: vector search (when EmbeddingService attached)
│   ├── In-memory Vec<VectorEntry> per store instance
│   ├── Cosine similarity (brute-force, FLAT like memsearch)
│   └── RRF merge with FTS5 results (k=60)
└── Transparent to callers — search() auto-upgrades

EmbeddingService (lr-embeddings crate)
├── BertModel (all-MiniLM-L6-v2, 384 dims, ~80MB)
├── Candle inference (Metal/CUDA/CPU)
├── HuggingFace Hub download
└── Shared across all ContentStore instances
```

### How it works

```
Caller creates ContentStore (existing API, unchanged)
  ↓
Optionally: store.set_embedding_service(Arc<EmbeddingService>)
  ↓
store.index("label", content)
  → FTS5 indexing (always)
  → If embedding service: chunk → embed → store vectors in-memory
  ↓
store.search(queries, limit, source)
  → FTS5 BM25 search (always)
  → If embedding service: embed query → cosine search → RRF merge
  → Returns same SearchResult type (transparent upgrade)
```

### Performance expectations

| Operation | Scale | Expected Time |
|-----------|-------|---------------|
| Embed 1 chunk (~500 chars) | — | ~1ms Metal, ~5ms CPU |
| Embed 100 chunks (full rebuild) | — | ~100ms Metal, ~500ms CPU |
| Vector search 1K chunks | cosine scan | <1ms |
| Vector search 10K chunks | cosine scan | <5ms |
| RRF merge | 2 result lists | <0.1ms |

**For MCP indexing**: Typical session has 50-200 chunks of tool definitions, resource schemas, prompts. Embedding adds ~50-200ms one-time cost on index. Worth it because LLMs often search with intent-based queries ("find the tool that reads files") where FTS5 misses but vectors hit. Since it's optional and transparent, zero cost if model isn't loaded.

**For Memory**: Conversations are ~5-50 chunks per session. Embedding is instant. Biggest win because natural language recall benefits most from semantic search.

## Implementation

### Phase 1: `lr-embeddings` crate (ML inference only)

Small crate, single responsibility: load model, generate embeddings.

**`crates/lr-embeddings/Cargo.toml`**:
```toml
[dependencies]
lr-utils = { workspace = true }
tokenizers = { workspace = true }
hf-hub = { workspace = true }
safetensors = { workspace = true }
tracing = { workspace = true }
tokio = { workspace = true }
parking_lot = { workspace = true }
once_cell = { workspace = true }
tauri = { workspace = true }

[target.'cfg(target_os = "macos")'.dependencies]
candle-core = { version = "0.8", features = ["metal"] }
candle-nn = { version = "0.8", features = ["metal"] }
candle-transformers = { version = "0.8", features = ["metal"] }

[target.'cfg(not(target_os = "macos"))'.dependencies]
candle-core = "0.8"
candle-nn = "0.8"
candle-transformers = "0.8"
```

**`src/model.rs`** — `SentenceEmbedder`:
- Follow `lr-compression/src/model.rs` pattern exactly
- `BertModel::load()` from SafeTensors, `select_device()` (Metal→CUDA→CPU)
- Config: hidden_size=384, num_layers=6, num_heads=12, intermediate_size=1536, vocab_size=30522
- Forward: tokenize → BertModel.forward() → mean pooling → L2 normalize → `Vec<f32>`
- `embed(&self, text: &str) -> Result<Vec<f32>>`
- `embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>>`

**`src/downloader.rs`**:
- Mirror `lr-compression/src/downloader.rs` (HF Hub, retries, progress events)
- Repo: `sentence-transformers/all-MiniLM-L6-v2`
- Files: `model.safetensors`, `tokenizer.json`
- Dir: `{config_dir}/embeddings/all-MiniLM-L6-v2/`

**`src/lib.rs`** — `EmbeddingService`:
```rust
pub struct EmbeddingService {
    model: Arc<RwLock<Option<SentenceEmbedder>>>,
    model_dir: PathBuf,
}

impl EmbeddingService {
    pub fn new(config_dir: &Path) -> Self;
    pub fn is_downloaded(&self) -> bool;
    pub async fn download(...) -> Result<()>;
    pub async fn ensure_loaded(&self) -> Result<()>;
    pub fn embed(&self, text: &str) -> Result<Vec<f32>>;
    pub fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>>;
    pub fn dimension(&self) -> usize; // 384
}
```

### Phase 2: Add vector search to lr-context ContentStore

**`crates/lr-context/Cargo.toml`** — add optional dep: `lr-embeddings = { workspace = true, optional = true }`

**`crates/lr-context/src/lib.rs`** — extend ContentStore:

New internal storage (alongside existing FTS5):
```rust
pub struct ContentStore {
    conn: Arc<Mutex<Connection>>,
    // NEW: optional vector search layer
    embedding_service: Option<Arc<lr_embeddings::EmbeddingService>>,
    vector_entries: Arc<Mutex<Vec<VectorEntry>>>,
}

struct VectorEntry {
    source: String,
    title: String,
    content: String,
    embedding: Vec<f32>,
    line_start: usize,
    line_end: usize,
}
```

New public methods:
```rust
impl ContentStore {
    /// Attach an embedding service to enable hybrid search.
    /// Can be called after construction. Existing indexed content is NOT
    /// retroactively embedded — call rebuild_vectors() for that.
    pub fn set_embedding_service(&self, service: Arc<EmbeddingService>);

    /// Whether vector search is available (embedding service attached and loaded)
    pub fn has_vector_search(&self) -> bool;

    /// Rebuild vector index from all currently indexed FTS5 content.
    /// Call this after attaching embedding service to an existing store.
    pub fn rebuild_vectors(&self) -> Result<(), ContextError>;
}
```

Modify existing methods:
- **`index()`**: After FTS5 indexing, if embedding service attached → chunk → embed → add to `vector_entries`
- **`search()`**: If embedding service attached → run vector search alongside FTS5 → RRF merge. If not → FTS5 only (unchanged behavior)
- **`delete()`**: Also remove corresponding vector entries

**`crates/lr-context/src/chunk.rs`** — make `pub`:
- `chunk_content()` — currently `pub(crate)`, needs to be `pub` for external consumers
- `Chunk` type — same

**`crates/lr-context/src/hybrid.rs`** — new file:
```rust
/// Reciprocal Rank Fusion merge of FTS5 and vector results.
/// k=60 (standard RRF parameter, same as memsearch).
pub fn rrf_merge(
    fts_hits: &[SearchHit],
    vector_hits: &[VectorSearchHit],
    limit: usize,
) -> Vec<SearchHit>
```

### Phase 3: Wire up consumers

**Memory** (`crates/lr-memory/src/lib.rs`):
- `MemoryService` already holds `DashMap<String, ContentStore>`
- Accept `Option<Arc<EmbeddingService>>` in constructor
- When creating stores via `get_or_create_store()`, call `store.set_embedding_service()` if available
- `search()` method unchanged — ContentStore internally does hybrid when available

**MCP context management** (`crates/lr-mcp/src/gateway/context_mode.rs`):
- `create_session_state()` creates `ContentStore::new()` (in-memory)
- Optionally attach embedding service if available in AppState
- IndexSearch tool gets semantic search transparently

**App startup** (`src-tauri/src/main.rs`):
- Create `EmbeddingService` and store in `AppState`
- Pass to `MemoryService` constructor
- Make available to MCP gateway for context management stores

### Phase 4: Config & UI

**`crates/lr-config/src/types.rs`**:
- Add `vector_search_enabled: bool` to `MemoryConfig` (default: true)

**`src-tauri/src/ui/commands.rs`** — new commands:
- `embedding_status()` → `{ downloaded, loaded, model_name, model_size_mb }`
- `embedding_download()` → triggers download with progress events

**`src/views/memory/index.tsx`** — Settings tab:
- Add "Semantic Search" section with model download button + status
- Follow compression model download pattern

**`src/types/tauri-commands.ts`**: Add `EmbeddingStatus` type
**`website/.../TauriMockSetup.ts`**: Add mock handlers

## Files to modify

| File | Action |
|------|--------|
| `Cargo.toml` (root) | Add lr-embeddings to workspace |
| `crates/lr-embeddings/` | **CREATE** — model.rs, downloader.rs, lib.rs |
| `crates/lr-context/Cargo.toml` | Add optional lr-embeddings dep |
| `crates/lr-context/src/lib.rs` | Add vector_entries, set_embedding_service(), modify search()/index() |
| `crates/lr-context/src/chunk.rs` | Make chunk_content() and Chunk pub |
| `crates/lr-context/src/hybrid.rs` | **CREATE** — RRF merge logic |
| `crates/lr-memory/Cargo.toml` | Add lr-embeddings dep |
| `crates/lr-memory/src/lib.rs` | Accept EmbeddingService, pass to ContentStores |
| `crates/lr-config/src/types.rs` | Add vector_search_enabled to MemoryConfig |
| `src-tauri/src/main.rs` | Create EmbeddingService, wire into AppState |
| `src-tauri/src/ui/commands.rs` | Add embedding_status/download commands |
| `src/types/tauri-commands.ts` | Add EmbeddingStatus type |
| `src/views/memory/index.tsx` | Add embedding model download UI |
| `website/.../TauriMockSetup.ts` | Add mock handlers |

## Key patterns to follow

| Pattern | Source File |
|---------|------------|
| BertModel loading + forward pass | `crates/lr-compression/src/model.rs` |
| Device selection (Metal/CUDA/CPU) | `crates/lr-compression/src/model.rs:362` |
| HF Hub download + progress | `crates/lr-compression/src/downloader.rs` |
| ContentStore persistent SQLite | `crates/lr-context/src/lib.rs:86` (`open()`) |
| Markdown chunking | `crates/lr-context/src/chunk.rs:766` (`chunk_content()`) |
| Per-client DashMap stores | `crates/lr-memory/src/lib.rs` |
| Optional service in AppState | `crates/lr-server/src/state.rs` (compression_service pattern) |

## Verification

1. **Unit tests (lr-embeddings)**: model loads, embed returns 384-dim L2-normalized vector, deterministic output
2. **Unit tests (lr-context)**: hybrid search returns RRF-merged results; FTS5-only fallback when no embedding service
3. **Semantic test**: Index "We chose PostgreSQL for authentication" → search "SQL database for user login" → hybrid finds it, FTS5 alone misses it
4. **MCP test**: IndexSearch with semantic query finds tool by intent description
5. **Graceful degradation**: Without model downloaded, all search works exactly as today (FTS5 only)
6. **Performance**: Embed 100 chunks <500ms; vector search 10K entries <5ms

## Optional future: Embedding cache

To avoid re-embedding on app restart (rebuild from markdown), add a lightweight cache table:
```sql
CREATE TABLE embedding_cache (
    content_hash TEXT PRIMARY KEY,
    embedding BLOB NOT NULL,
    model_id TEXT NOT NULL
);
```
Store in the same `memory.db`. On rebuild: check cache by content hash → skip embedding if hit. This makes rebuilds near-instant. The cache is derived data and can be safely deleted.
