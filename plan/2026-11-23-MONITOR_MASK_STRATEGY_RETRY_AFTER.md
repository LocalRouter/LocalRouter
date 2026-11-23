# Monitor masking of refused requests; Retry-After for strategy limits

Date: 2026-11-23
Branch: `fix/monitor-mask-strategy-retry-after`

Sources: "Final cleanup as built" of `plan/2026-11-16-STREAMING_REQUEST_PIPELINE.md`
(remaining: strategy-limit Retry-After) and the known privacy gap that a WebSocket
message refused at stream open is stored in the monitor unmasked.

## Findings (verified in the code before changing it)

### 1. Request content kept by the monitor on refusal / early-exit paths

On the success path the monitor keeps what was forwarded: the `Protect` body (masked
for Redact/Ask), the original body for Off/Notify, and for a streamed body the head of
the forwarded (masked) bytes. On paths where LocalRouter refuses a request itself
(`refused_locally`), nothing is forwarded and the firewall's masking never ran:

| Path | Where | What the monitor kept |
|------|-------|-----------------------|
| MITM HTTP, head refusal (`BodyPolicy::Reject`) | `transport.rs` `BodyPolicy::Reject` → `tap::drain` | raw head (≤ 256 KiB) **unmasked** |
| MITM HTTP, stream refused at open | `stream_prepare` `Prepared::Rejected` → `intake.raw_head` | raw head **unmasked** |
| MITM HTTP, buffered/held refusal (rate limit, model not permitted, GuardRails, Secret Scan Ask denial, uninspectable) | `transport.rs` `reject` after `on_request` | whole decoded body **unmasked** (raw text, parsed display, conversation title/hashes) |
| MITM HTTP, rewritten pass-through refused | `transport.rs` | pass-through head (raw) |
| MITM HTTP, streamed body refused before connecting | `transport.rs` `stream_ready` | head of forwarded output (already masked / tee original) |
| MITM HTTP, body read failure | `body_read_failed` | nothing |
| WebSocket held message refused | `websocket.rs` `on_client_message` | whole message **unmasked** |
| WebSocket message refused at stream open (known issue) | `open_message_stream` | raw head **unmasked** |
| WebSocket pass-through message refused | `open_pass` | raw head; also **not marked `refused_locally`** (counted in metrics) |
| WebSocket streamed message stopped | `stop_streamed` | head of forwarded (masked) bytes — as success |
| Reverse proxy: the same head/stream/buffered refusals | `reverse.rs` `reject` | as MITM: **unmasked** |
| Reverse proxy, streamed body refused before connecting / body read failure | `aborted`, `body_read_failed` | masked output / nothing; **not marked `refused_locally`** (counted as requests) |
| Upstream failures after acceptance | `fail_pending*`, `fail` | what was forwarded — as success |

Error text on these paths is LocalRouter's own (`StreamAbort` messages, hyper errors);
refusal bodies quote at most the model name or a GuardRails message.

### 2. Strategy-limit 429s

Native router: `strategy_limits_have_room` returns `AppError::RateLimitExceeded`
(429 without Retry-After). Auto-routing loops treat a strategy denial as a per-model
failure with a fixed 60 s and end in a 502 "All auto-routing models failed" although
the limit is the strategy's and the same for every candidate. Proxy:
`rate_limit_exceeded` (a copy of the router check) answers `reject_json(429)` without
`retry_after` (head policy, `open_stream`, `judge_policy`, `reserve`).

