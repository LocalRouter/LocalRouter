# Issue #33 — custom provider model discovery

Approved by the maintainer on 2026-10-11: all phases (A+B+C) plus a typed
FastFlowLM provider. Issue #32 (free-model rules,
2026-10-11-ISSUE_32_FREE_MODEL_RULES.md) is built on top of this work.

## Todo
- [x] A1 Free.ai `{"models":[…]}` envelope in `parse_models_response`
      (+ optional `name`, and explicit `type` → capability mapping for
      `embeddings`, `image`, `tts`, `vision` only — a field the API declares)
- [x] A2 `model_discovery` setup parameter on the OpenAI-compatible factory;
      discovery off → `list_models` makes no request; health: any reply other
      than 401/403/429/5xx counts as reachable
- [x] A3 Manual models + `discovery_error` kept in the registry cache; Info tab
      shows the discovery error instead of "No models available"
- [x] B  Visibility: `show_new_models` + per-model enable/disable; hidden
      models leave `/v1/models`, pickers, unprefixed resolution and
      auto-routing, and explicit `provider/model` requests for a known,
      disabled model are refused (`model_disabled` outcome in auto-routing)
- [x] C  Per-model overrides: display name, context window, capabilities
      (replace set), price per 1M; `registry.get_pricing` used for real cost
      everywhere (router ~7 sites, systemone 2, finalize/embeddings/models);
      `PricingSource::Override` in OpenAPI
- [x] FastFlowLM typed provider (Local, default port 52625, OpenAI-compatible
      chat) that lists only installed models: `flm list --filter installed
      --json` when the binary is available and the base URL is loopback, else
      a scan of the model folder (`FLM_MODEL_PATH`, `~/.config/flm/`,
      `C:\Users\<u>\.flm\models\`), else the full `/v1/models` list with
      `show_new_models` defaulting off; logo per AGENTS.md (official asset or
      the generic provider icon)
- [x] Tauri commands `get_provider_model_settings`,
      `set_provider_model_settings`, `list_provider_models_managed` + TS types
      + demo mock; startup/rename/clone wiring
- [x] UI: provider "Models" tab (discovery card, model table with switches
      and override editor, "Add models"); Info tab error + "N hidden" + link
- [x] Search that filters (reporter request, approved): the Models tab
      filters rows by id/name with "N of M models", and Enable/Disable act on
      the filtered rows; the Info tab model list filters too; the command
      palette's model results open the provider with its list filtered to
      that model (they only opened the provider page before)
- [x] Tests (see Plan §3), plan review, test coverage review, bug hunt, commit

## Current behaviour (verified)
- `openai_compatible::list_models` → `GET {base}/models`; parses only
  `{"data":[…]}`, bare arrays and `{"result":[…]}`; every model becomes
  `context_window 4096, [Chat, Completion]` + catalog enrichment by name, so
  unknown models lack FunctionCalling and are skipped for tool requests.
- `health_check` also uses `GET /models` → servers without it show Unhealthy.
- `get_pricing` always free. ~13 preset types share this provider.
- `ModelListSource` is declared on factories but never read.
- No manual model list, no per-provider hide/disable, no per-model override.
- `list_provider_models_cached`: TTL cache → live list → catalog fallback →
  stale cache → `Err`; `list_all_models*` drop erroring providers; the Info
  tab silently shows `[]`.
- Unprefixed requests: `find_provider_for_model` calls `list_models()` live
  on every candidate provider per request.
- `pricing_overrides` is keyed by provider *type*, UI-only, unused for cost.
- FastFlowLM (`ROCm/FastFlowLM`): `/v1/models` and `/api/tags` list every
  supported model with no installed flag; requesting a missing model starts
  a blocking multi-GB download; `flm list --filter installed --json` knows.
- Free.ai `GET /v1/models` → `{"models":[{"id","name","type","pricing":{…}}]}`.

## Design
1. Effective list = discovered + manual entries, overrides applied, then
   visibility. Ids that are neither discovered nor manual still pass through
   for explicit `provider/model` requests (as today).
2. Config: `ProviderConfig.models: ProviderModelSettings` (`#[serde(default,
   skip_serializing_if = is_default)]`) with `show_new_models` (default true,
   hand-written Default) and `entries: Vec<ModelEntry{id, manual, enabled:
   Option<bool>, display_name, context_window, capabilities:
   Option<Vec<ModelCapabilityKind>>, pricing: Option<ModelPricingOverride>}>`.
   Additive, no CONFIG_VERSION bump. `ModelCapabilityKind` lives in lr-config
   with `From` into `lr_providers::Capability`.
3. Pure overlay `crates/lr-providers/src/model_settings.rs::apply(discovered,
   settings, instance) -> Vec<ManagedModel{info, source:
   Discovered|Manual|ManualAndDiscovered|Missing, enabled, has_overrides}>`.
4. Registry: `ProviderInstance.model_settings`, `set_model_settings`
   (recompute from cached raw list without a fetch), `ModelCache{raw,
   discovery_error, models}`, keep manual models on discovery errors,
   `managed_models`, `is_model_disabled`, `get_pricing` (override first).
5. Router: `find_provider_for_model` uses the registry cache; refuse disabled
   models; auto-routing skips them; pricing through the registry.
6. Price override > $0 on a local/always-free provider makes that model
   count as not free for free-only strategies (maintainer decision; #32
   evaluates per-model freedom — coordinate there).

## Risks
- Unprefixed resolution moves from live listing to cache-TTL freshness.
- Refusing disabled models on explicit requests is new behaviour (intended).
- Pricing refactor touches hot paths; keep `registry.get_pricing` lock-light.
- Downgrading drops the `models` block on save (release note).

## Implementation notes (as built)
- Config: `ProviderConfig.models` (`ProviderModelSettings{show_new_models:
  Option<bool>, entries: Vec<ModelEntry>}`), skipped on save while empty.
  `show_new_models` is an `Option` (not a hand-written `true` default) so
  "provider default" stays distinguishable: on, except a listing the
  provider marks `new_models_hidden` (FastFlowLM's full catalog fallback),
  where it is off until the user turns it on. `ModelCapabilityKind` has a
  `#[serde(other)] Unknown` so a newer capability never breaks loading;
  `Capability::from_kind`/`kind` convert. FastFlowLM is
  `ProviderType::FastFlowLM` (`fastflowlm`).
