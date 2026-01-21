//! API route handlers

pub mod chat;
pub mod completions;
pub mod embeddings;
pub mod generation;
pub mod mcp;
pub mod mcp_ws;
pub mod models;
pub mod oauth;

pub use chat::chat_completions;
pub use completions::completions;
pub use embeddings::embeddings;
pub use generation::get_generation;
pub use mcp::{elicitation_response_handler, mcp_gateway_handler, mcp_server_handler, mcp_server_streaming_handler};
pub use mcp_ws::mcp_websocket_handler;
pub use models::{get_model, get_model_pricing, list_models};
pub use oauth::token_endpoint;
