<!-- @entry supported-providers -->

LocalRouter supports 19 LLM providers out of the box:

**Cloud Providers**: OpenAI, Anthropic, Google Gemini, Mistral, Cohere, xAI (Grok), Perplexity

**Aggregators**: OpenRouter, Together AI, DeepInfra, Groq, Cerebras

**Local Providers**: Ollama, LM Studio

**Generic**: Any OpenAI-compatible endpoint via the generic provider adapter

Each provider implements the `LlmProvider` trait, which defines model listing, chat completions, text completions, and embeddings. Provider-specific quirks (auth headers, model ID formats, streaming behavior) are handled internally — consumers always use the standard OpenAI request format.

<!-- @entry adding-provider-keys -->

Provider API keys are added through the UI's Resources view or via configuration. When you add a key, it is stored in your OS keychain under the `LocalRouter-Providers` service — never written to config files. The provider entry in the config only stores metadata (provider type, enabled status, custom base URL).

After adding a key, LocalRouter queries the provider's model list endpoint to populate the model catalog with available models.

<!-- @entry provider-health-checks -->

LocalRouter tracks provider health through two mechanisms: a circuit breaker pattern for fault isolation and latency tracking for performance monitoring.

Both systems operate in-memory and update automatically with each request. Unhealthy providers are temporarily skipped during routing, with automatic recovery when the provider stabilizes.

<!-- @entry circuit-breaker -->

The circuit breaker tracks consecutive failures per provider and transitions through three states: **Closed** (healthy, requests pass through), **Open** (unhealthy, requests are immediately rejected), and **Half-Open** (recovery, a single test request is allowed through).

After a configurable number of consecutive failures, the breaker opens and remains open for a cooldown period. During half-open, a single request is permitted — if it succeeds, the breaker closes; if it fails, it reopens. This prevents cascading failures when a provider is down.

<!-- @entry latency-tracking -->

Each request's round-trip latency is recorded in the monitoring system with time-series bucketing. The metrics engine calculates P50, P95, and P99 latency percentiles per provider, per model, and globally.

These metrics are available in the dashboard and can inform routing decisions — for example, a "lowest latency" strategy could prioritize providers with the fastest recent response times.

<!-- @entry feature-adapters -->

Feature adapters extend base provider capabilities with opt-in features. Rather than every provider implementing every feature, adapters wrap provider responses to add functionality like prompt caching, JSON mode, structured outputs, and logprobs.

Each adapter is registered per-provider based on what that provider supports, ensuring feature requests are only sent to providers that can handle them.

<!-- @entry prompt-caching -->

Prompt caching reduces latency and cost for repeated prefixes. When enabled, the adapter marks the system prompt or conversation prefix with caching headers specific to the provider (e.g., Anthropic's `cache_control` blocks, OpenAI's `cached` field). The provider stores the prefix computation and reuses it on subsequent requests.

Cache hit rates and savings are tracked in the monitoring system. Supported by Anthropic, OpenAI, Google Gemini, and DeepInfra.

<!-- @entry json-mode -->

JSON mode forces the model to return valid JSON in its response. When `response_format: { type: "json_object" }` is set in the request, the adapter applies provider-specific parameters: OpenAI and compatible providers use the `response_format` field directly, while others may inject system prompt instructions.

The adapter validates that the response is parseable JSON before returning it. Supported by OpenAI, Anthropic, Gemini, Mistral, Groq, and others.

<!-- @entry structured-outputs -->

Structured outputs extend JSON mode by enforcing a specific JSON Schema on the response. The request includes `response_format: { type: "json_schema", json_schema: { ... } }` with a full schema definition.

The adapter translates this into provider-specific formats — OpenAI uses native structured outputs, while other providers may use tool-calling tricks or system prompt injection to approximate schema adherence. Supported by OpenAI, Gemini, and select other providers.

<!-- @entry logprobs -->

The logprobs adapter surfaces token-level log probabilities from the model's output. When `logprobs: true` and optionally `top_logprobs: N` are set in the request, the adapter ensures the provider returns per-token probability data in the response.

This is useful for confidence scoring, calibration, and advanced prompting techniques. Supported by OpenAI, Groq, and other providers that expose logprob data.
