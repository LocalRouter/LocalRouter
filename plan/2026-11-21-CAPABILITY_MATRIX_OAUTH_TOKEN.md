# Capability matrix truthfulness and OAuth token endpoint

Date: 2026-11-21
Branch: `fix/capability-matrix-oauth-token`

## Problem

1. The provider compatibility matrix (`lr_providers::default_feature_support`)
   reports Moderations, Responses API, Batch Processing and Realtime as
   "not yet available in LocalRouter", although `lr-server` routes
   `/v1/moderations`, `/v1/responses`, `/v1/batches*` and `/v1/realtime`.
   The audio rows carry the same stale wording for providers without an audio
   mapping.
2. `POST /oauth/token` only accepts JSON, while OAuth 2.0 (RFC 6749 §3.2,
   §4.4) and the in-app "How to connect" text specify
   `application/x-www-form-urlencoded`. Its per-client attempt map is keyed by
   an unauthenticated `client_id` and never shrinks.
3. `src-tauri/src/ui/mod.rs` carries a `TODO: Implement UI integration`
   comment above modules that implement exactly that.

## Findings

| Endpoint | Real behaviour | Structured source |
|---|---|---|
| `/v1/moderations` | Answered by GuardRails safety models hosted on a chat provider; never forwarded to a provider's moderation API | `supports_chat()` |
| `/v1/responses` | Chat pipeline; native only on the ChatGPT subscription backend | `api_path_support("responses")` |
| `/v1/batches` | Native only (OpenAI API key, OpenRouter) | `native_batch_kind()` |
| `/v1/realtime` | Native only (OpenAI API key, LLM Gateway without gateway controls) | `realtime_transport()` |
| audio | Registry filters providers by `supports_transcription` / `supports_speech` | same hooks |

## Review notes

- Try-it-out availability for batches and realtime reads `get_api_path_support`
  (`src-tauri/src/ui/commands_providers.rs`), which already derives from
  `native_batch_kind()` / `realtime_transport()`; the matrix now agrees with it.
- `/v1/responses` storage claim checked against `routes/responses.rs`
  (`store` / `previous_response_id` persisted in the local session store).
- Limiter eviction is O(tracked clients) only when the table is full; with
  the 4096 cap that is a short critical section. An attacker can still reset a
  victim's count by filling the table with ids at the full allowance
  (~41k requests per reset of 10 guesses); client secrets are high-entropy, so
  this is accepted.
- HTTP Basic credentials are not form-urldecoded (RFC 6749 §2.3.1); issued
  ids and secrets are URL-safe, so the behaviour is unchanged from before.

- Bug hunt found axum's `Form`/`Json` extractors match `Content-Type`
  case-sensitively, so `Application/X-WWW-Form-Urlencoded` was classified as
  form but then refused (400). The handler now rewrites the header to the
  canonical media type before delegating; tests cover mixed-case form and JSON.

## Validation

- `cargo fmt --all -- --check`: pass
- `cargo clippy --workspace --all-targets -- -D warnings`: pass (re-run
  `-p lr-server` after the final fix: pass)
- `cargo test -p lr-providers`: lib 739 passed; doctests could not run
  (`extern location for lr_catalog does not exist` in the shared target dir,
  another build replaced the rlib; not related to this change)
- `cargo test -p lr-server`: 268 passed
- `npx tsc --noEmit`, `npm run test:unit` (151 passed), `npm run build`,
  `npm --prefix website run build:check`: pass
- `src-tauri` change is comment-only; compiled by clippy, tests not run

## Checklist

- [x] Read the routes and provider hooks; record findings above
- [x] `default_feature_support`: derive the four rows from the hooks the
      routes call; audio rows report `NotSupported` instead of `NotImplemented`
- [x] Drop the stale OpenAI overrides and the generic-provider
      `NotImplemented -> Partial` endpoint rewrite
- [x] Keep `SupportLevel::NotImplemented` (serialized value) but document it
      as reserved
- [x] Frontend: remove the unused legend entry in the compatibility panel,
      annotate the TypeScript type, update the website demo mock
- [x] Tests pinning the matrix rows to the provider hooks
- [x] `/oauth/token`: accept form-encoded and JSON bodies, 415 otherwise;
      body credentials or HTTP Basic
- [x] Bounded attempt tracker (expired entries swept, then least-active/oldest
      evicted; over-long client ids never tracked)
- [x] OpenAPI annotation lists both content types and the 415/429 responses
- [x] Tests: form, JSON, Basic, bad content type, malformed body, cap
- [x] Remove the stale TODO in `src-tauri/src/ui/mod.rs`
- [x] Plan Review: compare this plan against the implementation
- [x] Test Coverage Review: new and modified paths have tests
- [x] Bug Hunt: re-read the diff for logic, locking and compatibility bugs
- [x] Validation (fmt, clippy, crate tests, tsc, unit tests, builds), commit, push
