//! OAuth flow manager - orchestrates complete OAuth authorization flows
#![allow(dead_code)]

use crate::api_keys::CachedKeychain;
use crate::oauth_browser::{
    generate_pkce_challenge, generate_state, CallbackServerManager, FlowId, FlowStatus,
    OAuthFlowConfig, OAuthFlowResult, OAuthFlowStart, OAuthFlowState, TokenExchanger,
};
use crate::utils::errors::{AppError, AppResult};
use chrono::{Duration, Utc};
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{debug, error, info, warn};

/// Default flow timeout in seconds (5 minutes)
const FLOW_TIMEOUT_SECS: i64 = 300;

/// OAuth flow manager
///
/// Orchestrates complete OAuth authorization code flows with PKCE.
/// Manages multiple concurrent flows, callback servers, and token exchange.
pub struct OAuthFlowManager {
    /// Active OAuth flows
    flows: Arc<RwLock<HashMap<FlowId, OAuthFlowState>>>,

    /// Callback server manager
    callback_manager: Arc<CallbackServerManager>,

    /// Token exchanger
    token_exchanger: Arc<TokenExchanger>,

    /// Keychain for token storage
    keychain: CachedKeychain,
}

impl OAuthFlowManager {
    /// Create a new OAuth flow manager
    pub fn new(keychain: CachedKeychain) -> Self {
        Self {
            flows: Arc::new(RwLock::new(HashMap::new())),
            callback_manager: Arc::new(CallbackServerManager::new()),
            token_exchanger: Arc::new(TokenExchanger::new()),
            keychain,
        }
    }

    /// Start a new OAuth browser flow
    ///
    /// # Arguments
    /// * `config` - OAuth flow configuration
    ///
    /// # Returns
    /// * Flow start information (flow_id, auth_url, state)
    ///
    /// # Example
    /// ```no_run
    /// let config = OAuthFlowConfig { ... };
    /// let start_result = manager.start_flow(config).await?;
    /// // Open start_result.auth_url in browser
    /// // Poll with manager.poll_status(start_result.flow_id)
    /// ```
    pub async fn start_flow(&self, config: OAuthFlowConfig) -> AppResult<OAuthFlowStart> {
        let flow_id = FlowId::new();

        info!(
            "Starting OAuth flow {} for account: {}",
            flow_id, config.account_id
        );

        // Generate PKCE challenge and CSRF state
        let pkce = generate_pkce_challenge();
        let csrf_state = generate_state();

        // Build authorization URL
        let auth_url = self.build_authorization_url(&config, &pkce.code_challenge, &csrf_state)?;

        // Register callback listener
        let callback_rx = self
            .callback_manager
            .register_listener(flow_id, config.callback_port, csrf_state.clone())
            .await?;

        // Create flow state
        let flow_state = OAuthFlowState {
            flow_id,
            config: config.clone(),
            code_verifier: pkce.code_verifier.clone(),
            csrf_state: csrf_state.clone(),
            auth_url: auth_url.clone(),
            started_at: Utc::now(),
            status: FlowStatus::Pending,
            tokens: None,
        };

        // Store flow state
        self.flows.write().insert(flow_id, flow_state);

        // Spawn background task to handle callback and token exchange
        let flows = Arc::clone(&self.flows);
        let callback_manager = Arc::clone(&self.callback_manager);
        let token_exchanger = Arc::clone(&self.token_exchanger);
        let keychain = self.keychain.clone();

        tokio::spawn(async move {
            Self::handle_callback(
                flow_id,
                callback_rx,
                flows,
                callback_manager,
                token_exchanger,
                keychain,
            )
            .await;
        });

        debug!("OAuth flow {} started successfully", flow_id);

        Ok(OAuthFlowStart {
            flow_id,
            auth_url,
            state: csrf_state,
            redirect_uri: config.redirect_uri,
        })
    }

    /// Build authorization URL
    fn build_authorization_url(
        &self,
        config: &OAuthFlowConfig,
        code_challenge: &str,
        state: &str,
    ) -> AppResult<String> {
        let mut url = format!(
            "{}?client_id={}&response_type=code&redirect_uri={}&code_challenge={}&code_challenge_method=S256&state={}",
            config.auth_url,
            urlencoding::encode(&config.client_id),
            urlencoding::encode(&config.redirect_uri),
            urlencoding::encode(code_challenge),
            urlencoding::encode(state),
        );

        // Add scopes if provided
        if !config.scopes.is_empty() {
            let scopes = config.scopes.join(" ");
            url.push_str(&format!("&scope={}", urlencoding::encode(&scopes)));
        }

        // Add extra authorization parameters
        for (key, value) in &config.extra_auth_params {
            url.push_str(&format!(
                "&{}={}",
                urlencoding::encode(key),
                urlencoding::encode(value)
            ));
        }

        Ok(url)
    }

