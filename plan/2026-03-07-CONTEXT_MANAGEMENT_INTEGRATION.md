# Context Management: Unified Deferred Loading + Context-Mode Integration

**Date**: 2026-03-07
**Status**: Approved

## Overview

Replace LocalRouter's existing Deferred Loading with a unified **Context Management** system integrating [context-mode](https://github.com/mksglu/context-mode) — an MCP server providing FTS5 search, polyglot code execution, and content indexing (~98% context reduction).

**Problem**: MCP tool responses, welcome texts, and tool catalogs can consume massive context window space (10-50 KB+). Current deferred loading only addresses tool catalog size with basic regex/BM25 search.

**Solution**: Spawn a per-client context-mode STDIO process via npx. Index large content (tool catalogs, welcome texts, tool responses) into FTS5. Replace the deferred `search` tool with `ctx_search`. Optionally expose indexing tools for broader AI use. Use a progressive catalog compression algorithm that only compresses what's needed to stay under a configurable threshold.

---

## Feature: Context Management

When Context Management is **enabled**:
- `ctx_search` is always exposed (bare name, no namespace)
- Catalog content progressively compressed based on catalog threshold (see Catalog Compression Algorithm)
- Tool responses > response threshold → indexed in FTS5 + truncated
- Tool/resource/prompt discovery via `ctx_search` (replaces deferred regex/BM25)
- Skills script output compressed automatically
- `ctx_stats`, `ctx_doctor`, `ctx_upgrade` are NOT exposed (stats via UI, version management via UI only)

### Indexing Tools Toggle

When **Indexing Tools** is enabled (on top of Context Management):
- Additional tools exposed: `ctx_execute`, `ctx_execute_file`, `ctx_batch_execute`, `ctx_index`, `ctx_fetch_and_index`
- AI can use these for code execution, file processing, content indexing, and URL fetching beyond MCP
- Descriptions passed through from context-mode library (see Tool Description Strategy)

When **Indexing Tools** is disabled:
- Only `ctx_search` is exposed
- Context Management still compresses the MCP catalog and responses
- AI is not encouraged to use context-mode for non-MCP tasks

---

## Architecture

### Per-Client Instance

```
Client A → Gateway → context-mode STDIO process A (PID-based FTS5 DB)
Client B → Gateway → context-mode STDIO process B (separate FTS5 DB)
```

Spawned via `npx -y context-mode` reusing existing `StdioTransport::spawn()` + shell PATH resolution from `crates/lr-mcp/src/manager.rs`.

### Source Label Convention

Source labels use a colon-based format consistent with context-mode's own auto-generated labels (e.g., `execute:shell`). Context-mode source filtering uses `LIKE '%source%'` — partial/substring matching.

```
Catalog content (indexed during session init, based on catalog compression):
  catalog:filesystem                      ← server welcome/instructions text
  catalog:filesystem__read_file           ← tool description (namespaced tool name)
  catalog:github__create_pr               ← prompt description (namespaced prompt name)
  catalog:filesystem__project_files       ← resource description (namespaced resource name)

  Searching "catalog:" finds ALL catalog entries (tools, resources, prompts, welcome).
  Searching "catalog:filesystem" finds filesystem welcome AND all filesystem__* items.
  Searching "filesystem__read_file" finds that tool across catalog and responses.

Response content (indexed on-demand when over response threshold):
  {namespaced_tool_name}:{run_id}         ← tool call output (incremental ID per session)
  e.g., filesystem__read_file:1           ← first call to this tool
  e.g., filesystem__read_file:2           ← second call to this tool
  e.g., github__create_pr:1              ← first prompt get
  e.g., filesystem__project_files:1      ← first resource read

  Searching "filesystem__read_file:" finds ALL responses from that tool.
  Searching "filesystem__read_file:2" finds the specific invocation.

Context-mode auto-generated (from AI's own ctx_execute/ctx_index usage):
  execute:shell                           ← ctx_execute output
  execute:python                          ← ctx_execute output
  batch:{label}                           ← ctx_batch_execute output
  execute_file:{path}                     ← ctx_execute_file output
  {user-chosen label}                     ← ctx_index / ctx_fetch_and_index
```

**Catalog indexing examples**:

```
// Tool
ctx_index(source="catalog:filesystem__read_file", content="filesystem__read_file\nRead file contents from disk.\nArgs: path (string, required)")

// Prompt
ctx_index(source="catalog:github__create_pr", content="github__create_pr\nCreate a pull request.\nArgs: title, body, reviewers")

// Resource
ctx_index(source="catalog:filesystem__project_files", content="filesystem__project_files\nList of all files in the project directory.\nURI: file:///project")

// Welcome text (per server)
ctx_index(source="catalog:filesystem", content="<full tool+prompt+resource listing + server instructions>")
```

**Response indexing examples** (incremental run_id per session):

```
// First call to filesystem__read_file that exceeds threshold
ctx_index(source="filesystem__read_file:1", content=full_response)

// Second call to same tool
ctx_index(source="filesystem__read_file:2", content=full_response)
```

### Two Thresholds

| Threshold | Applies To | Default | When |
|-----------|-----------|---------|------|
| **Catalog threshold** | Welcome text + tool/prompt/resource description listings (total size) | 8192 bytes | Session init — progressive compression until under threshold |
| **Response threshold** | Tool call output, `prompts/get` output, `resources/read` output | 4096 bytes | On-demand — only when response exceeds threshold |

The catalog threshold determines how aggressively the welcome text is compressed. If the total catalog is small (few servers, few tools), nothing gets compressed. As it grows past the threshold, the progressive algorithm kicks in (see below).

### Catalog Compression Algorithm

