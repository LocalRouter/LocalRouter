# Search, web fetch and provider-executed tools

- [x] Audit request/tool conversion, native payload contexts, MCP execution, citation/usage preservation, endpoint capability discovery and existing quota/approval paths. Verify current primary provider contracts before enabling adapters.
- [x] Add typed non-function search/fetch tool variants without fabricating function definitions or silently dropping vendor constraints. Keep client function tools and provider-executed tools distinct; never execute an upstream server tool as a local MCP/function call.
- [x] Add default-off per-client authorization for provider-executed tools with clear supported limits. Preserve current model permissions, privacy and spending controls. Reject unsupported count/domain/privacy mappings before IO instead of relying on provider fields that may be ignored.
- [x] Support verified OpenRouter web search/web fetch and LLM Gateway native web search contracts, including mixed client/server tools, tool-choice validation and bounded request options. Preserve normal/streaming citations, provider usage and known/unknown tool costs. Explicitly reject unverified tool types or parameters; do not add a hosted shell platform.
- [x] Add bounded ranked Search API ingress compatible with verified Perplexity/LLM Gateway schemas. Resolve an explicit permitted search provider/model or a deliberately configured default, apply normal authentication/permissions/scans/rate and budget policy, and keep ranked results separate from model-written answers. Do not pretend unknown flat/request pricing is a zero-cost token quote.
- [x] Reuse configured MCP mediation for search/fetch on other models, including current tool permissions and approvals. Provide clear setup/selection UI and provenance; do not add an autonomous local URL-fetch crawler or automatically install services/accounts.
- [x] Update endpoint discovery, OpenAPI annotations/schemas/registration, monitoring and Try It Out. Synchronize any new Tauri Rust commands, TypeScript response/parameter contracts, typed calls and demo handlers. Bundle all assets; do not fetch favicons or remote UI assets.
- [x] Disable exact-cache/session shortcuts and replay/tool re-execution where server-tool semantics make them unsafe. Treat retrieved documents as untrusted tool content, preserve citation provenance, and prevent any newly supported tool from bypassing strict profiles or existing MCP safeguards.
- [x] Add bounded malformed/unsupported wire, mixed-tool, streaming/citation/cost, permissions/privacy, unknown-budget, cancellation and SDK/OpenAPI loopback regressions. No paid live calls.
- [x] Plan Review: compare every promised behavior with the implementation, finish omissions and document unsupported mappings.
- [x] Test Coverage Review: add meaningful boundary/error/isolation coverage for changed paths.
- [x] Bug Hunt: inspect remote/local execution confusion, SSRF, ignored constraints, duplicate calls, stale authorization, citation loss, payload retention, unknown spend and fallback after side effects.
- [x] Update/pin Rust stable; run exact workspace Clippy, fmt and tests, frontend types/unit regressions and both production builds. Use the isolated Cargo target to avoid unrelated repository-wide cleanup.
- [ ] Commit and normal push validated changes and automatic catalog updates, preserving unrelated work. Continue document inputs/OCR, rerank and video/realtime until the approved roadmap is complete.

The user approved all core and later roadmap additions and promoted replay to P1. Named-route versions are completed and pushed as 09c0c2d0; privacy/key hygiene is f51e2b29. This phase extends those policies rather than granting implicit new tool permissions.

Verified primary sources (2026-10-15):

- OpenRouter server tools: https://openrouter.ai/docs/guides/features/server-tools . Provider inference ZDR and regional policy do not establish the retention policy of every tool backend. The current catalog distinguishes provider/OpenRouter/client execution. Tool definitions belong in tools, while max_tool_calls is a top-level loop budget. Arbitrary stop_server_tools_when must not silently override a local cap.
- OpenRouter web search: https://openrouter.ai/docs/guides/features/server-tools/web-search . The old web plugin/online suffix is deprecated. Search engines have different domain/result/use-limit semantics; native execution can ignore parameters, so verify or reject strict constraints.
- OpenRouter web fetch: https://openrouter.ai/docs/guides/features/server-tools/web-fetch . Upstream fetch extracts URL content; LocalRouter need not fetch retrieved URLs itself. Engine and domain/size/use limits need verified mappings.
- LLM Gateway native search: https://docs.llmgateway.io/features/web-search . Basic native web_search tools and URL citations are documented, while several optional limits/filters vary by provider. Do not silently accept a hard requirement that is ignored downstream.
- Ranked search: https://docs.llmgateway.io/features/search and https://docs.llmgateway.io/v1_search ; direct provider contract https://docs.perplexity.ai/api-reference/search-post . This is ranked retrieval rather than generated answers. Validate provider differences (LLM Gateway currently excludes people search; direct Perplexity supports it, with different result bounds). Charges are request-based; unknown pricing/accounting must remain explicit.

