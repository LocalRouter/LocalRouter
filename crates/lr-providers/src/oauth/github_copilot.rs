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
    /// Current minimum seconds between token-endpoint polls. Starts at the
    /// value GitHub returned from `/login/device/code` and is bumped by 5 s
    /// per RFC 8628 §3.5 every time we see a `slow_down` response.
    interval: u64,
    started_at: i64,
    expires_in: u64,
    /// Earliest Unix-timestamp at which the next token-endpoint poll is
    /// permitted. Guards against the frontend polling faster than `interval`
    /// and ensures we actually honour `slow_down` backoff — without this
    /// gate GitHub just keeps returning `slow_down` with ever-increasing
    /// intervals and never issues the token, even after the user has
    /// authorized in the browser.
    next_poll_after: i64,
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

impl GitHubCopilotOAuthProvider {
    /// Dispatch on a parsed GitHub token-endpoint response.
    ///
    /// Extracted so the caller can try parsing the body BEFORE checking the
    /// HTTP status — GitHub returns HTTP 400 for `authorization_pending` and
    /// friends per RFC 6749 §5.2, so a status-first check would turn every
    /// normal poll into a surfaced error.
    ///
    /// This method also owns poll-scheduling side effects (updating
    /// `next_poll_after` and bumping `interval` on `slow_down` per RFC 8628
    /// §3.5) so the in-memory flow state stays in sync with what GitHub
    /// expects between polls.
    async fn handle_token_response(
        &self,
        token_response: TokenResponse,
        flow_state: &FlowState,
    ) -> AppResult<OAuthFlowResult> {
        match token_response {
            TokenResponse::Success { access_token, .. } => {
                info!("GitHub Copilot OAuth flow completed successfully");

                // Clear flow state
                *self.current_flow.write().await = None;

                let credentials = OAuthCredentials {
                    provider_id: "github-copilot".to_string(),
                    access_token,
                    refresh_token: None, // GitHub Copilot tokens don't have refresh tokens
                    expires_at: None,    // GitHub Copilot tokens don't expire
                    account_id: None,
                    created_at: Utc::now(),
                };

                Ok(OAuthFlowResult::Success { credentials })
            }
            TokenResponse::Pending {
                error,
                error_description,
            } => match error.as_str() {
                "authorization_pending" => {
                    // Schedule the next poll respecting GitHub's minimum
                    // interval. The frontend's 2 s poll cadence is faster
                    // than GitHub's default (usually 5 s), so the
                    // interval-gate in `poll_oauth_status` is what
                    // actually spaces the real HTTP calls.
                    self.schedule_next_poll(flow_state.interval).await;
                    Ok(OAuthFlowResult::Pending {
                        user_code: Some(flow_state.user_code.clone()),
                        verification_url: flow_state.verification_uri.clone(),
                        instructions: "Waiting for authorization...".to_string(),
                    })
                }
                "slow_down" => {
                    // RFC 8628 §3.5: "the client MUST add 5 seconds to
                    // this polling interval." GitHub enforces this hard —
                    // if we keep polling at the old rate it will keep
                    // returning slow_down (with an ever-increasing
                    // `interval` hint) and never issue the token, even
                    // after the user authorizes in the browser.
                    let new_interval = flow_state.interval.saturating_add(5);
                    debug!(
                        "GitHub returned slow_down; bumping poll interval from {}s to {}s",
                        flow_state.interval, new_interval
                    );
                    self.bump_interval_and_schedule(new_interval).await;
                    Ok(OAuthFlowResult::Pending {
                        user_code: Some(flow_state.user_code.clone()),
                        verification_url: flow_state.verification_uri.clone(),
                        instructions: "Polling too frequently, please wait...".to_string(),
                    })
                }
                "expired_token" => {
                    // Device code has expired — clear flow so a restart works.
                    *self.current_flow.write().await = None;
                    Ok(OAuthFlowResult::Error {
                        message: "Device code expired before authorization. Please start again."
                            .to_string(),
                    })
                }
                "access_denied" => {
                    // User explicitly denied — clear flow.
                    *self.current_flow.write().await = None;
                    Ok(OAuthFlowResult::Error {
                        message: format!("Authorization denied: {}", error_description),
                    })
                }
                _ => {
                    // Unknown OAuth error — surface it and clear flow.
                    error!("GitHub OAuth error: {} - {}", error, error_description);
                    *self.current_flow.write().await = None;
                    Ok(OAuthFlowResult::Error {
                        message: format!("{}: {}", error, error_description),
                    })
                }
            },
        }
    }