The catalog is the combined welcome text sent to the client at session init: server listings, tool/resource/prompt descriptions, and per-server instructions. When Context Management is enabled, the gateway measures the total catalog size and progressively compresses until it fits under `catalog_threshold_bytes`.

**Algorithm** (run during session init, after merging all server capabilities):

```
catalog = build_full_catalog()  // uncompressed welcome text

while catalog.size > catalog_threshold_bytes:
    // Phase 1: Compress individual descriptions (largest first)
    // Index into FTS5, replace with one-liner + search hint
    if any_uncompressed_descriptions_remain():
        item = find_largest_uncompressed_item()  // tool, resource, prompt, or welcome text
        ctx_index(source="catalog:{namespaced_name}", content=item.full_description)
        item.replace_with_summary()
        // Summary example for a tool: "read_file — Use ctx_search(source='catalog:filesystem__read_file')"
        // Summary example for welcome: "{server} — Use ctx_search(source='catalog:filesystem')"
        catalog.recalculate_size()
        continue

    // Phase 2: Switch to deferred loading (hide tools/resources/prompts entirely)
    // Only if client supports the *_changed notifications for that type
    if any_non_deferred_items_remain():
        server = find_server_with_most_items()
        if client.supports_tools_list_changed():
            defer_tools(server)  // remove from tools/list, activate via ctx_search
        if client.supports_resources_list_changed():
            defer_resources(server)
        if client.supports_prompts_list_changed():
            defer_prompts(server)
        catalog.recalculate_size()
        continue

    // Phase 3: Truncate remaining lists
    // Remove individual tool/resource/prompt lines from welcome, keep only counts
    if any_listed_items_remain():
        server = find_server_with_most_listed_items()
        server.replace_item_list_with_counts()
        // "filesystem: 12 tools, 3 resources — Use ctx_search(source='catalog:filesystem')"
        catalog.recalculate_size()
        continue

    break  // Nothing left to compress
```

**Optimization**: Before entering the loop, estimate how many items need compression:
- Calculate `bytes_to_save = catalog.size - catalog_threshold_bytes`
- Sort all compressible items by size (descending)
- Batch-compress the top N items whose cumulative size exceeds `bytes_to_save`
- This avoids re-measuring after every single compression

**Key principle**: Every compressed portion includes a specific `ctx_search` hint telling the AI exactly how to retrieve the full content. The AI never loses access to information — it just has to search for it.

### Search-Based Activation (Replaces deferred.rs)

Works for tools, prompts, AND resources — same mechanism. The gateway maintains a mapping from `catalog:*` source labels to their item type (tool/resource/prompt) and namespaced identity.

```
1. AI calls ctx_search(queries: ["file read"], source: "catalog:")
2. Gateway forwards to context-mode STDIO process
3. Results return with source per result:
   --- [catalog:filesystem__read_file] ---
   filesystem__read_file: Read file contents...
4. Gateway checks: "catalog:filesystem__read_file" is in catalog_sources → it's a tool → activate
5. Sends tools_changed notification
6. Appends activation info to response (see ctx_search Response Format below)
7. Returns augmented results to AI
```

The gateway tracks a `catalog_sources: HashMap<String, CatalogItemType>` populated during session init. When `ctx_search` results come back:

1. For each result, check if `source` is in `catalog_sources`
2. If yes AND item is deferred (not yet activated):
   - Activate the tool/resource/prompt
   - Send appropriate `*_changed` notification
   - Collect into `activated` list
3. If not in `catalog_sources` → pass through as-is (user content, response retrieval, etc.)
4. Append activation summary to the response (only if any items were activated)

### ctx_search Response Format

The gateway augments `ctx_search` responses when deferred items are activated. The original context-mode response is returned as-is, with an additional section appended:

**No activations** (all results already active or non-catalog):
```
## file read

--- [catalog:filesystem__read_file] ---
filesystem__read_file: Read file contents from disk.
Args: path (string, required) — Absolute path to the file to read
```

**With activations** (deferred items now made available):
```
## file read

--- [catalog:filesystem__read_file] ---
filesystem__read_file: Read file contents from disk.
Args: path (string, required) — Absolute path to the file to read

--- [catalog:filesystem__write_file] ---
filesystem__write_file: Write content to a file.
Args: path (string, required), content (string, required)

---
Activated tools: filesystem__read_file, filesystem__write_file
These tools are now available for use.
```

The `Activated tools/resources/prompts` lines are only appended when deferred loading is active AND at least one new item was activated by this search. This tells the AI that the MCP server's tool list has been updated.

### Response Compression Flow

Applies to tool call responses, `resources/read` responses, and `prompts/get` responses. **Does NOT apply to ctx_* tool responses** — those are from the context-mode virtual server itself and must never be double-compressed.

The gateway maintains a per-session `run_counter: HashMap<String, u32>` to generate incremental run IDs per namespaced tool/resource/prompt name.

```
tools/call response from backend server (NOT ctx_* tools)
  → Size check: > response_threshold_bytes?
    → YES: run_id = next_run_id("{namespaced_tool}")  // e.g., 1, 2, 3...
           ctx_index(source="{namespaced_tool}:{run_id}", content=full)
           Truncate + "Full output indexed. Use ctx_search(source='{namespaced_tool}:{run_id}') to retrieve."
    → NO:  Pass through unchanged

  Example: filesystem__read_file called twice, both large:
    First:  source="filesystem__read_file:1"
    Second: source="filesystem__read_file:2"

resources/read response
  → Size check: > response_threshold_bytes?
    → YES: run_id = next_run_id("{namespaced_resource}")
           ctx_index(source="{namespaced_resource}:{run_id}", content=full)
           Truncate + search hint
    → NO:  Pass through unchanged

prompts/get response
  → Size check: > response_threshold_bytes?
    → YES: run_id = next_run_id("{namespaced_prompt}")
           ctx_index(source="{namespaced_prompt}:{run_id}", content=full)
           Truncate + search hint
    → NO:  Pass through unchanged
```

