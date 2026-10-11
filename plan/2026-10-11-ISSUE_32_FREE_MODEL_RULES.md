# Issue #32 — rule-based free model matching

Approved by the maintainer on 2026-10-11: all rule types including JSONPath
(no custom code snippets), plus all four default changes below. Built on top
of issue #33 (2026-10-11-ISSUE_33_MODEL_DISCOVERY.md — per-model settings,
per-instance pricing, raw listing).

## Todo
- [ ] Config: extend `FreeTierKind::FreeModelsOnly` additively —
      `free_model_patterns` gets `#[serde(default)]` but is always serialized
      (older builds require it); `excluded_models`, `rules: Vec<FreeModelRule>`
      (skip if empty). `FreeModelRule{enabled, effect: include|exclude, when:
      FreeModelCondition, label}`; conditions tagged `type`: `model_id{pattern,
      syntax: exact|glob|regex, case_insensitive}`, `field{pointer (RFC 6901),
      op, value}`, `json_path{expression}` (RFC 9535 filter predicate on the
      single model object via `serde_json_path`; check licence), `zero_price
      {sources: [pricing_override, provider_listing], input_pointer,
      output_pointer}`, `#[serde(other)] unsupported`. No version bump.
- [ ] Raw model metadata: `list_models_with_metadata` trait method (default
      wraps `list_models`), implemented for openai_compatible (+ `{"models":
      […]}` envelope from #33) and OpenRouter; `ModelCache.metadata`;
      `registry.model_metadata`; shared `listing_pricing(&Value)` extracted
      from `systemone::gateway::parse_listing`/`per_1k`; OpenRouter
      `get_pricing` reads cached metadata instead of re-downloading.
- [ ] Evaluator `crates/lr-providers/src/free_models.rs`: order = excluded
      list → included list (glob with any number of `*`) → first enabled
      matching rule → not free. Unknown data fails closed. Regex size/length
      limits, JSONPath length cap, compile errors → never-match + error list.
      Catalog prices are never a source (0.0 means unknown there).
- [ ] Router: `classify_for_free_tier(provider, model)` replaces the 8 call
      sites (lr-router lib.rs ×6, systemone.rs ×2); fetch the model list once
      if a metadata/price rule needs it and the cache is empty;
      `classify_model(..., model_is_free)`; `record_usage` counts only free
      models for FreeModelsOnly RPM.
- [ ] Defaults (maintainer-approved):
      1. OpenRouter default → `rules: [zero_price(provider_listing)]` (today
         `":free"` is an exact match and matches nothing); same for Together
         if its listing reports prices.
      2. New custom `openai_compatible` providers on a non-loopback base URL no
         longer default to `AlwaysFreeLocal`; a config migration pins existing
         instances to their current effective value.
      3. `openai_compatible::get_pricing` uses listing prices when present
         (cost tracking + cost watchdog).
      4. A per-model price override > $0 on a local/always-free provider
         makes that model not free for free-only strategies.
- [ ] Tauri: `free_tier_model: ModelFreeTierInfo{is_free, decided_by,
      rule_index, reason}` on detailed models (and stop hiding $0 listing
      prices; `pricing_source: listing`); new `preview_free_model_rules`,
      `get_provider_model_metadata`; `set_provider_free_tier` validates rules.
      TS types + demo mock.
- [ ] UI: Free Tier tab extracted (`src/views/resources/provider-free-tier/`)
      with rule editor (order, enable, include/exclude, per-type inputs,
      inline errors, match counts), debounced live preview table with "why",
      Auto/Always free/Never free per model, presets, raw-metadata inspector
      that fills a `field` rule. Per-model badges in the Info tab, models
      panel and the 4 strategy selectors.
- [ ] List delay fix: drop `useIncrementalModels` from providers-panel;
      `refresh_instance_models` emits `models-provider-loaded` after
      create/update/rename/enable; hook refetches cached models on
      `models-changed`.
- [ ] Tests (config round-trips, evaluator per condition/op/order/limits,
      metadata envelopes, router free-only behaviour, preview helper),
      plan review, test coverage review, bug hunt, commit.

## Current behaviour (verified)
- `model_matches_patterns`: exact match, or a single `*` glob; two or more
  `*` never match. OpenRouter default `":free"` matches nothing.
- `openai_compatible` defaults to `AlwaysFreeLocal` even for remote APIs.
- `ModelInfo` has no pricing/raw JSON; listings drop every field but the id.
- Free badge is per provider in the Info tab, models panel and selectors.
- Free Tier list reads `useIncrementalModels`, which is only refreshed by a
  global refresh; creating a provider never triggers one.

## Risks
- A wrong rule means paid usage in free-only mode; mitigated by fail-closed
  evaluation, preview, and listing-based pricing for the watchdog.
- Changing the OpenRouter default changes routing (intended fix).
- New dependency `serde_json_path`.