Initial implementation choices must follow the code audit, with a concrete contract amendment before adapter code. Prefer a private typed server-tool context similar to existing protected native Messages payloads if that preserves established function-tool consumers more safely than a broad provider Tool refactor. The public API still needs truthful typed variants and OpenAPI, and private context must always be derived/validated at ingress and stripped from ordinary wire metadata. Support only documented execution paths; unsupported protocols/constraints remain explicit. No new plugin installation, paid call, background benchmark or analytics is authorized by inference from this plan.

Concrete contract before adapter code: keep existing provider/client function-tool structs intact and introduce a typed protected server-tool context. Public Chat/Responses tool schemas must describe the accepted union truthfully. HTTP ingress validates/splits native definitions and choices before ordinary function-only conversion; callers cannot inject the private context. A non-deserializable provider context is derived during conversion, checked on every dispatch and never forwarded as an opaque extension. Mixed client functions remain client-owned. Native Messages, legacy Completions, persistent batches and automatic MCP combinations without a verified mapping reject explicitly; configured MCP search remains the mediated alternative for other models.

Add default-off per-client search/fetch authorization under security settings, with optional provider-tool step bounds. OpenRouter has a verified top-level max_tool_calls mapping; LLM Gateway must reject an imposed unsupported global cap rather than ignore it, with deliberate provider-managed settings available. Domain/result/use controls need verified engine mappings; uncertain native/auto combinations reject strict constraints. Strict privacy profiles reject unverified tool chains. Unknown tool price ceilings reject cost-limited/free-only requests before classification or dispatch. Disable nested gateway fallback and local retry after an upstream tool dispatch, and run guardrails sequentially before requests that can execute tools.

Stage the phase as complete reviewable slices if needed: protected native tool contracts/authorization/transports/citations first, then ranked Search API and its UI/accounting/MCP setup. The master phase remains incomplete until both are delivered. Preserve citations/tool-use metadata through streams and replay artifacts, and ensure replay disables re-execution. Exact caching and provider-session shortcuts must not reuse dynamic server-tool output. Finalize unknown-cost handling against verified upstream cost semantics before enabling transports; never convert missing tool charges into a complete zero-cost claim.

## Native provider-tool slice review

Implemented protected typed tool projection, explicit default-off per-client grants,
verified cap intersection, canonical OpenRouter search/fetch and basic LLM Gateway
search. Unsupported mappings fail before tool dispatch. Current client policy is
checked before routing and at the provider boundary. Strict privacy, unknown-cost
ceilings/free-only strategies, automatic MCP and duplicate hops reject unsupported
chains. Local and gateway fallback cannot repeat a dispatched tool request;
redirects are refused. Existing mediated MCP remains available for other models.

Normal and streaming citations survive Chat and translated Responses output.
Provider tool counters remain separate from tokens and use cumulative snapshots.
Missing charges remain unknown. Monitor/export restores public definitions;
replay removes callable definitions, caps and private contexts. Cache/session and
classifier reuse are disabled. Tauri settings, camelCase parameters, typed calls,
demo state and the client settings panel are synchronized.

Plan Review, Test Coverage Review and Bug Hunt are complete for this slice.
Reviewed mixed ordering, nullable caps, forced choice, ignored engine constraints,
unknown fees, current permission changes, privacy, fallback after side effects,
redirect reposts, private-context injection, stream metadata and replay safety.
Contract and loopback regressions cover these boundaries; no paid calls were made.
The schema check initially expected anyOf; the generated union correctly uses
oneOf, and its corrected assertion passes. Full stable CI is being rerun before
commit. All 72 frontend tests, app/demo type checks and both production builds
passed. Installed-Chrome production smoke saved search permission and cap with no
page errors; screenshot /private/tmp/localrouter-server-tools-ui.png was inspected.

Ranked Search API ingress remains a separate pending slice in this same phase;
the master roadmap is not marked complete. Documents/OCR, rerank and video/realtime
also remain pending. Do not present this native-tool slice as completion of the
whole authorized roadmap.

Native slice final validation: rustup stable 1.99.0, compiler PATH and RUSTC pinned
to /Users/matus/.rustup/toolchains/stable-aarch64-apple-darwin/bin/rustc;
workspace Clippy with warnings denied, fmt check and workspace tests all passed
(111 suites; 4,443 passed, 69 ignored, zero failures). All frontend checks above passed.
The native provider-tool slice is ready for its normal commit and push. The ranked
Search API step remains pending.

