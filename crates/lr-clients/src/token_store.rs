//! In-memory OAuth token store for short-lived access tokens
//!
//! This module provides a temporary token storage system for OAuth 2.0 client credentials flow.
//! Tokens are stored in-memory only (never persisted to disk) and expire after 1 hour.

#![allow(dead_code)]

use crate::utils::crypto;
use crate::utils::errors::{AppError, AppResult};
use chrono::{DateTime, Duration, Utc};
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;

/// Default token expiration time (1 hour)
const TOKEN_EXPIRATION_SECS: i64 = 3600;

/// OAuth access token with metadata
#[derive(Debug, Clone)]
struct TokenInfo {
    /// The access token (random string, stored for reference but looked up via HashMap key)
    _token: String,
    /// Client ID that owns this token
    client_id: String,
    /// When the token was created (kept for debugging/auditing)
    _created_at: DateTime<Utc>,
    /// When the token expires
    expires_at: DateTime<Utc>,
}

impl TokenInfo {
    /// Create a new token info
    fn new(token: String, client_id: String) -> Self {
        let created_at = Utc::now();
        let expires_at = created_at + Duration::seconds(TOKEN_EXPIRATION_SECS);

        Self {
            _token: token,
            client_id,
            _created_at: created_at,
            expires_at,
        }
    }

    /// Check if the token is expired
    fn is_expired(&self) -> bool {
        Utc::now() > self.expires_at
    }

    /// Get remaining seconds until expiration
    fn expires_in(&self) -> i64 {
        (self.expires_at - Utc::now()).num_seconds().max(0)
    }
}

/// In-memory store for OAuth access tokens
///
/// Tokens are:
/// - Generated via OAuth 2.0 client credentials flow
/// - Valid for 1 hour (3600 seconds)
/// - Stored in-memory only (never persisted)
/// - Automatically cleaned up when expired
pub struct TokenStore {
    /// Map of token -> token info
    tokens: Arc<RwLock<HashMap<String, TokenInfo>>>,
}

