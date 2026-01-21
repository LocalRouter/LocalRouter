//! Anthropic Claude Pro OAuth provider implementation
//!
//! Implements OAuth 2.0 with PKCE for Anthropic Claude Pro subscriptions using
//! the unified oauth_browser module.
//!
//! Flow:
//! 1. Start OAuth flow via unified OAuthFlowManager
//! 2. User authorizes in browser
//! 3. Callback server captures authorization code
//! 4. Automatic token exchange
//! 5. Tokens stored in keychain

use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, warn};

use super::{OAuthCredentials, OAuthFlowResult, OAuthProvider};
use crate::api_keys::CachedKeychain;
use crate::oauth_browser::{FlowId, OAuthFlowConfig, OAuthFlowManager};
use crate::utils::errors::{AppError, AppResult};

const ANTHROPIC_CLIENT_ID: &str = "claude-web";
const ANTHROPIC_AUTHORIZE_URL: &str = "https://console.anthropic.com/oauth/authorize";
const ANTHROPIC_TOKEN_URL: &str = "https://console.anthropic.com/oauth/token";
const REDIRECT_URI: &str = "http://127.0.0.1:1456/callback";
pub const CALLBACK_PORT: u16 = 1456;

/// Anthropic Claude Pro OAuth provider
pub struct AnthropicClaudeOAuthProvider {
    /// Unified OAuth flow manager
    flow_manager: Arc<OAuthFlowManager>,

    /// Current active flow ID
    current_flow: Arc<RwLock<Option<FlowId>>>,
}

impl AnthropicClaudeOAuthProvider {
    /// Create a new Anthropic Claude OAuth provider
    pub fn new(keychain: CachedKeychain) -> Self {
        Self {
            flow_manager: Arc::new(OAuthFlowManager::new(keychain)),
            current_flow: Arc::new(RwLock::new(None)),
        }
    }
}

impl Default for AnthropicClaudeOAuthProvider {
    fn default() -> Self {
        Self::new(CachedKeychain::system())
    }
}

#[async_trait]
impl OAuthProvider for AnthropicClaudeOAuthProvider {
    fn provider_id(&self) -> &str {
        "anthropic-claude"
    }

    fn provider_name(&self) -> &str {
        "Anthropic Claude Pro"
    }

    async fn start_oauth_flow(&self) -> AppResult<OAuthFlowResult> {
        info!("Starting Anthropic Claude OAuth flow");

        // Create unified OAuth flow config
        let config = OAuthFlowConfig {
            client_id: ANTHROPIC_CLIENT_ID.to_string(),
            client_secret: None, // Anthropic uses public client (PKCE only)
            auth_url: ANTHROPIC_AUTHORIZE_URL.to_string(),
            token_url: ANTHROPIC_TOKEN_URL.to_string(),
            scopes: vec!["api".to_string(), "offline_access".to_string()],
            redirect_uri: REDIRECT_URI.to_string(),
            callback_port: CALLBACK_PORT,
            keychain_service: "LocalRouter-ProviderTokens".to_string(),
            account_id: "anthropic-claude".to_string(),
            extra_auth_params: std::collections::HashMap::new(),
            extra_token_params: std::collections::HashMap::new(),
        };

        // Start flow via unified manager
        let start_result = self.flow_manager.start_flow(config).await?;

        // Store flow ID for polling
        *self.current_flow.write().await = Some(start_result.flow_id);

        // Return in provider format
        Ok(OAuthFlowResult::Pending {
            user_code: None,
            verification_url: start_result.auth_url,
            instructions: "Click the link to authorize with your Claude Pro account. You will be redirected back to LocalRouter AI.".to_string(),
        })
    }

