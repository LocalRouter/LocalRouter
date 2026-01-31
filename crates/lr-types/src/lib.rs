//! Shared types, error types, and traits for LocalRouter

pub mod errors;
pub mod mcp_types;

pub use errors::{AppError, AppResult};
pub use mcp_types::McpTool;

/// Trait for recording token usage (used to decouple server from UI)
pub trait TokenRecorder: Send + Sync {
    fn record_tokens(&self, tokens: u64);
}
