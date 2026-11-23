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

- [ ] `routes/access.rs`: step functions, `precheck`, `Refusal::record`
- [ ] Handlers use the step functions: chat pipeline (`apply_model_access_checks`,
      rate limit), completions, embeddings, speech, transcriptions/translations,
      moderations, images, systemone/decisions
- [ ] Boundary calls `precheck` before scanning; Notify left to the pipeline on
      pipeline content routes
- [ ] Multipart video: pre-check before the form scan; form scan only in Redact/Ask
- [ ] Tests: per covered route, refused (access, model, rate limit) in Ask and
      Notify: no popup, no unscanned secret in the monitor, nothing forwarded;
      admitted still masked/asked; popup order (secret Ask before model Ask);
      single notification on pipeline content routes
- [ ] Plan Review
- [ ] Test Coverage Review
- [ ] Bug Hunt
- [ ] fmt, clippy, tests; commit and push
