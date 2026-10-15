# Native rerank API

- [x] Verify direct Cohere v2 and LLM Gateway rerank contracts, current provider builders, native-operation authorization and quota meters before adapter code.
- [x] Add bounded typed request/response contracts: explicit qualified model, nonempty query up to 16 KiB, 1–128 text candidates up to 32 KiB each / 1 MiB total, top_n within candidate count. Preserve original zero-based indices and upstream order; reject duplicate/out-of-range/nonfinite results. Do not replace reranking with keyword heuristics or ordinary LLM calls.
- [x] Add a separate default-off rerank grant to native operations with synchronized enum/types/demo/client UI. Preserve model permissions, credential lifetime, strict privacy, rate quotas and unknown-cost restrictions. Use native billing units rather than inventing Cohere text tokens. Reject limit types whose semantics cannot be safely enforced by the selected native meter.
- [x] Add direct Cohere /v2/rerank and LLM Gateway /v1/rerank transports with bounded success/error bodies, no redirects/retries/fallbacks and explicit unsupported-control errors. Cohere max_tokens_per_doc is verified; reject unverified gateway chunk/token caps. Preserve native billed units and unknown fees.
- [x] Add authenticated /v1/rerank and /rerank, OpenAPI/schema registration, capability discovery and monitoring. Reuse sequential permission/scanning/approval checks on actual query/candidate text. Disable classification/compression/cache/affinity/automatic MCP/replay, and reject silent content/model rewrites.
- [x] Support return_documents by mapping validated indices to the exact original candidates locally. Do not trust mismatched upstream document text or invent document IDs. Document native truncation and that relevance scores are not calibrated across models or queries.
- [x] Add client-only Try It Out Rerank with manual model/query/candidates, explicit action, no SDK retries, cancellation and native score/source presentation. Use a clearly labeled local demo fixture, no external assets or implicit corpus retrieval.
- [x] Add meaningful bounds/index/order/score/unit/permission/privacy/limit/revocation/cancellation/no-retry regressions and live aliases/OpenAPI/native payload tests against local mock providers.
- [x] Plan Review: compare implementation with every selected contract, bounds, wire mapping, policy, accounting, discovery and UI requirement and finish omissions.
- [x] Test Coverage Review: cover missing error, isolation, live policy, quota and cancellation paths.
- [x] Bug Hunt: inspect reordered/mismatched documents, invalid score/index handling, truncated native content, billing-unit confusion, privacy/scan bypasses, retries, allocations and stale authorization.
- [x] Update stable, pin actual compiler PATH/RUSTC, run exact workspace Clippy/fmt/tests plus frontend regressions, both type checks/production builds and production browser smoke. Use /private/tmp/localrouter-gateway-validation.
- [x] Commit and normal push completed validated changes and automatic catalog updates, preserving unrelated work. Then continue video/realtime and the overall roadmap review.

Implement this after the document/OCR phase. Reuse its explicit native-operation
permission and cancellation accounting; do not fold rerank into a web-search grant.
This is a native scoring operation over caller-provided candidates, not a hosted
knowledge base, crawler or vector database. No documents are fetched implicitly.

Verified primary contracts (2026-10-15):
- https://docs.cohere.com/v2/reference/rerank (current example rerank-v4.0-pro;
  string candidates, top_n, max_tokens_per_doc; meta.billed_units.search_units)
- https://docs.llmgateway.io/features/rerank (Cohere-compatible shape; current
  example qwen3-reranker-8b, optional explicit deepinfra/ prefix; native input-token
  billed units; max_chunks_per_doc accepted but not honored by every provider)

Use the verified protocol's native models, not stale model-name assumptions.
Missing billed units and charges remain unknown. Native truncation is explicit
provider behavior; mapping a result index back to full source text is provenance,
not a claim that every byte was scored. No raw executable HTML or remote images
are rendered from candidate strings.

Implemented and reviewed (2026-10-15): typed native Cohere v2 / LLM Gateway
rerank, exact original candidate mapping, native order/score validation, retained
billed units and unknown fees, independent default-off grant, current model/
credential/privacy/budget checks, sequential actual-text scans, authenticated
aliases/OpenAPI/endpoint discovery, monitoring and client-only Try It Out.
Unverified chunk caps and gateway token caps fail; native token ceilings reject
rather than silently treating Cohere search units as text tokens. Reported LLM
Gateway integer input units feed token statistics. No keyword/LLM emulation,
hidden retrieval, rewrite, cache, session binding or automatic replay is added.

Plan Review: checked selected native contracts, default-off/migration behavior,
index-source binding, top_n and byte bounds, direct controls, billable unit
semantics, rate/request accounting, no-retry errors and UI/demo synchronization.
Returned native document text is discarded; optional source text is reconstructed
only from validated original indices. Both OCR and rerank permissions are
preserved when toggling/saving independently.

Test Coverage Review: five native provider regressions and six live HTTP
regressions cover input/aggregate/control bounds, malformed scores/order/indices,
Cohere auth/endpoint/redirect refusal, exact source mapping/native billed units,
request token statistics, grants/revocation, privacy/free-only/cost/token ceilings,
error/cancellation quotas, no-retry semantics, authenticated aliases and live
OpenAPI. Tests use local mocks with no paid calls.

Bug Hunt: checked invalid-index panics, duplicate source text versus duplicate
indices, stale/mismatched upstream text, accidental unit/price conversion,
metadata trust, retry/redirect paths, cancelled futures, policy projection rewrites
and permission toggle isolation. The demo labels illustrative ranking plainly;
candidate content is rendered as plain text, with no external asset loading.

Final validation: stable 1.99.0 and actual rustup compiler pinned in PATH/RUSTC;
exact workspace Clippy -D warnings, fmt check and all 111 test suites passed
(4,492 passed, 69 ignored, zero failures). Logs: /tmp/rerank-ci-clippy.log,
/tmp/rerank-ci-fmt.log, /tmp/rerank-ci-workspace.log. Both frontend type checks,
app/demo production builds, 72 frontend regressions and production Chrome smoke
passed; screenshot /private/tmp/localrouter-rerank-ui.png inspected. Continue
native video jobs and realtime sessions before overall roadmap completion.
