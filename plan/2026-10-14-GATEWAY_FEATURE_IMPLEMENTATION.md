# Gateway feature implementation

- [x] Read the approved roadmap, current routes/provider contracts, and authoritative Decisions documentation.
- [x] P0 Decisions API: typed contract, validation, native gateway support, legacy translation, refusal/image handling, route/OpenAPI, monitoring and tests.
- [x] P1 Advanced gateway options: typed configuration/request options, policy intersection, upstream attribution/costs and UI.
- [x] P1 Routing explanation and dry-run preview.
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

- [x] Implement typed gateway preferences and validation in lr-providers, with per-instance defaults saved through the existing provider configuration API.
- [x] Merge request extensions.localrouter.gateway with configured policy: intersect allowlists (including narrower endpoint variants), union exclusions, choose stricter price ceilings/privacy requirements, and never enable fallbacks/unsupported parameters against a configured restriction. Reject empty eligibility and unsupported strict requirements before IO.
- [x] Adapt each gateway's verified contract: OpenRouter provider object and region-specific base URL; LLM Gateway routing factor plus provider prefix/X-No-Fallback; Vercel providerOptions.gateway. Unsupported cross-gateway fields return explicit errors instead of silently disappearing. Apply options on streaming and non-streaming paths and native decisions where supported.
- [x] Preserve returned inference-provider identity, service tier, routing attempts and authoritative costs in typed attribution carried through response/chunk extensions. Keep the LocalRouter provider-instance identity separate. Unknown attribution/pricing remains labelled unknown/estimated.
- [x] Add structured provider settings controls, TypeScript preference/attribution types, demo configuration/metadata, API examples, meaningful merge/adapter/error/streaming tests, and monitor display.
- [x] Plan Review, Test Coverage Review, Bug Hunt, final stable CI checks and frontend/demo checks, then commit and push.

Provider defaults apply to that instance. Request constraints apply throughout the request's candidate chain and cannot be silently dropped on a direct-provider fallback. Later per-client privacy profiles extend these requirements to explicit client policy. Regional provider selection is a configuration of the upstream service, not a certification of residency by LocalRouter. Exact gateway field availability and plan restrictions must follow primary docs. UI should explain upstream inference-provider restrictions separately from LocalRouter's existing provider/model permissions.

Verified sources: https://openrouter.ai/docs/guides/routing/provider-selection; https://docs.llmgateway.io/features/routing; https://vercel.com/docs/ai-gateway/models-and-providers/provider-options; https://vercel.com/docs/ai-gateway/sdks-and-apis/openai-chat-completions. Reuse CompletionRequest/CompletionResponse/CompletionChunk extensions and existing provider-config commands to avoid expanding every provider's wire struct or adding parallel configuration APIs.

### P1 implementation review

Typed per-instance defaults and request overrides now map separately to OpenRouter, LLM Gateway and Vercel. Saved allowlists, exclusions, price ceilings, privacy flags and disabled fallbacks cannot be relaxed by a request. Provider settings expose structured controls; OpenRouter's configurable HTTPS base URL supports regional hostnames. Unsupported strict constraints fail before IO, and automatic routing skips incompatible candidates. Chat, legacy Completions, Responses, native Decisions and translated decisions preserve the request namespace. Classifier requests retain the same privacy controls. The OpenRouter Alpha Decisions contract has no verified routing-control support, so constrained native Alpha requests fail closed; its settings UI states this limitation. Service-tier controls are enabled only on OpenRouter, per its current documented values.

Attribution separates the local gateway instance from the reported inference provider, region, tier and sanitized routing attempts. Credential IDs, key fingerprints/labels and raw errors are omitted from the typed attribution. SSE metadata is retained for the same upstream request, without inheriting another call's identity or cost. Valid reported costs take precedence over catalog estimates; tool-loop and server stream aggregation require every upstream call's cost to be known. A final call's cost is never presented as a complete multi-call turn. Catalog component splits remain estimates even when the gateway reports an authoritative total. The monitor explicitly labels unknown identity and gateway cost.

