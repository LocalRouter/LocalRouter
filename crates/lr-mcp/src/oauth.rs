//! OAuth support for MCP servers
//!
//! Handles OAuth discovery, token acquisition, and token management for MCP servers
//! that require OAuth authentication.

#![allow(dead_code)]

use axum::{
    extract::Query,
    http::StatusCode,
    response::{Html, IntoResponse},
    Router,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use chrono::{DateTime, Duration, Utc};
use lr_api_keys::{CachedKeychain, KeychainStorage};
use lr_config::McpOAuthConfig;
use lr_types::{AppError, AppResult};
use parking_lot::{Mutex, RwLock};
use reqwest::Client;
use ring::rand::{SecureRandom, SystemRandom};
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

    browser_refresh_locks: dashmap::DashMap<String, Arc<tokio::sync::Mutex<()>>>,

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
    /// Authorization server issuer identifier (RFC 8414; used for
    /// RFC 9207 `iss` validation)
    #[serde(default)]
    pub issuer: Option<String>,

    /// Authorization endpoint URL
    pub authorization_endpoint: String,

    /// Token endpoint URL
    pub token_endpoint: String,

    /// Dynamic public-client registration endpoint (RFC 7591).
    #[serde(default)]
    pub registration_endpoint: Option<String>,

    /// Supported scopes
    #[serde(default)]
    pub scopes_supported: Vec<String>,

    /// Supported grant types
    #[serde(default)]
    pub grant_types_supported: Vec<String>,
}

/// Parse an HTTP authentication challenge parameter (quoted-string or token).
pub(crate) fn challenge_parameter(challenge: &str, name: &str) -> Option<String> {
    let pattern = format!(
        r#"(?i)\b{}\s*=\s*(?:"((?:\\.|[^"\\])*)"|([^\s,]+))"#,
        regex::escape(name)
    );
    let regex = regex::Regex::new(&pattern).ok()?;
    let captures = regex.captures(challenge)?;
    captures
        .get(1)
        .or_else(|| captures.get(2))
        .map(|v| v.as_str().replace("\\\"", "\"").replace("\\\\", "\\"))
}

/// Combined OAuth discovery response
///
/// This is the unified response returned by discover_oauth, combining
/// information from protected resource metadata and authorization server metadata
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OAuthDiscoveryResponse {
    /// Authorization server issuer identifier, when known.
    /// Used to validate the `iss` authorization-response parameter
    /// (RFC 9207) and to bind cached credentials to their issuer.
    #[serde(default)]
    pub issuer: Option<String>,

    /// Authorization endpoint URL
    #[serde(rename = "authorization_endpoint")]
    pub auth_url: String,

    /// Token endpoint URL
    pub token_endpoint: String,

    /// Dynamic client registration endpoint.
    #[serde(default)]
    pub registration_endpoint: Option<String>,

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
pub fn generate_pkce_challenge() -> Result<PkceChallenge, &'static str> {
    // Generate random code_verifier (64 bytes, base64url-encoded = 86 characters)
    // RFC 7636 specifies 43-128 characters from unreserved URI characters
    let rng = SystemRandom::new();
    let mut verifier_bytes = [0u8; 64];
    rng.fill(&mut verifier_bytes)
        .map_err(|_| "Failed to generate random PKCE verifier")?;
    let code_verifier = URL_SAFE_NO_PAD.encode(verifier_bytes);

    // Generate code_challenge = BASE64URL(SHA256(code_verifier))
    let mut hasher = Sha256::new();
    hasher.update(code_verifier.as_bytes());
    let hash = hasher.finalize();
    let code_challenge = URL_SAFE_NO_PAD.encode(hash);

    Ok(PkceChallenge {
        code_verifier,
        code_challenge,
        code_challenge_method: "S256".to_string(),
    })
}

