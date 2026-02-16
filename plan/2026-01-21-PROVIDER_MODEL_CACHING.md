# Implementation Plan: Provider-Specific Model Information with Caching

## Overview

Implement dynamic model fetching from provider-specific API endpoints with intelligent caching and OpenRouter catalog fallback. This ensures fresh model information while minimizing API calls and maintaining offline capability.

## Current State Analysis

### Providers Status

**Already Dynamic** (6 providers):
- OpenAI - `GET /v1/models`
- Gemini - `GET /v1beta/models`
- OpenRouter - `GET /api/v1/models`
- Ollama - `GET /api/tags`
- LMStudio - `GET /v1/models`
- OpenAI-compatible - `GET /models`

**Hardcoded but API Available** (6 providers):
- Anthropic - `GET /v1/models` ✅ Available
- Mistral - `GET /v1/models` ✅ Available
- Groq - `GET /openai/v1/models` ✅ Available
- Cohere - `GET /v1/models` ✅ Available
- TogetherAI - `GET /v1/models` ✅ Available
- Cerebras - `GET /v1/models` ✅ Available

**No API Endpoint** (3 providers):
- Perplexity - Keep hardcoded
- xAI - Keep hardcoded
- DeepInfra - Keep hardcoded

### OpenRouter Catalog

- 339 models embedded at build time
- Provides: pricing, context window, capabilities, modality
- Zero runtime network requests
- Enrichment methods: `enrich_with_catalog(provider)`, `enrich_with_catalog_by_name()`

## Implementation Approach

### Three-Tier Fallback System

1. **Cache** - Check per-provider cache (configurable TTL)
2. **API** - Fetch from provider endpoint if cache expired
3. **Fallback** - Use OpenRouter catalog → stale cache → error

### Cache Design

**Per-Provider Caching** with individual TTL configuration:
- Local providers (Ollama, LMStudio): 5 minutes
- Cloud providers with API: 1 hour
- Hardcoded providers: 24 hours
- Stale cache: Used indefinitely if API fails

**Cache Structure**:
```rust
struct ModelCache {
    models: Vec<ModelInfo>,
    fetched_at: DateTime<Utc>,
    ttl_seconds: u64,
}
```

## Critical Files to Modify

1. **src-tauri/src/config/mod.rs** - Add `ModelCacheConfig`
2. **src-tauri/src/providers/registry.rs** - Core cache infrastructure
3. **src-tauri/src/providers/anthropic.rs** - Add dynamic fetching
4. **src-tauri/src/providers/mistral.rs** - Add dynamic fetching
5. **src-tauri/src/providers/groq.rs** - Add dynamic fetching
6. **src-tauri/src/providers/cohere.rs** - Add dynamic fetching
7. **src-tauri/src/providers/togetherai.rs** - Add dynamic fetching
8. **src-tauri/src/providers/cerebras.rs** - Add dynamic fetching

## Step-by-Step Implementation

### Phase 1: Infrastructure (No Breaking Changes)

**Step 1.1: Add Configuration** (`src-tauri/src/config/mod.rs`)

Add before `AppConfig` struct:
```rust
/// Model cache configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModelCacheConfig {
    /// Default TTL for model cache (seconds)
    #[serde(default = "default_model_cache_ttl")]
    pub default_ttl_seconds: u64,

    /// Per-provider TTL overrides
    #[serde(default)]
    pub provider_ttl_overrides: HashMap<String, u64>,

    /// Whether to use OpenRouter catalog as fallback
    #[serde(default = "default_true")]
    pub use_catalog_fallback: bool,
}

fn default_model_cache_ttl() -> u64 {
    3600 // 1 hour
}

fn default_true() -> bool {
    true
}

impl Default for ModelCacheConfig {
    fn default() -> Self {
        Self {
            default_ttl_seconds: 3600,
            provider_ttl_overrides: HashMap::new(),
            use_catalog_fallback: true,
        }
    }
}
```

Add to `AppConfig` struct:
```rust
/// Model cache configuration
#[serde(default)]
pub model_cache: ModelCacheConfig,
```

**Step 1.2: Add Cache Structure** (`src-tauri/src/providers/registry.rs`)

