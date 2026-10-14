# Gateway feature implementation

- [x] Read the approved roadmap, current routes/provider contracts, and authoritative Decisions documentation.
- [x] P0 Decisions API: typed contract, validation, native gateway support, legacy translation, refusal/image handling, route/OpenAPI, monitoring and tests.
- [ ] P1 Advanced gateway options: typed configuration/request options, policy intersection, upstream attribution/costs and UI.
- [ ] P1 Routing explanation and dry-run preview.
- [ ] P1 Adaptive ordering and bounded attempts/deadlines.
- [ ] P1 Session affinity, classifier reuse and prompt-cache accounting.
- [ ] P1 Local replay and model/policy evaluation (promoted by user).
- [ ] P2 Opt-in exact response cache.
- [ ] P2 Native Anthropic Messages ingress.
- [ ] P2 Native discounted Files/Batches lifecycle.
- [ ] P2/P3 Privacy profiles and client-key hygiene.
- [ ] P3 Versioned named routes and deterministic rules.
- [ ] P3 Search/web fetch and upstream server tools.
- [ ] P3 PDF/document inputs and OCR.
- [ ] P3 Rerank API.
- [ ] Later Video jobs and realtime sessions.
- [ ] Plan Review: compare implemented behavior with each requirement and finish omissions.
- [ ] Test Coverage Review: add meaningful edge, error, isolation and migration tests.
- [ ] Bug Hunt: review policy precedence, streaming, cancellation, signatures, billing, persistence and retries.
- [ ] Validate with updated rustup stable, explicitly pinned compiler PATH/RUSTC, workspace Clippy/fmt/tests, frontend types/build and relevant UI checks.
- [ ] Commit and push completed validated changes and automatic catalog updates, preserving unrelated work.

User authorization (2026-10-14): implement all proposed core and later candidate features in priority order; promote local replay to P1. This excludes the roadmap's “Do not prioritize” business/telemetry features. Use incremental complete commits, and retain the remaining checklist until every authorized feature is complete. Repository documentation remains in plan/.

## First implementation slice: Decisions

Create a provider-level Decisions module with typed ordered question/answer arrays, string or user-message input, text/image parts, explicit refusal, usage and metadata. Verify contract against OpenAI and LLM Gateway published schemas. Reject duplicate named questions and typed options, invalid ranges and unsupported streaming before routing.

Use existing System One machinery as the text/local/chat compatibility path, preserving options order and score scale. Do not silently turn images into JSON text for engines that cannot evaluate them. Preserve refusals without fabricated distributions. Add native Decisions execution for LLM Gateway; legacy Jev engines remain on System One. Existing decision-routing classifiers use the gateway's new wire format through an adapter. Add /v1/decisions and /decisions to server routes, shared request authentication/model access/guardrails/secret scanning/rate limits, monitoring, pricing and OpenAPI. Preserve System One response/header compatibility; label translated/chat probabilities by backend rather than claiming calibration.

Acceptance: request/response fixtures round-trip; native gateway requests reach /v1/decisions; legacy System One still works; choices, predicate and score mappings preserve meaning/order; refusal cannot be interpreted as a successful classifier answer; images reach only supported native providers; endpoint validation and client permission/quota checks are enforced; schema exposes all wire variants. No Tauri command changes unless necessary; if changed, synchronize TS and demo mocks.

## Remaining slices

Implement each subsequent checklist item with its own detail section before writing its code, using the approved research roadmap's requirements and acceptance criteria. Preserve existing local-only/privacy defaults, free-tier and explicit-mode rules. Defaults for new cache, affinity, remote search, experiments and adaptive routing remain opt-in. Every meaningful commit receives Plan Review, Test Coverage Review, Bug Hunt and exact stable CI parity before normal push.

Sources: https://developers.openai.com/api/docs/guides/decisions and https://docs.llmgateway.io/features/decisions. Overall rationale and source comparison: plan/2026-10-14-GATEWAY_FEATURE_REVIEW_ROADMAP.md.

## P0 review and checks

