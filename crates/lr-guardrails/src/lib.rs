//! GuardRails: LLM-based content safety for requests and responses
//!
//! Uses safety models (Llama Guard 4, ShieldGemma, Nemotron, Granite Guardian)
//! to classify content and enforce per-category actions (Allow/Notify/Ask).
//!
//! # Architecture
//!
//! - **SafetyModel trait**: Abstracts over all model implementations
//! - **ModelExecutor**: Routes inference through providers or local GGUF
//! - **SafetyEngine**: Orchestrates checks across multiple models
//! - **text_extractor**: Extracts text from OpenAI-format request/response JSON

pub mod downloader;
pub mod engine;
pub mod executor;
pub mod models;
pub mod safety_model;
pub mod text_extractor;
pub mod types;

pub use engine::{ProviderInfo, SafetyEngine, SafetyModelConfigInput};
pub use executor::{loaded_model_count, unload_all_models, unload_idle_models};
pub use safety_model::*;
pub use types::*;