### Welcome Text Format

The welcome text format depends on the catalog compression level. The header is kept brief — detailed source hierarchy docs live in the `ctx_search` tool description itself (since ctx_* tools are never deferred or compressed).

At minimum (no compression needed):

```
Unified MCP Gateway — Context-Managed

Servers: filesystem (12 tools, 3 resources), github (8 tools, 2 prompts), slack (5 tools)

Use ctx_search to discover MCP capabilities, retrieve compressed content, and search server docs.

[Full tool/resource/prompt listings follow as normal...]
[Per-server instruction sections follow as normal...]
```

At maximum compression (everything compressed):

```
Unified MCP Gateway — Context-Managed

Servers: filesystem (12 tools, 3 resources), github (8 tools, 2 prompts), slack (5 tools)

Use ctx_search to discover MCP capabilities, retrieve compressed content, and search server docs.

[If Indexing Tools enabled: additions about ctx_execute, ctx_batch_execute...]
```

Virtual server instructions (Skills, Context-Mode itself) stay inline — they're generated by the gateway, not from MCP servers.

### Tool Description Strategy

Context-mode's prompting lives **entirely in tool descriptions** — there are no server prompts, resources, or system instructions. The library's descriptions are heavily opinionated and designed for Claude Code:

| Tool | Library Description Style |
|------|--------------------------|
| `ctx_execute` | "MANDATORY: Use for any command where output exceeds 20 lines. PREFER THIS OVER BASH for: API calls, test runners, git queries..." |
| `ctx_execute_file` | "PREFER THIS OVER Read/cat for: log files, data files..." |
| `ctx_batch_execute` | "THIS IS THE PRIMARY TOOL. Use this instead of multiple execute() calls." |
| `ctx_index` | Detailed "WHEN TO USE" list (Documentation, API refs, MCP tools/list, README files...) |
| `ctx_search` | Short: "Search indexed content. TIPS: 2-4 specific terms per query." |

**Approach: Hybrid — replace `ctx_search`, pass through others**

- **`ctx_search` (both modes)**: **Replace entirely.** Our usage is fundamentally different — MCP catalog search with activation behavior, source label conventions (`catalog:`, `{tool}:{run_id}`), and mode-specific guidance. The library's description is too generic for our needs.

- **Other ctx_* tools (when Indexing Tools enabled)**: **Pass through from library.** When we spawn context-mode via STDIO and call `tools/list`, we get back the library's tool definitions with their descriptions. We forward these as-is. They're broadly applicable (any AI client with shell/file tools benefits from "PREFER THIS OVER BASH" guidance). If the library updates descriptions, we automatically inherit improvements.

- **Never compressed**: ctx_* tool descriptions are always sent in full — they are never subject to catalog compression or deferred loading. The compression algorithm skips virtual server tools entirely.

**Why not replace all descriptions?**
- The library's descriptions are battle-tested prompting that drives good AI behavior
- They include runtime-specific details (available languages, Bun detection) computed at startup
- Replacing them means falling out of sync when the library updates
- The descriptions reference each other correctly (e.g., ctx_index says "use 'search' to retrieve")

**Implementation**: In `ContextModeVirtualServer::list_tools()`:
1. Call `tools/list` on the STDIO process to get library tool definitions
2. Filter: remove `ctx_stats`, `ctx_doctor`, `ctx_upgrade`
3. Replace `ctx_search` description with our mode-specific version
4. If indexing_tools disabled: filter to only `ctx_search`
5. If indexing_tools enabled: return all remaining tools with library descriptions

### Exposed ctx_* Tool Definitions

These tools are exposed as gateway virtual tools with bare names (no namespace). These tools are **never** deferred or compressed — they are always visible in the tool list.

#### `ctx_search` (always exposed — description injected)

The gateway takes the library's original `ctx_search` tool definition and **injects** additional description text. This keeps us in sync when the library updates while adding our MCP-specific guidance.

**Library original description:**
```
Search indexed content. Pass ALL search questions as queries array in ONE call.

TIPS: 2-4 specific terms per query. Use 'source' to scope results.
```

**Library original `source` parameter description:**
```
Filter to a specific indexed source (partial match).
```

**Gateway injection — appended to tool description:**

Always appended (both with and without indexing tools):
```

MCP Gateway source labels (use with 'source' parameter):
  source="catalog:"                — search all MCP catalog entries (tools, resources, prompts, server docs)
  source="catalog:filesystem"      — search within a specific server (docs + all its items)
  source="catalog:filesystem__"    — search tools/resources/prompts from a specific server
  source="filesystem__read_file:"  — find all compressed responses from a specific tool
  source="filesystem__read_file:3" — find a specific invocation

Searching catalog entries automatically activates matching tools/resources/prompts for use.
```

Additionally appended when indexing tools are enabled:
```

Other indexed content (from ctx_execute, ctx_index, etc.):
  source="execute:"     — find auto-indexed output from ctx_execute
  source="batch:"       — find auto-indexed output from ctx_batch_execute
  (omit source to search everything)
```

**Gateway injection — appended to `source` parameter description:**

Always:
```
 MCP examples: "catalog:" for all MCP entries, "catalog:filesystem" for one server, "filesystem__read_file:" for a tool's responses.
```

