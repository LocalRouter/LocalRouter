# Auto-router priority skip not recorded (streaming)

## Problem

The streaming auto-router (`stream_complete_with_auto_routing` in
`crates/lr-router/src/lib.rs`) silently skips four pre-request filter
reasons without pushing to `attempts`:

- `is_in_backoff(...)` returns Some → backoff
- `classify_model(...)` returns `NotFree` → not_free
- `check_cost_backoff(...)` returns Some → cost_backoff
- `check_strategy_rate_limits(...)` returns Err → rate_limited

The non-streaming path (`complete_with_auto_routing`, ~80 lines above) does
the right thing: every `continue;` push is preceded by an
`attempts.push(...)` entry, so the routing UI's
`{total_attempts, attempts}` is complete. The streaming path was missed
during a copy-paste and never updated.

## Symptom

User reported "I have set two plans in place MiniMax Starter and MiniMax
Max. It should use the starter first, but seems to be sending to Max
despite Starter being the top priority." Screenshot of the routing tab
showed:

- 2 candidate models
- **1 attempt** across 2 candidate models
- MiniMax Max/MiniMax-M3 success

Real flow was: Starter returned a 429 earlier → `record_rate_limit_error`
pushed it into the in-memory backoff → on the next streaming request,
Starter was filtered at the `is_in_backoff` check with **no push to
`attempts`** → loop iterated 1 time → total_attempts = 1, attempts = [Max
success].

The routing UI was lying: it looked like only one candidate was even
considered, so the user couldn't see that Starter was deliberately
skipped (vs. just not being a candidate).

## Fix

Mirror the non-streaming path exactly. In
`crates/lr-router/src/lib.rs`, `stream_complete_with_auto_routing`,
add four `attempts.push(...)` calls before each `continue;` that is
missing one:

1. Backoff (after `is_in_backoff`): push `outcome: "backoff", error: &backoff.reason`
2. `not_free` (inside `if strategy.free_tier_only` branch): push
   `outcome: "not_free"`
3. `cost_backoff` (also inside the `free_tier_only` branch): push
   `outcome: "cost_backoff", error: format!("Cost backoff ({}s)", retry_secs)`
4. Strategy rate-limit (after `check_strategy_rate_limits`): push
   `outcome: "rate_limited", error: e.to_string()`

## Tests

Added `test_routing_metadata_records_all_skips_alongside_success` in
`crates/lr-router/src/lib.rs::tests` that exercises `build_routing_metadata`
with a backoff skip + success sequence and asserts the metadata correctly
records both attempts and links the success to attempt index 1. The
function is purely derived from its inputs, so this is a contract test
that future call sites must satisfy — not a mock-driven integration test.

Full mock-driven integration tests for the streaming auto-router are
out of scope: the existing `mod tests` has no mock provider infrastructure
(see TODO comment at line 3230), and standing one up would dominate this
fix. The structural test plus a live API re-test is sufficient given that
the change is a 4-line mechanical copy of an already-tested pattern.

## Verification

1. `cargo test -p lr-router --lib` passes (the new test plus existing 99).
2. `cargo test --workspace` — full workspace green.
3. `cargo fmt --all -- --check` clean.
4. `cargo clippy -p lr-router --all-targets -- -D warnings` clean
   (pre-existing `block v0.1.6` future-incompat warning is unrelated).
5. Live API re-test: send `model: "auto"` to `127.0.0.1:3625/v1/chat/completions`
   with the user's bearer token, then read the most recent routing event
   and assert attempts.length == 2 with Starter carrying a `backoff`
   outcome instead of the previous misleading 1-attempt display.

## What this does NOT fix

The underlying cause — Starter being in backoff — is independent. After
the fix, the UI will honestly show "Starter: backoff (60s), Max: success"
and the user can decide whether to wait out the backoff, clear it via a
CLI/API, or accept the fallback behaviour. This fix makes that decision
visible; it does not change the routing policy.
