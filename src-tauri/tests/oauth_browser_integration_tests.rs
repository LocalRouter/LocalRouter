//! Integration tests for OAuth browser flow functionality
//!
//! Tests the unified oauth_browser module including:
//! - PKCE generation and uniqueness
//! - Flow lifecycle management
//! - Concurrent flow handling
//! - Provider integration (Anthropic, OpenAI)

use localrouter::api_keys::CachedKeychain;
use localrouter::oauth_browser::{
    generate_pkce_challenge, generate_state, FlowId, OAuthFlowConfig, OAuthFlowManager,
    OAuthFlowResult,
};
use std::collections::HashMap;

#[test]
fn test_pkce_generation() {
    let pkce = generate_pkce_challenge();

    // Verify code verifier length (64 characters)
    assert_eq!(pkce.code_verifier.len(), 64);

    // Verify code verifier uses only URL-safe characters
    assert!(pkce
        .code_verifier
        .chars()
        .all(|c| c.is_ascii_alphanumeric()));

    // Verify code challenge is not empty
    assert!(!pkce.code_challenge.is_empty());

    // Verify method is S256
    assert_eq!(pkce.code_challenge_method, "S256");

    // Verify code challenge is base64url encoded (no padding)
    assert!(!pkce.code_challenge.contains('='));
}

#[test]
fn test_pkce_uniqueness() {
    let pkce1 = generate_pkce_challenge();
    let pkce2 = generate_pkce_challenge();

    // Each call should generate different values
    assert_ne!(pkce1.code_verifier, pkce2.code_verifier);
    assert_ne!(pkce1.code_challenge, pkce2.code_challenge);
}

#[test]
fn test_pkce_batch_uniqueness() {
    // Generate 100 PKCE challenges and verify all are unique
    let mut verifiers = std::collections::HashSet::new();
    let mut challenges = std::collections::HashSet::new();

    for _ in 0..100 {
        let pkce = generate_pkce_challenge();
        assert!(
            verifiers.insert(pkce.code_verifier.clone()),
            "Generated duplicate PKCE verifier"
        );
        assert!(
            challenges.insert(pkce.code_challenge.clone()),
            "Generated duplicate PKCE challenge"
        );
    }

    assert_eq!(verifiers.len(), 100);
    assert_eq!(challenges.len(), 100);
}

#[test]
fn test_state_generation() {
    let state = generate_state();

    // Verify length (32 characters)
    assert_eq!(state.len(), 32);

    // Verify uses only alphanumeric characters
    assert!(state.chars().all(|c| c.is_ascii_alphanumeric()));
}

#[test]
fn test_state_uniqueness() {
    let state1 = generate_state();
    let state2 = generate_state();

    // Each call should generate different values
    assert_ne!(state1, state2);
}

#[test]
fn test_state_batch_uniqueness() {
    // Generate 100 states and verify all are unique
    let mut states = std::collections::HashSet::new();

    for _ in 0..100 {
        let state = generate_state();
        assert!(states.insert(state), "Generated duplicate state");
    }

    assert_eq!(states.len(), 100);
}

#[test]
fn test_flow_id_uniqueness() {
    let id1 = FlowId::new();
    let id2 = FlowId::new();

    assert_ne!(id1, id2);
}

#[test]
fn test_flow_id_display() {
    let id = FlowId::new();
    let display = format!("{}", id);

    assert!(!display.is_empty());
    assert_eq!(display, id.as_uuid().to_string());
}

#[test]
fn test_flow_manager_creation() {
    let keychain = CachedKeychain::system();
    let manager = OAuthFlowManager::new(keychain);

    assert_eq!(manager.active_flow_count(), 0);
}

#[tokio::test]
async fn test_flow_config_creation() {
    let config = OAuthFlowConfig {
        client_id: "test-client".to_string(),
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
    };

    assert_eq!(config.client_id, "test-client");
    assert_eq!(config.callback_port, 8080);
    assert_eq!(config.scopes.len(), 2);
}

#[test]
fn test_flow_result_is_pending() {
    let pending = OAuthFlowResult::Pending {
        time_remaining: Some(300),
    };
    assert!(pending.is_pending());
    assert!(!pending.is_complete());
    assert!(!pending.is_success());

    let exchanging = OAuthFlowResult::ExchangingToken;
    assert!(exchanging.is_pending());
    assert!(!exchanging.is_complete());
}

