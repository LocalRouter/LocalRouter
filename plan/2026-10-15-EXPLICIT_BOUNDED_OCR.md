# Explicit bounded OCR

- [ ] Verify current direct Mistral and LLM Gateway OCR wire contracts, provider builders, shared client policies, monitoring and Try It Out document controls.
- [ ] Add typed OCR request/response contracts. Require one bounded inline PDF or image and an explicit qualified native model; reject file IDs, remote URLs, unknown options and automatic model selection. Require 1–20 unique zero-index pages (or a bounded normalized range), never implicit whole-document processing.
- [ ] Add a separate default-off client native-operation OCR grant, with config validation, synchronized Tauri/TS/demo commands and client controls. Recheck credentials, model permissions, privacy, free-only/cost limits and request quotas before dispatch.
- [ ] Add verified direct Mistral and LLM Gateway transports. Disable redirects, retries and gateway fallback; bound response/error bodies. Restrict wire options to those explicitly supported and report unknown fees without inventing page prices or text-token usage.
- [ ] Add POST /v1/ocr and /ocr with authenticated aliases, OpenAPI schemas/registration, operation discovery and monitoring. Reuse the normal sequential permission/rate/approval pipeline; opaque input must reject policies requiring extracted-content scans. Avoid classification/compression/cache/session/replay/MCP orchestration.
- [ ] Preserve source filename/hash and native page indices, dimensions and reported pages processed. Validate returned pages are requested and usage does not exceed the explicit page budget. Treat markdown/image references as untrusted data and never auto-fetch or execute them.
- [ ] Extend Try It Out with explicit OCR model/pages and a labeled local-only demo. Show plain markdown/text and raw response data without remote images. Cancellation aborts local IO and retains unknown upstream outcome/charges.
- [ ] Add meaningful contract, size, malformed page/range, default-off/live authorization, privacy/cost, requested-page provenance, quota, cancellation and no-retry HTTP regressions against local mock providers.
- [ ] Plan Review: compare promised mappings, page bounds, policies, aliases, UI and accounting with implementation and finish omissions.
- [ ] Test Coverage Review: cover missed boundaries, ownership, policy revocation, provider error and cancellation paths.
- [ ] Bug Hunt: inspect page off-by-one errors, implicit whole-document billing, opaque scan bypasses, retention, redirects/retries, response allocation, unsafe markdown and metadata loss.
- [ ] Update stable, pin actual compiler PATH/RUSTC, run exact workspace Clippy/fmt/tests plus frontend regressions, app/demo types/builds and a production browser smoke. Use /private/tmp/localrouter-gateway-validation.
- [ ] Commit and normal push validated changes and automatic catalog updates, preserving unrelated work. Mark document/OCR complete only after this operation is fully validated, then continue rerank and video/realtime.

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
