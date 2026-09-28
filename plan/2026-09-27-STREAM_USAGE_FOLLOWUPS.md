# Stream usage follow-ups

## Context
v0.0.145 made streams carry the upstream's usage and record it at stream end
(`plan/2026-09-27-TOOL_CHOICE_AND_STREAM_USAGE.md`). Gaps left open:
1. Cohere has no streaming (`stream_complete` returns "not implemented"), so no stream usage.
2. The LM Studio, Jan, GPT4All, LocalAI and llama.cpp stream parsers split each network read into lines without buffering, so an SSE line split across two reads (usage lines included) is lost.
3. Gateway providers built on `OpenAICompatibleProvider` (NVIDIA NIM, Vercel AI Gateway, LLM Gateway, …) do not pass their provider type, so the include-usage table never applies to them.
4. Legacy `/v1/completions` ignores `stream_options.include_usage`.
5. Cost ignores prompt caching: Anthropic reports cache reads/writes separately from `prompt_tokens` and they are never charged; OpenAI/Gemini cached tokens are charged at the full input rate.

## Changes
1. Cohere v2 streaming (`POST /v2/chat`, `stream: true`): SSE events `message-start`, `content-delta`, `tool-plan-delta`, `tool-call-start`/`-delta`/`-end`, `message-end` (finish reason, `usage.tokens` / `billed_units`) mapped to `CompletionChunk`s; usage on the finish chunk.
2. A shared line buffer for those five parsers: keep the trailing partial line across reads, parse complete lines, flush at end.
3. The factories that construct `OpenAICompatibleProvider` pass their provider type (`with_provider_type`). The include-usage table itself only lists types documented to accept the field.
4. `CompletionRequest.stream_options`; legacy `CompletionChunk.usage` (optional); a final usage chunk with empty `choices` when asked, plus `[DONE]`.
5. Usage semantics: `prompt_tokens` is the whole prompt; `prompt_tokens_details.cached_tokens` / `cache_read_tokens` and `cache_creation_tokens` are subsets of it. Anthropic (streaming and non-streaming) reports `prompt_tokens = input + cache_read + cache_creation`. `PricingInfo` gains optional `cache_read_cost_per_1k` / `cache_write_cost_per_1k` (from the catalog, else the provider's known rates); one `PricingInfo::cost(&TokenUsage)` charges uncached input, cache reads, cache writes, output and reasoning, used by the router (non-streaming and stream recorder) and the server finalize (metrics, access log, monitor, generation tracker).

## Mandatory final steps
1. Plan review against the implementation.
2. Test coverage: Cohere stream parsing (text, tool calls, usage), split-line parsing for each parser, gateway include-usage, legacy completions usage chunk, cost with cache (Anthropic + OpenAI shapes, missing cache prices fall back to input price).
3. Bug hunt: double-charging cache tokens, saturating subtraction, config/serde compatibility of `PricingInfo`.
4. CI parity (stable clippy/fmt/workspace tests), commit, merge to master; start a debug app for manual checking.
