//! Secret scanning for outbound LLM requests
//!
//! Detects potential secrets (API keys, tokens, passwords, connection strings)
//! in chat messages before they are sent to LLM providers. Uses a multi-stage
//! pipeline: keyword pre-filter -> regex matching -> entropy filtering ->
//! optional ML verification.

pub mod engine;
pub mod entropy;
pub mod patterns;
pub mod regex_engine;
pub mod types;

pub use engine::{SecretScanEngine, SecretScanEngineConfig};
pub use regex_engine::RuleMetadata;
pub use types::{ExtractedText, ScanResult, SecretFinding, SecretScanAction};
