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

## Ranked Search API

`POST /v1/search` and `/search` return ranked page records rather than generated
answers. They require the same explicit client web-search grant. Configure a
qualified default ranked search model in Client Settings, or provide one per call.
A default is a preference and never grants model access.

For a configured direct Perplexity instance named `perplexity-account`:

```json
{
  "model": "perplexity-account/search",
  "query": ["first related query", "second related query"],
  "search_type": "fast",
  "max_results": 5,
  "max_tokens_per_page": 1000
}
```

`search` here is a local capability route, not an invented upstream model: the
adapter calls Perplexity `/search` without a model field. Direct Perplexity supports
web, fast and people modes; web/fast results are capped at 20 and people at 50.
Direct context-size shorthand is rejected; use explicit content token bounds.

For LLM Gateway, use `gateway/perplexity/perplexity-search` or
`gateway/perplexity/perplexity-search-fast`, with the matching explicit search type
when supplied. A fast model also selects fast mode when type is omitted. People
search and unsupported models are rejected. Automatic model selection and arbitrary
gateway routing controls have no verified ranked-search mapping in this version.

Queries accept a string or 1–5 related strings, each up to 4096 UTF-8 bytes. Result,
content-token, date, domain and country/language options are validated before a
search dispatch and checked again by the adapter. Date filters use MM/DD/YYYY.
Domain lists accept either included hosts or excluded hosts prefixed with `-`, not
both. URLs and URL credentials are not accepted as domain filters.

The response preserves the upstream id, ranked order, URLs, snippets and dates.
LocalRouter does not fetch a result URL. Results are untrusted content. Response
reads are limited to 4 MiB, with a separate 64 KiB limit for upstream error bodies.
A result rejected after dispatch may still have incurred a provider fee.

Ranked search performs one HTTP operation without retries, redirects, classifiers,
response caching or affinity. Guardrails and secret scans run sequentially on the
queries through the shared pipeline. Chat-specific approval rewrites are rejected
rather than silently sending an unapproved original query. Automatic MCP and
duplicate hops are unsupported. Configured MCP mediation remains independent.

Requests and unsuccessful/cancelled dispatches contribute to normal request
quotas. Unknown request fees cannot satisfy cost ceilings or free-only routing, so
those requests fail before dispatch. Tokenless metric rows do not establish a zero
provider bill: the response and monitor preserve unknown cost explicitly. Strict
privacy rejects unverified retrieval chains. SDK error responses after the dispatch
boundary include `x-should-retry: false`; Try It Out also disables SDK retries.

Try It Out → Search uses client mode, an explicit model or its client default, and
shows ranked sources separately from model-written answers. The website's result
fixture is clearly labeled as a demo and makes no external search request. It uses
fake local demo credentials and the saved demo search permission/privacy settings.

Primary ranked-search contracts and billing basis:

- https://docs.llmgateway.io/v1_search
- https://docs.perplexity.ai/api-reference/search-post
- https://docs.perplexity.ai/docs/getting-started/pricing
