# Proxy admission reservations

Date: 2026-11-23
Branch: `feat/proxy-admission-reservations`

Sources: "Left for follow-up" of
`plan/2026-11-21-ROUTER_ADMISSION_FAILOVER_TESTS.md` and the `AdmissionHandoff`
seam of `plan/2026-11-22-ROUTER_OAUTH_PROVIDER_FOLLOWUPS.md`.

## Findings (verified in the code)

- `AppFirewall` (`src-tauri/src/launcher/proxy.rs`) checks strategy limits in
  `body_policy` (header time), `open_stream` and `evaluate_policy`, counting
  the gateway's in-flight reservations, but never reserves. Concurrent proxied
  requests all see the same room.
- The proxy never scopes `lr_types::OUTBOUND_TRACE`, so
  `RateLimiterManager::strategy_in_flight` does not know a proxied request is
  a duplicate hop: a request that loops through the native server and then the
  proxy (secrets protected, so the firewall sees the duplicate) is counted
  against its own first hop's reservation.
- Proxied usage reaches the limits only through the metrics rows the passive
  recorder writes in `PassiveInterceptor::finalize` (spawned after the
  response ends). Client (API key) limiters are never charged for proxied
  traffic.
- WebSocket cycles are built from a per-connection template exchange
  (`WsSession::base.clone()`), so anything attached to the exchange must be
  per cycle.

## Design

- `lr-proxy` gets an opaque per-exchange admission slot,
  `interceptor::ExchangeAdmission` (shared by clones of one exchange), holding
  a `Box<dyn AdmissionGuard>`. `ObservedExchange` and `ReverseExchange` carry
  it; `FirewallRequest` / `StreamRequest` carry a handle to it and the
  request trace.
- `ActiveInterceptor::on_request` / `open_stream` release whatever the slot
  holds before asking the firewall (one reservation per exchange: a new
  decision replaces an earlier one, e.g. a stream offer that fell back to the
  buffered path) and release it again when the answer is a refusal (or, for
  `open_stream`, anything but a stream).
