//! OAuth authentication for subscription-based LLM providers
//!
//! This module implements OAuth flows for providers that require subscription-based authentication:
//! - GitHub Copilot (Device Code Flow)
//! - OpenAI Codex/ChatGPT Plus (PKCE OAuth Flow)
//!
//! OAuth credentials are stored separately from API keys in encrypted storage.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use chrono::{DateTime, Utc};
use parking_lot::RwLock;

use crate::utils::errors::{AppError, AppResult};

pub mod anthropic_claude;
pub mod github_copilot;
pub mod openai_codex;
pub mod storage;

/// OAuth authentication credentials
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthCredentials {
    /// Provider ID (e.g., "github-copilot", "openai-codex")
    pub provider_id: String,
    /// Access token
    pub access_token: String,
    /// Refresh token (if available)
    pub refresh_token: Option<String>,
    /// Token expiration timestamp (Unix timestamp)
    pub expires_at: Option<i64>,
    /// Account/Organization ID (if available)
    pub account_id: Option<String>,
    /// When these credentials were created
    pub created_at: DateTime<Utc>,
}

impl OAuthCredentials {
    /// Check if the access token is expired
    pub fn is_expired(&self) -> bool {
        if let Some(expires_at) = self.expires_at {
            // Consider expired if less than 5 minutes remaining
            Utc::now().timestamp() + 300 > expires_at
        } else {
            false // No expiration
        }
    }
}

/// OAuth flow result
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OAuthFlowResult {
    /// OAuth flow initiated, waiting for user action
    Pending {
        /// User code to enter (for device code flow)
        user_code: Option<String>,
        /// Verification URL
        verification_url: String,
        /// Instructions for the user
        instructions: String,
    },
    /// OAuth flow completed successfully
    Success {
        /// OAuth credentials
        credentials: OAuthCredentials,
    },
    /// OAuth flow failed
    Error {
        /// Error message
        message: String,
    },
}

/// OAuth provider trait
#[async_trait::async_trait]
pub trait OAuthProvider: Send + Sync {
    /// Provider ID (e.g., "github-copilot")
    fn provider_id(&self) -> &str;

    /// Provider display name
    fn provider_name(&self) -> &str;

    /// Start the OAuth flow
    ///
    /// Returns a pending result with instructions for the user, or immediate success/error
    async fn start_oauth_flow(&self) -> AppResult<OAuthFlowResult>;

    /// Poll for OAuth completion (for device code flow)
    ///
    /// Returns Success when complete, Pending while waiting, or Error if failed/expired
    async fn poll_oauth_status(&self) -> AppResult<OAuthFlowResult>;

    /// Refresh OAuth tokens
    ///
    /// Returns new credentials or error if refresh failed
    async fn refresh_tokens(&self, credentials: &OAuthCredentials) -> AppResult<OAuthCredentials>;

    /// Cancel the current OAuth flow
    async fn cancel_oauth_flow(&self);
}

/// OAuth manager for handling multiple OAuth providers
pub struct OAuthManager {
    /// Active OAuth providers
    providers: RwLock<HashMap<String, Arc<dyn OAuthProvider>>>,
    /// Stored OAuth credentials
    storage: Arc<storage::OAuthStorage>,
}

impl OAuthManager {
    /// Create a new OAuth manager
    pub fn new(storage: Arc<storage::OAuthStorage>) -> Self {
        Self {
            providers: RwLock::new(HashMap::new()),
            storage,
        }
    }

    /// Register an OAuth provider
    pub fn register_provider(&self, provider: Arc<dyn OAuthProvider>) {
        let provider_id = provider.provider_id().to_string();
        self.providers.write().insert(provider_id, provider);
    }

    /// Get an OAuth provider by ID
    pub fn get_provider(&self, provider_id: &str) -> Option<Arc<dyn OAuthProvider>> {
        self.providers.read().get(provider_id).cloned()
    }

