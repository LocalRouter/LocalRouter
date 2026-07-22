# Model Selection: Allow-All / Per-Provider Allow + Model List Refresh

**Date**: 2026-07-22
**Status**: In progress

## Problem

1. In the client Models tab (`UnifiedModelsTab` → `ThreeZoneModelSelector`), a fresh
   strategy allows all models (`model_permissions.global = 'allow'`), but the moment
   any model is selected the tab hard-codes `global: 'off', providers: {}` and there
   is no way back to "all models" and no way to allow a whole provider — even though
   the backend `ModelPermissions` (global → provider → model resolution) fully
   supports both levels.
2. Model lists are cached per provider (5 min TTL in `ProviderRegistry`) and the UI
   only exposes a force-refresh in the Try-It-Out LLM tab. Toggling a provider
   off/on does not even invalidate its cache — users have no reliable way to pick up
   newly released models.

## Design

### A. Access level vs. priority order

`auto_config.prioritized_models` (the drag-ordered "Enabled" zone) keeps driving
auto-router priority. `model_permissions` controls API access. New UI semantics:

- **Allow all models** (switch above the selector):
  `{ global: 'allow', providers: {}, models: {} }` — includes future models.
  The three-zone list stays visible and only sets auto-router priority; the
  "Disabled" zone is relabeled since those models remain accessible.
- **Allow whole provider** (toggle on each provider group header in the disabled
  zone): `providers[p] = 'allow'` with `global: 'off'` — all current + future
  models of that provider accessible; list still sets priority.
- **Specific models** (current behavior): `global: 'off'`, `models` map built from
  enabled + weak lists.

Remove the frontend "self-heal" in `unified-models-tab.tsx` (`ensureAutoConfig`
lines 144-165 + the persistence block in `loadData` lines 257-278) which force-
converts `global: 'allow'` + prioritized models into specific mode. That code
actively fights the new feature and contradicts request-time enforcement
(`lr-server/src/routes/helpers.rs` allow-all fast path) — such strategies were
already allow-all at request time.

### B. Refresh models

Backend plumbing exists end-to-end: `refresh_models_incremental { force: true }` →
`invalidate_all_caches()` → parallel refetch with `models-refresh-started` /
`models-provider-loaded` / `models-changed` events, consumed by
`useIncrementalModels`.

- New shared `RefreshModelsButton` component (spinner driven by refresh events).
- Surface it in: client Models tab (Model Selection card header), Resources →
  Models panel, guardrails `SafetyModelPicker`. Try-It-Out already has one.
- Backend fix: `set_provider_enabled` now calls
  `registry.invalidate_provider_cache(&instance_name)` so toggling a provider
  genuinely refetches its models.

## Tasks

- [x] Investigate current selection UI + permissions model + cache paths
- [x] Save plan (this file)
- [x] `ThreeZoneModelSelector`: `allowAll` / `allowedProviders` props + UI
- [x] `unified-models-tab.tsx`: derive state, new handlers, preserve levels in
      `handleModelsChange`, remove self-heal sync
- [x] `RefreshModelsButton` shared component
- [x] Wire refresh into unified-models-tab, models-panel, SafetyModelPicker
- [x] `set_provider_enabled` cache invalidation (Rust)
- [x] Plan review (verified: legacy `find_provider_for_model` path is
      resolution-only, does not block new states; `update_strategy` None =
      unchanged; demo mocks already cover refresh commands; no Tauri command
      signature changes so no type/mock sync needed)
- [x] Test coverage review (no frontend test infra exists; Rust change is a
      single call into the already-tested `invalidate_provider_cache`)
- [x] Bug hunt (debounce merge interactions between allow-all toggle, provider
      toggle, and drag changes verified; `ensureAutoConfig` default global
      'allow' keeps fresh strategies showing allow-all ON)
- [x] `npx tsc --noEmit` clean (app + website)
- [ ] clippy/fmt clean, commit

## Final Steps (mandatory)

1. **Plan Review**: re-check implementation against this plan
2. **Test Coverage Review**: add tests for uncovered new paths
3. **Bug Hunt**: fresh-eyes pass over the diff
4. **Commit**: only files modified by this task
