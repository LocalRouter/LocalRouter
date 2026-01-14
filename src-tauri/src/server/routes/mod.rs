//! API route handlers

pub mod chat;
pub mod completions;
pub mod embeddings;
pub mod generation;
pub mod models;

pub use chat::chat_completions;
pub use completions::completions;
pub use embeddings::embeddings;
pub use generation::get_generation;
pub use models::list_models;
