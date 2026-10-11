# Issue #33 — custom provider model discovery

Approved by the maintainer on 2026-10-11: all phases (A+B+C) plus a typed
FastFlowLM provider. Issue #32 (free-model rules,
2026-10-11-ISSUE_32_FREE_MODEL_RULES.md) is built on top of this work.

## Todo
- [ ] A1 Free.ai `{"models":[…]}` envelope in `parse_models_response`
      (+ optional `name`, and explicit `type` → capability mapping for
      `embeddings`, `image`, `tts`, `vision` only — a field the API declares)
- [ ] A2 `model_discovery` setup parameter on the OpenAI-compatible factory;
      discovery off → `list_models` makes no request; health: any reply other
      than 401/403/429/5xx counts as reachable
- [ ] A3 Manual models + `discovery_error` kept in the registry cache; Info tab
      shows the discovery error instead of "No models available"
- [ ] B  Visibility: `show_new_models` + per-model enable/disable; hidden
      models leave `/v1/models`, pickers, unprefixed resolution and
      auto-routing, and explicit `provider/model` requests for a known,
      disabled model are refused (`model_disabled` outcome in auto-routing)
- [ ] C  Per-model overrides: display name, context window, capabilities
      (replace set), price per 1M; `registry.get_pricing` used for real cost
      everywhere (router ~7 sites, systemone 2, finalize/embeddings/models);
      `PricingSource::Override` in OpenAPI
- [ ] FastFlowLM typed provider (Local, default port 52625, OpenAI-compatible
      chat) that lists only installed models: `flm list --filter installed
      --json` when the binary is available and the base URL is loopback, else
      a scan of the model folder (`FLM_MODEL_PATH`, `~/.config/flm/`,
      `C:\Users\<u>\.flm\models\`), else the full `/v1/models` list with
      `show_new_models` defaulting off; logo per AGENTS.md (official asset or
      the generic provider icon)
- [ ] Tauri commands `get_provider_model_settings`,
      `set_provider_model_settings`, `list_provider_models_managed` + TS types
      + demo mock; startup/rename/clone wiring
- [ ] UI: provider "Models" tab (discovery card, model table with switches
      and override editor, "Add models"); Info tab error + "N hidden" + link
- [ ] Tests (see Plan §3), plan review, test coverage review, bug hunt, commit

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
