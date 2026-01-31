//! Unified OAuth 2.0 browser-based authorization flow
//!
//! This module provides reusable OAuth browser flow components that work for both
//! MCP servers and LLM provider OAuth implementations.
//!
//! # Features
//! - OAuth 2.0 Authorization Code Flow with PKCE (S256)
//! - Multi-port callback server manager for concurrent flows
//! - Token exchange and refresh
//! - CSRF protection with state parameter
//! - OS keychain integration for secure token storage
//! - Flow timeout and cancellation support
//!
//! # Usage Example
//! ```no_run
//! use oauth_browser::{OAuthFlowManager, OAuthFlowConfig};
//!
//! let manager = OAuthFlowManager::new(keychain);
//! let config = OAuthFlowConfig {
//!     client_id: "my-client-id".to_string(),
//!     // ... other config
//! };
//! let start_result = manager.start_flow(config).await?;
//! // Open start_result.auth_url in browser
//! // Poll for completion
//! let result = manager.poll_status(start_result.flow_id)?;
//! ```

mod callback_server;
mod flow_manager;
mod pkce;
mod token_exchange;
mod types;

// Re-export public API
pub use callback_server::CallbackServerManager;
pub use flow_manager::OAuthFlowManager;
pub use pkce::{generate_pkce_challenge, generate_state};
pub use token_exchange::TokenExchanger;
pub use types::{
    FlowId, FlowStatus, OAuthFlowConfig, OAuthFlowResult, OAuthFlowStart, OAuthFlowState,
    OAuthTokens,
};