**Result — final tool as seen by the AI (without indexing tools):**
```json
{
  "name": "ctx_search",
  "description": "Search indexed content. Pass ALL search questions as queries array in ONE call.\n\nTIPS: 2-4 specific terms per query. Use 'source' to scope results.\n\nMCP Gateway source labels (use with 'source' parameter):\n  source=\"catalog:\"                — search all MCP catalog entries (tools, resources, prompts, server docs)\n  source=\"catalog:filesystem\"      — search within a specific server (docs + all its items)\n  source=\"catalog:filesystem__\"    — search tools/resources/prompts from a specific server\n  source=\"filesystem__read_file:\"  — find all compressed responses from a specific tool\n  source=\"filesystem__read_file:3\" — find a specific invocation\n\nSearching catalog entries automatically activates matching tools/resources/prompts for use.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "queries": {
        "type": "array",
        "items": { "type": "string" },
        "description": "Array of search queries. Batch ALL questions in one call."
      },
      "source": {
        "type": "string",
        "description": "Filter to a specific indexed source (partial match). MCP examples: \"catalog:\" for all MCP entries, \"catalog:filesystem\" for one server, \"filesystem__read_file:\" for a tool's responses."
      },
      "limit": {
        "type": "number",
        "description": "Results per query (default: 3)"
      }
    }
  }
}
```

**Implementation**: In `ContextModeVirtualServer::list_tools()`:
1. Get `ctx_search` tool definition from STDIO `tools/list`
2. Append MCP source label guide to description
3. If indexing_tools enabled: also append execute/batch/index source docs
4. Append MCP examples to the `source` parameter description
5. Return modified tool definition

#### `ctx_execute`, `ctx_execute_file`, `ctx_batch_execute`, `ctx_index`, `ctx_fetch_and_index` (Indexing Tools enabled only)

**Passed through from the context-mode library.** When the gateway spawns the context-mode STDIO process and calls `tools/list`, it gets back these tool definitions with the library's own descriptions and schemas. We forward them as-is.

The library's actual descriptions (from source):

| Tool | Library Description (key parts) |
|------|--------------------------------|
| `ctx_execute` | "MANDATORY: Use for any command where output exceeds 20 lines. Execute code in a sandboxed subprocess. Only stdout enters context. PREFER THIS OVER BASH for: API calls, test runners, git queries, data processing..." |
| `ctx_execute_file` | "Read a file and process it without loading contents into context. FILE_CONTENT variable. PREFER THIS OVER Read/cat for: log files, data files, large source files..." |
| `ctx_batch_execute` | "THIS IS THE PRIMARY TOOL. Execute multiple commands in ONE call, auto-index all output, and search with multiple queries. One batch_execute call replaces 30+ execute calls + 10+ search calls." |
| `ctx_index` | "Index documentation or knowledge content into searchable BM25 knowledge base. Chunks markdown by headings. WHEN TO USE: Documentation, API references, MCP tools/list output, README files..." |
| `ctx_fetch_and_index` | "Fetches URL content, converts HTML to markdown, indexes into searchable knowledge base, returns ~3KB preview. Better than WebFetch." |

These descriptions include:
- Runtime detection (available languages, Bun optimization notes) — computed at startup
- Strongly worded usage guidance ("MANDATORY", "PREFER THIS OVER", "THIS IS THE PRIMARY TOOL")
- Cross-references between tools ("use 'search' to retrieve", "use 'execute_file' for those")
- Parameter-level guidance (intent, background, timeout)

**No modifications needed.** The descriptions are broadly applicable to any AI client and automatically stay in sync with library updates. The only addition we could consider is appending a note about `catalog:` reserved source prefix to `ctx_index`, but the ctx_search description already documents this.

#### `ctx_fetch_and_index` schema (for reference)

```json
{
  "name": "ctx_fetch_and_index",
  "inputSchema": {
    "type": "object",
    "properties": {
      "url": { "type": "string", "description": "The URL to fetch and index" },
      "source": { "type": "string", "description": "Label for the indexed content" }
    },
    "required": ["url"]
  }
}
```

### Source Label Separation (MCP vs User Content)

All source labels use a consistent `{type}:{identifier}` colon-based format:

| Origin | Source Label Pattern | Examples |
|--------|---------------------|----------|
| MCP catalog (gateway-indexed) | `catalog:{namespaced_name}` | `catalog:filesystem__read_file`, `catalog:filesystem` |
| MCP response compression | `{namespaced_name}:{run_id}` | `filesystem__read_file:1`, `filesystem__read_file:2` |
| ctx_execute auto-index | `execute:{language}` | `execute:shell`, `execute:python` |
| ctx_batch_execute auto-index | `batch:{label}` | `batch:git-log`, `batch:test-output` |
| ctx_execute_file auto-index | `execute_file:{path}` | `execute_file:/tmp/logs.txt` |
| ctx_index (AI-provided) | User-chosen label | `API docs`, `README`, `project notes` |
| ctx_fetch_and_index | User-chosen or URL-based | `React docs`, `Supabase Auth API` |

**No wrapper needed.** The gateway maintains a `catalog_sources: HashMap<String, CatalogItemType>` mapping `catalog:*` source labels to their item type. When `ctx_search` results come back:

1. Parse each result's `source` field
2. If source starts with `catalog:` AND is in `catalog_sources` AND item is deferred → activate + send `*_changed` notification
3. Otherwise → pass result through as-is (no activation)

This means the AI can freely search across both MCP catalog content and their own indexed content without false activations. A broad `ctx_search(queries=["error handling"])` might return both `catalog:filesystem__read_file` and user-indexed `API docs` — only the catalog entries trigger activation.

### Prompt Templates

Every compressed portion must include a clear, specific hint telling the AI how to retrieve the full content. Here are the exact templates used at each compression stage.

#### 1. Welcome Text Header (always present when Context Management enabled)

Brief — detailed source hierarchy is in `ctx_search` tool description (always visible, never compressed).

