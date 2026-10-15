# Document inputs and explicit OCR

- [x] Audit current native Messages documents, Chat/Responses parts and provider conversion, request/response size limits, safety/secret/compression projections, cache/affinity/replay, existing MCP resources and endpoint discovery. Verify current native file/OCR contracts from primary sources before each adapter is enabled.
- [x] Add bounded typed file content with inline data, MIME/encoding validation and provenance. Support verified native document protocols; unsupported models, file IDs, formats, URL/reference semantics and conversions must reject explicitly. Keep existing native Messages document bytes/ordering/citations intact.
- [x] Add an explicit native processing policy for gateway documents so an omitted parser option cannot trigger an unexpected third-party OCR chain. Preserve current permissions, privacy, budget and scan/approval behavior. Do not treat opaque PDF bytes as inspected plain text or silently drop a document before classification.
- [x] Reuse existing MCP resources/extraction workflows for optional local extraction. Support bounded locally supplied extracted text with clear provenance; do not automatically install an extractor, fetch documents or create a knowledge-base service. Add a bundled local PDF parser only if the audit establishes a bounded safe implementation worth the dependency.
- [ ] Add an explicit OCR API for verified direct Mistral and LLM Gateway protocols, with default-off per-client authorization, explicit bounded pages, per-page provenance/usage and unknown-fee handling. Keep returned markdown/images as untrusted data. No implicit whole-document paid OCR or hidden inference fallback.
- [ ] Preserve normal/streaming file/citation metadata where supported. Disable unsafe cache, affinity, classifier reuse, automatic MCP/batch/replay re-execution and unsupported continuation state. Apply native dispatch controls and avoid retries/redirects after an ambiguous OCR dispatch.
- [ ] Add model/protocol eligibility discovery, OpenAPI annotations/schemas/registration, monitoring and Try It Out. Synchronize any Tauri commands, TS responses/camelCase parameters, typed invokes and demo handlers. Use bundled icons and assets only.
- [ ] Add meaningful malformed/oversized/opaque input, unsupported conversion, permissions/privacy/budget, page/filename provenance, normal/streaming, cancellation/no-retry and live HTTP/schema regressions without paid provider calls.
- [ ] Plan Review: compare every promised mapping and limit with the implementation; finish omitted behavior and document explicit unsupported mappings.
- [ ] Test Coverage Review: add coverage for uncovered isolation, boundary and error behavior in new/changed paths.
- [ ] Bug Hunt: inspect opaque scan bypasses, PDF parser billing, retention, file ownership, content injection, provenance loss, retries, response allocations, cancellation and stale permissions.
- [ ] Update stable, pin actual compiler PATH/RUSTC, then run exact workspace Clippy/fmt/tests, relevant frontend regressions, both type checks/production builds and browser smoke. Use /private/tmp/localrouter-gateway-validation.
- [ ] Commit and normal push completed validated changes and automatic catalog updates while preserving unrelated user changes. Continue rerank and video/realtime, then finish the master final review.

The approved roadmap requests native document pass-through and optional extraction,
not a hosted knowledge base. Native Messages already validates and preserves
provider document blocks. Extend that implementation instead of rebuilding it.
Search/server tools are completed in a56c5fac and 92f20a23; this phase must not
reopen those features except where a shared contract needs a justified extension.

Initial concrete scope before code: typed inline Chat file parts and corresponding
Responses input_file conversion, restricted to verified provider/endpoint mappings.
OpenRouter must explicitly select its native file engine rather than default to
paid Mistral OCR; no inferred retention promise follows from that field. LLM Gateway
currently documents inline data URLs for Google AI Studio/Vertex Gemini models;
file_id is not a supported Google mapping. Native direct providers must be verified
against their actual request builders before opting in. Unknown capabilities and
opaque auxiliary projections must remain explicit, not a fabricated text summary.

OCR is a separate user-requested operation. Require explicit bounded page indices
(or a bounded normalized range) and a per-client grant. Query/model permission,
credential lifetime, privacy, rate/budget and supported scan/approval requirements
apply before IO. Whole-document defaults, arbitrary file references, implicit paid
conversion and unsupported tool chains are not enabled by inference from a file
content block. Exact request/response bounds and native model IDs will be recorded
after the existing conversion audit and before adapter code.

Existing MCP resources and user-configured extraction remain the optional local
workflow. Do not introduce an unsafe or unbounded PDF parser just to claim local
extraction. Plain extracted text must retain filename/page provenance and be scanned
as actual text before inference. Any unsupported opaque scan requirement rejects
rather than silently bypassing the configured policy.

