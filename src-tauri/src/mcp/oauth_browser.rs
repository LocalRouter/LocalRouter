//! Browser-based OAuth 2.0 authorization code flow for MCP servers
//!
//! Manages user-interactive OAuth flows (GitHub, GitLab, etc.) where the user
//! completes authentication in their browser. Uses PKCE for security and integrates
//! with the existing McpOAuthManager for token storage and refresh.

use crate::api_keys::{CachedKeychain, keychain_trait::KeychainStorage};
use crate::config::McpAuthConfig;
use crate::mcp::oauth::{
    generate_pkce_challenge, generate_state, start_callback_server, McpOAuthManager,
    OAuthCallbackResult,
};
use crate::utils::errors::{AppError, AppResult};
use chrono::{DateTime, Duration, Utc};
use parking_lot::RwLock;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::oneshot;
use tracing::{error, info};

/// Default redirect URI for OAuth callbacks
const DEFAULT_REDIRECT_URI: &str = "http://localhost:8080/callback";

/// Default callback server port
const DEFAULT_CALLBACK_PORT: u16 = 8080;

/// Flow timeout in seconds (5 minutes)
const FLOW_TIMEOUT_SECS: i64 = 300;

/// Manager for browser-based OAuth flows
pub struct McpOAuthBrowserManager {
    /// HTTP client
    client: Client,

    /// Keychain storage
    keychain: CachedKeychain,

    /// Active OAuth flows (server_id -> flow state)
    active_flows: Arc<RwLock<HashMap<String, OAuthFlowState>>>,

    /// OAuth manager for token storage/refresh
    oauth_manager: Arc<McpOAuthManager>,
}

/// State of an active OAuth flow
#[derive(Debug)]
struct OAuthFlowState {
    /// PKCE code verifier
    code_verifier: String,

    /// CSRF state parameter
    state: String,

    /// Authorization URL to open in browser
    auth_url: String,

    /// Redirect URI used
    redirect_uri: String,

    /// OAuth configuration (needed for token exchange)
    client_id: String,
    token_url: String,

    /// Result channel for callback
    result_tx: Option<oneshot::Sender<OAuthCallbackResult>>,

    /// When this flow started
    started_at: DateTime<Utc>,

    /// Flow status
    status: FlowStatus,
}

/// Status of an OAuth flow
#[derive(Debug, Clone, PartialEq)]
enum FlowStatus {
    /// Waiting for user to complete authorization
    Pending,

    /// Successfully completed
    Success { expires_in: i64 },

    /// Failed with error
    Error { message: String },

    /// Timed out
    Timeout,
}

/// Result of starting a browser OAuth flow
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthBrowserFlowResult {
    /// URL to open in browser
    pub auth_url: String,

    /// Expected callback URI
    pub redirect_uri: String,

    /// CSRF state parameter (for debug/verification)
    pub state: String,
}

/// Status of an OAuth browser flow
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum OAuthBrowserFlowStatus {
    /// Still waiting for user to complete authorization
    Pending,

    /// Successfully completed
    Success {
        /// Seconds until token expires
        expires_in: i64,
    },

    /// Failed with error
    Error {
        /// Error message
        message: String,
    },

    /// Timed out after 5 minutes
    Timeout,
}

impl McpOAuthBrowserManager {
    /// Create a new OAuth browser manager
    pub fn new(keychain: CachedKeychain, oauth_manager: Arc<McpOAuthManager>) -> Self {
        Self {
            client: Client::new(),
            keychain,
            active_flows: Arc::new(RwLock::new(HashMap::new())),
            oauth_manager,
        }
    }