    /// List all available OAuth providers
    pub fn list_providers(&self) -> Vec<(String, String)> {
        self.providers
            .read()
            .values()
            .map(|p| (p.provider_id().to_string(), p.provider_name().to_string()))
            .collect()
    }

    /// Start OAuth flow for a provider
    pub async fn start_oauth(&self, provider_id: &str) -> AppResult<OAuthFlowResult> {
        let provider = self.get_provider(provider_id).ok_or_else(|| {
            AppError::Provider(format!("OAuth provider '{}' not found", provider_id))
        })?;

        provider.start_oauth_flow().await
    }

    /// Poll OAuth status for a provider
    pub async fn poll_oauth(&self, provider_id: &str) -> AppResult<OAuthFlowResult> {
        let provider = self.get_provider(provider_id).ok_or_else(|| {
            AppError::Provider(format!("OAuth provider '{}' not found", provider_id))
        })?;

        let result = provider.poll_oauth_status().await?;

        // If successful, store the credentials
        if let OAuthFlowResult::Success { ref credentials } = result {
            self.storage.store_credentials(credentials).await?;
        }

        Ok(result)
    }

    /// Cancel OAuth flow for a provider
    pub async fn cancel_oauth(&self, provider_id: &str) -> AppResult<()> {
        let provider = self.get_provider(provider_id).ok_or_else(|| {
            AppError::Provider(format!("OAuth provider '{}' not found", provider_id))
        })?;

        provider.cancel_oauth_flow().await;
        Ok(())
    }

    /// Get stored credentials for a provider
    pub async fn get_credentials(&self, provider_id: &str) -> AppResult<Option<OAuthCredentials>> {
        self.storage.get_credentials(provider_id).await
    }

    /// Get valid (non-expired) credentials, refreshing if needed
    pub async fn get_valid_credentials(
        &self,
        provider_id: &str,
    ) -> AppResult<Option<OAuthCredentials>> {
        let credentials = self.storage.get_credentials(provider_id).await?;

        if let Some(creds) = credentials {
            if creds.is_expired() && creds.refresh_token.is_some() {
                // Try to refresh
                let provider = self.get_provider(provider_id).ok_or_else(|| {
                    AppError::Provider(format!("OAuth provider '{}' not found", provider_id))
                })?;

                match provider.refresh_tokens(&creds).await {
                    Ok(new_creds) => {
                        self.storage.store_credentials(&new_creds).await?;
                        Ok(Some(new_creds))
                    }
                    Err(_) => {
                        // Refresh failed, return None (user needs to re-authenticate)
                        Ok(None)
                    }
                }
            } else {
                Ok(Some(creds))
            }
        } else {
            Ok(None)
        }
    }

    /// Delete stored credentials for a provider
    pub async fn delete_credentials(&self, provider_id: &str) -> AppResult<()> {
        self.storage.delete_credentials(provider_id).await
    }

    /// List all providers with stored credentials
    pub async fn list_authenticated_providers(&self) -> AppResult<Vec<String>> {
        self.storage.list_providers().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_credentials_expiration() {
        let creds = OAuthCredentials {
            provider_id: "test".to_string(),
            access_token: "token".to_string(),
            refresh_token: None,
            expires_at: Some(Utc::now().timestamp() - 3600), // Expired 1 hour ago
            account_id: None,
            created_at: Utc::now(),
        };

        assert!(creds.is_expired());
    }

    #[test]
    fn test_credentials_not_expired() {
        let creds = OAuthCredentials {
            provider_id: "test".to_string(),
            access_token: "token".to_string(),
            refresh_token: None,
            expires_at: Some(Utc::now().timestamp() + 3600), // Expires in 1 hour
            account_id: None,
            created_at: Utc::now(),
        };

        assert!(!creds.is_expired());
    }

    #[test]
    fn test_credentials_no_expiration() {
        let creds = OAuthCredentials {
            provider_id: "test".to_string(),
            access_token: "token".to_string(),
            refresh_token: None,
            expires_at: None, // Never expires
            account_id: None,
            created_at: Utc::now(),
        };

        assert!(!creds.is_expired());
    }
}
