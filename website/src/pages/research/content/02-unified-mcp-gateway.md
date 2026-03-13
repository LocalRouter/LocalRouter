<!-- @entry unified-gateway-abstract -->

The **Unified MCP Gateway** aggregates multiple Model Context Protocol servers behind a single client-facing endpoint with automatic tool namespacing, intelligent request routing, and progressive catalog compression. It extends the MCP model by injecting virtual servers — coding agents, skills, a marketplace, and context management — as first-class participants that are indistinguishable from real MCP servers. A three-phase compression algorithm reduces catalog size to fit within LLM context windows while maintaining full functionality through search-based activation.

<!-- @entry unified-gateway-problem -->

As the MCP ecosystem grows, AI applications face a practical scaling problem. Each MCP server connection requires its own transport, authentication, and session management. An application using 5-10 MCP servers must manage 5-10 separate connections and present dozens or hundreds of tools to the LLM — consuming precious context window tokens just to describe available capabilities.

This creates three distinct challenges:

1. **Connection complexity**: Each MCP server needs its own STDIO process, SSE stream, or HTTP connection with independent lifecycle management
2. **Context window pressure**: Tool descriptions from many servers can consume thousands of tokens before any user content is processed
3. **Namespace collisions**: Multiple servers may define tools with the same name (e.g., `search`, `read`)

<!-- @entry unified-gateway-architecture -->

The gateway presents itself as a single MCP server to clients. Internally, it maintains connections to multiple upstream servers and handles routing transparently.

**Namespace convention**: All tools, resources, and prompts are prefixed with their server's slug using double underscores: `filesystem__read_file`, `github__search_repos`. This eliminates collisions while remaining MCP-spec compliant (the `__` separator is parsed to route calls to the correct upstream server).

**Routing strategy**: The gateway uses two routing modes based on the MCP method:

| Method | Routing | Behavior |
|--------|---------|----------|
| `tools/list`, `resources/list` | Broadcast | Send to ALL servers in parallel, merge results |
| `tools/call`, `resources/read` | Direct | Parse namespace, route to single server |
| `initialize`, `ping` | Broadcast | Send to all, merge capabilities |

**Unified welcome message**: On initialization, the gateway constructs a merged server description that lists all available servers with their capabilities. The protocol version is set to the *minimum* across all servers (most restrictive for compatibility), while capabilities are the *union* (if any server supports a capability, the gateway advertises it).

```
LocalRouter Unified MCP Gateway

Available servers:
1. filesystem (Filesystem Access)
   Description: Read and write files on the local filesystem
2. github (GitHub Integration)
   Description: Search repos, manage issues, create PRs
3. postgres (PostgreSQL)
   Description: Query and manage PostgreSQL databases

Failed servers:
- slack: Connection timeout (will retry on next request)
```

<!-- @entry unified-gateway-compression -->

The most novel aspect of the gateway is its **three-phase progressive catalog compression** algorithm. When total catalog size exceeds a configurable threshold (default: 8,192 bytes), the algorithm applies increasingly aggressive compression — stopping as soon as the catalog fits within budget.

**Phase 1: Description Compression**

Individual tool/resource/prompt descriptions are compressed, starting with the largest items (for maximum byte savings per operation). The full content is indexed into an FTS5 full-text search database, and the description is replaced with a one-liner search hint:

```
Before (847 bytes):
  filesystem__read_file
  Read a file from the local filesystem. Supports text and binary files.
  Accepts absolute or relative paths. Returns file content as text...
  [full schema with 12 parameters]

After (89 bytes):
  filesystem__read_file — [compressed] ctx_search(source='catalog:filesystem__read_file')
```

**Phase 2: Server Deferral**

Entire tool/resource/prompt categories are hidden from listing responses. Tools exist in the index but are not returned by `tools/list` until the LLM calls `ctx_search` with a matching query, which triggers a `tools/changed` notification to the client. The server's entry in the welcome message becomes:

```
3. postgres — [deferred] ctx_search(source='catalog:postgres') to activate tools
```

**Phase 3: List Truncation**

Item listings are collapsed to counts only:

```
3. postgres: 12 tools, 3 resources — ctx_search(source='catalog:postgres') to explore
```

Each phase only activates if the previous phase wasn't sufficient. The algorithm is greedy-optimal: Phase 1 compresses the largest items first, Phase 2 defers the servers with the most items, and Phase 3 is the final fallback.

<!-- @entry unified-gateway-context-mode -->

The compression algorithm is powered by a **Context Management** virtual server that provides FTS5 full-text search capabilities. This is implemented as a per-client STDIO process (lazily spawned on first use) that maintains an isolated search index.

**Source label convention** enables precise search across different content types:

| Source Pattern | Content |
|---------------|---------|
| `catalog:` | All MCP catalog entries |
| `catalog:filesystem` | A specific server and its items |
| `catalog:filesystem__read_file` | A specific tool's full description |
| `filesystem__read_file:1` | First invocation's response |
| `filesystem__read_file:2` | Second invocation's response |

**Response compression**: When a tool call response exceeds a configurable threshold (default: 4,096 bytes), the full response is indexed with an incremental run ID, and a truncated version with a search hint is returned to the LLM. This prevents large tool outputs from consuming excessive context.

The tools exposed by the context management system are:

- `ctx_search` — Full-text search across indexed catalog and response content
- `ctx_index` — Manually index content with a custom source label
- `ctx_execute` — Execute a shell command and index the output
- `ctx_execute_file` — Execute a script file and index the output
- `ctx_batch_execute` — Execute multiple commands in parallel
- `ctx_fetch_and_index` — Fetch a URL and index the content

<!-- @entry unified-gateway-virtual-servers -->

