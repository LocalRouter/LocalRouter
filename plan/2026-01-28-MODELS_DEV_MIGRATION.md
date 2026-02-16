# Plan: Migrate Model Catalog from OpenRouter to models.dev

## Overview

Switch from OpenRouter API to models.dev API for sourcing model information and pricing. This enables:
1. **Provider-catalog mapping**: Each provider exposes its models.dev provider ID for catalog matching
2. **Catalog-based model lists**: Providers without model APIs (e.g., Perplexity) use catalog as model source
3. **Rich UI metadata**: Display new fields (reasoning, tool_call, knowledge cutoff, etc.) in UI

Maintains privacy-first design (build-time only, no runtime network requests).

## Key Differences

| Aspect | OpenRouter | models.dev |
|--------|------------|------------|
| Structure | Flat array | Nested provider → models |
| Model ID | `provider/model` | Model name within provider object |
| Pricing | Per token (e.g., `0.000015`) | Per million tokens (e.g., `15`) |
| Modality | Single string | Input/output arrays |
| New fields | - | `reasoning`, `tool_call`, `structured_output`, cache pricing, `knowledge` cutoff |

## Files to Modify

### Build-time (src-tauri/buildtools/)

1. **`models.rs`** - Complete rewrite for models.dev schema
   - New types: `ModelsDevResponse`, `ModelsDevProvider`, `ModelsDevModel`, `Cost`, `Limits`, `Modalities`
   - Price conversion: `cost.input / 1_000_000.0` for per-token pricing

2. **`scraper.rs`** - Update fetching logic
   - Change endpoint: `https://models.dev/api.json`
   - Change cache file: `catalog/modelsdev_raw.json`
   - Flatten nested structure: `provider_id/model_id` format for IDs

3. **`codegen.rs`** - Update code generation
   - Add new fields: `capabilities`, `knowledge_cutoff`, `open_weights`
   - Remove deprecated: `supported_parameters`, `image_per_token`, `request_cost`

### Runtime (src-tauri/src/catalog/)

4. **`types.rs`** - Extend data structures
   - Add `CatalogCapabilities` struct (`reasoning`, `tool_call`, `structured_output`, `vision`)
   - Add cache pricing: `cache_read_per_token`, `cache_write_per_token`
   - Add metadata: `knowledge_cutoff`, `open_weights`, `max_output_tokens`
   - Remove deprecated: `created`, `supported_parameters`, `image_per_token`, `request_cost`

5. **`matcher.rs`** - No changes (operates on `id` and `aliases`)

6. **`mod.rs`** - Export new types

## Implementation Steps

### Step 1: Update Build-time Types
```rust
// buildtools/models.rs
pub struct ModelsDevModel {
    pub id: String,
    pub name: String,
    pub reasoning: bool,
    pub tool_call: bool,
    pub structured_output: bool,
    pub modalities: Modalities,
    pub cost: Cost,           // per million tokens
    pub limit: Limits,
    pub knowledge: Option<String>,
    pub open_weights: bool,
}

impl ModelsDevModel {
    pub fn input_cost_per_token(&self) -> f64 {
        self.cost.input / 1_000_000.0
    }
}
```

### Step 2: Update Scraper
```rust
// buildtools/scraper.rs
const MODELS_DEV_API: &str = "https://models.dev/api.json";
const CACHE_FILE: &str = "catalog/modelsdev_raw.json";

// Flatten nested structure into "provider/model" format
fn flatten_models(response: ModelsDevResponse) -> Vec<FlattenedModel> {
    response.providers.into_iter()
        .flat_map(|(provider_id, provider)| {
            provider.models.into_iter().map(move |(model_id, model)| {
                FlattenedModel {
                    full_id: format!("{}/{}", provider_id, model_id),
                    provider_id: provider_id.clone(),
                    model,
                }
            })
        })
        .collect()
}
```

### Step 3: Update Runtime Types
```rust
// src/catalog/types.rs
pub struct CatalogCapabilities {
    pub reasoning: bool,
    pub tool_call: bool,
    pub structured_output: bool,
    pub vision: bool,
}

pub struct CatalogModel {
    pub id: &'static str,
    pub aliases: &'static [&'static str],
    pub name: &'static str,
    pub context_length: u32,
    pub max_output_tokens: Option<u32>,
    pub modality: Modality,
    pub capabilities: CatalogCapabilities,
    pub pricing: CatalogPricing,
    pub knowledge_cutoff: Option<&'static str>,
    pub open_weights: bool,
}

pub struct CatalogPricing {
    pub prompt_per_token: f64,
    pub completion_per_token: f64,
    pub cache_read_per_token: Option<f64>,
    pub cache_write_per_token: Option<f64>,
    pub currency: &'static str,
}
```

### Step 4: Update Code Generator
- Generate new fields in `catalog/catalog.rs`
- Update alias generation for models.dev ID format
- Detect vision capability from `modalities.input.contains("image")`

### Step 5: Update Tests
- Fix test fixtures in `matcher.rs` to match new struct
- Update integration tests in `mod.rs`
- Add price conversion tests

## Verification

