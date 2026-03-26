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
                by_provider.insert((normalize_id(provider), normalize_id(model_name)), model);
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

        if let Some(model) = self
            .by_provider
            .get(&(norm_provider.clone(), norm_model.clone()))
        {
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
        if let Some(model) = self.fuzzy_match(provider, model_id) {
            return Some(model);
        }

        // 5. Try stripping date suffix (-YYYY-MM-DD) and retrying
        //
        // Providers like OpenAI return dated model variants (e.g., gpt-4.1-2025-04-14)
        // that share pricing/metadata with the base model (e.g., gpt-4.1).
        if let Some(base) = strip_date_suffix(&norm_model) {
            let base = base.to_string();
            if let Some(model) = self.by_provider.get(&(norm_provider.clone(), base.clone())) {
                return Some(model);
            }
            let base_id = format!("{}/{}", norm_provider, base);
            if let Some(model) = self.by_id.get(&base_id) {
                return Some(model);
            }
            if let Some(model) = self.by_alias.get(&base) {
                return Some(model);
            }
        }

        None
    }

    /// Find a model by name only (ignoring provider)
    ///
    /// This is useful for multi-provider systems like Ollama, LMStudio, DeepInfra,
    /// TogetherAI, and OpenRouter where the model can come from various providers.
    pub fn find_model_by_name(&self, model_id: &str) -> Option<&'static CatalogModel> {
        let norm_model = normalize_id(model_id);

        // 1. Try exact alias match
        if let Some(model) = self.by_alias.get(&norm_model) {
            return Some(model);
        }

        // 2. Try matching against the model part of "provider/model" IDs
        for (full_id, model) in &self.by_id {
            if let Some((_provider, model_name)) = full_id.split_once('/') {
                if normalize_id(model_name) == norm_model {
                    return Some(model);
                }
            }
        }

        // 3. Try fuzzy/prefix matching on model names
        if let Some(model) = self.fuzzy_match_by_name(&norm_model) {
            return Some(model);
        }

        // 4. Try stripping date suffix (-YYYY-MM-DD) and retrying
        if let Some(base) = strip_date_suffix(&norm_model) {
            if let Some(model) = self.by_alias.get(base) {
                return Some(model);
            }
            for (full_id, model) in &self.by_id {
                if let Some((_provider, model_name)) = full_id.split_once('/') {
                    if normalize_id(model_name) == base {
                        return Some(model);
                    }
                }
            }
        }

        None
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

    /// Fuzzy matching by model name only (provider-agnostic)
    fn fuzzy_match_by_name(&self, model_id: &str) -> Option<&'static CatalogModel> {
        // Try prefix matching on the model part of IDs
        for (full_id, model) in &self.by_id {
            if let Some((_provider, model_name)) = full_id.split_once('/') {
                if model_name.starts_with(model_id) {
                    return Some(model);
                }
            }
        }
        None
    }
}

/// Normalize model ID for matching
///
/// Converts to lowercase and replaces various separators with hyphens
fn normalize_id(id: &str) -> String {
    id.to_lowercase().replace(['_', ':', ' '], "-")
}

