//! Live access tokens for OAuth (subscription) providers.
//!
//! Provider instances live in the registry for the whole process lifetime, so
//! a token read at construction time goes stale: it expires, or the user
//! reconnects and the instance keeps sending the token it snapshotted at
//! startup. Both show up as an upstream 401 that no amount of reconnecting
//! clears.
//!
//! An [`OAuthTokenSource`] is the process-wide owner of one provider's access
//! token. Providers ask it for a token per request; it serves the token it has
//! while that token is good, re-reads the keychain when it isn't, and exchanges
//! the refresh token when the stored one has expired.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use base64::{engine::general_purpose, Engine as _};
use chrono::Utc;
use lr_api_keys::{keychain_trait::KeychainStorage, CachedKeychain};
use lr_oauth::browser::{OAuthFlowConfig, OAuthTokens, TokenExchanger};
use lr_types::{AppError, AppResult};
use parking_lot::RwLock;
use serde::Deserialize;
use tracing::{debug, info, warn};

use super::OAuthCredentials;

/// Refresh this many seconds before the recorded expiry, so a request issued
/// just under the wire doesn't race the expiry on the way upstream.
const REFRESH_SKEW_SECS: i64 = 60;

/// After a failed refresh, don't call the token endpoint again for this long.
/// A revoked grant fails every time, and periodic health checks would
/// otherwise turn that into a steady drum of requests at the provider.
const REFRESH_RETRY_BACKOFF_SECS: i64 = 300;

/// An access token plus what we know about how long it stays good.
#[derive(Clone, Debug, PartialEq)]
struct StoredToken {
    access_token: String,
    /// Unix seconds. `None` when neither the keychain nor the token itself
    /// says — such a token is used until the upstream rejects it.
    expires_at: Option<i64>,
}

impl StoredToken {
    fn needs_refresh(&self, now: i64) -> bool {
        self.expires_at
            .is_some_and(|expires_at| now + REFRESH_SKEW_SECS >= expires_at)
    }
}

/// The current access token for one OAuth provider.
///
/// Cheap to call: the common path is an in-memory read. The keychain is only
/// consulted when the cached token is missing or due, and the refresh grant
/// only when the keychain's copy is due as well.
pub struct OAuthTokenSource {
    config: OAuthFlowConfig,
    cached: RwLock<Option<StoredToken>>,
    /// Serializes refreshes: a burst of concurrent requests that all see an
    /// expired token must spend one refresh token, not one each.
    refresh_lock: tokio::sync::Mutex<()>,
    /// When the last refresh failed (unix seconds) and why, so the same dead
    /// grant isn't retried on every request.
    last_failure: RwLock<Option<(i64, String)>>,
    /// Test seam. When unset the process default keychain is used, re-opened
    /// per load so tokens written by another component are picked up.
    keychain: Option<CachedKeychain>,
}

impl OAuthTokenSource {
    /// Create a token source for the given refresh-grant config.
    pub fn new(config: OAuthFlowConfig) -> Self {
        Self {
            config,
            cached: RwLock::new(None),
            refresh_lock: tokio::sync::Mutex::new(()),
            last_failure: RwLock::new(None),
            keychain: None,
        }
    }

    /// Create a token source backed by a specific keychain (tests).
    pub fn with_keychain(config: OAuthFlowConfig, keychain: CachedKeychain) -> Self {
        Self {
            keychain: Some(keychain),
            ..Self::new(config)
        }
    }

    /// A token that should be accepted upstream, refreshing if it is due.
    pub async fn access_token(&self) -> AppResult<String> {
        if let Some(token) = self.cached.read().clone() {
            if !token.needs_refresh(Utc::now().timestamp()) {
                return Ok(token.access_token);
            }
        }

        self.resolve(None).await
    }

    /// The upstream rejected `rejected` with a 401 — get a token to retry with.
    ///
    /// If the keychain already holds a different token (the user reconnected,
    /// or a concurrent request refreshed) that one is adopted; otherwise the
    /// refresh grant runs.
    pub async fn refresh_after_unauthorized(&self, rejected: &str) -> AppResult<String> {
        debug!(
            "Access token for {} was rejected; re-resolving",
            self.config.account_id
        );
        self.resolve(Some(rejected)).await
    }

    /// Drop the cached token so the next request re-reads the keychain.
    ///
    /// Called when the stored tokens change underneath us — a reconnect or a
    /// revoke — which is what makes those take effect on already-constructed
    /// provider instances.
    pub fn invalidate(&self) {
        *self.cached.write() = None;
        // New credentials deserve a real attempt, whatever happened last time.
        *self.last_failure.write() = None;
    }

