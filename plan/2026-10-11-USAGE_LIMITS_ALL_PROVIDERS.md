# Usage limits for every provider that reports them

Extends usage-limits tracking (2026-10-10-SUBSCRIPTION_USAGE_TRACKING,
2026-10-10-USAGE_TRACKING_FOLLOWUPS) from OpenAI / Anthropic / ChatGPT /
Copilot / OpenRouter to every provider LocalRouter supports that documents
rate-limit headers or a usage / credits endpoint.

## Progress

- [x] 1. Research every provider's official docs (headers, endpoints, quotas)
- [x] 2. Provider-aware header parsing onto the quota model (period-aware ids)
- [x] 3. Attribution: hosts → provider family, provider types → accounts
- [x] 4. Polling of documented usage / credits endpoints (opt-in, back-off)
- [x] 5. Settings → Usage per-provider switches from a backend list
- [x] 6. Dashboard API cards render any reported limits; spend-only cards
- [x] 7. Types + demo mock in sync, demo accounts, screenshots
- [x] 8. Plan review, test coverage review, bug hunt
- [x] 9. Commit (no push, per the task)

## Findings per provider (official docs, checked 2026-10-11)

"None documented" means nothing in the official docs; passive parsing still
reads any generic `x-ratelimit-*` triple that arrives, without a period.

