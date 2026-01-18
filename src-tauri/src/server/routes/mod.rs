//! API route handlers

pub mod chat;
pub mod completions;
pub mod embeddings;
pub mod generation;
pub mod mcp;
pub mod models;
pub mod oauth;

pub use chat::chat_completions;
pub use completions::completions;
pub use embeddings::embeddings;
pub use generation::get_generation;
pub use mcp::{mcp_health_handler, mcp_proxy_handler};
pub use models::{get_model, get_model_pricing, list_models};
pub use oauth::token_endpoint;