Add before `ProviderRegistry`:
```rust
use std::collections::HashMap;
use chrono::{DateTime, Utc, Duration};

/// Cached model list for a single provider
#[derive(Clone)]
struct ModelCache {
    /// The cached models
    models: Vec<ModelInfo>,
    /// When this cache was populated
    fetched_at: DateTime<Utc>,
    /// Provider-specific TTL (from config or default)
    ttl_seconds: u64,
}

impl ModelCache {
    fn new(models: Vec<ModelInfo>, ttl_seconds: u64) -> Self {
        Self {
            models,
            fetched_at: Utc::now(),
            ttl_seconds,
        }
    }

    /// Check if cache is expired
    fn is_expired(&self) -> bool {
        let elapsed = Utc::now().signed_duration_since(self.fetched_at);
        elapsed.num_seconds() as u64 >= self.ttl_seconds
    }

    /// Get remaining seconds until expiration
    fn expires_in(&self) -> i64 {
        let elapsed = Utc::now().signed_duration_since(self.fetched_at);
        (self.ttl_seconds as i64) - elapsed.num_seconds()
    }
}
```

Add to `ProviderRegistry` struct:
```rust
/// Per-provider model cache
model_cache: Arc<RwLock<HashMap<String, ModelCache>>>,

/// Cache configuration
cache_config: Arc<RwLock<ModelCacheConfig>>,
```

Update `ProviderRegistry::new()`:
```rust
pub fn new(health_manager: Arc<HealthCheckManager>) -> Self {
    info!("Creating new provider registry");
    Self {
        factories: RwLock::new(HashMap::new()),
        instances: RwLock::new(HashMap::new()),
        health_manager,
        cached_models: RwLock::new(Vec::new()),
        model_cache: Arc::new(RwLock::new(HashMap::new())),
        cache_config: Arc::new(RwLock::new(ModelCacheConfig::default())),
    }
}
```

**Step 1.3: Add Cache Management Methods** (`src-tauri/src/providers/registry.rs`)

Add to `ProviderRegistry` impl block:
```rust
/// Get TTL for a specific provider instance
fn get_provider_ttl(&self, instance_name: &str) -> u64 {
    let config = self.cache_config.read();

    // Check instance-specific override first
    if let Some(&ttl) = config.provider_ttl_overrides.get(instance_name) {
        return ttl;
    }

    // Check provider-type override
    if let Some(instance) = self.instances.read().get(instance_name) {
        if let Some(&ttl) = config.provider_ttl_overrides.get(&instance.provider_type) {
            return ttl;
        }
    }

    // Use default
    config.default_ttl_seconds
}

/// Get cached models for a provider
fn get_cached_models_for_provider(&self, instance_name: &str) -> Option<ModelCache> {
    self.model_cache.read().get(instance_name).cloned()
}

/// Update cache for a provider
async fn update_model_cache(&self, instance_name: &str, models: Vec<ModelInfo>) {
    let ttl = self.get_provider_ttl(instance_name);
    let cache = ModelCache::new(models, ttl);
    self.model_cache.write().insert(instance_name.to_string(), cache);
    debug!("Updated model cache for '{}' (TTL: {}s)", instance_name, ttl);
}

/// Get models from OpenRouter catalog for a provider type
fn get_models_from_catalog(&self, provider_type: &str) -> Vec<ModelInfo> {
    crate::catalog::models()
        .iter()
        .filter(|m| m.id.starts_with(&format!("{}/", provider_type)))
        .map(|m| ModelInfo {
            id: m.id.split('/').last().unwrap_or(&m.id).to_string(),
            name: m.name.to_string(),
            provider: provider_type.to_string(),
            parameter_count: None,
            context_window: m.context_length,
            supports_streaming: true,
            capabilities: match m.modality {
                crate::catalog::Modality::Multimodal => {
                    vec![Capability::Chat, Capability::Vision]
                }
                _ => vec![Capability::Chat],
            },
            detailed_capabilities: None,
        })
        .collect()
}

/// Fallback to catalog or stale cache
async fn fallback_to_catalog_or_stale_cache(
    &self,
    instance_name: &str,
    provider_type: &str,
) -> AppResult<Vec<ModelInfo>> {
    let config = self.cache_config.read();

    if config.use_catalog_fallback {
        // Try OpenRouter catalog
        let catalog_models = self.get_models_from_catalog(provider_type);
        if !catalog_models.is_empty() {
            info!(
                "Using OpenRouter catalog fallback for '{}' ({} models)",
                instance_name,
                catalog_models.len()
            );
            return Ok(catalog_models);
        }
    }

    // Use stale cache if available
    if let Some(cached) = self.get_cached_models_for_provider(instance_name) {
        warn!(
            "Using stale cache for '{}' (expired {} seconds ago)",
            instance_name,
            -cached.expires_in()
        );
        return Ok(cached.models.clone());
    }

    // Complete failure
    Err(AppError::Provider(format!(
        "No models available for '{}': API failed, no catalog data, no stale cache",
        instance_name
    )))
}

/// List models from a provider with caching
pub async fn list_provider_models_cached(
    &self,
    instance_name: &str
) -> AppResult<Vec<ModelInfo>> {
    // 1. Try cache first
    if let Some(cached) = self.get_cached_models_for_provider(instance_name) {
        if !cached.is_expired() {
            debug!(
                "Using cached models for '{}' (expires in {}s)",
                instance_name,
                cached.expires_in()
            );
            return Ok(cached.models.clone());
        }
    }

    // 2. Try fetching from provider API
    let provider_instance = self.instances.read()
        .get(instance_name)
        .cloned()
        .ok_or_else(|| AppError::Config(format!("Provider '{}' not found", instance_name)))?;

    match provider_instance.provider.list_models().await {
        Ok(models) => {
            // Success: Cache and return
            self.update_model_cache(instance_name, models.clone()).await;
            Ok(models)
        }
        Err(e) => {
            warn!(
                "Failed to fetch models from '{}': {}",
                instance_name,
                e
            );

            // 3. Fallback strategy
            self.fallback_to_catalog_or_stale_cache(
                instance_name,
                &provider_instance.provider_type
            ).await
        }
    }
}

/// Invalidate cache for a specific provider
pub fn invalidate_provider_cache(&self, instance_name: &str) {
    self.model_cache.write().remove(instance_name);
    info!("Invalidated model cache for '{}'", instance_name);
}

/// Invalidate all caches
pub fn invalidate_all_caches(&self) {
    self.model_cache.write().clear();
    info!("Invalidated all model caches");
}

/// Update cache configuration at runtime
pub fn update_cache_config(&self, config: ModelCacheConfig) {
    *self.cache_config.write() = config;
    info!("Model cache configuration updated");
}
```