    /// Start a browser-based OAuth flow
    ///
    /// # Arguments
    /// * `server_id` - MCP server ID
    /// * `auth_config` - OAuth browser authentication configuration
    ///
    /// # Returns
    /// * Flow result with authorization URL to open in browser
    pub async fn start_browser_flow(
        &self,
        server_id: &str,
        auth_config: &McpAuthConfig,
    ) -> AppResult<OAuthBrowserFlowResult> {
        // Extract OAuth browser config
        let (client_id, auth_url, token_url, scopes, redirect_uri) = match auth_config {
            McpAuthConfig::OAuthBrowser {
                client_id,
                auth_url,
                token_url,
                scopes,
                redirect_uri,
                ..
            } => (
                client_id.clone(),
                auth_url.clone(),
                token_url.clone(),
                scopes.clone(),
                redirect_uri.clone(),
            ),
            _ => {
                return Err(AppError::Mcp(
                    "Invalid auth config type, expected OAuthBrowser".to_string(),
                ))
            }
        };

        // Check if flow already in progress
        if self.active_flows.read().contains_key(server_id) {
            return Err(AppError::Mcp(format!(
                "OAuth flow already in progress for server: {}",
                server_id
            )));
        }

        // Generate PKCE challenge
        let pkce = generate_pkce_challenge();

        // Generate CSRF state
        let state = generate_state();

        // Build authorization URL
        let auth_url_full = McpOAuthManager::build_authorization_url(
            &auth_url,
            &client_id,
            &redirect_uri,
            &scopes,
            &pkce,
            &state,
        );

        info!(
            "Starting browser OAuth flow for server {}: {}",
            server_id, auth_url_full
        );

        // Create flow state
        let flow_state = OAuthFlowState {
            code_verifier: pkce.code_verifier.clone(),
            state: state.clone(),
            auth_url: auth_url_full.clone(),
            redirect_uri: redirect_uri.clone(),
            client_id: client_id.clone(),
            token_url: token_url.clone(),
            result_tx: None, // Will be set when polling starts
            started_at: Utc::now(),
            status: FlowStatus::Pending,
        };

        // Store flow state
        self.active_flows
            .write()
            .insert(server_id.to_string(), flow_state);

        // Start callback server in background
        self.start_background_callback_server(server_id.to_string(), state.clone())
            .await?;

        Ok(OAuthBrowserFlowResult {
            auth_url: auth_url_full,
            redirect_uri,
            state,
        })
    }

    /// Start callback server in background for this flow
    async fn start_background_callback_server(
        &self,
        server_id: String,
        expected_state: String,
    ) -> AppResult<()> {
        let active_flows = Arc::clone(&self.active_flows);
        let oauth_manager = Arc::clone(&self.oauth_manager);
        let keychain = self.keychain.clone();

        tokio::spawn(async move {
            // Start callback server
            match start_callback_server(DEFAULT_CALLBACK_PORT, expected_state.clone()).await {
                Ok(callback_result) => {
                    info!("OAuth callback received for server: {}", server_id);

                    // Get flow state
                    let (code_verifier, redirect_uri, client_id, token_url) = {
                        let flows = active_flows.read();
                        if let Some(flow) = flows.get(&server_id) {
                            (
                                flow.code_verifier.clone(),
                                flow.redirect_uri.clone(),
                                flow.client_id.clone(),
                                flow.token_url.clone(),
                            )
                        } else {
                            error!("Flow state not found for server: {}", server_id);
                            return;
                        }
                    };

                    // Exchange code for token
                    match Self::exchange_code_for_token_static(
                        &oauth_manager,
                        &keychain,
                        &server_id,
                        &callback_result.code,
                        &redirect_uri,
                        &code_verifier,
                        &client_id,
                        &token_url,
                    )
                    .await
                    {
                        Ok(expires_in) => {
                            // Update flow status to success
                            if let Some(flow) = active_flows.write().get_mut(&server_id) {
                                flow.status = FlowStatus::Success { expires_in };
                                info!(
                                    "OAuth flow completed successfully for server: {}",
                                    server_id
                                );
                            }
                        }
                        Err(e) => {
                            // Update flow status to error
                            if let Some(flow) = active_flows.write().get_mut(&server_id) {
                                flow.status = FlowStatus::Error {
                                    message: e.to_string(),
                                };
                                error!("Token exchange failed for server {}: {}", server_id, e);
                            }
                        }
                    }
                }
                Err(e) => {
                    error!("OAuth callback server failed for server {}: {}", server_id, e);

                    // Update flow status to error
                    if let Some(flow) = active_flows.write().get_mut(&server_id) {
                        flow.status = FlowStatus::Error {
                            message: e.to_string(),
                        };
                    }
                }
            }
        });

        Ok(())
    }

