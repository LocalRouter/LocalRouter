# Fix Free Tier Usage Tracking

## Context

Free tier usage (credits and rate limits) is never recorded after API requests. The `FreeTierManager` has `record_credit_usage()` and `record_rate_limit_usage()` methods, but they're never called from the request path. This means:
- Credit-based providers (e.g., Ollama overridden to CreditBased) show stale usage
- Rate-limited providers' local counters never increment
- Even with `free_tier_only: true` on a strategy, the credit check passes forever because usage stays at 0

## Files to Modify

1. **`crates/lr-router/src/free_tier.rs`** — Add `record_usage()` convenience method + comprehensive tests
2. **`crates/lr-router/src/lib.rs`** — Wire up recording in `execute_request()`, `execute_embedding_request()`, and `wrap_stream_with_usage_tracking()`

## Step 1: Add tests (TDD)

Add to `mod tests` in `free_tier.rs`:

- `test_record_usage_rate_limited_free` — records to rate tracker, not credit tracker
- `test_record_usage_credit_based` — records to credit tracker, not rate tracker
- `test_record_usage_free_models_only` — records to rate tracker (has max_rpm)
- `test_record_usage_always_free_local_is_noop` — no trackers touched
- `test_record_usage_subscription_is_noop` — no trackers touched
- `test_record_usage_none_is_noop` — no trackers touched
- `test_record_usage_accumulates_across_requests` — 3 requests, verify sums
- `test_record_usage_credit_exhaustion` — fill budget, verify `check_credit_balance` returns `!has_capacity`
- `test_record_usage_rate_limit_exhaustion` — hit RPM limit, verify `check_rate_limit_capacity` returns `!has_capacity`

## Step 2: Add `record_usage()` method to `FreeTierManager`

New public method that dispatches based on `FreeTierKind`:
- `RateLimitedFree` → `record_rate_limit_usage(provider, tokens)`
- `CreditBased` → `record_credit_usage(provider, cost_usd)`
- `FreeModelsOnly` → `record_rate_limit_usage(provider, tokens)`
- `AlwaysFreeLocal` / `Subscription` / `None` → no-op

## Step 3: Fix `execute_request()` (non-streaming)

After the existing `rate_limiter.record_api_key_usage()` call (~line 779), add 3 lines:

```rust
let free_tier = self.get_effective_free_tier(provider);
let total_tokens = usage.input_tokens + usage.output_tokens;
self.free_tier_manager.record_usage(provider, &free_tier, total_tokens, cost);
```

Cost and usage are already computed at this point.

## Step 4: Fix `execute_embedding_request()`

Same pattern — add recording after the rate_limiter call.

## Step 5: Fix `wrap_stream_with_usage_tracking()` (streaming)

Expand the function signature to accept `free_tier_manager: Arc<FreeTierManager>`, `free_tier: FreeTierKind`, and `pricing: PricingInfo`.

In the `then` closure (on stream end), after recording to rate_limiter:
- Compute `est_cost = calculate_cost(est_prompt, est_completion, &pricing)`
- Extract provider name from `resolved_model` (split on `/`)
- Call `free_tier_manager.record_usage(provider, &free_tier, tokens, est_cost)`

## Step 6: Update all 4 call sites of `wrap_stream_with_usage_tracking`

Each call site already has access to `provider_instance` — add `get_pricing()` and `get_effective_free_tier()` calls, pass the 3 new args:

1. `stream_complete_with_paid_fallback` (~line 423)
2. `stream_complete_with_auto_routing` (~line 1144)
3. `stream_complete` internal-test path (~line 1483)
4. `stream_complete` main path (~line 1572)

## Verification

```bash
cargo test -p lr-router && cargo clippy -p lr-router
```
