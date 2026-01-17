//! GitHub Copilot OAuth provider implementation
//!
//! Implements OAuth 2.0 Device Code Flow for GitHub Copilot subscriptions.
//!
//! Flow:
//! 1. Request device code from GitHub
//! 2. User visits verification URL and enters code
//! 3. Poll for authorization completion
//! 4. Receive access token

use async_trait::async_trait;
use chrono::Utc;
use reqwest::Client;
use serde::Deserialize;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, error, info};

use super::{OAuthCredentials, OAuthFlowResult, OAuthProvider};
use crate::utils::errors::{AppError, AppResult};

const GITHUB_CLIENT_ID: &str = "Ov23li8tweQw6odWQebz";
const GITHUB_DEVICE_CODE_URL: &str = "https://github.com/login/device/code";
const GITHUB_ACCESS_TOKEN_URL: &str = "https://github.com/login/oauth/access_token";

/// GitHub device code response
#[derive(Debug, Deserialize)]
struct DeviceCodeResponse {
    device_code: String,
    user_code: String,
    verification_uri: String,
    expires_in: u64,
    interval: u64,
}

/// GitHub access token response
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum TokenResponse {
    Success {
        access_token: String,
        token_type: String,
        scope: String,
    },
    Pending {
        error: String,
        error_description: String,
    },
}

/// OAuth flow state
#[derive(Debug, Clone)]
struct FlowState {
    device_code: String,
    user_code: String,
    verification_uri: String,
    interval: u64,
    started_at: i64,
    expires_in: u64,
}

/// GitHub Copilot OAuth provider
pub struct GitHubCopilotOAuthProvider {
    client: Client,
    current_flow: Arc<RwLock<Option<FlowState>>>,
}

impl GitHubCopilotOAuthProvider {
    /// Create a new GitHub Copilot OAuth provider
    pub fn new() -> Self {
        Self {
            client: Client::new(),
            current_flow: Arc::new(RwLock::new(None)),
        }
    }
}

impl Default for GitHubCopilotOAuthProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl OAuthProvider for GitHubCopilotOAuthProvider {
    fn provider_id(&self) -> &str {
        "github-copilot"
    }

    fn provider_name(&self) -> &str {
        "GitHub Copilot"
    }