```
Unified MCP Gateway — Context-Managed

{server_count} servers connected. Use ctx_search to discover MCP capabilities, retrieve compressed content, and search server docs.
```

#### 2. Tool Description Compressed (Phase 1 of catalog compression)

Original in catalog:
```
- filesystem__read_file: Read the complete contents of a file from the file system.
  Handles text encodings, provides detailed error messages for missing/inaccessible files.
  Args: path (string, required) — Absolute path to the file to read
```

Compressed to:
```
- filesystem__read_file — [compressed] ctx_search(queries=["read_file"], source="catalog:filesystem__read_file")
```

#### 3. Prompt Description Compressed

Original:
```
- github__create_pr: Create a pull request with title, body, base branch, and reviewers.
  Args: title (string), body (string), base (string), reviewers (string[])
```

Compressed to:
```
- github__create_pr — [compressed] ctx_search(queries=["create_pr"], source="catalog:github__create_pr")
```

#### 4. Resource Description Compressed

Original:
```
- filesystem__project_files (file:///project): List of all files in the project directory
  with metadata including size, modification time, and permissions.
```

Compressed to:
```
- filesystem__project_files — [compressed] ctx_search(queries=["project_files"], source="catalog:filesystem__project_files")
```

#### 5. Welcome/Instructions Text Compressed (per-server)

Original inline XML section:
```
<filesystem>
The filesystem server provides read/write access to the local file system.
Always use absolute paths. The server has access to /Users/matus/dev/...
[... potentially large instructions ...]
</filesystem>
```

Compressed to:
```
- filesystem instructions — [compressed] ctx_search(queries=["filesystem"], source="catalog:filesystem")
```

#### 6. Server List Truncated to Counts (Phase 3 of catalog compression)

Original listing:
```
filesystem (12 tools):
  - read_file: Read file contents
  - write_file: Write to a file
  - list_directory: List directory contents
  [... 9 more tools ...]
```

Truncated to:
```
- filesystem: 12 tools, 3 resources — ctx_search(source="catalog:filesystem") to explore
```

#### 7. Tool Call Response Compressed (runtime, > response_threshold)

Original response (e.g., 15 KB file content):
```
{contents of a large file...}
```

Compressed to (run_id is incremental per tool per session):
```
[Response compressed — 15,234 bytes indexed as filesystem__read_file:1]

{first ~500 bytes of content as preview...}

Full output indexed. Use ctx_search(queries=["your search terms"], source="filesystem__read_file:1") to retrieve specific sections.
```

#### 8. Resource Read Response Compressed

```
[Response compressed — 8,192 bytes indexed as filesystem__project_files:1]

{first ~500 bytes as preview...}

Full output indexed. Use ctx_search(queries=["your search terms"], source="filesystem__project_files:1") to retrieve specific sections.
```

#### 9. Prompt Get Response Compressed

```
[Response compressed — 5,120 bytes indexed as github__create_pr:1]

{first ~500 bytes as preview...}

Full output indexed. Use ctx_search(queries=["your search terms"], source="github__create_pr:1") to retrieve specific sections.
```

#### Template Principles

- Every `[compressed]` marker includes the **exact `ctx_search` call** with source to retrieve the content
- Response compressions include a **preview** (first ~500 bytes) so the AI can decide if it needs the full content
- Response sources include the **run_id** so each invocation is individually retrievable
- Catalog sources use `catalog:` prefix; response sources use `{name}:{run_id}` format — consistent colon-based convention
- The `ctx_search` tool description (always visible, never compressed) documents the full source hierarchy

---

## Configuration

### Global Config

```yaml
mcp:
  context_management:
    enabled: false                        # Master toggle (replaces mcp_deferred_loading)
    indexing_tools: true                  # Expose ctx_execute, ctx_execute_file, etc.
    catalog_threshold_bytes: 8192         # Progressive compression kicks in above this
    response_threshold_bytes: 4096        # Compress individual responses above this
```

All settings (indexing_tools, thresholds) are global. Clients cannot override individual settings — only disable context management entirely.

### Per-Client Override

Clients can ONLY override to disable context management. The field is optional — when absent (default), the client inherits the global setting:

```yaml
clients:
  - name: "My Client"
    # context_management_enabled omitted → inherits global (enabled/disabled)
  - name: "Claude Code"
    context_management_enabled: false   # Explicitly disabled for this client
```

In Rust: `Option<bool>` where `None` = inherit global, `Some(false)` = disabled regardless of global.

The `mcp_deferred_loading: bool` field is replaced by this.

**Migration**: `mcp_deferred_loading: true` → field omitted (inherits global). `mcp_deferred_loading: false` → `context_management_enabled: false`.

### Client Already Has Context-Mode

If a client (e.g., Claude Code) already has context-mode installed via its own MCP config, the user should disable context management for that client via the per-client override (`context_management_enabled: false`). The UI shows this clearly with a note explaining why you'd disable it.

---

## UI

### Context Management Settings (Global)

In the Settings or MCP section:

```
┌─────────────────────────────────────────────────────┐
│ Context Management                                    │
│                                                       │
│ ┌─ Enable Context Management ──── [Toggle: ON] ─────┐│
│ │ Reduces context window usage by indexing large MCP  ││
│ │ outputs and providing FTS5-powered search.          ││
│ └─────────────────────────────────────────────────────┘│
│                                                       │
│ ┌─ Indexing Tools ──────── [Toggle: ON] ─────────────┐│
│ │ Adds helper tools for executing code, processing   ││
│ │ script files, and reading files with indexed search.││
│ │ (ctx_execute, ctx_execute_file, ctx_batch_execute,  ││
│ │  ctx_index, ctx_fetch_and_index)                    ││
│ └─────────────────────────────────────────────────────┘│
│                                                       │
│ ┌─ Thresholds ─────────────────────────────────────┐  │
│ │ Catalog threshold:  [8192] bytes                  │  │
│ │ Response threshold: [4096] bytes                  │  │
│ └───────────────────────────────────────────────────┘  │
│                                                       │
│ ┌─ Context-Mode Version ───────────────────────────┐  │
│ │ Installed: v1.0.14    Latest: v1.0.16             │  │
│ │ [Install / Upgrade]                               │  │
│ └───────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────┘
```

