# Foundational libraries second-wave review — 2026-10-01

- [x] Inventory the additional assigned modules.
- [x] Inspect client/token management, metrics persistence, event buffers, catalog matching and utility contracts.
- [x] Implement independently verified improvements and local regressions.
- [x] Review the final changes and coverage; run focused CPU-only checks and hand final stable compilation to the coordinating root agent.
- [x] Record precise validation, coverage boundaries and remaining follow-ups.

Scope: lr-monitoring, lr-monitor, lr-json-repair, lr-types, lr-utils, lr-clients and lr-catalog. The catalog snapshot is pre-existing user work and must remain untouched. Root previously tested types, utils, json-repair and monitor; this wave supplements that work.

## Implemented fixes

### Public trace headers no longer grant enforcement/accounting bypass

Root identified that the deduplication trace was trusted across an HTTP boundary. A caller could send `X-LocalRouter-Trace: anything;hop=99`; code treating hop > 1 as already handled could skip policy checks and usage accounting.

`lr-types/src/trace.rs::RequestTrace::outbound_for` now keeps the trace ID for correlation while resetting its hop to 1. Module/API documentation explains that a wire header is not proof of prior authorization. `next_hop` and task-local trusted internal traces remain available, but all inspected HTTP ingress call sites use `outbound_for` via the server trace middleware or proxy `stamp_trace` helper.

Regressions cover claimed hops 1, 2, 99 and u32::MAX, including propagation into spawned tasks. Proxy tests verify a forged trace still runs the firewall and counts the request, and reverse-proxy integration expectations now preserve the ID while treating the request as a fresh enforcement/accounting hop. The API agent owns the equivalent server boundary fixture.

Compatibility effect: real multi-hop wire requests can now prompt/count more than once. Authenticated request-bound provenance is required before wire-triggered deduplication can safely be restored. This deliberate tradeoff was coordinated with the root agent before editing.

### Metrics aggregation no longer counts the same request multiple times

`lr-monitoring/src/storage.rs::get_aggregated_usage` summed minute, hourly and daily copies of the same traffic. It is used by recent strategy usage, pre-estimates and feature totals, so existing rollups could inflate limits and displayed savings. It now delegates to the shared non-overlapping totals query.

The previous supposedly deduplicated `get_usage_for_type` implementation had its own gap bug: it excluded all fine rows before the newest coarse row, even if earlier hours had never been aggregated. It could also count minute rows again when a daily row existed without hourly rows. The new query excludes a fine row only when a selected, coarser row for the same metric type covers that exact interval. A coarse row outside the requested time window does not hide fine rows inside the window.

Regressions now assert the same correct totals from both public aggregation methods and exercise an unaggregated old hour, a later hourly rollup, a daily rollup after hourly rows are removed, and a partial-window query. The production SQL was also extracted directly from the Rust source and executed against in-memory Python SQLite fixtures: all three gap/daily/partial-window assertions passed.

The final performance check caught an initial correlated-query regression on 10,000 minute-only rows, which was interrupted after three seconds. A partial covering index containing only hourly/daily rows and a bounded coverage lookup now avoid repeatedly scanning minute history or unrelated older rollups. Repeating the exact production SQL on the same synthetic fixture returned the correct totals in 0.0022 seconds with minute-only data, and 0.0262 seconds with 167 hourly and seven daily rollups also present. These are local smoke measurements, not a portable performance guarantee. SQLite's query plan confirmed use of the new partial index.

This keeps the existing bucket timestamp interpretation of query windows. It does not reconstruct intra-bucket usage once finer history has been deleted; exact partial-hour/day historical accounting would require a different retention contract.

### Monitor callbacks can safely replace the emitter

`lr-monitor/src/store.rs` invoked create/update notification callbacks while holding the emitter read lock. A callback that called `set_emitter` would wait forever for the same lock. Both paths now clone the callback and release the lock before invocation. The regression replaces the callback from both create and update notifications; it checks lock availability first so a regression fails without hanging the test process.

## Supplemental review coverage

The root had already reviewed and tested several foundational crates. This wave is an additional structural and selected semantic pass, not a duplicate full audit.

| Crate | Supplemental inspection | Outcome |
|---|---|---|
| lr-types | Error/MCP/common helper surfaces; request trace parse/serialization/outbound/task-local propagation | Confirmed and fixed wire-header trust issue; other pure helpers retained |
| lr-monitoring | SQLite schema and rollup queries, successful/failed recordings, metrics consumers, scheduling interfaces, logger/MCP graph module inventory | Fixed double counting and gap handling; other areas require deeper persistence/rotation performance work |
| lr-monitor | Event count/byte budget, event size and summary interfaces, push/update/list, completion guard and truncation contracts | Fixed emitter callback reentrancy deadlock. Existing guard marks abandoned events failed. Single oversized newest event remains intentionally allowed by documented policy |
| lr-clients | Client CRUD/secret-store sequencing, enabled checks, synchronization interfaces and in-memory token generation/expiry/revocation | No real secrets/keychain calls performed. Token store uses cryptographic random keys and expiry checks; further atomicity/error-recovery review remains useful |
| lr-utils | Crypto key generation, paths, install-source detection, host invocation, shell-path timeout and binary-discovery interfaces | No additional concrete correction isolated beyond root's work; platform-specific helpers not exercised live |
| lr-json-repair | Public repair options, streaming frame/state interfaces, finish/action handling and pure test structure | No model-dependent behavior; root's CPU suite supplies runtime coverage. Exhaustive schema/fuzz testing not performed |
| lr-catalog | Build-time cache switch, fetch/generation interfaces, model matcher exact/alias/prefix/date-suffix stages, generated catalog boundary | No source/snapshot changes. Prefix/name matches can still be ambiguous across providers; deterministic matching policy deserves separate design and pricing fixtures |