    /// Push `next_poll_after` out to `now + seconds` without touching the
    /// stored `interval`. Used for `authorization_pending`, where GitHub's
    /// minimum interval is unchanged.
    async fn schedule_next_poll(&self, seconds: u64) {
        let mut guard = self.current_flow.write().await;
        if let Some(state) = guard.as_mut() {
            state.next_poll_after = Utc::now().timestamp() + seconds as i64;
        }
    }

    /// Raise both the stored `interval` and `next_poll_after`. Used for
    /// `slow_down`, where GitHub has told us to back off.
    async fn bump_interval_and_schedule(&self, new_interval: u64) {
        let mut guard = self.current_flow.write().await;
        if let Some(state) = guard.as_mut() {
            state.interval = new_interval;
            state.next_poll_after = Utc::now().timestamp() + new_interval as i64;
        }
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
        let flow_state = FlowState {
            device_code: device_response.device_code,
            user_code: device_response.user_code.clone(),
            verification_uri: device_response.verification_uri.clone(),
            interval: device_response.interval,
            started_at: Utc::now().timestamp(),
            expires_in: device_response.expires_in,
            // First poll is allowed immediately; after that, `interval`
            // (and any slow_down bumps) gate subsequent polls.
            next_poll_after: 0,
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
        // Snapshot the flow state and drop the read lock before any HTTP I/O
        // or subsequent write-lock acquisition. Holding a tokio RwLock read
        // guard across an `.await` that later tries to take a write lock on
        // the same RwLock is a classic deadlock, not just bad hygiene.
        let flow_state = {
            let flow = self.current_flow.read().await;
            flow.as_ref()
                .ok_or_else(|| AppError::Provider("No OAuth flow in progress".to_string()))?
                .clone()
        };

        // Check if expired
        let now = Utc::now().timestamp();
        if now > flow_state.started_at + flow_state.expires_in as i64 {
            *self.current_flow.write().await = None;
            return Ok(OAuthFlowResult::Error {
                message: "OAuth flow expired. Please start again.".to_string(),
            });
        }

        // Poll-rate gate. The frontend polls this Tauri command every 2 s
        // for snappy UI, but GitHub's token endpoint enforces the
        // `interval` returned in the device-code response (and bumps it by
        // +5 s on every `slow_down` per RFC 8628 §3.5). If we hit GitHub
        // faster than that, `slow_down` keeps escalating indefinitely and
        // GitHub never issues the token even after the user authorizes in
        // the browser. Return `Pending` without an HTTP request when we're
        // inside the backoff window.
        if now < flow_state.next_poll_after {
            return Ok(OAuthFlowResult::Pending {
                user_code: Some(flow_state.user_code.clone()),
                verification_url: flow_state.verification_uri.clone(),
                instructions: "Waiting for authorization...".to_string(),
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
                "device_code": flow_state.device_code,
                "grant_type": "urn:ietf:params:oauth:grant-type:device_code"
            }))
            .send()
            .await
            .map_err(|e| {
                AppError::Provider(format!("Failed to poll for GitHub access token: {}", e))
            })?;

        let status = response.status();
        let response_text = response.text().await.unwrap_or_default();

        debug!(
            "GitHub token poll response (status {}): {}",
            status, response_text
        );

        // Per RFC 6749 §5.2, the token endpoint returns HTTP 400 for
        // `authorization_pending`, `slow_down`, `expired_token`, and
        // `access_denied`. We must therefore parse the body BEFORE treating
        // any non-2xx as a hard error — otherwise every normal poll during
        // the user's authorization window bubbles up as an error and the
        // frontend's retry budget is exhausted in seconds.
        if let Ok(token_response) = serde_json::from_str::<TokenResponse>(&response_text) {
            return self
                .handle_token_response(token_response, &flow_state)
                .await;
        }

        // Body was not a recognizable TokenResponse. Only now is non-2xx a
        // true transport-level failure we should surface.
        if !status.is_success() {
            error!("GitHub token request failed {}: {}", status, response_text);
            return Err(AppError::Provider(format!(
                "GitHub token request failed {}: {}",
                status, response_text
            )));
        }

        // 2xx but body doesn't parse — keep the original diagnostic.
        error!(
            "Failed to parse GitHub token response (status {}): {}",
            status, response_text
        );
        Err(AppError::Provider(format!(
            "Failed to parse token response: {}",
            response_text
        )))
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
        *provider.current_flow.write().await = Some(FlowState {
            device_code: "test".to_string(),
            user_code: "TEST-CODE".to_string(),
            verification_uri: "https://github.com/login/device".to_string(),
            interval: 5,
            started_at: Utc::now().timestamp(),
            expires_in: 900,
            next_poll_after: 0,
        });

        assert!(provider.current_flow.read().await.is_some());

        provider.cancel_oauth_flow().await;

        assert!(provider.current_flow.read().await.is_none());
    }

    // --- Regression coverage for the "enter code, app never updates" bug ---
    //
    // GitHub's token endpoint returns HTTP 400 with a JSON error body for
    // `authorization_pending`, `slow_down`, `expired_token`, and
    // `access_denied` (RFC 6749 §5.2). Previously the polling loop checked
    // `!status.is_success()` before attempting to deserialize the body, so
    // every normal "still waiting" poll bubbled up as an error and exhausted
    // the frontend's 3-strikes retry budget. The fix parses the body first
    // and lets these codes round-trip through `OAuthFlowResult::Pending`.

    fn seeded_flow_state() -> FlowState {
        FlowState {
            device_code: "dev_code".to_string(),
            user_code: "ABCD-1234".to_string(),
            verification_uri: "https://github.com/login/device".to_string(),
            interval: 5,
            started_at: Utc::now().timestamp(),
            expires_in: 900,
            next_poll_after: 0,
        }
    }

    #[test]
    fn test_token_response_deserializes_success() {
        let body = r#"{
            "access_token": "gho_xxx",
            "token_type": "bearer",
            "scope": "read:user"
        }"#;
        let parsed: TokenResponse = serde_json::from_str(body).unwrap();
        assert!(matches!(parsed, TokenResponse::Success { .. }));
    }

