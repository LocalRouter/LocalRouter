//! OAuth token exchange and refresh logic

use crate::browser::{OAuthFlowConfig, OAuthTokens};
use chrono::{Duration, Utc};
use lr_api_keys::{keychain_trait::KeychainStorage, CachedKeychain};
use lr_types::{AppError, AppResult};
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

/// Marker key in [`OAuthFlowConfig::extra_token_params`] that switches the
/// token request from `application/x-www-form-urlencoded` to a JSON body.
/// OpenAI's `auth.openai.com/oauth/token` expects JSON on the refresh grant
/// (this is what codex-rs sends). The marker is stripped from the request —
/// it is never sent upstream.
pub const USE_JSON_BODY_PARAM: &str = "_use_json_body";

/// Only standard error codes reach logs/UI. OAuth endpoints can echo secrets
/// in arbitrary descriptions, bodies or redirect URLs.
fn token_error_detail(body: &str) -> &'static str {
    let value = serde_json::from_str::<serde_json::Value>(body).ok();
    match value
        .as_ref()
        .and_then(|v| v.get("error"))
        .and_then(|v| v.as_str())
    {
        Some("invalid_grant") => {
            "invalid_grant; sign in again because the refresh token or grant is expired or revoked"
        }
        Some("invalid_client") => "invalid_client; check the registered OAuth app credentials",
        Some("invalid_scope") => "invalid_scope; review the authorized scopes and sign in again",
        Some("access_denied") => "access_denied; review provider authorization",
        Some("temporarily_unavailable") => "temporarily_unavailable; retry later",
        Some("server_error") => "server_error; retry later",
        _ => "token endpoint rejected the request",
    }
}

/// Token exchanger for OAuth flows
pub struct TokenExchanger {
    client: Client,
}

