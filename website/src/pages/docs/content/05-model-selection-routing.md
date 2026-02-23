<!-- @entry auto-routing -->

When a client sends a request with the model set to `localrouter/auto`, the router activates auto-routing mode. Instead of targeting a specific provider/model pair, the router consults the strategy's `AutoModelConfig` to select from a prioritized list of models.

If RouteLLM is enabled, the classifier first determines whether a strong or weak model tier is appropriate; otherwise, the `prioritized_models` list is used directly. The router iterates through the selected models in order, attempting each one until a request succeeds or all options are exhausted.

<!-- @entry routellm-classifier -->

The RouteLLM classifier is a pure Rust implementation of an XLM-RoBERTa BERT model built on the Candle framework. It runs entirely locally with no external API calls, loading SafeTensors weights (~440 MB on disk).

**Inference performance.** Inference takes approximately 15-20ms per prediction. The model tokenizes the prompt (truncated to 512 tokens), runs a forward pass through 12 transformer layers, and applies a classification head with softmax to produce a win-rate probability.

**Resource usage.** The model consumes ~2.5-3 GB of memory when loaded and supports GPU acceleration via Metal (macOS) and CUDA (Linux/Windows) with automatic CPU fallback.

<!-- @entry strong-weak-classification -->

The classifier outputs a `win_rate` between 0.0 and 1.0 representing the probability that a strong model is needed. This value is compared against a configurable `threshold`: if `win_rate >= threshold`, the request routes to the strong model tier; otherwise, it routes to the weak tier.

Recommended thresholds are **0.2** (quality-prioritized), **0.3** (balanced/default), and **0.7** (cost-optimized). This approach achieves 30-60% cost savings while retaining 85-95% quality compared to always using the strong tier.

<!-- @entry fallback-chains -->

When auto-routing is active, the router iterates through the selected model list in priority order, attempting each model sequentially. If a request fails with a retryable error (as determined by `RouterError::should_retry()`), the router advances to the next model in the chain.

Before each attempt, strategy-level rate limits are also checked — if a model's limits are exceeded, it is skipped. The chain terminates either on the first successful response or when all models have been exhausted.

<!-- @entry provider-failover -->

Provider failover is driven by the `RouterError` classification system. When a provider returns an error, it is categorized as `RateLimited`, `PolicyViolation`, `ContextLengthExceeded`, `Unreachable`, or `Other`.

The first four categories are retryable and trigger automatic failover to the next model in the prioritized list. `Other` errors (e.g., validation failures) are non-retryable and cause immediate request failure without further attempts.

<!-- @entry offline-fallback -->

When a remote provider is unreachable (connection refused, timeout, DNS failure), the error is classified as `Unreachable`, which is retryable.

If the strategy's prioritized model list includes local providers like Ollama or LM Studio alongside remote providers, the router automatically falls through to these local models after remote failures. No special configuration is needed beyond including local models in the fallback chain.

<!-- @entry routing-strategies -->

A **Strategy** is the core routing configuration unit, referenced by clients via `strategy_id`. Each strategy defines an `AvailableModelsSelection` controlling which provider/model pairs the client can access, an optional `AutoModelConfig` for `localrouter/auto` support, and a list of rate limit entries.

Strategies are decoupled from clients and reusable — multiple clients can share the same strategy. The `allowed_models` field supports three modes: `selected_all` (all models), `selected_providers` (all models from specific providers), or `selected_models` (individual provider/model pairs).

<!-- @entry strategy-lowest-cost -->

Order the `prioritized_models` list from cheapest to most expensive. When a request hits `localrouter/auto`, the cheapest available model is always attempted first. Combined with a cost-optimized RouteLLM threshold (e.g., 0.7), most requests route to the weak (cheaper) tier.

<!-- @entry strategy-highest-performance -->

Place the most capable models first in `prioritized_models`. The top-tier model handles every request unless it fails, in which case the router falls back to the next best model. Combined with a quality-prioritized RouteLLM threshold (e.g., 0.2), most requests go to the strong tier.

<!-- @entry strategy-local-first -->

Place local providers (Ollama, LM Studio) at the top of `prioritized_models` with remote providers as fallbacks. Requests route to locally-running models by default, avoiding API costs and network latency. If the local provider is down, the router falls through to remote providers.

