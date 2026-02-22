// Build-time types for models.dev API schema
//
// These types are used ONLY during compilation to parse the models.dev API response.
// They are NOT included in the final binary.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Root response from models.dev API
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelsDevResponse {
    /// Map of provider ID to provider object
    #[serde(flatten)]
    pub providers: HashMap<String, ModelsDevProvider>,
}

/// A provider in the models.dev catalog
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModelsDevProvider {
    /// Provider identifier
    #[serde(default)]
    pub id: String,
    /// Environment variable names for API keys
    #[serde(default)]
    pub env: Vec<String>,
    /// NPM package name
    #[serde(default)]
    pub npm: Option<String>,
    /// API endpoint
    #[serde(default)]
    pub api: Option<String>,
    /// Human-readable provider name
    #[serde(default)]
    pub name: String,
    /// Documentation URL
    #[serde(default)]
    pub doc: Option<String>,
    /// Map of model ID to model object
    #[serde(default)]
    pub models: HashMap<String, ModelsDevModel>,
}

/// A model in the models.dev catalog
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelsDevModel {
    /// Model identifier within the provider
    #[serde(default)]
    pub id: String,
    /// Human-readable model name
    #[serde(default)]
    pub name: String,
    /// Model family (e.g., "GPT-4", "Claude")
    #[serde(default)]
    pub family: Option<String>,
    /// Whether the model supports attachments
    #[serde(default)]
    pub attachment: bool,
    /// Whether the model supports extended reasoning
    #[serde(default)]
    pub reasoning: bool,
    /// Whether the model supports tool/function calling
    #[serde(default)]
    pub tool_call: bool,
    /// Whether the model supports structured output (JSON schema)
    #[serde(default)]
    pub structured_output: bool,
    /// Whether the model supports temperature parameter
    #[serde(default)]
    pub temperature: bool,
    /// Knowledge cutoff date (e.g., "2024-04")
    #[serde(default)]
    pub knowledge: Option<String>,
    /// Model release date
    #[serde(default)]
    pub release_date: Option<String>,
    /// Last update date
    #[serde(default)]
    pub last_updated: Option<String>,
    /// Input/output modalities
    #[serde(default)]
    pub modalities: Modalities,
    /// Whether the model weights are openly available
    #[serde(default)]
    pub open_weights: bool,
    /// Pricing information (per million tokens)
    #[serde(default)]
    pub cost: Cost,
    /// Token limits
    #[serde(default)]
    pub limit: Limits,
}

/// Input/output modalities for a model
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Modalities {
    /// Supported input types (e.g., ["text", "image", "pdf"])
    #[serde(default)]
    pub input: Vec<String>,
    /// Supported output types (e.g., ["text"])
    #[serde(default)]
    pub output: Vec<String>,
}

/// Pricing information (per million tokens)
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Cost {
    /// Input cost per million tokens (USD)
    #[serde(default)]
    pub input: f64,
    /// Output cost per million tokens (USD)
    #[serde(default)]
    pub output: f64,
    /// Cache read cost per million tokens (USD)
    #[serde(default)]
    pub cache_read: Option<f64>,
    /// Cache write cost per million tokens (USD)
    #[serde(default)]
    pub cache_write: Option<f64>,
    /// Reasoning tokens cost per million tokens (USD)
    #[serde(default)]
    pub reasoning: Option<f64>,
}

/// Token limits for a model
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Limits {
    /// Maximum context window size
    #[serde(default)]
    pub context: u32,
    /// Maximum output tokens
    #[serde(default)]
    pub output: Option<u32>,
}

impl ModelsDevModel {
    /// Get prompt cost per token (converted from per-million)
    pub fn prompt_cost_per_token(&self) -> f64 {
        self.cost.input / 1_000_000.0
    }

    /// Get completion cost per token (converted from per-million)
    pub fn completion_cost_per_token(&self) -> f64 {
        self.cost.output / 1_000_000.0
    }

    /// Get cache read cost per token (converted from per-million)
    pub fn cache_read_cost_per_token(&self) -> Option<f64> {
        self.cost.cache_read.map(|c| c / 1_000_000.0)
    }

    /// Get cache write cost per token (converted from per-million)
    pub fn cache_write_cost_per_token(&self) -> Option<f64> {
        self.cost.cache_write.map(|c| c / 1_000_000.0)
    }

