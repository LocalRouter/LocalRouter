//! OpenAI Codex OAuth provider implementation
//!
//! Implements OAuth 2.0 with PKCE for OpenAI ChatGPT Plus/Pro subscriptions using
//! the unified oauth_browser module.
//!
//! Flow:
//! 1. Start OAuth flow via unified OAuthFlowManager
//! 2. User authorizes in browser
//! 3. Callback server captures authorization code
//! 4. Automatic token exchange
//! 5. Parse JWT to extract account ID
//! 6. Tokens stored in keychain

use async_trait::async_trait;
use base64::{engine::general_purpose, Engine as _};
use serde::Deserialize;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, warn};

use super::{OAuthCredentials, OAuthFlowResult, OAuthProvider};
use crate::api_keys::CachedKeychain;
use crate::oauth_browser::{FlowId, OAuthFlowConfig, OAuthFlowManager};
use crate::utils::errors::{AppError, AppResult};

const OPENAI_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const OPENAI_AUTHORIZE_URL: &str = "https://auth.openai.com/authorize";
const OPENAI_TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
const REDIRECT_URI: &str = "http://127.0.0.1:1455/callback";
pub const CALLBACK_PORT: u16 = 1455;

/// JWT payload (simplified, for extracting account ID)
#[derive(Debug, Deserialize)]
struct JwtPayload {
    #[serde(rename = "https://api.openai.com/auth")]
    auth_info: Option<AuthInfo>,
}

#[derive(Debug, Deserialize)]
struct AuthInfo {
    user_id: Option<String>,
}

/// OpenAI Codex OAuth provider
pub struct OpenAICodexOAuthProvider {
    /// Unified OAuth flow manager
    flow_manager: Arc<OAuthFlowManager>,

    /// Current active flow ID
    current_flow: Arc<RwLock<Option<FlowId>>>,
}

impl OpenAICodexOAuthProvider {
    /// Create a new OpenAI Codex OAuth provider
    pub fn new(keychain: CachedKeychain) -> Self {
        Self {
            flow_manager: Arc::new(OAuthFlowManager::new(keychain)),
            current_flow: Arc::new(RwLock::new(None)),
        }
    }

    /// Parse JWT without verification (for extracting account ID)
    fn parse_jwt_payload(token: &str) -> AppResult<JwtPayload> {
        let parts: Vec<&str> = token.split('.').collect();
        if parts.len() != 3 {
            return Err(AppError::Provider("Invalid JWT format".to_string()));
        }

        let payload_b64 = parts[1];
        let payload_bytes = general_purpose::STANDARD
            .decode(payload_b64)
            .map_err(|e| AppError::Provider(format!("Failed to decode JWT payload: {}", e)))?;

        let payload: JwtPayload = serde_json::from_slice(&payload_bytes)
            .map_err(|e| AppError::Provider(format!("Failed to parse JWT payload: {}", e)))?;

        Ok(payload)
    }
}

impl Default for OpenAICodexOAuthProvider {
    fn default() -> Self {
        Self::new(CachedKeychain::system())
    }
}

#[async_trait]
impl OAuthProvider for OpenAICodexOAuthProvider {
    fn provider_id(&self) -> &str {
        "openai-codex"
    }

    fn provider_name(&self) -> &str {
        "OpenAI ChatGPT Plus/Pro"
    }

