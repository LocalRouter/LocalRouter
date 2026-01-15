//! OpenAI Codex OAuth provider implementation
//!
//! Implements OAuth 2.0 with PKCE for OpenAI ChatGPT Plus/Pro subscriptions.
//!
//! Flow:
//! 1. Generate PKCE code verifier and challenge
//! 2. Open authorization URL in browser
//! 3. Start local callback server to receive authorization code
//! 4. Exchange authorization code for access/refresh tokens
//! 5. Parse JWT to extract account ID

use async_trait::async_trait;
use base64::{engine::general_purpose, Engine as _};
use chrono::Utc;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use tokio::sync::{RwLock, oneshot};
use tracing::{debug, info};

use super::{OAuthCredentials, OAuthFlowResult, OAuthProvider};
use crate::utils::errors::{AppError, AppResult};

const OPENAI_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const OPENAI_ISSUER: &str = "https://auth.openai.com";
const OPENAI_AUTHORIZE_URL: &str = "https://auth.openai.com/authorize";
const OPENAI_TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
const REDIRECT_URI: &str = "http://127.0.0.1:1455/callback";

/// PKCE code verifier and challenge
struct PkceChallenge {
    code_verifier: String,
    code_challenge: String,
}

/// OAuth flow state
#[derive(Debug, Clone)]
struct FlowState {
    code_verifier: String,
    state: String,
}

/// Token response from OpenAI
#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: String,
    expires_in: u64,
    token_type: String,
}

/// JWT payload (simplified)
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
    client: Client,
    current_flow: Arc<RwLock<Option<FlowState>>>,
    callback_sender: Arc<RwLock<Option<oneshot::Sender<String>>>>,
}

impl OpenAICodexOAuthProvider {
    /// Create a new OpenAI Codex OAuth provider
    pub fn new() -> Self {
        Self {
            client: Client::new(),
            current_flow: Arc::new(RwLock::new(None)),
            callback_sender: Arc::new(RwLock::new(None)),
        }
    }

    /// Generate PKCE code verifier and challenge
    fn generate_pkce() -> AppResult<PkceChallenge> {
        // Generate random code verifier (128 characters)
        let code_verifier: String = (0..128)
            .map(|_| {
                let chars = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~";
                chars[rand::random::<usize>() % chars.len()] as char
            })
            .collect();

        // Generate code challenge (SHA256 hash of code verifier, base64url encoded)
        let mut hasher = Sha256::new();
        hasher.update(code_verifier.as_bytes());
        let hash = hasher.finalize();
        let code_challenge = general_purpose::URL_SAFE_NO_PAD.encode(hash);

        Ok(PkceChallenge {
            code_verifier,
            code_challenge,
        })
    }

    /// Generate random state parameter
    fn generate_state() -> String {
        (0..32)
            .map(|_| {
                let chars = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
                chars[rand::random::<usize>() % chars.len()] as char
            })
            .collect()
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
        Self::new()
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

        // Generate PKCE challenge
        let pkce = Self::generate_pkce()?;
        let state = Self::generate_state();

        // Build authorization URL
        let auth_url = format!(
            "{}?client_id={}&response_type=code&redirect_uri={}&scope={}&code_challenge={}&code_challenge_method=S256&state={}",
            OPENAI_AUTHORIZE_URL,
            OPENAI_CLIENT_ID,
            urlencoding::encode(REDIRECT_URI),
            urlencoding::encode("openid profile email"),
            pkce.code_challenge,
            state
        );

        // Store flow state
        let flow_state = FlowState {
            code_verifier: pkce.code_verifier,
            state: state.clone(),
        };

        *self.current_flow.write().await = Some(flow_state);

        // Create channel for receiving authorization code
        let (tx, _rx) = oneshot::channel();
        *self.callback_sender.write().await = Some(tx);

        // Note: We can't start a local HTTP server in this synchronous context
        // The UI will need to handle the callback and send it to us via poll_oauth_status

        Ok(OAuthFlowResult::Pending {
            user_code: None,
            verification_url: auth_url,
            instructions: "Click the link to authorize in your browser. You will be redirected back to LocalRouter AI.".to_string(),
        })
    }

    async fn poll_oauth_status(&self) -> AppResult<OAuthFlowResult> {
        let flow = self.current_flow.read().await;
        let flow_state = flow.as_ref().ok_or_else(|| {
            AppError::Provider("No OAuth flow in progress".to_string())
        })?;

        // Check if we have received the authorization code
        // (This would be set by an external callback handler)
        // For now, return pending
        Ok(OAuthFlowResult::Pending {
            user_code: None,
            verification_url: "Waiting for browser authorization...".to_string(),
            instructions: "Complete the authorization in your browser".to_string(),
        })
    }

    async fn refresh_tokens(&self, credentials: &OAuthCredentials) -> AppResult<OAuthCredentials> {
        let refresh_token = credentials
            .refresh_token
            .as_ref()
            .ok_or_else(|| AppError::Provider("No refresh token available".to_string()))?;

        debug!("Refreshing OpenAI Codex tokens");

        let response = self
            .client
            .post(OPENAI_TOKEN_URL)
            .header("Content-Type", "application/json")
            .json(&serde_json::json!({
                "client_id": OPENAI_CLIENT_ID,
                "grant_type": "refresh_token",
                "refresh_token": refresh_token
            }))
            .send()
            .await
            .map_err(|e| {
                AppError::Provider(format!("Failed to refresh OpenAI tokens: {}", e))
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(AppError::Provider(format!(
                "Token refresh failed {}: {}",
                status, error_text
            )));
        }

        let token_response: TokenResponse = response.json().await.map_err(|e| {
            AppError::Provider(format!("Failed to parse token response: {}", e))
        })?;

        // Parse JWT to extract account ID
        let account_id = Self::parse_jwt_payload(&token_response.access_token)
            .ok()
            .and_then(|p| p.auth_info)
            .and_then(|a| a.user_id);

        let new_credentials = OAuthCredentials {
            provider_id: "openai-codex".to_string(),
            access_token: token_response.access_token,
            refresh_token: Some(token_response.refresh_token),
            expires_at: Some(Utc::now().timestamp() + token_response.expires_in as i64),
            account_id,
            created_at: Utc::now(),
        };

        info!("OpenAI Codex tokens refreshed successfully");

        Ok(new_credentials)
    }

    async fn cancel_oauth_flow(&self) {
        *self.current_flow.write().await = None;
        *self.callback_sender.write().await = None;
        info!("OpenAI Codex OAuth flow cancelled");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_provider_info() {
        let provider = OpenAICodexOAuthProvider::new();
        assert_eq!(provider.provider_id(), "openai-codex");
        assert_eq!(provider.provider_name(), "OpenAI ChatGPT Plus/Pro");
    }

    #[test]
    fn test_generate_pkce() {
        let pkce = OpenAICodexOAuthProvider::generate_pkce().unwrap();
        assert_eq!(pkce.code_verifier.len(), 128);
        assert!(!pkce.code_challenge.is_empty());
    }

    #[test]
    fn test_generate_state() {
        let state = OpenAICodexOAuthProvider::generate_state();
        assert_eq!(state.len(), 32);
    }
}