| Provider (type) | Rate-limit headers | Usage / credits endpoint with the inference key | Supported here |
|---|---|---|---|
| OpenAI (`openai`) | `x-ratelimit-{limit,remaining,reset}-{requests,tokens}` (+ `-project-tokens`), resets Go durations (`1s`, `6m0s`); period not stated in headers | None (Usage/Costs API needs an Admin key) | Headers (unchanged ids `requests`, `tokens`) |
| Anthropic (`anthropic`) | `anthropic-ratelimit-{requests,tokens,input-tokens,output-tokens}-{limit,remaining,reset}` (RFC 3339), plus `anthropic-priority-*` and `anthropic-fast-*` | None (Usage/Cost and Rate Limits APIs are Admin-only) | Headers, now incl. priority / fast |
| Groq (`groq`) | `x-ratelimit-*-requests` = **per day**, `-tokens` = **per minute**, resets `2m59.56s` / `7.66s`, on every response | None (Prometheus metrics Enterprise-only) | Headers → `requests_day`, `tokens_minute` |
| Cerebras (`cerebras`) | `x-ratelimit-*-requests-day`, `-tokens-minute`, resets float seconds (documented in the 2025-12 docs; current page no longer lists them) | None | Headers (names carry the period) |
| Mistral (`mistral`) | Only `X-RateLimit-Remaining`, no semantics | Admin key only (`/v1/admin/usage`, `/rate-limit`) | Generic headers (no period) |
| xAI (`xai`) | None documented | `GET /v1/api-key` has no balance; billing needs a management key | Spend only |
| Together AI (`togetherai`) | `x-ratelimit-{limit,remaining,reset}` (requests **per second**), `x-tokenlimit-{limit,remaining}` (tokens per second) (2025-12 docs) | `GET /v1/billing/usage` is beta, per-org gated (404 otherwise); no balance endpoint | Headers → `requests_second`, `tokens_second`; endpoint not polled |
| DeepInfra (`deepinfra`) | None documented (200 concurrent requests per model) | `GET /payment/checklist` (Bearer); units of `stripe_balance` / `recent` / `limit` undocumented | Spend only; not polled (dollars vs cents undocumented) |
| Gemini (`gemini`) | None documented | None (AI Studio only) | Spend only |
| Perplexity (`perplexity`) | `Retry-After` on 429 only | None (console) | Spend only |
| Cohere (`cohere`) | None documented | `POST /v1/check-api-key` has no limits | Spend only |
| OpenRouter (`openrouter`) | `X-RateLimit-*` only on its own 429s | `GET /api/v1/key`: `limit`, `limit_remaining`, `limit_reset`, `usage_{daily,weekly,monthly}`, `free_model_daily_requests` | Polled (key limit or this month's spend, free-model requests/day) |
| LLM Gateway (`llmgateway`) | IETF `RateLimit-Policy: "requests";q=600;w=60` + `RateLimit: "requests";r=599;t=60`; legacy triples get overwritten by spend caps | `GET /v1/key`: `usage`, `limit`, DevPass `devPlan*` credits and weekly premium window | Headers → `requests_minute`; polled (spend/limit, DevPass windows) |
| Vercel AI Gateway (`vercel_ai_gateway`) | None (only `Retry-After: 60` on one endpoint) | `GET /v1/credits` → `{"balance","total_used"}` | Polled (credits) |
| DigitalOcean (`digitalocean`) | `x-ratelimit-*-{requests,tokens-per-minute,tokens-per-day}`, resets epoch seconds, `0` = none pending | None | Headers → `requests_minute`, `tokens_minute`, `tokens_day` |
| NVIDIA NIM (`nvidia_nim`) | None documented (credits removed 2025) | None | Spend only |
| Cloudflare Workers AI (`cloudflare_ai`) | None documented | None (dashboard) | Spend only (`api.cloudflare.com` paths containing `/ai/`) |
| Hugging Face (`huggingface`) | None for the router (Hub's IETF headers are for Hub APIs) | Billing APIs Enterprise-only or unspecified token type | Spend only |
| LLM7 (`llm7`) | None documented | None | Spend only |
| kluster.ai (`kluster_ai`) | Inference shut down 2025-07 | — | Spend only |
| Zhipu (`zhipu`) | None (concurrency only) | Coding-plan quota endpoint is undocumented | Spend only; undocumented endpoint not polled |
| GitHub Models (`github_models`) | Service retired 2026-07-30 | — | Spend only |
| OpenCode Zen (`opencode_zen`) | None documented | None (console) | Spend only |
| OpenCode Go (`opencode_go`, subscription) | None documented | None (console) | Subscription card, value only |
| TypeSafe (`typesafe`) | None (429 + backoff) | None | Spend only |
| GitHub Copilot (`github-copilot`) | None documented | `copilot_internal/user` is private (already polled, unchanged) | Unchanged |
| ChatGPT Plus/Pro (`openai-chatgpt-plus`) | `x-codex-*` | `wham/usage` (unchanged) | Unchanged |
| Ollama, LM Studio, Jan, GPT4All, LocalAI, llama.cpp | None (LocalAI: 429/503 + `Retry-After` for concurrency) | Ollama Cloud documents `/api/balance`, but LocalRouter's Ollama provider has no API key / cloud mode | Local: not tracked |
| OpenAI-compatible generic (`openai_compatible`) | — | — | Attributed to the API its base URL points at (e.g. `api.groq.com` → Groq) |

Doc URLs:
- OpenAI: https://developers.openai.com/api/docs/guides/rate-limits, https://developers.openai.com/api/docs/guides/admin-apis
- Anthropic: https://platform.claude.com/docs/en/api/rate-limits, https://platform.claude.com/docs/en/manage-claude/usage-cost-api, https://platform.claude.com/docs/en/build-with-claude/fast-mode
- Groq: https://console.groq.com/docs/rate-limits, https://console.groq.com/docs/prometheus-metrics, https://console.groq.com/docs/spend-limits
- Cerebras: https://inference-docs.cerebras.ai/support/rate-limits (headers: web.archive.org snapshot 2025-12-08), https://inference-docs.cerebras.ai/console/usage-monitoring
- Mistral: https://docs.mistral.ai/resources/known-limitations, https://docs.mistral.ai/admin/admin-api/usage-metrics
- xAI: https://docs.x.ai/developers/rate-limits.md, https://docs.x.ai/developers/rest-api-reference/inference/other.md, https://docs.x.ai/developers/management-api-guide.md
- Together: https://docs.together.ai/docs/serverless/rate-limits.md (headers: web.archive.org snapshot 2025-12-21 of /docs/rate-limits), https://docs.together.ai/reference/billing-usage.md
- DeepInfra: https://docs.deepinfra.com/account/rate-limits.md, https://docs.deepinfra.com/api-reference/billing/get-checklist.md
- Gemini: https://ai.google.dev/gemini-api/docs/rate-limits, https://ai.google.dev/gemini-api/docs/api-errors
- Perplexity: https://docs.perplexity.ai/docs/admin/rate-limits-usage-tiers.md
- Cohere: https://docs.cohere.com/docs/rate-limits.md, https://docs.cohere.com/reference/errors.md
- OpenRouter: https://openrouter.ai/docs/api/reference/limits, https://openrouter.ai/docs/api/api-reference/api-keys/get-current-key
- LLM Gateway: https://docs.llmgateway.io/resources/rate-limits, https://docs.llmgateway.io/v1_key_retrieve, https://docs.llmgateway.io/developers/devpass-usage
- Vercel AI Gateway: https://vercel.com/docs/ai-gateway/sdks-and-apis/rest-api
- DigitalOcean: https://docs.digitalocean.com/products/inference/reference/quota-specific-response-headers/
- NVIDIA NIM: https://docs.api.nvidia.com/nim/docs/api-quickstart
- Cloudflare: https://developers.cloudflare.com/workers-ai/platform/limits/
- Hugging Face: https://huggingface.co/docs/inference-providers/pricing, https://huggingface.co/docs/hub/rate-limits
- LLM7: https://docs.llm7.io/limits.md
- kluster.ai: https://kluster.ai/blog/end-of-life-announcement
- Zhipu: https://docs.bigmodel.cn/cn/api/rate-limit, https://docs.bigmodel.cn/cn/coding-plan/overview
- GitHub Models: https://github.blog/changelog/2026-07-30-github-models-is-now-retired/
- OpenCode: https://opencode.ai/docs/zen/, https://opencode.ai/docs/go/
- TypeSafe: https://docs.typesafe.ai/api.md
- GitHub Copilot: https://docs.github.com/en/copilot/concepts/rate-limits
- Ollama: https://docs.ollama.com/faq.md, https://docs.ollama.com/api/balance.md

## Decisions

- **One provider table** (`crates/lr-usage/src/providers.rs`): family id
  (= LocalRouter provider type), label, hosts + path rule, account kind,
  header scheme, and whether a documented usage endpoint exists. `classify`,
  `account_for_provider_type`, `provider_label`, `reports_limits` and the
  polled-types list all derive from it.
- **Quota ids name their period**: `<metric>[_<period>]` (`requests_day`,
  `tokens_minute`, `requests_second`). Header schemes rename each family's
  quotas to what its docs say they count (`HeaderScheme`). OpenAI's and
  Anthropic's ids are unchanged (`requests`, `tokens`, …): OpenAI's headers do
  not say the period, and keeping Anthropic's avoids reshuffling existing
  cards.
- **Quotas refill in the view**: once a quota's reset has passed, or a whole
  period has passed since it was read without a reset time, it shows as full
  rather than the stale remaining count.
- **Superseded quotas are pruned**: a quota not reported for a day while the
  account's other quotas were is dropped (covers ids renamed by this change,
  e.g. Groq's old `requests`).
- **Counts are sanitized**: non-finite or negative limit/remaining values are
  dropped before they are stored.
- **Polling**: new `ModelProvider::fetch_usage()` (default: none, no request).
  OpenRouter (`/key`), LLM Gateway (`/v1/key`), Vercel AI Gateway
  (`/v1/credits`). Requests go through the provider's own client with the
  `UsagePoll` extension; the gateways ask only when their base URL is the
  provider's own host, so a key is never sent elsewhere. The poller polls only
  `polled_api_provider_types()` instances, with the existing schedule,
  exclusions and back-off.
- **Not polled**: DeepInfra `/payment/checklist` (units undocumented), Together
  `/v1/billing/usage` (beta, per-org gated), Zhipu quota endpoint
  (undocumented), admin-key APIs (xAI, Mistral, OpenAI, Anthropic).
- **Config v32**: existing LLM Gateway / Vercel AI Gateway instances are added
  to `poll_excluded_providers`, so upgrading makes no new background requests
  (same rule as v31). Providers added later are polled while "Ask connected
  providers" is on.
- **Settings → Usage** lists a switch for instances of
  `get_usage_polled_provider_types` (new command) instead of a hardcoded list.
- **Dashboard**: quota rows use short labels (`Req/day`, `Tokens/min`,
  `Free req/day`), a meter and a tooltip spelling the limit out; up to four per
  card, requests before tokens. Providers that report nothing
  (`reports_limits == false`) say so instead of "No limits reported yet", and
  show their spend via LocalRouter.
- **Attribution**: the generic OpenAI-compatible provider bills to the API its
  base URL points at (`OpenAICompatibleProvider::usage_account`).

## Tests

- lr-usage: header parsing per scheme with the documented examples (Groq,
  Cerebras, Together, DigitalOcean, LLM Gateway structured fields incl.
  garbage, Anthropic priority/fast); body parsers (OpenRouter `/key` documented
  example and variants, LLM Gateway DevPass and pay-as-you-go, Vercel);
  provider table (unique families, host + path matching, base URLs); classify
  for every new host and provider type; view (period labels, ordering,
  refill rules, `reports_limits`); tracker (superseded-quota pruning,
  sanitized counts); `observe_response` end to end for Groq.
- lr-providers: `fetch_usage_json` against a local server (body, unrecognized,
  401, 429 + Retry-After); generic provider attribution by base URL; gateways
  never poll a foreign host or without a key; every remote factory type has a
  usage account and every polled type is a real provider type.
- lr-config: v32 migration.
- TS unit tests: short labels, tooltip lines, spend-only accounts.

## Verification

- `cargo clippy --workspace --all-targets -- -D warnings` (rustup stable
  1.99), `cargo fmt --all -- --check`,
  `cargo test -p lr-usage -p lr-providers -p lr-proxy -p lr-config`,
  `cargo test -p localrouter --lib -- usage`: pass.
- `npx tsc --noEmit` (app and website), `tests/e2e/unit/usage-limits.spec.ts`:
  pass.
- Screenshots of the demo's Usage limits section (light/dark, 2x):
  `/tmp/cm-providers-shots/`.

## Final steps

- Plan review: every documented header scheme is parsed; every documented
  same-key endpoint is either polled or listed with the reason it is not;
  settings list derived from the backend; demo accounts added (OpenRouter,
  Groq, Gemini).
- Test coverage review: added the tracker sanitize test, the factory
  consistency test and the HTTP-level poll test after the first pass.
- Bug hunt: found and fixed NaN / negative counts reaching the view, quota ids
  renamed by this change lingering forever, upgrade starting new background
  requests for gateways added since v31 (config v32), and a stale per-second
  reading showing as exhausted.
