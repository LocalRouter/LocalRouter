// Build-time types for OpenRouter API schema
//
// These types are used ONLY during compilation to parse the OpenRouter API response.
// They are NOT included in the final binary.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenRouterModelsResponse {
    pub data: Vec<OpenRouterModel>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenRouterModel {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub created: i64,
    pub context_length: u32,
    #[serde(default)]
    pub architecture: Architecture,
    pub pricing: PricingTiers,
    #[serde(default)]
    pub supported_parameters: Vec<String>,
    #[serde(default)]
    pub top_provider: Option<TopProvider>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Architecture {
    #[serde(default)]
    pub modality: String, // "text", "text+image", "multimodal"
    #[serde(default)]
    pub tokenizer: String,
    #[serde(default)]
    pub instruct_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PricingTiers {
    /// Cost per token (as string, e.g., "0.00003")
    pub prompt: String,
    /// Cost per token (as string, e.g., "0.00006")
    pub completion: String,
    #[serde(default)]
    pub image: Option<String>,
    #[serde(default)]
    pub request: Option<String>, // Fixed per-request cost
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopProvider {
    #[serde(default)]
    pub context_length: Option<u32>,
    #[serde(default)]
    pub max_completion_tokens: Option<u32>,
    #[serde(default)]
    pub is_moderated: Option<bool>,
}

impl OpenRouterModel {
    /// Parse pricing string to f64
    pub fn parse_price(price_str: &str) -> f64 {
        price_str.parse::<f64>().unwrap_or(0.0)
    }

    /// Get prompt cost per token (not per 1K)
    pub fn prompt_cost_per_token(&self) -> f64 {
        Self::parse_price(&self.pricing.prompt)
    }

    /// Get completion cost per token (not per 1K)
    pub fn completion_cost_per_token(&self) -> f64 {
        Self::parse_price(&self.pricing.completion)
    }

    /// Get image cost per token if available
    pub fn image_cost_per_token(&self) -> Option<f64> {
        self.pricing.image.as_ref().map(|s| Self::parse_price(s))
    }

    /// Get fixed request cost if available
    pub fn request_cost(&self) -> Option<f64> {
        self.pricing.request.as_ref().map(|s| Self::parse_price(s))
    }

    /// Determine modality from architecture
    pub fn modality(&self) -> &str {
        match self.architecture.modality.as_str() {
            "text+image" | "multimodal" => "multimodal",
            "text->image" | "image" => "image",
            _ => "text",
        }
    }
}