    async fn resolve(&self, rejected: Option<&str>) -> AppResult<String> {
        let _guard = self.refresh_lock.lock().await;

        let keychain = self.keychain();
        let stored = self.load(&keychain);

        // Whoever held the lock before us may have left a usable token behind,
        // and so may a reconnect. Take it rather than spending a refresh.
        if let Some(token) = &stored {
            let superseded = rejected.is_none_or(|rejected| rejected != token.access_token);
            if superseded && !token.needs_refresh(Utc::now().timestamp()) {
                *self.cached.write() = Some(token.clone());
                return Ok(token.access_token.clone());
            }
        }

        self.exchange_refresh_token(&keychain).await
    }

    /// Read the stored token (and its expiry) out of the keychain.
    fn load(&self, keychain: &CachedKeychain) -> Option<StoredToken> {
        let access_token = keychain
            .get(&self.config.keychain_service, &self.key("access_token"))
            .ok()
            .flatten()?;

        // Prefer the expiry recorded next to the token; fall back to the `exp`
        // claim for tokens stored before we started recording it (and for
        // providers whose token endpoint omits `expires_in`).
        let expires_at = keychain
            .get(&self.config.keychain_service, &self.key("expires_at"))
            .ok()
            .flatten()
            .and_then(|raw| raw.parse::<i64>().ok())
            .or_else(|| jwt_expiry(&access_token));

        Some(StoredToken {
            access_token,
            expires_at,
        })
    }

    async fn exchange_refresh_token(&self, keychain: &CachedKeychain) -> AppResult<String> {
        if let Some((at, reason)) = self.last_failure.read().clone() {
            if Utc::now().timestamp() - at < REFRESH_RETRY_BACKOFF_SECS {
                return Err(self.reauth_error(&reason));
            }
        }

        let refresh_token = keychain
            .get(&self.config.keychain_service, &self.key("refresh_token"))
            .ok()
            .flatten()
            .ok_or_else(|| self.record_failure("no refresh token is stored"))?;

        // `refresh_tokens` writes the new access/refresh/expiry back to the
        // keychain, so a restart picks up where we leave off here.
        let tokens = TokenExchanger::new()
            .refresh_tokens(&self.config, &refresh_token, keychain)
            .await
            .map_err(|e| {
                warn!(
                    "OAuth token refresh failed for {}: {}",
                    self.config.account_id, e
                );
                self.record_failure(&e.to_string())
            })?;

        info!(
            "Refreshed OAuth access token for {}",
            self.config.account_id
        );

        let token = StoredToken {
            access_token: tokens.access_token.clone(),
            expires_at: tokens.expires_at.map(|at| at.timestamp()),
        };
        *self.cached.write() = Some(token.clone());
        *self.last_failure.write() = None;

        self.mirror_to_credential_store(&tokens).await;

        Ok(token.access_token)
    }

    /// Keep the credentials the settings UI reads in step with the keychain,
    /// so a background refresh doesn't leave it showing a dead token.
    async fn mirror_to_credential_store(&self, tokens: &OAuthTokens) {
        let Some(storage) = super::credential_store() else {
            return;
        };

        let previous = storage
            .get_credentials(&self.config.account_id)
            .await
            .ok()
            .flatten();

        let credentials = OAuthCredentials {
            provider_id: self.config.account_id.clone(),
            access_token: tokens.access_token.clone(),
            refresh_token: tokens
                .refresh_token
                .clone()
                .or_else(|| previous.as_ref().and_then(|c| c.refresh_token.clone())),
            expires_at: tokens.expires_at.map(|at| at.timestamp()),
            // The account id comes from the JWT at connect time and doesn't
            // change on refresh.
            account_id: previous.as_ref().and_then(|c| c.account_id.clone()),
            created_at: tokens.acquired_at,
        };

        if let Err(e) = storage.store_credentials(&credentials).await {
            warn!(
                "Failed to record refreshed credentials for {}: {}",
                self.config.account_id, e
            );
        }
    }

    fn keychain(&self) -> CachedKeychain {
        self.keychain.clone().unwrap_or_else(|| {
            // A fresh instance re-reads the backing store, so tokens written
            // by the OAuth flow (which holds its own keychain) are visible.
            CachedKeychain::auto().unwrap_or_else(|_| CachedKeychain::system())
        })
    }