Native tool slice committed and normally pushed as a56c5fac.

## Ranked Search API contract before code

Expose authenticated POST /v1/search and /search for ranked results, using typed
Perplexity-compatible query/options/results rather than model-generated answers.
Support direct Perplexity and the known LLM Gateway adapter only. Reject unknown
parameters and unsupported provider-specific filters/types. Bound queries (1–5,
4 KiB each), results, content budgets, dates, domains and language/country codes;
validate again in the adapter. Transport performs one request, refuses redirects,
bounds response bytes and never retries. Preserve result order, URL/page provenance
and upstream identifiers without fetching a result URL.

Resolve an explicit provider/model or a per-client deliberately configured default
search model. Do not silently select an unrelated model or use chat Sonar as a
ranked search adapter. Reuse web_search authorization; current client credentials,
model/strategy access, privacy, rate policy and configured input scans/approvals
apply before dispatch. Strict privacy, automatic MCP, duplicate hops, free-only and
unknown-fee cost ceilings reject. Ranked search is a single request-based operation;
provider-managed loop settings do not apply. Missing charges remain unknown rather
than being fabricated from a token estimate. Model IDs/types must match the verified
search engine contract. No automatic fallback or classifier is needed.

Reuse the sequential shared request pipeline with compression/classification off
for the query projection. Keep ranked results separate from assistant output in
monitoring and Try It Out; record request-based accounting and unknown charges.
Add endpoint discovery/OpenAPI, typed default search settings, demo behavior and
relevant local UI. Do not capture payloads against metadata-only settings. No new
persistent retrieval cache or replay endpoint is implied.

Review/test/bug-hunt focus: client/default isolation, unsupported people search on
LLM Gateway, direct web/fast result caps, no hidden fallback/redirect, query bounds,
invalid date/domain mappings, unknown charges, permissions/privacy/cost rejection
before tool IO, safe cancellations and live schema/HTTP fixtures. Finish exact
stable workspace validation, frontend checks/builds, normal commit and push before
moving on to documents/OCR, rerank and video/realtime.

Primary schemas rechecked 2026-10-15:
https://docs.llmgateway.io/v1_search and
https://docs.perplexity.ai/api-reference/search-post . LLM Gateway accepts a string
or bounded query list and returns ranked page records; its people mode is currently
unsupported. Direct Perplexity has different result bounds for web/fast and people.

## Ranked slice final review and validation

Implemented both authenticated aliases, direct Perplexity and known LLM Gateway
adapters, explicit per-client defaults, native request/result contracts, provider
mode differences, bounded reads, provenance and unknown-fee accounting. A direct
Perplexity `provider/search` is an explicit local operation alias; it is never sent
as a fictitious native model. The gateway fast model works as a configured default
without an extra type parameter. Current grants, privacy, model permissions and
rate policy are rechecked. One dispatch counts even on cancellation/failure; SDK
no-retry hints are emitted for post-boundary failures. No result URL is fetched.

Plan Review, Test Coverage Review and Bug Hunt are complete. They addressed default
isolation, unsupported type/model/filter combinations, duplicate/MCP hops, native
versus chat conversion, unknown costs, sequential approval, quota accounting on
abandonment, redirect refusal, bounded error bodies and fake demo credential
consistency. Existing native Messages document handling was only read for the next
phase; no document/OCR implementation is included in this slice.

Exact stable 1.99.0 Clippy/fmt/workspace tests passed: 111 suites, 4,451 passed,
69 ignored, zero failures. Targeted native Perplexity and four authenticated
ranked-search/cancellation/SDK-hint fixtures passed. App/demo TypeScript checks,
all 72 frontend regressions and both production builds passed. Production Chrome
smoke saved the per-client default/grant and displayed a clearly marked local
ranked result with no page errors or external requests. Final screenshot was
inspected; tab wrapping and unused generation-model controls were adjusted for the
search UI, followed by rebuilt app/demo bundles and another successful smoke.

Search/server tools are ready for normal commit/push. The remaining approved work
is documents/OCR, rerank, and video/realtime, followed by the master final review.

Final browser recheck after tab wrapping/search-only controls passed; screenshot /private/tmp/localrouter-ranked-search-ui-final.png was inspected. Official Perplexity pricing documentation additionally confirms request-based (not per-query/token) billing; actual response fees remain unknown and are not fabricated from that public rate card.
