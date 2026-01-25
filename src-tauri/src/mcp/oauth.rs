//! OAuth support for MCP servers
//!
//! Handles OAuth discovery, token acquisition, and token management for MCP servers
//! that require OAuth authentication.

#![allow(dead_code)]

use crate::api_keys::{CachedKeychain, KeychainStorage};
use crate::config::McpOAuthConfig;
use crate::utils::errors::{AppError, AppResult};
use axum::{
    extract::Query,
    http::StatusCode,
    response::{Html, IntoResponse},
    Router,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use chrono::{DateTime, Duration, Utc};
use parking_lot::{Mutex, RwLock};
use rand::{thread_rng, Rng};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::oneshot;

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

/// Protected Resource Metadata (RFC 9728)
///
/// Response from .well-known/oauth-protected-resource endpoint
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ProtectedResourceMetadata {
    /// Human-readable name of the resource
    #[serde(default)]
    pub resource_name: Option<String>,

    /// Protected resource identifier
    #[serde(default)]
    pub resource: Option<String>,

    /// Authorization servers that can issue tokens for this resource
    #[serde(default)]
    pub authorization_servers: Vec<String>,

    /// Methods for sending bearer tokens (e.g., "header")
    #[serde(default)]
    pub bearer_methods_supported: Vec<String>,

    /// Supported scopes for this resource
    #[serde(default)]
    pub scopes_supported: Vec<String>,
}

/// OAuth Authorization Server Metadata (RFC 8414)
///
/// Response from .well-known/oauth-authorization-server endpoint
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AuthorizationServerMetadata {
    /// Authorization endpoint URL
    pub authorization_endpoint: String,

    /// Token endpoint URL
    pub token_endpoint: String,

    /// Supported scopes
    #[serde(default)]
    pub scopes_supported: Vec<String>,

    /// Supported grant types
    #[serde(default)]
    pub grant_types_supported: Vec<String>,
}

/// Combined OAuth discovery response
///
/// This is the unified response returned by discover_oauth, combining
/// information from protected resource metadata and authorization server metadata
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

/// PKCE (Proof Key for Code Exchange) data
#[derive(Debug, Clone)]
pub struct PkceChallenge {
    /// Code verifier (random string, 43-128 characters)
    pub code_verifier: String,

    /// Code challenge (BASE64URL(SHA256(code_verifier)))
    pub code_challenge: String,

    /// Challenge method (always "S256" for SHA-256)
    pub code_challenge_method: String,
}

/// OAuth callback query parameters
#[derive(Debug, Deserialize)]
struct OAuthCallbackQuery {
    /// Authorization code
    code: Option<String>,

    /// State parameter (for CSRF protection)
    state: Option<String>,

    /// Error code (if authorization failed)
    error: Option<String>,

    /// Error description
    error_description: Option<String>,
}

/// OAuth callback result
#[derive(Debug, Clone)]
pub struct OAuthCallbackResult {
    /// Authorization code
    pub code: String,

    /// State parameter
    pub state: String,
}

/// Generate PKCE challenge for OAuth authorization code flow
///
/// Creates a cryptographically secure code verifier and derives the code challenge
/// using SHA-256 hashing.
///
/// # Returns
/// * PKCE challenge containing verifier and challenge
pub fn generate_pkce_challenge() -> PkceChallenge {
    // Generate random code_verifier (43-128 characters, URL-safe)
    let mut rng = thread_rng();
    let code_verifier: String = (0..64)
        .map(|_| {
            let idx = rng.gen_range(0..62);
            match idx {
                0..=25 => (b'A' + idx) as char,
                26..=51 => (b'a' + (idx - 26)) as char,
                _ => (b'0' + (idx - 52)) as char,
            }
        })
        .collect();

    // Generate code_challenge = BASE64URL(SHA256(code_verifier))
    let mut hasher = Sha256::new();
    hasher.update(code_verifier.as_bytes());
    let hash = hasher.finalize();
    let code_challenge = URL_SAFE_NO_PAD.encode(hash);

    PkceChallenge {
        code_verifier,
        code_challenge,
        code_challenge_method: "S256".to_string(),
    }
}

/// Generate a random state string for CSRF protection
pub fn generate_state() -> String {
    let mut rng = thread_rng();
    (0..32)
        .map(|_| {
            let idx = rng.gen_range(0..62);
            match idx {
                0..=25 => (b'A' + idx) as char,
                26..=51 => (b'a' + (idx - 26)) as char,
                _ => (b'0' + (idx - 52)) as char,
            }
        })
        .collect()
}

/// Build a well-known URL for OAuth protected resource discovery per RFC 8615
///
/// When the protected resource identifier has a path component, the
/// `/.well-known/oauth-protected-resource` segment is inserted between
/// the host and the path component.
///
/// # Arguments
/// * `resource_url` - The protected resource identifier URL
///
/// # Returns
/// * The well-known discovery URL
///
/// # Examples
/// - `https://api.example.com` → `https://api.example.com/.well-known/oauth-protected-resource`
/// - `https://api.example.com/mcp` → `https://api.example.com/.well-known/oauth-protected-resource/mcp`
/// - `https://api.example.com/api/v4/mcp` → `https://api.example.com/.well-known/oauth-protected-resource/api/v4/mcp`
pub fn build_well_known_url(resource_url: &str) -> String {
    let url = resource_url.trim_end_matches('/');

    // Find the start of the path (after the scheme and host)
    // URL format: scheme://host[:port][/path]
    if let Some(scheme_end) = url.find("://") {
        let after_scheme = &url[scheme_end + 3..];

        // Find the first slash after the host (start of path)
        if let Some(path_start) = after_scheme.find('/') {
            let host_end = scheme_end + 3 + path_start;
            let origin = &url[..host_end]; // scheme://host[:port]
            let path = &url[host_end..]; // /path

            // Insert well-known between origin and path
            format!("{}/.well-known/oauth-protected-resource{}", origin, path)
        } else {
            // No path, just append well-known
            format!("{}/.well-known/oauth-protected-resource", url)
        }
    } else {
        // Malformed URL, just append (shouldn't happen)
        format!("{}/.well-known/oauth-protected-resource", url)
    }
}

/// Build a well-known URL for OAuth Authorization Server Metadata (RFC 8414)
///
/// Similar to protected resource metadata, but for authorization servers.
/// The `.well-known/oauth-authorization-server` segment is inserted between
/// the host and any path component.
///
/// # Arguments
/// * `auth_server_url` - The authorization server URL
///
/// # Returns
/// * The well-known metadata URL
///
/// # Examples
/// - `https://github.com/login/oauth` → `https://github.com/.well-known/oauth-authorization-server/login/oauth`
/// - `https://auth.example.com` → `https://auth.example.com/.well-known/oauth-authorization-server`
pub fn build_authorization_server_metadata_url(auth_server_url: &str) -> String {
    let url = auth_server_url.trim_end_matches('/');

    if let Some(scheme_end) = url.find("://") {
        let after_scheme = &url[scheme_end + 3..];

        if let Some(path_start) = after_scheme.find('/') {
            let host_end = scheme_end + 3 + path_start;
            let origin = &url[..host_end];
            let path = &url[host_end..];

            format!("{}/.well-known/oauth-authorization-server{}", origin, path)
        } else {
            format!("{}/.well-known/oauth-authorization-server", url)
        }
    } else {
        format!("{}/.well-known/oauth-authorization-server", url)
    }
}

/// Start a temporary HTTP server to receive OAuth callback
///
/// This server listens on http://localhost:{port}/callback and waits for the OAuth
/// provider to redirect the user back with an authorization code.
///
/// # Arguments
/// * `port` - Port to listen on (e.g., 8080)
/// * `expected_state` - Expected state parameter for CSRF protection
///
/// # Returns
/// * OAuth callback result containing the authorization code
pub async fn start_callback_server(
    port: u16,
    expected_state: String,
) -> AppResult<OAuthCallbackResult> {
    let (tx, rx) = oneshot::channel();
    let tx = Arc::new(Mutex::new(Some(tx)));
    let expected_state = Arc::new(expected_state);

    // Create callback handler
    let callback_handler = {
        let tx = Arc::clone(&tx);
        let expected_state = Arc::clone(&expected_state);

        move |Query(params): Query<OAuthCallbackQuery>| {
            let tx = Arc::clone(&tx);
            let expected_state = Arc::clone(&expected_state);

            async move {
                // Check for errors
                if let Some(error) = params.error {
                    let description = params
                        .error_description
                        .unwrap_or_else(|| "Unknown error".to_string());
                    tracing::error!("OAuth authorization failed: {} - {}", error, description);

                    return (
                        StatusCode::BAD_REQUEST,
                        Html(format!(
                            r#"
                            <html>
                                <head><title>Authorization Failed</title></head>
                                <body>
                                    <h1>Authorization Failed</h1>
                                    <p>Error: {}</p>
                                    <p>Description: {}</p>
                                    <p>You can close this window.</p>
                                </body>
                            </html>
                            "#,
                            error, description
                        )),
                    )
                        .into_response();
                }

                // Extract authorization code
                let code = match params.code {
                    Some(c) => c,
                    None => {
                        return (
                            StatusCode::BAD_REQUEST,
                            Html("<html><body><h1>Error: No authorization code received</h1></body></html>"),
                        ).into_response();
                    }
                };

                // Validate state
                let state = match params.state {
                    Some(s) => s,
                    None => {
                        return (
                            StatusCode::BAD_REQUEST,
                            Html("<html><body><h1>Error: No state parameter received</h1></body></html>"),
                        ).into_response();
                    }
                };

                if state != *expected_state {
                    tracing::error!(
                        "State mismatch: expected {}, got {}",
                        *expected_state,
                        state
                    );
                    return (
                        StatusCode::BAD_REQUEST,
                        Html("<html><body><h1>Error: Invalid state parameter (CSRF protection)</h1></body></html>"),
                    ).into_response();
                }

                // Send result through channel
                if let Some(sender) = tx.lock().take() {
                    let result = OAuthCallbackResult {
                        code: code.clone(),
                        state: state.clone(),
                    };

                    if sender.send(result).is_err() {
                        tracing::error!("Failed to send OAuth callback result");
                    }
                }

                // Return success page
                (
                    StatusCode::OK,
                    Html(
                        r#"
                        <html>
                            <head><title>Authorization Successful</title></head>
                            <body>
                                <h1>Authorization Successful!</h1>
                                <p>You have successfully authorized the application.</p>
                                <p>You can close this window and return to LocalRouter AI.</p>
                                <script>
                                    setTimeout(function() { window.close(); }, 3000);
                                </script>
                            </body>
                        </html>
                        "#,
                    ),
                )
                    .into_response()
            }
        }
    };

    // Build router
    let app = Router::new().route("/callback", axum::routing::get(callback_handler));

    // Start server
    let addr = format!("127.0.0.1:{}", port);
    tracing::info!("Starting OAuth callback server on http://{}/callback", addr);

    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .map_err(|e| AppError::Mcp(format!("Failed to bind callback server: {}", e)))?;

    // Spawn server in background
    tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            tracing::error!("OAuth callback server error: {}", e);
        }
    });

    // Wait for callback with timeout
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(300), // 5 minute timeout
        rx,
    )
    .await
    .map_err(|_| AppError::Mcp("OAuth authorization timeout (5 minutes)".to_string()))?
    .map_err(|_| AppError::Mcp("OAuth callback channel closed unexpectedly".to_string()))?;

    tracing::info!("OAuth callback received successfully");

    Ok(result)
}

