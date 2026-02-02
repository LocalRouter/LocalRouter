//! Browser-based OAuth 2.0 authorization code flow for MCP servers
//!
//! Manages user-interactive OAuth flows (GitHub, GitLab, etc.) where the user
//! completes authentication in their browser. Uses PKCE for security and integrates
//! with the existing McpOAuthManager for token storage and refresh.
//!
//! This module now uses the unified oauth_browser module for the core OAuth flow,
//! while maintaining backward compatibility with the original MCP OAuth API.

use crate::oauth::McpOAuthManager;
use chrono::{DateTime, Duration, Utc};
use lr_api_keys::CachedKeychain;
use lr_config::McpAuthConfig;
use lr_oauth::browser::{FlowId, OAuthFlowConfig, OAuthFlowManager, OAuthFlowResult};
use lr_types::{AppError, AppResult};
use parking_lot::RwLock;
use reqwest::Url;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{error, info};

/// Flow timeout in seconds (5 minutes)
const FLOW_TIMEOUT_SECS: i64 = 300;

/// Manager for browser-based OAuth flows
pub struct McpOAuthBrowserManager {
    /// Unified OAuth flow manager
    flow_manager: Arc<OAuthFlowManager>,

    /// OAuth manager for token storage/refresh
    oauth_manager: Arc<McpOAuthManager>,

    /// Map server_id -> flow_id for tracking active flows
    #[allow(clippy::type_complexity)]
    server_flows: Arc<RwLock<HashMap<String, (FlowId, DateTime<Utc>)>>>,
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
            flow_manager: Arc::new(OAuthFlowManager::new(keychain)),
            oauth_manager,
            server_flows: Arc::new(RwLock::new(HashMap::new())),
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
        if self.server_flows.read().contains_key(server_id) {
            return Err(AppError::Mcp(format!(
                "OAuth flow already in progress for server: {}",
                server_id
            )));
        }

        // Parse port from redirect URI
        let callback_port = Self::parse_port(&redirect_uri)?;

        info!(
            "Starting browser OAuth flow for server {}: {}",
            server_id, redirect_uri
        );

        // Create unified OAuth flow config
        let config = OAuthFlowConfig {
            client_id,
            client_secret: None, // Will be loaded from keychain if needed
            auth_url,
            token_url,
            scopes,
            redirect_uri: redirect_uri.clone(),
            callback_port,
            keychain_service: "LocalRouter-McpServerTokens".to_string(),
            account_id: server_id.to_string(),
            extra_auth_params: HashMap::new(),
            extra_token_params: HashMap::new(),
        };

        // Start flow via unified manager
        let start_result = self.flow_manager.start_flow(config).await?;

        // Track flow_id for this server
        self.server_flows
            .write()
            .insert(server_id.to_string(), (start_result.flow_id, Utc::now()));

        // Return in MCP format (unchanged public API)
        Ok(OAuthBrowserFlowResult {
            auth_url: start_result.auth_url,
            redirect_uri,
            state: start_result.state,
        })
    }

    /// Parse port from redirect URI
    fn parse_port(redirect_uri: &str) -> AppResult<u16> {
        let url = Url::parse(redirect_uri)
            .map_err(|e| AppError::Mcp(format!("Invalid redirect URI: {}", e)))?;

        url.port()
            .or_else(|| {
                // Default ports
                match url.scheme() {
                    "http" => Some(80),
                    "https" => Some(443),
                    _ => None,
                }
            })
            .ok_or_else(|| AppError::Mcp("Could not determine port from redirect URI".to_string()))
    }

    /// Poll the status of an OAuth browser flow
    ///
    /// # Arguments
    /// * `server_id` - MCP server ID
    ///
    /// # Returns
    /// * Current flow status
    pub fn poll_flow_status(&self, server_id: &str) -> AppResult<OAuthBrowserFlowStatus> {
        let server_flows = self.server_flows.read();
        let (flow_id, started_at) = server_flows
            .get(server_id)
            .ok_or_else(|| AppError::Mcp(format!("No active flow for server: {}", server_id)))?;

        // Check for timeout
        let elapsed = Utc::now() - *started_at;
        if elapsed > Duration::seconds(FLOW_TIMEOUT_SECS) {
            return Ok(OAuthBrowserFlowStatus::Timeout);
        }

        // Get flow result from unified manager
        let result = self.flow_manager.poll_status(*flow_id)?;

        // Convert to MCP format
        match result {
            OAuthFlowResult::Pending { .. } => Ok(OAuthBrowserFlowStatus::Pending),
            OAuthFlowResult::ExchangingToken => Ok(OAuthBrowserFlowStatus::Pending),
            OAuthFlowResult::Success { tokens } => {
                // Update OAuth manager's token cache
                if let Err(e) = self.oauth_manager.update_token_cache(
                    server_id,
                    &tokens.access_token,
                    tokens.expires_at,
                ) {
                    error!("Failed to update token cache: {}", e);
                }

                // Clean up flow tracking
                drop(server_flows);
                self.server_flows.write().remove(server_id);

                Ok(OAuthBrowserFlowStatus::Success {
                    expires_in: tokens.expires_in.unwrap_or(3600),
                })
            }
            OAuthFlowResult::Error { message } => {
                // Clean up flow tracking
                drop(server_flows);
                self.server_flows.write().remove(server_id);

                Ok(OAuthBrowserFlowStatus::Error { message })
            }
            OAuthFlowResult::Timeout => {
                // Clean up flow tracking
                drop(server_flows);
                self.server_flows.write().remove(server_id);

                Ok(OAuthBrowserFlowStatus::Timeout)
            }
            OAuthFlowResult::Cancelled => {
                // Clean up flow tracking
                drop(server_flows);
                self.server_flows.write().remove(server_id);

                Ok(OAuthBrowserFlowStatus::Error {
                    message: "Flow cancelled".to_string(),
                })
            }
        }
    }

    /// Cancel an active OAuth browser flow
    ///
    /// # Arguments
    /// * `server_id` - MCP server ID
    pub fn cancel_flow(&self, server_id: &str) -> AppResult<()> {
        let mut server_flows = self.server_flows.write();

        if let Some((flow_id, _)) = server_flows.remove(server_id) {
            self.flow_manager.cancel_flow(flow_id)?;
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
    #[allow(dead_code)]
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
        self.oauth_manager
            .get_cached_token(server_id)
            .await
            .is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_port() {
        assert_eq!(
            McpOAuthBrowserManager::parse_port("http://localhost:8080/callback").unwrap(),
            8080
        );
        assert_eq!(
            McpOAuthBrowserManager::parse_port("http://127.0.0.1:1455/callback").unwrap(),
            1455
        );
        assert_eq!(
            McpOAuthBrowserManager::parse_port("http://localhost/callback").unwrap(),
            80
        );
        assert_eq!(
            McpOAuthBrowserManager::parse_port("https://localhost/callback").unwrap(),
            443
        );
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
