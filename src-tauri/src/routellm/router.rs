//! RouteLLM Router wrapper
//!
//! This module provides a thin wrapper around the CandleRouter
//! to integrate with LocalRouter AI's architecture.

use crate::routellm::candle_router::CandleRouter;
use crate::routellm::errors::RouteLLMResult;
use std::path::Path;

/// Wrapper around CandleRouter (Candle-based BERT classifier)
pub struct RouterWrapper {
    router: CandleRouter,
}

impl RouterWrapper {
    /// Create a new router from SafeTensors model and tokenizer paths
    ///
    /// # Arguments
    /// * `model_path` - Path to directory containing model.safetensors
    /// * `tokenizer_path` - Path to directory containing tokenizer.json
    pub fn new(model_path: &Path, tokenizer_path: &Path) -> RouteLLMResult<Self> {
        let router = CandleRouter::new(model_path, tokenizer_path)?;
        Ok(Self { router })
    }

    /// Calculate strong model win rate for a prompt
    ///
    /// Returns a value between 0.0 and 1.0:
    /// - Higher values (closer to 1.0) suggest using a strong model
    /// - Lower values (closer to 0.0) suggest using a weak model
    pub fn calculate_strong_win_rate(&self, prompt: &str) -> RouteLLMResult<f32> {
        self.router.calculate_strong_win_rate(prompt)
    }
}

// Implement Send + Sync to allow sharing across threads
unsafe impl Send for RouterWrapper {}
unsafe impl Sync for RouterWrapper {}
