# Access before the Secret Scan Ask popup (native server)

Branch `fix/access-before-secret-ask`. Follows the "Not fixed" note of
`plan/2026-11-23-SERVER_SKILLS_PLATFORM_LEFTOVERS.md` (Plan Review, Bug Hunt 5).

## Problem

The Secret Scan boundary (`secret_placeholders::run_scoped`) reads, scans and masks
JSON requests before the route handler runs: inference routes (chat, completions,
responses, messages, systemone, decisions) in Redact/Ask, content routes
(embeddings, moderations, rerank, search, OCR, image generation, speech, JSON video,
`messages/count_tokens`) in every action. A request the handler then refuses
(client disabled, LLM access Off, strategy missing, model not allowed, strategy
permission Off, native operation not enabled, rate limited) still opened a Secret
Scan Ask popup (blocking, the user is asked about a request that will not run), a
Notify popup, and a Secret Scan monitor event with the unmasked finding (Notify).

## Findings per route (before this change)

Every refusal below ran in the handler, after the boundary scan.

| Route | Non-interactive refusals the handler runs (in order) | Interactive approvals |
|---|---|---|
| chat | privacy auxiliaries; model access (auto alias, auto candidates/not configured, strategy permission, model allow-list, LLM access); rate limit (not on duplicate hops) | auto-router popup (between the auto checks and strategy permission), model Ask |
| completions | client + LLM access; then as chat | as chat |
| responses | as chat (built chat request) | as chat |
| messages, count_tokens | model resolution (bare names); as chat | as chat (messages) |
| systemone, decisions | client + LLM access; auto not configured/empty, strategy permission, model allow-list; rate limit | auto-router popup (before the strategy checks), model Ask |
| embeddings | client + LLM access; auto alias; strategy permission + model allow-list; rate limit | none |
| speech | client + LLM access; auto alias; strategy permission + model allow-list | none |
| image generation | `check_image_access` (client, LLM access, strategy, model) | none |
| moderations | client + LLM access | none |
| rerank, OCR, JSON video | native operation authorization; (OCR: opaque-document policy); then as chat | model Ask |
| search | ranked-search authorization (default model); then as chat | model Ask |
| multipart video | native operation authorization; **form prompt scanned here**, then as chat | model Ask |

Popup order today where both apply: Redact/Ask — Secret Scan Ask (boundary) before
the auto-router / model Ask (handler). Notify on chat-family routes — model Ask
before the Secret Scan notification (`run_turn_pipeline` stage 4). On the content
routes that run the turn pipeline (rerank, search, OCR, video, count_tokens) Notify
notified at the boundary and again in stage 4 (bug 1).

## Design (chosen)

**Access pre-check in the boundary, through the handlers' own access functions.**

New `routes/access.rs` holds every content-independent refusal as a function, and
the handlers call those functions in place of their inline checks:

- `client_llm_access` (client found/enabled, LLM gateway mode),
- `normalized_model` (strategy auto alias) and `strategy_model_access`
  (strategy permission + model allow-list),
