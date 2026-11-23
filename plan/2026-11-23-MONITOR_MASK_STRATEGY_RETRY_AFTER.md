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
- [ ] 2 Retry-After: lr-monitoring replay + tests (minute, hour, daily, roll-ups)
- [ ] 2 lr-router shared check + AppError variant + auto-routing + tests
- [ ] 2 lr-server mapping (header, code, extension) + test
- [ ] 2 proxy: head/open_stream/judge/reserve use the shared check + Retry-After + tests
- [ ] 1 Firewall hook + ActiveInterceptor masking on refused exchanges + marker
- [ ] 1 AppFirewall::mask_for_monitor + tests (Off/Notify/Redact, vault, attachments, no scanner)
- [ ] 1 Path tests: WS refused at open / held, MITM / reverse refusals never store the secret
- [ ] 3 Bug hunt fixes with regression tests (listed below)
- [ ] Plan Review
- [ ] Test Coverage Review
- [ ] Bug Hunt (final pass)
- [ ] fmt, clippy, targeted tests
- [ ] Commit and push

## Bugs found
- WebSocket pass-through message refusal (`open_pass`) not marked `refused_locally`.
- Reverse proxy `aborted` and `body_read_failed` not marked `refused_locally`.
- Auto-routing turned a strategy-limit denial into a 502 after trying every model.
