// Documentation content for each subsection, keyed by subsection ID.
// Content is rendered as markdown via ReactMarkdown.

const docsContent: Record<string, string> = {

  // ============================================================
  // Section 5: Model Selection & Routing
  // ============================================================

  'auto-routing': `When a client sends a request with the model set to \`localrouter/auto\`, the router activates auto-routing mode. Instead of targeting a specific provider/model pair, the router consults the strategy's \`AutoModelConfig\` to select from a prioritized list of models. If RouteLLM is enabled, the classifier first determines whether a strong or weak model tier is appropriate; otherwise, the \`prioritized_models\` list is used directly. The router iterates through the selected models in order, attempting each one until a request succeeds or all options are exhausted.`,

  'routellm-classifier': `The RouteLLM classifier is a pure Rust implementation of an XLM-RoBERTa BERT model built on the Candle framework. It runs entirely locally with no external API calls, loading SafeTensors weights (~440 MB on disk). Inference takes approximately 15-20ms per prediction — the model tokenizes the prompt (truncated to 512 tokens), runs a forward pass through 12 transformer layers, and applies a classification head with softmax to produce a win-rate probability. The model consumes ~2.5-3 GB of memory when loaded and supports GPU acceleration via Metal (macOS) and CUDA (Linux/Windows) with automatic CPU fallback.`,

  'strong-weak-classification': `The classifier outputs a \`win_rate\` between 0.0 and 1.0 representing the probability that a strong model is needed. This value is compared against a configurable \`threshold\`: if \`win_rate >= threshold\`, the request routes to the strong model tier; otherwise, it routes to the weak tier. Recommended thresholds are **0.2** (quality-prioritized), **0.3** (balanced/default), and **0.7** (cost-optimized). This approach achieves 30-60% cost savings while retaining 85-95% quality compared to always using the strong tier.`,

  'fallback-chains': `When auto-routing is active, the router iterates through the selected model list in priority order, attempting each model sequentially. If a request fails with a retryable error (as determined by \`RouterError::should_retry()\`), the router advances to the next model in the chain. Before each attempt, strategy-level rate limits are also checked — if a model's limits are exceeded, it is skipped. The chain terminates either on the first successful response or when all models have been exhausted.`,

  'provider-failover': `Provider failover is driven by the \`RouterError\` classification system. When a provider returns an error, it is categorized as \`RateLimited\`, \`PolicyViolation\`, \`ContextLengthExceeded\`, \`Unreachable\`, or \`Other\`. The first four categories are retryable and trigger automatic failover to the next model in the prioritized list. \`Other\` errors (e.g., validation failures) are non-retryable and cause immediate request failure without further attempts.`,

  'offline-fallback': `When a remote provider is unreachable (connection refused, timeout, DNS failure), the error is classified as \`Unreachable\`, which is retryable. If the strategy's prioritized model list includes local providers like Ollama or LM Studio alongside remote providers, the router automatically falls through to these local models after remote failures. No special configuration is needed beyond including local models in the fallback chain.`,

  'routing-strategies': `A **Strategy** is the core routing configuration unit, referenced by clients via \`strategy_id\`. Each strategy defines an \`AvailableModelsSelection\` controlling which provider/model pairs the client can access, an optional \`AutoModelConfig\` for \`localrouter/auto\` support, and a list of rate limit entries. Strategies are decoupled from clients and reusable — multiple clients can share the same strategy. The \`allowed_models\` field supports three modes: \`selected_all\` (all models), \`selected_providers\` (all models from specific providers), or \`selected_models\` (individual provider/model pairs).`,

  'strategy-lowest-cost': `Order the \`prioritized_models\` list from cheapest to most expensive. When a request hits \`localrouter/auto\`, the cheapest available model is always attempted first. Combined with a cost-optimized RouteLLM threshold (e.g., 0.7), most requests route to the weak (cheaper) tier.`,

  'strategy-highest-performance': `Place the most capable models first in \`prioritized_models\`. The top-tier model handles every request unless it fails, in which case the router falls back to the next best model. Combined with a quality-prioritized RouteLLM threshold (e.g., 0.2), most requests go to the strong tier.`,

  'strategy-local-first': `Place local providers (Ollama, LM Studio) at the top of \`prioritized_models\` with remote providers as fallbacks. Requests route to locally-running models by default, avoiding API costs and network latency. If the local provider is down, the router falls through to remote providers.`,

  'strategy-remote-first': `Place cloud providers (OpenAI, Anthropic, etc.) at the top of \`prioritized_models\` with local models as fallbacks. This prioritizes the quality and speed of remote API models while maintaining resilience — if the remote provider is rate-limited or unreachable, requests automatically fail over to local models.`,

  'error-classification': `The \`RouterError\` enum classifies provider errors into five categories: \`RateLimited\`, \`PolicyViolation\`, \`ContextLengthExceeded\`, \`Unreachable\`, and \`Other\`. Classification inspects the \`AppError\` variant and matches error message strings for keywords. The \`should_retry()\` method returns \`true\` for the first four categories, enabling automatic fallback. Only \`Other\` errors halt the retry loop immediately.`,

  'error-rate-limited': `When a provider returns a rate limit error, it is classified as \`RateLimited\` with a default \`retry_after_secs\` of 60 seconds. Rather than waiting for the cooldown, the router immediately advances to the next model in the fallback chain.`,

  'error-policy-violation': `Content policy violations are classified as \`PolicyViolation\`. Despite being caused by content rather than infrastructure, this error is retryable — the router attempts the next model in the chain, which may have different content policies. This allows requests rejected by one provider's safety filters to potentially succeed with a different provider.`,

  'error-context-length': `When a provider rejects a request because the input exceeds its context window, the error is classified as \`ContextLengthExceeded\`. This is retryable, enabling automatic failover to a model with a larger context window in the fallback chain.`,

  // ============================================================
  // Section 6: Rate Limiting
  // ============================================================

  'request-rate-limits': `Request-based rate limits enforce a maximum number of API calls within a time window (\`Minute\`, \`Hour\`, or \`Day\`). The \`RateLimiterManager\` uses a sliding window algorithm backed by a \`VecDeque<UsageEvent>\` — each request increments the counter by 1.0, and events older than the window are pruned on each check. At the strategy level, the router uses metrics-based projection: it queries recent usage and adds 1.0 to the current count, rejecting the request if the projected total exceeds the limit.`,

  'token-limits': `Token-based rate limits (\`TotalTokens\`, \`InputTokens\`, \`OutputTokens\`) cap the number of tokens consumed within a sliding time window. Since actual token counts are unknown until the response arrives, token limits are enforced *after* completion at the client level and via pre-request projection at the strategy level. For streaming responses, tokens are estimated at ~1 token per 4 characters of content.`,

  'cost-limits': `Cost-based rate limits cap spending in USD within a time window. Cost is calculated per-request as \`(input_tokens / 1000) × input_cost + (output_tokens / 1000) × output_cost\` using provider pricing data. At the strategy level, cost projection uses the average cost from recent metrics; free/local models are excluded from cost limit calculations. Useful for enforcing budget caps like $10/day per client.`,

  'per-key-vs-per-router': `Rate limits operate at two levels. **Client-level** limits are enforced per individual client using sliding window counters — each client has independent counters for requests, tokens, and cost. **Strategy-level** limits use metrics-based pre-request projection, querying aggregate usage across all clients sharing that strategy. Both levels are checked before a request proceeds. Rate limiter state is persisted to disk periodically and restored on startup to survive application restarts.`,
}

export default docsContent
