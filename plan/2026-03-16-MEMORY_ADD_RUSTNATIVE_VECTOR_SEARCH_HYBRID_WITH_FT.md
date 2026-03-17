# Memory: Add Rust-native vector search (hybrid with FTS5)

## Context

FTS5 keyword search is already implemented and working for memory recall. But keyword search misses semantic relationships — searching "SQL database for user login" won't find "We chose PostgreSQL for the authentication service." Vector search adds this semantic layer. We follow memsearch's proven architecture (hybrid dense+BM25 with RRF) but implement it entirely in Rust with no Python, no Milvus, and no external deps.

**Key user requirement**: Do NOT persist the vector index. Only persist memories as markdown files. Rebuild vectors on demand from the markdown source of truth.

## Architecture

```
Transcript Write (orchestrator)
  → append_exchange() writes markdown (source of truth)
  → index into FTS5 (keyword search, persistent SQLite)
  → embed chunks → add to in-memory vector index (ephemeral)

MemoryRecall tool (search)
  → Lazy rebuild: first search loads all markdown → chunks → embed → populate index
  → FTS5 BM25 search (keyword matches)
  → Vector cosine search (semantic matches)
  → RRF merge (k=60) → top-k results
```

```
~/.localrouter/memory/{client_id}/
├── sessions/*.md          # Source of truth (persisted)
├── archive/*.md           # Compacted originals (persisted)
├── memory.db              # FTS5 index (persistent SQLite)
└── embeddings.db          # Embedding cache only (content_hash → Vec<f32>)
                           # Derived, deletable, speeds up rebuild
```

## Key Decisions

### 1. Embedding model: `sentence-transformers/all-MiniLM-L6-v2`

- 384 dimensions, ~80MB SafeTensors download
- MiniLM = BERT architecture — candle's `BertModel` already proven in lr-compression
- Small download, fast inference (~1ms/chunk on Metal)
- De facto standard for lightweight sentence embeddings

### 2. Vector index: brute-force cosine in-memory (no persistence)

- Simple `Vec<VectorEntry>` per client, rebuilt on demand
- memsearch also uses FLAT index (exact NN, not approximate) — same approach
- <10K chunks per client typical → cosine scan takes <1ms
- No new dependency (no usearch, no hora)

### 3. Chunking: reuse existing lr-context chunks

- Same chunks for FTS5 AND vectors — one chunk = one FTS5 entry = one embedding
- lr-context's `chunk_content()` is already markdown-aware (heading hierarchy, code blocks)
- Need to make `chunk_content()` and `Chunk` type `pub` (currently `pub(crate)`)

### 4. Hybrid search: RRF (Reciprocal Rank Fusion, k=60)

- Same algorithm as memsearch
- `RRF_score(doc) = Σ 1/(k + rank_S(doc))` across FTS5 and vector search
- No score normalization needed — RRF uses rank positions only

### 5. Embedding cache: lightweight SQLite table

- Map `content_hash → embedding bytes` — avoids re-computing on app restart
- Stored in `embeddings.db` (separate from `memory.db` to keep concerns clean)
- Derived data — can be deleted; will be rebuilt from markdown + model
- Cache invalidated when model changes (model_id stored alongside)

### 6. Crate: new `lr-embeddings`

- Follows lr-compression pattern exactly (BertModel + SafeTensors + HF Hub download)
- Clean separation: embedding is a general capability, not memory-specific

## Implementation

### Phase 1: `lr-embeddings` crate

**New files:**

`crates/lr-embeddings/Cargo.toml`:
- candle-core/nn/transformers 0.8 (Metal on macOS, plain otherwise — same as lr-compression)
- tokenizers, hf-hub, safetensors (workspace)
- sha2 (workspace)
- tokio, tracing, parking_lot, once_cell, lr-utils

`crates/lr-embeddings/src/model.rs` — `SentenceEmbedder`:
- Follow `lr-compression/src/model.rs` pattern exactly
- `BertModel::load()` from SafeTensors (weight prefix `"bert"`)
- Config: hidden_size=384, num_layers=6, num_heads=12, intermediate_size=1536, vocab_size=30522, max_position=512
- Forward: tokenize → BertModel.forward() → **mean pooling** (not classifier) → L2 normalize
- Mean pooling: mask hidden states by attention_mask, average, normalize
- `select_device()`: Metal → CUDA → CPU (copy from lr-compression)
- `embed(&self, text: &str) -> Result<Vec<f32>>`
- `embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>>` (batched for rebuild)

