//! GitHub Copilot OAuth provider implementation
//!
//! Implements OAuth 2.0 Device Code Flow for GitHub Copilot subscriptions.
//!
//! Flow:
//! 1. Request device code from GitHub
//! 2. User visits verification URL and enters code
//! 3. Poll for authorization completion
//! 4. Receive access token

use async_trait::async_trait;
use chrono::Utc;
use reqwest::Client;
use serde::Deserialize;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, error, info};

use super::{OAuthCredentials, OAuthFlowResult, OAuthProvider};
use lr_types::{AppError, AppResult};

const GITHUB_CLIENT_ID: &str = "Ov23li8tweQw6odWQebz";
const GITHUB_DEVICE_CODE_URL: &str = "https://github.com/login/device/code";
const GITHUB_ACCESS_TOKEN_URL: &str = "https://github.com/login/oauth/access_token";

/// GitHub device code response
#[derive(Debug, Deserialize)]
struct DeviceCodeResponse {
    device_code: String,
    user_code: String,
    verification_uri: String,
    expires_in: u64,
    interval: u64,
}

/// GitHub access token response
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum TokenResponse {
    Success {
        access_token: String,
        token_type: String,
        scope: String,
    },
    Pending {
        error: String,
        error_description: String,
    },
}

/// OAuth flow state
#[derive(Debug, Clone)]
struct FlowState {
    device_code: String,
    user_code: String,
    verification_uri: String,
    interval: u64,
    started_at: i64,
    expires_in: u64,
    /// Earliest unix timestamp at which the next poll may hit the token endpoint.
    /// Set to `started_at` initially so the very first poll proceeds immediately.
    /// Updated after every server response (and bumped extra on `slow_down`).
    next_poll_after: i64,
}

/// Outcome of applying a parsed token response to the current flow state.
/// Caller decides whether to clear state based on the variant.
enum TokenAction {
    /// Authorization complete; caller should clear state.
    Success { access_token: String },
    /// Still pending or polled too fast; state already updated, caller keeps it.
    Pending { instructions: String },
    /// Terminal error (`expired_token`, `access_denied`, unknown); caller clears state.
    Terminal { message: String },
}

/// Apply a parsed token response to the flow state. Returns the action the
/// caller should take. Mutates `state.next_poll_after` and, on `slow_down`,
/// `state.interval`.
///
/// Pulled out as a free function so it can be unit-tested without a live
/// HTTP server.
fn apply_token_response(state: &mut FlowState, response: TokenResponse, now: i64) -> TokenAction {
    match response {
        TokenResponse::Success { access_token, .. } => TokenAction::Success { access_token },
        TokenResponse::Pending {
            error,
            error_description,
        } => {
            match error.as_str() {
                "authorization_pending" => {
                    state.next_poll_after = now + state.interval as i64;
                    TokenAction::Pending {
                        instructions: "Waiting for authorization...".to_string(),
                    }
                }
                "slow_down" => {
                    // RFC 8628 §3.5: client MUST increase interval by 5 seconds.
                    state.interval = state.interval.saturating_add(5);
                    state.next_poll_after = now + state.interval as i64;
                    TokenAction::Pending {
                        instructions: "Polling too frequently, backing off...".to_string(),
                    }
                }
                _ => TokenAction::Terminal {
                    message: format!("{}: {}", error, error_description),
                },
            }
        }
    }
}

/// GitHub Copilot OAuth provider
pub struct GitHubCopilotOAuthProvider {
    client: Client,
    current_flow: Arc<RwLock<Option<FlowState>>>,
}

impl GitHubCopilotOAuthProvider {
    /// Create a new GitHub Copilot OAuth provider
    pub fn new() -> Self {
        Self {
            client: crate::http_client::default_client(),
            current_flow: Arc::new(RwLock::new(None)),
        }
    }
}

impl Default for GitHubCopilotOAuthProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl OAuthProvider for GitHubCopilotOAuthProvider {
    fn provider_id(&self) -> &str {
        "github-copilot"
    }

    fn provider_name(&self) -> &str {
        "GitHub Copilot"
    }

