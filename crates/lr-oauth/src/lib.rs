//! OAuth browser flow and client management for LocalRouter

pub mod browser;
pub mod clients;

// Re-export browser public API
pub use browser::{
    CallbackServerManager, OAuthFlowManager, OAuthFlowConfig, OAuthFlowResult, OAuthFlowStart,
    OAuthFlowState, OAuthTokens, FlowId, FlowStatus,
    generate_pkce_challenge, generate_state,
    TokenExchanger,
};
