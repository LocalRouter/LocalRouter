# Adaptive ordering and bounded routing attempts

- [ ] Inspect candidate selection, execution, streaming accounting and config/UI contracts.
- [ ] Add opt-in per-strategy ordering (priority, estimated cost, observed TTFT, observed throughput, least in-flight), bounded passive observations with minimum samples and deterministic cold starts.
- [ ] Preserve selected-route/default/baseline boundaries and apply shared eligibility before scoring; unknown prices are never cheapest.
- [ ] Add explicit maximum actual attempts, per-attempt deadline and total request deadline including classification/preflight. Count gateway calls once, do not multiply upstream retries or issue speculative calls.
- [ ] Track cancellation-safe in-flight permits, streaming TTFT/output rate and provider recovery; never restart after output or tool execution. Preserve usage accounting for cancelled/deadline streams and label unknown costs on failed calls.
- [ ] Expose settings and sanitized candidate scores/attempt outcomes in preview, response and monitor; sync config/TS/demo and command types.
- [ ] Add meaningful ordering/cold-start/filter/recovery/timeout/cancellation/no-stream-restart regression tests.
- [ ] Plan Review: complete missed requirements and endpoint parity.
- [ ] Test Coverage Review: inspect error, isolation and migration paths.
- [ ] Bug Hunt: review cancellation, deadline propagation, stale observations and retry amplification.
- [ ] Update stable, pin compiler PATH/RUSTC; run workspace Clippy/fmt/tests and frontend/demo checks.
- [ ] Commit and push validated changes and automatic catalog updates, preserving unrelated work.

Defaults retain configured priority and existing retry behavior unless a user enables bounds/ordering. The acceptable pool is the explicit route group; sorting cannot move a baseline model ahead of the selected route. Passive samples are local memory only, bounded by model keys, count and age, and are never collected via probes. Failed calls do not fabricate token/cost usage. Per-attempt timeout may leave an unknown upstream charge; expose that uncertainty. Gateway call count is local dispatches; upstream fallback is governed by existing gateway preferences.

Streaming uses a deadline for setup/first visible chunk and a total deadline during the returned stream. Once an upstream stream begins emitting, errors end that stream without automatic failover. Output-throughput statistics require completed streams with reported output tokens and positive generation time; estimates do not train ordering. TTL-aged observations and too few successful samples preserve deterministic priority. Recovery uses existing Retry-After cooldown with bounded infrastructure-error cooldown; no sleeping retry loop or same-model retry.

All chat surfaces share the router. Native Decisions get endpoint-appropriate passive measurements and bounded attempts; TTFT/throughput stay unknown for nonstreaming decisions. Explicit models are not reordered. New settings belong to AutoModelConfig and are saved through update_strategy with existing permissions intact.
