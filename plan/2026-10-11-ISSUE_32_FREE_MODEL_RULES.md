# Issue #32 — rule-based free model matching

Approved by the maintainer on 2026-10-11: all rule types including JSONPath
(no custom code snippets), plus all four default changes below. Built on top
of issue #33 (2026-10-11-ISSUE_33_MODEL_DISCOVERY.md — per-model settings,
per-instance pricing, raw listing).

## Todo
- [x] Config: extend `FreeTierKind::FreeModelsOnly` additively —
      `free_model_patterns` gets `#[serde(default)]` but is always serialized
      (older builds require it); `excluded_models`, `rules: Vec<FreeModelRule>`
      (skip if empty). `FreeModelRule{enabled, effect: include|exclude, when:
      FreeModelCondition, label}`; conditions tagged `type`: `model_id{pattern,
      syntax: exact|glob|regex, case_insensitive}`, `field{pointer (RFC 6901),
      op, value}`, `json_path{expression}` (RFC 9535 filter predicate on the
      single model object via `serde_json_path`; check licence), `zero_price
      {sources: [pricing_override, provider_listing], input_pointer,
      output_pointer}`, `#[serde(other)] unsupported`. No version bump.
- [x] Raw model metadata: `list_models_with_metadata` trait method (default
      wraps `list_models`), implemented for openai_compatible (+ `{"models":
      […]}` envelope from #33) and OpenRouter; `ModelCache.metadata`;
      `registry.model_metadata`; shared `listing_pricing(&Value)` extracted
      from `systemone::gateway::parse_listing`/`per_1k`; OpenRouter
      `get_pricing` reads cached metadata instead of re-downloading.
- [x] Evaluator `crates/lr-providers/src/free_models.rs`: order = excluded
      list → included list (glob with any number of `*`) → first enabled
      matching rule → not free. Unknown data fails closed. Regex size/length
      limits, JSONPath length cap, compile errors → never-match + error list.
      Catalog prices are never a source (0.0 means unknown there).
- [x] Router: `classify_for_free_tier(provider, model)` replaces the 8 call
      sites (lr-router lib.rs ×6, systemone.rs ×2); fetch the model list once
      if a metadata/price rule needs it and the cache is empty;
      `classify_model(..., model_is_free)`; `record_usage` counts only free
      models for FreeModelsOnly RPM.
- [x] Defaults (maintainer-approved):
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
- [x] Tauri: `free_tier_model: ModelFreeTierInfo{is_free, decided_by,
      rule_index, reason}` on detailed models (and stop hiding $0 listing
      prices; `pricing_source: listing`); new `preview_free_model_rules`,
      `get_provider_model_metadata`; `set_provider_free_tier` validates rules.
      TS types + demo mock.
- [x] UI: Free Tier tab extracted (`src/views/resources/provider-free-tier/`)
      with rule editor (order, enable, include/exclude, per-type inputs,
      inline errors, match counts), debounced live preview table with "why",
      Auto/Always free/Never free per model, presets, raw-metadata inspector
      that fills a `field` rule. Per-model badges in the Info tab, models
      panel and the 4 strategy selectors.
- [x] List delay fix: drop `useIncrementalModels` from providers-panel;
      `refresh_instance_models` emits `models-provider-loaded` after
      create/update/rename/enable; hook refetches cached models on
      `models-changed`.
- [x] Tests (config round-trips, evaluator per condition/op/order/limits,
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

## Implementation notes (as built)
- **Finding: type defaults were never applied for several types.** The
  router and the Free Tier tab looked the default up under the *serialized*
  config type (`open_router`, `together_a_i`, `deep_infra`, `x_a_i`,
  `custom`), which matched no factory, so OpenRouter, Together AI,
  DeepInfra, xAI and every OpenAI-compatible provider had *no* free tier
  unless the user set one (the "verified" AlwaysFreeLocal default for
  OpenAI-compatible was the factory's value, not the effective one).
  `ProviderType::factory_id()` (lr-config) now maps config types to
  factories (also used by provider loading in `main.rs`), and
  `registry.default_free_tier_for_instance(name, type)` asks the factory
  with the instance's config (`ProviderFactory::default_free_tier_for`).
- Migration v33 (CONFIG_VERSION 33) therefore pins what existing
  installs actually had: DeepInfra and xAI instances without their own
  free tier get `free_tier: none` (otherwise they would silently become
  credit-based free). Remote OpenAI-compatible instances need no pin (their
  effective value was and stays "none"). OpenRouter and Together AI get
  their defaults and local OpenAI-compatible servers become always free, as
  decided.
- Default 1: OpenRouter → `rules: [zero_price(provider_listing)]`,
  `free_model_patterns: []`, 20 RPM. Together AI keeps its explicit free
  model id: its listing prices are per 1M tokens and (as far as known,
  not verifiable without a key) $0 for models that are not serverless, so a
  zero-price default could mark unusable or paid models free. Together's
  listing entries are kept, so users can write rules on them.
- Default 2: `OpenAICompatibleProviderFactory::default_free_tier_for` →
  AlwaysFreeLocal only for a loopback base URL (`fastflowlm::is_loopback_url`),
  else None; the type-level default (Add Provider text) stays "always free"
  with a note.
- Default 3: `openai_compatible::get_pricing` → gateway price, else the
  listing's per-token price (`listing_metadata::listing_pricing`), else free;
  the per-instance override still wins in `registry.get_pricing`. A listed
  price above $1,000 per 1M tokens is taken as another unit and ignored.
  Pricing fetches the list itself only when it was never fetched, with a 60 s
  back-off after a failure. `/v1/models` and the detailed model list show
  listing prices (`pricing_source: provider` / `listing`, $0 included).
- Default 4: `FreeTierManager::classify_model(provider, kind,
  model_is_free)`; for always-free kinds a model whose price override
  (instance, else legacy type-keyed `pricing_overrides`) is above $0 is not
  free.
- Listing data: `ModelListing.metadata: Arc<HashMap<id, Value>>` (the
  plan's `list_models_with_metadata` became this field, per #33's
  `list_model_listing`), filled by openai_compatible (all envelopes),
  OpenRouter and Together AI; registry: `model_metadata`,
  `listing_metadata`, `has_cached_models`, `decide_free_model`,
  `effective_pricing_override`, `sync_cached_models`. OpenRouter and
  openai_compatible keep the latest listing (`ListingMetadataCache`) so
  pricing needs no download of its own.
- Evaluator `lr_providers::free_models::FreeTierEvaluator` (compiled once
  per call site), `validate`. Outcomes are match / no match / unknown;
  unknown (no listing entry, no price, a rule with an error) never includes
  and always excludes. A field that is absent from a known entry is a known
  "no match" (except `not_exists`). Regex: 1,000 chars, 1 MiB compiled
  limit, syntax errors reduced to their last line; JSONPath `$[?<expr>]`
  over `[entry]`, 1,000 chars, a leading `?` accepted. Unknown effect acts
  as exclude.
- Router: `classify_for_free_tier` / `classify_with_free_tier` at all 8
  sites; `free_model_decision` fetches the model list first when an
  enabled rule needs listing data and the instance was never listed;
  usage recording uses `free_tier_usage(provider, model)` so only free
  models count against a free-models-only RPM (System One usage now passes
  the model).
- Tauri: `free_tier_model` on detailed models; `preview_free_model_rules`
  (rows with decision, rule errors, per-rule match counts, free count,
  discovery error, whether listing data exists); `get_provider_model_metadata`;
  `set_provider_free_tier` refuses rules with errors and emits
  `models-changed`; `get_free_tier_status` reports `factory_id` types.
- UI: `src/views/resources/provider-free-tier/` (tab, rules editor with
  presets, debounced preview with per-model Auto/Always/Never, metadata
  inspector that adds a field rule). Save is blocked while the preview
  reports rule errors. Per-model pills ("Free", "Not free", "Paid" for a
  priced local model) in the Info tab, Models panel and the 4 strategy
  selectors (ThreeZone's free filter is per model).
- List delay: `refresh_instance_models` after create, update, rename,
  clone, enable/disable and remove; `useIncrementalModels` reloads the
  cached lists on `models-changed`; providers-panel no longer uses it.
- Licence: `serde_json_path` (+ `_core`, `_macros`, `_macros_internal`) is
  MIT, compatible with AGPL-3.0-or-later; added to THIRD-PARTY-LICENSES.md.
  Adding it let cargo unify `socket2`/`windows-sys` versions in Cargo.lock
  (ranges allowed by their dependents).
- Docs: website docs "free-tier-types" updated.

## Verification
- `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --
  -D warnings` (rustup stable): clean.
- `cargo test -p lr-config -p lr-providers -p lr-router -p lr-server` and
  `cargo test -p localrouter --lib --test router_strategy_tests`: pass.
  New tests: config (old format loads, new keys skipped while empty,
  round-trip, newer-build values load as Unknown/Unsupported, factory ids,
  migration v33); evaluator (globs, list order, rule order/enabled, each
  id syntax, each field op, missing data fails closed, JSONPath, zero price
  sources/order/custom paths/request fee/variable price, every compile
  error incl. regex size, unknown effect, whole-provider kinds); listing
  prices/envelopes/back-off; openai_compatible (listing entries, listed
  prices with one fetch, failed listing not retried per price); OpenRouter
  (prices from the kept list, no second download); registry (entries
  cached and kept across failures/settings, instance defaults by factory
  type, rules-only text); factory (OpenRouter rule default validates,
  OpenAI-compatible loopback default); router (rules fetch the list once,
  zero-price decision, free-only System One refuses non-free models, usage
  only counts free models, priced always-free model); Tauri (preview rows,
  match counts and errors, detailed models with listing price and free
  decision, remote/local instance tier).
- `npx tsc --noEmit` (app and website): clean.
- Website demo (headless Chromium, light + dark) with a Free.ai-like
  provider: Free Tier tab with lists, 3 rules and the preview, metadata
  inspector, rule added from the inspector, invalid regex inline error
  with Save blocked, Info tab per-model pills and detail, a priced model on
  an always-free provider. Scripted: per-model "Never free", disabling a
  rule and saving give the expected counts and the Info tab follows.

## Final steps
- [x] Plan review: every Todo item checked against the code; the
      default-lookup bug found while doing it is fixed and migrated.
- [x] Test coverage review: tests added for each new path above.
- [x] Bug hunt: fixed an empty field value passing validation, regex
      errors rendering as a multi-line diagram, the editor allowing save
      with rule errors, dirty detection treating a cleared note as a
      change, the badge calling an excluded $0 model "Paid", and the type's
      free-tier text showing for providers that no longer use the default.
- [x] Commit (no push, per instruction).
