# Unify Indexing: Shared Tools, Vector Search Everywhere, Compression Add-on

## Context

Currently there are **two separate indexing systems** that the LLM interacts with:

1. **Context Management** (`_context` virtual server): `IndexSearch` + `IndexRead` tools — indexes MCP catalogs and tool responses into an **in-memory per-session** ContentStore
2. **Memory** (`_memory` virtual server): `MemorySearch` + `MemoryRead` tools — indexes conversation transcripts into a **persistent per-client** ContentStore on disk

The user's mental model: there should be **one indexing system** with one set of search/read tools. Memory is just another thing that gets indexed. Vector search and compression should be opt-in add-ons that apply to any indexed content.

### Current Problems
- LLM sees 4 tools (IndexSearch, IndexRead, MemorySearch, MemoryRead) when it should see 2
- Vector search (EmbeddingService) only attached to Memory stores, not context_mode stores
- LLMLingua-2 compression exists but isn't usable as an indexing add-on
- Two completely separate virtual servers with duplicated search/read logic

### Architectural Constraint
Memory stores are **persistent** (per-client SQLite on disk, survives restarts). Context management stores are **ephemeral** (in-memory, per-session). They **cannot share a single ContentStore**. But we can make them share the same **tools** by having a single virtual server that federates search across both stores.

## Plan

### Phase 1: Merge Memory into Context Management's tools

**Goal**: Single set of tools (`IndexSearch` + `IndexRead`) that searches both ephemeral session content AND persistent memory.

