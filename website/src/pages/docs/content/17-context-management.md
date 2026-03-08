<!-- @entry context-management-overview -->

Context Management is an intelligent compression and search system that reduces the context window footprint of MCP tool catalogs by up to 98%. Instead of loading every tool, resource, and prompt description into the AI's context at session start, Context Management progressively compresses the catalog and provides a full-text search index for on-demand discovery.

When enabled, the gateway spawns a per-session `context-mode` process backed by an FTS5 (SQLite full-text search) database. All original tool descriptions, resource templates, and prompt definitions are indexed into this database. The AI receives a single `ctx_search` tool instead of hundreds of individual tool schemas.

**Key benefits:**

- Reduces initial context consumption from tens of thousands of tokens to under 1,000
- No information loss — all capabilities remain searchable and activatable
- Automatic progressive compression tuned to configurable thresholds
- Per-client enable/disable with global defaults

<!-- @entry catalog-compression -->

Catalog compression runs automatically during MCP session initialization. It operates in three progressive phases, applying each phase in order until the total catalog size falls below the configured threshold (default: 8,192 bytes).

<!-- @entry compression-phase-1 -->

### Phase 1: Description Compression

Each tool, resource, and prompt description is individually compressed using an extractive summarizer. The original full descriptions are indexed into the FTS5 database with search hints, then replaced with one-line summaries in the catalog.

For example, a tool with a 500-token description like:

> `filesystem__read_file` — Reads the contents of a file at the specified path. Supports text and binary files. Returns the file content as a string. Can optionally specify encoding...

Becomes:

> `filesystem__read_file` — Read file contents. *Search: file read content path encoding*

<!-- @entry compression-phase-2 -->

### Phase 2: Server Deferral

If Phase 1 doesn't bring the catalog under the threshold, entire MCP servers are deferred. Tools from deferred servers are removed from the `tools/list` response entirely. When the client supports `tools/listChanged` notifications, deferred tools can be transparently re-activated when discovered via search.

Servers are deferred in order of least to most frequently used (based on session history), preserving the most relevant tools.

<!-- @entry compression-phase-3 -->

### Phase 3: List Truncation

As a final measure, remaining tool/resource/prompt lists are truncated to just item counts with search directions. For example:

> *5 MCP servers with 42 tools available. Use `ctx_search` to discover and activate tools by keyword.*

This achieves maximum compression while still informing the AI about the scope of available capabilities.

<!-- @entry search-based-activation -->

The gateway exposes a `ctx_search` tool that queries the FTS5 full-text index. When the AI needs a capability, it searches by keyword:

```json
{
  "tool": "ctx_search",
  "arguments": {
    "queries": ["create github issue", "file management"]
  }
}
```

The search returns matching tools, resources, and prompts with their full descriptions. Any deferred tools found in the search results are automatically activated — the gateway sends a `tools/listChanged` notification, and the newly available tools appear in the next `tools/list` response.

Each search result includes a source label (e.g., `catalog:github__create_issue`) for traceability, and the response summary tells the AI exactly which tools were activated.

<!-- @entry response-compression -->

Beyond catalog compression, Context Management also compresses large tool call responses. When a tool response exceeds the response threshold (default: 4,096 bytes), the full output is indexed into the FTS5 database with a unique label (e.g., `filesystem__read_file:3` for the third invocation), and the response is truncated with a search hint:

> *[Content truncated — 12,400 bytes indexed as `filesystem__read_file:3`. Use `ctx_search` with relevant keywords to retrieve specific sections.]*

This prevents a single large tool response from consuming the AI's entire context window while keeping the full content searchable.

<!-- @entry context-management-config -->

Context Management is configured globally and can be overridden per client.

<!-- @entry context-thresholds -->

### Threshold Settings

Two thresholds control compression behavior:

| Setting | Default | Description |
|---------|---------|-------------|
| `catalog_threshold_bytes` | 8,192 | Maximum total size of all tool/resource/prompt descriptions after compression |
| `response_threshold_bytes` | 4,096 | Maximum individual tool response size before indexing and truncation |

Lower thresholds produce more aggressive compression. A `catalog_threshold_bytes` of 2,048 is suitable for models with very small context windows.

<!-- @entry context-per-client -->

### Per-Client Override

Each client can override the global Context Management setting:

- **Inherit** (default) — Uses the global enable/disable setting
- **Enabled** — Forces context management on for this client regardless of global setting
- **Disabled** — Forces context management off, delivering full uncompressed catalogs

This is configured in the client's settings under the Context Management tab.