    /// Exchange authorization code for token (static method for background task)
    async fn exchange_code_for_token_static(
        oauth_manager: &McpOAuthManager,
        keychain: &CachedKeychain,
        server_id: &str,
        authorization_code: &str,
        redirect_uri: &str,
        code_verifier: &str,
        client_id: &str,
        token_url: &str,
    ) -> AppResult<i64> {
        info!("Exchanging authorization code for token: {}", server_id);

        // Retrieve client_secret from keychain
        let client_secret = keychain
            .get("LocalRouter-McpServers", &format!("{}_client_secret", server_id))
            .map_err(|e| AppError::Mcp(format!("Failed to retrieve client secret: {}", e)))?
            .ok_or_else(|| AppError::Mcp("Client secret not found in keychain".to_string()))?;

        // Prepare token exchange request
        let mut params = HashMap::new();
        params.insert("grant_type", "authorization_code".to_string());
        params.insert("code", authorization_code.to_string());
        params.insert("redirect_uri", redirect_uri.to_string());
        params.insert("client_id", client_id.to_string());
        params.insert("client_secret", client_secret);
        params.insert("code_verifier", code_verifier.to_string());

        // Send token request
        let client = Client::new();
        let response = client
            .post(token_url)
            .form(&params)
            .send()
            .await
            .map_err(|e| AppError::Mcp(format!("Failed to exchange code for token: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(AppError::Mcp(format!(
                "Token exchange failed with status {}: {}",
                status, body
            )));
        }

        // Parse token response
        let token_response: serde_json::Value = response
            .json()
            .await
            .map_err(|e| AppError::Mcp(format!("Failed to parse token response: {}", e)))?;

        let access_token = token_response["access_token"]
            .as_str()
            .ok_or_else(|| AppError::Mcp("Missing access_token in response".to_string()))?
            .to_string();

        let expires_in = token_response["expires_in"]
            .as_i64()
            .unwrap_or(3600); // Default to 1 hour

        let refresh_token = token_response["refresh_token"].as_str().map(|s| s.to_string());

        // Store tokens in keychain using OAuth manager's service name
        keychain
            .store(
                "LocalRouter-McpServerTokens",
                &format!("{}_access_token", server_id),
                &access_token,
            )
            .map_err(|e| AppError::Mcp(format!("Failed to store token in keychain: {}", e)))?;

        if let Some(ref refresh_token) = refresh_token {
            keychain
                .store(
                    "LocalRouter-McpServerTokens",
                    &format!("{}_refresh_token", server_id),
                    refresh_token,
                )
                .ok(); // Ignore errors for refresh token
        }

        info!("Token exchange successful for: {}", server_id);

