# Native accounting fixes and authenticated trace provenance

Date: 2026-11-23
Branch: `fix/native-accounting-trace-provenance`

Sources: "Known limitations" of `plan/2026-11-23-PROXY_ADMISSION_RESERVATIONS.md`
and the trace-provenance follow-up of `plan/review-2026-10-01/foundations.md`.

## Findings (verified in the code)

1. Native local refusals are counted as failed requests. Every route
   (`routes/{chat,completions,embeddings,audio,systemone}.rs`) calls
   `MetricsCollector::record_failure` for any router error, including the
   router's own refusals (`Router::admit_client` / `reserve_strategy_rate_limits`
   return `RateLimitExceeded`, validation errors, `Unauthorized`). The row is
   written to `llm_strategy:<id>` and counts against the strategy's request
   limit, so a client retrying at its limit keeps its window full. Provider
   429s share the same error variant, so the error alone cannot tell a local
   refusal from an upstream failure.
2. The native ledger (`routes/finalize.rs::upstream_ledger_entry`) records
   `input_tokens = prompt_tokens`, which on the native path is the whole
   prompt for every provider (OpenAI reports `prompt_tokens` including
   `cached_tokens`; `anthropic::token_usage` sums `input + cache_read +
   cache_write` into `prompt_tokens`), next to `cache_read_tokens` and
   `cache_write_tokens`. `LedgerEntry::total_tokens` adds all three, so
   cached tokens are counted twice. Cost (`PricingInfo::input_cost`) already
   treats reads and writes as parts of `prompt_tokens` (clamped) and is
   correct; metrics rows record the whole prompt (correct).
3. `RequestTrace::outbound_for` always yields hop 1, so a request crossing
   the native server and the proxy is enforced, prompted and counted by both.

## Design

### Local refusals (item 1)

- `lr_types::upstream` adds a request-scoped dispatch probe (task-local
  `Option<Arc<AtomicBool>>`): `with_upstream_probe(fut)`,
  `mark_upstream_dispatch()`, `refused_locally()` (probe in scope and nothing
  dispatched). `spawn_traced` carries it like the trace.
- The server's trace middleware scopes a fresh probe per request.
- Marked where a provider is called: `adaptive::reserve_inference_dispatch`
  (every chat / stream / System One dispatch), the embedding / audio executors,
  `NativeDispatch` / `SearchDispatch`, and the shared HTTP client middleware
  (any provider HTTP request).
- `MetricsCollector::record_failure` skips the row when `refused_locally()`.
  Monitor events and the access log still record the refusal. Without a probe
  in scope (proxy recorder, background tasks) behaviour is unchanged.

### Ledger (item 2)

`upstream_ledger_entry` writes uncached input: `prompt - read - write`,
clamped like `PricingInfo::input_cost`; reads and writes are recorded apart.

### Signed trace headers (item 3)

- Wire format (backwards compatible, extra `;key=value` parts are ignored by
  older parsers): `<trace_id>;hop=<n>;ts=<unix secs>;nonce=<32 hex>;sig=<64 hex>`.
- `sig = HMAC-SHA256(key, "lr-trace-v1\n<trace_id>\n<hop>\n<ts>\n<nonce>")`.
- Key: 32 random bytes per install, stored hex-encoded in the app keychain
  (`LocalRouter-TraceSigning` / `install`) through the existing
  `KeychainStorage` abstraction, loaded or created at startup in `main.rs`
  (`lr_api_keys::trace_signing::load_or_create`), installed once into
  `lr_types` (`install_trace_signing_key`). If the keychain fails, an
  ephemeral in-memory key is used (both hops live in this process). The key
  never leaves the process; only MACs are sent.
- Emission: `RequestTrace::signed_header_value()` is used wherever LocalRouter
  stamps an outgoing header (shared provider HTTP client, realtime upstream,
  proxy `stamp_trace`). Without an installed key it is the plain header.
- Acceptance: `RequestTrace::from_inbound_header` verifies MAC (constant
  time), freshness (at most 60 s old, at most 5 s in the future) and a
  single-use nonce (in-memory replay cache, entries expire with the window,
  bounded; a full cache rejects). Verified → `inbound.next_hop()` (duplicate
  hop); anything else → `outbound_for` (hop 1, untrusted, exactly as today).
- Native server: `resolve_outbound_trace` uses it, so the existing duplicate
  hop paths (no reservation, no metrics/ledger row, no firewall/guardrails
  popups, passthrough) apply to verified hops only.
