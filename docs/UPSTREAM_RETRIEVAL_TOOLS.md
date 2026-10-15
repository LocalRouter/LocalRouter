# Provider search and web fetch

Client Settings → Provider search and web fetch grants explicit permission for
retrieval executed upstream. Both permissions default off. These permissions
are independent of client-owned function tools and LocalRouter MCP permissions.

Chat Completions and Responses accept the following typed tools:

| Gateway | Tool | Supported local cap |
| --- | --- | --- |
| OpenRouter | `openrouter:web_search` | `max_tool_calls`, narrowed by current client settings |
| OpenRouter | `openrouter:web_fetch` | Same per-request cap |
| LLM Gateway | `web_search` | Provider-managed only; no verified global cap mapping |

For example, with OpenRouter configured as `gateway` and search enabled:

```json
{
  "model": "gateway/provider/model",
  "messages": [{"role": "user", "content": "Find current documentation and cite it."}],
  "tools": [{"type": "openrouter:web_search", "parameters": {"engine": "exa", "max_results": 5}}],
  "max_tool_calls": 3
}
```

Use Responses `input` in place of `messages` at `/v1/responses`. Include complete
explicit history; provider-bound `previous_response_id` is not supported with
these tools. Function definitions remain client-owned and may share the array.
LocalRouter restores their original order without inventing function wrappers.
LLM Gateway additionally supports `tool_choice: {"type":"web_search"}`.

The request cap and current client cap intersect. A caller cannot raise the
configured cap. The cap applies to each upstream HTTP request, not an entire
conversation across client requests. LLM Gateway requires deliberately selecting
provider-managed call limits; requesting a local cap there fails before IO.

Supported engine parameters follow verified gateway mappings. Native/automatic
search rejects options that native providers can ignore. Firecrawl rejects
content-size options it ignores. Exa accepts both domain lists; other explicit
search engines accept at most one list. Strict fetch limits and filters currently
require the Exa mapping. Unknown parameters, tool types, engines, stop conditions
and convenience search shorthands fail explicitly. Runtime upstream/model support
can still change; protocol support is not a claim that every model supports a
particular native backend.

Calls require normal model permissions and existing scans/approvals. Guardrails
finish before dispatch. Restricted local-only/no-retention profiles reject these
unverified third-party chains, even when inference has a gateway ZDR setting.
Automatic MCP orchestration, duplicate gateway hops, native Messages, legacy
Completions and persistent batches have no enabled server-tool mapping. For
LocalRouter-mediated search/fetch, configure an MCP server and grant that server's
tools through the client's MCP settings; LocalRouter does not install services
or fetch cited URLs automatically.

Unknown tool fee ceilings cannot satisfy a client/strategy cost limit or free-only
routing, so those combinations fail before classification or dispatch. Gateway
reported charges retain their upstream-call attribution. Missing tool charges
remain unknown; token estimates do not establish the complete search/fetch bill.
Provider `usage.server_tool_use` counters are preserved separately from token
usage and retained as cumulative stream snapshots, never summed per chunk.

Dynamic tool requests bypass exact response caching, session affinity,
classification reuse and forwarded prompt-cache keys. Bound session continuity is
rejected. Gateway fallback is disabled and LocalRouter does not retry a failed
dispatch, including errors where the provider may already have executed a tool.
The transport refuses redirects instead of reposting to another target.

URL annotations survive normal and streaming Chat responses. The Responses
translator maps nested chat citations to flat URL citations and includes them in
completed output. Streaming annotation snapshots are bounded and deduplicated.
Monitor/export reconstructs the public tool array; the internal projection is not
an API input. Replay strips callable definitions, call caps and private contexts,
retaining the original captured input as provenance.

Primary contracts reviewed 2026-10-15:

- https://openrouter.ai/docs/guides/features/server-tools
- https://openrouter.ai/docs/guides/features/server-tools/web-search
- https://openrouter.ai/docs/guides/features/server-tools/web-fetch
- https://docs.llmgateway.io/features/web-search

Ranked `/v1/search` is a separate retrieval API and is tracked in the implementation
plan. This document describes model-triggered provider tools only.
