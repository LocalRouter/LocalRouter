# Router admission, OAuth and provider capability follow-ups

Date: 2026-11-22
Branch: `fix/router-oauth-provider-followups`

Sources: the "Left for follow-up" and review notes of
`plan/2026-11-21-ROUTER_ADMISSION_FAILOVER_TESTS.md` and
`plan/2026-11-21-CAPABILITY_MATRIX_OAUTH_TOKEN.md`, plus older survey items.

## Findings (verified in the code before changing it)

1. **Strategy reservation released before the metrics row** — confirmed.
   Strategy limits are checked against `llm_strategy:<id>` metrics rows,
   which `lr-server` writes in `finalize_metrics_and_monitor` /
   `finalize_streaming_at_end` after the router call returns (or after the
   router's stream wrapper has finished). The router drops the strategy
   reservation at the end of `complete` (`_strategy_admission`) and inside
   `StreamUsageRecorder::finish`, so for a moment neither counts. The
   client reservation is not affected: the router charges the client
   limiter itself before releasing it.
2. **Non-chat endpoints only check strategy limits** — confirmed for
   embeddings, audio (transcription, translation, speech), System One,
   native operations (OCR, rerank, realtime/native dispatch), search and
   token counting (`check_strategy_rate_limits`). Images and moderations do
   not go through the router at all and check no limit (neither client nor
   strategy).
3. **Retry-After of 1 s when only in-flight requests block** — confirmed
   (`admit_api_key`). One second is wrong for request/token limits: an
   in-flight request that finishes becomes recorded usage for a whole
   window, so room only returns when enough recorded usage expires.
4. **OAuth Basic credentials not form-urldecoded** — confirmed
   (`extract_basic_auth`).
5. **Moderations "Translated" on every chat provider** — confirmed
   (`default_feature_support` ignores GuardRails configuration).
6. **`n > 1` accepted but one completion generated** — partly: the
   pipeline passes `n` to the provider request, and providers that forward
   it (OpenAI API, OpenRouter, OpenAI-compatible, TogetherAI, DeepInfra)
   return several choices. Every other provider drops `n` and returns one
   choice. Streaming with `n > 1` is already a 400.
7. **Video cancel 501 / delete 501 for non-OpenAI** — confirmed. Neither
   upstream protocol has a verified cancel endpoint; only the OpenAI Videos
   API has a verified delete. The capability matrix has no videos row, and
   `ApiPathSupport` does not say whether cancel/delete are available.
8. **Native batches and free-tier-only routing** — confirmed rejected at
   submission (`batch_jobs/context.rs::authorize`, 400). Native batches run
   on paid batch APIs billed asynchronously; free-tier limits cannot be
   checked or tracked for them, so rejection is the correct behaviour.
9. **Gemini embeddings reject batch input** — confirmed; Gemini has
   `batchEmbedContents`. **`reasoning_tokens` not extracted** — confirmed:
   the adapter returns a placeholder gated on model-name prefixes and
   claims the tokens are in `prompt_tokens`.

## Design / fixes

1. `lr_router::AdmissionHandoff`: a request-scoped keeper (task-local) for
   reservations whose usage is recorded outside the router. The router
   hands a successful call's strategy reservation to the current handoff
   instead of dropping it (non-streaming at the end of the call; streams
   when the usage recorder finishes). `lr-server` opens one handoff per
   HTTP request in a middleware and keeps it alive in the response body,
   so it is released when the response has been produced/streamed — after
   the finalize step has written the metrics row. Abandoned streams keep it
   until their spawned finalize has run. No handoff (background callers,
   other crates) keeps the old behaviour: released at the end of the call.
   This is the seam the proxy wave can use.
2. Every non-chat router entry point reserves its strategy limits
   (`reserve_strategy_rate_limits_with`, token estimate where one exists:
   embeddings input estimate, audio file-size estimate) and hands the
   reservation off on success, drops it on failure. Auto-routing loops
   reserve per attempt. `Router::admit_direct_request` reserves client and
   strategy limits for images and moderations, which `lr-server` serves
   without the router; their request counts are charged to the client
   limiter.
3. Retry-After: the time until enough recorded usage expires for recorded
   usage plus in-flight reservations to fall below the limit; in-flight
   usage that cannot expire earlier counts as a full window.
4. Basic credentials are form-urldecoded (`+` and `%XX`); issued ids and
   secrets are URL-safe, so they decode to themselves.
5. Moderations row derived from structured GuardRails configuration:
   `Translated` only when the moderation API is enabled and an enabled
   safety model is configured on that provider instance; otherwise
   `NotSupported` with the reason. Applied in the Tauri commands; demo mock
   updated.
6. `n > 1` (non-streaming) is answered with `n` choices for every provider:
   `n` is passed to every provider; missing choices are generated by
   further single-choice dispatches on the same provider/model, merged
   with consecutive indexes and summed usage (and gateway-reported cost
   while every response reports one). Whether a provider honours `n` is
   read from the choices it returns, so no `supports_n_choices()` flag is
   needed. A failed top-up fails the request after charging the calls that
   answered. Streaming with `n > 1` stays a clear 400.
7. Video cancel/delete: capability methods on `VideoTransport`; cancel and
   unsupported delete return a structured 501 naming the operation; a
   "Videos" row in the matrix and `native_video_cancel` /
   `native_video_delete` flags in `ApiPathSupport` (TS + mock).
8. Keep the rejection; reword it to state why and test it.
9. Gemini `batchEmbedContents` for multiple inputs (chunks of 100);
   reasoning adapter reads `usage.completion_tokens_details.reasoning_tokens`
   (no model-name gating) and writes its own `reasoning_tokens` response
   extension. The router still discards `FeatureData` from adapters in
   general: surfacing it for every adapter would add keys (e.g. duplicated
   logprobs) to every response that requests an extension.
10. Bug hunt in router / rate limiter / free tier / admission plumbing.

## Checklist

- [x] Plan saved and committed
- [x] 1 Strategy reservation hand-off to lr-server finalize (chat, responses,
      messages; streaming and not) + tests
- [x] 2 Non-chat endpoints reserve strategy limits; images/moderations admit
      + tests
- [x] 3 Retry-After from window expiry + tests
- [x] 4 OAuth Basic form-urldecoding + tests
- [x] 5 Moderations matrix row from GuardRails config; TS/mock + tests
- [x] 6 `n > 1` choices + tests
- [x] 7 Video cancel/delete capability + matrix + tests
- [x] 8 Batch free-tier rejection reworded + test
- [x] 9 Gemini batch embeddings; reasoning tokens extraction + tests
- [ ] 10 Bug hunt; fixes with regression tests
- [x] Plan Review
- [ ] Test Coverage Review
- [ ] Bug Hunt (final pass over the diff)
- [ ] fmt, clippy, targeted tests, frontend checks
- [ ] Commit and push
