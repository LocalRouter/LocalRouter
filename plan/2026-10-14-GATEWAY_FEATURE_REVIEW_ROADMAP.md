# Gateway feature review and proposed LocalRouter roadmap

- [x] Review current official documentation and recent announcements for OpenRouter, LLM Gateway, LiteLLM, Portkey, Cloudflare AI Gateway, Helicone, and Vercel AI Gateway.
- [x] Compare those features with current LocalRouter code and recent plans.
- [x] Separate existing capabilities, upstream compatibility gaps, and proposed native additions.
- [x] Prioritize a phased roadmap with implementation locations and acceptance criteria.
- [ ] For each selected feature, create its own implementation plan before changing application code.
- [ ] Plan review after implementation: compare every promised behavior and edge case with the implementation and complete missing work.
- [ ] Test coverage review after implementation: add meaningful coverage for new paths, errors, security boundaries, and migrations.
- [ ] Bug hunt after implementation: inspect streaming, retries, concurrency, policy precedence, cancellation, and cost accounting.
- [ ] Commit and push each completed, validated implementation and any automatic catalog update, preserving unrelated changes.

Research date: **2026-10-14**. Implementation was approved on 2026-10-14, with local replay promoted to P1; execution is tracked in plan/2026-10-14-GATEWAY_FEATURE_IMPLEMENTATION.md. Current documentation establishes feature availability; only dated announcements below establish recency. Enterprise restrictions and availability may change. Estimates are relative engineering size, not delivery commitments: S = contained change, M = cross-layer feature, L = new API/lifecycle or substantial routing work.

## Recommendation

First address Decisions API compatibility and preserve advanced gateway options. Then make routing more explainable, adaptive, and cache-aware. Add opt-in exact response caching, native Anthropic compatibility, local evaluation, and discounted batches in subsequent releases. Keep LocalRouter's differentiators: desktop/local operation, subscription credentials, local decision engines, MCP mediation, and no telemetry.

The biggest opportunity is making existing routing and provider functionality work better together. Many competitor checklist features are already present here; copying those would add little.

## What the gateways offer

| Gateway | Verified features relevant to LocalRouter | What to take from it |
| --- | --- | --- |
| OpenRouter | Provider ordering/filtering, price/latency/throughput sorting, parameter requirements, privacy filters, server tools and plugins; recent batches, regional endpoints, key security overview, router benchmarks | Advanced upstream options, endpoint policy enforcement, discounted jobs, and evidence-driven routing |
| LLM Gateway | Adaptive routing and sticky sessions, versioned dynamic routes (Enterprise), separate provider prompt caching and gateway response caching; Anthropic compatibility, search, rerank, documents, video/realtime, guardrails | Decisions migration, session affinity, reusable route versions, exact cache, and incremental endpoint compatibility |
| LiteLLM | Deployment load balancing, cooldowns, retries/timeouts, multiple routing strategies, budget/tag routing, traffic mirroring and gateway management | Measured routing, explicit retry/time budgets, and optional controlled experiments |
| Portkey (currently branded Prisma AIRS AI Gateway in its docs) | Conditional routing, exact and semantic caches, gateway controls | Start with exact caching and simple rules; semantic caching is a later experiment (currently select Enterprise plans) |
| Cloudflare AI Gateway | Named versioned routes with conditional and percentage branches, model nodes, gateway controls | Versioned route configuration and gradual experiments without requiring a visual graph editor |
| Helicone | Unified gateway plus request observability; experiment datasets, prompt variants, evaluators | User-triggered local replay and comparison, with cost and measured quality |
| Vercel AI Gateway | Provider filters/order, cost/TTFT/TPS sorting, automatic prompt-cache configuration, virtual-model options | Typed provider options and stable user-facing model aliases |

### Recent changes that matter