    #[test]
    fn test_token_response_deserializes_pending_error_body() {
        let body = r#"{
            "error": "authorization_pending",
            "error_description": "The authorization request is still pending."
        }"#;
        let parsed: TokenResponse = serde_json::from_str(body).unwrap();
        match parsed {
            TokenResponse::Pending { error, .. } => assert_eq!(error, "authorization_pending"),
            other => panic!("expected Pending, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_handle_authorization_pending_returns_pending() {
        let provider = GitHubCopilotOAuthProvider::new();
        *provider.current_flow.write().await = Some(seeded_flow_state());

        let parsed = TokenResponse::Pending {
            error: "authorization_pending".to_string(),
            error_description: "still waiting".to_string(),
        };
        let result = provider
            .handle_token_response(parsed, &seeded_flow_state())
            .await
            .unwrap();

        match result {
            OAuthFlowResult::Pending { user_code, .. } => {
                assert_eq!(user_code.as_deref(), Some("ABCD-1234"));
            }
            other => panic!("expected Pending, got {other:?}"),
        }

        // Flow should still be present — we're not done yet.
        assert!(provider.current_flow.read().await.is_some());
    }

    #[tokio::test]
    async fn test_handle_slow_down_returns_pending() {
        let provider = GitHubCopilotOAuthProvider::new();
        *provider.current_flow.write().await = Some(seeded_flow_state());

        let parsed = TokenResponse::Pending {
            error: "slow_down".to_string(),
            error_description: "back off".to_string(),
        };
        let result = provider
            .handle_token_response(parsed, &seeded_flow_state())
            .await
            .unwrap();

        assert!(matches!(result, OAuthFlowResult::Pending { .. }));
        assert!(provider.current_flow.read().await.is_some());
    }

    #[tokio::test]
    async fn test_handle_expired_token_clears_flow_and_errors() {
        let provider = GitHubCopilotOAuthProvider::new();
        *provider.current_flow.write().await = Some(seeded_flow_state());

        let parsed = TokenResponse::Pending {
            error: "expired_token".to_string(),
            error_description: "device code expired".to_string(),
        };
        let result = provider
            .handle_token_response(parsed, &seeded_flow_state())
            .await
            .unwrap();

        match result {
            OAuthFlowResult::Error { message } => {
                assert!(message.to_lowercase().contains("expired"));
            }
            other => panic!("expected Error, got {other:?}"),
        }
        // Terminal error — flow must be cleared so restart works.
        assert!(provider.current_flow.read().await.is_none());
    }

    #[tokio::test]
    async fn test_handle_access_denied_clears_flow_and_errors() {
        let provider = GitHubCopilotOAuthProvider::new();
        *provider.current_flow.write().await = Some(seeded_flow_state());

        let parsed = TokenResponse::Pending {
            error: "access_denied".to_string(),
            error_description: "user said no".to_string(),
        };
        let result = provider
            .handle_token_response(parsed, &seeded_flow_state())
            .await
            .unwrap();

        assert!(matches!(result, OAuthFlowResult::Error { .. }));
        assert!(provider.current_flow.read().await.is_none());
    }

    #[tokio::test]
    async fn test_handle_unknown_error_clears_flow_and_errors() {
        let provider = GitHubCopilotOAuthProvider::new();
        *provider.current_flow.write().await = Some(seeded_flow_state());

        let parsed = TokenResponse::Pending {
            error: "some_new_error".to_string(),
            error_description: "unknown".to_string(),
        };
        let result = provider
            .handle_token_response(parsed, &seeded_flow_state())
            .await
            .unwrap();

        match result {
            OAuthFlowResult::Error { message } => assert!(message.contains("some_new_error")),
            other => panic!("expected Error, got {other:?}"),
        }
        assert!(provider.current_flow.read().await.is_none());
    }

    #[tokio::test]
    async fn test_handle_success_clears_flow_and_returns_credentials() {
        let provider = GitHubCopilotOAuthProvider::new();
        *provider.current_flow.write().await = Some(seeded_flow_state());

        let parsed = TokenResponse::Success {
            access_token: "gho_test".to_string(),
            token_type: "bearer".to_string(),
            scope: "read:user".to_string(),
        };
        let result = provider
            .handle_token_response(parsed, &seeded_flow_state())
            .await
            .unwrap();

        match result {
            OAuthFlowResult::Success { credentials } => {
                assert_eq!(credentials.provider_id, "github-copilot");
                assert_eq!(credentials.access_token, "gho_test");
                assert!(credentials.refresh_token.is_none());
                assert!(credentials.expires_at.is_none());
            }
            other => panic!("expected Success, got {other:?}"),
        }
        // Successful terminal state — flow must be cleared.
        assert!(provider.current_flow.read().await.is_none());
    }

    // --- Regression coverage for the runaway slow_down loop ---
    //
    // GitHub's device-flow token endpoint enforces a minimum poll interval
    // (RFC 8628 §3.5) and bumps it by +5 s on every `slow_down` response.
    // If the provider polls faster than that, GitHub escalates forever and
    // never issues the token even after the user completes authorization.
    // The fix records a `next_poll_after` timestamp on the FlowState and
    // updates the stored `interval` on each `slow_down`.

    #[tokio::test]
    async fn test_slow_down_bumps_interval_by_5s_and_schedules_next_poll() {
        let provider = GitHubCopilotOAuthProvider::new();
        *provider.current_flow.write().await = Some(seeded_flow_state());

        let parsed = TokenResponse::Pending {
            error: "slow_down".to_string(),
            error_description: "too fast".to_string(),
        };
        let before = Utc::now().timestamp();
        let result = provider
            .handle_token_response(parsed, &seeded_flow_state())
            .await
            .unwrap();

        assert!(matches!(result, OAuthFlowResult::Pending { .. }));

        // Flow must still exist — slow_down is non-terminal.
        let guard = provider.current_flow.read().await;
        let state = guard.as_ref().expect("flow should still be present");
        // Seeded interval is 5 s; slow_down adds 5 → 10 s.
        assert_eq!(state.interval, 10);
        // next_poll_after should land ~now + 10 s.
        assert!(
            state.next_poll_after >= before + 9 && state.next_poll_after <= before + 12,
            "next_poll_after {} not within expected window around {}+10",
            state.next_poll_after,
            before
        );
    }

    #[tokio::test]
    async fn test_authorization_pending_schedules_next_poll_at_interval() {
        let provider = GitHubCopilotOAuthProvider::new();
        let mut seed = seeded_flow_state();
        seed.interval = 7;
        *provider.current_flow.write().await = Some(seed.clone());

        let parsed = TokenResponse::Pending {
            error: "authorization_pending".to_string(),
            error_description: "still waiting".to_string(),
        };
        let before = Utc::now().timestamp();
        let _ = provider.handle_token_response(parsed, &seed).await.unwrap();

        let guard = provider.current_flow.read().await;
        let state = guard.as_ref().expect("flow should still be present");
        // authorization_pending must NOT change the interval.
        assert_eq!(state.interval, 7);
        // But it SHOULD schedule the next HTTP poll ~now + interval.
        assert!(
            state.next_poll_after >= before + 6 && state.next_poll_after <= before + 9,
            "next_poll_after {} not within expected window around {}+7",
            state.next_poll_after,
            before
        );
    }

    #[tokio::test]
    async fn test_poll_gate_returns_pending_without_http_when_in_backoff() {
        // If we ever loosen the gate, a live HTTP round-trip would happen
        // here and the test would start depending on network state. The
        // gate is what prevents GitHub's escalation; test its contract.
        let provider = GitHubCopilotOAuthProvider::new();
        let mut seed = seeded_flow_state();
        // Block the next poll 60 s into the future.
        seed.next_poll_after = Utc::now().timestamp() + 60;
        *provider.current_flow.write().await = Some(seed);

        let result = provider.poll_oauth_status().await.unwrap();
        assert!(
            matches!(result, OAuthFlowResult::Pending { .. }),
            "poll during backoff window should return Pending without HTTP, got {result:?}"
        );
    }
}
