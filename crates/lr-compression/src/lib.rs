//! Prompt compression via LLMLingua-2 using Candle
//!
//! Runs a BERT token classifier locally to identify which tokens to keep/drop.
//! Extractive compression — keeps exact original tokens, zero hallucination risk.
//!
//! Uses the same Candle framework as lr-routellm for consistency.

pub mod downloader;
pub mod engine;
pub mod model;
pub mod types;

pub use downloader::{is_downloaded, repo_id_for_model};
pub use engine::CompressionService;
pub use types::{CompressedMessage, CompressionResult, CompressionStatus};