Update `list_all_models()` to use caching:
```rust
pub async fn list_all_models(&self) -> AppResult<Vec<ModelInfo>> {
    let enabled_instances: Vec<String> = self.instances.read()
        .values()
        .filter(|inst| inst.enabled)
        .map(|inst| inst.instance_name.clone())
        .collect();

    let mut all_models = Vec::new();

    for instance_name in enabled_instances {
        match self.list_provider_models_cached(&instance_name).await {
            Ok(models) => {
                all_models.extend(models);
            }
            Err(e) => {
                warn!(
                    "Failed to list models from '{}': {}",
                    instance_name,
                    e
                );
            }
        }
    }

    debug!(
        "Listed {} models from all enabled providers",
        all_models.len()
    );
    Ok(all_models)
}
```

### Phase 2: Provider Updates (Dynamic Fetching)

**Step 2.1: Anthropic** (`src-tauri/src/providers/anthropic.rs`)

Add response structs after imports:
```rust
#[derive(Debug, Deserialize)]
struct AnthropicModelsResponse {
    data: Vec<AnthropicModelData>,
}

#[derive(Debug, Deserialize)]
struct AnthropicModelData {
    id: String,
    #[serde(default)]
    display_name: String,
    #[serde(default)]
    created_at: String,
}
```

Replace `list_models()` implementation:
```rust
async fn list_models(&self) -> AppResult<Vec<ModelInfo>> {
    // Try fetching from API
    match self.fetch_models_from_api().await {
        Ok(models) => Ok(models),
        Err(e) => {
            warn!("Failed to fetch Anthropic models from API, using hardcoded list: {}", e);
            Ok(Self::get_known_models())
        }
    }
}
```

Add new method in impl block:
```rust
/// Fetch models from Anthropic API
async fn fetch_models_from_api(&self) -> AppResult<Vec<ModelInfo>> {
    let response = self
        .client
        .get(format!("{}/models", self.base_url))
        .header("x-api-key", &self.api_key)
        .header("anthropic-version", ANTHROPIC_VERSION)
        .send()
        .await
        .map_err(|e| AppError::Provider(format!("Failed to fetch models: {}", e)))?;

    if !response.status().is_success() {
        return Err(AppError::Provider(format!(
            "API returned status: {}",
            response.status()
        )));
    }

    let models_response: AnthropicModelsResponse = response
        .json()
        .await
        .map_err(|e| AppError::Provider(format!("Failed to parse models: {}", e)))?;

    Ok(models_response.data.iter().map(|m| {
        let display_name = if m.display_name.is_empty() {
            m.id.clone()
        } else {
            m.display_name.clone()
        };

        ModelInfo {
            id: m.id.clone(),
            name: display_name,
            provider: "anthropic".to_string(),
            parameter_count: None,
            context_window: 200_000, // Default, enriched from catalog
            supports_streaming: true,
            capabilities: vec![Capability::Chat, Capability::Vision, Capability::FunctionCalling],
            detailed_capabilities: None,
        }
        .enrich_with_catalog("anthropic")
    }).collect())
}
```

