//! Core types for unified OAuth browser flows

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// Unique identifier for an OAuth flow
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FlowId(Uuid);

impl FlowId {
    /// Generate a new unique flow ID
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Get the underlying UUID
    pub fn as_uuid(&self) -> &Uuid {
        &self.0
    }
}

impl Default for FlowId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for FlowId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// OAuth flow configuration (input)
#[derive(Debug, Clone)]
pub struct OAuthFlowConfig {
    /// OAuth client ID
    pub client_id: String,

    /// OAuth client secret (optional, for confidential clients)
    pub client_secret: Option<String>,

    /// Authorization endpoint URL
    pub auth_url: String,

    /// Token endpoint URL
    pub token_url: String,

    /// Requested scopes
    pub scopes: Vec<String>,

    /// Redirect URI for OAuth callbacks
    pub redirect_uri: String,

    /// Port for local callback server
    pub callback_port: u16,

    /// Keychain service name for storing tokens
    /// e.g., "LocalRouter-McpServerTokens" or "LocalRouter-ProviderTokens"
    pub keychain_service: String,

    /// Account identifier for keychain storage
    /// e.g., server_id or provider_id
    pub account_id: String,

    /// Additional authorization parameters (optional)
    pub extra_auth_params: HashMap<String, String>,

    /// Additional token exchange parameters (optional)
    pub extra_token_params: HashMap<String, String>,
}

/// OAuth flow state (internal tracking)
#[derive(Debug, Clone)]
pub struct OAuthFlowState {
    /// Unique flow identifier
    pub flow_id: FlowId,

    /// Flow configuration
    pub config: OAuthFlowConfig,

    /// PKCE code verifier
    pub code_verifier: String,

    /// CSRF state parameter
    pub csrf_state: String,

    /// Full authorization URL for user
    pub auth_url: String,

    /// When this flow started
    pub started_at: DateTime<Utc>,

    /// Current flow status
    pub status: FlowStatus,

    /// OAuth tokens (if completed)
    pub tokens: Option<OAuthTokens>,
}

/// OAuth flow status
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FlowStatus {
    /// Waiting for user to complete authorization in browser
    Pending,

    /// Exchanging authorization code for tokens
    ExchangingToken,

    /// Successfully completed
    Success,

    /// Failed with error
    Error { message: String },

    /// Timed out (default: 5 minutes)
    Timeout,

    /// Cancelled by user or system
    Cancelled,
}

/// OAuth tokens (result)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthTokens {
    /// Access token for API requests
    pub access_token: String,

    /// Refresh token (if available)
    pub refresh_token: Option<String>,

    /// Token type (usually "Bearer")
    pub token_type: String,

    /// Token expiration in seconds (if provided by server)
    pub expires_in: Option<i64>,

    /// Absolute expiration timestamp (calculated from expires_in)
    pub expires_at: Option<DateTime<Utc>>,

    /// Granted scope (may differ from requested)
    pub scope: Option<String>,

    /// When tokens were acquired
    pub acquired_at: DateTime<Utc>,
}

/// Result of starting a flow
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthFlowStart {
    /// Unique flow identifier for polling
    pub flow_id: FlowId,

    /// Authorization URL to open in browser
    pub auth_url: String,

    /// CSRF state parameter
    pub state: String,

    /// Redirect URI used
    pub redirect_uri: String,
}

/// Result of polling or completing a flow
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum OAuthFlowResult {
    /// Still waiting for user authorization
    Pending {
        /// Time remaining in seconds (if timeout is configured)
        time_remaining: Option<i64>,
    },

    /// Exchanging authorization code for tokens
    ExchangingToken,

    /// Successfully completed with tokens
    Success {
        /// The acquired tokens
        tokens: OAuthTokens,
    },

    /// Failed with error message
    Error {
        /// Error description
        message: String,
    },

    /// Flow timed out
    Timeout,

    /// Flow was cancelled
    Cancelled,
}

impl OAuthFlowResult {
    /// Check if the flow is still in progress
    pub fn is_pending(&self) -> bool {
        matches!(
            self,
            OAuthFlowResult::Pending { .. } | OAuthFlowResult::ExchangingToken
        )
    }

    /// Check if the flow is complete (success or failure)
    pub fn is_complete(&self) -> bool {
        !self.is_pending()
    }

    /// Check if the flow completed successfully
    pub fn is_success(&self) -> bool {
        matches!(self, OAuthFlowResult::Success { .. })
    }

    /// Extract tokens if successful
    pub fn tokens(self) -> Option<OAuthTokens> {
        match self {
            OAuthFlowResult::Success { tokens } => Some(tokens),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_flow_id_uniqueness() {
        let id1 = FlowId::new();
        let id2 = FlowId::new();
        assert_ne!(id1, id2);
    }

    #[test]
    fn test_flow_id_display() {
        let id = FlowId::new();
        let display = format!("{}", id);
        assert!(!display.is_empty());
        assert_eq!(display, id.as_uuid().to_string());
    }

    #[test]
    fn test_flow_result_is_pending() {
        let pending = OAuthFlowResult::Pending {
            time_remaining: Some(300),
        };
        assert!(pending.is_pending());
        assert!(!pending.is_complete());
        assert!(!pending.is_success());

        let exchanging = OAuthFlowResult::ExchangingToken;
        assert!(exchanging.is_pending());
        assert!(!exchanging.is_complete());
    }

    #[test]
    fn test_flow_result_is_complete() {
        let success = OAuthFlowResult::Success {
            tokens: OAuthTokens {
                access_token: "test".to_string(),
                refresh_token: None,
                token_type: "Bearer".to_string(),
                expires_in: None,
                expires_at: None,
                scope: None,
                acquired_at: Utc::now(),
            },
        };
        assert!(!success.is_pending());
        assert!(success.is_complete());
        assert!(success.is_success());

        let error = OAuthFlowResult::Error {
            message: "test error".to_string(),
        };
        assert!(!error.is_pending());
        assert!(error.is_complete());
        assert!(!error.is_success());
    }

    #[test]
    fn test_flow_result_extract_tokens() {
        let tokens = OAuthTokens {
            access_token: "test_token".to_string(),
            refresh_token: Some("refresh".to_string()),
            token_type: "Bearer".to_string(),
            expires_in: Some(3600),
            expires_at: None,
            scope: Some("read write".to_string()),
            acquired_at: Utc::now(),
        };

        let success = OAuthFlowResult::Success {
            tokens: tokens.clone(),
        };

        let extracted = success.tokens();
        assert!(extracted.is_some());
        assert_eq!(extracted.unwrap().access_token, "test_token");

        let error = OAuthFlowResult::Error {
            message: "test".to_string(),
        };
        assert!(error.tokens().is_none());
    }
}