/// Strip a trailing date suffix (-YYYY-MM-DD) from a model ID.
///
/// Providers like OpenAI version models with date suffixes (e.g., `gpt-4.1-2025-04-14`).
/// These dated variants share pricing and metadata with the base model (`gpt-4.1`).
/// Returns the base model ID if a date suffix is found, None otherwise.
fn strip_date_suffix(model_id: &str) -> Option<&str> {
    // Pattern: -YYYY-MM-DD (11 chars at the end)
    let bytes = model_id.as_bytes();
    if bytes.len() > 11 {
        let s = bytes.len() - 11;
        if bytes[s] == b'-'
            && bytes[s + 1..s + 5].iter().all(u8::is_ascii_digit)
            && bytes[s + 5] == b'-'
            && bytes[s + 6..s + 8].iter().all(u8::is_ascii_digit)
            && bytes[s + 8] == b'-'
            && bytes[s + 9..s + 11].iter().all(u8::is_ascii_digit)
        {
            return Some(&model_id[..s]);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{CatalogCapabilities, CatalogModel, CatalogPricing, Modality};

    fn create_test_models() -> Vec<CatalogModel> {
        vec![
            CatalogModel {
                id: "openai/gpt-4",
                aliases: &["gpt-4", "gpt4"],
                name: "GPT-4",
                context_length: 8192,
                max_output_tokens: Some(4096),
                modality: Modality::Text,
                capabilities: CatalogCapabilities {
                    reasoning: false,
                    tool_call: true,
                    structured_output: true,
                    vision: false,
                    embedding: false,
                    audio_input: false,
                    audio_output: false,
                    image_output: false,
                },
                pricing: CatalogPricing {
                    prompt_per_token: 0.00003,
                    completion_per_token: 0.00006,
                    cache_read_per_token: None,
                    cache_write_per_token: None,
                    reasoning_per_token: None,
                    currency: "USD",
                },
                knowledge_cutoff: Some("2023-12"),
                open_weights: false,
            },
            CatalogModel {
                id: "anthropic/claude-opus-4-20250514",
                aliases: &["claude-opus-4", "opus"],
                name: "Claude Opus 4",
                context_length: 200000,
                max_output_tokens: Some(64000),
                modality: Modality::Multimodal,
                capabilities: CatalogCapabilities {
                    reasoning: true,
                    tool_call: true,
                    structured_output: true,
                    vision: true,
                    embedding: false,
                    audio_input: false,
                    audio_output: false,
                    image_output: false,
                },
                pricing: CatalogPricing {
                    prompt_per_token: 0.000015,
                    completion_per_token: 0.000075,
                    cache_read_per_token: Some(0.0000015),
                    cache_write_per_token: Some(0.00001875),
                    reasoning_per_token: None,
                    currency: "USD",
                },
                knowledge_cutoff: Some("2025-03"),
                open_weights: false,
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

    #[test]
    fn test_find_by_name_exact_alias() {
        let models = create_test_models();
        let models_static: &'static [CatalogModel] = Box::leak(models.into_boxed_slice());
        let matcher = ModelMatcher::new(models_static);

        // Should find by alias
        let result = matcher.find_model_by_name("gpt-4");
        assert!(result.is_some());
        assert_eq!(result.unwrap().id, "openai/gpt-4");

        let result = matcher.find_model_by_name("opus");
        assert!(result.is_some());
        assert_eq!(result.unwrap().id, "anthropic/claude-opus-4-20250514");
    }

    #[test]
    fn test_find_by_name_model_part() {
        let models = create_test_models();
        let models_static: &'static [CatalogModel] = Box::leak(models.into_boxed_slice());
        let matcher = ModelMatcher::new(models_static);

        // Should find by model part of "provider/model"
        let result = matcher.find_model_by_name("gpt-4");
        assert!(result.is_some());
        assert_eq!(result.unwrap().id, "openai/gpt-4");
    }

    #[test]
    fn test_find_by_name_case_insensitive() {
        let models = create_test_models();
        let models_static: &'static [CatalogModel] = Box::leak(models.into_boxed_slice());
        let matcher = ModelMatcher::new(models_static);

        // Should be case insensitive
        let result = matcher.find_model_by_name("GPT-4");
        assert!(result.is_some());
        assert_eq!(result.unwrap().id, "openai/gpt-4");

        let result = matcher.find_model_by_name("OPUS");
        assert!(result.is_some());
    }

    #[test]
    fn test_find_by_name_normalization() {
        let models = create_test_models();
        let models_static: &'static [CatalogModel] = Box::leak(models.into_boxed_slice());
        let matcher = ModelMatcher::new(models_static);

        // Should normalize underscores to hyphens
        let result = matcher.find_model_by_name("gpt_4");
        assert!(result.is_some());
        assert_eq!(result.unwrap().id, "openai/gpt-4");
    }

    #[test]
    fn test_find_by_name_prefix_match() {
        let models = create_test_models();
        let models_static: &'static [CatalogModel] = Box::leak(models.into_boxed_slice());
        let matcher = ModelMatcher::new(models_static);

        // Should find by prefix
        let result = matcher.find_model_by_name("claude-opus");
        assert!(result.is_some());
        assert_eq!(result.unwrap().id, "anthropic/claude-opus-4-20250514");
    }

    #[test]
    fn test_find_by_name_no_match() {
        let models = create_test_models();
        let models_static: &'static [CatalogModel] = Box::leak(models.into_boxed_slice());
        let matcher = ModelMatcher::new(models_static);

        let result = matcher.find_model_by_name("nonexistent-model");
        assert!(result.is_none());
    }

    #[test]
    fn test_strip_date_suffix() {
        // Valid ISO date suffixes
        assert_eq!(strip_date_suffix("gpt-4.1-2025-04-14"), Some("gpt-4.1"));
        assert_eq!(strip_date_suffix("o1-2024-12-17"), Some("o1"));
        assert_eq!(
            strip_date_suffix("gpt-4o-audio-preview-2024-12-17"),
            Some("gpt-4o-audio-preview")
        );
        assert_eq!(
            strip_date_suffix("gpt-5.4-nano-2026-03-17"),
            Some("gpt-5.4-nano")
        );

        // No date suffix - should return None
        assert_eq!(strip_date_suffix("gpt-4o"), None);
        assert_eq!(strip_date_suffix("gpt-4o-mini"), None);
        assert_eq!(strip_date_suffix("o1"), None);

        // Too short to have a date suffix
        assert_eq!(strip_date_suffix("-2025-01-01"), None);
        // Single char base is still valid (won't match any real model anyway)
        assert_eq!(strip_date_suffix("x-2025-01-01"), Some("x"));

        // MMDD format (not handled - intentionally)
        assert_eq!(strip_date_suffix("gpt-3.5-turbo-instruct-0914"), None);
    }

    #[test]
    fn test_date_suffix_stripping_find_model() {
        let models = create_test_models();
        let models_static: &'static [CatalogModel] = Box::leak(models.into_boxed_slice());
        let matcher = ModelMatcher::new(models_static);

        // gpt-4-2024-06-01 should resolve to gpt-4 via date suffix stripping
        let result = matcher.find_model("openai", "gpt-4-2024-06-01");
        assert!(
            result.is_some(),
            "Should find gpt-4 via date suffix stripping"
        );
        assert_eq!(result.unwrap().id, "openai/gpt-4");
    }

    #[test]
    fn test_date_suffix_stripping_find_by_name() {
        let models = create_test_models();
        let models_static: &'static [CatalogModel] = Box::leak(models.into_boxed_slice());
        let matcher = ModelMatcher::new(models_static);

        // gpt-4-2024-06-01 should resolve to gpt-4 by name
        let result = matcher.find_model_by_name("gpt-4-2024-06-01");
        assert!(
            result.is_some(),
            "Should find gpt-4 by name via date suffix stripping"
        );
        assert_eq!(result.unwrap().id, "openai/gpt-4");
    }

    #[test]
    fn test_date_suffix_no_false_positive() {
        let models = create_test_models();
        let models_static: &'static [CatalogModel] = Box::leak(models.into_boxed_slice());
        let matcher = ModelMatcher::new(models_static);

        // A model that doesn't exist even after stripping should still return None
        let result = matcher.find_model("openai", "nonexistent-2025-01-01");
        assert!(result.is_none());
    }
}