    /// Determine if the model supports vision (image input)
    pub fn supports_vision(&self) -> bool {
        self.modalities.input.iter().any(|m| m == "image")
    }

    /// Determine modality category from input/output types
    pub fn modality_category(&self) -> &'static str {
        let has_image_input = self.modalities.input.iter().any(|m| m == "image");
        let has_pdf_input = self.modalities.input.iter().any(|m| m == "pdf");
        let has_audio_input = self.modalities.input.iter().any(|m| m == "audio");
        let has_image_output = self.modalities.output.iter().any(|m| m == "image");

        if has_image_output {
            "image"
        } else if has_image_input || has_pdf_input || has_audio_input {
            "multimodal"
        } else {
            "text"
        }
    }
}

/// Flattened model with full ID (provider/model format)
#[derive(Debug, Clone)]
pub struct FlattenedModel {
    /// Full model ID in "provider/model" format
    pub full_id: String,
    /// Provider ID
    pub provider_id: String,
    /// Original model ID within provider
    pub model_id: String,
    /// The model data
    pub model: ModelsDevModel,
}

/// Flatten nested provider->models structure into a flat list
pub fn flatten_models(response: ModelsDevResponse) -> Vec<FlattenedModel> {
    response
        .providers
        .into_iter()
        .flat_map(|(provider_id, provider)| {
            provider
                .models
                .into_iter()
                .map(move |(model_id, model)| FlattenedModel {
                    full_id: format!("{}/{}", provider_id, model_id),
                    provider_id: provider_id.clone(),
                    model_id: model_id.clone(),
                    model,
                })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_price_conversion() {
        let model = ModelsDevModel {
            cost: Cost {
                input: 15.0,  // $15 per million
                output: 60.0, // $60 per million
                cache_read: Some(1.5),
                cache_write: Some(3.75),
                reasoning: None,
            },
            ..Default::default()
        };

        assert!((model.prompt_cost_per_token() - 0.000015).abs() < 1e-10);
        assert!((model.completion_cost_per_token() - 0.00006).abs() < 1e-10);
        assert!((model.cache_read_cost_per_token().unwrap() - 0.0000015).abs() < 1e-10);
        assert!((model.cache_write_cost_per_token().unwrap() - 0.00000375).abs() < 1e-10);
    }

    #[test]
    fn test_modality_detection() {
        // Text-only model
        let text_model = ModelsDevModel {
            modalities: Modalities {
                input: vec!["text".to_string()],
                output: vec!["text".to_string()],
            },
            ..Default::default()
        };
        assert_eq!(text_model.modality_category(), "text");
        assert!(!text_model.supports_vision());

        // Vision model
        let vision_model = ModelsDevModel {
            modalities: Modalities {
                input: vec!["text".to_string(), "image".to_string()],
                output: vec!["text".to_string()],
            },
            ..Default::default()
        };
        assert_eq!(vision_model.modality_category(), "multimodal");
        assert!(vision_model.supports_vision());

        // Image generation model
        let image_gen_model = ModelsDevModel {
            modalities: Modalities {
                input: vec!["text".to_string()],
                output: vec!["image".to_string()],
            },
            ..Default::default()
        };
        assert_eq!(image_gen_model.modality_category(), "image");
    }

    #[test]
    fn test_flatten_models() {
        let mut providers = HashMap::new();
        let mut models = HashMap::new();
        models.insert(
            "gpt-4".to_string(),
            ModelsDevModel {
                id: "gpt-4".to_string(),
                name: "GPT-4".to_string(),
                ..Default::default()
            },
        );
        providers.insert(
            "openai".to_string(),
            ModelsDevProvider {
                id: "openai".to_string(),
                name: "OpenAI".to_string(),
                models,
                ..Default::default()
            },
        );

        let response = ModelsDevResponse { providers };
        let flattened = flatten_models(response);

        assert_eq!(flattened.len(), 1);
        assert_eq!(flattened[0].full_id, "openai/gpt-4");
        assert_eq!(flattened[0].provider_id, "openai");
        assert_eq!(flattened[0].model_id, "gpt-4");
    }
}

impl Default for ModelsDevModel {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            family: None,
            attachment: false,
            reasoning: false,
            tool_call: false,
            structured_output: false,
            temperature: true,
            knowledge: None,
            release_date: None,
            last_updated: None,
            modalities: Modalities::default(),
            open_weights: false,
            cost: Cost::default(),
            limit: Limits::default(),
        }
    }
}