Plan review and bug hunt addressed policy intersection across endpoint variants, Vercel fallback pinning, unsupported constraints, raw vendor fallback fields, missing Responses/Completions propagation, classifier/emulation propagation, shared native-adapter configuration, metadata erasure across chunks, multi-call cost undercounting and custom-header precedence over fallback restrictions. Coverage includes policy/transport/SSE/metadata/cost/isolation fixtures, malformed captured responses and live OpenAPI schema checks. Validation is in progress before commit.

Request example (OpenRouter, within the client's existing model permissions):

```json
{
  "model": "my-openrouter/openai/gpt-5",
  "messages": [{"role": "user", "content": "Explain this change."}],
  "extensions": {
    "localrouter.gateway": {
      "only": ["openai"],
      "zdr": true,
      "allow_fallbacks": false,
      "max_price": {"prompt": 5, "completion": 20},
      "service_tier": "default"
    }
  }
}
```

Prompt/completion ceilings use USD per million tokens; request/image ceilings use USD per request/image. Provider-level configuration stores the same object under gateway_preferences through the existing provider configuration command. Vercel maps ZDR/no-training to its documented providerOptions.gateway fields; LLM Gateway uses model provider pinning, routing and X-No-Fallback and rejects unsupported strict fields. OpenRouter service tiers: https://openrouter.ai/docs/guides/features/service-tiers. Additional Vercel privacy mappings: https://vercel.com/docs/ai-gateway/security-and-compliance/zdr and https://vercel.com/docs/ai-gateway/security-and-compliance/disallow-prompt-training.

Next P1 slice has been saved before code in plan/2026-10-14-ROUTING_EXPLANATION_PREVIEW.md. All remaining master roadmap items remain pending until implemented and validated.

Final P1 review additionally fixed the pre-existing decision-auto permission omission before gateway discovery, standard service-tier aliases, OpenRouter's unified reasoning-effort mapping with a saved compute ceiling, raw vendor-control loss at typed HTTP ingress, native routing metadata that could disclose credential details, and final metadata delivery on every LLM surface. Gateway cost_scope distinguishes an upstream call from a complete request; initial Vercel resolution stays separate from final inference identity. Tool-loop billing excludes already charged calls from an earlier pause/resume request. Tests cover these cases plus streaming with include_usage on/off and rejection of raw controls before inference.

Validation: rustup stable 1.99.0, actual compiler PATH/RUSTC pinned, workspace Clippy with warnings denied, fmt check and workspace tests passed (4,173 passed, 69 ignored, zero failures). Live HTTP fixtures verify gateway options/attribution on Chat, legacy Completions, Responses and Decisions, and published OpenAPI definitions. App/demo type checks, all 62 frontend regressions and both production builds passed. P0 was pushed in 16be1102; P1 preferences/attribution is the next complete slice, with all later roadmap items still pending.

Reasoning-effort source: https://openrouter.ai/docs/guides/best-practices/reasoning-tokens. Standard reasoning_effort maps to the unified reasoning.effort object on OpenRouter; saved effort is a ceiling, and request overrides can lower it. Native Decisions reasoning controls remain explicitly unsupported where no contract is verified. Raw vendor fields such as provider/providerOptions/routing/models and top-level privacy/price controls must use the namespaced LocalRouter preferences, so typed ingress cannot silently discard a restriction.

P1 Plan Review, Test Coverage Review and Bug Hunt are complete. Final CI parity and live schema/API verification passed; the slice is ready for its normal commit and push. Remaining delivery begins with the saved routing-explanation/preview plan.

P1 preferences/attribution committed and pushed as d63e7311. Routing explanations and cached-only preview are now in progress, following the saved plan.

P1 routing explanations and cached-only preview are complete. Shared filters, exact-mode preview, unknown semantic decisions, policy/config fingerprints and classifier overhead now reach normal/streaming responses, native Decisions, structured failures and the monitor. Explicit classifier tests remain opt-in. Review fixed adapter metadata restoration, post-request eligibility drift, dropped Responses/MCP traces, permanent-error status preservation and missing failed stream attempts. Validation: stable 1.99.0 pinned, workspace Clippy/fmt/tests (4,182 passed, 69 ignored), all 23 live HTTP/OpenAPI fixtures, 62 frontend unit regressions and app/demo types/builds passed.

Next implementation plan saved before code: plan/2026-10-14-ADAPTIVE_ROUTING_BOUNDS.md. Remaining roadmap items remain pending, including P1 local replay.
