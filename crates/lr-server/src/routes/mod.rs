//! API route handlers

pub mod audio;
pub mod chat;
pub mod completions;
pub mod embeddings;
pub mod generation;
pub mod helpers;
pub mod images;
pub mod mcp;
pub mod mcp_ws;
pub mod models;
pub mod moderations;
pub mod monitor_helpers;
pub mod oauth;
pub mod responses;

pub use audio::{audio_speech, audio_transcriptions, audio_translations};
pub use chat::chat_completions;
pub use completions::completions;
pub use embeddings::embeddings;
pub use generation::get_generation;
pub use images::image_generations;
pub use mcp::{
    elicitation_response_handler, mcp_gateway_get_handler, mcp_gateway_handler,
    sampling_passthrough_response_handler,
};
pub use mcp_ws::mcp_websocket_handler;
pub use models::{get_model, get_model_pricing, list_models};
pub use moderations::moderations;
pub use oauth::token_endpoint;
pub use responses::create_response;