**How it works**:
- Remove the `_memory` virtual server entirely
- Extend `ContextModeSessionState` with an optional `memory_store: Option<Arc<ContentStore>>` reference (the client's persistent memory store)
- `IndexSearch` handler: search **both** stores, merge results (interleave by rank, dedup), return unified results
- `IndexRead` handler: try the session store first, fall back to memory store (source labels are unique — memory uses `session/{id}`, catalog uses `mcp/{slug}/...`)
- Memory's summary fallback (listing sources when nothing found) stays — just moves into the context_mode handler

**Source label convention**:
- `mcp/{server}/{type}/{name}` — catalog entries (existing)
- `{tool_name}:{run_id}` — tool responses (existing)
- `memory/{session_id}` — conversation transcripts (from Memory)

**Changes**:

| File | Change |
|------|--------|
| `crates/lr-mcp/src/gateway/virtual_memory.rs` | **DELETE** — functionality moves into context_mode |
| `crates/lr-mcp/src/gateway/context_mode.rs` | Add `memory_store` to session state; extend search/read handlers to federate across both stores; add memory source listing for summary fallback |
| `crates/lr-mcp/src/gateway/context_mode.rs` | Update `create_session_state` to accept memory store reference; update instructions to mention memory |
| `src-tauri/src/main.rs` | Stop registering `_memory` virtual server; instead pass MemoryService reference to context_mode so it can get per-client stores |
| `crates/lr-mcp/src/gateway/mod.rs` | Remove virtual_memory module |
| `crates/lr-config/src/types.rs` | Remove `recall_tool_name` from MemoryConfig (tools are now IndexSearch/IndexRead); keep `search_tool_name`/`read_tool_name` in ContextManagementConfig as the single source |

**Federated search logic** (pseudo):
```rust
fn handle_search(session_store, memory_store, query, limit, source) {
    let mut all_results = session_store.search_combined(query, queries, limit, source)?;

    if let Some(mem) = memory_store {
        let mem_results = mem.search_combined(query, queries, limit, source)?;
        // Merge: interleave hits by rank, dedup by (source, line_start, line_end)
        for (i, sr) in all_results.iter_mut().enumerate() {
            if let Some(mr) = mem_results.get(i) {
                sr.hits.extend(mr.hits.iter().cloned());
                sr.hits.sort_by(|a, b| a.rank.partial_cmp(&b.rank).unwrap());
                sr.hits.truncate(limit);
            }
        }
    }

    // If no hits at all and memory exists, return summary fallback
    if no_hits && memory_store.is_some() {
        return build_memory_summary_fallback(memory_service, client_id);
    }
}
```

### Phase 2: Vector search for context_mode stores

**Goal**: Attach EmbeddingService to in-memory session stores too, so catalog search benefits from semantic matching.

**Changes**:

| File | Change |
|------|--------|
| `crates/lr-mcp/src/gateway/context_mode.rs` | Store `Option<Arc<EmbeddingService>>` in the virtual server; pass to `ContentStore::new()` → `set_embedding_service()` in `create_session_state()` |
| `src-tauri/src/main.rs` | Pass EmbeddingService reference when creating ContextModeVirtualServer |
| `crates/lr-mcp/src/gateway/context_mode.rs` | After batch-indexing catalog entries, call `store.rebuild_vectors()` |

**Important**: For ephemeral session stores with ~50-200 catalog chunks, embedding adds ~50-200ms one-time cost. Worth it because LLMs search with intent-based queries ("find the tool that reads files") where FTS5 misses but vectors hit.

### Phase 3: LLMLingua-2 compression as indexing add-on

**Goal**: Allow compressing specific sources or collections before indexing, reducing both stored content size and what the LLM reads via `IndexRead`.

**How it works**:
- Add an optional `compression_service` to ContentStore (similar to how embedding_service is attached)
- New method: `index_compressed(label, content, rate)` — runs LLMLingua-2 on content before chunking+indexing
- The compressed version is what gets stored and returned by `IndexRead` — extractive compression keeps exact tokens, zero hallucination risk
- Original content is NOT stored (compression is lossy — user must opt in per-source)

**Use cases**:
- Catalog entries: compress verbose tool descriptions to save context
- Tool responses: compress large responses before indexing (better than simple truncation)
- Memory transcripts: optionally compress old sessions to save FTS5 storage

**Changes**:

| File | Change |
|------|--------|
| `crates/lr-context/Cargo.toml` | Add optional `lr-compression` dep behind `compression` feature |
| `crates/lr-context/src/lib.rs` | Add `set_compression_service()`, `index_compressed(label, content, rate)` method |
| `crates/lr-mcp/src/gateway/gateway_tools.rs` | In `maybe_compress_response()`, optionally use LLMLingua-2 instead of simple truncation |
| `crates/lr-config/src/types.rs` | Add `use_llm_compression: bool` to ContextManagementConfig (default: false, opt-in) |

### Phase 4: UI updates

| File | Change |
|------|--------|
| `src/views/memory/index.tsx` | Remove tool preview section (tools are now IndexSearch/IndexRead from context management). Update "How it works" to explain memory is part of the unified indexing. |
| `src/views/indexing/index.tsx` | Update to show unified tool names (IndexSearch/IndexRead). Add note that memory sources are included in search results. |
| `src/constants/features.ts` | Remove `recall_tool_name` references |
| `src/types/tauri-commands.ts` | Remove `recall_tool_name` from MemoryConfig |

## Key Design Decisions

1. **Memory labels prefixed with `memory/`** — unique namespace, won't collide with `mcp/` or tool responses
2. **Federated search, not merged stores** — memory is persistent, session is ephemeral, they can't share a single SQLite. But the tool handler searches both transparently.
3. **Catalog activation only for catalog sources** — the `catalog_sources` HashMap check already ensures only `mcp/...` labeled hits trigger activation. Memory hits pass through without side effects.
4. **Compression is opt-in per use** — `index_compressed()` is a separate method, not automatic. The default `index()` stays unchanged.
5. **Vector search attached at store creation** — both session and memory stores get it if EmbeddingService is available.

## Verification

1. **Single tool set**: LLM sees only IndexSearch + IndexRead (not MemorySearch/MemoryRead)
2. **Federated search**: `IndexSearch(queries: ["past auth decisions"])` returns memory hits alongside catalog hits
3. **IndexRead works for both**: `IndexRead(label: "memory/abc123")` reads memory; `IndexRead(label: "mcp/github/tool/...")` reads catalog
4. **Vector search in catalog**: Semantic query "find the email tool" matches `compose_message` tool
5. **Compression add-on**: `index_compressed()` produces smaller indexed content, IndexRead returns compressed version
6. **Graceful degradation**: Without embedding model, FTS5-only everywhere. Without compression model, simple truncation.
7. **Summary fallback**: When no results found and memory exists, lists available memory sources