    /// Background task to handle callback and token exchange
    async fn handle_callback(
        flow_id: FlowId,
        callback_rx: tokio::sync::oneshot::Receiver<
            AppResult<crate::oauth_browser::callback_server::CallbackResult>,
        >,
        flows: Arc<RwLock<HashMap<FlowId, OAuthFlowState>>>,
        callback_manager: Arc<CallbackServerManager>,
        token_exchanger: Arc<TokenExchanger>,
        keychain: CachedKeychain,
    ) {
        // Wait for callback with timeout
        let timeout_duration = tokio::time::Duration::from_secs(FLOW_TIMEOUT_SECS as u64);
        let callback_result = tokio::time::timeout(timeout_duration, callback_rx).await;

        // Get flow config before we update status
        let (config, code_verifier) = {
            let flows = flows.read();
            let flow = flows.get(&flow_id);
            match flow {
                Some(f) => (f.config.clone(), f.code_verifier.clone()),
                None => {
                    error!("Flow {} not found in handle_callback", flow_id);
                    return;
                }
            }
        };

        match callback_result {
            Ok(Ok(Ok(callback))) => {
                info!("Received callback for flow {}", flow_id);

                // Update status to exchanging token
                {
                    let mut flows = flows.write();
                    if let Some(flow) = flows.get_mut(&flow_id) {
                        flow.status = FlowStatus::ExchangingToken;
                    }
                }

                // Exchange code for tokens
                let exchange_result = token_exchanger
                    .exchange_code(&config, &callback.code, &code_verifier, &keychain)
                    .await;

                // Update flow with result
                let mut flows = flows.write();
                if let Some(flow) = flows.get_mut(&flow_id) {
                    match exchange_result {
                        Ok(tokens) => {
                            info!("Token exchange successful for flow {}", flow_id);
                            flow.status = FlowStatus::Success;
                            flow.tokens = Some(tokens);
                        }
                        Err(e) => {
                            error!("Token exchange failed for flow {}: {}", flow_id, e);
                            flow.status = FlowStatus::Error {
                                message: format!("Token exchange failed: {}", e),
                            };
                        }
                    }
                }
            }
            Ok(Ok(Err(e))) => {
                // Callback error
                error!("Callback error for flow {}: {}", flow_id, e);
                let mut flows = flows.write();
                if let Some(flow) = flows.get_mut(&flow_id) {
                    flow.status = FlowStatus::Error {
                        message: format!("Callback error: {}", e),
                    };
                }
            }
            Ok(Err(_)) => {
                // Channel closed unexpectedly
                warn!("Callback channel closed for flow {}", flow_id);
                let mut flows = flows.write();
                if let Some(flow) = flows.get_mut(&flow_id) {
                    flow.status = FlowStatus::Error {
                        message: "Callback cancelled".to_string(),
                    };
                }
            }
            Err(_) => {
                // Timeout
                warn!(
                    "Flow {} timed out after {} seconds",
                    flow_id, FLOW_TIMEOUT_SECS
                );
                let mut flows = flows.write();
                if let Some(flow) = flows.get_mut(&flow_id) {
                    flow.status = FlowStatus::Timeout;
                }
            }
        }

        // Cleanup callback listener
        callback_manager.cancel_flow(flow_id, config.callback_port);
    }

    /// Poll flow status
    ///
    /// # Arguments
    /// * `flow_id` - Flow identifier from start_flow()
    ///
    /// # Returns
    /// * Current flow status and result
    pub fn poll_status(&self, flow_id: FlowId) -> AppResult<OAuthFlowResult> {
        let flows = self.flows.read();
        let flow = flows
            .get(&flow_id)
            .ok_or_else(|| AppError::OAuthBrowser(format!("Flow {} not found", flow_id)))?;

        // Calculate time remaining
        let elapsed = Utc::now()
            .signed_duration_since(flow.started_at)
            .num_seconds();
        let time_remaining = Some(FLOW_TIMEOUT_SECS - elapsed).filter(|&t| t > 0);

        let result = match &flow.status {
            FlowStatus::Pending => OAuthFlowResult::Pending { time_remaining },
            FlowStatus::ExchangingToken => OAuthFlowResult::ExchangingToken,
            FlowStatus::Success => {
                let tokens = flow.tokens.clone().ok_or_else(|| {
                    AppError::OAuthBrowser("No tokens in successful flow".to_string())
                })?;
                OAuthFlowResult::Success { tokens }
            }
            FlowStatus::Error { message } => OAuthFlowResult::Error {
                message: message.clone(),
            },
            FlowStatus::Timeout => OAuthFlowResult::Timeout,
            FlowStatus::Cancelled => OAuthFlowResult::Cancelled,
        };

        Ok(result)
    }