    async fn start_oauth_flow(&self) -> AppResult<OAuthFlowResult> {
        info!("Starting GitHub Copilot OAuth flow");

        // Request device code
        let response = self
            .client
            .post(GITHUB_DEVICE_CODE_URL)
            .header("Accept", "application/json")
            .header("Content-Type", "application/json")
            .json(&serde_json::json!({
                "client_id": GITHUB_CLIENT_ID,
                "scope": "read:user"
            }))
            .send()
            .await
            .map_err(|e| {
                AppError::Provider(format!("Failed to request GitHub device code: {}", e))
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(AppError::Provider(format!(
                "GitHub device code request failed {}: {}",
                status, error_text
            )));
        }

        let device_response: DeviceCodeResponse = response.json().await.map_err(|e| {
            AppError::Provider(format!("Failed to parse device code response: {}", e))
        })?;

        debug!(
            "Device code received: user_code={}, expires_in={}s",
            device_response.user_code, device_response.expires_in
        );

        // Store flow state
        let started_at = Utc::now().timestamp();
        let flow_state = FlowState {
            device_code: device_response.device_code,
            user_code: device_response.user_code.clone(),
            verification_uri: device_response.verification_uri.clone(),
            interval: device_response.interval,
            started_at,
            expires_in: device_response.expires_in,
            // Allow first poll immediately; subsequent polls are gated by the
            // server's reported interval (and bumped on `slow_down`).
            next_poll_after: started_at,
        };

        *self.current_flow.write().await = Some(flow_state);

        Ok(OAuthFlowResult::Pending {
            user_code: Some(device_response.user_code),
            verification_url: device_response.verification_uri,
            instructions:
                "Visit the verification URL and enter the code to authorize GitHub Copilot access."
                    .to_string(),
        })
    }

    async fn poll_oauth_status(&self) -> AppResult<OAuthFlowResult> {
        // Snapshot the values we need from the read guard, then drop it so we
        // never hold a tokio RwLock guard across an `.await`. Holding a read
        // guard across the HTTP request below and then taking a write lock to
        // mutate state would deadlock if any other task tried to acquire
        // either lock half. (See https://github.com/tokio-rs/tokio/discussions/3147.)
        let (device_code, user_code, verification_uri, started_at, expires_in, next_poll_after) = {
            let flow = self.current_flow.read().await;
            let s = flow
                .as_ref()
                .ok_or_else(|| AppError::Provider("No OAuth flow in progress".to_string()))?;
            (
                s.device_code.clone(),
                s.user_code.clone(),
                s.verification_uri.clone(),
                s.started_at,
                s.expires_in,
                s.next_poll_after,
            )
        };

        let now = Utc::now().timestamp();

        // Check if the device code expired (RFC 8628 §3.5 `expired_token` —
        // we detect it locally so we don't bother the server).
        if now > started_at + expires_in as i64 {
            *self.current_flow.write().await = None;
            return Ok(OAuthFlowResult::Error {
                message: "OAuth flow expired. Please start again.".to_string(),
            });
        }

        // Server-side rate gate: if the caller polls faster than `interval`
        // (or our `slow_down` backoff), short-circuit without an HTTP request.
        // This prevents UI tick rates from translating into actual traffic.
        if now < next_poll_after {
            return Ok(OAuthFlowResult::Pending {
                user_code: Some(user_code),
                verification_url: verification_uri,
                instructions: "Waiting before next poll...".to_string(),
            });
        }

        // Poll for access token
        let response = self
            .client
            .post(GITHUB_ACCESS_TOKEN_URL)
            .header("Accept", "application/json")
            .header("Content-Type", "application/json")
            .json(&serde_json::json!({
                "client_id": GITHUB_CLIENT_ID,
                "device_code": device_code,
                "grant_type": "urn:ietf:params:oauth:grant-type:device_code"
            }))
            .send()
            .await
            .map_err(|e| {
                AppError::Provider(format!("Failed to poll for GitHub access token: {}", e))
            })?;

        // RFC 6749 §5.2: the OAuth token endpoint returns 400 with a JSON body
        // containing the `error` field for `authorization_pending`,
        // `slow_down`, `expired_token`, and `access_denied`. We must therefore
        // read and parse the body BEFORE reacting to a non-success status.
        let status = response.status();
        let response_text = response.text().await.unwrap_or_default();
        debug!(
            "GitHub token poll response (status {}): {}",
            status, response_text
        );

        let token_response: Option<TokenResponse> = serde_json::from_str(&response_text).ok();

        let token_response = match token_response {
            Some(t) => t,
            None => {
                // Body wasn't a recognizable OAuth response. Surface as a real
                // error and propagate the HTTP status if it was a failure.
                error!(
                    "Failed to parse GitHub token response (status {}): {}",
                    status, response_text
                );
                return Err(AppError::Provider(format!(
                    "GitHub token request failed {}: {}",
                    status, response_text
                )));
            }
        };

        // Apply the response to the flow state under a single write lock,
        // then act on the resulting TokenAction outside the lock.
        let action = {
            let mut guard = self.current_flow.write().await;
            match guard.as_mut() {
                Some(state) => apply_token_response(state, token_response, now),
                None => {
                    // Flow was cancelled mid-poll. Treat as terminal.
                    return Ok(OAuthFlowResult::Error {
                        message: "OAuth flow was cancelled".to_string(),
                    });
                }
            }
        };

        match action {
            TokenAction::Success { access_token } => {
                info!("GitHub Copilot OAuth flow completed successfully");
                *self.current_flow.write().await = None;
                Ok(OAuthFlowResult::Success {
                    credentials: OAuthCredentials {
                        provider_id: "github-copilot".to_string(),
                        access_token,
                        refresh_token: None,
                        expires_at: None,
                        account_id: None,
                        created_at: Utc::now(),
                    },
                })
            }
            TokenAction::Pending { instructions } => Ok(OAuthFlowResult::Pending {
                user_code: Some(user_code),
                verification_url: verification_uri,
                instructions,
            }),
            TokenAction::Terminal { message } => {
                error!("GitHub OAuth terminal error: {}", message);
                *self.current_flow.write().await = None;
                Ok(OAuthFlowResult::Error { message })
            }
        }
    }

    async fn refresh_tokens(&self, _credentials: &OAuthCredentials) -> AppResult<OAuthCredentials> {
        // GitHub Copilot tokens don't expire, so no refresh needed
        Err(AppError::Provider(
            "GitHub Copilot tokens do not require refresh".to_string(),
        ))
    }

    async fn cancel_oauth_flow(&self) {
        *self.current_flow.write().await = None;
        info!("GitHub Copilot OAuth flow cancelled");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_provider_info() {
        let provider = GitHubCopilotOAuthProvider::new();
        assert_eq!(provider.provider_id(), "github-copilot");
        assert_eq!(provider.provider_name(), "GitHub Copilot");
    }

    #[tokio::test]
    async fn test_cancel_flow() {
        let provider = GitHubCopilotOAuthProvider::new();

        // Manually set a flow
        let now = Utc::now().timestamp();
        *provider.current_flow.write().await = Some(FlowState {
            device_code: "test".to_string(),
            user_code: "TEST-CODE".to_string(),
            verification_uri: "https://github.com/login/device".to_string(),
            interval: 5,
            started_at: now,
            expires_in: 900,
            next_poll_after: now,
        });

        assert!(provider.current_flow.read().await.is_some());

        provider.cancel_oauth_flow().await;

        assert!(provider.current_flow.read().await.is_none());
    }

    fn test_state(now: i64, interval: u64) -> FlowState {
        FlowState {
            device_code: "DEVICE".to_string(),
            user_code: "USER-CODE".to_string(),
            verification_uri: "https://github.com/login/device".to_string(),
            interval,
            started_at: now,
            expires_in: 900,
            next_poll_after: now,
        }
    }

    #[test]
    fn test_apply_authorization_pending_sets_next_poll_after() {
        let now = 1_000_000;
        let mut state = test_state(now, 5);
        let action = apply_token_response(
            &mut state,
            TokenResponse::Pending {
                error: "authorization_pending".to_string(),
                error_description: "user has not yet authorized".to_string(),
            },
            now,
        );
        assert!(matches!(action, TokenAction::Pending { .. }));
        assert_eq!(state.next_poll_after, now + 5);
        assert_eq!(state.interval, 5, "interval unchanged on pending");
    }

    #[test]
    fn test_apply_slow_down_bumps_interval_by_5() {
        let now = 1_000_000;
        let mut state = test_state(now, 5);
        let action = apply_token_response(
            &mut state,
            TokenResponse::Pending {
                error: "slow_down".to_string(),
                error_description: "polling too fast".to_string(),
            },
            now,
        );
        assert!(matches!(action, TokenAction::Pending { .. }));
        assert_eq!(state.interval, 10, "RFC 8628 §3.5: bump by exactly 5");
        assert_eq!(state.next_poll_after, now + 10);
    }

    #[test]
    fn test_apply_expired_token_is_terminal() {
        let now = 1_000_000;
        let mut state = test_state(now, 5);
        let action = apply_token_response(
            &mut state,
            TokenResponse::Pending {
                error: "expired_token".to_string(),
                error_description: "the device code expired".to_string(),
            },
            now,
        );
        match action {
            TokenAction::Terminal { message } => {
                assert!(message.contains("expired_token"));
            }
            _ => panic!("expected Terminal"),
        }
    }

    #[test]
    fn test_apply_access_denied_is_terminal() {
        let now = 1_000_000;
        let mut state = test_state(now, 5);
        let action = apply_token_response(
            &mut state,
            TokenResponse::Pending {
                error: "access_denied".to_string(),
                error_description: "user declined".to_string(),
            },
            now,
        );
        assert!(matches!(action, TokenAction::Terminal { .. }));
    }

    #[test]
    fn test_apply_unknown_error_is_terminal() {
        let now = 1_000_000;
        let mut state = test_state(now, 5);
        let action = apply_token_response(
            &mut state,
            TokenResponse::Pending {
                error: "incomprehensible_widget_error".to_string(),
                error_description: "well that's new".to_string(),
            },
            now,
        );
        assert!(matches!(action, TokenAction::Terminal { .. }));
    }

    #[test]
    fn test_apply_success_returns_token() {
        let now = 1_000_000;
        let mut state = test_state(now, 5);
        let action = apply_token_response(
            &mut state,
            TokenResponse::Success {
                access_token: "ghu_xxx".to_string(),
                token_type: "bearer".to_string(),
                scope: "read:user".to_string(),
            },
            now,
        );
        match action {
            TokenAction::Success { access_token } => assert_eq!(access_token, "ghu_xxx"),
            _ => panic!("expected Success"),
        }
    }

    #[tokio::test]
    async fn test_poll_returns_pending_without_http_when_gated() {
        // If next_poll_after is in the future, poll_oauth_status must return
        // Pending WITHOUT making any HTTP call. We verify by setting
        // next_poll_after far in the future — if the gate fails, the test
        // would either hit network or block; with the gate it returns
        // immediately.
        let provider = GitHubCopilotOAuthProvider::new();
        let now = Utc::now().timestamp();
        *provider.current_flow.write().await = Some(FlowState {
            device_code: "DEVICE".to_string(),
            user_code: "USER-CODE".to_string(),
            verification_uri: "https://github.com/login/device".to_string(),
            interval: 5,
            started_at: now,
            expires_in: 900,
            next_poll_after: now + 1_000_000, // far future
        });

        let result = provider.poll_oauth_status().await.unwrap();
        match result {
            OAuthFlowResult::Pending {
                user_code,
                verification_url,
                ..
            } => {
                assert_eq!(user_code.as_deref(), Some("USER-CODE"));
                assert_eq!(verification_url, "https://github.com/login/device");
            }
            other => panic!("expected Pending, got {:?}", other),
        }

        // State must NOT have been cleared by the rate-gated path.
        assert!(provider.current_flow.read().await.is_some());
    }

    #[tokio::test]
    async fn test_poll_clears_state_on_local_expiry() {
        let provider = GitHubCopilotOAuthProvider::new();
        let now = Utc::now().timestamp();
        *provider.current_flow.write().await = Some(FlowState {
            device_code: "DEVICE".to_string(),
            user_code: "USER-CODE".to_string(),
            verification_uri: "https://github.com/login/device".to_string(),
            interval: 5,
            // started 2 hours ago, 15-minute expiry → expired locally
            started_at: now - 7200,
            expires_in: 900,
            next_poll_after: now,
        });

        let result = provider.poll_oauth_status().await.unwrap();
        assert!(matches!(result, OAuthFlowResult::Error { .. }));
        assert!(provider.current_flow.read().await.is_none());
    }
}