/// Generate a random state string for CSRF protection
pub fn generate_state() -> Result<String, &'static str> {
    let rng = SystemRandom::new();
    let mut state_bytes = [0u8; 32];
    rng.fill(&mut state_bytes)
        .map_err(|_| "Failed to generate random state")?;
    Ok(URL_SAFE_NO_PAD.encode(state_bytes))
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
    let url = resource_url
        .split(['?', '#'])
        .next()
        .unwrap_or(resource_url)
        .trim_end_matches('/');

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
                                <p>You can close this window and return to LocalRouter.</p>
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
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .expect("OAuth HTTP client"),
            keychain,
            token_cache: Arc::new(RwLock::new(HashMap::new())),
            browser_refresh_locks: dashmap::DashMap::new(),
        }
    }

    /// Create a new OAuth manager with a custom keychain
    ///
    /// Useful for testing with MockKeychain or custom keychain implementations.
    pub fn new_with_keychain(keychain: CachedKeychain) -> Self {
        Self {
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .expect("OAuth HTTP client"),
            keychain,
            token_cache: Arc::new(RwLock::new(HashMap::new())),
            browser_refresh_locks: dashmap::DashMap::new(),
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
        // A POST-only MCP server may challenge POST requests but reject GET.
        let probe = self
            .client
            .post(base_url)
            .header("Accept", "application/json, text/event-stream")
            .header("Mcp-Method", "server/discover")
            .header(
                "Mcp-Protocol-Version",
                crate::protocol::MCP_PROTOCOL_VERSION_STATELESS,
            )
            .json(&crate::discovery::discovery_request())
            .send()
            .await;
        let mut headers = probe
            .map(|response| response.headers().clone())
            .unwrap_or_default();
        // Legacy SSE endpoints can advertise authentication only on their GET stream.
        if !headers.contains_key(reqwest::header::WWW_AUTHENTICATE) {
            if let Ok(response) = self
                .client
                .get(base_url)
                .header("Accept", "application/json, text/event-stream")
                .send()
                .await
            {
                headers = response.headers().clone();
            }
        }
        self.discover_oauth_from_headers(base_url, &headers).await
    }

    /// Discover authorization from a previously observed MCP response. Custom
    /// credential headers are never forwarded to metadata or authorization servers.
    pub async fn discover_oauth_from_headers(
        &self,
        base_url: &str,
        headers: &reqwest::header::HeaderMap,
    ) -> AppResult<Option<OAuthDiscoveryResponse>> {
        // Step 1: Fetch Protected Resource Metadata (RFC 9728)
        let discovery_url = build_well_known_url(base_url);
        tracing::info!(
            "Discovering protected resource metadata at: {}",
            discovery_url
        );

        let origin = reqwest::Url::parse(base_url)
            .map_err(|e| AppError::Mcp(format!("Invalid MCP URL: {e}")))?
            .origin()
            .ascii_serialization();
        let root_url = format!("{origin}/.well-known/oauth-protected-resource");
        let mut response = None;
        let mut candidates = vec![discovery_url, root_url];
        // RFC 9728 permits servers to advertise a custom metadata location.
        {
            for challenge in headers.get_all(reqwest::header::WWW_AUTHENTICATE) {
                if let Ok(challenge) = challenge.to_str() {
                    if let Some(metadata_url) = challenge_parameter(challenge, "resource_metadata")
                    {
                        if reqwest::Url::parse(&metadata_url).is_ok_and(|u| {
                            matches!(u.scheme(), "http" | "https")
                                && u.username().is_empty()
                                && u.password().is_none()
                        }) {
                            candidates.insert(0, metadata_url);
                        }
                    }
                }
            }
        }
        for candidate in candidates {
            if let Ok(resp) = self.client.get(&candidate).send().await {
                if resp.status().is_success() {
                    response = Some(resp);
                    break;
                }
            }
        }
        let Some(response) = response else {
            // Older MCP OAuth deployments publish AS metadata at the origin.
            return self.discover_authorization_server(&origin, &[]).await;
        };

        // Parse protected resource metadata
        let mut resource_metadata: ProtectedResourceMetadata =
            response.json().await.map_err(|e| {
                AppError::Mcp(format!(
                    "Failed to parse protected resource metadata: {}",
                    e
                ))
            })?;

        // The challenge specifies the scopes for the operation being attempted.
        if let Some(scopes) = headers
            .get_all(reqwest::header::WWW_AUTHENTICATE)
            .iter()
            .filter_map(|h| h.to_str().ok())
            .find_map(|h| challenge_parameter(h, "scope"))
        {
            resource_metadata.scopes_supported =
                scopes.split_whitespace().map(str::to_string).collect();
        }

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
        let issuer = reqwest::Url::parse(auth_server_url)
            .map_err(|_| AppError::Mcp("Invalid OAuth authorization server URL".into()))?;
        if !matches!(issuer.scheme(), "http" | "https")
            || !issuer.username().is_empty()
            || issuer.password().is_some()
        {
            return Err(AppError::Mcp(
                "Invalid OAuth authorization server URL".into(),
            ));
        }
        let origin = issuer.origin().ascii_serialization();
        let path = issuer.path().trim_end_matches('/');
        let mut candidates = vec![
            build_authorization_server_metadata_url(auth_server_url),
            format!("{origin}/.well-known/openid-configuration{path}"),
        ];
        if !path.is_empty() {
            candidates.push(format!("{origin}{path}/.well-known/openid-configuration"));
        }

        for metadata_url in candidates {
            tracing::info!("Trying authorization server metadata at: {}", metadata_url);

            let response = self.client.get(&metadata_url).send().await;

            if let Ok(resp) = response {
                if resp.status().is_success() {
                    if let Ok(metadata) = resp.json::<AuthorizationServerMetadata>().await {
                        // Metadata must be bound to exactly the advertised issuer.
                        if metadata.issuer.as_deref() != Some(auth_server_url) {
                            return Err(AppError::Mcp(
                                "OAuth metadata issuer does not match the authorization server"
                                    .into(),
                            ));
                        }
                        tracing::info!(
                            "Authorization server metadata found: auth={}, token={}",
                            metadata.authorization_endpoint,
                            metadata.token_endpoint
                        );

                        // Use scopes from auth server if available, otherwise from resource
                        let scopes = if resource_scopes.is_empty() {
                            metadata.scopes_supported
                        } else {
                            resource_scopes.to_vec()
                        };

                        return Ok(Some(OAuthDiscoveryResponse {
                            issuer: metadata.issuer,
                            registration_endpoint: metadata.registration_endpoint,
                            auth_url: metadata.authorization_endpoint,
                            token_endpoint: metadata.token_endpoint,
                            scopes_supported: scopes,
                            grant_types_supported: metadata.grant_types_supported,
                        }));
                    }
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
                registration_endpoint: None,
                issuer: Some("https://github.com".to_string()),
                auth_url: "https://github.com/login/oauth/authorize".to_string(),
                token_endpoint: "https://github.com/login/oauth/access_token".to_string(),
                scopes_supported: resource_scopes.to_vec(),
                grant_types_supported: vec!["authorization_code".to_string()],
            });
        }

        // Google OAuth
        if url_lower.contains("google.com") || url_lower.contains("googleapis.com") {
            return Some(OAuthDiscoveryResponse {
                registration_endpoint: None,
                issuer: Some("https://accounts.google.com".to_string()),
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
                registration_endpoint: None,
                issuer: Some("https://login.microsoftonline.com/common/v2.0".to_string()),
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

    /// Discover browser login settings and register a native public client when needed.
    pub async fn prepare_browser_config(
        &self,
        server_id: &str,
        mcp_url: &str,
        config: &lr_config::McpAuthConfig,
    ) -> AppResult<lr_config::McpAuthConfig> {
        let lr_config::McpAuthConfig::OAuthBrowser {
            client_id,
            client_secret_ref,
            auth_url,
            token_url,
            scopes,
            redirect_uri,
            issuer,
        } = config
        else {
            return Err(AppError::Mcp("Browser OAuth is not configured".into()));
        };
        let discovery = self.discover_oauth(mcp_url).await?.ok_or_else(|| {
            AppError::Mcp("This MCP server does not publish OAuth metadata".into())
        })?;
        let issuer_changed = issuer
            .as_ref()
            .is_some_and(|old| discovery.issuer.as_ref() != Some(old));
        let mut registered_id = client_id.clone();
        if registered_id.is_empty() || issuer_changed {
            let endpoint = discovery.registration_endpoint.as_ref().ok_or_else(||
                AppError::Mcp("This server requires a registered OAuth client ID. Use OAuth client settings to provide one.".into()))?;
            let response = self
                .client
                .post(endpoint)
                .json(&serde_json::json!({
                    "client_name": "LocalRouter",
                    "redirect_uris": [redirect_uri],
                    "grant_types": ["authorization_code", "refresh_token"],
                    "response_types": ["code"],
                    "token_endpoint_auth_method": "none",
                    "application_type": "native"
                }))
                .send()
                .await
                .map_err(|e| AppError::Mcp(format!("OAuth client registration failed: {e}")))?;
            if !response.status().is_success() {
                return Err(AppError::Mcp(format!(
                    "OAuth client registration rejected ({})",
                    response.status()
                )));
            }
            let registration: serde_json::Value = response
                .json()
                .await
                .map_err(|e| AppError::Mcp(format!("Invalid registration response: {e}")))?;
            registered_id = registration
                .get("client_id")
                .and_then(|v| v.as_str())
                .filter(|id| !id.is_empty())
                .ok_or_else(|| AppError::Mcp("Registration response missing client_id".into()))?
                .to_string();
        }
        if issuer_changed {
            self.keychain
                .delete(lr_config::MCP_KEYRING_SERVICE, client_secret_ref)?;
        }
        self.enforce_issuer_binding(
            server_id,
            discovery
                .issuer
                .as_deref()
                .unwrap_or(&discovery.token_endpoint),
        );
        Ok(lr_config::McpAuthConfig::OAuthBrowser {
            client_id: registered_id,
            client_secret_ref: client_secret_ref.clone(),
            auth_url: if auth_url.is_empty() || issuer_changed {
                discovery.auth_url
            } else {
                auth_url.clone()
            },
            token_url: if token_url.is_empty() || issuer_changed {
                discovery.token_endpoint
            } else {
                token_url.clone()
            },
            scopes: if scopes.is_empty() || issuer_changed {
                discovery.scopes_supported
            } else {
                scopes.clone()
            },
            redirect_uri: redirect_uri.clone(),
            issuer: discovery.issuer,
        })
    }

    /// Return a browser token, refreshing expired credentials for public clients.
    pub async fn get_browser_token(
        &self,
        server_id: &str,
        auth: &lr_config::McpAuthConfig,
        resource_url: &str,
    ) -> AppResult<String> {
        self.browser_token(server_id, auth, resource_url, false)
            .await
    }

    /// Refresh the browser token even if the stored one has not expired —
    /// used after the server rejected it (revoked, or expiry clock skew).
    pub async fn refresh_browser_token(
        &self,
        server_id: &str,
        auth: &lr_config::McpAuthConfig,
        resource_url: &str,
    ) -> AppResult<String> {
        self.browser_token(server_id, auth, resource_url, true)
            .await
    }

    async fn browser_token(
        &self,
        server_id: &str,
        auth: &lr_config::McpAuthConfig,
        resource_url: &str,
        force_refresh: bool,
    ) -> AppResult<String> {
        // Refresh slightly before expiry so a token is never sent just as it lapses
        const EXPIRY_MARGIN_SECS: i64 = 60;
        let refresh_lock = self
            .browser_refresh_locks
            .entry(server_id.to_string())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone();
        let _refresh_guard = refresh_lock.lock().await;
        let lr_config::McpAuthConfig::OAuthBrowser {
            client_id,
            client_secret_ref,
            auth_url,
            token_url,
            scopes,
            redirect_uri,
            issuer,
        } = auth
        else {
            return Err(AppError::Mcp("Expected browser OAuth configuration".into()));
        };
        self.check_issuer_binding(server_id, issuer.as_deref().unwrap_or(token_url));
        let expiry = self
            .keychain
            .get(MCP_OAUTH_SERVICE, &format!("{server_id}_expires_at"))?
            .and_then(|value| value.parse::<i64>().ok());
        if !force_refresh
            && expiry
                .is_none_or(|timestamp| timestamp > Utc::now().timestamp() + EXPIRY_MARGIN_SECS)
        {
            if let Some(token) = self
                .keychain
                .get(MCP_OAUTH_SERVICE, &format!("{server_id}_access_token"))?
            {
                return Ok(token);
            }
        }
        let refresh = self
            .keychain
            .get(MCP_OAUTH_SERVICE, &format!("{server_id}_refresh_token"))?
            .ok_or_else(|| {
                AppError::Mcp("Browser login required. Authenticate this MCP server first.".into())
            })?;
        let mut resource = reqwest::Url::parse(resource_url)
            .map_err(|e| AppError::Mcp(format!("Invalid resource URL: {e}")))?;
        resource.set_query(None);
        resource.set_fragment(None);
        let flow = lr_oauth::browser::OAuthFlowConfig {
            client_id: client_id.clone(),
            client_secret: self
                .keychain
                .get(lr_config::MCP_KEYRING_SERVICE, client_secret_ref)?
                .filter(|s| !s.is_empty()),
            auth_url: auth_url.clone(),
            token_url: token_url.clone(),
            scopes: scopes.clone(),
            redirect_uri: redirect_uri.clone(),
            callback_port: 8080,
            keychain_service: MCP_OAUTH_SERVICE.into(),
            account_id: server_id.into(),
            extra_auth_params: HashMap::new(),
            extra_token_params: HashMap::from([("resource".into(), resource.to_string())]),
            expected_issuer: issuer.clone(),
        };
        let tokens = lr_oauth::browser::TokenExchanger::new()
            .refresh_tokens(&flow, &refresh, &self.keychain)
            .await?;
        self.update_token_cache(server_id, &tokens.access_token, tokens.expires_at)?;
        Ok(tokens.access_token)
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
        // Never reuse credentials with a different authorization server
        // (read-only on the hot path; the issuer is recorded on acquisition)
        self.check_issuer_binding(server_id, &oauth_config.token_url);

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

        // Bind the new credentials to the authorization server that issued
        // them (adopt-write happens here, alongside the token writes).
        self.keychain
            .store(
                MCP_OAUTH_SERVICE,
                &format!("{}_issuer", server_id),
                &oauth_config.token_url,
            )
            .ok();

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

        if self
            .keychain
            .get(MCP_OAUTH_SERVICE, &format!("{server_id}_expires_at"))
            .ok()
            .flatten()
            .and_then(|value| value.parse::<i64>().ok())
            .is_some_and(|expiry| expiry <= Utc::now().timestamp())
        {
            return None;
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

        // Never send a refresh token to a different authorization server
        // than the one that issued it.
        self.check_issuer_binding(server_id, &oauth_config.token_url);

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

        // Re-record the issuer binding alongside the refreshed tokens
        self.keychain
            .store(
                MCP_OAUTH_SERVICE,
                &format!("{}_issuer", server_id),
                &oauth_config.token_url,
            )
            .ok();

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
        self.keychain
            .delete(MCP_OAUTH_SERVICE, &format!("{server_id}_expires_at"))
            .ok();
    }

    /// Enforce that cached credentials are only reused with the authorization
    /// server that issued them (MCP spec / SEP-2352).
    ///
    /// `current_issuer` identifies the authorization server about to be used:
    /// the RFC 8414 `issuer` when known, otherwise the token endpoint URL as
    /// a stable stand-in. If a *different* issuer was recorded for this
    /// server's cached credentials, they are dropped so the user
    /// re-authenticates against the new authorization server. Caches created
    /// before issuer recording existed adopt the current issuer on first use.
    pub fn enforce_issuer_binding(&self, server_id: &str, current_issuer: &str) {
        let key = format!("{}_issuer", server_id);
        match self.keychain.get(MCP_OAUTH_SERVICE, &key) {
            Ok(Some(recorded)) if recorded == current_issuer => {}
            Ok(Some(recorded)) => {
                tracing::warn!(
                    "Authorization server for MCP server '{}' changed from '{}' to '{}'; \
                     discarding cached OAuth credentials and requiring re-authentication",
                    server_id,
                    recorded,
                    current_issuer
                );
                self.clear_token(server_id);
                self.keychain
                    .store(MCP_OAUTH_SERVICE, &key, current_issuer)
                    .ok();
            }
            _ => {
                // First use, or a cache from before issuer recording existed:
                // adopt the current issuer.
                self.keychain
                    .store(MCP_OAUTH_SERVICE, &key, current_issuer)
                    .ok();
            }
        }
    }

    /// Read-only variant of [`enforce_issuer_binding`] for hot paths (every
    /// backend connection builds auth headers): drops credentials on an
    /// issuer mismatch but does NOT adopt-write on first use, so ordinary
    /// connections never write to the keychain. The issuer gets recorded by
    /// the token-acquisition flows instead.
    pub fn check_issuer_binding(&self, server_id: &str, current_issuer: &str) {
        let key = format!("{}_issuer", server_id);
        if let Ok(Some(recorded)) = self.keychain.get(MCP_OAUTH_SERVICE, &key) {
            if recorded != current_issuer {
                tracing::warn!(
                    "Authorization server for MCP server '{}' changed from '{}' to '{}'; \
                     discarding cached OAuth credentials and requiring re-authentication",
                    server_id,
                    recorded,
                    current_issuer
                );
                self.clear_token(server_id);
                self.keychain
                    .store(MCP_OAUTH_SERVICE, &key, current_issuer)
                    .ok();
            }
        }
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

        // Record which authorization server the new credentials come from
        // (drops any stale credentials from a previous one).
        self.enforce_issuer_binding(server_id, &oauth_config.token_url);

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
    fn authentication_challenge_parameters_allow_spacing_case_and_tokens() {
        assert_eq!(
            challenge_parameter(
                "Bearer Resource_Metadata = \"https://example.com/meta\", scope=read",
                "resource_metadata"
            )
            .as_deref(),
            Some("https://example.com/meta")
        );
        assert_eq!(
            challenge_parameter(
                "Bearer Resource_Metadata = \"https://example.com/meta\", scope=read",
                "scope"
            )
            .as_deref(),
            Some("read")
        );
        assert_eq!(
            challenge_parameter("Bearer scope=\"read write\"", "scope").as_deref(),
            Some("read write")
        );
        assert_eq!(
            challenge_parameter("Basic realm=\"login\"", "resource_metadata"),
            None
        );
    }

    #[test]
    fn discovery_urls_strip_query_and_fragment() {
        assert_eq!(build_well_known_url("https://mcp.datadoghq.com/api/unstable/mcp-server/mcp?toolsets=ddsql#tools"),
            "https://mcp.datadoghq.com/.well-known/oauth-protected-resource/api/unstable/mcp-server/mcp");
        assert_eq!(
            build_well_known_url("https://mcp.atlassian.com/v2/mcp"),
            "https://mcp.atlassian.com/.well-known/oauth-protected-resource/v2/mcp"
        );
    }

    fn browser_config() -> lr_config::McpAuthConfig {
        lr_config::McpAuthConfig::OAuthBrowser {
            client_id: String::new(),
            client_secret_ref: "secret".into(),
            auth_url: String::new(),
            token_url: String::new(),
            scopes: vec![],
            redirect_uri: "http://localhost:8080/callback".into(),
            issuer: None,
        }
    }

    #[tokio::test]
    async fn browser_discovery_registration_and_issuer_change() {
        use axum::{
            routing::{get, post},
            Json,
        };
        use std::sync::atomic::{AtomicUsize, Ordering};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let resource_origin = origin.clone();
        let auth_origin = origin.clone();
        let count = Arc::new(AtomicUsize::new(0));
        let registrations = count.clone();
        let app = Router::new()
            // Only origin metadata: exercises fallback from nested MCP paths.
            .route("/.well-known/oauth-protected-resource", get(move || {
                let origin = resource_origin.clone();
                async move { Json(serde_json::json!({"authorization_servers": [origin], "scopes_supported": ["resource_scope"]})) }
            }))
            .route("/.well-known/oauth-authorization-server", get(move || {
                let origin = auth_origin.clone();
                async move { Json(serde_json::json!({
                    "issuer": origin, "authorization_endpoint": format!("{origin}/authorize"),
                    "token_endpoint": format!("{origin}/token"), "registration_endpoint": format!("{origin}/register"),
                    "scopes_supported": ["unrelated_scope"]
                })) }
            }))
            .route("/register", post(move |Json(body): Json<serde_json::Value>| {
                registrations.fetch_add(1, Ordering::SeqCst);
                async move {
                    assert_eq!(body["application_type"], "native");
                    assert_eq!(body["token_endpoint_auth_method"], "none");
                    assert_eq!(body["redirect_uris"][0], "http://localhost:8080/callback");
                    Json(serde_json::json!({"client_id":"registered-client"}))
                }
            }));
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let manager = mock_manager();
        let url = format!("{origin}/v2/mcp?toolsets=ddsql");
        let prepared = manager
            .prepare_browser_config("srv", &url, &browser_config())
            .await
            .unwrap();
        if let lr_config::McpAuthConfig::OAuthBrowser {
            client_id,
            issuer,
            scopes,
            ..
        } = &prepared
        {
            assert_eq!(client_id, "registered-client");
            assert_eq!(issuer.as_deref(), Some(origin.as_str()));
            assert_eq!(scopes, &vec!["resource_scope".to_string()]);
        } else {
            panic!("wrong configuration");
        }
        manager
            .prepare_browser_config("srv", &url, &prepared)
            .await
            .unwrap();
        assert_eq!(count.load(Ordering::SeqCst), 1, "reuse registered client");
        let mut changed = prepared;
        if let lr_config::McpAuthConfig::OAuthBrowser { issuer, .. } = &mut changed {
            *issuer = Some("https://old.example.com".into());
        }
        manager
            .prepare_browser_config("srv", &url, &changed)
            .await
            .unwrap();
        assert_eq!(
            count.load(Ordering::SeqCst),
            2,
            "issuer change requires registration"
        );
        task.abort();
    }

    #[tokio::test]
    async fn public_client_refresh_is_resource_bound_and_serialized() {
        use axum::{routing::post, Form, Json};
        use std::sync::atomic::{AtomicUsize, Ordering};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let resource = format!("{origin}/mcp");
        let expected_resource = resource.clone();
        let count = Arc::new(AtomicUsize::new(0));
        let requests = count.clone();
        let app = Router::new().route("/token", post(move |Form(body): Form<HashMap<String, String>>| {
            let resource = expected_resource.clone();
            requests.fetch_add(1, Ordering::SeqCst);
            async move {
                assert_eq!(body.get("resource"), Some(&resource));
                assert_eq!(body.get("grant_type").map(String::as_str), Some("refresh_token"));
                assert!(!body.contains_key("client_secret"), "native public client uses PKCE, no secret");
                Json(serde_json::json!({"access_token":"fresh", "token_type":"Bearer", "expires_in":3600, "refresh_token":"rotated"}))
            }
        }));
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let manager = mock_manager();
        manager
            .keychain
            .store(MCP_OAUTH_SERVICE, "srv_access_token", "expired")
            .unwrap();
        manager
            .keychain
            .store(MCP_OAUTH_SERVICE, "srv_expires_at", "1")
            .unwrap();
        manager
            .keychain
            .store(MCP_OAUTH_SERVICE, "srv_refresh_token", "refresh")
            .unwrap();
        let mut auth = browser_config();
        if let lr_config::McpAuthConfig::OAuthBrowser {
            client_id,
            token_url,
            ..
        } = &mut auth
        {
            *client_id = "registered-client".into();
            *token_url = format!("{origin}/token");
        }
        let url = format!("{resource}?toolsets=ddsql");
        let (first, second) = tokio::join!(
            manager.get_browser_token("srv", &auth, &url),
            manager.get_browser_token("srv", &auth, &url)
        );
        assert_eq!(first.unwrap(), "fresh");
        assert_eq!(second.unwrap(), "fresh");
        assert_eq!(count.load(Ordering::SeqCst), 1);
        assert_eq!(
            manager
                .keychain
                .get(MCP_OAUTH_SERVICE, "srv_refresh_token")
                .unwrap()
                .as_deref(),
            Some("rotated")
        );
        task.abort();
    }

    #[tokio::test]
    async fn browser_registration_errors_are_actionable() {
        use axum::{
            routing::{get, post},
            Json,
        };
        for mode in ["unavailable", "rejected", "invalid"] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let origin = format!("http://{}", listener.local_addr().unwrap());
            let auth_origin = origin.clone();
            let app = Router::new()
                .route("/.well-known/oauth-authorization-server", get(move || {
                    let origin = auth_origin.clone();
                    async move { Json(serde_json::json!({
                        "issuer":origin, "authorization_endpoint": format!("{origin}/authorize"),
                        "token_endpoint":format!("{origin}/token"),
                        "registration_endpoint": if mode == "unavailable" { None } else { Some(format!("{origin}/register")) }
                    })) }
                }))
                .route("/register", post(move || async move {
                    (if mode == "rejected" { StatusCode::BAD_REQUEST } else { StatusCode::OK }, Json(serde_json::json!({})))
                }));
            let task = tokio::spawn(async move {
                axum::serve(listener, app).await.unwrap();
            });
            let error = mock_manager()
                .prepare_browser_config("srv", &format!("{origin}/mcp"), &browser_config())
                .await
                .unwrap_err()
                .to_string();
            assert!(
                error.contains(match mode {
                    "unavailable" => "registered OAuth client ID",
                    "rejected" => "registration rejected",
                    _ => "missing client_id",
                }),
                "{error}"
            );
            task.abort();
        }
    }

    #[tokio::test]
    async fn expired_browser_token_is_not_reported_as_authenticated() {
        let manager = mock_manager();
        manager
            .keychain
            .store(MCP_OAUTH_SERVICE, "srv_access_token", "expired")
            .unwrap();
        manager
            .keychain
            .store(MCP_OAUTH_SERVICE, "srv_expires_at", "1")
            .unwrap();
        assert!(manager.get_cached_token("srv").await.is_none());
        assert!(manager
            .get_browser_token("srv", &browser_config(), "https://example.com/mcp")
            .await
            .is_err());
        manager.clear_token("srv");
        assert!(manager
            .keychain
            .get(MCP_OAUTH_SERVICE, "srv_expires_at")
            .unwrap()
            .is_none());
    }

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

    fn mock_manager() -> McpOAuthManager {
        let keychain = CachedKeychain::new(Arc::new(lr_api_keys::MockKeychain::new()));
        McpOAuthManager::new_with_keychain(keychain)
    }

    #[test]
    fn test_issuer_binding_adopts_issuer_on_first_use() {
        let manager = mock_manager();

        manager.enforce_issuer_binding("srv", "https://as.example.com");

        assert_eq!(
            manager
                .keychain
                .get(MCP_OAUTH_SERVICE, "srv_issuer")
                .unwrap()
                .as_deref(),
            Some("https://as.example.com")
        );
    }

    #[test]
    fn test_issuer_binding_same_issuer_keeps_tokens() {
        let manager = mock_manager();
        manager
            .keychain
            .store(MCP_OAUTH_SERVICE, "srv_access_token", "tok")
            .unwrap();
        manager.enforce_issuer_binding("srv", "https://as.example.com");

        // Same issuer again: token survives
        manager.enforce_issuer_binding("srv", "https://as.example.com");
        assert_eq!(
            manager
                .keychain
                .get(MCP_OAUTH_SERVICE, "srv_access_token")
                .unwrap()
                .as_deref(),
            Some("tok")
        );
    }

    #[test]
    fn test_check_issuer_binding_is_read_only_on_first_use() {
        let manager = mock_manager();

        // Hot-path check must NOT adopt-write on first use — connection
        // paths run this constantly and writes caused secrets-file
        // contention (the issuer is recorded by token-acquisition flows).
        manager.check_issuer_binding("srv", "https://as.example.com");
        assert_eq!(
            manager
                .keychain
                .get(MCP_OAUTH_SERVICE, "srv_issuer")
                .unwrap(),
            None
        );
    }

    #[test]
    fn test_check_issuer_binding_drops_credentials_on_mismatch() {
        let manager = mock_manager();
        manager
            .keychain
            .store(MCP_OAUTH_SERVICE, "srv_access_token", "tok")
            .unwrap();
        manager
            .keychain
            .store(MCP_OAUTH_SERVICE, "srv_issuer", "https://old.example.com")
            .unwrap();

        manager.check_issuer_binding("srv", "https://new.example.com");

        assert_eq!(
            manager
                .keychain
                .get(MCP_OAUTH_SERVICE, "srv_access_token")
                .unwrap(),
            None
        );
        assert_eq!(
            manager
                .keychain
                .get(MCP_OAUTH_SERVICE, "srv_issuer")
                .unwrap()
                .as_deref(),
            Some("https://new.example.com")
        );
    }

    #[test]
    fn test_issuer_binding_change_drops_credentials() {
        let manager = mock_manager();
        manager
            .keychain
            .store(MCP_OAUTH_SERVICE, "srv_access_token", "tok")
            .unwrap();
        manager
            .keychain
            .store(MCP_OAUTH_SERVICE, "srv_refresh_token", "ref")
            .unwrap();
        manager.enforce_issuer_binding("srv", "https://old-as.example.com");

        // Authorization server changed: credentials must be dropped and the
        // new issuer recorded.
        manager.enforce_issuer_binding("srv", "https://new-as.example.com");

        assert_eq!(
            manager
                .keychain
                .get(MCP_OAUTH_SERVICE, "srv_access_token")
                .unwrap(),
            None
        );
        assert_eq!(
            manager
                .keychain
                .get(MCP_OAUTH_SERVICE, "srv_refresh_token")
                .unwrap(),
            None
        );
        assert_eq!(
            manager
                .keychain
                .get(MCP_OAUTH_SERVICE, "srv_issuer")
                .unwrap()
                .as_deref(),
            Some("https://new-as.example.com")
        );
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
        let pkce = generate_pkce_challenge().unwrap();

        // Verify code verifier is base64url-encoded 64 bytes (86 characters)
        assert_eq!(pkce.code_verifier.len(), 86);

        // Verify code_verifier contains only base64url characters
        assert!(pkce
            .code_verifier
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));

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
        let pkce1 = generate_pkce_challenge().unwrap();
        let pkce2 = generate_pkce_challenge().unwrap();
        let pkce3 = generate_pkce_challenge().unwrap();

        assert_ne!(pkce1.code_verifier, pkce2.code_verifier);
        assert_ne!(pkce1.code_verifier, pkce3.code_verifier);
        assert_ne!(pkce2.code_verifier, pkce3.code_verifier);

        assert_ne!(pkce1.code_challenge, pkce2.code_challenge);
        assert_ne!(pkce1.code_challenge, pkce3.code_challenge);
        assert_ne!(pkce2.code_challenge, pkce3.code_challenge);
    }

    #[test]
    fn test_build_authorization_url() {
        let pkce = generate_pkce_challenge().unwrap();
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