<!-- @entry strategy-remote-first -->

Place cloud providers (OpenAI, Anthropic, etc.) at the top of `prioritized_models` with local models as fallbacks. This prioritizes the quality and speed of remote API models while maintaining resilience — if the remote provider is rate-limited or unreachable, requests automatically fail over to local models.

<!-- @entry free-tier-mode -->

**Free-Tier Mode** restricts the router to only use providers with available free-tier capacity. Enable it by toggling `free_tier_only` on a strategy. When active, each candidate model is classified before the request is attempted — models from providers without free-tier availability are skipped entirely.

The router understands that providers have fundamentally different free-tier models. Each provider declares a `FreeTierKind` that determines how free-tier availability is tracked and enforced.

<!-- @entry free-tier-types -->

Each provider is assigned one of six free-tier types. The type determines how the router tracks usage and decides whether the provider still has free capacity.

**No Free Tier** (`None`) — The provider has no free API access. All requests are treated as paid. When free-tier mode is enabled on a strategy, this provider is always skipped. Default for: OpenAI, Anthropic.

**Always Free (Local)** (`AlwaysFreeLocal`) — A local or self-hosted provider with no external billing. Always treated as free with no usage limits tracked by the router. Default for: Ollama, LM Studio, OpenAI Compatible.

**Subscription** (`Subscription`) — Access is included in an existing subscription plan. Always treated as free with no usage counters tracked. Default for: GitHub Copilot.

**Rate Limited** (`RateLimitedFree`) — Free access within rate limits imposed by the provider. The router tracks usage against six configurable limits:

- **Requests per Minute (RPM)** — Maximum API calls allowed per 60-second window. Resets automatically every minute.
- **Requests per Day (RPD)** — Maximum API calls per calendar day. Resets at midnight (UTC for most providers; PT for Gemini).
- **Tokens per Minute (TPM)** — Maximum input + output tokens per 60-second window.
- **Tokens per Day (TPD)** — Maximum tokens per calendar day.
- **Monthly Call Limit** — Total API calls allowed per calendar month. Resets on the 1st. Used by Cohere (1,000 calls/month).
- **Monthly Token Limit** — Total tokens allowed per calendar month. Resets on the 1st. Used by Mistral (1B tokens/month).

Set any limit to 0 to disable tracking for that dimension. When any tracked limit is reached, the provider is skipped in the routing chain. Default for: Gemini, Groq, Cerebras, Mistral, Cohere.

**Credit Based** (`CreditBased`) — Dollar-budget credits that are consumed per request. The router estimates cost from token usage and model pricing from the catalog, then compares accumulated spend against the configured budget. Configuration fields:

- **Credit Budget (USD)** — The total free credit allowance. When estimated spend reaches this amount, the provider is treated as exhausted.
- **Reset Period** — When the budget resets: **Daily** (resets every 24 hours), **Monthly** (resets on the 1st of each month), or **One-time** (never resets — for promotional credits that expire).

Cost estimation is local by default. For providers that expose a balance API (currently OpenRouter via `GET /api/v1/key`), the router can sync the actual remaining balance directly. Default for: OpenRouter, xAI, DeepInfra, Perplexity.

**Free Models Only** (`FreeModelsOnly`) — Only specific models from this provider are free. The router checks each model ID against a list of patterns. Models that don't match are treated as paid and skipped in free-tier mode. Configuration fields:

- **Free Model Patterns** — A list of model IDs (one per line) that are free. Only requests targeting these exact model IDs are allowed in free-tier mode.
- **Requests per Minute (RPM)** — Rate limit applied to the free models. Set to 0 to disable.

Default for: Together AI (free model: `meta-llama/Llama-3.3-70B-Instruct-Turbo-Free`).

<!-- @entry free-tier-override -->

Every provider has a default free-tier type assigned automatically based on the provider type (e.g., Groq defaults to Rate Limited, OpenAI defaults to No Free Tier). You can **override** this default on any provider instance from the provider's Free Tier tab.

Common override scenarios:

