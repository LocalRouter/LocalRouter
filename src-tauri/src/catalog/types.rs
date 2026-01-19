// Runtime catalog types
//
// These types are embedded in the binary and used for model lookup at runtime.

use chrono::{DateTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Modality {
    Text,
    Multimodal,
    Image,
}

#[derive(Debug, Clone, Copy)]
pub struct CatalogMetadata {
    pub fetch_timestamp: u64,
    pub api_version: &'static str,
    pub total_models: usize,
}

impl CatalogMetadata {
    /// Get fetch date as DateTime
    pub fn fetch_date(&self) -> DateTime<Utc> {
        Utc.timestamp_opt(self.fetch_timestamp as i64, 0)
            .single()
            .unwrap_or_else(|| Utc::now())
    }
}

#[derive(Debug, Clone)]
pub struct CatalogModel {
    pub id: &'static str,
    pub aliases: &'static [&'static str],
    pub name: &'static str,
    pub created: i64,
    pub context_length: u32,
    pub modality: Modality,
    pub pricing: CatalogPricing,
    pub supported_parameters: &'static [&'static str],
}

#[derive(Debug, Clone, Copy)]
pub struct CatalogPricing {
    /// Cost per token (not per 1K!)
    pub prompt_per_token: f64,
    /// Cost per token (not per 1K!)
    pub completion_per_token: f64,
    /// Cost per image token (if applicable)
    pub image_per_token: Option<f64>,
    /// Fixed cost per request (if applicable)
    pub request_cost: Option<f64>,
    /// Currency code (always "USD" for OpenRouter)
    pub currency: &'static str,
}

impl CatalogPricing {
    /// Get prompt cost per 1K tokens
    pub fn prompt_cost_per_1k(&self) -> f64 {
        self.prompt_per_token * 1000.0
    }

    /// Get completion cost per 1K tokens
    pub fn completion_cost_per_1k(&self) -> f64 {
        self.completion_per_token * 1000.0
    }

    /// Calculate total cost for a request
    pub fn calculate_cost(&self, prompt_tokens: u32, completion_tokens: u32) -> f64 {
        let prompt_cost = self.prompt_per_token * prompt_tokens as f64;
        let completion_cost = self.completion_per_token * completion_tokens as f64;
        let request_cost = self.request_cost.unwrap_or(0.0);

        prompt_cost + completion_cost + request_cost
    }
}
