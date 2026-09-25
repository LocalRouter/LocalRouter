//! Hugging Face credentials: a pasted access token or OAuth tokens, kept in a
//! [`SecretStore`] (the app backs it with the OS keychain). Tokens are never
//! logged and never written to the settings file.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::hub::{HubClient, HubError};

/// OAuth client id: a Client ID Metadata Document URL, so no app
/// registration on huggingface.co is needed.
pub const HF_OAUTH_CLIENT_ID: &str = "https://localrouter.ai/oauth/huggingface-client.json";
/// OAuth scopes requested at sign-in.
pub const HF_OAUTH_SCOPES: &[&str] = &["openid", "profile", "read-repos", "gated-repos"];
/// Authorization endpoint.
pub const HF_AUTHORIZE_URL: &str = "https://huggingface.co/oauth/authorize";
/// Token endpoint (for a custom Hub endpoint, `{endpoint}/oauth/token` is used).
pub const HF_TOKEN_URL: &str = "https://huggingface.co/oauth/token";

/// Secret-store key of a pasted access token.
pub const KEY_TOKEN: &str = "hf_token";
/// Secret-store key of the OAuth access token.
pub const KEY_OAUTH_ACCESS: &str = "hf_oauth_access_token";
/// Secret-store key of the OAuth refresh token.
pub const KEY_OAUTH_REFRESH: &str = "hf_oauth_refresh_token";
/// Secret-store key of the OAuth access-token expiry (unix seconds).
pub const KEY_OAUTH_EXPIRES_AT: &str = "hf_oauth_expires_at";

const ALL_KEYS: &[&str] = &[
    KEY_TOKEN,
    KEY_OAUTH_ACCESS,
    KEY_OAUTH_REFRESH,
    KEY_OAUTH_EXPIRES_AT,
];

/// Refresh the OAuth access token when it expires within this many seconds.
const REFRESH_MARGIN_SECS: i64 = 60;

/// Where secrets are kept (the app implements it on top of the keychain).
pub trait SecretStore: Send + Sync {
    fn get(&self, key: &str) -> Option<String>;
    fn set(&self, key: &str, value: &str) -> Result<(), String>;
    fn delete(&self, key: &str) -> Result<(), String>;
}

/// An in-memory [`SecretStore`] (tests, headless runs).
#[derive(Default)]
pub struct MemorySecretStore(parking_lot::Mutex<std::collections::HashMap<String, String>>);

impl SecretStore for MemorySecretStore {
    fn get(&self, key: &str) -> Option<String> {
        self.0.lock().get(key).cloned()
    }

    fn set(&self, key: &str, value: &str) -> Result<(), String> {
        self.0.lock().insert(key.to_string(), value.to_string());
        Ok(())
    }

    fn delete(&self, key: &str) -> Result<(), String> {
        self.0.lock().remove(key);
        Ok(())
    }
}

/// Sign-in status for the UI.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct HfAccount {
    pub signed_in: bool,
    /// `"token"` or `"oauth"`.
    pub method: Option<String>,
    pub username: Option<String>,
    /// OAuth access-token expiry (unix seconds).
    pub expires_at: Option<i64>,
}

impl HfAccount {
    fn signed_out() -> Self {
        Self {
            signed_in: false,
            method: None,
            username: None,
            expires_at: None,
        }
    }
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: Option<i64>,
}

/// Hugging Face credentials.
pub struct HfCredentials {
    store: Arc<dyn SecretStore>,
    hub: HubClient,
    refresh_lock: tokio::sync::Mutex<()>,
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

impl HfCredentials {
    pub fn new(store: Arc<dyn SecretStore>, hub: HubClient) -> Self {
        Self {
            store,
            hub,
            refresh_lock: tokio::sync::Mutex::new(()),
        }
    }

    fn get(&self, key: &str) -> Option<String> {
        self.store.get(key).filter(|v| !v.is_empty())
    }

    fn expires_at(&self) -> Option<i64> {
        self.get(KEY_OAUTH_EXPIRES_AT)
            .and_then(|v| v.trim().parse().ok())
    }