    fn key(&self, suffix: &str) -> String {
        format!("{}_{}", self.config.account_id, suffix)
    }

    /// Remember that the refresh failed (so it isn't retried in a hot loop)
    /// and build the error to hand back.
    fn record_failure(&self, reason: &str) -> AppError {
        *self.last_failure.write() = Some((Utc::now().timestamp(), reason.to_string()));
        self.reauth_error(reason)
    }

    fn reauth_error(&self, reason: &str) -> AppError {
        AppError::Provider(format!(
            "OAuth session for '{}' has expired and could not be refreshed ({}). \
             Reconnect the provider in Settings.",
            self.config.account_id, reason
        ))
    }
}

/// `exp` claim of a JWT access token, if it is one.
fn jwt_expiry(token: &str) -> Option<i64> {
    #[derive(Deserialize)]
    struct Claims {
        exp: Option<i64>,
    }

    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return None;
    }

    let payload = general_purpose::URL_SAFE_NO_PAD.decode(parts[1]).ok()?;
    serde_json::from_slice::<Claims>(&payload).ok()?.exp
}

/// Process-wide token sources, one per provider id.
static SOURCES: OnceLock<RwLock<HashMap<String, Arc<OAuthTokenSource>>>> = OnceLock::new();

fn sources() -> &'static RwLock<HashMap<String, Arc<OAuthTokenSource>>> {
    SOURCES.get_or_init(|| RwLock::new(HashMap::new()))
}

/// The shared token source for `provider_id`, or `None` when we don't know how
/// to refresh that provider's tokens.
///
/// Every provider instance for the same id shares one source, so a reconnect
/// invalidates all of them at once.
pub fn token_source(provider_id: &str) -> Option<Arc<OAuthTokenSource>> {
    let config = match provider_id {
        super::openai_codex::PROVIDER_ID => super::openai_codex::refresh_flow_config(),
        _ => return None,
    };

    let mut sources = sources().write();
    Some(Arc::clone(
        sources
            .entry(provider_id.to_string())
            .or_insert_with(|| Arc::new(OAuthTokenSource::new(config))),
    ))
}