- Providers: `ModelProvider::list_model_listing() -> ModelListing{models,
  new_models_hidden}` (default wraps `list_models`). **#32 hook**: add the
  per-model raw listing metadata as a field of `ModelListing`; the registry
  caches the whole `ModelListing` (`ModelCache.listing`), so it is kept
  across settings changes and failed refreshes without further plumbing.
- Overlay: `lr_providers::model_settings::apply` (pure) →
  `ManagedModel{info, source, enabled, has_overrides}`; manual models are
  catalog-enriched by name, then overridden.
- Registry: `ModelCache{listing, discovery_error, managed, models}`; failed
  listings fall back to catalog → previous listing → empty, are cached for at
  most 60 s, and keep manual models; `list_provider_models_cached` errs only
  when listing failed and nothing is offered. API: `get/set_model_settings`
  (re-filters the cached listing, no fetch), `managed_models`,
  `is_model_disabled` (known ids only; exact then case-insensitive),
  `pricing_override(instance, model)` (**the per-instance price API for
  #32**), `get_pricing(instance, model)` (override → provider → free).
  `update_provider` keeps the settings and drops the cached list.
- Router: unprefixed resolution reads the registry cache (fetch only when
  missing/expired) and skips hidden models and hidden strategy-selected
  pairs; `require_enabled_model` refuses known disabled models on every
  execution path (chat, stream, embeddings, audio, System One) with
  `InvalidParams` (HTTP 400); auto-routing loops skip them (`model_disabled`
  outcome for chat/stream; decision routing falls back with
  `decision_model_disabled`).
- Decision 6 implemented directly: `Router::classify_model_free` returns
  `NotFree` for a model with a price override > $0 on an `AlwaysFreeLocal`
  or `Subscription` provider; all 8 former `classify_model` call sites use
  it. `TODO(#32)` marks it for replacement by the per-model classifier.
- FastFlowLM detection verified against upstream source (ROCm/FastFlowLM
  `src/src/main.cpp`, `src/common/utils.cpp`, `src/include/model_list.hpp`):
  `flm list --filter installed --json` prints `{"models":[{<catalog record>,
  "name":"<family>:<size>","installed":true}]}` (status lines may precede
  it); model folder = `FLM_MODEL_PATH`, else `%USERPROFILE%\.flm` (Windows)
  / `$HOME/.config/flm` (every other OS — there is no separate macOS path),
  models at `<folder>/<catalog model_path = "models">/<record name>/` and
  present when `config.json` exists; catalog `model_list.json` from
  `FLM_CONFIG_PATH`, next to `flm`, `<flm dir>/../share/flm/`, Windows
  `%ProgramFiles%\flm` / `%LOCALAPPDATA%\flm`, Linux
  `/opt/fastflowlm/share/flm` (deb), `/usr/share/flm`, `/usr/local/share/flm`.
  Capabilities from the record's declared `label` list (`embeddings`,
  `transcription`, `vision`, `tool-calling`) and `vlm`; context from
  `default_context_length`. Stream usage is not requested (not verified).