### Per-Client Toggle

In the client's MCP tab (replacing the old deferred loading toggle):

```
Context Management: [Toggle: ON]  (uses global settings)
```

When OFF: "Context management disabled for this client."

### Active Sessions View

New section showing active context-mode sessions (visible in dashboard or MCP section):

```
┌─────────────────────────────────────────────────────────┐
│ Active Context-Mode Sessions                              │
│                                                           │
│ ┌─ Claude Code (session abc123) ─────────────────────┐   │
│ │ Indexing Tools: ON | Uptime: 14m                    │   │
│ │ Indexed: 47 sources (23 tools, 12 resources, ...)   │   │
│ │ Compressed: 12 responses (~34 KB saved)             │   │
│ │ [View Index] [Search Index]                         │   │
│ └─────────────────────────────────────────────────────┘   │
│                                                           │
│ ┌─ Cursor (session def456) ──────────────────────────┐   │
│ │ Indexing Tools: OFF | Uptime: 3m                    │   │
│ │ Indexed: 23 sources (15 tools, 5 resources, ...)    │   │
│ │ Compressed: 3 responses (~8 KB saved)               │   │
│ │ [View Index] [Search Index]                         │   │
│ └─────────────────────────────────────────────────────┘   │
└───────────────────────────────────────────────────────────┘
```

