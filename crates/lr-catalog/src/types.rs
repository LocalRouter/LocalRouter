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
    pub source: &'static str,
    pub total_models: usize,
}

impl CatalogMetadata {
    /// Get fetch date as DateTime
    pub fn fetch_date(&self) -> DateTime<Utc> {
        Utc.timestamp_opt(self.fetch_timestamp as i64, 0)
            .single()
            .unwrap_or_else(Utc::now)
    }
}

/// Model capabilities from models.dev
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogCapabilities {
    /// Whether the model supports extended reasoning/thinking
    pub reasoning: bool,
    /// Whether the model supports tool/function calling
    pub tool_call: bool,
    /// Whether the model supports structured output (JSON schema)
    pub structured_output: bool,
    /// Whether the model supports image input (vision)
    pub vision: bool,
}


#[derive(Debug, Clone)]
#[allow(dead_code)]
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

#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
pub struct CatalogPricing {
    /// Cost per token (not per 1K!)
    pub prompt_per_token: f64,
    /// Cost per token (not per 1K!)
    pub completion_per_token: f64,
    /// Cost per token for reading from cache (prompt caching)
    pub cache_read_per_token: Option<f64>,
    /// Cost per token for writing to cache (prompt caching)
    pub cache_write_per_token: Option<f64>,
    /// Currency code (always "USD")
    pub currency: &'static str,
}

#[allow(dead_code)]
impl CatalogPricing {
    /// Get prompt cost per 1K tokens
    pub fn prompt_cost_per_1k(&self) -> f64 {
        self.prompt_per_token * 1000.0
    }

    /// Get completion cost per 1K tokens
    pub fn completion_cost_per_1k(&self) -> f64 {
        self.completion_per_token * 1000.0
    }

    /// Get prompt cost per 1M tokens
    pub fn prompt_cost_per_1m(&self) -> f64 {
        self.prompt_per_token * 1_000_000.0
    }

    /// Get completion cost per 1M tokens
    pub fn completion_cost_per_1m(&self) -> f64 {
        self.completion_per_token * 1_000_000.0
    }

    /// Get cache read cost per 1M tokens (if available)
    pub fn cache_read_cost_per_1m(&self) -> Option<f64> {
        self.cache_read_per_token.map(|c| c * 1_000_000.0)
    }

    /// Get cache write cost per 1M tokens (if available)
    pub fn cache_write_cost_per_1m(&self) -> Option<f64> {
        self.cache_write_per_token.map(|c| c * 1_000_000.0)
    }

    /// Calculate total cost for a request
    pub fn calculate_cost(&self, prompt_tokens: u32, completion_tokens: u32) -> f64 {
        let prompt_cost = self.prompt_per_token * prompt_tokens as f64;
        let completion_cost = self.completion_per_token * completion_tokens as f64;

        prompt_cost + completion_cost
    }

    /// Calculate total cost for a request with cache hits
    pub fn calculate_cost_with_cache(
        &self,
        prompt_tokens: u32,
        completion_tokens: u32,
        cache_read_tokens: u32,
        cache_write_tokens: u32,
    ) -> f64 {
        let prompt_cost = self.prompt_per_token * prompt_tokens as f64;
        let completion_cost = self.completion_per_token * completion_tokens as f64;
        let cache_read_cost = self
            .cache_read_per_token
            .map(|c| c * cache_read_tokens as f64)
            .unwrap_or(0.0);
        let cache_write_cost = self
            .cache_write_per_token
            .map(|c| c * cache_write_tokens as f64)
            .unwrap_or(0.0);

        prompt_cost + completion_cost + cache_read_cost + cache_write_cost
    }
}