    fn oauth_access_valid(&self) -> Option<String> {
        let access = self.get(KEY_OAUTH_ACCESS)?;
        match self.expires_at() {
            Some(exp) if exp <= now() => None,
            _ => Some(access),
        }
    }

    /// The token to use: the OAuth access token if present and not expired,
    /// else the pasted token.
    pub fn token(&self) -> Option<String> {
        self.oauth_access_valid().or_else(|| self.get(KEY_TOKEN))
    }

    /// Validate a pasted token with `whoami-v2`, then store it (replacing any
    /// OAuth sign-in).
    pub async fn set_token(&self, token: &str) -> Result<HfAccount, HubError> {
        let token = token.trim();
        if token.is_empty() {
            return Err(HubError::InvalidRequest("the token is empty".into()));
        }
        let user = self.hub.whoami(token).await.map_err(|e| match e {
            HubError::NotFoundOrPrivate(_) => {
                HubError::InvalidRequest("Hugging Face rejected this token".into())
            }
            other => other,
        })?;
        self.store
            .set(KEY_TOKEN, token)
            .map_err(|e| HubError::Storage(format!("could not save the token: {e}")))?;
        for key in [KEY_OAUTH_ACCESS, KEY_OAUTH_REFRESH, KEY_OAUTH_EXPIRES_AT] {
            if let Err(e) = self.store.delete(key) {
                tracing::warn!("could not delete {key}: {e}");
            }
        }
        Ok(HfAccount {
            signed_in: true,
            method: Some("token".into()),
            username: Some(user.name),
            expires_at: None,
        })
    }

    /// Store tokens from a completed OAuth sign-in (replacing a pasted token).
    /// `expires_at` is in unix seconds.
    pub fn store_oauth_tokens(&self, access: &str, refresh: Option<&str>, expires_at: Option<i64>) {
        let mut results = vec![self.store.set(KEY_OAUTH_ACCESS, access)];
        results.push(match refresh {
            Some(r) => self.store.set(KEY_OAUTH_REFRESH, r),
            None => self.store.delete(KEY_OAUTH_REFRESH),
        });
        results.push(match expires_at {
            Some(t) => self.store.set(KEY_OAUTH_EXPIRES_AT, &t.to_string()),
            None => self.store.delete(KEY_OAUTH_EXPIRES_AT),
        });
        results.push(self.store.delete(KEY_TOKEN));
        for r in results {
            if let Err(e) = r {
                tracing::warn!("could not store Hugging Face OAuth tokens: {e}");
            }
        }
    }

    /// Current sign-in status. Calls `whoami-v2` only when a token exists; if
    /// that fails the account is reported as signed in without a username.
    pub async fn account(&self) -> HfAccount {
        let oauth = self.get(KEY_OAUTH_ACCESS).is_some();
        let token = if oauth {
            self.refresh_if_needed().await
        } else {
            self.token()
        };
        let Some(token) = token else {
            return HfAccount::signed_out();
        };
        let username = match self.hub.whoami(&token).await {
            Ok(user) => Some(user.name),
            Err(e) => {
                tracing::debug!("whoami failed: {e}");
                None
            }
        };
        let oauth_active = self.oauth_access_valid().is_some_and(|a| a == token);
        HfAccount {
            signed_in: true,
            method: Some(if oauth_active { "oauth" } else { "token" }.into()),
            username,
            expires_at: if oauth_active {
                self.expires_at()
            } else {
                None
            },
        }
    }

    /// Delete every stored credential.
    pub fn sign_out(&self) {
        for key in ALL_KEYS {
            if let Err(e) = self.store.delete(key) {
                tracing::warn!("could not delete {key}: {e}");
            }
        }
    }

    fn token_url(&self) -> String {
        if self.hub.endpoint() == crate::hub::DEFAULT_ENDPOINT {
            HF_TOKEN_URL.to_string()
        } else {
            format!("{}/oauth/token", self.hub.endpoint())
        }
    }