`crates/lr-embeddings/src/downloader.rs`:
- Mirror `lr-compression/src/downloader.rs` exactly
- Repo: `sentence-transformers/all-MiniLM-L6-v2`
- Files: `model.safetensors`, `tokenizer.json`, `tokenizer_config.json`
- Dir: `{config_dir}/embeddings/all-MiniLM-L6-v2/`
- Progress event: `"embedding-download-progress"`

`crates/lr-embeddings/src/vector_index.rs` — `InMemoryVectorIndex`:
```rust
pub struct VectorEntry {
    pub source: String,       // "session/abc123"
    pub title: String,        // chunk heading breadcrumb
    pub content: String,      // chunk text
    pub embedding: Vec<f32>,  // 384-dim L2-normalized
    pub content_hash: String, // SHA-256 hex
}

impl InMemoryVectorIndex {
    pub fn new() -> Self;
    pub fn add(&mut self, entry: VectorEntry);
    pub fn search(&self, query_embedding: &[f32], top_k: usize) -> Vec<VectorSearchHit>;
    pub fn clear(&mut self);
    pub fn len(&self) -> usize;
}
```
- Cosine similarity = dot product (since vectors are L2-normalized)
- Sort descending, return top-k with rank

`crates/lr-embeddings/src/lib.rs` — `EmbeddingService`:
```rust
pub struct EmbeddingService {
    model: Arc<RwLock<Option<SentenceEmbedder>>>,
    model_dir: PathBuf,
}

impl EmbeddingService {
    pub fn new(config_dir: &Path) -> Self;
    pub fn is_downloaded(&self) -> bool;
    pub async fn download(progress_tx: Option<...>) -> Result<()>;
    pub async fn ensure_loaded(&self) -> Result<()>;  // download if needed, load if not loaded
    pub fn embed(&self, text: &str) -> Result<Vec<f32>>;  // blocking (run via spawn_blocking)
    pub fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>>;
}
```

### Phase 2: Embedding cache in lr-context

**File: `crates/lr-context/src/lib.rs`**

Add a separate `EmbeddingCache` type (NOT inside ContentStore — separate DB):
```rust
pub struct EmbeddingCache {
    conn: Arc<Mutex<Connection>>,
}

impl EmbeddingCache {
    pub fn open(path: &Path) -> Result<Self>;
    pub fn get(&self, content_hash: &str) -> Option<Vec<f32>>;
    pub fn store(&self, content_hash: &str, embedding: &[f32], model_id: &str);
    pub fn clear(&self);
}
```

Schema:
```sql
CREATE TABLE IF NOT EXISTS cache (
    content_hash TEXT PRIMARY KEY,
    embedding BLOB NOT NULL,         -- f32 bytes
    model_id TEXT NOT NULL,
    created_at TEXT DEFAULT (datetime('now'))
);
```

Also: make `chunk_content()` and `Chunk` type `pub` (currently `pub(crate)`) so lr-memory can call them for re-chunking markdown content during vector rebuild.

### Phase 3: Integrate into MemoryService

**File: `crates/lr-memory/Cargo.toml`** — add `lr-embeddings`, `sha2`

**File: `crates/lr-memory/src/lib.rs`**:

Add to `MemoryService`:
```rust
pub struct MemoryService {
    // ... existing fields ...
    vector_indices: DashMap<String, parking_lot::RwLock<InMemoryVectorIndex>>,
    embedding_service: Option<Arc<EmbeddingService>>,
    embedding_caches: DashMap<String, EmbeddingCache>,
    vector_built: DashMap<String, bool>,  // tracks lazy rebuild state
}
```

New methods:

