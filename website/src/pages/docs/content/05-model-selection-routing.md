<!-- @entry auto-routing -->

When a client sends a request with the model set to `localrouter/auto`, the router activates auto-routing mode. Instead of targeting a specific provider/model pair, the router consults the strategy's auto-routing configuration to select from a prioritized list of models.

If RouteLLM is enabled, the classifier first determines whether a strong or weak model tier is appropriate; otherwise, the prioritized models list is used directly. The router tries each model in order until a request succeeds or all options are exhausted.

<!-- @entry routellm-classifier -->

The RouteLLM classifier is a machine learning model that runs entirely on your machine — no external API calls required. It analyzes each prompt and predicts whether a strong (more capable, more expensive) or weak (faster, cheaper) model is needed.

**Performance.** Classification takes approximately 15-20ms per prediction, adding negligible latency to requests.

**Resource usage.** The model uses ~2.5-3 GB of memory when loaded and supports GPU acceleration via Metal (macOS) and CUDA (Linux/Windows) with automatic CPU fallback.

<!-- @entry strong-weak-classification -->

The classifier outputs a score between 0.0 and 1.0 representing the probability that a strong model is needed. This score is compared against a configurable threshold: if the score meets or exceeds the threshold, the request routes to the strong model tier; otherwise, it routes to the weak tier.

Recommended thresholds:
- **0.2** — Quality-prioritized: most requests go to the strong tier
- **0.3** — Balanced (default): good mix of quality and cost savings
- **0.7** — Cost-optimized: most requests go to the weak tier

This approach typically achieves 30-60% cost savings while retaining 85-95% quality compared to always using the strong tier.

<!-- @entry fallback-chains -->

When auto-routing is active, the router tries each model in the prioritized list sequentially. If a request fails with a retryable error (rate limit, provider down, context too long), the router advances to the next model.

Rate limits are also checked before each attempt — if a model's limits are exceeded, it is skipped. The chain stops on the first successful response or when all models have been tried.

<!-- @entry provider-failover -->

Provider failover is automatic and based on the type of error received:

- **Rate limited** — The provider is at capacity. The router moves to the next model immediately.
- **Policy violation** — Content was rejected by the provider's safety filters. The router tries the next model, which may have different content policies.
- **Context length exceeded** — The input is too long for this model. The router tries the next model, which may have a larger context window.
- **Unreachable** — The provider is down or not responding. The router tries the next model.
- **Other errors** (e.g., validation failures) — These are not retryable and cause an immediate error response.

<!-- @entry offline-fallback -->

If the strategy's prioritized model list includes local providers (Ollama, LM Studio) alongside remote providers, the router automatically falls through to local models when remote providers are unreachable. No special configuration is needed — just include local models in your fallback chain.

<!-- @entry routing-strategies -->

A **Strategy** is the core routing configuration unit, referenced by each client. Each strategy defines:

- **Allowed models** — Which provider/model pairs the client can access (all, specific providers, or specific models)
- **Auto-routing config** — Prioritized model lists and RouteLLM settings for `localrouter/auto`
- **Rate limits** — Request, token, and cost limits per time window

Strategies are reusable — multiple clients can share the same strategy.

<!-- @entry strategy-lowest-cost -->

Order the prioritized models list from cheapest to most expensive. When a request hits `localrouter/auto`, the cheapest available model is always tried first. Combined with a cost-optimized RouteLLM threshold (e.g., 0.7), most requests route to the cheaper tier.

<!-- @entry strategy-highest-performance -->

Place the most capable models first in the prioritized list. The top-tier model handles every request unless it fails, in which case the router falls back to the next best model. Combined with a quality-prioritized RouteLLM threshold (e.g., 0.2), most requests go to the strong tier.

<!-- @entry strategy-local-first -->

Place local providers (Ollama, LM Studio) at the top of the prioritized list with remote providers as fallbacks. Requests route to locally-running models by default, avoiding API costs and network latency. If the local provider is down, the router falls through to remote providers.

<!-- @entry strategy-remote-first -->

Place cloud providers (OpenAI, Anthropic, etc.) at the top of the prioritized list with local models as fallbacks. This prioritizes the quality and speed of remote API models while maintaining resilience — if the remote provider is rate-limited or unreachable, requests automatically fail over to local models.

<!-- @entry free-tier-mode -->

**Free-Tier Mode** restricts the router to only use providers with available free-tier capacity. Enable it by toggling `free_tier_only` on a strategy. When active, models from providers without free-tier availability are skipped entirely.

Each provider has a free-tier type that determines how availability is tracked and enforced.

<!-- @entry free-tier-types -->

Each provider is assigned one of six free-tier types. The type determines how the router tracks usage and decides whether the provider still has free capacity.

**No Free Tier** — The provider has no free API access. Always skipped in free-tier mode. Default for: OpenAI, Anthropic.

