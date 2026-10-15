# Native asynchronous video jobs

- [x] Reconcile the approved later-feature scope with existing BatchStore ownership/mutation patterns and current OpenAI SDK/official OpenAI + LLM Gateway video contracts. Keep native jobs separate from image/audio translation.
- [x] Add typed bounded text-to-video requests and sanitized job/status metadata. Support direct OpenAI API-key Sora and LLM Gateway native video protocols; reject unsupported reference assets, callbacks, edits/remix/extensions and routing controls explicitly. Require explicit provider/model, duration and bounded resolution.
- [x] Add a separate default-off video grant, synchronized Rust/Tauri/TS/demo/client controls and endpoint discovery. Apply current credentials/model permissions, sequential prompt scans/approvals, privacy, request quotas and unsupported budget-meter restrictions before creation.
- [x] Add durable owner-scoped video intent/status storage using existing private SQLite/storage protections. Require a caller Idempotency-Key and atomically bind it to the normalized request; persist intent before one upstream create. Repeated keys never resubmit, payload mismatch rejects, interrupted/ambiguous creates remain explicit unknown outcomes across restarts.
- [x] Bind native job IDs to original provider origin/credential fingerprint. Do not expose upstream global job listing or allow guessed/foreign IDs, credential rebinding or URLs to choose a transport. Store bounded sanitized metadata and prompt hash, not raw prompts, credentials or signed media URLs.
- [x] Add authenticated JSON and SDK text multipart create, local owner list, retrieve/status and bounded native content download. Parse both numeric/string durations. No background provider polling, automatic resubmission, redirects or fallback. Keep actual reported job fees distinct from unknown spend and token-based generation metrics; repeated status reads must not multiply charges.
- [x] Handle lifecycle limits honestly: native remote cancellation is not currently documented for these selected mappings. Expose that capability as unsupported; stopping local waiting never claims rendering/billing stopped. Only implement remote deletion where a verified completed/failed asset-delete contract exists, without pretending it cancels an active generation.
- [x] Preserve monotonic job states under concurrent polls, stale responses and restart. Bound active jobs, retained rows, metadata/body/media allocations and concurrent IO. Apply owner checks before any provider IO. Recheck live permissions/privacy/binding for status/content operations.
- [x] Add OpenAPI annotations/registration/live schema, native-operation monitoring and client-only Try It Out generation/list/refresh/download. Generate a fresh local idempotency key only for an explicit new request; preserve the key on retry. Use clearly labeled bundled demo jobs, no remote assets or paid calls in validation.
- [x] Add meaningful storage race/restart/owner/idempotency and provider/HTTP normal/error/ambiguous outcome/privacy/quota/binding/size/download regressions against local mocks. Verify SDK multipart compatibility and that no unsupported cancellation/action reaches the provider.
- [x] Plan Review: compare every promised endpoint, native mapping, owner/binding/idempotency/state behavior, limit, fee and UI behavior with implementation and finish omissions.
- [x] Test Coverage Review: cover missing state transitions, crash/cancellation windows, isolation, live policy changes, content bounds and error paths.
- [x] Bug Hunt: inspect double-submit/retry races, guessed IDs, stale polls, cross-provider credentials, misleading cancellation/zero fees, signed URL leakage, allocations, request rewriting and payload retention.
- [x] Update stable, pin actual compiler PATH/RUSTC, run exact workspace Clippy/fmt/tests plus frontend regressions, both types/production builds and a production browser smoke. Use /private/tmp/localrouter-gateway-validation.
- [x] Commit and normal push validated changes and automatic catalog updates, preserving unrelated work. Continue native realtime before the overall roadmap final review.

Concrete initial native surface: text prompts up to 16 KiB, explicit seconds up
to 30 and 720p landscape/portrait sizes. Direct OpenAI uses verified Sora models
and its stricter duration mapping; ChatGPT OAuth is not a public video API key.
LLM Gateway forwards its documented native JSON shape with explicit model and
no-fallback behavior. Model-specific unsupported sizes/durations fail explicitly.
No remote reference fetch, webhook, hidden conversion, or automatically chained
video variation is part of this surface.

Use a local opaque video ID per owner and keep native IDs/private credential
binding in the durable row. Local list does not enumerate the provider account.
Require Idempotency-Key even if the upstream cannot honor one; local intent
uniqueness protects compatible SDK retries and lost create responses. A retry
with the same key returns the recorded state without a second native create.
Do not treat an ambiguous dispatch as a confirmed failed/unbilled generation.

Status/content are explicit user actions. Remote rendering can outlive a client
connection. Provider cancellation is unsupported unless a native contract is
verified; OpenAI asset deletion is only for completed/failed jobs. Native reported
usage.cost is authoritative when present; missing fees stay unknown. Per-job fees
remain available in the video lifecycle instead of being fabricated from seconds
or folded into text-token estimates. Local metadata uses normal retention/privacy
protections; strict no-retention/local-only profiles reject unverified remote jobs.

Verified primary sources (2026-10-15):
- Installed OpenAI SDK resources/videos.d.ts (create/retrieve/list/delete/download;
  deletion is completed/failed asset deletion, not active cancellation).
- https://developers.openai.com/api/docs/guides/video-generation
- https://docs.llmgateway.io/features/video-generation
- Existing crates/lr-batches storage/journal and the approved gateway roadmap.

Validation complete: stable Rust 1.99.0 workspace Clippy/fmt/tests passed
(111 suites, 4,507 passed, 69 ignored, zero failures); all 72 frontend tests,
app/demo type checks and production builds, and final production Chrome smoke
passed. Native video transport/download/deletion regressions: five; durable
journal regressions: four; live HTTP lifecycle regressions: six. No paid calls.
Native mappings and limits are documented in docs/VIDEO_JOBS.md.

Realtime remains pending under 2026-10-15-NATIVE_REALTIME_SESSIONS.md.
