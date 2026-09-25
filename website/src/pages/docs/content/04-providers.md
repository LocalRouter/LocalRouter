<!-- @entry supported-providers -->

LocalRouter supports these providers out of the box:

**Local Embedded Providers**: Laya, Kev, Von, Decider. LocalRouter starts and stops these engines itself; you install them once with your package manager (see Local Embedded Providers).

**Cloud Providers**: OpenAI, Anthropic, Google Gemini, Mistral, Cohere, xAI (Grok), Perplexity

**Aggregators and gateways**: OpenRouter, LLM Gateway, Vercel AI Gateway, Cloudflare Workers AI, Together AI, DeepInfra, Groq, Cerebras

**Local Providers**: Ollama, LM Studio, Jan, GPT4All, LocalAI, llama.cpp

**System One (decision) providers**: TypeSafe (Jev), the Local Embedded providers Laya, Kev, Von and Decider, and any System One compatible server. These answer `POST /systemone` decision requests only, not chat. OpenRouter, LLM Gateway, Vercel AI Gateway and Cloudflare Workers AI also serve TypeSafe Jev alongside their chat models.

**Generic**: Any OpenAI-compatible endpoint via the generic provider adapter

Provider-specific quirks (auth headers, model ID formats, streaming behavior) are handled internally — you always use the standard OpenAI request format regardless of which provider handles the request.

<!-- @entry local-embedded-providers -->

Local Embedded providers run an inference engine on your machine that LocalRouter launches and manages. You do not start a server or enter a URL. You install the engine once with your own package manager, and LocalRouter finds it on your PATH (or at the file you choose in the Engine tab). The one exception is stable-diffusion.cpp, which no package manager carries on macOS or Windows: its Engine tab can download the latest official release for your computer when you click Download.

- **Engine tab**: each Local Embedded provider has an Engine tab. It shows whether the engine was found, lists the install commands for your operating system with a Copy button, and can run commands that need no password prompt (Install button) with live output. Click Refresh after installing from a terminal.
- **Requirements**: Laya, Kev, Von and Decider are Python engines installed and run with Astral's `uv`. If `uv` is missing, the Engine tab offers its install first (`brew install uv` on macOS, or `curl -LsSf https://astral.sh/uv/install.sh | sh`).
- **Adding one**: the Add Provider form asks only for a name. Engine settings are optional and live in the provider's Settings tab.
- **Models tab**: download models here before using them. llama.cpp models come from a Hugging Face search or a local GGUF file; Laya, Kev, Von and Decider list their checkpoints with sizes, and Download runs the engine once to fetch one from Hugging Face (Progress shows the engine output). Requests never download: a model that is not downloaded fails at once with an error naming the Models tab, and it does not appear in model lists.
- **Lifecycle**: the engine starts on the first request for a downloaded model, or when you click Load, and stops after it has been idle for `idle_unload_minutes` (default 15, `0` keeps it running). The Engine tab lists running engine processes with their port, uptime and logs, and can stop them.
- **Offline serving**: engines serving requests run with Hugging Face offline mode and telemetry off. Only a download you start contacts Hugging Face.
- **Local only**: engines listen on `127.0.0.1` on a free port. Where the engine supports it, LocalRouter passes a new API key on every launch, so other programs on the machine cannot use it.
- **Platforms**: macOS on Apple Silicon, Windows and Linux. The Python engines are not available on Intel Macs, because current PyTorch releases no longer support them.

**stable-diffusion.cpp (image generation)**: LocalRouter runs `sd-server` for `POST /v1/images/generations` (model `<provider>/<model>`, for example `stable-diffusion.cpp/z-image-turbo`). In the Engine tab, choose an `sd-server` you already have, or click Download to fetch the latest release from github.com/leejet/stable-diffusion.cpp: Metal on Apple Silicon Macs; Vulkan (recommended), CUDA 12, CPU or ROCm builds on Windows and Linux. Image models are bundles of three files (diffusion weights, VAE and text encoder), downloaded together in the Models tab:

- **FLUX.2 Klein 4B** (about 5.3 GB): fast 4-step generation.
- **Z-Image Turbo** (about 6.7 GB): photorealistic 8-step generation, good with text in images.
- **Qwen-Image 2.1** (about 10 GB): strong prompt following and text rendering.

A Qwen-Image 2.1 GGUF you downloaded or imported in the llama.cpp Models tab (for example a fine-tune) is listed as its own image model and only needs the Qwen-Image 2.1 VAE and text encoder. One image model is loaded at a time. sd-server has no API key option, so it only ever listens on `127.0.0.1` behind LocalRouter's own authentication. Responses carry base64 images; `response_format: "url"` returns a `data:` URL.