1. **Build test**: `LOCALROUTER_REBUILD_CATALOG=1 cargo build`
2. **Unit tests**: `cargo test -p localrouter-ai catalog`
3. **Verify generated catalog**: Check `catalog/catalog.rs` has models with correct pricing
4. **Manual verification**: Compare a few known models (gpt-4, claude-opus-4) pricing against models.dev website

## Part 2: Provider-Catalog Mapping

### Add `catalog_provider_id()` to ProviderFactory trait

```rust
// src-tauri/src/providers/factory.rs
pub trait ProviderFactory: Send + Sync {
    fn provider_type(&self) -> &str;
    fn display_name(&self) -> &str;

    /// models.dev provider ID for catalog matching (None if no mapping)
    fn catalog_provider_id(&self) -> Option<&str> {
        Some(self.provider_type())  // Default: same as provider_type
    }
    // ...existing methods...
}
```

### Provider ID Mapping Table

| LocalRouter provider_type | models.dev provider ID | Notes |
|--------------------------|------------------------|-------|
| `openai` | `openai` | Direct match |
| `anthropic` | `anthropic` | Direct match |
| `gemini` | `google` | Different name |
| `mistral` | `mistral` | Direct match |
| `perplexity` | `perplexity` | Direct match |
| `cohere` | `cohere` | Direct match |
| `groq` | `groq` | Direct match |
| `xai` | `xai` | Direct match |
| `deepinfra` | `deepinfra` | Direct match |
| `togetherai` | `together` | Different name |
| `cerebras` | `cerebras` | Direct match |
| `ollama` | `None` | Local, no catalog |
| `lmstudio` | `None` | Local, no catalog |
| `openai_compatible` | `None` | Generic, no catalog |
| `openrouter` | `None` | Aggregator, uses model name matching |

### Providers Needing Override

```rust
// gemini.rs
fn catalog_provider_id(&self) -> Option<&str> {
    Some("google")  // models.dev uses "google" not "gemini"
}

// togetherai.rs
fn catalog_provider_id(&self) -> Option<&str> {
    Some("together")  // models.dev uses "together"
}

// ollama.rs, lmstudio.rs, openai_compatible.rs
fn catalog_provider_id(&self) -> Option<&str> {
    None  // Use name-based matching instead
}
```

## Part 3: Catalog-Based Model Lists (Configurable Per-Provider)

### Add Model Source Strategy to ProviderFactory

```rust
// src-tauri/src/providers/factory.rs

/// Where a provider gets its model list from
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelListSource {
    /// Use provider's API, fall back to catalog if API fails/empty
    ApiWithCatalogFallback,
    /// Use catalog as primary source (no API available)
    CatalogOnly,
    /// Use provider's API only, no catalog fallback
    ApiOnly,
}

pub trait ProviderFactory: Send + Sync {
    // ...existing methods...

    /// How this provider gets its model list
    fn model_list_source(&self) -> ModelListSource {
        ModelListSource::ApiWithCatalogFallback  // Default behavior
    }
}
```

### Provider Model Source Configuration

| Provider | model_list_source | Reason |
|----------|------------------|--------|
| `perplexity` | `CatalogOnly` | No public `/models` endpoint |
| `ollama` | `ApiOnly` | Local models, catalog irrelevant |
| `lmstudio` | `ApiOnly` | Local models, catalog irrelevant |
| `openai_compatible` | `ApiOnly` | Generic, catalog irrelevant |
| All others | `ApiWithCatalogFallback` | API primary, catalog backup |

### Add `get_provider_models()` function

```rust
// src-tauri/src/catalog/mod.rs
/// Get all models for a specific provider from the embedded catalog
pub fn get_provider_models(provider_id: &str) -> Vec<&'static CatalogModel> {
    CATALOG_MODELS
        .iter()
        .filter(|m| m.id.starts_with(&format!("{}/", provider_id)))
        .collect()
}
```

### Update Provider Base Implementation

Add helper in `ModelProvider` trait or providers/mod.rs:

```rust
// src-tauri/src/providers/mod.rs
impl ModelInfo {
    /// Create ModelInfo from a catalog model
    pub fn from_catalog(cm: &CatalogModel, provider: &str) -> Self {
        let model_id = cm.id
            .split_once('/')
            .map(|(_, id)| id)
            .unwrap_or(cm.id);

        ModelInfo {
            id: model_id.to_string(),
            name: cm.name.to_string(),
            provider: provider.to_string(),
            context_window: cm.context_length,
            supports_streaming: true,
            capabilities: Self::capabilities_from_catalog(cm),
            detailed_capabilities: None,
        }
    }

    fn capabilities_from_catalog(cm: &CatalogModel) -> Vec<Capability> {
        let mut caps = vec![Capability::Chat];
        if cm.capabilities.vision {
            caps.push(Capability::Vision);
        }
        caps
    }
}
```

### Update Perplexity Provider