- Proxy: `lr_proxy::stamp_trace` uses it (two-line change); the exchange trace
  then carries hop > 1 only when verified, which `ActiveInterceptor`,
  `PassiveInterceptor` and `AppFirewall` already honour. `AppFirewall`'s
  policy for a verified duplicate hop that still runs (secrets protected)
  skips the guardrails and model approval popups the first hop already asked.

Residual risk: the MAC is not bound to the request (method / host / body). A
party that sees a signed header LocalRouter sent to a non-LocalRouter upstream
could replay it once, within 60 s, against a LocalRouter listener it can reach,
and still needs valid client credentials there; the replayed request would
skip limits and accounting. Binding to the destination host is not possible
across the reverse proxy (it rewrites `Host`).

## Checklist

- [x] Plan saved
- [x] Item 1: probe, marks, `record_failure` gate, tests
- [x] Item 2: ledger normalization, tests
- [x] Item 3: signing in lr-types, key storage, server + proxy wiring,
      AppFirewall duplicate-hop popups, tests (forged / stale / replayed
      untrusted; genuine recognized; native → proxy loop counted once)
- [x] Item 4: bug hunt in touched metrics / ledger / cost paths
- [x] Plan Review
- [x] Test Coverage Review
- [x] Bug Hunt (final pass over the diff)
- [x] fmt, clippy, targeted tests
- [x] Commit and push

## Plan Review

- Item 1: `lr_types::upstream` (probe), scoped in the server's trace
  middleware, carried by `spawn_traced`; marked in
  `adaptive::reserve_inference_dispatch`, the embedding / transcription /
  translation / speech executors, `NativeDispatch::new`, before
  `SearchDispatch`, and in the provider HTTP client middleware (not for
  usage polls). `MetricsCollector::record_failure` skips local refusals.
- Item 2: `finalize::upstream_ledger_entry` writes uncached input. Cost was
  verified correct on the native chat path (`PricingInfo::input_cost`).
- Item 3: as designed. The proxy edit is limited to `lr_proxy::stamp_trace`
  (inbound verification + signed outbound header); `AppFirewall`
  (`judge_policy`, `open_stream`) skips GuardRails and the model approval
  popup for a duplicate hop; secret scanning / masking still runs for it.

## Test Coverage Review

- lr-types: signed header readable by old parsers; genuine verifies once
  (replay, re-cased nonce rejected); other key, tampered hop / id / ts,
  made-up signature rejected; partial signatures unsigned; no key; stale and
  future timestamps; full replay cache fails safe and expired nonces are
  swept; `from_inbound_header` end to end; probe semantics incl. spawned tasks.
- lr-api-keys: key generated once and reused, unreadable key replaced,
  unavailable store falls back to an ephemeral key.
- lr-monitoring: local refusals write no row; dispatched failures and
  probe-less failures do.
- lr-router: strategy and client rate-limit refusals (direct and auto) write
  no failed-request row and keep the window from filling; provider 429 / 503
  / 400 still count. System One emulation cost prices cached tokens at the
  cache rate.
- lr-server: middleware trusts only a fresh, first-use signed header; ledger
  counts every prompt token once (OpenAI, Anthropic, clamped estimate).
- lr-providers: provider requests carry a header the next hop verifies, and
  mark the dispatch.
- App (`launcher::proxy::admission_tests::provenance_tests`): native → proxy
  loop through the real MITM proxy passes the share the native hop holds and
  writes no proxy metrics row, while replayed / stale / tampered / bad-MAC /
  unsigned / forged headers are refused 429; proxy → native loop admitted
  once, replay refused; a verified duplicate hop is not prompted again.

## Bug Hunt

Fixed (with regression tests):

1. (Item 1) Native local refusals were written as failed requests to the
   strategy metrics.
2. (Item 2) Native ledger counted cached prompt tokens twice.
3. System One emulation (`systemone::chat_cost`) priced every prompt token at
   the input rate: cached reads / writes, reported inside `prompt_tokens`,
   were not charged at their cache rates (overcharge vs `/v1/chat/completions`).
4. Found during development: the first replay-cache version swept the whole
   map on every insert once half full (quadratic); sweeps now run at most
   once a second.

Known limitations:

- The signature is not bound to the request (see the residual risk above).
- Streaming failures recorded while the response body is polled (outside the
  middleware scope) see no probe and still count, as before.
- Requests the native server answers without the router (images,
  moderations, passthroughs not using the shared HTTP client) are covered only
  where their failures call `record_failure` inside the probe scope after a
  marked dispatch; a provider that bypasses both the router marks and the
  shared HTTP client would have its failures treated as local refusals. All
  current `record_failure` call sites go through marked paths.
