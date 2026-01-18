//! OAuth support for MCP servers
//!
//! Handles OAuth discovery, token acquisition, and token management for MCP servers
//! that require OAuth authentication.

use crate::api_keys::{CachedKeychain, KeychainStorage};
use crate::config::McpOAuthConfig;
use crate::utils::errors::{AppError, AppResult};
use chrono::{DateTime, Duration, Utc};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use parking_lot::RwLock;

/// Keychain service name for MCP server OAuth tokens
const MCP_OAUTH_SERVICE: &str = "LocalRouter-McpServerTokens";

/// OAuth token manager for MCP servers
///
/// Manages OAuth tokens for MCP servers that require authentication.
/// Tokens are cached in the system keyring and refreshed as needed.
pub struct McpOAuthManager {
    /// HTTP client for OAuth requests
    client: Client,

    /// Keychain for storing tokens
    keychain: CachedKeychain,

    /// Cached tokens (server_id -> token info)
    token_cache: Arc<RwLock<HashMap<String, CachedTokenInfo>>>,
}

/// Cached token information
#[derive(Debug, Clone)]
struct CachedTokenInfo {
    /// Access token
    access_token: String,

    /// Token expiration time
    expires_at: DateTime<Utc>,

    /// Refresh token (if available)
    refresh_token: Option<String>,
}

/// OAuth discovery response from .well-known endpoint
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OAuthDiscoveryResponse {
    /// Authorization endpoint URL
    #[serde(rename = "authorization_endpoint")]
    pub auth_url: String,

    /// Token endpoint URL
    pub token_endpoint: String,

    /// Supported scopes
    #[serde(default)]
    pub scopes_supported: Vec<String>,

    /// Supported grant types
    #[serde(default)]
    pub grant_types_supported: Vec<String>,
}

/// OAuth token response
#[derive(Debug, Clone, Deserialize, Serialize)]
struct TokenResponse {
    /// Access token
    access_token: String,

    /// Token type (usually "Bearer")
    token_type: String,

    /// Expires in seconds
    #[serde(default)]
    expires_in: Option<i64>,

    /// Refresh token (if available)
    #[serde(default)]
    refresh_token: Option<String>,

    /// Scope
    #[serde(default)]
    scope: Option<String>,
}

