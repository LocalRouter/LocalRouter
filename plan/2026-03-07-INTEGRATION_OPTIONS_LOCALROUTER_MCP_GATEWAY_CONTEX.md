# Integration Options: LocalRouter MCP Gateway + Context-Mode

## What is Context-Mode?

An MCP server that solves two problems for AI coding assistants:
1. **Context savings** — Sandboxes large tool outputs (98% reduction). 315 KB raw output becomes 5.4 KB via polyglot code execution in 11 languages, FTS5 indexing, and smart summarization.
2. **Session continuity** — Tracks file edits, git ops, tasks, errors in SQLite. Rebuilds state after compaction into ~2 KB snapshots with 13 event categories.

Exposes 9 MCP tools (`ctx_execute`, `ctx_batch_execute`, `ctx_search`, `ctx_index`, `ctx_fetch_and_index`, etc.). Supports Claude Code, Gemini CLI, VS Code Copilot, OpenCode, Codex CLI via platform-specific hooks that intercept tool calls pre/post execution.

---

## Architectural Relationships

The options below are organized by WHERE context-mode sits relative to the gateway:

```
A. BEHIND the gateway    — context-mode is one of many backend MCP servers
B. IN FRONT of gateway   — context-mode wraps/intercepts gateway traffic
C. INSIDE the gateway    — native Rust implementation
D. BESIDE the gateway    — peer systems sharing data/state
E. AS A LAYER            — gateway middleware processing responses
F. FEATURE EXTRACTION    — cherry-pick specific capabilities
```

---

## A. Context-Mode BEHIND the Gateway

### A1. Backend MCP Server (Plug-and-Play)

Context-mode is added as just another MCP server behind the gateway, like filesystem or GitHub servers.

```
AI Client (Claude, Cursor, etc.)
  → LocalRouter MCP Gateway (port 3625/mcp)
    → context-mode (STDIO transport, npx context-mode)
    → filesystem-server
    → github-server
    → ... other servers
```

- Gateway spawns context-mode via STDIO, namespaces its tools (`context-mode__ctx_execute`, etc.)
- Firewall rules gate dangerous operations
- Deferred loading hides ctx_* tools until searched for
- **Effort**: Very Low (config/UI only) | **Value**: High

### A2. First-Class Recommended Server

Like A1, but with deeper UI integration — a dedicated "Context-Mode" card in the MCP servers page with:
- One-click enable/disable
- Auto-detection of installed context-mode binary
- Configuration UI for context-mode settings (execution languages, index paths)
- Health status indicator using `ctx_doctor`
- Context savings dashboard using `ctx_stats`

- **Effort**: Low-Medium | **Value**: High

### A3. Auto-Provisioned Server Per Client

Each client connecting to the gateway gets its own isolated context-mode instance, so sessions/indexes don't bleed across clients.

```
Client A → Gateway → context-mode instance A (separate SQLite DB)
Client B → Gateway → context-mode instance B (separate SQLite DB)
```

- Uses gateway's per-client session system to manage instances
- Instances spin up lazily on first ctx_* tool call
- Auto-cleanup when client session expires
- **Effort**: Medium | **Value**: High (true isolation)

---

## B. Context-Mode IN FRONT of the Gateway

### B1. Context-Mode Wraps the Gateway (Hook Interception)

Context-mode's hook system intercepts ALL tool calls made by AI assistants, including those routed through LocalRouter's gateway. Context-mode sits between the AI and LocalRouter.

```
AI Client (e.g., Claude Code)
  → Context-Mode hooks (PreToolUse/PostToolUse)
    → LocalRouter MCP Gateway
      → backend MCP servers
    ← responses flow back through hooks
  ← compressed responses enter AI context
```

- Context-mode hooks intercept MCP tool calls BEFORE they reach the gateway
- Large responses from ANY backend server get automatically compressed
- Session continuity tracks all gateway interactions
- **This is how context-mode already works** with Claude Code — it hooks all tool calls
- **Effort**: Very Low (already works!) | **Value**: Very High
- **Key insight**: Users can install BOTH. Context-mode hooks + LocalRouter gateway. No code changes needed.

### B2. Context-Mode as SSE/HTTP Proxy in Front of Gateway

Context-mode acts as an HTTP proxy that sits between clients and LocalRouter's MCP endpoint.

```
AI Client
  → Context-Mode HTTP Proxy (port 3626)
    → LocalRouter Gateway (port 3625/mcp)
      → backend servers
    ← proxy compresses responses
  ← compressed response to client
```