- **Reclassify a provider** — Change an OpenAI-compatible provider from "Always Free (Local)" to "Rate Limited" if your self-hosted endpoint has rate limits, or to "Credit Based" if it bills per-token.
- **Adjust limits** — A provider changed their free tier limits and the defaults are outdated. Override with the correct values.
- **Add a free tier** — Your organization has a custom agreement with a provider that normally has no free tier. Override "No Free Tier" to "Credit Based" with your budget.
- **Remove a free tier** — You don't want a provider used in free-tier mode even though it technically has one. Override to "No Free Tier".

Use **Reset to Default** to remove the override and revert to the provider type's built-in default.

<!-- @entry free-tier-set-usage -->

The router tracks usage locally based on token counts from API responses. Since this is an estimate, it can drift from your actual usage — especially for credit-based providers where the provider's internal accounting may differ.

**Set Usage** lets you manually adjust the tracked counters to match reality. Open it from the Usage & Status section of a provider's Free Tier tab.

For **credit-based** providers:
- **Credits Used (USD)** — Sets how much of the budget has been consumed this period. The router uses `budget - used` to calculate remaining capacity.
- **Credits Remaining (USD)** — Sets the actual remaining balance directly (e.g., from your provider dashboard). This overrides the calculated estimate and is the most accurate way to sync.

For **rate-limited** providers:
- **Daily Requests Used** — Number of requests counted toward the daily limit (RPD).
- **Monthly Requests Used** — Number of requests counted toward the monthly call cap.
- **Monthly Tokens Used** — Total tokens counted toward the monthly token cap.

**Reset Usage** clears all counters to zero, as if the provider were freshly added. It also clears any active backoff state.

<!-- @entry free-tier-tracking -->

The `FreeTierManager` tracks usage per provider using two systems. **Rate limit tracking** maintains client-side counters (RPM, RPD, TPM, TPD, monthly calls, monthly tokens) with automatic window resets. When a provider returns rate limit response headers, header-reported values take precedence over client-side counters. **Credit tracking** estimates cost from token usage and compares against the configured budget, with support for daily, monthly, and one-time reset periods.

A **universal rate limit header parser** handles all known naming conventions without per-provider code: `x-ratelimit-remaining-requests` (OpenAI/Groq/xAI), `x-ratelimit-remaining-requests-day` (Cerebras), `x-ratelimit-remaining`/`x-tokenlimit-remaining` (Together AI), and `anthropic-ratelimit-requests-remaining` (Anthropic). Custom OpenAI-compatible providers automatically benefit from header parsing if their backend returns standard headers.

For OpenRouter, the router can also query the `GET /api/v1/key` endpoint to sync credit balance and free-tier status directly from the provider.

<!-- @entry free-tier-backoff -->

When a provider returns 429 (rate limited) or 402 (credits exhausted), the `FreeTierManager` records a backoff for that provider-model pair. Subsequent requests skip backed-off providers instantly — no wasted round-trips to providers already known to be unavailable.

Backoff duration is resolved from (in priority order): the `retry-after` response header, the `x-ratelimit-reset-*` header, credit replenishment schedule, or exponential backoff (1s → 2s → 4s → ... → 60s max). A successful request clears the backoff immediately.

When all free-tier providers are exhausted, the router returns HTTP 429 with a `retry-after` header set to the minimum remaining backoff across all providers, telling the client exactly when to retry.

<!-- @entry error-classification -->

The `RouterError` enum classifies provider errors into five categories: `RateLimited`, `PolicyViolation`, `ContextLengthExceeded`, `Unreachable`, and `Other`.

Classification inspects the `AppError` variant and matches error message strings for keywords. The `should_retry()` method returns `true` for the first four categories, enabling automatic fallback. Only `Other` errors halt the retry loop immediately.

<!-- @entry error-rate-limited -->

When a provider returns a rate limit error, it is classified as `RateLimited` with a default `retry_after_secs` of 60 seconds. Rather than waiting for the cooldown, the router immediately advances to the next model in the fallback chain.

<!-- @entry error-policy-violation -->

Content policy violations are classified as `PolicyViolation`. Despite being caused by content rather than infrastructure, this error is retryable — the router attempts the next model in the chain, which may have different content policies.

This allows requests rejected by one provider's safety filters to potentially succeed with a different provider.

<!-- @entry error-context-length -->

When a provider rejects a request because the input exceeds its context window, the error is classified as `ContextLengthExceeded`. This is retryable, enabling automatic failover to a model with a larger context window in the fallback chain.