    /// Refresh the OAuth access token when it expires within 60 seconds
    /// (refresh-token grant; refresh tokens rotate). Returns the token to use
    /// afterwards (see [`HfCredentials::token`]).
    pub async fn refresh_if_needed(&self) -> Option<String> {
        let _guard = self.refresh_lock.lock().await;
        let needs = self.get(KEY_OAUTH_ACCESS).is_some()
            && self
                .expires_at()
                .is_some_and(|exp| exp - now() <= REFRESH_MARGIN_SECS);
        if !needs {
            return self.token();
        }
        let Some(refresh) = self.get(KEY_OAUTH_REFRESH) else {
            return self.token();
        };
        let form = [
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh.as_str()),
            ("client_id", HF_OAUTH_CLIENT_ID),
        ];
        let result = self
            .hub
            .http_client()
            .post(self.token_url())
            .form(&form)
            .timeout(std::time::Duration::from_secs(30))
            .send()
            .await;
        match result {
            Ok(resp) if resp.status().is_success() => match resp.json::<TokenResponse>().await {
                Ok(t) => {
                    let expires_at = t.expires_in.map(|s| now() + s);
                    let refresh = t.refresh_token.as_deref().unwrap_or(refresh.as_str());
                    self.store_oauth_tokens(&t.access_token, Some(refresh), expires_at);
                }
                Err(e) => tracing::warn!("unexpected Hugging Face token response: {e}"),
            },
            Ok(resp) => tracing::warn!(
                "Hugging Face token refresh failed with HTTP {}",
                resp.status().as_u16()
            ),
            Err(e) => tracing::warn!("Hugging Face token refresh failed: {}", e.without_url()),
        }
        self.token()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{body_string_contains, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const WHOAMI: &str = r#"{"type":"user","name":"alice","auth":{"accessToken":{"role":"read"}}}"#;

    async fn setup() -> (MockServer, Arc<MemorySecretStore>, HfCredentials) {
        let server = MockServer::start().await;
        Mock::given(path("/api/whoami-v2"))
            .and(header("authorization", "Bearer hf_good"))
            .respond_with(ResponseTemplate::new(200).set_body_string(WHOAMI))
            .mount(&server)
            .await;
        Mock::given(path("/api/whoami-v2"))
            .and(header("authorization", "Bearer oauth_new"))
            .respond_with(ResponseTemplate::new(200).set_body_string(WHOAMI))
            .mount(&server)
            .await;
        Mock::given(path("/api/whoami-v2"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;
        let store = Arc::new(MemorySecretStore::default());
        let creds = HfCredentials::new(store.clone(), HubClient::new(&server.uri()));
        (server, store, creds)
    }

    #[tokio::test]
    async fn pasted_token_is_validated_then_stored() {
        let (_server, store, creds) = setup().await;
        assert_eq!(creds.token(), None);
        assert_eq!(creds.account().await, HfAccount::signed_out());

        let err = creds.set_token("hf_bad").await.unwrap_err();
        assert!(matches!(err, HubError::InvalidRequest(_)));
        assert!(!err.to_string().contains("hf_bad"));
        assert_eq!(store.get(KEY_TOKEN), None);
        assert!(creds.set_token("   ").await.is_err());

        let acct = creds.set_token(" hf_good ").await.unwrap();
        assert_eq!(acct.username.as_deref(), Some("alice"));
        assert_eq!(acct.method.as_deref(), Some("token"));
        assert_eq!(store.get(KEY_TOKEN).as_deref(), Some("hf_good"));
        assert_eq!(creds.token().as_deref(), Some("hf_good"));

        let acct = creds.account().await;
        assert!(acct.signed_in);
        assert_eq!(acct.username.as_deref(), Some("alice"));
        assert_eq!(acct.method.as_deref(), Some("token"));

        creds.sign_out();
        assert_eq!(creds.token(), None);
        assert!(!creds.account().await.signed_in);
    }

    #[tokio::test]
    async fn account_survives_whoami_failure() {
        let (_server, store, creds) = setup().await;
        store.set(KEY_TOKEN, "hf_revoked").unwrap();
        let acct = creds.account().await;
        assert!(acct.signed_in);
        assert_eq!(acct.username, None);
    }

    #[tokio::test]
    async fn oauth_token_preferred_until_expired() {
        let (_server, store, creds) = setup().await;
        store.set(KEY_TOKEN, "hf_good").unwrap();
        creds.store_oauth_tokens("oauth_a", Some("refresh_a"), Some(now() + 3600));
        // OAuth replaces the pasted token.
        assert_eq!(store.get(KEY_TOKEN), None);
        assert_eq!(creds.token().as_deref(), Some("oauth_a"));
        creds.store_oauth_tokens("oauth_b", None, Some(now() - 10));
        assert_eq!(creds.token(), None);
        creds.store_oauth_tokens("oauth_c", None, None);
        assert_eq!(creds.token().as_deref(), Some("oauth_c"));
        assert_eq!(store.get(KEY_OAUTH_EXPIRES_AT), None);
    }

    #[tokio::test]
    async fn refresh_uses_refresh_token_grant() {
        let (server, store, creds) = setup().await;
        Mock::given(method("POST"))
            .and(path("/oauth/token"))
            .and(body_string_contains("grant_type=refresh_token"))
            .and(body_string_contains("refresh_token=refresh_old"))
            .and(body_string_contains(
                "client_id=https%3A%2F%2Flocalrouter.ai%2Foauth%2Fhuggingface-client.json",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"access_token":"oauth_new","refresh_token":"refresh_new","expires_in":2592000,"token_type":"bearer"}"#,
            ))
            .expect(1)
            .mount(&server)
            .await;

        // Not due: no request.
        creds.store_oauth_tokens("oauth_cur", Some("refresh_old"), Some(now() + 3600));
        assert_eq!(
            creds.refresh_if_needed().await.as_deref(),
            Some("oauth_cur")
        );

        // Expires within 60 s → refreshed, refresh token rotated.
        creds.store_oauth_tokens("oauth_cur", Some("refresh_old"), Some(now() + 30));
        assert_eq!(
            creds.refresh_if_needed().await.as_deref(),
            Some("oauth_new")
        );
        assert_eq!(store.get(KEY_OAUTH_REFRESH).as_deref(), Some("refresh_new"));
        let exp = creds.expires_at().unwrap();
        assert!(exp > now() + 2_000_000);

        let acct = creds.account().await;
        assert_eq!(acct.method.as_deref(), Some("oauth"));
        assert_eq!(acct.username.as_deref(), Some("alice"));
        assert_eq!(acct.expires_at, Some(exp));
    }

    #[tokio::test]
    async fn failed_refresh_of_expired_token_falls_back() {
        let (server, store, creds) = setup().await;
        Mock::given(method("POST"))
            .and(path("/oauth/token"))
            .respond_with(
                ResponseTemplate::new(400).set_body_string(r#"{"error":"invalid_grant"}"#),
            )
            .mount(&server)
            .await;
        creds.store_oauth_tokens("oauth_old", Some("refresh_x"), Some(now() - 5));
        assert_eq!(creds.refresh_if_needed().await, None);
        // A still-valid token is kept when the refresh fails.
        creds.store_oauth_tokens("oauth_old", Some("refresh_x"), Some(now() + 20));
        assert_eq!(
            creds.refresh_if_needed().await.as_deref(),
            Some("oauth_old")
        );
        assert_eq!(store.get(KEY_OAUTH_REFRESH).as_deref(), Some("refresh_x"));
    }

    #[test]
    fn constants() {
        assert!(HF_OAUTH_SCOPES.contains(&"gated-repos"));
        assert!(HF_TOKEN_URL.ends_with("/oauth/token"));
        assert!(HF_AUTHORIZE_URL.ends_with("/oauth/authorize"));
        let creds =
            HfCredentials::new(Arc::new(MemorySecretStore::default()), HubClient::default());
        assert_eq!(creds.token_url(), HF_TOKEN_URL);
    }
}
