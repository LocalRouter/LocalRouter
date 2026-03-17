# Memory: Replace memsearch with native FTS5

## Context

memsearch depends on Milvus Lite which has a broken gRPC stack on Python 3.10 + Apple Silicon (hangs on `MilvusClient()` with `dns:///` error). This is a fundamental platform incompatibility that can't be fixed from our side. We need to drop the memsearch/Milvus dependency entirely and use our existing FTS5 infrastructure instead.

LocalRouter already has a production-ready FTS5 search system (`lr-context` crate) with BM25 ranking, Porter stemming, trigram fallback, and fuzzy correction — used for Response RAG and Catalog Compression. We'll use this for memory search too.

## What changes

**Drop**: memsearch CLI, Python dependency, Milvus, `--provider`/`--base-url`/`--api-key` args, memory_secret auth, all memsearch subprocess calls.

**Keep**: Session management, transcript writing, conversation detection, session monitor, compaction (future), virtual MCP server, per-client isolation, UI.

**Add**: Persistent per-client `ContentStore` (SQLite on disk), direct indexing of transcripts into FTS5 after each write.

## Architecture

```
Transcript Write (orchestrator)
  → append_exchange() writes markdown
  → index into per-client ContentStore (FTS5, on disk)
  → immediately searchable via MemoryRecall tool

MemoryRecall tool (virtual server)
  → ContentStore.search() — milliseconds, no external deps
  → ContentStore.read() — drill into full section
```

No Python. No subprocess. No embedding model. No network calls for search.

## Implementation

### 1. Replace CLI with ContentStore in lr-memory

**File: `crates/lr-memory/src/lib.rs`**

Replace `MemsearchCli` with a `ContentStore` per client:
```rust
pub struct MemoryService {
    pub session_manager: SessionManager,
    pub transcript: TranscriptWriter,
    config: RwLock<MemoryConfig>,
    memory_dir: PathBuf,
    /// Per-client FTS5 stores (persistent on disk)
    stores: DashMap<String, lr_context::ContentStore>,
    last_indexed: DashMap<String, Instant>,
}
```

New methods:
- `get_or_create_store(client_id) -> &ContentStore` — opens/creates SQLite DB at `memory/{client_id}/memory.db`
- `index_transcript(client_id, session_id, content)` — indexes into the client's store with label `"session/{session_id}"`
- `search(client_id, query, top_k) -> Vec<SearchResult>` — calls `store.search()`
- `read(client_id, label, offset, limit) -> ReadResult` — calls `store.read()`

**File: `crates/lr-memory/src/cli.rs`** — DELETE entirely (no more CLI wrapper)

**File: `crates/lr-memory/Cargo.toml`** — Replace memsearch deps with `lr-context = { workspace = true }`

### 2. Update MemoryService construction

No more `server_port` or `memory_secret` params:
```rust
pub fn new(config: MemoryConfig, memory_dir: PathBuf) -> Self
```

Remove from `AppState`: `memory_secret` field
Remove from auth middleware: `memory_secret` check
Remove from router: `memory-service` bypass

### 3. Update indexing flow

**Orchestrator** (`orchestrator.rs`, `orchestrator_stream.rs`):

After `append_exchange()`, instead of `index_client()` (which shelled out to memsearch):
```rust
// Index the exchange into FTS5
let label = format!("session/{}", session_id);
svc.index_transcript(&client_id, &label, &format!("{}\n\n{}", user_text, assistant_text)).await;
```

The `ContentStore` handles chunking, FTS5 insertion, and vocabulary building internally. No debouncing needed — FTS5 inserts are microseconds.

### 4. Update virtual server

**File: `crates/lr-mcp/src/gateway/virtual_memory.rs`**

`handle_tool_call` for MemoryRecall:
```rust
let results = memory_service.search(client_id, &query, top_k)?;
// Format results (same output format as before)
```

Remove: `expand()` call, `ensure_client_dir()` (stores auto-create), daemon references.

### 5. Simplify config

**File: `crates/lr-config/src/types.rs`**

Remove from `MemoryConfig`:
- `embedding_model` (no embedding needed)
- `compaction_model` (keep for future, but not used yet)
- Legacy `embedding`, `compaction`, `auto_start_daemon` fields

