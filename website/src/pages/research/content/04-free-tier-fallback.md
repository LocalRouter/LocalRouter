<!-- @entry free-tier-abstract -->

**Free-Tier Mode with Paid Fallback** is a routing strategy that maximizes usage of free API tiers across LLM providers while providing configurable fallback behavior when free capacity is exhausted. The system models five distinct free-tier types (always-free local, subscription-included, rate-limited, credit-based, and none), implements a universal rate-limit header parser that works across all known provider formats, and coordinates backoff state across all clients to prevent thundering herd retries.

<!-- @entry free-tier-problem -->

The LLM provider landscape offers a patchwork of free tiers: Gemini provides rate-limited free access (10 RPM, 250 RPD), OpenRouter offers credit-based free usage ($0 initial credits, community models), Ollama and LM Studio are always free (local), and providers like OpenAI have no free tier at all.

Users who want to minimize costs face several challenges:

1. **Heterogeneous rate limit formats**: Every provider uses different HTTP headers. Groq uses `x-ratelimit-remaining-requests`, Anthropic uses `anthropic-ratelimit-requests-remaining`, Cerebras adds `-day` and `-minute` suffixes, Together AI uses `x-tokenlimit-remaining`
2. **Mixed free/paid models**: Within a single provider, some models may be free while others are paid
3. **Thundering herd problem**: When a free provider hits its rate limit, all concurrent clients retry the same provider simultaneously, worsening the situation
4. **User consent for cost**: Automatically falling back to paid models without user awareness can lead to unexpected charges

<!-- @entry free-tier-taxonomy -->

The system models free tiers as a discriminated union with five variants:

**AlwaysFreeLocal** — Local/self-hosted providers (Ollama, LM Studio). No limits, no tracking needed. Always available as fallback.

**Subscription** — Included in an existing subscription (GitHub Copilot). Usage is effectively unlimited within subscription terms.

**RateLimitedFree** — Rate-limited access with no dollar cost (Gemini, Groq, Cerebras, Mistral, Cohere). Tracked across multiple windows:

```
Requests:  per-minute (RPM), per-day (RPD), per-month
Tokens:    per-minute (TPM), per-day (TPD), per-month
```

**CreditBased** — Dollar-based credits with optional reset periods (OpenRouter, xAI, DeepInfra, Perplexity). Detection can be local-only (client-side cost estimation), provider API (query balance endpoint), or custom endpoint.

**None** — No free tier (OpenAI, Anthropic). These models are always classified as paid.

Each provider declares its default free-tier type via a factory method. Users can override per-instance (e.g., marking a self-hosted provider as `AlwaysFreeLocal`).

<!-- @entry free-tier-tracking -->

The `FreeTierManager` is a shared-state component (one instance across all clients) that tracks usage and backoff state per provider.

**Rate limit tracking** combines two data sources:

1. **Header-reported values** (preferred): Parsed from response headers after each successful request. A universal parser handles all known formats:

```
OpenAI/Groq/xAI:  x-ratelimit-remaining-requests, x-ratelimit-limit-tokens
Cerebras:          x-ratelimit-remaining-requests-day, x-ratelimit-limit-tokens-minute
Together AI:       x-ratelimit-remaining, x-tokenlimit-remaining
Anthropic:         anthropic-ratelimit-requests-remaining
Universal:         retry-after (seconds or HTTP-date), retry-after-ms
```

2. **Client-side counting** (fallback): When headers are unavailable, the manager counts requests and tokens per window.

Header values always take precedence because they reflect the true server-side state, including usage from other applications sharing the same API key.

**Credit tracking** estimates cumulative USD cost per billing period using model pricing data and token counts. For providers with API-based detection (e.g., OpenRouter), the actual remaining balance is periodically queried.

<!-- @entry free-tier-backoff -->

When a provider returns 429 (rate limited) or 402 (payment required), the manager enters a **coordinated backoff** state for that provider-model pair. The backoff duration is determined by a priority chain:

1. `retry-after` response header (most accurate — provider tells us exactly when to retry)
2. `x-ratelimit-reset-*` header calculations (provider tells us when the window resets)
3. Exponential backoff for rate limits: 1s, 2s, 4s, 8s, 16s, 32s, 60s cap
4. Longer exponential backoff for credit exhaustion: 5min, 15min, 1hr, 6hr, 24hr cap

**Cross-client coordination**: Because the `FreeTierManager` is shared, when Groq hits its rate limit serving Client A, Clients B and C immediately see the backoff and skip Groq in their routing — preventing the thundering herd problem where every client independently discovers and retries the same exhausted provider.

<!-- @entry free-tier-fallback -->

When all free-tier providers are exhausted for a request, the behavior depends on the strategy's `free_tier_fallback` setting:

**Off** (default): Return HTTP 429 with a `retry-after` header set to the minimum backoff duration across all free-tier providers. The client knows exactly when to retry.

**Ask**: Trigger a real-time approval popup in the LocalRouter UI. The user sees which models are exhausted and can choose:

| Action | Effect |
|--------|--------|
| Deny | Return 429 to the client |
| Allow Once | Retry this single request with paid models |
| Allow 1 Minute | Auto-approve paid fallback for the next minute |
| Allow 1 Hour | Auto-approve paid fallback for the next hour |
| Allow Permanent | Always fall back to paid models |

Time-based approvals are tracked per-client. Once approved for 1 hour, subsequent free-tier exhaustion events within that window automatically fall back without showing the popup — reducing approval fatigue during sustained usage.

**Allow**: Automatically proceed with paid models whenever free-tier capacity is exhausted. No user interaction required.

<!-- @entry free-tier-routing -->

The free-tier system integrates with the routing engine at three points:

**Pre-routing filter**: Before attempting each provider, the router checks `is_in_backoff(provider, model)`. If the provider is backed off, it's skipped immediately — no request is sent.

**Classification filter**: When `free_tier_only` is enabled, each candidate model is classified via `classify_model()`:

```
AlwaysFree / FreeWithinLimits → Allow (proceed with request)
NotFree                       → Skip (try next provider)
```

**Post-request recording**: After each successful request, `update_from_headers()` parses response headers and `record_usage()` updates token/request counts. After each error, `record_rate_limit_error()` computes and stores the backoff duration.

When all free providers are filtered out, the router generates a specific error type (`FreeTierExhausted` or `FreeTierFallbackAvailable`) that the chat route handler intercepts to implement the fallback flow.

<!-- @entry free-tier-results -->

The system makes the "free-tier first" strategy practical by handling the full complexity of provider heterogeneity behind a single boolean toggle (`free_tier_only: true`).

| Provider | Free Tier Type | Default Limits |
|----------|---------------|----------------|
| Ollama, LM Studio | AlwaysFreeLocal | Unlimited |
| Gemini | RateLimitedFree | 10 RPM, 250 RPD |
| Groq | RateLimitedFree | 30 RPM, 14.4K RPD |
| Cerebras | RateLimitedFree | 30 RPM, 1K RPD |
| OpenRouter | CreditBased | Community models free |
| OpenAI, Anthropic | None | N/A |

**Key design properties**:

- **Zero per-provider code**: Providers only declare their `FreeTierKind` — all tracking logic is generic
- **Universal header parsing**: Custom OpenAI-compatible providers automatically benefit from header-based tracking
- **Shared backoff state**: Prevents thundering herd across all clients
- **Persisted usage**: Rate/credit counters survive app restart (backoff state is in-memory only)
- **Smart retry-after**: When returning 429, the API includes the exact time until the next free-tier window opens
- **Approval memory**: Time-based approvals reduce popup fatigue during sustained usage
