//! System One decision protocol support.
//!
//! - [`types`]: wire types for `POST /v1/systemone` and validation.
//! - [`provider`]: native providers (TypeSafe Jev, Laya, Kev, generic).
//! - [`gateway`]: System One on multi-model gateways (OpenRouter, LLM
//!   Gateway, Vercel AI Gateway, Cloudflare Workers AI).
//! - [`emulation`]: translation of System One requests onto chat completions.

pub mod emulation;
pub mod gateway;
pub mod provider;
pub mod types;

pub use gateway::SystemOneGateway;
pub use provider::{SystemOneFlavor, SystemOneProvider, TYPESAFE_REQUEST_ID_HEADER};
pub use types::{
    answer_from_distribution, confidence_from_probabilities, validate_systemone_request,
    SystemOneAnswer, SystemOneBackend, SystemOneQuestion, SystemOneRequest, SystemOneResponse,
    SystemOneUsage,
};