Keep:
- `search_top_k`, `session_inactivity_minutes`, `max_session_minutes`
- `recall_tool_name`
- `compaction_model` (for future LLM summarization — would call LocalRouter's chat endpoint directly from Rust, not via memsearch)

### 6. Simplify setup

**File: `src-tauri/src/ui/commands.rs`**

Remove: `memory_setup`, `memory_test_index`, `memory_test_search`, `memory_test_compact`, `memory_test_reset`, `memory_test_dir`, `memory_test_cli`, `MEMORY_TEST_DIR_NAME`.

New test commands (much simpler):
```rust
pub async fn memory_test_index(content: String, state: ...) -> Result<(), String> {
    let svc = get_memory_service(&state)?;
    svc.index_transcript("_test", "test", &content)?;
    Ok(())
}

pub async fn memory_test_search(query: String, state: ...) -> Result<String, String> {
    let svc = get_memory_service(&state)?;
    let results = svc.search("_test", &query, 5)?;
    // Format results
}
```

No Python, no memsearch, no temp dirs, no process management.

### 7. Update UI

**File: `src/views/memory/index.tsx`**

Info tab:
- Remove Setup section entirely (no Python/memsearch needed)
- Update "How It Works" to describe FTS5 search
- Keep tool preview, privacy warning

Settings tab:
- Remove embedding model picker (no embedding needed)
- Keep compaction model picker (future use)
- Keep tool name, search top-k, session grouping

Try It Out tab:
- Same 3 steps (Index, Search, Compact) but calls are instant
- No "memsearch not installed" errors

### 8. Per-client storage

```
~/.localrouter/memory/
└── {client_id}/
    ├── sessions/          # Markdown transcripts (source of truth)
    │   └── {session_id}.md
    ├── archive/           # Compacted originals
    │   └── {session_id}.md
    └── memory.db          # Persistent FTS5 SQLite database
```

On first search/index for a client, `ContentStore` opens `memory.db` (creates if needed). The DB persists across app restarts. If it gets corrupted, it can be rebuilt by re-indexing all markdown files in `sessions/`.

## Files to modify

| File | Action |
|------|--------|
| `crates/lr-memory/src/cli.rs` | DELETE |
| `crates/lr-memory/src/lib.rs` | Replace CLI with ContentStore, remove memory_secret |
| `crates/lr-memory/src/compaction.rs` | Update to not use CLI |
| `crates/lr-memory/Cargo.toml` | Replace deps with lr-context |
| `crates/lr-config/src/types.rs` | Simplify MemoryConfig |
| `crates/lr-server/src/state.rs` | Remove memory_secret |
| `crates/lr-server/src/middleware/auth_layer.rs` | Remove memory_secret check |
| `crates/lr-server/src/middleware/client_auth.rs` | Remove memory_secret check |
| `crates/lr-server/src/routes/helpers.rs` | Remove memory-service from is_internal_client |
| `crates/lr-mcp/src/gateway/virtual_memory.rs` | Use ContentStore search instead of CLI |
| `crates/lr-mcp-via-llm/src/orchestrator.rs` | Call index_transcript instead of index_client |
| `crates/lr-mcp-via-llm/src/orchestrator_stream.rs` | Same |
| `src-tauri/src/main.rs` | Simplify MemoryService construction |
| `src-tauri/src/ui/commands.rs` | Replace test commands, remove setup |
| `src/views/memory/index.tsx` | Remove setup, embedding picker |
| `src/types/tauri-commands.ts` | Simplify MemoryConfig |

## Verification

1. `cargo build` — compiles with no memsearch/Python deps
2. Try It Out: Index text → instant. Search → instant results. No setup needed.
3. MCP via LLM session: enable memory → have conversation → MemoryRecall returns results
4. Restart app → memories still searchable (persistent SQLite)
5. Cross-platform: works on macOS, Linux, Windows — no Python, no gRPC, no Milvus

---

## Future: Rust-native vector search (effort estimate)

When we want semantic search (find conceptually related content even with different words):

**Candidate libraries:**
- **usearch** — Rust bindings, ~1MB binary, HNSW index, supports multiple distance metrics. MIT license.
- **hora** — Pure Rust approximate nearest neighbor. Small, no native deps.
- **qdrant** (embedded mode) — Full-featured vector DB with Rust API. Heavier.
- **lancedb** — Rust-native, columnar storage + vector index. Good for persistence.

**What's needed:**
1. Embedding model: Need a Rust-native inference engine (candle, ort) to run a small model (~30MB ONNX) in-process. Similar to how lr-compression already uses candle for LLMLingua-2.
2. Vector storage: One of the above libraries for persistent index.
3. Hybrid search: Combine FTS5 (keyword) + vector (semantic) results with reciprocal rank fusion.

**Effort estimate**: ~2-3 weeks for a developer familiar with the codebase.
- Week 1: Embed a small model via candle/ort, generate embeddings in Rust
- Week 2: Integrate vector index (usearch/hora), hybrid search with FTS5
- Week 3: Testing, benchmarking, UI for model management

**This is additive** — FTS5 stays as the keyword search layer, vector search adds semantic understanding on top. The architecture we build now (persistent ContentStore per client) carries forward.