- Context-mode would need a new HTTP proxy mode (doesn't exist today)
- Transparent to clients — they connect to context-mode's port instead of LocalRouter's
- **Effort**: High (new context-mode feature) | **Value**: High

---

## C. Context-Mode INSIDE the Gateway

### C1. Virtual MCP Server (Native Rust Reimplementation)

Reimplement context-mode's core as a `VirtualMcpServer` in Rust, running in-process like Skills/Marketplace/Coding Agents.

```
Gateway
  ├─ VirtualContextModeServer (Rust, in-process)
  │   ├─ Polyglot executor (tokio::process::Command)
  │   ├─ FTS5 store (rusqlite)
  │   └─ Session tracker
  ├─ VirtualSkillsServer
  ├─ VirtualMarketplaceServer
  └─ backend MCP servers (STDIO/SSE/WS)
```

- No Node.js dependency — pure Rust
- Shared memory, no IPC overhead
- Integrates with gateway's monitoring/metrics
- **Effort**: Very High (5k+ LOC reimplementation) | **Value**: High
- **Risk**: Must track upstream changes independently

### C2. Embedded Node.js Runtime

Bundle context-mode's JavaScript and run it in an embedded V8/QuickJS runtime within the Rust process.

- `deno_core` or `quickjs-rs` to execute context-mode's bundled JS
- No external process, but keeps the TypeScript implementation
- **Effort**: High | **Value**: Medium
- **Risk**: Compatibility issues, hard to debug

### C3. WASM Module

Compile context-mode (or key parts) to WebAssembly, run inside the gateway.

- Sandboxed execution within the Rust process
- Cross-platform by design
- **Effort**: Very High | **Value**: Medium
- **Risk**: WASM limitations (no filesystem, no subprocess spawning without WASI)

---

## D. Context-Mode BESIDE the Gateway (Peers)

### D1. Shared FTS5 Knowledge Base

Both systems contribute to and query from a shared FTS5 index. Context-mode indexes file content and tool outputs; LocalRouter indexes MCP tool catalogs and responses.

```
Context-Mode → writes to → Shared FTS5 DB ← reads from ← LocalRouter Gateway
                              ↑ writes to ↑
                           LocalRouter (tool responses, catalogs)
```

- Unified search across all indexed content
- `ctx_search` returns results from both sources
- Gateway's deferred loading search also queries the shared index
- **Effort**: Medium | **Value**: Medium

### D2. Shared Session State

Context-mode's session tracker and LocalRouter's gateway sessions share state, so both systems know what tools were called, what files were modified, etc.

- Gateway publishes tool-call events to context-mode's session DB
- Context-mode's snapshots include gateway-specific state (active providers, routing decisions)
- On compaction, gateway injects context-mode's session snapshot
- **Effort**: High | **Value**: High

### D3. Event Bus / Pub-Sub Integration

Both systems publish events to a shared channel (e.g., Unix socket, named pipe, or HTTP webhook).

```
Context-Mode ──publish──→ Event Bus ←──subscribe── LocalRouter
             ←subscribe──           ──publish────→
```

Events: tool calls, file changes, session state, cache invalidations, health status
- **Effort**: High | **Value**: Medium (infrastructure overhead)

---

## E. Context-Mode AS A LAYER (Gateway Middleware)

### E1. Response Compression Middleware

Gateway intercepts MCP tool responses above a size threshold and routes them through context-mode for compression before delivering to the client.

```
Backend Server returns 50 KB response
  → Gateway middleware checks: size > threshold?
    → YES: send to context-mode ctx_execute for summarization
      → Return 2 KB summary + index original in FTS5
    → NO: pass through unchanged
  → Client receives compressed response
```

- Configurable per-client, per-server, or per-tool thresholds
- Original content always preserved in FTS5 for later `ctx_search`
- Transparent to both clients and backend servers
- **Effort**: Medium | **Value**: Very High (unique combined value)

### E2. Request Enhancement Middleware

Gateway enriches MCP tool calls with context from context-mode's session state before forwarding to backend servers.

- Example: `tools/call` for a code search tool gets augmented with "recent files modified" from session
- **Effort**: Medium | **Value**: Low-Medium (niche use case)

### E3. Caching Layer with Context-Mode Index

Use context-mode's FTS5 as a semantic cache. Before forwarding a tool call, check if a similar recent result exists in the index.

- Hash-based exact match + BM25 semantic similarity for near-matches
- Reduces redundant tool calls (e.g., reading the same file twice)
- **Effort**: Medium | **Value**: Medium

---

## F. Feature Extraction (Cherry-Pick Capabilities)

### F1. Port FTS5 Search Algorithm

Extract context-mode's 3-layer search fallback (Porter stemming → trigram → Levenshtein) and use it in LocalRouter's deferred loading.

- Replace current regex/BM25 search in `deferred.rs`
- Better tool discovery for LLMs
- Can be done in pure Rust with `rusqlite` FTS5
- **Effort**: Low-Medium | **Value**: Low-Medium

### F2. Port Session Snapshot System

Extract context-mode's 13-category event tracking and priority-tiered snapshot builder. Adapt to Rust for gateway session persistence.

- Events: files, tasks, rules, decisions, git, errors, environment, MCP tools, subagents, etc.
- Snapshots compressed to ~2 KB budget with critical items preserved first
- **Effort**: High | **Value**: Medium

### F3. Port Polyglot Executor

Extract the sandboxed code execution capability (11 languages) as a standalone gateway tool.

- Already similar to CodingAgentVirtualServer's execution model
- Could be a new virtual server: `VirtualSandboxServer`
- **Effort**: Medium | **Value**: Medium

### F4. Port Hook System Concept

Add a pre/post tool-call hook system to the gateway itself, inspired by context-mode's platform hooks.

- Before any `tools/call`: run pre-hooks (modify args, block, redirect)
- After any `tools/call`: run post-hooks (compress, log, transform)
- Hooks configurable per-client, per-server, per-tool
- **Effort**: Medium-High | **Value**: High (extensibility)

---

## G. LocalRouter Provides Services TO Context-Mode

### G1. Model Routing for Summarization

Context-mode could use an LLM for smarter summarization of large outputs. LocalRouter routes these LLM calls to the best/cheapest model.

```
Context-Mode needs to summarize 50 KB output
  → calls LocalRouter's /v1/chat/completions
    → LocalRouter routes to cheapest capable model (e.g., Haiku)
  ← receives summarized output
```

- Context-mode currently uses code execution for compression, not LLMs
- Adding LLM-powered summarization could be even more effective
- LocalRouter's routing engine picks the optimal model for cost/quality
- **Effort**: Medium | **Value**: High

### G2. Provider-Backed Indexing

Use LocalRouter's embedding providers to create vector embeddings of indexed content, enabling semantic search alongside FTS5's keyword search.

```
Context-Mode indexes content
  → sends to LocalRouter /v1/embeddings
    → LocalRouter routes to embedding provider
  ← stores vectors alongside FTS5 index
```

- Hybrid search: BM25 keyword + cosine similarity
- **Effort**: High | **Value**: Medium

---

## H. Distribution / Packaging Options

### H1. Bundled Distribution

Ship context-mode's pre-built bundle (`server.bundle.mjs`) inside LocalRouter's app bundle.

- Users get context-mode "for free" when they install LocalRouter
- No separate npm install needed
- LocalRouter manages the lifecycle
- **Effort**: Low | **Value**: High (UX)

### H2. Plugin Marketplace Entry

Add context-mode to LocalRouter's marketplace virtual server as a featured/recommended plugin.

- One-click install from LocalRouter's UI
- Auto-configures as backend MCP server
- **Effort**: Low | **Value**: Medium

---

## Summary Matrix

| # | Option | Where | Effort | Value | Code Changes |
|---|--------|-------|--------|-------|--------------|
| A1 | Backend MCP server | Behind | Very Low | High | Config only |
| A2 | First-class recommended server | Behind | Low-Med | High | UI + config |
| A3 | Auto-provisioned per client | Behind | Medium | High | Gateway logic |
| B1 | Hook interception (already works!) | In front | **Zero** | Very High | None |
| B2 | HTTP proxy mode | In front | High | High | Context-mode changes |
| C1 | Virtual MCP server (Rust) | Inside | Very High | High | Major new code |
| C2 | Embedded JS runtime | Inside | High | Medium | Complex integration |
| C3 | WASM module | Inside | Very High | Medium | Experimental |
| D1 | Shared FTS5 knowledge base | Beside | Medium | Medium | Both projects |
| D2 | Shared session state | Beside | High | High | Both projects |
| D3 | Event bus | Beside | High | Medium | Both projects |
| E1 | Response compression middleware | Layer | Medium | Very High | Gateway middleware |
| E2 | Request enhancement middleware | Layer | Medium | Low-Med | Gateway middleware |
| E3 | Semantic caching layer | Layer | Medium | Medium | Gateway + context-mode |
| F1 | Port FTS5 search | Feature | Low-Med | Low-Med | Deferred loading |
| F2 | Port session snapshots | Feature | High | Medium | New gateway feature |
| F3 | Port polyglot executor | Feature | Medium | Medium | New virtual server |
| F4 | Port hook system | Feature | Med-High | High | New gateway feature |
| G1 | Model routing for summarization | Service | Medium | High | Context-mode changes |
| G2 | Provider-backed embeddings | Service | High | Medium | Both projects |
| H1 | Bundled distribution | Packaging | Low | High | Build system |
| H2 | Marketplace entry | Packaging | Low | Medium | Marketplace config |

---

## Quick Wins (can do today with zero/minimal code)

1. **B1** — Users install both tools. Context-mode hooks already intercept everything, including MCP calls through LocalRouter. Works out of the box.
2. **A1** — Add context-mode as a backend server in LocalRouter's MCP server config. 5 minutes of configuration.
3. **H1/H2** — Bundle or list context-mode in marketplace for easy discovery.
