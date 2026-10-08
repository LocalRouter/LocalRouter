<!-- @entry context-management-overview -->

Context Management keeps MCP traffic from flooding the AI's context window. It does two independent jobs:

- **Tool Responses Indexing** — a tool response larger than the response threshold is indexed and replaced with a preview plus the exact calls that read or search the rest.
- **MCP Catalog Indexing** (catalog compression) — when the combined tool, resource, and prompt catalog is larger than the catalog threshold, server instructions and tool definitions are indexed and deferred behind search.

Both are backed by a per-session, in-memory SQLite FTS5 index (with optional hybrid vector search). The AI gets two tools to use it: `IndexSearch` and `IndexRead` (names are configurable).

**Key benefits:**

- Large tool responses no longer consume the context window, yet nothing is lost
- Small and medium responses (up to 16 KB by default) pass through untouched
- Large catalogs shrink to summaries; deferred tools are activated on demand
- Per-client enable/disable with global defaults

<!-- @entry catalog-compression -->

Catalog compression runs during MCP session initialization when MCP Catalog Indexing is on. It applies progressive phases, in order, until the estimated catalog size falls below the catalog threshold (default: 1,000 bytes).

<!-- @entry compression-phase-1 -->

### Phase 1: Index Server Instructions

The largest server instructions (welcome messages) are indexed under `mcp/<server>` and replaced in the gateway instructions by a one-line summary and a table of contents with line references:

> Indexed "mcp/github" — 45 lines, 2.1KB, 8 chunks
>
> \## Contents
> \- [L5] Issues
> \- [L12] Pull Requests

<!-- @entry compression-phase-2 -->

### Phase 2: Defer Tool Definitions

If that is not enough, tool definitions are indexed under `mcp/<server>/tool/<name>` and removed from `tools/list`, starting with the servers whose definitions save the most. A deferred tool stays callable; searching or reading its catalog entry activates it, and the gateway sends `notifications/tools/list_changed` so the client lists it again.

<!-- @entry compression-phase-3 -->

### Phase 3: Drop Tables of Contents

As a final measure, the tables of contents are dropped, leaving one summary line per indexed server and per deferred batch, each telling the AI which `source` to search.

<!-- @entry search-based-activation -->

`IndexSearch` queries the index. Pass every question in one call:

```json
{
  "tool": "IndexSearch",
  "arguments": {
    "queries": ["create github issue", "file management"],
    "source": "mcp/"
  }
}
```

- Hits are ranked best-first, show line numbers, and come with up to ~3,000 characters of context each. A chunk already shown for an earlier query is referenced instead of repeated.
- `limit` sets hits per query (default 5, max 20).
- `source` keeps only sources whose label starts with it — `mcp/` for the catalog, `mcp/github` for one server, `jira__getIssue:` for one tool's responses. A `source` that matches nothing returns an error listing the indexed labels.
- Deferred tools, resources, and prompts found by a search are activated, and the gateway notifies the client with `notifications/tools/list_changed`.

`IndexRead` returns a source by label as numbered lines — 200 lines by default, up to ~64 KB per call. When it stops early it ends with the offset to continue from:

```
[173 more lines — continue with offset="61"]
```

Lines longer than 2,000 characters are split into parts labelled `N-M` (part M of line N); `limit` counts whole lines.

<!-- @entry response-compression -->

When a tool response exceeds the response threshold (default: 16,384 bytes), the full output is indexed under a unique label — the tool name and a run number, e.g. `filesystem__read_file:3` — and the response is replaced with:

```
[Response compressed — 34176 bytes, 233 lines, indexed as "filesystem__read_file:3"]

<first ~2 KB of the response>
[… preview ends at line 41 of 233]

## Contents
- [L1] Overview
- [L42] Endpoints

Read all of it: IndexRead(label="filesystem__read_file:3", limit=233)
Read a range:   IndexRead(label="filesystem__read_file:3", offset="<line>", limit=<lines>)
Search it:      IndexSearch(queries=["<terms>"], source="filesystem__read_file:3")
```

A response is left unchanged when the placeholder would not be smaller than the response itself. Run numbers are never reused while LocalRouter runs, so a label always names the same content.

<!-- @entry context-management-config -->

Context Management is configured globally and can be overridden per client. Settings left at their defaults are not written to the config file, so improved defaults reach you automatically.

<!-- @entry context-thresholds -->

### Threshold Settings

Two thresholds control compression behavior:

| Setting | Default | Description |
|---------|---------|-------------|
| `catalog_threshold_bytes` | 1,000 | Catalog size above which MCP Catalog Indexing starts compressing |
| `response_threshold_bytes` | 16,384 | Tool response size above which the response is indexed and replaced with a preview |

The preview shown in a compressed response is an eighth of the response threshold, between 256 bytes and 2 KB. Raise the response threshold to compress fewer responses; lower it for models with small context windows.

<!-- @entry context-per-client -->

### Per-Client Override

Each client can override the global Context Management setting:

- **Inherit** (default) — Uses the global enable/disable setting
- **Enabled** — Forces context management on for this client regardless of global setting
- **Disabled** — Forces context management off, delivering full uncompressed catalogs

This is configured in the client's settings under the Context Management tab.