        Ok(expires_in)
    }

    /// Poll the status of an OAuth browser flow
    ///
    /// # Arguments
    /// * `server_id` - MCP server ID
    ///
    /// # Returns
    /// * Current flow status
    pub fn poll_flow_status(&self, server_id: &str) -> AppResult<OAuthBrowserFlowStatus> {
        let flows = self.active_flows.read();

        let flow = flows
            .get(server_id)
            .ok_or_else(|| AppError::Mcp(format!("No active flow for server: {}", server_id)))?;

        // Check for timeout
        let elapsed = Utc::now() - flow.started_at;
        if elapsed > Duration::seconds(FLOW_TIMEOUT_SECS) && flow.status == FlowStatus::Pending {
            // Don't modify here, let cancel handle cleanup
            return Ok(OAuthBrowserFlowStatus::Timeout);
        }

        // Return current status
        match &flow.status {
            FlowStatus::Pending => Ok(OAuthBrowserFlowStatus::Pending),
            FlowStatus::Success { expires_in } => Ok(OAuthBrowserFlowStatus::Success {
                expires_in: *expires_in,
            }),
            FlowStatus::Error { message } => Ok(OAuthBrowserFlowStatus::Error {
                message: message.clone(),
            }),
            FlowStatus::Timeout => Ok(OAuthBrowserFlowStatus::Timeout),
        }
    }

    /// Cancel an active OAuth browser flow
    ///
    /// # Arguments
    /// * `server_id` - MCP server ID
    pub fn cancel_flow(&self, server_id: &str) -> AppResult<()> {
        let mut flows = self.active_flows.write();

        if flows.remove(server_id).is_some() {
            info!("Cancelled OAuth flow for server: {}", server_id);
            Ok(())
        } else {
            Err(AppError::Mcp(format!(
                "No active flow to cancel for server: {}",
                server_id
            )))
        }
    }

    /// Get valid token for a server (checks cache, auto-refreshes if needed)
    ///
    /// # Arguments
    /// * `server_id` - MCP server ID
    ///
    /// # Returns
    /// * Valid access token
    pub async fn get_valid_token(&self, server_id: &str) -> AppResult<String> {
        // Try to get cached token from OAuth manager
        if let Some(token) = self.oauth_manager.get_cached_token(server_id).await {
            return Ok(token);
        }

        Err(AppError::Mcp(format!(
            "No valid token found for server: {}. Please authenticate.",
            server_id
        )))
    }

    /// Revoke OAuth tokens for a server
    ///
    /// # Arguments
    /// * `server_id` - MCP server ID
    pub fn revoke_tokens(&self, server_id: &str) -> AppResult<()> {
        // Clear from OAuth manager cache
        self.oauth_manager.clear_token(server_id);

        // Also delete client secret if stored
        self.keychain
            .delete("LocalRouter-McpServers", &format!("{}_client_secret", server_id))
            .ok();

        info!("Revoked OAuth tokens for server: {}", server_id);

        Ok(())
    }

    /// Check if a server has valid authentication
    ///
    /// # Arguments
    /// * `server_id` - MCP server ID
    ///
    /// # Returns
    /// * `true` if server has valid token
    pub async fn has_valid_auth(&self, server_id: &str) -> bool {
        self.oauth_manager.get_cached_token(server_id).await.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_flow_state_creation() {
        let pkce = generate_pkce_challenge();
        let state = generate_state();

        let flow_state = OAuthFlowState {
            code_verifier: pkce.code_verifier.clone(),
            state: state.clone(),
            auth_url: "https://example.com/auth".to_string(),
            redirect_uri: DEFAULT_REDIRECT_URI.to_string(),
            client_id: "test_client".to_string(),
            token_url: "https://example.com/token".to_string(),
            result_tx: None,
            started_at: Utc::now(),
            status: FlowStatus::Pending,
        };

        assert_eq!(flow_state.status, FlowStatus::Pending);
        assert!(!flow_state.code_verifier.is_empty());
        assert!(!flow_state.state.is_empty());
    }

    #[test]
    fn test_flow_status_serialization() {
        let status = OAuthBrowserFlowStatus::Success { expires_in: 3600 };
        let json = serde_json::to_string(&status).unwrap();
        assert!(json.contains("Success"));
        assert!(json.contains("3600"));

        let error_status = OAuthBrowserFlowStatus::Error {
            message: "Test error".to_string(),
        };
        let json = serde_json::to_string(&error_status).unwrap();
        assert!(json.contains("Error"));
        assert!(json.contains("Test error"));
    }
}
