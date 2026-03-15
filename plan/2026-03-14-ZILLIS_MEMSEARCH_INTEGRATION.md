# Zillis Memsearch Integration Plan

**Date**: 2026-03-14
**Status**: Planning
**Goal**: Give any LLM persistent memory via Zillis memsearch, integrated through LocalRouter's MCP-via-LLM pipeline.

---

## What is Zillis Memsearch?

[Memsearch](https://github.com/zilliztech/memsearch) is an open-source Python library (MIT, v0.1.7+) by Zilliz that provides **markdown-first persistent memory** for AI agents:

- **Storage**: Memories are plain-text markdown files (source of truth), indexed into Milvus (vector DB)
- **Retrieval**: Hybrid search (dense vectors + BM25 sparse + RRF reranking)
- **Embeddings**: Pluggable — ONNX bge-m3 (local), OpenAI, Google, Voyage, Ollama
- **Backend**: Milvus Lite (in-process SQLite), Milvus Server, or Zilliz Cloud
- **Interface**: Python SDK + CLI (`memsearch index`, `memsearch search`, `memsearch watch`, `memsearch compact`)
- **No REST API**: Library only — no standalone HTTP server

### Key Constraint
Memsearch is a **Python library** with no HTTP API. LocalRouter is Rust. Integration options:
1. **Sidecar process** — Run a thin Python HTTP wrapper around memsearch
2. **Direct Milvus** — Use Milvus REST API directly from Rust, reimplement chunking/embedding
3. **MCP Server** — Zilliz has a separate [Zilliz MCP Server](https://github.com/zilliztech/zilliz-mcp-server) for Milvus access
4. **Subprocess/CLI** — Shell out to `memsearch search`/`memsearch index` commands

---

## Integration Options Analysis

### Option A: Memsearch as MCP Server (Recommended)

**Approach**: Create or use a thin MCP server wrapper around memsearch that exposes `memory_search`, `memory_store`, and `memory_compact` as MCP tools. Users configure it like any other MCP server in LocalRouter.

**Pros**:
- Zero Rust code changes needed for basic integration — just add an MCP server
- Works with existing MCP-via-LLM pipeline (tools auto-injected into LLM requests)
- Users can use memsearch with any client mode, not just MCP-via-LLM
- Familiar configuration pattern (add MCP server in UI)
- Memsearch's Claude Code plugin already works similarly

**Cons**:
- Requires Python sidecar process
- No automatic context injection (LLM must actively call the tool)
- Memory retrieval adds latency (tool call round-trip)

**Integration Points**: None needed — pure configuration. User adds memsearch MCP server, enables MCP-via-LLM on client.

### Option B: Native Memory Layer in Chat Pipeline (Most Powerful)

**Approach**: Add a memory middleware step in the chat completions pipeline that automatically:
1. Searches memsearch for relevant memories before LLM call
2. Injects retrieved memories into system prompt
3. After LLM response, optionally stores conversation summaries

**Pros**:
- Transparent to client — no tool calls needed
- Works with ALL client modes (not just MCP-via-LLM)
- Lower latency (parallel with other pipeline steps like compression, RouteLLM)
- Richer context injection (system prompt augmentation vs. tool responses)

**Cons**:
- Requires Rust changes + Python sidecar communication
- New middleware/feature to maintain
- Need to design the sidecar protocol
- System prompt injection may conflict with existing prompt compression

**Integration Points**:
- `crates/lr-server/src/routes/chat.rs` — Add parallel memory retrieval alongside guardrails/compression/RouteLLM (line ~287-323)
- `crates/lr-server/src/state.rs` — Add `MemoryService` to `AppState`
- `crates/lr-config/src/types.rs` — Add `MemoryConfig` section
- New crate: `crates/lr-memory/` — Memory service abstraction

### Option C: Hybrid — MCP Server + Automatic Injection (Best of Both)

**Approach**: Combine Options A and B:
1. Memsearch runs as an MCP server (for explicit tool use)
2. LocalRouter adds a thin memory middleware that calls the MCP memory_search tool automatically before each LLM request
3. Retrieved context is injected into the system prompt
4. LLM can also call memory tools explicitly for storing/searching

**Pros**:
- Automatic context injection (no LLM action needed for retrieval)
- Explicit tools still available for storing/managing memories
- Leverages existing MCP infrastructure
- Works with any memsearch-compatible MCP server

**Cons**:
- Most complex implementation
- Dual latency paths (auto-inject + potential tool calls)

---

## Recommended Approach: Option C (Hybrid), Phased

### Phase 1: MCP Server Integration (No LocalRouter code changes)

**Goal**: Get memsearch working as an MCP server that LLMs can use via tool calls.

**Work**:
1. Create a lightweight MCP server wrapper for memsearch (Python, using `mcp` SDK)
   - Tools: `memory_search(query, top_k)`, `memory_store(content, tags)`, `memory_list(filter)`, `memory_compact()`
   - Resources: `memory://recent`, `memory://stats`
2. Document setup: install memsearch, configure Milvus Lite, add MCP server to LocalRouter
3. Users configure client as MCP-via-LLM mode → tools auto-injected → LLM can search/store memories

**Files to create**:
- `tools/memsearch-mcp-server/` — Python MCP server wrapper
- `tools/memsearch-mcp-server/server.py` — MCP server implementation
- `tools/memsearch-mcp-server/requirements.txt` — Dependencies
- `docs/memsearch-setup.md` — Setup guide

### Phase 2: Automatic Memory Injection (LocalRouter code changes)

**Goal**: Automatically retrieve and inject relevant memories into every LLM request.

**Architecture**:
```
Client Request
    │
    ├─► Guardrails scan (parallel)
    ├─► Prompt compression (parallel)
    ├─► RouteLLM classification (parallel)
    ├─► Memory retrieval (parallel) ◄── NEW
    │
    ▼
Merge results → Provider request → LLM
```

**Work**:
1. New crate `crates/lr-memory/` — Memory service abstraction
   - `MemoryService` trait with `search(query, top_k) -> Vec<MemoryResult>`
   - `McpMemoryBackend` — calls memsearch MCP server's `memory_search` tool via gateway
   - `HttpMemoryBackend` — calls a generic HTTP memory endpoint (future-proofing)
2. Configuration in `lr-config`:
   ```rust
   pub struct MemoryConfig {
       pub enabled: bool,
       pub backend: MemoryBackend,        // Mcp { server_id } | Http { url }
       pub auto_inject: bool,             // Auto-retrieve on each request
       pub auto_inject_top_k: u32,        // How many memories to inject (default: 5)
       pub auto_inject_min_score: f32,    // Minimum relevance score (default: 0.3)
       pub auto_store: bool,              // Auto-store conversation summaries
       pub injection_position: InjectionPosition, // SystemPrompt | FirstMessage | LastMessage
   }
   ```
3. Per-client override:
   ```rust
   // In Client struct
   pub memory_enabled: Option<bool>,  // Overrides global
   pub memory_config_overrides: Option<MemoryConfigOverrides>,
   ```
4. Chat pipeline integration (`chat.rs`):
   - Spawn memory retrieval in parallel with guardrails/compression/RouteLLM
   - After retrieval, inject memories into system prompt or as a context message
   - Query = last user message content (or configurable extraction)
5. Post-response memory storage (optional):
   - After LLM response, optionally call `memory_store` with conversation summary
   - Can be async/fire-and-forget to avoid adding latency

**Files to modify**:
- `crates/lr-server/src/routes/chat.rs` — Add memory retrieval parallel task + injection
- `crates/lr-server/src/state.rs` — Add `memory_service: Option<Arc<MemoryService>>`
- `crates/lr-config/src/types.rs` — Add `MemoryConfig`, client overrides
- `crates/lr-server/src/lib.rs` — Wire up memory service initialization

**Files to create**:
- `crates/lr-memory/Cargo.toml`
- `crates/lr-memory/src/lib.rs` — Trait + types
- `crates/lr-memory/src/mcp_backend.rs` — MCP-based backend
- `crates/lr-memory/src/http_backend.rs` — HTTP-based backend (future)

### Phase 3: UI Integration

**Goal**: Configure memory settings from LocalRouter UI.

**Work**:
1. Tauri commands for memory config CRUD
2. Settings page section for memory configuration
3. Per-client memory toggle in client editor
4. Memory stats/viewer in dashboard (optional)

**Files to modify**:
- `src-tauri/src/ui/commands*.rs` — New Tauri commands
- `src/types/tauri-commands.ts` — TypeScript types
- `src/views/settings/` — Memory settings section
- `src/views/clients/` — Per-client memory toggle
- `website/src/components/demo/TauriMockSetup.ts` — Demo mocks

---

## Memory Injection Design Detail

### Query Extraction Strategy
For each incoming chat request, extract a search query:
1. **Last user message** (default) — Simple, works well for conversational use
2. **All user messages** — More context but noisier
3. **Summarized conversation** — Best quality but adds LLM call latency
4. **Custom extraction** — User-configurable prompt template

### Injection Format
```
[Memory Context]
The following memories may be relevant to this conversation:

1. [score: 0.85] Previously discussed implementing a caching layer for the API...
   Source: project-notes/2026-03-10.md

2. [score: 0.72] User prefers TypeScript over JavaScript for new projects...
   Source: preferences/coding.md

[End Memory Context]
```

### Injection Position Options
- **System prompt append** (recommended) — Append to existing system message
- **Dedicated system message** — Add as a separate system message before user messages
- **Context message** — Insert as an assistant/system message before the last user message

### Token Budget
- Memory injection should respect a configurable token budget (e.g., 2000 tokens max)
- Memories exceeding the budget are truncated by relevance score
- Budget should be subtracted from max_tokens if needed

---

## Alternative: Direct Milvus from Rust (No Python)

If avoiding the Python sidecar is important, LocalRouter could:
1. Use the [Milvus REST API](https://milvus.io/api-reference/restful/v2.5.x/About.md) directly
2. Implement markdown chunking in Rust (port memsearch's logic)
3. Call embedding APIs (OpenAI, local ONNX) from existing provider infrastructure
4. Store/retrieve vectors via Milvus HTTP endpoints

**Pros**: No Python dependency, lower latency, single binary
**Cons**: Significant Rust implementation work, must maintain parity with memsearch updates

This could be a Phase 4 optimization if the Python sidecar proves problematic.

---

## Open Questions

1. **MCP server wrapper**: Should we create our own or contribute to Zilliz's ecosystem? Their existing MCP server is for general Milvus access, not memsearch-specific.
2. **Memory scoping**: Should memories be per-client, per-user, or global? Per-client is simplest but limits cross-client knowledge sharing.
3. **Auto-store granularity**: Store every response? Only on explicit request? Periodic summaries?
4. **Conflict with prompt compression**: When both memory injection and prompt compression are active, should compression run before or after memory injection? (After = memories preserved, before = more room for memories)
5. **Privacy**: Memory storage must be local-only by default (aligns with LocalRouter's privacy policy). Zilliz Cloud backend should require explicit opt-in.

---

## Summary

| Phase | Scope | Effort | Benefit |
|-------|-------|--------|---------|
| **1** | MCP server wrapper | Small (Python only) | LLMs can use memory tools explicitly |
| **2** | Auto-injection pipeline | Medium (Rust + config) | Transparent memory for all LLMs |
| **3** | UI integration | Medium (React + Tauri) | User-friendly configuration |
| **4** | Native Rust backend | Large (optional) | No Python dependency |

Phase 1 can ship independently and provides immediate value. Phase 2 is the key differentiator — automatic memory injection that works with any LLM through any provider.
