# GuardRail Gating + Proxy Monitor Attribution

**Date**: 2026-08-03
**Status**: In progress

## Problem statement

Reported against the production build (`~/.localrouter/settings.yaml`):

1. **Monitor shows a client UUID instead of the client name** for HTTPS-proxy
   requests (e.g. `a2e3d53d` instead of `Claude Code`).
2. **A "proxy" pill** is rendered next to the event type in the Monitor list;
   it is noise.
3. **GuardRails runs even though it is configured off.** The global config is
   `__global: allow` (the UI's "All Categories → Allow", i.e. off), the client
   `Jazyk` inherits it, and yet `guardrail_scan` events appear in Monitor.
   Enabling the per-client override does not help — because the override is
   also all-Allow. It only *looks* fixed for proxy-mode clients, where
   guardrails never run at all.

## Root causes

### 1. Proxy events carry no client name

`crates/lr-proxy/src/passive.rs` pushes monitor events with
`client_name = None`:

```rust
self.monitor.push(MonitorEventType::LlmCall, Some(ex.client_id.clone()), None, ...)
```

`lr-proxy` deliberately does not depend on `lr-clients`, so it has no way to
resolve the name. The gateway path resolves it via
`monitor_helpers::resolve_client`.

The frontend then falls back to `event.client_id?.slice(0, 8)`.

### 2. GuardRails master toggle is vestigial

`GuardrailsConfig::enabled` and `scan_responses` are explicitly marked
"migration shim (deserialize only, not serialized)". The master toggle was
replaced by the `__global` ("All Categories") category action. But
`run_guardrails_scan` (`crates/lr-server/src/routes/pipeline.rs`) only checks:

- `scan_requests` (defaults **true**), and
- `effective_category_actions.is_empty()`

An all-Allow policy is *not* empty, so the scan runs the safety model, emits a
`guardrail_scan` monitor event, and only *afterwards* throws the verdict away
in `apply_client_category_overrides`. Net effect: a full local LLM call per
request for a feature the user turned off.

### 3. `__model:<type>` group actions are silently ignored

The category tree in the UI groups categories under a safety-model-type node
(`__model:llama_guard`) and lets that node carry an action which children
inherit. `SafetyCheckResult::apply_client_category_overrides` resolves only
`specific category → __global`, so a group-level Block/Ask never takes effect.

## Changes

### A. Proxy client-name resolution

- Add `ClientNameResolver` to `crates/lr-proxy/src/interceptor.rs`, mirroring
  the existing `PricingResolver` trait (keeps `lr-proxy` free of a
  `lr-clients` dependency).
- Add `PassiveInterceptor::with_client_names(...)`; resolve the name in
  `emit_pending` and `record`.
- Implement it in `src-tauri/src/launcher/proxy.rs` over `ClientManager` and
  wire it in `ProxyService::new`.

### B. Remove the proxy pill

Delete the amber badge in `src/views/monitor/event-list.tsx`. The `source`
field stays in the data model and in the event detail view.

### C. Short-circuit all-Allow guardrail policies

In `run_guardrails_scan`, after merging global + per-client category actions,
skip the scan entirely when the policy cannot flag anything:

> The `__global` fallback is `Allow` **and** no specific entry overrides it
> with `Block`/`Ask`/`Notify`.

Without a `__global` entry, unlisted categories keep the engine default
(`Ask`), so the scan must still run — that case is preserved.

### D. Resolve `__model:<type>` group actions

- Add `model_type` to `CategoryActionRequired` (populated from the
  `SafetyModel::model_type_id()` of the model that flagged it).
- Resolution order in `apply_client_category_overrides`:
  `specific category → __model:<type> → __global → engine default`.
- Include `__model:<type>` entries in the all-Allow short-circuit check.

### E. GuardRails were never enforced on the HTTPS-proxy path

`AppFirewall::evaluate` (`src-tauri/src/launcher/proxy.rs`) ran model
permissions, rate limits, secret scanning and model approval — but never
guardrails. A proxy-mode client (`llm_mode: proxy`, e.g. Claude Code, Codex)
showed a GuardRails tab whose settings did nothing.

Fixed by refactoring the two pipeline stages into body-shape-agnostic cores
(exactly how `scan_request_for_secrets` already works) and calling the combined
entry point from the proxy firewall:

- `guardrails_scan_request(state, client_id, model, &body)` — the scan
- `handle_guardrail_result(...)` — block / notify / approval popup
- `scan_request_for_guardrails(...)` — the combined allow/deny form

`run_guardrails_scan` / `handle_guardrail_approval` remain as thin
`ChatCompletionRequest` wrappers, so the three gateway surfaces are unchanged.

### F. `guardrails_active` in the UI used replace, not merge

`list_clients` / `get_client` / `get_client_feature_status` computed the
client's effective category actions as `client.category_actions.unwrap_or(global)`,
dropping global entries whenever the client had a sparse override — while the
pipeline merges per-category. Both now call the shared
`merge_guardrail_category_actions`, and the "active" badge uses the same
predicate as the runtime short-circuit (`!guardrails_allow_everything`). The
client GuardRails tab merges for display too.

## Configuration audit

| Feature | Gate | Gateway | Proxy |
|---|---|---|---|
| GuardRails | `__global` + per-category actions (merged) | ✅ fixed | ✅ added |
| Secret scanning | `action == Off`, per-client override | ✅ | ✅ |
| Prompt compression | `enabled`, per-client override | ✅ | n/a (needs rewrite) |
| JSON repair | `enabled` + `syntax_repair`, per-client override | ✅ | n/a (response rewrite) |
| Model permissions | `auto_config.permission`, `is_model_allowed` | ✅ | ✅ |
| Rate limits | `StrategyRateLimit.enabled` | ✅ | ✅ |
| RouteLLM | `routellm_config.enabled` | ✅ | n/a |
| Memory | `client.memory_enabled` | ✅ | n/a |

Notes:

- `GuardrailsConfig::enabled` and `scan_responses` are deserialize-only
  migration shims and are intentionally not read; the `__global` category
  action is the master switch. Response-side scanning is not implemented.
- `lr-router`'s RouteLLM branch (`crates/lr-router/src/lib.rs:1165`) does not
  re-check `routellm_config.enabled`, but it is only reachable when
  `pre_computed_routing` is `Some`, which the pipeline only sets after
  filtering on `enabled`. Not a live bug; left as is.
- Prompt compression and JSON repair rewrite request/response bodies, which the
  inspect-only proxy deliberately does not do. Not a gap.

## Mandatory final steps

1. **Plan review** — re-read this plan against the implementation.
2. **Test coverage review** — unit tests for the all-Allow short-circuit
   (including the "no `__global`" case), the model-type group resolution, and
   proxy client-name resolution.
3. **Bug hunt** — fresh-eyes pass over the changed code.
4. **Config audit** — verify every per-client / global feature gate
   (guardrails, secret scanning, prompt compression, JSON repair, rate limits,
   model permissions) is honored on both the gateway and proxy paths; report
   findings.
5. **Commit**, then cut the release.