Strategy usage is the metrics rows `llm_strategy:<id>` (per minute, stamped at the
minute's start; hourly/daily roll-ups suppress the rows they cover). The check counts
rows stamped at or after the minute containing `now - window`.

## Decisions

1. **One masking hook, applied centrally.** `Firewall::mask_for_monitor(client_id,
   &mut Vec<u8>) -> bool` (default: unchanged). `ActiveInterceptor` applies it to every
   exchange recorded with `refused_locally` (MITM `on_response`, WebSocket cycles,
   reverse `record`): request body, request capture head, refusal body and error text.
   A capture that does not hold the whole body is cut before its last JSON string (a
   secret cut by the capture cap cannot be detected). When masking fails the content is
   replaced by a length-only marker (`BoundedCapture::withheld`).
   `AppFirewall` masks like the success path would for the client's Secret Scan action:
   Off → unchanged (Off forwards and records the original); Notify/Redact/Ask → every
   scanner finding (strings of the body and of decoded text attachments) and every
   original the client's vault protects. For Notify and for findings an Ask/Redact
   decision would have let through this masks more than a forwarded request would show;
   the request was never sent and no decision was taken, so the safe side is chosen.
   Scanner or vault unavailable → withheld.
2. **Retry-After from window expiry.** `MetricsCollector::strategy_room_after` replays
   the check's query over the rows of the window at each future instant a row stops
   counting (row stamp + 60 s + window, including roll-up suppression) and returns the
   first instant at which recorded usage plus in-flight plus the new request fits.
   In-flight reservations are counted as usage recorded now (a request that finishes
   becomes a metrics row that stays for a whole window), so they free at the end of a
   window from now, not at their completion; if nothing fits before that, the answer is
   that full window (+ the minute rounding). This is the client limiter's choice.
   Every denied limit is evaluated; the answer is the latest.
3. Shared check in `lr_router::strategy_limits` used by the router and the app proxy
   (removes the duplicated proxy copy). Router error: new
   `AppError::StrategyRateLimited { limit, window, retry_after_secs }` → 429,
   `Retry-After`, `code: strategy_rate_limit_exceeded`, extension
   `localrouter.rate_limit`. Proxy: `RequestAction::rate_limited` with the same seconds;
   its JSON error carries `retry_after_secs` and the code.
4. Auto-routing returns the strategy denial at once (429) instead of trying every model.

## Checklist

- [x] Plan saved and committed
- [x] 2 Retry-After: lr-monitoring replay + tests (minute, hour, daily, roll-ups)
- [x] 2 lr-router shared check + AppError variant + auto-routing + tests
- [x] 2 lr-server mapping (header, code, extension) + test
- [x] 2 proxy: head/open_stream/judge/reserve use the shared check + Retry-After + tests
- [x] 1 Firewall hook + ActiveInterceptor masking on refused exchanges + marker
- [x] 1 AppFirewall::mask_for_monitor + tests (Off/Notify/Ask/Redact, attachments, no scanner)
- [x] 1 Path tests: WS refused at open / held, refused captures, reverse record
- [x] 3 Bug hunt fixes with regression tests (listed below)
- [x] Plan Review
- [x] Test Coverage Review
- [x] Bug Hunt (final pass)
- [x] fmt, clippy, targeted tests
- [x] Commit and push

## As built

- `lr_monitoring::storage::{UsageRow, sum_usage_rows}`, `MetricsDatabase::get_usage_rows`
  (parity-tested against the SQL query over minute rows and hourly/daily roll-ups);
  `MetricsCollector::strategy_retry_after` (bisection over the instants rows leave the
  window; usage never grows as rows leave).
- `lr_router::strategy_limits::{check, StrategyLimitDenial}`; the router's
  `strategy_limits_have_room` and the app proxy's `rate_limit_exceeded` both use it.
  `AppError::StrategyRateLimited` (new variant; nothing renamed) → 429, `Retry-After`,
  `code: strategy_rate_limit_exceeded`, extension `localrouter.rate_limit`
  (`scope`, `limit`, `window`, `retry_after_secs`); kept through `RoutingFailure` and the
  Anthropic Messages error conversion (headers are kept there).
- Auto-routing (chat, streaming chat, System One) checks the strategy limits once before
  selecting candidates and answers a full limit with the 429; the per-attempt reservation
  of every auto loop (chat, stream, embeddings, System One) returns the denial instead of
  trying the next model. Candidate explanations carry the strategy `retry_after_secs`.
- Proxy: `BodyPolicy::RateLimited { message, code, retry_after }` (new variant) for the
  head-time refusal; `RequestAction::rate_limited_with_code`; the JSON error carries
  `code` and `retry_after_secs`. `BodyPolicy::refusal()` builds the answer of both
  refusing policies.
- `Firewall::mask_for_monitor`, applied by `ActiveInterceptor` to every
  `refused_locally` exchange (MITM `on_response`, WebSocket cycles, reverse `record`) off
  the runtime (`spawn_blocking`); `BoundedCapture::withheld` and its raw-text marker.
  `AppFirewall::mask_for_monitor` / `mask_monitor_content` (JSON string by string with
  decoded text attachments, otherwise as one text; then the vault's `redact_monitor`).

## Bugs found and fixed
- WebSocket pass-through message refusal (`open_pass`) was not marked `refused_locally`
  (counted as a request in the metrics; content not treated as a refusal).
  Test: `websocket_leftover_tests::pass_through_refusals_are_answered_and_rewrites_refused`.
- Reverse proxy: a streamed body refused before connecting (`aborted` from the ready
  path) and an unreadable body (`body_read_failed`) were not marked `refused_locally`
  (the MITM proxy marks both). Test: `reverse_e2e::unreadable_reverse_bodies_are_recorded_as_local_refusals`.
- Auto-routing turned a strategy-limit denial into a 502 "All auto-routing models
  failed" (pre-filtered as `strategy_rate_limit` for every candidate, or a fixed 60 s
  per attempt). Test: `failover_tests::a_full_daily_strategy_limit_answers_when_it_frees`.
- `passive::raw_capture` computed the truncated byte count with a plain subtraction
  that underflows when a (masked) head is longer than the bytes it stands for.
- Every refusal path listed in Findings stored unmasked request content (item 1).

## Known limitations
- Known vault originals inside base64 text attachments of a refused request are masked
  only where the scanner also detects them (`redact_monitor` works on JSON strings).
- Monitor masking of a refusal masks more than a forwarded request would show for
  Notify and for findings an Ask/Redact decision would have let through (by decision).
- Token estimates of in-flight requests (prompt averages) can be above their real
  usage, so a Retry-After driven by in-flight usage can be later than needed.
- The OpenAPI path annotations in `routes/` do not declare the `Retry-After` header
  (it was already sent on client-limit 429s; `routes/` is being changed concurrently).