    async fn start_oauth_flow(&self) -> AppResult<OAuthFlowResult> {
        info!("Starting OpenAI Codex OAuth flow");

        // Create unified OAuth flow config
        let config = OAuthFlowConfig {
            client_id: OPENAI_CLIENT_ID.to_string(),
            client_secret: None, // OpenAI uses public client (PKCE only)
            auth_url: OPENAI_AUTHORIZE_URL.to_string(),
            token_url: OPENAI_TOKEN_URL.to_string(),
            scopes: vec![
                "openid".to_string(),
                "profile".to_string(),
                "email".to_string(),
            ],
            redirect_uri: REDIRECT_URI.to_string(),
            callback_port: CALLBACK_PORT,
            keychain_service: "LocalRouter-ProviderTokens".to_string(),
            account_id: "openai-codex".to_string(),
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
            instructions: "Click the link to authorize with your OpenAI account. You will be redirected back to LocalRouter AI.".to_string(),
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

                // Parse JWT to extract account ID
                let account_id = Self::parse_jwt_payload(&tokens.access_token)
                    .ok()
                    .and_then(|p| p.auth_info)
                    .and_then(|a| a.user_id);

                // Convert to provider credentials format
                let credentials = OAuthCredentials {
                    provider_id: "openai-codex".to_string(),
                    access_token: tokens.access_token,
                    refresh_token: tokens.refresh_token,
                    expires_at: tokens.expires_at.map(|dt| dt.timestamp()),
                    account_id,
                    created_at: tokens.acquired_at,
                };

                info!("OpenAI Codex OAuth flow completed successfully");
                Ok(OAuthFlowResult::Success { credentials })
            }
            crate::oauth_browser::OAuthFlowResult::Error { message } => {
                // Clean up flow tracking
                *self.current_flow.write().await = None;

                warn!("OpenAI Codex OAuth flow failed: {}", message);
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

        info!("Refreshing OpenAI Codex tokens");

        // Create config for token refresh
        // Note: OpenAI uses JSON body for token requests instead of form-encoded
        let mut extra_token_params = std::collections::HashMap::new();
        extra_token_params.insert("_use_json_body".to_string(), "true".to_string());

        let config = OAuthFlowConfig {
            client_id: OPENAI_CLIENT_ID.to_string(),
            client_secret: None,
            auth_url: OPENAI_AUTHORIZE_URL.to_string(),
            token_url: OPENAI_TOKEN_URL.to_string(),
            scopes: vec![
                "openid".to_string(),
                "profile".to_string(),
                "email".to_string(),
            ],
            redirect_uri: REDIRECT_URI.to_string(),
            callback_port: CALLBACK_PORT,
            keychain_service: "LocalRouter-ProviderTokens".to_string(),
            account_id: "openai-codex".to_string(),
            extra_auth_params: std::collections::HashMap::new(),
            extra_token_params,
        };

        // Use unified token exchanger
        let token_exchanger = crate::oauth_browser::TokenExchanger::new();
        let keychain = CachedKeychain::system();

        let new_tokens = token_exchanger
            .refresh_tokens(&config, refresh_token, &keychain)
            .await?;

        // Parse JWT to extract account ID
        let account_id = Self::parse_jwt_payload(&new_tokens.access_token)
            .ok()
            .and_then(|p| p.auth_info)
            .and_then(|a| a.user_id);

        // Convert to provider credentials format
        let new_credentials = OAuthCredentials {
            provider_id: "openai-codex".to_string(),
            access_token: new_tokens.access_token,
            refresh_token: new_tokens.refresh_token,
            expires_at: new_tokens.expires_at.map(|dt| dt.timestamp()),
            account_id,
            created_at: new_tokens.acquired_at,
        };

        info!("OpenAI Codex tokens refreshed successfully");

        Ok(new_credentials)
    }

    async fn cancel_oauth_flow(&self) {
        if let Some(flow_id) = self.current_flow.write().await.take() {
            if let Err(e) = self.flow_manager.cancel_flow(flow_id) {
                warn!("Failed to cancel OpenAI Codex OAuth flow: {}", e);
            } else {
                info!("OpenAI Codex OAuth flow cancelled");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_provider_info() {
        let provider = OpenAICodexOAuthProvider::default();
        assert_eq!(provider.provider_id(), "openai-codex");
        assert_eq!(provider.provider_name(), "OpenAI ChatGPT Plus/Pro");
    }

    #[test]
    fn test_constants() {
        assert_eq!(OPENAI_CLIENT_ID, "app_EMoamEEZ73f0CkXaXp7hrann");
        assert_eq!(CALLBACK_PORT, 1455);
        assert!(REDIRECT_URI.contains("1455"));
    }

    #[test]
    fn test_jwt_parsing() {
        // Test JWT with proper payload (simplified)
        let test_jwt = "header.eyJodHRwczovL2FwaS5vcGVuYWkuY29tL2F1dGgiOnsidXNlcl9pZCI6InRlc3RfdXNlciJ9fQ.signature";
        let result = OpenAICodexOAuthProvider::parse_jwt_payload(test_jwt);
        assert!(result.is_ok());
        if let Ok(payload) = result {
            assert!(payload.auth_info.is_some());
            if let Some(auth_info) = payload.auth_info {
                assert_eq!(auth_info.user_id, Some("test_user".to_string()));
            }
        }
    }
}
