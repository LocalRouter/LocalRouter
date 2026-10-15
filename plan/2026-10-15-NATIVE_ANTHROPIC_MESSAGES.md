# Native Anthropic Messages ingress (P2)

- [x] Review current Anthropic/gateway API documentation, existing provider transport, proxy wire parsing and shared policy pipeline. The proxy currently observes native Messages; it does not provide native gateway ingress.
- [x] Save this plan before code and finish the provider/SDK contract matrix before enabling each native transport.
- [x] Add validated native Messages request/response/error contracts and `/v1/messages` (with token counting only for verified transports). Accept local client Bearer or x-api-key authentication without forwarding the caller's credential upstream. Preserve API version/beta semantics through a bounded header allowlist; do not forward SDK fingerprint/telemetry headers.
- [x] Route through current client/model permissions, approval, scanner, safety, limits, gateway privacy constraints, adaptive deadlines, usage/account accounting and monitoring. Reuse shared policy helpers and dispatch budgets; do not introduce a second authorization stack.
- [x] Add native provider transport support for direct Anthropic and verified gateway contracts. Preserve complete native content blocks, tool results, cache-control markers/TTL, thinking/signatures and ordered SSE events. Preserve errors after streaming begins and never restart the request after the first event.
- [x] Translate the supported text/image/client-tool subset for compatible chat providers, with explicit validation for semantic gaps. Never silently strip protected blocks, native-only options, unsupported safety/tool-search settings or unknown actions. Keep native versus translated support visible.
- [x] Reuse the existing MCP orchestrator for losslessly translatable requests. Reject unsupported combinations of opaque native state and automatic MCP execution before dispatch; client-owned native tool loops must retain their full native history. No unsupported native tool action may bypass MCP/server-tool permissions.
- [x] Require a stable native provider/model for provider-bound state: explicit/unambiguous native selection or an established allowed session pin. Do not pay a classifier or move opaque state to another provider during fallback. Keep protected blocks byte-for-byte intact through routing and any approval edits.
- [x] Add native count_tokens where supported, enforcing the same current client/privacy/model checks. Return explicit unsupported capability errors elsewhere; do not present a character estimate as an authoritative token count.
- [x] Preserve native usage, cache read/write/TTL breakdown, reasoning and provenance. Audit 1-hour cache-write pricing before estimating it; unknown rates remain unknown. Keep raw native envelopes outside exact response caching until their identity/state contract is safe.
- [x] Integrate OpenAPI, capability/API-path reporting, client setup instructions and a Try It Out native Messages surface, with synchronized Tauri types/mocks where commands change. Reuse existing bundled icons.
- [x] Cover native/translated requests, API-key auth and errors, complete client tool loops, cache markers, opaque state, streaming event order/signatures/errors/cancellation, token counts, client/model isolation and denied capabilities with loopback fixtures and frontend regressions. No paid external inference in tests.
- [x] Plan Review: compare implementation against every behavior and finish omissions.
- [x] Test Coverage Review: cover new edge/error/ownership/state paths.
- [x] Bug Hunt: review silent field loss, unsafe header forwarding, provider binding, side effects, native identifiers and usage/cost aggregation.
- [x] Update stable and pin compiler PATH/RUSTC; run exact workspace Clippy/fmt/tests, frontend types/unit/build checks and live OpenAPI/native SDK fixtures.
- [x] Commit and push validated changes and automatic catalog updates, preserving unrelated work.

Use a native request representation alongside the shared policy projection so the native transport does not round-trip signed blocks through OpenAI text messages. A projection used for scanning is not permission to rewrite, drop or relocate native content. Any supported approval edit must change the actual dispatched request; protected blocks cannot be silently modified. Provider credentials remain controlled by the configured provider instance.

Direct native pass-through and API-format compatibility are distinct capabilities. Gateway implementations can translate requests or alter supported histories; enable only verified behavior and keep strict gateway preferences intact or fail explicitly before IO. Do not advertise complete parity for every provider, server tool or beta. File/container references and provider-side actions require ownership/capability enforcement; do not bypass the subsequent Files/Batches and server-tool work with an unrestricted native raw path.

