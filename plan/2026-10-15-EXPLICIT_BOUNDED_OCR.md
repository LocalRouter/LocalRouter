# Explicit bounded OCR

- [x] Verify current direct Mistral and LLM Gateway OCR wire contracts, provider builders, shared client policies, monitoring and Try It Out document controls.
- [x] Add typed OCR request/response contracts. Require one bounded inline PDF or image and an explicit qualified native model; reject file IDs, remote URLs, unknown options and automatic model selection. Require 1–20 unique zero-index pages (or a bounded normalized range), never implicit whole-document processing.
- [x] Add a separate default-off client native-operation OCR grant, with config validation, synchronized Tauri/TS/demo commands and client controls. Recheck credentials, model permissions, privacy, free-only/cost limits and request quotas before dispatch.
- [x] Add verified direct Mistral and LLM Gateway transports. Disable redirects, retries and gateway fallback; bound response/error bodies. Restrict wire options to those explicitly supported and report unknown fees without inventing page prices or text-token usage.
- [x] Add POST /v1/ocr and /ocr with authenticated aliases, OpenAPI schemas/registration, operation discovery and monitoring. Reuse the normal sequential permission/rate/approval pipeline; opaque input must reject policies requiring extracted-content scans. Avoid classification/compression/cache/session/replay/MCP orchestration.
- [x] Preserve source filename/hash and native page indices, dimensions and reported pages processed. Validate returned pages are requested and usage does not exceed the explicit page budget. Treat markdown/image references as untrusted data and never auto-fetch or execute them.
- [x] Extend Try It Out with explicit OCR model/pages and a labeled local-only demo. Show plain markdown/text and raw response data without remote images. Cancellation aborts local IO and retains unknown upstream outcome/charges.
- [x] Add meaningful contract, size, malformed page/range, default-off/live authorization, privacy/cost, requested-page provenance, quota, cancellation and no-retry HTTP regressions against local mock providers.
- [x] Plan Review: compare promised mappings, page bounds, policies, aliases, UI and accounting with implementation and finish omissions.
- [x] Test Coverage Review: cover missed boundaries, ownership, policy revocation, provider error and cancellation paths.
- [x] Bug Hunt: inspect page off-by-one errors, implicit whole-document billing, opaque scan bypasses, retention, redirects/retries, response allocation, unsafe markdown and metadata loss.
- [x] Update stable, pin actual compiler PATH/RUSTC, run exact workspace Clippy/fmt/tests plus frontend regressions, app/demo types/builds and a production browser smoke. Use /private/tmp/localrouter-gateway-validation.
- [x] Commit and normal push validated changes and automatic catalog updates, preserving unrelated work. Mark document/OCR complete only after this operation is fully validated, then continue rerank and video/realtime.

This is the second increment of 2026-10-15-DOCUMENT_INPUTS_AND_OCR.md. Native
Chat/Responses files and local text input are a distinct validated increment;
OCR must never become an implicit parser choice on that path.

Use an independent native-operation authorization policy rather than pretending
OCR is a web-search tool. Defaults grant nothing. The first protocol surface
accepts inline PDFs and supported PNG/JPEG images only: LocalRouter does not
fetch arbitrary documents or accept unowned file references. Keep the same
4 MiB decoded document bound. Explicit page selection bounds the requested
operation, not a promise about an upstream that violates its own contract.
If a response reports excess work, fail without claiming no charge occurred.
No unverified whole-document or structured-annotation chain is enabled.

Direct model: mistral-ocr-latest (or explicit verified versioned OCR model).
LLM Gateway currently exposes mistral-ocr-latest; require its documented native
model form and reject unverified provider pins/routing controls instead of
silently ignoring restrictions. Confirm the precise IDs before transport tests.

Primary references verified 2026-10-15:
- https://docs.mistral.ai/api/endpoint/ocr
- https://docs.mistral.ai/studio/document-processing/basic_ocr
- https://docs.llmgateway.io/features/ocr
- https://docs.llmgateway.io/v1_ocr

Implementation review (2026-10-15): verified direct Mistral and LLM Gateway native
contracts; typed inline PDF/PNG/JPEG, explicit pages and bounded responses are
implemented. Client native operations have an independent default-off OCR grant,
with full Rust/Tauri/TS/demo synchronization and live state updates. API aliases,
OpenAPI, endpoint discovery, page/source provenance, native usage and unknown-fee
monitoring, request quotas/cancellation, no-retry behavior and Try It Out are wired.
Opaque scan requirements are shared with native documents and reject content
policies that cannot inspect the binary input. No remote document URL/file-ID
fetch, structured annotation chain or implicit whole-document OCR is enabled.

Plan Review: checked every advertised protocol, model, bound and UI field. Native
filename is retained locally and stripped from wire payloads; returned page indices
must belong to requested pages, duplicates/excess work fail, native page counts
are not fabricated from output text. Strict gateway config controls without a
verified OCR mapping fail instead of being silently ignored. Existing native
Messages/files/search flows remain independent.

Test Coverage Review: contract tests cover range/array bounds, omitted pages,
unknown fields, model mapping, remote references, native page usage and source
provenance, MIME/header/image validation, response allocation and direct native
Mistral authorization/redirect behavior. Live HTTP regressions cover aliases/auth/
OpenAPI, default-off grant/live revocation, privacy/free-only/cost restrictions,
exact payloads, quotas on failure/cancellation, unknown charges and opaque blocking
secret scans. Tests use only loopback mock providers.

Bug Hunt: reviewed zero-index/inclusive-range math, limits before allocation,
credential/grant checks at dispatch, dropped futures, local-only source labels,
JSON/SSE metadata, guarded request projections, native body field stripping,
client setting update isolation and untrusted OCR display. No automatic retry or
OCR job resubmission is introduced. Browser smoke and screenshot inspection show
page provenance as plain text without executing markup or loading image URLs.

Validation in progress: initial seven live HTTP tests, OCR contract tests,
72 frontend regressions, app/demo types and production builds and a production
Chrome permission/page-range smoke passed. Final exact stable workspace parity
includes additional image/body-bound and opaque-scan coverage. Commit/push follows
only when those complete successfully. Document commit 472f2885 is green in CI.

Final validation completed: updated stable 1.99.0, actual rustup compiler pinned
in PATH/RUSTC, exact workspace Clippy (-D warnings), fmt check and all 111 test
suites passed (4,481 passed, 69 ignored, zero failures). Final logs:
/tmp/ocr-ci-clippy-final.log, /tmp/ocr-ci-fmt-final.log and
/tmp/ocr-ci-workspace-final.log. Both frontend type checks, app/demo production
builds, 72 frontend regressions and production Chrome permission/page-range
smoke passed. Screenshot inspected: /private/tmp/localrouter-ocr-ui.png.
Eight live OCR HTTP regressions passed, including blocking secret scans; native
Mistral redirect/auth and bounded image/response tests passed in the full suite.
No paid provider calls were made. Native document + OCR phase is complete;
rerank proceeds under 2026-10-15-NATIVE_RERANK_API.md.