The gateway supports **virtual servers** — server implementations that don't correspond to external MCP processes but are injected into the unified namespace as first-class participants. They implement the same `VirtualMcpServer` trait as real servers, meaning clients cannot distinguish them from external MCP servers.

**Coding Agents** (`_coding_agents`): Exposes four tools for managing AI coding agent sessions (start, say/interrupt, status, list). Uses BloopAI/vibe-kanban's executors crate for robust process management with Claude Code SDK control protocol support. Only the start tool requires firewall approval. Tool prefix is configurable (default: `Agent`).

**Skills** (`_skills`): Exposes available script-based workflows as callable MCP tools. Each skill provides metadata via `skill_get_info` and can execute multi-step workflows that compose multiple tool calls.

**Marketplace** (`_marketplace`): Provides MCP server/skill discovery and installation through search and install tools. Search operations are read-only (no approval needed); installations go through the firewall approval flow.

**Context Management** (`_context_mode`): The FTS5-backed search and indexing system described above.

All virtual servers:
- Participate in tool listing and namespacing
- Go through the same firewall/permission checks
- Can defer their tools (hidden until activated via `ctx_search`)
- Build custom instruction sections for the system prompt
- Maintain per-session state
- Are priority-sorted in the welcome message (context management first, then coding agents, marketplace, and skills)

<!-- @entry unified-gateway-caching -->

The gateway implements **adaptive cache TTL** for tool/resource/prompt listings. A `DynamicCacheTTL` mechanism tracks how frequently a server's listings change (via `tools/changed` notifications):

| Invalidations (per hour) | Cache TTL |
|:---:|:---:|
| 0–5 | 5 minutes (configured base) |
| 6–20 | 2 minutes |
| 20+ | 1 minute |

This ensures that stable servers benefit from aggressive caching while rapidly-changing servers stay fresh — all without configuration.

**Partial failure handling**: When some servers are unreachable, the gateway continues with working servers and reports failures in response metadata:

```json
{
  "tools": ["... tools from working servers ..."],
  "_meta": {
    "partial_failure": true,
    "failures": [
      { "server_id": "github", "error": "Connection timeout" }
    ]
  }
}
```

<!-- @entry unified-gateway-parallel-pipeline -->

Beyond MCP aggregation, the gateway orchestrates a **parallel request processing pipeline** for chat completions. Multiple independent operations run concurrently to minimize latency — the total time is `max(parallel tasks)` rather than the sum.

**Pipeline architecture**:

```
Request arrives
        │
        ▼
┌────────────────────────────────┐
│  Phase 1: Sequential Checks    │  ~1-5ms
│  Validation → Firewall →       │
│  Rate Limiting → Access        │
└───────────────┬────────────────┘
                │
    ┌───────────┼───────────┐
    ▼           ▼           ▼
┌────────┐ ┌──────────┐ ┌──────────┐
│Guard-  │ │Compressio│ │ RouteLLM │  Phase 2:
│rails   │ │n (Lingua)│ │Classifier│  Parallel
│~200ms  │ │ ~300ms   │ │ ~100ms   │  Spawn
└───┬────┘ └────┬─────┘ └────┬─────┘
    │           │            │
    │           ▼            │
    │  ┌─────────────────┐   │
    │  │Apply compressed │   │
    │  │messages to req  │   │
    │  └────────┬────────┘   │
    │           │            │
    │           ▼            │
    │  ┌─────────────────┐   │
    │  │ LLM Provider    │◄──┘ (routing decision)
    │  │ ~1000-3000ms    │
    │  └────────┬────────┘
    │           │
    ▼           ▼
┌────────────────────────────────┐
│  Guardrail Gate                │
│  If parallel: buffer LLM       │
│  chunks until guardrails pass  │
│  If sequential: guardrails     │
│  must pass before LLM starts   │
└───────────────┬────────────────┘
                │
                ▼
┌────────────────────────────────┐
│  Response Finalization         │
│  JSON Repair (streaming)       │
│  Metrics + Logging             │
│  SSE / JSON response           │
└────────────────────────────────┘
```

**Latency calculation** in parallel guardrails mode:

```
Sequential:  Guardrails(200ms) + LLM(2000ms) = 2200ms
Parallel:    max(Guardrails(200ms), LLM(2000ms)) = 2000ms  (10% savings)
```

The parallel mode is eligible when the request has no side effects (no web search, code interpreter, or tool calls that modify external state). When side effects are present, guardrails run sequentially to prevent executing dangerous operations before safety checks complete.

**Streaming with parallel guardrails** uses a buffer-and-gate pattern:

1. A `watch::channel` broadcasts the guardrail gate state (`Pending` → `Passed`/`Denied`)
2. LLM streaming chunks are buffered in memory while the gate is `Pending`
3. Once guardrails pass, buffered chunks are flushed and subsequent chunks stream directly
4. If guardrails deny, buffered chunks are discarded and an error event is sent

This means the client sees streaming start as soon as guardrails pass — if the LLM is slower than guardrails (the common case), streaming appears uninterrupted.

<!-- @entry unified-gateway-results -->

The Unified MCP Gateway reduces the integration burden from N separate MCP connections to a single endpoint. Combined with progressive catalog compression, it enables practical use of 10+ MCP servers within typical context windows (8K-32K tokens for tool descriptions).

| Metric | Without Gateway | With Gateway |
|--------|:---:|:---:|
| Client connections | N (one per server) | 1 |
| Namespace conflicts | Possible | Eliminated |
| Context usage (10 servers) | ~12,000 tokens | ~2,000 tokens (compressed) |
| Failed server impact | Client crash | Graceful degradation |
| Adding new capabilities | Client code change | Config change only |
