//! Shared types, error types, and traits for LocalRouter

pub mod errors;
pub mod fuzzy;
pub mod mcp_types;
pub mod trace;

pub use errors::{AppError, AppResult};
pub use mcp_types::McpTool;
pub use trace::{
    current_outbound_trace, is_duplicate_hop, spawn_traced, with_outbound_trace, RequestTrace,
    TRACE_HEADER,
};

/// Trait for recording token usage (used to decouple server from UI)
pub trait TokenRecorder: Send + Sync {
    fn record_tokens(&self, tokens: u64);
}