`index_transcript()` — extend: after FTS5 index, if embedding service loaded:
1. Chunk the new content (lr-context's `chunk_content()`)
2. For each chunk: compute SHA-256, check embedding cache, embed if miss, store in cache
3. Add to client's `InMemoryVectorIndex`

`rebuild_vector_index(client_id)` — lazy, called on first search:
1. List all sources from FTS5 store
2. For each source: read content, re-chunk
3. For each chunk: check embedding cache, embed if miss
4. Populate `InMemoryVectorIndex`
5. Mark as built

`hybrid_search(client_id, query, top_k)`:
1. Ensure vector index built (lazy rebuild)
2. FTS5 search → ranked BM25 results
3. Embed query → vector cosine search → ranked semantic results
4. RRF merge (k=60) — match chunks by source+title
5. Return merged top-k

`search()` — modify: call `hybrid_search()` when embedding service is available and loaded, fallback to FTS5-only otherwise. This means memory works immediately (FTS5) and transparently upgrades to hybrid when the model is downloaded.

### Phase 4: Wire into app + UI

**File: `src-tauri/src/main.rs`**:
- Create `EmbeddingService` alongside `MemoryService`
- Pass as `Option<Arc<EmbeddingService>>` (None if config_dir unavailable)

**File: `src-tauri/src/ui/commands.rs`** — new commands:
- `embedding_status()` → `{ downloaded: bool, loaded: bool, model_name: str }`
- `embedding_download()` → triggers download with progress events

**File: `crates/lr-config/src/types.rs`**:
- Add to `MemoryConfig`: `vector_search_enabled: bool` (default true, serde default)

**File: `src/views/memory/index.tsx`** — Settings tab:
- Add "Embedding Model" section with download button + status indicator
- Follow the compression model download pattern from guardrails UI

**File: `src/types/tauri-commands.ts`** — add types:
- `EmbeddingStatus { downloaded: boolean; loaded: boolean; modelName: string }`

**File: `website/src/components/demo/TauriMockSetup.ts`** — add mock handlers

## Files to modify

| File | Action |
|------|--------|
| `Cargo.toml` (root) | Add lr-embeddings to workspace |
| `crates/lr-embeddings/` (entire crate) | **CREATE** — model, downloader, vector_index, lib |
| `crates/lr-context/src/lib.rs` | Add `EmbeddingCache` type, make `chunk_content()` + `Chunk` pub |
| `crates/lr-context/src/chunk.rs` | Change `pub(crate)` to `pub` on `chunk_content` and `Chunk` |
| `crates/lr-memory/Cargo.toml` | Add lr-embeddings, sha2 deps |
| `crates/lr-memory/src/lib.rs` | Add vector_indices, embedding_service, hybrid_search(), rebuild |
| `crates/lr-config/src/types.rs` | Add vector_search_enabled to MemoryConfig |
| `src-tauri/src/main.rs` | Create EmbeddingService, pass to MemoryService |
| `src-tauri/src/ui/commands.rs` | Add embedding_status, embedding_download commands |
| `src/types/tauri-commands.ts` | Add EmbeddingStatus type |
| `src/views/memory/index.tsx` | Add embedding model download UI in settings |
| `website/.../TauriMockSetup.ts` | Add mock handlers |

## Key reference files (patterns to follow)

| Pattern | Source File |
|---------|------------|
| BertModel loading + forward pass | `crates/lr-compression/src/model.rs` |
| Device selection (Metal/CUDA/CPU) | `crates/lr-compression/src/model.rs:362` (`select_device()`) |
| HF Hub download + progress | `crates/lr-compression/src/downloader.rs` |
| BERT Config struct | `crates/lr-compression/src/model.rs:317` (`model_config()`) |
| Persistent SQLite (WAL mode) | `crates/lr-context/src/lib.rs:86` (`ContentStore::open()`) |
| Markdown chunking | `crates/lr-context/src/chunk.rs:766` (`chunk_content()`) |
| Per-client DashMap stores | `crates/lr-memory/src/lib.rs` (`stores: DashMap<String, ContentStore>`) |

## Verification

1. **Unit tests (lr-embeddings)**: vector index cosine correctness, top-k ordering, empty index edge case
2. **Unit tests (lr-memory)**: RRF merge ordering with mock FTS5 + mock vector results
3. **Integration**: Index "We chose PostgreSQL for authentication" → search "SQL database for user login" → hybrid search finds it (FTS5 would miss)
4. **Performance**: Embed 100 chunks <10s on Metal; vector search 10K entries <5ms; rebuild from cache <100ms
5. **Graceful degradation**: Without model downloaded, search falls back to FTS5-only (zero behavior change from current)
6. **Restart**: App restarts → vectors gone → first search triggers lazy rebuild from markdown + embedding cache → instant subsequent searches