impl TokenStore {
    /// Create a new empty token store
    pub fn new() -> Self {
        Self {
            tokens: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Generate a new access token for a client
    ///
    /// # Arguments
    /// * `client_id` - The client ID to generate a token for
    ///
    /// # Returns
    /// Returns (access_token, expires_in) tuple where expires_in is in seconds
    pub fn generate_token(&self, client_id: String) -> AppResult<(String, i64)> {
        // Generate a random token (same format as API keys)
        let token = crypto::generate_api_key()
            .map_err(|e| AppError::Config(format!("Failed to generate access token: {}", e)))?;

        let token_info = TokenInfo::new(token.clone(), client_id);
        let expires_in = token_info.expires_in();

        // Store the token
        self.tokens.write().insert(token.clone(), token_info);

        Ok((token, expires_in))
    }

    /// Verify an access token and return the associated client_id
    ///
    /// Returns None if the token doesn't exist or is expired.
    /// Expired tokens are automatically removed.
    pub fn verify_token(&self, token: &str) -> Option<String> {
        let mut tokens = self.tokens.write();

        match tokens.get(token) {
            Some(info) => {
                if info.is_expired() {
                    // Token expired, remove it
                    tokens.remove(token);
                    None
                } else {
                    // Token is valid
                    Some(info.client_id.clone())
                }
            }
            None => None,
        }
    }

    /// Revoke a specific token (e.g., for logout)
    pub fn revoke_token(&self, token: &str) -> bool {
        self.tokens.write().remove(token).is_some()
    }

    /// Revoke all tokens for a specific client
    ///
    /// Returns the number of tokens revoked
    pub fn revoke_client_tokens(&self, client_id: &str) -> usize {
        let mut tokens = self.tokens.write();

        let tokens_to_remove: Vec<String> = tokens
            .iter()
            .filter(|(_, info)| info.client_id == client_id)
            .map(|(token, _)| token.clone())
            .collect();

        let count = tokens_to_remove.len();
        for token in tokens_to_remove {
            tokens.remove(&token);
        }

        count
    }

    /// Clean up all expired tokens
    ///
    /// Returns the number of tokens removed
    pub fn cleanup_expired(&self) -> usize {
        let mut tokens = self.tokens.write();

        let expired_tokens: Vec<String> = tokens
            .iter()
            .filter(|(_, info)| info.is_expired())
            .map(|(token, _)| token.clone())
            .collect();

        let count = expired_tokens.len();
        for token in expired_tokens {
            tokens.remove(&token);
        }

        count
    }

    /// Get the number of active (non-expired) tokens
    pub fn active_token_count(&self) -> usize {
        let tokens = self.tokens.read();
        tokens.iter().filter(|(_, info)| !info.is_expired()).count()
    }

    /// Get the total number of tokens (including expired)
    pub fn total_token_count(&self) -> usize {
        self.tokens.read().len()
    }
}

impl Default for TokenStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_token() {
        let store = TokenStore::new();

        let (token, expires_in) = store
            .generate_token("client1".to_string())
            .expect("Failed to generate token");

        // Token should be a valid string
        assert!(!token.is_empty());

        // Expires in should be approximately 1 hour (3600 seconds)
        assert!((3599..=3600).contains(&expires_in));

        // Token should be valid
        let client_id = store.verify_token(&token);
        assert_eq!(client_id, Some("client1".to_string()));
    }

    #[test]
    fn test_verify_invalid_token() {
        let store = TokenStore::new();

        let client_id = store.verify_token("invalid-token");
        assert_eq!(client_id, None);
    }

    #[test]
    fn test_revoke_token() {
        let store = TokenStore::new();

        let (token, _) = store
            .generate_token("client1".to_string())
            .expect("Failed to generate token");

        // Token should be valid before revocation
        assert!(store.verify_token(&token).is_some());

        // Revoke the token
        let revoked = store.revoke_token(&token);
        assert!(revoked);

        // Token should be invalid after revocation
        assert!(store.verify_token(&token).is_none());

        // Revoking again should return false
        let revoked_again = store.revoke_token(&token);
        assert!(!revoked_again);
    }

    #[test]
    fn test_revoke_client_tokens() {
        let store = TokenStore::new();

        // Generate 3 tokens for client1
        let (token1, _) = store.generate_token("client1".to_string()).unwrap();
        let (token2, _) = store.generate_token("client1".to_string()).unwrap();
        let (token3, _) = store.generate_token("client1".to_string()).unwrap();

        // Generate 2 tokens for client2
        let (token4, _) = store.generate_token("client2".to_string()).unwrap();
        let (token5, _) = store.generate_token("client2".to_string()).unwrap();

        assert_eq!(store.total_token_count(), 5);

        // Revoke all tokens for client1
        let count = store.revoke_client_tokens("client1");
        assert_eq!(count, 3);

        // client1 tokens should be invalid
        assert!(store.verify_token(&token1).is_none());
        assert!(store.verify_token(&token2).is_none());
        assert!(store.verify_token(&token3).is_none());

        // client2 tokens should still be valid
        assert_eq!(store.verify_token(&token4), Some("client2".to_string()));
        assert_eq!(store.verify_token(&token5), Some("client2".to_string()));

        assert_eq!(store.total_token_count(), 2);
    }

    #[test]
    fn test_cleanup_expired() {
        let store = TokenStore::new();

        // Generate a token
        let (token, _) = store.generate_token("client1".to_string()).unwrap();

        // Manually expire the token by modifying the expiration time
        {
            let mut tokens = store.tokens.write();
            if let Some(info) = tokens.get_mut(&token) {
                info.expires_at = Utc::now() - Duration::seconds(1); // Already expired
            }
        }

        assert_eq!(store.total_token_count(), 1);

        // Cleanup expired tokens
        let count = store.cleanup_expired();
        assert_eq!(count, 1);
        assert_eq!(store.total_token_count(), 0);
    }

    #[test]
    fn test_verify_removes_expired_token() {
        let store = TokenStore::new();

        // Generate a token
        let (token, _) = store.generate_token("client1".to_string()).unwrap();

        // Manually expire the token
        {
            let mut tokens = store.tokens.write();
            if let Some(info) = tokens.get_mut(&token) {
                info.expires_at = Utc::now() - Duration::seconds(1);
            }
        }

        assert_eq!(store.total_token_count(), 1);

        // Verify should remove the expired token
        let client_id = store.verify_token(&token);
        assert_eq!(client_id, None);
        assert_eq!(store.total_token_count(), 0);
    }

    #[test]
    fn test_active_token_count() {
        let store = TokenStore::new();

        // Generate 2 valid tokens
        let (token1, _) = store.generate_token("client1".to_string()).unwrap();
        let (_token2, _) = store.generate_token("client2".to_string()).unwrap();

        assert_eq!(store.total_token_count(), 2);
        assert_eq!(store.active_token_count(), 2);

        // Expire one token
        {
            let mut tokens = store.tokens.write();
            if let Some(info) = tokens.get_mut(&token1) {
                info.expires_at = Utc::now() - Duration::seconds(1);
            }
        }

        // Total should be 2, active should be 1
        assert_eq!(store.total_token_count(), 2);
        assert_eq!(store.active_token_count(), 1);
    }

    #[test]
    fn test_token_uniqueness() {
        let store = TokenStore::new();

        // Generate multiple tokens
        let (token1, _) = store.generate_token("client1".to_string()).unwrap();
        let (token2, _) = store.generate_token("client1".to_string()).unwrap();
        let (token3, _) = store.generate_token("client2".to_string()).unwrap();

        // All tokens should be unique
        assert_ne!(token1, token2);
        assert_ne!(token2, token3);
        assert_ne!(token1, token3);
    }
}