Implemented ordered Decisions types, boolean/string identities, optional names, score labels, refusals, image input, native LLM Gateway/public OpenAI transports, legacy/text translation and private internal adapter state. Both routes use the shared decision pipeline; /decisions is protected by authentication. Decision input and question rubrics are scanned, and image URLs/envelopes are excluded from prompt compression. Existing classifier policies use native adapters without schema changes. Discovery marks native image capability; automatic image routing excludes legacy/chat-only and known text-only candidates. Monitor/passive proxy, provider compatibility, Try It Out and demo OpenAPI include Decisions. Native usage details and cache pricing are preserved. No Tauri commands changed.

Plan review identified and addressed the unprotected alias, ordered-array monitor rendering, malformed captured traffic, typed-value collisions, refusal preservation, prompt compression of envelope/image fields, and missing rubric scanning. Coverage includes wire round trips, invalid native responses, native OpenAI refusal/usage, legacy text/image rejection, gateway image/boolean/refusal HTTP behavior, live OpenAPI, passive detection and frontend capture/translation tests. Final P0 validation passed on rustup stable 1.99.0 with explicit compiler PATH/RUSTC: workspace Clippy with warnings denied, fmt check and all workspace tests (4,154 passed, 69 ignored, zero failures). Frontend: 60 unit regressions, app/website type checks and production builds passed. Live HTTP tests verify both aliases, native/legacy paths and generated OpenAPI. P0 is ready for commit and push; all later slices remain pending.

Additional P0 source verification: Vercel https://vercel.com/docs/ai-gateway/sdks-and-apis/openai-decisions and OpenRouter https://openrouter.ai/docs/guides/community/multimodal-decisions establish distinct native transports. Vercel uses /v1/decisions with explicit data-URL/choice-collision restrictions. OpenRouter uses /api/alpha/decisions with System One maps and data image_url parts; the adapter preserves typed identities and answer order locally. Native adapters and fixtures cover these transports alongside LLM Gateway and OpenAI.

## P1 next slice: gateway preferences and attribution

- [ ] Implement typed gateway preferences and validation in lr-providers, with per-instance defaults saved through the existing provider configuration API.
- [ ] Merge request extensions.localrouter.gateway with configured policy: intersect allowlists (including narrower endpoint variants), union exclusions, choose stricter price ceilings/privacy requirements, and never enable fallbacks/unsupported parameters against a configured restriction. Reject empty eligibility and unsupported strict requirements before IO.
- [ ] Adapt each gateway's verified contract: OpenRouter provider object and region-specific base URL; LLM Gateway routing factor plus provider prefix/X-No-Fallback; Vercel providerOptions.gateway. Unsupported cross-gateway fields return explicit errors instead of silently disappearing. Apply options on streaming and non-streaming paths and native decisions where supported.
- [ ] Preserve returned inference-provider identity, service tier, routing attempts and authoritative costs in typed attribution carried through response/chunk extensions. Keep the LocalRouter provider-instance identity separate. Unknown attribution/pricing remains labelled unknown/estimated.
- [ ] Add structured provider settings controls, TypeScript preference/attribution types, demo configuration/metadata, API examples, meaningful merge/adapter/error/streaming tests, and monitor display.
- [ ] Plan Review, Test Coverage Review, Bug Hunt, final stable CI checks and frontend/demo checks, then commit and push.

Provider defaults apply to that instance. Request constraints apply throughout the request's candidate chain and cannot be silently dropped on a direct-provider fallback. Later per-client privacy profiles extend these requirements to explicit client policy. Regional provider selection is a configuration of the upstream service, not a certification of residency by LocalRouter. Exact gateway field availability and plan restrictions must follow primary docs. UI should explain upstream inference-provider restrictions separately from LocalRouter's existing provider/model permissions.

Verified sources: https://openrouter.ai/docs/guides/routing/provider-selection; https://docs.llmgateway.io/features/routing; https://vercel.com/docs/ai-gateway/models-and-providers/provider-options; https://vercel.com/docs/ai-gateway/sdks-and-apis/openai-chat-completions. Reuse CompletionRequest/CompletionResponse/CompletionChunk extensions and existing provider-config commands to avoid expanding every provider's wire struct or adding parallel configuration APIs.
