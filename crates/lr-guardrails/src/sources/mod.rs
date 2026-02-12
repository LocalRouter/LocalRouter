//! Guardrail source implementations
//!
//! Sources provide rules from various origins:
//! - Built-in: hardcoded high-confidence patterns
//! - Regex: JSON pattern files downloaded from GitHub
//! - YARA: .yar file parser that extracts patterns as regex
//! - Model: Candle ML models (feature-gated behind `ml-models`)

pub mod builtin;
pub mod model_source;
pub mod python_source;
pub mod regex_source;
pub mod yara_source;

use crate::types::RawRule;
use lr_types::AppResult;

/// Trait for guardrail rule sources
pub trait GuardrailSource: Send + Sync {
    /// Unique identifier for this source
    fn id(&self) -> &str;

    /// Human-readable label
    fn label(&self) -> &str;

    /// Parse raw data into rules
    fn parse_rules(&self, data: &[u8]) -> AppResult<Vec<RawRule>>;
}