impl McpOAuthManager {
    /// Create a new OAuth manager
    pub fn new() -> Self {
        let keychain = CachedKeychain::auto().expect("Failed to initialize MCP OAuth keychain");

        Self {
            client: Client::new(),
            keychain,
            token_cache: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Discover OAuth configuration for an MCP server
    ///
    /// Implements the two-step OAuth discovery process per RFC 9728 and RFC 8414:
    /// 1. Fetch Protected Resource Metadata from `.well-known/oauth-protected-resource`
    /// 2. Fetch Authorization Server Metadata from each `authorization_servers` entry
    ///
    /// For providers that don't publish authorization server metadata (like GitHub),
    /// falls back to well-known OAuth endpoints.
    ///
    /// # Arguments
    /// * `base_url` - Base URL of the MCP server (the protected resource identifier)
    ///
    /// # Returns
    /// * OAuth discovery response if the server supports OAuth
    pub async fn discover_oauth(
        &self,
        base_url: &str,
    ) -> AppResult<Option<OAuthDiscoveryResponse>> {
        // Step 1: Fetch Protected Resource Metadata (RFC 9728)
        let discovery_url = build_well_known_url(base_url);
        tracing::info!(
            "Discovering protected resource metadata at: {}",
            discovery_url
        );

        let response = match self.client.get(&discovery_url).send().await {
            Ok(resp) => resp,
            Err(e) => {
                tracing::debug!(
                    "OAuth discovery failed (server may not require OAuth): {}",
                    e
                );
                return Ok(None);
            }
        };

        if !response.status().is_success() {
            tracing::debug!(
                "OAuth discovery returned status {} (server may not require OAuth)",
                response.status()
            );
            return Ok(None);
        }

        // Parse protected resource metadata
        let resource_metadata: ProtectedResourceMetadata = response.json().await.map_err(|e| {
            AppError::Mcp(format!(
                "Failed to parse protected resource metadata: {}",
                e
            ))
        })?;

        tracing::info!(
            "Protected resource metadata: authorization_servers={:?}, scopes={:?}",
            resource_metadata.authorization_servers,
            resource_metadata.scopes_supported
        );

        if resource_metadata.authorization_servers.is_empty() {
            tracing::debug!("No authorization servers found in protected resource metadata");
            return Ok(None);
        }

        // Step 2: Try to fetch Authorization Server Metadata (RFC 8414) from each server
        for auth_server in &resource_metadata.authorization_servers {
            if let Some(discovery) = self
                .discover_authorization_server(auth_server, &resource_metadata.scopes_supported)
                .await?
            {
                return Ok(Some(discovery));
            }
        }

        tracing::debug!("Could not discover authorization server metadata from any server");
        Ok(None)
    }

    /// Discover Authorization Server Metadata (RFC 8414)
    ///
    /// Tries to fetch metadata from `.well-known/oauth-authorization-server`.
    /// Falls back to well-known endpoints for common providers.
    async fn discover_authorization_server(
        &self,
        auth_server_url: &str,
        resource_scopes: &[String],
    ) -> AppResult<Option<OAuthDiscoveryResponse>> {
        // Try standard .well-known/oauth-authorization-server endpoint
        let metadata_url = build_authorization_server_metadata_url(auth_server_url);
        tracing::info!("Trying authorization server metadata at: {}", metadata_url);

        let response = self.client.get(&metadata_url).send().await;

        if let Ok(resp) = response {
            if resp.status().is_success() {
                if let Ok(metadata) = resp.json::<AuthorizationServerMetadata>().await {
                    tracing::info!(
                        "Authorization server metadata found: auth={}, token={}",
                        metadata.authorization_endpoint,
                        metadata.token_endpoint
                    );

                    // Use scopes from auth server if available, otherwise from resource
                    let scopes = if metadata.scopes_supported.is_empty() {
                        resource_scopes.to_vec()
                    } else {
                        metadata.scopes_supported
                    };

                    return Ok(Some(OAuthDiscoveryResponse {
                        auth_url: metadata.authorization_endpoint,
                        token_endpoint: metadata.token_endpoint,
                        scopes_supported: scopes,
                        grant_types_supported: metadata.grant_types_supported,
                    }));
                }
            }
        }

        // Fall back to well-known endpoints for common providers
        if let Some(discovery) = self.fallback_endpoints(auth_server_url, resource_scopes) {
            tracing::info!("Using fallback OAuth endpoints for: {}", auth_server_url);
            return Ok(Some(discovery));
        }

        Ok(None)
    }

    /// Provide fallback OAuth endpoints for well-known providers
    ///
    /// Some providers (like GitHub) don't publish RFC 8414 metadata,
    /// so we provide known endpoints as fallbacks.
    fn fallback_endpoints(
        &self,
        auth_server_url: &str,
        resource_scopes: &[String],
    ) -> Option<OAuthDiscoveryResponse> {
        let url_lower = auth_server_url.to_lowercase();

        // GitHub OAuth
        if url_lower.contains("github.com") {
            return Some(OAuthDiscoveryResponse {
                auth_url: "https://github.com/login/oauth/authorize".to_string(),
                token_endpoint: "https://github.com/login/oauth/access_token".to_string(),
                scopes_supported: resource_scopes.to_vec(),
                grant_types_supported: vec!["authorization_code".to_string()],
            });
        }

        // Google OAuth
        if url_lower.contains("google.com") || url_lower.contains("googleapis.com") {
            return Some(OAuthDiscoveryResponse {
                auth_url: "https://accounts.google.com/o/oauth2/v2/auth".to_string(),
                token_endpoint: "https://oauth2.googleapis.com/token".to_string(),
                scopes_supported: resource_scopes.to_vec(),
                grant_types_supported: vec![
                    "authorization_code".to_string(),
                    "refresh_token".to_string(),
                ],
            });
        }

        // Microsoft / Azure AD OAuth
        if url_lower.contains("microsoft.com")
            || url_lower.contains("microsoftonline.com")
            || url_lower.contains("live.com")
        {
            return Some(OAuthDiscoveryResponse {
                auth_url: "https://login.microsoftonline.com/common/oauth2/v2.0/authorize"
                    .to_string(),
                token_endpoint: "https://login.microsoftonline.com/common/oauth2/v2.0/token"
                    .to_string(),
                scopes_supported: resource_scopes.to_vec(),
                grant_types_supported: vec![
                    "authorization_code".to_string(),
                    "refresh_token".to_string(),
                ],
            });
        }

        None
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
            .store(
                MCP_OAUTH_SERVICE,
                &format!("{}_access_token", server_id),
                &token_info.access_token,
            )
            .map_err(|e| AppError::Mcp(format!("Failed to store token in keychain: {}", e)))?;

        if let Some(ref refresh_token) = token_info.refresh_token {
            self.keychain
                .store(
                    MCP_OAUTH_SERVICE,
                    &format!("{}_refresh_token", server_id),
                    refresh_token,
                )
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
        if let Ok(Some(token)) = self
            .keychain
            .get(MCP_OAUTH_SERVICE, &format!("{}_access_token", server_id))
        {
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
            self.keychain
                .delete(MCP_OAUTH_SERVICE, &format!("{}_access_token", server_id))
                .ok();
            self.keychain
                .delete(MCP_OAUTH_SERVICE, &format!("{}_refresh_token", server_id))
                .ok();

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
            .store(
                MCP_OAUTH_SERVICE,
                &format!("{}_access_token", server_id),
                &token_info.access_token,
            )
            .map_err(|e| AppError::Mcp(format!("Failed to update token in keychain: {}", e)))?;

        if let Some(ref refresh_token) = token_info.refresh_token {
            self.keychain
                .store(
                    MCP_OAUTH_SERVICE,
                    &format!("{}_refresh_token", server_id),
                    refresh_token,
                )
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
        self.keychain
            .delete(MCP_OAUTH_SERVICE, &format!("{}_access_token", server_id))
            .ok();
        self.keychain
            .delete(MCP_OAUTH_SERVICE, &format!("{}_refresh_token", server_id))
            .ok();
    }

    /// Update token cache with new access token
    ///
    /// # Arguments
    /// * `server_id` - MCP server ID
    /// * `access_token` - Access token
    /// * `expires_at` - Optional expiration time
    ///
    /// # Returns
    /// * Result indicating success or failure
    pub fn update_token_cache(
        &self,
        server_id: &str,
        access_token: &str,
        expires_at: Option<DateTime<Utc>>,
    ) -> AppResult<()> {
        let token_info = CachedTokenInfo {
            access_token: access_token.to_string(),
            expires_at: expires_at.unwrap_or_else(|| Utc::now() + Duration::hours(1)),
            refresh_token: None,
        };

        self.token_cache
            .write()
            .insert(server_id.to_string(), token_info);

        Ok(())
    }

    /// Build authorization URL for OAuth authorization code flow with PKCE
    ///
    /// # Arguments
    /// * `auth_url` - Authorization endpoint URL
    /// * `client_id` - OAuth client ID
    /// * `redirect_uri` - Redirect URI for callback
    /// * `scopes` - Requested scopes
    /// * `pkce` - PKCE challenge
    /// * `state` - Random state parameter for CSRF protection
    ///
    /// # Returns
    /// * Authorization URL
    pub fn build_authorization_url(
        auth_url: &str,
        client_id: &str,
        redirect_uri: &str,
        scopes: &[String],
        pkce: &PkceChallenge,
        state: &str,
    ) -> String {
        let scope_str = scopes.join(" ");

        format!(
            "{}?response_type=code&client_id={}&redirect_uri={}&scope={}&code_challenge={}&code_challenge_method={}&state={}",
            auth_url,
            urlencoding::encode(client_id),
            urlencoding::encode(redirect_uri),
            urlencoding::encode(&scope_str),
            urlencoding::encode(&pkce.code_challenge),
            urlencoding::encode(&pkce.code_challenge_method),
            urlencoding::encode(state),
        )
    }

    /// Exchange authorization code for access token (with PKCE)
    ///
    /// # Arguments
    /// * `server_id` - MCP server ID
    /// * `oauth_config` - OAuth configuration
    /// * `authorization_code` - Authorization code from callback
    /// * `redirect_uri` - Redirect URI used in authorization request
    /// * `code_verifier` - PKCE code verifier
    ///
    /// # Returns
    /// * Access token
    pub async fn exchange_code_for_token(
        &self,
        server_id: &str,
        oauth_config: &McpOAuthConfig,
        authorization_code: &str,
        redirect_uri: &str,
        code_verifier: &str,
    ) -> AppResult<String> {
        tracing::info!("Exchanging authorization code for token: {}", server_id);

        // Retrieve client_secret from keychain
        let client_secret = self
            .keychain
            .get(MCP_OAUTH_SERVICE, &format!("{}_client_secret", server_id))
            .map_err(|e| AppError::Mcp(format!("Failed to retrieve client secret: {}", e)))?
            .ok_or_else(|| AppError::Mcp("Client secret not found in keychain".to_string()))?;

        // Prepare token exchange request
        let mut params = HashMap::new();
        params.insert("grant_type", "authorization_code");
        params.insert("code", authorization_code);
        params.insert("redirect_uri", redirect_uri);
        params.insert("client_id", &oauth_config.client_id);
        params.insert("client_secret", &client_secret);
        params.insert("code_verifier", code_verifier);

        // Send token request
        let response = self
            .client
            .post(&oauth_config.token_url)
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
        let token_response: TokenResponse = response
            .json()
            .await
            .map_err(|e| AppError::Mcp(format!("Failed to parse token response: {}", e)))?;

        // Calculate expiration time
        let expires_at = if let Some(expires_in) = token_response.expires_in {
            Utc::now() + Duration::seconds(expires_in)
        } else {
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
            .store(
                MCP_OAUTH_SERVICE,
                &format!("{}_access_token", server_id),
                &token_info.access_token,
            )
            .map_err(|e| AppError::Mcp(format!("Failed to store token in keychain: {}", e)))?;

        if let Some(ref refresh_token) = token_info.refresh_token {
            self.keychain
                .store(
                    MCP_OAUTH_SERVICE,
                    &format!("{}_refresh_token", server_id),
                    refresh_token,
                )
                .ok();
        }

        tracing::info!("Token exchange successful for: {}", server_id);

        Ok(token_response.access_token)
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

    #[test]
    fn test_pkce_generation() {
        let pkce = generate_pkce_challenge();

        // Verify code_verifier length (should be 64 characters)
        assert_eq!(pkce.code_verifier.len(), 64);

        // Verify code_verifier contains only valid characters
        assert!(pkce
            .code_verifier
            .chars()
            .all(|c| c.is_ascii_alphanumeric()));

        // Verify code_challenge is base64url encoded
        assert!(!pkce.code_challenge.is_empty());

        // Verify challenge method
        assert_eq!(pkce.code_challenge_method, "S256");

        // Verify challenge is deterministic for same verifier
        let mut hasher = Sha256::new();
        hasher.update(pkce.code_verifier.as_bytes());
        let hash = hasher.finalize();
        let expected_challenge = URL_SAFE_NO_PAD.encode(hash);
        assert_eq!(pkce.code_challenge, expected_challenge);
    }

    #[test]
    fn test_pkce_uniqueness() {
        // Generate multiple PKCE challenges and verify they're all unique
        let pkce1 = generate_pkce_challenge();
        let pkce2 = generate_pkce_challenge();
        let pkce3 = generate_pkce_challenge();

        assert_ne!(pkce1.code_verifier, pkce2.code_verifier);
        assert_ne!(pkce1.code_verifier, pkce3.code_verifier);
        assert_ne!(pkce2.code_verifier, pkce3.code_verifier);

        assert_ne!(pkce1.code_challenge, pkce2.code_challenge);
        assert_ne!(pkce1.code_challenge, pkce3.code_challenge);
        assert_ne!(pkce2.code_challenge, pkce3.code_challenge);
    }

    #[test]
    fn test_build_authorization_url() {
        let pkce = generate_pkce_challenge();
        let auth_url = "https://auth.example.com/authorize";
        let client_id = "test_client_id";
        let redirect_uri = "http://localhost:8080/callback";
        let scopes = vec!["read".to_string(), "write".to_string()];
        let state = "random_state_string";

        let url = McpOAuthManager::build_authorization_url(
            auth_url,
            client_id,
            redirect_uri,
            &scopes,
            &pkce,
            state,
        );

        // Verify URL contains all required parameters
        assert!(url.contains("response_type=code"));
        assert!(url.contains(&format!("client_id={}", urlencoding::encode(client_id))));
        assert!(url.contains(&format!(
            "redirect_uri={}",
            urlencoding::encode(redirect_uri)
        )));
        assert!(url.contains("scope=read%20write"));
        assert!(url.contains(&format!(
            "code_challenge={}",
            urlencoding::encode(&pkce.code_challenge)
        )));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains(&format!("state={}", state)));
        assert!(url.starts_with(auth_url));
    }

    #[test]
    fn test_build_well_known_url_no_path() {
        // URL without path - append well-known directly
        assert_eq!(
            build_well_known_url("https://api.example.com"),
            "https://api.example.com/.well-known/oauth-protected-resource"
        );

        // URL with trailing slash - same result
        assert_eq!(
            build_well_known_url("https://api.example.com/"),
            "https://api.example.com/.well-known/oauth-protected-resource"
        );
    }

    #[test]
    fn test_build_well_known_url_with_path() {
        // URL with simple path - insert well-known between host and path
        assert_eq!(
            build_well_known_url("https://api.githubcopilot.com/mcp"),
            "https://api.githubcopilot.com/.well-known/oauth-protected-resource/mcp"
        );

        // URL with multi-segment path
        assert_eq!(
            build_well_known_url("https://gitlab.com/api/v4/mcp"),
            "https://gitlab.com/.well-known/oauth-protected-resource/api/v4/mcp"
        );

        // URL with trailing slash on path
        assert_eq!(
            build_well_known_url("https://api.example.com/mcp/"),
            "https://api.example.com/.well-known/oauth-protected-resource/mcp"
        );
    }

    #[test]
    fn test_build_well_known_url_with_port() {
        // URL with port and no path
        assert_eq!(
            build_well_known_url("https://api.example.com:8443"),
            "https://api.example.com:8443/.well-known/oauth-protected-resource"
        );

        // URL with port and path
        assert_eq!(
            build_well_known_url("https://api.example.com:8443/mcp"),
            "https://api.example.com:8443/.well-known/oauth-protected-resource/mcp"
        );
    }

    #[test]
    fn test_build_authorization_server_metadata_url_no_path() {
        assert_eq!(
            build_authorization_server_metadata_url("https://auth.example.com"),
            "https://auth.example.com/.well-known/oauth-authorization-server"
        );

        assert_eq!(
            build_authorization_server_metadata_url("https://auth.example.com/"),
            "https://auth.example.com/.well-known/oauth-authorization-server"
        );
    }

    #[test]
    fn test_build_authorization_server_metadata_url_with_path() {
        // GitHub-style OAuth URL with path
        assert_eq!(
            build_authorization_server_metadata_url("https://github.com/login/oauth"),
            "https://github.com/.well-known/oauth-authorization-server/login/oauth"
        );

        // Multi-segment path
        assert_eq!(
            build_authorization_server_metadata_url("https://example.com/oauth2/v1"),
            "https://example.com/.well-known/oauth-authorization-server/oauth2/v1"
        );
    }
}
