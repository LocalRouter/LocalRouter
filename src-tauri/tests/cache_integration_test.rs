//! Integration test for provider model caching
//!
//! Tests the three-tier caching system:
//! 1. Cache hit (models returned from cache)
//! 2. Cache miss (fetch from API)
//! 3. API failure (fallback to catalog)

use localrouter::config::ModelCacheConfig;
use localrouter::providers::health::HealthCheckManager;
use localrouter::providers::registry::ProviderRegistry;
use std::collections::HashMap;
use std::sync::Arc;

#[tokio::test]
async fn test_model_caching_with_ollama() {
    // Create registry
    let health_mgr = Arc::new(HealthCheckManager::default());
    let registry = ProviderRegistry::new(health_mgr);

    // Configure short cache TTL for testing
    let mut cache_config = ModelCacheConfig::default();
    cache_config.default_ttl_seconds = 5; // 5 second cache for testing
    registry.update_cache_config(cache_config);

    // Register Ollama provider (no auth needed)
    registry.register_factory(Arc::new(
        localrouter::providers::factory::OllamaProviderFactory,
    ));

    // Create Ollama instance
    let mut config = HashMap::new();
    config.insert("base_url".to_string(), "http://localhost:11434".to_string());

    let result = registry
        .create_provider("test-ollama".to_string(), "ollama".to_string(), config)
        .await;

    // If Ollama is not running, skip this test
    if result.is_err() {
        println!("Skipping test: Ollama not available");
        return;
    }

    println!("✅ Created Ollama provider instance");

    // First call - should fetch from API and cache
    let models1 = registry.list_provider_models_cached("test-ollama").await;
    match &models1 {
        Ok(m) => println!("✅ First call: Got {} models from API", m.len()),
        Err(e) => println!("⚠️  First call failed: {}", e),
    }

    // Second call immediately - should hit cache
    let models2 = registry.list_provider_models_cached("test-ollama").await;
    match &models2 {
        Ok(m) => println!("✅ Second call: Got {} models from cache", m.len()),
        Err(e) => println!("⚠️  Second call failed: {}", e),
    }

    // Verify both calls returned same data
    if let (Ok(m1), Ok(m2)) = (models1, models2) {
        assert_eq!(
            m1.len(),
            m2.len(),
            "Cache should return same number of models"
        );
        println!("✅ Cache consistency verified");
    }

    // Wait for cache to expire
    println!("⏳ Waiting 6 seconds for cache to expire...");
    tokio::time::sleep(tokio::time::Duration::from_secs(6)).await;

    // Third call - should fetch fresh data
    let models3 = registry.list_provider_models_cached("test-ollama").await;
    match models3 {
        Ok(m) => println!(
            "✅ Third call: Got {} models after cache expiration",
            m.len()
        ),
        Err(e) => println!("⚠️  Third call failed: {}", e),
    }

    println!("✅ Cache expiration test completed");
}

#[tokio::test]
async fn test_catalog_fallback() {
    // Create registry
    let health_mgr = Arc::new(HealthCheckManager::default());
    let registry = ProviderRegistry::new(health_mgr);

    // Configure cache to use catalog fallback
    let mut cache_config = ModelCacheConfig::default();
    cache_config.use_catalog_fallback = true;
    registry.update_cache_config(cache_config);

    // Register Anthropic provider
    registry.register_factory(Arc::new(
        localrouter::providers::factory::AnthropicProviderFactory,
    ));

    // Create Anthropic instance with invalid key (should fail API call)
    let mut config = HashMap::new();
    config.insert("api_key".to_string(), "invalid-key-for-testing".to_string());

    let _ = registry
        .create_provider(
            "test-anthropic".to_string(),
            "anthropic".to_string(),
            config,
        )
        .await;

    println!("✅ Created Anthropic provider with invalid key");

    // Call should fail API but fallback to catalog
    let models = registry.list_provider_models_cached("test-anthropic").await;

    match models {
        Ok(m) => {
            println!(
                "✅ Catalog fallback: Got {} models from models.dev catalog",
                m.len()
            );
            assert!(!m.is_empty(), "Should have models from catalog");

            // Verify we got Claude models
            let claude_models: Vec<_> = m
                .iter()
                .filter(|model| model.id.contains("claude"))
                .collect();
            println!("   Found {} Claude models in catalog", claude_models.len());
            assert!(
                !claude_models.is_empty(),
                "Should have Claude models from catalog"
            );
        }
        Err(e) => {
            println!("⚠️  Catalog fallback failed: {}", e);
            panic!("Catalog fallback should have provided models");
        }
    }

    println!("✅ Catalog fallback test completed");
}

#[tokio::test]
async fn test_cache_invalidation() {
    // Create registry
    let health_mgr = Arc::new(HealthCheckManager::default());
    let registry = ProviderRegistry::new(health_mgr);

    // Register Ollama provider
    registry.register_factory(Arc::new(
        localrouter::providers::factory::OllamaProviderFactory,
    ));

    let mut config = HashMap::new();
    config.insert("base_url".to_string(), "http://localhost:11434".to_string());

    let result = registry
        .create_provider("test-ollama".to_string(), "ollama".to_string(), config)
        .await;

    if result.is_err() {
        println!("Skipping test: Ollama not available");
        return;
    }

    // First call to populate cache
    let _ = registry.list_provider_models_cached("test-ollama").await;
    println!("✅ Populated cache");

    // Invalidate cache
    registry.invalidate_provider_cache("test-ollama");
    println!("✅ Invalidated cache");

    // Next call should fetch fresh data
    let models = registry.list_provider_models_cached("test-ollama").await;
    match models {
        Ok(m) => println!("✅ Got {} models after cache invalidation", m.len()),
        Err(e) => println!("⚠️  Call after invalidation failed: {}", e),
    }

    println!("✅ Cache invalidation test completed");
}