```rust
// src-tauri/src/providers/perplexity.rs
async fn list_models(&self) -> AppResult<Vec<ModelInfo>> {
    // Use catalog models (CatalogOnly strategy)
    let catalog_models = crate::catalog::get_provider_models("perplexity");

    if !catalog_models.is_empty() {
        return Ok(catalog_models.iter()
            .map(|cm| ModelInfo::from_catalog(cm, "perplexity"))
            .collect());
    }

    // Fallback to hardcoded models if catalog doesn't have perplexity
    Ok(Self::get_known_models())
}
```

### Example: Provider with ApiWithCatalogFallback

```rust
// src-tauri/src/providers/cohere.rs (or any other)
async fn list_models(&self) -> AppResult<Vec<ModelInfo>> {
    // Try API first
    match self.fetch_models_from_api().await {
        Ok(models) if !models.is_empty() => Ok(models),
        _ => {
            // Fall back to catalog
            let catalog_models = crate::catalog::get_provider_models("cohere");
            if !catalog_models.is_empty() {
                tracing::info!("Using catalog fallback for Cohere models");
                Ok(catalog_models.iter()
                    .map(|cm| ModelInfo::from_catalog(cm, "cohere"))
                    .collect())
            } else {
                // Neither worked
                Err(AppError::Provider("No models available".into()))
            }
        }
    }
}
```

## Part 4: UI Enhancements

### Extend DetailedModelInfo (Backend)

```rust
// src-tauri/src/ui/commands.rs
#[derive(Serialize)]
pub struct DetailedModelInfo {
    // ...existing fields...

    // New fields from models.dev
    pub reasoning: bool,
    pub tool_call: bool,
    pub structured_output: bool,
    pub vision: bool,
    pub knowledge_cutoff: Option<String>,
    pub open_weights: bool,
    pub cache_read_price_per_million: Option<f64>,
    pub cache_write_price_per_million: Option<f64>,
    pub max_output_tokens: Option<u32>,
    pub pricing_source: String,  // "catalog" | "provider" | "fallback"
}
```

### Update Models Panel UI

```tsx
// src/views/resources/models-panel.tsx
interface Model {
  // ...existing fields...

  // New fields
  reasoning?: boolean
  tool_call?: boolean
  structured_output?: boolean
  vision?: boolean
  knowledge_cutoff?: string
  open_weights?: boolean
  cache_read_price_per_million?: number
  cache_write_price_per_million?: number
  max_output_tokens?: number
  pricing_source?: string
}

// Display capability badges
{selectedModel.reasoning && <Badge variant="secondary">Reasoning</Badge>}
{selectedModel.tool_call && <Badge variant="secondary">Tool Calling</Badge>}
{selectedModel.structured_output && <Badge variant="secondary">Structured Output</Badge>}
{selectedModel.vision && <Badge variant="secondary">Vision</Badge>}
{selectedModel.open_weights && <Badge variant="outline">Open Weights</Badge>}

// Display knowledge cutoff
{selectedModel.knowledge_cutoff && (
  <div>
    <p className="text-muted-foreground">Knowledge Cutoff</p>
    <p className="font-medium">{selectedModel.knowledge_cutoff}</p>
  </div>
)}

// Display cache pricing
{(selectedModel.cache_read_price_per_million || selectedModel.cache_write_price_per_million) && (
  <div>
    <p className="text-muted-foreground">Cache Pricing/M</p>
    <p className="font-medium">
      {formatPrice(selectedModel.cache_read_price_per_million)} read /
      {formatPrice(selectedModel.cache_write_price_per_million)} write
    </p>
  </div>
)}
```

### Add Capability Filters

Update filter dropdown to include new capability types:

```tsx
const CAPABILITY_GROUPS = {
  "Core": ["Chat", "Completion", "Embedding"],
  "Advanced": ["Reasoning", "Tool Calling", "Structured Output"],
  "Multimodal": ["Vision", "Audio", "PDF"],
}
```

## Implementation Order

1. **Phase 1: Catalog Migration** (no breaking changes)
   - Update build-time types for models.dev schema
   - Update scraper to fetch from models.dev API
   - Update codegen to generate new fields
   - Update runtime types with new fields (defaults for backward compat)

2. **Phase 2: Provider Integration**
   - Add `catalog_provider_id()` to ProviderFactory trait
   - Implement overrides for gemini, togetherai
   - Add `get_provider_models()` catalog function
   - Update Perplexity to use catalog models

3. **Phase 3: UI Enhancements**
   - Extend DetailedModelInfo with new fields
   - Update models-panel.tsx to display new metadata
   - Add capability badges and filters

## Verification

1. **Build test**: `LOCALROUTER_REBUILD_CATALOG=1 cargo build`
2. **Unit tests**: `cargo test -p localrouter-ai catalog`
3. **Verify generated catalog**: Check `catalog/catalog.rs` has models with correct pricing
4. **Manual verification**:
   - Compare pricing against models.dev website
   - Check Perplexity shows catalog models
   - Verify UI displays new capability badges

## Rollback

Keep `openrouter_raw.json` for one release cycle. If issues arise:
1. Revert scraper endpoint to OpenRouter
2. Restore old types in `models.rs`

## Notes

- models.dev is community-maintained; may have slightly different coverage than OpenRouter
- Provider IDs generally match (openai, anthropic, google, mistral)
- Cache mechanism (7-day) remains unchanged