**Step 2.2: Mistral** (`src-tauri/src/providers/mistral.rs`)

Same pattern as Anthropic, OpenAI-compatible endpoint.

**Step 2.3: Groq** (`src-tauri/src/providers/groq.rs`)

Same pattern as Anthropic, OpenAI-compatible endpoint at `/openai/v1/models`.

**Step 2.4: Cohere** (`src-tauri/src/providers/cohere.rs`)

Add response structs:
```rust
#[derive(Debug, Deserialize)]
struct CohereModelsResponse {
    models: Vec<CohereModelData>,
}

#[derive(Debug, Deserialize)]
struct CohereModelData {
    name: String,
    endpoints: Vec<String>,
    context_length: u32,
}
```

Similar implementation to Anthropic with Cohere-specific response parsing.

**Step 2.5: TogetherAI** (`src-tauri/src/providers/togetherai.rs`)

OpenAI-compatible endpoint, similar to Anthropic pattern.

**Step 2.6: Cerebras** (`src-tauri/src/providers/cerebras.rs`)

OpenAI-compatible endpoint, similar to Anthropic pattern.

### Phase 3: Testing

**Unit Tests** in `src-tauri/src/providers/registry.rs`:
```rust
#[cfg(test)]
mod cache_tests {
    use super::*;

    #[test]
    fn test_model_cache_expiration() {
        let cache = ModelCache::new(vec![], 3600);
        assert!(!cache.is_expired());

        let mut old_cache = ModelCache::new(vec![], 3600);
        old_cache.fetched_at = Utc::now() - Duration::seconds(3601);
        assert!(old_cache.is_expired());
    }

    #[test]
    fn test_cache_expires_in() {
        let cache = ModelCache::new(vec![], 3600);
        assert!(cache.expires_in() > 3590);
        assert!(cache.expires_in() <= 3600);
    }
}
```

**Integration Tests** in `src-tauri/tests/provider_model_cache_tests.rs`:
```rust
#[tokio::test]
async fn test_cache_hit_avoids_api_call() {
    // Test that cached models are returned without API call
}

#[tokio::test]
async fn test_cache_miss_triggers_fetch() {
    // Test that expired cache triggers new fetch
}

#[tokio::test]
async fn test_api_failure_uses_catalog() {
    // Test OpenRouter catalog fallback
}

#[tokio::test]
async fn test_stale_cache_fallback() {
    // Test fallback to stale cache when catalog unavailable
}
```

### Phase 4: Configuration

**Default Configuration** (embedded in code):
```rust
ModelCacheConfig {
    default_ttl_seconds: 3600,
    provider_ttl_overrides: HashMap::new(),
    use_catalog_fallback: true,
}
```

**User Configuration Example** (`~/.localrouter/config.yml`):
```yaml
model_cache:
  default_ttl_seconds: 3600  # 1 hour
  use_catalog_fallback: true
  provider_ttl_overrides:
    ollama: 300              # 5 minutes
    lmstudio: 300            # 5 minutes
    anthropic: 86400         # 24 hours
```

## Verification Strategy

1. **Build Check**: `cargo build` - Ensure no compilation errors
2. **Unit Tests**: `cargo test --lib` - Test cache expiration logic
3. **Integration Tests**: `cargo test` - Test end-to-end caching behavior
4. **Manual Testing**:
   - Start app with `cargo tauri dev`
   - Call `GET /v1/models` - Should see models from all providers
   - Wait for cache to expire
   - Call again - Should fetch from API
   - Disconnect network
   - Call again - Should use stale cache or catalog
5. **Configuration Test**: Modify `config.yml` with TTL overrides, verify behavior

## Benefits

1. **Performance**: 95%+ reduction in API calls (cache hit on most requests)
2. **Reliability**: Graceful degradation (API → catalog → stale cache)
3. **Accuracy**: Live API data when available, catalog enrichment always
4. **Flexibility**: Per-provider TTL configuration
5. **Privacy**: No additional network requests (catalog embedded)
6. **Consistency**: Follows existing codebase patterns (TokenStore, HealthCheckManager)

## Edge Cases Handled

- Network unavailable → Use stale cache indefinitely
- API key invalid → Return catalog models (browse without auth)
- Provider endpoint changes → Fails gracefully, uses catalog
- New models not in catalog → Shows anyway (API data primary)
- Concurrent refreshes → RwLock prevents race conditions
- Provider disabled → Cache remains but isn't used

## Migration Strategy

- Phase 1: Infrastructure (no breaking changes)
- Phase 2: Provider updates (one at a time, independently testable)
- Phase 3: Testing
- Phase 4: Configuration and documentation
- All existing functionality preserved during migration
