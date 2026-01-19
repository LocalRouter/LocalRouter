// Model matching logic
//
// Handles fuzzy matching between provider model names and catalog entries

use super::types::CatalogModel;
use std::collections::HashMap;

pub struct ModelMatcher {
    /// Exact ID lookup: "openai/gpt-4" → model
    by_id: HashMap<String, &'static CatalogModel>,
    /// Alias lookup: "gpt-4", "gpt4" → model
    by_alias: HashMap<String, &'static CatalogModel>,
    /// Provider+model lookup: ("openai", "gpt-4") → model
    by_provider: HashMap<(String, String), &'static CatalogModel>,
}

impl ModelMatcher {
    pub fn new(models: &'static [CatalogModel]) -> Self {
        let mut by_id = HashMap::new();
        let mut by_alias = HashMap::new();
        let mut by_provider = HashMap::new();

        for model in models {
            // Index by exact ID
            let normalized_id = normalize_id(model.id);
            by_id.insert(normalized_id.clone(), model);

            // Index by provider+model
            if let Some((provider, model_name)) = model.id.split_once('/') {
                by_provider.insert(
                    (normalize_id(provider), normalize_id(model_name)),
                    model,
                );
            }

            // Index by aliases
            for alias in model.aliases {
                by_alias.insert(normalize_id(alias), model);
            }
        }

        Self {
            by_id,
            by_alias,
            by_provider,
        }
    }

    /// Find a model by provider and model ID with multi-level matching
    pub fn find_model(&self, provider: &str, model_id: &str) -> Option<&'static CatalogModel> {
        // 1. Try exact provider+model match
        let norm_provider = normalize_id(provider);
        let norm_model = normalize_id(model_id);

        if let Some(model) = self.by_provider.get(&(norm_provider.clone(), norm_model.clone())) {
            return Some(model);
        }

        // 2. Try normalized OpenRouter ID format
        let openrouter_id = format!("{}/{}", norm_provider, norm_model);
        if let Some(model) = self.by_id.get(&openrouter_id) {
            return Some(model);
        }

        // 3. Try alias matching (just the model name, ignore provider)
        if let Some(model) = self.by_alias.get(&norm_model) {
            return Some(model);
        }

        // 4. Try fuzzy/prefix matching
        self.fuzzy_match(provider, model_id)
    }

    /// Fuzzy matching for common variations
    fn fuzzy_match(&self, provider: &str, model_id: &str) -> Option<&'static CatalogModel> {
        let norm_provider = normalize_id(provider);
        let norm_model = normalize_id(model_id);

        // Try prefix matching on model name
        let prefix_key = format!("{}/{}", norm_provider, norm_model);

        self.by_id
            .keys()
            .filter(|k| k.starts_with(&prefix_key))
            .filter_map(|k| self.by_id.get(k).copied())
            .next()
    }
}

/// Normalize model ID for matching
///
/// Converts to lowercase and replaces various separators with hyphens
fn normalize_id(id: &str) -> String {
    id.to_lowercase()
        .replace('_', "-")
        .replace(':', "-")
        .replace(' ', "-")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::types::{CatalogMetadata, CatalogModel, CatalogPricing, Modality};

    fn create_test_models() -> Vec<CatalogModel> {
        vec![
            CatalogModel {
                id: "openai/gpt-4",
                aliases: &["gpt-4", "gpt4"],
                name: "GPT-4",
                created: 0,
                context_length: 8192,
                modality: Modality::Text,
                pricing: CatalogPricing {
                    prompt_per_token: 0.00003,
                    completion_per_token: 0.00006,
                    image_per_token: None,
                    request_cost: None,
                    currency: "USD",
                },
                supported_parameters: &[],
            },
            CatalogModel {
                id: "anthropic/claude-opus-4-20250514",
                aliases: &["claude-opus-4", "opus"],
                name: "Claude Opus 4",
                created: 0,
                context_length: 200000,
                modality: Modality::Text,
                pricing: CatalogPricing {
                    prompt_per_token: 0.000015,
                    completion_per_token: 0.000075,
                    image_per_token: None,
                    request_cost: None,
                    currency: "USD",
                },
                supported_parameters: &[],
            },
        ]
    }

    #[test]
    fn test_exact_match() {
        let models = create_test_models();
        let models_static: &'static [CatalogModel] = Box::leak(models.into_boxed_slice());
        let matcher = ModelMatcher::new(models_static);

        let result = matcher.find_model("openai", "gpt-4");
        assert!(result.is_some());
        assert_eq!(result.unwrap().id, "openai/gpt-4");
    }

    #[test]
    fn test_alias_match() {
        let models = create_test_models();
        let models_static: &'static [CatalogModel] = Box::leak(models.into_boxed_slice());
        let matcher = ModelMatcher::new(models_static);

        let result = matcher.find_model("openai", "gpt4");
        assert!(result.is_some());
        assert_eq!(result.unwrap().id, "openai/gpt-4");
    }

    #[test]
    fn test_normalization() {
        let models = create_test_models();
        let models_static: &'static [CatalogModel] = Box::leak(models.into_boxed_slice());
        let matcher = ModelMatcher::new(models_static);

        // Case insensitive
        let result = matcher.find_model("OpenAI", "GPT-4");
        assert!(result.is_some());

        // Underscore normalization
        let result = matcher.find_model("openai", "gpt_4");
        assert!(result.is_some());
    }

    #[test]
    fn test_fuzzy_match() {
        let models = create_test_models();
        let models_static: &'static [CatalogModel] = Box::leak(models.into_boxed_slice());
        let matcher = ModelMatcher::new(models_static);

        // Prefix match
        let result = matcher.find_model("anthropic", "claude-opus");
        assert!(result.is_some());
        assert_eq!(result.unwrap().id, "anthropic/claude-opus-4-20250514");
    }

    #[test]
    fn test_no_match() {
        let models = create_test_models();
        let models_static: &'static [CatalogModel] = Box::leak(models.into_boxed_slice());
        let matcher = ModelMatcher::new(models_static);

        let result = matcher.find_model("nonexistent", "model");
        assert!(result.is_none());
    }
}