impl McpOAuthManager {
    /// Create a new OAuth manager
    pub fn new() -> Self {
        let keychain = CachedKeychain::auto()
            .expect("Failed to initialize MCP OAuth keychain");

        Self {
            client: Client::new(),
            keychain,
            token_cache: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Discover OAuth configuration for an MCP server
    ///
    /// # Arguments
    /// * `base_url` - Base URL of the MCP server
    ///
    /// # Returns
    /// * OAuth discovery response if the server supports OAuth
    pub async fn discover_oauth(
        &self,
        base_url: &str,
    ) -> AppResult<Option<OAuthDiscoveryResponse>> {
        // Construct .well-known URL
        let discovery_url = format!(
            "{}/.well-known/oauth-protected-resource",
            base_url.trim_end_matches('/')
        );

        tracing::info!("Discovering OAuth configuration at: {}", discovery_url);

        // Attempt to fetch discovery document
        let response = match self.client.get(&discovery_url).send().await {
            Ok(resp) => resp,
            Err(e) => {
                tracing::debug!("OAuth discovery failed (server may not require OAuth): {}", e);
                return Ok(None);
            }
        };

        // Check if discovery endpoint exists
        if !response.status().is_success() {
            tracing::debug!(
                "OAuth discovery returned status {} (server may not require OAuth)",
                response.status()
            );
            return Ok(None);
        }

        // Parse discovery response
        let discovery: OAuthDiscoveryResponse = response
            .json()
            .await
            .map_err(|e| AppError::Mcp(format!("Failed to parse OAuth discovery response: {}", e)))?;

        tracing::info!("OAuth discovery successful: token_endpoint={}", discovery.token_endpoint);

        Ok(Some(discovery))
    }

    /// Acquire an OAuth token for an MCP server
    ///
    /// # Arguments
    /// * `server_id` - MCP server ID
    /// * `oauth_config` - OAuth configuration
    ///
    /// # Returns
    /// * Access token
    pub async fn acquire_token(
        &self,
        server_id: &str,
        oauth_config: &McpOAuthConfig,
    ) -> AppResult<String> {
        // Check cache first
        if let Some(token) = self.get_cached_token(server_id).await {
            return Ok(token);
        }

        tracing::info!("Acquiring OAuth token for MCP server: {}", server_id);

        // Retrieve client_secret from keychain
        let client_secret = self
            .keychain
            .get(MCP_OAUTH_SERVICE, &format!("{}_client_secret", server_id))
            .map_err(|e| AppError::Mcp(format!("Failed to retrieve client secret: {}", e)))?
            .ok_or_else(|| AppError::Mcp("Client secret not found in keychain".to_string()))?;

        // Prepare token request (OAuth 2.0 Client Credentials flow)
        let scopes = oauth_config.scopes.join(" ");
        let mut params = HashMap::new();
        params.insert("grant_type", "client_credentials");
        params.insert("client_id", &oauth_config.client_id);
        params.insert("client_secret", &client_secret);

        if !scopes.is_empty() {
            params.insert("scope", &scopes);
        }

        // Send token request
        let response = self
            .client
            .post(&oauth_config.token_url)
            .form(&params)
            .send()
            .await
            .map_err(|e| AppError::Mcp(format!("Failed to request OAuth token: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(AppError::Mcp(format!(
                "OAuth token request failed with status {}: {}",
                status, body
            )));
        }

        // Parse token response
        let token_response: TokenResponse = response
            .json()
            .await
            .map_err(|e| AppError::Mcp(format!("Failed to parse token response: {}", e)))?;

        // Calculate expiration time
        let expires_at = if let Some(expires_in) = token_response.expires_in {
            Utc::now() + Duration::seconds(expires_in)
        } else {
            // Default to 1 hour if not specified
            Utc::now() + Duration::hours(1)
        };

        // Cache token
        let token_info = CachedTokenInfo {
            access_token: token_response.access_token.clone(),
            expires_at,
            refresh_token: token_response.refresh_token.clone(),
        };

        self.token_cache
            .write()
            .insert(server_id.to_string(), token_info.clone());

        // Store in keyring
        self.keychain
            .store(MCP_OAUTH_SERVICE, &format!("{}_access_token", server_id), &token_info.access_token)
            .map_err(|e| AppError::Mcp(format!("Failed to store token in keychain: {}", e)))?;

        if let Some(ref refresh_token) = token_info.refresh_token {
            self.keychain
                .store(MCP_OAUTH_SERVICE, &format!("{}_refresh_token", server_id), refresh_token)
                .ok(); // Ignore errors for refresh token
        }

        tracing::info!("OAuth token acquired successfully for: {}", server_id);

        Ok(token_response.access_token)
    }

    /// Get cached OAuth token for an MCP server
    ///
    /// # Arguments
    /// * `server_id` - MCP server ID
    ///
    /// # Returns
    /// * Access token if available and not expired
    pub async fn get_cached_token(&self, server_id: &str) -> Option<String> {
        // Check memory cache first
        if let Some(token_info) = self.token_cache.read().get(server_id) {
            // Check if token is still valid (with 5-minute buffer)
            let buffer = Duration::minutes(5);
            if token_info.expires_at > Utc::now() + buffer {
                tracing::debug!("Using cached OAuth token for: {}", server_id);
                return Some(token_info.access_token.clone());
            }
        }

        // Try to load from keychain
        if let Ok(Some(token)) = self.keychain.get(MCP_OAUTH_SERVICE, &format!("{}_access_token", server_id)) {
            tracing::debug!("Loaded OAuth token from keychain for: {}", server_id);
            // Note: We don't have expiration info from keychain, so we'll try to use it
            // and let the server reject it if expired
            return Some(token);
        }

        None
    }

    /// Refresh an OAuth token
    ///
    /// # Arguments
    /// * `server_id` - MCP server ID
    /// * `oauth_config` - OAuth configuration
    ///
    /// # Returns
    /// * New access token
    pub async fn refresh_token(
        &self,
        server_id: &str,
        oauth_config: &McpOAuthConfig,
    ) -> AppResult<String> {
        tracing::info!("Refreshing OAuth token for: {}", server_id);

        // Get refresh token from cache or keychain
        let refresh_token = if let Some(token_info) = self.token_cache.read().get(server_id) {
            token_info.refresh_token.clone()
        } else {
            self.keychain
                .get(MCP_OAUTH_SERVICE, &format!("{}_refresh_token", server_id))
                .ok()
                .flatten()
        };

        let refresh_token = refresh_token.ok_or_else(|| {
            AppError::Mcp("No refresh token available, must re-authenticate".to_string())
        })?;

        // Retrieve client_secret from keychain
        let client_secret = self
            .keychain
            .get(MCP_OAUTH_SERVICE, &format!("{}_client_secret", server_id))
            .map_err(|e| AppError::Mcp(format!("Failed to retrieve client secret: {}", e)))?
            .ok_or_else(|| AppError::Mcp("Client secret not found in keychain".to_string()))?;

        // Prepare refresh request
        let mut params = HashMap::new();
        params.insert("grant_type", "refresh_token");
        params.insert("refresh_token", &refresh_token);
        params.insert("client_id", &oauth_config.client_id);
        params.insert("client_secret", &client_secret);

        // Send refresh request
        let response = self
            .client
            .post(&oauth_config.token_url)
            .form(&params)
            .send()
            .await
            .map_err(|e| AppError::Mcp(format!("Failed to refresh token: {}", e)))?;

        if !response.status().is_success() {
            // Clear cached token and force re-authentication
            self.token_cache.write().remove(server_id);
            self.keychain.delete(MCP_OAUTH_SERVICE, &format!("{}_access_token", server_id)).ok();
            self.keychain.delete(MCP_OAUTH_SERVICE, &format!("{}_refresh_token", server_id)).ok();

            return Err(AppError::Mcp(
                "Token refresh failed, re-authentication required".to_string(),
            ));
        }

        // Parse new token
        let token_response: TokenResponse = response
            .json()
            .await
            .map_err(|e| AppError::Mcp(format!("Failed to parse refresh response: {}", e)))?;

        // Update cache
        let expires_at = if let Some(expires_in) = token_response.expires_in {
            Utc::now() + Duration::seconds(expires_in)
        } else {
            Utc::now() + Duration::hours(1)
        };

        let token_info = CachedTokenInfo {
            access_token: token_response.access_token.clone(),
            expires_at,
            refresh_token: token_response.refresh_token.clone(),
        };

        self.token_cache
            .write()
            .insert(server_id.to_string(), token_info.clone());

        // Update keychain
        self.keychain
            .store(MCP_OAUTH_SERVICE, &format!("{}_access_token", server_id), &token_info.access_token)
            .map_err(|e| AppError::Mcp(format!("Failed to update token in keychain: {}", e)))?;

        if let Some(ref refresh_token) = token_info.refresh_token {
            self.keychain
                .store(MCP_OAUTH_SERVICE, &format!("{}_refresh_token", server_id), refresh_token)
                .ok();
        }

        tracing::info!("OAuth token refreshed successfully for: {}", server_id);

        Ok(token_response.access_token)
    }

    /// Clear cached token for a server
    ///
    /// # Arguments
    /// * `server_id` - MCP server ID
    pub fn clear_token(&self, server_id: &str) {
        self.token_cache.write().remove(server_id);
        self.keychain.delete(MCP_OAUTH_SERVICE, &format!("{}_access_token", server_id)).ok();
        self.keychain.delete(MCP_OAUTH_SERVICE, &format!("{}_refresh_token", server_id)).ok();
    }
}

impl Default for McpOAuthManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_token_cache() {
        let manager = McpOAuthManager::new();

        let token_info = CachedTokenInfo {
            access_token: "test_token".to_string(),
            expires_at: Utc::now() + Duration::hours(1),
            refresh_token: Some("refresh_token".to_string()),
        };

        manager
            .token_cache
            .write()
            .insert("test_server".to_string(), token_info.clone());

        // Should find the token
        assert!(manager.token_cache.read().contains_key("test_server"));
    }

    #[test]
    fn test_expired_token() {
        let manager = McpOAuthManager::new();

        let token_info = CachedTokenInfo {
            access_token: "expired_token".to_string(),
            expires_at: Utc::now() - Duration::hours(1), // Expired
            refresh_token: None,
        };

        manager
            .token_cache
            .write()
            .insert("test_server".to_string(), token_info);

        // Manually check expiration logic
        let cache_guard = manager.token_cache.read();
        if let Some(info) = cache_guard.get("test_server") {
            assert!(info.expires_at < Utc::now());
        }
    }
}