    async fn poll_oauth_status(&self) -> AppResult<OAuthFlowResult> {
        let flow_id = self
            .current_flow
            .read()
            .await
            .ok_or_else(|| AppError::Provider("No OAuth flow in progress".to_string()))?;

        // Poll unified flow manager
        let result = self.flow_manager.poll_status(flow_id)?;

        // Convert to provider format
        match result {
            crate::oauth_browser::OAuthFlowResult::Pending { .. } => Ok(OAuthFlowResult::Pending {
                user_code: None,
                verification_url: "Waiting for browser authorization...".to_string(),
                instructions: "Complete the authorization in your browser".to_string(),
            }),
            crate::oauth_browser::OAuthFlowResult::ExchangingToken => {
                Ok(OAuthFlowResult::Pending {
                    user_code: None,
                    verification_url: "Exchanging authorization code for tokens...".to_string(),
                    instructions: "Please wait...".to_string(),
                })
            }
            crate::oauth_browser::OAuthFlowResult::Success { tokens } => {
                // Clean up flow tracking
                *self.current_flow.write().await = None;

                // Convert to provider credentials format
                let credentials = OAuthCredentials {
                    provider_id: "anthropic-claude".to_string(),
                    access_token: tokens.access_token,
                    refresh_token: tokens.refresh_token,
                    expires_at: tokens.expires_at.map(|dt| dt.timestamp()),
                    account_id: None,
                    created_at: tokens.acquired_at,
                };

                info!("Anthropic Claude OAuth flow completed successfully");
                Ok(OAuthFlowResult::Success { credentials })
            }
            crate::oauth_browser::OAuthFlowResult::Error { message } => {
                // Clean up flow tracking
                *self.current_flow.write().await = None;

                warn!("Anthropic Claude OAuth flow failed: {}", message);
                Ok(OAuthFlowResult::Error { message })
            }
            crate::oauth_browser::OAuthFlowResult::Timeout => {
                // Clean up flow tracking
                *self.current_flow.write().await = None;

                Ok(OAuthFlowResult::Error {
                    message: "Authorization timed out after 5 minutes".to_string(),
                })
            }
            crate::oauth_browser::OAuthFlowResult::Cancelled => {
                // Clean up flow tracking
                *self.current_flow.write().await = None;

                Ok(OAuthFlowResult::Error {
                    message: "Authorization was cancelled".to_string(),
                })
            }
        }
    }

    async fn refresh_tokens(&self, credentials: &OAuthCredentials) -> AppResult<OAuthCredentials> {
        let refresh_token = credentials
            .refresh_token
            .as_ref()
            .ok_or_else(|| AppError::Provider("No refresh token available".to_string()))?;

        info!("Refreshing Anthropic Claude tokens");

        // Create config for token refresh
        let config = OAuthFlowConfig {
            client_id: ANTHROPIC_CLIENT_ID.to_string(),
            client_secret: None,
            auth_url: ANTHROPIC_AUTHORIZE_URL.to_string(),
            token_url: ANTHROPIC_TOKEN_URL.to_string(),
            scopes: vec!["api".to_string(), "offline_access".to_string()],
            redirect_uri: REDIRECT_URI.to_string(),
            callback_port: CALLBACK_PORT,
            keychain_service: "LocalRouter-ProviderTokens".to_string(),
            account_id: "anthropic-claude".to_string(),
            extra_auth_params: std::collections::HashMap::new(),
            extra_token_params: std::collections::HashMap::new(),
        };

        // Use unified token exchanger
        let token_exchanger = crate::oauth_browser::TokenExchanger::new();
        let keychain = CachedKeychain::system();

        let new_tokens = token_exchanger
            .refresh_tokens(&config, refresh_token, &keychain)
            .await?;

        // Convert to provider credentials format
        let new_credentials = OAuthCredentials {
            provider_id: "anthropic-claude".to_string(),
            access_token: new_tokens.access_token,
            refresh_token: new_tokens.refresh_token,
            expires_at: new_tokens.expires_at.map(|dt| dt.timestamp()),
            account_id: None,
            created_at: new_tokens.acquired_at,
        };

        info!("Anthropic Claude tokens refreshed successfully");

        Ok(new_credentials)
    }

    async fn cancel_oauth_flow(&self) {
        if let Some(flow_id) = self.current_flow.write().await.take() {
            if let Err(e) = self.flow_manager.cancel_flow(flow_id) {
                warn!("Failed to cancel Anthropic Claude OAuth flow: {}", e);
            } else {
                info!("Anthropic Claude OAuth flow cancelled");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_provider_info() {
        let provider = AnthropicClaudeOAuthProvider::default();
        assert_eq!(provider.provider_id(), "anthropic-claude");
        assert_eq!(provider.provider_name(), "Anthropic Claude Pro");
    }

    #[test]
    fn test_constants() {
        assert_eq!(ANTHROPIC_CLIENT_ID, "claude-web");
        assert_eq!(CALLBACK_PORT, 1456);
        assert!(REDIRECT_URI.contains("1456"));
    }
}
