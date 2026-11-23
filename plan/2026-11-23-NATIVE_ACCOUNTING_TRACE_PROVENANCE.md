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

- [ ] Plan saved
- [ ] Item 1: probe, marks, `record_failure` gate, tests
- [ ] Item 2: ledger normalization, tests
- [ ] Item 3: signing in lr-types, key storage, server + proxy wiring,
      AppFirewall duplicate-hop popups, tests (forged / stale / replayed
      untrusted; genuine recognized; native → proxy loop counted once)
- [ ] Item 4: bug hunt in touched metrics / ledger / cost paths
- [ ] Plan Review
- [ ] Test Coverage Review
- [ ] Bug Hunt (final pass over the diff)
- [ ] fmt, clippy, targeted tests
- [ ] Commit and push