Verified primary references (2026-10-15):
- https://openrouter.ai/docs/guides/overview/multimodal/pdfs
- https://docs.llmgateway.io/features/documents
- https://docs.llmgateway.io/features/ocr
- https://docs.llmgateway.io/v1_ocr
- https://docs.mistral.ai/api/endpoint/ocr
- Installed OpenAI SDK responses.d.ts input_file/file_data definitions (official OpenAI documentation must be checked for direct OpenAI mappings before enabling one).

## Native document increment (2026-10-15)

Implemented inline Chat `file` and Responses `input_file` parts with four-file /
4 MiB each / 8 MiB decoded total bounds, UTF-8 validation, protected filename /
SHA-256 provenance and explicit local text mode. The verified native mappings
are OpenRouter PDF (always file-parser/native, never implicit paid OCR) and
LLM Gateway's explicitly named Google/Google Vertex Gemini models. Other Chat
conversions explicitly reject file input. Existing native Messages remains intact.

Opaque documents bypass compression, classifiers, caches, affinity and automatic
MCP loops; they require an explicit model and reject automatic replay, cost caps,
free-only strategies and policies requiring document content scans. Text files
project decoded contents into scans without changing their native wire bytes.
The main JSON limit now matches the existing 16 MiB transport limit. Native
Responses turns disable history storage/continuation. Provider dispatch disables
fallbacks and redirects, bounds JSON and retained SSE lines, and marks errors
non-retryable to compatible SDKs. Missing native stream usage is an error;
cancelled streams retain unknown document accounting rather than fabricated
PDF text tokens. Public response/monitor provenance records scan coverage.

Try It Out has a client-only Documents panel with bounded local file selection,
explicit processing mode, no SDK retries, cancellation, plain response text and
untrusted JSON. The website uses a clearly labeled bundled demo response and
never sends the selected document to a provider.

Plan Review: checked ingress/projection/provider conversion, Chat + Responses,
stream/error/cancellation accounting, request bounds, private metadata, native
Messages compatibility, replay/batch/cache/session boundaries. Persistent batch
validation already only permits text content and rejects file parts. Existing
MCP tools supply optional extraction; no hidden PDF parser/install/network fetch
was added. OCR and its distinct permission/page limits are still pending, as are
OCR capability discovery and UI. Direct OpenAI/Anthropic/Gemini Chat file
conversions remain unsupported until a separately verified adapter is enabled.

Test Coverage Review: unit regressions cover malformed types/data/filenames,
local text and provenance, scan projections, continuation/private/plugin rejection,
file/aggregate byte bounds, stream accounting and unterminated SSE allocation.
Live local HTTP regressions cover exact native file bytes on both ingress APIs,
normal/streaming output, explicit native OpenRouter parser/no-fallback, unsupported
model/free-only/no-classifier dispatch, replay rejection, SDK no-retry errors and
missing native usage. No paid requests are used.

Bug Hunt: fixed Responses dropping easy-message file content, rechecked qualified
versus native model IDs, prevented implicit OCR/fallback/redirect and opaque scan
bypasses, preserved unknown cost through stream cancellation, disabled native file
history persistence and ensured caller-provided provenance cannot be trusted.

Validation in progress: five initial live HTTP tests and document units passed;
72 frontend regressions, both app/website type checks and production builds, and
the production Documents Chrome smoke passed. Full stable workspace parity is
running before the native increment is committed. Overall document/OCR checkbox
remains open until the explicit OCR operation is complete.

Native increment final validation completed: stable 1.99.0 with the actual
rustup compiler pinned in PATH/RUSTC; exact workspace Clippy with -D warnings,
format check and all 111 test suites passed (4,466 passed, 69 ignored, zero
failures). Final logs: /tmp/documents-ci-clippy-final.log,
/tmp/documents-ci-fmt-final.log and /tmp/documents-ci-workspace-final.log.
Both final frontend type checks, app/demo production builds, 72 frontend
regressions and production Chrome smoke passed. Screenshot inspected:
/private/tmp/localrouter-documents-ui.png. No provider billing was incurred.
The final review also closed malformed Responses item shapes that could have
dropped an attachment. Explicit OCR proceeds under
2026-10-15-EXPLICIT_BOUNDED_OCR.md; overall document/OCR remains pending.
