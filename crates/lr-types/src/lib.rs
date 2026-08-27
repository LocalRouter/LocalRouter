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
    /// Cost in millionths of a dollar (integer so the struct stays `Eq`).
    pub cost_micro_usd: u64,
}

impl RecordedRequest {
    /// Convert a dollar amount to the `cost_micro_usd` representation.
    pub fn micro_usd(cost_usd: f64) -> u64 {
        if cost_usd.is_finite() && cost_usd > 0.0 {
            (cost_usd * 1_000_000.0).round() as u64
        } else {
            0
        }
    }
}

/// Trait for recording request usage (used to decouple server from UI)
pub trait TokenRecorder: Send + Sync {
    fn record_request(&self, request: &RecordedRequest);
}