    /// Cancel a flow
    ///
    /// # Arguments
    /// * `flow_id` - Flow identifier to cancel
    pub fn cancel_flow(&self, flow_id: FlowId) -> AppResult<()> {
        let mut flows = self.flows.write();
        let flow = flows
            .get_mut(&flow_id)
            .ok_or_else(|| AppError::OAuthBrowser(format!("Flow {} not found", flow_id)))?;

        info!("Cancelling flow {}", flow_id);

        // Update status
        flow.status = FlowStatus::Cancelled;

        // Cancel callback listener
        self.callback_manager
            .cancel_flow(flow_id, flow.config.callback_port);

        Ok(())
    }

    /// Remove old completed flows
    ///
    /// Cleans up flows that completed more than 1 hour ago.
    pub fn cleanup_flows(&self) {
        let cutoff = Utc::now() - Duration::hours(1);
        let mut flows = self.flows.write();

        let before_count = flows.len();
        flows.retain(|_, flow| {
            // Keep flows that are still pending or recent
            matches!(
                flow.status,
                FlowStatus::Pending | FlowStatus::ExchangingToken
            ) || flow.started_at > cutoff
        });

        let removed = before_count - flows.len();
        if removed > 0 {
            debug!("Cleaned up {} old flows", removed);
        }
    }

    /// Get count of active flows
    #[allow(dead_code)]
    pub fn active_flow_count(&self) -> usize {
        self.flows.read().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn create_test_config() -> OAuthFlowConfig {
        OAuthFlowConfig {
            client_id: "test_client".to_string(),
            client_secret: None,
            auth_url: "https://example.com/oauth/authorize".to_string(),
            token_url: "https://example.com/oauth/token".to_string(),
            scopes: vec!["read".to_string(), "write".to_string()],
            redirect_uri: "http://localhost:8080/callback".to_string(),
            callback_port: 8080,
            keychain_service: "TestService".to_string(),
            account_id: "test_account".to_string(),
            extra_auth_params: HashMap::new(),
            extra_token_params: HashMap::new(),
        }
    }

    #[test]
    fn test_build_authorization_url() {
        let keychain = CachedKeychain::system();
        let manager = OAuthFlowManager::new(keychain);
        let config = create_test_config();

        let url = manager
            .build_authorization_url(&config, "test_challenge", "test_state")
            .unwrap();

        assert!(url.contains("client_id=test_client"));
        assert!(url.contains("response_type=code"));
        assert!(url.contains("code_challenge=test_challenge"));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains("state=test_state"));
        assert!(url.contains("scope=read%20write"));
    }

    #[test]
    fn test_build_authorization_url_extra_params() {
        let keychain = CachedKeychain::system();
        let manager = OAuthFlowManager::new(keychain);
        let mut config = create_test_config();
        config
            .extra_auth_params
            .insert("prompt".to_string(), "consent".to_string());

        let url = manager
            .build_authorization_url(&config, "test_challenge", "test_state")
            .unwrap();

        assert!(url.contains("prompt=consent"));
    }

    #[test]
    fn test_flow_manager_creation() {
        let keychain = CachedKeychain::system();
        let manager = OAuthFlowManager::new(keychain);
        assert_eq!(manager.active_flow_count(), 0);
    }

    #[test]
    fn test_cleanup_flows() {
        let keychain = CachedKeychain::system();
        let manager = OAuthFlowManager::new(keychain);

        // Manually insert an old completed flow
        let flow_id = FlowId::new();
        let config = create_test_config();
        let old_flow = OAuthFlowState {
            flow_id,
            config,
            code_verifier: "test".to_string(),
            csrf_state: "test".to_string(),
            auth_url: "test".to_string(),
            started_at: Utc::now() - Duration::hours(2),
            status: FlowStatus::Success,
            tokens: None,
        };

        manager.flows.write().insert(flow_id, old_flow);
        assert_eq!(manager.active_flow_count(), 1);

        manager.cleanup_flows();
        assert_eq!(manager.active_flow_count(), 0);
    }
}