**[View Index]**: Shows all indexed sources and their sizes in a modal/drawer.
**[Search Index]**: Opens a search interface where the user can query the FTS5 index directly (calls `ctx_search` on the session's context-mode process). Useful for debugging what content is indexed and verifying search results.

Stats (token savings, indexed content counts) come from calling `ctx_stats` on the session's context-mode process — but this is done server-side via Tauri commands, NOT exposed as an AI tool.

---

## Files to Modify/Create

### New Files

**`crates/lr-mcp/src/gateway/context_mode.rs`** — `ContextModeVirtualServer` + `ContextModeSessionState`

```rust
pub struct ContextModeVirtualServer {
    // No shared state — all state is per-session
}

pub struct ContextModeSessionState {
    pub indexing_tools_enabled: bool, // whether ctx_execute etc. are exposed
    pub transport: Option<StdioTransport>, // Lazy — spawned on first use
    pub catalog_sources: HashMap<String, CatalogItemType>,  // "catalog:name" → type
    pub run_counters: HashMap<String, u32>,  // per-tool/resource/prompt response run IDs
    // Tool activation (replaces DeferredLoadingState)
    pub full_tool_catalog: Vec<NamespacedTool>,
    pub activated_tools: HashSet<String>,
    // Resource/prompt activation
    pub full_resource_catalog: Vec<NamespacedResource>,
    pub activated_resources: HashSet<String>,
    pub full_prompt_catalog: Vec<NamespacedPrompt>,
    pub activated_prompts: HashSet<String>,
}
```

Implements `VirtualMcpServer`:
- `id()` → `"_context_mode"`
- `owns_tool()` → matches `ctx_search`, `ctx_execute`, `ctx_execute_file`, `ctx_batch_execute`, `ctx_index`, `ctx_fetch_and_index`
- `is_enabled()` → resolved from global config + per-client override
- `list_tools()` → always `ctx_search`; if indexing_tools_enabled: also ctx_execute, ctx_execute_file, ctx_batch_execute, ctx_index, ctx_fetch_and_index (no stats/doctor/upgrade)
- `handle_tool_call()` → forwards to per-session StdioTransport; for `ctx_search`, post-processes results to activate tools/resources/prompts
- `build_instructions()` → search usage guide with source hierarchy examples
- `create_session_state()` → creates lazy state

### Modified Files

**`crates/lr-config/src/types.rs`** — Config changes
- Add `ContextManagementConfig { enabled: bool, indexing_tools: bool, catalog_threshold_bytes: usize, response_threshold_bytes: usize }`
- Remove `ContextManagementMode` enum — replaced by `indexing_tools: bool`
- Replace `Client.mcp_deferred_loading: bool` with `Client.context_management_enabled: Option<bool>` (`None` = inherit global, `Some(false)` = disabled)
- Default global: `{ enabled: false, indexing_tools: true, catalog_threshold_bytes: 8192, response_threshold_bytes: 4096 }`

**`crates/lr-mcp/src/gateway/merger.rs`** — Welcome text restructuring
- `InstructionsContext`: replace `deferred_loading: bool` with `context_management_enabled: bool` + `indexing_tools_enabled: bool`
- Add `catalog_compression: Option<CatalogCompressionPlan>` to `InstructionsContext`
- `build_gateway_instructions()`: When context management enabled:
  - Always include header with `ctx_search` source hierarchy guide
  - Apply catalog compression plan: items marked as compressed show one-liner + search hint
  - Items marked as deferred are omitted from tools/list (activated via ctx_search)
  - Items marked as truncated show only server name + counts
  - Virtual server instructions (Skills, Context-Mode itself) stay inline
- New fn `build_full_server_content(server: &McpServerInstructionInfo) -> String` — builds merged content per server for indexing
- New fn `compute_catalog_compression_plan(context: &InstructionsContext, threshold: usize) -> CatalogCompressionPlan` — implements the progressive compression algorithm

**`crates/lr-mcp/src/gateway/gateway.rs`** — Session init indexing
- After initialize merges capabilities, if context management enabled:
  - Compute catalog compression plan
  - Spawn context-mode process (lazy, via virtual server state)
  - Index compressed items into FTS5:
    - Tools: `ctx_index(source="catalog:{namespaced_tool}", ...)`
    - Prompts: `ctx_index(source="catalog:{namespaced_prompt}", ...)`
    - Resources: `ctx_index(source="catalog:{namespaced_resource}", ...)`
    - Welcome text: `ctx_index(source="catalog:{server_slug}", ...)`
  - Only items that the compression plan marked for compression get indexed

**`crates/lr-mcp/src/gateway/gateway_tools.rs`** — Core changes
- `handle_tools_list()`: Replace deferred loading logic with context management activation — show `ctx_search` (+ other ctx_* in Full mode) + activated tools + non-deferred tools
- `handle_tools_call()`: After response from backend (NOT ctx_* tools), call `maybe_compress_response()` if enabled and > response_threshold
- `handle_resources_read()`: Same compression for resource read responses (source: `{namespaced_resource}:{run_id}`)
- `handle_prompts_get()`: Same compression for prompt get responses (source: `{namespaced_prompt}:{run_id}`)
- `handle_resources_list()`: Return only activated resources (when deferred) + non-deferred resources
- `handle_prompts_list()`: Return only activated prompts (when deferred) + non-deferred prompts
- Remove all references to `deferred.rs` search/server_info tools
- New `maybe_compress_response()`: index content via ctx_index, truncate, append search hint with source path

**`crates/lr-mcp/src/gateway/session.rs`** — Cleanup
- Remove `deferred_loading_requested: bool`
- Remove `deferred_loading: Option<DeferredLoadingState>`
- Context management state lives in `virtual_server_state["_context_mode"]`

**`crates/lr-mcp/src/gateway/types.rs`** — Remove `DeferredLoadingState`
- Delete entire `DeferredLoadingState` struct and related types
- Add `CatalogCompressionPlan` struct:
  ```rust
  pub struct CatalogCompressionPlan {
      // Items whose descriptions are compressed (indexed, replaced with one-liner)
      pub compressed_descriptions: Vec<CompressedItem>,
      // Items deferred entirely (hidden from tools/list, activated via search)
      pub deferred_items: Vec<DeferredItem>,
      // Servers whose item lists are truncated to counts only
      pub truncated_servers: Vec<String>,
  }
  ```

**`crates/lr-mcp/src/gateway/mod.rs`** — Module changes
- Add `pub mod context_mode;`
- Remove `pub mod deferred;`

**`crates/lr-mcp/src/gateway/deferred.rs`** — DELETE entirely

**`crates/lr-skills/src/mcp_tools.rs`** — Skills restructuring
- Remove: `build_run_tool()`, `build_run_async_tool()`, `build_read_tool()`, `build_get_async_status_tool()`
- Remove: `SkillToolParsed::Run`, `RunAsync`, `Read`, `GetAsyncStatus` and their match arms
- `build_skill_tools()`: only returns `get_info` tools (remove `info_loaded`, `deferred_loading` params)
- `build_get_info_response()`: show absolute script paths instead of tool names:
  ```
  ## Scripts
  Base path: /tmp/localrouter-skills/code-review-abc123/scripts/
  - review.py → /tmp/localrouter-skills/code-review-abc123/scripts/review.py
  - lint.sh → /tmp/localrouter-skills/code-review-abc123/scripts/lint.sh

  Run via ctx_execute or your code execution tools.
  ```

**`crates/lr-mcp/src/gateway/virtual_skills.rs`** — Simplify
- `SkillsSessionState`: remove `info_loaded: HashSet<String>`, `async_enabled: bool`
- `list_tools()`: just returns get_info tools
- `handle_tool_call()`: only handles GetInfo
- `build_instructions()`: update to mention script base path and ctx_execute

### Frontend Changes

**`src/types/tauri-commands.ts`** — Update types
- Add `ContextManagementConfig` (global) type — `{ enabled, indexingTools, catalogThresholdBytes, responseThresholdBytes }`
- Update `Client` interface: replace `mcpDeferredLoading: boolean` with `contextManagementEnabled: boolean | null` (null = inherit global)

**`src/views/clients/tabs/mcp-tab.tsx`** — Replace deferred loading toggle
- Simple toggle: "Context Management: [ON/OFF]"
- Label: "Uses global settings" when ON, "Disabled for this client" when OFF

**Global settings page** — New Context Management section
- Enable/disable toggle
- Indexing Tools toggle
- Catalog threshold input
- Response threshold input
- Context-mode version display with install/upgrade buttons

**Dashboard or MCP page** — Active Sessions section
- List active context-mode sessions per client
- Per-session: mode, uptime, indexed source count, compression stats
- [View Index] button → modal showing all indexed sources
- [Search Index] button → search interface querying the session's FTS5 index

**`website/src/components/demo/TauriMockSetup.ts`** — Update mock to reflect new config

### Website Changes

The marketing website needs to be updated to replace "Deferred Loading" references with "Context Management":

**`website/`** — Landing page and feature descriptions
- Replace any "Deferred Loading" feature mentions with "Context Management"
- Update feature descriptions to reflect the new capabilities (FTS5 search, progressive compression, response indexing)
- Update screenshots/diagrams if they show the old deferred loading toggle

**`/Users/matus/dev/winXP`** (external repo) — Windows XP demo
- Update mock data to use `contextManagementEnabled` instead of `mcpDeferredLoading`
- Update any demo UI that showed the deferred loading toggle
- After changes, run `./scripts/build-winxp.sh` to rebuild and copy to `website/public/winxp/`

### Tauri Commands (New)

- `get_context_management_config() -> ContextManagementConfig` — get global config
- `update_context_management_config(config: ContextManagementConfig)` — update global config
- `toggle_client_context_management(client_id: String, enabled: Option<bool>)` — per-client override (None = inherit global, Some(false) = disabled)
- `get_context_mode_sessions() -> Vec<ContextModeSessionInfo>` — list active sessions
- `get_context_mode_session_stats(session_id: String) -> ContextModeStats` — get stats for a session (calls ctx_stats internally)
- `search_context_mode_index(session_id: String, query: String, source: Option<String>) -> Vec<SearchResult>` — search a session's index from UI
- `get_context_mode_version() -> Option<String>` — detect installed version

### Claude Code MCP Config

When showing the JSON snippet for adding LocalRouter as an MCP server for Claude Code, include `deferred_loading: false` to disable Claude Code's built-in deferred loading (since our Context Management handles this):

```json
{
  "mcpServers": {
    "localrouter": {
      "command": "localrouter",
      "args": ["--mcp-bridge"],
      "deferred_loading": false
    }
  }
}
```

---

## Implementation Phases

### Phase 1: Config + Virtual Server Shell
1. Add `ContextManagementConfig` to `lr-config` types
2. Replace `mcp_deferred_loading: bool` with `context_management_enabled: Option<bool>` on Client
3. Create `context_mode.rs` with `ContextModeVirtualServer` skeleton
4. Register in gateway's virtual server list
5. Spawn context-mode via npx, verify STDIO JSON-RPC communication
6. Expose `ctx_search` tool (bare name, no namespace)

### Phase 2: Catalog Compression + Search-Based Activation
1. Implement `compute_catalog_compression_plan()` in merger.rs — progressive algorithm
2. On session init, index items marked for compression into FTS5 (source: `catalog:{namespaced_name}`, etc.)
3. Implement `ctx_search` forwarding via virtual server's `handle_tool_call()`
4. Post-process search results: parse source labels → activate matching tools/resources/prompts
5. Send appropriate `*_changed` notifications after activation
6. Wire `handle_tools_list()` to return only activated + non-deferred tools + ctx_* tools
7. Delete `deferred.rs`

### Phase 3: Welcome Text Restructuring
1. Modify `merger.rs` — apply compression plan to welcome text
2. Add `build_full_server_content()` for per-server indexing
3. Always include `ctx_search` source hierarchy guide in header
4. Compressed items show one-liner + search hint
5. Virtual server instructions (Skills, Context-Mode) stay inline

### Phase 4: Response Compression
1. Add `maybe_compress_response()` in `gateway_tools.rs`
2. Hook into tool call responses (source: `{namespaced_tool}:{run_id}`) — NOT ctx_* tools
3. Hook into `resources/read` responses (source: `{namespaced_resource}:{run_id}`)
4. Hook into `prompts/get` responses (source: `{namespaced_prompt}:{run_id}`)
5. All use `response_threshold_bytes` config value
6. Each truncated response includes specific `ctx_search` hint with exact source path

### Phase 5: Skills Restructuring
1. Remove run/read/async tools from `mcp_tools.rs`
2. Modify `get_info` to show absolute script paths
3. Simplify `virtual_skills.rs` (remove info_loaded tracking)
4. Update `build_instructions()` to reference ctx_execute and absolute paths

### Phase 6: Indexing Tools
1. Wire `indexing_tools` toggle — when enabled, expose ctx_execute, ctx_execute_file, ctx_batch_execute, ctx_index, ctx_fetch_and_index (passed through from library)
2. Inject additional source docs into ctx_search description when indexing tools enabled
3. Test with various AI clients

### Phase 7: UI + Session Management
1. Replace deferred loading toggle with per-client context management toggle
2. Global context management settings page (mode, thresholds)
3. Active sessions view with per-session stats
4. [View Index] and [Search Index] UI for each session (backed by Tauri commands)
5. Context-mode version detection, install/upgrade buttons
6. Claude Code config snippet with `deferred_loading: false`

---

## Verification

1. **Unit tests**: `context_mode.rs` — tool activation from search results, source parsing, mode-based tool listing
2. **Unit tests**: `merger.rs` — catalog compression plan computation (various catalog sizes vs thresholds)
3. **Integration tests**: Spawn context-mode via npx, index content, search, verify results
4. **Manual testing**:
   - Enable Context Management with small catalog → verify no compression happens
   - Enable with large catalog → verify progressive compression kicks in
   - Call `ctx_search(queries: ["file"], source: "tool/")` → verify matching tools activated across servers
   - Call `ctx_search(source: "catalog:filesystem")` → verify results scoped to one server
   - Call a tool that returns >4KB → verify response indexed + truncated with search hint
   - `ctx_search(source: "response/")` → retrieve full response
   - Indexing Tools ON: verify all ctx_* tools listed (no stats), ctx_execute works
   - Indexing Tools ON: verify ctx_execute auto-indexes with `execute:{lang}` source, doesn't trigger tool activation
   - Indexing Tools OFF: verify only ctx_search exposed, no execute/index tools
   - ctx_search description: verify MCP source labels injected, and indexing source labels only present when toggle is on
   - Skills: verify get_info shows absolute paths, run tools removed
5. **Prompts/resources**: `ctx_search(source: "prompt/")` activates prompts, `ctx_search(source: "resource/")` activates resources
6. **Response compression**: `prompts/get` and `resources/read` responses over threshold → indexed + truncated with source-specific hint
7. **UI**: Active sessions view shows stats, search works from UI
8. **Cargo checks**: `cargo test && cargo clippy && cargo fmt`
9. **Frontend**: `npx tsc --noEmit` for type checking