Local Embedded providers are always free.

<!-- @entry systemone-providers -->

System One providers answer typed decisions (`POST /systemone`) with calibrated probabilities. Decision models carry the `decision` capability; in model pickers, use the **Decision** filter to find them. Decision-only providers cannot chat, so their models are only used for System One requests. Add them in Resources → Providers like any other provider.

**Local Embedded providers** (see Local Embedded Providers for how LocalRouter runs them):

- **Laya**: Laya decision models (typed choice, score and yes/no answers). Install with `uv tool install --python 3.12 "laya[serve]"` (add `--torch-backend auto` for a PyTorch build matching your CUDA driver). LocalRouter runs `laya-serve` with a per-launch API key. Models: `english`, `multilingual`, `typed-decisions`; one engine process serves every downloaded checkpoint. Settings also cover `device` (`auto`, `cpu`, `cuda`, `mps`) and CPU `threads`.
- **Kev**: Qwen-based decision models. Kev has no package of its own: LocalRouter runs it through `uv` from a pinned commit of `github.com/jaredpalmer/kev`, so installing `uv` is enough. The Engine tab's optional Prepare command fetches Kev and PyTorch ahead of the first download. Models: `kev-0.8b` (1.8 GB, runs on any Apple Silicon Mac), `kev-4b` (9.5 GB), `kev-9b` (19.5 GB, needs a GPU with about 17 GB VRAM). Each checkpoint runs in its own engine process.
- **Von**: a ModernBERT-large decision model. Install with `uv tool install --python 3.12 von-sdk`. LocalRouter runs `von serve` on localhost with a per-launch API key and sends a warm-up request when it starts, because Von loads its model on the first decision. Downloading `von-latest` (about 3.2 GB) in the Models tab runs that warm-up online once. One model: `von-latest`. The `device` setting accepts `auto`, `cuda`, `rocm`, `mps`, `openvino`, `dml` or `cpu`.
- **Decider**: Qwen-based decision models. Like Kev, it runs through `uv` (`uv tool run --from "decider-ai[serve]" uvicorn decider.serve:app`, with the `metal` extra added on Apple Silicon), so installing `uv` is enough. Decider has no authentication, so LocalRouter binds it to `127.0.0.1` only. Models: `decider-0.8b` (1.5 GB), `decider-2b` (3.8 GB), `decider-4b` (8.4 GB). Each checkpoint runs in its own engine process.

**Hosted providers**:

- **TypeSafe (Jev)**: TypeSafe's hosted Jev model (`jev-latest`, `jev-preview`, or a pinned version such as `jev-1.13.0`). Create an API key at `https://console.typesafe.ai/keys` and paste it into the provider form. The base URL defaults to `https://api.typesafe.ai`.
- **System One compatible**: any other server that implements `POST /v1/systemone`. Enter its base URL (the part before `/v1/systemone`) and an optional Bearer key. Examples include OpenJev, codesoda's `systemone` (`s1 serve`, port 8080), jev-agent.com, and LiteLLM's `/typesafe` passthrough route.

**Gateways serving TypeSafe Jev**: these providers are chat providers that also serve Jev natively on `POST /v1/systemone`. Their Jev models appear in the model list with the `decision` capability, and their chat models can still answer System One questions through the translation layer (see POST /systemone).

- **OpenRouter**: `typesafe/jev-1.13` and `~typesafe/jev-latest` (used when a request names no model). LocalRouter discovers them from OpenRouter's decision model listing (`?output_modalities=decisions`).
- **LLM Gateway**: `jev-1.13.0`, also reachable through aliases such as `jev-latest`. The base URL defaults to `https://api.llmgateway.io/v1`; change it only for a self-hosted gateway.
- **Vercel AI Gateway**: `typesafe-ai/jev`. LocalRouter sends decisions to Vercel's TypeSafe-compatible API, which keeps the confidence and legend fields. The base URL defaults to `https://ai-gateway.vercel.sh/v1`.
- **Cloudflare Workers AI**: the partner model `typesafe/jev`, called through the account's `/ai/run` endpoint. The base URL must identify your account (`https://api.cloudflare.com/client/v4/accounts/{account_id}/ai/v1`) or be an AI Gateway URL.

Jev through these gateways is priced at $0.042 per million input tokens; output tokens are free. The Local Embedded providers are always free. A System One provider is optional: chat providers can also answer `/systemone` through LocalRouter's translation layer (see POST /systemone).

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