- **Oct 12, 2026 — LLM Gateway Decisions API:** adds `/v1/decisions`; legacy `/v1/systemone` continues for TypeSafe models, receives no new decision models, and has a deprecation date to be announced. This is a compatibility issue, not an already-announced shutdown. [Announcement](https://llmgateway.io/changelog/openai-compatible-decisions-api), [migration/schema](https://docs.llmgateway.io/features/decisions), [OpenAI primary schema](https://developers.openai.com/api/docs/guides/decisions).
- **Sep 25 — LLM Gateway smart routing:** configurable classifier/model pool, with a session verdict reused across turns. LocalRouter already offers configurable decision-model routing; session reuse is the useful incremental idea. [Announcement](https://llmgateway.io/changelog/smart-routing).
- **Sep 28 — LLM Gateway standalone search:** ranked retrieved results rather than a generated answer. [Changelog](https://llmgateway.io/changelog).
- **Sep 22 — OpenRouter batches:** asynchronous discounted inference; its inline request/results contract differs from the OpenAI Files/JSONL batch shape in LocalRouter's older plan. [Announcement](https://openrouter.ai/blog/announcements/batch-api/).
- **Sep 28 — OpenRouter Security Center:** key inventory, idle/expiry/spend-limit review, bulk actions and network restrictions. [Announcement](https://openrouter.ai/blog/announcements/security-center/).
- **Sep 9 — OpenRouter regional routing; Sep 8 — hosted shell/files; Oct 2 — router benchmarks; Oct 7 — ElevenLabs audio:** useful signals for privacy configuration, tool compatibility, evaluation and modality coverage. Regional routing currently requires Business or Enterprise. [Regional routing](https://openrouter.ai/blog/announcements/us-in-region-routing/), [shell](https://openrouter.ai/blog/announcements/shell-tool/), [benchmarks](https://openrouter.ai/blog/announcements/model-router-benchmarks/), [dated announcement index](https://openrouter.ai/blog/).

## LocalRouter baseline: extend these, do not rebuild them

Evidence comes from current code rather than treating an old plan as shipped functionality:

- `crates/lr-server/src/lib.rs` registers chat/completions, Responses, embeddings, moderation, image generation/editing, audio, System One, model/pricing/generation and MCP endpoints. No registered `/v1/messages`, `/v1/decisions`, Files/Batches, rerank, search, video or realtime API was found.
- `crates/lr-router/src/lib.rs`, `free_tier.rs`, `rate_limit.rs`, and `decision_routing.rs` already implement ordered fallback, rate/cost backoff, capability checks, model permissions, free-model policies and configurable decision routing. Provider health/cooldown exists in `crates/lr-providers/src/health_cache.rs`. A new proposal should not claim health-based fallback is absent.
- `crates/lr-config/src/routing_policy.rs` and `src/components/strategy/RoutingPolicyEditor.tsx` already support policy choices and explicit client mode. Proposed deterministic rules and versions extend that mechanism.
- `crates/lr-providers/src/features/` already contains prompt caching, structured outputs, JSON/logprob and reasoning adapters. `lr-json-repair`, `lr-compression`, `lr-guardrails`, `lr-secret-scanner` and `lr-mcp-via-llm` provide related functionality.
- `lr-monitor`, `lr-monitoring`, and `src/views/monitor/` already show request details, routing attempts, cost, durations and cached-input tokens. Responses session storage and trace IDs exist. These are different from route affinity or a repeatable evaluation runner.
- Both OpenRouter and LLM Gateway are already provider integrations (`openrouter.rs`, `factory.rs`). Adding them as providers is unnecessary. The OpenRouter request struct currently enumerates fields and has no `provider` or `plugins` field; its response struct does not capture the resolved upstream provider. Function-tool structs require `function`, limiting non-function server-tool compatibility.
- Model discovery/manual models, visibility, capability and price overrides already shipped per the Oct 11 model-discovery work. Reuse those capabilities.
- Existing `plan/2026-03-15-API-FILES-BATCHES.md` and `plan/2026-03-15-API-REALTIME.md` are proposals; route registration does not show them as implemented. Reconcile rather than create parallel designs.

## Phase 0: maintain upstream compatibility

### 1. Decisions API alongside System One — P0, M/L

Add client-facing `/v1/decisions`, typed provider request/response support, and explicit endpoint capability detection. Preserve `/v1/systemone` for existing integrations and embedded engines. Translate between contracts only where equivalent: predicate/noul, named ordered questions versus maps, choice/score distributions, refusals, structured state serialization, and text/image eligibility. Do not invent calibrated probabilities for translated chat models.

Use the native Decisions interface for LLM Gateway's new models and, where verified, other decision providers. Reuse internal decision routing and the existing local engines; do not remove them just because one gateway changes wire formats. Surface native/translated status in discovery and monitoring.

**Locations:** `lr-providers/src/systemone/` and provider traits; `lr-router/src/systemone.rs` and `decision_routing.rs`; `lr-server/src/routes/` and OpenAPI; types/UI/demo commands where applicable.

**Done when:** both APIs work for supported models; new Decisions-only models are not sent to System One; mapping round trips preserve meaning; image rejection/refusal/unknown fields have clear behavior; existing policies/configs continue working. First obtain upstream fixtures and verify exact score semantics rather than implementing a superficial URL alias.

### 2. Advanced gateway options and accurate upstream attribution — P1, M

Create typed per-provider configuration plus a namespaced request override for supported options. Start with OpenRouter provider order/allowlist, fallback permission, required-parameter support, price ceilings, ZDR/data-use requirements, region/base URL and reasoning/service tier where documented. Extend LLM Gateway and Vercel through their own adapters, not by assuming OpenRouter's schema applies to all gateways.

Apply the intersection of client permissions and configured policy; request overrides may narrow but never relax privacy/budget constraints. Separate LocalRouter's chosen provider instance from the gateway's resolved inference endpoint. Show upstream identity and authoritative costs when returned; distinguish estimated catalog costs and unknown identity. Handle streaming final metadata as well as normal responses. Expose configured constraints even if the gateway does not disclose its final provider.

**Locations:** provider request structs/adapters, `lr-config`, `lr-server` request conversion, `lr-monitor`, provider/client settings and demo types.

**Done when:** settings survive serialization and are sent correctly on streaming/non-streaming calls; invalid/unsupported options are explicit; nested fallback cannot escape constraints; privacy filters fail closed. A regional hostname is configuration, not a LocalRouter certification of residency.

## Phase 1: improve routing for daily use

### 3. Routing explanation and preview — P1, M

Extend existing attempt details into a consistent explanation: eligible/excluded candidates, permission/capability/privacy reasons, priority/score, cooldown/quota, classifier overhead, effective config version, and final local/upstream identities. Provide a dry-run preview that does not call classifiers or upstreams; mark unknown decisions and data rather than pretending they were executed. Reuse the existing preview where possible.

**Done when:** the UI can explain success and no-route failures; preview and execution share filtering code; classifier time/cost is separated from inference; all LLM surfaces retain policy metadata. This is the foundation for diagnosing the following features.

### 4. Adaptive ordering with bounded retries — P1, L

Within a user's explicitly acceptable model pool, offer existing priority order plus lowest estimated cost, lowest observed TTFT, highest observed output throughput, and least in-flight load. Use passive local observations, bounded rolling windows/minimum samples, and deterministic cold-start defaults. Do not assume different models have equivalent quality.

Extend current cooldowns with per-provider/model recovery state where useful, `Retry-After` handling, capped jitter/backoff, max attempts, per-attempt deadline and total request deadline. Avoid multiplying LocalRouter retries by an upstream gateway's retry chain. Preserve free-only, budget and permission rules before scoring. Unknown pricing must not become “cheapest.”

**Done when:** controlled latency/error fixtures demonstrate sensible ordering, recovery and cold starts; cancellation releases load slots; a stream never restarts after visible output/tool execution; actual attempt costs are recorded. Avoid speculative duplicate requests initially: they increase cost and may repeat actions.

### 5. Session affinity and prompt-cache optimization — P1, M

Use an explicit client-scoped opaque session ID (or documented Responses continuity) to prefer the same eligible provider/model and credential across turns. Allow decision verdict reuse as an opt-in policy; invalidate on relevant policy/capability/permission changes, explicit model/mode changes, TTL or user reset. Permit deliberate reclassification as a conversation changes.

Respect provider-bound reasoning signatures and continuation state: some sessions cannot transparently fail over and need a clear reset-required error. A pin never overrides local-only, revoked credentials, quotas or model access. Extend existing provider prompt-cache controls/metrics and report read/write tokens and realized discounts. Keep this separate from full response caching.

**Done when:** sessions do not collide across clients; restart/expiry behavior is documented; supported sessions retain cache affinity; incompatible continuation cannot silently migrate; cache statistics distinguish provider prompt reuse from gateway replay.

### 8. Local replay and model/policy evaluation — P1, M

Add “open this request in Try It Out,” editable replay and side-by-side comparison against user-selected allowed models. Use only payloads the user chose to retain/import. Persist local experiment datasets/results and policy snapshots; report cost, latency, validity and user-defined task checks. A judge model is optional and explicitly user-triggered, with its cost shown separately.

**Done when:** no request is automatically mirrored to another provider; tools and MCP actions are disabled/stubbed by default for replay; redacted logs cannot be reconstructed; quality results have sample size and reproducible policy version. Begin with manual comparison before traffic splits or a prompt-management platform.

## Phase 2: add local cost and compatibility features

### 6. Opt-in exact response caching — P2, M

Start with bounded in-memory TTL caching for non-streaming pure-text chat/classification. Partition by client/credential identity, effective policy version, resolved provider/model and all output-affecting inputs (full messages, tools, format, reasoning, sampling and provider options). Run authentication, permissions, request validation and current safety/quota policy before serving a hit; rerun required response checks. A hash is a key, not encryption for stored payloads.

Default off. Add bypass, invalidation, clear controls, hit/miss and realized-cost metrics. Exclude tool-executing/MCP loops, changing web results, provider-bound state and incomplete/error/cancelled responses. No persistent payload cache in the first version. Streaming replay follows only after event IDs, cancellation, terminal usage and billing semantics are defined.

**Done when:** cross-client/config isolation tests pass; disabled models cannot be resurrected by cached data; replay shows zero upstream spend while distinguishing served tokens from billed tokens. Semantic caching remains experimental: similar wording is not proof that answers are interchangeable.

### 7. Native Anthropic Messages ingress — P2, L

Expose `/v1/messages` (and token counting if required by target clients) so compatible coding clients can use the local API directly. Preserve native content blocks, tool results, thinking/signatures and streaming event sequence on Anthropic routes; translate supported subsets elsewhere and reject unsupported semantics explicitly. Reuse LocalRouter permissions, guardrails, secrets, MCP, budgets and routing instead of making a second policy stack.

**Done when:** recorded client-contract fixtures cover tool loops, cache markers, errors, thinking and cancellation. Native safeguards/tool search need explicit provider capability checks; do not strip them silently or advertise full parity. Review existing interception/client setup first so this adds direct API support without duplicating proxy workflows.

### 9. Native discounted batch jobs — P2, L

Revise the existing Files/Batches plan around current provider contracts. Add resumable local jobs with per-client ownership, validation, cancellation, output/error retrieval and retention. Begin with one native batch provider plus OpenRouter's distinct inline adapter; verify contemporary model/endpoint support. Snapshot policy at submission, enforce permissions and budget before dispatch, and distinguish submitted/unknown outcome from safe-to-retry work.

**Done when:** restart cannot duplicate an already submitted billed job; partial errors/cancellation are handled; remote files/results are cleaned up according to explicit retention; paid discount comes from native pricing. Locally queued normal requests can be supported separately but must never be described as discounted inference.

## Phase 3: selective additions after the foundations

| Addition | Priority/size | Proposed scope and evidence to justify it |
| --- | --- | --- |
| Versioned named routes and simple deterministic rules | P3, M/L | Extend existing policies with stable aliases, drafts/publish/rollback, bounded conditions on trusted client metadata, capabilities, context size and budget; optional percentage splits with client/session consistency. Start with a list editor. Justify via repeated requests for reusable strategies and safe rollout. |
| Search/web fetch and server tools | P3, M/L | Preserve supported upstream tools and citations first; expose user-configured search via existing MCP mediation for other models. Typed non-function tool variants are required. Respect local-only policy and charge/monitor tool costs. Treat documents/web text as untrusted content; do not add an autonomous remote fetch path. |
| PDF/document inputs and OCR | P3, M/L | Native provider pass-through plus optional local extraction. Preserve page provenance, input limits and model capability checks. OCR only where user demand warrants it. Leverage MCP resources instead of creating a full hosted knowledge-base product. |
| Rerank endpoint | P3, M | Cohere/Jina-compatible routing and optionally existing local decision/relevance engines, with explicit semantic/capability differences. Worth doing when external RAG clients need it; internal memory reranking alone does not imply a public API. |
| Privacy profiles and client-key hygiene | P2/P3, M | Reuse budgets/last-use/secret scanner; add local-only/no-retention route profiles, expiry/idle review and one-click revocation, plus deliberate log redaction/retention controls. Audit current monitor storage before claiming missing controls. Provider attestations need provenance and must not be inferred from brand. |
| Video and realtime | Later, L each | Separate long-lived/asynchronous protocols, cancellation, budgets and client isolation. Reconcile existing realtime plan. Add after real client demand; audio/image support alone does not cover these lifecycles. |

## Do not prioritize

- A credit resale marketplace, referral program, payments SDK, hosted agent platform or org billing: these are gateway business models, not LocalRouter's desktop purpose.
- Enterprise SSO/SCIM, cloud SIEM exports and centralized team administration before users need a shared deployment.
- Automatic cloud analytics, mirrored prompts, remote logo/assets or background quality benchmarks. All observations stay local; extra inference/search/evaluation comes from user actions or explicitly configured workflows.
- A hosted/local shell platform solely to match OpenRouter. Reuse existing authorized MCP tools and preserve upstream tool semantics first.
- Fusion/multi-model voting, hedged requests or semantic cache by default: cost and correctness need task-specific evidence.
- New general guardrails, JSON healing, compression, generic provider discovery or another decision router: extend existing subsystems.

## Suggested delivery order and validation

1. Decisions compatibility; typed gateway options and attribution.
2. Routing explanations; measured ordering/deadlines; session/cache affinity.
3. P1 user-triggered local replay/comparisons; then P2 exact cache and native Messages.
4. Discounted batch lifecycle; then choose named-route versions/search/rerank based on actual client needs.

Every implementation needs its own saved plan, migration/default behavior, tests and release criteria. Endpoint changes require utoipa annotations, ToSchema types, schema registration and live OpenAPI verification; Tauri changes require Rust/TS parameters and results, demo mocks and frontend type checks. Preserve current explicit-mode and free-tier semantics.

Mandatory final implementation steps: **Plan Review**, **Test Coverage Review**, **Bug Hunt**, then stable CI parity (`rustup update stable`; pinned stable compiler PATH/RUSTC; workspace clippy with warnings denied, fmt check, workspace tests), relevant frontend checks, and **Commit and push** task changes plus automatic catalog updates. Review particularly cross-client leakage, retries after output, continuation signatures, stale capabilities, unknown costs, partial batch submissions and configuration precedence.

## Other primary references

- [OpenRouter provider routing](https://openrouter.ai/docs/guides/routing/provider-selection), [plugins](https://openrouter.ai/docs/guides/features/plugins/overview), [server tools](https://openrouter.ai/docs/guides/features/server-tools/overview).
- [LLM Gateway routing](https://docs.llmgateway.io/features/routing), [sessions](https://docs.llmgateway.io/features/sessions), [dynamic routes](https://docs.llmgateway.io/features/dynamic-routes), [gateway caching](https://docs.llmgateway.io/features/caching/gateway-caching), [Messages](https://docs.llmgateway.io/features/anthropic-endpoint), [documents](https://docs.llmgateway.io/features/documents), [rerank](https://docs.llmgateway.io/features/rerank).
- [LiteLLM routing](https://docs.litellm.ai/docs/routing).
- [Portkey caching](https://portkey.ai/docs/product/ai-gateway/cache-simple-and-semantic), [conditional routing](https://portkey.ai/docs/product/ai-gateway/conditional-routing).
- [Cloudflare dynamic routing](https://developers.cloudflare.com/ai-gateway/features/dynamic-routing/).
- [Helicone gateway](https://docs.helicone.ai/gateway/overview), [experiments](https://docs.helicone.ai/features/experiments).
- [Vercel provider options](https://vercel.com/docs/ai-gateway/models-and-providers/provider-options).

## Research delivery validation

Only this plan is changed by this task; no gateway/application behavior is implemented. Current-code pointers and dated primary sources were reviewed. Pre-commit validation results are recorded below when complete.

Research delivery checks: updated rustup stable (1.99.0), verified the actual rustup compiler path, and ran the required workspace Clippy (`-D warnings`), formatting check, and workspace tests; all passed. Tests: 4,144 passed, 69 ignored, zero failures. Plan structure, required final review steps, key local file references, and whitespace were checked. No application code or tests were added by this research task.

User approved the full proposed roadmap on 2026-10-14, with local replay promoted to P1. Execution is tracked in plan/2026-10-14-GATEWAY_FEATURE_IMPLEMENTATION.md.
