//! OAuth browser flow and client management for LocalRouter

pub mod browser;
pub mod clients;

// Re-export browser public API
pub use browser::{
    generate_pkce_challenge, generate_state, CallbackServerManager, FlowId, FlowStatus,
    OAuthFlowConfig, OAuthFlowManager, OAuthFlowResult, OAuthFlowStart, OAuthFlowState,
    OAuthTokens, TokenExchanger,
};