- Logo: official `assets/logo.png` (pinned revision, SHA-256 in
  `public/icons/SOURCES.md`), MIT licence bundled.
- UI: the Models tab shows for every non-embedded provider (visibility and
  overrides are generic); the "Discover models" switch only where the type
  has the `model_discovery` setup parameter (OpenAI-compatible,
  FastFlowLM), and it is moved out of the Settings tab. Boolean setup
  parameters now honour their `default_value` in the forms.

## Verification
- `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --
  -D warnings` (rustup stable 1.99): clean.
- `cargo test -p lr-config -p lr-providers -p lr-router -p lr-server` and
  `cargo test -p localrouter --lib --test router_strategy_tests`: pass.
  New tests: config round-trip/normalize/unknown capability; overlay
  (`model_settings::tests`); registry (re-filter without refetch, manual
  models survive failures, stale list kept, untrusted listing, disabled
  detection, price override, update/remove); OpenAI-compatible (Free.ai
  envelope, kind mapping, health statuses, 404 healthy, discovery off makes
  no request, FastFlowLM fallback hides its catalog); FastFlowLM (loopback,
  CLI output with status lines, record → capabilities, folder scan,
  remote skipped); factory (discovery switch, FastFlowLM metadata); router
  (explicit refusal, unknown ids pass, unprefixed + selected-pair skipping,
  auto-routing `model_disabled`, price override cost, priced local model
  not free); server (`/v1/models` override pricing); Tauri (instance
  override beats type override).
- `npx tsc --noEmit` (app and website): clean.
- Website demo, headless Chromium: Models tab light/dark, active filter,
  override editor, Info tab, command palette → filtered provider page,
  FastFlowLM; scripted interactions (filter + Enable/Disable shown, Add,
  Remove via AlertDialog, Offer new models off, Discover models off) give
  the expected counts.

## Final steps
- [x] Plan review: every Todo item and design point checked against the
      code; decision 6 implemented (not deferred).
- [x] Test coverage review: tests added for each new path listed above.
- [x] Bug hunt: fixed a race where a model list fetched with old settings
      could overwrite a concurrent settings change (settings now read under
      the cache lock); hidden strategy-selected pairs were still preferred
      by unprefixed resolution (now skipped); embeddings cost used the
      `provider/model` string as the model id (now split).
- [x] Commit (no push, per instruction).