impl TokenExchanger {
    /// Create a new token exchanger
    pub fn new() -> Self {
        Self {
            client: Client::builder()
                .connect_timeout(std::time::Duration::from_secs(10))
                .timeout(std::time::Duration::from_secs(30))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .expect("OAuth token HTTP client"),
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
            .token_request(&config.token_url, params)
            .send()
            .await
            .map_err(|e| {
                AppError::OAuthBrowser(format!("Failed to send token request: {}", e.without_url()))
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            let detail = token_error_detail(&body);
            error!("Token exchange failed with status {}: {}", status, detail);
            return Err(AppError::OAuthBrowser(format!(
                "Token exchange failed with status {}: {}",
                status, detail
            )));
        }

        // Parse token response
        let token_response: TokenResponse = response.json().await.map_err(|e| {
            AppError::OAuthBrowser(format!(
                "Failed to parse token response: {}",
                e.without_url()
            ))
        })?;

        // Calculate expiration time (with 5-minute buffer for safety)
        let expires_at = token_response.expires_in.map(|expires_in| {
            Utc::now()
                + Duration::seconds(expires_in.saturating_sub((expires_in / 10).clamp(0, 300)))
        });

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
        let tokens = self
            .refresh_tokens_unstored(config, refresh_token, keychain)
            .await?;
        self.store_tokens(&tokens, config, keychain)?;
        Ok(tokens)
    }

    /// Fetch a refreshed grant without committing it. Consumers with concurrent
    /// revocation can validate ownership under their credential lock before saving.
    pub async fn refresh_tokens_unstored(
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
            .token_request(&config.token_url, params)
            .send()
            .await
            .map_err(|e| {
                AppError::OAuthBrowser(format!(
                    "Failed to send refresh request: {}",
                    e.without_url()
                ))
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            let detail = token_error_detail(&body);
            error!("Token refresh failed with status {}: {}", status, detail);
            return Err(AppError::OAuthBrowser(format!(
                "Token refresh failed with status {}: {}",
                status, detail
            )));
        }

        // Parse token response
        let token_response: TokenResponse = response.json().await.map_err(|e| {
            AppError::OAuthBrowser(format!(
                "Failed to parse refresh response: {}",
                e.without_url()
            ))
        })?;

        // Calculate expiration time (with 5-minute buffer)
        let expires_at = token_response.expires_in.map(|expires_in| {
            Utc::now()
                + Duration::seconds(expires_in.saturating_sub((expires_in / 10).clamp(0, 300)))
        });

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

        info!("Token refresh successful for: {}", config.account_id);

        Ok(tokens)
    }

    /// Build the POST to the token endpoint, encoding the body the way the
    /// authorization server expects.
    ///
    /// Form encoding is the OAuth 2.0 default; providers that opt in with
    /// [`USE_JSON_BODY_PARAM`] get a JSON body instead. The marker itself is
    /// removed either way so it never reaches the server.
    fn token_request(
        &self,
        token_url: &str,
        mut params: HashMap<String, String>,
    ) -> reqwest::RequestBuilder {
        let use_json = params.remove(USE_JSON_BODY_PARAM).is_some();
        let request = self.client.post(token_url);
        if use_json {
            request.json(&params)
        } else {
            request.form(&params)
        }
    }

    /// Store tokens in keychain
    pub fn store_tokens(
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

        // Store the expiry (unix seconds) so consumers can tell a stale access
        // token from a good one without calling upstream — the access token
        // itself is opaque for some providers. Missing key = unknown expiry.
        match tokens.expires_at {
            Some(expires_at) => {
                keychain
                    .store(
                        &config.keychain_service,
                        &format!("{}_expires_at", config.account_id),
                        &expires_at.timestamp().to_string(),
                    )
                    .ok();
            }
            None => {
                keychain
                    .delete(
                        &config.keychain_service,
                        &format!("{}_expires_at", config.account_id),
                    )
                    .ok();
            }
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
    fn token_errors_never_echo_provider_descriptions_or_credentials() {
        let error = token_error_detail(
            r#"{"error":"invalid_grant","error_description":"refresh_token=super-secret"}"#,
        );
        assert!(error.contains("sign in again"));
        assert!(!error.contains("super-secret"));
        assert_eq!(
            token_error_detail("Bearer super-secret"),
            "token endpoint rejected the request"
        );
        assert_eq!(
            token_error_detail(r#"{"error":"super-secret"}"#),
            "token endpoint rejected the request"
        );
    }

    #[tokio::test]
    async fn refresh_preserves_rotation_and_buffers_short_lived_tokens() {
        use axum::{routing::post, Form, Json, Router};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let app = Router::new().route(
            "/token",
            post(|Form(body): Form<HashMap<String, String>>| async move {
                assert_eq!(
                    body.get("refresh_token").map(String::as_str),
                    Some("original-refresh")
                );
                Json(serde_json::json!({"access_token":"short-lived", "expires_in":60}))
            }),
        );
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let keychain = CachedKeychain::new(std::sync::Arc::new(lr_api_keys::MockKeychain::new()));
        let config = OAuthFlowConfig {
            client_id: "public".into(),
            client_secret: None,
            auth_url: format!("{origin}/authorize"),
            token_url: format!("{origin}/token"),
            scopes: vec![],
            redirect_uri: "http://localhost:8080/callback".into(),
            callback_port: 8080,
            keychain_service: "test-oauth".into(),
            account_id: "short".into(),
            extra_auth_params: HashMap::new(),
            extra_token_params: HashMap::new(),
            expected_issuer: None,
        };
        let tokens = TokenExchanger::new()
            .refresh_tokens(&config, "original-refresh", &keychain)
            .await
            .unwrap();
        assert_eq!(tokens.refresh_token.as_deref(), Some("original-refresh"));
        let remaining = (tokens.expires_at.unwrap() - Utc::now()).num_seconds();
        assert!(
            (50..=54).contains(&remaining),
            "short tokens must remain usable: {remaining}"
        );
        assert_eq!(
            keychain
                .get("test-oauth", "short_refresh_token")
                .unwrap()
                .as_deref(),
            Some("original-refresh")
        );
        task.abort();
    }

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

    fn test_config() -> OAuthFlowConfig {
        OAuthFlowConfig {
            client_id: "client".to_string(),
            client_secret: None,
            auth_url: "https://example.test/authorize".to_string(),
            token_url: "https://example.test/token".to_string(),
            scopes: vec![],
            redirect_uri: "http://localhost:1455/auth/callback".to_string(),
            callback_port: 1455,
            keychain_service: "LocalRouter-ProviderTokens".to_string(),
            account_id: "test-provider".to_string(),
            extra_auth_params: HashMap::new(),
            extra_token_params: HashMap::new(),
            expected_issuer: None,
        }
    }

    fn body_of(request: reqwest::RequestBuilder) -> (Option<String>, String) {
        let request = request.build().expect("request builds");
        let content_type = request
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .map(|v| v.to_str().unwrap().to_string());
        let body = String::from_utf8(
            request
                .body()
                .and_then(|b| b.as_bytes())
                .unwrap_or_default()
                .to_vec(),
        )
        .unwrap();
        (content_type, body)
    }

    #[test]
    fn token_request_defaults_to_form_encoding() {
        let mut params = HashMap::new();
        params.insert("grant_type".to_string(), "refresh_token".to_string());

        let (content_type, body) =
            body_of(TokenExchanger::new().token_request("https://example.test/token", params));

        assert_eq!(
            content_type.as_deref(),
            Some("application/x-www-form-urlencoded")
        );
        assert_eq!(body, "grant_type=refresh_token");
    }

    #[test]
    fn token_request_honors_the_json_body_marker_without_sending_it() {
        let mut params = HashMap::new();
        params.insert("grant_type".to_string(), "refresh_token".to_string());
        params.insert(USE_JSON_BODY_PARAM.to_string(), "true".to_string());

        let (content_type, body) =
            body_of(TokenExchanger::new().token_request("https://example.test/token", params));

        assert_eq!(content_type.as_deref(), Some("application/json"));
        assert_eq!(body, r#"{"grant_type":"refresh_token"}"#);
        assert!(!body.contains(USE_JSON_BODY_PARAM));
    }

    #[test]
    fn store_tokens_records_the_expiry_next_to_the_tokens() {
        let keychain = CachedKeychain::new(std::sync::Arc::new(lr_api_keys::MockKeychain::new()));
        let expires_at = Utc::now() + Duration::seconds(3600);
        let tokens = OAuthTokens {
            access_token: "access".to_string(),
            refresh_token: Some("refresh".to_string()),
            token_type: "Bearer".to_string(),
            expires_in: Some(3600),
            expires_at: Some(expires_at),
            scope: None,
            acquired_at: Utc::now(),
        };

        TokenExchanger::new()
            .store_tokens(&tokens, &test_config(), &keychain)
            .unwrap();

        assert_eq!(
            keychain
                .get("LocalRouter-ProviderTokens", "test-provider_expires_at")
                .unwrap(),
            Some(expires_at.timestamp().to_string())
        );
        assert_eq!(
            keychain
                .get("LocalRouter-ProviderTokens", "test-provider_refresh_token")
                .unwrap(),
            Some("refresh".to_string())
        );
    }

    #[test]
    fn store_tokens_clears_a_stale_expiry_when_the_new_one_is_unknown() {
        let keychain = CachedKeychain::new(std::sync::Arc::new(lr_api_keys::MockKeychain::new()));
        keychain
            .store(
                "LocalRouter-ProviderTokens",
                "test-provider_expires_at",
                "1700000000",
            )
            .unwrap();

        let tokens = OAuthTokens {
            access_token: "access".to_string(),
            refresh_token: None,
            token_type: "Bearer".to_string(),
            expires_in: None,
            expires_at: None,
            scope: None,
            acquired_at: Utc::now(),
        };

        TokenExchanger::new()
            .store_tokens(&tokens, &test_config(), &keychain)
            .unwrap();

        assert_eq!(
            keychain
                .get("LocalRouter-ProviderTokens", "test-provider_expires_at")
                .unwrap(),
            None
        );
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