/// The stored tokens for `provider_id` changed (reconnect, revoke): drop the
/// cached copy so live provider instances pick the new ones up.
pub fn notify_tokens_updated(provider_id: &str) {
    if let Some(source) = sources().read().get(provider_id) {
        debug!(
            "OAuth tokens updated for {}; invalidating cache",
            provider_id
        );
        source.invalidate();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lr_api_keys::MockKeychain;

    const SERVICE: &str = "LocalRouter-ProviderTokens";

    fn config() -> OAuthFlowConfig {
        OAuthFlowConfig {
            client_id: "client".to_string(),
            client_secret: None,
            auth_url: "https://example.test/authorize".to_string(),
            token_url: "https://example.test/token".to_string(),
            scopes: vec![],
            redirect_uri: "http://localhost:1455/auth/callback".to_string(),
            callback_port: 1455,
            keychain_service: SERVICE.to_string(),
            account_id: "test-provider".to_string(),
            extra_auth_params: HashMap::new(),
            extra_token_params: HashMap::new(),
            expected_issuer: None,
        }
    }

    /// A JWT whose payload is `{"exp": <exp>}` — signature is irrelevant, we
    /// never verify it.
    fn jwt_with_exp(exp: i64) -> String {
        let payload = general_purpose::URL_SAFE_NO_PAD.encode(format!(r#"{{"exp":{}}}"#, exp));
        format!("header.{}.signature", payload)
    }

    fn keychain() -> CachedKeychain {
        CachedKeychain::new(Arc::new(MockKeychain::new()))
    }

    fn source_with(keychain: &CachedKeychain) -> OAuthTokenSource {
        OAuthTokenSource::with_keychain(config(), keychain.clone())
    }

    #[tokio::test]
    async fn serves_the_stored_token() {
        let keychain = keychain();
        keychain
            .store(SERVICE, "test-provider_access_token", "live-token")
            .unwrap();

        let source = source_with(&keychain);
        assert_eq!(source.access_token().await.unwrap(), "live-token");
    }

    #[tokio::test]
    async fn picks_up_a_token_written_after_construction() {
        // What a reconnect looks like: the source has cached a token, the
        // keychain then gets a new one, and the cache is invalidated.
        let keychain = keychain();
        keychain
            .store(SERVICE, "test-provider_access_token", "old-token")
            .unwrap();

        let source = source_with(&keychain);
        assert_eq!(source.access_token().await.unwrap(), "old-token");

        keychain
            .store(SERVICE, "test-provider_access_token", "new-token")
            .unwrap();
        source.invalidate();

        assert_eq!(source.access_token().await.unwrap(), "new-token");
    }

    #[tokio::test]
    async fn adopts_a_replacement_token_after_a_401() {
        let keychain = keychain();
        keychain
            .store(SERVICE, "test-provider_access_token", "reconnected-token")
            .unwrap();

        let source = source_with(&keychain);

        // The rejected token is not the stored one, so the stored one is used
        // instead of burning a refresh.
        assert_eq!(
            source
                .refresh_after_unauthorized("stale-token")
                .await
                .unwrap(),
            "reconnected-token"
        );
    }

    #[tokio::test]
    async fn expired_token_without_a_refresh_token_asks_for_a_reconnect() {
        let keychain = keychain();
        keychain
            .store(
                SERVICE,
                "test-provider_access_token",
                &jwt_with_exp(Utc::now().timestamp() - 60),
            )
            .unwrap();

        let source = source_with(&keychain);
        let err = source.access_token().await.unwrap_err().to_string();
        assert!(err.contains("Reconnect"), "unexpected error: {err}");
    }

    #[tokio::test]
    async fn missing_credentials_ask_for_a_reconnect() {
        let source = source_with(&keychain());
        let err = source.access_token().await.unwrap_err().to_string();
        assert!(err.contains("Reconnect"), "unexpected error: {err}");
    }

    #[tokio::test]
    async fn a_failed_refresh_is_not_retried_until_the_backoff_lapses() {
        let keychain = keychain();
        keychain
            .store(
                SERVICE,
                "test-provider_access_token",
                &jwt_with_exp(Utc::now().timestamp() - 60),
            )
            .unwrap();

        let source = source_with(&keychain);
        assert!(source.access_token().await.is_err());
        let failed_at = source.last_failure.read().clone().unwrap().0;

        // A second call inside the window reuses the recorded failure rather
        // than calling the token endpoint again — and does not push the
        // window out.
        assert!(source.access_token().await.is_err());
        assert_eq!(source.last_failure.read().clone().unwrap().0, failed_at);

        // Reconnecting clears it so the new credentials get a real attempt.
        source.invalidate();
        assert!(source.last_failure.read().is_none());
    }

    #[tokio::test]
    async fn recorded_expiry_wins_over_the_jwt_claim() {
        // Token that claims to be expired, but the recorded expiry (written by
        // the token exchanger, which already applies its own safety buffer)
        // says it is still good.
        let keychain = keychain();
        keychain
            .store(
                SERVICE,
                "test-provider_access_token",
                &jwt_with_exp(Utc::now().timestamp() - 60),
            )
            .unwrap();
        keychain
            .store(
                SERVICE,
                "test-provider_expires_at",
                &(Utc::now().timestamp() + 3600).to_string(),
            )
            .unwrap();

        let source = source_with(&keychain);
        assert!(source.access_token().await.is_ok());
    }

    #[test]
    fn reads_the_exp_claim_of_a_jwt() {
        assert_eq!(
            jwt_expiry(&jwt_with_exp(1_700_000_000)),
            Some(1_700_000_000)
        );
    }

    #[test]
    fn opaque_tokens_have_no_expiry() {
        assert_eq!(jwt_expiry("sk-ant-oat01-not-a-jwt"), None);
        assert_eq!(jwt_expiry("header.not-base64!.sig"), None);
        assert_eq!(jwt_expiry("two.parts"), None);
    }

    #[test]
    fn needs_refresh_only_within_the_skew_window() {
        let now = 1_700_000_000;
        let token = |expires_at| StoredToken {
            access_token: "t".to_string(),
            expires_at,
        };

        assert!(!token(Some(now + REFRESH_SKEW_SECS + 1)).needs_refresh(now));
        assert!(token(Some(now + REFRESH_SKEW_SECS)).needs_refresh(now));
        assert!(token(Some(now - 1)).needs_refresh(now));
        // Unknown expiry: used until upstream says otherwise.
        assert!(!token(None).needs_refresh(now));
    }

    #[test]
    fn token_source_is_shared_per_provider() {
        let first = token_source(super::super::openai_codex::PROVIDER_ID).unwrap();
        let second = token_source(super::super::openai_codex::PROVIDER_ID).unwrap();
        assert!(Arc::ptr_eq(&first, &second));
        assert!(token_source("not-an-oauth-provider").is_none());
    }
}