## Validation and boundaries

- No GPU, inference, model download, provider call, app launch, real credential inspection or live account mutation.
- First-wave library binaries passed 800 tests; those results are recorded in `mcp-tools-context.md`.
- Stable rustfmt check passed after the trace and metrics edits.
- Extracted production SQL passed three in-memory SQLite cases for rollup gaps/daily-only/partial-window selection.
- Extracted production SQL also returned correct totals for 10,000 minute-only rows and the same rows with overlapping hourly/daily rollups; query-plan inspection confirmed indexed coverage lookup.
- Final stable Rust 1.99.0 tests passed: lr-types 29, lr-monitoring 45, lr-monitor 33. These include the final trace, indexed metrics query and emitter regressions. The shared run also passed lr-proxy 73, lr-marketplace 29 and lr-local-models 88: 297 total, zero failures. Workspace Clippy is recorded in the consolidated report.
- No commits were made; the pre-existing catalog JSON modification was preserved.

## Follow-ups not represented as completed work

- Authenticated cross-process trace provenance before re-enabling wire-based enforcement/accounting deduplication.
- Rollup backfill after long application downtime; failed aggregation retries; exact reporting for windows whose leading/trailing buckets have lost fine detail.
- Client deletion/secret rotation recovery if secret-store operations fail, and capacity control for generated tokens.
- Deterministic catalog prefix matching and ambiguity handling, with provider-specific pricing expectations. No live pricing assertions were made.
- Exhaustive repair-parser/schema fuzzing and cross-platform binary-discovery/install-source testing remain separate work.

## Supplemental source inventory

| Crate | First-party Rust files | Rust lines at end of this wave |
|---|---:|---:|
| lr-monitoring | 9 | 4,961 |
| lr-monitor | 7 | 2,897 |
| lr-json-repair | 3 | 2,020 |
| lr-types | 5 | 688 |
| lr-utils | 7 | 1,768 |
| lr-clients | 3 | 1,165 |
| lr-catalog | 8 | 1,712 |

Generated `lr-catalog/catalog/catalog.rs` and the raw catalog snapshot are excluded from first-party review counts.

## Supplemental file inventory

These paths were included in the structural pass. Only the three changed modules received the detailed fixes described above; other modules were sampled at their interfaces and selected stateful operations.

| File | Review depth |
|---|---|
| `crates/lr-monitoring/src/aggregation_task.rs` | Structural / sampled |
| `crates/lr-monitoring/src/graphs.rs` | Structural / sampled |
| `crates/lr-monitoring/src/lib.rs` | Structural / sampled |
| `crates/lr-monitoring/src/logger.rs` | Structural / sampled |
| `crates/lr-monitoring/src/mcp_graphs.rs` | Structural / sampled |
| `crates/lr-monitoring/src/mcp_logger.rs` | Structural / sampled |
| `crates/lr-monitoring/src/mcp_metrics.rs` | Structural / sampled |
| `crates/lr-monitoring/src/metrics.rs` | Structural / sampled |
| `crates/lr-monitoring/src/storage.rs` | Changed / focused review |
| `crates/lr-monitor/src/guard.rs` | Structural / sampled |
| `crates/lr-monitor/src/lib.rs` | Structural / sampled |
| `crates/lr-monitor/src/size.rs` | Structural / sampled |
| `crates/lr-monitor/src/store.rs` | Changed / focused review |
| `crates/lr-monitor/src/summary.rs` | Structural / sampled |
| `crates/lr-monitor/src/truncate.rs` | Structural / sampled |
| `crates/lr-monitor/src/types.rs` | Structural / sampled |
| `crates/lr-json-repair/src/lib.rs` | Structural / sampled |
| `crates/lr-json-repair/src/streaming.rs` | Structural / sampled |
| `crates/lr-json-repair/src/types.rs` | Structural / sampled |
| `crates/lr-types/src/errors.rs` | Structural / sampled |
| `crates/lr-types/src/fuzzy.rs` | Structural / sampled |
| `crates/lr-types/src/lib.rs` | Structural / sampled |
| `crates/lr-types/src/mcp_types.rs` | Structural / sampled |
| `crates/lr-types/src/trace.rs` | Changed / focused review |
| `crates/lr-utils/src/binary.rs` | Structural / sampled |
| `crates/lr-utils/src/crypto.rs` | Structural / sampled |
| `crates/lr-utils/src/install_source.rs` | Structural / sampled |
| `crates/lr-utils/src/lib.rs` | Structural / sampled |
| `crates/lr-utils/src/paths.rs` | Structural / sampled |
| `crates/lr-utils/src/sandbox.rs` | Structural / sampled |
| `crates/lr-utils/src/test_mode.rs` | Structural / sampled |
| `crates/lr-clients/src/lib.rs` | Structural / sampled |
| `crates/lr-clients/src/manager.rs` | Structural / sampled |
| `crates/lr-clients/src/token_store.rs` | Structural / sampled |
| `crates/lr-catalog/build.rs` | Structural / sampled |
| `crates/lr-catalog/buildtools/codegen.rs` | Structural / sampled |
| `crates/lr-catalog/buildtools/mod.rs` | Structural / sampled |
| `crates/lr-catalog/buildtools/models.rs` | Structural / sampled |
| `crates/lr-catalog/buildtools/scraper.rs` | Structural / sampled |
| `crates/lr-catalog/src/lib.rs` | Structural / sampled |
| `crates/lr-catalog/src/matcher.rs` | Structural / sampled |
| `crates/lr-catalog/src/types.rs` | Structural / sampled |