#[test]
fn test_flow_result_is_complete() {
    let error = OAuthFlowResult::Error {
        message: "test error".to_string(),
    };
    assert!(!error.is_pending());
    assert!(error.is_complete());
    assert!(!error.is_success());

    let timeout = OAuthFlowResult::Timeout;
    assert!(!timeout.is_pending());
    assert!(timeout.is_complete());
    assert!(!timeout.is_success());

    let cancelled = OAuthFlowResult::Cancelled;
    assert!(!cancelled.is_pending());
    assert!(cancelled.is_complete());
    assert!(!cancelled.is_success());
}

#[test]
fn test_flow_result_extract_tokens() {
    use chrono::Utc;
    use localrouter::oauth_browser::OAuthTokens;

    let tokens = OAuthTokens {
        access_token: "test_token".to_string(),
        refresh_token: Some("refresh".to_string()),
        token_type: "Bearer".to_string(),
        expires_in: Some(3600),
        expires_at: None,
        scope: Some("read write".to_string()),
        acquired_at: Utc::now(),
    };

    let success = OAuthFlowResult::Success {
        tokens: tokens.clone(),
    };

    let extracted = success.tokens();
    assert!(extracted.is_some());
    assert_eq!(extracted.unwrap().access_token, "test_token");

    let error = OAuthFlowResult::Error {
        message: "test".to_string(),
    };
    assert!(error.tokens().is_none());
}

#[tokio::test]
async fn test_flow_cleanup() {
    let keychain = CachedKeychain::system();
    let manager = OAuthFlowManager::new(keychain);

    // Verify cleanup doesn't crash on empty manager
    manager.cleanup_flows();
    assert_eq!(manager.active_flow_count(), 0);
}

#[test]
fn test_anthropic_oauth_constants() {
    use localrouter::providers::oauth::anthropic_claude::CALLBACK_PORT;

    assert_eq!(CALLBACK_PORT, 1456);
}

#[test]
fn test_openai_oauth_constants() {
    use localrouter::providers::oauth::openai_codex::CALLBACK_PORT;

    assert_eq!(CALLBACK_PORT, 1455);
}

#[test]
fn test_provider_oauth_port_uniqueness() {
    use localrouter::providers::oauth::anthropic_claude::CALLBACK_PORT as ANTHROPIC_PORT;
    use localrouter::providers::oauth::openai_codex::CALLBACK_PORT as OPENAI_PORT;

    // Verify different providers use different ports
    assert_ne!(ANTHROPIC_PORT, OPENAI_PORT);
}

#[cfg(test)]
mod provider_oauth_integration {
    use super::*;
    use localrouter::providers::oauth::{
        anthropic_claude::AnthropicClaudeOAuthProvider, openai_codex::OpenAICodexOAuthProvider,
        OAuthProvider,
    };

    #[test]
    fn test_anthropic_provider_info() {
        let provider = AnthropicClaudeOAuthProvider::default();
        assert_eq!(provider.provider_id(), "anthropic-claude");
        assert_eq!(provider.provider_name(), "Anthropic Claude Pro");
    }

    #[test]
    fn test_openai_provider_info() {
        let provider = OpenAICodexOAuthProvider::default();
        assert_eq!(provider.provider_id(), "openai-codex");
        assert_eq!(provider.provider_name(), "OpenAI ChatGPT Plus/Pro");
    }
}

#[cfg(test)]
mod keychain_integration {
    use super::*;

    #[test]
    fn test_keychain_creation() {
        let keychain = CachedKeychain::system();
        // Just verify it can be created without panicking
        drop(keychain);
    }

    #[test]
    fn test_keychain_system() {
        let keychain = CachedKeychain::system();
        // Just verify it can be created without panicking
        drop(keychain);
    }
}

#[cfg(test)]
mod mcp_oauth_integration {
    use super::*;
    use localrouter::mcp::oauth_browser::{McpOAuthBrowserManager, OAuthBrowserFlowStatus};

    #[test]
    fn test_mcp_oauth_status_serialization() {
        let status = OAuthBrowserFlowStatus::Success { expires_in: 3600 };
        let json = serde_json::to_string(&status).unwrap();
        assert!(json.contains("Success"));
        assert!(json.contains("3600"));
    }

    #[test]
    fn test_mcp_oauth_error_status() {
        let status = OAuthBrowserFlowStatus::Error {
            message: "Test error".to_string(),
        };
        let json = serde_json::to_string(&status).unwrap();
        assert!(json.contains("Error"));
        assert!(json.contains("Test error"));
    }

    #[test]
    fn test_mcp_oauth_timeout_status() {
        let status = OAuthBrowserFlowStatus::Timeout;
        let json = serde_json::to_string(&status).unwrap();
        assert!(json.contains("Timeout"));
    }
}