- `chat_model_access` (the non-interactive part of `apply_model_access_checks`),
- `systemone_model_access` (the non-interactive part of systemone's checks),
- `image_access` (moved from `images.rs`),
- `rate_limit` (the pre-flight API-key check; all three handler copies built the
  same error and the limiter ignores the estimate).

`access::precheck(state, auth, path, body)` lists, per route, the same functions in
the handler's order. The boundary calls it after parsing the body and before any
scan, popup or monitor event, whenever it would scan (inference routes in
Redact/Ask, content routes unless Off). A refusal is answered by the boundary and
the handler never runs; it is recorded as the handler would (rate-limit event,
LLM-call event with the error) but with the body left out (`{"model": …}`), since
it was not scanned. Route authorizations that handlers answer before recording a
call (native operations, ranked search, model resolution) record nothing extra,
as in the handlers.

Why not defer only the Ask popup: in Ask the masked request depends on the answer
(allow sends the value, replace masks it, deny refuses), so detection-and-masking
cannot finish at the boundary without the answer, and the handlers record the
request in their LLM-call event before their access checks. Deferring would mean
forwarding provisionally masked content and unmasking later inside every handler.
The pre-check keeps masking where it is and keeps one definition of each rule.

Interactive approvals (auto-router popup, model Ask) stay in the handlers, after
the scan in Redact/Ask (today's order). Within the handlers, refusals now come
before those popups: `apply_model_access_checks` and systemone's checks run their
non-interactive checks first and re-check a model chosen in the auto-router popup.

Notify on the content routes whose handler runs the turn pipeline (rerank, search,
OCR, video, count_tokens) is left to the pipeline (stage 4, after access and model
Ask), as on chat; multipart video scans its form only in Redact/Ask for the same
reason. This removes the second notification.

Multipart video runs the same pre-check (`access::video_admission`) before
`scan_form_fields`.

Not moved: dispatch-time admission (router reservations, image `admit_direct_request`)
reserves capacity and would hold it across a user prompt; it stays at dispatch.
Validation (400s) still follows the boundary scan where it did.

## Checklist

- [x] `routes/access.rs`: step functions, `precheck`, `Refusal::record`
- [x] Handlers use the step functions: chat pipeline (`apply_model_access_checks`,
      rate limit), completions, embeddings, speech, transcriptions/translations,
      moderations, images, systemone/decisions
- [x] Boundary calls `precheck` before scanning; Notify left to the pipeline on
      pipeline content routes
- [x] Multipart video: pre-check before the form scan; form scan only in Redact/Ask
- [x] Tests (see Test Coverage Review)
- [x] Plan Review
- [x] Test Coverage Review
- [x] Bug Hunt
- [x] fmt, clippy, tests; commit and push

## As built

Per route (the boundary runs `access::precheck`, `secret_placeholders.rs` `admit`,
before any scan whenever it would scan; the handler calls the same functions):

| Route | Pre-check (in order) |
|---|---|
| chat, responses | `check_auxiliaries`, `chat_model_access` (if a model is named), `rate_limit` (not on duplicate hops) |
| completions | `client_llm_access`, then as chat |
| messages, count_tokens | `resolve_messages_model` for a bare name, then as chat |
| systemone, decisions | `client_llm_access`, `systemone_model_access`, `rate_limit` |
| embeddings | `client_llm_access`, `strategy_model_access`, `rate_limit` |
| speech | `client_llm_access`, `strategy_model_access` |
| image generation | `image_access` |
| moderations | `client_llm_access` |
| rerank, OCR, JSON video | `authorize_native_operation`, then as chat |
| search | `authorize_ranked_search` (default model), then as chat |
| multipart video (handler) | `video_admission` (as chat) before `scan_form_fields` |

- `pipeline::apply_model_access_checks` normalizes the model, runs
  `chat_model_access` (refusals), then the auto-router popup (a model picked there
  is checked again), then the model firewall. `check_rate_limits` delegates to
  `access::rate_limit`; the embeddings and systemone copies are removed.
- systemone `apply_access_checks`: alias, `systemone_model_access`, auto-router
  popup (re-check after edits), model firewall.
- `chat_model_access`, `strategy_model_access`, `systemone_model_access` and
  `image_access` refuse a missing client or strategy (`get_client_with_strategy`
  errors) instead of skipping the strategy checks; the router refused those
  requests at dispatch anyway (`validate_client_and_strategy`).
- Boundary refusals: `Refusal::record` emits the LLM call event (body
  `{"model": …}`, the error captured) and, for rate limits, the rate-limit event;
  route authorizations and model resolution record nothing more, as in the
  handlers. Status codes and bodies are the handlers' (same functions), so the
  OpenAPI responses are unchanged.
- `content_scan::scanned_in_turn_pipeline`: rerank, search, OCR, video,
  count_tokens. In Notify the boundary does not scan these; the pipeline does.

Popup order: Redact/Ask — Secret Scan Ask (boundary), then auto-router / model Ask
(handler), on every route. Notify — auto-router / model Ask, then the
notification (pipeline stage 4), on chat-family and pipeline content routes; the
other content routes have no model popups.

### Plan Review

All checklist items are implemented. Not done, by design:

- Validation (400/422) still follows the boundary scan: a malformed request in Ask
  can open a popup before the handler rejects it. Validation is per route and
  typed; it is not an access rule. Requests that are both invalid and refused now
  get the refusal (the boundary runs before validation), where the handler
  validated first on some routes.
- Dispatch-time admission (router reservations, image `admit_direct_request`) stays
  at dispatch; it holds capacity and would hold it across a user prompt.
- OCR's opaque-document policy (`enforce_opaque_scan_policy`) is not in the
  pre-check: it emits a warning as a side effect and is a scan policy, not access.
- The moderation endpoint's "disabled" / "no models" 503s are not in the pre-check
  (configuration of the endpoint, not access).
- "Strategy missing" cannot be configured (config validation refuses a client
  whose strategy does not exist), so it is not in the test matrix; the code path
  refuses it.

### Test Coverage Review

`secret_placeholders_access_tests.rs` (boundary with recording handlers unless noted):

- `refused_requests_open_no_popup_and_record_no_secret`: every covered route
  (chat, completions, responses, messages, systemone, embeddings, moderations,
  rerank, search, OCR, image generation, JSON video, count_tokens, speech) × client
  disabled, LLM Off, model not allowed, rate limited (where the route checks it) ×
  Ask, Notify (content routes), Redact: a 4xx (429 for rate limits) that is not a
  scan refusal, nothing forwarded, no Secret Scan popup, no monitor event with the
  secret. Fails (waits for the popup) with the pre-check removed (checked).
- `a_refusal_is_recorded_without_the_request_content`: LLM call event with the
  endpoint and error; rate-limit event; no secret.
- `admitted_requests_are_still_asked_and_masked`: every route, Ask asks and a
  denial stops the request; Redact forwards no secret.
- `the_secret_scan_popup_comes_before_the_model_popup` (real chat handler): Secret
  Scan Ask first, then the model Ask showing the masked request; deny → 403.
- `the_handler_refuses_before_the_auto_router_popup` (real chat handler, Notify):
  LLM Off with auto-router Ask → 403 with no popup.
- `notify_on_a_pipeline_content_route_notifies_once` (real count_tokens handler):
  one notification and one Secret Scan event (2 before the fix, checked).
- `a_refused_multipart_video_opens_no_popup` (real video handler, Ask and Notify).
- `the_model_firewall_reads_the_clients_strategy` (real chat handler, bug 2).
- `secret_placeholders_content_tests.rs`: setup enables native operations and web
  search; Notify expects the boundary notification only on non-pipeline routes.

### Bug Hunt

1. (Fixed) Notify on rerank, search, OCR, video and count_tokens notified twice and
   recorded two Secret Scan events: the boundary scanned without a secret context,
   so the pipeline's stage-4 scan ran again. Multipart video did the same
   (`scan_form_fields` in Notify, then stage 4). Regression test above.
2. (Fixed) The model firewall (`check_model_firewall_for`) read the permissions of
   the strategy whose `parent` is the client, not the client's `strategy_id`. For a
   client on a strategy it does not own, the firewall used empty permissions
   (global Off) and denied every model the strategy allowed, after the Secret Scan
   popup. It now reads the client's strategy, as `validate_strategy_model_access`
   and the router do.
3. (Fixed) Model access refusals ran after the auto-router popup on chat and
   systemone (strategy permission, model allow-list, LLM access), so a client
   without LLM access was first asked to approve auto routing.
4. (Fixed) Embeddings, speech, transcription/translation, image routes and System
   One skipped every strategy check when the client's strategy could not be read,
   and the handlers emitted the `strategy_not_found` event twice per request.

Checked without finding bugs: `Refusal::record` inside the boundary's monitor
scope (same request session); refusals of Messages routes are converted to the
Anthropic error shape by the auth layer, outside the boundary; duplicate hops
(rate limit skipped as in the pipeline); internal Try It Out client (synthetic
client passes `client_llm_access`, as the explicit skips did).

### Validation

Rustup stable toolchain, shared target dir, `LOCALROUTER_SKIP_CATALOG_FETCH=1`.

- `cargo fmt --all -- --check` — clean
- `cargo clippy --workspace --all-targets -- -D warnings` (aarch64-apple-darwin) —
  clean (the existing `unused dependency` manifest notes for `lr-guardrails` and
  `lr-monitor`)
- `cargo test -p lr-server` — 371 passed (doc tests: 4 ignored)
- `cargo test -p localrouter --test image_edits_endpoint_tests --test
  audio_endpoint_tests --test unified_api_tests --test openapi_tests` — 1, 32, 14, 13
  passed
- `cargo test -p localrouter --test completions_endpoint_tests --test
  permission_inheritance_tests --test route_helpers_tests --test
  router_strategy_tests --test stream_usage_endpoint_tests --test
  systemone_endpoint_tests` — 9, 48, 14, 39, 3, 110 passed
- Not run: Linux and Windows cross-target clippy (no platform-specific code added).
