//! GuardRails: Content inspection for LLM requests and responses
//!
//! Provides optional per-client guardrails that scan both requests and responses
//! for prompt injection, jailbreaks, PII leakage, code injection, and more.
//!
//! # Architecture
//!
//! - **Engine**: Loads compiled rule sets, provides `check_input()` and `check_output()`
//! - **Sources**: Dynamically downloaded at runtime (regex patterns, YARA rules, ML models)
//! - **Built-in**: ~50 hardcoded high-confidence patterns always available
//! - **Source Manager**: Downloads, caches, updates, and hot-reloads rule sources
//!
//! # Usage
//!
//! ```rust,no_run
//! use lr_guardrails::{GuardrailsEngine, SourceManager};
//! use std::path::PathBuf;
//!
//! let source_manager = SourceManager::new(PathBuf::from("/tmp/guardrails"));
//! let engine = GuardrailsEngine::new(source_manager);
//!
//! let body = serde_json::json!({
//!     "messages": [{"role": "user", "content": "Hello!"}]
//! });
//!
//! let result = engine.check_input(&body);
//! if result.has_matches() {
//!     // Handle detection
//! }
//! ```

#![allow(dead_code)]

pub mod compiled_rules;
pub mod engine;
#[cfg(feature = "ml-models")]
pub mod model_manager;
pub mod source_manager;
pub mod sources;
pub mod text_extractor;
pub mod types;

pub use engine::GuardrailsEngine;
pub use source_manager::SourceManager;
pub use types::*;

/// Validate that a regex pattern compiles successfully
pub fn validate_regex_pattern(pattern: &str) -> Result<(), String> {
    regex::Regex::new(pattern)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod integration_tests;