**Always Free (Local)** — A local or self-hosted provider with no external billing. Always treated as free. Default for: Ollama, LM Studio, OpenAI Compatible.

**Subscription** — Access is included in an existing subscription plan. Always treated as free. Default for: GitHub Copilot.

**Rate Limited** — Free access within rate limits imposed by the provider. The router tracks usage against configurable limits:

- **Requests per Minute (RPM)** — Resets every minute.
- **Requests per Day (RPD)** — Resets at midnight.
- **Tokens per Minute (TPM)** — Input + output tokens per minute.
- **Tokens per Day (TPD)** — Tokens per calendar day.
- **Monthly Call Limit** — Total API calls per month. Used by Cohere (1,000 calls/month).
- **Monthly Token Limit** — Total tokens per month. Used by Mistral (1B tokens/month).

When any tracked limit is reached, the provider is skipped. Default for: Gemini, Groq, Cerebras, Mistral, Cohere.

**Credit Based** — Dollar-budget credits consumed per request. The router estimates cost from token usage and compares against the configured budget.

- **Credit Budget (USD)** — Total free credit allowance.
- **Reset Period** — When the budget resets: Daily, Monthly, or One-time (for promotional credits).

For providers that expose a balance API (currently OpenRouter), the router can sync the actual remaining balance directly. Default for: OpenRouter, xAI, DeepInfra, Perplexity.

**Free Models Only** — Only specific models from this provider are free. The router checks each model against a list of free model IDs. Non-matching models are treated as paid.

- **Free Model Patterns** — List of model IDs that are free.
- **Requests per Minute (RPM)** — Rate limit for the free models.

Default for: Together AI.

<!-- @entry free-tier-override -->

Every provider has a default free-tier type based on the provider kind. You can override this default from the provider's Free Tier tab.

Common override scenarios:

- **Reclassify a provider** — Change an OpenAI-compatible provider from "Always Free (Local)" to "Rate Limited" if your endpoint has rate limits.
- **Adjust limits** — A provider changed their free tier limits and the defaults are outdated.
- **Add a free tier** — Your organization has a custom agreement with a provider that normally has no free tier.
- **Remove a free tier** — You don't want a provider used in free-tier mode even though it has one.

Use **Reset to Default** to remove the override.

<!-- @entry free-tier-set-usage -->

The router tracks free-tier usage locally based on token counts from API responses. Since this is an estimate, it can drift from your actual usage.

**Set Usage** lets you manually adjust the tracked counters to match reality. Open it from the Usage & Status section of a provider's Free Tier tab.

For **credit-based** providers:
- **Credits Used (USD)** — Sets how much of the budget has been consumed this period.
- **Credits Remaining (USD)** — Sets the remaining balance directly (e.g., from your provider dashboard). This overrides the calculated estimate.

For **rate-limited** providers:
- **Daily Requests Used** — Requests counted toward the daily limit.
- **Monthly Requests Used** — Requests counted toward the monthly cap.
- **Monthly Tokens Used** — Tokens counted toward the monthly cap.

**Reset Usage** clears all counters to zero and clears any active backoff state.

<!-- @entry free-tier-tracking -->

Free-tier usage is tracked automatically in the background. For rate-limited providers, counters reset at the appropriate intervals (per-minute, daily, monthly). When a provider returns rate limit headers in its response, those values take precedence over local counters for more accurate tracking.

For credit-based providers, cost is estimated from token usage. Providers that expose a balance API (like OpenRouter) can sync their actual remaining balance directly.

<!-- @entry free-tier-backoff -->

When a provider returns a rate limit (429) or credits exhausted (402) error, the router records a backoff for that provider. Subsequent requests skip backed-off providers instantly, avoiding wasted round-trips.

Backoff duration is determined from the provider's response headers when available, or uses automatic exponential backoff. A successful request clears the backoff immediately.

When all free-tier providers are exhausted, the router returns a 429 error with a `Retry-After` header indicating when to try again.

<!-- @entry error-classification -->

Provider errors are classified into categories that determine whether the router retries with the next model or returns an error immediately:

- **Rate Limited** — Retryable. The router advances to the next model.
- **Policy Violation** — Retryable. A different provider may have different content policies.
- **Context Length Exceeded** — Retryable. The next model may have a larger context window.
- **Unreachable** — Retryable. The provider is down.
- **Other** — Not retryable. Returns an error immediately.

<!-- @entry error-rate-limited -->

When a provider returns a rate limit error, the router immediately advances to the next model in the fallback chain rather than waiting for a cooldown.

<!-- @entry error-policy-violation -->

Content policy violations are retryable — the router tries the next model, which may have different content policies. This allows requests rejected by one provider's safety filters to potentially succeed with a different provider.

<!-- @entry error-context-length -->

When a provider rejects a request because the input exceeds its context window, the router tries the next model in the fallback chain, which may support a larger context window.
