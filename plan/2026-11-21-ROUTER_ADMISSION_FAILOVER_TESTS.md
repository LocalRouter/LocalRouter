# Router admission reservations and failover integration tests

Branch: `fix/router-admission-failover-tests`

Source: `plan/review-2026-10-01/api-routing-providers.md` ("Rate limiting and
concurrency" follow-ups) and the stale TODO in `crates/lr-router/src/lib.rs`.

## Findings from reading the code

- `RateLimiterManager` (`crates/lr-router/src/rate_limit.rs`) checks a limit
  and records usage in two separate steps; only native batches reserve (the
  durable `lr_batches` ledger, added to the check through
  `pending_batch_usage`). Concurrent ordinary requests all pass the check.
- Nothing in production configures per-client limiters
  (`add_api_key_limiters` is only called from tests). The limits a user
  configures are the strategy limits (`Strategy::rate_limits`), checked
  against recorded metrics by `Router::check_strategy_rate_limits` and by the
  copy in `src-tauri/src/launcher/proxy.rs`. That check has the same race:
  metrics are written when a request completes.
- The proxy firewall does not call `RateLimiterManager` at all.
- The server pipeline (`pipeline.rs` stage 3), `embeddings.rs` and
  `systemone.rs` call `check_api_key` before the router is invoked; the router
  checks again at its entry points. The router is the one place every ordinary
  request passes through with a lifetime that covers dispatch.
- `get_api_key_usage` / `get_router_usage` report the first configured window
  of a metric only. They have no production callers.

## Design

- One in-flight ledger in `RateLimiterManager`, in the same unit the batch
  ledger uses (`lr_batches::BatchUsage`, projected per metric with
  `BatchMetric::amount`). Admission is one equation for every source:
  recorded usage + in-flight ordinary reservations + durable batch
  reservations.
- `reserve_api_key` checks and reserves under one per-scope lock and returns an
  `AdmissionReservation` drop guard. `check_api_key` is the same check without
  reserving. `batch_budget_snapshot` counts in-flight reservations as ordinary
  usage so a batch reservation sees them.
- `reserve_strategy` does the same for a strategy's metrics-based limits.
- The router reserves once per request at its entry points and holds the guard
  until usage has been recorded: to the end of the call for non-streaming
  requests, inside the usage-tracking stream wrapper for streams (so a dropped
  stream releases after its partial usage is recorded). Failover attempts share
  the request's reservation.
- Sliding-window state moves to a synchronous lock so a guard can release in
  `Drop`.

## Checklist

- [x] Read review notes, rate limiter, router, pipeline, proxy firewall
- [x] In-flight ledger, `reserve_api_key`, `AdmissionReservation` guard
- [x] `reserve_strategy` and in-flight aware strategy checks
- [x] Router entry points hold the reservation; streams carry it
- [x] `get_*_usage_windows` reporting every configured window
- [x] Rate limiter tests: N concurrent vs limit K, release, reconcile, batch
- [x] Router tests: release on error, on cancelled stream, reconcile, failover
      does not double count
- [x] Failover integration tests with mock providers; remove the stale TODO
- [x] Plan Review: compare this plan with the implementation
- [x] Test Coverage Review: new and modified paths have tests
- [x] Bug Hunt: re-read the changes for races, leaks and wrong state
- [x] fmt, clippy, targeted tests
- [x] Commit and push the branch

## Plan Review

- Every Design item is implemented: `reserve_api_key` / `reserve_strategy`
  check and hold under one per-scope lock; `check_api_key`,
  `check_strategy_rate_limits`, `batch_budget_snapshot` and the proxy
  firewall count in-flight reservations; chat entry points (`complete`,
  `stream_complete`, both auto-routing loops) hold the client reservation for
  every failover attempt and take a strategy share per attempt, released when
  that attempt fails; streams release in `StreamUsageRecorder::finish`, after
  the actual usage is charged, including when the stream is dropped.
- Embeddings, audio, System One, token counting, native operations and search
  hold a request-only client reservation across their check and charge.
- `get_*_usage` keep their single-window signature (first window);
  `get_*_usage_windows` report every window plus what is reserved.

## Test Coverage Review

- `rate_limit.rs`: N=40 concurrent vs K=5 admits exactly 5; release on drop;
  replacement by recorded usage; token/cost estimates held; non-reserving
  check; unlimited keys; duplicate hop (client and strategy scope); every
  window reported; strategy reservation and `absorb`; batch + in-flight share
  one budget.
- `failover_tests.rs` (router through mock providers registered via a
  `ProviderFactory`): success first try, retryable failover order, non-retryable
  stop (upstream 400, invalid params), backed-off provider skipped, all-fail
  `RoutingExhausted`, all backing off, disallowed / unknown providers skipped,
  specific model not failed over, failing health check does not block, only
  unreachable providers marked unhealthy; streaming failover before the first
  chunk, none after output began, non-retryable and all-fail streams;
  admission: concurrent client and strategy limits, release on error /
  all-models-failed / task cancellation / stream dropped mid-flight / stream
  failing before first chunk, reconcile to actual usage, in-flight token
  estimates blocking, failover attempts sharing one reservation.

## Bug Hunt

- Fixed: a duplicate hop's strategy check counted its own first hop's
  reservation, so a strategy limit of one request blocked a request that
  loops through LocalRouter. `reserve_strategy` and `strategy_in_flight` now
  ignore in-flight reservations on a duplicate hop, as the client path does.
- Fixed: `complete_with_paid_fallback` / `stream_complete_with_paid_fallback`
  (the retry after the user accepts paid fallback) dispatched without any
  client rate-limit check. They now admit like every chat request; the
  stream carries the reservation. Test:
  `paid_fallback_retries_are_admitted_like_any_request`.
- Test corrections from the first run: auto-routing permissions come from
  `Strategy::model_permissions` (not the legacy `allowed_models`); auto
  routing streams end with a choice-less `localrouter.routing` trailer chunk
  and wrap mid-stream errors in `RoutingFailure`. The router behaved
  correctly; the tests' expectations were wrong.
- Transient over-count, accepted: between charging actual usage and dropping
  the reservation both count, so a concurrent request can be denied for that
  instant (conservative direction).
- In-flight scope maps keep one empty entry per client/strategy id ever seen;
  bounded by configuration.

## Left for follow-up

- The proxy firewall's strategy check (`src-tauri/src/launcher/proxy.rs`) is
  not reserved: an allowed exchange finishes inside `lr-proxy`, so a guard has
  to travel with the exchange.
- A strategy reservation is released when the router call (or stream) ends;
  the server writes the metrics row just after. Closing that gap needs the
  guard to reach `lr-server`'s finalize step.
- Embeddings, audio, System One, native operations and search keep the
  non-reserving strategy check (it does count in-flight chat reservations).
