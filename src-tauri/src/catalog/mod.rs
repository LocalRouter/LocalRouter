// Model catalog module
//
// Provides offline-capable model metadata and pricing from OpenRouter.
// All data is embedded at build time - no runtime network requests.

pub mod matcher;
pub mod types;

pub use types::{CatalogMetadata, CatalogModel, CatalogPricing, Modality};

// Include the generated catalog data
include!(concat!(env!("CARGO_MANIFEST_DIR"), "/catalog/catalog.rs"));

// Lazy-initialized matcher
use matcher::ModelMatcher;
use once_cell::sync::Lazy;

pub static MATCHER: Lazy<ModelMatcher> = Lazy::new(|| ModelMatcher::new(CATALOG_MODELS));

/// Find a model by provider and model ID
pub fn find_model(provider: &str, model_id: &str) -> Option<&'static CatalogModel> {
    MATCHER.find_model(provider, model_id)
}

/// Find a model by name only (provider-agnostic search)
///
/// This is useful for multi-provider systems like Ollama, LMStudio, DeepInfra,
/// TogetherAI, and OpenRouter where models can come from various providers.
///
/// The search will:
/// 1. Try exact alias matches
/// 2. Match against the model part of "provider/model" IDs
/// 3. Try fuzzy/prefix matching
pub fn find_model_by_name(model_id: &str) -> Option<&'static CatalogModel> {
    MATCHER.find_model_by_name(model_id)
}

/// Get catalog metadata
pub fn metadata() -> &'static CatalogMetadata {
    &CATALOG_METADATA
}

/// Get all catalog models
pub fn models() -> &'static [CatalogModel] {
    CATALOG_MODELS
}

#[cfg(test)]
mod integration_tests {
    use super::*;

    #[test]
    fn test_catalog_metadata_valid() {
        let meta = metadata();
        assert_eq!(meta.api_version, "v1");
        assert_eq!(meta.total_models, 339);
        assert!(meta.fetch_timestamp > 0);

        // Fetch date should be recent (within last 365 days)
        let date = meta.fetch_date();
        let now = chrono::Utc::now();
        let age = now.signed_duration_since(date);
        assert!(age.num_days() < 365, "Catalog is too old");
    }

    #[test]
    fn test_catalog_has_major_providers() {
        let all_models = models();

        // Check for major providers
        let has_openai = all_models.iter().any(|m| m.id.starts_with("openai/"));
        let has_anthropic = all_models.iter().any(|m| m.id.starts_with("anthropic/"));
        let has_google = all_models.iter().any(|m| m.id.starts_with("google/"));

        assert!(has_openai, "Catalog should include OpenAI models");
        assert!(has_anthropic, "Catalog should include Anthropic models");
        assert!(has_google, "Catalog should include Google models");
    }

    #[test]
    fn test_find_popular_models() {
        // Test finding popular models by various names
        assert!(find_model("openai", "gpt-4").is_some(), "Should find GPT-4");
        assert!(
            find_model("openai", "gpt4").is_some(),
            "Should find GPT-4 by alias"
        );
        assert!(
            find_model("anthropic", "claude-opus-4").is_some(),
            "Should find Claude Opus"
        );
        assert!(
            find_model("google", "gemini-2.0-flash").is_some(),
            "Should find Gemini"
        );
    }

    #[test]
    fn test_pricing_data_valid() {
        let all_models = models();

        for model in all_models.iter().take(50) {
            // All models should have non-negative pricing
            assert!(
                model.pricing.prompt_per_token >= 0.0,
                "Model {} has negative prompt pricing",
                model.id
            );
            assert!(
                model.pricing.completion_per_token >= 0.0,
                "Model {} has negative completion pricing",
                model.id
            );

            // Pricing should be reasonable (not absurdly high)
            assert!(
                model.pricing.prompt_per_token < 1.0,
                "Model {} has suspiciously high pricing",
                model.id
            );

            // Currency should always be USD
            assert_eq!(model.pricing.currency, "USD");
        }
    }

    #[test]
    fn test_context_window_reasonable() {
        let all_models = models();

        for model in all_models {
            // Context window should be at least 1K tokens
            assert!(
                model.context_length >= 1000,
                "Model {} has unreasonably small context window: {}",
                model.id,
                model.context_length
            );

            // Context window should not exceed 10M tokens (sanity check)
            assert!(
                model.context_length <= 10_000_000,
                "Model {} has unreasonably large context window: {}",
                model.id,
                model.context_length
            );
        }
    }

    #[test]
    fn test_find_model_by_name() {
        // Test finding models by name only (provider-agnostic)

        // Should find by exact alias
        assert!(
            find_model_by_name("gpt-4").is_some(),
            "Should find GPT-4 by alias"
        );
        assert!(
            find_model_by_name("opus").is_some(),
            "Should find Opus by alias"
        );

        // Should find by model part of "provider/model"
        let gpt4_result = find_model_by_name("gpt-4");
        assert!(gpt4_result.is_some());
        assert!(
            gpt4_result.unwrap().id.contains("gpt-4"),
            "Should match GPT-4 model"
        );

        // Should be case insensitive
        assert!(
            find_model_by_name("GPT-4").is_some(),
            "Should be case insensitive"
        );

        // Should handle different separators
        assert!(
            find_model_by_name("gpt_4").is_some(),
            "Should normalize underscores"
        );

        // Should return None for non-existent models
        assert!(
            find_model_by_name("nonexistent-model-xyz").is_none(),
            "Should return None for non-existent models"
        );
    }

    #[test]
    fn test_find_model_by_name_with_multi_provider_models() {
        // Test that models from different providers can be found by model name alone
        // This is critical for Ollama, LMStudio, DeepInfra, etc.

        // Llama models might be hosted by multiple providers
        let llama_result = find_model_by_name("llama");
        assert!(
            llama_result.is_some(),
            "Should find Llama model by name prefix"
        );

        // Gemini models
        let gemini_result = find_model_by_name("gemini");
        assert!(
            gemini_result.is_some(),
            "Should find Gemini model by name prefix"
        );
    }
}