Current primary sources:
- https://platform.claude.com/docs/en/api/messages/create
- https://platform.claude.com/docs/en/build-with-claude/streaming
- https://platform.claude.com/docs/en/api/messages/count_tokens
- https://docs.llmgateway.io/features/anthropic-endpoint
- https://docs.llmgateway.io/v1_messages
- https://vercel.com/docs/ai-gateway/sdks-and-apis/anthropic-messages-api
- https://openrouter.ai/docs/api/api-reference/anthropic-messages/create-messages?explorer=true

The first native release must prove direct signed-history preservation and ordinary tool loops, not claim universal parity. SDK headers/auth and token-count contracts must be checked against actual fixtures. Existing HTTPS inspection/reverse-proxy functionality remains useful; this feature gives ordinary gateway clients a direct native endpoint.

## Verified transport matrix and implementation progress

Direct Anthropic API-key instances: verified Messages create and count_tokens contracts, semantic version/beta headers and native content preservation. The first transport accepts raw envelopes and preserves upstream HTTP status/SSE for the router to handle. OAuth Claude, OpenRouter, Vercel and LLM Gateway native transport remain disabled until their credential, privacy-option and state contracts are integrated and tested. Chat compatibility is not treated as native transport support.

The request contract and direct transport are foundations only; HTTP ingress, router policy/usage integration, streaming adapters and UI are still pending. No native endpoint is advertised or exposed by this foundation. Native server tools, foreign file/container references and unknown action fields fail validation. Token counting rejects generation-only options rather than silently discarding them.

## Implementation and validation progress

Native create/count contracts, bounded payload-preserving direct Anthropic transport, semantic header allowlisting, local API-key normalization, native SDK error envelopes and OpenAPI schemas are implemented. A non-deserializable provider-request sidecar carries original native history through the existing shared policy and router dispatch path. Native history cannot be rewritten through its scanning projection; supported sampling edits change the dispatched native request. Image and tool capabilities remain visible to candidate filters.

Other providers use a validated Chat translation subset, including ordered client tool-use/result history. Signed thinking, cache markers, tool errors and unsupported block ordering fail explicitly when translation would lose semantics. Plain MCP requests reuse the existing authorized orchestrator, retaining the actual Messages ingress label; protected native/MCP combinations are rejected before dispatch. Strict native auto continuations require an established allowed direct-provider session pin. Raw native responses remain outside exact response caching.

The enabled raw native transport is direct Anthropic API-key authentication. Existing OpenRouter, LLM Gateway and Vercel providers remain available through the verified Chat translation subset. Their documented Messages endpoints do not by themselves prove the signed-state/inference-provider-pinning and privacy-control contracts needed to enable unrestricted raw transport. Such unsupported native semantics are rejected explicitly. This is an intentional capability boundary; the feature does not claim universal native parity.

Native SSE preserves original event data and signature values, validates framing/usage/termination, bounds frames and retained usage state, and does not restart after output. Translated SSE handles deferred client tool names without leaving content-block index gaps. Local stream finalization survives disconnects. Non-streaming native responses are bounded to 32 MiB; count/error bodies to 64 KiB. One-hour native cache creation is accounted at the documented 2x base-input rate; missing catalog cache rates stay unknown instead of receiving a universal read multiplier.

Try It Out has an editable Messages panel, event inspection, Stop and explicit native input-token counting. Client setup explains SDK root URLs and local client secrets. Tauri capability types, demo handlers/OpenAPI and repository documentation are synchronized. Six authenticated loopback HTTP fixtures and 24 provider cases passed before the final edge-case additions. Both production builds and 72 frontend regressions passed. Remaining validation includes strict session continuation, the MCP subset, OpenAPI, deferred stream blocks, body limits, full workspace CI and final review.

## Final review and validation

Plan Review, Test Coverage Review and Bug Hunt completed. Review fixed deferred SSE tool block indexing, bounded native response/error/count reads, pending-usage retention, native SDK error formatting, direct Try It Out token counting and unambiguous bare model resolution. Ten normal-authentication loopback fixtures pass, including native and translated client tool loops, current strict session pins, denied capabilities, native errors/retry hints, the MCP subset, token counting and live OpenAPI. The native contract/framing selection passes 26 provider cases, and two server codec regressions pass.

Final CI: updated rustup stable 1.99.0, actual compiler PATH/RUSTC pinned to rustup, workspace Clippy with warnings denied, formatting check and all 109 workspace suites pass (4,330 tests, 69 ignored). Frontend types, 72 unit regressions and app/website production builds pass. Completed changes are committed and pushed with this plan; unrelated work is preserved.