- `AppFirewall` keeps the non-reserving header-time check (`body_policy`) and
  the early checks before secret scan / GuardRails / approval popups, so a
  limited client is refused before any popup. The reservation is taken once,
  atomically, when the decision is final: at the end of `evaluate_policy`
  (when it admits) and right before `open_stream` returns a stream. It
  reserves the client's limiters (`reserve_api_key`, only when configured)
  and the strategy's (`reserve_strategy`), with the estimate the proxy check
  already uses (one request, the strategy's average tokens and cost), inside
  `lr_types::with_outbound_trace(trace)` so a duplicate hop is admitted
  without counting or holding in-flight shares, exactly as on the native path.
  Every early check ignores in-flight shares on a duplicate hop too.
- The guard (`ProxyAdmission`) lives in the slot until the recorder writes the
  exchange's metrics row: `PassiveInterceptor::finalize` settles it with the
  recorded usage (charges the client limiter for billed usage, then drops
  both reservations). Refusal, abort, client disconnect, upstream failure and
  panic all end with the slot dropped (every clone of the exchange gone) or
  settled by the failure record.
- WebSocket: each client message gets a fresh slot; the cycle carries it to
  `record_cycle`. Reverse proxy: `observed_from_reverse` shares the slot.
- Retrievals / cancels keep skipping limits (no reservation).

## Checklist

- [x] Plan saved
- [x] `ExchangeAdmission` / `AdmissionGuard` / `RecordedUsage` in lr-proxy;
      fields on `ObservedExchange`, `ReverseExchange`, `FirewallRequest`,
      `StreamRequest`
- [x] `ActiveInterceptor` releases on new decisions and refusals
- [x] WebSocket per-cycle slot; reverse proxy shares the slot
- [x] `PassiveInterceptor::finalize` settles the slot
- [x] `AppFirewall`: duplicate-hop aware checks; reserve at final decision
      (`evaluate_policy`, `open_stream`); client limiter reserve + charge
- [x] Tests: concurrent N vs K (MITM buffered, streamed, WS, reverse);
      release on refusal / abort / disconnect / upstream failure; reconcile;
      no double count with the native path
- [x] Bug hunt in proxy usage recording / metrics / cost
- [x] Plan Review
- [x] Test Coverage Review
- [x] Bug Hunt (final pass over the diff)
- [x] fmt, clippy, targeted tests
- [x] Commit and push

## Plan Review

- Every design item is implemented: the slot (`interceptor.rs`), release on
  new decisions / refusals / non-stream opens and after recording
  (`active.rs`), per-cycle slots (`websocket.rs`), shared slot on the
  reverse proxy (`reverse.rs`, `observed_from_reverse`), settlement in
  `PassiveInterceptor::finalize`, and in `AppFirewall` the duplicate-hop
  aware checks plus one reservation at the final decision of
  `evaluate_policy` and `open_stream` (client limiter + strategy).
- The client limiter is reserved only when limiters are configured for the
  client (production configures none today, so in practice the strategy
  share is what is held). It is checked at decision time, not at header
  time: `body_policy` is synchronous and the client check reads the batch
  ledger asynchronously. The header-time refusal stays a subset of what
  `evaluate` refuses.

## Test Coverage Review

- lr-proxy `active::admission_tests`: share kept until recorded, replaced
  (not added) by a second decision, released on refusal, on drop of every
  clone, on non-stream opens; duplicate hops never reach the firewall.
- lr-proxy `passive::admission_tests`: settlement with the whole prompt;
  Anthropic cache tokens counted and priced apart; OpenAI cached tokens not
  priced twice (and clamped); the ledger counts cached tokens once; local
  refusals write no metrics row or ledger entry; upstream failures still do;
  interrupted 2xx streams record the usage they reported; duplicate hops
  settle unbilled; reverse exchanges share the slot.
- lr-monitoring: the minute row a window starts in is counted.
- App `launcher::proxy::admission_tests` (real AppFirewall): 12 concurrent
  `evaluate` vs 3, 8 concurrent `open_stream` vs 2; header-time refusal
  matches the reservations; retrievals reserve nothing; duplicate hops both
  ways with the native `admit_direct_request`; client limiter reserve,
  charge on billed settle, release on unbilled settle; end to end through
  the MITM proxy (8 buffered vs 3, 3 streamed 17 MiB bodies vs 1, forged
  trace header still limited, client hang-up, upstream 500, unreachable
  upstream), WebSocket relay (5 connections vs 2, 3 messages on one
  connection vs 2, connection drop releases), reverse proxy (6 vs 2).
  Each concurrency test also checks the metrics rows equal the admitted
  count after release (no gap, refusals not counted). A mutation that drops
  the reservation instead of keeping it fails 11 of the 13 tests.

## Bug Hunt

Fixed (with regression tests):

1. Duplicate hops were counted against their own first hop's in-flight
   share in the proxy's checks (`body_policy`, `open_stream`,
   `evaluate_policy`): the proxy never scopes the request trace, so
   `strategy_in_flight` could not tell. Now passed explicitly.
2. OpenAI-style usage (`prompt_tokens` / `input_tokens` include
   `cached_tokens`) was priced as uncached input plus cache reads: cached
   tokens were charged twice in proxy cost.
3. Anthropic cache read/write tokens were left out of the proxy's metrics
   rows (strategy token limits and dashboards undercounted next to the
   native path, which records the whole prompt).
4. The usage ledger counted OpenAI cached tokens twice (its total adds
   `input + cache_read + cache_write`).
5. Firewall refusals (including the 429 itself) and unreadable bodies were
   written as failed requests into the strategy's metrics, so a client
   retrying at its request limit kept the window full and never recovered.
   Requests LocalRouter answers itself are no longer requests against any
   limit (still shown in the monitor).
6. A successful response cut off after it reported usage (client hung up
   mid-stream) recorded a failure with no tokens or cost; it now records
   the usage it reported, as the native path does for dropped streams.
7. Strategy windows read per-minute rows stamped with the minute's start,
   so usage inside the window could be missed (a burst at hh:mm:59 was
   invisible a second later to a one-minute limit). The window now starts
   at the beginning of its first minute (conservative).

Known limitations (left as they are):

- Trace headers on the wire never mark a duplicate hop (by design: they are
  untrusted). A request that really loops through the native server and the
  proxy is therefore enforced and counted by both hops; only a trusted
  in-process handoff (none exists yet) would let the second hop skip it.
- The metrics row of a proxied exchange is keyed by the strategy resolved
  when the tunnel was opened; the reservation by the strategy at decision
  time. If a client's strategy changes while a tunnel stays open, the two
  differ for that tunnel's requests.
- Proxy refusals carry no Retry-After header.
- The native path still writes a failed-request row for a request its
  router refuses with a rate-limit error (finding 5 on the native side).
- The native usage ledger records `input_tokens` including cached tokens
  next to `cache_read_tokens` (finding 4 on the native side).