    async fn start_oauth_flow(&self) -> AppResult<OAuthFlowResult> {
        info!("Starting GitHub Copilot OAuth flow");

        // Request device code
        let response = self
            .client
            .post(GITHUB_DEVICE_CODE_URL)
            .header("Accept", "application/json")
            .header("Content-Type", "application/json")
            .json(&serde_json::json!({
                "client_id": GITHUB_CLIENT_ID,
                "scope": "read:user"
            }))
            .send()
            .await
            .map_err(|e| {
                AppError::Provider(format!("Failed to request GitHub device code: {}", e))
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(AppError::Provider(format!(
                "GitHub device code request failed {}: {}",
                status, error_text
            )));
        }

        let device_response: DeviceCodeResponse = response.json().await.map_err(|e| {
            AppError::Provider(format!("Failed to parse device code response: {}", e))
        })?;

        debug!(
            "Device code received: user_code={}, expires_in={}s",
            device_response.user_code, device_response.expires_in
        );

        // Store flow state
        let flow_state = FlowState {
            device_code: device_response.device_code,
            user_code: device_response.user_code.clone(),
            verification_uri: device_response.verification_uri.clone(),
            interval: device_response.interval,
            started_at: Utc::now().timestamp(),
            expires_in: device_response.expires_in,
        };

        *self.current_flow.write().await = Some(flow_state);

        Ok(OAuthFlowResult::Pending {
            user_code: Some(device_response.user_code),
            verification_url: device_response.verification_uri,
            instructions: "Visit the verification URL and enter the code to authorize GitHub Copilot access.".to_string(),
        })
    }

    async fn poll_oauth_status(&self) -> AppResult<OAuthFlowResult> {
        let flow = self.current_flow.read().await;
        let flow_state = flow.as_ref().ok_or_else(|| {
            AppError::Provider("No OAuth flow in progress".to_string())
        })?;

        // Check if expired
        let now = Utc::now().timestamp();
        if now > flow_state.started_at + flow_state.expires_in as i64 {
            return Ok(OAuthFlowResult::Error {
                message: "OAuth flow expired. Please start again.".to_string(),
            });
        }

        // Poll for access token
        let response = self
            .client
            .post(GITHUB_ACCESS_TOKEN_URL)
            .header("Accept", "application/json")
            .header("Content-Type", "application/json")
            .json(&serde_json::json!({
                "client_id": GITHUB_CLIENT_ID,
                "device_code": flow_state.device_code,
                "grant_type": "urn:ietf:params:oauth:grant-type:device_code"
            }))
            .send()
            .await
            .map_err(|e| {
                AppError::Provider(format!("Failed to poll for GitHub access token: {}", e))
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(AppError::Provider(format!(
                "GitHub token request failed {}: {}",
                status, error_text
            )));
        }

        let token_response: TokenResponse = response.json().await.map_err(|e| {
            AppError::Provider(format!("Failed to parse token response: {}", e))
        })?;

        match token_response {
            TokenResponse::Success { access_token, .. } => {
                info!("GitHub Copilot OAuth flow completed successfully");

                // Clear flow state
                drop(flow);
                *self.current_flow.write().await = None;

                let credentials = OAuthCredentials {
                    provider_id: "github-copilot".to_string(),
                    access_token,
                    refresh_token: None, // GitHub Copilot tokens don't have refresh tokens
                    expires_at: None,    // GitHub Copilot tokens don't expire
                    account_id: None,
                    created_at: Utc::now(),
                };

                Ok(OAuthFlowResult::Success { credentials })
            }
            TokenResponse::Pending { error, error_description } => {
                if error == "authorization_pending" {
                    // Still waiting for user
                    Ok(OAuthFlowResult::Pending {
                        user_code: Some(flow_state.user_code.clone()),
                        verification_url: flow_state.verification_uri.clone(),
                        instructions: "Waiting for authorization...".to_string(),
                    })
                } else if error == "slow_down" {
                    // Poll too frequently, back off
                    Ok(OAuthFlowResult::Pending {
                        user_code: Some(flow_state.user_code.clone()),
                        verification_url: flow_state.verification_uri.clone(),
                        instructions: "Polling too frequently, please wait...".to_string(),
                    })
                } else {
                    // Other error
                    error!("GitHub OAuth error: {} - {}", error, error_description);
                    Ok(OAuthFlowResult::Error {
                        message: format!("{}: {}", error, error_description),
                    })
                }
            }
        }
    }

    async fn refresh_tokens(&self, _credentials: &OAuthCredentials) -> AppResult<OAuthCredentials> {
        // GitHub Copilot tokens don't expire, so no refresh needed
        Err(AppError::Provider(
            "GitHub Copilot tokens do not require refresh".to_string(),
        ))
    }

    async fn cancel_oauth_flow(&self) {
        *self.current_flow.write().await = None;
        info!("GitHub Copilot OAuth flow cancelled");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_provider_info() {
        let provider = GitHubCopilotOAuthProvider::new();
        assert_eq!(provider.provider_id(), "github-copilot");
        assert_eq!(provider.provider_name(), "GitHub Copilot");
    }

    #[tokio::test]
    async fn test_cancel_flow() {
        let provider = GitHubCopilotOAuthProvider::new();

        // Manually set a flow
        *provider.current_flow.write().await = Some(FlowState {
            device_code: "test".to_string(),
            user_code: "TEST-CODE".to_string(),
            verification_uri: "https://github.com/login/device".to_string(),
            interval: 5,
            started_at: Utc::now().timestamp(),
            expires_in: 900,
        });

        assert!(provider.current_flow.read().await.is_some());

        provider.cancel_oauth_flow().await;

        assert!(provider.current_flow.read().await.is_none());
    }
}
