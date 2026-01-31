//! OAuth token exchange and refresh logic

use lr_api_keys::{keychain_trait::KeychainStorage, CachedKeychain};
use lr_oauth::browser::{OAuthFlowConfig, OAuthTokens};
use lr_types::{AppError, AppResult};
use chrono::{Duration, Utc};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::{debug, error, info};

/// Token response from OAuth server
#[derive(Debug, Deserialize, Serialize)]
struct TokenResponse {
    /// Access token
    access_token: String,

    /// Token type (usually "Bearer")
    #[serde(default)]
    token_type: String,

    /// Expires in seconds
    #[serde(default)]
    expires_in: Option<i64>,

    /// Refresh token (optional)
    #[serde(default)]
    refresh_token: Option<String>,

    /// Granted scope (optional)
    #[serde(default)]
    scope: Option<String>,
}

/// Token exchanger for OAuth flows
pub struct TokenExchanger {
    client: Client,
}

impl TokenExchanger {
    /// Create a new token exchanger
    pub fn new() -> Self {
        Self {
            client: Client::new(),
        }
    }

    /// Exchange authorization code for access token
    ///
    /// # Arguments
    /// * `config` - OAuth flow configuration
    /// * `authorization_code` - Authorization code from callback
    /// * `code_verifier` - PKCE code verifier
    /// * `keychain` - Keychain for storing tokens
    ///
    /// # Returns
    /// * OAuth tokens (access, refresh, expiration)
    pub async fn exchange_code(
        &self,
        config: &OAuthFlowConfig,
        authorization_code: &str,
        code_verifier: &str,
        keychain: &CachedKeychain,
    ) -> AppResult<OAuthTokens> {
        info!(
            "Exchanging authorization code for token: {}",
            config.account_id
        );

        // Build token request parameters
        let mut params = HashMap::new();
        params.insert("grant_type".to_string(), "authorization_code".to_string());
        params.insert("code".to_string(), authorization_code.to_string());
        params.insert("redirect_uri".to_string(), config.redirect_uri.clone());
        params.insert("client_id".to_string(), config.client_id.clone());
        params.insert("code_verifier".to_string(), code_verifier.to_string());

        // Add client secret if configured (for confidential clients)
        if let Some(ref client_secret) = config.client_secret {
            params.insert("client_secret".to_string(), client_secret.clone());
        } else {
            // Try to retrieve client secret from keychain
            if let Ok(Some(secret)) = keychain.get(
                &config.keychain_service,
                &format!("{}_client_secret", config.account_id),
            ) {
                debug!("Using client secret from keychain");
                params.insert("client_secret".to_string(), secret);
            }
        }

        // Add extra token parameters
        for (key, value) in &config.extra_token_params {
            params.insert(key.clone(), value.clone());
        }

        // Send token request
        let response = self
            .client
            .post(&config.token_url)
            .form(&params)
            .send()
            .await
            .map_err(|e| AppError::OAuthBrowser(format!("Failed to send token request: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            error!("Token exchange failed with status {}: {}", status, body);
            return Err(AppError::OAuthBrowser(format!(
                "Token exchange failed with status {}: {}",
                status, body
            )));
        }

        // Parse token response
        let token_response: TokenResponse = response.json().await.map_err(|e| {
            AppError::OAuthBrowser(format!("Failed to parse token response: {}", e))
        })?;

        // Calculate expiration time (with 5-minute buffer for safety)
        let expires_at = token_response
            .expires_in
            .map(|expires_in| Utc::now() + Duration::seconds(expires_in - 300));

        // Create token structure
        let tokens = OAuthTokens {
            access_token: token_response.access_token.clone(),
            refresh_token: token_response.refresh_token.clone(),
            token_type: token_response.token_type.clone(),
            expires_in: token_response.expires_in,
            expires_at,
            scope: token_response.scope.clone(),
            acquired_at: Utc::now(),
        };

        // Store tokens in keychain
        self.store_tokens(&tokens, config, keychain)?;

        info!("Token exchange successful for: {}", config.account_id);

        Ok(tokens)
    }

    /// Refresh tokens using refresh token
    ///
    /// # Arguments
    /// * `config` - OAuth flow configuration
    /// * `refresh_token` - Refresh token
    /// * `keychain` - Keychain for storing tokens
    ///
    /// # Returns
    /// * New OAuth tokens
    pub async fn refresh_tokens(
        &self,
        config: &OAuthFlowConfig,
        refresh_token: &str,
        keychain: &CachedKeychain,
    ) -> AppResult<OAuthTokens> {
        info!("Refreshing tokens for: {}", config.account_id);

        // Build refresh request parameters
        let mut params = HashMap::new();
        params.insert("grant_type".to_string(), "refresh_token".to_string());
        params.insert("refresh_token".to_string(), refresh_token.to_string());
        params.insert("client_id".to_string(), config.client_id.clone());

        // Add client secret if available
        if let Some(ref client_secret) = config.client_secret {
            params.insert("client_secret".to_string(), client_secret.clone());
        } else if let Ok(Some(secret)) = keychain.get(
            &config.keychain_service,
            &format!("{}_client_secret", config.account_id),
        ) {
            params.insert("client_secret".to_string(), secret);
        }

        // Add extra token parameters
        for (key, value) in &config.extra_token_params {
            params.insert(key.clone(), value.clone());
        }

        // Send refresh request
        let response = self
            .client
            .post(&config.token_url)
            .form(&params)
            .send()
            .await
            .map_err(|e| {
                AppError::OAuthBrowser(format!("Failed to send refresh request: {}", e))
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            error!("Token refresh failed with status {}: {}", status, body);
            return Err(AppError::OAuthBrowser(format!(
                "Token refresh failed with status {}: {}",
                status, body
            )));
        }

        // Parse token response
        let token_response: TokenResponse = response.json().await.map_err(|e| {
            AppError::OAuthBrowser(format!("Failed to parse refresh response: {}", e))
        })?;

        // Calculate expiration time (with 5-minute buffer)
        let expires_at = token_response
            .expires_in
            .map(|expires_in| Utc::now() + Duration::seconds(expires_in - 300));

        // Create token structure (preserve original refresh token if not provided)
        let tokens = OAuthTokens {
            access_token: token_response.access_token.clone(),
            refresh_token: token_response
                .refresh_token
                .or_else(|| Some(refresh_token.to_string())),
            token_type: token_response.token_type.clone(),
            expires_in: token_response.expires_in,
            expires_at,
            scope: token_response.scope.clone(),
            acquired_at: Utc::now(),
        };

        // Store tokens in keychain
        self.store_tokens(&tokens, config, keychain)?;

        info!("Token refresh successful for: {}", config.account_id);

        Ok(tokens)
    }

    /// Store tokens in keychain
    fn store_tokens(
        &self,
        tokens: &OAuthTokens,
        config: &OAuthFlowConfig,
        keychain: &CachedKeychain,
    ) -> AppResult<()> {
        // Store access token
        keychain
            .store(
                &config.keychain_service,
                &format!("{}_access_token", config.account_id),
                &tokens.access_token,
            )
            .map_err(|e| AppError::OAuthBrowser(format!("Failed to store access token: {}", e)))?;

        // Store refresh token if available
        if let Some(ref refresh_token) = tokens.refresh_token {
            keychain
                .store(
                    &config.keychain_service,
                    &format!("{}_refresh_token", config.account_id),
                    refresh_token,
                )
                .map_err(|e| {
                    AppError::OAuthBrowser(format!("Failed to store refresh token: {}", e))
                })?;
        }

        debug!("Tokens stored in keychain for: {}", config.account_id);

        Ok(())
    }
}

impl Default for TokenExchanger {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_token_exchanger_creation() {
        let _exchanger = TokenExchanger::new();
        // TokenExchanger created successfully
    }

    #[test]
    fn test_token_response_deserialization() {
        let json = r#"{
            "access_token": "test_access",
            "token_type": "Bearer",
            "expires_in": 3600,
            "refresh_token": "test_refresh"
        }"#;

        let response: TokenResponse = serde_json::from_str(json).unwrap();
        assert_eq!(response.access_token, "test_access");
        assert_eq!(response.token_type, "Bearer");
        assert_eq!(response.expires_in, Some(3600));
        assert_eq!(response.refresh_token, Some("test_refresh".to_string()));
    }

    #[test]
    fn test_token_response_minimal() {
        let json = r#"{
            "access_token": "test_access"
        }"#;

        let response: TokenResponse = serde_json::from_str(json).unwrap();
        assert_eq!(response.access_token, "test_access");
        assert_eq!(response.token_type, ""); // default
        assert_eq!(response.expires_in, None);
        assert_eq!(response.refresh_token, None);
    }
}
