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

/// A completed LLM request as reported to real-time consumers (the tray
/// activity graph). Carries every dimension the tray can break usage down
/// by, so one request can move the global, client, provider and model
/// panels at once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedRequest {
    /// Client id (the `api_key_name` metrics dimension).
    pub client_id: String,
    /// Provider instance name.
    pub provider: String,
    /// Model id in the `{provider_instance}/{model_id}` form used by the
    /// metrics `llm_model:` tier.
    pub model: String,
    /// Input + output tokens.
    pub tokens: u64,
}

/// Trait for recording request usage (used to decouple server from UI)
pub trait TokenRecorder: Send + Sync {
    fn record_request(&self, request: &RecordedRequest);
}
