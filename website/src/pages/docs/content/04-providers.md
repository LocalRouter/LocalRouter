<!-- @entry supported-providers -->

LocalRouter supports 19 LLM providers out of the box:

**Cloud Providers**: OpenAI, Anthropic, Google Gemini, Mistral, Cohere, xAI (Grok), Perplexity

**Aggregators**: OpenRouter, Together AI, DeepInfra, Groq, Cerebras

**Local Providers**: Ollama, LM Studio

**Generic**: Any OpenAI-compatible endpoint via the generic provider adapter

Provider-specific quirks (auth headers, model ID formats, streaming behavior) are handled internally — you always use the standard OpenAI request format regardless of which provider handles the request.

<!-- @entry adding-provider-keys -->

Provider API keys are added through the UI's Resources view. When you add a key, it is stored in your OS keychain — never written to config files. The provider entry in the config only stores metadata (provider type, enabled status, custom base URL).

After adding a key, LocalRouter queries the provider's model list to populate the model catalog with available models.

<!-- @entry provider-health-checks -->

LocalRouter tracks provider health through two mechanisms: a circuit breaker for fault isolation and latency tracking for performance monitoring.

Both operate automatically in the background. Unhealthy providers are temporarily skipped during routing, with automatic recovery when the provider stabilizes.

<!-- @entry circuit-breaker -->

The circuit breaker tracks consecutive failures per provider and transitions through three states: **Closed** (healthy, requests pass through), **Open** (unhealthy, requests are immediately rejected), and **Half-Open** (recovery, a single test request is allowed through).

After a configurable number of consecutive failures, the breaker opens and remains open for a cooldown period. During half-open, a single request is permitted — if it succeeds, the breaker closes; if it fails, it reopens. This prevents cascading failures when a provider is down.

<!-- @entry latency-tracking -->

Each request's round-trip latency is recorded and visible in the dashboard. The monitoring system calculates P50, P95, and P99 latency percentiles per provider, per model, and globally.

These metrics can inform routing decisions — for example, a strategy could prioritize providers with the fastest recent response times.

<!-- @entry feature-adapters -->

Feature adapters extend base provider capabilities with opt-in features. Rather than every provider needing to support every feature, adapters are registered per-provider based on what that provider actually supports. This ensures feature requests are only sent to compatible providers.

<!-- @entry prompt-caching -->

Prompt caching reduces latency and cost for repeated prefixes. When enabled, the provider stores the computation for your system prompt or conversation prefix and reuses it on subsequent requests.

Cache hit rates and savings are tracked in the monitoring dashboard. Supported by Anthropic, OpenAI, Google Gemini, and DeepInfra.

<!-- @entry json-mode -->

JSON mode forces the model to return valid JSON in its response. Set `response_format: { type: "json_object" }` in your request, and LocalRouter applies the appropriate provider-specific parameters automatically.

Supported by OpenAI, Anthropic, Gemini, Mistral, Groq, and others.

<!-- @entry structured-outputs -->

Structured outputs extend JSON mode by enforcing a specific JSON Schema on the response. Include `response_format: { type: "json_schema", json_schema: { ... } }` with a full schema definition in your request.

LocalRouter translates this into the correct format for each provider. Supported by OpenAI, Gemini, and select other providers.

<!-- @entry logprobs -->

The logprobs feature surfaces token-level log probabilities from the model's output. Set `logprobs: true` and optionally `top_logprobs: N` in your request to receive per-token probability data.

Useful for confidence scoring, calibration, and advanced prompting techniques. Supported by OpenAI, Groq, and other providers that expose logprob data.
