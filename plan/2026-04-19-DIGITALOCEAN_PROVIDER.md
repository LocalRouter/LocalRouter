# DigitalOcean Gradient Serverless Inference Provider

**Date:** 2026-04-19
**Branch:** `feat/digitalocean-provider`
**Scope:** Add a first-class `digitalocean` provider for DigitalOcean's Gradient
serverless inference platform so users don't have to hand-configure the generic
OpenAI-compatible adapter.

## Problem

Users adding DigitalOcean via the Custom (generic OpenAI-compatible) provider
must know:
1. The base URL must end in `/v1` (`https://inference.do-ai.run/v1`) or the
   model list will 404.
2. The key must be a **Gradient Model Access Key**, not a Personal Access Token.
3. There's no catalog/pricing enrichment for the returned model IDs.

These are easy to get wrong, and the generic-provider error messages don't
guide the user. A typed factory encodes the URL, writes the right Bearer
header through `OpenAICompatibleProvider`, and hooks into the same model-cache
and catalog-fallback machinery every other third-party provider uses.

## Discovery

- `https://inference.do-ai.run/v1` is documented in DigitalOcean's serverless
  inference guide and matches the Gradient Python SDK's default
  `GRADIENT_INFERENCE_ENDPOINT` (see `gradientai-python` README).
- `crates/lr-catalog/catalog/modelsdev_raw.json` already ships a
  `"digitalocean"` entry with `"api": "https://inference.do-ai.run/v1"` and 20+
  models (chat + embeddings), so `catalog_provider_id("digitalocean")` wires up
  pricing/context enrichment and catalog fallback for free.
- `OpenAICompatibleProvider` hits `GET {base_url}/models`,
  `POST {base_url}/chat/completions`, `POST {base_url}/embeddings`, and
  streams SSE the OpenAI way — exactly what DO's endpoint speaks.

## Design

Mirror `NvidiaNimProviderFactory`:

- Fixed base URL `https://inference.do-ai.run/v1` (no user knob to get wrong).
- Single required setup parameter: `api_key` (the Gradient Model Access Key).
- Category: `ThirdParty`.
- Free tier: `RateLimitedFree` with conservative defaults (DO publishes limits
  but treats the exact numbers as plan-dependent — router auto-skips on 429
  regardless).
- `catalog_provider_id()` returns `Some("digitalocean")` so:
  - The UI shows pricing/context for each returned model ID.
  - `list_models()` failures fall back to the catalog.
- `ModelListSource`: default (`ApiWithCatalogFallback`) — API is source of
  truth, catalog fills gaps when the API is down.

Caching, health checks, and the `/v1/models` aggregation are all inherited
from `ProviderRegistry` — no new cache plumbing needed.

## Changes

### Backend
1. `crates/lr-providers/src/factory.rs`
   - Add `DigitalOceanProviderFactory` after `ZhipuProviderFactory`.
   - Factory tests (metadata, free-tier, setup params, create success,
     create-missing-key failure).
   - Extend cross-cutting aggregate tests (unique provider types, notes
     present, catalog mapping).

2. `crates/lr-config/src/types.rs`
   - Add `ProviderType::DigitalOcean` with `#[serde(rename = "digitalocean")]`.
   - Extend enum roundtrip tests.

3. `src-tauri/src/main.rs`
   - Import `DigitalOceanProviderFactory`.
   - `provider_registry.register_factory(Arc::new(DigitalOceanProviderFactory));`

### Frontend
4. `src/components/ServiceIcon.tsx`
   - Map `digitalocean` to an icon (fallback to generic providers icon until
     a real asset lands — matches how other no-asset services degrade).

No TypeScript `tauri-commands.ts` changes — the provider is data-driven via
`list_provider_types()`, which already flows through existing commands.

## Acceptance Criteria

- [ ] `DigitalOceanProviderFactory::create` with `{api_key: "do_model_abc"}`
      produces a provider that `name()`s `"digitalocean"` and whose base URL
      is `https://inference.do-ai.run/v1`.
- [ ] `validate_config` rejects a missing API key with a helpful error.
- [ ] Registry-level test: type string `"digitalocean"` is unique across
      factories.
- [ ] `cargo test --workspace` passes.
- [ ] `rustup run stable cargo clippy --workspace --all-targets -- -D warnings`
      passes.
- [ ] `rustup run stable cargo fmt --all -- --check` passes.

## Final Steps (mandatory)

1. **Plan Review** — Re-read plan against diff, ensure every described change
   exists.
2. **Test Coverage Review** — Every branch of the new factory code hit by
   tests.
3. **Bug Hunt** — Re-check base URL (no trailing slash), enum serde rename,
   factory registration order doesn't shadow anything, icon fallback path.
4. **Commit** — Stage only modified files, conventional-commit message.
