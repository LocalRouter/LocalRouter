# Tool strict / tool_choice passthrough and real streaming usage

## Context
Reported by the jazyk session (gpt-5.5 via the ChatGPT Plus backend):
1. Tools lost `strict`; the Responses API treats a missing `strict` as true, so every property became required (fixed in 67ad3f8a).
2. The Chat Completions → Responses translation hardcodes `tool_choice: "auto"`, ignoring the client's choice.
3. Usage logging is wrong:
   - Streaming output tokens are `content chars / 4` (min 1), so tool-call-only turns log 1 token.
   - Input tokens count only the last message (a per-message display choice from commit 542dd6c0) but that number also feeds cost, metrics and the `usage` returned to clients, so cost is understated and clients see wrong usage.
   - The provider is logged as `router` when the model has no `provider/` prefix, so pricing lookups fail and cost is 0.
   - The router's streaming rate-limit/free-tier accounting uses a fixed 10 prompt tokens and records on the `finish_reason` chunk, before any final usage chunk arrives.
   - Clients' `stream_options.include_usage` is dropped (not in `ChatCompletionRequest`), so no usage chunk is ever sent to them.

## Changes
1. `translate_completion_request`: `tool_choice` becomes a JSON value mapped from the client's (`"auto"|"none"|"required"` strings pass through; `{type:function, function:{name}}` → `{type:"function", name}`), default `"auto"`.
2. `CompletionChunk` gains `usage: Option<TokenUsage>` (upstream usage when the provider reports it) and `provider: Option<String>` (serde-skipped; stamped by the router with the provider instance that served the stream).
3. Providers fill `usage` from their stream's usage data: OpenAI (chat + Responses `response.completed`), OpenAI-compatible (final usage chunk; request `stream_options.include_usage` for provider types known to accept it), OpenRouter, Anthropic (`message_start` input + `message_delta` output), Gemini (`usageMetadata`), Cohere (`message-end`), Ollama (`prompt_eval_count`/`eval_count`), and the other OpenAI-shaped local providers.
4. Router stream wrapper: stamps `provider` on chunks, keeps the last upstream usage, and records rate-limit / free-tier usage once the stream ends (real usage, else an estimate from the full prompt and all generated text incl. tool-call arguments).
5. Server streaming paths (`chat.rs` ×3, `completions.rs`, `responses.rs` where applicable) share a tracker: accumulates content, reasoning and tool-call argument text, finish reason, upstream usage and provider; finalizes when the stream ends (not at `finish_reason`), or when the client disconnects. Usage = upstream usage, else estimate (full prompt, all generated text). Provider = stamped provider, else the model prefix.
6. `ChatCompletionRequest.stream_options { include_usage }`: when set, the final SSE chunk carries `usage` (OpenAI semantics: an extra chunk with empty `choices`). Upstream usage-only chunks are not forwarded otherwise.
7. Non-streaming: client `usage`, metrics and cost use the upstream's real `prompt_tokens` (estimate only when the upstream reports 0).

## Mandatory final steps
1. Plan review against the implementation.
2. Test coverage: tool_choice mapping; per-provider usage parsing; tracker (usage chunk after finish_reason, tool-call-only turn, provider fallback, disconnect); include_usage chunk; router recording at end.
3. Bug hunt: double finalization, usage-only chunks leaking to clients, providers whose final chunk carries both content and usage, cancellation.
4. CI parity (stable clippy/fmt/workspace tests), commit, merge to master, release.

## Implementation notes
- Providers normalize usage to arrive once per stream (`openai_compatible/stream_usage.rs::usage_once_at_end`: a trailing chunk with empty `choices`), or on the finish chunk (Anthropic, Gemini, Ollama, Responses). `stream_options.include_usage` is requested only for provider types documented to accept it (OpenAI, Groq, OpenRouter, llama.cpp); Mistral rejects it. Cohere streaming is not implemented, so it has no stream usage.
- The server tracker (`routes/stream_usage.rs`) keeps the latest usage per upstream stream id and sums across ids (MCP-via-LLM tool loops stitch several upstream streams; the orchestrator forwards the finish chunk's usage as a usage-only chunk).
- Chat streams now end with `[DONE]` on every path. Legacy `/v1/completions` has no `stream_options` (its chunk type has no `usage`); its accounting uses the same tracker.
- `/v1/responses` streaming finalizes on client disconnect through `FinalizeOnDrop`.
