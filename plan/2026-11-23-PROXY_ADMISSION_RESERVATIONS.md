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
- [ ] `ExchangeAdmission` / `AdmissionGuard` / `RecordedUsage` in lr-proxy;
      fields on `ObservedExchange`, `ReverseExchange`, `FirewallRequest`,
      `StreamRequest`
- [ ] `ActiveInterceptor` releases on new decisions and refusals
- [ ] WebSocket per-cycle slot; reverse proxy shares the slot
- [ ] `PassiveInterceptor::finalize` settles the slot
- [ ] `AppFirewall`: duplicate-hop aware checks; reserve at final decision
      (`evaluate_policy`, `open_stream`); client limiter reserve + charge
- [ ] Tests: concurrent N vs K (MITM buffered, streamed, WS, reverse);
      release on refusal / abort / disconnect / upstream failure; reconcile;
      no double count with the native path
- [ ] Bug hunt in proxy usage recording / metrics / cost
- [ ] Plan Review
- [ ] Test Coverage Review
- [ ] Bug Hunt (final pass over the diff)
- [ ] fmt, clippy, targeted tests
- [ ] Commit and push
